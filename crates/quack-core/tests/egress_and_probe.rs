//! End-to-end integration tests for Tracks 1 & 3:
//! - Live `EgressLedger` recording & replay across `/resume`
//! - Pre-egress Secret Shield (`ask` + `RedactAndSend` + `Deny`) on prompts and tool results
//! - Selective context pruning (`Op::PruneContext`) and on-demand tool-calling probe (`Op::ProbeToolCalling`)

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{bodies, sse_call, sse_text, turn};
use quack_core::protocol::{
    ApprovalDecision, ApprovalRequest, ContextZone, EgressKind, Event, InputItem, Op, TurnOverrides,
};
use quack_core::{Engine, Resume};
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn next_event(engine: &mut quack_core::EngineHandle) -> Event {
    tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
        .await
        .expect("event within 10s")
        .expect("channel open")
}

async fn wait_for_startup(engine: &mut quack_core::EngineHandle) {
    loop {
        if matches!(next_event(engine).await, Event::StartupComplete) {
            return;
        }
    }
}

#[tokio::test]
async fn live_egress_ledger_records_and_replays_across_resume() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_text("hello back"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    let mut engine = Engine::spawn(Arc::new(loaded));
    let events = turn(&mut engine, "ping").await;

    let live_recorded = events.iter().any(|e| {
        matches!(
            e,
            Event::EgressRecorded { entry }
                if entry.kind == EgressKind::Inference && entry.status == Some(200) && entry.local
        )
    });
    assert!(
        live_recorded,
        "expected live EgressRecorded inference event"
    );

    engine.submit(Op::ListEgress).await;
    let (live_entries, live_stats) = loop {
        if let Event::EgressLedger { entries, stats } = next_event(&mut engine).await {
            break (entries, stats);
        }
    };
    assert!(!live_entries.is_empty());
    assert_eq!(live_stats.total_requests, live_entries.len() as u64);
    assert!(live_stats.local_requests >= 1);
    engine.shutdown().await;

    // Re-open the same project and resume the latest session: ledger history + stats persist.
    let reloaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    let mut resumed = Engine::spawn_with(Arc::new(reloaded), Resume::Latest);
    wait_for_startup(&mut resumed).await;
    resumed.submit(Op::ListEgress).await;
    let (resumed_entries, resumed_stats) = loop {
        if let Event::EgressLedger { entries, stats } = next_event(&mut resumed).await {
            break (entries, stats);
        }
    };
    assert_eq!(resumed_entries.len(), live_entries.len());
    assert_eq!(resumed_stats, live_stats);
    resumed.shutdown().await;
}

