//! Plain diagnostic printing. miette ships without its `fancy` renderer (18
//! fewer crates, among them the icu segmenter data tables), so errors print as
//! `error:` / `· path:line:col` / `help:` lines instead.

use quack_config::{ConfigError, FileDiagnostic};

pub(crate) fn file(d: &FileDiagnostic) {
    eprintln!("error: {}", d.message);
    eprintln!("  · {}", d.location());
    if let Some(help) = &d.help {
        eprintln!("  help: {help}");
    }
}

pub(crate) fn config(e: &ConfigError) {
    eprintln!("error: {e}");
    if let Some(help) = e.help() {
        eprintln!("  help: {help}");
    }
}
