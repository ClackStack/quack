//! Locating and spawning the detached desktop client (`quack-gui`).
//!
//! Shared between `quack gui` in the CLI and `/gui` in the TUI so both resolve the
//! sibling binary and build launch flags uniformly.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Platform filename of the desktop GUI binary.
pub const BIN_NAME: &str = if cfg!(windows) {
    "quack-gui.exe"
} else {
    "quack-gui"
};

/// Locate `quack-gui` beside the running executable, or on `PATH`.
pub fn find_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok();
    let path_env = std::env::var_os("PATH");
    find_binary_in(exe.as_deref(), path_env.as_deref())
}

/// Pure helper for locating `quack-gui` given `current_exe` and `PATH` (unit-tested).
pub fn find_binary_in(current_exe: Option<&Path>, path_env: Option<&OsStr>) -> Option<PathBuf> {
    if let Some(dir) = current_exe.and_then(|p| p.parent()) {
        let sibling = dir.join(BIN_NAME);
        if is_executable(&sibling) {
            return Some(sibling);
        }
    }
    let path_env = path_env?;
    for dir in std::env::split_paths(path_env) {
        let candidate = dir.join(BIN_NAME);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    is_executable_mode(&meta)
}

#[cfg(unix)]
fn is_executable_mode(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable_mode(_meta: &std::fs::Metadata) -> bool {
    true
}

/// Build CLI arguments understood by `quack-gui`.
pub fn build_args(
    paths: &[PathBuf],
    resume: Option<&str>,
    continue_latest: bool,
    no_restore: bool,
) -> Vec<OsString> {
    let mut args = Vec::new();
    if no_restore {
        args.push(OsString::from("--no-restore"));
    }
    if let Some(id) = resume.filter(|s| !s.is_empty()) {
        args.push(OsString::from("--resume"));
        args.push(OsString::from(id));
    } else if continue_latest {
        args.push(OsString::from("--continue"));
    }
    if paths.iter().any(|p| p.to_string_lossy().starts_with('-')) {
        args.push(OsString::from("--"));
    }
    for path in paths {
        args.push(path.as_os_str().to_os_string());
    }
    args
}

/// Spawn `bin` detached with stdio closed so it outlives a terminating CLI/TUI process.
pub fn spawn(bin: &Path, args: &[OsString], cwd: Option<&Path>) -> io::Result<()> {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = cwd.filter(|d| !d.as_os_str().is_empty()) {
        cmd.current_dir(dir);
    }
    cmd.spawn().map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_exec(path: &Path) {
        std::fs::write(path, b"").expect("write dummy bin");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(path).expect("meta").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(path, perms).expect("chmod");
        }
    }

    #[test]
    fn prefers_sibling_binary_over_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let bin_dir = tmp.path().join("bin");
        let path_dir = tmp.path().join("path");
        std::fs::create_dir_all(&bin_dir).expect("bin_dir");
        std::fs::create_dir_all(&path_dir).expect("path_dir");

        let fake_cli = bin_dir.join("quack");
        let sibling_gui = bin_dir.join(BIN_NAME);
        let path_gui = path_dir.join(BIN_NAME);
        make_exec(&sibling_gui);
        make_exec(&path_gui);

        let joined_path = std::env::join_paths([&path_dir]).expect("join PATH");
        assert_eq!(
            find_binary_in(Some(&fake_cli), Some(&joined_path)),
            Some(sibling_gui)
        );
        std::fs::remove_file(bin_dir.join(BIN_NAME)).expect("remove sibling");
        assert_eq!(
            find_binary_in(Some(&fake_cli), Some(&joined_path)),
            Some(path_gui)
        );
    }

    #[test]
    fn builds_flags_and_protects_leading_dash_paths() {
        let args = build_args(
            &[PathBuf::from("-weird-dir"), PathBuf::from("/tmp/repo")],
            Some("abc123"),
            true,
            true,
        );
        let rendered: Vec<String> = args
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            rendered,
            [
                "--no-restore",
                "--resume",
                "abc123",
                "--",
                "-weird-dir",
                "/tmp/repo"
            ]
        );
    }
}
