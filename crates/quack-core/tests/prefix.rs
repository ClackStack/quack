//! Prefix stability (docs/plan/13-testing-and-evals.md#invariant-tests-must-never-regress):
//! for every shipped profile, the system prompt and tool block are byte-identical across
//! the requests of a session, and each request strictly extends the one before it without
//! mutating earlier messages, so server KV caches stay warm. Plan and working-file
//! reminders are written into history only when they change, and only compaction (an epoch
//! boundary) may rewrite the history.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quack_config::{LoadOptions, Loaded, load_with};
use quack_core::protocol::{ApprovalDecision, Event, InputItem, Op, TurnOverrides};
use quack_core::{Engine, EngineHandle};
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const SUMMARIZER: &str = "You summarize a coding session";

fn sse(chunks: &[Value]) -> ResponseTemplate {
    let body: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body + "data: [DONE]\n\n")
}

fn text(t: &str) -> ResponseTemplate {
    sse(&[json!({"choices":[{"delta":{"content":t},"finish_reason":"stop"}]})])
}

/// The name a profile gives an internal tool.
fn named(profile: &str, internal: &str) -> String {
    let path = defaults_bundle()
        .join("profiles.d")
        .join(profile)
        .join("profile.toml");
    let file: quack_config::profile::ProfileFile =
        toml::from_str(&std::fs::read_to_string(path).expect("profile")).expect("profile toml");
    file.tools
        .iter()
        .find(|t| t.maps_to.as_deref().unwrap_or(&t.name) == internal)
        .map_or_else(|| internal.to_owned(), |t| t.name.clone())
}

fn call(id: &str, name: &str, args: Value) -> ResponseTemplate {
    sse(&[
        json!({"choices":[{"delta":{"content":"On it."}}]}),
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":id,"function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":"tool_calls"}]}),
    ])
}

/// Replies in order, then "ok" for anything after.
struct Script(Mutex<VecDeque<ResponseTemplate>>);

impl Respond for Script {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        self.0
            .lock()
            .ok()
            .and_then(|mut q| q.pop_front())
            .unwrap_or_else(|| text("ok"))
    }
}

fn defaults_bundle() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../defaults")
}

