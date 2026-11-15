//! Recover the login-shell PATH before the desktop runtime starts. The lookup has
//! bounded time and output, ignores startup banners, and preserves non-UTF-8 paths.

#[cfg(unix)]
mod unix {
    use std::ffi::{OsStr, OsString};
    use std::io::{self, Read};
    use std::os::fd::OwnedFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::CommandExt;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    const TIMEOUT: Duration = Duration::from_secs(2);
    const MAX_OUTPUT: usize = 64 * 1024;
    const MARKER: &[u8] = b"\0quack-path\0";
    // The inner POSIX shell reads the exported PATH, even when the login shell is
    // fish (whose unquoted PATH is a list). NUL framing excludes banners/newlines.
    const SCRIPT: &str = "/bin/sh -c 'printf \"\\000quack-path\\000%s\\000\" \"$PATH\"'";

    /// Called only by main, before Tauri or any other threads are created.
    #[allow(unsafe_code)]
    pub fn apply() -> bool {
        let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
        let login = match lookup(Path::new(&shell), TIMEOUT) {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(%error, "could not recover the login-shell PATH; keeping the inherited PATH");
                return false;
            }
        };
        let current = std::env::var_os("PATH").unwrap_or_default();
        let Some(merged) = merge(&login, &current) else {
            return false;
        };
        // SAFETY: main calls this during single-threaded startup, before Tauri,
        // tokio, engines, or any worker threads can read the process environment.
        unsafe { std::env::set_var("PATH", merged) };
        true
    }

    fn merge(login: &OsStr, current: &OsStr) -> Option<OsString> {
        let mut paths = Vec::new();
        for path in std::env::split_paths(login).chain(std::env::split_paths(current)) {
            if !path.as_os_str().is_empty() && !paths.contains(&path) {
                paths.push(path);
            }
        }
        let joined = std::env::join_paths(paths).ok()?;
        (!joined.is_empty() && joined != current).then_some(joined)
    }

    fn parse(output: &[u8]) -> io::Result<OsString> {
        let start = output
            .windows(MARKER.len())
            .position(|w| w == MARKER)
            .map(|position| position + MARKER.len())
            .ok_or_else(|| io::Error::other("the login shell did not return a PATH"))?;
        let path = &output[start..];
        let end = path
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| io::Error::other("the login shell returned an incomplete PATH"))?;
        if end == 0 {
            return Err(io::Error::other("the login shell returned an empty PATH"));
        }
        Ok(OsStr::from_bytes(&path[..end]).to_os_string())
    }

    fn lookup(shell: &Path, timeout: Duration) -> io::Result<OsString> {
        capture(Command::new(shell).args(["-l", "-c", SCRIPT]), timeout)
    }

    fn capture(command: &mut Command, timeout: Duration) -> io::Result<OsString> {
        // A nonblocking socket avoids reader threads (environment mutation follows
        // this call) and pipes held open by shell-startup grandchildren.
        let (mut reader, writer) = UnixStream::pair()?;
        reader.set_nonblocking(true)?;
        let stdout: OwnedFd = writer.into();
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        let deadline = Instant::now() + timeout;
        let result = (|| {
            let mut output = Vec::new();
            let mut buffer = [0; 4096];
            let mut exited = None;
            loop {
                loop {
                    if Instant::now() >= deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "login-shell PATH lookup timed out",
                        ));
                    }
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(size) => {
                            if output.len() + size > MAX_OUTPUT {
                                return Err(io::Error::other("login-shell output exceeded 64 KiB"));
                            }
                            output.extend_from_slice(&buffer[..size]);
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(error) => return Err(error),
                    }
                }
                // Read once after observing exit to include the shell's final write.
                if let Some(status) = exited {
                    return if status {
                        parse(&output)
                    } else {
                        Err(io::Error::other("the login shell exited unsuccessfully"))
                    };
                }
                exited = child.try_wait()?.map(|status| status.success());
                if exited.is_none() {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        })();
        // Startup commands must not outlive the lookup, including on success.
        if let Ok(pid) = i32::try_from(child.id()) {
            let _ = nix::sys::signal::killpg(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
        let _ = child.kill();
        let _ = child.wait();
        result
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn framed_path_ignores_banners_and_preserves_bytes() {
            let path = parse(b"welcome\n\0quack-path\0/home/a b:/odd\xff/bin:/bin\0\nbye\n")
                .expect("framed path");
            assert_eq!(path.as_bytes(), b"/home/a b:/odd\xff/bin:/bin");
            assert!(parse(b"/bin\n").is_err());
            assert!(parse(b"\0quack-path\0/bin").is_err());
            assert!(parse(b"\0quack-path\0\0").is_err());
        }

        #[test]
        fn merge_prefers_login_entries_without_empty_or_duplicate_paths() {
            assert_eq!(
                merge(OsStr::new("/custom:/bin:"), OsStr::new("/bin:/inherited")),
                Some(OsString::from("/custom:/bin:/inherited"))
            );
            assert_eq!(merge(OsStr::new("/bin"), OsStr::new("/bin")), None);
        }

        fn shell(root: &Path, script: &str) -> Command {
            let path = root.join("shell");
            std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).expect("fake shell");
            let mut command = Command::new("/bin/sh");
            command.arg(path).args(["-l", "-c", SCRIPT]);
            command
        }

        #[test]
        fn lookup_reads_only_the_exported_path_after_startup_output() {
            let tmp = tempfile::tempdir().expect("tempdir");
            let mut shell = shell(
                tmp.path(),
                "printf 'welcome\\n'\nexport PATH='/custom tools:/bin'\nexec /bin/sh -c \"$3\"",
            );
            assert_eq!(
                capture(&mut shell, Duration::from_secs(1)).expect("lookup"),
                OsString::from("/custom tools:/bin")
            );
        }

        #[test]
        fn lookup_bounds_shell_startup_and_output() {
            let tmp = tempfile::tempdir().expect("tempdir");
            let mut slow = shell(tmp.path(), "exec /bin/sleep 10");
            let start = Instant::now();
            assert_eq!(
                capture(&mut slow, Duration::from_millis(30))
                    .expect_err("timeout")
                    .kind(),
                io::ErrorKind::TimedOut
            );
            assert!(start.elapsed() < Duration::from_secs(2));
            let mut noisy = shell(
                tmp.path(),
                "while :; do printf 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx'; done",
            );
            let error = capture(&mut noisy, Duration::from_secs(1)).expect_err("output cap");
            assert!(error.to_string().contains("64 KiB"), "{error}");
        }
    }
}

/// Merge the login-shell PATH with inherited entries before starting the runtime.
#[cfg(unix)]
pub fn apply_login_shell_path() -> bool {
    unix::apply()
}

/// Windows GUI processes already inherit the user PATH.
#[cfg(not(unix))]
pub fn apply_login_shell_path() -> bool {
    false
}
