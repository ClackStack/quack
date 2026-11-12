//! One-file-per-item directories (`providers.d`, `mcp.d`, `lsp.d`, `hooks.d`, `agents.d`).
//!
//! Items merge by id (default: file stem). A higher layer's item replaces a lower one
//! entirely, except where [`load_items_merging`] is used (providers): there files that
//! share an id merge field by field, the `config.toml` rules, so a user file can hold
//! only what it changes and the rest still comes from the defaults bundle. A file
//! containing only `enabled = false` disables the lower item. A file that fails to parse
//! fails only that item.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use toml::{Table, Value};

use crate::diagnostic::{FileDiagnostic, read_text};
use crate::merge;
use crate::paths::LayerKind;
use crate::settings::AllowList;

#[derive(Debug, Clone)]
pub struct Item<T> {
    pub id: String,
    pub path: PathBuf,
    pub layer: LayerKind,
    pub state: ItemState<T>,
}

#[derive(Debug, Clone)]
pub enum ItemState<T> {
    Loaded(T),
    Disabled(String),
    Failed(FileDiagnostic),
}

impl<T> Item<T> {
    pub fn loaded(&self) -> Option<&T> {
        match &self.state {
            ItemState::Loaded(t) => Some(t),
            _ => None,
        }
    }
}

/// Load `<layer>/<subdir>/*.toml` for each layer, lowest precedence first.
pub fn load_items<T: DeserializeOwned>(
    layers: &[(LayerKind, PathBuf)],
    subdir: &str,
) -> Vec<Item<T>> {
    load_items_except(layers, subdir, &[])
}

pub(crate) fn load_items_except<T: DeserializeOwned>(
    layers: &[(LayerKind, PathBuf)],
    subdir: &str,
    excluded: &[PathBuf],
) -> Vec<Item<T>> {
    let mut by_id: BTreeMap<String, Item<T>> = BTreeMap::new();
    for (layer, dir) in layers {
        for path in toml_files(&dir.join(subdir))
            .into_iter()
            .filter(|p| !excluded.contains(p))
        {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let (id, state) = load_one::<T>(&path, stem);
            let state = match state {
                Some(state) => state,
                None => ItemState::Disabled(format!("disabled by {}", path.display())),
            };
            by_id.insert(
                id.clone(),
                Item {
                    id,
                    path,
                    layer: *layer,
                    state,
                },
            );
        }
    }
    by_id.into_values().collect()
}

/// One item file, parsed far enough to know its id.
struct Parsed {
    id: String,
    layer: LayerKind,
    path: PathBuf,
    /// The file's own text, for diagnostics that point into it.
    text: String,
    doc: Result<toml_edit::DocumentMut, FileDiagnostic>,
}

fn parse_file(layer: LayerKind, path: PathBuf) -> Parsed {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let text = match read_text(&path) {
        Ok(t) => t,
        Err(e) => {
            let doc = Err(FileDiagnostic::io(&path, &e));
            return Parsed {
                id: stem,
                layer,
                path,
                text: String::new(),
                doc,
            };
        }
    };
    // toml is built on toml_edit's parser, so what validates here parses there too; the
    // toml pass runs first only because its errors carry spans for diagnostics.
    let doc = match toml::from_str::<Table>(&text) {
        Err(e) => Err(FileDiagnostic::from_toml(&path, &text, &e)),
        Ok(_) => text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| FileDiagnostic::new(&path, &text, e.message().trim(), None)),
    };
    let id = doc
        .as_ref()
        .ok()
        .and_then(|d| d.get("id"))
        .and_then(toml_edit::Item::as_str)
        .map_or(stem, str::to_owned);
    Parsed {
        id,
        layer,
        path,
        text,
        doc,
    }
}

/// Load `<layer>/<subdir>/*.toml` for each layer, merging files that share an id field by
/// field: tables merge recursively, scalars and arrays replace, and `{ append = [...] }`
/// extends a lower layer's array, the `config.toml` rules. So a user file can hold only
/// what it changes (a key, a header) and everything else, the model list above all, still
/// comes from the defaults bundle beneath it, staying current across updates. A file with
/// only `enabled = false` disables the item; a `enabled = false` among other fields
/// disables it too, while `enabled = true` can switch a lower layer's off back on. The
/// item reports the highest layer's file, and a file anywhere in the chain that does not
/// parse fails the item, since its fields cannot merge.
pub fn load_items_merging<T: DeserializeOwned>(
    layers: &[(LayerKind, PathBuf)],
    subdir: &str,
) -> Vec<Item<T>> {
    let mut by_id: BTreeMap<String, Vec<Parsed>> = BTreeMap::new();
    for (layer, dir) in layers {
        for path in toml_files(&dir.join(subdir)) {
            let parsed = parse_file(*layer, path);
            by_id.entry(parsed.id.clone()).or_default().push(parsed);
        }
    }
    by_id
        .into_iter()
        .map(|(id, mut files)| {
            let top = files.pop().expect("every id has a file");
            let state = merged_state::<T>(&top, &files);
            Item {
                id,
                path: top.path,
                layer: top.layer,
                state,
            }
        })
        .collect()
}

