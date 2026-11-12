//! What quack remembers between runs, in `<user data>/state.toml`: not configuration, so
//! it never lives in a config layer, and deleting the file only forgets.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::load::{Loaded, Origin};
use crate::paths::LayerKind;

const FILE: &str = "state.toml";

/// The state file for rewriting; a missing file starts a fresh one with a comment.
fn open_doc(user_data: &Path) -> Result<toml_edit::DocumentMut, String> {
    let file = user_data.join(FILE);
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            "# Remembered by quack between runs. Safe to delete.\n".to_owned()
        }
        Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
    };
    text.parse().map_err(|e| format!("{}: {e}", file.display()))
}

/// Write the document beside the file and rename, so two quacks never leave a half-written
/// file.
fn write_doc(user_data: &Path, doc: &toml_edit::DocumentMut) -> Result<PathBuf, String> {
    let file = user_data.join(FILE);
    std::fs::create_dir_all(user_data).map_err(|e| format!("{}: {e}", user_data.display()))?;
    let tmp = file.with_extension("toml.new");
    std::fs::write(&tmp, doc.to_string())
        .and_then(|()| std::fs::rename(&tmp, &file))
        .map_err(|e| format!("cannot write {}: {e}", file.display()))?;
    Ok(file)
}

/// The model last switched to, as `<provider>/<model>`.
pub fn last_model(user_data: &Path) -> Option<String> {
    let text = std::fs::read_to_string(user_data.join(FILE)).ok()?;
    let doc: toml::Table = text.parse().ok()?;
    doc.get("last_model")?
        .as_str()
        .filter(|m| m.contains('/'))
        .map(str::to_owned)
}

/// Remember `model` for the next start. Other keys and comments in the file are kept.
pub fn save_last_model(user_data: &Path, model: &str) -> Result<PathBuf, String> {
    let mut doc = open_doc(user_data)?;
    if doc.get("last_model").and_then(|v| v.as_str()) == Some(model) {
        return Ok(user_data.join(FILE));
    }
    doc["last_model"] = toml_edit::value(model);
    write_doc(user_data, &doc)
}