fn profiles() -> Vec<String> {
    let mut ids: Vec<String> = std::fs::read_dir(defaults_bundle().join("profiles.d"))
        .expect("profiles.d")
        .flatten()
        .filter(|e| e.path().join("profile.toml").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort();
    ids
}

fn configure(root: &Path, base_url: &str, profile: &str, config: &str) -> Loaded {
    let user = root.join("user");
    std::fs::create_dir_all(user.join("providers.d")).expect("mkdir");
    std::fs::write(
        user.join("providers.d").join("mock.toml"),
        format!(
            "base_url = \"{base_url}/v1\"\nmodels = [{{ id = \"tiny\", profile = \"{profile}\" }}]\n"
        ),
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
    vars.insert(
        "QUACK_DEFAULTS_DIR".into(),
        defaults_bundle().into_os_string(),
    );
    let ws = root.join("ws");
    std::fs::create_dir_all(&ws).expect("mkdir");
    load_with(&move |k| vars.get(k).cloned(), &LoadOptions::new(ws)).expect("load")
}

async fn next(engine: &mut EngineHandle) -> Event {
    tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
        .await
        .expect("event within 10s")
        .expect("channel open")
}

/// Wait for the end of a turn (or `/compact`), approving whatever is asked.
async fn finish(engine: &mut EngineHandle) {
    loop {
        match next(engine).await {
            Event::ApprovalRequest { call_id, .. } => {
                engine
                    .submit(Op::ApprovalResponse {
                        call_id,
                        decision: ApprovalDecision::AllowOnce,
                    })
                    .await;
            }
            Event::TurnComplete { .. } => return,
            Event::TurnAborted { reason, .. } => panic!("aborted: {reason}"),
            Event::Error { error } => panic!("engine error: {}", error.message),
            _ => {}
        }
    }
}

async fn turn(engine: &mut EngineHandle, text: &str) {
    engine
        .submit(Op::UserTurn {
            items: vec![InputItem::text(text)],
            overrides: TurnOverrides::default(),
        })
        .await;
    finish(engine).await;
}

async fn mode(engine: &mut EngineHandle, mode: &str) {
    engine
        .submit(Op::SetPermissionMode {
            mode: mode.parse().expect("mode"),
        })
        .await;
    loop {
        if let Event::PermissionMode { .. } = next(engine).await {
            return;
        }
    }
}

/// Every request after the first strictly extends the one before it: the same system
/// prompt, tools and parameters, and every earlier message (including the previous
/// request's last message) byte-identical.
fn assert_extends(profile: &str, i: usize, before: &Value, after: &Value) {
    let strip = |b: &Value| {
        let mut b = b.clone();
        b["messages"] = Value::Null;
        b
    };
    assert_eq!(
        strip(before),
        strip(after),
        "{profile}: request {i} changed the tools or parameters"
    );
    let a = before["messages"].as_array().expect("messages");
    let b = after["messages"].as_array().expect("messages");
    assert!(b.len() > a.len(), "{profile}: request {i} dropped messages");
    for (k, m) in a.iter().enumerate() {
        assert_eq!(m, &b[k], "{profile}: request {i} changed message {k}");
    }
}

async fn session(profile: &str, config: &str) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains(SUMMARIZER))
        .respond_with(text("SUMMARY-TEXT"))
        .mount(&server)
        .await;
    let call = |id: &str, internal: &str, args: Value| call(id, &named(profile, internal), args);
    let script = [
        // Turn 1: write, read, a plan, a memory note.
        call(
            "c1",
            "write",
            json!({"path":"notes.txt","content":"one\ntwo\n"}),
        ),
        call("c2", "read", json!({"path":"notes.txt"})),
        call(
            "c3",
            "todo",
            json!({"items":[{"text":"edit notes","status":"in_progress"}]}),
        ),
        call(
            "c4",
            "memory",
            json!({"action":"create","scope":"project","name":"notes","description":"The notes file","type":"project","content":"notes.txt holds two lines."}),
        ),
        text("Written."),
        // Turn 2, in plan mode: a read-only look around.
        call("c5", "list_dir", json!({"path":"."})),
        text("Here is the plan."),
        // Turn 3: edit and a command.
        call(
            "c6",
            "edit",
            json!({"path":"notes.txt","old_string":"two","new_string":"three"}),
        ),
        call("c7", "shell", json!({"command":"echo done"})),
        text("Edited."),
        // Turn 4: a plain answer.
        text("Fine."),
    ];
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(Script(Mutex::new(script.into())))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = configure(tmp.path(), &server.uri(), profile, config);
    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(&mut engine, "write notes").await;
    mode(&mut engine, "plan").await;
    turn(&mut engine, "plan the edit").await;
    mode(&mut engine, "full-auto").await;
    turn(&mut engine, "do the edit").await;
    turn(&mut engine, "anything else?").await;
    // An epoch boundary: the history is rewritten, the system prompt and tools are not.
    engine.submit(Op::Compact { focus: None }).await;
    finish(&mut engine).await;
    turn(&mut engine, "after compaction").await;
    engine.shutdown().await;

    let requests = server.received_requests().await.unwrap_or_default();
    let bodies: Vec<Value> = requests
        .iter()
        .filter(|r| r.url.path().ends_with("/chat/completions"))
        .filter(|r| !String::from_utf8_lossy(&r.body).contains(SUMMARIZER))
        .map(|r| serde_json::from_slice(&r.body).expect("json body"))
        .collect();
    assert_eq!(bodies.len(), 12, "{profile}: {} requests", bodies.len());
    let (live, compacted) = bodies.split_at(11);
    for (i, pair) in live.windows(2).enumerate() {
        assert_extends(profile, i + 1, &pair[0], &pair[1]);
    }
    // Plan (and modified-files when the profile exposes `write`) reminders were written
    // into history on state changes and not duplicated on later unchanged tool steps.
    let final_live = live.last().expect("live").to_string();
    assert_eq!(
        final_live.matches("[Your plan").count(),
        1,
        "{profile}: plan reminder should be emitted once when changed"
    );
    if tmp.path().join("ws").join("notes.txt").exists() {
        assert_eq!(
            final_live
                .matches("[Modified files in this session:")
                .count(),
            1,
            "{profile}: modified-files reminder should be emitted once when changed"
        );
    }
    let system = |b: &Value| b["messages"][0].clone();
    let tools = |b: &Value| b["tools"].clone();
    assert_eq!(system(&live[0])["role"], "system", "{profile}");
    assert_eq!(
        system(&live[0]),
        system(&compacted[0]),
        "{profile}: compaction changed the system prompt"
    );
    assert_eq!(
        tools(&live[0]),
        tools(&compacted[0]),
        "{profile}: compaction changed the tools"
    );
    assert!(
        compacted[0].to_string().contains("SUMMARY-TEXT"),
        "{profile}: not compacted"
    );
}

