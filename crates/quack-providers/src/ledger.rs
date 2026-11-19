//! Session-scoped record of outbound HTTP activity and blocked egress attempts.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use quack_config::is_loopback;
use quack_protocol::{EgressEntry, EgressKind, EgressStats};
use reqwest::Url;

const MAX_LEDGER_ENTRIES: usize = 1000;

/// Callback invoked whenever a new entry is recorded into an [`EgressLedger`].
pub type LedgerHook = Arc<dyn Fn(&EgressEntry) + Send + Sync>;

/// Input fields for recording an egress event into an [`EgressLedger`].
#[derive(Debug, Clone)]
pub struct EgressDraft {
    pub kind: EgressKind,
    pub url: String,
    pub protocol: String,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub status: Option<u16>,
    pub redactions: u32,
    pub blocked_reason: Option<String>,
}

impl EgressDraft {
    /// A request about to start: nothing sent or received yet, no outcome. The builder
    /// methods fill in what happened before the draft is recorded.
    pub fn new(kind: EgressKind, url: impl Into<String>, protocol: impl Into<String>) -> Self {
        Self {
            kind,
            url: url.into(),
            protocol: protocol.into(),
            bytes_sent: 0,
            bytes_received: 0,
            status: None,
            redactions: 0,
            blocked_reason: None,
        }
    }

    /// Bytes sent on the wire, in total.
    pub fn sent(mut self, bytes: u64) -> Self {
        self.bytes_sent = bytes;
        self
    }

    /// Bytes received from the wire, in total.
    pub fn received(mut self, bytes: u64) -> Self {
        self.bytes_received = bytes;
        self
    }

    /// The HTTP status the request ended with, when it got that far.
    pub fn status(mut self, status: Option<u16>) -> Self {
        self.status = status;
        self
    }

    /// Redactions to attribute to this request.
    pub fn redactions(mut self, count: u32) -> Self {
        self.redactions = count;
        self
    }

    /// The request failed or was refused, with the reason.
    pub fn blocked(mut self, reason: impl Into<String>) -> Self {
        self.blocked_reason = Some(reason.into());
        self
    }
}

#[derive(Default)]
struct LedgerInner {
    entries: Vec<EgressEntry>,
    stats: EgressStats,
    next_seq: u64,
    hook: Option<LedgerHook>,
}

/// Thread-safe accumulator for outbound network requests and egress policy blocks.
#[derive(Default)]
pub struct EgressLedger {
    inner: Mutex<LedgerInner>,
}

impl std::fmt::Debug for EgressLedger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.inner.lock().map(|g| g.entries.len()).unwrap_or(0);
        f.debug_struct("EgressLedger")
            .field("entries", &count)
            .finish()
    }
}

impl EgressLedger {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(LedgerInner {
                next_seq: 1,
                ..LedgerInner::default()
            }),
        })
    }

    pub fn set_hook(&self, hook: LedgerHook) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.hook = Some(hook);
        }
    }

    /// Replay persisted entries (e.g. during `/resume`), updating cumulative stats and sequence.
    pub fn seed(&self, entries: &[EgressEntry]) {
        if entries.is_empty() {
            return;
        }
        if let Ok(mut guard) = self.inner.lock() {
            seed_inner(&mut guard, entries);
        }
    }

    /// Clear entries and stats for a session switch, then seed any replayed entries without firing hooks.
    pub fn reset(&self, entries: &[EgressEntry]) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.entries.clear();
            guard.stats = EgressStats::default();
            guard.next_seq = 1;
            seed_inner(&mut guard, entries);
        }
    }

    pub fn record(&self, draft: EgressDraft) -> EgressEntry {
        let (host, path) = split_url(&draft.url);
        let local = is_loopback(&draft.url);
        let timestamp_ms = now_ms();
        let (entry, hook) = match self.inner.lock() {
            Ok(mut guard) => {
                let seq = guard.next_seq.max(1);
                guard.next_seq = seq.saturating_add(1);
                let entry = EgressEntry {
                    seq,
                    timestamp_ms,
                    kind: draft.kind,
                    host,
                    path,
                    protocol: draft.protocol,
                    local,
                    bytes_sent: draft.bytes_sent,
                    bytes_received: draft.bytes_received,
                    status: draft.status,
                    redactions: draft.redactions,
                    blocked_reason: draft.blocked_reason,
                };
                bump_stats(&mut guard.stats, &entry);
                push_bounded(&mut guard.entries, entry.clone());
                (entry, guard.hook.clone())
            }
            Err(_) => {
                let entry = EgressEntry {
                    seq: 0,
                    timestamp_ms,
                    kind: draft.kind,
                    host,
                    path,
                    protocol: draft.protocol,
                    local,
                    bytes_sent: draft.bytes_sent,
                    bytes_received: draft.bytes_received,
                    status: draft.status,
                    redactions: draft.redactions,
                    blocked_reason: draft.blocked_reason,
                };
                (entry, None)
            }
        };
        if let Some(hook) = hook {
            hook(&entry);
        }
        entry
    }

    pub fn snapshot(&self) -> (Vec<EgressEntry>, EgressStats) {
        match self.inner.lock() {
            Ok(guard) => (guard.entries.clone(), guard.stats),
            Err(_) => (Vec::new(), EgressStats::default()),
        }
    }
}

