//! Windows enterprise policy from `HKLM\SOFTWARE\Policies\quack`.
//!
//! On Windows, `%ProgramData%` is an environment variable that a non-admin user can
//! override for their own process. Reading `HKLM\SOFTWARE\Policies\quack` gives
//! administrators a user-immutable policy location (written via GPO, Intune/MDM, or
//! an elevated MSI transform) that cannot be bypassed by `%ProgramData%` or
//! `QUACK_SYSTEM_DIR`:
//!
//! - `SystemDir` (`REG_SZ`, absolute path): pins the system configuration directory.
//! - `PolicyToml` (`REG_SZ` or `REG_MULTI_SZ`): inline TOML merged at the system layer,
//!   after `<system>/config.toml`, before `[policy]` locks are enforced.

use std::path::PathBuf;

/// Registry subkey under `HKEY_LOCAL_MACHINE`.
pub const REGISTRY_SUBKEY: &str = r"SOFTWARE\Policies\quack";
/// Display origin for settings coming from the Windows policy registry key.
pub const REGISTRY_ORIGIN: &str = r"HKLM\SOFTWARE\Policies\quack";

/// Enterprise policy values read from `HKLM\SOFTWARE\Policies\quack` on Windows.
/// Always empty on non-Windows platforms.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegistryPolicy {
    /// Absolute path pinning the system layer directory (`SystemDir`).
    pub system_dir: Option<PathBuf>,
    /// Inline TOML applied at the system layer (`PolicyToml`).
    pub policy_toml: Option<String>,
}

impl RegistryPolicy {
    /// Whether any Windows machine-level registry policy value is configured.
    pub fn is_active(&self) -> bool {
        self.system_dir.is_some() || self.policy_toml.is_some()
    }
}

/// Read machine-level policy from `HKLM\SOFTWARE\Policies\quack` (Windows only).
pub fn read() -> RegistryPolicy {
    #[cfg(windows)]
    {
        windows_imp::read()
    }
    #[cfg(not(windows))]
    {
        RegistryPolicy::default()
    }
}

/// Parse a UTF-16 NUL-terminated `REG_SZ` buffer into a trimmed string, if non-empty.
#[cfg_attr(not(windows), allow(dead_code))]
fn decode_reg_sz(wide: &[u16]) -> Option<String> {
    let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    let text = String::from_utf16_lossy(&wide[..end]);
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Parse a UTF-16 `REG_MULTI_SZ` buffer (NUL-separated lines, terminated by double NUL)
/// into a newline-joined string, if non-empty.
#[cfg_attr(not(windows), allow(dead_code))]
fn decode_reg_multi_sz(wide: &[u16]) -> Option<String> {
    let mut lines = Vec::new();
    for chunk in wide.split(|&c| c == 0) {
        if chunk.is_empty() {
            break;
        }
        lines.push(String::from_utf16_lossy(chunk));
    }
    let joined = lines.join("\n");
    (!joined.trim().is_empty()).then_some(joined)
}

/// Validate a `SystemDir` value: must be non-empty and absolute so `cwd` cannot hijack it.
#[cfg_attr(not(windows), allow(dead_code))]
fn sanitize_system_dir(raw: Option<String>) -> Option<PathBuf> {
    let text = raw?;
    let path = PathBuf::from(text);
    // Reject relative paths so a user cannot redirect SystemDir via cwd.
    // On non-Windows tests, also recognize Windows drive roots like `C:\...`.
    let looks_absolute = path.is_absolute()
        || path.to_str().is_some_and(|s| {
            s.len() >= 3
                && s.as_bytes()[1] == b':'
                && (s.as_bytes()[2] == b'\\' || s.as_bytes()[2] == b'/')
        });
    looks_absolute.then_some(path)
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_imp {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY_LOCAL_MACHINE, REG_MULTI_SZ, RRF_NOEXPAND, RRF_RT_REG_MULTI_SZ, RRF_RT_REG_SZ,
        RegGetValueW,
    };

    use super::{
        REGISTRY_SUBKEY, RegistryPolicy, decode_reg_multi_sz, decode_reg_sz, sanitize_system_dir,
    };

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    fn query_value(name: &str, flags: u32) -> Option<(u32, Vec<u16>)> {
        let subkey = wide(REGISTRY_SUBKEY);
        let value = wide(name);
        let mut kind: u32 = 0;
        let mut bytes: u32 = 0;
        // SAFETY: `subkey` and `value` are NUL-terminated UTF-16 buffers; `pvData` is null
        // so `RegGetValueW` only queries the required buffer size and value type.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                subkey.as_ptr(),
                value.as_ptr(),
                flags,
                &mut kind,
                ptr::null_mut(),
                &mut bytes,
            )
        };
        if status != ERROR_SUCCESS || bytes == 0 || bytes > 1024 * 1024 {
            return None;
        }
        let len_u16 = (bytes as usize).div_ceil(2);
        let mut buf = vec![0u16; len_u16];
        let mut out_bytes = (buf.len() * 2) as u32;
        // SAFETY: `buf` is a writable UTF-16 buffer of `out_bytes` bytes; all pointers remain
        // valid for the duration of the synchronous Win32 call.
        let status = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                subkey.as_ptr(),
                value.as_ptr(),
                flags,
                &mut kind,
                buf.as_mut_ptr().cast(),
                &mut out_bytes,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        buf.truncate((out_bytes as usize) / 2);
        Some((kind, buf))
    }

    pub(super) fn read() -> RegistryPolicy {
        // `RRF_RT_REG_SZ | RRF_NOEXPAND` prevents env-var expansion (`%ProgramData%`) in SystemDir.
        let system_dir = query_value("SystemDir", RRF_RT_REG_SZ | RRF_NOEXPAND)
            .and_then(|(_, buf)| decode_reg_sz(&buf));
        let policy_toml = query_value(
            "PolicyToml",
            RRF_RT_REG_SZ | RRF_RT_REG_MULTI_SZ | RRF_NOEXPAND,
        )
        .and_then(|(kind, buf)| {
            if kind == REG_MULTI_SZ {
                decode_reg_multi_sz(&buf)
            } else {
                decode_reg_sz(&buf)
            }
        });
        RegistryPolicy {
            system_dir: sanitize_system_dir(system_dir),
            policy_toml,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_reg_sz_and_multi_sz() {
        let sz: Vec<u16> = "  C:\\ProgramData\\quack \0ignored"
            .encode_utf16()
            .collect();
        assert_eq!(decode_reg_sz(&sz).as_deref(), Some(r"C:\ProgramData\quack"));

        let empty: Vec<u16> = "   \0".encode_utf16().collect();
        assert_eq!(decode_reg_sz(&empty), None);

        let multi: Vec<u16> = "[network]\0offline = true\0\0ignored"
            .encode_utf16()
            .collect();
        assert_eq!(
            decode_reg_multi_sz(&multi).as_deref(),
            Some("[network]\noffline = true")
        );
    }

    #[test]
    fn rejects_relative_system_dir() {
        assert_eq!(sanitize_system_dir(Some("relative\\quack".into())), None);
        assert_eq!(
            sanitize_system_dir(Some(r"C:\ProgramData\quack".into())),
            Some(PathBuf::from(r"C:\ProgramData\quack"))
        );
        assert_eq!(
            sanitize_system_dir(Some("/etc/quack".into())),
            Some(PathBuf::from("/etc/quack"))
        );
    }
}
