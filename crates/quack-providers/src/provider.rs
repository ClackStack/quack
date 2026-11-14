//! A provider ready to use: resolved secrets, one HTTP client, retries and streaming.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures::StreamExt;
use quack_config::provider::ReasoningFormat;
use quack_config::settings::NetworkSettings;
use quack_config::{Env, ProviderFile, expand_home};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "proto-anthropic")]
use crate::anthropic;
use crate::error::{ErrorKind, ProviderError};
#[cfg(feature = "proto-ollama")]
use crate::ollama;
use crate::openai_chat::{self, ChunkParser};
#[cfg(feature = "proto-openai-responses")]
use crate::responses;
use crate::sse::SseDecoder;
use crate::types::{ChatRequest, DiscoveredModel, Message, StreamEvent, fingerprint, merge};

/// Longest wait we honour from a `Retry-After` header.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

/// Whether this build has the adapter for `api` (`openai-chat`, `anthropic-messages`, ...).
pub fn supports(api: &str) -> bool {
    api == "openai-chat"
        || (cfg!(feature = "proto-ollama") && api == "ollama")
        || (cfg!(feature = "proto-anthropic") && api == "anthropic-messages")
        || (cfg!(feature = "proto-openai-responses") && api == "openai-responses")
}

/// Wire protocol of a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Api {
    OpenAiChat,
    #[cfg(feature = "proto-ollama")]
    Ollama,
    #[cfg(feature = "proto-anthropic")]
    Anthropic,
    #[cfg(feature = "proto-openai-responses")]
    Responses,
}

#[derive(Debug, Clone)]
pub struct Provider {
    pub id: String,
    /// Shared so a clone for streaming copies one word, not the whole file.
    pub file: Arc<ProviderFile>,
    api: Api,
    /// What the client is built from, to build a fresh one when a connection goes stale.
    spec: Arc<ClientSpec>,
    /// Shared by every clone, so a client replaced after a stall ([`Provider::renew_client`])
    /// serves them all.
    client: Arc<RwLock<Pooled>>,
    base_url: String,
    /// Shared by every clone, so a new key ([`Provider::take_key`]) reaches them all.
    /// The map under the lock is itself shared, so a request copies one `Arc`.
    headers: Arc<RwLock<Arc<HeaderMap>>>,
    dialect: openai_chat::Dialect,
    /// The file's `extra_body`, under each model's.
    extra: serde_json::Map<String, serde_json::Value>,
    #[cfg(feature = "proto-openai-responses")]
    route: responses::Route,
    #[cfg(feature = "auth-chatgpt")]
    login: Option<Arc<crate::login::Login>>,
}

impl Provider {
    /// Build the client and resolve secrets. May run `cmd` secrets, so call it from a blocking context.
    pub fn new(
        id: &str,
        file: ProviderFile,
        network: &NetworkSettings,
        env: Env<'_>,
    ) -> Result<Self, ProviderError> {
        let config = |msg: String| ProviderError::new(ErrorKind::Config, msg);
        let api = match file.api.as_str() {
            "openai-chat" => Api::OpenAiChat,
            #[cfg(feature = "proto-ollama")]
            "ollama" => Api::Ollama,
            #[cfg(not(feature = "proto-ollama"))]
            "ollama" => {
                return Err(config(
                    "this build has no `ollama` adapter; use `openai-chat` with `/v1`".into(),
                ));
            }
            #[cfg(feature = "proto-anthropic")]
            "anthropic-messages" => Api::Anthropic,
            #[cfg(feature = "proto-openai-responses")]
            "openai-responses" => Api::Responses,
            #[cfg(not(feature = "proto-anthropic"))]
            "anthropic-messages" => {
                return Err(config(
                    "this build has no `anthropic-messages` adapter; use `openai-chat` if the \
                     service offers it"
                        .into(),
                ));
            }
            #[cfg(not(feature = "proto-openai-responses"))]
            "openai-responses" => {
                return Err(config(
                    "this build has no `openai-responses` adapter; use `openai-chat` if the \
                     service offers it"
                        .into(),
                ));
            }
            other => return Err(config(format!("unknown api `{other}`"))),
        };

        let mut headers = HeaderMap::new();
        if let Some(key) = file.api_key.as_ref().filter(|_| file.auth.is_none()) {
            let key = key
                .resolve(env)
                .map_err(|e| config(format!("api_key: {e}")))?;
            // Anthropic's API takes the bare key in `x-api-key`.
            let anthropic = file.api == "anthropic-messages";
            let name = file.auth_header.as_deref().unwrap_or(if anthropic {
                "x-api-key"
            } else {
                "Authorization"
            });
            let value = match file.auth_scheme.as_deref() {
                None if anthropic && file.auth_header.is_none() => key,
                None => format!("Bearer {key}"),
                Some("") => key,
                Some(scheme) => format!("{scheme} {key}"),
            };
            insert_header(&mut headers, name, &value, true)?;
        }
        for (name, secret) in &file.headers {
            let value = secret
                .resolve(env)
                .map_err(|e| config(format!("header `{name}`: {e}")))?;
            insert_header(&mut headers, name, &value, true)?;
        }

        #[cfg(feature = "proto-anthropic")]
        if api == Api::Anthropic && !headers.contains_key("anthropic-version") {
            insert_header(&mut headers, "anthropic-version", anthropic::VERSION, false)?;
        }

        let spec = ClientSpec::resolve(&file, network, env)?;
        let client = spec.build()?;
        #[cfg(feature = "auth-chatgpt")]
        let mut login = None;
        if let Some(a) = &file.auth {
            match a.kind {
                #[cfg(feature = "auth-chatgpt")]
                quack_config::provider::AuthKind::Chatgpt => {
                    if api != Api::Responses {
                        return Err(config(
                            "a ChatGPT sign-in needs `api = \"openai-responses\"`".into(),
                        ));
                    }
                    login = Some(Arc::new(crate::login::Login::new(
                        id,
                        &a.issuer,
                        &file.base_url,
                        &quack_config::user_data_dir(env),
                        client.clone(),
                    )));
                }
                #[cfg(not(feature = "auth-chatgpt"))]
                quack_config::provider::AuthKind::Chatgpt => {
                    return Err(config(
                        "this build has no ChatGPT sign-in; use an API key provider".into(),
                    ));
                }
            }
        }
        // Only a ChatGPT sign-in gets this far with `[auth]`.
        #[cfg(feature = "proto-openai-responses")]
        let route = if file.auth.is_some() {
            responses::Route::ChatgptPlan
        } else {
            responses::Route::Api
        };
        let extra = serde_json::to_value(&file.extra_body)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        let base_url = match api {
            #[cfg(feature = "proto-ollama")]
            Api::Ollama => ollama::native_base(&file.base_url),
            _ => file.base_url.trim_end_matches('/'),
        }
        .to_owned();
        Ok(Self {
            id: id.to_owned(),
            api,
            dialect: openai_chat::Dialect::of(&base_url),
            base_url,
            file: Arc::new(file),
            spec: Arc::new(spec),
            client: Arc::new(RwLock::new(Pooled {
                generation: 0,
                client,
            })),
            headers: Arc::new(RwLock::new(Arc::new(headers))),
            extra,
            #[cfg(feature = "proto-openai-responses")]
            route,
            #[cfg(feature = "auth-chatgpt")]
            login,
        })
    }

