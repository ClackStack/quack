//! First-run setup: detecting a server, adding it as a provider, and using it.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{sse_text, turn};
use quack_core::protocol::{Event, Op};
use quack_core::setup::{Candidate, detect_in};
use quack_core::{Engine, EngineHandle};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn next_matching(engine: &mut EngineHandle, want: impl Fn(&Event) -> bool) -> Event {
    loop {
        let e = tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
            .await
            .expect("event within 10s")
            .expect("channel open");
        if let Event::Error { error } = &e {
            panic!("engine error: {}", error.message);
        }
        if want(&e) {
            return e;
        }
    }
}

#[tokio::test]
async fn detected_server_becomes_a_provider_and_the_default_model() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": [{"id": "tiny"}]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse_text("hello"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = common::workspace(tmp.path(), &server.uri(), "[]", "");
    // Start with no providers at all.
    let providers_d = tmp.path().join("user").join("providers.d");
    std::fs::remove_file(providers_d.join("mock.toml")).expect("rm");
    let config = Arc::new(loaded);

    let base_url = format!("{}/v1", server.uri());
    let candidates = vec![
        Candidate {
            id: "llama-cpp".into(),
            name: "llama.cpp".into(),
            base_url: base_url.clone(),
            api: None,
            note: String::new(),
        },
        // Nothing listens here.
        Candidate {
            id: "vllm".into(),
            name: "vLLM".into(),
            base_url: "http://127.0.0.1:9/v1".into(),
            api: None,
            note: String::new(),
        },
    ];
    let found = detect_in(config.clone(), candidates.clone()).await;
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].id, "llama-cpp");
    assert_eq!(found[0].models[0].id, "tiny");
    assert_eq!(found[0].configured, None);

    let mut engine = Engine::spawn(config.clone());
    next_matching(&mut engine, |e| matches!(e, Event::StartupComplete)).await;
    engine
        .submit(Op::AddProvider {
            id: "llama-cpp".into(),
            base_url: base_url.clone(),
            api: None,
            model: "tiny".into(),
        })
        .await;
    let added = next_matching(&mut engine, |e| matches!(e, Event::ProviderAdded { .. })).await;
    assert!(
        matches!(&added, Event::ProviderAdded { id, path: Some(_), note: None } if id == "llama-cpp"),
        "{added:?}"
    );
    let text = std::fs::read_to_string(providers_d.join("llama-cpp.toml")).expect("file");
    assert_eq!(
        text,
        format!("# Added by setup.\nbase_url = \"{base_url}\"\n")
    );
    let configured = next_matching(&mut engine, |e| {
        matches!(e, Event::SessionConfigured { .. })
    })
    .await;
    let Event::SessionConfigured { session } = configured else {
        unreachable!()
    };
    assert_eq!(session.model.as_deref(), Some("llama-cpp/tiny"));
    let user_config =
        std::fs::read_to_string(tmp.path().join("user").join("config.toml")).expect("config");
    assert!(
        user_config.contains("default = \"llama-cpp/tiny\""),
        "{user_config}"
    );

    turn(&mut engine, "hi").await;

    // Adding the same server again reuses the provider.
    engine
        .submit(Op::AddProvider {
            id: "llama-cpp".into(),
            base_url: base_url.clone(),
            api: None,
            model: "tiny".into(),
        })
        .await;
    let again = next_matching(&mut engine, |e| matches!(e, Event::ProviderAdded { .. })).await;
    assert!(
        matches!(&again, Event::ProviderAdded { id, path: None, .. } if id == "llama-cpp"),
        "{again:?}"
    );
    engine.shutdown().await;

    let found = detect_in(config, candidates).await;
    assert_eq!(found[0].configured.as_deref(), Some("llama-cpp"));
}

