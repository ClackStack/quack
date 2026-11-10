//! The login-shell `PATH` fix (docs/plan/15-desktop-client.md#the-bridge): when
//! `quack-gui` is launched from Finder/Dock or a desktop launcher rather than a
//! terminal, the process `PATH` is the system default and misses Homebrew, `~/.cargo/
//! bin`, `npx` and friends. Resolving the login shell's `PATH` once at startup, before
//! any engine spawns, lets `quack-shell`, `quack-mcp` (`npx`/`uvx`), `quack-lsp` and
//! `secret-tool` find the same tools the TUI finds.

/// Merge the login shell's `PATH` into this process's, keeping entries the current
/// `PATH` has that the login shell does not. Returns whether it changed anything.
#[cfg(unix)]
#[allow(unsafe_code)]
pub fn apply_login_shell_path() -> bool {
    let login = login_shell_path();
    let current = std::env::var_os("PATH").unwrap_or_default();
    let current = current.to_string_lossy().into_owned();
    let mut merged: Vec<String> = Vec::new();
    let entries = login
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .chain(current.split(':').map(str::to_owned));
    for entry in entries {
        if !entry.is_empty() && !merged.iter().any(|m| m == &entry) {
            merged.push(entry);
        }
    }
    if merged.is_empty() {
        return false;
    }
    let joined = merged.join(":");
    if joined == current {
        return false;
    }
    // Called once from the desktop shell before any engine spawns, so nothing reads
    // the old PATH afterwards.
    // SAFETY: single-threaded startup, before any engine or MCP server is spawned; no
    // other thread reads the environment.
    unsafe { std::env::set_var("PATH", joined) };
    true
}

/// Not needed on Windows: GUI processes launched from the shell get the user PATH from
/// the registry.
#[cfg(not(unix))]
pub fn apply_login_shell_path() -> bool {
    false
}

/// The login shell's `PATH`, from `$SHELL -l -c` (sh-compatible shells and fish both
/// print a `PATH` listing there).
#[cfg(unix)]
fn login_shell_path() -> Vec<std::path::PathBuf> {
    let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
    let output = std::process::Command::new(&shell)
        .args(["-l", "-c", "echo $PATH"])
        .output();
    match output {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            text.split(':').map(std::path::PathBuf::from).collect()
        }
        Ok(out) => {
            tracing::warn!(
                shell = %shell.to_string_lossy(),
                "the login shell exited with {} looking up the PATH",
                out.status
            );
            Vec::new()
        }
        Err(e) => {
            tracing::warn!(%e, shell = %shell.to_string_lossy(), "could not run the login shell");
            Vec::new()
        }
    }
}
