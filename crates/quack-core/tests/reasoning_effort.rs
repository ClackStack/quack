//! Thinking levels reach the wire and stay with their model and session, across starts.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{sse_text, turn, workspace};
use quack_core::protocol::{Event, InputItem, ModelId, Op, ProviderId, SessionInfo};
use quack_core::{Engine, EngineHandle, Resume};
use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};

const MODELS: &str = r#"[
    { id = "tiny", alias = "t", context_window = 32768, reasoning = { effort = "medium", efforts = ["low", "medium", "high"] } },
    { id = "fixed", context_window = 32768 }
]"#;

async fn next(engine: &mut EngineHandle) -> Event {
    tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
        .await
        .expect("event within 10s")
        .expect("engine open")
}

async fn configured(engine: &mut EngineHandle) -> SessionInfo {
    loop {
        match next(engine).await {
            Event::SessionConfigured { session } => return session,
            Event::Error { error } => panic!("{}", error.message),
            Event::Warning { message } => panic!("{message}"),
            _ => {}
        }
    }
}

async fn started(engine: &mut EngineHandle) -> SessionInfo {
    let mut session = None;
    loop {
        match next(engine).await {
            Event::SessionConfigured { session: ready } => session = Some(ready),
            Event::StartupComplete => return session.expect("configured at startup"),
            Event::Error { error } => panic!("{}", error.message),
            _ => {}
        }
    }
}

async fn select(engine: &mut EngineHandle, effort: Option<&str>) -> SessionInfo {
    engine
        .submit(Op::SetReasoningEffort {
            effort: effort.map(str::to_owned),
        })
        .await;
    configured(engine).await
}

async fn switch(engine: &mut EngineHandle, model: &str) -> SessionInfo {
    engine
        .submit(Op::SetModel {
            provider: ProviderId("mock".into()),
            model: ModelId(model.into()),
        })
        .await;
    configured(engine).await
}

async fn requests(server: &MockServer) -> Vec<Value> {
    common::bodies(server)
        .await
        .into_iter()
        .map(|body| serde_json::from_str::<Value>(&body).expect("request JSON"))
        .filter(|body| body.get("tools").is_some())
        .collect()
}

#[tokio::test]
async fn selections_survive_switch_reload_resume_and_reset_without_changing_the_prefix() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse_text("done"))
        .mount(&server)
        .await;
    let loaded = workspace(tmp.path(), &server.uri(), MODELS, "");
    std::fs::write(
        tmp.path().join("user/config.toml"),
        "[model]\ndefault = \"mock/t\"\nremember_last = false\n",
    )
    .expect("config");
    let config = Arc::new(loaded.reload().expect("reload"));
    let mut engine = Engine::spawn(config.clone());
    let original = started(&mut engine).await;
    assert_eq!(original.reasoning_effort.as_deref(), Some("medium"));
    assert_eq!(original.reasoning_efforts, ["low", "medium", "high"]);

    // A selection made before the first turn must be saved when its log is created.
    assert_eq!(
        select(&mut engine, Some("high"))
            .await
            .reasoning_effort
            .as_deref(),
        Some("high")
    );
    turn(&mut engine, "first").await;
    engine.shutdown().await;
    let mut engine = Engine::spawn_with(config.clone(), Resume::Latest);
    assert_eq!(
        started(&mut engine).await.reasoning_effort.as_deref(),
        Some("high")
    );
    assert_eq!(
        select(&mut engine, Some("low"))
            .await
            .reasoning_effort
            .as_deref(),
        Some("low")
    );
    turn(&mut engine, "second").await;
    let bodies = requests(&server).await;
    assert_eq!(bodies[0]["reasoning_effort"], "high");
    assert_eq!(bodies[1]["reasoning_effort"], "low");
    assert_eq!(bodies[0]["messages"][0], bodies[1]["messages"][0]);
    assert_eq!(bodies[0]["tools"], bodies[1]["tools"]);

    // Setting an effort is queued behind turn completion, so it is also a barrier.
    select(&mut engine, Some("low")).await;
    let fixed = switch(&mut engine, "fixed").await;
    assert!(fixed.reasoning_effort.is_none() && fixed.reasoning_efforts.is_empty());
    assert_eq!(
        switch(&mut engine, "tiny")
            .await
            .reasoning_effort
            .as_deref(),
        Some("low")
    );
    engine.submit(Op::ReloadConfig).await;
    assert_eq!(
        started(&mut engine).await.reasoning_effort.as_deref(),
        Some("low")
    );
    engine.shutdown().await;

    let mut engine = Engine::spawn_with(config.clone(), Resume::Latest);
    let resumed = started(&mut engine).await;
    assert_eq!(resumed.id, original.id);
    assert_eq!(resumed.reasoning_effort.as_deref(), Some("low"));
    turn(&mut engine, "resumed").await;
    assert_eq!(
        requests(&server).await.last().expect("request")["reasoning_effort"],
        "low"
    );
    assert_eq!(
        select(&mut engine, None).await.reasoning_effort.as_deref(),
        Some("medium")
    );
    engine.shutdown().await;

    let mut engine = Engine::spawn_with(config, Resume::Latest);
    assert_eq!(
        started(&mut engine).await.reasoning_effort.as_deref(),
        Some("medium")
    );
    select(&mut engine, Some("high")).await;
    engine.submit(Op::NewSession).await;
    // A new session keeps the remembered level instead of falling back to the default.
    assert_eq!(
        configured(&mut engine).await.reasoning_effort.as_deref(),
        Some("high")
    );
    engine.submit(Op::ResumeSession { id: original.id }).await;
    assert_eq!(
        configured(&mut engine).await.reasoning_effort.as_deref(),
        Some("high")
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn unsupported_levels_are_rejected_without_a_model_call() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let server = MockServer::start().await;
    let config = Arc::new(workspace(tmp.path(), &server.uri(), MODELS, ""));
    let mut engine = Engine::spawn(config);
    started(&mut engine).await;
    for (level, message) in [("max", "unavailable"), ("", "unavailable")] {
        engine
            .submit(Op::SetReasoningEffort {
                effort: Some(level.into()),
            })
            .await;
        loop {
            match next(&mut engine).await {
                Event::Warning { message: warning } => {
                    assert!(warning.contains(message), "{warning}");
                    break;
                }
                Event::SessionConfigured { .. } => panic!("invalid selection changed the model"),
                _ => {}
            }
        }
    }
    switch(&mut engine, "fixed").await;
    engine
        .submit(Op::SetReasoningEffort {
            effort: Some("high".into()),
        })
        .await;
    loop {
        if let Event::Warning { message } = next(&mut engine).await {
            assert!(
                message.contains("no selectable thinking levels"),
                "{message}"
            );
            break;
        }
    }
    assert!(requests(&server).await.is_empty());
    engine.shutdown().await;
}