#[tokio::test]
async fn a_server_added_by_url_lists_its_models_and_a_service_waits_for_its_key() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": [{"id": "tiny"}]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse_text("hello"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = common::workspace(tmp.path(), &server.uri(), "[]", "");
    let providers_d = tmp.path().join("user").join("providers.d");
    std::fs::remove_file(providers_d.join("mock.toml")).expect("rm");
    // A hosted service whose key is read from a variable nobody sets.
    std::fs::write(
        providers_d.join("example.toml"),
        "name = \"Example\"\ngroup = \"Hosted\"\nbase_url = \"https://api.example.com/v1\"\n\
         api_key = { env = \"QUACK_TEST_UNSET_KEY\" }\n",
    )
    .expect("write");
    let mut engine = Engine::spawn(Arc::new(loaded));
    next_matching(&mut engine, |e| matches!(e, Event::StartupComplete)).await;

    engine.submit(Op::ListServices).await;
    let Event::Services { services } =
        next_matching(&mut engine, |e| matches!(e, Event::Services { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(services.len(), 1, "{services:?}");
    assert_eq!(services[0].id, "example");
    assert_eq!(services[0].env.as_deref(), Some("QUACK_TEST_UNSET_KEY"));
    assert!(!services[0].ready);

    // A server with no key: the file names only its URL, and its models follow.
    let base_url = format!("{}/v1", server.uri());
    engine
        .submit(Op::AddServer {
            base_url: base_url.clone(),
            key: None,
        })
        .await;
    let Event::ProviderAdded { id, path, note } =
        next_matching(&mut engine, |e| matches!(e, Event::ProviderAdded { .. })).await
    else {
        unreachable!()
    };
    let port = server.address().port();
    assert_eq!(id, format!("server-{port}"));
    assert!(path.is_some());
    assert_eq!(note, None, "no key, so nothing to say about the store");
    let text = std::fs::read_to_string(providers_d.join(format!("{id}.toml"))).expect("file");
    assert_eq!(
        text,
        format!("# Added by setup.\nbase_url = \"{base_url}\"\n")
    );
    let Event::ModelCatalog { providers, .. } =
        next_matching(&mut engine, |e| matches!(e, Event::ModelCatalog { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(providers.len(), 1, "only the new provider: {providers:?}");
    assert!(providers[0].available, "{providers:?}");
    assert_eq!(providers[0].models[0].id, "tiny");

    // Picking the model starts the session on it.
    engine
        .submit(Op::SetModel {
            provider: quack_core::protocol::ProviderId(id.clone()),
            model: quack_core::protocol::ModelId("tiny".into()),
        })
        .await;
    next_matching(&mut engine, |e| {
        matches!(e, Event::SessionConfigured { .. })
    })
    .await;
    turn(&mut engine, "hi").await;

    // A bad URL and an unknown service are refused before anything is stored.
    for op in [
        Op::AddServer {
            base_url: "gpu-box:8000".into(),
            key: None,
        },
        Op::SetProviderKey {
            id: "nope".into(),
            key: quack_core::protocol::SecretText("k".into()),
        },
    ] {
        engine.submit(op).await;
        let e = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(Event::Error { error }) = engine.events.recv().await {
                    return error.message;
                }
            }
        })
        .await
        .expect("an error");
        assert!(e.starts_with("cannot add the provider"), "{e}");
    }
    engine.shutdown().await;
}

#[test]
fn a_new_key_replaces_a_stored_one_or_moves_the_key_to_the_store() {
    use quack_core::setup::set_provider_key_with;
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = common::workspace(tmp.path(), "http://127.0.0.1:9/v1", "[]", "");
    let providers_d = tmp.path().join("user").join("providers.d");
    let stored = "base_url = \"https://api.example.com/v1\"\n\
                  api_key = { keyring = \"example/me\" }\n";
    std::fs::write(providers_d.join("stored.toml"), stored).expect("write");
    std::fs::write(
        providers_d.join("from-env.toml"),
        "base_url = \"https://api.example.org/v1\"\n\
         api_key = { env = \"QUACK_TEST_UNSET_KEY\" }\n",
    )
    .expect("write");
    let mut saved: Vec<(String, String)> = Vec::new();
    let mut store = |name: &str, key: &str| {
        saved.push((name.to_owned(), key.to_owned()));
        Ok(())
    };

    // Already in the store: replaced under its own name, and the file is left alone.
    let out = set_provider_key_with(&loaded, "stored", "new-key", &mut store).expect("stored");
    assert_eq!(out.path, None);
    assert_eq!(out.note, None);
    assert_eq!(
        std::fs::read_to_string(providers_d.join("stored.toml")).expect("file"),
        stored
    );

    // Read from elsewhere: stored under the provider's id, and the file points at it.
    let out = set_provider_key_with(&loaded, "from-env", "k2", &mut store).expect("moved");
    assert_eq!(out.path, Some(providers_d.join("from-env.toml")));
    assert_eq!(out.note, None);
    let text = std::fs::read_to_string(providers_d.join("from-env.toml")).expect("file");
    assert!(
        text.contains("api_key = { keyring = \"from-env\" }"),
        "{text}"
    );

    // Nothing is stored for an unknown provider or an empty key.
    for (id, key) in [("nope", "k"), ("stored", "")] {
        set_provider_key_with(&loaded, id, key, &mut store).expect_err("refused");
    }
    assert_eq!(
        saved,
        [
            ("example/me".to_owned(), "new-key".to_owned()),
            ("from-env".to_owned(), "k2".to_owned())
        ]
    );
}

#[test]
fn a_key_without_a_secret_service_goes_into_a_file_the_provider_points_at() {
    use quack_core::setup::{KeyStore, save_key_at_with};
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = common::workspace(tmp.path(), "http://127.0.0.1:9/v1", "[]", "");
    let providers_d = tmp.path().join("user").join("providers.d");
    let example = "base_url = \"https://api.example.com/v1\"\n\
                   api_key = { env = \"QUACK_TEST_UNSET_KEY\" }\n";
    std::fs::write(providers_d.join("example.toml"), example).expect("write");
    // A provider that already reads its key from a file keeps that file. The fixture
    // goes through `with_file_key` because a Windows path in a basic string would be
    // invalid TOML (its `\` are escape starts).
    let their_file = tmp.path().join("elsewhere").join("custom.key");
    let keyed = quack_config::provider::with_file_key(
        "base_url = \"https://api.example.org/v1\"\n",
        &their_file.display().to_string(),
    )
    .expect("fixture");
    std::fs::write(providers_d.join("keyed.toml"), &keyed).expect("write");
    let reason = "no Secret Service is running".to_owned();
    let no_ring = |_name: &str, _key: &str| {
        panic!("the keyring must not be touched when the store is a file")
    };

    let key_file = tmp.path().join("user").join("keys").join("example.key");
    let store = (KeyStore::File(key_file.clone()), Some(reason.clone()));
    let out = save_key_at_with(&loaded, "example", "k1", &store, no_ring).expect("stored");
    assert_eq!(std::fs::read_to_string(&key_file).expect("key file"), "k1");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = key_file.metadata().expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "only this user reads the key");
    }
    assert_eq!(
        out.note.as_deref(),
        Some(
            format!(
                "{reason}; the key is stored in {} instead of the OS credential store",
                key_file.display()
            )
            .as_str()
        )
    );
    // The quoting of the path depends on the platform (a literal string on Windows,
    // where backslashes cannot go into a basic string unescaped), so check the parsed
    // file instead of its text.
    let text = std::fs::read_to_string(providers_d.join("example.toml")).expect("file");
    let pointed: quack_config::ProviderFile = toml::from_str(&text).expect("parses");
    assert_eq!(
        pointed.api_key,
        Some(quack_config::Secret::File {
            file: key_file.display().to_string()
        }),
        "{text}"
    );

    let store = (
        KeyStore::File(tmp.path().join("user").join("keys").join("keyed.key")),
        Some(reason),
    );
    let out = save_key_at_with(&loaded, "keyed", "k2", &store, no_ring).expect("stored");
    assert_eq!(out.path, None, "the provider file stays as it is");
    assert_eq!(
        std::fs::read_to_string(&their_file).expect("their key file"),
        "k2"
    );
    let text = std::fs::read_to_string(providers_d.join("keyed.toml")).expect("file");
    assert_eq!(text, keyed);
}

