//! Writing settings back to a layer's `config.toml` (`/config`, "save as default" in `/models`).
//!
//! Edits keep the file's comments and layout, are validated against the schema before
//! anything is written, and replace the file atomically.

use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use toml::Value;

use crate::load::{Loaded, Origin, ProjectStatus, parse_override};
use crate::merge;
use crate::paths::LayerKind;
use crate::settings::Settings;
use crate::trust::{self, TrustDecision};

/// Which family of the settings tree a key belongs to. `[policy]` is only honoured in
/// the system layer and `[keys]` holds key rebinds, so neither is a flat setting: the
/// `/config` editor leaves both out and [`Loaded::save_setting`] refuses them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyFamily {
    /// `[policy]`: only read from the system layer.
    Policy,
    /// `[keys]`: key rebinds, edited as their own thing.
    Keys,
    /// An ordinary setting.
    Setting,
}

impl KeyFamily {
    /// The one place a key's family is decided.
    fn of(key: &str) -> Self {
        if key == "policy" || key.starts_with("policy.") {
            Self::Policy
        } else if key == "keys" || key.starts_with("keys.") {
            Self::Keys
        } else {
            Self::Setting
        }
    }
}

/// One effective setting, for the `/config` editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingRow {
    pub key: String,
    /// The effective value as TOML.
    pub value: String,
    pub origin: String,
    pub description: String,
    pub is_bool: bool,
    /// Locked by the system policy; cannot be changed here.
    pub locked: bool,
}

impl Loaded {
    /// Path of a layer's `config.toml`, if the layer has a directory.
    pub fn layer_file(&self, layer: LayerKind) -> Option<PathBuf> {
        self.dirs.layer_dir(layer).map(|d| d.join("config.toml"))
    }

    /// Every editable leaf setting with its value, origin and schema description.
    pub fn setting_rows(&self) -> Vec<SettingRow> {
        let table = toml::Table::try_from(&self.settings).unwrap_or_default();
        let docs = descriptions();
        merge::leaves(&table)
            .into_iter()
            .filter(|(k, _)| KeyFamily::of(k) == KeyFamily::Setting)
            .map(|(key, v)| SettingRow {
                value: v.to_string(),
                origin: self.origin_of(&key).to_string(),
                description: docs
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, d)| d.clone())
                    .unwrap_or_default(),
                is_bool: v.is_bool(),
                locked: self.settings.policy.locks(&key),
                key,
            })
            .collect()
    }

    /// Set `key` in the given layer's `config.toml`. `raw` parses as a TOML value and falls
    /// back to a string, like `-c key=value`. Returns the file written and, when a higher
    /// layer, the environment or the command line still wins, a note saying so.
    pub fn save_setting(
        &self,
        layer: LayerKind,
        key: &str,
        raw: &str,
    ) -> Result<(PathBuf, Option<String>), String> {
        if KeyFamily::of(key) != KeyFamily::Setting {
            return Err(format!("`{key}` cannot be set as a flat setting"));
        }
        if self.settings.policy.locks(key) {
            return Err(format!("`{key}` is locked by the system policy"));
        }
        if layer == LayerKind::Project && self.project == ProjectStatus::DisabledByPolicy {
            return Err("project config is disabled by the system policy".into());
        }
        let file = self
            .layer_file(layer)
            .ok_or_else(|| format!("the {layer} layer has no directory"))?;
        let in_project = self.active_project_trust(layer);
        if raw.trim() == "null" {
            if !is_known_setting_key(key) {
                return Err(format!("unknown setting `{key}`"));
            }
            remove_in_file(&file, key)?;
        } else {
            let value = parse_override(raw.trim());
            set_in_file(&file, key, &value)?;
        }
        if let Some(decision) = in_project {
            self.trust_project(decision)?;
        } else if layer == LayerKind::Project
            && self.project == ProjectStatus::None
            && self.dirs.project.is_dir()
        {
            // Newly created by the user for settings only; leave extensions unapproved until trusted.
            self.trust_project(TrustDecision::SettingsOnly)?;
        }
        let note = match self.origin_of(key) {
            Origin::File { layer: l, path } if *l > layer => Some(format!(
                "still overridden by the {l} layer ({})",
                path.display()
            )),
            Origin::Env(var) => Some(format!("still overridden by {var}")),
            Origin::Cli => Some("overridden for this run by a command-line flag".into()),
            _ if layer == LayerKind::Project
                && raw.trim() != "null"
                && matches!(
                    self.project,
                    ProjectStatus::Untrusted | ProjectStatus::Changed | ProjectStatus::Ignored
                ) =>
            {
                Some("project config is not loaded until trusted".into())
            }
            _ => None,
        };
        Ok((file, note))
    }
}

