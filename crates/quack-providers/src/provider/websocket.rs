//! Responses continuation on an authenticated socket. A socket can extend only the
//! exact conversation it already holds. The full local request is always available.

use std::fmt;
use std::io;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use reqwest::header::{self, HeaderMap};
use ring::digest;
use serde::Serialize;
use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout, timeout_at};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{
    Message, handshake::client::generate_key, handshake::derive_accept_key, protocol::Role,
};

use super::{
    EgressDraft, EgressKind, ErrorKind, Provider, ProviderError, RequestHeaders, ResponsesSession,
    SocketAuth,
};
use super::{StreamEvent, StreamLedgerGuard, forward, responses};

const PROTOCOL: &str = "openai-responses-websocket";
const RECONNECT_DELAY: Duration = Duration::from_secs(60);
const MAX_SOCKET_AGE: Duration = Duration::from_secs(55 * 60);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(20);
const KEEPALIVE_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(super) struct SessionSocket {
    connection: Option<ParkedConnection>,
    retry_after: Option<Instant>,
    #[cfg(test)]
    keepalive_override: Option<Duration>,
}

impl SessionSocket {
    fn keepalive_interval(&self) -> Duration {
        #[cfg(test)]
        if let Some(d) = self.keepalive_override {
            return d;
        }
        KEEPALIVE_INTERVAL
    }
}

impl fmt::Debug for SessionSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionSocket")
            .field("connected", &self.connection.is_some())
            .field("retry_after", &self.retry_after)
            .finish()
    }
}

/// An idle Responses WebSocket kept warm by a background task that sends periodic Ping
/// frames, flushes Pong replies to server Pings, and drops the socket promptly on Close.
struct ParkedWire {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<Option<WebSocketStream<reqwest::Upgraded>>>,
}

impl ParkedWire {
    fn park(mut wire: WebSocketStream<reqwest::Upgraded>, interval: Duration) -> Self {
        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            tick.tick().await;
            loop {
                tokio::select! {
                    biased;
                    _ = &mut stop_rx => return Some(wire),
                    _ = tick.tick() => {
                        if !matches!(
                            timeout(
                                KEEPALIVE_WRITE_TIMEOUT,
                                wire.send(Message::Ping(Default::default())),
                            )
                            .await,
                            Ok(Ok(()))
                        ) {
                            return None;
                        }
                    }
                    incoming = wire.next() => match incoming {
                        Some(Ok(Message::Ping(_))) => {
                            if !matches!(
                                timeout(KEEPALIVE_WRITE_TIMEOUT, wire.flush()).await,
                                Ok(Ok(()))
                            ) {
                                return None;
                            }
                        }
                        Some(Ok(Message::Pong(_))) => {}
                        _ => return None,
                    }
                }
            }
        });
        Self {
            stop: Some(stop_tx),
            task,
        }
    }

    async fn take(mut self) -> Option<WebSocketStream<reqwest::Upgraded>> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        match timeout(KEEPALIVE_WRITE_TIMEOUT, &mut self.task).await {
            Ok(Ok(wire)) => wire,
            _ => {
                self.task.abort();
                None
            }
        }
    }
}

impl Drop for ParkedWire {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct ParkedConnection {
    wire: ParkedWire,
    opened: Instant,
    provider: Arc<quack_config::ProviderFile>,
    auth: SocketAuth,
    previous: Option<Continuation>,
}

impl ParkedConnection {
    fn belongs_to(&self, provider: &Provider, auth: &SocketAuth) -> bool {
        self.opened.elapsed() < MAX_SOCKET_AGE
            && Arc::ptr_eq(&self.provider, &provider.file)
            && &self.auth == auth
    }

    async fn into_active(self) -> Option<Connection> {
        let wire = self.wire.take().await?;
        Some(Connection {
            wire,
            opened: self.opened,
            provider: self.provider,
            auth: self.auth,
            previous: self.previous,
        })
    }
}

struct Connection {
    wire: WebSocketStream<reqwest::Upgraded>,
    opened: Instant,
    provider: Arc<quack_config::ProviderFile>,
    auth: SocketAuth,
    previous: Option<Continuation>,
}

impl Connection {
    fn park(self, interval: Duration) -> ParkedConnection {
        ParkedConnection {
            wire: ParkedWire::park(self.wire, interval),
            opened: self.opened,
            provider: self.provider,
            auth: self.auth,
            previous: self.previous,
        }
    }
}

struct Continuation {
    id: String,
    settings: digest::Digest,
    prefix_len: usize,
    prefix: digest::Digest,
}

struct Request<'a> {
    input: &'a [Value],
    fields: Map<String, Value>,
    settings: digest::Digest,
}