#[test]
fn add_server_without_a_secret_service_writes_the_key_file() {
    use quack_core::setup::add_server;
    // When `secret-tool` is missing or has no service to talk to, the key goes to a
    // file. With a real store the key would be touched, so this only checks headless.
    if quack_config::keyring_available().is_ok() {
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = common::workspace(tmp.path(), "http://127.0.0.1:9/v1", "[]", "");
    let key = "sk-test";
    let added = add_server(&loaded, "https://api.example.com/v1", Some(key)).expect("added");
    assert_eq!(added.id, "example");
    let text = std::fs::read_to_string(
        tmp.path()
            .join("user")
            .join("providers.d")
            .join("example.toml"),
    )
    .expect("file");
    let key_file = tmp.path().join("user").join("keys").join("example.key");
    let pointed: quack_config::ProviderFile = toml::from_str(&text).expect("parses");
    assert_eq!(
        pointed.api_key,
        Some(quack_config::Secret::File {
            file: key_file.display().to_string()
        }),
        "{text}"
    );
    assert_eq!(std::fs::read_to_string(&key_file).expect("key file"), key);
    assert!(
        added
            .note
            .as_deref()
            .unwrap_or_default()
            .contains("the key is stored in")
    );
}

/// A subscription stays in Hosted services even when its issuer is a loopback test server.
fn subscription_config(root: &std::path::Path, issuer: &str) -> quack_config::Loaded {
    let config = common::workspace(root, issuer, "[]", "");
    let dir = root.join("user").join("providers.d");
    std::fs::remove_file(dir.join("mock.toml")).expect("remove mock");
    std::fs::write(
        dir.join("plan.toml"),
        format!(
            "name = \"Test plan\"\napi = \"openai-responses\"\nbase_url = \"{issuer}/v1\"\n\
         auth = {{ type = \"chatgpt\", issuer = \"{issuer}\" }}\nmodels = [\"coder\"]\n"
        ),
    )
    .expect("provider");
    config
}

#[test]
fn subscription_readiness_requires_refresh_credentials_and_plan_consent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = subscription_config(tmp.path(), "https://plan.example");
    // Core-only builds can omit Responses and subscription support.
    if let Err(error) = quack_core::setup::sign_in_provider(&config, "plan") {
        assert!(error.contains("this build has no"), "{error}");
        return;
    }
    let services = quack_core::setup::service_infos(&config);
    assert_eq!(services.len(), 1);
    assert!(services[0].subscription);
    assert!(!services[0].ready);
    assert!(services[0].key.is_none() && services[0].env.is_none());
    let auth = config.dirs.user_data.join("auth");
    std::fs::create_dir_all(&auth).expect("auth dir");
    for (refresh, scopes, ready) in [
        ("refresh", vec!["openid"], false),
        ("refresh", vec!["chatgpt.tokens.use.direct"], true),
        ("", vec!["chatgpt.tokens.use.direct"], false),
    ] {
        // An expired access token is usable when its refresh token and consent remain.
        std::fs::write(
            auth.join("plan.json"),
            serde_json::json!({
                "client_id": "test-app", "subject": "user", "email": "test@example.com",
                "refresh_token": refresh, "expires_at": 0, "scopes": scopes,
            })
            .to_string(),
        )
        .expect("credentials");
        assert_eq!(quack_core::setup::service_infos(&config)[0].ready, ready);
    }
    std::fs::write(auth.join("plan.json"), "broken JSON").expect("corrupt credentials");
    assert!(!quack_core::setup::service_infos(&config)[0].ready);
}