    /// The sign-in behind this provider, for `quack login` and `quack logout`.
    #[cfg(feature = "auth-chatgpt")]
    pub fn login(&self) -> Option<&crate::login::Login> {
        self.login.as_deref()
    }

    /// Whether a saved subscription sign-in allows plan usage, without network access.
    pub fn subscription_ready(&self) -> bool {
        #[cfg(feature = "auth-chatgpt")]
        if let Some(login) = &self.login {
            return login
                .load()
                .is_some_and(|c| c.signed_in() && c.plan_granted());
        }
        false
    }

    /// Sign in through the configured issuer. The callback receives the browser URL.
    pub async fn sign_in(&self, open: impl FnOnce(&str)) -> Result<(), String> {
        #[cfg(feature = "auth-chatgpt")]
        if let Some(login) = &self.login {
            let done = login.sign_in(false, open).await?;
            return if done.plan {
                Ok(())
            } else {
                Err("The account did not allow plan usage. Sign in again and allow it.".into())
            };
        }
        let _ = open;
        Err("This provider has no supported subscription sign-in.".into())
    }

    /// Why this provider can't be used yet, without contacting it: a sign-in not done.
    pub fn not_ready(&self) -> Option<String> {
        #[cfg(feature = "auth-chatgpt")]
        if let Some(login) = &self.login
            && !login
                .load()
                .is_some_and(|c| c.signed_in() && c.plan_granted())
        {
            return Some(format!(
                "run `quack login {}` to use this provider",
                self.id
            ));
        }
        None
    }

    /// Send `fresh`'s headers from now on: it was built from this provider's file after its
    /// key changed. Every clone of this provider, in a running turn too, uses them.
    pub fn take_key(&self, fresh: &Provider) {
        let headers = fresh.headers();
        match self.headers.write() {
            Ok(mut h) => *h = headers,
            Err(poisoned) => *poisoned.into_inner() = headers,
        }
    }

    fn headers(&self) -> Arc<HeaderMap> {
        match self.headers.read() {
            Ok(h) => h.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// The current client, with its generation for [`Provider::renew_client`].
    fn client(&self) -> (u64, reqwest::Client) {
        let pooled = match self.client.read() {
            Ok(p) => p,
            Err(poisoned) => poisoned.into_inner(),
        };
        (pooled.generation, pooled.client.clone())
    }

    /// After a timeout or network error, replace the client, and with it every pooled
    /// connection. Dropping a request leaves its HTTP/2 connection in the pool, so without
    /// this a retry would go down the same dead connection; this is a browser's refresh.
    /// Several requests failing on one client replace it once.
    fn renew_client(&self, stale: u64, e: &ProviderError) {
        if !matches!(e.kind, ErrorKind::Timeout | ErrorKind::Network) {
            return;
        }
        let fresh = match self.spec.build() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(provider = %self.id, error = %e, "could not renew the HTTP client");
                return;
            }
        };
        let mut pooled = match self.client.write() {
            Ok(p) => p,
            Err(poisoned) => poisoned.into_inner(),
        };
        if pooled.generation == stale {
            tracing::debug!(provider = %self.id, "new HTTP client after a stalled request");
            pooled.generation += 1;
            pooled.client = fresh;
        }
    }

    /// The headers for one request, with a current token for a signed-in provider.
    async fn request_headers(&self) -> Result<RequestHeaders, ProviderError> {
        #[cfg(feature = "auth-chatgpt")]
        if let Some(login) = &self.login {
            let mut h = (*self.headers()).clone();
            let token = login.bearer().await?;
            insert_header(&mut h, "Authorization", &format!("Bearer {token}"), true)?;
            return Ok(RequestHeaders::Owned(h));
        }
        Ok(RequestHeaders::Shared(self.headers()))
    }