#[derive(Serialize)]
struct Frame<'a> {
    #[serde(rename = "type")]
    event: &'static str,
    #[serde(flatten)]
    fields: &'a Map<String, Value>,
    input: &'a [Value],
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_response_id: Option<&'a str>,
}

impl<'a> Request<'a> {
    fn new(body: &'a Value) -> Option<Self> {
        let input = body["input"].as_array()?;
        if body["store"] != false
            || body.get("previous_response_id").is_some()
            || body.get("conversation").is_some()
            || body["background"] == true
        {
            return None;
        }
        let mut fields: Map<_, _> = body
            .as_object()?
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "input" | "stream" | "background" | "type"))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let mut settings = DigestWriter::new();
        let cache_options = fields.remove("prompt_cache_options");
        serde_json::to_writer(&mut settings, &fields).ok()?;
        if let Some(options) = cache_options {
            let mut comparable = options.clone();
            if let Some(object) = comparable.as_object_mut() {
                object.remove("comparison_response_id");
            }
            if !comparable.as_object().is_some_and(Map::is_empty) {
                serde_json::to_writer(&mut settings, &comparable).ok()?;
            }
            fields.insert("prompt_cache_options".into(), options);
        }
        Some(Self {
            input,
            fields,
            settings: settings.0.finish(),
        })
    }

    fn frame<'b>(&'b self, previous: Option<&'b Continuation>) -> Frame<'b> {
        let (input, previous_response_id) = if let Some(prev) = previous
            && prev.settings.as_ref() == self.settings.as_ref()
            && let Some(prefix) = self.input.get(..prev.prefix_len)
            && digest_items(prefix).is_some_and(|digest| digest.as_ref() == prev.prefix.as_ref())
        {
            (&self.input[prev.prefix_len..], Some(prev.id.as_str()))
        } else {
            (self.input, None)
        };
        Frame {
            event: "response.create",
            fields: &self.fields,
            input,
            previous_response_id,
        }
    }

    fn completed(&self, response: &Value, route: responses::Route) -> Option<Continuation> {
        let id = response["id"].as_str()?.to_owned();
        let output = responses::replay_output(response, route)?;
        Some(Continuation {
            id,
            settings: self.settings,
            prefix_len: self.input.len() + output.len(),
            prefix: digest_items(self.input.iter().chain(&output))?,
        })
    }
}

struct DigestWriter(digest::Context);

impl DigestWriter {
    fn new() -> Self {
        Self(digest::Context::new(&digest::SHA256))
    }
}

impl io::Write for DigestWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn digest_items<'a>(items: impl IntoIterator<Item = &'a Value>) -> Option<digest::Digest> {
    let mut writer = DigestWriter::new();
    for item in items {
        serde_json::to_writer(&mut writer, item).ok()?;
        // A raw newline never occurs inside a compact JSON value.
        writer.0.update(b"\n");
    }
    Some(writer.0.finish())
}

enum Outcome {
    Completed(Option<Continuation>),
    Reopen(ProviderError),
    Failed(ProviderError),
    ReceiverClosed,
}

