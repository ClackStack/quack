//! The model catalog and the version check: where a model's facts come from, and that the
//! background checks do what they were asked and nothing more.

mod common;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use common::{sse_raw, turn};
use quack_config::{LoadOptions, Loaded, load_with};
use quack_core::checks::{self, Refreshed};
use quack_core::protocol::{Event, ModelInfo, Op};
use quack_core::{Engine, EngineHandle};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FEED: &str = r#"{
    "acme": { "api": "https://acme.example/v1", "models": {
        "tiny": { "limit": { "context": 4096 }, "cost": { "input": 2, "output": 8 } } } },
    "stranger": { "models": { "other": { "limit": { "context": 1 } } } }
}"#;

/// Isolated directories with one provider, `mock`, known to the catalog as `acme`.
fn load(root: &Path, base_url: &str, provider: &str, config: &str) -> Loaded {
    let user = root.join("user");
    std::fs::create_dir_all(user.join("providers.d")).expect("mkdir");
    std::fs::write(
        user.join("providers.d").join("mock.toml"),
        format!("base_url = \"{base_url}/v1\"\ncatalog = \"acme\"\n{provider}\n"),
    )
    .expect("provider");
    std::fs::write(user.join("config.toml"), config).expect("config");
    let mut vars: HashMap<String, OsString> = HashMap::new();
    vars.insert("QUACK_CONFIG_DIR".into(), user.into_os_string());
    for (k, sub) in [
        ("QUACK_SYSTEM_DIR", "system"),
        ("QUACK_DATA_DIR", "data"),
        ("QUACK_CACHE_DIR", "cache"),
    ] {
        vars.insert(k.into(), root.join(sub).into_os_string());
    }
    vars.insert("QUACK_DEFAULTS_DIR".into(), "none".into());
    let ws = root.join("ws");
    std::fs::create_dir_all(&ws).expect("mkdir");
    load_with(&move |k| vars.get(k).cloned(), &LoadOptions::new(ws)).expect("load")
}

fn downloaded(root: &Path) -> PathBuf {
    root.join("cache").join("catalog.json")
}

fn state(root: &Path) -> PathBuf {
    root.join("cache").join("checks.json")
}

fn put_catalog(root: &Path, text: &str) {
    std::fs::create_dir_all(root.join("cache")).expect("mkdir");
    std::fs::write(downloaded(root), text).expect("catalog");
}

async fn until_exists(file: &Path) {
    for _ in 0..500 {
        if file.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{} never appeared", file.display());
}

/// A server that answers chat requests, reporting 3000 prompt and 500 completion tokens.
async fn chat_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse_raw(&[
            r#"{"choices":[{"delta":{"content":"Hi."},"finish_reason":"stop"}]}"#.into(),
            r#"{"choices":[],"usage":{"prompt_tokens":3000,"completion_tokens":500}}"#.into(),
        ]))
        .mount(&server)
        .await;
    server
}

async fn session_cost(engine: &mut EngineHandle) -> Option<f64> {
    turn(engine, "hello").await;
    engine.submit(Op::GetStats).await;
    loop {
        let e = tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
            .await
            .expect("event within 10s")
            .expect("channel open");
        if let Event::Stats { report } = e {
            return report.cost;
        }
    }
}

async fn listed(engine: &mut EngineHandle) -> Vec<ModelInfo> {
    engine.submit(Op::ListModels).await;
    loop {
        let e = tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
            .await
            .expect("event within 10s")
            .expect("channel open");
        if let Event::ModelCatalog { providers, .. } = e {
            return providers.into_iter().flat_map(|p| p.models).collect();
        }
    }
}

/// 3000 in at $2 and 500 out at $8, per million.
const CATALOG_COST: f64 = (6000.0 + 4000.0) / 1e6;

