//! Compaction against a mock server: at the threshold, on overflow, and on `/compact`.

mod common;

use std::sync::Arc;

use common::{bodies, sse_text as sse, turn, until_done};
use quack_core::protocol::{Event, Op, PermissionMode, ReplayItem};
use quack_core::{Engine, Resume};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SUMMARIZER: &str = "You summarize a coding session";

fn workspace(root: &std::path::Path, base_url: &str, models: &str) -> quack_config::Loaded {
    common::workspace(root, base_url, models, "")
}

fn compacted(events: &[Event]) -> bool {
    events.iter().any(|e| matches!(e, Event::Compacted { .. }))
}

fn responses_workspace(root: &std::path::Path, server: &MockServer) -> Arc<quack_config::Loaded> {
    let loaded = workspace(
        root,
        &server.uri(),
        r#"[{ id = "gpt-6-luna", context_window = 272000, cost = { input = 0.1, output = 0.5 } }]"#,
    );
    let path = loaded
        .dirs
        .user_config
        .join("providers.d")
        .join("mock.toml");
    let provider = std::fs::read_to_string(&path).expect("mock provider");
    std::fs::write(
        path,
        format!("{provider}api = \"openai-responses\"\nnative_compaction = true\n"),
    )
    .expect("Responses provider");
    Arc::new(loaded)
}

fn responses_reply(text: &str) -> ResponseTemplate {
    use serde_json::json;
    common::sse_raw(&[
        json!({"type": "response.output_item.done", "output_index": 0,
            "item": {"type": "reasoning", "encrypted_content": "reasoning-for-replay"}})
        .to_string(),
        json!({"type": "response.output_text.delta", "output_index": 1, "delta": text}).to_string(),
        json!({"type": "response.output_item.done", "output_index": 1,
            "item": {"type": "message", "role": "assistant", "phase": "final_answer",
                "content": [{"type": "output_text", "text": text}]}})
        .to_string(),
        json!({"type": "response.completed", "response": {}}).to_string(),
    ])
}

#[tokio::test]
async fn native_compaction_replays_the_window_and_kept_reasoning_after_resume() {
    use serde_json::json;
    if !quack_providers::supports("openai-responses") {
        return;
    }
    let server = MockServer::start().await;
    let output = json!([
        {"type": "message", "role": "user", "id": "msg_retained", "content": [{"type": "input_text", "text": "FIRST"}]},
        {"type": "compaction", "id": "cmp_native", "encrypted_content": "sealed-window"}
    ]);
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(body_string_contains("compaction_trigger"))
        .respond_with(common::sse_raw(&[
            json!({"type": "response.output_item.done", "output_index": 1, "item": output[1]}).to_string(),
            json!({"type": "response.output_item.done", "output_index": 0, "item": output[0]}).to_string(),
            json!({"type": "response.completed", "response": {"usage": {"input_tokens": 40, "output_tokens": 12}}}).to_string(),
        ]))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(responses_reply("reply"))
        .expect(4)
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = responses_workspace(tmp.path(), &server);
    let mut engine = Engine::spawn(loaded.clone());
    turn(&mut engine, "FIRST").await;
    turn(&mut engine, "SECOND").await;
    engine.submit(Op::Compact { focus: None }).await;
    assert!(compacted(&until_done(&mut engine).await));
    engine.shutdown().await;

    let mut engine = Engine::spawn_with(loaded.clone(), Resume::Latest);
    loop {
        if let Event::SessionResumed { items, .. } = engine.events.recv().await.expect("event") {
            assert!(items.contains(&ReplayItem::Compacted));
            break;
        }
    }
    turn(&mut engine, "THIRD").await;
    engine.shutdown().await;

    let path = loaded
        .dirs
        .user_config
        .join("providers.d")
        .join("mock.toml");
    let provider = std::fs::read_to_string(&path).expect("mock provider");
    std::fs::write(
        path,
        provider.replace("native_compaction = true", "native_compaction = false"),
    )
    .expect("disable native compaction");
    let mut engine = Engine::spawn_with(loaded, Resume::Latest);
    turn(&mut engine, "FOURTH").await;
    engine.shutdown().await;

    let requests = server.received_requests().await.expect("recorded requests");
    let compact: serde_json::Value = requests[2].body_json().expect("compact body");
    assert_eq!(requests[2].url.path(), "/v1/responses");
    assert_eq!(compact["model"], "gpt-6-luna");
    assert_eq!(compact["store"], false);
    assert_eq!(
        compact["input"]
            .as_array()
            .expect("input")
            .last()
            .expect("trigger")["type"],
        "compaction_trigger"
    );
    let submitted = compact["input"].to_string();
    assert!(submitted.contains("FIRST") && submitted.contains("reasoning-for-replay"));
    assert!(!submitted.contains("SECOND"));

    let resumed: serde_json::Value = requests[3].body_json().expect("resumed body");
    let input = resumed["input"].as_array().expect("input array");
    assert_eq!(&input[..2], output.as_array().expect("compacted output"));
    assert_eq!(input[2]["content"], "SECOND");
    assert_eq!(input[3]["encrypted_content"], "reasoning-for-replay");
    assert_eq!(input.last().expect("new message")["content"], "THIRD");
    let portable = String::from_utf8_lossy(&requests[4].body);
    assert!(portable.contains("[compaction summary]") && portable.contains("FOURTH"));
    assert!(!portable.contains("sealed-window") && !portable.contains("reasoning-for-replay"));
}

