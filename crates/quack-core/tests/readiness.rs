//! Regressions for unattended runs: limits, incomplete streams, and usage recovery.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{bodies, sse_call, sse_raw, sse_text, turn, workspace};
use quack_core::protocol::{Event, InputItem, Op, TurnOverrides};
use quack_core::{Engine, EngineHandle, Resume};
use serde_json::json;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn outcome(engine: &mut EngineHandle) -> (bool, String) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
            .await
            .expect("event within 10s")
            .expect("channel open");
        match event {
            Event::TurnComplete { .. } => return (true, String::new()),
            Event::TurnAborted { reason, .. } => return (false, reason),
            _ => {}
        }
    }
}

/// Waits for the turn to end, answering each step-limit ask with `answer`.
async fn finish(engine: &mut EngineHandle, mut answer: impl FnMut() -> bool) -> (bool, String) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
            .await
            .expect("event within 10s")
            .expect("channel open");
        match event {
            Event::TurnComplete { .. } => return (true, String::new()),
            Event::TurnAborted { reason, .. } => return (false, reason),
            Event::StepLimitReached { turn, .. } => {
                let proceed = answer();
                engine.submit(Op::ContinueTurn { turn, proceed }).await;
            }
            _ => {}
        }
    }
}

async fn submit(engine: &mut EngineHandle) {
    engine
        .submit(Op::UserTurn {
            items: vec![InputItem::text("Finish the task.")],
            overrides: TurnOverrides::default(),
        })
        .await;
}

#[tokio::test]
async fn a_declined_step_limit_aborts_instead_of_completing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_call("c1", "list_dir", json!({})))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    config.settings.agent.max_steps = std::num::NonZeroU32::new(3).expect("positive");
    let mut engine = Engine::spawn(Arc::new(config));
    submit(&mut engine).await;
    assert_eq!(
        finish(&mut engine, || false).await,
        (false, "step limit reached".into())
    );
    assert_eq!(bodies(&server).await.len(), 3);
    engine.shutdown().await;
}

#[tokio::test]
async fn continuing_at_the_step_limit_grants_another_budget_of_steps() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_call("c1", "list_dir", json!({})))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    config.settings.agent.max_steps = std::num::NonZeroU32::new(3).expect("positive");
    let mut engine = Engine::spawn(Arc::new(config));
    submit(&mut engine).await;
    // Continue once, then decline: 3 calls, another 3 after the go-ahead.
    let mut asks = 0;
    assert_eq!(
        finish(&mut engine, || {
            asks += 1;
            asks == 1
        })
        .await,
        (false, "step limit reached".into())
    );
    assert_eq!(asks, 2);
    assert_eq!(bodies(&server).await.len(), 6);
    engine.shutdown().await;
}

#[tokio::test]
async fn interrupting_while_paused_at_the_step_limit_aborts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_call("c1", "list_dir", json!({})))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    config.settings.agent.max_steps = std::num::NonZeroU32::new(3).expect("positive");
    let mut engine = Engine::spawn(Arc::new(config));
    submit(&mut engine).await;
    loop {
        if matches!(
            engine.events.recv().await.expect("event"),
            Event::StepLimitReached { .. }
        ) {
            break;
        }
    }
    engine.submit(Op::Interrupt).await;
    assert_eq!(outcome(&mut engine).await, (false, "interrupted".into()));
    engine.shutdown().await;
}

#[test]
fn zero_steps_is_rejected_by_configuration() {
    assert!(toml::from_str::<quack_config::settings::Settings>("[agent]\nmax_steps=0").is_err());
}