    fn has_login(&self) -> bool {
        #[cfg(feature = "auth-chatgpt")]
        {
            self.login.is_some()
        }
        #[cfg(not(feature = "auth-chatgpt"))]
        {
            false
        }
    }

    /// Whether the history should only ever grow: per-request text (the plan reminder) is
    /// then kept in it rather than sent once. Anthropic's signed thinking only verifies
    /// against the exact history it was made after.
    pub fn append_only(&self) -> bool {
        #[cfg(feature = "proto-anthropic")]
        if self.api == Api::Anthropic {
            return true;
        }
        false
    }

    /// Keep a sealed reply only for the provider, system prompt and tools that made it.
    /// OpenAI's encrypted reasoning is also tied to its model; Anthropic's API itself
    /// drops thinking the current model can't read.
    fn keeps(&self, s: &crate::Sealed, model: &str, fp: &str) -> bool {
        let model_ok = s.model == model || {
            #[cfg(feature = "proto-anthropic")]
            {
                self.api == Api::Anthropic
            }
            #[cfg(not(feature = "proto-anthropic"))]
            false
        };
        s.provider == self.id && s.fingerprint == fp && model_ok
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{path}", self.base_url)
    }

    /// Headers are a request's first byte; a dropped request never sends them. Servers send
    /// them after prefill, so each retry waits twice as long: a stall is retried quickly and a
    /// long prompt still gets time, up to the idle timeout.
    fn first_byte_timeout(&self, attempt: u32) -> Duration {
        let http = &self.file.http;
        let idle = http.timeout_idle_secs.max(1);
        let secs = match http.timeout_first_byte_secs {
            0 => idle,
            s => s.saturating_mul(1 << attempt.min(16)).min(idle),
        };
        Duration::from_secs(secs.max(1))
    }

    /// A models listing: the connect timeout plus time for the server to answer.
    fn discovery_timeout(&self) -> Duration {
        Duration::from_secs(self.file.http.timeout_connect_secs.max(1) + 20)
    }

    async fn get_json(&self, path: &str) -> Result<serde_json::Value, ProviderError> {
        self.get_json_at(self.url(path)).await
    }

    async fn get_json_at(&self, url: String) -> Result<serde_json::Value, ProviderError> {
        let (generation, client) = self.client();
        let resp = self
            .request_headers()
            .await?
            .apply(client.get(url))
            .timeout(self.discovery_timeout())
            .send()
            .await
            .map_err(|e| ProviderError::from_reqwest(&e))
            .inspect_err(|e| self.renew_client(generation, e))?;
        let resp = check_status(resp).await?;
        resp.json()
            .await
            .map_err(|e| ProviderError::from_reqwest(&e))
    }

    /// Query the models endpoint.
    pub async fn list_models(&self) -> Result<Vec<DiscoveredModel>, ProviderError> {
        match self.api {
            Api::OpenAiChat => {
                let body = self.get_json(openai_chat::MODELS_PATH).await?;
                let mut models = openai_chat::parse_models(&body);
                // llama-server serves one model with the window it was started with.
                if let [only] = models.as_mut_slice()
                    && openai_chat::is_llama_cpp(&body)
                {
                    let root = self.base_url.strip_suffix("/v1").unwrap_or(&self.base_url);
                    let props = format!("{root}/{}", openai_chat::LLAMA_CPP_PROPS_PATH);
                    if let Ok(p) = self.get_json_at(props).await
                        && let Some(n) = openai_chat::llama_cpp_window(&p)
                    {
                        only.context_window = Some(n);
                    }
                }
                Ok(models)
            }
            #[cfg(feature = "proto-openai-responses")]
            Api::Responses => Ok(openai_chat::parse_models(
                &self.get_json(openai_chat::MODELS_PATH).await?,
            )),
            #[cfg(feature = "proto-anthropic")]
            Api::Anthropic => Ok(openai_chat::parse_models(
                &self
                    .get_json(&anthropic::path(&self.base_url, "models"))
                    .await?,
            )),
            #[cfg(feature = "proto-ollama")]
            Api::Ollama => {
                let names = ollama::parse_tags(&self.get_json(ollama::TAGS_PATH).await?);
                // A local server: asking each model for its window is cheap.
                let shown =
                    futures::future::join_all(names.iter().map(|n| self.ollama_show(n))).await;
                Ok(names
                    .into_iter()
                    .zip(shown)
                    .map(|(name, trained)| ollama::discovered(name, trained))
                    .collect())
            }
        }
    }

    #[cfg(feature = "proto-ollama")]
    async fn ollama_show(&self, model: &str) -> Option<u32> {
        let resp = self
            .request_headers()
            .await
            .ok()?
            .apply(self.client().1.post(self.url(ollama::SHOW_PATH)))
            .timeout(self.discovery_timeout())
            .json(&serde_json::json!({ "model": model }))
            .send()
            .await
            .ok()?;
        let body: serde_json::Value = check_status(resp).await.ok()?.json().await.ok()?;
        ollama::parse_show(&body)
    }

