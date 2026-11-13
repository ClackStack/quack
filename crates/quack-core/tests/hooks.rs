//! Hooks (docs/plan/11-security.md#hooks): `hooks.d` programs on agent events, against a
//! mock model. The hooks are small Python scripts run through `{python}`, as real ones
//! would be; without Python on PATH these tests have nothing to run and pass.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use common::{bodies, sse_call, sse_text, turn};
use quack_core::Engine;
use quack_core::protocol::{Event, HookOutcome, InputItem, Op, TurnOverrides};
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn has_python() -> bool {
    ["python3", "python", "py"].iter().any(|p| {
        std::process::Command::new(p)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

/// Write `<dir>/hooks.d/<id>.toml` running `<dir>/hooks.d/scripts/<id>.py`.
fn hook(dir: &Path, id: &str, toml: &str, script: &str) {
    let hooks = dir.join("hooks.d");
    std::fs::create_dir_all(hooks.join("scripts")).expect("mkdir");
    std::fs::write(hooks.join("scripts").join(format!("{id}.py")), script).expect("script");
    std::fs::write(
        hooks.join(format!("{id}.toml")),
        format!("{toml}\ncommand = [\"{{python}}\", \"{{dir}}/scripts/{id}.py\"]\n"),
    )
    .expect("hook");
}

/// Submit a prompt and collect events until the turn ends either way.
async fn run_turn(engine: &mut quack_core::EngineHandle, text: &str) -> Vec<Event> {
    engine
        .submit(Op::UserTurn {
            items: vec![InputItem::text(text)],
            overrides: TurnOverrides::default(),
        })
        .await;
    let mut seen = Vec::new();
    loop {
        let e = tokio::time::timeout(Duration::from_secs(20), engine.events.recv())
            .await
            .expect("event within 20s")
            .expect("channel open");
        let end = matches!(e, Event::TurnComplete { .. } | Event::TurnAborted { .. });
        seen.push(e);
        if end {
            return seen;
        }
    }
}

fn hook_runs(events: &[Event]) -> Vec<(String, HookOutcome)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::HookRan { id, outcome, .. } => Some((id.clone(), *outcome)),
            _ => None,
        })
        .collect()
}

const ECHO_MATCHES: &str = r#"
import json, sys
event = json.load(sys.stdin)
keys = ", ".join(m.rsplit("/", 1)[-1] for m in event["matches"])
print(json.dumps({"additional_context": "ISSUE CONTEXT for " + keys, "message": "fetched " + keys}))
"#;

