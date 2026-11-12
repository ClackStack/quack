//! Configuration: per-OS layer directories, `config.toml` layering with policy locks,
//! project trust, and one-file-per-item directories such as `providers.d/`.
//!
//! See docs/plan/01-configuration.md. Precedence, lowest to highest:
//! compiled defaults → defaults bundle → system → user → project (trusted only) → env → CLI,
//! then keys locked by the system layer's `[policy]` are reset to the system value.

use std::path::PathBuf;

pub mod agent;
pub mod bundle;
pub mod catalog;
mod diagnostic;
mod edit;
pub mod enable;
pub mod hook_setup;
pub mod hooks;
mod items;
#[cfg(feature = "keyring")]
mod keyring;
mod list;
mod load;
pub mod lsp;
pub mod mcp;
mod merge;
mod paths;
pub mod profile;
pub mod provider;
pub mod schema;
mod secret;
pub mod settings;
mod sha256;
mod state;
mod trust;

pub use agent::AgentFile;
pub use bundle::{Bundle, BundleFile};
pub use catalog::{Catalog, Catalogs};
pub use diagnostic::FileDiagnostic;
pub use edit::{SettingRow, set_in_file, write_atomic};
pub use enable::{HookEnableOutcome, HookEnabled};
pub use hooks::HookFile;
pub use items::{
    Item, ItemState, glob_match, is_loopback, load_dir_items, load_items, load_items_merging,
    url_host,
};
pub use load::{
    ConfigError, LayerReport, LayerStatus, LoadOptions, Loaded, Origin, ProjectStatus, load,
    load_with,
};
pub use lsp::LspServerFile;
pub use mcp::McpServerFile;
pub use paths::{Dirs, Env, LayerKind, find_workspace_root, user_data_dir};
pub use provider::ProviderFile;
pub use secret::{
    Secret, SecretError, expand_home, keyring_available, keyring_delete, keyring_parts, keyring_set,
};
pub use settings::Settings;
pub use state::{last_model, reasoning_efforts, save_last_model, save_reasoning_efforts};
pub use trust::{ProjectSummary, TrustDecision};

/// Product name, used for directory names. Kept in one place for the eventual rename.
pub const APP_NAME: &str = "quack";
/// Prefix for environment variables.
pub const ENV_PREFIX: &str = "QUACK_";
/// Name of the project-layer directory inside a repository.
pub const PROJECT_DIR_NAME: &str = ".quack";

/// Load `mcp.d/*.toml` from every active layer, merged by id, then the servers the client
/// supplied (`LoadOptions::mcp`). A project's files load only when the project is trusted.
pub fn load_mcp(loaded: &Loaded) -> Vec<Item<McpServerFile>> {
    let mut items = load_items::<McpServerFile>(&loaded.item_layers(), "mcp.d");
    for (id, file) in &loaded.opts.mcp {
        items.retain(|i| i.id != *id);
        items.push(Item {
            id: id.clone(),
            path: PathBuf::from("(from the editor)"),
            layer: LayerKind::User,
            state: ItemState::Loaded(file.clone()),
        });
    }
    items::apply_toggles(
        &mut items,
        &loaded.settings.mcp.disabled,
        &loaded.settings.policy.mcp,
        |f| (!f.url.is_empty()).then_some(f.url.as_str()),
    );
    items
}

/// Load `hooks.d/*.toml` from every active layer, merged by id. A project's files load
/// only when the project is trusted. The system layer's hooks are enforced: a higher
/// layer cannot replace or disable them, by file or with `[hooks] disabled`.
pub fn load_hooks(loaded: &Loaded) -> Vec<Item<HookFile>> {
    let layers = loaded.item_layers();
    let mut items = load_items::<HookFile>(&layers, "hooks.d");
    let system: Vec<_> = layers
        .iter()
        .filter(|(k, _)| *k == LayerKind::System)
        .cloned()
        .collect();
    for enforced in load_items::<HookFile>(&system, "hooks.d") {
        if let Some(slot) = items.iter_mut().find(|i| i.id == enforced.id) {
            *slot = enforced;
        }
    }
    for item in &mut items {
        let listed = loaded.settings.hooks.disabled.contains(&item.id);
        if listed && item.layer != LayerKind::System && !matches!(item.state, ItemState::Failed(_))
        {
            item.state = ItemState::Disabled("listed in `disabled`".into());
        }
    }
    items
}

