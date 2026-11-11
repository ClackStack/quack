//! Hook form validation and credential persistence. No service-specific UI or URLs.

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use quack_config::hook_setup::{read, writable};
use quack_config::hooks::HookBinding;
use quack_config::{Loaded, Secret};
use quack_protocol::{
    HookConnectionDraft, HookConnectionInfo, HookCredentialChange, HookDescriptor, HookSetupInfo,
};
use quack_providers::reqwest::Url;

pub(crate) fn inspect(config: &Loaded, id: &str) -> Result<HookSetupInfo, String> {
    let doc = read(config, id)?;
    let descriptor = doc.file.descriptor()?;
    let secret_fields = descriptor
        .fields
        .iter()
        .filter(|f| f.kind == "secret")
        .map(|f| f.id.clone())
        .collect::<BTreeSet<_>>();
    let can_write = writable(&doc, config).is_ok();
    let problem = writable(&doc, config).err().or_else(|| {
        crate::hooks::command_problem(
            &doc.file,
            doc.item
                .path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("")),
            &config.dirs.workspace_root,
        )
    });
    Ok(HookSetupInfo {
        id: id.into(),
        revision: doc.revision,
        descriptor,
        connections: doc
            .file
            .connections
            .unwrap_or_default()
            .into_iter()
            .map(|mut binding| {
                binding
                    .connection
                    .values
                    .retain(|id, _| !secret_fields.contains(id));
                HookConnectionInfo {
                    connection: binding.connection,
                    credentials: binding
                        .credentials
                        .iter()
                        .map(|(id, secret)| (id.clone(), crate::setup::key_source(secret)))
                        .collect(),
                }
            })
            .collect(),
        enabled: doc.file.enabled && !config.settings.hooks.disabled.iter().any(|d| d == id),
        source: doc.item.path.display().to_string(),
        writable: can_write,
        problem,
    })
}

pub(crate) fn url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "Enter a complete HTTP or HTTPS URL.".to_owned())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Use an HTTP(S) URL without credentials, query strings or fragments.".into());
    }
    Ok(url)
}

pub(crate) fn matches_base(base: &str, resource: &str) -> bool {
    let (Ok(base), Ok(resource)) = (url(base), Url::parse(resource)) else {
        return false;
    };
    base.origin() == resource.origin()
        && resource.username().is_empty()
        && resource.password().is_none()
        && resource
            .path()
            .starts_with(&format!("{}/", base.path().trim_end_matches('/')))
}

