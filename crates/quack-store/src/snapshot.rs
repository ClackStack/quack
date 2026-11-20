//! Git shadow snapshots for shell-command undo/rewind and watched-file change detection
//! (docs/plan/07-sessions-memory-checkpoints.md#checkpoints-file-undo).
//!
//! Uses a private bare git directory (`<session>/shadow.git`) with `--work-tree=<workspace>`
//! so the user's own `.git` directory, index, branches and stashes are never modified.
//! Respects `.gitignore` automatically and preserves exact bytes (`core.autocrlf=false`).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Maximum number of files recorded per shell command (guards against giant archive unpacks).
const MAX_CHANGES: usize = 256;
/// Maximum pre-image blob size saved for undo (10 MiB).
const MAX_BLOB_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChangeKind {
    Created,
    Changed,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: PathBuf,
    pub kind: FileChangeKind,
    /// Pre-command bytes when `kind` is `Changed` or `Deleted` (if within size cap).
    pub before: Option<Vec<u8>>,
}

fn git_cmd(shadow_git: &Path, work_tree: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-c")
        .arg("core.autocrlf=false")
        .arg("-c")
        .arg("core.safecrlf=false")
        .arg("-c")
        .arg("core.quotepath=false")
        .arg("-c")
        .arg("core.symlinks=false")
        .arg("--git-dir")
        .arg(shadow_git)
        .arg("--work-tree")
        .arg(work_tree)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    cmd
}

fn ensure_shadow_repo(shadow_git: &Path) -> bool {
    if shadow_git.join("HEAD").exists() {
        return true;
    }
    if let Some(parent) = shadow_git.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return false;
    }
    let ok = Command::new("git")
        .args(["init", "--bare", "-q"])
        .arg(shadow_git)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        return false;
    }
    let info = shadow_git.join("info");
    let _ = std::fs::create_dir_all(&info);
    let _ = std::fs::write(
        info.join("exclude"),
        ".git/\n.quack/worktrees/\n.quack/sessions/\n.quack/checkpoints/\n",
    );
    true
}

/// Snapshot the current workspace into `shadow_git` and return the resulting tree OID.
/// Returns `None` if `git` is unavailable or the directory cannot be snapshotted.
pub fn capture_tree(shadow_git: &Path, work_tree: &Path) -> Option<String> {
    if !work_tree.is_dir() || !ensure_shadow_repo(shadow_git) {
        return None;
    }
    let added = git_cmd(shadow_git, work_tree)
        .args(["add", "-A", "--", "."])
        .status()
        .ok()?;
    if !added.success() {
        return None;
    }
    let out = git_cmd(shadow_git, work_tree)
        .arg("write-tree")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let oid = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (!oid.is_empty()).then_some(oid)
}

/// Snapshot the workspace after a command finishes and return every file created, modified,
/// or deleted since `before_tree`, along with its exact pre-command bytes where applicable.
pub fn diff_after(shadow_git: &Path, work_tree: &Path, before_tree: &str) -> Vec<FileChange> {
    let Some(after_tree) = capture_tree(shadow_git, work_tree) else {
        return Vec::new();
    };
    if after_tree == before_tree {
        return Vec::new();
    }
    let Ok(out) = git_cmd(shadow_git, work_tree)
        .args([
            "diff-tree",
            "-r",
            "--no-renames",
            "--name-status",
            "-z",
            before_tree,
            &after_tree,
        ])
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }

    let mut changes = Vec::new();
    let mut parts = out.stdout.split(|&b| b == 0).filter(|p| !p.is_empty());
    while let Some(status_bytes) = parts.next() {
        let Some(path_bytes) = parts.next() else {
            break;
        };
        if changes.len() >= MAX_CHANGES {
            break;
        }
        let Ok(rel) = std::str::from_utf8(path_bytes) else {
            continue;
        };
        let kind = match status_bytes.first().copied() {
            Some(b'A') => FileChangeKind::Created,
            Some(b'M') | Some(b'T') => FileChangeKind::Changed,
            Some(b'D') => FileChangeKind::Deleted,
            _ => continue,
        };
        let before = match kind {
            FileChangeKind::Created => None,
            FileChangeKind::Changed | FileChangeKind::Deleted => {
                read_blob(shadow_git, work_tree, before_tree, rel)
            }
        };
        changes.push(FileChange {
            path: work_tree.join(rel),
            kind,
            before,
        });
    }
    changes
}

fn read_blob(shadow_git: &Path, work_tree: &Path, tree: &str, rel: &str) -> Option<Vec<u8>> {
    let spec = format!("{tree}:{rel}");
    let size_out = git_cmd(shadow_git, work_tree)
        .args(["cat-file", "-s", &spec])
        .output()
        .ok()?;
    if !size_out.status.success() {
        return None;
    }
    let size: usize = String::from_utf8(size_out.stdout)
        .ok()?
        .trim()
        .parse()
        .ok()?;
    if size > MAX_BLOB_BYTES {
        return None;
    }
    let out = git_cmd(shadow_git, work_tree)
        .args(["cat-file", "blob", &spec])
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_created_modified_and_deleted_files_with_exact_crlf_bytes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ws = tmp.path().join("ws");
        let shadow = tmp.path().join("shadow.git");
        std::fs::create_dir_all(&ws).expect("ws");
        std::fs::write(ws.join(".gitignore"), "ignored.log\n").expect("gitignore");
        std::fs::write(ws.join("keep.txt"), b"one\r\ntwo\r\n").expect("keep");
        std::fs::write(ws.join("gone.txt"), b"bye\n").expect("gone");

        let Some(before) = capture_tree(&shadow, &ws) else {
            return; // git not installed
        };

        std::fs::write(ws.join("keep.txt"), b"changed\r\n").expect("modify");
        std::fs::remove_file(ws.join("gone.txt")).expect("delete");
        std::fs::write(ws.join("added.txt"), b"fresh\n").expect("add");
        std::fs::write(ws.join("ignored.log"), b"skip me\n").expect("ignored");

        let changes = diff_after(&shadow, &ws, &before);
        assert_eq!(changes.len(), 3, "{changes:?}");

        let keep = changes
            .iter()
            .find(|c| c.path.ends_with("keep.txt"))
            .expect("keep");
        assert_eq!(keep.kind, FileChangeKind::Changed);
        assert_eq!(keep.before.as_deref(), Some(b"one\r\ntwo\r\n".as_slice()));

        let gone = changes
            .iter()
            .find(|c| c.path.ends_with("gone.txt"))
            .expect("gone");
        assert_eq!(gone.kind, FileChangeKind::Deleted);
        assert_eq!(gone.before.as_deref(), Some(b"bye\n".as_slice()));

        let added = changes
            .iter()
            .find(|c| c.path.ends_with("added.txt"))
            .expect("added");
        assert_eq!(added.kind, FileChangeKind::Created);
        assert_eq!(added.before, None);
    }
}
