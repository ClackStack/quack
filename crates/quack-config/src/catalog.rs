//! The model catalog: prices and context windows for models their provider doesn't describe.
//!
//! This is data about models and never says where to send anything: it can only fill in a
//! number, so it is safe to download. The format is models.dev's, so a mirror can serve the
//! original; quack keeps only what it uses. Precedence for a model's facts, highest first:
//! the provider file, the provider's models endpoint, the downloaded catalog
//! (`quack catalog refresh`), the snapshot in the defaults bundle (`catalog.json`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::paths::Dirs;
use crate::provider::{Cost, ProviderFile};

const FILE: &str = "catalog.json";
const SOURCE: &str = "From models.dev (MIT). Regenerate with scripts/update-catalog.sh.";

/// What the catalog says about one model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelFacts {
    pub context_window: Option<u32>,
    pub cost: Option<Cost>,
}

/// One catalog file, by catalog provider id and then model id.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalog {
    providers: BTreeMap<String, BTreeMap<String, ModelFacts>>,
}

impl Catalog {
    /// Read a catalog, skipping any entry it can't use: one odd model in a feed quack doesn't
    /// control must not cost the rest. Fails when nothing at all is usable.
    pub fn parse(text: &str) -> Result<Self, String> {
        let root: Value =
            serde_json::from_str(text).map_err(|e| format!("the catalog is not JSON: {e}"))?;
        let mut providers = BTreeMap::new();
        for (id, provider) in root.as_object().into_iter().flatten() {
            let models: BTreeMap<String, ModelFacts> = provider
                .get("models")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .filter_map(|(id, m)| Some((id.clone(), facts(m)?)))
                .collect();
            if !models.is_empty() {
                providers.insert(id.clone(), models);
            }
        }
        if providers.is_empty() {
            return Err("the catalog lists no models quack can use".into());
        }
        Ok(Self { providers })
    }

    /// Keep only these providers.
    pub fn retain(&mut self, ids: &BTreeSet<String>) {
        self.providers.retain(|id, _| ids.contains(id));
    }

    pub fn model_count(&self) -> usize {
        self.providers.values().map(BTreeMap::len).sum()
    }

    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }

    pub fn get(&self, provider: &str, model: &str) -> Option<&ModelFacts> {
        self.providers.get(provider)?.get(model)
    }

    /// The file form: models.dev's shape, one model per line so changes read well in a diff.
    pub fn render(&self) -> String {
        let quoted = |s: &str| Value::from(s).to_string();
        let mut out = format!("{{\"source\":{}", quoted(SOURCE));
        for (id, models) in &self.providers {
            out.push_str(&format!(",\n{}:{{\"models\":{{", quoted(id)));
            let lines: Vec<String> = models
                .iter()
                .map(|(id, m)| format!("\n{}:{}", quoted(id), entry(m)))
                .collect();
            out.push_str(&lines.join(","));
            out.push_str("\n}}");
        }
        out.push_str("\n}\n");
        out
    }
}

fn facts(model: &Value) -> Option<ModelFacts> {
    let context_window = model
        .pointer("/limit/context")
        .and_then(Value::as_u64)
        .filter(|n| *n > 0)
        .and_then(|n| u32::try_from(n).ok());
    let cost = model.get("cost").and_then(cost);
    (context_window.is_some() || cost.is_some()).then_some(ModelFacts {
        context_window,
        cost,
    })
}

fn cost(cost: &Value) -> Option<Cost> {
    let price = |key: &str| {
        cost.get(key)
            .and_then(Value::as_f64)
            .filter(|p| p.is_finite() && *p >= 0.0)
    };
    Some(Cost {
        input: price("input")?,
        output: price("output")?,
        cached_input: price("cache_read"),
        cache_write: price("cache_write"),
    })
}

fn entry(m: &ModelFacts) -> Value {
    let mut out = json!({});
    if let Some(c) = m.context_window {
        out["limit"] = json!({ "context": c });
    }
    if let Some(c) = &m.cost {
        out["cost"] = json!({ "input": c.input, "output": c.output });
        if let Some(p) = c.cached_input {
            out["cost"]["cache_read"] = p.into();
        }
        if let Some(p) = c.cache_write {
            out["cost"]["cache_write"] = p.into();
        }
    }
    out
}

/// Where `quack catalog refresh` keeps the catalog it downloaded.
pub fn downloaded_path(dirs: &Dirs) -> PathBuf {
    dirs.user_cache.join(FILE)
}

/// The catalogs on this machine. Neither has to exist.
#[derive(Debug, Clone, Default)]
pub struct Catalogs {
    pub downloaded: Option<Catalog>,
    pub bundled: Option<Catalog>,
}

