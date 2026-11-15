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
    let registry = Registry::with_parts(fixed_loader(loaded), root.join("gui-data"));
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

fn warnings_in(events: &[Event]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Warning { message } => Some(message.as_str()),
            _ => None,
        })
        .collect()
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
    assert!(info.settings.iter().any(|s| s.key == "memory.path"));
    assert!(info.paths.memory_global.ends_with("memory"));
    assert!(info.paths.sessions_root.ends_with("sessions"));
    assert!(!info.paths.user_config.is_empty());
    assert!(!info.project_status.is_empty());
    assert!(!info.egress.offline);
    assert!(!info.egress.telemetry_enabled);
    assert!(
        info.egress
            .providers
            .iter()
            .any(|p| p.id == "mock" && p.local && p.enabled),
        "{:?}",
        info.egress
    );
    assert_eq!(info.sandbox.mode, "auto");
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
    let (registry, tab, mut rx) = open(tmp.path(), &server).await;
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
    let (registry, tab, mut rx) = open(tmp.path(), &server).await;

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
    let (registry, tab, mut rx) = open(tmp.path(), &server).await;
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
    std::fs::remove_file(
        tmp.path()
            .join("user")
            .join("providers.d")
            .join("mock.toml"),
    )
    .expect("remove provider");
    let registry = Registry::with_parts(fixed_loader(loaded), tmp.path().join("gui-data"));
    let (sink, mut rx) = collecting_sink();
    let tab = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        registry.open(ws_of(tmp.path()), Resume::Latest, sink),
    )
    .await
    .expect("open does not wait for model readiness")
    .expect("open");
    let (events, _) = collect_until(&mut rx, |ev| matches!(ev, Event::StartupComplete)).await;
    let warnings = warnings_in(&events);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    registry.close(tab).await;
}

#[tokio::test]
async fn workspace_ownership_survives_new_session_and_releases_on_close() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let (registry, tab, mut rx) = open(tmp.path(), &server).await;
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

#[tokio::test]
async fn first_run_setup_adds_a_server_and_selects_a_model_through_ipc() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"data": [{"id": "org/tiny"}]})),
        )
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = common::workspace(tmp.path(), &server.uri());
    std::fs::remove_file(
        tmp.path()
            .join("user")
            .join("providers.d")
            .join("mock.toml"),
    )
    .expect("remove provider");
    let registry = Registry::with_parts(fixed_loader(loaded), tmp.path().join("gui-data"));
    let (sink, mut rx) = collecting_sink();
    let tab = registry
        .open(ws_of(tmp.path()), Resume::New, sink)
        .await
        .expect("open");
    collect_until(&mut rx, |e| matches!(e, Event::StartupComplete)).await;
    registry
        .send_op(
            tab,
            serde_json::json!({
                "op": "add_server", "base_url": format!("{}/v1", server.uri())
            }),
        )
        .await
        .expect("add server");
    let (_, catalog) = collect_until(&mut rx, |e| matches!(e, Event::ModelCatalog { .. })).await;
    let Event::ModelCatalog { providers, .. } = catalog else {
        unreachable!()
    };
    let provider = providers.first().expect("the connected provider");
    assert!(provider.available, "{provider:?}");
    assert_eq!(provider.models[0].id, "org/tiny");
    registry
        .send_op(
            tab,
            serde_json::json!({
                "op": "set_model", "provider": provider.id, "model": "org/tiny"
            }),
        )
        .await
        .expect("select model");
    let (_, configured) =
        collect_until(&mut rx, |e| matches!(e, Event::SessionConfigured { .. })).await;
    let Event::SessionConfigured { session } = configured else {
        unreachable!()
    };
    assert_eq!(session.model, Some(format!("{}/org/tiny", provider.id)));
    assert!(
        server
            .received_requests()
            .await
            .expect("requests")
            .iter()
            .all(|r| r.method == "GET")
    );
    registry.close(tab).await;
}

#[tokio::test]
async fn project_trust_choices_round_trip_through_ipc_and_reload_the_snapshot() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = ws_of(tmp.path()).join(".quack");
    std::fs::create_dir_all(&project).expect("project config dir");
    std::fs::write(project.join("config.toml"), "[ui]\ntheme = \"dark\"\n")
        .expect("project settings");
    let loaded = common::workspace(tmp.path(), &server.uri());
    let registry = Registry::with_parts(fixed_loader(loaded), tmp.path().join("gui-data"));
    let (sink, mut rx) = collecting_sink();
    let tab = registry
        .open(ws_of(tmp.path()), Resume::New, sink)
        .await
        .expect("open");
    let (seen, _) = collect_until(&mut rx, |e| matches!(e, Event::StartupComplete)).await;
    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::ProjectTrust { changed: false, .. }))
    );
    registry
        .send_op(
            tab,
            serde_json::json!({
                "op": "trust_project", "decision": "settings-only"
            }),
        )
        .await
        .expect("trust settings");
    collect_until(&mut rx, |e| matches!(e, Event::ConfigReloaded)).await;
    let message = until(&mut rx, |m| matches!(m.payload, Payload::Info { .. })).await;
    let Payload::Info { info } = message.payload else {
        unreachable!()
    };
    assert_eq!(info.ui.theme, "dark");
    registry.close(tab).await;
}

