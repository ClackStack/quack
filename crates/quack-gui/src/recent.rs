//! Recently opened folders, persisted in the user data dir so launching `quack-gui`
//! from an icon can reopen the last workspace and the Welcome view can list the rest
//! (docs/plan/15-desktop-client.md#the-ui). Best-effort: a missing or corrupt file is
//! just an empty list.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const FILE: &str = "recent-folders.json";
const MAX: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecentFolder {
    /// The absolute path of the workspace root.
    pub path: String,
    /// Unix seconds of the last successful open.
    pub opened: u64,
}

/// The list, loaded once per app run; `touch` rewrites the file on each open.
#[derive(Debug, Clone)]
pub struct RecentFolders {
    file: PathBuf,
    entries: Vec<RecentFolder>,
}

impl RecentFolders {
    /// Load from `<user data>/gui/recent-folders.json`, or start empty.
    pub fn load() -> Self {
        Self::at(quack_config::user_data_dir(&|k| std::env::var_os(k)).join("gui"))
    }

    /// Load from an explicit directory (the GUI data dir; tests pass a temp dir).
    pub fn at(dir: PathBuf) -> Self {
        Self::at_with(dir, &|k| std::env::var_os(k))
    }

    /// Load from `dir` using `env` to expand leading `~` segments and collapse
    /// any out-of-sync duplicate or malformed entries on disk.
    pub fn at_with(dir: PathBuf, env: &dyn Fn(&str) -> Option<OsString>) -> Self {
        let file = dir.join(FILE);
        let raw = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<RecentFolder>>(&bytes).ok())
            .unwrap_or_default();
        let entries = dedupe_entries(&raw, env);
        if entries != raw && file.exists() {
            write_entries(&file, &entries);
        }
        Self { file, entries }
    }

    /// Move `path` to the front and persist; failures are logged, never fatal.
    pub fn touch(&mut self, path: PathBuf) {
        self.touch_with(path, &|k| std::env::var_os(k));
    }

    /// Same as [`Self::touch`], with an explicit environment lookup for tests.
    pub fn touch_with(&mut self, path: PathBuf, env: &dyn Fn(&str) -> Option<OsString>) {
        let Some(normalized) = normalize_recent_path(&path, env) else {
            return;
        };
        let key = workspace_key(&normalized);
        let path_str = normalized.display().to_string();
        self.entries = dedupe_entries(&self.entries, env);
        self.entries
            .retain(|e| workspace_key(Path::new(&e.path)) != key);
        self.entries.insert(
            0,
            RecentFolder {
                path: path_str,
                opened: now(),
            },
        );
        self.entries.truncate(MAX);
        if let Some(parent) = self.file.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            return;
        }
        write_entries(&self.file, &self.entries);
    }

    /// The first `limit` unique entries, most recent first.
    pub fn list(&self, limit: usize) -> Vec<RecentFolder> {
        dedupe_entries(&self.entries, &|k| std::env::var_os(k))
            .into_iter()
            .take(limit)
            .collect()
    }
}

pub(crate) fn expand_tilde_with(path: PathBuf, env: &dyn Fn(&str) -> Option<OsString>) -> PathBuf {
    let Some(s) = path.to_str() else {
        return path;
    };
    if s == "~" || s.starts_with("~/") || s.starts_with("~\\") {
        let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        if let Some(home) = env(var).filter(|v| !v.is_empty()) {
            let rest = s[1..].trim_start_matches(['/', '\\']);
            return if rest.is_empty() {
                PathBuf::from(home)
            } else {
                PathBuf::from(home).join(rest)
            };
        }
    }
    path
}

/// Resolve symlinks before comparing workspace ownership; compare case-insensitively
/// on the platforms where workspace paths are normally case-insensitive.
pub(crate) fn workspace_key(path: &Path) -> PathBuf {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.components().collect());
    if cfg!(any(windows, target_os = "macos")) {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path
    }
}