fn merged_state<T: DeserializeOwned>(top: &Parsed, lower: &[Parsed]) -> ItemState<T> {
    let top_doc = match &top.doc {
        Ok(d) => d,
        Err(d) => return ItemState::Failed(d.clone()),
    };
    if lower.is_empty() {
        // A single file: deserialize it from its own text, so diagnostics keep their
        // exact spans, exactly as `load_one` reports them.
        return match load_one::<T>(&top.path, String::new()) {
            (_, Some(state)) => state,
            (_, None) => ItemState::Disabled(format!("disabled by {}", top.path.display())),
        };
    }
    if let Some(d) = lower.iter().find_map(|f| f.doc.as_ref().err()) {
        return ItemState::Failed(d.clone());
    }
    let enabled =
        |doc: &toml_edit::DocumentMut| doc.get("enabled").and_then(toml_edit::Item::as_bool);
    if enabled(top_doc) == Some(false)
        && top_doc
            .as_table()
            .iter()
            .all(|(k, _)| k == "enabled" || k == "id")
    {
        return ItemState::Disabled(format!("disabled by {}", top.path.display()));
    }
    let mut merged = lower[0].doc.clone().expect("checked above");
    for f in &lower[1..] {
        merge::merge_docs(&mut merged, f.doc.as_ref().expect("checked above"));
    }
    merge::merge_docs(&mut merged, top_doc);
    if enabled(&merged) == Some(false) {
        return ItemState::Disabled("enabled = false".into());
    }
    match toml::from_str::<T>(&merged.to_string()) {
        Ok(item) => ItemState::Loaded(item),
        // The merged text is nowhere on disk, so the diagnostic carries no span; it
        // names the highest file, where the fix usually goes.
        Err(e) => ItemState::Failed(FileDiagnostic::new(
            &top.path,
            &top.text,
            e.message().trim(),
            None,
        )),
    }
}

/// Load `<layer>/<subdir>/<id>/<file>` for each layer (directory items such as profiles).
/// The id is the directory name unless the file sets `id`.
pub fn load_dir_items<T: DeserializeOwned>(
    layers: &[(LayerKind, PathBuf)],
    subdir: &str,
    file: &str,
) -> Vec<Item<T>> {
    let mut by_id: BTreeMap<String, Item<T>> = BTreeMap::new();
    for (layer, dir) in layers {
        let Ok(entries) = std::fs::read_dir(dir.join(subdir)) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.join(file).is_file())
            .collect();
        dirs.sort();
        for d in dirs {
            let name = d
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let path = d.join(file);
            let (id, state) = load_one::<T>(&path, name);
            let state = state
                .unwrap_or_else(|| ItemState::Disabled(format!("disabled by {}", path.display())));
            by_id.insert(
                id.clone(),
                Item {
                    id,
                    path,
                    layer: *layer,
                    state,
                },
            );
        }
    }
    by_id.into_values().collect()
}

/// `None` state means the file is a bare `enabled = false` override.
fn load_one<T: DeserializeOwned>(path: &Path, stem: String) -> (String, Option<ItemState<T>>) {
    let text = match read_text(path) {
        Ok(t) => t,
        Err(e) => return (stem, Some(ItemState::Failed(FileDiagnostic::io(path, &e)))),
    };
    let table = match toml::from_str::<Table>(&text) {
        Ok(t) => t,
        Err(e) => {
            return (
                stem,
                Some(ItemState::Failed(FileDiagnostic::from_toml(
                    path, &text, &e,
                ))),
            );
        }
    };
    let id = table
        .get("id")
        .and_then(Value::as_str)
        .map_or(stem, str::to_owned);
    let enabled = table.get("enabled").and_then(Value::as_bool);
    if enabled == Some(false) && table.keys().all(|k| k == "enabled" || k == "id") {
        return (id, None);
    }
    let state = match toml::from_str::<T>(&text) {
        Ok(_) if enabled == Some(false) => ItemState::Disabled("enabled = false".into()),
        Ok(item) => ItemState::Loaded(item),
        Err(e) => ItemState::Failed(FileDiagnostic::from_toml(path, &text, &e)),
    };
    (id, Some(state))
}

pub(crate) fn toml_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    files
}