    /// Start a streaming request. Events arrive on the returned channel; it closes after
    /// `Finish` (plus a trailing `Usage`), an error, or cancellation.
    pub fn stream(
        &self,
        req: ChatRequest,
        reasoning: ReasoningFormat,
        cancel: CancellationToken,
    ) -> mpsc::Receiver<Result<StreamEvent, ProviderError>> {
        let (tx, rx) = mpsc::channel(256);
        let this = self.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = cancel.cancelled() => {}
                () = this.run_stream(req, reasoning, &tx) => {}
            }
        });
        rx
    }

    async fn run_stream(
        &self,
        mut req: ChatRequest,
        reasoning: ReasoningFormat,
        tx: &mpsc::Sender<Result<StreamEvent, ProviderError>>,
    ) {
        let fp = fingerprint(&req);
        let model = req.model.clone();
        for m in &mut req.messages {
            if let Message::Assistant { sealed, .. } = m
                && sealed.as_ref().is_some_and(|s| !self.keeps(s, &model, &fp))
            {
                *sealed = None;
            }
        }
        if !self.extra.is_empty() {
            let mut extra = serde_json::Value::Object(self.extra.clone());
            merge(&mut extra, &req.extra_body);
            if let serde_json::Value::Object(m) = extra {
                req.extra_body = m;
            }
        }
        let (path, body, accept) = match self.api {
            Api::OpenAiChat => (
                openai_chat::CHAT_PATH.to_owned(),
                openai_chat::build_body(&req, self.dialect),
                "text/event-stream",
            ),
            #[cfg(feature = "proto-ollama")]
            Api::Ollama => (
                ollama::CHAT_PATH.to_owned(),
                ollama::build_body(&req),
                "application/x-ndjson",
            ),
            #[cfg(feature = "proto-anthropic")]
            Api::Anthropic => (
                anthropic::path(&self.base_url, "messages"),
                anthropic::build_body(&req),
                "text/event-stream",
            ),
            #[cfg(feature = "proto-openai-responses")]
            Api::Responses => (
                responses::RESPONSES_PATH.to_owned(),
                responses::build_body(&req, self.route),
                "text/event-stream",
            ),
        };
        let mut attempt = 0;
        // A plain header set never changes mid-turn, so every retry reuses the first clone;
        // a signed-in provider refreshes its token per attempt.
        let mut shared: Option<RequestHeaders> = None;
        let (generation, resp) = loop {
            let headers = match &mut shared {
                Some(h) => h.clone(),
                None => {
                    let h = match self.request_headers().await {
                        Ok(h) => h,
                        Err(e) => {
                            let _ = tx.send(Err(e)).await;
                            return;
                        }
                    };
                    if !self.has_login() {
                        shared = Some(h.clone());
                    }
                    h
                }
            };
            // Taken per attempt: a retry after a stall gets the fresh client.
            let (generation, client) = self.client();
            let send = headers
                .apply(
                    client
                        .post(self.url(&path))
                        .header(reqwest::header::ACCEPT, accept)
                        .json(&body),
                )
                .send();
            let first_byte = self.first_byte_timeout(attempt);
            let result = match tokio::time::timeout(first_byte, send).await {
                Ok(Ok(r)) => check_status(r).await,
                Ok(Err(e)) => Err(ProviderError::from_reqwest(&e)),
                Err(_) => Err(ProviderError::new(
                    ErrorKind::Timeout,
                    format!("no response in {}s", first_byte.as_secs()),
                )),
            };
            if let Err(e) = &result {
                self.renew_client(generation, e);
            }
            match result {
                Ok(r) => break (generation, r),
                Err(e) if e.retryable() && attempt < self.file.http.max_retries => {
                    let wait = backoff(attempt, &e);
                    tracing::warn!(provider = %self.id, attempt, ?wait, error = %e, "retrying");
                    attempt += 1;
                    tokio::time::sleep(wait).await;
                }
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    return;
                }
            }
        };

        // Once bytes flow we never retry: the user has already seen part of the answer.
        let mut wire = match self.api {
            Api::OpenAiChat => Wire::Sse(
                SseDecoder::default(),
                SseParser::Chat(ChunkParser::new(reasoning)),
            ),
            #[cfg(feature = "proto-anthropic")]
            Api::Anthropic => Wire::Sse(
                SseDecoder::default(),
                SseParser::Anthropic(anthropic::EventParser::default()),
            ),
            #[cfg(feature = "proto-openai-responses")]
            Api::Responses => Wire::Sse(
                SseDecoder::default(),
                SseParser::Responses(responses::EventParser::default()),
            ),
            #[cfg(feature = "proto-ollama")]
            Api::Ollama => Wire::Lines(
                ollama::LineDecoder::default(),
                ollama::LineParser::new(reasoning),
            ),
        };
        let mut bytes = resp.bytes_stream();
        // Reused across chunks, so a chunk costs no allocation of its own.
        let mut frames: Vec<String> = Vec::new();
        let mut parsed: Vec<Parsed> = Vec::new();
        while let Some(chunk) = bytes.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    // Not retried, but the next request shouldn't use this connection.
                    let e = ProviderError::from_reqwest(&e);
                    self.renew_client(generation, &e);
                    let _ = tx.send(Err(e)).await;
                    return;
                }
            };
            wire.feed(&chunk, &mut frames, &mut parsed);
            for parsed in parsed.drain(..) {
                if !forward(parsed, tx).await {
                    return;
                }
            }
        }
        for parsed in wire.finish() {
            if !forward(parsed, tx).await {
                return;
            }
        }
    }
}

