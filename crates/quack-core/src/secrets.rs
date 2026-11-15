//! Pre-egress Secret Shield built on `redact-secret` plus active resolved session
//! credentials (provider headers, runtime OAuth tokens, and remote MCP headers).

use std::sync::OnceLock;

use quack_config::is_loopback;
use quack_config::settings::{NetworkSettings, SecretGuardMode};
use quack_config::sha256::Sha256;
use quack_protocol::SecretFinding;
use redact_secret::{
    Action, BuiltInRegistry, ByteRange, Confidence, DefaultPolicy, Finding, WholeInputLimits,
    redact_with_limits, typed_placeholder_formatter,
};

static BUILTIN_REGISTRY: OnceLock<Result<BuiltInRegistry, String>> = OnceLock::new();

fn builtin_registry() -> Result<&'static BuiltInRegistry, String> {
    let res = BUILTIN_REGISTRY.get_or_init(|| {
        BuiltInRegistry::with_built_in().map_err(|e| format!("secret scanner init: {e}"))
    });
    match res {
        Ok(reg) => Ok(reg),
        Err(msg) => Err(msg.clone()),
    }
}

/// Output of scanning outbound text before it enters model history or crosses HTTP.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub findings: Vec<SecretFinding>,
    pub redacted: String,
    pub redaction_count: u32,
}