impl Provider {
    /// True once the socket delivered a response or a terminal error. A refused upgrade
    /// or a socket unavailable before a response starts can use the same body over HTTP.
    pub(super) async fn stream_websocket(
        &self,
        session: &ResponsesSession,
        body: &Value,
        tx: &mpsc::Sender<Result<StreamEvent, ProviderError>>,
    ) -> bool {
        let mut state = session.socket.lock().await;
        let Some(request) = Request::new(body) else {
            state.connection = None;
            return false;
        };
        if state
            .retry_after
            .is_some_and(|until| until > Instant::now())
        {
            return false;
        }

        let url = self.url(responses::RESPONSES_PATH);
        let keepalive = state.keepalive_interval();
        // Take ownership until completion so cancellation drops any partial exchange.
        let existing = state.connection.take();
        let (headers, auth) = match self.websocket_auth().await {
            Ok(resolved) => resolved,
            Err(error) => {
                self.record_ledger(
                    EgressDraft::new(EgressKind::Inference, &url, PROTOCOL)
                        .blocked(error.message.clone()),
                );
                let _ = tx.send(Err(error)).await;
                return true;
            }
        };
        let mut connection = match existing {
            Some(parked) if parked.belongs_to(self, &auth) => parked.into_active().await,
            _ => None,
        };
        loop {
            let reused = connection.is_some();
            let mut socket = match connection.take() {
                Some(socket) => socket,
                None => match self
                    .open_websocket(&url, headers.clone(), auth.clone())
                    .await
                {
                    Ok(socket) => socket,
                    Err(error) => {
                        self.record_ledger(
                            EgressDraft::new(EgressKind::Inference, &url, PROTOCOL)
                                .status(error.status)
                                .blocked(error.message),
                        );
                        self.websocket_fallback(&mut state);
                        return false;
                    }
                },
            };
            let frame = match serde_json::to_string(&request.frame(socket.previous.as_ref())) {
                Ok(frame) => frame,
                Err(error) => {
                    let _ = tx
                        .send(Err(ProviderError::new(
                            ErrorKind::Protocol,
                            format!("encode Responses WebSocket request: {error}"),
                        )))
                        .await;
                    return true;
                }
            };
            let redactions = self.pending_redactions.swap(0, Ordering::Relaxed);
            let mut ledger = StreamLedgerGuard {
                provider: self,
                draft: Some(
                    EgressDraft::new(EgressKind::Inference, &url, PROTOCOL)
                        .sent(frame.len() as u64)
                        .status(Some(101))
                        .redactions(redactions),
                ),
            };
            match self
                .websocket_turn(&mut socket, &request, frame, &mut ledger, tx)
                .await
            {
                Outcome::Completed(previous) => {
                    socket.previous = previous;
                    state.connection = Some(socket.park(keepalive));
                    state.retry_after = None;
                    return true;
                }
                Outcome::Reopen(error) => {
                    if let Some(draft) = &mut ledger.draft {
                        draft.blocked_reason = Some(error.message);
                    }
                    if !reused {
                        self.websocket_fallback(&mut state);
                        return false;
                    }
                    // The server never confirmed this exchange. Open a fresh socket and
                    // send the full history without the connection-local response ID.
                }
                Outcome::Failed(error) => {
                    if let Some(draft) = &mut ledger.draft {
                        draft.blocked_reason = Some(error.message.clone());
                    }
                    let _ = tx.send(Err(error)).await;
                    return true;
                }
                Outcome::ReceiverClosed => return true,
            }
        }
    }

    fn websocket_fallback(&self, state: &mut SessionSocket) {
        state.retry_after = Some(Instant::now() + RECONNECT_DELAY);
        tracing::debug!(provider = %self.id, "Responses WebSocket unavailable; using HTTP");
    }

