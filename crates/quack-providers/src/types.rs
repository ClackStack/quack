//! Provider-neutral request and stream types. Adapters translate these to and from the wire.

use quack_config::provider::{Cost, PromptCache, ReasoningHistory, ReasoningWire, Sampling};
use quack_protocol::{Image, Usage};
use serde::{Deserialize, Serialize};

/// One request to a model. The profile layer shapes it before an adapter serializes it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    /// The window the session plans for. Servers that must be told (Ollama) get it.
    pub context_window: Option<u32>,
    pub max_tokens: Option<u32>,
    pub sampling: Sampling,
    pub reasoning_effort: Option<String>,
    /// Send earlier reasoning back with each assistant message: never, only within the
    /// current turn, or across all turns. `None`: the server's convention (`all_turns`).
    pub preserve_reasoning: Option<ReasoningHistory>,
    /// How `reasoning_effort` is placed on the wire for `openai-chat` providers.
    pub reasoning_wire: ReasoningWire,
    /// Whether the model may emit parallel tool calls (`Some(false)` sends
    /// `parallel_tool_calls: false` where supported).
    pub parallel_tool_calls: Option<bool>,
    /// Prompt-caching wire options (`openai-responses`).
    pub prompt_cache: PromptCache,
    /// Previous response id for Responses `prompt_cache_options.comparison_response_id`
    /// diagnostics when `prompt_cache.diagnostics` is enabled.
    pub comparison_response_id: Option<String>,
    /// Merged into the request body last; objects merge key by key.
    pub extra_body: serde_json::Map<String, serde_json::Value>,
    /// A stable id for the conversation, for servers that route by it to keep their prompt
    /// cache warm (OpenAI's `prompt_cache_key`).
    pub cache_key: Option<String>,
    /// Tool calling mode (e.g. grammar / guided decoding).
    pub tool_mode: Option<quack_config::provider::ToolCallMode>,
    /// Lightweight utility request (session title, text compaction): strip thinking and
    /// reasoning-summary overhead from provider and model `extra_body`.
    pub utility: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Message {
    System {
        content: String,
    },
    User {
        content: String,
        /// Images sent with the message (pasted, attached, or from a tool the profile
        /// turned into input); sent by adapters that support vision.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<Image>,
    },
    /// An internal mid-turn retry prompt (empty reply / output truncation) sent as
    /// `role: "user"` on the wire without starting a new user turn, so
    /// `preserve_in_history = "current_turn"` keeps earlier reasoning in the active chain.
    Nudge {
        content: String,
    },
    /// A Responses checkpoint with a portable text summary for other setups.
    Compaction {
        content: String,
        sealed: Sealed,
        /// Per-session context (`context.md`: workspace, AGENTS.md, skills, memory) kept
        /// verbatim alongside the encrypted compaction window so rules never degrade
        /// across compactions.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_context: Option<String>,
    },
    Assistant {
        #[serde(default)]
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCall>,
        /// The reply as the server produced it, with the reasoning it asks to get back
        /// unchanged (Anthropic's signed thinking, OpenAI's encrypted reasoning).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sealed: Option<Sealed>,
    },
    Tool {
        call_id: String,
        content: String,
        /// Images the tool returned (`view_image`); sent where the protocol takes them,
        /// next to or after the text.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<Image>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Raw JSON text, exactly as the model produced it.
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments object.
    pub parameters: serde_json::Value,
}

/// A reply's layout and opaque reasoning, replayed to the API that produced it. `items` is
/// the adapter's own format; text and tool calls stay in the message, and items refer to
/// tool calls by position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sealed {
    /// The provider id and model that produced it.
    pub provider: String,
    pub model: String,
    /// [`fingerprint`] of the request that produced it. Signed reasoning is tied to the
    /// system prompt and tools it was made under, so it is not sent once they change.
    pub fingerprint: String,
    pub items: Vec<serde_json::Value>,
}

