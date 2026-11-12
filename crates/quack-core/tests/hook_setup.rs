//! Setup operations run through the engine, using isolated config and mock services.

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use quack_core::protocol::{
    Event, HookConnectionDraft, HookCredentialChange, HookSetupInfo, Op, SecretText,
};
use quack_core::{Engine, EngineHandle};
use wiremock::matchers::{header, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn has_python() -> bool {
    ["python3", "python", "py"].iter().any(|program| {
        std::process::Command::new(program)
            .arg("--version")
            .output()
            .is_ok_and(|out| {
                out.status.success()
                    && (String::from_utf8_lossy(&out.stdout).starts_with("Python 3")
                        || String::from_utf8_lossy(&out.stderr).starts_with("Python 3"))
            })
    })
}

async fn event(engine: &mut EngineHandle, wanted: impl Fn(&Event) -> bool) -> Event {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(15), engine.events.recv())
            .await
            .expect("event deadline")
            .expect("engine event");
        assert!(
            !format!("{event:?}").contains("private-test-token"),
            "secret in event"
        );
        if wanted(&event) {
            return event;
        }
        assert!(
            !matches!(event, Event::Error { .. } | Event::HookSetupFailed { .. }),
            "{event:?}"
        );
    }
}

fn install_defaults(root: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../defaults/hooks.d");
    let dest = root.join("defaults/hooks.d");
    std::fs::create_dir_all(dest.join("forge-review")).expect("mkdir");
    for name in ["forge-review.toml", "forge-review/review.py"] {
        std::fs::copy(source.join(name), dest.join(name)).expect("copy default");
    }
}

async fn inspect(engine: &mut EngineHandle, request: u64) -> HookSetupInfo {
    engine
        .submit(Op::InspectHookSetup {
            id: "forge-review".into(),
            request,
        })
        .await;
    match event(
        engine,
        |e| matches!(e, Event::HookSetup { request: r, .. } if *r == request),
    )
    .await
    {
        Event::HookSetup { info, .. } => info,
        _ => unreachable!("matched event"),
    }
}

fn draft(info: &HookSetupInfo, server: &str, file: &Path) -> HookConnectionDraft {
    let mut conn = info.connections[0].connection.clone();
    conn.values.insert("web_url".into(), server.into());
    conn.values.insert("api_url".into(), server.into());
    conn.values.insert("auth".into(), "token".into());
    HookConnectionDraft {
        connection: conn,
        credentials: BTreeMap::from([(
            "token".into(),
            HookCredentialChange::External {
                source: "file".into(),
                location: SecretText(file.display().to_string()),
            },
        )]),
    }
}