impl Loaded {
    pub(crate) fn active_project_trust(&self, layer: LayerKind) -> Option<TrustDecision> {
        if layer != LayerKind::Project {
            return None;
        }
        let decision = match self.project {
            ProjectStatus::Trusted => TrustDecision::Trusted,
            ProjectStatus::SettingsOnly => TrustDecision::SettingsOnly,
            _ => return None,
        };
        trust::lookup(
            &self.dirs.user_data,
            &self.dirs.workspace_root,
            &self.dirs.project,
        )
        .is_some_and(|f| f.current)
        .then_some(decision)
    }

    /// Add "always allow" rules to `[permissions] allow`: in the project's
    /// `.quack/config.toml` when `project` is asked for and that file is loaded (its trust
    /// is recorded again, so the edit doesn't make the project untrusted), else in the
    /// user's. Returns the file written.
    pub fn save_allow_rules(&self, project: bool, rules: &[String]) -> Result<PathBuf, String> {
        if self.settings.policy.locks("permissions.allow") {
            return Err("`permissions.allow` is locked by the system policy".into());
        }
        // Only while the files still match the trust given to them.
        let in_project = if project {
            self.active_project_trust(LayerKind::Project)
        } else {
            None
        };
        let layer = if in_project.is_some() {
            LayerKind::Project
        } else {
            LayerKind::User
        };
        let file = self
            .layer_file(layer)
            .ok_or_else(|| format!("the {layer} layer has no directory"))?;
        add_to_list(&file, "permissions.allow", rules)?;
        if let Some(decision) = in_project {
            self.trust_project(decision)?;
        }
        Ok(file)
    }
}

/// Add items to a list setting in a `config.toml`, skipping ones already there. A key
/// that isn't set yet is written as `{ append = [...] }`, so the lists of lower layers
/// still apply.
pub fn add_to_list(file: &Path, key: &str, items: &[String]) -> Result<(), String> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
    };
    let updated =
        add_to_list_text(&text, key, items).map_err(|e| format!("{}: {e}", file.display()))?;
    write_atomic(file, &updated)
}

pub(crate) fn add_to_list_text(text: &str, key: &str, items: &[String]) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| format!("not valid TOML: {}", e.message()))?;
    let (table, leaf) = table_for(&mut doc, key)?;
    let array = match table.get_mut(leaf) {
        None => {
            let mut append = toml_edit::InlineTable::new();
            append.insert("append", toml_edit::Value::Array(toml_edit::Array::new()));
            table.insert(leaf, toml_edit::value(append));
            table
                .get_mut(leaf)
                .and_then(|i| i.as_inline_table_mut())
                .and_then(|t| t.get_mut("append"))
                .and_then(toml_edit::Value::as_array_mut)
        }
        Some(item) => match item.as_value_mut() {
            Some(toml_edit::Value::Array(a)) => Some(a),
            Some(toml_edit::Value::InlineTable(t)) => {
                t.get_mut("append").and_then(toml_edit::Value::as_array_mut)
            }
            _ => item
                .as_table_mut()
                .and_then(|t| t.get_mut("append"))
                .and_then(|i| i.as_array_mut()),
        },
    }
    .ok_or_else(|| format!("`{key}` is not a list"))?;
    for item in items {
        if !array.iter().any(|v| v.as_str() == Some(item.as_str())) {
            array.push(item.as_str());
        }
    }
    let out = doc.to_string();
    toml::from_str::<Settings>(&out).map_err(|e| e.message().to_owned())?;
    Ok(out)
}

