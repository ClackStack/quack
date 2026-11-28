//! Append-only JSONL session logs, a per-project index, and loading for resume.
//!
//! Layout: `<sessions>/<project-key>/index.jsonl` plus one directory per session holding
//! `events.jsonl`. Every line is `{"v":1,"t":"<type>","ts":"…", …payload}`. The store
//! does not interpret payloads; the engine owns their meaning.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::time;

pub const FORMAT_VERSION: u64 = 1;

static SESSION_SEQ: AtomicU64 = AtomicU64::new(0);

/// Generate a 12-character lowercase hex session id (48 bits) from OS-seeded randomness.
pub fn new_session_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let seq = SESSION_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let mut h = RandomState::new().build_hasher();
    h.write_u128(nanos);
    h.write_u64(seq);
    h.write_u32(std::process::id());
    let mut z = h.finish() ^ 0x9e37_79b9_7f4a_7c15;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    let mut id = z & 0xffff_ffff_ffff;
    if id >> 44 == 0 {
        id |= 1 << 44;
    }
    format!("{id:012x}")
}

/// One line of `index.jsonl`. Later lines for the same id update earlier ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub id: String,
    pub dir: String,
    pub started: u64,
    pub updated: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub turns: u32,
}

/// Sessions of one project.
#[derive(Debug, Clone)]
pub struct ProjectSessions {
    pub dir: PathBuf,
}

impl ProjectSessions {
    pub fn new(sessions_root: &Path, workspace_root: &Path) -> Self {
        Self {
            dir: sessions_root.join(project_key(workspace_root)),
        }
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("index.jsonl")
    }

    /// Sessions, most recently updated first.
    pub fn list(&self) -> Vec<IndexEntry> {
        let Ok(f) = File::open(self.index_path()) else {
            return Vec::new();
        };
        // Keep the last write per id, remembering its line, to break ties within a second
        // (later line = later write, first among equal `updated`).
        let mut by_id: HashMap<String, (usize, IndexEntry)> = HashMap::new();
        for (line_no, line) in BufReader::new(f).lines().map_while(Result::ok).enumerate() {
            let Ok(e) = serde_json::from_str::<IndexEntry>(&line) else {
                continue;
            };
            by_id.insert(e.id.clone(), (line_no, e));
        }
        let mut entries: Vec<(usize, IndexEntry)> = by_id.into_values().collect();
        entries.sort_by(|a, b| b.1.updated.cmp(&a.1.updated).then(b.0.cmp(&a.0)));
        entries.into_iter().map(|(_, e)| e).collect()
    }

    /// Find by exact id or unique prefix.
    pub fn find(&self, id: &str) -> Result<IndexEntry, String> {
        let id = id.trim();
        if id.is_empty() {
            return Err("session id cannot be empty".into());
        }
        let all = self.list();
        if let Some(e) = all.iter().find(|e| e.id == id) {
            return Ok(e.clone());
        }
        let hits: Vec<&IndexEntry> = all.iter().filter(|e| e.id.starts_with(id)).collect();
        match hits.as_slice() {
            [one] => Ok((*one).clone()),
            [] => Err(format!("no session `{id}` in this project")),
            _ => Err(format!(
                "session id `{id}` is ambiguous; give more characters"
            )),
        }
    }

    pub fn latest(&self) -> Option<IndexEntry> {
        self.list().into_iter().next()
    }

