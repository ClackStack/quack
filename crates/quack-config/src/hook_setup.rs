//! Reading and saving hook forms without executing hooks or resolving secrets.

use std::path::PathBuf;

use crate::hooks::HookBinding;
use crate::{HookFile, Item, LayerKind, Loaded, load_hooks, write_atomic};

#[derive(Debug)]
pub struct HookDocument {
    pub item: Item<HookFile>,
    pub file: HookFile,
    pub text: String,
    pub revision: String,
    pub destination: PathBuf,
}

pub fn read(config: &Loaded, id: &str) -> Result<HookDocument, String> {
    let item = load_hooks(config)
        .into_iter()
        .find(|i| i.id == id)
        .ok_or_else(|| format!("No hook `{id}` is configured; open /hooks to choose one."))?;
    let text = std::fs::read_to_string(&item.path)
        .map_err(|_| "Cannot read the hook file; check its permissions.".to_owned())?;
    let file: HookFile = toml::from_str(&text)
        .map_err(|_| "The hook file is invalid; fix its TOML first.".to_owned())?;
    let destination = if item.layer == LayerKind::Defaults {
        config
            .dirs
            .user_config
            .join("hooks.d")
            .join(item.path.file_name().ok_or("The hook needs a file name.")?)
    } else {
        item.path.clone()
    };
    let mut hash = crate::sha256::Sha256::default();
    hash.update(text.as_bytes());
    hash.update(item.path.to_string_lossy().as_bytes());
    if destination != item.path {
        hash.update(&std::fs::read(&destination).unwrap_or_default());
    }
    // Layer changes and disabling edits also invalidate an open form.
    for layer in &config.layers {
        if let Some(dir) = &layer.dir {
            hash.update(&std::fs::read(dir.join("config.toml")).unwrap_or_default());
        }
    }
    Ok(HookDocument {
        item,
        file,
        text,
        revision: hash.hex(),
        destination,
    })
}

pub fn writable(document: &HookDocument, config: &Loaded) -> Result<(), String> {
    if document.item.layer == LayerKind::System {
        return Err(
            "This hook is enforced by the system layer; ask your administrator to configure it."
                .into(),
        );
    }
    if config.settings.policy.locks("hooks.disabled") {
        return Err("Hook configuration is locked by the system policy.".into());
    }
    Ok(())
}

/// Save the complete connection list atomically, retaining the descriptor and comments.
pub fn save(
    config: &Loaded,
    id: &str,
    revision: &str,
    bindings: &[HookBinding],
    enable: Option<bool>,
    replace_payload: Option<bool>,
) -> Result<PathBuf, String> {
    let document = read(config, id)?;
    writable(&document, config)?;
    if document.revision != revision {
        return Err(
            "The hook changed while this form was open. Reload its settings before saving.".into(),
        );
    }
    let mut doc: toml_edit::DocumentMut = document
        .text
        .parse()
        .map_err(|_| "Cannot edit the hook's TOML.".to_owned())?;
    let values: toml_edit::DocumentMut = toml::to_string(&Connections {
        connections: bindings,
    })
    .map_err(|_| "Cannot serialize the hook connections.".to_owned())?
    .parse()
    .map_err(|_| "Cannot format the hook connections.".to_owned())?;
    doc["connections"] = values["connections"].clone();
    if let Some(enabled) = enable {
        doc["enabled"] = toml_edit::value(enabled);
    }
    let text = doc.to_string();
    toml::from_str::<HookFile>(&text)
        .map_err(|_| "The updated hook would not load; nothing was saved.".to_owned())?
        .check()?;
    if document.item.layer == LayerKind::Defaults {
        let stem = document
            .item
            .path
            .file_stem()
            .ok_or("The hook needs a file name.")?;
        let from = document
            .item
            .path
            .parent()
            .ok_or("The hook needs a directory.")?
            .join(stem);
        let to = document
            .destination
            .parent()
            .ok_or("The hook needs a directory.")?
            .join(stem);
        if from.is_dir() {
            crate::enable::install_payload(&from, &to, replace_payload, || {
                write_atomic(&document.destination, &text)
            })?;
        } else {
            write_atomic(&document.destination, &text)?;
        }
    } else {
        write_atomic(&document.destination, &text)?;
    }
    if document.item.layer == LayerKind::Project {
        config.trust_project(crate::TrustDecision::Trusted)?;
    }
    Ok(document.destination)
}

#[derive(serde::Serialize)]
struct Connections<'a> {
    connections: &'a [HookBinding],
}