#[tokio::test]
async fn a_selection_during_a_turn_applies_to_the_next_turn() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse_text("done").set_delay(Duration::from_millis(100)))
        .mount(&server)
        .await;
    let config = Arc::new(workspace(tmp.path(), &server.uri(), MODELS, ""));
    let mut engine = Engine::spawn(config);
    started(&mut engine).await;
    engine
        .submit(Op::UserTurn {
            items: vec![InputItem::text("first")],
            overrides: Default::default(),
        })
        .await;
    loop {
        if matches!(next(&mut engine).await, Event::TurnStarted { .. }) {
            break;
        }
    }
    engine
        .submit(Op::SetReasoningEffort {
            effort: Some("high".into()),
        })
        .await;
    assert_eq!(
        configured(&mut engine).await.reasoning_effort.as_deref(),
        Some("high")
    );
    turn(&mut engine, "second").await;
    let bodies = requests(&server).await;
    assert_eq!(bodies[0]["reasoning_effort"], "medium");
    assert_eq!(bodies[1]["reasoning_effort"], "high");
    engine.shutdown().await;
}

#[tokio::test]
async fn selections_persist_across_starts_and_resets_stick_too() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse_text("done"))
        .mount(&server)
        .await;
    let start = || Engine::spawn(Arc::new(workspace(tmp.path(), &server.uri(), MODELS, "")));

    // Select, and make a turn so the session exists; then start over without resuming.
    let mut engine = start();
    assert_eq!(
        started(&mut engine).await.reasoning_effort.as_deref(),
        Some("medium")
    );
    select(&mut engine, Some("low")).await;
    turn(&mut engine, "first").await;
    engine.shutdown().await;

    // A fresh start uses the remembered level, and sends it on the wire.
    let mut engine = start();
    assert_eq!(
        started(&mut engine).await.reasoning_effort.as_deref(),
        Some("low"),
        "remembered across starts"
    );
    turn(&mut engine, "second").await;
    assert_eq!(
        requests(&server).await.last().expect("request")["reasoning_effort"],
        "low"
    );

    // Resetting to the model's default is a choice as well: it is what the next start
    // uses, not the level picked before it.
    assert_eq!(
        select(&mut engine, None).await.reasoning_effort.as_deref(),
        Some("medium")
    );
    engine.shutdown().await;
    let mut engine = start();
    assert_eq!(
        started(&mut engine).await.reasoning_effort.as_deref(),
        Some("medium"),
        "a reset is remembered too"
    );

    // A level the model no longer declares is ignored rather than rejected.
    let state = tmp.path().join("data").join("state.toml");
    std::fs::write(&state, "[reasoning_efforts]\n\"mock/tiny\" = \"ultra\"\n").expect("state");
    let mut engine = start();
    assert_eq!(
        started(&mut engine).await.reasoning_effort.as_deref(),
        Some("medium"),
        "an undeclared level falls back to the default"
    );
    engine.shutdown().await;
}
