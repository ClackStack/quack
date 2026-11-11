//! Recently opened folders, persisted in the user data dir so launching `quack-gui`
//! from an icon can reopen the last workspace and the Welcome view can list the rest
//! (docs/plan/15-desktop-client.md#the-ui). Best-effort: a missing or corrupt file is
//! just an empty list.

use std::path::PathBuf;

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
        let file = dir.join(FILE);
        let entries = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<RecentFolder>>(&bytes).ok())
            .unwrap_or_default();
        Self { file, entries }
    }

    /// Move `path` to the front and persist; failures are logged, never fatal.
    pub fn touch(&mut self, path: PathBuf) {
        let path = path.display().to_string();
        self.entries.retain(|e| e.path != path);
        self.entries.insert(
            0,
            RecentFolder {
                path,
                opened: now(),
            },
        );
        self.entries.truncate(MAX);
        if let Some(parent) = self.file.parent()
            && std::fs::create_dir_all(parent).is_err()
        {
            return;
        }
        match serde_json::to_vec_pretty(&self.entries) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(&self.file, bytes) {
                    tracing::warn!(%e, "could not save the recent folders");
                }
            }
            Err(e) => tracing::warn!(%e, "could not serialize the recent folders"),
        }
    }

    /// The first `limit` entries, most recent first.
    pub fn list(&self, limit: usize) -> Vec<RecentFolder> {
        self.entries.iter().take(limit).cloned().collect()
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
}