#[tokio::test]
async fn save_applies_private_review_configuration_without_model_calls_and_survives_restart() {
    if !has_python() {
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(common::sse_text("reviewed"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer private-test-token"))
        .and(header("accept", "application/json"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"title":"Private review"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer private-test-token"))
        .and(header("accept", "application/vnd.github.diff"))
        .respond_with(ResponseTemplate::new(200).set_body_string("FULL_PRIVATE_DIFF"))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    install_defaults(tmp.path());
    let loaded = common::workspace_in(
        tmp.path(),
        &server.uri(),
        r#"["tiny"]"#,
        "",
        Some(&tmp.path().join("defaults")),
    );
    let config = Arc::new(loaded);
    let mut engine = Engine::spawn(config.clone());
    event(&mut engine, |e| matches!(e, Event::StartupComplete)).await;
    let info = inspect(&mut engine, 1).await;
    assert!(!info.enabled && info.writable);
    let secret = tmp.path().join("test-credential");
    std::fs::write(&secret, "private-test-token").expect("test secret");
    let conn = draft(&info, &server.uri(), &secret);
    let before_save = server.received_requests().await.expect("requests").len();
    engine
        .submit(Op::SaveHookSetup {
            id: info.id,
            request: 2,
            revision: info.revision,
            connections: vec![conn],
            enable: Some(true),
            replace_payload: None,
        })
        .await;
    let saved = event(&mut engine, |e| {
        matches!(e, Event::HookSetupSaved { request: 2, .. })
    })
    .await;
    assert!(matches!(saved,Event::HookSetupSaved { info, .. } if info.enabled));
    assert!(
        common::bodies(&server).await.is_empty(),
        "saving does not call a model"
    );
    assert!(
        server.received_requests().await.expect("requests").len() == before_save,
        "saving makes no remote requests"
    );
    assert!(
        tmp.path()
            .join("user/hooks.d/forge-review/review.py")
            .exists()
    );
    common::turn(&mut engine, &format!("Review {}/a/b/pull/1", server.uri())).await;
    assert!(common::bodies(&server).await[0].contains("FULL_PRIVATE_DIFF"));
    let text = std::fs::read_to_string(tmp.path().join("user/hooks.d/forge-review.toml"))
        .expect("saved config");
    assert!(!text.contains("private-test-token"));
    engine.shutdown().await;
    let mut engine = Engine::spawn(Arc::new(config.reload().expect("reload")));
    common::turn(&mut engine, &format!("Review {}/a/b/pull/2", server.uri())).await;
    assert!(
        common::bodies(&server)
            .await
            .last()
            .expect("request")
            .contains("FULL_PRIVATE_DIFF")
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn unsaved_checks_do_not_enable_or_save_and_wrong_hosts_are_rejected_before_credentials() {
    if !has_python() {
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("accept", "application/json"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"title":"Review"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(header("accept", "application/vnd.github.diff"))
        .respond_with(ResponseTemplate::new(200).set_body_string("diff"))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    install_defaults(tmp.path());
    let loaded = common::workspace_in(
        tmp.path(),
        &server.uri(),
        r#"["tiny"]"#,
        "",
        Some(&tmp.path().join("defaults")),
    );
    let mut engine = Engine::spawn(Arc::new(loaded));
    let info = inspect(&mut engine, 1).await;
    let secret = tmp.path().join("test-credential");
    std::fs::write(&secret, "private-test-token").expect("secret");
    let conn = draft(&info, &server.uri(), &secret);
    engine
        .submit(Op::CheckHookSetup {
            id: info.id.clone(),
            request: 2,
            revision: info.revision.clone(),
            connection: conn.clone(),
            url: format!("{}/a/b/pull/1", server.uri()),
        })
        .await;
    let result = event(&mut engine, |e| {
        matches!(e, Event::HookCheckResult { request: 2, .. })
    })
    .await;
    assert!(matches!(result,Event::HookCheckResult{status,..} if status=="verified"));
    assert!(!tmp.path().join("user/hooks.d/forge-review.toml").exists());
    assert!(common::bodies(&server).await.is_empty());
    let mut bad = conn;
    bad.credentials.insert(
        "token".into(),
        HookCredentialChange::External {
            source: "file".into(),
            location: SecretText(tmp.path().join("does-not-exist").display().to_string()),
        },
    );
    engine
        .submit(Op::CheckHookSetup {
            id: info.id,
            request: 3,
            revision: info.revision,
            connection: bad,
            url: "https://unconfigured.example/a/b/pull/1".into(),
        })
        .await;
    let result = event(&mut engine, |e| {
        matches!(e, Event::HookCheckResult { request: 3, .. })
    })
    .await;
    assert!(
        matches!(result,Event::HookCheckResult{status,message,..} if status=="invalid" && message.contains("does not belong"))
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn save_only_stale_revision_and_destination_changes_preserve_the_saved_connection() {
    if !has_python() {
        return;
    }
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    install_defaults(tmp.path());
    let loaded = common::workspace_in(
        tmp.path(),
        &server.uri(),
        r#"["tiny"]"#,
        "",
        Some(&tmp.path().join("defaults")),
    );
    let mut engine = Engine::spawn(Arc::new(loaded));
    let info = inspect(&mut engine, 1).await;
    let secret = tmp.path().join("test-credential");
    std::fs::write(&secret, "private-test-token").expect("secret");
    let conn = draft(&info, &server.uri(), &secret);
    engine
        .submit(Op::SaveHookSetup {
            id: info.id.clone(),
            request: 2,
            revision: info.revision.clone(),
            connections: vec![conn.clone()],
            enable: None,
            replace_payload: None,
        })
        .await;
    let info = match event(&mut engine, |e| {
        matches!(e, Event::HookSetupSaved { request: 2, .. })
    })
    .await
    {
        Event::HookSetupSaved { info, .. } => info,
        _ => unreachable!(),
    };
    assert!(!info.enabled, "Save keeps a disabled hook disabled");
    let mut changed = conn;
    changed
        .connection
        .values
        .insert("api_url".into(), "https://another.example/api".into());
    changed.credentials.clear();
    engine
        .submit(Op::SaveHookSetup {
            id: info.id.clone(),
            request: 3,
            revision: info.revision.clone(),
            connections: vec![changed],
            enable: None,
            replace_payload: None,
        })
        .await;
    let failed = event(&mut engine, |e| {
        matches!(e, Event::HookSetupFailed { request: 3, .. })
    })
    .await;
    assert!(
        matches!(failed,Event::HookSetupFailed{message,..} if message.contains("destination changed"))
    );
    std::fs::OpenOptions::new()
        .append(true)
        .open(&info.source)
        .and_then(|mut f| std::io::Write::write_all(&mut f, b"\n# external edit\n"))
        .expect("external edit");
    engine
        .submit(Op::SaveHookSetup {
            id: info.id,
            request: 4,
            revision: info.revision,
            connections: Vec::new(),
            enable: None,
            replace_payload: None,
        })
        .await;
    let failed = event(&mut engine, |e| {
        matches!(e, Event::HookSetupFailed { request: 4, .. })
    })
    .await;
    assert!(
        matches!(failed,Event::HookSetupFailed{message,..} if message.contains("changed while"))
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn cancelling_a_check_keeps_the_engine_available_for_a_turn() {
    if !has_python() {
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("accept", "application/json"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(5))
                .set_body_json(serde_json::json!({"title":"Slow review"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(common::sse_text("still available"))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    install_defaults(tmp.path());
    let loaded = common::workspace_in(
        tmp.path(),
        &server.uri(),
        r#"["tiny"]"#,
        "",
        Some(&tmp.path().join("defaults")),
    );
    let mut engine = Engine::spawn(Arc::new(loaded));
    let info = inspect(&mut engine, 1).await;
    let mut conn = HookConnectionDraft {
        connection: info.connections[0].connection.clone(),
        credentials: BTreeMap::new(),
    };
    conn.connection
        .values
        .insert("web_url".into(), server.uri());
    conn.connection
        .values
        .insert("api_url".into(), server.uri());
    engine
        .submit(Op::CheckHookSetup {
            id: info.id,
            request: 2,
            revision: info.revision,
            connection: conn,
            url: format!("{}/a/b/pull/1", server.uri()),
        })
        .await;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if server
                .received_requests()
                .await
                .expect("requests")
                .iter()
                .any(|r| r.url.path().contains("/pulls/1"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("check started");
    engine.submit(Op::CancelHookCheck { request: 2 }).await;
    let result = event(&mut engine, |e| {
        matches!(e, Event::HookCheckResult { request: 2, .. })
    })
    .await;
    assert!(matches!(result,Event::HookCheckResult{status,..} if status=="cancelled"));
    common::turn(&mut engine, "continue working").await;
    assert!(!common::bodies(&server).await.is_empty());
    engine.shutdown().await;
}
