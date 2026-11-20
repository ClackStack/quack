//! OS sandboxing for shell commands and servers (docs/plan/11-security.md#os-sandboxing).
//!
//! A sandboxed process can read everything, write only inside [`Policy::writable`] (plus
//! `/dev/null` and similar), and, on Linux and macOS unless [`Policy::network`] is set,
//! open no network connections. Local (Unix-domain) sockets keep working.
//!
//! - Linux: Landlock restricts writes and a seccomp filter refuses network sockets. Both
//!   are applied in the child just before it runs the program, and bind everything it
//!   starts.
//! - macOS: the program runs under `sandbox-exec` with a generated Seatbelt profile.
//! - Windows: Restricted Token (`DISABLE_MAX_PRIVILEGE` + Low Integrity `S-1-16-4096`,
//!   `NO_WRITE_UP`) combined with Low-Integrity labels on [`Policy::writable`] and a Job
//!   Object with UI/clipboard restrictions and kill-on-close. The temp folder is replaced
//!   by a `quack-sandbox` folder inside it (and `TEMP`/`TMP` point there), so the user's
//!   temp folder itself is never relabelled. The network is not restricted.

use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::{ExitStatus, Stdio};
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    /// Folders (or files) that may be written. Empty means read-only.
    pub writable: Vec<PathBuf>,
    /// Whether network connections are allowed.
    pub network: bool,
}

impl Policy {
    /// The usual places programs write besides the workspace: the temp folders.
    pub fn temp_dirs() -> Vec<PathBuf> {
        let mut dirs = vec![std::env::temp_dir()];
        if cfg!(unix) {
            dirs.push("/tmp".into());
            dirs.push("/var/tmp".into());
        }
        dirs.dedup();
        dirs
    }
}

/// The mechanism in use on this machine, or why sandboxing is not available.
pub fn available() -> Result<&'static str, String> {
    #[cfg(target_os = "linux")]
    return linux::available();
    #[cfg(target_os = "macos")]
    return macos::available();
    #[cfg(target_os = "windows")]
    return windows::available();
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    Err("the sandbox is not available on this OS yet".into())
}

/// A raw `tokio::process::Command` that runs `program` with `args` inside the sandbox.
/// Available on Unix (Linux/macOS); cross-platform callers should prefer [`sandboxed`].
pub fn command<I, S>(
    policy: &Policy,
    program: &OsStr,
    args: I,
) -> Result<tokio::process::Command, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    #[cfg(target_os = "linux")]
    return linux::command(policy, program, args);
    #[cfg(target_os = "macos")]
    return macos::command(policy, program, args);
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (policy, program, args.into_iter().count());
        Err("use quack_sandbox::sandboxed() on this OS".into())
    }
}

/// Cross-platform command builder supporting both unsandboxed and OS-sandboxed execution
/// across Linux (Landlock+seccomp), macOS (Seatbelt), and Windows (Restricted Token).
#[derive(Debug)]
pub struct Command {
    inner: CommandInner,
}

#[derive(Debug)]
enum CommandInner {
    Tokio(tokio::process::Command),
    #[cfg(target_os = "windows")]
    Windows(windows::WindowsCommand),
}

/// Build a cross-platform command running `program` with `args` inside the OS sandbox.
pub fn sandboxed<I, S>(policy: &Policy, program: &OsStr, args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    #[cfg(target_os = "linux")]
    return Ok(Command {
        inner: CommandInner::Tokio(linux::command(policy, program, args)?),
    });
    #[cfg(target_os = "macos")]
    return Ok(Command {
        inner: CommandInner::Tokio(macos::command(policy, program, args)?),
    });
    #[cfg(target_os = "windows")]
    return Ok(Command {
        inner: CommandInner::Windows(windows::WindowsCommand::new(policy, program, args)?),
    });
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (policy, program, args.into_iter().count());
        Err("the sandbox is not available on this OS yet".into())
    }
}