/// Load `lsp.d/*.toml` from every active layer, merged by id. A project's files load
/// only when the project is trusted.
pub fn load_lsp(loaded: &Loaded) -> Vec<Item<LspServerFile>> {
    let mut items = load_items::<LspServerFile>(&loaded.item_layers(), "lsp.d");
    items::apply_toggles(
        &mut items,
        &loaded.settings.lsp.disabled,
        &settings::AllowList::default(),
        |_| None,
    );
    items
}

/// Load `agents.d/*.toml` from every active layer, merged by id. A project's files load
/// only when the project is trusted.
pub fn load_agents(loaded: &Loaded) -> Vec<Item<AgentFile>> {
    load_items::<AgentFile>(&loaded.item_layers(), "agents.d")
}

/// Load `providers.d/*.toml` from every active layer, merged by id. Provider files are
/// the one item kind that merges field by field across layers, so a user file can hold
/// only what it changes (setup writes exactly that) and the bundled file still supplies
/// the model list beneath it.
pub fn load_providers(loaded: &Loaded) -> Vec<Item<ProviderFile>> {
    let mut items = load_items_merging::<ProviderFile>(&loaded.item_layers(), "providers.d");
    items::apply_toggles(
        &mut items,
        &loaded.settings.providers.disabled,
        &loaded.settings.policy.providers,
        |f| Some(f.base_url.as_str()),
    );
    // A sign-in server is contacted too, so policy must allow its host as well.
    let policy = &loaded.settings.policy.providers;
    for item in &mut items {
        if let ItemState::Loaded(f) = &item.state
            && let Some(auth) = &f.auth
            && !policy.allows(&item.id, Some(&auth.issuer))
        {
            item.state = ItemState::Disabled("not allowed by policy".into());
        }
    }
    items
}

/// A provider file above the defaults bundle that pins its own model list, which differs
/// from the bundled file it overrides. Its list replaces the bundled one whole, so the
/// provider misses the bundle's new models and keeps ones the bundle dropped. Surfaced by
/// `quack doctor` and `/models`; the fix is to edit the file, or remove its `models` list
/// to follow the bundle's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDrift {
    pub id: String,
    /// The file that pins the list.
    pub path: PathBuf,
    /// The layer `path` is in.
    pub layer: LayerKind,
    /// Model ids the bundle lists that the pinned list is missing.
    pub added: Vec<String>,
    /// Model ids the pinned list keeps that the bundle dropped.
    pub removed: Vec<String>,
}

impl ProviderDrift {
    /// One line naming what moved, for `quack doctor` and `/models`.
    pub fn describe(&self) -> String {
        let says = |list: &[String], verb: &str| {
            let names = list.join(", ");
            format!("{verb} {names}")
        };
        let mut parts = Vec::new();
        if !self.added.is_empty() {
            parts.push(says(&self.added, "the bundle adds"));
        }
        if !self.removed.is_empty() {
            parts.push(says(&self.removed, "the bundle drops"));
        }
        parts.join("; ")
    }
}

/// Every provider whose file pins a model list that no longer matches the bundled file it
/// overrides. Files that leave `models` out (the form `quack setup` writes) follow the
/// bundle and never drift.
pub fn provider_drift(loaded: &Loaded) -> Vec<ProviderDrift> {
    let layers = loaded.item_layers();
    let Some((_, bundle)) = layers.iter().find(|(k, _)| *k == LayerKind::Defaults) else {
        return Vec::new();
    };
    let bundle_dir = bundle.join("providers.d");
    let mut out = Vec::new();
    for (layer, dir) in layers.iter().filter(|(k, _)| *k != LayerKind::Defaults) {
        for path in items::toml_files(&dir.join("providers.d")) {
            let ParsedIds { id, models: pinned } = match model_ids(&path) {
                Some(p) => p,
                None => continue,
            };
            if pinned.is_empty() {
                continue;
            }
            let bundled = bundle_dir.join(path.file_name().expect("a file has a name"));
            let Some(bundled) = model_ids(&bundled) else {
                continue;
            };
            if bundled.id != id || bundled.models.is_empty() {
                continue;
            }
            let added = bundled
                .models
                .iter()
                .filter(|m| !pinned.contains(m))
                .cloned()
                .collect::<Vec<_>>();
            let removed = pinned
                .iter()
                .filter(|m| !bundled.models.contains(m))
                .cloned()
                .collect::<Vec<_>>();
            if added.is_empty() && removed.is_empty() {
                continue;
            }
            out.push(ProviderDrift {
                id,
                path,
                layer: *layer,
                added,
                removed,
            });
        }
    }
    out
}