/// Apply `[<kind>] disabled = [...]` and the policy allow-list. `url` gives a network
/// item's address, checked against the policy's hosts.
pub(crate) fn apply_toggles<T>(
    items: &mut [Item<T>],
    disabled: &[String],
    policy: &AllowList,
    url: impl Fn(&T) -> Option<&str>,
) {
    for item in items {
        let allowed = match &item.state {
            ItemState::Failed(_) => continue,
            ItemState::Loaded(t) => policy.allows(&item.id, url(t)),
            ItemState::Disabled(_) => policy.allows(&item.id, None),
        };
        if disabled.iter().any(|d| d == &item.id) {
            item.state = ItemState::Disabled("listed in `disabled`".into());
        } else if !allowed {
            item.state = ItemState::Disabled("not allowed by policy".into());
        }
    }
}

/// The host of a URL: `https://user@api.example.com:443/v1` → `api.example.com`, and
/// `http://[::1]:8080` → `[::1]`.
pub fn url_host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    if host.starts_with('[') {
        return host.find(']').map_or(host, |i| &host[..=i]);
    }
    host.rsplit_once(':').map_or(host, |(h, _)| h)
}

/// A loopback address: the server is on this machine.
pub fn is_loopback(url: &str) -> bool {
    let host = url_host(url).to_ascii_lowercase();
    host == "localhost" || host == "[::1]" || host.starts_with("127.")
}