/// Build an unsandboxed cross-platform command running `program` with `args`.
pub fn unsandboxed<I, S>(program: &OsStr, args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = tokio::process::Command::new(program);
    #[cfg(windows)]
    {
        let args: Vec<std::ffi::OsString> = args
            .into_iter()
            .map(|a| a.as_ref().to_os_string())
            .collect();
        if let Some((switches, script)) = cmd_script(program, &args) {
            let mut raw = std::ffi::OsString::from("\"");
            raw.push(&script);
            raw.push("\"");
            cmd.args(switches).raw_arg(raw);
        } else {
            cmd.args(args);
        }
    }
    #[cfg(not(windows))]
    cmd.args(args);
    Command {
        inner: CommandInner::Tokio(cmd),
    }
}

/// `cmd.exe` doesn't split its command line the way C programs do: after `/S /C` it drops
/// the first and last quote and runs the rest as written, so a script whose quotes were
/// escaped as `\"` (the usual Windows quoting) breaks. For a `cmd` call this returns the
/// switches up to `/C` (or `/K`), with `/S` added, and the script to append unescaped
/// inside one pair of quotes. A script given as several arguments is joined with spaces,
/// quoting the ones that hold whitespace.
#[cfg(any(windows, test))]
fn cmd_script(
    program: &OsStr,
    args: &[std::ffi::OsString],
) -> Option<(Vec<std::ffi::OsString>, std::ffi::OsString)> {
    let stem = Path::new(program).file_stem()?;
    if !stem.eq_ignore_ascii_case("cmd") {
        return None;
    }
    let at = args
        .iter()
        .position(|a| a.eq_ignore_ascii_case("/c") || a.eq_ignore_ascii_case("/k"))?;
    let mut switches = args[..=at].to_vec();
    if !switches.iter().any(|a| a.eq_ignore_ascii_case("/s")) {
        switches.insert(at, "/S".into());
    }
    let rest = &args[at + 1..];
    let mut script = std::ffi::OsString::new();
    for (i, arg) in rest.iter().enumerate() {
        if i > 0 {
            script.push(" ");
        }
        let spaced = arg.to_string_lossy().contains(char::is_whitespace);
        if rest.len() > 1 && spaced {
            script.push("\"");
            script.push(arg);
            script.push("\"");
        } else {
            script.push(arg);
        }
    }
    Some((switches, script))
}