/// The thinking level last picked for each model, as `<provider>/<model>` → effort. A
/// session's own selections (its log records) come on top of these.
pub fn reasoning_efforts(user_data: &Path) -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(user_data.join(FILE)).unwrap_or_default();
    let doc: toml::Table = text.parse().unwrap_or_default();
    doc.get("reasoning_efforts")
        .and_then(|v| v.as_table())
        .map(|table| {
            table
                .iter()
                .filter(|(model, _)| model.contains('/'))
                .filter_map(|(model, effort)| Some((model.to_owned(), effort.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

/// Remember the thinking levels for the next start: entries absent from `efforts` (a reset
/// to the model's default) are forgotten. Other keys and comments in the file are kept.
pub fn save_reasoning_efforts(
    user_data: &Path,
    efforts: &BTreeMap<String, String>,
) -> Result<PathBuf, String> {
    let mut doc = open_doc(user_data)?;
    let unchanged = doc
        .get("reasoning_efforts")
        .and_then(|v| v.as_table())
        .is_some_and(|table| {
            table.len() == efforts.len()
                && efforts.iter().all(|(model, effort)| {
                    table.get(model).and_then(|v| v.as_str()) == Some(effort.as_str())
                })
        });
    if unchanged {
        return Ok(user_data.join(FILE));
    }
    doc.remove("reasoning_efforts");
    if !efforts.is_empty() {
        let mut table = toml_edit::Table::new();
        for (model, effort) in efforts {
            table.insert(model.as_str(), toml_edit::value(effort.as_str()));
        }
        doc["reasoning_efforts"] = toml_edit::Item::Table(table);
    }
    write_doc(user_data, &doc)
}

impl Loaded {
    /// The remembered model to start with instead of `model.default`, if any. Only a
    /// personal default gives way to it: a project's choice, `QUACK_MODEL`, `-m` and a
    /// policy lock all win, and `model.remember_last = false` turns it off.
    pub fn remembered_model(&self) -> Option<String> {
        if !self.settings.model.remember_last {
            return None;
        }
        let key = "model.default";
        if self.settings.policy.locks(key) {
            return None;
        }
        if matches!(
            self.origin_of(key),
            Origin::Env(_)
                | Origin::Cli
                | Origin::File {
                    layer: LayerKind::Project,
                    ..
                }
        ) {
            return None;
        }
        last_model(&self.dirs.user_data).filter(|m| *m != self.settings.model.default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_reads_and_keeps_other_keys() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("data");
        assert_eq!(last_model(&dir), None);
        save_last_model(&dir, "charm-hyper/glm-5").expect("save");
        assert_eq!(last_model(&dir).as_deref(), Some("charm-hyper/glm-5"));

        let file = dir.join(FILE);
        let text = std::fs::read_to_string(&file).expect("read");
        std::fs::write(&file, format!("{text}# mine\nother = 1\n")).expect("write");
        save_last_model(&dir, "ollama/qwen3").expect("save");
        let text = std::fs::read_to_string(&file).expect("read");
        assert!(text.contains("last_model = \"ollama/qwen3\""), "{text}");
        assert!(
            text.contains("# mine") && text.contains("other = 1"),
            "{text}"
        );
    }

    #[test]
    fn ignores_a_broken_or_odd_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join(FILE), "last_model = ").expect("write");
        assert_eq!(last_model(tmp.path()), None);
        assert!(reasoning_efforts(tmp.path()).is_empty());
        std::fs::write(tmp.path().join(FILE), "last_model = \"no-slash\"").expect("write");
        assert_eq!(last_model(tmp.path()), None);
    }

    #[test]
    fn remembers_reasoning_efforts_per_model() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("data");
        assert!(reasoning_efforts(&dir).is_empty());

        let mut efforts = BTreeMap::new();
        efforts.insert("charm-hyper/glm-5".to_owned(), "high".to_owned());
        efforts.insert("ollama/qwen3".to_owned(), "low".to_owned());
        save_reasoning_efforts(&dir, &efforts).expect("save");
        assert_eq!(reasoning_efforts(&dir), efforts);

        // The file keeps the other keys, and re-saving the same map leaves it alone.
        let file = dir.join(FILE);
        let text = std::fs::read_to_string(&file).expect("read");
        std::fs::write(&file, format!("# mine\nlast_model = \"a/b\"\n{text}")).expect("write");
        save_reasoning_efforts(&dir, &efforts).expect("save");
        let text = std::fs::read_to_string(&file).expect("read");
        assert!(
            text.contains("# mine") && text.contains("last_model"),
            "{text}"
        );
        assert!(text.contains("\"charm-hyper/glm-5\" = \"high\""), "{text}");

        // A reset drops only its model's entry.
        efforts.remove("ollama/qwen3");
        save_reasoning_efforts(&dir, &efforts).expect("save");
        assert_eq!(reasoning_efforts(&dir), efforts);

        // An empty map removes the table entirely.
        efforts.clear();
        save_reasoning_efforts(&dir, &efforts).expect("save");
        assert!(reasoning_efforts(&dir).is_empty());
        let text = std::fs::read_to_string(&file).expect("read");
        assert!(!text.contains("reasoning_efforts"), "{text}");
        assert!(text.contains("last_model"), "{text}");
    }

    #[test]
    fn skips_entries_without_a_model_shape() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join(FILE),
            "[reasoning_efforts]\nnoslash = \"high\"\n\"a/b\" = 3\n\"c/d\" = \"low\"\n",
        )
        .expect("write");
        let efforts = reasoning_efforts(tmp.path());
        assert_eq!(efforts.len(), 1);
        assert_eq!(efforts["c/d"], "low");
    }
}