    fn append_index(&self, e: &IndexEntry) -> std::io::Result<()> {
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.index_path())?;
        let line = serde_json::to_string(e)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(f, "{line}")
    }

    /// Start a new session log.
    pub fn create(&self, id: &str, model: &str) -> std::io::Result<SessionLog> {
        let now = time::unix_now();
        let stamp = time::rfc3339(now).replace(':', "-");
        let dir_name = format!("{}_{id}", stamp.trim_end_matches('Z'));
        let dir = self.dir.join(&dir_name);
        std::fs::create_dir_all(&dir)?;
        let entry = IndexEntry {
            id: id.to_owned(),
            dir: dir_name,
            started: now,
            updated: now,
            title: String::new(),
            model: model.to_owned(),
            turns: 0,
        };
        self.append_index(&entry)?;
        SessionLog::open(self.clone(), entry, &dir)
    }

    /// Reopen an existing session to continue appending to it.
    pub fn reopen(&self, entry: IndexEntry) -> std::io::Result<SessionLog> {
        let dir = self.dir.join(&entry.dir);
        SessionLog::open(self.clone(), entry, &dir)
    }

    /// Read a session's records. A truncated last line (from a crash) is skipped.
    pub fn read(&self, entry: &IndexEntry) -> std::io::Result<Vec<Record>> {
        read_events(&self.dir.join(&entry.dir).join("events.jsonl"))
    }

    pub fn history_path(&self) -> PathBuf {
        self.dir.join("history.jsonl")
    }

    pub fn load_history(&self) -> Vec<String> {
        load_history_file(&self.history_path())
    }

    pub fn append_history(&self, text: &str) -> std::io::Result<()> {
        append_history_file(&self.history_path(), text)
    }

    /// Remove saved sessions whose last update is strictly older than `cutoff_unix`
    /// (never removing `keep_id`), deleting their session folders and compacting
    /// `index.jsonl`. When `dry_run` is true, nothing is deleted or rewritten.
    pub fn prune_older_than(
        &self,
        cutoff_unix: u64,
        dry_run: bool,
        keep_id: Option<&str>,
    ) -> std::io::Result<PruneReport> {
        let all = self.list();
        if all.is_empty() {
            return Ok(PruneReport::default());
        }
        let mut removed = Vec::new();
        let mut kept = Vec::new();
        for entry in all {
            let protected = keep_id.is_some_and(|k| k == entry.id);
            if !protected && entry.updated < cutoff_unix {
                removed.push(entry);
            } else {
                kept.push(entry);
            }
        }
        if removed.is_empty() {
            return Ok(PruneReport {
                removed: Vec::new(),
                kept: kept.len(),
                bytes_freed: 0,
            });
        }
        let mut bytes_freed = 0u64;
        for entry in &removed {
            let path = self.dir.join(&entry.dir);
            bytes_freed = bytes_freed.saturating_add(dir_size(&path));
            if !dry_run && path.exists() {
                let _ = std::fs::remove_dir_all(&path);
            }
        }
        if !dry_run {
            // Write oldest-first so the append-only order matches normal operation.
            kept.reverse();
            let mut buf = String::new();
            for entry in &kept {
                let line = serde_json::to_string(entry)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                buf.push_str(&line);
                buf.push('\n');
            }
            let index = self.index_path();
            let seq = HISTORY_SEQ.fetch_add(1, Ordering::Relaxed);
            let tmp = index.with_extension(format!("jsonl.{}.{seq}.tmp", std::process::id()));
            std::fs::write(&tmp, buf)
                .and_then(|()| rename_replacing(&tmp, &index))
                .inspect_err(|_| {
                    let _ = std::fs::remove_file(&tmp);
                })?;
        }
        Ok(PruneReport {
            removed,
            kept: kept.len(),
            bytes_freed,
        })
    }
}

/// Summary returned by [`ProjectSessions::prune_older_than`] and [`prune_all_projects`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneReport {
    pub removed: Vec<IndexEntry>,
    pub kept: usize,
    pub bytes_freed: u64,
}

/// Prune expired sessions across every project directory under `sessions_root`.
pub fn prune_all_projects(
    sessions_root: &Path,
    cutoff_unix: u64,
    dry_run: bool,
    keep_id: Option<&str>,
) -> std::io::Result<PruneReport> {
    let Ok(read_dir) = std::fs::read_dir(sessions_root) else {
        return Ok(PruneReport::default());
    };
    let mut total = PruneReport::default();
    for entry in read_dir.flatten() {
        let dir = entry.path();
        if !dir.is_dir() || !dir.join("index.jsonl").is_file() {
            continue;
        }
        let ps = ProjectSessions { dir };
        let part = ps.prune_older_than(cutoff_unix, dry_run, keep_id)?;
        total.removed.extend(part.removed);
        total.kept += part.kept;
        total.bytes_freed = total.bytes_freed.saturating_add(part.bytes_freed);
    }
    Ok(total)
}

/// Parse a retention duration like `30d`, `2w`, `12h`, `45m`, or `60s` into seconds.
pub fn parse_duration_secs(spec: &str) -> Result<u64, String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err("duration cannot be empty; expected e.g. `30d`, `2w`, `12h`".into());
    }
    let (num, unit) = trimmed.split_at(trimmed.len().saturating_sub(1));
    let multiplier: u64 = match unit {
        "s" | "S" => 1,
        "m" | "M" => 60,
        "h" | "H" => 3_600,
        "d" | "D" => 86_400,
        "w" | "W" => 7 * 86_400,
        _ => {
            return Err(format!(
                "invalid duration `{trimmed}`: end with `s`, `m`, `h`, `d`, or `w` (e.g. `30d`)"
            ));
        }
    };
    let count: u64 = num.parse().map_err(|_| {
        format!("invalid duration `{trimmed}`: expected a positive number before `{unit}`")
    })?;
    if count == 0 {
        return Err("duration must be greater than zero".into());
    }
    count
        .checked_mul(multiplier)
        .ok_or_else(|| format!("duration `{trimmed}` is too large"))
}