impl Command {
    pub fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        match &mut self.inner {
            CommandInner::Tokio(c) => {
                c.current_dir(dir);
            }
            #[cfg(target_os = "windows")]
            CommandInner::Windows(c) => {
                c.current_dir(dir);
            }
        }
        self
    }

    pub fn env_clear(&mut self) -> &mut Self {
        match &mut self.inner {
            CommandInner::Tokio(c) => {
                c.env_clear();
            }
            #[cfg(target_os = "windows")]
            CommandInner::Windows(c) => {
                c.env_clear();
            }
        }
        self
    }

    pub fn env(&mut self, k: impl AsRef<OsStr>, v: impl AsRef<OsStr>) -> &mut Self {
        match &mut self.inner {
            CommandInner::Tokio(c) => {
                c.env(k, v);
            }
            #[cfg(target_os = "windows")]
            CommandInner::Windows(c) => {
                c.env(k, v);
            }
        }
        self
    }

    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        match &mut self.inner {
            CommandInner::Tokio(c) => {
                c.envs(vars);
            }
            #[cfg(target_os = "windows")]
            CommandInner::Windows(c) => {
                c.envs(vars);
            }
        }
        self
    }

    pub fn stdin_null(&mut self) -> &mut Self {
        match &mut self.inner {
            CommandInner::Tokio(c) => {
                c.stdin(Stdio::null());
            }
            #[cfg(target_os = "windows")]
            CommandInner::Windows(c) => {
                c.stdin(windows::StdioCfg::Null);
            }
        }
        self
    }

    pub fn stdin_piped(&mut self) -> &mut Self {
        match &mut self.inner {
            CommandInner::Tokio(c) => {
                c.stdin(Stdio::piped());
            }
            #[cfg(target_os = "windows")]
            CommandInner::Windows(c) => {
                c.stdin(windows::StdioCfg::Piped);
            }
        }
        self
    }

    pub fn stdout_piped(&mut self) -> &mut Self {
        match &mut self.inner {
            CommandInner::Tokio(c) => {
                c.stdout(Stdio::piped());
            }
            #[cfg(target_os = "windows")]
            CommandInner::Windows(c) => {
                c.stdout(windows::StdioCfg::Piped);
            }
        }
        self
    }

    pub fn stderr_piped(&mut self) -> &mut Self {
        match &mut self.inner {
            CommandInner::Tokio(c) => {
                c.stderr(Stdio::piped());
            }
            #[cfg(target_os = "windows")]
            CommandInner::Windows(c) => {
                c.stderr(windows::StdioCfg::Piped);
            }
        }
        self
    }

    pub fn kill_on_drop(&mut self, yes: bool) -> &mut Self {
        match &mut self.inner {
            CommandInner::Tokio(c) => {
                c.kill_on_drop(yes);
            }
            #[cfg(target_os = "windows")]
            CommandInner::Windows(c) => {
                c.kill_on_drop(yes);
            }
        }
        self
    }

    #[cfg(unix)]
    pub fn process_group(&mut self, pgroup: i32) -> &mut Self {
        match &mut self.inner {
            CommandInner::Tokio(c) => {
                c.process_group(pgroup);
            }
        }
        self
    }

    #[cfg(windows)]
    pub fn creation_flags(&mut self, flags: u32) -> &mut Self {
        match &mut self.inner {
            CommandInner::Tokio(c) => {
                c.creation_flags(flags);
            }
            CommandInner::Windows(c) => {
                c.creation_flags(flags);
            }
        }
        self
    }

    pub fn spawn(&mut self) -> io::Result<Child> {
        match &mut self.inner {
            CommandInner::Tokio(c) => Ok(Child {
                inner: ChildInner::Tokio(c.spawn()?),
            }),
            #[cfg(target_os = "windows")]
            CommandInner::Windows(c) => Ok(Child {
                inner: ChildInner::Windows(c.spawn()?),
            }),
        }
    }
}

/// Cross-platform child process returned by [`Command::spawn`].
#[derive(Debug)]
pub struct Child {
    inner: ChildInner,
}

#[derive(Debug)]
enum ChildInner {
    Tokio(tokio::process::Child),
    #[cfg(target_os = "windows")]
    Windows(windows::WindowsChild),
}

impl Child {
    pub fn id(&self) -> Option<u32> {
        match &self.inner {
            ChildInner::Tokio(c) => c.id(),
            #[cfg(target_os = "windows")]
            ChildInner::Windows(c) => c.id(),
        }
    }

    #[cfg(windows)]
    pub fn raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        match &self.inner {
            ChildInner::Tokio(c) => c.raw_handle(),
            ChildInner::Windows(c) => c.raw_handle(),
        }
    }

    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        match &mut self.inner {
            ChildInner::Tokio(c) => c.stdin.take().map(ChildStdin::Tokio),
            #[cfg(target_os = "windows")]
            ChildInner::Windows(c) => c.stdin.take().map(ChildStdin::File),
        }
    }

    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        match &mut self.inner {
            ChildInner::Tokio(c) => c.stdout.take().map(ChildStdout::Tokio),
            #[cfg(target_os = "windows")]
            ChildInner::Windows(c) => c.stdout.take().map(ChildStdout::File),
        }
    }

    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        match &mut self.inner {
            ChildInner::Tokio(c) => c.stderr.take().map(ChildStderr::Tokio),
            #[cfg(target_os = "windows")]
            ChildInner::Windows(c) => c.stderr.take().map(ChildStderr::File),
        }
    }

    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        match &mut self.inner {
            ChildInner::Tokio(c) => c.wait().await,
            #[cfg(target_os = "windows")]
            ChildInner::Windows(c) => c.wait().await,
        }
    }

    pub async fn kill(&mut self) -> io::Result<()> {
        match &mut self.inner {
            ChildInner::Tokio(c) => c.kill().await,
            #[cfg(target_os = "windows")]
            ChildInner::Windows(c) => c.kill().await,
        }
    }
}

