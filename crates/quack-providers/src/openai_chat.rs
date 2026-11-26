//! `api = "openai-chat"`: `/chat/completions` with SSE streaming.
//! Covers vLLM, SGLang, llama.cpp, LM Studio, TGI, Ollama `/v1` and most hosted services.

use std::collections::{HashMap, HashSet};

use quack_config::provider::{
    CacheControlMode, Cost, ReasoningFormat, ReasoningHistory, ReasoningWire, ToolCallMode,
};
use quack_protocol::Usage;
use serde_json::{Map, Value, json};

use crate::error::{ErrorKind, ProviderError, body_message};
use crate::reasoning::{Piece, ThinkSplitter};
use crate::types::{
    ChatRequest, DiscoveredModel, FinishReason, Message, StreamEvent, ToolSpec,
    cache_control_object, merge, split_session_context,
};

pub(crate) const CHAT_PATH: &str = "chat/completions";
pub(crate) const MODELS_PATH: &str = "models";

/// The `tools` array both the openai-chat and ollama `/api/chat` bodies use.
pub(crate) fn openai_tools_json(tools: &[ToolSpec]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                }
            })
        })
        .collect()
}

/// How a server's `/chat/completions` differs from the common shape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Dialect {
    #[default]
    Standard,
    /// Mistral's API: it rejects fields it doesn't know, takes `reasoning_effort` as
    /// `none` or `high`, and wants earlier reasoning sent back as `thinking` parts.
    Mistral,
    /// Google Gemini's OpenAI-compatible API: `/models` returns `models/<id>` names and
    /// tool calls carry `extra_content.google.thought_signature` that must come back on
    /// the next turn (or the documented skip sentinel after `/resume` or compaction).
    Google,
    /// Cohere's OpenAI-compatible API: it rejects `stream_options`.
    Cohere,
}

impl Dialect {
    pub(crate) fn of(base_url: &str) -> Self {
        let host = base_url
            .split("://")
            .nth(1)
            .unwrap_or(base_url)
            .split(['/', ':'])
            .next()
            .unwrap_or_default();
        if host.eq_ignore_ascii_case("api.mistral.ai") {
            Self::Mistral
        } else if host.eq_ignore_ascii_case("generativelanguage.googleapis.com") {
            Self::Google
        } else if host.eq_ignore_ascii_case("api.cohere.ai")
            || host.eq_ignore_ascii_case("api.cohere.com")
        {
            Self::Cohere
        } else {
            Self::Standard
        }
    }
}

/// How messages are written for one request.
struct Shape<'a> {
    mistral: bool,
    google: bool,
    /// Tool-call ids as Mistral's tokenizer takes them: exactly 9 letters or digits.
    short_ids: bool,
    /// Tool names by call id, for tool results.
    names: HashMap<&'a str, &'a str>,
}

/// An image as an OpenAI-compatible content part.
fn image_part(img: &quack_protocol::Image) -> Value {
    json!({ "type": "image_url", "image_url": { "url": img.data_url() } })
}

/// Images from tool results ride in a user message after the run of tool messages:
/// many chat-completions servers take image parts only on user content, and a tool
/// message's `content` must stay a string. Mistral is the exception (`message_json`).
fn flush_images(messages: &mut Vec<Value>, pending: &mut Vec<Value>) {
    if !pending.is_empty() {
        messages.push(json!({
            "role": "user",
            "content": Value::Array(std::mem::take(pending)),
        }));
    }
}

/// Prompt block describing tools for `ToolCallMode::Text` (when the wire `tools` array is
/// omitted because the server or chat template lacks native tool calling).
pub(crate) fn text_tools_prompt(tools: &[ToolSpec]) -> String {
    let mut out = String::from(
        "\n\n<tools>\nTo call a tool, output a `<tool_call>` block with JSON containing `name` and `arguments`:\n\
         <tool_call>{\"name\": \"tool_name\", \"arguments\": {...}}</tool_call>\n\n\
         Available tools:\n",
    );
    for t in tools {
        out.push_str(&format!(
            "- {}: {}\n  parameters: {}\n",
            t.name, t.description, t.parameters
        ));
    }
    out.push_str("</tools>");
    out
}