#[tokio::test]
async fn the_catalog_prices_a_model_nothing_else_describes() {
    let server = chat_server().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    put_catalog(tmp.path(), FEED);
    let config = load(tmp.path(), &server.uri(), "models = [\"tiny\"]", "");
    let mut engine = Engine::spawn(Arc::new(config));
    let cost = session_cost(&mut engine).await.expect("priced");
    assert!((cost - CATALOG_COST).abs() < 1e-12, "{cost}");
    let models = listed(&mut engine).await;
    assert_eq!(models[0].context_window, Some(4096));
    assert_eq!(
        models[0].price.as_deref(),
        Some("$2.00 in · $8.00 out per 1M tokens")
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn the_provider_file_beats_the_catalog() {
    let server = chat_server().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    put_catalog(tmp.path(), FEED);
    let models = "[[models]]\nid = \"tiny\"\ncontext_window = 9000\n\
                  cost = { input = 1.0, output = 1.0 }";
    let config = load(tmp.path(), &server.uri(), models, "");
    let mut engine = Engine::spawn(Arc::new(config));
    let cost = session_cost(&mut engine).await.expect("priced");
    assert!((cost - 3500.0 / 1e6).abs() < 1e-12, "{cost}");
    let models = listed(&mut engine).await;
    assert_eq!(models[0].context_window, Some(9000));
    assert!(
        models[0]
            .price
            .as_deref()
            .is_some_and(|p| p.starts_with("$1.00"))
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn declared_thinking_levels_reach_the_model_catalog() {
    let server = chat_server().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let models = "[[models]]\nid = \"tiny\"\nreasoning = { efforts = [\"low\", \"high\"] }";
    let config = load(tmp.path(), &server.uri(), models, "");
    let mut engine = Engine::spawn(Arc::new(config));
    let models = listed(&mut engine).await;
    assert_eq!(
        models[0].reasoning_efforts,
        vec!["low".to_owned(), "high".to_owned()]
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn the_models_endpoint_beats_the_catalog() {
    let server = chat_server().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"data":[{"id":"tiny","context_window":32768,"pricing":{"input":1.0,"output":1.0}}]}"#,
        ))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    put_catalog(tmp.path(), FEED);
    let config = load(tmp.path(), &server.uri(), "models = [\"tiny\"]", "");
    let mut engine = Engine::spawn(Arc::new(config));
    let cost = session_cost(&mut engine).await.expect("priced");
    assert!((cost - 3500.0 / 1e6).abs() < 1e-12, "{cost}");
    assert_eq!(listed(&mut engine).await[0].context_window, Some(32768));
    engine.shutdown().await;
}

#[tokio::test]
async fn refreshing_keeps_only_the_configured_providers() {
    let server = MockServer::start().await;
    Mock::given(path("/api.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(FEED))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let url = format!("{}/api.json", server.uri());
    let config = load(
        tmp.path(),
        "http://127.0.0.1:9",
        "",
        &format!("[catalog]\nurl = \"{url}\"\n"),
    );
    let got = checks::refresh_catalog(&config, None)
        .await
        .expect("refresh");
    assert_eq!(
        got,
        Refreshed {
            models: 1,
            providers: 1
        }
    );
    let text = std::fs::read_to_string(downloaded(tmp.path())).expect("written");
    assert!(
        text.contains("acme") && !text.contains("stranger"),
        "{text}"
    );
    assert!(!text.contains("acme.example"), "no hosts are kept: {text}");

    let out = tmp.path().join("snapshot.json");
    checks::refresh_catalog(&config, Some(&out))
        .await
        .expect("refresh");
    assert_eq!(std::fs::read_to_string(out).expect("written"), text);
}

#[tokio::test]
async fn a_bad_download_leaves_the_old_catalog() {
    let server = MockServer::start().await;
    Mock::given(path("/api.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>sorry</html>"))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    put_catalog(tmp.path(), FEED);
    let url = format!("{}/api.json", server.uri());
    let config = load(
        tmp.path(),
        "http://127.0.0.1:9",
        "",
        &format!("[catalog]\nurl = \"{url}\"\n"),
    );
    assert!(checks::refresh_catalog(&config, None).await.is_err());
    let text = std::fs::read_to_string(downloaded(tmp.path())).expect("kept");
    assert_eq!(text, FEED);
}

#[tokio::test]
async fn nothing_is_fetched_unless_a_check_is_on() {
    let server = chat_server().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = format!(
        "[catalog]\nurl = \"{0}/api.json\"\n[updates]\nurl = \"{0}/latest\"\n",
        server.uri()
    );
    let loaded = load(tmp.path(), &server.uri(), "models = [\"tiny\"]", &config);
    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(&mut engine, "hello").await;
    engine.shutdown().await;
    let requests = server.received_requests().await.unwrap_or_default();
    assert!(
        requests.iter().all(|r| r.url.path().starts_with("/v1/")),
        "{requests:?}"
    );
    assert!(!state(tmp.path()).exists());
}

#[tokio::test]
async fn background_checks_run_at_most_once_a_day() {
    let server = chat_server().await;
    Mock::given(path("/api.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(FEED))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = format!(
        "[catalog]\nrefresh = true\nurl = \"{}/api.json\"\n",
        server.uri()
    );
    for _ in 0..2 {
        let loaded = load(tmp.path(), &server.uri(), "models = [\"tiny\"]", &config);
        let mut engine = Engine::spawn(Arc::new(loaded));
        turn(&mut engine, "hello").await;
        until_exists(&state(tmp.path())).await;
        engine.shutdown().await;
    }
    assert!(downloaded(tmp.path()).exists());
    let requests = server.received_requests().await.unwrap_or_default();
    let feeds = requests.iter().filter(|r| r.url.path() == "/api.json");
    assert_eq!(feeds.count(), 1);
}

async fn next_update(engine: &mut EngineHandle) -> Option<(String, String)> {
    let wait = Duration::from_secs(2);
    while let Ok(e) = tokio::time::timeout(wait, engine.events.recv()).await {
        if let Ok(Event::UpdateAvailable { current, latest }) = e {
            return Some((current, latest));
        }
    }
    None
}

#[tokio::test]
async fn a_newer_release_is_announced_and_remembered_for_the_next_start() {
    let server = chat_server().await;
    Mock::given(path("/releases/latest"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/releases/tag/v99.0.0"))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = format!(
        "[updates]\ncheck = true\nurl = \"{}/releases/latest\"\n",
        server.uri()
    );
    for _ in 0..2 {
        let loaded = load(tmp.path(), &server.uri(), "models = [\"tiny\"]", &config);
        let mut engine = Engine::spawn(Arc::new(loaded));
        let (current, latest) = next_update(&mut engine).await.expect("announced");
        assert_eq!(
            (current.as_str(), latest.as_str()),
            (checks::CURRENT, "99.0.0")
        );
        engine.shutdown().await;
    }
    let requests = server.received_requests().await.unwrap_or_default();
    let asked = requests
        .iter()
        .filter(|r| r.url.path().contains("releases"));
    assert_eq!(asked.count(), 1, "the redirect is read, not followed");
}

#[tokio::test]
async fn an_older_or_absent_release_says_nothing() {
    for location in ["/releases/tag/v0.0.0-rc1", "/releases"] {
        let server = chat_server().await;
        Mock::given(path("/releases/latest"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", location))
            .mount(&server)
            .await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = format!(
            "[updates]\ncheck = true\nurl = \"{}/releases/latest\"\n",
            server.uri()
        );
        let loaded = load(tmp.path(), &server.uri(), "models = [\"tiny\"]", &config);
        let mut engine = Engine::spawn(Arc::new(loaded));
        assert_eq!(next_update(&mut engine).await, None, "{location}");
        engine.shutdown().await;
    }
}