/// Remove one entry from a list setting in a `config.toml`. Returns whether the file
/// changed; a missing file, list or entry changes nothing. Empty replacement lists
/// continue masking lower layers.
pub fn remove_from_list(file: &Path, key: &str, item: &str) -> Result<bool, String> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
    };
    match remove_from_list_text(&text, key, item).map_err(|e| format!("{}: {e}", file.display()))? {
        None => Ok(false),
        Some(updated) => {
            write_atomic(file, &updated)?;
            Ok(true)
        }
    }
}

pub(crate) fn remove_from_list_text(
    text: &str,
    key: &str,
    item: &str,
) -> Result<Option<String>, String> {
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| format!("not valid TOML: {}", e.message()))?;
    let Some((table, leaf)) = existing_table(&mut doc, key) else {
        return Ok(None);
    };
    let array = match table.get_mut(leaf) {
        Some(toml_edit::Item::Value(value)) => match value {
            toml_edit::Value::Array(a) => Some(a),
            toml_edit::Value::InlineTable(t) => {
                t.get_mut("append").and_then(toml_edit::Value::as_array_mut)
            }
            _ => None,
        },
        Some(toml_edit::Item::Table(t)) => t
            .get_mut("append")
            .and_then(toml_edit::Item::as_value_mut)
            .and_then(toml_edit::Value::as_array_mut),
        _ => None,
    };
    let Some(array) = array else {
        return Ok(None);
    };
    let before = array.len();
    array.retain(|v| v.as_str() != Some(item));
    if array.len() == before {
        return Ok(None);
    }
    let out = doc.to_string();
    toml::from_str::<Settings>(&out).map_err(|e| e.message().to_owned())?;
    Ok(Some(out))
}

/// The table holding a dotted key's leaf, when it and its parents exist, and the leaf's
/// name. Unlike [`table_for`], nothing is created.
fn existing_table<'a, 'k>(
    doc: &'a mut toml_edit::DocumentMut,
    key: &'k str,
) -> Option<(&'a mut toml_edit::Table, &'k str)> {
    let parts: Vec<&str> = key.split('.').collect();
    let (leaf, parents) = parts.split_last()?;
    let mut table = doc.as_table_mut();
    for p in parents {
        table = table.get_mut(p)?.as_table_mut()?;
    }
    Some((table, leaf))
}

/// The table holding a dotted key's leaf, created as needed, and the leaf's name.
fn table_for<'a, 'k>(
    doc: &'a mut toml_edit::DocumentMut,
    key: &'k str,
) -> Result<(&'a mut toml_edit::Table, &'k str), String> {
    let parts: Vec<&str> = key.split('.').collect();
    let Some((leaf, parents)) = parts.split_last() else {
        return Err("empty key".into());
    };
    let mut table = doc.as_table_mut();
    for p in parents {
        let entry = table
            .entry(p)
            .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));
        table = match entry {
            toml_edit::Item::Table(t) => t,
            _ => return Err(format!("`{p}` in `{key}` is not a table")),
        };
    }
    Ok((table, leaf))
}

/// Replace `file` with `text` atomically: a temporary file beside it, then a rename,
/// creating parent directories as needed. See [`atomic::write_via_tmp`] for the
/// mechanism.
pub fn write_atomic(file: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    crate::atomic::write_via_tmp(file, text, "toml.tmp", true)
}

/// Set one dotted key in a `config.toml`, creating the file and tables as needed.
pub fn set_in_file(file: &Path, key: &str, value: &Value) -> Result<(), String> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
    };
    let updated = set_in_text(&text, key, value).map_err(|e| format!("{}: {e}", file.display()))?;
    write_atomic(file, &updated)
}

/// The edit itself, on text: keeps everything else in the document untouched.
pub(crate) fn set_in_text(text: &str, key: &str, value: &Value) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| format!("not valid TOML: {}", e.message()))?;
    let new_value = to_edit_value(value)?;
    let (table, leaf) = table_for(&mut doc, key)?;
    match table.get_mut(leaf) {
        // Replace the value but keep the comment after it.
        Some(toml_edit::Item::Value(old)) => {
            let decor = old.decor().clone();
            *old = new_value;
            *old.decor_mut() = decor;
        }
        _ => {
            table.insert(leaf, toml_edit::Item::Value(new_value));
        }
    }
    let out = doc.to_string();
    // Reject anything the loader would reject (unknown key, wrong type).
    toml::from_str::<Settings>(&out).map_err(|e| e.message().to_owned())?;
    Ok(out)
}

