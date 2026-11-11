//! Enabling a hook from the UI (`Op::EnableHook`, the `/hooks` modal and
//! `/hooks toggle <id>`).
//!
//! The defaults bundle is never edited: a hook that ships with `enabled = false` is
//! copied into the user layer's `hooks.d/`, its payload folder with it, so an upgrade
//! replaces the shipped copy freely. A hook disabled in its own file in the user or
//! project layer is enabled in place. A bare `enabled = false` override (a file that
//! only switches the hook off) is removed in the user or trusted project layer.

use std::path::{Path, PathBuf};

use crate::diagnostic::read_text;
use crate::edit::{remove_from_list, write_atomic};
use crate::items::{Item, load_items_except};
use crate::load::{LayerStatus, Loaded, Origin, ProjectStatus};
use crate::paths::LayerKind;
use crate::{HookFile, ItemState, TrustDecision, load_hooks};

/// A completed enable or a payload decision required before writing any files.
#[derive(Debug)]
pub enum HookEnableOutcome {
    Enabled(HookEnabled),
    ChoosePayload(PathBuf),
}

/// What enabling a hook changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookEnabled {
    /// The hook's file afterwards: the user-layer copy, the file enabled in place, or
    /// the bare `enabled = false` override that was removed.
    pub path: PathBuf,
    /// Set when the file was copied from the defaults bundle, its payload folder with it.
    pub copied_from: Option<PathBuf>,
    /// Set when `[hooks] disabled` in the user's `config.toml` no longer lists the hook.
    pub unlisted: bool,
}

impl Loaded {
    /// Make hook `id` run: a hook shipped disabled is copied into the user layer with
    /// `enabled = false` removed, one in the user or project layer is enabled in its own
    /// file, and `[hooks] disabled` in the user's `config.toml` stops listing it. Takes
    /// effect from the next load. Refuses what it cannot or should not do, with a
    /// message that says what to do instead.
    pub fn enable_hook(&self, id: &str) -> Result<HookEnabled, String> {
        match self.enable_hook_with_payload(id, None)? {
            HookEnableOutcome::Enabled(outcome) => Ok(outcome),
            HookEnableOutcome::ChoosePayload(path) => Err(format!(
                "choose whether to keep or replace {}",
                path.display()
            )),
        }
    }