#[tokio::test]
async fn subscription_sign_in_reports_decline_and_cancellation_closes_the_listener() {
    use quack_providers::reqwest;
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = subscription_config(tmp.path(), &server.uri());
    if let Err(error) = quack_core::setup::sign_in_provider(&config, "plan") {
        assert!(error.contains("this build has no"), "{error}");
        return;
    }
    let mut engine = Engine::spawn(Arc::new(config));
    next_matching(&mut engine, |e| matches!(e, Event::StartupComplete)).await;
    for cancel in [false, true] {
        engine
            .submit(Op::SignIn {
                id: Some("plan".into()),
            })
            .await;
        let Event::SignInUrl { id, url } =
            next_matching(&mut engine, |e| matches!(e, Event::SignInUrl { .. })).await
        else {
            unreachable!()
        };
        assert_eq!(id, "plan");
        let url = reqwest::Url::parse(&url).expect("authorize URL");
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        let mut back = reqwest::Url::parse(&query["redirect_uri"]).expect("callback URL");
        if cancel {
            engine.submit(Op::SignIn { id: None }).await;
            // The next operation is answered after the sign-in task has been dropped.
            engine.submit(Op::ListServices).await;
            next_matching(&mut engine, |e| matches!(e, Event::Services { .. })).await;
            assert!(
                reqwest::get(back).await.is_err(),
                "cancel must close the callback listener"
            );
        } else {
            back.query_pairs_mut()
                .append_pair("state", &query["state"])
                .append_pair("error", "access_denied");
            reqwest::get(back).await.expect("mock browser callback");
            let result =
                next_matching(&mut engine, |e| matches!(e, Event::SignInResult { .. })).await;
            assert!(
                matches!(result, Event::SignInResult { error: Some(e), .. } if e.contains("declined"))
            );
        }
    }
    engine.shutdown().await;
    assert!(
        server
            .received_requests()
            .await
            .expect("requests")
            .is_empty(),
        "no token exchange or inference"
    );
}