/// JSON Schema response_format for `ToolCallMode::Grammar`: constrains structured output to
/// `{ content, tool_calls }` naming only offered tools.
fn grammar_response_format(tools: &[ToolSpec]) -> Value {
    if tools.is_empty() {
        return json!({ "type": "json_object" });
    }
    let names: Vec<Value> = tools.iter().map(|t| json!(t.name)).collect();
    json!({
        "type": "json_schema",
        "json_schema": {
            "name": "assistant_turn",
            "schema": {
                "type": "object",
                "properties": {
                    "content": { "type": "string" },
                    "tool_calls": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string", "enum": names },
                                "arguments": { "type": "object" }
                            },
                            "required": ["name", "arguments"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["content"],
                "additionalProperties": false
            }
        }
    })
}

/// Whether earlier reasoning should be echoed on message `index`, given the last user turn.
pub(crate) fn should_echo_reasoning(
    mode: Option<ReasoningHistory>,
    effort: Option<&str>,
    index: usize,
    last_user: Option<usize>,
) -> bool {
    if effort == Some("none") {
        return false;
    }
    match mode {
        Some(ReasoningHistory::Never) => false,
        Some(ReasoningHistory::CurrentTurn) => last_user.is_none_or(|u| index > u),
        Some(ReasoningHistory::AllTurns) | None => true,
    }
}

/// Serialize a request. Earlier reasoning is sent back according to `preserve_in_history`
/// (`all_turns` by default, `current_turn` to keep reasoning only within the active
/// tool-call chain, or `never` / `false` to disable).
pub(crate) fn build_body(req: &ChatRequest, dialect: Dialect) -> Value {
    let mistral = dialect == Dialect::Mistral;
    let effort = req
        .reasoning_effort
        .as_deref()
        .map(|e| if mistral { mistral_effort(e) } else { e });
    let last_user = req
        .messages
        .iter()
        .rposition(|m| matches!(m, Message::User { .. } | Message::Compaction { .. }));
    let text_tool_mode = req.tool_mode == Some(ToolCallMode::Text) && !req.tools.is_empty();
    let shape = Shape {
        mistral,
        google: dialect == Dialect::Google,
        short_ids: mistral || is_mistral_large(&req.model),
        names: req
            .messages
            .iter()
            .flat_map(|m| match m {
                Message::Assistant { tool_calls, .. } => tool_calls.as_slice(),
                _ => &[],
            })
            .map(|c| (c.id.as_str(), c.name.as_str()))
            .collect(),
    };
    let mut messages: Vec<Value> = Vec::with_capacity(req.messages.len() + 1);
    let mut pending: Vec<Value> = Vec::new();
    for (idx, m) in req.messages.iter().enumerate() {
        let echo = should_echo_reasoning(req.preserve_reasoning, effort, idx, last_user);
        match m {
            // Mistral takes image chunks in the tool message itself (`message_json`).
            Message::Tool { images, .. } if !images.is_empty() && !shape.mistral => {
                // The text goes back as the tool result; the pixels follow as user parts.
                messages.push(message_json(m, &shape, echo));
                pending.extend(images.iter().map(image_part));
            }
            Message::Tool { .. } => {
                // A run of tool results stays contiguous; the pixels wait for its end.
                messages.push(message_json(m, &shape, echo));
            }
            m => {
                flush_images(&mut messages, &mut pending);
                // Mistral rejects a user or system message right after a tool result
                // (an interrupted turn leaves one), so an empty reply goes between.
                if shape.mistral
                    && !matches!(m, Message::Assistant { .. })
                    && messages.last().is_some_and(|v| v["role"] == "tool")
                {
                    messages.push(json!({ "role": "assistant", "content": "" }));
                }
                messages.push(message_json(m, &shape, echo));
            }
        }
    }
    flush_images(&mut messages, &mut pending);
    if text_tool_mode {
        let guide = text_tools_prompt(&req.tools);
        if let Some(sys) = messages
            .first_mut()
            .filter(|m| m["role"] == "system")
            .and_then(|m| m["content"].as_str().map(str::to_owned))
        {
            messages[0]["content"] = json!(format!("{sys}{guide}"));
        } else {
            messages.insert(
                0,
                json!({ "role": "system", "content": guide.trim_start() }),
            );
        }
    }
    if req.prompt_cache.cache_control == Some(CacheControlMode::Breakpoints) {
        apply_cache_control_breakpoints(&mut messages, req.prompt_cache.ttl.as_deref());
    }
    let mut body = Map::new();
    body.insert("model".into(), json!(req.model));
    body.insert("messages".into(), Value::Array(messages));
    body.insert("stream".into(), json!(true));
    match dialect {
        Dialect::Mistral => {
            // Prefer the session or shared cache key so routing survives compaction and
            // parallel CI runs; fall back to hashing the opening messages when none is set.
            let key = req
                .cache_key
                .as_deref()
                .filter(|k| !k.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| cache_key(&req.messages));
            body.insert("prompt_cache_key".into(), json!(key));
        }
        Dialect::Cohere => {}
        Dialect::Standard | Dialect::Google => {
            body.insert("stream_options".into(), json!({ "include_usage": true }));
        }
    }
    if req.prompt_cache.cache_key
        && !body.contains_key("prompt_cache_key")
        && let Some(key) = req.cache_key.as_deref().filter(|k| !k.is_empty())
    {
        body.insert("prompt_cache_key".into(), json!(key));
    }
    if req.prompt_cache.cache_control == Some(CacheControlMode::Auto) {
        body.insert(
            "cache_control".into(),
            cache_control_object(req.prompt_cache.ttl.as_deref()),
        );
    } else if req.prompt_cache.cache_control.is_none()
        && matches!(dialect, Dialect::Standard)
        && let Some(ttl) = req
            .prompt_cache
            .ttl
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
    {
        body.insert(
            "prompt_cache_options".into(),
            json!({ "mode": "implicit", "ttl": ttl }),
        );
    }
    if !req.tools.is_empty() && !text_tool_mode {
        let tools = openai_tools_json(&req.tools);
        body.insert("tools".into(), Value::Array(tools));
        if req.parallel_tool_calls == Some(false) && matches!(dialect, Dialect::Standard) {
            body.insert("parallel_tool_calls".into(), json!(false));
        }
    }
    if let Some(n) = req.max_tokens {
        body.insert("max_tokens".into(), json!(n));
    }
    let s = &req.sampling;
    // Hosted APIs with strict schemas reject sampling settings they don't have.
    let local = |v: Option<Value>| v.filter(|_| matches!(dialect, Dialect::Standard));
    let wire_effort = match req.reasoning_wire {
        ReasoningWire::Effort => effort,
        ReasoningWire::EnableThinking | ReasoningWire::None => None,
    };
    let optional = [
        ("temperature", s.temperature.map(Value::from)),
        ("top_p", s.top_p.map(Value::from)),
        ("top_k", local(s.top_k.map(Value::from))),
        ("min_p", local(s.min_p.map(Value::from))),
        (
            "repetition_penalty",
            local(s.repetition_penalty.map(Value::from)),
        ),
        ("reasoning_effort", wire_effort.map(Value::from)),
    ];
    for (k, v) in optional {
        if let Some(v) = v {
            body.insert(k.into(), v);
        }
    }
    if req.tool_mode == Some(ToolCallMode::Grammar) {
        body.insert(
            "response_format".into(),
            grammar_response_format(&req.tools),
        );
    }
    let mut body = Value::Object(body);
    merge(&mut body, &req.extra_body);
    match req.reasoning_wire {
        ReasoningWire::Effort => {
            if let Some(effort) = effort {
                body["reasoning_effort"] = json!(effort);
            }
        }
        ReasoningWire::EnableThinking => {
            if let Some(effort) = effort {
                if !body["chat_template_kwargs"].is_object() {
                    body["chat_template_kwargs"] = json!({});
                }
                body["chat_template_kwargs"]["enable_thinking"] =
                    json!(!matches!(effort, "none" | "off"));
            }
        }
        ReasoningWire::None => {}
    }
    body
}

/// Attach Anthropic-style `cache_control` breakpoints (at most 4) to `openai-chat`
/// system and user content blocks for servers such as Alibaba DashScope and OpenRouter.
/// Leaves `role: "tool"` content as plain strings for strict OpenAI-compatible servers.
fn apply_cache_control_breakpoints(messages: &mut [Value], ttl: Option<&str>) {
    let cc = cache_control_object(ttl);
    let mut remaining: usize = 4;

    let mark_part = |msg: &mut Value, cc: &Value| -> bool {
        if let Some(parts) = msg.get_mut("content").and_then(Value::as_array_mut) {
            if let Some(last) = parts.last_mut()
                && last.get("cache_control").is_none()
            {
                last["cache_control"] = cc.clone();
                return true;
            }
            return false;
        }
        if let Some(text) = msg.get("content").and_then(Value::as_str)
            && !text.is_empty()
        {
            let text = text.to_owned();
            msg["content"] = json!([{
                "type": "text",
                "text": text,
                "cache_control": cc.clone(),
            }]);
            return true;
        }
        false
    };

    if let Some(first) = messages.first_mut()
        && first["role"] == "system"
        && remaining > 0
        && mark_part(first, &cc)
    {
        remaining -= 1;
    }

    if let Some(idx) = messages.iter().position(|m| m["role"] == "user")
        && remaining > 0
        && let Some(text) = messages[idx]["content"].as_str()
        && let Some((ctx, prompt)) = split_session_context(text)
    {
        let (ctx, prompt) = (ctx.to_owned(), prompt.to_owned());
        messages[idx]["content"] = json!([
            { "type": "text", "text": ctx, "cache_control": cc.clone() },
            { "type": "text", "text": prompt },
        ]);
        remaining -= 1;
    }

    let user_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter_map(|(i, m)| (m["role"] == "user").then_some(i))
        .collect();
    for idx in user_indices.into_iter().rev().take(2) {
        if remaining == 0 {
            break;
        }
        if mark_part(&mut messages[idx], &cc) {
            remaining -= 1;
        }
    }
}

/// Mistral takes `none` or `high`, as Mistral's own harness maps them.
fn mistral_effort(effort: &str) -> &'static str {
    match effort {
        "none" | "off" | "minimal" | "low" => "none",
        _ => "high",
    }
}

/// Mistral Large, whose tokenizer takes only 9-character tool-call ids, on any server.
fn is_mistral_large(model: &str) -> bool {
    model.to_ascii_lowercase().contains("mistral-large")
}

/// An id Mistral accepts: kept when it already is one, otherwise derived from it, so the
/// call and its result still match.
fn short_id(id: &str) -> String {
    if id.len() == 9 && id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return id.to_owned();
    }
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut n = fnv1a(id.as_bytes());
    (0..9)
        .map(|_| {
            let c = DIGITS[(n % 62) as usize] as char;
            n /= 62;
            c
        })
        .collect()
}

/// The same for every request of a session: the system prompt and the first user message.
fn cache_key(messages: &[Message]) -> String {
    let mut bytes = Vec::new();
    for m in messages.iter().take(2) {
        if let Message::System { content } | Message::User { content, .. } = m {
            bytes.extend_from_slice(content.as_bytes());
            bytes.push(0);
        }
    }
    format!("ch-{:016x}", fnv1a(&bytes))
}

