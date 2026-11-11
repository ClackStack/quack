//! The bridge core driven headless (docs/plan/15-desktop-client.md#milestones, G0): a
//! wiremock model plays a turn, the sink is an mpsc channel, and the assertions cover
//! the AppInfo snapshot, delta coalescing, the approval round-trip, the
//! duplicate-session guard and a clean shutdown.

mod common;

use std::path::Path;

use quack_core::Resume;
use quack_gui::{OpenError, Payload, Registry};
use quack_protocol::{ApprovalRequest, Event};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer};

use common::{
    collect_until, collecting_sink, fixed_loader, next, sse_text, sse_write_call, until, ws_of,
};

/// Open `root/ws` on a fresh registry and return (registry, tab, receiver). The first
/// message on the receiver is asserted to be the AppInfo snapshot.
async fn open(
    root: &Path,
    server: &MockServer,
) -> (
    Registry,
    u64,
    tokio::sync::mpsc::UnboundedReceiver<quack_gui::TabMessage>,
) {
    let loaded = common::workspace(root, &server.uri());
    let session_cwd = loaded.dirs.workspace_root.display().to_string();
    let mut registry = Registry::with_parts(fixed_loader(loaded), root.join("gui-data"));
    let (sink, mut rx) = collecting_sink();
    let tab = registry
        .open(ws_of(root), Resume::New, sink)
        .await
        .expect("open");
    let msg = next(&mut rx).await;
    assert!(
        matches!(&msg.payload, Payload::Info { info } if info.cwd == session_cwd),
        "first message is the AppInfo snapshot, got {msg:?}"
    );
    (registry, tab, rx)
}

#[tokio::test]
async fn appinfo_snapshot_carries_the_static_facts() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_registry, _tab, mut rx) = open(tmp.path(), &server).await;
    let msg = until(&mut rx, |m| matches!(&m.payload, Payload::Info { .. })).await;
    let Payload::Info { info } = msg.payload else {
        unreachable!("until matched an Info");
    };
    assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(info.layers.len(), 4, "{:?}", info.layers);
    assert!(
        info.layers.iter().any(|l| l.layer == "user"),
        "{:?}",
        info.layers
    );
    assert!(
        info.permission_mode == "ask" || info.permission_mode == "read-only",
        "the default permission mode labels as ask, got {}",
        info.permission_mode
    );
    assert!(info.settings.iter().any(|s| s.key == "model.default"));
}

#[tokio::test]
async fn a_turn_streams_through_the_bridge_with_merged_deltas() {
    let server = MockServer::start().await;
    let words = ["hel", "lo ", "wor", "ld", "."];
    let mut reply: Vec<String> = words
        .iter()
        .map(|t| serde_json::json!({"choices":[{"delta":{"content":t}}]}).to_string())
        .collect();
    reply.push(
        serde_json::json!({"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":6}})
            .to_string(),
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(common::sse_raw(&reply))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let (registry, tab, mut rx) = open(tmp.path(), &server).await;
    registry
        .send_op(
            tab,
            serde_json::json!({"op":"user_turn","items":[{"type":"text","text":"hi"}],"overrides":{}}),
        )
        .await
        .expect("send the turn");

    let (seen, done) = collect_until(&mut rx, |e| matches!(e, Event::TurnComplete { .. })).await;
    let Event::TurnComplete { usage, .. } = done else {
        unreachable!("collect_until matched TurnComplete");
    };
    assert_eq!(usage.completion_tokens, 6);
    let mut text = String::new();
    for event in &seen {
        if let Event::AssistantDelta { text: chunk } = event {
            text.push_str(chunk);
        }
    }
    assert_eq!(text, "hello world.");
    // The five chunks arrive inside one SSE body, so the 16 ms coalescer merges them;
    // a machine hiccup could flush between chunks, so only bound the count from above
    // and leave the strict merge to the coalescer unit tests.
    let deltas = seen
        .iter()
        .filter(|e| matches!(e, Event::AssistantDelta { .. }))
        .count();
    assert!(deltas <= words.len(), "{deltas} delta events for 5 chunks");
}

#[tokio::test]
async fn approval_round_trip_over_send_op() {
    let server = MockServer::start().await;
    // Step 2: the request carries the tool result; the model answers.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("\"role\":\"tool\""))
        .respond_with(sse_text("Done."))
        .mount(&server)
        .await;
    // Step 1: the model asks to write a file.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse_write_call("c1", "notes.txt", "hi\n"))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let (mut registry, tab, mut rx) = open(tmp.path(), &server).await;
    registry
        .send_op(
            tab,
            serde_json::json!({"op":"user_turn","items":[{"type":"text","text":"write notes"}],"overrides":{}}),
        )
        .await
        .expect("send the turn");

    let (_seen, approval) =
        collect_until(&mut rx, |e| matches!(e, Event::ApprovalRequest { .. })).await;
    let Event::ApprovalRequest {
        call_id, request, ..
    } = approval
    else {
        unreachable!("collect_until matched ApprovalRequest");
    };
    assert!(
        matches!(&request,
            ApprovalRequest::Edit { path, diff }
                if path == "notes.txt" && diff.contains("+hi")),
        "{request:?}"
    );
    registry
        .send_op(
            tab,
            serde_json::json!({"op":"approval_response","call_id": call_id.0, "decision":{"decision":"allow_once"}}),
        )
        .await
        .expect("send the approval");

    let (seen, done) = collect_until(&mut rx, |e| matches!(e, Event::TurnComplete { .. })).await;
    assert!(matches!(done, Event::TurnComplete { .. }));
    assert!(seen.iter().any(|e| matches!(
        e,
        Event::ToolCallEnd { result, .. } if result.ok
    )));
    let mut text = String::new();
    for event in &seen {
        if let Event::AssistantDelta { text: chunk } = event {
            text.push_str(chunk);
        }
    }
    assert_eq!(text, "Done.");
    assert_eq!(
        std::fs::read_to_string(ws_of(tmp.path()).join("notes.txt")).expect("written"),
        "hi\n"
    );
    registry.close_all().await;
}