#[tokio::test]
async fn user_prompt_hook_adds_context_only_when_its_pattern_matches() {
    if !has_python() {
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_text("ok"))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    hook(
        &tmp.path().join("user"),
        "issues",
        "event = \"user_prompt\"\nmatch = { prompt = 'https://jira\\.corp/browse/[A-Z]+-\\d+' }",
        ECHO_MATCHES,
    );
    let loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    let mut engine = Engine::spawn(Arc::new(loaded));

    let events = run_turn(
        &mut engine,
        "look at https://jira.corp/browse/PAY-12 please",
    )
    .await;
    assert_eq!(
        hook_runs(&events),
        [("issues".into(), HookOutcome::Context)]
    );
    let ran = events.iter().find_map(|e| match e {
        Event::HookRan {
            message,
            context_tokens,
            ..
        } => Some((message.clone(), *context_tokens)),
        _ => None,
    });
    assert!(
        matches!(&ran, Some((Some(m), n)) if m == "fetched PAY-12" && *n > 0),
        "{ran:?}"
    );

    let events = run_turn(&mut engine, "no links here").await;
    assert!(hook_runs(&events).is_empty());

    let sent = bodies(&server).await;
    assert!(
        sent[0].contains(
            "look at https://jira.corp/browse/PAY-12 please\\n\\nISSUE CONTEXT for PAY-12"
        ),
        "{}",
        sent[0]
    );
    // The second request repeats the first turn, context included, and adds nothing.
    assert_eq!(
        sent[1].matches("ISSUE CONTEXT for").count(),
        1,
        "{}",
        sent[1]
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn a_denying_hook_stops_the_prompt_and_a_failing_one_follows_on_error() {
    if !has_python() {
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_text("ok"))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let user = tmp.path().join("user");
    // Exit 2: deny, with stderr as the reason.
    hook(
        &user,
        "no-secrets",
        "event = \"user_prompt\"\nmatch = { prompt = 'AKIA[0-9A-Z]{16}' }",
        "import sys\nprint('that looks like an AWS key', file=sys.stderr)\nsys.exit(2)\n",
    );
    // A crash with the default on_error = "warn": a warning, and the prompt goes ahead.
    hook(
        &user,
        "flaky",
        "event = \"user_prompt\"\nmatch = { prompt = 'flaky' }",
        "raise SystemExit('jira is down')\n",
    );
    let loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    let mut engine = Engine::spawn(Arc::new(loaded));

    let events = run_turn(&mut engine, "deploy with AKIAABCDEFGHIJKLMNOP").await;
    assert!(
        events.iter().any(|e| matches!(e, Event::Warning { message }
            if message.contains("no-secrets") && message.contains("AWS key"))),
        "{events:?}"
    );
    assert!(
        matches!(events.last(), Some(Event::TurnAborted { reason, .. }) if reason.contains("no-secrets")),
        "{events:?}"
    );
    assert!(
        bodies(&server).await.is_empty(),
        "nothing reaches the model"
    );

    let events = run_turn(&mut engine, "flaky thing").await;
    assert!(
        events.iter().any(|e| matches!(e, Event::Warning { message }
            if message.contains("hook `flaky` failed") && message.contains("jira is down"))),
        "{events:?}"
    );
    assert!(matches!(events.last(), Some(Event::TurnComplete { .. })));
    assert_eq!(bodies(&server).await.len(), 1);
    engine.shutdown().await;
}

#[tokio::test]
async fn pre_tool_hooks_allow_deny_and_rewrite_calls() {
    if !has_python() {
        return;
    }
    let server = MockServer::start().await;
    // First match wins: each turn's call once, then plain answers.
    Mock::given(method("POST"))
        .and(body_string_contains("write the notes"))
        .respond_with(sse_call(
            "c1",
            "write",
            serde_json::json!({"path": "notes.txt", "content": "draft"}),
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("write the secrets"))
        .respond_with(sse_call(
            "c2",
            "write",
            serde_json::json!({"path": "secrets.env", "content": "x"}),
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse_text("done"))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let user = tmp.path().join("user");
    // Writing notes needs no approval, and the hook changes what is written.
    hook(
        &user,
        "notes",
        "event = \"pre_tool\"\nmatch = { tool = \"write\", pattern = \"notes*\" }",
        r#"
import json, sys
event = json.load(sys.stdin)
args = dict(event["args"], content=event["args"]["content"] + " (reviewed)")
print(json.dumps({"decision": "allow", "updated_args": args}))
"#,
    );
    hook(
        &user,
        "guard",
        "event = \"pre_tool\"\nmatch = { pattern = \"*.env\" }",
        r#"print('{"decision": "deny", "message": "env files hold credentials"}')"#,
    );
    let loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    let ws = loaded.dirs.workspace_root.clone();
    let mut engine = Engine::spawn(Arc::new(loaded));

    // In the default ask mode a write would ask; the hook allowed it.
    let events = turn(&mut engine, "write the notes").await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::ApprovalRequest { .. })),
        "{events:?}"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("notes.txt")).expect("written"),
        "draft (reviewed)"
    );

    let events = turn(&mut engine, "write the secrets").await;
    assert!(hook_runs(&events).contains(&("guard".into(), HookOutcome::Denied)));
    assert!(!ws.join("secrets.env").exists());
    let sent = bodies(&server).await;
    assert!(
        sent.iter()
            .any(|b| b.contains("blocked by hook `guard`: env files hold credentials")),
        "{sent:?}"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn post_tool_context_reaches_the_model_with_the_result() {
    if !has_python() {
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("\"role\":\"tool\""))
        .respond_with(sse_text("done"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse_call("c1", "read", serde_json::json!({"path": "a.txt"})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    hook(
        &tmp.path().join("user"),
        "lint",
        "event = \"post_tool\"\nmatch = { tool = \"read\" }",
        r#"
import json, sys
event = json.load(sys.stdin)
print("reminder: " + event["target"] + " is generated; edit the template" if event["ok"] else "")
"#,
    );
    let loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    std::fs::write(loaded.dirs.workspace_root.join("a.txt"), "hello").expect("file");
    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(&mut engine, "read it").await;
    let sent = bodies(&server).await;
    assert!(
        sent[1].contains("reminder: a.txt is generated; edit the template"),
        "{}",
        sent[1]
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn system_hooks_cannot_be_turned_off_by_the_user() {
    if !has_python() {
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_text("ok"))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    hook(
        &tmp.path().join("system"),
        "audit",
        "event = \"user_prompt\"",
        "print('POLICY: follow the corporate guidelines')\n",
    );
    let user = tmp.path().join("user");
    std::fs::create_dir_all(user.join("hooks.d")).expect("mkdir");
    std::fs::write(user.join("hooks.d").join("audit.toml"), "enabled = false\n").expect("file");
    std::fs::write(
        user.join("config.toml"),
        "[hooks]\ndisabled = [\"audit\"]\n",
    )
    .expect("file");
    let loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(&mut engine, "hello").await;
    let sent = bodies(&server).await;
    assert!(
        sent[0].contains("POLICY: follow the corporate guidelines"),
        "{}",
        sent[0]
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn session_start_context_rides_on_the_first_prompt_only() {
    if !has_python() {
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_text("ok"))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    hook(
        &tmp.path().join("user"),
        "standup",
        "event = \"session_start\"",
        "import json, sys\nprint('SESSION NOTE (' + json.load(sys.stdin)['source'] + ')')\n",
    );
    let loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(&mut engine, "first").await;
    turn(&mut engine, "second").await;
    let sent = bodies(&server).await;
    assert!(
        sent[0].contains("first\\n\\nSESSION NOTE (startup)"),
        "{}",
        sent[0]
    );
    assert_eq!(sent[1].matches("SESSION NOTE").count(), 1, "{}", sent[1]);
    engine.shutdown().await;
}

/// The shipped Jira example, end to end: the prompt links to an issue, `jira.py` fetches
/// it from a mock Jira with the token, and the model gets it as XML.
#[tokio::test]
async fn shipped_jira_hook_fetches_linked_issues() {
    if !has_python() {
        return;
    }
    let jira = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PAY-1234"))
        .and(header("authorization", "Bearer pat-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "key": "PAY-1234",
            "fields": {
                "summary": "Checkout fails on retry",
                "issuetype": {"name": "Bug"},
                "status": {"name": "In Progress"},
                "assignee": {"displayName": "Dana Lee"},
                "labels": ["payments"],
                "description": "Retrying a declined card returns 500 & logs <nil>.",
                "comment": {"comments": [
                    {"author": {"displayName": "Sam"}, "created": "2026-10-01", "body": "Seen in prod."}
                ]},
                "issuelinks": [{
                    "type": {"outward": "blocks"},
                    "outwardIssue": {"key": "PAY-1300", "fields": {"summary": "Release 4.2", "status": {"name": "Open"}}}
                }]
            }
        })))
        .mount(&jira)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PAY-9"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&jira)
        .await;
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_text("ok"))
        .mount(&model)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let shipped = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("defaults")
        .join("hooks.d");
    let hooks = tmp.path().join("user").join("hooks.d");
    std::fs::create_dir_all(hooks.join("jira")).expect("mkdir");
    std::fs::copy(
        shipped.join("jira").join("jira.py"),
        hooks.join("jira").join("jira.py"),
    )
    .expect("script");
    // Enabled, pointed at the mock, with a token written in (tests never read real env).
    // Normalize line endings first: a Windows checkout has CRLF, and the replaces below
    // (notably removing the `enabled = false` line) match LF text.
    let toml = std::fs::read_to_string(shipped.join("jira.toml"))
        .expect("jira.toml")
        .replace("\r\n", "\n")
        .replace("enabled = false\n", "")
        .replace("https://jira.example.com", &jira.uri())
        .replace("{ env = \"JIRA_TOKEN\" }", "\"pat-123\"");
    let mut config: toml::Value = toml::from_str(&toml).expect("hook TOML");
    config.as_table_mut().expect("table").remove("connections");
    let env = toml::Value::try_from(std::collections::BTreeMap::from([
        ("JIRA_URL", jira.uri()),
        ("JIRA_TOKEN", "pat-123".to_owned()),
    ]))
    .expect("env");
    config
        .as_table_mut()
        .expect("table")
        .insert("env".into(), env);
    std::fs::write(
        hooks.join("jira.toml"),
        toml::to_string(&config).expect("serialize"),
    )
    .expect("hook");

    let loaded = common::workspace(tmp.path(), &model.uri(), r#"["tiny"]"#, "");
    let mut engine = Engine::spawn(Arc::new(loaded));
    let prompt = format!(
        "why is {0}/browse/PAY-1234 failing? see also {0}/browse/PAY-9 and \
         https://other.example/browse/OPS-1",
        jira.uri()
    );
    let events = run_turn(&mut engine, &prompt).await;
    assert!(
        events.iter().any(
            |e| matches!(e, Event::HookRan { id, outcome, message: Some(m), .. }
            if id == "jira" && *outcome == HookOutcome::Context
                && m == "fetched PAY-1234; could not fetch PAY-9")
        ),
        "{events:?}"
    );
    let sent = bodies(&model).await;
    let body = &sent[0];
    for part in [
        "<jira_issue key=\\\"PAY-1234\\\"",
        "<summary>Checkout fails on retry</summary>",
        "<status>In Progress</status>",
        "<assignee>Dana Lee</assignee>",
        "returns 500 &amp; logs &lt;nil&gt;.",
        "<comment author=\\\"Sam\\\"",
        "relation=\\\"blocks\\\" key=\\\"PAY-1300\\\"",
        "<jira_issue key=\\\"PAY-9\\\" unavailable=\\\"not found, or no permission to see it\\\" />",
    ] {
        assert!(body.contains(part), "missing {part} in {body}");
    }
    assert!(
        !body.contains("OPS-1\\\""),
        "links to another Jira are skipped"
    );
    engine.shutdown().await;
}

/// Enabling a hook that ships disabled: `Op::EnableHook` copies it (and its payload
/// folder) from the defaults layer into the user's `hooks.d` with `enabled = true`, and
/// the hook then runs on the next prompt. The shipped files stay as they are.
#[tokio::test]
async fn enabling_a_shipped_hook_copies_it_into_the_user_layer_and_runs_it() {
    if !has_python() {
        return;
    }
    let jira = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/2/issue/PAY-1234"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "key": "PAY-1234",
            "fields": {
                "summary": "Checkout fails on retry",
                "description": "Retrying a declined card returns 500.",
            }
        })))
        .mount(&jira)
        .await;
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_text("ok"))
        .mount(&model)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let shipped = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("defaults")
        .join("hooks.d");
    let defaults = tmp.path().join("defaults").join("hooks.d");
    std::fs::create_dir_all(defaults.join("jira")).expect("mkdir");
    std::fs::copy(
        shipped.join("jira").join("jira.py"),
        defaults.join("jira").join("jira.py"),
    )
    .expect("script");
    // As shipped, but pointed at the mock with the token written in (tests never read
    // real env). Line endings normalized: a Windows checkout has CRLF.
    let toml = std::fs::read_to_string(shipped.join("jira.toml"))
        .expect("jira.toml")
        .replace("\r\n", "\n")
        .replace("https://jira.example.com", &jira.uri())
        .replace("{ env = \"JIRA_TOKEN\" }", "\"pat-123\"");
    let mut config: toml::Value = toml::from_str(&toml).expect("hook TOML");
    config.as_table_mut().expect("table").remove("connections");
    let env = toml::Value::try_from(std::collections::BTreeMap::from([
        ("JIRA_URL", jira.uri()),
        ("JIRA_TOKEN", "pat-123".to_owned()),
    ]))
    .expect("env");
    config
        .as_table_mut()
        .expect("table")
        .insert("env".into(), env);
    std::fs::write(
        defaults.join("jira.toml"),
        format!(
            "{}\n{}",
            toml.lines()
                .filter(|line| line.starts_with('#'))
                .collect::<Vec<_>>()
                .join("\n"),
            toml::to_string(&config).expect("serialize")
        ),
    )
    .expect("hook");

    let loaded = common::workspace_in(
        tmp.path(),
        &model.uri(),
        r#"["tiny"]"#,
        "",
        Some(&tmp.path().join("defaults")),
    );
    let mut engine = Engine::spawn(Arc::new(loaded));
    engine
        .submit(Op::EnableHook {
            id: "jira".into(),
            replace_payload: None,
        })
        .await;
    let mut enabled = None;
    loop {
        let e = tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
            .await
            .expect("event within 10s")
            .expect("channel open");
        match e {
            Event::Error { error } => panic!("engine error: {}", error.message),
            Event::HookEnabled { path, .. } => enabled = Some(path),
            Event::ConfigReloaded => break,
            _ => {}
        }
    }
    let enabled = enabled.expect("HookEnabled before the reload");

    let copy = tmp.path().join("user").join("hooks.d").join("jira.toml");
    assert_eq!(enabled, copy.display().to_string());
    assert_eq!(PathBuf::from(&enabled), copy);
    let text = std::fs::read_to_string(&copy).expect("copy");
    assert!(text.contains("enabled = true"), "{text}");
    assert!(
        !text
            .lines()
            .any(|l| l.trim_start().starts_with("enabled = false")),
        "{text}"
    );
    assert!(
        text.contains("Configure connections"),
        "the shipped comments survive the copy: {text}"
    );
    assert_eq!(
        std::fs::read_to_string(
            tmp.path()
                .join("user")
                .join("hooks.d")
                .join("jira")
                .join("jira.py")
        )
        .expect("payload"),
        std::fs::read_to_string(shipped.join("jira").join("jira.py")).expect("shipped payload")
    );
    let shipped_text = std::fs::read_to_string(defaults.join("jira.toml")).expect("shipped");
    assert!(
        shipped_text.contains("enabled = false"),
        "the shipped file is untouched"
    );

    let prompt = format!("why is {0}/browse/PAY-1234 failing?", jira.uri());
    let events = run_turn(&mut engine, &prompt).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::HookRan { id, outcome, .. }
            if id == "jira" && *outcome == HookOutcome::Context)),
        "{events:?}"
    );
    let sent = bodies(&model).await;
    assert!(
        sent[0].contains("<jira_issue key=\\\"PAY-1234\\\""),
        "{}",
        sent[0]
    );
    engine.shutdown().await;
}