fn is_known_setting_key(key: &str) -> bool {
    if key == "context.window" {
        return true;
    }
    let table = toml::Table::try_from(Settings::default()).unwrap_or_default();
    merge::leaves(&table).into_iter().any(|(k, _)| k == key)
}

/// Remove one dotted key from a `config.toml`, keeping comments and other entries untouched.
fn remove_in_file(file: &Path, key: &str) -> Result<(), String> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
    };
    let Some(updated) =
        remove_in_text(&text, key).map_err(|e| format!("{}: {e}", file.display()))?
    else {
        return Ok(());
    };
    write_atomic(file, &updated)
}

pub(crate) fn remove_in_text(text: &str, key: &str) -> Result<Option<String>, String> {
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| format!("not valid TOML: {}", e.message()))?;
    let Some((table, leaf)) = existing_table(&mut doc, key) else {
        return Ok(None);
    };
    if table.remove(leaf).is_none() {
        return Ok(None);
    }
    let out = doc.to_string();
    toml::from_str::<Settings>(&out).map_err(|e| e.message().to_owned())?;
    Ok(Some(out))
}

fn to_edit_value(value: &Value) -> Result<toml_edit::Value, String> {
    if value.is_table() {
        return Err("set the keys inside a table one at a time".into());
    }
    value
        .to_string()
        .parse::<toml_edit::Value>()
        .map_err(|e| e.to_string())
}

/// `(dotted key, description)` for every leaf documented in the settings schema.
fn descriptions() -> Vec<(String, String)> {
    let schema = serde_json::to_value(schemars::schema_for!(Settings)).unwrap_or_default();
    let mut out = Vec::new();
    walk(&schema, &schema, "", &mut out);
    out
}

fn walk(root: &Json, node: &Json, prefix: &str, out: &mut Vec<(String, String)>) {
    let node = resolve(root, node);
    let Some(props) = node.get("properties").and_then(Json::as_object) else {
        return;
    };
    for (name, prop) in props {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        if let Some(d) = prop.get("description").and_then(Json::as_str) {
            out.push((path.clone(), d.to_owned()));
        } else if let Some(d) = resolve(root, prop)
            .get("description")
            .and_then(Json::as_str)
        {
            // Enums document themselves on their definition.
            out.push((path.clone(), d.to_owned()));
        }
        walk(root, prop, &path, out);
    }
}

