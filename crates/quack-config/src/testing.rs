//! The shared test fixture: one isolated config tree per test, so no test ever reads
//! the developer's real config or keyring (see AGENTS.md). Used by the test modules of
//! `lib.rs`, `load.rs`, `enable.rs`, `mcp_manage.rs` and `edit.rs`.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

use crate::load::{LoadOptions, Loaded, load_with};

/// A config environment in its own temp dir: system, user, data and cache layers, a Git
/// repo for the project layer, and the environment variable map that points quack at
/// them all. The defaults bundle is off (`QUACK_DEFAULTS_DIR=none`), as CI runs;
/// [`Fixture::with_defaults`] puts one at `<root>/defaults`.
pub(crate) struct Fixture {
    /// Keeps the temp dir alive for as long as the fixture lives.
    pub(crate) _tmp: tempfile::TempDir,
    /// Everything the fixture creates lives under here.
    pub(crate) root: PathBuf,
    /// The environment quack sees, as `load_with` takes it.
    pub(crate) vars: HashMap<String, OsString>,
}

impl Fixture {
    /// Layers with no defaults bundle.
    pub(crate) fn new() -> Self {
        let mut f = Self::build();
        f.vars.insert("QUACK_DEFAULTS_DIR".into(), "none".into());
        f
    }

    /// Layers with a defaults bundle at `<root>/defaults`.
    pub(crate) fn with_defaults() -> Self {
        let mut f = Self::build();
        f.vars.insert(
            "QUACK_DEFAULTS_DIR".into(),
            f.root.join("defaults").into_os_string(),
        );
        f
    }

    fn build() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let mut vars = HashMap::new();
        for (k, sub) in [
            ("QUACK_SYSTEM_DIR", "system"),
            ("QUACK_CONFIG_DIR", "user"),
            ("QUACK_DATA_DIR", "data"),
            ("QUACK_CACHE_DIR", "cache"),
        ] {
            vars.insert(k.to_owned(), root.join(sub).into_os_string());
        }
        std::fs::create_dir_all(root.join("repo").join(".git")).expect("mkdir repo root");
        Self {
            _tmp: tmp,
            root,
            vars,
        }
    }

    /// A path under the fixture's root, as `/`-separated segments.
    pub(crate) fn path(&self, rel: &str) -> PathBuf {
        rel.split('/').fold(self.root.clone(), |p, seg| p.join(seg))
    }

    /// Write a file under the fixture's root, creating parent directories as needed.
    pub(crate) fn write(&self, rel: &str, text: &str) {
        let path = self.path(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, text).expect("write");
    }

    /// Load the fixture's config with no command-line options.
    pub(crate) fn load(&self) -> Loaded {
        self.load_opts(|_| {})
    }

    /// Load the fixture's config with command-line options applied (`-m`, `-c`).
    pub(crate) fn load_opts(&self, opts: impl FnOnce(&mut LoadOptions)) -> Loaded {
        let mut o = LoadOptions::new(self.root.join("repo"));
        opts(&mut o);
        let vars = self.vars.clone();
        load_with(&move |k| vars.get(k).cloned(), &o).expect("load")
    }
}