#[tokio::test]
async fn workspace_files_path_resolution_and_clipboard_image_staging_stay_safe() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let ws = ws_of(tmp.path());
    std::fs::create_dir_all(ws.join("src")).expect("mkdir src");
    std::fs::write(ws.join("src/main.rs"), "fn main() {}\n").expect("write main.rs");
    std::fs::write(tmp.path().join("outside.txt"), "secret").expect("write outside");

    let loaded = common::workspace(tmp.path(), &server.uri());
    let registry = Registry::with_parts(fixed_loader(loaded), tmp.path().join("gui-data"));
    let client = registry.connect().await.expect("connect");
    let (sink, _rx) = collecting_sink();
    let tab = registry
        .open_for(client, ws.clone(), Resume::New, sink)
        .await
        .expect("open");

    let files = registry
        .list_files_for(client, tab, 50)
        .await
        .expect("list files");
    assert!(files.iter().any(|f| f == "src/main.rs"), "{files:?}");

    let resolved = registry
        .resolve_workspace_path_for(client, tab, Path::new("src/main.rs"))
        .expect("resolve inside workspace");
    assert!(resolved.ends_with("src/main.rs"));

    let escaped = registry.resolve_workspace_path_for(client, tab, Path::new("../outside.txt"));
    assert!(escaped.is_err(), "path escape must fail: {escaped:?}");

    let png: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, b'I', b'H', b'D', b'R', 0,
        0, 0, 2, 0, 0, 0, 1, 8, 2, 0, 0, 0, 0x7D, 0x0C, 0xC0, 0xFA,
    ];
    let staged = registry
        .save_pasted_image_for(client, tab, png)
        .expect("stage valid png");
    assert!(Path::new(&staged).is_file());
    let _ = std::fs::remove_file(&staged);
    assert!(
        registry
            .save_pasted_image_for(client, tab, b"not an image")
            .is_err()
    );

    registry.close(tab).await;
}

#[tokio::test]
async fn shares_input_history_and_memory_edits_with_tui() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_text("ok"))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = common::workspace(tmp.path(), &server.uri());
    let hist_path = quack_core::history_path(&loaded);
    quack_core::append_history(&hist_path, "from-tui").expect("seed tui prompt");

    let registry = Registry::with_parts(fixed_loader(loaded), tmp.path().join("gui-data"));
    let client = registry.connect().await.expect("connect");
    let (sink, mut rx) = collecting_sink();
    let tab = registry
        .open_for(client, ws_of(tmp.path()), Resume::New, sink)
        .await
        .expect("open");

    let info_msg = until(&mut rx, |m| matches!(m.payload, Payload::Info { .. })).await;
    let Payload::Info { info } = info_msg.payload else {
        unreachable!()
    };
    assert_eq!(info.history, vec!["from-tui"]);

    collect_until(&mut rx, |e| matches!(e, Event::StartupComplete)).await;

    registry
        .send_op_for(
            client,
            tab,
            serde_json::json!({
                "op": "user_turn",
                "items": [{"type": "text", "text": "from-gui"}],
                "overrides": {}
            }),
        )
        .await
        .expect("user_turn");
    let hist_msg = until(&mut rx, |m| matches!(m.payload, Payload::History { .. })).await;
    let Payload::History { history } = hist_msg.payload else {
        unreachable!()
    };
    assert_eq!(history, vec!["from-tui", "from-gui"]);
    collect_until(&mut rx, |e| matches!(e, Event::TurnComplete { .. })).await;
    assert_eq!(
        quack_core::load_history(&hist_path),
        vec!["from-tui", "from-gui"]
    );

    quack_core::append_history(&hist_path, "later-from-tui").expect("tui append");
    let refreshed = registry
        .input_history_for(client, tab)
        .await
        .expect("refresh history");
    assert_eq!(refreshed, vec!["from-tui", "from-gui", "later-from-tui"]);

    registry
        .send_op_for(
            client,
            tab,
            serde_json::json!({
                "op": "save_memory",
                "scope": "project",
                "name": "bridge-note",
                "description": "Created via bridge",
                "kind": "project",
                "body": "Remember this across TUI and GUI."
            }),
        )
        .await
        .expect("save_memory");
    let (_, mem_ev) = collect_until(&mut rx, |e| matches!(e, Event::MemoryNotes { .. })).await;
    let Event::MemoryNotes { notes, .. } = mem_ev else {
        unreachable!()
    };
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].name, "bridge-note");

    registry.close(tab).await;
}

