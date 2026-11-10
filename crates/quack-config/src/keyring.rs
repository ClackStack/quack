//! The OS credential store: Credential Manager on Windows, Keychain on macOS, and the
//! Secret Service elsewhere. The Secret Service is reached through `secret-tool` (from
//! libsecret) rather than a D-Bus stack compiled in, which would cost over 100 crates.
//! Headless Linux (a server, WSL, a container) has no Secret Service, so [`available`]
//! asks first and callers fall back to `file` or `env` secrets.
//!
//! Every function takes `(service, user)` and reports failures as plain text.

/// Whether a secret can be stored right now. `Err` says why not, in one sentence that
/// names the way around it.
pub fn available() -> Result<(), String> {
    imp::available()
}

/// The stored value, or `None` when nothing is stored.
pub fn get(service: &str, user: &str) -> Result<Option<String>, String> {
    imp::get(service, user)
}

pub fn set(service: &str, user: &str, value: &str) -> Result<(), String> {
    imp::set(service, user, value)
}

pub fn delete(service: &str, user: &str) -> Result<(), String> {
    imp::delete(service, user)
}

#[cfg(any(windows, target_os = "macos"))]
mod imp {
    use std::sync::LazyLock;

    use keyring_core::{Entry, Error};

    static STORE: LazyLock<Result<(), String>> = LazyLock::new(|| {
        #[cfg(windows)]
        let store = windows_native_keyring_store::Store::new();
        #[cfg(target_os = "macos")]
        let store = apple_native_keyring_store::keychain::Store::new();
        keyring_core::set_default_store(store.map_err(|e| e.to_string())?);
        Ok(())
    });

    fn entry(service: &str, user: &str) -> Result<Entry, String> {
        (*STORE).clone()?;
        Entry::new(service, user).map_err(|e| e.to_string())
    }

    pub fn available() -> Result<(), String> {
        entry("quack", "__probe__").map(|_| ())
    }

    pub fn get(service: &str, user: &str) -> Result<Option<String>, String> {
        match entry(service, user)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn set(service: &str, user: &str, value: &str) -> Result<(), String> {
        entry(service, user)?
            .set_password(value)
            .map_err(|e| e.to_string())
    }

    pub fn delete(service: &str, user: &str) -> Result<(), String> {
        match entry(service, user)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use std::io::Write;
    use std::process::{Command, Output, Stdio};

    const TOOL: &str = "secret-tool";

    /// Why the Secret Service is unreachable, when the error names it. One short
    /// sentence; `with_advice` appends the ways around it.
    fn no_service(stderr: &str) -> Option<&'static str> {
        if stderr.contains("Cannot autolaunch D-Bus")
            || stderr.contains("D-Bus library appeared to be initialized")
            || stderr.contains("Failed to connect to socket")
            || stderr.contains("Could not connect")
            || stderr.contains("Connection refused")
        {
            Some("no D-Bus session bus is running")
        } else if stderr.contains("org.freedesktop.secrets") {
            Some("no Secret Service is running (nothing provides org.freedesktop.secrets)")
        } else {
            None
        }
    }

    /// The error for a stored or read secret: the reason, then the ways around it.
    fn with_advice(reason: &str) -> String {
        format!(
            "{reason}; the OS credential store needs one (gnome-keyring, or another Secret \
             Service), or read the key from elsewhere instead: `api_key = {{ env = \"VAR\" }}`, \
             `{{ file = \"path\" }}` or `{{ cmd = [..] }}` in the provider file"
        )
    }

    pub fn available() -> Result<(), String> {
        // A lookup for attributes nothing stores exits non-zero with no output when the
        // service answers; when it does not, the error says which piece is missing.
        let out = run(
            &["lookup", "service", "quack", "username", "__probe__"],
            None,
        )?;
        if out.status.success() || out.stderr.is_empty() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        match no_service(&stderr) {
            Some(reason) => Err(reason.to_owned()),
            None => Err(failure(&out)),
        }
    }

    pub fn get(service: &str, user: &str) -> Result<Option<String>, String> {
        let out = run(&["lookup", "service", service, "username", user], None)
            .map_err(|e| with_advice(&e))?;
        // `lookup` exits non-zero with no output when nothing matches.
        if !out.status.success() {
            return if out.stderr.is_empty() {
                Ok(None)
            } else {
                Err(failure(&out))
            };
        }
        let v =
            String::from_utf8(out.stdout).map_err(|_| "stored value is not UTF-8".to_owned())?;
        Ok(Some(v.strip_suffix('\n').unwrap_or(&v).to_owned()))
    }

    pub fn set(service: &str, user: &str, value: &str) -> Result<(), String> {
        let label = format!("{service}/{user}");
        let out = run(
            &[
                "store", "--label", &label, "service", service, "username", user,
            ],
            Some(value),
        )
        .map_err(|e| with_advice(&e))?;
        out.status
            .success()
            .then_some(())
            .ok_or_else(|| failure(&out))
    }

    pub fn delete(service: &str, user: &str) -> Result<(), String> {
        let out = run(&["clear", "service", service, "username", user], None)
            .map_err(|e| with_advice(&e))?;
        // `clear` also exits non-zero when nothing matched; that is fine for a delete.
        if out.status.success() || out.stderr.is_empty() {
            Ok(())
        } else {
            Err(failure(&out))
        }
    }

    /// Why the tool cannot be reached, as a short reason callers can add advice to.
    fn no_tool() -> String {
        format!(
            "`{TOOL}` not found; install libsecret-tools (Debian/Ubuntu) or libsecret \
             (Fedora/Arch)"
        )
    }

    fn run(args: &[&str], input: Option<&str>) -> Result<Output, String> {
        let mut child = Command::new(TOOL)
            .args(args)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => no_tool(),
                _ => format!("{TOOL}: {e}"),
            })?;
        if let (Some(value), Some(mut stdin)) = (input, child.stdin.take()) {
            stdin
                .write_all(value.as_bytes())
                .map_err(|e| format!("{TOOL}: {e}"))?;
        }
        child.wait_with_output().map_err(|e| format!("{TOOL}: {e}"))
    }

    fn failure(out: &Output) -> String {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err
            .trim()
            .strip_prefix("secret-tool: ")
            .unwrap_or(err.trim());
        match err {
            "" => format!("{TOOL} failed ({})", out.status),
            msg => no_service(msg).map_or_else(|| format!("{TOOL}: {msg}"), with_advice),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{no_service, with_advice};

        #[test]
        fn headless_errors_name_the_ways_around_the_keyring() {
            for stderr in [
                "secret-tool: Cannot autolaunch D-Bus without X11 $DISPLAY",
                "secret-tool: The name org.freedesktop.secrets was not provided by any \
                 .service files",
                "secret-tool: Failed to connect to socket /run/user/1000/bus: Connection \
                 refused",
                "secret-tool: Could not connect: No such file or directory",
            ] {
                let reason = no_service(stderr).expect("classified as no service");
                let msg = with_advice(reason);
                assert!(msg.contains("read the key from elsewhere"), "{msg}");
            }
            assert!(no_service("secret-tool: something else").is_none());
        }
    }
}