#[tokio::test]
async fn native_compaction_falls_back_at_the_threshold_when_the_route_is_unavailable() {
    if !quack_providers::supports("openai-responses") {
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(body_string_contains("compaction_trigger"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(body_string_contains(SUMMARIZER))
        .respond_with(responses_reply("SUMMARY-TEXT"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(responses_reply("reply"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let mut loaded = responses_workspace(tmp.path(), &server);
    Arc::get_mut(&mut loaded)
        .expect("unique config")
        .settings
        .context
        .window = Some(20_000);
    let mut engine = Engine::spawn(loaded);
    let big = "x".repeat(17_000);
    turn(&mut engine, &format!("FIRST {big}")).await;
    turn(&mut engine, &format!("SECOND {big}")).await;
    let events = turn(&mut engine, &format!("THIRD {big}")).await;
    assert!(compacted(&events));
    assert!(events.iter().any(|event| matches!(event, Event::Warning { message } if message.contains("using a text summary"))));
    let last = server
        .received_requests()
        .await
        .expect("recorded requests")
        .pop()
        .expect("request");
    let last = String::from_utf8_lossy(&last.body);
    assert!(last.contains("SUMMARY-TEXT") && last.contains("THIRD"));
    assert!(!last.contains("reasoning-for-replay"));
    engine.shutdown().await;
}

#[tokio::test]
async fn compacts_at_the_threshold_and_resumes_compacted() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains(SUMMARIZER))
        .respond_with(sse("SUMMARY-TEXT"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse("ok"))
        .mount(&server)
        .await;

    // Budget 17,000 tokens, threshold 13,600. Each prompt is about 4,900 tokens, so the
    // third request goes over and the first two turns get summarized.
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = Arc::new(workspace(
        tmp.path(),
        &server.uri(),
        r#"[{ id = "tiny", context_window = 20000 }]"#,
    ));
    let big = "x".repeat(17_000);
    let mut engine = Engine::spawn(loaded.clone());
    assert!(!compacted(
        &turn(&mut engine, &format!("FIRST {big}")).await
    ));
    assert!(!compacted(
        &turn(&mut engine, &format!("SECOND {big}")).await
    ));
    let third = turn(&mut engine, &format!("THIRD {big}")).await;
    assert!(compacted(&third), "{third:?}");

    let all = bodies(&server).await;
    let summarize = &all[all.len() - 2];
    // The summarizer gets about 60% of the budget, so the oldest turn is left out.
    assert!(summarize.contains(SUMMARIZER) && summarize.contains("SECOND"));
    assert!(summarize.contains("older messages left out"));
    assert!(
        !summarize.contains("\"tools\""),
        "the summary request has no tools"
    );
    let last = all.last().expect("request");
    assert!(last.contains("[compaction summary]") && last.contains("SUMMARY-TEXT"));
    assert!(last.contains("THIRD") && !last.contains("FIRST") && !last.contains("SECOND"));
    engine.shutdown().await;

    // A resumed session continues from the compacted history.
    let mut engine = Engine::spawn_with(loaded, Resume::Latest);
    let resumed = loop {
        let e = engine.events.recv().await.expect("event");
        if let Event::SessionResumed { items, .. } = e {
            break items;
        }
    };
    assert!(resumed.contains(&ReplayItem::Compacted));
    turn(&mut engine, "FOURTH").await;
    let last = bodies(&server).await.pop().expect("request");
    assert!(last.contains("SUMMARY-TEXT") && last.contains("THIRD") && last.contains("FOURTH"));
    assert!(!last.contains("FIRST"));
    engine.shutdown().await;
}

#[tokio::test]
async fn an_overflow_compacts_and_retries_once() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse("first answer"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains(SUMMARIZER))
        .respond_with(sse("SUMMARY-TEXT"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("[compaction summary]"))
        .respond_with(sse("second answer"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"error":{"message":"This model's maximum context length is 32768 tokens"}}"#,
        ))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = workspace(tmp.path(), &server.uri(), r#"["tiny"]"#);
    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(&mut engine, "first question").await;
    let events = turn(&mut engine, "second question").await;
    assert!(compacted(&events));
    let text: String = events
        .iter()
        .filter_map(|e| match e {
            Event::AssistantDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "second answer");
    engine.shutdown().await;
}

#[tokio::test]
async fn compact_command_keeps_only_the_last_turn() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains(SUMMARIZER))
        .respond_with(sse("SUMMARY-TEXT"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse("ok"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = workspace(tmp.path(), &server.uri(), r#"["tiny"]"#);
    let mut engine = Engine::spawn(Arc::new(loaded));

    // Nothing before the current turn: nothing to compact.
    turn(&mut engine, "ONE").await;
    engine.submit(Op::Compact { focus: None }).await;
    let reason = loop {
        match engine.events.recv().await.expect("event") {
            Event::TurnAborted { reason, .. } => break reason,
            Event::TurnComplete { .. } => panic!("compacted a single turn"),
            _ => {}
        }
    };
    assert!(reason.contains("nothing to compact"), "{reason}");

    turn(&mut engine, "TWO").await;
    engine
        .submit(Op::Compact {
            focus: Some("the parser".into()),
        })
        .await;
    assert!(compacted(&until_done(&mut engine).await));
    let summarize = bodies(&server).await.pop().expect("request");
    assert!(summarize.contains("Pay particular attention to: the parser"));
    assert!(summarize.contains("ONE") && !summarize.contains("TWO"));

    turn(&mut engine, "THREE").await;
    let last = bodies(&server).await.pop().expect("request");
    assert!(last.contains("SUMMARY-TEXT") && last.contains("TWO") && last.contains("THREE"));
    assert!(!last.contains("\"ONE\""));
    engine.shutdown().await;
}

const DSML: &str = "<｜DSML｜ calls>\n<｜DSML｜ invoke name=\"read\">\n<｜DSML｜ parameter name=\"path\" \
                    string=\"true\">src/shop.py</｜DSML｜ parameter>\n</｜DSML｜ invoke>";

/// Three big turns, so the third one compacts the first two (as in the threshold test).
async fn three_turns(server: &MockServer, tmp: &std::path::Path) -> Vec<Event> {
    let loaded = Arc::new(workspace(
        tmp,
        &server.uri(),
        r#"[{ id = "tiny", context_window = 20000 }]"#,
    ));
    let big = "x".repeat(17_000);
    let mut engine = Engine::spawn(loaded);
    turn(&mut engine, &format!("FIRST {big}")).await;
    turn(&mut engine, &format!("SECOND {big}")).await;
    let third = turn(&mut engine, &format!("THIRD {big}")).await;
    engine.shutdown().await;
    third
}

#[tokio::test]
async fn a_tool_call_for_a_summary_is_asked_again() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains(SUMMARIZER))
        .respond_with(sse(DSML))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains(SUMMARIZER))
        .respond_with(sse("## Goal\nSUMMARY-TEXT, written the second time round."))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse("ok"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let third = three_turns(&server, tmp.path()).await;
    assert!(compacted(&third), "{third:?}");
    assert!(
        !third.iter().any(|e| matches!(e, Event::Warning { .. })),
        "{third:?}"
    );
    let all = bodies(&server).await;
    let retry = &all[all.len() - 2];
    assert!(retry.contains("You cannot call tools here"));
    let last = all.last().expect("request");
    assert!(last.contains("SUMMARY-TEXT") && !last.contains("DSML"));
}

#[tokio::test]
async fn a_summary_that_is_a_tool_call_twice_falls_back_to_the_list() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains(SUMMARIZER))
        .respond_with(sse(DSML))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse("ok"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let third = three_turns(&server, tmp.path()).await;
    assert!(compacted(&third), "{third:?}");
    assert!(third.iter().any(|e| matches!(
        e,
        Event::Warning { message } if message.contains("tool call instead of a summary")
    )));
    let last = bodies(&server).await.pop().expect("request");
    assert!(last.contains("No summary could be written") && !last.contains("DSML"));
    assert!(last.contains("THIRD"));
}

#[tokio::test]
async fn the_summary_lists_the_files_the_session_changed() {
    let server = MockServer::start().await;
    // Mounted first: the summarizer's transcript holds the other turns' text too.
    Mock::given(method("POST"))
        .and(body_string_contains(SUMMARIZER))
        .respond_with(sse("SUMMARY-TEXT"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("FIRST"))
        .respond_with(common::sse_call(
            "w1",
            "write",
            serde_json::json!({"path": "notes.txt", "content": "written"}),
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse("ok"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = Arc::new(workspace(
        tmp.path(),
        &server.uri(),
        r#"[{ id = "tiny", context_window = 20000 }]"#,
    ));
    let ws = loaded.dirs.workspace_root.clone();
    let big = "x".repeat(17_000);
    let mut engine = Engine::spawn(loaded);
    engine
        .submit(Op::SetPermissionMode {
            mode: PermissionMode::Allow,
        })
        .await;
    turn(&mut engine, &format!("FIRST {big}")).await;
    assert!(ws.join("notes.txt").exists());
    turn(&mut engine, &format!("SECOND {big}")).await;
    assert!(compacted(&turn(&mut engine, &format!("THIRD {big}")).await));
    let last = bodies(&server).await.pop().expect("request");
    assert!(
        last.contains("## Files changed in this session\\n- notes.txt"),
        "{last}"
    );
    assert!(!last.contains("read them again"));
    engine.shutdown().await;
}

#[tokio::test]
async fn a_context_window_setting_caps_the_models_window() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains(SUMMARIZER))
        .respond_with(sse("SUMMARY-TEXT"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse("ok"))
        .mount(&server)
        .await;

    // The model says 200k; the setting holds it to 20k, so the third turn compacts, as in
    // the threshold test.
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("user")).expect("mkdir");
    std::fs::write(
        tmp.path().join("user").join("config.toml"),
        "[context]\nwindow = 20000\n",
    )
    .expect("config");
    let loaded = Arc::new(workspace(
        tmp.path(),
        &server.uri(),
        r#"[{ id = "tiny", context_window = 200000 }]"#,
    ));
    assert_eq!(loaded.settings.context.window, Some(20_000));
    let big = "x".repeat(17_000);
    let mut engine = Engine::spawn(loaded);
    assert!(!compacted(
        &turn(&mut engine, &format!("FIRST {big}")).await
    ));
    assert!(!compacted(
        &turn(&mut engine, &format!("SECOND {big}")).await
    ));
    assert!(compacted(&turn(&mut engine, &format!("THIRD {big}")).await));
    engine.shutdown().await;
}