#[tokio::test]
async fn truncated_tool_calls_do_not_run_and_the_model_can_recover() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("output-token limit and was discarded"))
        .respond_with(sse_text("Recovered."))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse_raw(&[json!({
            "choices":[{"delta":{"tool_calls":[{"index":0,"id":"cut","function":{
                "name":"write", "arguments":json!({"path":"should-not-exist","content":"partial"}).to_string()
            }}]},"finish_reason":"length"}],
            "usage":{"prompt_tokens":100,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":0}}
        }).to_string()]))
        .up_to_n_times(1).mount(&server).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    config.settings.permissions.mode = quack_config::settings::PermissionMode::Allow;
    let mut engine = Engine::spawn(Arc::new(config));
    let events = turn(&mut engine, "Finish the task.").await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::ToolCallBegin { .. }))
    );
    assert!(!tmp.path().join("ws/should-not-exist").exists());
    let requests = bodies(&server).await;
    assert_eq!(requests.len(), 2);
    assert!(!requests[1].contains("should-not-exist"));
    engine.shutdown().await;
}

#[tokio::test]
async fn repeated_output_cutoffs_abort_after_two_recoveries() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_raw(&[
            json!({"choices":[{"delta":{"content":"unfinished"},"finish_reason":"length"}]})
                .to_string(),
        ]))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut engine = Engine::spawn(Arc::new(workspace(
        tmp.path(),
        &server.uri(),
        r#"["tiny"]"#,
        "",
    )));
    submit(&mut engine).await;
    assert_eq!(
        outcome(&mut engine).await,
        (false, "output-token limit reached".into())
    );
    assert_eq!(bodies(&server).await.len(), 3);
    engine.shutdown().await;
}

#[tokio::test]
async fn eof_without_a_finish_reason_is_retried_without_running_partial_calls() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(format!("data: {}\n\n", json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"cut","function":{
                "name":"write","arguments":json!({"path":"should-not-exist","content":"partial"}).to_string()
            }}]}}]}))))
        .up_to_n_times(1).mount(&server).await;
    Mock::given(method("POST"))
        .respond_with(sse_text("Recovered."))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut engine = Engine::spawn(Arc::new(workspace(
        tmp.path(),
        &server.uri(),
        r#"["tiny"]"#,
        "",
    )));
    let events = turn(&mut engine, "Finish the task.").await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::ToolCallBegin { .. }))
    );
    assert_eq!(bodies(&server).await.len(), 2);
    engine.shutdown().await;
}

#[tokio::test]
async fn compaction_usage_is_priced_and_survives_resume() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("You summarize a coding session"))
        .respond_with(sse_raw(&[json!({
            "choices":[{"delta":{"content":"Summary."},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":100,"completion_tokens":20,"prompt_tokens_details":{"cached_tokens":40}}
        }).to_string()]))
        .mount(&server).await;
    Mock::given(method("POST"))
        .respond_with(sse_raw(&[json!({
            "choices":[{"delta":{"content":"Done."},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":10,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":0}}
        }).to_string()]))
        .mount(&server).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = Arc::new(workspace(
        tmp.path(),
        &server.uri(),
        r#"[{ id="tiny", cost={input=1.0, output=2.0, cached_input=0.5} }]"#,
        "",
    ));
    let mut engine = Engine::spawn(config.clone());
    turn(&mut engine, "First task.").await;
    turn(&mut engine, "Second task.").await;
    engine.submit(Op::Compact { focus: None }).await;
    assert!(outcome(&mut engine).await.0);
    let mut reports = Vec::new();
    for resumed in [false, true] {
        if resumed {
            engine.shutdown().await;
            engine = Engine::spawn_with(config.clone(), Resume::Latest);
            loop {
                if matches!(
                    engine.events.recv().await.expect("event"),
                    Event::SessionResumed { .. }
                ) {
                    break;
                }
            }
        }
        engine.submit(Op::GetStats).await;
        loop {
            if let Event::Stats { report } = engine.events.recv().await.expect("event") {
                reports.push(report);
                break;
            }
        }
    }
    for report in reports {
        // The deliberately short summary is rejected and retried; both calls count.
        assert_eq!(
            (
                report.requests,
                report.prompt_tokens,
                report.completion_tokens
            ),
            (4, 220, 44)
        );
        assert!((report.cost.expect("priced") - 0.000268).abs() < 1e-12);
    }
    engine.shutdown().await;
}