/// The parser behind an SSE stream, per protocol.
enum SseParser {
    Chat(ChunkParser),
    #[cfg(feature = "proto-anthropic")]
    Anthropic(anthropic::EventParser),
    #[cfg(feature = "proto-openai-responses")]
    Responses(responses::EventParser),
}

impl SseParser {
    fn parse(&mut self, data: &str) -> Parsed {
        match self {
            Self::Chat(p) => p.parse(data),
            #[cfg(feature = "proto-anthropic")]
            Self::Anthropic(p) => p.parse(data),
            #[cfg(feature = "proto-openai-responses")]
            Self::Responses(p) => p.parse(data),
        }
    }

    fn finish(&mut self) -> Vec<StreamEvent> {
        match self {
            Self::Chat(p) => p.finish(),
            #[cfg(feature = "proto-anthropic")]
            Self::Anthropic(p) => p.finish(),
            #[cfg(feature = "proto-openai-responses")]
            Self::Responses(p) => p.finish(),
        }
    }
}

/// Framing plus parsing for each wire format.
enum Wire {
    Sse(SseDecoder, SseParser),
    #[cfg(feature = "proto-ollama")]
    Lines(ollama::LineDecoder, ollama::LineParser),
}

type Parsed = Result<Vec<StreamEvent>, ProviderError>;

/// One request's headers: the shared set, or a per-request one with a fresh token.
#[derive(Clone)]
enum RequestHeaders {
    Shared(Arc<HeaderMap>),
    /// Only a signed-in provider builds a per-request map.
    #[cfg_attr(not(feature = "auth-chatgpt"), allow(dead_code))]
    Owned(HeaderMap),
}

impl RequestHeaders {
    /// Apply to a request. Per-header iteration visits the same names in the same order
    /// as `RequestBuilder::headers` would, so the wire bytes are unchanged.
    fn apply(self, mut rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self {
            Self::Owned(m) => rb.headers(m),
            Self::Shared(m) => {
                for (name, value) in m.iter() {
                    rb = rb.header(name, value.clone());
                }
                rb
            }
        }
    }
}

impl Wire {
    fn feed(&mut self, bytes: &[u8], frames: &mut Vec<String>, out: &mut Vec<Parsed>) {
        match self {
            Self::Sse(d, p) => {
                d.feed(bytes, frames);
                out.extend(frames.drain(..).map(|x| p.parse(&x)));
            }
            #[cfg(feature = "proto-ollama")]
            Self::Lines(d, p) => {
                d.feed(bytes, frames);
                out.extend(frames.drain(..).map(|x| p.parse(&x)));
            }
        }
    }

    /// End of stream: trailing frames, then text the parser still holds.
    fn finish(&mut self) -> Vec<Parsed> {
        let mut out: Vec<Parsed> = Vec::new();
        let mut frames: Vec<String> = Vec::new();
        match self {
            Self::Sse(d, p) => {
                d.finish(&mut frames);
                out.extend(frames.drain(..).map(|x| p.parse(&x)));
            }
            #[cfg(feature = "proto-ollama")]
            Self::Lines(d, p) => {
                frames.extend(d.finish());
                out.extend(frames.drain(..).map(|x| p.parse(&x)));
            }
        }
        out.push(Ok(match self {
            Self::Sse(_, p) => p.finish(),
            #[cfg(feature = "proto-ollama")]
            Self::Lines(_, p) => p.finish(),
        }));
        out
    }
}

/// Send parsed events; false when the stream should stop.
async fn forward(
    parsed: Result<Vec<StreamEvent>, ProviderError>,
    tx: &mpsc::Sender<Result<StreamEvent, ProviderError>>,
) -> bool {
    match parsed {
        Ok(events) => {
            for ev in events {
                if tx.send(Ok(ev)).await.is_err() {
                    return false;
                }
            }
            true
        }
        Err(e) => {
            let _ = tx.send(Err(e)).await;
            false
        }
    }
}

async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response, ProviderError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let retry_after = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => format!("<body unreadable: {e}>"),
    };
    Err(ProviderError::from_status(
        status.as_u16(),
        retry_after,
        &body,
    ))
}

/// Exponential backoff with jitter: about 0.5s, 1s, 2s, … capped at 30s.
fn backoff(attempt: u32, e: &ProviderError) -> Duration {
    if let ErrorKind::RateLimited {
        retry_after: Some(d),
    } = e.kind
    {
        return d.min(MAX_RETRY_AFTER);
    }
    let base = 500u64.saturating_mul(1 << attempt.min(6)).min(30_000);
    let jitter = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_millis()))
        .unwrap_or(0)
        % (base / 4 + 1);
    Duration::from_millis(base + jitter)
}

fn insert_header(
    headers: &mut HeaderMap,
    name: &str,
    value: &str,
    sensitive: bool,
) -> Result<(), ProviderError> {
    let config = |msg: String| ProviderError::new(ErrorKind::Config, msg);
    let name = HeaderName::from_bytes(name.as_bytes())
        .map_err(|_| config(format!("invalid header name `{name}`")))?;
    let mut value = HeaderValue::from_str(value).map_err(|_| {
        config(format!(
            "header `{name}` has characters not allowed in HTTP"
        ))
    })?;
    value.set_sensitive(sensitive);
    headers.insert(name, value);
    Ok(())
}