#[tokio::test]
async fn every_profile_keeps_its_prefix_across_turns() {
    for p in profiles() {
        session(
            &p,
            "[permissions]\nwork_mode = \"edit\"\nmode = \"full-auto\"\n",
        )
        .await;
    }
}

#[tokio::test]
async fn hashline_edits_keep_the_prefix_too() {
    for p in profiles() {
        let path = defaults_bundle()
            .join("profiles.d")
            .join(&p)
            .join("profile.toml");
        let file: quack_config::profile::ProfileFile =
            toml::from_str(&std::fs::read_to_string(path).expect("profile")).expect("profile toml");
        // Incompatible exclusive surfaces are rejected at startup (see edit_formats.rs).
        if file.exclusive
            && !file
                .tools
                .iter()
                .any(|t| t.maps_to.as_deref().unwrap_or(&t.name) == "edit_hashline")
        {
            continue;
        }
        session(
            &p,
            "[permissions]\nwork_mode = \"edit\"\nmode = \"full-auto\"\n[tools]\nedit_format = \"hashline\"\n",
        )
        .await;
    }
}

/// The system prompt and tools don't depend on the workspace: what does (the path, the
/// files, AGENTS.md) rides at the start of the first user message, so a server's prompt
/// cache reuses the system prompt and tool block across sessions and projects.
#[tokio::test]
async fn system_prompt_and_tools_are_the_same_in_every_workspace() {
    for profile in profiles() {
        let mut bodies = Vec::new();
        for project in ["alpha", "beta"] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(text("ok"))
                .mount(&server)
                .await;
            let tmp = tempfile::tempdir().expect("tempdir");
            let ws = tmp.path().join("ws");
            std::fs::create_dir_all(&ws).expect("mkdir");
            std::fs::write(ws.join("AGENTS.md"), format!("Use tabs in {project}.\n")).expect("md");
            std::fs::write(ws.join(format!("{project}.py")), "x = 1\n").expect("file");
            let loaded = configure(tmp.path(), &server.uri(), &profile, "");
            let mut engine = Engine::spawn(Arc::new(loaded));
            turn(&mut engine, "hello").await;
            engine.shutdown().await;
            let requests = server.received_requests().await.unwrap_or_default();
            let body: Value = requests
                .iter()
                .find_map(|r| serde_json::from_slice(&r.body).ok())
                .expect("a chat request");
            bodies.push((project, body));
        }
        let [(_, a), (_, b)] = &bodies[..] else {
            unreachable!()
        };
        assert_eq!(a["messages"][0]["role"], "system", "{profile}");
        assert_eq!(
            a["messages"][0], b["messages"][0],
            "{profile}: the system prompt changed with the workspace"
        );
        assert_eq!(a["tools"], b["tools"], "{profile}: the tools changed");
        for (project, body) in &bodies {
            let first = body["messages"][1]["content"].as_str().expect("user");
            assert!(first.starts_with("<context>"), "{profile}: {first}");
            assert!(first.contains("</context>\n\nhello"), "{profile}: {first}");
            assert!(
                first.contains(&format!("Use tabs in {project}.")),
                "{profile}"
            );
            assert!(first.contains(&format!("{project}.py")), "{profile}");
            assert!(!first.contains("{{") && !first.contains("{%"), "{profile}");
        }
    }
}