pub(crate) fn validate(
    descriptor: &HookDescriptor,
    drafts: &[HookConnectionDraft],
    old: &[HookBinding],
) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    let mut scopes = BTreeSet::new();
    for draft in drafts {
        let conn = &draft.connection;
        if conn.id.is_empty()
            || conn.id.len() > 80
            || !conn
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || !ids.insert(&conn.id)
        {
            return Err(
                "Each connection needs a unique identifier using letters, numbers, '-' or '_'."
                    .into(),
            );
        }
        let preset = descriptor
            .presets
            .iter()
            .find(|p| p.id == conn.preset)
            .ok_or("Choose a service preset for each connection.")?;
        let previous = old.iter().find(|b| b.connection.id == conn.id);
        if draft.credentials.keys().any(|id| {
            !descriptor
                .fields
                .iter()
                .any(|f| f.id == *id && f.kind == "secret")
        }) {
            return Err(
                "The draft contains an unknown credential field; reload the setup form.".into(),
            );
        }
        for field in &descriptor.fields {
            let value = conn.values.get(&field.id).map(String::as_str).unwrap_or("");
            let required = field.required
                || (!field.required_when.is_empty()
                    && field.required_when.iter().all(|(key, values)| {
                        conn.values.get(key).is_some_and(|v| values.contains(v))
                    }));
            if field.kind == "secret" {
                if conn.values.contains_key(&field.id) {
                    return Err("Credentials must use a secret field, not a plain setting.".into());
                }
                let change = draft
                    .credentials
                    .get(&field.id)
                    .unwrap_or(&HookCredentialChange::Keep);
                let present = match change {
                    HookCredentialChange::Keep => {
                        previous.is_some_and(|b| b.credentials.contains_key(&field.id))
                    }
                    HookCredentialChange::Remove => false,
                    HookCredentialChange::Replace { value } => !value.0.trim().is_empty(),
                    HookCredentialChange::External { source, location } => {
                        external(source, &location.0)?;
                        true
                    }
                };
                if required && !present {
                    return Err(format!(
                        "{} is required for this authentication method.",
                        field.label
                    ));
                }
                continue;
            }
            if required && value.trim().is_empty() {
                return Err(format!("{} is required.", field.label));
            }
            if value.is_empty() {
                continue;
            }
            match field.kind.as_str() {
                "url" => {
                    let parsed = url(value).map_err(|e| format!("{}: {e}", field.label))?;
                    let has_credentials = previous.is_some_and(|b| {
                        b.credentials.keys().any(|id| {
                            !matches!(
                                draft.credentials.get(id),
                                Some(HookCredentialChange::Remove)
                            )
                        })
                    }) || draft.credentials.values().any(|c| {
                        matches!(
                            c,
                            HookCredentialChange::Replace { .. }
                                | HookCredentialChange::External { .. }
                        )
                    });
                    if field.credential_scope
                        && field.id != descriptor.scope_field
                        && has_credentials
                        && parsed.scheme() != "https"
                        && !quack_config::is_loopback(value)
                    {
                        return Err(format!(
                            "{} must use HTTPS when a credential is configured (local loopback servers are allowed).",
                            field.label
                        ));
                    }
                }
                "integer" if value.parse::<u32>().is_err() => {
                    return Err(format!("{} needs a nonnegative whole number.", field.label));
                }
                "boolean" if !matches!(value, "true" | "false") => {
                    return Err(format!("{} needs true or false.", field.label));
                }
                "choice"
                    if !preset
                        .choices
                        .get(&field.id)
                        .unwrap_or(&field.choices)
                        .iter()
                        .any(|c| c == value) =>
                {
                    return Err(format!("Choose a valid value for {}.", field.label));
                }
                _ => {}
            }
        }
        if let Some(previous) = previous {
            let scope_changed = previous.connection.preset != conn.preset
                || descriptor
                    .fields
                    .iter()
                    .filter(|f| f.credential_scope)
                    .any(|f| previous.connection.values.get(&f.id) != conn.values.get(&f.id));
            if scope_changed
                && previous.credentials.keys().any(|id| {
                    matches!(
                        draft
                            .credentials
                            .get(id)
                            .unwrap_or(&HookCredentialChange::Keep),
                        HookCredentialChange::Keep
                    )
                })
            {
                return Err("The credential destination changed. Replace, remove, or explicitly select the credential source again before saving or checking.".into());
            }
        }
        if conn.enabled
            && let Some(base) = conn.values.get(&descriptor.scope_field)
        {
            let base = url(base)?;
            if !scopes.insert((
                base.origin().ascii_serialization(),
                base.path().trim_end_matches('/').to_owned(),
            )) {
                return Err("Two enabled connections match the same web address. Edit or disable one of them.".into());
            }
        }
    }
    Ok(())
}

fn external(source: &str, location: &str) -> Result<Secret, String> {
    if location.trim().is_empty() {
        return Err("Enter the credential source location.".into());
    }
    match source {
        "env" => Ok(Secret::Env {
            env: location.into(),
            format: None,
        }),
        "file" => Ok(Secret::File {
            file: location.into(),
        }),
        "keyring" => Ok(Secret::Keyring {
            keyring: location.into(),
        }),
        "cmd" => {
            let cmd: Vec<String> = serde_json::from_str(location)
                .map_err(|_| "Enter the credential command as a JSON argument array.".to_owned())?;
            if cmd.first().is_none_or(|c| c.is_empty()) {
                return Err("The credential command cannot be empty.".into());
            }
            Ok(Secret::Cmd { cmd })
        }
        _ => Err("Choose env, file, keyring or cmd as the credential source.".into()),
    }
}