/// A short stable hash of a request's system prompt and tools (FNV-1a, hex).
pub fn fingerprint(req: &ChatRequest) -> String {
    fingerprint_with(req, &[])
}

pub(crate) fn fingerprint_with(req: &ChatRequest, binding: &[&str]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    if let Some(Message::System { content }) = req.messages.first() {
        feed(content.as_bytes());
    }
    for t in &req.tools {
        feed(&[0]);
        feed(t.name.as_bytes());
        feed(t.description.as_bytes());
        feed(t.parameters.to_string().as_bytes());
    }
    for value in binding {
        feed(&[0]);
        feed(value.as_bytes());
    }
    format!("{h:016x}")
}

/// Remove sealed replies from a history. Callers do this after editing earlier messages
/// (pruning, compaction): signed reasoning after the edit would no longer verify.
pub fn unseal(history: &mut [Message]) {
    for m in history {
        if let Message::Assistant { sealed, .. } = m {
            *sealed = None;
        }
    }
}

/// Split a leading `<context>...</context>\n\n<prompt>` prefix (from a profile's
/// `context.md`) from the user's opening prompt so adapters with explicit breakpoints can
/// place a cache boundary right after the shared workspace context.
#[cfg_attr(
    not(any(
        feature = "proto-anthropic",
        feature = "proto-openai-responses",
        feature = "proto-openai-chat"
    )),
    allow(dead_code)
)]
pub(crate) fn split_session_context(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("<context>")?;
    let close = rest.find("</context>")?;
    let end = "<context>".len() + close + "</context>".len();
    let after = text[end..].strip_prefix("\n\n")?;
    (!after.is_empty()).then_some((&text[..end], after))
}

/// Build an Anthropic-style `cache_control` object (`{"type": "ephemeral", "ttl"?: ...}`).
#[cfg_attr(
    not(any(feature = "proto-anthropic", feature = "proto-openai-chat")),
    allow(dead_code)
)]
pub(crate) fn cache_control_object(ttl: Option<&str>) -> serde_json::Value {
    match ttl.map(str::trim).filter(|t| !t.is_empty()) {
        Some(ttl) => serde_json::json!({ "type": "ephemeral", "ttl": ttl }),
        None => serde_json::json!({ "type": "ephemeral" }),
    }
}

/// Merge `from` into `into`: objects key by key, everything else replaced.
pub(crate) fn merge(
    into: &mut serde_json::Value,
    from: &serde_json::Map<String, serde_json::Value>,
) {
    let Some(into) = into.as_object_mut() else {
        return;
    };
    for (k, v) in from {
        if let (Some(a @ serde_json::Value::Object(_)), serde_json::Value::Object(b)) =
            (into.get_mut(k), v)
        {
            merge(a, b);
            continue;
        }
        into.insert(k.clone(), v.clone());
    }
}

/// Strip thinking and reasoning-summary overhead from merged provider/model `extra_body`
/// for lightweight utility calls (session titles, text compaction), preserving unrelated
/// options such as `fallbacks`.
pub fn strip_utility_extra(extra: &mut serde_json::Map<String, serde_json::Value>) {
    if let Some(serde_json::Value::Object(r)) = extra.get_mut("reasoning") {
        r.remove("summary");
        r.remove("context");
        r.remove("effort");
        if r.is_empty() {
            extra.remove("reasoning");
        }
    }
    extra.remove("thinking");
    extra.remove("reasoning_effort");
    extra.remove("think");
    if let Some(serde_json::Value::Object(k)) = extra.get_mut("chat_template_kwargs") {
        k.remove("enable_thinking");
        if k.is_empty() {
            extra.remove("chat_template_kwargs");
        }
    }
}