fn dir_size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.is_file() {
        return meta.len();
    }
    if !meta.is_dir() {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| dir_size(&e.path()))
        .fold(0u64, u64::saturating_add)
}

const MAX_HISTORY: usize = 100;
static HISTORY_SEQ: AtomicU64 = AtomicU64::new(0);
static HISTORY_LOCK: Mutex<()> = Mutex::new(());

/// Load prompt history from a `history.jsonl` file, oldest first.
pub fn load_history_file(path: &Path) -> Vec<String> {
    let Ok(f) = File::open(path) else {
        return Vec::new();
    };
    let mut all: Vec<String> = BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<String>(&line).ok())
        .filter(|s| !s.trim().is_empty())
        .collect();
    if all.len() > MAX_HISTORY {
        all.drain(..all.len() - MAX_HISTORY);
    }
    all
}

/// Append a prompt to a `history.jsonl` file, keeping at most [`MAX_HISTORY`] entries
/// and skipping consecutive duplicates. Written via a sibling temp file and rename so
/// concurrent readers (e.g. TUI and GUI) never observe a half-written file.
pub fn append_history_file(path: &Path, text: &str) -> std::io::Result<()> {
    if text.trim().is_empty() {
        return Ok(());
    }
    // Serialize in-process updates so Windows NTFS does not collide on concurrent renames
    // and concurrent threads do not drop entries.
    let _guard = HISTORY_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let mut all = load_history_file(path);
    if all.last().map(String::as_str) == Some(text) {
        return Ok(());
    }
    all.push(text.to_owned());
    if all.len() > MAX_HISTORY {
        all.drain(..all.len() - MAX_HISTORY);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut buf = String::new();
    for entry in &all {
        let line = serde_json::to_string(entry)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        buf.push_str(&line);
        buf.push('\n');
    }
    let seq = HISTORY_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("jsonl.{}.{seq}.tmp", std::process::id()));
    std::fs::write(&tmp, buf)
        .and_then(|()| rename_replacing(&tmp, path))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
}

fn rename_replacing(from: &Path, to: &Path) -> std::io::Result<()> {
    for attempt in 0..5u64 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            // On Windows, a concurrent external reader can briefly cause `PermissionDenied`.
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                std::thread::sleep(Duration::from_millis(2 * (attempt + 1)));
            }
            Err(err) => return Err(err),
        }
    }
    std::fs::rename(from, to)
}

/// A parsed log line.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub kind: String,
    pub ts: String,
    /// The payload fields (everything except `v`, `t`, `ts`).
    pub data: Map<String, Value>,
}

pub fn read_events(path: &Path) -> std::io::Result<Vec<Record>> {
    let f = File::open(path)?;
    let mut out = Vec::new();
    for (n, line) in BufReader::new(f).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Map<String, Value>>(&line) {
            Ok(mut m) => {
                let kind = m
                    .remove("t")
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default();
                let ts = m
                    .remove("ts")
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default();
                m.remove("v");
                out.push(Record { kind, ts, data: m });
            }
            Err(e) => {
                tracing::warn!(line = n + 1, error = %e, path = %path.display(), "skipping unreadable session line")
            }
        }
    }
    Ok(out)
}

/// Serialize one record and buffer it as a JSONL line. Serialization failures are
/// surfaced as [`std::io::ErrorKind::InvalidData`] instead of writing a blank line.
fn write_line<W: Write>(w: &mut W, record: &Value) -> std::io::Result<()> {
    let mut line = serde_json::to_string(record)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    line.push('\n');
    w.write_all(line.as_bytes())
}

/// An open session log. Appends are buffered; `sync` flushes and fsyncs (end of turn).
#[derive(Debug)]
pub struct SessionLog {
    project: ProjectSessions,
    pub entry: IndexEntry,
    file: BufWriter<File>,
}

impl SessionLog {
    fn open(project: ProjectSessions, entry: IndexEntry, dir: &Path) -> std::io::Result<Self> {
        let path = dir.join("events.jsonl");
        // A crash can leave a partial last line; end it so the next record starts clean.
        let torn = std::fs::read(&path).is_ok_and(|b| b.last().is_some_and(|&c| c != b'\n'));
        let mut file = BufWriter::new(OpenOptions::new().create(true).append(true).open(&path)?);
        if torn {
            file.write_all(b"\n")?;
            file.flush()?;
        }
        Ok(Self {
            project,
            entry,
            file,
        })
    }