/// FNV-1a: stable across builds, unlike `std`'s hasher.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn message_json(m: &Message, shape: &Shape<'_>, echo_reasoning: bool) -> Value {
    let id = |id: &str| {
        if shape.short_ids {
            short_id(id)
        } else {
            id.to_owned()
        }
    };
    match m {
        Message::System { content } => json!({ "role": "system", "content": content }),
        Message::Compaction {
            content,
            session_context,
            ..
        } => {
            let text = match session_context.as_deref().filter(|c| !c.is_empty()) {
                Some(ctx) if content.is_empty() => ctx.to_owned(),
                Some(ctx) => format!("{ctx}\n\n{content}"),
                None => content.clone(),
            };
            json!({ "role": "user", "content": text })
        }
        Message::Nudge { content } => json!({ "role": "user", "content": content }),
        Message::User { content, images } => {
            if images.is_empty() {
                json!({ "role": "user", "content": content })
            } else {
                let mut parts: Vec<Value> = images.iter().map(image_part).collect();
                if !content.is_empty() {
                    parts.push(json!({ "type": "text", "text": content }));
                }
                json!({ "role": "user", "content": parts })
            }
        }
        Message::Assistant {
            content,
            reasoning,
            tool_calls,
            sealed,
        } => {
            let mut v = json!({ "role": "assistant", "content": content });
            let echo = reasoning
                .as_deref()
                .filter(|r| echo_reasoning && !r.is_empty());
            if let Some(r) = echo.filter(|_| !shape.mistral) {
                v["reasoning_content"] = json!(r);
            }
            if let Some(r) = echo.filter(|_| shape.mistral) {
                let mut parts = vec![json!({
                    "type": "thinking",
                    "thinking": [{ "type": "text", "text": r }],
                })];
                if !content.is_empty() {
                    parts.push(json!({ "type": "text", "text": content }));
                }
                v["content"] = Value::Array(parts);
            }
            if !tool_calls.is_empty() {
                // Some servers reject an empty string next to tool calls.
                if content.is_empty() && !shape.mistral {
                    v["content"] = Value::Null;
                }
                let extra_by_pos = sealed_tool_extra(sealed.as_ref());
                v["tool_calls"] = tool_calls
                    .iter()
                    .enumerate()
                    .map(|(pos, c)| {
                        let mut call = json!({
                            "id": id(&c.id),
                            "type": "function",
                            "function": { "name": c.name, "arguments": c.arguments },
                        });
                        if let Some(extra) = extra_by_pos.get(&pos) {
                            call["extra_content"] = (*extra).clone();
                        } else if shape.google && pos == 0 && extra_by_pos.is_empty() {
                            // Gemini rejects prior tool calls after `/resume` or compaction unless a
                            // thought signature or its documented bypass sentinel is attached.
                            call["extra_content"] = json!({
                                "google": { "thought_signature": "skip_thought_signature_validator" }
                            });
                        }
                        call
                    })
                    .collect();
            }
            v
        }
        Message::Tool {
            call_id,
            content,
            images,
        } => {
            let mut v = json!({ "role": "tool", "tool_call_id": id(call_id), "content": content });
            // Elsewhere the pixels follow the run of tool results (`build_body`).
            if shape.mistral && !images.is_empty() {
                let mut parts = vec![json!({ "type": "text", "text": content })];
                parts.extend(images.iter().map(image_part));
                v["content"] = Value::Array(parts);
            }
            if shape.mistral
                && let Some(name) = shape.names.get(call_id.as_str())
            {
                v["name"] = json!(name);
            }
            v
        }
    }
}

/// Opaque per-tool-call `extra_content` saved in [`crate::types::Sealed`] by call index.
fn sealed_tool_extra(sealed: Option<&crate::types::Sealed>) -> HashMap<usize, &Value> {
    let Some(sealed) = sealed else {
        return HashMap::new();
    };
    sealed
        .items
        .iter()
        .filter_map(|item| {
            let idx = item.get("tool_call").and_then(Value::as_u64)?;
            let pos = usize::try_from(idx).ok()?;
            let extra = item.get("extra_content")?;
            Some((pos, extra))
        })
        .collect()
}

/// Turns `data:` payloads into [`StreamEvent`]s.
#[derive(Debug)]
pub(crate) struct ChunkParser {
    think: Option<ThinkSplitter>,
    started: HashSet<u32>,
    call_pos: HashMap<u32, usize>,
    tool_extra: HashMap<u32, Value>,
}

impl ChunkParser {
    pub(crate) fn new(format: ReasoningFormat) -> Self {
        let think = matches!(format, ReasoningFormat::Auto | ReasoningFormat::ThinkTags)
            .then(ThinkSplitter::default);
        Self {
            think,
            started: HashSet::new(),
            call_pos: HashMap::new(),
            tool_extra: HashMap::new(),
        }
    }