#[tokio::test]
async fn secret_shield_redacts_prompt_and_blocks_denied_tool_output() {
    let server = MockServer::start().await;
    let secret_token = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

    Mock::given(method("POST"))
        .and(body_string_contains("Read secret.txt"))
        .and(body_string_contains("\"role\":\"tool\""))
        .respond_with(sse_text("Got it, handled safely."))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("Read secret.txt"))
        .respond_with(sse_call(
            "call_read_secret",
            "read",
            serde_json::json!({ "path": "secret.txt" }),
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse_text("Redacted prompt accepted."))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("user")).expect("mkdir");
    std::fs::write(
        tmp.path().join("user").join("config.toml"),
        "[network]\nsecret_guard = \"ask\"\nsecret_guard_local = true\n",
    )
    .expect("config");

    let loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    std::fs::write(
        loaded.dirs.workspace_root.join("secret.txt"),
        format!("TOKEN={secret_token}\n"),
    )
    .expect("write secret file");

    let mut engine = Engine::spawn(Arc::new(loaded));
    wait_for_startup(&mut engine).await;

    // 1. Prompt containing a secret -> Ask -> RedactAndSend -> plaintext never reaches wire.
    engine
        .submit(Op::UserTurn {
            items: vec![InputItem::text(format!("Here is my token: {secret_token}"))],
            overrides: TurnOverrides::default(),
        })
        .await;
    loop {
        match next_event(&mut engine).await {
            Event::ApprovalRequest {
                call_id,
                request: ApprovalRequest::EgressSecrets { findings, .. },
                ..
            } => {
                assert!(!findings.is_empty());
                assert!(
                    !findings[0].preview.contains("0123456789"),
                    "finding preview leaked full secret"
                );
                engine
                    .submit(Op::ApprovalResponse {
                        call_id,
                        decision: ApprovalDecision::RedactAndSend,
                    })
                    .await;
            }
            Event::TurnComplete { .. } => break,
            Event::Error { error } => panic!("unexpected error: {}", error.message),
            Event::TurnAborted { reason, .. } => panic!("unexpected abort: {reason}"),
            _ => {}
        }
    }

    // 2. Tool output containing a secret -> Ask -> Deny -> raw secret withheld and blocked entry logged.
    engine
        .submit(Op::UserTurn {
            items: vec![InputItem::text("Read secret.txt")],
            overrides: TurnOverrides::default(),
        })
        .await;
    loop {
        match next_event(&mut engine).await {
            Event::ApprovalRequest {
                call_id,
                request: ApprovalRequest::EgressSecrets { .. },
                ..
            } => {
                engine
                    .submit(Op::ApprovalResponse {
                        call_id,
                        decision: ApprovalDecision::Deny,
                    })
                    .await;
            }
            Event::TurnComplete { .. } => break,
            Event::Error { error } => panic!("unexpected error: {}", error.message),
            Event::TurnAborted { reason, .. } => panic!("unexpected abort: {reason}"),
            _ => {}
        }
    }

    let sent_bodies = bodies(&server).await;
    assert!(!sent_bodies.is_empty());
    for body in &sent_bodies {
        assert!(
            !body.contains(secret_token),
            "raw secret leaked to provider HTTP request: {body}"
        );
    }

    engine.submit(Op::ListEgress).await;
    let (entries, stats) = loop {
        if let Event::EgressLedger { entries, stats } = next_event(&mut engine).await {
            break (entries, stats);
        }
    };
    assert!(
        stats.redacted_count >= 1,
        "expected redaction counter >= 1, got {stats:?}"
    );
    assert!(
        entries.iter().any(|e| e.kind == EgressKind::BlockedEgress),
        "expected BlockedEgress entry on Denied tool output: {entries:?}"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn manual_prune_and_tool_calling_probe() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("Read notes.txt"))
        .and(body_string_contains("\"role\":\"tool\""))
        .respond_with(sse_text("Done reading."))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("Read notes.txt"))
        .respond_with(sse_call(
            "c_notes",
            "read",
            serde_json::json!({ "path": "notes.txt" }),
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("echo_probe"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(concat!(
                    "data: {\"id\":\"p1\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"p1\",\"type\":\"function\",\"function\":{\"name\":\"echo_probe\",\"arguments\":\"{\\\"value\\\":\\\"quack-ok\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
                    "data: {\"id\":\"p1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
                    "data: [DONE]\n\n"
                )),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    let long_notes = "important detail line\n".repeat(120);
    std::fs::write(loaded.dirs.workspace_root.join("notes.txt"), &long_notes).expect("write notes");

    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(&mut engine, "Read notes.txt").await;

    // Inspect context zones + prunable tail items.
    engine.submit(Op::GetContext).await;
    let report = loop {
        if let Event::ContextReport { report } = next_event(&mut engine).await {
            break report;
        }
    };
    assert!(
        report
            .segments
            .iter()
            .any(|s| s.zone == Some(ContextZone::CachedPrefix))
    );
    assert!(
        report
            .segments
            .iter()
            .any(|s| s.zone == Some(ContextZone::UncachedTail))
    );
    let prunable: Vec<usize> = report
        .tail_items
        .iter()
        .filter(|i| i.prunable && !i.pruned)
        .map(|i| i.index)
        .collect();
    assert!(
        !prunable.is_empty(),
        "expected prunable tool result in tail_items"
    );

    // Manually prune selected tail indices.
    engine
        .submit(Op::PruneContext {
            indices: Some(prunable),
        })
        .await;
    let (pruned_report, refreshed_report) = {
        let mut pr = None;
        let mut cr = None;
        while pr.is_none() || cr.is_none() {
            match next_event(&mut engine).await {
                Event::ContextPruned { report } => pr = Some(report),
                Event::ContextReport { report } => cr = Some(report),
                _ => {}
            }
        }
        (pr.expect("pruned"), cr.expect("refreshed"))
    };
    assert!(pruned_report.removed_tokens > 0);
    assert!(
        refreshed_report.used < report.used,
        "expected used tokens {} < {}",
        refreshed_report.used,
        report.used
    );

    // Run on-demand synthetic tool-calling capability probe.
    engine.submit(Op::ProbeToolCalling).await;
    let probe = loop {
        if let Event::ToolProbeResult { report } = next_event(&mut engine).await {
            break report;
        }
    };
    assert_eq!(probe.recommended_mode, "native");
    assert!(probe.cases.iter().any(|c| c.passed));

    engine.shutdown().await;
}