/// Make ring the process-wide rustls [`CryptoProvider`](rustls::crypto::CryptoProvider).
/// reqwest is built with `rustls-no-provider`, so something must choose one; ring needs
/// no cmake C build when cross-compiling, unlike aws-lc-sys. Other clients built before
/// the first quack client (rmcp builds its own) get the same provider from the `ring`
/// feature on our `rustls` dependency.
fn install_ring() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        // An error means another provider was already installed; leave it in place.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// An HTTP client with the user's proxy and CA settings, for other HTTP services such as
/// remote MCP servers.
pub fn http_client(network: &NetworkSettings, env: Env<'_>) -> Result<reqwest::Client, String> {
    let file: ProviderFile =
        serde_json::from_value(serde_json::json!({ "base_url": "" })).map_err(|e| e.to_string())?;
    ClientSpec::resolve(&file, network, env)
        .and_then(|spec| spec.build())
        .map_err(|e| e.message)
}

/// Like [`http_client`], leaving redirects to the caller, who can check each destination.
pub(crate) fn unredirected_client(
    network: &NetworkSettings,
    env: Env<'_>,
) -> Result<reqwest::Client, String> {
    let file: ProviderFile =
        serde_json::from_value(serde_json::json!({ "base_url": "" })).map_err(|e| e.to_string())?;
    ClientSpec::resolve(&file, network, env)
        .map_err(|e| e.message)?
        .builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| format!("HTTP client: {e}"))
}

#[cfg(test)]
fn build_client(
    file: &ProviderFile,
    network: &NetworkSettings,
    env: Env<'_>,
) -> Result<reqwest::Client, ProviderError> {
    ClientSpec::resolve(file, network, env)?.build()
}

/// How often an HTTP/2 connection with nothing arriving is PINGed, and how long the answer
/// may take. A connection that died quietly (a VPN that reconnected, a local proxy that
/// keeps its socket open after the path behind it went) is then closed and leaves the
/// pool, instead of taking every request sent down it. TCP keepalive can't see this when
/// the socket's other end is a local proxy or VPN client that is still alive.
const H2_PING_INTERVAL: Duration = Duration::from_secs(20);
const H2_PING_TIMEOUT: Duration = Duration::from_secs(10);

/// The client in use and how many times it has been replaced.
#[derive(Debug)]
struct Pooled {
    generation: u64,
    client: reqwest::Client,
}

/// Everything a client is built from, read once from the provider file, the settings and
/// the environment (proxy, CA and key files), so a fresh client can be built mid-turn.
#[derive(Debug, Clone)]
struct ClientSpec {
    connect: Duration,
    idle: Duration,
    proxy: Option<reqwest::Proxy>,
    roots: Vec<reqwest::Certificate>,
    identity: Option<reqwest::Identity>,
    insecure: bool,
    http1_only: bool,
    network: NetworkSettings,
    /// HTTP/2 without TLS, so tests can reach a plain mock server over HTTP/2.
    #[cfg(test)]
    h2c: bool,
}

impl ClientSpec {
    fn resolve(
        file: &ProviderFile,
        network: &NetworkSettings,
        env: Env<'_>,
    ) -> Result<Self, ProviderError> {
        let config = |msg: String| ProviderError::new(ErrorKind::Config, msg);
        let read = |label: &str, path: &str| {
            std::fs::read(expand_home(path, env))
                .map_err(|e| config(format!("{label} `{path}`: {e}")))
        };
        let http = &file.http;
        let env_str = |key: &str| {
            env(key)
                .or_else(|| std::env::var_os(key))
                .and_then(|v| v.into_string().ok())
                .filter(|s| !s.is_empty())
        };

        let proxy = Some(http.proxy.as_str())
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .or_else(|| Some(network.proxy.clone()).filter(|p| !p.is_empty()))
            .or_else(|| {
                [
                    "HTTPS_PROXY",
                    "https_proxy",
                    "HTTP_PROXY",
                    "http_proxy",
                    "ALL_PROXY",
                    "all_proxy",
                ]
                .into_iter()
                .find_map(env_str)
            });
        // A proxy set here replaces reqwest's own lookup, which would have read NO_PROXY:
        // read it ourselves, or hosts meant to go direct (a VPN's internal ones) go through
        // the proxy.
        let no_proxy = ["NO_PROXY", "no_proxy"]
            .into_iter()
            .find_map(env_str)
            .and_then(|s| reqwest::NoProxy::from_string(&s));
        let proxy = match proxy {
            Some(p) => Some(
                reqwest::Proxy::all(&p)
                    .map_err(|e| config(format!("proxy `{p}`: {e}")))?
                    .no_proxy(no_proxy),
            ),
            None => None,
        };

        let mut roots = Vec::new();
        let mut ca_files = Vec::new();
        if !network.ca_bundle.is_empty() {
            ca_files.push(network.ca_bundle.clone());
        }
        if let Some(ca) = file.tls.ca_file.as_deref().filter(|s| !s.is_empty()) {
            ca_files.push(ca.to_owned());
        }
        if ca_files.is_empty() {
            ca_files.extend(
                ["SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE"]
                    .into_iter()
                    .find_map(env_str),
            );
        }
        for ca in &ca_files {
            let pem = read("CA file", ca)?;
            roots.extend(
                reqwest::Certificate::from_pem_bundle(&pem)
                    .map_err(|e| config(format!("CA file `{ca}`: {e}")))?,
            );
        }
        let identity = match (&file.tls.client_cert, &file.tls.client_key) {
            (Some(cert), Some(key)) => {
                let mut pem = read("client_cert", cert)?;
                pem.push(b'\n');
                pem.extend(read("client_key", key)?);
                Some(
                    reqwest::Identity::from_pem(&pem)
                        .map_err(|e| config(format!("client certificate: {e}")))?,
                )
            }
            (None, None) => None,
            _ => {
                return Err(config(
                    "tls.client_cert and tls.client_key must be set together".into(),
                ));
            }
        };
        if file.tls.insecure_skip_verify {
            tracing::warn!(provider = ?file.id, "TLS certificate verification is disabled");
        }
        Ok(Self {
            connect: Duration::from_secs(http.timeout_connect_secs.max(1)),
            idle: Duration::from_secs(http.timeout_idle_secs.max(1)),
            proxy,
            roots,
            identity,
            insecure: file.tls.insecure_skip_verify,
            http1_only: network.http1_only,
            network: network.clone(),
            #[cfg(test)]
            h2c: false,
        })
    }

