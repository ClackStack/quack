//! Enabling a hook from the UI (`Op::EnableHook`, the `/hooks` modal and
//! `/hooks toggle <id>`).
//!
//! The defaults bundle is never edited: a hook that ships with `enabled = false` is
//! copied into the user layer's `hooks.d/`, its payload folder with it, so an upgrade
//! replaces the shipped copy freely. A hook disabled in its own file in the user or
//! project layer is enabled in place. A bare `enabled = false` override (a file that
//! only switches the hook off) is removed when it is the user's own; anywhere else it
//! names the file to delete.

use std::path::{Path, PathBuf};

use crate::diagnostic::read_text;
use crate::edit::{remove_from_list, write_atomic};
use crate::items::Item;
use crate::load::Loaded;
use crate::paths::LayerKind;
use crate::{HookFile, ItemState, load_hooks};

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
        let Some(item) = load_hooks(self).into_iter().find(|h| h.id == id) else {
            return Err(format!(
                "no hook `{id}` is configured; add it to hooks.d first"
            ));
        };
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
        let file = match file_off(&item.path)? {
            FileOff::On => None,
            FileOff::Flag => Some(self.enable_flag(&item)?),
            FileOff::Bare => Some(self.remove_override(&item)?),
        };
        let unlisted = match self.layer_file(LayerKind::User) {
            Some(config) => remove_from_list(&config, "hooks.disabled", id)?,
            None => false,
        };
        match file {
            Some(outcome) => Ok(HookEnabled {
                unlisted,
                ..outcome
            }),
            None if unlisted => Ok(HookEnabled {
                path: item.path.clone(),
                copied_from: None,
                unlisted,
            }),
            None => Err(format!("hook `{id}` is already enabled")),
        }
    }

    /// A full hook file with `enabled = false`: copy a defaults hook into the user
    /// layer without the flag, or set `enabled = true` in the user's or project's own
    /// file.
    fn enable_flag(&self, item: &Item<HookFile>) -> Result<HookEnabled, String> {
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
                if dest.exists() {
                    return Err(format!(
                        "{} already exists; set `enabled = true` in it",
                        dest.display()
                    ));
                }
                // The payload folder named after the file (`jira.toml` → `jira/`) goes
                // first, so the copied file marks a completed enable.
                if let Some(stem) = dest.file_stem()
                    && let Some(from) = item.path.parent().map(|p| p.join(stem))
                    && from.is_dir()
                {
                    let to = user.join(stem);
                    if to.exists() {
                        std::fs::remove_dir_all(&to)
                            .map_err(|e| format!("cannot replace {}: {e}", to.display()))?;
                    }
                    copy_tree(&from, &to)
                        .map_err(|e| format!("cannot copy {}: {e}", from.display()))?;
                }
                write_atomic(&dest, &out)?;
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

    /// A file that only switches the hook off: removing it is the enable. Only the
    /// user's own override is removed; anywhere else the user is told where it is.
    fn remove_override(&self, item: &Item<HookFile>) -> Result<HookEnabled, String> {
        if item.layer == LayerKind::User {
            std::fs::remove_file(&item.path)
                .map_err(|e| format!("cannot remove {}: {e}", item.path.display()))?;
            return Ok(HookEnabled {
                path: item.path.clone(),
                copied_from: None,
                unlisted: false,
            });
        }
        Err(format!(
            "hook `{}` is switched off by {}, which quack does not edit; \
             delete that file to enable it",
            item.id,
            item.path.display()
        ))
    }
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

    use crate::LoadOptions;
    use crate::{ItemState, LayerKind, load_hooks, load_with};

    struct Fixture {
        _tmp: tempfile::TempDir,
        root: std::path::PathBuf,
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

        fn write(&self, rel: &str, text: &str) {
            let path = self.root.join(rel);
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
        assert_eq!(
            done.copied_from,
            Some(f.root.join("defaults/hooks.d/jira.toml"))
        );
        assert!(!done.unlisted);

        let copy = f.root.join("user/hooks.d/jira.toml");
        let text = std::fs::read_to_string(&copy).expect("copy");
        assert!(text.starts_with("# shipped\n"), "comments kept: {text}");
        assert!(text.contains("enabled = true"), "{text}");
        assert!(!text.contains("enabled = false"), "{text}");
        assert_eq!(
            std::fs::read_to_string(f.root.join("user/hooks.d/jira/run.py")).expect("payload"),
            "print('hi')\n"
        );
        // The shipped files are untouched.
        assert!(
            std::fs::read_to_string(f.root.join("defaults/hooks.d/jira.toml"))
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
        let config = std::fs::read_to_string(f.root.join("user/config.toml")).expect("config");
        assert!(config.contains("disabled = [\"other\"] # mine"), "{config}");
        assert!(!config.contains("jira"), "{config}");
        let copy = std::fs::read_to_string(f.root.join("user/hooks.d/jira.toml")).expect("copy");
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
        assert_eq!(done.path, f.root.join("user/hooks.d/watch.toml"));
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
            std::fs::read_to_string(f.root.join("user/hooks.d/jira.toml")).expect("kept"),
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
        assert_eq!(done.path, f.root.join("user/hooks.d/jira.toml"));
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
