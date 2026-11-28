//! Wire-protocol adapters, streaming, tool-call normalization and tokenizers.
//! See docs/plan/02-providers.md.
//!
//! `openai-chat` is the baseline adapter and is always built: every build needs at least
//! one, and it covers nearly every open-model server. `ollama` speaks Ollama's native API,
//! `anthropic-messages` the Anthropic Messages API and `openai-responses` the OpenAI
//! Responses API, each behind its `proto-*` feature. `auth-chatgpt` adds the ChatGPT
//! sign-in ([`login`]).

#[cfg(feature = "proto-anthropic")]
mod anthropic;
mod error;
pub mod fetch;
pub mod ledger;
#[cfg(feature = "auth-chatgpt")]
pub mod login;
#[cfg(feature = "proto-ollama")]
mod ollama;
mod openai_chat;
pub mod probe;
mod provider;
mod reasoning;
#[cfg(feature = "proto-openai-responses")]
mod responses;
mod sse;
pub mod tokens;
pub mod toolparse;
mod types;
pub mod web_fetch;

pub use error::{ErrorKind, ProviderError};
pub use ledger::{EgressDraft, EgressLedger};
pub use provider::{Provider, ResponsesSession, http_client, http_client_observed, supports};
pub use reqwest;
pub use types::{
    ChatRequest, DiscoveredModel, FinishReason, Message, ResponseBuilder, Sealed, StreamEvent,
    ToolCall, ToolSpec, fingerprint, strip_utility_extra, unseal,
};