/// A normalized streaming event.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    TextDelta(String),
    ReasoningDelta(String),
    ToolCallStart {
        index: u32,
        id: Option<String>,
        name: String,
    },
    ToolCallArgsDelta {
        index: u32,
        fragment: String,
    },
    Usage(Usage),
    Finish(FinishReason),
    /// The server's response id, when reported (e.g. OpenAI Responses `response.id`).
    ResponseId(String),
    /// The reply's layout and opaque reasoning, sent before `Finish`.
    Sealed(Vec<serde_json::Value>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Other(String),
}

impl FinishReason {
    pub fn parse(s: &str) -> Self {
        match s {
            "stop" | "eos" | "end_turn" => Self::Stop,
            "length" | "max_tokens" | "model_context_window_exceeded" => Self::Length,
            "tool_calls" | "function_call" | "tool_use" => Self::ToolCalls,
            "content_filter" | "refusal" => Self::ContentFilter,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// Folds stream events into the final assistant message.
#[derive(Debug, Default)]
pub struct ResponseBuilder {
    content: String,
    reasoning: String,
    calls: Vec<(u32, ToolCall)>,
    sealed: Vec<serde_json::Value>,
    pub usage: Option<Usage>,
    pub finish: Option<FinishReason>,
    pub response_id: Option<String>,
}

impl ResponseBuilder {
    pub fn push(&mut self, ev: &StreamEvent) {
        match ev {
            StreamEvent::TextDelta(t) => self.content.push_str(t),
            StreamEvent::ReasoningDelta(t) => self.reasoning.push_str(t),
            StreamEvent::ToolCallStart { index, id, name } => {
                let n = self.calls.len();
                self.calls.push((
                    *index,
                    ToolCall {
                        id: id.clone().unwrap_or_else(|| format!("call_{n}")),
                        name: name.clone(),
                        arguments: String::new(),
                    },
                ));
            }
            StreamEvent::ToolCallArgsDelta { index, fragment } => {
                if let Some((_, c)) = self.calls.iter_mut().rev().find(|(i, _)| i == index) {
                    c.arguments.push_str(fragment);
                }
            }
            StreamEvent::Usage(u) => self.usage = Some(*u),
            StreamEvent::Finish(f) => self.finish = Some(f.clone()),
            StreamEvent::ResponseId(id) => self.response_id = Some(id.clone()),
            StreamEvent::Sealed(items) => self.sealed.clone_from(items),
        }
    }

    pub fn has_tool_calls(&self) -> bool {
        !self.calls.is_empty()
    }

    pub fn into_message(self) -> Message {
        Message::Assistant {
            content: self.content,
            reasoning: Some(self.reasoning).filter(|r| !r.is_empty()),
            tool_calls: self.calls.into_iter().map(|(_, c)| c).collect(),
            sealed: (!self.sealed.is_empty()).then(|| Sealed {
                provider: String::new(),
                model: String::new(),
                fingerprint: String::new(),
                items: self.sealed,
            }),
        }
    }
}

/// A model reported by the provider's models endpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredModel {
    pub id: String,
    pub context_window: Option<u32>,
    pub max_output: Option<u32>,
    /// Prices, when the endpoint lists them.
    pub cost: Option<Cost>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strip_utility_extra_drops_thinking_and_keeps_unrelated_fields() {
        let mut extra: serde_json::Map<String, serde_json::Value> = serde_json::from_value(json!({
            "reasoning": { "summary": "auto", "context": "all_turns", "effort": "high" },
            "thinking": { "type": "adaptive", "display": "summarized" },
            "reasoning_effort": "max",
            "think": true,
            "chat_template_kwargs": { "enable_thinking": true, "add_generation_prompt": true },
            "fallbacks": "default",
        }))
        .expect("extra map");

        strip_utility_extra(&mut extra);

        assert!(!extra.contains_key("reasoning"));
        assert!(!extra.contains_key("thinking"));
        assert!(!extra.contains_key("reasoning_effort"));
        assert!(!extra.contains_key("think"));
        assert_eq!(
            extra["chat_template_kwargs"],
            json!({ "add_generation_prompt": true })
        );
        assert_eq!(extra["fallbacks"], "default");
    }
}
