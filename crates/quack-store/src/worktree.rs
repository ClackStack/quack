//! Parallel sessions in git worktrees (`quack --worktree <name>`, `/fork --worktree`).
//!
//! Creates `<repo>/.quack/worktrees/<name>` on a dedicated `quack/<name>` branch and adds
//! `.quack/worktrees/` to the repository's local `.git/info/exclude` so the main working
//! tree stays clean.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const MAX_NAME_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
    pub created: bool,
}

/// Validate and normalize a worktree name (letters, digits, `.`, `_`, `-`; no path traversal).
pub fn sanitize_name(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("worktree name cannot be empty".into());
    }
    if trimmed.len() > MAX_NAME_LEN {
        return Err(format!(
            "worktree name must be at most {MAX_NAME_LEN} characters"
        ));
    }
    if trimmed.starts_with('-') || trimmed.starts_with('.') || trimmed.contains("..") {
        return Err(format!("invalid worktree name `{trimmed}`"));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(format!(
            "worktree name `{trimmed}` may contain only ASCII letters, digits, `.`, `_`, and `-`"
        ));
    }
    Ok(trimmed.to_owned())
}

fn git_output(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        return Err(if err.is_empty() {
            format!("git {} failed", args.join(" "))
        } else {
            err
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Create (or open, when `reuse_existing` is true) a git worktree under
/// `<repo>/.quack/worktrees/<name>` on branch `quack/<name>`.
pub fn create_or_open(
    repo_cwd: &Path,
    raw_name: &str,
    reuse_existing: bool,
) -> Result<Worktree, String> {
    let name = sanitize_name(raw_name)?;
    let repo_root = PathBuf::from(
        git_output(repo_cwd, &["rev-parse", "--show-toplevel"])
            .map_err(|_| "not inside a git repository; worktrees require git".to_owned())?,
    );
    let common_raw = git_output(&repo_root, &["rev-parse", "--git-common-dir"])?;
    let common_dir = {
        let p = PathBuf::from(&common_raw);
        if p.is_absolute() {
            p
        } else {
            repo_root.join(p)
        }
    };
    let branch = format!("quack/{name}");
    let worktrees_dir = repo_root.join(".quack").join("worktrees");
    let target = worktrees_dir.join(&name);

    if target.exists() {
        if reuse_existing && target.is_dir() {
            let current_branch = git_output(&target, &["rev-parse", "--abbrev-ref", "HEAD"])
                .unwrap_or_else(|_| branch.clone());
            return Ok(Worktree {
                path: target,
                branch: current_branch,
                created: false,
            });
        }
        return Err(format!(
            "worktree `{name}` already exists at {}",
            target.display()
        ));
    }

    ensure_excluded(&common_dir);
    std::fs::create_dir_all(&worktrees_dir)
        .map_err(|e| format!("cannot create {}: {e}", worktrees_dir.display()))?;

    let target_str = target.to_string_lossy().into_owned();
    let branch_exists = Command::new("git")
        .arg("-C")
        .arg(&repo_root)
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());

    if branch_exists {
        if !reuse_existing {
            return Err(format!("git branch `{branch}` already exists"));
        }
        git_output(&repo_root, &["worktree", "add", &target_str, &branch])?;
    } else {
        git_output(
            &repo_root,
            &["worktree", "add", "-b", &branch, &target_str, "HEAD"],
        )?;
    }

    Ok(Worktree {
        path: target,
        branch,
        created: true,
    })
}

fn ensure_excluded(common_dir: &Path) {
    let info = common_dir.join("info");
    let _ = std::fs::create_dir_all(&info);
    let exclude = info.join("exclude");
    let existing = std::fs::read_to_string(&exclude).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == ".quack/worktrees/") {
        return;
    }
    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(".quack/worktrees/\n");
    let _ = std::fs::write(&exclude, updated);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_worktree_names() {
        assert_eq!(sanitize_name("feat-auth_1.2").expect("ok"), "feat-auth_1.2");
        assert!(sanitize_name("").is_err());
        assert!(sanitize_name("../escape").is_err());
        assert!(sanitize_name("a/b").is_err());
        assert!(sanitize_name("-flag").is_err());
    }

    #[test]
    fn creates_and_reopens_git_worktree_without_dirtying_main_repo() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("mkdir");
        let run = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .env("GIT_AUTHOR_NAME", "quack")
                .env("GIT_AUTHOR_EMAIL", "quack@example.com")
                .env("GIT_COMMITTER_NAME", "quack")
                .env("GIT_COMMITTER_EMAIL", "quack@example.com")
                .status()
                .expect("git")
        };
        if !run(&["init", "-q"]).success() {
            return;
        }
        std::fs::write(repo.join("hello.txt"), "hello\n").expect("write");
        assert!(run(&["add", "hello.txt"]).success());
        assert!(run(&["commit", "-q", "-m", "init"]).success());

        let wt = create_or_open(&repo, "side-task", false).expect("create worktree");
        assert!(wt.created);
        assert_eq!(wt.branch, "quack/side-task");
        assert!(wt.path.join("hello.txt").exists());
        assert!(create_or_open(&repo, "side-task", false).is_err());

        let reopened = create_or_open(&repo, "side-task", true).expect("reopen");
        assert!(!reopened.created);
        assert_eq!(reopened.path, wt.path);

        let status = git_output(&repo, &["status", "--porcelain"]).expect("status");
        assert!(status.is_empty(), "main repo dirtied: {status}");
    }
}