/// Extract `host[:port]` from an HTTP(S) URL for display and approval cards.
pub fn host_of(url: &str) -> String {
    let without_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Whether `secret_guard` can be skipped for `base_url` based on policy & loopback rules.
pub fn should_skip_guard(network: &NetworkSettings, base_url: &str) -> bool {
    network.secret_guard == SecretGuardMode::Off
        || (!network.secret_guard_local && is_loopback(base_url))
}

/// Scan `input` at `location` (`"user_prompt"`, `"tool:shell"`, etc.) using `redact-secret`'s
/// built-in detectors plus exact active resolved credentials, honoring `allowlist`.
/// Never includes raw secret values in findings or errors.
pub fn scan_text(
    input: &str,
    location: &str,
    active_secrets: &[String],
    allowlist: &[String],
) -> ScanReport {
    if input.is_empty() {
        return ScanReport {
            findings: Vec::new(),
            redacted: String::new(),
            redaction_count: 0,
        };
    }

    let limits = WholeInputLimits::default();
    let mut builtin_findings = Vec::new();
    if let Ok(reg) = builtin_registry()
        && let Ok(found) = reg.scan_with_limits(input, &DefaultPolicy, &limits)
    {
        builtin_findings = found;
    }

    // Track every span evaluated by built-in detectors so active-token matching does not
    // duplicate or overlap already classified regions.
    let mut occupied: Vec<(usize, usize)> = builtin_findings
        .iter()
        .map(|f| (f.range().start(), f.range().end()))
        .collect();

    let mut active_findings: Vec<Finding> = Vec::new();
    let mut active_seq = 0usize;
    for secret in active_secrets {
        if secret.len() < 8 {
            continue;
        }
        let mut cursor = 0usize;
        while let Some(pos) = input[cursor..].find(secret.as_str()) {
            let start = cursor + pos;
            let end = start + secret.len();
            cursor = end;
            let overlaps = occupied.iter().any(|&(s, e)| start < e && s < end);
            if overlaps {
                continue;
            }
            if let Some(range) = ByteRange::new(start, end) {
                active_seq += 1;
                if let Ok(finding) = Finding::new(
                    format!("active_{active_seq}"),
                    "active_credential",
                    "active_session_credential",
                    Confidence::High,
                    Action::Redact,
                    range,
                ) {
                    occupied.push((start, end));
                    active_findings.push(finding);
                }
            }
        }
    }

    let mut candidates: Vec<Finding> = builtin_findings
        .into_iter()
        .filter(|f| matches!(f.action(), Action::Redact | Action::Block))
        .chain(active_findings)
        .collect();

    candidates.sort_by_key(|f| f.range().start());

    // Enforce strict non-overlap before calling `redact_with_limits`.
    let mut non_overlapping: Vec<Finding> = Vec::with_capacity(candidates.len());
    let mut last_end = 0usize;
    for f in candidates {
        if f.range().start() < last_end {
            continue;
        }
        last_end = f.range().end();
        non_overlapping.push(f);
    }

    let mut surviving: Vec<Finding> = Vec::new();
    let mut ui_findings: Vec<SecretFinding> = Vec::new();

    for f in non_overlapping {
        let range = f.range();
        let matched = &input[range.start()..range.end()];
        let hex = Sha256::digest_hex(matched.as_bytes());
        let fingerprint = format!("sha256:{hex}");
        if is_allowlisted(allowlist, &hex, &fingerprint, f.type_name(), f.detector()) {
            continue;
        }
        let redacted_finding = match Finding::new(
            f.id(),
            f.type_name(),
            f.detector(),
            f.confidence(),
            Action::Redact,
            range,
        ) {
            Ok(item) => item,
            Err(_) => continue,
        };
        let line = 1 + input[..range.start()]
            .bytes()
            .filter(|&b| b == b'\n')
            .count() as u32;
        ui_findings.push(SecretFinding {
            id: format!("sec-{}", ui_findings.len() + 1),
            kind: f.type_name().to_owned(),
            location: location.to_owned(),
            preview: mask_preview(matched),
            fingerprint,
            line: Some(line),
        });
        surviving.push(redacted_finding);
    }

    if surviving.is_empty() {
        return ScanReport {
            findings: Vec::new(),
            redacted: input.to_owned(),
            redaction_count: 0,
        };
    }

    let redaction_count = u32::try_from(surviving.len()).unwrap_or(u32::MAX);
    let redacted = redact_with_limits(input, &surviving, &typed_placeholder_formatter, &limits)
        .unwrap_or_else(|_| manual_redact(input, &surviving));

    ScanReport {
        findings: ui_findings,
        redacted,
        redaction_count,
    }
}

fn is_allowlisted(
    allowlist: &[String],
    hex: &str,
    fingerprint: &str,
    type_name: &str,
    detector: &str,
) -> bool {
    allowlist.iter().any(|entry| {
        let trimmed = entry.trim();
        !trimmed.is_empty()
            && (trimmed == fingerprint
                || trimmed == hex
                || trimmed == type_name
                || trimmed == detector)
    })
}

fn mask_preview(raw: &str) -> String {
    let total = raw.chars().count();
    if total >= 12 {
        let prefix: String = raw.chars().take(4).collect();
        let suffix_rev: Vec<char> = raw.chars().rev().take(4).collect();
        let suffix: String = suffix_rev.into_iter().rev().collect();
        format!("{prefix}••••{suffix}")
    } else if total >= 8 {
        let prefix: String = raw.chars().take(2).collect();
        let suffix_rev: Vec<char> = raw.chars().rev().take(2).collect();
        let suffix: String = suffix_rev.into_iter().rev().collect();
        format!("{prefix}••••{suffix}")
    } else {
        "••••••••".to_owned()
    }
}

fn manual_redact(input: &str, findings: &[Finding]) -> String {
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;
    for (idx, f) in findings.iter().enumerate() {
        let start = f.range().start();
        let end = f.range().end();
        if start >= cursor && end <= input.len() {
            out.push_str(&input[cursor..start]);
            let norm = f.type_name().to_ascii_uppercase().replace(['.', '-'], "_");
            out.push_str(&format!("<{norm}_{}>", idx + 1));
            cursor = end;
        }
    }
    out.push_str(&input[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_built_in_patterns_and_active_secrets_without_leaking() {
        let gh_token = ["ghp_", "SYNTHETICREVOKED00000000000000000000"].concat();
        let session_secret = "custom_gateway_bearer_xyz987654".to_owned();
        let sample = format!("line1\nGITHUB_TOKEN={gh_token}\nCUSTOM={session_secret}\nline4\n");
        let report = scan_text(
            &sample,
            "user_prompt",
            std::slice::from_ref(&session_secret),
            &[],
        );
        assert_eq!(report.redaction_count, 2);
        assert_eq!(report.findings.len(), 2);
        assert!(!report.redacted.contains(&gh_token));
        assert!(!report.redacted.contains(&session_secret));
        assert!(report.findings[0].preview.contains("••••"));
        assert!(!report.findings[0].preview.contains("SYNTHETIC"));
        assert!(report.findings[0].fingerprint.starts_with("sha256:"));
        assert_eq!(report.findings[0].line, Some(2));
        assert_eq!(report.findings[1].kind, "active_credential");
        assert_eq!(report.findings[1].line, Some(3));
    }

    #[test]
    fn allowlist_skips_matching_fingerprint_or_detector() {
        let secret = ["ghp_", "SYNTHETICREVOKED00000000000000000000"].concat();
        let first = scan_text(&secret, "tool:shell", &[], &[]);
        assert_eq!(first.redaction_count, 1);
        let fp = first.findings[0].fingerprint.clone();

        let skipped = scan_text(&secret, "tool:shell", &[], &[fp]);
        assert_eq!(skipped.redaction_count, 0);
        assert_eq!(skipped.redacted, secret);
    }

    #[test]
    fn respects_loopback_and_off_policy() {
        let mut net = NetworkSettings::default();
        assert_eq!(net.secret_guard, SecretGuardMode::Ask);
        assert!(!net.secret_guard_local);
        assert!(should_skip_guard(&net, "http://127.0.0.1:11434/v1"));
        assert!(!should_skip_guard(&net, "https://api.openai.com/v1"));

        net.secret_guard_local = true;
        assert!(!should_skip_guard(&net, "http://127.0.0.1:11434/v1"));

        net.secret_guard = SecretGuardMode::Off;
        assert!(should_skip_guard(&net, "https://api.openai.com/v1"));
    }
}