/// A cloned repository's `.quack/hooks.d` runs nothing until the project is trusted.
#[tokio::test]
async fn an_untrusted_projects_hooks_do_not_run() {
    if !has_python() {
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_text("ok"))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let marker = tmp.path().join("ran");
    hook(
        &tmp.path().join("ws").join(".quack"),
        "evil",
        "event = \"user_prompt\"",
        &format!(
            "open({:?}, 'w').write('x')\nprint('INJECTED')\n",
            marker.display().to_string()
        ),
    );
    let loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(&mut engine, "hello").await;
    assert!(!marker.exists(), "the project's hook ran");
    assert!(!bodies(&server).await[0].contains("INJECTED"));
    engine.shutdown().await;
}

/// Exercise each forge's shipped URL matcher, REST routes and context through the engine.
#[tokio::test]
async fn shipped_forge_review_hook_uses_forge_rest_endpoints() {
    if !has_python() {
        return;
    }
    let model = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_text("ok"))
        .mount(&model)
        .await;
    let shipped = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("defaults")
        .join("hooks.d");
    for (kind, link_path, api_path, diff_suffix, accept) in [
        (
            "github",
            "/org/repo/pull/12/files",
            "/repos/org/repo/pulls/12",
            "",
            "application/vnd.github.diff",
        ),
        (
            "gitlab",
            "/group/sub/repo/-/merge_requests/12/diffs",
            "/projects/group%2Fsub%2Frepo/merge_requests/12",
            "/raw_diffs",
            "text/plain",
        ),
        (
            "gitea",
            "/org/repo/pulls/12",
            "/repos/org/repo/pulls/12",
            ".diff",
            "text/plain",
        ),
        (
            "bitbucket",
            "/org/repo/pull-requests/12",
            "/repositories/org/repo/pullrequests/12",
            "/diff",
            "text/plain",
        ),
        (
            "bitbucket-server",
            "/projects/ORG/repos/repo/pull-requests/12/overview",
            "/projects/ORG/repos/repo/pull-requests/12",
            ".diff",
            "text/plain",
        ),
    ] {
        let forge = MockServer::start().await;
        let endpoint = format!("/prefix/api{api_path}");
        Mock::given(method("GET"))
            .and(path(&endpoint))
            .and(header("accept", "application/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "title": format!("Review on {kind}"),
                "body": "Fix <retry> & ordering",
                "description": "Fix <retry> & ordering",
                "head": {"ref": "fix", "sha": "abcdef"},
                "source_branch": "fix",
                "diff_refs": {"head_sha": "abcdef"},
                "source": {"branch": {"name": "fix"}, "commit": {"hash": "abcdef"}},
                "fromRef": {"id": "fix", "latestCommit": "abcdef"}
            })))
            .expect(1)
            .mount(&forge)
            .await;
        let diff_path = format!("{endpoint}{diff_suffix}");
        let diff_path = if kind == "bitbucket" {
            let redirected = "/prefix/api/repositories/org/repo/diff/head..base";
            Mock::given(method("GET"))
                .and(path(&diff_path))
                .respond_with(ResponseTemplate::new(302).insert_header("location", redirected))
                .expect(1)
                .mount(&forge)
                .await;
            redirected.to_owned()
        } else {
            diff_path
        };
        let mut diff = "diff --git a/f b/f\n-old\n+<fixed> & tested\n".to_owned();
        if kind == "github" {
            diff.push_str(&"+changed line\n".repeat(90_000));
        }
        diff.push_str("END_OF_FULL_DIFF\n");
        Mock::given(method("GET"))
            .and(path(diff_path))
            .and(header("accept", accept))
            .respond_with(ResponseTemplate::new(200).set_body_string(diff))
            .expect(1)
            .mount(&forge)
            .await;

        let tmp = tempfile::tempdir().expect("tempdir");
        let hooks = tmp.path().join("user").join("hooks.d");
        std::fs::create_dir_all(hooks.join("forge-review")).expect("mkdir");
        std::fs::copy(
            shipped.join("forge-review").join("review.py"),
            hooks.join("forge-review").join("review.py"),
        )
        .expect("copy script");
        let config =
            std::fs::read_to_string(shipped.join("forge-review.toml")).expect("shipped hook");
        let mut config: toml::Value = toml::from_str(&config).expect("valid hook TOML");
        config["enabled"] = toml::Value::Boolean(true);
        config.as_table_mut().expect("table").remove("connections");
        config["command"] = toml::Value::try_from(vec![
            "{python}".to_owned(),
            "{dir}/forge-review/review.py".to_owned(),
            "--host".to_owned(),
            kind.to_owned(),
            format!("{}/prefix", forge.uri()),
            format!("{}/prefix/api", forge.uri()),
            "-".to_owned(),
        ])
        .expect("command");
        std::fs::write(
            hooks.join("forge-review.toml"),
            toml::to_string(&config).expect("serialize"),
        )
        .expect("write hook");
        let loaded = common::workspace(
            tmp.path(),
            &model.uri(),
            r#"[{ id = "tiny", context_window = 2000000 }]"#,
            "",
        );
        let mut engine = Engine::spawn(Arc::new(loaded));
        let link = format!("{}/prefix{link_path}", forge.uri());
        let events = run_turn(
            &mut engine,
            &format!("Review ({link}?secret=hide#fragment), and {link}"),
        )
        .await;
        assert!(
            events.iter().any(|event| matches!(event,
                Event::HookRan { id, outcome, .. }
                if id == "forge-review" && *outcome == HookOutcome::Context
            )),
            "{kind}: {events:?}"
        );
        let sent = bodies(&model).await;
        let body = sent.last().expect("model request");
        assert!(body.contains(&format!("Review on {kind}")), "{body}");
        assert!(body.contains("Fix &lt;retry&gt; &amp; ordering"), "{body}");
        assert!(body.contains("+&lt;fixed&gt; &amp; tested"), "{body}");
        assert!(body.contains("abcdef"), "{body}");
        assert!(
            body.contains("END_OF_FULL_DIFF"),
            "diff tail missing for {kind}"
        );
        if kind == "github" {
            assert!(body.len() > 1024 * 1024, "full stdout must survive capture");
            assert!(events.iter().any(|event| matches!(event,
                Event::Warning { message } if message.contains("full context was retained")
            )));
            assert!(events.iter().any(|event| matches!(event,
                Event::HookRan { message: Some(message), .. }
                if message.contains("Warning: diff") && message.contains("full diff was retained")
            )));
        }
        forge.verify().await;
        engine.shutdown().await;
    }
}
