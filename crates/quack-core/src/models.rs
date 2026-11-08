//! The model catalogue for `/models`: every configured provider, its declared models, and
//! what its models endpoint reports. Discovery calls run in parallel with a short timeout.

use std::sync::Arc;
use std::time::Duration;

use quack_config::provider::Cost;
use quack_config::{
    Catalogs, ItemState, LayerKind, Loaded, ProviderFile, glob_match, load_providers,
};
use quack_protocol::{ModelInfo, ProviderModels};
use quack_providers::Provider;
use tokio::task::JoinSet;

const CATALOG_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) async fn catalog(config: Arc<Loaded>) -> Vec<ProviderModels> {
    catalog_of(config, None).await
}

/// The catalogue of every provider, or only of `only`.
pub(crate) async fn catalog_of(config: Arc<Loaded>, only: Option<&str>) -> Vec<ProviderModels> {
    let cfg = config.clone();
    let (items, catalogs) =
        tokio::task::spawn_blocking(move || (load_providers(&cfg), Catalogs::load(&cfg.dirs)))
            .await
            .unwrap_or_default();
    let catalogs = Arc::new(catalogs);
    let mut tasks = JoinSet::new();
    let items = items
        .into_iter()
        .filter(|i| only.is_none_or(|id| i.id == id));
    for (order, item) in items.enumerate() {
        let network = config.settings.network.clone();
        let catalogs = catalogs.clone();
        tasks.spawn(async move {
            let bundled = item.layer == LayerKind::Defaults;
            let key = item
                .loaded()
                .filter(|f| f.auth.is_none())
                .and_then(|f| f.api_key.as_ref())
                .map(crate::setup::key_source);
            let mut entry = match item.state {
                ItemState::Loaded(file) => probe(&item.id, file, network, &catalogs).await,
                ItemState::Disabled(why) => unavailable(&item.id, why),
                ItemState::Failed(d) => unavailable(&item.id, d.one_line()),
            };
            entry.bundled = bundled;
            entry.key = key;
            (order, entry)
        });
    }
    let mut out: Vec<(usize, ProviderModels)> = Vec::new();
    while let Some(Ok(r)) = tasks.join_next().await {
        out.push(r);
    }
    // Usable providers first, each group in file order.
    out.sort_by_key(|(order, p)| (!p.available, *order));
    out.into_iter().map(|(_, p)| p).collect()
}

fn unavailable(id: &str, note: String) -> ProviderModels {
    ProviderModels {
        id: id.to_owned(),
        available: false,
        note,
        models: Vec::new(),
        bundled: false,
        key: None,
    }
}

/// A price as `/models` shows it.
fn price(c: &Cost) -> String {
    format!(
        "{} in · {} out per 1M tokens",
        dollars(c.input),
        dollars(c.output)
    )
}

/// Two decimals, or as many as the price needs.
fn dollars(amount: f64) -> String {
    let cents = format!("{amount:.2}");
    if cents.parse() == Ok(amount) {
        format!("${cents}")
    } else {
        format!("${amount}")
    }
}

async fn probe(
    id: &str,
    file: ProviderFile,
    network: quack_config::settings::NetworkSettings,
    catalogs: &Catalogs,
) -> ProviderModels {
    if let Some(quack_config::Secret::Env { env, .. }) = &file.api_key
        && std::env::var_os(env).is_none()
    {
        return unavailable(id, format!("set {env} to use this provider"));
    }
    let declared: Vec<ModelInfo> = file
        .models
        .iter()
        .map(|m| ModelInfo {
            id: m.id.clone(),
            alias: m.alias.clone(),
            context_window: m.context_window,
            price: m.cost.as_ref().map(price),
        })
        .collect();
    let include = file.discovery.include.clone();
    let described = file.clone();
    let pid = id.to_owned();
    let built = tokio::task::spawn_blocking(move || {
        Provider::new(&pid, file, &network, &|k| std::env::var_os(k))
    })
    .await;
    let provider = match built {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return unavailable(id, e.message),
        Err(e) => return unavailable(id, e.to_string()),
    };
    if let Some(why) = provider.not_ready() {
        return unavailable(id, why);
    }
    let (discovered, note) =
        match tokio::time::timeout(CATALOG_TIMEOUT, provider.list_models()).await {
            Ok(Ok(list)) => (list, String::new()),
            Ok(Err(e)) => (Vec::new(), e.message),
            Err(_) => (Vec::new(), "listing models timed out".to_owned()),
        };
    let mut models = declared;
    for d in discovered {
        if !include.is_empty() && !include.iter().any(|g| glob_match(g, &d.id)) {
            continue;
        }
        match models.iter_mut().find(|m| m.id == d.id) {
            Some(m) => {
                m.context_window = m.context_window.or(d.context_window);
                m.price = m.price.take().or(d.cost.as_ref().map(price));
            }
            None => models.push(ModelInfo {
                id: d.id,
                alias: None,
                context_window: d.context_window,
                price: d.cost.as_ref().map(price),
            }),
        }
    }
    for m in &mut models {
        let facts = catalogs.facts(id, &described, &m.id);
        m.context_window = m.context_window.or(facts.context_window);
        m.price = m.price.take().or(facts.cost.as_ref().map(price));
    }
    let available = !models.is_empty();
    let note = match (available, note.is_empty()) {
        (true, true) if models.len() == 1 => "1 model".to_owned(),
        (true, true) => format!("{} models", models.len()),
        (true, false) => format!("{} declared models; discovery failed: {note}", models.len()),
        (false, true) => "no models listed or discovered".to_owned(),
        (false, false) => note,
    };
    ProviderModels {
        id: id.to_owned(),
        available,
        note,
        models,
        bundled: false,
        key: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_read_like_money() {
        let c = Cost {
            input: 0.3,
            output: 0.015,
            ..Cost::default()
        };
        assert_eq!(price(&c), "$0.30 in · $0.015 out per 1M tokens");
    }
}