    pub fn dir(&self) -> PathBuf {
        self.project.dir.join(&self.entry.dir)
    }

    /// A log of its own for a sub-agent run: `agents/<name>.jsonl` in this session's folder.
    /// Resuming the session never reads it.
    pub fn child(&self, name: &str) -> std::io::Result<SessionLog> {
        let dir = self.dir().join("agents");
        std::fs::create_dir_all(&dir)?;
        let file = BufWriter::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join(format!("{name}.jsonl")))?,
        );
        Ok(Self {
            project: self.project.clone(),
            entry: self.entry.clone(),
            file,
        })
    }

    /// Append one record. `payload` must serialize to a JSON object.
    pub fn append(&mut self, kind: &str, payload: impl Serialize) -> std::io::Result<()> {
        let mut m = Map::new();
        m.insert("v".into(), Value::from(FORMAT_VERSION));
        m.insert("t".into(), Value::from(kind));
        m.insert("ts".into(), Value::from(time::rfc3339(time::unix_now())));
        match serde_json::to_value(payload) {
            Ok(Value::Object(fields)) => m.extend(fields),
            Ok(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "session payload did not serialize to a JSON object",
                ));
            }
            Err(e) => {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e));
            }
        }
        write_line(&mut self.file, &Value::Object(m))
    }

    /// Append a record read from another log, keeping its time (`/fork`).
    pub fn copy(&mut self, r: &Record) -> std::io::Result<()> {
        let mut m = Map::new();
        m.insert("v".into(), Value::from(FORMAT_VERSION));
        m.insert("t".into(), Value::from(r.kind.as_str()));
        m.insert("ts".into(), Value::from(r.ts.as_str()));
        m.extend(r.data.clone());
        write_line(&mut self.file, &Value::Object(m))
    }

    /// Durably store everything so far and update the index (end of each turn).
    pub fn sync(&mut self, title: Option<&str>, turns: u32) -> std::io::Result<()> {
        self.file.flush()?;
        self.file.get_ref().sync_data()?;
        self.entry.updated = time::unix_now();
        self.entry.turns = turns;
        if let Some(t) = title {
            self.entry.title = t.to_owned();
        }
        self.project.append_index(&self.entry)
    }
}

