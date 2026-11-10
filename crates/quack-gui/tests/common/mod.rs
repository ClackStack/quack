//! Helpers shared by the bridge tests, mirroring `quack-core/tests/common`.
#![allow(dead_code)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use quack_config::{LoadOptions, Loaded, load_with};
use quack_gui::TabMessage;
use quack_gui::registry::Loader;
use wiremock::ResponseTemplate;

/// A streamed reply with only text.
pub fn sse_text(text: &str) -> ResponseTemplate {
    let chunk = serde_json::json!({"choices":[{"delta":{"content":text},"finish_reason":"stop"}]});
    sse_raw(&[chunk.to_string()])
}

/// A streamed reply that makes one `write` tool call, split over two chunks the way
/// real streams arrive.
pub fn sse_write_call(id: &str, path: &str, content: &str) -> ResponseTemplate {
    let args = serde_json::json!({"path": path, "content": content}).to_string();
    let (head, tail) = args.split_at(args.len() / 2);
    sse_raw(&[
        serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":id,"function":{"name":"write","arguments":head.to_owned()}}]}}]}).to_string(),
        serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":tail.to_owned()}}]},"finish_reason":"tool_calls"}]}).to_string(),
    ])
}

pub fn sse_raw(chunks: &[String]) -> ResponseTemplate {
    let body: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body + "data: [DONE]\n\n")
}

/// Isolated directories under `root`, one provider file `mock.toml`, no defaults
/// bundle, workspace at `root/ws`. Loaded with an explicit variable map, never the
/// process environment.
pub fn workspace(root: &Path, base_url: &str) -> Loaded {
    let user = root.join("user");
    std::fs::create_dir_all(user.join("providers.d")).expect("mkdir");
    std::fs::write(
        user.join("providers.d").join("mock.toml"),
        format!("base_url = \"{base_url}/v1\"\nmodels = [\"tiny\"]\n"),
    )
    .expect("provider");
    let mut vars: HashMap<String, OsString> = HashMap::new();
    vars.insert("QUACK_CONFIG_DIR".into(), user.into_os_string());
    for (k, sub) in [
        ("QUACK_SYSTEM_DIR", "system"),
        ("QUACK_DATA_DIR", "data"),
        ("QUACK_CACHE_DIR", "cache"),
    ] {
        vars.insert(k.into(), root.join(sub).into_os_string());
    }
    vars.insert("QUACK_DEFAULTS_DIR".into(), "none".into());
    let ws = root.join("ws");
    std::fs::create_dir_all(&ws).expect("mkdir");
    load_with(&move |k| vars.get(k).cloned(), &LoadOptions::new(ws)).expect("load")
}

/// A loader that ignores the requested folder and returns `loaded`; enough for tests
/// that open one workspace, or the same workspace twice.
pub fn fixed_loader(loaded: Loaded) -> Loader {
    Arc::new(move |_opts: LoadOptions| Ok(loaded.clone()))
}

/// A sink collecting into an unbounded channel.
pub fn collecting_sink() -> (
    Arc<dyn quack_gui::Sink>,
    tokio::sync::mpsc::UnboundedReceiver<TabMessage>,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let sink: Arc<dyn quack_gui::Sink> = Arc::new(move |m: TabMessage| {
        let _ = tx.send(m);
    });
    (sink, rx)
}

/// The next message within 10 s, or panic with a hint of where we got stuck.
pub async fn next(rx: &mut tokio::sync::mpsc::UnboundedReceiver<TabMessage>) -> TabMessage {
    tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("message within 10s")
        .expect("sink open")
}

/// Skip messages until `want` matches; `want` must not swallow the message it wants.
pub async fn until(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<TabMessage>,
    mut want: impl FnMut(&TabMessage) -> bool,
) -> TabMessage {
    loop {
        let m = next(rx).await;
        if want(&m) {
            return m;
        }
    }
}

/// Events seen while `want` returns `None`, in order, panicking on engine errors.
pub async fn collect_until(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<TabMessage>,
    mut want: impl FnMut(&quack_protocol::Event) -> bool,
) -> (Vec<quack_protocol::Event>, quack_protocol::Event) {
    use quack_protocol::Event;
    let mut seen = Vec::new();
    loop {
        let m = next(rx).await;
        let quack_gui::Payload::Event { event } = m.payload else {
            continue;
        };
        match &event {
            Event::Error { error } => panic!("engine error: {}", error.message),
            Event::TurnAborted { reason, .. } => panic!("aborted: {reason}"),
            e if want(e) => return (seen, event),
            _ => seen.push(event),
        }
    }
}

/// The workspace root of a `Loaded` built by [`workspace`].
pub fn ws_of(root: &Path) -> PathBuf {
    root.join("ws")
}
