//! Plan mode: nothing changes while planning; the request asks for a plan.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{bodies, sse_call, sse_text, turn};
use quack_core::protocol::{
    ApprovalDecision, ApprovalPolicy, Event, InputItem, Op, PermissionMode, TurnOverrides, WorkMode,
};
use quack_core::{Engine, EngineHandle, Resume};
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer};

#[tokio::test]
async fn plan_mode_refuses_edits_and_asks_for_a_plan() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("plan mode: no changes"))
        .respond_with(sse_text("1. Write notes.txt"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse_call(
            "c1",
            "write",
            serde_json::json!({"path": "notes.txt", "content": "hi"}),
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    let ws = loaded.dirs.workspace_root.clone();
    let mut engine = Engine::spawn(Arc::new(loaded));
    engine
        .submit(Op::SetPermissionMode {
            mode: PermissionMode::Plan,
        })
        .await;
    let mode = loop {
        let e = tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
            .await
            .expect("event")
            .expect("open");
        if let Event::PermissionMode { mode } = e
            && mode == PermissionMode::Plan
        {
            break mode;
        }
    };
    assert_eq!(mode, PermissionMode::Plan);

    turn(&mut engine, "add notes").await;
    assert!(!ws.join("notes.txt").exists());
    let first = bodies(&server).await.remove(0);
    assert!(first.contains("add notes\\n\\n[Plan mode]"), "{first}");
    engine.shutdown().await;
}

async fn next_mode(engine: &mut EngineHandle) -> (WorkMode, ApprovalPolicy) {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
            .await
            .expect("mode event")
            .expect("open");
        if let Event::ModeChanged {
            work_mode,
            approvals,
        } = event
        {
            return (work_mode, approvals);
        }
    }
}

#[tokio::test]
async fn new_sessions_use_plan_and_allow_until_defaults_are_changed() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = common::workspace_in(tmp.path(), &server.uri(), r#"["tiny"]"#, "", None);
    let mut engine = Engine::spawn(Arc::new(loaded));
    let defaults = (WorkMode::Plan, ApprovalPolicy::Allow);
    assert_eq!(next_mode(&mut engine).await, defaults);

    engine
        .submit(Op::SetWorkMode {
            mode: WorkMode::Edit,
        })
        .await;
    assert_eq!(
        next_mode(&mut engine).await,
        (WorkMode::Edit, ApprovalPolicy::Allow)
    );
    engine
        .submit(Op::SetApprovalPolicy {
            policy: ApprovalPolicy::Ask,
        })
        .await;
    assert_eq!(
        next_mode(&mut engine).await,
        (WorkMode::Edit, ApprovalPolicy::Ask)
    );

    engine.submit(Op::NewSession).await;
    assert_eq!(next_mode(&mut engine).await, defaults);

    for (key, value) in [
        ("permissions.work_mode", "\"edit\""),
        ("permissions.mode", "\"ask\""),
    ] {
        engine
            .submit(Op::SaveSetting {
                key: key.into(),
                value: value.into(),
                scope: quack_core::protocol::WriteScope::User,
            })
            .await;
    }
    engine.submit(Op::NewSession).await;
    assert_eq!(
        next_mode(&mut engine).await,
        (WorkMode::Edit, ApprovalPolicy::Ask)
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn all_four_combinations_enforce_work_mode_before_approvals() {
    for work_mode in [WorkMode::Plan, WorkMode::Edit] {
        for approvals in [ApprovalPolicy::Ask, ApprovalPolicy::Allow] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(body_string_contains("\"role\":\"tool\""))
                .respond_with(sse_text("Proposed tests: cargo test verifies the changes."))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .respond_with(sse_call(
                    "write-1",
                    "write",
                    serde_json::json!({"path":"notes.txt", "content":"edited"}),
                ))
                .up_to_n_times(1)
                .mount(&server)
                .await;
            let tmp = tempfile::tempdir().expect("tempdir");
            let mut loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
            loaded.settings.permissions.work_mode = work_mode;
            loaded.settings.permissions.mode = approvals;
            // Neither a matching rule nor auto approval can bypass Plan.
            if work_mode == WorkMode::Plan {
                loaded.settings.permissions.allow.push("edit:**".into());
            }
            let ws = loaded.dirs.workspace_root.clone();
            let mut engine = Engine::spawn(Arc::new(loaded));
            assert_eq!(next_mode(&mut engine).await, (work_mode, approvals));
            engine
                .submit(Op::UserTurn {
                    items: vec![InputItem::text("investigate notes")],
                    overrides: TurnOverrides::default(),
                })
                .await;
            let mut asked = 0;
            loop {
                let event = tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
                    .await
                    .expect("turn event")
                    .expect("open");
                match event {
                    Event::ApprovalRequest { call_id, .. } => {
                        asked += 1;
                        engine
                            .submit(Op::ApprovalResponse {
                                call_id,
                                decision: ApprovalDecision::AllowOnce,
                            })
                            .await;
                    }
                    Event::TurnComplete { .. } => break,
                    Event::Error { error } => panic!("{error:?}"),
                    _ => {}
                }
            }
            assert_eq!(
                ws.join("notes.txt").exists(),
                work_mode == WorkMode::Edit,
                "{work_mode:?}/{approvals:?}"
            );
            assert_eq!(
                asked,
                usize::from(work_mode == WorkMode::Edit && approvals == ApprovalPolicy::Ask)
            );
            if work_mode == WorkMode::Plan {
                let requests = bodies(&server).await;
                assert!(
                    requests[0].contains("exact test commands"),
                    "{}",
                    requests[0]
                );
                assert!(
                    requests[1].contains("plan mode: no changes"),
                    "{}",
                    requests[1]
                );
            }
            engine.shutdown().await;
        }
    }
}

#[tokio::test]
async fn planning_does_not_run_allow_listed_test_commands() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("\"role\":\"tool\""))
        .respond_with(sse_text(
            "Proposed tests: cargo test --workspace (check regressions).",
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse_call(
            "test-1",
            "shell",
            serde_json::json!({"command":"cargo test --workspace"}),
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    loaded.settings.permissions.work_mode = WorkMode::Plan;
    loaded.settings.permissions.mode = ApprovalPolicy::Allow;
    loaded
        .settings
        .permissions
        .allow
        .push("shell:cargo *".into());
    let mut engine = Engine::spawn(Arc::new(loaded));
    // until_done rejects any approval: this turn should just refuse the command.
    turn(&mut engine, "plan the fix and tests").await;
    let requests = bodies(&server).await;
    assert!(
        requests[1].contains("no changes or test/build commands"),
        "{}",
        requests[1]
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn approval_changes_acceptance_reload_and_resume_preserve_both_controls() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_text("Proposed tests: cargo test checks behavior."))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    loaded.settings.permissions.work_mode = WorkMode::Plan;
    let config = Arc::new(loaded);
    let mut engine = Engine::spawn(config.clone());
    assert_eq!(
        next_mode(&mut engine).await,
        (WorkMode::Plan, ApprovalPolicy::Ask)
    );
    engine
        .submit(Op::SetApprovalPolicy {
            policy: ApprovalPolicy::Allow,
        })
        .await;
    assert_eq!(
        next_mode(&mut engine).await,
        (WorkMode::Plan, ApprovalPolicy::Allow)
    );
    turn(&mut engine, "plan a change").await;
    engine.submit(Op::ReloadConfig).await;
    assert_eq!(
        next_mode(&mut engine).await,
        (WorkMode::Plan, ApprovalPolicy::Allow)
    );
    engine.shutdown().await;

    let mut engine = Engine::spawn_with(config, Resume::Latest);
    // Restoring approvals may first report Edit, before restoring Plan.
    loop {
        if next_mode(&mut engine).await == (WorkMode::Plan, ApprovalPolicy::Allow) {
            break;
        }
    }
    engine
        .submit(Op::AcceptPlan {
            mode: String::new(),
            clear_context: false,
        })
        .await;
    loop {
        let (work_mode, approvals) = next_mode(&mut engine).await;
        assert_eq!(approvals, ApprovalPolicy::Allow);
        if work_mode == WorkMode::Edit {
            break;
        }
    }
    common::until_done(&mut engine).await;
    engine.shutdown().await;

    // Explicit command-line choices override saved selections on resume.
    let mut configured = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    configured
        .origins
        .insert("permissions.mode".into(), quack_config::Origin::Cli);
    configured
        .origins
        .insert("permissions.work_mode".into(), quack_config::Origin::Cli);
    let mut engine = Engine::spawn_with(Arc::new(configured), Resume::Latest);
    assert_eq!(
        next_mode(&mut engine).await,
        (WorkMode::Edit, ApprovalPolicy::Ask)
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn entering_plan_while_an_edit_waits_for_approval_prevents_the_edit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("\"role\":\"tool\""))
        .respond_with(sse_text("Proposed tests: cargo test."))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse_call(
            "pending-edit",
            "write",
            serde_json::json!({"path":"notes.txt", "content":"changed"}),
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    let ws = loaded.dirs.workspace_root.clone();
    let mut engine = Engine::spawn(Arc::new(loaded));
    engine
        .submit(Op::UserTurn {
            items: vec![InputItem::text("write notes")],
            overrides: TurnOverrides::default(),
        })
        .await;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
            .await
            .expect("approval")
            .expect("open");
        if let Event::ApprovalRequest { call_id, .. } = event {
            engine
                .submit(Op::SetWorkMode {
                    mode: WorkMode::Plan,
                })
                .await;
            engine
                .submit(Op::ApprovalResponse {
                    call_id,
                    decision: ApprovalDecision::AllowOnce,
                })
                .await;
            break;
        }
    }
    common::until_done(&mut engine).await;
    assert!(!ws.join("notes.txt").exists());
    assert!(bodies(&server).await[1].contains("plan mode: no changes"));
    engine.shutdown().await;
}

#[tokio::test]
async fn a_locked_plan_cannot_be_accepted_or_switched_to_edit() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("system")).expect("system dir");
    std::fs::write(
        tmp.path().join("system").join("config.toml"),
        "[permissions]\nwork_mode = \"plan\"\n[policy]\nlocked = [\"permissions.work_mode\"]\n",
    )
    .expect("policy");
    let loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    let mut engine = Engine::spawn(Arc::new(loaded));
    assert_eq!(
        next_mode(&mut engine).await,
        (WorkMode::Plan, ApprovalPolicy::Ask)
    );
    engine
        .submit(Op::SetWorkMode {
            mode: WorkMode::Edit,
        })
        .await;
    assert_eq!(
        next_mode(&mut engine).await,
        (WorkMode::Plan, ApprovalPolicy::Ask)
    );
    engine
        .submit(Op::AcceptPlan {
            mode: "allow".into(),
            clear_context: false,
        })
        .await;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
            .await
            .expect("lock warning")
            .expect("open");
        if let Event::Warning { message } = event
            && message.contains("cannot be implemented")
        {
            break;
        }
    }
    assert!(
        server
            .received_requests()
            .await
            .expect("requests")
            .iter()
            .all(|r| r.method != "POST")
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn plan_mode_prompt_guides_followup_and_finished_plans() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_text(
            "Everything is already complete; no changes needed.",
        ))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    loaded.settings.permissions.work_mode = WorkMode::Plan;
    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(&mut engine, "check whether anything else is needed").await;
    let first = bodies(&server).await.remove(0);
    assert!(
        first.contains("replace the checklist with the new pending follow-up steps")
            && first.contains("do not leave pending steps in the checklist"),
        "{first}"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn plan_mode_shell_denial_lists_only_profiles_read_only_tools() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("permission denied:"))
        .respond_with(sse_text("Proposed tests: cargo test."))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse_call(
            "run-test",
            "shell",
            serde_json::json!({"command": ["bash", "-lc", "cargo test"]}),
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut loaded = common::workspace(
        tmp.path(),
        &server.uri(),
        r#"[{ id = "tiny", profile = "codex" }]"#,
        "",
    );
    loaded.dirs.defaults =
        Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../defaults"));
    loaded.settings.permissions.work_mode = WorkMode::Plan;
    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(&mut engine, "run tests").await;
    let second = &bodies(&server).await[1];
    assert!(second.contains("permission denied: plan mode:"), "{second}");
    let expected = if quack_tools::outline::supported(std::path::Path::new("a.rs")) {
        "the dedicated read-only tools (read, grep, glob, index)"
    } else {
        "the dedicated read-only tools (read, grep, glob)"
    };
    assert!(second.contains(expected), "{second}");
    assert!(!second.contains("list_dir"), "{second}");
    engine.shutdown().await;
}
