//! System and terminal clipboard writes (OSC 52 plus local OS clipboard helpers).
//!
//! OSC 52 works over SSH and in terminals that support it (Kitty, Ghostty, WezTerm,
//! Alacritty, Windows Terminal, iTerm2, VS Code), while terminals such as GNOME Terminal
//! (VTE), Konsole's default setup, and macOS Terminal.app ignore it. Writing both OSC 52
//! and the local OS clipboard helper (serialized on a background thread so rendering never
//! stalls) covers both local desktops and remote SSH sessions with no extra dependencies.

use std::io::{self, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Maximum time to wait on exit for a queued OS clipboard write to complete.
const SHUTDOWN_WAIT: Duration = Duration::from_millis(500);

/// PowerShell snippet that reads UTF-8 text verbatim from stdin into the Windows clipboard.
const PS_SET_CLIPBOARD: &str = "[Console]::InputEncoding = [System.Text.Encoding]::UTF8; Set-Clipboard -Value ([Console]::In.ReadToEnd())";

/// Put text on the clipboard through the terminal (OSC 52). Works over SSH and in terminals
/// that support it; terminals that do not silently ignore the sequence.
pub(crate) fn write_osc52(out: &mut impl Write, text: &str) -> io::Result<()> {
    write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()))?;
    out.flush()
}

/// Serialized background writer for the local OS clipboard.
///
/// Spawned lazily on the first copy so startup pays nothing, and coalesces back-to-back
/// requests so an older copy can never finish after a newer one and overwrite it.
#[derive(Debug, Default)]
pub(crate) struct Worker {
    tx: Option<mpsc::UnboundedSender<String>>,
    task: Option<JoinHandle<()>>,
}

impl Worker {
    /// Queue `text` for the local OS clipboard without blocking the TUI event loop.
    pub(crate) fn copy(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        if self
            .tx
            .as_ref()
            .is_none_or(mpsc::UnboundedSender::is_closed)
        {
            let (tx, mut rx) = mpsc::unbounded_channel::<String>();
            let task = tokio::task::spawn_blocking(move || {
                while let Some(mut latest) = rx.blocking_recv() {
                    while let Ok(newer) = rx.try_recv() {
                        latest = newer;
                    }
                    copy_to_os(&latest);
                }
            });
            self.tx = Some(tx);
            self.task = Some(task);
        }
        if let Some(tx) = &self.tx {
            let _ = tx.send(text);
        }
    }

    /// Wait briefly for any in-flight clipboard write before exiting (e.g. `/copy` then `/quit`).
    pub(crate) async fn finish(mut self) {
        drop(self.tx.take());
        if let Some(task) = self.task.take() {
            let _ = tokio::time::timeout(SHUTDOWN_WAIT, task).await;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platform {
    Linux,
    MacOs,
    Windows,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClipboardCmd {
    program: &'static str,
    args: &'static [&'static str],
    env: &'static [(&'static str, &'static str)],
}

fn current_platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::MacOs
    } else if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Linux
    }
}

fn candidates_for(platform: Platform, env: impl Fn(&str) -> Option<String>) -> Vec<ClipboardCmd> {
    let has = |key: &str| env(key).is_some_and(|v| !v.is_empty());
    let is_ssh = has("SSH_TTY") || has("SSH_CONNECTION");
    let mut cmds = Vec::new();
    match platform {
        Platform::MacOs => {
            if !is_ssh {
                cmds.push(ClipboardCmd {
                    program: "pbcopy",
                    args: &[],
                    env: &[("LC_CTYPE", "UTF-8")],
                });
            }
            if has("TMUX") {
                cmds.push(ClipboardCmd {
                    program: "tmux",
                    args: &["load-buffer", "-w", "-"],
                    env: &[],
                });
            }
        }
        Platform::Windows => {
            if !is_ssh {
                cmds.push(ClipboardCmd {
                    program: "powershell.exe",
                    args: &[
                        "-NoProfile",
                        "-NonInteractive",
                        "-WindowStyle",
                        "Hidden",
                        "-Command",
                        PS_SET_CLIPBOARD,
                    ],
                    env: &[],
                });
                cmds.push(ClipboardCmd {
                    program: "pwsh.exe",
                    args: &[
                        "-NoProfile",
                        "-NonInteractive",
                        "-Command",
                        PS_SET_CLIPBOARD,
                    ],
                    env: &[],
                });
            }
        }
        Platform::Linux => {
            if has("WAYLAND_DISPLAY") {
                cmds.push(ClipboardCmd {
                    program: "wl-copy",
                    args: &["--type", "text/plain;charset=utf-8"],
                    env: &[],
                });
            }
            if has("DISPLAY") {
                cmds.push(ClipboardCmd {
                    program: "xclip",
                    args: &["-selection", "clipboard", "-in"],
                    env: &[],
                });
                cmds.push(ClipboardCmd {
                    program: "xsel",
                    args: &["--clipboard", "--input"],
                    env: &[],
                });
            }
            let is_wsl = has("WSL_DISTRO_NAME") || has("WSL_INTEROP");
            if is_wsl && !is_ssh {
                cmds.push(ClipboardCmd {
                    program: "powershell.exe",
                    args: &[
                        "-NoProfile",
                        "-NonInteractive",
                        "-WindowStyle",
                        "Hidden",
                        "-Command",
                        PS_SET_CLIPBOARD,
                    ],
                    env: &[],
                });
            }
            if has("TMUX") {
                cmds.push(ClipboardCmd {
                    program: "tmux",
                    args: &["load-buffer", "-w", "-"],
                    env: &[],
                });
            }
        }
    }
    cmds
}