/// Apply edits without ever returning a credential value to a client.
pub(crate) fn bindings(
    drafts: &[HookConnectionDraft],
    old: &[HookBinding],
    mut store: impl FnMut(&str, &str, &str) -> Result<Secret, String>,
) -> Result<Vec<HookBinding>, String> {
    drafts
        .iter()
        .map(|draft| {
            let mut managed_credentials = old
                .iter()
                .find(|b| b.connection.id == draft.connection.id)
                .map(|b| b.managed_credentials.clone())
                .unwrap_or_default();
            let mut credentials = old
                .iter()
                .find(|b| b.connection.id == draft.connection.id)
                .map(|b| b.credentials.clone())
                .unwrap_or_default();
            for (field, change) in &draft.credentials {
                match change {
                    HookCredentialChange::Keep => {}
                    HookCredentialChange::Remove => {
                        credentials.remove(field);
                        managed_credentials.remove(field);
                    }
                    HookCredentialChange::Replace { value } => {
                        managed_credentials.insert(field.clone());
                        credentials.insert(
                            field.clone(),
                            store(&draft.connection.id, field, value.0.trim())?,
                        );
                    }
                    HookCredentialChange::External { source, location } => {
                        managed_credentials.remove(field);
                        credentials.insert(field.clone(), external(source, &location.0)?);
                    }
                }
            }
            Ok(HookBinding {
                connection: draft.connection.clone(),
                credentials,
                managed_credentials,
            })
        })
        .collect()
}

pub(crate) fn save(
    config: &Loaded,
    id: &str,
    revision: &str,
    drafts: &[HookConnectionDraft],
    enable: Option<bool>,
    replace_payload: Option<bool>,
) -> Result<Option<String>, String> {
    save_with_store(
        config,
        id,
        revision,
        drafts,
        enable,
        replace_payload,
        store_credential,
    )
}

fn save_with_store(
    config: &Loaded,
    id: &str,
    revision: &str,
    drafts: &[HookConnectionDraft],
    enable: Option<bool>,
    replace_payload: Option<bool>,
    mut store: impl FnMut(&Loaded, &str, &str) -> Result<(Secret, String), String>,
) -> Result<Option<String>, String> {
    let config = config
        .reload()
        .map_err(|_| "Cannot reload the configuration; fix its errors first.".to_owned())?;
    let doc = read(&config, id)?;
    writable(&doc, &config)?;
    if enable == Some(true)
        && config.settings.hooks.disabled.iter().any(|d| d == id)
        && matches!(
            config.origin_of("hooks.disabled"),
            quack_config::Origin::Cli | quack_config::Origin::Env(_)
        )
    {
        return Err("This hook is disabled by an environment or command-line override. Change that override before enabling it.".into());
    }
    if doc.revision != revision {
        return Err(
            "The hook changed while this form was open. Reload its settings before saving.".into(),
        );
    }
    let descriptor = doc.file.descriptor()?;
    let old = doc.file.connections.as_deref().unwrap_or_default();
    validate(&descriptor, drafts, old)?;
    if let Some(problem) = crate::hooks::command_problem(
        &doc.file,
        doc.item
            .path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("")),
        &config.dirs.workspace_root,
    ) {
        return Err(format!(
            "The hook is not ready: {problem}. Check Python and the hook payload before saving."
        ));
    }
    // Ask about payload collisions before writing a credential.
    if doc.item.layer == quack_config::LayerKind::Defaults && replace_payload.is_none() {
        let stem = doc
            .item
            .path
            .file_stem()
            .ok_or("The hook needs a file name.")?;
        let payload = doc
            .destination
            .parent()
            .ok_or("The hook needs a directory.")?
            .join(stem);
        if payload.exists() {
            return Err("The user hook payload already exists. Choose Keep payload or Replace payload before saving.".into());
        }
    }
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut created = Vec::new();
    let mut notes = BTreeSet::new();
    let result: Result<(), String> = (|| {
        let bindings = bindings(drafts, old, |conn, field, value| {
            let safe_id: String = id
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let safe_field: String = field
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let name = format!("hook-{safe_id}-{conn}-{safe_field}-{nonce}");
            let (secret, note) = store(&config, &name, value)?;
            notes.insert(note);
            created.push(secret.clone());
            Ok(secret)
        })?;
        quack_config::hook_setup::save(&config, id, revision, &bindings, enable, replace_payload)?;
        if enable == Some(true) && config.settings.hooks.disabled.iter().any(|d| d == id) {
            config
                .reload()
                .map_err(|_| {
                    "The hook was saved but could not be reloaded; run /reload.".to_owned()
                })?
                .enable_hook(id)?;
        }
        Ok(())
    })();
    if result.is_err() {
        // Remove only new staging entries not referenced by the saved file.
        let active = read(&config, id)
            .ok()
            .and_then(|d| d.file.connections)
            .unwrap_or_default();
        for secret in &created {
            if !active
                .iter()
                .any(|b| b.credentials.values().any(|s| s == secret))
            {
                remove_owned(secret);
            }
        }
    }
    result?;
    // Fresh, namespaced entries belong to setup. Keep any entry still referenced by
    // another connection or hook, including hooks which are currently disabled.
    if let Ok(fresh) = config.reload() {
        let mut referenced = Vec::new();
        for item in quack_config::load_hooks(&fresh) {
            if let Ok(document) = read(&fresh, &item.id) {
                referenced.extend(document.file.env.into_values());
                for binding in document.file.connections.unwrap_or_default() {
                    referenced.extend(binding.credentials.into_values());
                }
            }
        }
        for secret in old.iter().flat_map(|b| {
            b.credentials
                .iter()
                .filter(|(id, _)| b.managed_credentials.contains(*id))
                .map(|(_, secret)| secret)
        }) {
            let owned = match secret {
                Secret::Keyring { keyring } => {
                    keyring.starts_with("hook-")
                        && keyring
                            .rsplit('-')
                            .next()
                            .is_some_and(|s| s.len() > 10 && s.bytes().all(|b| b.is_ascii_digit()))
                }
                Secret::File { file } => {
                    let path = std::path::Path::new(file);
                    path.parent() == Some(config.dirs.user_config.join("keys").as_path())
                        && path.file_stem().and_then(|s| s.to_str()).is_some_and(|s| {
                            s.starts_with("hook-")
                                && s.rsplit('-').next().is_some_and(|n| {
                                    n.len() > 10 && n.bytes().all(|b| b.is_ascii_digit())
                                })
                        })
                }
                _ => false,
            };
            if owned && !referenced.contains(secret) {
                remove_owned(secret);
            }
        }
    }
    Ok((!notes.is_empty()).then(|| notes.into_iter().collect::<Vec<_>>().join(" ")))
}