/// Responses API sessions are strictly append-only (no trailing reminder stripped from prior
/// messages) and emit reminders only when the plan or working-file set actually changes.
#[tokio::test]
async fn responses_session_is_strictly_append_only_with_delta_reminders() {
    if !quack_providers::supports("openai-responses") {
        return;
    }
    let resp_call = |id: &str, name: &str, args: Value| {
        let events = [
            json!({"type": "response.created", "response": {"id": format!("resp_{id}")}}),
            json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {
                    "type": "function_call",
                    "call_id": id,
                    "name": name,
                }
            }),
            json!({
                "type": "response.function_call_arguments.done",
                "output_index": 0,
                "arguments": args.to_string(),
            }),
            json!({
                "type": "response.output_item.done",
                "output_index": 0,
                "item": {
                    "type": "function_call",
                    "call_id": id,
                    "name": name,
                    "arguments": args.to_string(),
                }
            }),
            json!({"type": "response.completed", "response": {"id": format!("resp_{id}")}}),
        ];
        let body: String = events.iter().map(|e| format!("data: {e}\n\n")).collect();
        ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(body)
    };
    let resp_text = |id: &str, t: &str| {
        let events = [
            json!({"type": "response.created", "response": {"id": id}}),
            json!({"type": "response.output_text.delta", "output_index": 0, "delta": t}),
            json!({
                "type": "response.output_item.done",
                "output_index": 0,
                "item": {
                    "type": "message",
                    "role": "assistant",
                    "phase": "final_answer",
                    "content": [{"type": "output_text", "text": t}]
                }
            }),
            json!({"type": "response.completed", "response": {"id": id}}),
        ];
        let body: String = events.iter().map(|e| format!("data: {e}\n\n")).collect();
        ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(body)
    };

    let server = MockServer::start().await;
    let script = [
        resp_call(
            "c1",
            "write",
            json!({"path": "notes.txt", "content": "one\ntwo\n"}),
        ),
        resp_call("c2", "read", json!({"path": "notes.txt"})),
        resp_call(
            "c3",
            "todo",
            json!({"items": [{"text": "edit notes", "status": "in_progress"}]}),
        ),
        // Same read again: neither plan nor working files changed, so no duplicate reminder.
        resp_call("c4", "read", json!({"path": "notes.txt"})),
        resp_call(
            "c5",
            "edit",
            json!({"path": "notes.txt", "old_string": "two", "new_string": "three"}),
        ),
        resp_text("resp_done", "All done."),
    ];
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(Script(Mutex::new(script.into())))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = configure(
        tmp.path(),
        &server.uri(),
        "generic",
        "[permissions]\nwork_mode = \"edit\"\nmode = \"full-auto\"\n",
    );
    let provider_path = loaded
        .dirs
        .user_config
        .join("providers.d")
        .join("mock.toml");
    std::fs::write(
        provider_path,
        format!(
            "api = \"openai-responses\"\n\
             base_url = \"{}/v1\"\n\
             models = [{{ id = \"tiny\", profile = \"generic\" }}]\n\
             [prompt_cache]\n\
             developer_instructions = true\n\
             explicit_breakpoints = true\n",
            server.uri()
        ),
    )
    .expect("provider");
    // Reload with the updated Responses provider file.
    let mut vars: HashMap<String, OsString> = HashMap::new();
    vars.insert(
        "QUACK_CONFIG_DIR".into(),
        tmp.path().join("user").into_os_string(),
    );
    for (k, sub) in [
        ("QUACK_SYSTEM_DIR", "system"),
        ("QUACK_DATA_DIR", "data"),
        ("QUACK_CACHE_DIR", "cache"),
    ] {
        vars.insert(k.into(), tmp.path().join(sub).into_os_string());
    }
    vars.insert(
        "QUACK_DEFAULTS_DIR".into(),
        defaults_bundle().into_os_string(),
    );
    let loaded = load_with(
        &move |k| vars.get(k).cloned(),
        &LoadOptions::new(tmp.path().join("ws")),
    )
    .expect("reload");

    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(&mut engine, "update notes").await;
    engine.shutdown().await;

    let bodies: Vec<Value> = server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| r.url.path() == "/v1/responses")
        .map(|r| serde_json::from_slice(&r.body).expect("json body"))
        .collect();
    assert_eq!(bodies.len(), 6, "expected 6 responses steps");

    for (i, pair) in bodies.windows(2).enumerate() {
        assert_eq!(pair[0]["tools"], pair[1]["tools"], "step {i} changed tools");
        assert_eq!(
            pair[0]["prompt_cache_key"], pair[1]["prompt_cache_key"],
            "step {i} changed prompt_cache_key"
        );
        assert_eq!(
            pair[0]["prompt_cache_options"], pair[1]["prompt_cache_options"],
            "step {i} changed prompt_cache_options"
        );
        assert!(
            pair[0].get("instructions").is_none(),
            "developer_instructions moves system prompt into input"
        );
        let before = pair[0]["input"].as_array().expect("before input");
        let after = pair[1]["input"].as_array().expect("after input");
        assert!(after.len() > before.len(), "step {i} did not grow");
        assert_eq!(
            before.as_slice(),
            &after[..before.len()],
            "step {i} mutated an earlier Responses input item (breaks KV prefix cache)"
        );
    }

    // Request 3 carries the todo tool output with the plan reminder.
    let after_todo = bodies[3]["input"]
        .as_array()
        .expect("input")
        .last()
        .expect("todo output")["output"]
        .as_str()
        .expect("output text");
    assert!(
        after_todo.contains("[Your plan"),
        "plan reminder missing after todo: {after_todo}"
    );
    // Request 4 carries the repeated read output: neither plan nor working files changed,
    // so the plan reminder must NOT be appended a second time.
    let after_unchanged_read = bodies[4]["input"]
        .as_array()
        .expect("input")
        .last()
        .expect("read output")["output"]
        .as_str()
        .expect("output text");
    assert!(
        !after_unchanged_read.contains("[Your plan"),
        "duplicate plan reminder appended when unchanged: {after_unchanged_read}"
    );
}