    fn builder(&self) -> reqwest::ClientBuilder {
        install_ring();
        let mut b = reqwest::Client::builder()
            .user_agent(concat!("quack/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(self.connect)
            .read_timeout(self.idle)
            .http2_keep_alive_interval(H2_PING_INTERVAL)
            .http2_keep_alive_timeout(H2_PING_TIMEOUT)
            .http2_keep_alive_while_idle(true);
        if self.http1_only {
            b = b.http1_only();
        }
        #[cfg(test)]
        if self.h2c {
            b = b.http2_prior_knowledge();
        }
        if let Some(p) = &self.proxy {
            b = b.proxy(p.clone());
        }
        if !self.roots.is_empty() {
            b = b.tls_certs_merge(self.roots.clone());
        }
        if let Some(id) = &self.identity {
            b = b.identity(id.clone());
        }
        if self.insecure {
            b = b.tls_danger_accept_invalid_certs(true);
        }
        let gate = self.network.clone();
        b.redirect(reqwest::redirect::Policy::custom(move |attempt| {
            gate_redirect(attempt, &gate)
        }))
    }

    fn build(&self) -> Result<reqwest::Client, ProviderError> {
        self.builder()
            .build()
            .map_err(|e| ProviderError::new(ErrorKind::Config, format!("HTTP client: {e}")))
    }
}

/// Every client this crate builds follows redirects only within the host the chain
/// started from; any other destination must pass the egress gate, so a redirect can
/// never lead somewhere the gate would refuse. A custom policy guards its own hop
/// count; reqwest still refuses loops and strips credentials on cross-origin hops.
fn gate_redirect(
    attempt: reqwest::redirect::Attempt<'_>,
    network: &NetworkSettings,
) -> reqwest::redirect::Action {
    const MAX_HOPS: usize = 10;
    if attempt.previous().len() > MAX_HOPS {
        return attempt.error("too many redirects");
    }
    let target = attempt.url();
    let scheme = target.scheme().to_owned();
    let host = target.host_str().unwrap_or_default().to_owned();
    let started_on = attempt
        .previous()
        .first()
        .and_then(reqwest::Url::host_str)
        .unwrap_or_default()
        .to_owned();
    let url = target.as_str().to_owned();
    if !matches!(scheme.as_str(), "http" | "https") {
        return attempt.error(format!("refused a redirect to the `{scheme}` scheme"));
    }
    if host == started_on {
        return attempt.follow();
    }
    match network.check_egress(&url) {
        Ok(()) => attempt.follow(),
        Err(why) => attempt.error(why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::OsString;

    #[test]
    fn proxy_and_ca_bundle_from_env() {
        let file: ProviderFile =
            serde_json::from_value(serde_json::json!({ "base_url": "https://api.example.com" }))
                .expect("provider file");
        let network = NetworkSettings::default();

        // Default empty env succeeds.
        let empty_env: HashMap<String, OsString> = HashMap::new();
        assert!(build_client(&file, &network, &|k| empty_env.get(k).cloned()).is_ok());

        // Environment proxy is picked up.
        let mut proxy_env: HashMap<String, OsString> = HashMap::new();
        proxy_env.insert("HTTPS_PROXY".into(), "http://127.0.0.1:8080".into());
        assert!(build_client(&file, &network, &|k| proxy_env.get(k).cloned()).is_ok());

        // Invalid proxy URL errors clearly.
        let mut bad_proxy_env: HashMap<String, OsString> = HashMap::new();
        bad_proxy_env.insert("HTTPS_PROXY".into(), "not a valid uri".into());
        let err = build_client(&file, &network, &|k| bad_proxy_env.get(k).cloned())
            .expect_err("invalid proxy must fail");
        assert!(err.message.contains("proxy `not a valid uri`"), "{err:?}");

        // CA bundle in environment pointing to non-existent file errors clearly.
        let mut ca_env: HashMap<String, OsString> = HashMap::new();
        ca_env.insert("SSL_CERT_FILE".into(), "/nonexistent/ca.pem".into());
        let err = build_client(&file, &network, &|k| ca_env.get(k).cloned())
            .expect_err("missing ca must fail");
        assert!(
            err.message.contains("CA file `/nonexistent/ca.pem`"),
            "{err:?}"
        );
    }

    /// A clone made before the key changed (a running turn's) sends the new one.
    #[tokio::test]
    async fn a_new_key_reaches_every_clone() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(header("authorization", "Bearer new"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "data": [{ "id": "tiny" }] })),
            )
            .mount(&server)
            .await;
        let build = |key: &str| {
            let file: ProviderFile = serde_json::from_value(
                serde_json::json!({ "base_url": server.uri(), "api_key": key }),
            )
            .expect("provider file");
            Provider::new("mock", file, &NetworkSettings::default(), &|_| None).expect("provider")
        };
        let session = build("old");
        let turn = session.clone();
        assert!(turn.list_models().await.is_err(), "the old key is refused");
        session.take_key(&build("new"));
        let models = turn.list_models().await.expect("the new key is sent");
        assert_eq!(models[0].id, "tiny");
    }

    const SSE: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";

    async fn sse_server() -> wiremock::MockServer {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(SSE),
            )
            .mount(&server)
            .await;
        server
    }