#[derive(Debug)]
pub enum ChildStdin {
    Tokio(tokio::process::ChildStdin),
    #[cfg(target_os = "windows")]
    File(tokio::fs::File),
}

impl AsyncWrite for ChildStdin {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Tokio(w) => Pin::new(w).poll_write(cx, buf),
            #[cfg(target_os = "windows")]
            Self::File(w) => Pin::new(w).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tokio(w) => Pin::new(w).poll_flush(cx),
            #[cfg(target_os = "windows")]
            Self::File(w) => Pin::new(w).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tokio(w) => Pin::new(w).poll_shutdown(cx),
            #[cfg(target_os = "windows")]
            Self::File(w) => Pin::new(w).poll_shutdown(cx),
        }
    }
}

#[derive(Debug)]
pub enum ChildStdout {
    Tokio(tokio::process::ChildStdout),
    #[cfg(target_os = "windows")]
    File(tokio::fs::File),
}

impl AsyncRead for ChildStdout {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tokio(r) => Pin::new(r).poll_read(cx, buf),
            #[cfg(target_os = "windows")]
            Self::File(r) => Pin::new(r).poll_read(cx, buf),
        }
    }
}

#[derive(Debug)]
pub enum ChildStderr {
    Tokio(tokio::process::ChildStderr),
    #[cfg(target_os = "windows")]
    File(tokio::fs::File),
}

impl AsyncRead for ChildStderr {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tokio(r) => Pin::new(r).poll_read(cx, buf),
            #[cfg(target_os = "windows")]
            Self::File(r) => Pin::new(r).poll_read(cx, buf),
        }
    }
}

/// Whether a failed command's output looks like the sandbox stopped it.
pub fn looks_blocked(output: &str) -> bool {
    const SIGNS: &[&str] = &[
        "Permission denied",
        "Operation not permitted",
        "Read-only file system",
        "Access is denied",
        "ERROR_ACCESS_DENIED",
        "UnauthorizedAccessException",
        "EACCES",
        "EPERM",
        "Could not resolve host",
        "Temporary failure in name resolution",
        "Name or service not known",
        "nodename nor servname provided",
        "Network is unreachable",
        "Failed to connect",
        "Couldn't connect",
        "getaddrinfo",
        "network is unreachable",
        "dial tcp",
    ];
    SIGNS.iter().any(|s| output.contains(s))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    fn args(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn cmd_scripts_go_after_s_c_unescaped() {
        let (switches, script) = cmd_script(
            "cmd.exe".as_ref(),
            &args(&["/D", "/C", r#"echo "hi" > "a b.txt""#]),
        )
        .expect("a cmd call");
        assert_eq!(switches, args(&["/D", "/S", "/C"]));
        assert_eq!(script, r#"echo "hi" > "a b.txt""#);

        let (switches, script) =
            cmd_script("CMD".as_ref(), &args(&["/s", "/c", "npx", "-y", "a b"])).expect("cmd");
        assert_eq!(switches, args(&["/s", "/c"]));
        assert_eq!(script, r#"npx -y "a b""#);

        assert!(cmd_script("pwsh.exe".as_ref(), &args(&["-Command", "x"])).is_none());
        assert!(cmd_script("cmd.exe".as_ref(), &args(&["/D"])).is_none());
    }
}