/// `<last two path components>-<hash>`, safe on every filesystem.
pub fn project_key(root: &Path) -> String {
    let full = root.to_string_lossy();
    // The persisted key truncates to the lower 32 bits so existing on-disk folders keep
    // lining up; do not change the cast without migrating `~/.cache/quack/sessions/`.
    let hash = crate::fnv1a(full.as_bytes()) as u32;
    let tail: Vec<String> = root
        .components()
        .rev()
        .take(2)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let name: String = tail
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{name}-{hash:08x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_append_list_reopen() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ps = ProjectSessions::new(tmp.path(), Path::new("/home/u/src/proj"));
        let mut log = ps.create("abc123", "p/m").expect("create");
        log.append(
            "message",
            serde_json::json!({"message": {"role": "user", "content": "hi"}}),
        )
        .expect("append");
        log.sync(Some("hi"), 1).expect("sync");

        let list = ps.list();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].title.as_str(), list[0].turns), ("hi", 1));
        let entry = ps.find("abc").expect("prefix");
        let recs = ps.read(&entry).expect("read");
        assert_eq!(recs[0].kind, "message");
        assert_eq!(recs[0].data["message"]["content"], "hi");

        // A crash leaves a partial line; it is skipped.
        let events = log.dir().join("events.jsonl");
        OpenOptions::new()
            .append(true)
            .open(&events)
            .and_then(|mut f| f.write_all(b"{\"v\":1,\"t\":\"mess"))
            .expect("corrupt");
        let mut again = ps.reopen(entry.clone()).expect("reopen");
        assert_eq!(ps.read(&entry).expect("read").len(), 1);
        again
            .append("usage", serde_json::json!({"prompt_tokens": 3}))
            .expect("append");
        // Appends are buffered; they reach the file at sync (end of turn).
        again.sync(None, 2).expect("sync");
        let recs = ps.read(&entry).expect("read");
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[1].kind, "usage");
        assert!(ps.project_key_is_stable());
    }

    impl ProjectSessions {
        fn project_key_is_stable(&self) -> bool {
            self.dir
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("src-proj-"))
        }
    }

    #[test]
    fn prompt_history_persists_multiline_dedupes_and_caps() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ps = ProjectSessions::new(tmp.path(), Path::new("/home/u/src/proj"));
        // Reading before anything is written creates no directory.
        assert!(ps.load_history().is_empty());
        assert!(!ps.dir.exists());

        ps.append_history("first\nsecond line").expect("append");
        ps.append_history("first\nsecond line").expect("dedupe");
        ps.append_history("   ").expect("skip blank");
        ps.append_history("third").expect("append");
        assert_eq!(ps.load_history(), vec!["first\nsecond line", "third"]);
    }

    #[test]
    fn append_history_file_writes_atomically_without_leaving_temp_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("proj").join("history.jsonl");
        let handles: Vec<_> = (0..4)
            .map(|t| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for i in 0..10 {
                        append_history_file(&path, &format!("worker-{t}-prompt-{i}"))
                            .expect("append");
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread join");
        }
        let entries: Vec<_> = std::fs::read_dir(path.parent().expect("parent"))
            .expect("readdir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["history.jsonl"]);
        assert!(!load_history_file(&path).is_empty());
    }

    #[test]
    fn new_session_ids_are_compact_random_and_findable_by_prefix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ps = ProjectSessions::new(tmp.path(), Path::new("/home/u/src/proj"));

        let id1 = new_session_id();
        let id2 = new_session_id();
        assert_eq!(id1.len(), 12);
        assert_eq!(id2.len(), 12);
        assert_ne!(id1, id2);
        assert!(!id1.starts_with('0'), "{id1}");
        assert!(!id2.starts_with('0'), "{id2}");

        ps.create(&id1, "p/m").expect("create 1");
        ps.create(&id2, "p/m").expect("create 2");
        assert_eq!(ps.find(&id1).expect("find 1").id, id1);
        assert_eq!(ps.find(&id2).expect("find 2").id, id2);
        assert!(ps.find("").is_err());
    }

    #[test]
    fn prune_older_than_respects_cutoff_dry_run_and_keep_id() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ps = ProjectSessions::new(tmp.path(), Path::new("/home/u/src/proj"));
        let mut old_log = ps.create("old000000001", "p/m").expect("old");
        old_log
            .append("message", serde_json::json!({"text": "ancient"}))
            .expect("append");
        old_log.sync(Some("old"), 1).expect("sync");
        let old_dir = old_log.dir();

        let mut keep_log = ps.create("keep00000002", "p/m").expect("keep");
        keep_log.sync(Some("keep"), 1).expect("sync");
        let keep_dir = keep_log.dir();

        // Rewrite index with deterministic old vs new timestamps.
        let mut entries = ps.list();
        for e in &mut entries {
            e.updated = if e.id == "old000000001" { 100 } else { 500 };
        }
        let body: String = entries
            .iter()
            .map(|e| format!("{}\n", serde_json::to_string(e).expect("json")))
            .collect();
        std::fs::write(ps.index_path(), body).expect("write index");

        // Dry run reports removal without deleting anything.
        let dry = ps.prune_older_than(200, true, None).expect("dry");
        assert_eq!(dry.removed.len(), 1);
        assert_eq!(dry.kept, 1);
        assert!(dry.bytes_freed > 0);
        assert!(old_dir.exists());

        // Protected id is kept even when older than cutoff.
        let protected = ps
            .prune_older_than(200, false, Some("old000000001"))
            .expect("protected");
        assert!(protected.removed.is_empty());
        assert!(old_dir.exists());

        // Real prune removes old session directory and compacts index.
        let pruned = ps.prune_older_than(200, false, None).expect("prune");
        assert_eq!(pruned.removed.len(), 1);
        assert_eq!(pruned.removed[0].id, "old000000001");
        assert_eq!(pruned.kept, 1);
        assert!(!old_dir.exists());
        assert!(keep_dir.exists());
        assert_eq!(ps.list().len(), 1);
        assert_eq!(ps.list()[0].id, "keep00000002");
    }

    #[test]
    fn parse_duration_handles_units_and_rejects_bad_values() {
        assert_eq!(parse_duration_secs("30d"), Ok(30 * 86_400));
        assert_eq!(parse_duration_secs("2w"), Ok(14 * 86_400));
        assert_eq!(parse_duration_secs("12h"), Ok(12 * 3_600));
        assert_eq!(parse_duration_secs("45m"), Ok(45 * 60));
        assert_eq!(parse_duration_secs("10s"), Ok(10));
        assert!(parse_duration_secs("0d").is_err());
        assert!(parse_duration_secs("30").is_err());
        assert!(parse_duration_secs("").is_err());
    }
}