/// Follow a local `$ref` (directly or through `allOf: [{$ref}]`).
fn resolve<'a>(root: &'a Json, node: &'a Json) -> &'a Json {
    let reference = node.get("$ref").and_then(Json::as_str).or_else(|| {
        node.get("allOf")
            .and_then(Json::as_array)
            .and_then(|a| a.first())
            .and_then(|f| f.get("$ref"))
            .and_then(Json::as_str)
    });
    reference
        .and_then(|r| r.strip_prefix("#/"))
        .and_then(|p| root.pointer(&format!("/{p}")))
        .unwrap_or(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Fixture;

    #[test]
    fn edits_keep_comments_and_validate() {
        let text = "# my settings\n[ui]\nascii = true # I like ascii\n\n[model]\n# pick later\n";
        let out = set_in_text(
            text,
            "model.default",
            &Value::String("charm/qwen3.8-flash".into()),
        )
        .expect("edit");
        assert!(out.starts_with("# my settings\n[ui]\nascii = true # I like ascii\n"));
        assert!(
            out.contains("[model]\ndefault = \"charm/qwen3.8-flash\""),
            "{out}"
        );
        assert!(out.contains("# pick later"), "{out}");

        let out = set_in_text(&out, "ui.ascii", &Value::Boolean(false)).expect("edit");
        assert!(out.contains("ascii = false # I like ascii"), "{out}");

        let typo = set_in_text("", "ui.asci", &Value::Boolean(true));
        assert!(typo.is_err_and(|e| e.contains("asci")));
        let wrong = set_in_text("", "ui.ascii", &Value::String("yes".into()));
        assert!(wrong.is_err());
    }

    #[test]
    fn list_additions_append_and_keep_the_rest() {
        let rules = ["shell:cargo test".to_owned(), "edit:src/a.rs".to_owned()];
        // Unset: appended to the lower layers' rules, not replacing them.
        let out = add_to_list_text("# mine\n[ui]\nascii = true\n", "permissions.allow", &rules)
            .expect("edit");
        assert!(out.starts_with("# mine\n[ui]\nascii = true\n"), "{out}");
        assert!(
            out.contains("allow = { append = [\"shell:cargo test\", \"edit:src/a.rs\"] }"),
            "{out}"
        );
        // Again: nothing doubled.
        assert_eq!(
            add_to_list_text(&out, "permissions.allow", &rules).expect("edit"),
            out
        );
        // A list stays a list, with its comment.
        let out = add_to_list_text(
            "[permissions]\nallow = [\"shell:ls\"] # mine\n",
            "permissions.allow",
            &rules[..1],
        )
        .expect("edit");
        assert!(
            out.contains("allow = [\"shell:ls\", \"shell:cargo test\"] # mine"),
            "{out}"
        );
        let out = add_to_list_text(
            "[permissions.allow]\nappend = [\"shell:ls\"]\n",
            "permissions.allow",
            &rules[..1],
        )
        .expect("edit");
        assert!(
            out.contains("append = [\"shell:ls\", \"shell:cargo test\"]"),
            "{out}"
        );
        assert!(
            add_to_list_text("[permissions]\nallow = 3\n", "permissions.allow", &rules).is_err()
        );
    }

    #[test]
    fn writes_file_atomically() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("sub").join("config.toml");
        set_in_file(&file, "ui.theme", &Value::String("light".into())).expect("write");
        let text = std::fs::read_to_string(&file).expect("read");
        assert_eq!(text, "[ui]\ntheme = \"light\"\n");
        assert!(!file.with_extension("toml.tmp").exists());
    }

    #[test]
    fn schema_descriptions_reach_leaves() {
        let docs = descriptions();
        assert!(
            docs.iter()
                .any(|(k, d)| k == "ui.ascii" && d.contains("ASCII")),
            "{docs:?}"
        );
    }

    #[test]
    fn null_removes_setting_and_project_trust_refreshes() {
        let out = remove_in_text(
            "[context]\n# custom cap\nwindow = 16384\nbudget_ratio = 0.85\n",
            "context.window",
        )
        .expect("remove")
        .expect("changed");
        assert!(!out.contains("window"), "{out}");
        assert!(out.contains("budget_ratio = 0.85"), "{out}");

        let f = Fixture::new();
        f.write("repo/.quack/config.toml", "[context]\nwindow = 8192\n");

        let loaded = f.load();
        loaded.trust_project(TrustDecision::Trusted).expect("trust");
        let loaded = f.load();
        assert_eq!(loaded.project, ProjectStatus::Trusted);
        assert_eq!(loaded.settings.context.window, Some(8192));

        loaded
            .save_setting(LayerKind::Project, "context.window", "null")
            .expect("unset");
        let reloaded = f.load();
        assert_eq!(reloaded.project, ProjectStatus::Trusted);
        assert_eq!(reloaded.settings.context.window, None);
        assert!(
            reloaded
                .save_setting(LayerKind::User, "ui.bogus", "null")
                .is_err()
        );
    }

    #[test]
    fn project_save_setting_clean_repo_and_policy_guard() {
        let f = Fixture::new();

        let clean = f.load();
        assert_eq!(clean.project, ProjectStatus::None);
        let (path, note) = clean
            .save_setting(LayerKind::Project, "ui.theme", "\"light\"")
            .expect("save project setting on clean repo");
        assert_eq!(path, f.path("repo/.quack/config.toml"));
        assert_eq!(note, None);

        let reloaded = f.load();
        assert_eq!(reloaded.project, ProjectStatus::SettingsOnly);
        assert_eq!(reloaded.settings.ui.theme, "light");

        f.write(
            "system/config.toml",
            "[policy]\nproject_config = \"never\"\n",
        );
        let locked = f.load();
        assert_eq!(locked.project, ProjectStatus::DisabledByPolicy);
        let err = locked
            .save_setting(LayerKind::Project, "ui.theme", "\"dark\"")
            .expect_err("disabled by policy");
        assert!(err.contains("policy"), "{err}");
    }
}
