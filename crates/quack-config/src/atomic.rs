//! Atomic file writes: a temporary file beside the target, then a rename, so two quacks
//! never leave a half-written file behind. Shared by the settings editor
//! (`config.toml`), the trust records (`trust.toml`) and the remembered state
//! (`state.toml`).

use std::path::Path;

/// Write `text` to `file` through a temporary file beside it, then rename the temporary
/// file onto `file`. `tmp_ext` names the temporary file (`config.toml` with `"toml.tmp"`
/// becomes `config.toml.tmp`), and the parent directory must already exist; each caller
/// creates it with the message that fits its file.
///
/// `cleanup` decides whether a failed write or rename removes the temporary file again:
/// the config editor and the trust records leave nothing behind, while the state file
/// is rewritten whole on every save and a leftover `.new` is harmless.
pub(crate) fn write_via_tmp(
    file: &Path,
    text: &str,
    tmp_ext: &str,
    cleanup: bool,
) -> Result<(), String> {
    let tmp = file.with_extension(tmp_ext);
    std::fs::write(&tmp, text)
        .and_then(|()| std::fs::rename(&tmp, file))
        .map_err(|e| {
            if cleanup {
                let _ = std::fs::remove_file(&tmp);
            }
            format!("cannot write {}: {e}", file.display())
        })
}