impl Catalogs {
    /// Read both files; one that is missing or damaged counts as absent.
    pub fn load(dirs: &Dirs) -> Self {
        Self {
            downloaded: read(&downloaded_path(dirs)),
            bundled: dirs.defaults.as_ref().and_then(|d| read(&d.join(FILE))),
        }
    }

    /// What is known about a provider's model. Each fact comes from the newest catalog that
    /// has it, and ids match exactly: a wrong price is worse than none.
    pub fn facts(&self, id: &str, file: &ProviderFile, model: &str) -> ModelFacts {
        let Some(provider) = file.catalog_id(id) else {
            return ModelFacts::default();
        };
        let found: Vec<&ModelFacts> = [&self.downloaded, &self.bundled]
            .into_iter()
            .flatten()
            .filter_map(|c| c.get(provider, model))
            .collect();
        ModelFacts {
            context_window: found.iter().find_map(|m| m.context_window),
            cost: found.iter().find_map(|m| m.cost.clone()),
        }
    }
}

fn read(path: &Path) -> Option<Catalog> {
    Catalog::parse(&std::fs::read_to_string(path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"{
        "source": "not a provider",
        "acme": { "name": "Acme", "api": "https://acme.example", "models": {
            "big": { "name": "Big", "limit": { "context": 1000, "output": 10 },
                     "cost": { "input": 1.5, "output": 3, "cache_read": 0.15, "reasoning": 9 } },
            "windowless": { "cost": { "input": 1, "output": 2 } },
            "priceless": { "limit": { "context": 500 } },
            "negative": { "cost": { "input": -1, "output": 2 } },
            "odd": { "limit": "lots", "cost": "free" }
        } },
        "empty": { "models": {} }
    }"#;

    fn file(extra: &str) -> ProviderFile {
        toml::from_str(&format!("base_url = \"https://acme.example/v1\"\n{extra}")).expect("toml")
    }

    #[test]
    fn keeps_what_it_can_use_and_skips_the_rest() {
        let c = Catalog::parse(FEED).expect("parse");
        assert_eq!((c.provider_count(), c.model_count()), (1, 3));
        let big = c.get("acme", "big").expect("big");
        assert_eq!(big.context_window, Some(1000));
        let cost = big.cost.as_ref().expect("cost");
        assert_eq!((cost.input, cost.cached_input), (1.5, Some(0.15)));
        assert_eq!(cost.cache_write, None, "unset means the input price");
        assert!(c.get("acme", "negative").is_none());
        assert!(c.get("acme", "odd").is_none());
    }

    #[test]
    fn a_catalog_with_nothing_usable_is_an_error() {
        assert!(Catalog::parse("<html>").is_err());
        assert!(Catalog::parse("{\"acme\":{\"models\":{}}}").is_err());
    }

    #[test]
    fn its_file_form_reads_back_and_names_no_hosts() {
        let c = Catalog::parse(FEED).expect("parse");
        let text = c.render();
        assert_eq!(Catalog::parse(&text).expect("again"), c);
        assert!(!text.contains("acme.example"), "{text}");
    }

    #[test]
    fn newer_catalog_wins_fact_by_fact() {
        let downloaded = r#"{"acme":{"models":{"big":{"cost":{"input":9,"output":9}}}}}"#;
        let bundled = r#"{"acme":{"models":{"big":{"limit":{"context":7},
            "cost":{"input":1,"output":1}},"old":{"limit":{"context":5}}}}}"#;
        let c = Catalogs {
            downloaded: Catalog::parse(downloaded).ok(),
            bundled: Catalog::parse(bundled).ok(),
        };
        let big = c.facts("acme", &file(""), "big");
        assert_eq!(big.context_window, Some(7));
        assert_eq!(big.cost.expect("cost").input, 9.0);
        assert_eq!(c.facts("acme", &file(""), "old").context_window, Some(5));
        assert_eq!(c.facts("acme", &file(""), "bi"), ModelFacts::default());
    }

    #[test]
    fn the_provider_decides_its_catalog_id() {
        let c = Catalogs {
            bundled: Catalog::parse(FEED).ok(),
            ..Catalogs::default()
        };
        let known = |id, f: &ProviderFile| c.facts(id, f, "big") != ModelFacts::default();
        assert!(known("acme", &file("")));
        assert!(known("mine", &file("catalog = \"acme\"")));
        assert!(!known("mine", &file("")));
        assert!(!known("acme", &file("catalog = \"\"")));
        let local: ProviderFile =
            toml::from_str("base_url = \"http://127.0.0.1:8080/v1\"").expect("toml");
        assert!(
            !known("acme", &local),
            "a local server serves its own window"
        );
    }
}
