//! The two things quack fetches for itself: the model catalog and the latest version number.
//!
//! Both come from URLs in the configuration and pass the egress gate
//! (`quack_providers::fetch`). The commands (`quack catalog refresh`, `quack update`) ask
//! when the user says so. The background checks run only when `catalog.refresh` or
//! `updates.check` is on, at most once a day, and stay quiet when they fail. Nothing here
//! delays startup or changes the running session: a new catalog applies from the next one.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use quack_config::{Catalog, Dirs, LayerKind, Loaded, Origin, load_providers, write_atomic};
use quack_protocol::Event;
use quack_providers::fetch::{self, Fetched, Get};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

const STATE_FILE: &str = "checks.json";
const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// models.dev is about 6 MB, and only grows.
const MAX_CATALOG_BYTES: usize = 64 << 20;
const MAX_VERSION_BYTES: usize = 1 << 10;

/// The running version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// When each check last ran, in the cache directory. Failed checks count too, so a machine
/// that can't reach the host doesn't try on every start.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct State {
    catalog_checked: u64,
    catalog_etag: Option<String>,
    updates_checked: u64,
    latest: Option<String>,
}

impl State {
    fn load(dirs: &Dirs) -> Self {
        std::fs::read_to_string(dirs.user_cache.join(STATE_FILE))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn save(&self, dirs: &Dirs) {
        let text = serde_json::to_string(self).unwrap_or_default();
        if let Err(e) = write_atomic(&dirs.user_cache.join(STATE_FILE), &text) {
            tracing::debug!("{e}");
        }
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn due(last: u64) -> bool {
    now().saturating_sub(last) >= INTERVAL.as_secs()
}

fn process_env(key: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(key)
}

/// A catalog that was just written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refreshed {
    pub models: usize,
    pub providers: usize,
}

/// Download the catalog now and write what concerns the configured providers to `out`, or to
/// the cache directory, where the next session reads it.
pub async fn refresh_catalog(config: &Loaded, out: Option<&Path>) -> Result<Refreshed, String> {
    let got = download(config, None).await?;
    let Fetched::Body { bytes, etag } = got else {
        return Err("the catalog server sent no catalog".into());
    };
    let written = store(config, &bytes, out)?;
    if out.is_none() {
        let mut state = State::load(&config.dirs);
        state.catalog_checked = now();
        state.catalog_etag = etag;
        state.save(&config.dirs);
    }
    Ok(written)
}

async fn download(config: &Loaded, etag: Option<&str>) -> Result<Fetched, String> {
    let url = &config.settings.catalog.url;
    if url.is_empty() {
        return Err("no catalog source is set · set catalog.url".into());
    }
    let req = Get {
        url,
        etag,
        follow: true,
        max_bytes: MAX_CATALOG_BYTES,
    };
    let client = fetch::client(&config.settings, &process_env)?;
    fetch::get(&client, &config.settings, req).await
}

/// Check the download before it replaces anything: a bad one leaves the old file in place.
fn store(config: &Loaded, bytes: &[u8], out: Option<&Path>) -> Result<Refreshed, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("the catalog is not text: {e}"))?;
    let mut catalog = Catalog::parse(text)?;
    catalog.retain(&catalog_ids(config));
    let path = out.map_or_else(
        || quack_config::catalog::downloaded_path(&config.dirs),
        Path::to_path_buf,
    );
    write_atomic(&path, &catalog.render())?;
    Ok(Refreshed {
        models: catalog.model_count(),
        providers: catalog.provider_count(),
    })
}

fn catalog_ids(config: &Loaded) -> BTreeSet<String> {
    load_providers(config)
        .iter()
        .filter_map(|item| Some(item.loaded()?.catalog_id(&item.id)?.to_owned()))
        .collect()
}

/// The latest published version, or `None` when the source has published none yet.
pub async fn latest_version(config: &Loaded) -> Result<Option<String>, String> {
    let url = &config.settings.updates.url;
    if url.is_empty() {
        return Err("no update source is set · set updates.url".into());
    }
    let req = Get {
        url,
        etag: None,
        follow: false,
        max_bytes: MAX_VERSION_BYTES,
    };
    let client = fetch::client(&config.settings, &process_env)?;
    Ok(match fetch::get(&client, &config.settings, req).await? {
        Fetched::Moved(to) => version_in_url(&to),
        Fetched::Body { bytes, .. } => {
            let text = String::from_utf8_lossy(&bytes);
            text.lines().next().and_then(version)
        }
        Fetched::NotModified => None,
    })
}