    async fn open_websocket(
        &self,
        url: &str,
        headers: RequestHeaders,
        auth: SocketAuth,
    ) -> Result<Connection, ProviderError> {
        // HTTP/1.1 upgrade, with the same TLS and proxy configuration as inference.
        // Never follow an upgrade redirect with authentication or a conversation body.
        let client = self
            .spec
            .builder()
            .http1_only()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| ProviderError::from_reqwest(&e))?;
        let key = generate_key();
        let mut request = headers
            .clone()
            .apply(client.get(url))
            .timeout(self.spec.connect.min(self.first_byte_timeout(0)))
            .build()
            .map_err(|e| ProviderError::from_reqwest(&e))?;
        let values = request.headers_mut();
        values.insert(
            header::CONNECTION,
            header::HeaderValue::from_static("Upgrade"),
        );
        values.insert(
            header::UPGRADE,
            header::HeaderValue::from_static("websocket"),
        );
        values.insert(
            header::SEC_WEBSOCKET_VERSION,
            header::HeaderValue::from_static("13"),
        );
        insert_key(values, &key)?;
        let response = client
            .execute(request)
            .await
            .map_err(|e| ProviderError::from_reqwest(&e))?;
        let status = response.status().as_u16();
        if status != 101 {
            return Err(ProviderError::from_status(
                status,
                None,
                "WebSocket upgrade was refused",
            ));
        }
        let expected = derive_accept_key(key.as_bytes());
        let values = response.headers();
        if !header_token(values, header::CONNECTION, "upgrade")
            || !header_token(values, header::UPGRADE, "websocket")
            || values
                .get(header::SEC_WEBSOCKET_ACCEPT)
                .and_then(|v| v.to_str().ok())
                != Some(expected.as_str())
        {
            return Err(ProviderError::new(
                ErrorKind::Protocol,
                "invalid WebSocket upgrade response",
            ));
        }
        let io = response
            .upgrade()
            .await
            .map_err(|e| ProviderError::from_reqwest(&e))?;
        Ok(Connection {
            wire: WebSocketStream::from_raw_socket(io, Role::Client, None).await,
            opened: Instant::now(),
            provider: self.file.clone(),
            auth,
            previous: None,
        })
    }

    async fn websocket_turn(
        &self,
        socket: &mut Connection,
        request: &Request<'_>,
        frame: String,
        ledger: &mut StreamLedgerGuard<'_>,
        tx: &mpsc::Sender<Result<StreamEvent, ProviderError>>,
    ) -> Outcome {
        let first = self.first_byte_timeout(0);
        let deadline = Instant::now() + first;
        match timeout_at(deadline, socket.wire.send(Message::Text(frame.into()))).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => return Outcome::Reopen(disconnected()),
            Err(_) => return Outcome::Reopen(stalled()),
        }
        let idle = Duration::from_secs(self.file.http.timeout_idle_secs.max(1));
        let mut parser = responses::EventParser::default();
        let mut started = false;
        loop {
            let read_deadline = if started {
                Instant::now() + idle
            } else {
                deadline
            };
            let incoming = match timeout_at(read_deadline, socket.wire.next()).await {
                Ok(Some(Ok(incoming))) => incoming,
                Ok(_) => return socket_error(started, disconnected()),
                Err(_) => return socket_error(started, stalled()),
            };
            if let Some(draft) = &mut ledger.draft {
                draft.bytes_received = draft.bytes_received.saturating_add(incoming.len() as u64);
            }
            let text = match incoming {
                Message::Text(text) => text,
                Message::Ping(_) => {
                    if !matches!(timeout(idle, socket.wire.flush()).await, Ok(Ok(()))) {
                        return socket_error(started, disconnected());
                    }
                    continue;
                }
                Message::Close(_) => return socket_error(started, disconnected()),
                _ => continue,
            };
            let value: Value = match serde_json::from_str(text.as_str()) {
                Ok(value) => value,
                Err(_) => {
                    return Outcome::Failed(ProviderError::new(
                        ErrorKind::Protocol,
                        "invalid Responses WebSocket event",
                    ));
                }
            };
            let kind = value["type"].as_str().unwrap_or_default();
            if matches!(kind, "error" | "response.failed") {
                let code = value["error"]["code"]
                    .as_str()
                    .or_else(|| value["response"]["error"]["code"].as_str());
                if !started
                    && matches!(
                        code,
                        Some("previous_response_not_found" | "websocket_connection_limit_reached")
                    )
                {
                    return Outcome::Reopen(ProviderError::new(
                        ErrorKind::Network,
                        "Responses WebSocket continuation expired",
                    ));
                }
            }
            started |= kind.starts_with("response.");
            let parsed = match parser.parse_value(&value) {
                Ok(events) => events,
                Err(error) => return Outcome::Failed(error),
            };
            if !forward(Ok(parsed), tx).await {
                return Outcome::ReceiverClosed;
            }
            match kind {
                "response.completed" => {
                    return Outcome::Completed(request.completed(&value["response"], self.route));
                }
                "response.incomplete" => return Outcome::Completed(None),
                _ => {}
            }
        }
    }
}

fn socket_error(started: bool, error: ProviderError) -> Outcome {
    if started {
        Outcome::Failed(error)
    } else {
        Outcome::Reopen(error)
    }
}

fn disconnected() -> ProviderError {
    ProviderError::new(ErrorKind::Network, "the Responses WebSocket disconnected")
}

fn stalled() -> ProviderError {
    ProviderError::new(
        ErrorKind::Timeout,
        "the Responses WebSocket stopped responding",
    )
}