    async fn reply(p: &Provider) -> Result<String, ProviderError> {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![Message::User {
                content: "hi".into(),
                images: Vec::new(),
            }],
            ..ChatRequest::default()
        };
        let mut rx = p.stream(req, ReasoningFormat::Auto, CancellationToken::new());
        let mut b = crate::ResponseBuilder::default();
        while let Some(ev) = rx.recv().await {
            b.push(&ev?);
        }
        match b.into_message() {
            Message::Assistant { content, .. } => Ok(content),
            other => panic!("not an assistant message: {other:?}"),
        }
    }

    /// A TCP relay to `upstream` whose first connection can go dead while both of its
    /// sockets stay open, as behind a VPN client or local proxy whose path has gone.
    /// Returns the relay's address, the switch, and the count of connections accepted.
    async fn relay(
        upstream: std::net::SocketAddr,
    ) -> (
        std::net::SocketAddr,
        Arc<std::sync::atomic::AtomicBool>,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        async fn pipe(
            mut from: tokio::net::tcp::OwnedReadHalf,
            mut to: tokio::net::tcp::OwnedWriteHalf,
            dead: Option<Arc<AtomicBool>>,
        ) {
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                let n = match from.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                if dead.as_ref().is_some_and(|d| d.load(Ordering::SeqCst)) {
                    continue;
                }
                if to.write_all(&buf[..n]).await.is_err() {
                    return;
                }
            }
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind relay");
        let addr = listener.local_addr().expect("relay address");
        let dead = Arc::new(AtomicBool::new(false));
        let accepted = Arc::new(AtomicUsize::new(0));
        let (switch, count) = (dead.clone(), accepted.clone());
        tokio::spawn(async move {
            while let Ok((inbound, _)) = listener.accept().await {
                let first = count.fetch_add(1, Ordering::SeqCst) == 0;
                let Ok(outbound) = tokio::net::TcpStream::connect(upstream).await else {
                    return;
                };
                let gate = first.then(|| switch.clone());
                let (ri, wi) = inbound.into_split();
                let (ro, wo) = outbound.into_split();
                tokio::spawn(pipe(ri, wo, gate.clone()));
                tokio::spawn(pipe(ro, wi, gate));
            }
        });
        (addr, dead, accepted)
    }

    /// Over HTTP/2 every request shares one connection. When it goes dead without closing,
    /// the retry after the first-byte timeout must not go down it again.
    #[tokio::test]
    async fn a_retry_leaves_a_dead_http2_connection_behind() {
        use std::sync::atomic::Ordering;
        let server = sse_server().await;
        let (addr, dead, accepted) = relay(*server.address()).await;
        let file: ProviderFile = toml::from_str(&format!(
            "base_url = \"http://{addr}/v1\"\nmodels = [\"m\"]\n\
             [http]\ntimeout_idle_secs = 30\ntimeout_first_byte_secs = 1\nmax_retries = 1\n"
        ))
        .expect("provider toml");
        let mut p =
            Provider::new("mock", file, &NetworkSettings::default(), &|_| None).expect("provider");
        let mut spec = (*p.spec).clone();
        spec.h2c = true;
        p.client = Arc::new(RwLock::new(Pooled {
            generation: 0,
            client: spec.build().expect("h2c client"),
        }));
        p.spec = Arc::new(spec);

        assert_eq!(reply(&p).await.expect("first reply"), "Hello");
        assert_eq!(accepted.load(Ordering::SeqCst), 1);
        dead.store(true, Ordering::SeqCst);
        let started = std::time::Instant::now();
        assert_eq!(reply(&p).await.expect("the retry answers"), "Hello");
        assert_eq!(
            accepted.load(Ordering::SeqCst),
            2,
            "the retry opened a new connection"
        );
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    /// A proxy from the environment skips the hosts in `NO_PROXY`.
    #[tokio::test]
    async fn no_proxy_hosts_go_direct() {
        let server = sse_server().await;
        let file: ProviderFile = toml::from_str(&format!(
            "base_url = \"{}/v1\"\nmodels = [\"m\"]\n[http]\nmax_retries = 0\n",
            server.uri()
        ))
        .expect("provider toml");
        // Nothing listens on port 1, so a request through the proxy fails at once. An empty
        // value hides the variable from the real environment.
        let build = |no_proxy: &str| {
            let vars: HashMap<&str, OsString> = [
                ("HTTPS_PROXY", "http://127.0.0.1:1"),
                ("NO_PROXY", no_proxy),
                ("no_proxy", ""),
            ]
            .into_iter()
            .map(|(k, v)| (k, OsString::from(v)))
            .collect();
            Provider::new("mock", file.clone(), &NetworkSettings::default(), &|k| {
                vars.get(k).cloned()
            })
            .expect("provider")
        };
        assert!(reply(&build("")).await.is_err(), "the proxy is used");
        let direct = build("localhost,127.0.0.1");
        assert_eq!(reply(&direct).await.expect("goes direct"), "Hello");
    }
}