#[tokio::test]
async fn per_model_prompt_cache_and_session_or_model_switch_reset_diagnostics() {
    if !quack_providers::supports("openai-responses") {
        return;
    }
    let resp = |id: &str, text: &str| {
        let events = [
            json!({"type": "response.created", "response": {"id": id}}),
            json!({"type": "response.output_text.delta", "output_index": 0, "delta": text}),
            json!({
                "type": "response.output_item.done",
                "output_index": 0,
                "item": {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": text}]
                }
            }),
            json!({"type": "response.completed", "response": {"id": id}}),
        ];
        let body: String = events.iter().map(|e| format!("data: {e}\n\n")).collect();
        ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(body)
    };
    let server = MockServer::start().await;
    let script = [
        resp("resp_1", "one"),
        resp("resp_2", "two"),
        resp("resp_3", "three"),
        resp("resp_4", "four"),
    ];
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(Script(Mutex::new(script.into())))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = configure(tmp.path(), &server.uri(), "generic", "");
    let provider_path = loaded
        .dirs
        .user_config
        .join("providers.d")
        .join("mock.toml");
    std::fs::write(
        provider_path,
        format!(
            "api = \"openai-responses\"\n\
             base_url = \"{}/v1\"\n\
             [[models]]\n\
             id = \"m_62\"\n\
             profile = \"generic\"\n\
             prompt_cache = {{ developer_instructions = true, explicit_breakpoints = true, diagnostics = true }}\n\
             [[models]]\n\
             id = \"m_60\"\n\
             profile = \"generic\"\n",
            server.uri()
        ),
    )
    .expect("provider");
    let mut vars: HashMap<String, OsString> = HashMap::new();
    vars.insert(
        "QUACK_CONFIG_DIR".into(),
        tmp.path().join("user").into_os_string(),
    );
    for (k, sub) in [
        ("QUACK_SYSTEM_DIR", "system"),
        ("QUACK_DATA_DIR", "data"),
        ("QUACK_CACHE_DIR", "cache"),
    ] {
        vars.insert(k.into(), tmp.path().join(sub).into_os_string());
    }
    vars.insert(
        "QUACK_DEFAULTS_DIR".into(),
        defaults_bundle().into_os_string(),
    );
    let loaded = load_with(
        &move |k| vars.get(k).cloned(),
        &LoadOptions::new(tmp.path().join("ws")),
    )
    .expect("reload");

    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(&mut engine, "first").await;
    turn(&mut engine, "second").await;
    // Start a brand new session: comparison_response_id must NOT carry over from the old session.
    engine.submit(Op::NewSession).await;
    loop {
        if matches!(
            engine.events.recv().await.expect("event"),
            Event::SessionConfigured { .. }
        ) {
            break;
        }
    }
    turn(&mut engine, "third").await;
    // Switch to m_60 (which has no prompt_cache options): must NOT send prompt_cache_options
    // and must keep instructions top-level.
    engine
        .submit(Op::SetModel {
            provider: quack_core::protocol::ProviderId("mock".into()),
            model: quack_core::protocol::ModelId("m_60".into()),
        })
        .await;
    loop {
        if matches!(
            engine.events.recv().await.expect("event"),
            Event::SessionConfigured { .. }
        ) {
            break;
        }
    }
    turn(&mut engine, "fourth").await;
    engine.shutdown().await;

    let bodies: Vec<Value> = server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| r.url.path() == "/v1/responses")
        .map(|r| serde_json::from_slice(&r.body).expect("json body"))
        .collect();
    assert_eq!(bodies.len(), 4);

    // Step 1 on m_62: developer_instructions + explicit_breakpoints, no prior response id.
    assert!(bodies[0].get("instructions").is_none());
    assert_eq!(
        bodies[0]["prompt_cache_options"],
        json!({ "mode": "implicit" })
    );
    // Step 2 on m_62: compares against resp_1.
    assert_eq!(
        bodies[1]["prompt_cache_options"]["comparison_response_id"],
        "resp_1"
    );
    // Step 3 after /new: comparison_response_id must be cleared!
    assert!(
        bodies[2]["prompt_cache_options"]
            .get("comparison_response_id")
            .is_none(),
        "new session leaked previous session's comparison_response_id: {}",
        bodies[2]
    );
    // Step 4 after /models mock/m_60: no prompt_cache_options and top-level instructions.
    assert!(
        bodies[3].get("prompt_cache_options").is_none(),
        "m_60 must not inherit m_62's prompt_cache_options: {}",
        bodies[3]
    );
    assert!(bodies[3].get("instructions").is_some());
}