fn store_credential(config: &Loaded, name: &str, value: &str) -> Result<(Secret, String), String> {
    let (store, reason) = crate::setup::key_store(config, name)?;
    match store {
        crate::setup::KeyStore::Keyring => {
            quack_config::keyring_set(name, value).map_err(|_| {
                "Cannot store the credential; unlock the OS credential store and retry.".to_owned()
            })?;
            Ok((
                Secret::Keyring {
                    keyring: name.into(),
                },
                "Credentials stored in the OS credential store.".into(),
            ))
        }
        crate::setup::KeyStore::File(path) => {
            crate::setup::write_key_file(&path, value)?;
            let note = format!(
                "OS credential storage is unavailable{}; credentials stored in private files under {}.",
                reason.map(|r| format!(" ({r})")).unwrap_or_default(),
                path.parent()
                    .unwrap_or_else(|| std::path::Path::new(""))
                    .display()
            );
            Ok((
                Secret::File {
                    file: path.display().to_string(),
                },
                note,
            ))
        }
    }
}

fn remove_owned(secret: &Secret) {
    match secret {
        Secret::Keyring { keyring } => {
            let _ = quack_config::keyring_delete(keyring);
        }
        Secret::File { file } => {
            let _ = std::fs::remove_file(file);
        }
        _ => {}
    }
}