fn insert_key(headers: &mut HeaderMap, key: &str) -> Result<(), ProviderError> {
    let value = header::HeaderValue::from_str(key)
        .map_err(|_| ProviderError::new(ErrorKind::Protocol, "invalid WebSocket handshake key"))?;
    headers.insert(header::SEC_WEBSOCKET_KEY, value);
    Ok(())
}

fn header_token(headers: &HeaderMap, name: header::HeaderName, expected: &str) -> bool {
    headers.get_all(name).iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case(expected))
        })
    })
}

#[cfg(test)]
mod tests {
    use quack_config::provider::ReasoningFormat;
    use serde_json::json;
    use tokio::net::{TcpListener, TcpStream};
    use tokio_tungstenite::accept_hdr_async;
    use tokio_tungstenite::tungstenite::handshake::server::Request as UpgradeRequest;
    use tokio_util::sync::CancellationToken;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::{ChatRequest, EgressLedger, Message as ChatMessage, ToolCall};

    fn provider(base: &str) -> Provider {
        let file = serde_json::from_value(json!({
            "api": "openai-responses",
            "base_url": format!("{base}/v1"),
            "api_key": "fixture-key",
            "responses_websocket": true,
            "http": { "no_proxy": true }
        }))
        .expect("provider config");
        Provider::new("test", file, &Default::default(), &|_| None).expect("provider")
    }

    fn user(content: &str) -> ChatMessage {
        ChatMessage::User {
            content: content.into(),
            images: Vec::new(),
        }
    }

    async fn stream(
        p: &Provider,
        session: &Arc<ResponsesSession>,
        messages: Vec<ChatMessage>,
    ) -> Vec<StreamEvent> {
        let mut rx = p.stream_in(
            ChatRequest {
                model: "test-model".into(),
                messages,
                ..Default::default()
            },
            ReasoningFormat::Auto,
            CancellationToken::new(),
            session.clone(),
        );
        let mut result = Vec::new();
        while let Some(event) = rx.recv().await {
            result.push(event.expect("successful response"));
        }
        result
    }