fn seed_inner(guard: &mut LedgerInner, entries: &[EgressEntry]) {
    for entry in entries {
        bump_stats(&mut guard.stats, entry);
        guard.next_seq = guard.next_seq.max(entry.seq.saturating_add(1));
        push_bounded(&mut guard.entries, entry.clone());
    }
}

fn push_bounded(entries: &mut Vec<EgressEntry>, entry: EgressEntry) {
    if entries.len() >= MAX_LEDGER_ENTRIES {
        let overflow = entries.len() + 1 - MAX_LEDGER_ENTRIES;
        entries.drain(0..overflow);
    }
    entries.push(entry);
}

fn bump_stats(stats: &mut EgressStats, entry: &EgressEntry) {
    stats.total_requests = stats.total_requests.saturating_add(1);
    stats.total_bytes_sent = stats.total_bytes_sent.saturating_add(entry.bytes_sent);
    stats.total_bytes_received = stats
        .total_bytes_received
        .saturating_add(entry.bytes_received);
    if entry.blocked_reason.is_some()
        || matches!(
            entry.kind,
            EgressKind::BlockedEgress | EgressKind::BlockedRedirect
        )
    {
        stats.blocked_count = stats.blocked_count.saturating_add(1);
    } else if entry.local {
        stats.local_requests = stats.local_requests.saturating_add(1);
    } else {
        stats.remote_requests = stats.remote_requests.saturating_add(1);
    }
    stats.redacted_count = stats
        .redacted_count
        .saturating_add(u64::from(entry.redactions));
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn split_url(raw: &str) -> (String, String) {
    if let Ok(parsed) = Url::parse(raw) {
        let host = match (parsed.host_str(), parsed.port()) {
            (Some(h), Some(p)) => format!("{h}:{p}"),
            (Some(h), None) => h.to_owned(),
            (None, _) => String::new(),
        };
        let path = if parsed.path().is_empty() {
            "/".to_owned()
        } else {
            parsed.path().to_owned()
        };
        return (host, path);
    }
    let trimmed = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))
        .unwrap_or(raw);
    match trimmed.split_once('/') {
        Some((h, p)) => (h.to_owned(), format!("/{p}")),
        None => (trimmed.to_owned(), "/".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn records_stats_hooks_and_replay_cleanly() {
        let ledger = EgressLedger::new();
        let fired = Arc::new(AtomicUsize::new(0));
        let fired_c = Arc::clone(&fired);
        ledger.set_hook(Arc::new(move |_| {
            fired_c.fetch_add(1, Ordering::Relaxed);
        }));

        let first = ledger.record(
            EgressDraft::new(
                EgressKind::Inference,
                "http://127.0.0.1:11434/v1/chat/completions",
                "openai-chat",
            )
            .sent(400)
            .received(120)
            .status(Some(200))
            .redactions(2),
        );
        assert!(first.local);
        assert_eq!(first.host, "127.0.0.1:11434");
        assert_eq!(first.path, "/v1/chat/completions");

        let blocked = ledger.record(
            EgressDraft::new(
                EgressKind::BlockedEgress,
                "https://example.com/secret",
                "fetch",
            )
            .blocked("blocked by network.allow"),
        );
        assert!(!blocked.local);
        assert_eq!(blocked.host, "example.com");

        assert_eq!(fired.load(Ordering::Relaxed), 2);
        let (entries, stats) = ledger.snapshot();
        assert_eq!(entries.len(), 2);
        assert_eq!(stats.local_requests, 1);
        assert_eq!(stats.remote_requests, 0);
        assert_eq!(stats.blocked_count, 1);
        assert_eq!(stats.redacted_count, 2);

        let replayed = EgressLedger::new();
        replayed.seed(&entries);
        let (_, r_stats) = replayed.snapshot();
        assert_eq!(r_stats, stats);
    }
}