fn copy_to_os(text: &str) {
    let cmds = candidates_for(current_platform(), |k| std::env::var(k).ok());
    for cmd in &cmds {
        if run_cmd(cmd.program, cmd.args, cmd.env, text) {
            return;
        }
    }
}

/// Pipe `text` into a clipboard helper and wait for the parent process to exit.
///
/// `stdout` and `stderr` are sent to `/dev/null` so X11/Wayland helpers (`xclip`, `xsel`,
/// `wl-copy`) that fork a background selection owner do not keep any pipe open and stall
/// waiting for EOF.
fn run_cmd(program: &str, args: &[&str], env: &[(&str, &str)], text: &str) -> bool {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (k, v) in env {
        command.env(k, v);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: avoid flashing a console window when running PowerShell.
        command.creation_flags(0x0800_0000);
    }
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(program, %e, "clipboard helper unavailable");
            return false;
        }
    };
    let wrote = if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .and_then(|()| stdin.flush())
    } else {
        Ok(())
    };
    let status = child.wait();
    match (wrote, status) {
        (Ok(()), Ok(s)) if s.success() => true,
        (Err(e), _) => {
            tracing::debug!(program, %e, "failed writing to clipboard helper");
            false
        }
        (_, Ok(s)) => {
            tracing::debug!(program, code = ?s.code(), "clipboard helper exited non-zero");
            false
        }
        (_, Err(e)) => {
            tracing::debug!(program, %e, "failed waiting on clipboard helper");
            false
        }
    }
}

fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ABC[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn base64_matches_the_standard() {
        for (input, want) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), want);
        }
    }

    #[test]
    fn write_osc52_encodes_clipboard_sequence() {
        let mut buf = Vec::new();
        write_osc52(&mut buf, "hello").expect("write osc52");
        assert_eq!(buf, b"\x1b]52;c;aGVsbG8=\x07");
    }

    #[test]
    fn candidates_follow_platform_and_session_environment() {
        let programs = |platform, env: &[(&str, &str)]| -> Vec<&'static str> {
            candidates_for(platform, map_env(env))
                .into_iter()
                .map(|c| c.program)
                .collect()
        };

        // Linux X11 (e.g. GNOME Terminal on X11): xclip then xsel.
        assert_eq!(
            programs(Platform::Linux, &[("DISPLAY", ":0")]),
            vec!["xclip", "xsel"]
        );
        // Linux Wayland with XWayland fallback.
        assert_eq!(
            programs(
                Platform::Linux,
                &[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")]
            ),
            vec!["wl-copy", "xclip", "xsel"]
        );
        // Headless SSH on Linux: no local display tools, rely on OSC 52.
        assert!(programs(Platform::Linux, &[("SSH_TTY", "/dev/pts/1")]).is_empty());
        // SSH inside tmux: tmux load-buffer -w forwards OSC 52 even under set-clipboard external.
        assert_eq!(
            programs(
                Platform::Linux,
                &[
                    ("SSH_TTY", "/dev/pts/1"),
                    ("TMUX", "/tmp/tmux-1000/default,1,0")
                ]
            ),
            vec!["tmux"]
        );
        // WSL without display uses Windows PowerShell UTF-8 Set-Clipboard.
        assert_eq!(
            programs(Platform::Linux, &[("WSL_DISTRO_NAME", "Ubuntu")]),
            vec!["powershell.exe"]
        );
        // macOS local vs SSH.
        assert_eq!(programs(Platform::MacOs, &[]), vec!["pbcopy"]);
        assert!(programs(Platform::MacOs, &[("SSH_CONNECTION", "1.2.3.4 1 2 22")]).is_empty());
        // Windows local vs SSH.
        assert_eq!(
            programs(Platform::Windows, &[]),
            vec!["powershell.exe", "pwsh.exe"]
        );
        assert!(programs(Platform::Windows, &[("SSH_TTY", "1")]).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn run_cmd_pipes_utf8_and_does_not_hang_on_background_child() {
        use std::os::unix::fs::PermissionsExt;
        use std::path::PathBuf;

        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        let dir = std::env::temp_dir().join(format!(
            "quack-clip-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let _cleanup = Cleanup(dir.clone());
        let out = dir.join("copied.txt");
        let helper = dir.join("fake-clip.sh");
        // Emulates xclip/xsel/wl-copy: consumes stdin, spawns a background child that stays
        // alive briefly (inheriting stdout/stderr), and exits 0 immediately.
        let script = format!(
            "#!/bin/sh\ncat > '{}'\n(sleep 2) &\nexit 0\n",
            out.display()
        );
        std::fs::write(&helper, script).expect("write helper");
        let mut perms = std::fs::metadata(&helper).expect("meta").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&helper, perms).expect("chmod");

        let started = std::time::Instant::now();
        let text = "hello — quack ✓\nsecond line";
        assert!(run_cmd(helper.to_str().expect("utf8 path"), &[], &[], text));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "helper with background child must not stall wait()"
        );
        assert_eq!(std::fs::read_to_string(&out).expect("read copied"), text);
    }

    #[test]
    fn run_cmd_returns_false_when_helper_is_missing() {
        assert!(!run_cmd(
            "definitely-nonexistent-quack-clipboard-binary-xyz",
            &[],
            &[],
            "hello"
        ));
    }
}
