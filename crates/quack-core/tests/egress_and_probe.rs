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

/// Tiny HTTP forward proxy that forwards every request to `target` (`127.0.0.1:port`),
/// allowing tests to use a non-loopback `base_url` (e.g. `http://cloud.test/v1`) so
/// `is_loopback` is genuinely false without touching any external host.
async fn spawn_forward_proxy(target: std::net::SocketAddr) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        while let Ok((mut client, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let header_end = loop {
                    let n = match client.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => n,
                    };
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break pos + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
                let content_len: usize = head
                    .lines()
                    .find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.eq_ignore_ascii_case("content-length")
                            .then(|| v.trim().parse().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                while buf.len() < header_end + content_len {
                    let n = match client.read(&mut tmp).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    buf.extend_from_slice(&tmp[..n]);
                }
                // Rewrite `POST http://cloud.test/v1/... HTTP/1.1` -> `POST /v1/... HTTP/1.1`
                let Some((req_line, rest)) = head.split_once("\r\n") else {
                    return;
                };
                let mut parts = req_line.splitn(3, ' ');
                let Some(method) = parts.next() else { return };
                let Some(uri) = parts.next() else { return };
                let Some(ver) = parts.next() else { return };
                let path = if let Some(after_scheme) = uri.split_once("://").map(|(_, r)| r) {
                    after_scheme
                        .find('/')
                        .map_or("/", |idx| &after_scheme[idx..])
                } else {
                    uri
                };
                let Ok(mut upstream) = tokio::net::TcpStream::connect(target).await else {
                    return;
                };
                let mut forwarded_headers = String::new();
                for line in rest.split("\r\n") {
                    if line.is_empty() {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if lower.starts_with("connection:") || lower.starts_with("proxy-connection:") {
                        continue;
                    }
                    forwarded_headers.push_str(line);
                    forwarded_headers.push_str("\r\n");
                }
                let rewritten = format!(
                    "{method} {path} {ver}\r\n{forwarded_headers}Connection: close\r\n\r\n"
                );
                if upstream.write_all(rewritten.as_bytes()).await.is_err() {
                    return;
                }
                if upstream.write_all(&buf[header_end..]).await.is_err() {
                    return;
                }
                let mut resp = Vec::new();
                let _ = upstream.read_to_end(&mut resp).await;
                let _ = client.write_all(&resp).await;
            });
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn secret_shield_redacts_agents_md_and_local_history_on_remote_switch() {
    let local_server = MockServer::start().await;
    let cloud_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_text("local ok"))
        .mount(&local_server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse_text("cloud ok"))
        .mount(&cloud_server)
        .await;

    let proxy_url = spawn_forward_proxy(*cloud_server.address()).await;
    let agents_secret = "ghp_AGENTSMDSECRET0000000000000000000000";
    let local_turn_secret = "ghp_LOCALTURNSECRET000000000000000000000";

    let tmp = tempfile::tempdir().expect("tempdir");
    let user = tmp.path().join("user");
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(user.join("providers.d")).expect("mkdir providers");
    std::fs::create_dir_all(&ws).expect("mkdir ws");
    std::fs::write(
        ws.join("AGENTS.md"),
        format!("Deploy token for notes: {agents_secret}\n"),
    )
    .expect("write AGENTS.md");
    std::fs::write(
        user.join("config.toml"),
        "[model]\ndefault = \"local/tiny\"\n[network]\nsecret_guard = \"redact\"\nsecret_guard_local = false\n",
    )
    .expect("write config");
    std::fs::write(
        user.join("providers.d").join("local.toml"),
        format!(
            "base_url = \"{}/v1\"\nmodels = [\"tiny\"]\n",
            local_server.uri()
        ),
    )
    .expect("write local provider");
    std::fs::write(
        user.join("providers.d").join("cloud.toml"),
        format!(
            "base_url = \"http://cloud.example.test/v1\"\nmodels = [\"big\"]\n[http]\nproxy = \"{proxy_url}\"\n"
        ),
    )
    .expect("write cloud provider");

    let loaded = common::workspace(tmp.path(), &local_server.uri(), r#"["tiny"]"#, "");
    let mut engine = Engine::spawn(Arc::new(loaded));
    wait_for_startup(&mut engine).await;

    // Turn 1 on local loopback: secret_guard_local = false skips redaction.
    turn(&mut engine, &format!("Local note with {local_turn_secret}")).await;
    let local_reqs = bodies(&local_server).await;
    let local_dump = serde_json::to_string(&local_reqs).expect("json");
    assert!(local_dump.contains(local_turn_secret));
    assert!(local_dump.contains(agents_secret));

    // Switch mid-session from local loopback to remote cloud host.
    engine
        .submit(Op::SetModel {
            provider: quack_core::protocol::ProviderId("cloud".into()),
            model: quack_core::protocol::ModelId("big".into()),
        })
        .await;
    loop {
        if let Event::SessionConfigured { session } = next_event(&mut engine).await
            && session.model.as_deref() == Some("cloud/big")
        {
            break;
        }
    }

    // Turn 2 on cloud: prior local turn history AND AGENTS.md session_context must both be redacted!
    turn(&mut engine, "Now summarize safely").await;
    let cloud_reqs = bodies(&cloud_server).await;
    assert_eq!(cloud_reqs.len(), 1);
    let cloud_dump = serde_json::to_string(&cloud_reqs[0]).expect("json");
    assert!(
        !cloud_dump.contains(local_turn_secret),
        "prior local history secret leaked to cloud: {cloud_dump}"
    );
    assert!(
        !cloud_dump.contains(agents_secret),
        "AGENTS.md secret leaked to cloud: {cloud_dump}"
    );

    engine.shutdown().await;
}

#[tokio::test]
async fn input_item_file_and_plain_at_mentions_attach_workspace_context() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_text("reviewed"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    std::fs::write(
        loaded.dirs.workspace_root.join("alpha.txt"),
        "alpha_payload_value = 42\n",
    )
    .expect("write alpha");
    std::fs::write(
        loaded.dirs.workspace_root.join("beta.txt"),
        "beta_payload_value = 99\n",
    )
    .expect("write beta");

    let mut engine = Engine::spawn(Arc::new(loaded));
    wait_for_startup(&mut engine).await;

    engine
        .submit(Op::UserTurn {
            items: vec![
                InputItem::text("Compare with @beta.txt"),
                InputItem::File {
                    path: "alpha.txt".into(),
                },
            ],
            overrides: TurnOverrides::default(),
        })
        .await;
    loop {
        if matches!(next_event(&mut engine).await, Event::TurnComplete { .. }) {
            break;
        }
    }

    let reqs = bodies(&server).await;
    let dump = serde_json::to_string(&reqs[0]).expect("json");
    assert!(
        dump.contains("alpha_payload_value = 42"),
        "expected InputItem::File content attached: {dump}"
    );
    assert!(
        dump.contains("beta_payload_value = 99"),
        "expected plain @beta.txt content attached: {dump}"
    );

    engine.shutdown().await;
}