    #[expect(
        clippy::result_large_err,
        reason = "tungstenite fixes the handshake callback's error type"
    )]
    async fn accept(listener: &TcpListener) -> WebSocketStream<TcpStream> {
        let (stream, _) = listener.accept().await.expect("connect");
        accept_hdr_async(stream, |req: &UpgradeRequest, response| {
            assert_eq!(req.uri().path(), "/v1/responses");
            assert_eq!(req.headers()[header::AUTHORIZATION], "Bearer fixture-key");
            Ok(response)
        })
        .await
        .expect("upgrade")
    }

    async fn next_frame(wire: &mut WebSocketStream<TcpStream>) -> Value {
        let frame = wire.next().await.expect("frame").expect("valid frame");
        serde_json::from_str(frame.into_text().expect("text frame").as_str()).expect("JSON frame")
    }

    async fn send(wire: &mut WebSocketStream<TcpStream>, event: Value) {
        wire.send(Message::Text(event.to_string().into()))
            .await
            .expect("send");
    }

    async fn complete(wire: &mut WebSocketStream<TcpStream>, id: &str, output: Value) {
        send(
            wire,
            json!({
                "type": "response.completed",
                "response": {
                    "id": id, "output": output,
                    "usage": { "input_tokens": 2000, "output_tokens": 4,
                        "input_tokens_details": { "cached_tokens": 1500 } }
                }
            }),
        )
        .await;
    }

    #[tokio::test]
    async fn continues_tool_calls_and_replays_full_history_after_expiry() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let p = provider(&format!(
            "http://{}",
            listener.local_addr().expect("address")
        ));
        let session = Arc::new(ResponsesSession::default());
        let call = ToolCall {
            id: "call_one".into(),
            name: "read".into(),
            arguments: "{}".into(),
        };
        let history = vec![
            user("read a file"),
            ChatMessage::Assistant {
                content: String::new(),
                reasoning: None,
                tool_calls: vec![call.clone()],
                sealed: None,
            },
            ChatMessage::Tool {
                call_id: call.id.clone(),
                content: "a fixture".into(),
                images: Vec::new(),
            },
        ];
        let full_history = responses::build_body(
            &ChatRequest {
                model: "test-model".into(),
                messages: history.clone(),
                ..Default::default()
            },
            responses::Route::Api,
        )["input"]
            .clone();

        let server =
            tokio::spawn(async move {
                let mut wire = accept(&listener).await;
                let first = next_frame(&mut wire).await;
                assert_eq!(first["type"], "response.create");
                assert_eq!(first["store"], false);
                assert_eq!(
                    first["input"],
                    json!([{"role":"user","content":"read a file"}])
                );
                assert!(first.get("previous_response_id").is_none());
                assert!(first.get("stream").is_none());
                let call = json!({
                    "type": "function_call", "name": call.name,
                    "call_id": call.id, "arguments": call.arguments
                });
                send(
                    &mut wire,
                    json!({
                        "type":"response.output_item.added","output_index":0,"item":call
                    }),
                )
                .await;
                send(&mut wire, json!({
                "type":"response.function_call_arguments.done","output_index":0,"arguments":"{}"
            })).await;
                complete(&mut wire, "resp_first", json!([call])).await;

                let next = next_frame(&mut wire).await;
                assert_eq!(next["previous_response_id"], "resp_first");
                assert_eq!(next["input"], json!([full_history[2]]));
                send(
                    &mut wire,
                    json!({
                        "type": "error", "error": {"code": "previous_response_not_found"}
                    }),
                )
                .await;

                let mut fresh = accept(&listener).await;
                let replay = next_frame(&mut fresh).await;
                assert!(replay.get("previous_response_id").is_none());
                assert_eq!(replay["input"], full_history);
                complete(&mut fresh, "resp_after_reconnect", json!([])).await;

                let changed = next_frame(&mut fresh).await;
                assert!(changed.get("previous_response_id").is_none());
                assert_eq!(
                    changed["input"],
                    json!([{"role":"user","content":"rewound"}])
                );
                complete(&mut fresh, "resp_rewound", json!([])).await;
            });

        let first = stream(&p, &session, vec![user("read a file")]).await;
        assert!(
            first
                .iter()
                .any(|e| matches!(e, StreamEvent::ToolCallStart { .. }))
        );
        let second = stream(&p, &session, history).await;
        assert!(
            second
                .iter()
                .any(|e| matches!(e, StreamEvent::ResponseId(id) if id == "resp_after_reconnect"))
        );
        assert!(
            second.iter().any(
                |e| matches!(e, StreamEvent::Usage(u) if u.cached_prompt_tokens == Some(1500))
            )
        );
        stream(&p, &session, vec![user("rewound")]).await;
        server.await.expect("server assertions");
    }

    #[tokio::test]
    async fn falls_back_to_http_without_following_an_upgrade_redirect() {
        let server = MockServer::start().await;
        let redirected = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", redirected.uri()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string("data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_http\"}}\n\n"))
            .expect(2)
            .mount(&server).await;
        let p = provider(&server.uri());
        let ledger = EgressLedger::new();
        p.set_ledger(ledger.clone());
        let session = Arc::new(ResponsesSession::default());
        for _ in 0..2 {
            let events = stream(&p, &session, vec![user("hello")]).await;
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, StreamEvent::ResponseId(id) if id == "resp_http"))
            );
        }
        assert!(
            redirected
                .received_requests()
                .await
                .expect("requests")
                .is_empty()
        );
        server.verify().await;
        let (entries, _) = ledger.snapshot();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.protocol == PROTOCOL)
                .count(),
            1
        );
        assert_eq!(entries[0].status, Some(302));
    }

    #[tokio::test]
    async fn cancellation_drops_the_active_socket() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let p = provider(&format!(
            "http://{}",
            listener.local_addr().expect("address")
        ));
        let session = Arc::new(ResponsesSession::default());
        let server = tokio::spawn(async move {
            let mut wire = accept(&listener).await;
            next_frame(&mut wire).await;
            send(
                &mut wire,
                json!({"type":"response.output_text.delta","delta":"partial"}),
            )
            .await;
            assert!(!matches!(wire.next().await, Some(Ok(Message::Text(_)))));
            let mut fresh = accept(&listener).await;
            let replay = next_frame(&mut fresh).await;
            assert!(replay.get("previous_response_id").is_none());
            complete(&mut fresh, "resp_fresh", json!([])).await;
        });
        let cancel = CancellationToken::new();
        let mut rx = p.stream_in(
            ChatRequest {
                model: "test-model".into(),
                messages: vec![user("first")],
                ..Default::default()
            },
            ReasoningFormat::Auto,
            cancel.clone(),
            session.clone(),
        );
        assert!(matches!(
            rx.recv().await,
            Some(Ok(StreamEvent::TextDelta(_)))
        ));
        cancel.cancel();
        assert!(rx.recv().await.is_none());
        let events = stream(&p, &session, vec![user("second")]).await;
        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::ResponseId(id) if id == "resp_fresh"))
        );
        server.await.expect("server assertions");
    }

    #[tokio::test]
    async fn a_disconnect_after_output_does_not_replay_the_request() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let p = provider(&format!(
            "http://{}",
            listener.local_addr().expect("address")
        ));
        let ledger = EgressLedger::new();
        p.set_ledger(ledger.clone());
        let server = tokio::spawn(async move {
            let mut wire = accept(&listener).await;
            next_frame(&mut wire).await;
            send(
                &mut wire,
                json!({"type":"response.output_text.delta","delta":"partial"}),
            )
            .await;
            wire.close(None).await.expect("close");
        });
        let mut rx = p.stream_in(
            ChatRequest {
                model: "test-model".into(),
                messages: vec![user("hello")],
                ..Default::default()
            },
            ReasoningFormat::Auto,
            CancellationToken::new(),
            Arc::default(),
        );
        assert!(matches!(
            rx.recv().await,
            Some(Ok(StreamEvent::TextDelta(_)))
        ));
        assert!(matches!(rx.recv().await, Some(Err(error)) if error.kind == ErrorKind::Network));
        assert!(rx.recv().await.is_none());
        server.await.expect("server assertions");
        assert_eq!(ledger.snapshot().0.len(), 1);
    }

    #[tokio::test]
    async fn idle_keepalive_answers_server_ping_and_sends_client_ping() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let p = provider(&format!(
            "http://{}",
            listener.local_addr().expect("address")
        ));
        let session = Arc::new(ResponsesSession::default());
        session.socket.lock().await.keepalive_override = Some(Duration::from_millis(40));

        let server = tokio::spawn(async move {
            let mut wire = accept(&listener).await;
            let first = next_frame(&mut wire).await;
            assert!(first.get("previous_response_id").is_none());
            complete(&mut wire, "resp_warm", json!([])).await;

            // Send a server ping while the client is idle; expect Pong AND a client Ping.
            wire.send(Message::Ping(b"hi".to_vec().into()))
                .await
                .expect("server ping");
            let mut saw_pong = false;
            let mut saw_client_ping = false;
            let deadline = Instant::now() + Duration::from_secs(2);
            while !(saw_pong && saw_client_ping) {
                let frame = timeout_at(deadline, wire.next())
                    .await
                    .expect("keepalive frame within 2s")
                    .expect("stream open")
                    .expect("valid frame");
                match frame {
                    Message::Pong(p) if p.as_ref() == b"hi" => saw_pong = true,
                    Message::Ping(p) => {
                        saw_client_ping = true;
                        wire.send(Message::Pong(p)).await.expect("server pong");
                    }
                    other => panic!("unexpected idle frame: {other:?}"),
                }
            }

            // Second turn arrives on the same live socket with continuation intact.
            let second = loop {
                match wire.next().await.expect("frame").expect("valid") {
                    Message::Text(t) => {
                        break serde_json::from_str::<Value>(t.as_str()).expect("json");
                    }
                    Message::Ping(p) => wire.send(Message::Pong(p)).await.expect("pong"),
                    Message::Pong(_) => {}
                    other => panic!("unexpected frame: {other:?}"),
                }
            };
            assert_eq!(second["previous_response_id"], "resp_warm");
            complete(&mut wire, "resp_second", json!([])).await;
        });

        stream(&p, &session, vec![user("turn one")]).await;
        tokio::time::sleep(Duration::from_millis(120)).await;
        let events = stream(&p, &session, vec![user("turn one"), user("turn two")]).await;
        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::ResponseId(id) if id == "resp_second"))
        );
        server.await.expect("server assertions");
    }

    #[tokio::test]
    async fn server_close_while_idle_reopens_cleanly_without_http_fallback() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let p = provider(&format!(
            "http://{}",
            listener.local_addr().expect("address")
        ));
        let session = Arc::new(ResponsesSession::default());

        let server = tokio::spawn(async move {
            let mut first_wire = accept(&listener).await;
            next_frame(&mut first_wire).await;
            complete(&mut first_wire, "resp_one", json!([])).await;
            first_wire.close(None).await.expect("close idle socket");

            let mut second_wire = accept(&listener).await;
            let replay = next_frame(&mut second_wire).await;
            assert!(replay.get("previous_response_id").is_none());
            complete(&mut second_wire, "resp_two", json!([])).await;
        });

        stream(&p, &session, vec![user("turn one")]).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let events = stream(&p, &session, vec![user("turn one"), user("turn two")]).await;
        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::ResponseId(id) if id == "resp_two"))
        );
        server.await.expect("server assertions");
    }

    #[cfg(feature = "auth-chatgpt")]
    #[expect(
        clippy::result_large_err,
        reason = "tungstenite fixes the handshake callback's error type"
    )]
    #[tokio::test]
    async fn oauth_token_refresh_for_same_account_preserves_live_socket() {
        use std::sync::atomic::AtomicUsize;

        let issuer = MockServer::start().await;
        let refreshes = Arc::new(AtomicUsize::new(0));
        let counter = refreshes.clone();
        Mock::given(method("POST"))
            .and(path("/api/accounts/oauth/token"))
            .respond_with(move |_req: &wiremock::Request| {
                let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                // `expires_in: 10` is under `EARLY_SECS` (120), so each turn refreshes immediately.
                ResponseTemplate::new(200).set_body_json(json!({
                    "access_token": format!("access-token-{n}"),
                    "refresh_token": format!("refresh-token-{n}"),
                    "expires_in": 10,
                    "scope": "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct"
                }))
            })
            .mount(&issuer)
            .await;

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("address"));

        let data_dir = tempfile::tempdir().expect("tempdir");
        let auth_dir = data_dir.path().join("auth");
        std::fs::create_dir_all(&auth_dir).expect("mkdir auth");
        std::fs::write(
            auth_dir.join("chatgpt.json"),
            serde_json::to_string(&crate::login::Credentials {
                client_id: "client-1".into(),
                subject: "acct-same".into(),
                email: "dev@example.com".into(),
                access_token: "stale-token".into(),
                refresh_token: "refresh-token-0".into(),
                id_token: String::new(),
                expires_at: 1,
                scopes: vec!["chatgpt.tokens.use.direct".into()],
            })
            .expect("creds json"),
        )
        .expect("write creds");

        let file = serde_json::from_value(json!({
            "api": "openai-responses",
            "base_url": format!("{base}/v1"),
            "auth": { "type": "chatgpt", "issuer": issuer.uri() },
            "responses_websocket": true,
            "http": { "no_proxy": true }
        }))
        .expect("provider config");
        let data_os = data_dir.path().as_os_str().to_os_string();
        let p = Provider::new("chatgpt", file, &Default::default(), &|k| {
            (k == "QUACK_DATA_DIR").then(|| data_os.clone())
        })
        .expect("provider");
        let session = Arc::new(ResponsesSession::default());

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("connect");
            let mut wire = accept_hdr_async(stream, |req: &UpgradeRequest, response| {
                assert_eq!(
                    req.headers()[header::AUTHORIZATION],
                    "Bearer access-token-1"
                );
                Ok(response)
            })
            .await
            .expect("upgrade");

            let first = next_frame(&mut wire).await;
            assert!(first.get("previous_response_id").is_none());
            complete(&mut wire, "resp_oauth_1", json!([])).await;

            // Turn 2 must stay on the SAME socket even though the OAuth bearer token rotated.
            let second = next_frame(&mut wire).await;
            assert_eq!(second["previous_response_id"], "resp_oauth_1");
            complete(&mut wire, "resp_oauth_2", json!([])).await;
        });

        stream(&p, &session, vec![user("turn one")]).await;
        let events = stream(&p, &session, vec![user("turn one"), user("turn two")]).await;
        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::ResponseId(id) if id == "resp_oauth_2"))
        );
        assert_eq!(refreshes.load(Ordering::SeqCst), 2);
        server.await.expect("server assertions");
    }
}