#[tokio::test]
async fn openai_chat_prompt_cache_key_and_affinity_header_stay_stable_across_turns_and_resume() {
    let server = MockServer::start().await;
    let script = [
        call(
            "c1",
            "TodoList",
            json!({"todos":[{"text":"check","status":"in_progress"}]}),
        ),
        text("Turn 1 done."),
        text("Turn 2 done."),
        text("Resumed turn done."),
        text("Fresh session done."),
    ];
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(Script(Mutex::new(script.into())))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = configure(tmp.path(), &server.uri(), "kimi-code", "");
    std::fs::write(
        loaded
            .dirs
            .user_config
            .join("providers.d")
            .join("mock.toml"),
        format!(
            "base_url = \"{}/v1\"\n\
             [prompt_cache]\n\
             cache_key = true\n\
             affinity_header = \"x-session-affinity\"\n\
             [[models]]\n\
             id = \"accounts/fireworks/models/kimi-k3p1-pro\"\n\
             profile = \"kimi-code\"\n",
            server.uri()
        ),
    )
    .expect("provider");
    let reload = || {
        let mut vars: HashMap<String, OsString> = HashMap::new();
        vars.insert(
            "QUACK_CONFIG_DIR".into(),
            tmp.path().join("user").into_os_string(),
        );
        for (k, sub) in [
            ("QUACK_SYSTEM_DIR", "system"),
            ("QUACK_DATA_DIR", "data"),
            ("QUACK_CACHE_DIR", "cache"),
        ] {
            vars.insert(k.into(), tmp.path().join(sub).into_os_string());
        }
        vars.insert(
            "QUACK_DEFAULTS_DIR".into(),
            defaults_bundle().into_os_string(),
        );
        load_with(
            &move |k| vars.get(k).cloned(),
            &LoadOptions::new(tmp.path().join("ws")),
        )
        .expect("reload")
    };

    let mut engine = Engine::spawn(Arc::new(reload()));
    turn(&mut engine, "first").await;
    turn(&mut engine, "second").await;
    engine.shutdown().await;

    let mut resumed = Engine::spawn_with(Arc::new(reload()), quack_core::Resume::Latest);
    turn(&mut resumed, "third").await;
    resumed.submit(Op::NewSession).await;
    loop {
        if matches!(
            resumed.events.recv().await.expect("event"),
            Event::SessionConfigured { .. }
        ) {
            break;
        }
    }
    turn(&mut resumed, "fresh").await;
    resumed.shutdown().await;

    let reqs: Vec<_> = server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.url.path() == "/v1/chat/completions")
        .collect();
    assert_eq!(reqs.len(), 5);
    let bodies: Vec<Value> = reqs
        .iter()
        .map(|r| serde_json::from_slice(&r.body).expect("json"))
        .collect();
    let affinities: Vec<String> = reqs
        .iter()
        .map(|r| {
            r.headers
                .get("x-session-affinity")
                .and_then(|v| v.to_str().ok())
                .expect("x-session-affinity header")
                .to_owned()
        })
        .collect();

    // First 4 requests (2 steps in turn 1, turn 2, resumed turn 3) share one session key.
    let key = bodies[0]["prompt_cache_key"].as_str().expect("cache key");
    assert!(!key.is_empty());
    for i in 0..4 {
        assert_eq!(bodies[i]["prompt_cache_key"], key, "request {i}");
        assert_eq!(affinities[i], key, "affinity header {i}");
    }
    // Strictly append-only across tool step, turn 2, and resumed turn 3.
    for (i, pair) in bodies[..4].windows(2).enumerate() {
        assert_extends("kimi-code", i + 1, &pair[0], &pair[1]);
    }
    // `/new` starts a fresh session key for both body and affinity header.
    let fresh_key = bodies[4]["prompt_cache_key"]
        .as_str()
        .expect("fresh cache key");
    assert_ne!(fresh_key, key);
    assert_eq!(affinities[4], fresh_key);
}