/// Minimal `*` glob for id matching.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !text.starts_with(first) || !text[first.len()..].ends_with(last) {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    for mid in &parts[1..parts.len() - 1] {
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderFile;

    fn write(dir: &Path, name: &str, text: &str) {
        std::fs::create_dir_all(dir).expect("mkdir");
        std::fs::write(dir.join(name), text).expect("write");
    }

    const LLAMA: &str = "api = \"openai-chat\"\nbase_url = \"http://127.0.0.1:8080/v1\"\n";

    #[test]
    fn items_merge_by_id_and_disable_cleanly() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (defaults, user) = (tmp.path().join("d"), tmp.path().join("u"));
        write(&defaults.join("providers.d"), "llama.toml", LLAMA);
        write(&defaults.join("providers.d"), "other.toml", LLAMA);
        write(&defaults.join("providers.d"), "broken.toml", "api = 1");
        write(&user.join("providers.d"), "other.toml", "enabled = false");

        let layers = vec![(LayerKind::Defaults, defaults), (LayerKind::User, user)];
        let mut items = load_items::<ProviderFile>(&layers, "providers.d");
        apply_toggles(&mut items, &[], &AllowList::default(), |f| {
            Some(f.base_url.as_str())
        });
        let state = |id: &str| &items.iter().find(|i| i.id == id).expect("item").state;
        assert!(matches!(state("llama"), ItemState::Loaded(_)));
        assert!(matches!(state("other"), ItemState::Disabled(_)));
        assert!(matches!(state("broken"), ItemState::Failed(_)));
    }

    #[test]
    fn policy_hosts_disable_other_servers() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("u");
        write(&dir.join("providers.d"), "local.toml", LLAMA);
        write(
            &dir.join("providers.d"),
            "corp.toml",
            "base_url = \"https://llm.corp.example/v1\"\n",
        );
        // An allowed id cannot point somewhere else.
        write(
            &dir.join("providers.d"),
            "corp-2.toml",
            "base_url = \"https://api.elsewhere.example/v1\"\n",
        );
        let mut items = load_items::<ProviderFile>(&[(LayerKind::User, dir)], "providers.d");
        let policy = AllowList {
            allowed: vec!["corp*".into()],
            hosts: vec!["*.corp.example".into()],
        };
        apply_toggles(&mut items, &[], &policy, |f| Some(f.base_url.as_str()));
        let loaded: Vec<&str> = items
            .iter()
            .filter(|i| i.loaded().is_some())
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(loaded, ["corp"]);
    }

    #[test]
    fn hosts_of_urls() {
        assert_eq!(
            url_host("https://u@api.example.com:443/v1?x"),
            "api.example.com"
        );
        assert_eq!(url_host("http://[::1]:8080/v1"), "[::1]");
        assert_eq!(url_host("api.example.com/v1"), "api.example.com");
        assert!(is_loopback("http://localhost:11434"));
        assert!(is_loopback("http://127.0.0.1:8080/v1"));
        assert!(!is_loopback("http://10.0.0.5:8000/v1"));
    }

    #[test]
    fn missing_directory_is_empty() {
        let items = load_items::<ProviderFile>(
            &[(LayerKind::User, PathBuf::from("/nonexistent"))],
            "providers.d",
        );
        assert!(items.is_empty());
    }

    #[test]
    fn globs() {
        assert!(glob_match("corp-*", "corp-vllm"));
        assert!(glob_match("*", "x"));
        assert!(glob_match("a*b*c", "aXbYc"));
        assert!(!glob_match("corp-*", "novita"));
        assert!(!glob_match("a*a", "a"));
    }

    const CHARM: &str = "name = \"Charm Hyper\"\ngroup = \"Hosted\"\n\
        base_url = \"https://hyper.example/v1\"\n\
        api_key = { env = \"CHARM_API_KEY\" }\n\
        models = [\"glm-5.3\", \"kimi-k3\"]\n";

    #[test]
    fn provider_files_merge_field_by_field() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (defaults, user) = (tmp.path().join("d"), tmp.path().join("u"));
        write(&defaults.join("providers.d"), "charm.toml", CHARM);
        // The form `quack setup` writes: only what the user changed.
        write(
            &user.join("providers.d"),
            "charm.toml",
            "api_key = { keyring = \"charm\" }\n",
        );
        let layers = vec![(LayerKind::Defaults, defaults), (LayerKind::User, user)];
        let items = load_items_merging::<ProviderFile>(&layers, "providers.d");
        let item = &items[0];
        assert_eq!(item.id, "charm");
        assert_eq!(item.layer, LayerKind::User);
        assert!(item.path.ends_with("charm.toml"), "{}", item.path.display());
        let f = item.loaded().expect("loads");
        // The key comes from the user file; everything else from the bundle.
        assert_eq!(
            f.api_key,
            Some(crate::Secret::Keyring {
                keyring: "charm".into()
            })
        );
        assert_eq!(f.name, "Charm Hyper");
        assert_eq!(f.base_url, "https://hyper.example/v1");
        let ids: Vec<&str> = f.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["glm-5.3", "kimi-k3"]);
    }

    #[test]
    fn append_extends_the_bundled_model_list() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (defaults, user) = (tmp.path().join("d"), tmp.path().join("u"));
        write(&defaults.join("providers.d"), "charm.toml", CHARM);
        write(
            &user.join("providers.d"),
            "charm.toml",
            "models.append = [\"glm-5.4\"]\n",
        );
        let layers = vec![(LayerKind::Defaults, defaults), (LayerKind::User, user)];
        let items = load_items_merging::<ProviderFile>(&layers, "providers.d");
        let ids: Vec<&str> = items[0]
            .loaded()
            .expect("loads")
            .models
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(ids, ["glm-5.3", "kimi-k3", "glm-5.4"]);
    }

    #[test]
    fn a_user_file_can_switch_a_bundled_provider_back_on() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (defaults, user) = (tmp.path().join("d"), tmp.path().join("u"));
        write(
            &defaults.join("providers.d"),
            "charm.toml",
            &format!("enabled = false\n{CHARM}"),
        );
        write(&user.join("providers.d"), "charm.toml", "enabled = true\n");
        let layers = vec![
            (LayerKind::Defaults, defaults.clone()),
            (LayerKind::User, user.clone()),
        ];
        let items = load_items_merging::<ProviderFile>(&layers, "providers.d");
        assert_eq!(items[0].id, "charm");
        assert_eq!(
            items[0].loaded().expect("loads").base_url,
            "https://hyper.example/v1"
        );

        // A bare `enabled = false` still disables it.
        write(&user.join("providers.d"), "charm.toml", "enabled = false\n");
        let items = load_items_merging::<ProviderFile>(&layers, "providers.d");
        assert!(matches!(items[0].state, ItemState::Disabled(_)));
    }

    #[test]
    fn a_file_that_does_not_parse_fails_the_merged_item() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let (defaults, user) = (tmp.path().join("d"), tmp.path().join("u"));
        write(&defaults.join("providers.d"), "charm.toml", CHARM);
        write(&user.join("providers.d"), "charm.toml", "base_url = 123\n");
        let layers = vec![
            (LayerKind::Defaults, defaults.clone()),
            (LayerKind::User, user.clone()),
        ];
        let items = load_items_merging::<ProviderFile>(&layers, "providers.d");
        let ItemState::Failed(d) = &items[0].state else {
            panic!("must fail: {:?}", items[0].state);
        };
        assert!(d.one_line().contains("charm.toml"), "{}", d.one_line());
        assert!(
            d.path.starts_with(&user),
            "the error names the file that set the field: {}",
            d.one_line()
        );

        // A broken file deeper in the chain fails the item too: its fields cannot merge.
        write(
            &defaults.join("providers.d"),
            "charm.toml",
            "base_url = 123\n",
        );
        write(&user.join("providers.d"), "charm.toml", "enabled = true\n");
        let items = load_items_merging::<ProviderFile>(&layers, "providers.d");
        assert!(matches!(items[0].state, ItemState::Failed(_)));
    }
}