fn normalize_recent_path(path: &Path, env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if path.as_os_str().is_empty() || path.to_string_lossy().trim().is_empty() {
        return None;
    }
    let expanded = expand_tilde_with(path.to_path_buf(), env);
    if expanded.as_os_str().is_empty() || expanded.to_string_lossy().trim().is_empty() {
        return None;
    }
    // Collect components instead of calling `canonicalize` so display paths never gain `\\?\`.
    let cleaned: PathBuf = expanded.components().collect();
    if cleaned.as_os_str().is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn dedupe_entries(
    entries: &[RecentFolder],
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Vec<RecentFolder> {
    let mut out: Vec<RecentFolder> = Vec::with_capacity(entries.len());
    let mut seen: HashMap<PathBuf, usize> = HashMap::new();
    for entry in entries {
        let Some(normalized) = normalize_recent_path(Path::new(&entry.path), env) else {
            continue;
        };
        let key = workspace_key(&normalized);
        if let Some(&idx) = seen.get(&key) {
            out[idx].opened = out[idx].opened.max(entry.opened);
            continue;
        }
        seen.insert(key, out.len());
        out.push(RecentFolder {
            path: normalized.display().to_string(),
            opened: entry.opened,
        });
    }
    out
}

fn write_entries(file: &Path, entries: &[RecentFolder]) {
    match serde_json::to_vec_pretty(entries) {
        Ok(bytes) => {
            if let Err(e) = std::fs::write(file, bytes) {
                tracing::warn!(%e, "could not save the recent folders");
            }
        }
        Err(e) => tracing::warn!(%e, "could not serialize the recent folders"),
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_moves_to_front_and_deduplicates() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut recent = RecentFolders::at(dir.path().to_path_buf());
        recent.touch(PathBuf::from("/a"));
        recent.touch(PathBuf::from("/b"));
        recent.touch(PathBuf::from("/a"));
        let names: Vec<String> = recent.list(10).into_iter().map(|e| e.path).collect();
        assert_eq!(names, vec!["/a", "/b"]);
    }

    #[test]
    fn list_respects_the_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut recent = RecentFolders::at(dir.path().to_path_buf());
        for i in 0..5 {
            recent.touch(PathBuf::from(format!("/{i}")));
        }
        assert_eq!(recent.list(2).len(), 2);
    }

    #[test]
    fn deduplicates_tilde_trailing_slashes_and_empty_entries_on_load_and_touch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join(FILE);
        let fake_home = if cfg!(windows) {
            r"C:\Users\alice"
        } else {
            "/home/alice"
        };
        let proj = Path::new(fake_home)
            .join("dev")
            .join("proj")
            .display()
            .to_string();
        let proj_slash = format!("{proj}{}", std::path::MAIN_SEPARATOR);
        let other = Path::new(fake_home)
            .join("dev")
            .join("other")
            .display()
            .to_string();
        std::fs::write(
            &file,
            serde_json::to_vec_pretty(&serde_json::json!([
                { "path": "", "opened": 99 },
                { "path": proj_slash, "opened": 10 },
                { "path": "~/dev/proj", "opened": 25 },
                { "path": other, "opened": 15 },
                { "path": proj, "opened": 12 }
            ]))
            .expect("json"),
        )
        .expect("write");
        let home_var = OsString::from(fake_home);
        let env = |key: &str| {
            let expected = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
            (key == expected).then(|| home_var.clone())
        };

        let mut recent = RecentFolders::at_with(dir.path().to_path_buf(), &env);
        let initial = recent.list(10);
        assert_eq!(initial.len(), 2);
        assert_eq!(initial[0].path, proj);
        assert_eq!(initial[0].opened, 25);
        assert_eq!(initial[1].path, other);

        // Reloading from disk confirms the cleaned file was persisted.
        let reloaded = RecentFolders::at_with(dir.path().to_path_buf(), &env);
        assert_eq!(reloaded.list(10).len(), 2);

        recent.touch_with(PathBuf::from("~/dev/other/"), &env);
        let updated: Vec<String> = recent.list(10).into_iter().map(|e| e.path).collect();
        assert_eq!(updated, vec![other, proj]);
    }

    #[cfg(unix)]
    #[test]
    fn deduplicates_symlink_and_target_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let real_ws = dir.path().join("real-ws");
        let link_ws = dir.path().join("link-ws");
        std::fs::create_dir_all(&real_ws).expect("real ws");
        std::os::unix::fs::symlink(&real_ws, &link_ws).expect("symlink");

        let mut recent = RecentFolders::at(dir.path().join("gui"));
        recent.touch(link_ws);
        recent.touch(real_ws.clone());
        let listed = recent.list(10);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, real_ws.display().to_string());
    }
}