/// Providers like OpenRouter use an affinity header (`x-session-id`) without emitting body
/// `prompt_cache_key` (which non-OpenAI upstream endpoints may reject), and text compaction
/// carries the same session header so all calls in a session stay grouped.
#[tokio::test]
async fn affinity_header_only_omits_body_cache_key_and_tags_text_compaction() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(text("Turn 1 complete."))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(text("Turn 2 complete."))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(text(
            "## Goal\nKeep session sticky.\n\n## Done so far\nCompleted turn 1 and verified header.",
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let user = tmp.path().join("user");
    std::fs::create_dir_all(user.join("providers.d")).expect("mkdir");
    std::fs::create_dir_all(tmp.path().join("ws")).expect("mkdir");
    std::fs::write(
        user.join("providers.d/router.toml"),
        format!(
            "base_url = \"{}/v1\"\n\
             [prompt_cache]\n\
             affinity_header = \"x-session-id\"\n\
             [[models]]\n\
             id = \"m\"\n",
            server.uri()
        ),
    )
    .expect("provider");
    std::fs::write(
        user.join("config.toml"),
        "[model]\ndefault = \"router/m\"\n",
    )
    .expect("cfg");

    let mut vars = HashMap::<String, OsString>::new();
    vars.insert("QUACK_CONFIG_DIR".into(), user.into_os_string());
    for (k, sub) in [
        ("QUACK_SYSTEM_DIR", "system"),
        ("QUACK_DATA_DIR", "data"),
        ("QUACK_CACHE_DIR", "cache"),
    ] {
        vars.insert(k.into(), tmp.path().join(sub).into_os_string());
    }
    vars.insert(
        "QUACK_DEFAULTS_DIR".into(),
        defaults_bundle().into_os_string(),
    );
    let loaded = load_with(
        &move |k| vars.get(k).cloned(),
        &LoadOptions::new(tmp.path().join("ws")),
    )
    .expect("load");

    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(&mut engine, "first").await;
    turn(&mut engine, "second").await;
    engine.submit(Op::Compact { focus: None }).await;
    loop {
        if matches!(
            engine.events.recv().await.expect("event"),
            Event::Compacted { .. }
        ) {
            break;
        }
    }
    engine.shutdown().await;

    let reqs: Vec<_> = server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.url.path() == "/v1/chat/completions")
        .collect();
    assert_eq!(reqs.len(), 3, "two turns plus one text compaction summary");
    let first_id = reqs[0]
        .headers
        .get("x-session-id")
        .and_then(|v| v.to_str().ok())
        .expect("x-session-id header")
        .to_owned();
    assert!(!first_id.is_empty());
    for (i, req) in reqs.iter().enumerate() {
        let got = req
            .headers
            .get("x-session-id")
            .and_then(|v| v.to_str().ok())
            .expect("x-session-id header");
        assert_eq!(got, first_id, "request {i} keeps same x-session-id");
        let body: Value = serde_json::from_slice(&req.body).expect("json");
        assert!(
            body.get("prompt_cache_key").is_none(),
            "request {i} should not send body prompt_cache_key when cache_key is false"
        );
    }
}