    /// Enable the hook, asking before replacing an existing payload.
    pub fn enable_hook_with_payload(
        &self,
        id: &str,
        replace_payload: Option<bool>,
    ) -> Result<HookEnableOutcome, String> {
        const KEY: &str = "hooks.disabled";
        let mut item = load_hooks(self)
            .into_iter()
            .find(|h| h.id == id)
            .ok_or_else(|| format!("no hook `{id}` is configured; add it to hooks.d first"))?;
        let mut overrides = Vec::new();
        let off = loop {
            if item.layer == LayerKind::System {
                return Err(format!(
                    "hook `{id}` is enforced by the system layer and cannot be changed"
                ));
            }
            if let ItemState::Failed(d) = &item.state {
                return Err(format!(
                    "hook `{id}` does not load: {}; fix its file first",
                    d.one_line()
                ));
            }
            let off = file_off(&item.path)?;
            if !matches!(off, FileOff::Bare) {
                break off;
            }
            if !matches!(item.layer, LayerKind::User | LayerKind::Project) {
                return Err(format!(
                    "hook `{id}` is switched off by {}; edit that file to enable it",
                    item.path.display()
                ));
            }
            overrides.push(item.path);
            item = load_items_except::<HookFile>(&self.item_layers(), "hooks.d", &overrides)
                .into_iter()
                .find(|h| h.id == id)
                .ok_or_else(|| {
                    format!("no hook definition remains after removing the overrides for `{id}`")
                })?;
        };

        if matches!(off, FileOff::Flag)
            && item.layer == LayerKind::Defaults
            && let Some((from, to)) = self.payload_dirs(&item)
            && from.is_dir()
            && to.symlink_metadata().is_ok()
            && replace_payload.is_none()
        {
            return Ok(HookEnableOutcome::ChoosePayload(to));
        }

        if self.settings.hooks.disabled.iter().any(|d| d == id) {
            if self.settings.policy.locks(KEY) {
                return Err(format!("`{KEY}` is locked by the system policy"));
            }
            if matches!(self.origin_of(KEY), Origin::Cli | Origin::Env(_)) {
                return Err(format!(
                    "hook `{id}` is disabled by {}; change that override first",
                    self.origin_of(KEY)
                ));
            }
        }

        let file = match off {
            FileOff::Flag => Some(self.enable_flag(&item, &overrides, replace_payload)?),
            _ => None,
        };
        for path in &overrides {
            if file.as_ref().is_none_or(|f| f.path != *path) {
                std::fs::remove_file(path)
                    .map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
            }
        }
        let mut unlisted = false;
        let mut project_changed = item.layer == LayerKind::Project && file.is_some()
            || overrides.iter().any(|p| p.starts_with(&self.dirs.project));
        if !self.settings.policy.locks(KEY) {
            for layer in &self.layers {
                if matches!(layer.kind, LayerKind::User | LayerKind::Project)
                    && layer.status == LayerStatus::Loaded
                    && let Some(dir) = &layer.dir
                    && remove_from_list(&dir.join("config.toml"), KEY, id)?
                {
                    unlisted = true;
                    project_changed |= layer.kind == LayerKind::Project;
                }
            }
        }
        if project_changed {
            self.refresh_hook_project_trust()?;
        }

        // Mask an inherited entry in the user layer. If the project appends other
        // entries, leave those in the project so subsequent changes still take effect.
        let fresh = self.reload().map_err(|e| e.to_string())?;
        if fresh.settings.hooks.disabled.iter().any(|d| d == id) {
            let mut inherited = fresh.settings.hooks.disabled.as_slice();
            if let Origin::File {
                layer: LayerKind::Project,
                path,
            } = fresh.origin_of(KEY)
            {
                let text =
                    read_text(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                let table: toml::Table = toml::from_str(&text).map_err(|e| e.to_string())?;
                let appended = table
                    .get("hooks")
                    .and_then(toml::Value::as_table)
                    .and_then(|hooks| hooks.get("disabled"))
                    .and_then(toml::Value::as_table)
                    .and_then(|disabled| disabled.get("append"))
                    .and_then(toml::Value::as_array)
                    .ok_or_else(|| {
                        format!("hook `{id}` is still disabled in {}", path.display())
                    })?;
                inherited = &inherited[..inherited.len().saturating_sub(appended.len())];
            }
            let remaining = toml::Value::Array(
                inherited
                    .iter()
                    .filter(|d| *d != id)
                    .cloned()
                    .map(toml::Value::String)
                    .collect(),
            );
            fresh.save_setting(LayerKind::User, KEY, &remaining.to_string())?;
            unlisted = true;
        }
        let file = file.or_else(|| {
            overrides.first().map(|path| HookEnabled {
                path: path.clone(),
                copied_from: None,
                unlisted: false,
            })
        });
        Ok(HookEnableOutcome::Enabled(match file {
            Some(outcome) => HookEnabled {
                unlisted,
                ..outcome
            },
            None if unlisted => HookEnabled {
                path: item.path.clone(),
                copied_from: None,
                unlisted,
            },
            None => return Err(format!("hook `{id}` is already enabled")),
        }))
    }

    fn refresh_hook_project_trust(&self) -> Result<(), String> {
        let decision = match self.project {
            ProjectStatus::Trusted => TrustDecision::Trusted,
            ProjectStatus::SettingsOnly => TrustDecision::SettingsOnly,
            _ => return Err("project config is no longer trusted".into()),
        };
        self.trust_project(decision).map(|_| ())
    }

    /// A full hook file with `enabled = false`: copy a defaults hook into the user
    /// layer without the flag, or set `enabled = true` in the user's or project's own
    /// file.
    fn enable_flag(
        &self,
        item: &Item<HookFile>,
        overrides: &[PathBuf],
        replace_payload: Option<bool>,
    ) -> Result<HookEnabled, String> {
        let text = read_text(&item.path)
            .map_err(|e| format!("cannot read {}: {e}", item.path.display()))?;
        let mut doc: toml_edit::DocumentMut = text
            .parse()
            .map_err(|e: toml_edit::TomlError| format!("not valid TOML: {}", e.message()))?;
        // The flag flips to `true` in place rather than being deleted, so the comments
        // around it (decor attached to the key) survive.
        let Some(toml_edit::Item::Value(old)) = doc.as_table_mut().get_mut("enabled") else {
            return Err(format!(
                "{} has no `enabled` key; it should not look disabled",
                item.path.display()
            ));
        };
        let decor = old.decor().clone();
        *old = "true"
            .parse()
            .map_err(|e: toml_edit::TomlError| e.to_string())?;
        *old.decor_mut() = decor;
        let out = doc.to_string();
        toml::from_str::<HookFile>(&out)
            .map_err(|e| format!("{} does not load once enabled: {e}", item.path.display()))?;
        match item.layer {
            LayerKind::Defaults => {
                let user = self.dirs.user_config.join("hooks.d");
                let name = item
                    .path
                    .file_name()
                    .ok_or_else(|| format!("{} has no file name", item.path.display()))?;
                let dest = user.join(name);
                if dest.exists() && !overrides.contains(&dest) {
                    return Err(format!(
                        "{} already exists; set `enabled = true` in it",
                        dest.display()
                    ));
                }
                if let Some((from, to)) = self.payload_dirs(item)
                    && from.is_dir()
                {
                    install_payload(&from, &to, replace_payload, || write_atomic(&dest, &out))?;
                } else {
                    write_atomic(&dest, &out)?;
                }
                Ok(HookEnabled {
                    path: dest,
                    copied_from: Some(item.path.clone()),
                    unlisted: false,
                })
            }
            _ => {
                write_atomic(&item.path, &out)?;
                Ok(HookEnabled {
                    path: item.path.clone(),
                    copied_from: None,
                    unlisted: false,
                })
            }
        }
    }

    fn payload_dirs(&self, item: &Item<HookFile>) -> Option<(PathBuf, PathBuf)> {
        let stem = item.path.file_stem()?;
        Some((
            item.path.parent()?.join(stem),
            self.dirs.user_config.join("hooks.d").join(stem),
        ))
    }
}

/// Stage the defaults and keep the old payload available until the hook file is saved.
pub(crate) fn install_payload(
    from: &Path,
    to: &Path,
    replace: Option<bool>,
    save: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let existing = match to.symlink_metadata() {
        Ok(_) if replace == Some(false) => return save(),
        Ok(_) if replace == Some(true) => true,
        Ok(_) => {
            return Err(format!(
                "choose whether to keep or replace {}",
                to.display()
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(format!("cannot inspect {}: {e}", to.display())),
    };
    let parent = to.parent().expect("payload is under hooks.d");
    let error = |e| format!("cannot install {}: {e}", to.display());
    std::fs::create_dir_all(parent).map_err(error)?;
    let stage = tempfile::tempdir_in(parent).map_err(error)?;
    let new = stage.path().join("new");
    let old = stage.path().join("old");
    copy_tree(from, &new).map_err(error)?;
    if existing {
        std::fs::rename(to, &old).map_err(error)?;
    }
    let result = std::fs::rename(&new, to).map_err(error).and_then(|()| {
        save().inspect_err(|_| {
            let _ = std::fs::rename(to, &new);
        })
    });
    if result.is_err()
        && existing
        && let Err(e) = std::fs::rename(&old, to)
    {
        let recovery = stage.keep();
        return Err(format!(
            "cannot restore {}: {e}; previous payload is at {}",
            to.display(),
            recovery.join("old").display()
        ));
    }
    result
}

/// The `enabled` flag of a hook's file, read fresh from disk: the item state can hide
/// it, because `[hooks] disabled` overwrites the state of a listed hook.
enum FileOff {
    On,
    /// A full hook file with `enabled = false`.
    Flag,
    /// A file whose only job is to switch the hook off (`enabled = false` and `id`).
    Bare,
}

fn file_off(path: &Path) -> Result<FileOff, String> {
    let text = read_text(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let table: toml::Table =
        toml::from_str(&text).map_err(|e| format!("{} is not valid TOML: {e}", path.display()))?;
    if table.get("enabled").and_then(toml::Value::as_bool) != Some(false) {
        return Ok(FileOff::On);
    }
    if table.keys().all(|k| k == "enabled" || k == "id") {
        return Ok(FileOff::Bare);
    }
    toml::from_str::<HookFile>(&text)
        .map_err(|e| format!("{} does not load: {e}", path.display()))?;
    Ok(FileOff::Flag)
}

/// Copy a folder's files, recursively; a missing folder copies nothing.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(from) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    std::fs::create_dir_all(to)?;
    for e in entries {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_tree(&e.path(), &target)?;
        } else {
            std::fs::copy(e.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::path::PathBuf;

    use crate::LoadOptions;
    use crate::{ItemState, LayerKind, load_hooks, load_with};

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

        fn load(&self) -> crate::Loaded {
            let o = LoadOptions::new(self.root.join("repo"));
            let vars = self.vars.clone();
            load_with(&move |k| vars.get(k).cloned(), &o).expect("load")
        }
    }

    const HOOK: &str = "event = \"user_prompt\"\ndescription = \"test hook\"\n\
        command = [\"python3\", \"{dir}/jira/run.py\"]\n";

    #[test]
    fn a_defaults_hook_is_copied_with_its_payload_and_enabled() {
        let f = Fixture::new();
        f.write(
            "defaults/hooks.d/jira.toml",
            &format!("# shipped\nenabled = false\n{HOOK}"),
        );
        f.write("defaults/hooks.d/jira/run.py", "print('hi')\n");
        let loaded = f.load();

        let done = loaded.enable_hook("jira").expect("enable");
        assert_eq!(done.copied_from, Some(f.path("defaults/hooks.d/jira.toml")));
        assert!(!done.unlisted);

        let copy = f.path("user/hooks.d/jira.toml");
        let text = std::fs::read_to_string(&copy).expect("copy");
        assert!(text.starts_with("# shipped\n"), "comments kept: {text}");
        assert!(text.contains("enabled = true"), "{text}");
        assert!(!text.contains("enabled = false"), "{text}");
        assert_eq!(
            std::fs::read_to_string(f.path("user/hooks.d/jira/run.py")).expect("payload"),
            "print('hi')\n"
        );
        // The shipped files are untouched.
        assert!(
            std::fs::read_to_string(f.path("defaults/hooks.d/jira.toml"))
                .expect("shipped")
                .contains("enabled = false")
        );

        // And the copy is what loads now.
        let reloaded = f.load();
        let hooks = load_hooks(&reloaded);
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].layer, LayerKind::User);
        assert!(matches!(&hooks[0].state, ItemState::Loaded(_)));
    }

    #[test]
    fn an_existing_payload_requires_a_choice() {
        for replace in [false, true] {
            let f = Fixture::new();
            f.write(
                "defaults/hooks.d/jira.toml",
                &format!("enabled = false\n{HOOK}"),
            );
            f.write("defaults/hooks.d/jira/run.py", "bundled");
            f.write("user/hooks.d/jira/run.py", "custom");
            f.write("user/hooks.d/jira/extra", "custom extra");
            // A bare override must survive until the choice is made.
            f.write("user/hooks.d/jira.toml", "enabled = false\n");
            let loaded = f.load();
            let folder = f.path("user/hooks.d/jira");
            assert!(matches!(
                loaded.enable_hook_with_payload("jira", None).expect("choice"),
                super::HookEnableOutcome::ChoosePayload(path) if path == folder
            ));
            assert_eq!(
                std::fs::read_to_string(folder.with_extension("toml")).expect("override"),
                "enabled = false\n"
            );
            assert_eq!(
                std::fs::read_to_string(folder.join("run.py")).expect("custom"),
                "custom"
            );

            assert!(matches!(
                loaded
                    .enable_hook_with_payload("jira", Some(replace))
                    .expect("enable"),
                super::HookEnableOutcome::Enabled(_)
            ));
            assert_eq!(
                std::fs::read_to_string(folder.join("run.py")).expect("payload"),
                if replace { "bundled" } else { "custom" }
            );
            assert_eq!(folder.join("extra").exists(), !replace);
            assert!(load_hooks(&f.load())[0].loaded().is_some());
        }
    }

    #[test]
    fn enabling_clears_the_active_lists_and_preserves_other_entries() {
        let f = Fixture::new();
        f.write(
            "defaults/hooks.d/jira.toml",
            &format!("enabled = false\n{HOOK}"),
        );
        f.write(
            "system/config.toml",
            "[hooks]\ndisabled = [\"jira\", \"system-other\"]\n",
        );
        f.write(
            "user/config.toml",
            "[hooks]\ndisabled.append = [\"jira\", \"user-other\"]\n",
        );
        f.write(
            "repo/.quack/config.toml",
            "[hooks]\ndisabled.append = [\"jira\", \"project-other\"]\n",
        );
        f.load()
            .trust_project(crate::TrustDecision::Trusted)
            .expect("trust");
        f.load().enable_hook("jira").expect("enable");

        let loaded = f.load();
        assert_eq!(loaded.project, crate::ProjectStatus::Trusted);
        assert_eq!(
            loaded.settings.hooks.disabled,
            ["system-other", "user-other", "project-other"]
        );
        assert!(load_hooks(&loaded)[0].loaded().is_some());
        let project = std::fs::read_to_string(f.path("repo/.quack/config.toml")).expect("project");
        let project: toml::Table = toml::from_str(&project).expect("parse project");
        assert_eq!(
            project["hooks"]["disabled"]["append"]
                .as_array()
                .expect("append"),
            &[toml::Value::String("project-other".into())],
        );

        f.write("user/config.toml", "[hooks]\ndisabled = [\"jira\"]\n");
        assert!(matches!(
            load_hooks(&f.load())[0].state,
            ItemState::Disabled(_)
        ));
    }

    #[test]
    fn a_listed_and_file_disabled_hook_is_fixed_twice_over() {
        let f = Fixture::new();
        f.write(
            "defaults/hooks.d/jira.toml",
            &format!("enabled = false\n{HOOK}"),
        );
        f.write(
            "user/config.toml",
            "[hooks]\ndisabled = [\"other\", \"jira\"] # mine\n",
        );
        let loaded = f.load();

        let done = loaded.enable_hook("jira").expect("enable");
        assert!(done.unlisted);
        let config = std::fs::read_to_string(f.path("user/config.toml")).expect("config");
        assert!(config.contains("disabled = [\"other\"] # mine"), "{config}");
        assert!(!config.contains("jira"), "{config}");
        let copy = std::fs::read_to_string(f.path("user/hooks.d/jira.toml")).expect("copy");
        assert!(!copy.contains("enabled = false"), "{copy}");
    }

    #[test]
    fn a_user_hook_is_enabled_in_its_own_file() {
        let f = Fixture::new();
        f.write(
            "user/hooks.d/watch.toml",
            &format!("enabled = false # off for now\n{HOOK}"),
        );
        let loaded = f.load();

        let done = loaded.enable_hook("watch").expect("enable");
        assert_eq!(done.path, f.path("user/hooks.d/watch.toml"));
        assert_eq!(done.copied_from, None);
        let text = std::fs::read_to_string(&done.path).expect("file");
        assert!(text.contains("enabled = true # off for now"), "{text}");
    }

    #[test]
    fn an_existing_user_copy_is_not_clobbered() {
        let f = Fixture::new();
        f.write(
            "defaults/hooks.d/jira.toml",
            &format!("enabled = false\n{HOOK}"),
        );
        // A different hook that happens to share the file name.
        f.write(
            "user/hooks.d/jira.toml",
            "id = \"local-jira\"\nevent = \"turn_end\"\ncommand = [\"x\"]\n",
        );
        let loaded = f.load();
        let err = loaded.enable_hook("jira").expect_err("refused");
        assert!(err.contains("already exists"), "{err}");
        assert_eq!(
            std::fs::read_to_string(f.path("user/hooks.d/jira.toml")).expect("kept"),
            "id = \"local-jira\"\nevent = \"turn_end\"\ncommand = [\"x\"]\n"
        );
    }

    #[test]
    fn a_bare_override_in_the_user_layer_is_removed() {
        let f = Fixture::new();
        f.write("defaults/hooks.d/jira.toml", HOOK);
        f.write("user/hooks.d/jira.toml", "enabled = false\n");
        let loaded = f.load();

        let done = loaded.enable_hook("jira").expect("enable");
        assert_eq!(done.path, f.path("user/hooks.d/jira.toml"));
        assert!(!done.path.exists());
    }

    #[test]
    fn a_bare_override_in_the_defaults_bundle_is_named_not_touched() {
        let f = Fixture::new();
        f.write("defaults/hooks.d/jira.toml", "enabled = false\n");
        let loaded = f.load();
        let err = loaded.enable_hook("jira").expect_err("refused");
        assert!(err.contains("defaults"), "{err}");
    }

    #[test]
    fn a_system_hook_is_refused() {
        let f = Fixture::new();
        f.write("system/hooks.d/jira.toml", HOOK);
        let loaded = f.load();
        let err = loaded.enable_hook("jira").expect_err("refused");
        assert!(err.contains("system layer"), "{err}");
    }

    #[test]
    fn an_unknown_hook_is_refused() {
        let f = Fixture::new();
        let loaded = f.load();
        let err = loaded.enable_hook("jira").expect_err("refused");
        assert!(err.contains("no hook"), "{err}");
    }

    #[test]
    fn an_enabled_hook_says_so() {
        let f = Fixture::new();
        f.write("user/hooks.d/jira.toml", HOOK);
        let loaded = f.load();
        let err = loaded.enable_hook("jira").expect_err("refused");
        assert!(err.contains("already enabled"), "{err}");
    }
}