#[tokio::test]
async fn refuses_a_second_engine_on_the_same_session() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse_text("ok"))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let (mut registry, tab, mut rx) = open(tmp.path(), &server).await;

    let (_seen, configured) =
        collect_until(&mut rx, |e| matches!(e, Event::SessionConfigured { .. })).await;
    let Event::SessionConfigured { session } = configured else {
        unreachable!("collect_until matched SessionConfigured");
    };
    // A turn, so the session log exists on disk and `Resume::Id` finds it — the way a
    // real duplicate open always happens (the session was listed, so it has history).
    registry
        .send_op(
            tab,
            serde_json::json!({"op":"user_turn","items":[{"type":"text","text":"hi"}],"overrides":{}}),
        )
        .await
        .expect("send the turn");
    let (_seen, _done) = collect_until(&mut rx, |e| matches!(e, Event::TurnComplete { .. })).await;

    let loaded = common::workspace(tmp.path(), &server.uri());
    let (dir, entries) = quack_core::list_sessions(&loaded);
    let entry = entries
        .iter()
        .find(|e| e.id == session.id.0)
        .expect("saved session");
    let log = dir.join(&entry.dir).join("events.jsonl");
    let before = std::fs::read(&log).expect("read log");
    let (sink2, mut rx2) = collecting_sink();
    let err = registry
        .open(ws_of(tmp.path()), Resume::Id(session.id.0.clone()), sink2)
        .await
        .expect_err("the same session may not open twice");
    let OpenError::AlreadyOpen { tab: existing } = err else {
        panic!("wrong error: {err}");
    };
    assert_eq!(existing, tab);
    assert_eq!(
        std::fs::read(&log).expect("read log"),
        before,
        "reject the duplicate BEFORE the resume can append to its log"
    );
    assert!(
        rx2.try_recv().is_err(),
        "the refused open must not have sent anything"
    );
    assert_eq!(registry.tab_of(&session.id), Some(tab));
    registry.close_all().await;
    assert!(registry.is_empty());
}

#[tokio::test]
async fn close_announces_and_stops_the_engine() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let (mut registry, tab, mut rx) = open(tmp.path(), &server).await;
    assert!(!registry.is_empty());
    registry.close(tab).await;
    let msg = until(&mut rx, |m| matches!(&m.payload, Payload::Closed { .. })).await;
    assert_eq!(msg.tab, tab);
    assert!(registry.is_empty());
    registry.close(tab).await;
}

#[tokio::test]
async fn send_op_rejects_garbage_and_unknown_tabs() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let (registry, tab, _rx) = open(tmp.path(), &server).await;
    let e = registry
        .send_op(tab, serde_json::json!({"op":"not_a_real_op"}))
        .await
        .expect_err("garbage is refused");
    assert!(e.contains("not a protocol op"), "{e}");
    let e = registry
        .send_op(999, serde_json::json!({"op":"list_models"}))
        .await
        .expect_err("unknown tabs are refused");
    assert!(e.contains("no open tab 999"), "{e}");
}

#[tokio::test]
async fn opens_without_a_ready_model_even_when_latest_was_requested() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = common::workspace(tmp.path(), &server.uri());
    std::fs::remove_file(tmp.path().join("user/providers.d/mock.toml")).expect("remove provider");
    let mut registry = Registry::with_parts(fixed_loader(loaded), tmp.path().join("gui-data"));
    let (sink, mut rx) = collecting_sink();
    let tab = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        registry.open(ws_of(tmp.path()), Resume::Latest, sink),
    )
    .await
    .expect("open does not wait for model readiness")
    .expect("open");
    collect_until(&mut rx, |ev| matches!(ev, Event::StartupComplete)).await;
    registry.close(tab).await;
}

#[tokio::test]
async fn workspace_ownership_survives_new_session_and_releases_on_close() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let (mut registry, tab, mut rx) = open(tmp.path(), &server).await;
    collect_until(&mut rx, |ev| matches!(ev, Event::StartupComplete)).await;
    let (sink, _rx2) = collecting_sink();
    assert!(
        matches!(registry.open(ws_of(tmp.path()), Resume::New, sink).await,
        Err(OpenError::AlreadyOpen { tab: existing }) if existing == tab)
    );
    registry
        .send_op(tab, serde_json::json!({"op":"new_session"}))
        .await
        .expect("new session");
    let (_, configured) =
        collect_until(&mut rx, |ev| matches!(ev, Event::SessionConfigured { .. })).await;
    let Event::SessionConfigured { session } = configured else {
        unreachable!()
    };
    assert_eq!(registry.tab_of(&session.id), Some(tab));
    registry.close(tab).await;
    let (sink, _rx3) = collecting_sink();
    registry
        .open(ws_of(tmp.path()), Resume::New, sink)
        .await
        .expect("workspace ownership released");
    registry.close_all().await;
}