pub(crate) fn check_binding(
    config: &Loaded,
    id: &str,
    revision: &str,
    draft: &HookConnectionDraft,
    resource: &str,
) -> Result<(quack_config::HookFile, std::path::PathBuf), String> {
    let fresh = config
        .reload()
        .map_err(|_| "Cannot reload the hook configuration.".to_owned())?;
    let doc = read(&fresh, id)?;
    writable(&doc, &fresh)?;
    if doc.revision != revision {
        return Err("The hook changed; reload its settings before checking access.".into());
    }
    let descriptor = doc.file.descriptor()?;
    validate(
        &descriptor,
        std::slice::from_ref(draft),
        doc.file.connections.as_deref().unwrap_or_default(),
    )?;
    let base = draft
        .connection
        .values
        .get(&descriptor.scope_field)
        .ok_or("Enter the server URL first.")?;
    if !matches_base(base, resource) {
        return Err("The sample URL does not belong to this connection. Enter a review or issue URL on its configured server.".into());
    }
    let mut file = doc.file.clone();
    file.connections = Some(bindings(
        std::slice::from_ref(draft),
        doc.file.connections.as_deref().unwrap_or_default(),
        |_, _, value| Ok(Secret::Plain(value.into())),
    )?);
    // A disabled connection can still be checked deliberately.
    if let Some(connection) = file.connections.as_mut().and_then(|c| c.first_mut()) {
        connection.connection.enabled = true;
    }
    Ok((
        file,
        doc.item
            .path
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""))
            .to_path_buf(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use quack_protocol::SecretText;
    use std::collections::{BTreeMap, HashMap};
    use std::ffi::OsString;
    use std::path::Path;

    fn fixture() -> (tempfile::TempDir, Loaded) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let user = tmp.path().join("user");
        let hooks = user.join("hooks.d");
        std::fs::create_dir_all(hooks.join("forge-review")).expect("mkdir");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../defaults/hooks.d");
        for file in ["forge-review.toml", "forge-review/review.py"] {
            std::fs::copy(source.join(file), hooks.join(file)).expect("copy");
        }
        // Storage tests never execute the command and do not require Python.
        let path = hooks.join("forge-review.toml");
        let mut file: quack_config::HookFile =
            toml::from_str(&std::fs::read_to_string(&path).expect("hook")).expect("hook TOML");
        file.command = vec![
            std::env::current_exe()
                .expect("test binary")
                .display()
                .to_string(),
        ];
        std::fs::write(path, toml::to_string(&file).expect("serialize")).expect("fixture");
        let ws = tmp.path().join("ws");
        std::fs::create_dir(&ws).expect("workspace");
        let vars = HashMap::<String, OsString>::from([
            ("QUACK_CONFIG_DIR".into(), user.into_os_string()),
            (
                "QUACK_DATA_DIR".into(),
                tmp.path().join("data").into_os_string(),
            ),
            (
                "QUACK_CACHE_DIR".into(),
                tmp.path().join("cache").into_os_string(),
            ),
            (
                "QUACK_SYSTEM_DIR".into(),
                tmp.path().join("system").into_os_string(),
            ),
            ("QUACK_DEFAULTS_DIR".into(), "none".into()),
        ]);
        let config = quack_config::load_with(
            &|key| vars.get(key).cloned(),
            &quack_config::LoadOptions::new(ws),
        )
        .expect("isolated config");
        (tmp, config)
    }

    fn token_draft(config: &Loaded, value: &str) -> HookConnectionDraft {
        let info = inspect(config, "forge-review").expect("inspect");
        let mut conn = info
            .connections
            .first()
            .map(|c| c.connection.clone())
            .unwrap_or_else(|| {
                let file: quack_config::HookFile =
                    toml::from_str(include_str!("../../../defaults/hooks.d/forge-review.toml"))
                        .expect("default hook");
                file.connections.expect("default connections")[0]
                    .connection
                    .clone()
            });
        conn.values.insert("auth".into(), "token".into());
        HookConnectionDraft {
            connection: conn,
            credentials: BTreeMap::from([(
                "token".into(),
                HookCredentialChange::Replace {
                    value: SecretText(value.into()),
                },
            )]),
        }
    }

    fn fake_store(config: &Loaded, name: &str, value: &str) -> Result<(Secret, String), String> {
        let path = crate::setup::key_file(config, name)?;
        crate::setup::write_key_file(&path, value)?;
        Ok((
            Secret::File {
                file: path.display().to_string(),
            },
            "Stored in an isolated test file.".into(),
        ))
    }

    #[test]
    fn failed_storage_and_a_conflict_during_storage_leave_no_active_or_orphaned_key() {
        let (_tmp, config) = fixture();
        let info = inspect(&config, "forge-review").expect("inspect");
        let draft = token_draft(&config, "private-test-token");
        let before = std::fs::read_to_string(&info.source).expect("file");
        let error = save_with_store(
            &config,
            &info.id,
            &info.revision,
            std::slice::from_ref(&draft),
            None,
            None,
            |_, _, _| Err("store locked".into()),
        )
        .expect_err("storage fails");
        assert_eq!(error, "store locked");
        assert_eq!(std::fs::read_to_string(&info.source).expect("file"), before);
        let mut staged = None;
        let result = save_with_store(
            &config,
            &info.id,
            &info.revision,
            &[draft],
            None,
            None,
            |config, name, value| {
                let stored = fake_store(config, name, value)?;
                staged = Some(stored.0.clone());
                std::fs::write(&info.source, format!("{before}\n# external edit\n"))
                    .expect("external change");
                Ok(stored)
            },
        );
        assert!(result.is_err());
        if let Some(Secret::File { file }) = staged {
            assert!(
                !Path::new(&file).exists(),
                "staging credential is cleaned up"
            );
        }
        assert!(
            !std::fs::read_to_string(&info.source)
                .expect("file")
                .contains("private-test-token")
        );
    }

    #[test]
    fn rotation_uses_a_new_reference_and_removal_preserves_shared_and_external_secrets() {
        let (_tmp, config) = fixture();
        let info = inspect(&config, "forge-review").expect("inspect");
        let draft = token_draft(&config, "first-token");
        save_with_store(
            &config,
            &info.id,
            &info.revision,
            &[draft],
            None,
            None,
            fake_store,
        )
        .expect("save");
        let first = read(&config, &info.id).expect("read");
        let first_secret =
            first.file.connections.as_ref().expect("connections")[0].credentials["token"].clone();
        let Secret::File { file: first_path } = &first_secret else {
            panic!("test file source")
        };
        assert!(Path::new(first_path).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(first_path)
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let mut shared = first.file.connections.as_ref().expect("connections")[0].clone();
        shared.connection.id = "shared".into();
        shared
            .connection
            .values
            .insert("web_url".into(), "https://other.example".into());
        let mut bindings = first.file.connections.clone().expect("connections");
        bindings.push(shared);
        quack_config::hook_setup::save(&config, &info.id, &first.revision, &bindings, None, None)
            .expect("shared source");
        let info = inspect(&config, &info.id).expect("inspect");
        let first_draft = token_draft(&config, "second-token");
        let shared_draft = HookConnectionDraft {
            connection: info.connections[1].connection.clone(),
            credentials: BTreeMap::new(),
        };
        save_with_store(
            &config,
            &info.id,
            &info.revision,
            &[first_draft, shared_draft],
            None,
            None,
            fake_store,
        )
        .expect("rotate");
        assert!(
            Path::new(first_path).exists(),
            "another connection still references the key"
        );
        let current = read(&config, &info.id).expect("read");
        let new_secret =
            &current.file.connections.as_ref().expect("connections")[0].credentials["token"];
        assert_ne!(new_secret, &first_secret);
        save_with_store(
            &config,
            &info.id,
            &current.revision,
            &[],
            None,
            None,
            fake_store,
        )
        .expect("remove config");
        assert!(
            !Path::new(first_path).exists(),
            "unreferenced owned credentials are removed"
        );
        // An externally managed source is never modified by a credential Keep edit.
        let external_path = config
            .dirs
            .user_config
            .join("keys")
            .join("hook-external-token-10000000000000000000.key");
        std::fs::write(&external_path, "external").expect("external key");
        let info = inspect(&config, &info.id).expect("inspect");
        let mut external = token_draft(&config, "unused");
        external.credentials.insert(
            "token".into(),
            HookCredentialChange::External {
                source: "file".into(),
                location: SecretText(external_path.display().to_string()),
            },
        );
        save_with_store(
            &config,
            &info.id,
            &info.revision,
            &[external],
            None,
            None,
            fake_store,
        )
        .expect("external source");
        let info = inspect(&config, &info.id).expect("inspect");
        save_with_store(
            &config,
            &info.id,
            &info.revision,
            &[],
            None,
            None,
            fake_store,
        )
        .expect("remove");
        assert!(external_path.exists());
    }
}