#[tokio::test]
async fn shared_prompt_cache_key_env_applies_across_independent_sessions() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(text("ok"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let user = tmp.path().join("user");
    std::fs::create_dir_all(user.join("providers.d")).expect("mkdir");
    std::fs::create_dir_all(tmp.path().join("ws")).expect("mkdir");
    std::fs::write(
        user.join("providers.d/kimi.toml"),
        format!(
            "base_url = \"{}/v1\"\n\
             [prompt_cache]\n\
             cache_key = true\n\
             affinity_header = \"x-session-affinity\"\n\
             [[models]]\n\
             id = \"kimi-k3\"\n",
            server.uri()
        ),
    )
    .expect("provider");
    std::fs::write(
        user.join("config.toml"),
        "[model]\ndefault = \"kimi/kimi-k3\"\n",
    )
    .expect("cfg");

    for run in ["first", "second"] {
        let mut vars = HashMap::<String, OsString>::new();
        vars.insert("QUACK_CONFIG_DIR".into(), user.clone().into_os_string());
        vars.insert("QUACK_PROMPT_CACHE_KEY".into(), "ci-repo-shared-key".into());
        for (k, sub) in [
            ("QUACK_SYSTEM_DIR", "system"),
            ("QUACK_DATA_DIR", "data"),
            ("QUACK_CACHE_DIR", "cache"),
        ] {
            vars.insert(k.into(), tmp.path().join(sub).into_os_string());
        }
        vars.insert(
            "QUACK_DEFAULTS_DIR".into(),
            defaults_bundle().into_os_string(),
        );
        let loaded = load_with(
            &move |k| vars.get(k).cloned(),
            &LoadOptions::new(tmp.path().join("ws")),
        )
        .expect("load");
        let mut engine = Engine::spawn(Arc::new(loaded));
        turn(&mut engine, run).await;
        engine.shutdown().await;
    }

    let reqs: Vec<_> = server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.url.path() == "/v1/chat/completions")
        .collect();
    assert_eq!(reqs.len(), 2);
    for req in &reqs {
        assert_eq!(
            req.headers
                .get("x-session-affinity")
                .and_then(|v| v.to_str().ok()),
            Some("ci-repo-shared-key")
        );
        let body: Value = serde_json::from_slice(&req.body).expect("json");
        assert_eq!(body["prompt_cache_key"], "ci-repo-shared-key");
    }
}