#[tokio::test]
async fn save_setting_for_memory_path_updates_resolved_paths_on_reload() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let custom_memory = tmp.path().join("custom-memory");
    let expected_value = serde_json::Value::String(custom_memory.display().to_string()).to_string();
    let (registry, tab, mut rx) = open(tmp.path(), &server).await;
    collect_until(&mut rx, |e| matches!(e, Event::StartupComplete)).await;

    registry
        .send_op(
            tab,
            serde_json::json!({
                "op": "save_setting",
                "key": "memory.path",
                "value": expected_value,
                "scope": "user"
            }),
        )
        .await
        .expect("save memory.path");
    collect_until(
        &mut rx,
        |e| matches!(e, Event::SettingSaved { key, .. } if key == "memory.path"),
    )
    .await;

    registry
        .send_op(tab, serde_json::json!({ "op": "reload_config" }))
        .await
        .expect("reload");
    collect_until(&mut rx, |e| matches!(e, Event::ConfigReloaded)).await;
    let message = until(&mut rx, |m| matches!(m.payload, Payload::Info { .. })).await;
    let Payload::Info { info } = message.payload else {
        unreachable!()
    };
    assert_eq!(
        info.paths.memory_global,
        custom_memory.display().to_string()
    );
    registry.close(tab).await;
}

#[tokio::test]
async fn hooks_and_mcp_snapshot_and_save_enable_round_trip() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let user_hooks = tmp.path().join("user").join("hooks.d");
    std::fs::create_dir_all(&user_hooks).expect("mkdir hooks.d");
    std::fs::write(
        user_hooks.join("pre-edit.toml"),
        "enabled = false\ndescription = \"Verify edit prerequisites\"\nevent = \"pre_tool\"\ncommand = [\"true\"]\n",
    )
    .expect("write hook");

    let (registry, tab, mut rx) = open(tmp.path(), &server).await;
    let initial = until(&mut rx, |m| matches!(m.payload, Payload::Info { .. })).await;
    let Payload::Info { info } = initial.payload else {
        unreachable!()
    };
    let hook = info
        .hooks
        .iter()
        .find(|h| h.id == "pre-edit")
        .expect("pre-edit hook in snapshot");
    assert!(!hook.enabled);
    assert!(hook.can_toggle);
    assert_eq!(hook.event.as_deref(), Some("pre_tool"));
    assert_eq!(hook.description, "Verify edit prerequisites");

    collect_until(&mut rx, |e| matches!(e, Event::StartupComplete)).await;

    registry
        .send_op(
            tab,
            serde_json::json!({
                "op": "save_mcp_server",
                "id": "local-tools",
                "scope": "user",
                "description": "Local loopback MCP",
                "command": "",
                "args": [],
                "cwd": "",
                "url": "http://127.0.0.1:1/mcp",
                "env": {},
                "headers": {
                    "Authorization": { "kind": "env", "env": "LOCAL_MCP_TOKEN", "format": "Bearer {}" }
                }
            }),
        )
        .await
        .expect("save_mcp_server");
    collect_until(
        &mut rx,
        |e| matches!(e, Event::McpServerSaved { id, .. } if id == "local-tools"),
    )
    .await;
    let msg = until(&mut rx, |m| matches!(m.payload, Payload::Info { .. })).await;
    let Payload::Info { info } = msg.payload else {
        unreachable!()
    };
    let mcp = info
        .mcp
        .iter()
        .find(|s| s.id == "local-tools")
        .expect("saved mcp server in snapshot");
    assert!(mcp.enabled);
    assert!(mcp.can_toggle);
    assert_eq!(mcp.transport.as_deref(), Some("http"));
    assert_eq!(mcp.target, "http://127.0.0.1:1/mcp");
    assert_eq!(mcp.header_keys, vec!["Authorization"]);
    collect_until(&mut rx, |e| matches!(e, Event::StartupComplete)).await;

    registry
        .send_op(
            tab,
            serde_json::json!({
                "op": "save_setting",
                "key": "mcp.disabled",
                "value": "[\"local-tools\"]",
                "scope": "user"
            }),
        )
        .await
        .expect("disable mcp");
    collect_until(
        &mut rx,
        |e| matches!(e, Event::SettingSaved { key, .. } if key == "mcp.disabled"),
    )
    .await;
    registry
        .send_op(tab, serde_json::json!({ "op": "reload_config" }))
        .await
        .expect("reload");
    let msg = until(&mut rx, |m| matches!(m.payload, Payload::Info { .. })).await;
    let Payload::Info { info } = msg.payload else {
        unreachable!()
    };
    assert_eq!(info.mcp_disabled, vec!["local-tools"]);
    assert!(
        !info
            .mcp
            .iter()
            .find(|s| s.id == "local-tools")
            .expect("mcp")
            .enabled
    );
    collect_until(&mut rx, |e| matches!(e, Event::StartupComplete)).await;

    registry
        .send_op(
            tab,
            serde_json::json!({ "op": "enable_mcp", "id": "local-tools" }),
        )
        .await
        .expect("enable_mcp");
    collect_until(
        &mut rx,
        |e| matches!(e, Event::McpEnabled { id, .. } if id == "local-tools"),
    )
    .await;
    let msg = until(&mut rx, |m| matches!(m.payload, Payload::Info { .. })).await;
    let Payload::Info { info } = msg.payload else {
        unreachable!()
    };
    assert!(info.mcp_disabled.is_empty());
    assert!(
        info.mcp
            .iter()
            .find(|s| s.id == "local-tools")
            .expect("mcp")
            .enabled
    );
    collect_until(&mut rx, |e| matches!(e, Event::StartupComplete)).await;

    registry
        .send_op(
            tab,
            serde_json::json!({ "op": "enable_hook", "id": "pre-edit" }),
        )
        .await
        .expect("enable_hook");
    collect_until(
        &mut rx,
        |e| matches!(e, Event::HookEnabled { id, .. } if id == "pre-edit"),
    )
    .await;
    let msg = until(&mut rx, |m| matches!(m.payload, Payload::Info { .. })).await;
    let Payload::Info { info } = msg.payload else {
        unreachable!()
    };
    assert!(
        info.hooks
            .iter()
            .find(|h| h.id == "pre-edit")
            .expect("hook")
            .enabled
    );

    registry.close(tab).await;
}