fn version_in_url(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    version(path.trim_end_matches('/').rsplit('/').next()?)
}

/// `v1.2.3` or `1.2.3`, without the `v`. Anything else is refused, since this text comes
/// from the network and ends up on the screen.
fn version(text: &str) -> Option<String> {
    let text = text.trim();
    let text = text.strip_prefix('v').unwrap_or(text);
    let usable = text.len() <= 32
        && text.starts_with(|c: char| c.is_ascii_digit())
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    usable.then(|| text.to_owned())
}

/// Whether `latest` is a later release than `current`, comparing the numbers before any
/// `-rc1` or `+build`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let numbers = |v: &str| -> Vec<u64> {
        v.split(['-', '+'])
            .next()
            .unwrap_or_default()
            .split('.')
            .map(|n| n.parse().unwrap_or(0))
            .collect()
    };
    numbers(latest) > numbers(current)
}

/// Run what the configuration turned on, off to the side of the session.
pub(crate) fn background(config: Arc<Loaded>, tx: broadcast::Sender<Event>) {
    if !wanted_catalog(&config) && !wanted_updates(&config) {
        return;
    }
    tokio::spawn(async move {
        let mut state = State::load(&config.dirs);
        if wanted_catalog(&config) && due(state.catalog_checked) {
            background_catalog(&config, &mut state).await;
        }
        if wanted_updates(&config) {
            if due(state.updates_checked) {
                state.updates_checked = now();
                match latest_version(&config).await {
                    Ok(latest) => state.latest = latest,
                    Err(e) => tracing::debug!("update check: {e}"),
                }
                state.save(&config.dirs);
            }
            if let Some(latest) = state.latest.filter(|l| is_newer(l, CURRENT)) {
                let _ = tx.send(Event::UpdateAvailable {
                    current: CURRENT.to_owned(),
                    latest,
                });
            }
        }
    });
}

fn wanted_catalog(config: &Loaded) -> bool {
    let catalog = &config.settings.catalog;
    catalog.refresh && !catalog.url.is_empty()
}

fn wanted_updates(config: &Loaded) -> bool {
    let updates = &config.settings.updates;
    updates.check && !updates.url.is_empty()
}

async fn background_catalog(config: &Loaded, state: &mut State) {
    state.catalog_checked = now();
    // Without the file there is nothing for "unchanged" to mean.
    let have = quack_config::catalog::downloaded_path(&config.dirs).exists();
    let etag = state.catalog_etag.as_deref().filter(|_| have);
    match download(config, etag).await {
        Ok(Fetched::Body { bytes, etag }) => match store(config, &bytes, None) {
            Ok(_) => state.catalog_etag = etag,
            Err(e) => tracing::debug!("catalog refresh: {e}"),
        },
        Ok(_) => {}
        Err(e) => tracing::debug!("catalog refresh: {e}"),
    }
    state.save(&config.dirs);
}

/// A background check that setup can offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub key: &'static str,
    pub host: String,
}

/// The checks nobody has decided on: there is somewhere to check, the network and the policy
/// allow it, and no one has set the key. An answer of no is a decision too.
pub fn undecided(config: &Loaded) -> Vec<Offer> {
    let s = &config.settings;
    [
        ("catalog.refresh", &s.catalog.url),
        ("updates.check", &s.updates.url),
    ]
    .into_iter()
    .filter(|(key, url)| {
        !url.is_empty()
            && s.check_fetch(url).is_ok()
            && !s.policy.locks(key)
            && match config.origin_of(key) {
                Origin::Default => true,
                Origin::File { layer, .. } => *layer == LayerKind::Defaults,
                _ => false,
            }
    })
    .map(|(key, url)| Offer {
        key,
        host: quack_config::url_host(url).to_owned(),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_versions_and_refuses_anything_else() {
        let url = "https://github.com/o/r/releases/tag/v0.2.0";
        assert_eq!(version_in_url(url).as_deref(), Some("0.2.0"));
        assert_eq!(version("1.2.3-rc1\n").as_deref(), Some("1.2.3-rc1"));
        assert_eq!(version_in_url("https://github.com/o/r/releases"), None);
        assert_eq!(version("\x1b[31m1.0"), None);
        assert_eq!(version("<html>"), None);
    }

    #[test]
    fn compares_numbers_not_text() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0-rc1", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
    }
}
