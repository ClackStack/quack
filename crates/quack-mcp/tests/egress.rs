//! The egress gate for remote (streamable HTTP) MCP servers: offline mode blocks a
//! server off this machine unless `egress_allow` names it, a loopback server keeps
//! working, and a redirect can never step around the gate.

use std::sync::Arc;
use std::time::Duration;

use quack_config::McpServerFile;
use quack_config::settings::NetworkSettings;
use quack_mcp::{McpManager, ServerSpec, ServerState};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// Answers JSON-RPC like a small streamable MCP server. `server/discover` is
/// answered with a method-not-found error, which the 2026-07-28 lifecycle reads as
/// "legacy peer" and follows with the classic `initialize` handshake; the rest of
/// the exchange echoes each request's id.
#[derive(Debug)]
struct McpHttp;

impl wiremock::Respond for McpHttp {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: serde_json::Value =
            serde_json::from_slice(&request.body).unwrap_or(serde_json::Value::Null);
        let method = body["method"].as_str().unwrap_or_default().to_owned();
        let id = body.get("id").cloned();
        let Some(id) = id else {
            return ResponseTemplate::new(202);
        };
        let message = match method.as_str() {
            "server/discover" => serde_json::json!({
                "error": { "code": -32601, "message": "this server predates discovery" }
            }),
            "initialize" => serde_json::json!({ "result": {
                "protocolVersion": body["params"]["protocolVersion"].clone(),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "fx", "version": "1" },
            } }),
            "tools/list" => serde_json::json!({ "result": { "tools": [
                { "name": "echo", "inputSchema": { "type": "object" } }
            ] } }),
            "prompts/list" => serde_json::json!({ "result": { "prompts": [] } }),
            "resources/list" => serde_json::json!({ "result": { "resources": [] } }),
            "ping" => serde_json::json!({ "result": {} }),
            other => serde_json::json!({
                "error": { "code": -32601, "message": format!("no {other}") }
            }),
        };
        let mut reply = serde_json::json!({ "jsonrpc": "2.0", "id": id });
        if let Some(fields) = message.as_object() {
            for (k, v) in fields {
                reply[k] = v.clone();
            }
        }
        ResponseTemplate::new(200).set_body_json(reply)
    }
}

/// A wiremock server that speaks enough MCP to reach `Ready`.
async fn mcp_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(McpHttp)
        .mount(&server)
        .await;
    // The transport may ask for a server-stream; "no streams here" is a valid answer.
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(405))
        .mount(&server)
        .await;
    server
}

fn spec(url: &str) -> ServerSpec {
    let file: McpServerFile = toml::from_str(&format!("url = \"{url}\"\n")).expect("server file");
    ServerSpec {
        id: "fx".into(),
        file,
    }
}

fn manager(spec: ServerSpec, offline: bool) -> (Arc<McpManager>, tempfile::TempDir) {
    let network = NetworkSettings {
        offline,
        ..NetworkSettings::default()
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let m = McpManager::new(vec![spec], tmp.path().to_path_buf(), network);
    m.start();
    (m, tmp)
}

async fn wait_for(
    m: &Arc<McpManager>,
    want: impl Fn(&ServerState) -> bool,
    what: &str,
) -> ServerState {
    let mut rx = m.subscribe();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let s = m
            .status()
            .into_iter()
            .find(|s| s.id == "fx")
            .expect("server");
        if want(&s.state) {
            return s.state;
        }
        // One `changed()` per iteration, inside the deadline: an untimed second
        // await would sleep forever once the server settles in its final state.
        match tokio::time::timeout_at(deadline, rx.changed()).await {
            Err(_) => panic!("{what} within 20s, stuck at {:?}", s.state),
            Ok(Err(_)) => panic!("manager went away while waiting for {what}"),
            Ok(Ok(())) => {}
        }
    }
}

async fn ready(m: &Arc<McpManager>) {
    wait_for(m, |s| matches!(s, ServerState::Ready), "ready").await;
}

async fn failed(m: &Arc<McpManager>) -> String {
    match wait_for(m, |s| matches!(s, ServerState::Failed(_)), "failure").await {
        ServerState::Failed(e) => e,
        other => panic!("failed state, got {other:?}"),
    }
}

/// A server off this machine never starts while offline: the gate refuses it before
/// anything is sent.
#[tokio::test]
async fn an_off_machine_server_is_blocked_in_offline_mode() {
    let (m, _tmp) = manager(spec("http://elsewhere.example/mcp"), true);
    let e = failed(&m).await;
    assert!(e.contains("network.offline"), "{e}");
    assert!(e.contains("elsewhere.example"), "{e}");
    m.shutdown().await;
}

/// The gate is selective, not a blanket ban: a server on this machine starts fine.
#[tokio::test]
async fn a_loopback_server_works_in_offline_mode() {
    let server = mcp_server().await;
    let (m, _tmp) = manager(spec(&server.uri()), true);
    ready(&m).await;
    assert_eq!(m.tools()[0].name, "echo");
    m.shutdown().await;
}

/// A redirect from an allowed server to another host is refused by the same gate, so
/// the URL in the file cannot be used as a trampoline. The refusal names the
/// original URL (`error following redirect ... 127.0.0.1`); without the gate the
/// redirect would have been followed and failed against the target host instead.
#[tokio::test]
async fn a_redirect_cannot_step_around_the_offline_gate() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "http://elsewhere.example/mcp"),
        )
        .mount(&server)
        .await;
    let (m, _tmp) = manager(spec(&server.uri()), true);
    let e = failed(&m).await;
    assert!(e.contains("error following redirect"), "{e}");
    assert!(e.contains("127.0.0.1"), "{e}");
    assert!(!e.contains("elsewhere.example"), "{e}");
    m.shutdown().await;
}

/// Redirects within the host the chain started from still work (a path or scheme
/// change on the same server), in offline mode too.
#[tokio::test]
async fn a_redirect_within_the_same_host_is_still_followed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/real"))
        .respond_with(McpHttp)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/start"))
        // 308 (not 302): it preserves the POST method across the redirect, so the
        // handshake arrives at /real as the POST the JSON-RPC mocks answer.
        .respond_with(ResponseTemplate::new(308).insert_header("location", "/real"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(405))
        .mount(&server)
        .await;
    let (m, _tmp) = manager(spec(&format!("{}/start", server.uri())), true);
    ready(&m).await;
    m.shutdown().await;
}