    /// Parse one payload. `[DONE]` yields nothing.
    pub(crate) fn parse(&mut self, data: &str) -> Result<Vec<StreamEvent>, ProviderError> {
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            return Ok(Vec::new());
        }
        let v: Value = serde_json::from_str(data).map_err(|e| {
            ProviderError::new(ErrorKind::Protocol, format!("invalid stream chunk: {e}"))
        })?;
        if v.get("error").is_some() {
            // Classify by the error's own `type`/`code`, the way the Anthropic parser does.
            let kind = match v["error"]["type"]
                .as_str()
                .or_else(|| v["error"]["code"].as_str())
            {
                Some("server_error" | "api_error" | "overloaded_error" | "model_overloaded") => {
                    ErrorKind::ServerOverloaded
                }
                Some("rate_limit_error" | "rate_limit_exceeded" | "too_many_requests") => {
                    ErrorKind::RateLimited { retry_after: None }
                }
                Some(
                    "invalid_api_key"
                    | "authentication_error"
                    | "invalid_auth"
                    | "insufficient_quota"
                    | "quota_exceeded"
                    | "billing_not_active",
                ) => ErrorKind::Auth,
                _ => ErrorKind::BadRequest,
            };
            return Err(ProviderError::new(kind, body_message(data)));
        }
        let mut out = Vec::new();
        if let Some(choice) = v.pointer("/choices/0") {
            if let Some(delta) = choice.get("delta") {
                self.delta(delta, &mut out);
            }
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                self.flush(&mut out);
                self.flush_sealed(&mut out);
                out.push(StreamEvent::Finish(FinishReason::parse(reason)));
            }
        }
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            out.push(StreamEvent::Usage(parse_usage(u, &v)));
        } else if let Some(t) = v.get("timings").filter(|t| t.is_object())
            && let Some(u) = parse_llama_timings(t)
        {
            out.push(StreamEvent::Usage(u));
        }
        Ok(out)
    }

    /// End of stream: release text held by the think splitter and any sealed metadata.
    pub(crate) fn finish(&mut self) -> Vec<StreamEvent> {
        let mut out = Vec::new();
        self.flush(&mut out);
        self.flush_sealed(&mut out);
        out
    }

    fn flush_sealed(&mut self, out: &mut Vec<StreamEvent>) {
        if self.tool_extra.is_empty() {
            return;
        }
        let mut items: Vec<(usize, Value)> = std::mem::take(&mut self.tool_extra)
            .into_iter()
            .filter_map(|(index, extra_content)| {
                let pos = *self.call_pos.get(&index)?;
                Some((pos, extra_content))
            })
            .collect();
        items.sort_by_key(|(pos, _)| *pos);
        if !items.is_empty() {
            out.push(StreamEvent::Sealed(
                items
                    .into_iter()
                    .map(|(pos, extra_content)| {
                        json!({ "tool_call": pos, "extra_content": extra_content })
                    })
                    .collect(),
            ));
        }
    }

    fn flush(&mut self, out: &mut Vec<StreamEvent>) {
        if let Some(t) = &mut self.think {
            out.extend(t.finish().into_iter().map(piece_event));
        }
    }

    fn text(&mut self, text: &str, out: &mut Vec<StreamEvent>) {
        match &mut self.think {
            Some(t) => out.extend(t.feed(text).into_iter().map(piece_event)),
            None => out.push(StreamEvent::TextDelta(text.to_owned())),
        }
    }

    fn delta(&mut self, d: &Value, out: &mut Vec<StreamEvent>) {
        // Prefer `reasoning_content`; fall back to `reasoning` so gateways that mirror
        // the same delta under both keys don't double reasoning in history and break cache.
        if let Some(r) = ["reasoning_content", "reasoning"]
            .into_iter()
            .find_map(|key| d.get(key).and_then(Value::as_str).filter(|r| !r.is_empty()))
        {
            out.push(StreamEvent::ReasoningDelta(r.to_owned()));
        }
        match d.get("content") {
            Some(Value::String(text)) if !text.is_empty() => self.text(text, out),
            // Mistral sends content as parts: `text`, and `thinking` holding text parts.
            Some(Value::Array(parts)) => {
                for part in parts {
                    let text = |v: &Value| v.get("text").and_then(Value::as_str).map(str::to_owned);
                    match part.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(t) = text(part).filter(|t| !t.is_empty()) {
                                self.text(&t, out);
                            }
                        }
                        Some("thinking") => {
                            let r: String = match part.get("thinking") {
                                Some(Value::Array(inner)) => {
                                    inner.iter().filter_map(text).collect()
                                }
                                Some(Value::String(t)) => t.clone(),
                                _ => String::new(),
                            };
                            if !r.is_empty() {
                                out.push(StreamEvent::ReasoningDelta(r));
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        for call in d
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let index = call
                .get("index")
                .and_then(Value::as_u64)
                .and_then(|i| u32::try_from(i).ok())
                .unwrap_or(0);
            if let Some(extra) = call.get("extra_content").filter(|v| !v.is_null()) {
                self.tool_extra.insert(index, extra.clone());
            }
            let f = call.get("function");
            let name = f.and_then(|f| f.get("name")).and_then(Value::as_str);
            if !self.started.contains(&index) {
                let Some(name) = name.filter(|n| !n.is_empty()) else {
                    continue; // arguments before a name: nothing to attach them to
                };
                self.started.insert(index);
                self.call_pos.insert(index, self.call_pos.len());
                out.push(StreamEvent::ToolCallStart {
                    index,
                    id: call
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                    name: name.to_owned(),
                });
            }
            // Arguments arrive as string fragments; a few servers send an object instead.
            let args = f.and_then(|f| f.get("arguments"));
            let fragment = match args {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Null) | None => String::new(),
                Some(other) => other.to_string(),
            };
            if !fragment.is_empty() {
                out.push(StreamEvent::ToolCallArgsDelta { index, fragment });
            }
        }
    }
}

pub(crate) fn piece_event(p: Piece) -> StreamEvent {
    match p {
        Piece::Text(t) => StreamEvent::TextDelta(t),
        Piece::Reasoning(r) => StreamEvent::ReasoningDelta(r),
    }
}

fn parse_usage(u: &Value, chunk: &Value) -> Usage {
    let num = |v: &Value, p: &str| {
        v.pointer(p)
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
    };
    Usage {
        prompt_tokens: num(u, "/prompt_tokens").unwrap_or(0),
        completion_tokens: num(u, "/completion_tokens").unwrap_or(0),
        cached_prompt_tokens: num(u, "/prompt_tokens_details/cached_tokens")
            .or_else(|| num(u, "/cached_tokens"))
            .or_else(|| num(u, "/prompt_cache_hit_tokens"))
            .or_else(|| num(u, "/cache_read_input_tokens"))
            .or_else(|| num(chunk, "/timings/cache_n")),
        cache_write_tokens: num(u, "/prompt_tokens_details/cache_write_tokens")
            .or_else(|| num(u, "/prompt_tokens_details/cache_creation_input_tokens"))
            .or_else(|| num(u, "/cache_creation_input_tokens")),
        reasoning_tokens: num(u, "/completion_tokens_details/reasoning_tokens"),
    }
}

/// Fallback for llama.cpp `llama-server` chunks that report `timings` without `usage`.
fn parse_llama_timings(t: &Value) -> Option<Usage> {
    let num = |k: &str| {
        t.get(k)
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
    };
    let prompt_n = num("prompt_n");
    let cache_n = num("cache_n");
    let predicted_n = num("predicted_n");
    if prompt_n.is_none() && cache_n.is_none() && predicted_n.is_none() {
        return None;
    }
    Some(Usage {
        prompt_tokens: prompt_n.unwrap_or(0).saturating_add(cache_n.unwrap_or(0)),
        completion_tokens: predicted_n.unwrap_or(0),
        cached_prompt_tokens: cache_n,
        cache_write_tokens: None,
        reasoning_tokens: None,
    })
}

/// llama.cpp's server info, at the server root rather than under `/v1`.
pub(crate) const LLAMA_CPP_PROPS_PATH: &str = "props";

/// A `/models` response from llama.cpp's `llama-server` (`owned_by: "llamacpp"`).
pub(crate) fn is_llama_cpp(body: &Value) -> bool {
    body.get("data")
        .and_then(Value::as_array)
        .is_some_and(|d| d.iter().any(|m| m["owned_by"] == "llamacpp"))
}

/// The window llama.cpp actually serves (`-c`, per slot), which `/models` doesn't report:
/// there `meta.n_ctx_train` is the trained window, often far larger.
pub(crate) fn llama_cpp_window(props: &Value) -> Option<u32> {
    props
        .pointer("/default_generation_settings/n_ctx")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&n| n > 0)
}

/// Parse a `/models` response with dialect-specific model id normalization.
pub(crate) fn parse_models_for(body: &Value, dialect: Dialect) -> Vec<DiscoveredModel> {
    let list = body
        .get("data")
        .or_else(|| body.get("models"))
        .unwrap_or(body);
    let num = |m: &Value, keys: &[&str]| {
        keys.iter()
            .find_map(|k| m.pointer(k).and_then(Value::as_u64))
            .and_then(|n| u32::try_from(n).ok())
    };
    list.as_array()
        .into_iter()
        .flatten()
        // A ChatGPT sign-in's list marks the models it doesn't offer.
        .filter(|m| m["visibility"].as_str().is_none_or(|v| v == "list"))
        .filter_map(|m| {
            let raw_id = m
                .get("id")
                .or_else(|| m.get("slug"))
                .or_else(|| m.get("name"))?
                .as_str()?;
            let id = if dialect == Dialect::Google {
                raw_id.strip_prefix("models/").unwrap_or(raw_id)
            } else {
                raw_id
            }
            .to_owned();
            Some(DiscoveredModel {
                id,
                context_window: num(
                    m,
                    &[
                        "/context_window",
                        "/max_model_len",
                        "/context_length",
                        "/max_context_length",
                        "/meta/n_ctx_train",
                    ],
                ),
                max_output: num(
                    m,
                    &[
                        "/max_output_tokens",
                        "/max_completion_tokens",
                        "/top_provider/max_completion_tokens",
                    ],
                ),
                cost: m.get("pricing").and_then(pricing),
            })
        })
        .collect()
}