#[tokio::test]
async fn opening_new_project_with_latest_emits_no_warning() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(r#"{"data":[]}"#))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = common::workspace(tmp.path(), &server.uri());
    let registry = Registry::with_parts(fixed_loader(loaded), tmp.path().join("gui-data"));
    let (sink, mut rx) = collecting_sink();
    let tab = registry
        .open(ws_of(tmp.path()), Resume::Latest, sink)
        .await
        .expect("open");
    let (events, _) = collect_until(&mut rx, |e| matches!(e, Event::StartupComplete)).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::SessionConfigured { .. })),
        "expected SessionConfigured before StartupComplete: {events:?}"
    );
    let warnings = warnings_in(&events);
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    registry.close(tab).await;
}

#[tokio::test]
async fn bridge_respects_updates_check_off_and_runs_manual_check_updates() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(r#"{"data":[]}"#))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/releases/latest"))
        .respond_with(
            wiremock::ResponseTemplate::new(302).insert_header("location", "/releases/tag/v99.0.0"),
        )
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut loaded = common::workspace(tmp.path(), &server.uri());
    loaded.settings.updates.check = false;
    loaded.settings.updates.url = format!("{}/releases/latest", server.uri());
    let registry = Registry::with_parts(fixed_loader(loaded), tmp.path().join("gui-data"));
    let (sink, mut rx) = collecting_sink();
    let tab = registry
        .open(ws_of(tmp.path()), Resume::New, sink)
        .await
        .expect("open");
    match next(&mut rx).await.payload {
        Payload::Info { info } => assert!(!info.egress.updates_check),
        other => panic!("expected AppInfo first, got {other:?}"),
    }
    let _ = collect_until(&mut rx, |e| matches!(e, Event::StartupComplete)).await;
    let before = server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| r.url.path().contains("releases"))
        .count();
    assert_eq!(
        before, 0,
        "auto-check must stay off when updates.check = false"
    );

    registry
        .send_op(tab, serde_json::json!({ "op": "check_updates" }))
        .await
        .expect("send check_updates");
    let (_, result) =
        collect_until(&mut rx, |e| matches!(e, Event::UpdateCheckResult { .. })).await;
    assert_eq!(
        result,
        Event::UpdateCheckResult {
            current: quack_core::checks::CURRENT.to_owned(),
            latest: Some("99.0.0".to_owned()),
            update_available: true,
            url: Some(format!("{}/releases/latest", server.uri())),
            error: None,
        }
    );
    registry.close(tab).await;
}