/// A provider file's id and the model ids it declares, or `None` when it does not parse.
struct ParsedIds {
    id: String,
    models: Vec<String>,
}

fn model_ids(path: &std::path::Path) -> Option<ParsedIds> {
    let text = diagnostic::read_text(path).ok()?;
    let table = toml::from_str::<toml::Table>(&text).ok()?;
    let id = table.get("id").and_then(toml::Value::as_str).map_or_else(
        || {
            path.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        },
        str::to_owned,
    );
    let models = table
        .get("models")
        .and_then(toml::Value::as_array)?
        .iter()
        .filter_map(|m| match m {
            toml::Value::String(s) => Some(s.clone()),
            toml::Value::Table(t) => t.get("id").and_then(toml::Value::as_str).map(str::to_owned),
            _ => None,
        })
        .collect();
    Some(ParsedIds { id, models })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::OsString;

    struct Fixture {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        vars: HashMap<String, OsString>,
    }

    impl Fixture {
        fn new() -> Self {
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
            vars.insert(
                "QUACK_DEFAULTS_DIR".into(),
                root.join("defaults").into_os_string(),
            );
            std::fs::create_dir_all(root.join("repo").join(".git")).expect("mkdir");
            Self {
                _tmp: tmp,
                root,
                vars,
            }
        }

        fn path(&self, rel: &str) -> PathBuf {
            rel.split('/').fold(self.root.clone(), |p, seg| p.join(seg))
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.path(rel);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, text).expect("write");
        }

        fn load(&self) -> Loaded {
            let o = LoadOptions::new(self.root.join("repo"));
            let vars = self.vars.clone();
            load_with(&move |k| vars.get(k).cloned(), &o).expect("load")
        }
    }

    #[test]
    fn a_pinned_model_list_that_moved_on_is_drift() {
        let f = Fixture::new();
        f.write(
            "defaults/providers.d/charm.toml",
            "base_url = \"https://hyper.example/v1\"\n\
             api_key = { env = \"CHARM_API_KEY\" }\n\
             [[models]]\nid = \"glm-5.3\"\n[[models]]\nid = \"kimi-k3\"\n",
        );
        // A full copy from an older bundle: one model gone stale, one new one missing.
        f.write(
            "user/providers.d/charm.toml",
            "base_url = \"https://hyper.example/v1\"\n\
             api_key = { keyring = \"charm\" }\n\
             [[models]]\nid = \"glm-5.3\"\n[[models]]\nid = \"glm-4.7\"\n",
        );
        // The form setup writes now: no model list, so nothing to drift.
        f.write(
            "user/providers.d/plain.toml",
            "base_url = \"https://api.example.org/v1\"\n",
        );
        f.write(
            "defaults/providers.d/plain.toml",
            "base_url = \"https://api.example.org/v1\"\nmodels = [\"a\"]\n",
        );

        let drift = provider_drift(&f.load());
        assert_eq!(drift.len(), 1, "{drift:?}");
        assert_eq!(drift[0].id, "charm");
        assert_eq!(drift[0].layer, LayerKind::User);
        assert_eq!(drift[0].path, f.path("user/providers.d/charm.toml"));
        assert_eq!(drift[0].added, ["kimi-k3"]);
        assert_eq!(drift[0].removed, ["glm-4.7"]);
        assert_eq!(
            drift[0].describe(),
            "the bundle adds kimi-k3; the bundle drops glm-4.7"
        );
    }

    #[test]
    fn no_bundle_or_matching_list_is_no_drift() {
        let f = Fixture::new();
        f.write(
            "user/providers.d/own.toml",
            "base_url = \"https://api.example.org/v1\"\nmodels = [\"a\"]\n",
        );
        // Same list in both layers: pinned, but nothing moved.
        f.write(
            "defaults/providers.d/same.toml",
            "base_url = \"https://api.example.org/v1\"\nmodels = [\"a\"]\n",
        );
        f.write(
            "user/providers.d/same.toml",
            "base_url = \"https://api.example.org/v1\"\nmodels = [\"a\"]\n",
        );
        assert!(provider_drift(&f.load()).is_empty());

        let vars: HashMap<String, OsString> = f
            .vars
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    if k == "QUACK_DEFAULTS_DIR" {
                        "none".into()
                    } else {
                        v.clone()
                    },
                )
            })
            .collect();
        let o = LoadOptions::new(f.root.join("repo"));
        let without_bundle = load_with(&move |k| vars.get(k).cloned(), &o).expect("load");
        assert!(provider_drift(&without_bundle).is_empty());
    }
}