/// Prices from a model's `pricing`, in dollars per million tokens. Two shapes are common:
/// numbers per million (`input`, `output`, `cache_hit`), and OpenRouter's strings per
/// token (`prompt`, `completion`, `input_cache_read`). All zero means not priced.
fn pricing(p: &Value) -> Option<Cost> {
    let per_million = |keys: &[&str]| keys.iter().find_map(|k| p.get(*k)?.as_f64());
    let per_token = |key: &str| {
        let v = p.get(key)?;
        let n = v
            .as_str()
            .map_or_else(|| v.as_f64(), |s| s.trim().parse().ok())?;
        Some(n * 1_000_000.0)
    };
    let cost = match (per_million(&["input"]), per_million(&["output"])) {
        (Some(input), Some(output)) => Cost {
            input,
            output,
            cached_input: per_million(&["cache_hit", "cached_input", "cache_read"]),
            cache_write: None,
        },
        _ => Cost {
            input: per_token("prompt")?,
            output: per_token("completion")?,
            cached_input: per_token("input_cache_read"),
            cache_write: None,
        },
    };
    let valid = |n: f64| n.is_finite() && n >= 0.0;
    (valid(cost.input)
        && valid(cost.output)
        && cost.cached_input.is_none_or(valid)
        && cost.input + cost.output > 0.0)
        .then_some(cost)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_models(body: &Value) -> Vec<DiscoveredModel> {
        parse_models_for(body, Dialect::Standard)
    }
    use crate::types::{ResponseBuilder, ToolCall, ToolSpec};
    use quack_config::provider::Sampling;

    #[test]
    fn llama_cpp_serves_less_than_it_was_trained_on() {
        let models = serde_json::json!({"data":[{"id":"/models/q.gguf","owned_by":"llamacpp","meta":{"n_ctx_train":262144}}]});
        assert!(is_llama_cpp(&models));
        assert_eq!(parse_models(&models)[0].context_window, Some(262_144));
        let props = serde_json::json!({"default_generation_settings":{"n_ctx":16384}});
        assert_eq!(llama_cpp_window(&props), Some(16384));
        assert!(!is_llama_cpp(
            &serde_json::json!({"data":[{"id":"m","owned_by":"vllm"}]})
        ));
    }

    #[test]
    fn prices_come_from_either_pricing_shape() {
        let models = json!({"data":[
            {"id":"hyper","pricing":{"input":0.33,"output":1.31,"cache_create":0,"cache_hit":0.03}},
            {"id":"router","pricing":{"prompt":"0.0000003","completion":"0.0000012","input_cache_read":"0.00000003"}},
            {"id":"no-cache","pricing":{"input":1,"output":2}},
            {"id":"free","pricing":{"prompt":"0","completion":"0"}},
            {"id":"none"},
        ]});
        let cost: Vec<_> = parse_models(&models).into_iter().map(|m| m.cost).collect();
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let hyper = cost[0].as_ref().expect("hyper priced");
        assert!(near(hyper.input, 0.33) && near(hyper.output, 1.31));
        assert!(near(hyper.cached_input.expect("cache hit"), 0.03));
        let router = cost[1].as_ref().expect("openrouter priced");
        assert!(near(router.input, 0.3) && near(router.output, 1.2));
        assert!(near(router.cached_input.expect("cache read"), 0.03));
        assert_eq!(cost[2].as_ref().and_then(|c| c.cached_input), None);
        assert_eq!(cost[3], None, "all zero is not a price");
        assert_eq!(cost[4], None);
    }

    #[test]
    fn body_shape() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![
                Message::System {
                    content: "sys".into(),
                },
                Message::Assistant {
                    content: String::new(),
                    reasoning: Some("hidden".into()),
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        name: "read".into(),
                        arguments: "{}".into(),
                    }],
                    sealed: None,
                },
                Message::Tool {
                    call_id: "c1".into(),
                    content: "ok".into(),
                    images: Vec::new(),
                },
            ],
            tools: vec![ToolSpec {
                name: "read".into(),
                description: "Read a file".into(),
                parameters: json!({"type": "object"}),
            }],
            max_tokens: Some(64),
            extra_body: [(
                "chat_template_kwargs".to_owned(),
                json!({"enable_thinking": false}),
            )]
            .into_iter()
            .collect(),
            ..ChatRequest::default()
        };
        let b = build_body(&req, Dialect::Standard);
        assert_eq!(b["messages"][1]["content"], Value::Null);
        assert_eq!(
            b["messages"][1]["tool_calls"][0]["function"]["name"],
            "read"
        );
        assert_eq!(b["messages"][1]["reasoning_content"], "hidden");
        assert_eq!(b["messages"][2]["tool_call_id"], "c1");
        assert_eq!(b["tools"][0]["function"]["name"], "read");
        assert_eq!(b["max_tokens"], 64);
        assert_eq!(b["chat_template_kwargs"]["enable_thinking"], false);
        assert!(b.get("temperature").is_none());
    }

    fn img() -> quack_protocol::Image {
        quack_protocol::Image {
            media_type: "image/png".into(),
            data: "eHk=".into(),
            width: Some(2),
            height: Some(1),
        }
    }

    #[test]
    fn user_and_tool_images_take_their_places() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![
                Message::User {
                    content: "go".into(),
                    images: vec![img()],
                },
                Message::Assistant {
                    content: String::new(),
                    reasoning: None,
                    tool_calls: vec![
                        ToolCall {
                            id: "c1".into(),
                            name: "view_image".into(),
                            arguments: "{}".into(),
                        },
                        ToolCall {
                            id: "c2".into(),
                            name: "read".into(),
                            arguments: "{}".into(),
                        },
                    ],
                    sealed: None,
                },
                Message::Tool {
                    call_id: "c1".into(),
                    content: "[image: pic.png]".into(),
                    images: vec![img()],
                },
                Message::Tool {
                    call_id: "c2".into(),
                    content: "text".into(),
                    images: Vec::new(),
                },
                Message::User {
                    content: "and?".into(),
                    images: Vec::new(),
                },
            ],
            ..ChatRequest::default()
        };
        let b = build_body(&req, Dialect::Standard);
        let m = b["messages"].as_array().expect("messages");
        assert_eq!(m.len(), 6, "{m:?}");
        // Attached images become parts ahead of the user message's text.
        assert_eq!(
            m[0]["content"],
            json!([
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,eHk="}},
                {"type": "text", "text": "go"},
            ])
        );
        // The captioned result stays a plain tool string...
        assert_eq!(m[2]["role"], "tool");
        assert_eq!(m[2]["content"], "[image: pic.png]");
        // ...nothing is inserted inside the run of tool results...
        assert_eq!(m[3]["role"], "tool");
        // ...and the pixels follow as their own user message.
        assert_eq!(m[4]["role"], "user");
        assert_eq!(
            m[4]["content"],
            json!([{"type": "image_url", "image_url": {"url": "data:image/png;base64,eHk="}}])
        );
        assert_eq!(m[5]["content"], "and?");
    }

    #[test]
    fn mistral_takes_tool_images_inline_and_no_user_after_a_tool() {
        let mut req = mistral_history();
        let Some(Message::Tool { images, .. }) = req.messages.last_mut() else {
            panic!("history ends with a tool result")
        };
        images.push(img());
        // An interrupted turn: the next request follows the tool result directly.
        req.messages.push(Message::User {
            content: "stop, try this".into(),
            images: Vec::new(),
        });
        let b = build_body(&req, Dialect::Mistral);
        let m = b["messages"].as_array().expect("messages");
        let roles: Vec<&str> = m.iter().filter_map(|v| v["role"].as_str()).collect();
        assert_eq!(
            roles,
            ["system", "user", "assistant", "tool", "assistant", "user"]
        );
        assert_eq!(
            m[3]["content"],
            json!([
                {"type": "text", "text": "fn main() {}"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,eHk="}},
            ])
        );
        assert_eq!(m[4]["content"], "");
        // Other servers keep the tool string and get no inserted reply.
        let b = build_body(&req, Dialect::Standard);
        let roles: Vec<String> = b["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .filter_map(|v| v["role"].as_str().map(str::to_owned))
            .collect();
        assert_eq!(
            roles,
            ["system", "user", "assistant", "tool", "user", "user"]
        );
    }

    fn mistral_history() -> ChatRequest {
        ChatRequest {
            model: "mistral-large-4".into(),
            messages: vec![
                Message::System {
                    content: "sys".into(),
                },
                Message::User {
                    content: "fix it".into(),
                    images: Vec::new(),
                },
                Message::Assistant {
                    content: String::new(),
                    reasoning: Some("read first".into()),
                    tool_calls: vec![ToolCall {
                        id: "call_0".into(),
                        name: "read".into(),
                        arguments: "{}".into(),
                    }],
                    sealed: None,
                },
                Message::Tool {
                    call_id: "call_0".into(),
                    content: "fn main() {}".into(),
                    images: Vec::new(),
                },
            ],
            sampling: Sampling {
                temperature: Some(0.3),
                top_k: Some(20),
                min_p: Some(0.05),
                ..Sampling::default()
            },
            reasoning_effort: Some("medium".into()),
            ..ChatRequest::default()
        }
    }

    #[test]
    fn selected_effort_takes_precedence_over_extra_body_and_keeps_dialect_mapping() {
        let req = ChatRequest {
            reasoning_effort: Some("medium".into()),
            extra_body: serde_json::from_value(json!({ "reasoning_effort": "low" }))
                .expect("extra body"),
            ..ChatRequest::default()
        };
        assert_eq!(
            build_body(&req, Dialect::Standard)["reasoning_effort"],
            "medium"
        );
        assert_eq!(
            build_body(&req, Dialect::Mistral)["reasoning_effort"],
            "high"
        );
    }

    #[test]
    fn mistral_api_shape() {
        assert_eq!(Dialect::of("https://api.mistral.ai/v1"), Dialect::Mistral);
        assert_eq!(
            Dialect::of("https://openrouter.ai/api/v1"),
            Dialect::Standard
        );
        let req = mistral_history();
        let b = build_body(&req, Dialect::Mistral);
        // Only fields Mistral knows.
        assert!(b.get("stream_options").is_none());
        assert!(b.get("top_k").is_none() && b.get("min_p").is_none());
        assert_eq!(b["temperature"], 0.3);
        assert_eq!(b["reasoning_effort"], "high");
        assert_eq!(
            b["prompt_cache_key"],
            build_body(&req, Dialect::Mistral)["prompt_cache_key"]
        );
        // Reasoning goes back as a thinking part; the empty answer stays a string part list.
        let a = &b["messages"][2];
        assert_eq!(
            a["content"],
            json!([{"type": "thinking", "thinking": [{"type": "text", "text": "read first"}]}])
        );
        let id = a["tool_calls"][0]["id"].as_str().expect("id");
        assert_eq!(id.len(), 9);
        assert!(id.bytes().all(|c| c.is_ascii_alphanumeric()), "{id}");
        assert_eq!(b["messages"][3]["tool_call_id"], id);
        assert_eq!(b["messages"][3]["name"], "read");

        // `none` turns thinking off, and with it the echo.
        let mut off = mistral_history();
        off.reasoning_effort = Some("low".into());
        let b = build_body(&off, Dialect::Mistral);
        assert_eq!(b["reasoning_effort"], "none");
        assert_eq!(b["messages"][2]["content"], "");
        let mut kept_out = mistral_history();
        kept_out.preserve_reasoning = Some(ReasoningHistory::Never);
        assert_eq!(
            build_body(&kept_out, Dialect::Mistral)["messages"][2]["content"],
            ""
        );
    }

    #[test]
    fn mistral_large_elsewhere_gets_short_ids_only() {
        // For example vLLM serving the open weights: standard fields, Mistral's ids.
        let b = build_body(&mistral_history(), Dialect::Standard);
        assert_eq!(b["stream_options"]["include_usage"], true);
        assert_eq!(b["reasoning_effort"], "medium");
        assert_eq!(b["messages"][2]["content"], Value::Null);
        assert_eq!(
            b["messages"][2]["tool_calls"][0]["id"]
                .as_str()
                .map(str::len),
            Some(9)
        );
        assert!(b["messages"][3].get("name").is_none());
        assert_eq!(short_id("abcDEF123"), "abcDEF123");
        assert_ne!(short_id("call_0"), short_id("call_1"));
    }

    #[test]
    fn prompt_cache_key_is_opt_in_on_standard_openai_chat() {
        let mut req = ChatRequest {
            model: "accounts/fireworks/models/kimi-k3p1-pro".into(),
            cache_key: Some("session-42".into()),
            ..ChatRequest::default()
        };
        assert!(
            build_body(&req, Dialect::Standard)
                .get("prompt_cache_key")
                .is_none()
        );
        req.prompt_cache.cache_key = true;
        assert_eq!(
            build_body(&req, Dialect::Standard)["prompt_cache_key"],
            "session-42"
        );
        // Mistral prefers the session/shared cache key when set (survives compaction and
        // honors `--cache-key`), falling back to its message-derived hash when unset.
        assert_eq!(
            build_body(&req, Dialect::Mistral)["prompt_cache_key"],
            "session-42"
        );
        req.cache_key = None;
        let fallback = build_body(&req, Dialect::Mistral)["prompt_cache_key"]
            .as_str()
            .expect("derived mistral key")
            .to_owned();
        assert!(fallback.starts_with("ch-"), "{fallback}");
    }

    #[test]
    fn stream_with_think_tags_tool_call_and_usage() {
        let chunks = [
            r#"{"choices":[{"delta":{"role":"assistant","content":"<think>look"}}]}"#,
            r#"{"choices":[{"delta":{"content":" first</think>\n\nOk."}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"read","arguments":""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"a.rs\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5,"prompt_tokens_details":{"cached_tokens":8}}}"#,
            "[DONE]",
        ];
        let mut p = ChunkParser::new(ReasoningFormat::Auto);
        let mut b = ResponseBuilder::default();
        for c in chunks {
            for ev in p.parse(c).expect("chunk") {
                b.push(&ev);
            }
        }
        for ev in p.finish() {
            b.push(&ev);
        }
        assert_eq!(b.finish, Some(FinishReason::ToolCalls));
        assert_eq!(
            b.usage,
            Some(Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
                cached_prompt_tokens: Some(8),
                cache_write_tokens: None,
                reasoning_tokens: None,
            })
        );
        assert_eq!(
            b.into_message(),
            Message::Assistant {
                content: "Ok.".into(),
                reasoning: Some("look first".into()),
                tool_calls: vec![ToolCall {
                    id: "call_a".into(),
                    name: "read".into(),
                    arguments: r#"{"path":"a.rs"}"#.into()
                }],
                sealed: None,
            }
        );
    }

    #[test]
    fn mistral_content_parts() {
        let chunks = [
            r#"{"choices":[{"delta":{"role":"assistant","content":[{"type":"thinking","thinking":[{"type":"text","text":"check "}]}]}}]}"#,
            r#"{"choices":[{"delta":{"content":[{"type":"thinking","thinking":[{"type":"text","text":"the file"}]}]}}]}"#,
            r#"{"choices":[{"delta":{"content":[{"type":"text","text":"Done."}]}}]}"#,
            r#"{"choices":[{"delta":{"content":""},"finish_reason":"stop"}]}"#,
        ];
        let mut p = ChunkParser::new(ReasoningFormat::Auto);
        let mut b = ResponseBuilder::default();
        for c in chunks {
            for ev in p.parse(c).expect("chunk") {
                b.push(&ev);
            }
        }
        for ev in p.finish() {
            b.push(&ev);
        }
        assert_eq!(
            b.into_message(),
            Message::Assistant {
                content: "Done.".into(),
                reasoning: Some("check the file".into()),
                tool_calls: vec![],
                sealed: None,
            }
        );
        let models = parse_models(
            &serde_json::json!({"data": [{"id": "mistral-large-4", "max_context_length": 1048576}]}),
        );
        assert_eq!(models[0].context_window, Some(1_048_576));
    }

    #[test]
    fn mid_stream_error_and_models() {
        let mut p = ChunkParser::new(ReasoningFormat::None);
        let err = p
            .parse(r#"{"error":{"message":"overloaded"}}"#)
            .expect_err("error chunk");
        assert_eq!(err.message, "overloaded");

        let models = parse_models(&json!({"data": [
            {"id": "a", "context_window": 1000, "max_output_tokens": 10},
            {"id": "b", "max_model_len": 32768},
            {"id": "c"}
        ]}));
        assert_eq!(models[0].context_window, Some(1000));
        assert_eq!(models[0].max_output, Some(10));
        assert_eq!(models[1].context_window, Some(32768));
        assert_eq!(models[2].context_window, None);
    }

    #[test]
    fn google_dialect_models_thought_signature_and_fallback() {
        assert_eq!(
            Dialect::of("https://generativelanguage.googleapis.com/v1beta/openai"),
            Dialect::Google
        );
        let models = parse_models_for(
            &json!({"data": [
                {"id": "models/gemini-4-carbon"},
                {"id": "gemini-3.8-flash"}
            ]}),
            Dialect::Google,
        );
        assert_eq!(models[0].id, "gemini-4-carbon");
        assert_eq!(models[1].id, "gemini-3.8-flash");

        // Parallel tool calls: Gemini attaches thought_signature to the first call only.
        let chunks = [
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c0","type":"function","function":{"name":"read","arguments":"{\"path\":\"a\"}"},"extra_content":{"google":{"thought_signature":"sig-abc"}}},{"index":1,"id":"c1","type":"function","function":{"name":"read","arguments":"{\"path\":\"b\"}"}}]},"finish_reason":"tool_calls"}]}"#,
        ];
        let mut parser = ChunkParser::new(ReasoningFormat::Auto);
        let mut b = ResponseBuilder::default();
        for c in chunks {
            for ev in parser.parse(c).expect("chunk") {
                b.push(&ev);
            }
        }
        for ev in parser.finish() {
            b.push(&ev);
        }
        let msg = b.into_message();
        let req = ChatRequest {
            model: "gemini-4-carbon".into(),
            messages: vec![
                Message::User {
                    content: "read a and b".into(),
                    images: Vec::new(),
                },
                msg.clone(),
                Message::Tool {
                    call_id: "c0".into(),
                    content: "A".into(),
                    images: Vec::new(),
                },
                Message::Tool {
                    call_id: "c1".into(),
                    content: "B".into(),
                    images: Vec::new(),
                },
            ],
            ..ChatRequest::default()
        };
        let body = build_body(&req, Dialect::Google);
        let calls = &body["messages"][1]["tool_calls"];
        assert_eq!(
            calls[0]["extra_content"]["google"]["thought_signature"],
            "sig-abc"
        );
        assert!(calls[1].get("extra_content").is_none());

        // After `/resume` or compaction (`sealed = None`), inject the documented skip sentinel
        // on the first tool call for Google, and nowhere for standard providers.
        let mut unsealed_req = req.clone();
        if let Message::Assistant { sealed, .. } = &mut unsealed_req.messages[1] {
            *sealed = None;
        }
        let resumed_google = build_body(&unsealed_req, Dialect::Google);
        assert_eq!(
            resumed_google["messages"][1]["tool_calls"][0]["extra_content"]["google"]["thought_signature"],
            "skip_thought_signature_validator"
        );
        assert!(
            resumed_google["messages"][1]["tool_calls"][1]
                .get("extra_content")
                .is_none()
        );
        let resumed_std = build_body(&unsealed_req, Dialect::Standard);
        assert!(
            resumed_std["messages"][1]["tool_calls"][0]
                .get("extra_content")
                .is_none()
        );
    }

    #[test]
    fn cohere_and_google_omit_unsupported_fields() {
        assert_eq!(
            Dialect::of("https://api.cohere.ai/compatibility/v1"),
            Dialect::Cohere
        );
        let mut req = ChatRequest::default();
        req.sampling.top_k = Some(20);
        req.sampling.min_p = Some(0.05);
        req.sampling.repetition_penalty = Some(1.1);

        let cohere = build_body(&req, Dialect::Cohere);
        assert!(cohere.get("stream_options").is_none());
        assert!(cohere.get("top_k").is_none());
        assert!(cohere.get("min_p").is_none());
        assert!(cohere.get("repetition_penalty").is_none());

        let google = build_body(&req, Dialect::Google);
        assert_eq!(google["stream_options"]["include_usage"], true);
        assert!(google.get("top_k").is_none());
        assert!(google.get("min_p").is_none());
        assert!(google.get("repetition_penalty").is_none());
    }

    #[test]
    fn preserve_reasoning_current_turn_wire_modes_parallel_and_tool_modes() {
        let tool = ToolSpec {
            name: "read".into(),
            description: "Read file".into(),
            parameters: json!({"type":"object","properties":{"path":{"type":"string"}}}),
        };
        let mut req = ChatRequest {
            model: "deepseek-v4".into(),
            messages: vec![
                Message::System {
                    content: "sys".into(),
                },
                Message::User {
                    content: "turn 1".into(),
                    images: Vec::new(),
                },
                Message::Assistant {
                    content: "done 1".into(),
                    reasoning: Some("old thought".into()),
                    tool_calls: Vec::new(),
                    sealed: None,
                },
                Message::User {
                    content: "turn 2".into(),
                    images: Vec::new(),
                },
                Message::Assistant {
                    content: String::new(),
                    reasoning: Some("current thought".into()),
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        name: "read".into(),
                        arguments: "{}".into(),
                    }],
                    sealed: None,
                },
                Message::Tool {
                    call_id: "c1".into(),
                    content: "ok".into(),
                    images: Vec::new(),
                },
            ],
            tools: vec![tool],
            reasoning_effort: Some("high".into()),
            preserve_reasoning: Some(ReasoningHistory::CurrentTurn),
            reasoning_wire: ReasoningWire::EnableThinking,
            parallel_tool_calls: Some(false),
            ..ChatRequest::default()
        };
        let b = build_body(&req, Dialect::Standard);
        assert!(b["messages"][2].get("reasoning_content").is_none());
        assert_eq!(b["messages"][4]["reasoning_content"], "current thought");
        assert!(b.get("reasoning_effort").is_none());
        assert_eq!(b["chat_template_kwargs"]["enable_thinking"], true);
        assert_eq!(b["parallel_tool_calls"], false);

        req.reasoning_effort = Some("none".into());
        let off = build_body(&req, Dialect::Standard);
        assert_eq!(off["chat_template_kwargs"]["enable_thinking"], false);
        assert!(off["messages"][4].get("reasoning_content").is_none());

        req.reasoning_wire = ReasoningWire::None;
        req.reasoning_effort = Some("high".into());
        let none_wire = build_body(&req, Dialect::Standard);
        assert!(none_wire.get("reasoning_effort").is_none());
        assert!(none_wire.get("chat_template_kwargs").is_none());

        // ToolCallMode::Text moves tools into the system prompt and omits wire `tools`.
        req.tool_mode = Some(ToolCallMode::Text);
        let text_mode = build_body(&req, Dialect::Standard);
        assert!(text_mode.get("tools").is_none());
        assert!(
            text_mode["messages"][0]["content"]
                .as_str()
                .is_some_and(|s| s.contains("<tools>") && s.contains("- read: Read file"))
        );

        // ToolCallMode::Grammar emits a json_schema response_format naming the offered tools.
        req.tool_mode = Some(ToolCallMode::Grammar);
        let grammar_mode = build_body(&req, Dialect::Standard);
        assert_eq!(grammar_mode["response_format"]["type"], "json_schema");
        assert_eq!(
            grammar_mode["response_format"]["json_schema"]["schema"]["properties"]["tool_calls"]["items"]
                ["properties"]["name"]["enum"],
            json!(["read"])
        );
        let parsed = crate::toolparse::extract(
            r#"{"content":"checking","tool_calls":[{"name":"read","arguments":{"path":"a.rs"}}]}"#,
            crate::toolparse::TextFormat::Auto,
            &["read"],
        )
        .expect("parses grammar json reply");
        assert_eq!(parsed.0, "checking");
        assert_eq!(parsed.1.len(), 1);
        assert_eq!(parsed.1[0].name, "read");

        let usage = parse_usage(
            &json!({
                "prompt_tokens": 200,
                "completion_tokens": 50,
                "prompt_tokens_details": { "cached_tokens": 120, "cache_write_tokens": 40 },
                "completion_tokens_details": { "reasoning_tokens": 35 }
            }),
            &json!({}),
        );
        assert_eq!(usage.cached_prompt_tokens, Some(120));
        assert_eq!(usage.cache_write_tokens, Some(40));
        assert_eq!(usage.reasoning_tokens, Some(35));
    }

    #[test]
    fn nudge_preserves_current_turn_reasoning_and_cache_control_modes_work() {
        // 1. Mid-turn Message::Nudge sends as role="user" without stripping earlier
        //    reasoning under ReasoningHistory::CurrentTurn.
        let mut req = ChatRequest {
            model: "deepseek-v4".into(),
            messages: vec![
                Message::System {
                    content: "sys".into(),
                },
                Message::User {
                    content: "<context>AGENTS.md</context>\n\nfix bug".into(),
                    images: Vec::new(),
                },
                Message::Assistant {
                    content: String::new(),
                    reasoning: Some("step 1 thinking".into()),
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        name: "read".into(),
                        arguments: "{}".into(),
                    }],
                    sealed: None,
                },
                Message::Tool {
                    call_id: "c1".into(),
                    content: "ok".into(),
                    images: Vec::new(),
                },
                Message::Nudge {
                    content: "Your last reply was empty.".into(),
                },
            ],
            preserve_reasoning: Some(ReasoningHistory::CurrentTurn),
            ..ChatRequest::default()
        };
        let b = build_body(&req, Dialect::Standard);
        assert_eq!(b["messages"][2]["reasoning_content"], "step 1 thinking");
        assert_eq!(b["messages"][4]["role"], "user");
        assert_eq!(b["messages"][4]["content"], "Your last reply was empty.");

        // 2. CacheControlMode::Auto emits top-level `cache_control` with optional TTL.
        req.prompt_cache.cache_control = Some(CacheControlMode::Auto);
        req.prompt_cache.ttl = Some("1h".into());
        let auto = build_body(&req, Dialect::Standard);
        assert_eq!(
            auto["cache_control"],
            json!({ "type": "ephemeral", "ttl": "1h" })
        );
        assert!(auto.get("prompt_cache_options").is_none());

        // 3. CacheControlMode::Breakpoints marks system, split <context>, and recent user
        //    messages while keeping `role: "tool"` as a plain string (<= 4 breakpoints).
        req.prompt_cache.cache_control = Some(CacheControlMode::Breakpoints);
        req.prompt_cache.ttl = None;
        let bp = build_body(&req, Dialect::Standard);
        assert!(bp.get("cache_control").is_none());
        assert_eq!(
            bp["messages"][0]["content"][0]["cache_control"],
            json!({ "type": "ephemeral" })
        );
        assert_eq!(
            bp["messages"][1]["content"][0]["text"],
            "<context>AGENTS.md</context>"
        );
        assert_eq!(
            bp["messages"][1]["content"][0]["cache_control"],
            json!({ "type": "ephemeral" })
        );
        assert_eq!(bp["messages"][1]["content"][1]["text"], "fix bug");
        assert_eq!(bp["messages"][3]["role"], "tool");
        assert!(bp["messages"][3]["content"].is_string());

        // 4. Moonshot/Kimi implicit TTL on Standard dialect when cache_control is unset.
        req.prompt_cache.cache_control = None;
        req.prompt_cache.cache_key = true;
        req.prompt_cache.ttl = Some("1h".into());
        req.cache_key = Some("shared-ci-key".into());
        let kimi = build_body(&req, Dialect::Standard);
        assert_eq!(kimi["prompt_cache_key"], "shared-ci-key");
        assert_eq!(
            kimi["prompt_cache_options"],
            json!({ "mode": "implicit", "ttl": "1h" })
        );

        // 5. Mistral prefers explicit req.cache_key over message hashing.
        let mistral = build_body(&req, Dialect::Mistral);
        assert_eq!(mistral["prompt_cache_key"], "shared-ci-key");
    }

    #[test]
    fn parses_dashscope_and_llama_cpp_cache_usage_fields() {
        let dashscope = parse_usage(
            &json!({
                "prompt_tokens": 500,
                "completion_tokens": 30,
                "prompt_tokens_details": {
                    "cached_tokens": 320,
                    "cache_creation_input_tokens": 150
                }
            }),
            &json!({}),
        );
        assert_eq!(dashscope.cached_prompt_tokens, Some(320));
        assert_eq!(dashscope.cache_write_tokens, Some(150));

        // llama.cpp `timings.cache_n` fallback alongside usage and standalone.
        let mut p = ChunkParser::new(ReasoningFormat::Auto);
        let evs = p
            .parse(r#"{"choices":[],"usage":{"prompt_tokens":400,"completion_tokens":20},"timings":{"cache_n":350,"prompt_n":50,"predicted_n":20}}"#)
            .expect("chunk");
        assert_eq!(
            evs,
            vec![StreamEvent::Usage(Usage {
                prompt_tokens: 400,
                completion_tokens: 20,
                cached_prompt_tokens: Some(350),
                cache_write_tokens: None,
                reasoning_tokens: None,
            })]
        );
        let timings_only = p
            .parse(r#"{"choices":[],"timings":{"cache_n":300,"prompt_n":100,"predicted_n":15}}"#)
            .expect("timings chunk");
        assert_eq!(
            timings_only,
            vec![StreamEvent::Usage(Usage {
                prompt_tokens: 400,
                completion_tokens: 15,
                cached_prompt_tokens: Some(300),
                cache_write_tokens: None,
                reasoning_tokens: None,
            })]
        );
    }

    #[test]
    fn delta_dedupes_reasoning_content_and_reasoning_aliases() {
        let chunks = [
            // Gateways sometimes mirror the same delta under both keys; emit it once.
            r#"{"choices":[{"delta":{"reasoning_content":"check ","reasoning":"check "}}]}"#,
            r#"{"choices":[{"delta":{"reasoning":"the file"}}]}"#,
            r#"{"choices":[{"delta":{"content":"Done."},"finish_reason":"stop"}]}"#,
        ];
        let mut p = ChunkParser::new(ReasoningFormat::Auto);
        let mut b = ResponseBuilder::default();
        for c in chunks {
            for ev in p.parse(c).expect("chunk") {
                b.push(&ev);
            }
        }
        for ev in p.finish() {
            b.push(&ev);
        }
        assert_eq!(
            b.into_message(),
            Message::Assistant {
                content: "Done.".into(),
                reasoning: Some("check the file".into()),
                tool_calls: vec![],
                sealed: None,
            }
        );
    }
}
