//! On-demand synthetic tool-calling capability probe for configuring a model's
//! `[tool_calling]` mode (`native`, `text`, or `grammar`) and fallback parser format.

use std::time::Instant;

use quack_config::provider::{ReasoningFormat, ToolCallMode};
use quack_protocol::{ToolProbeCase, ToolProbeReport};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::ProviderError;
use crate::provider::Provider;
use crate::toolparse::{self, TextFormat};
use crate::types::{ChatRequest, Message, ResponseBuilder, ToolSpec};

const PROBE_MARKER: &str = "quack-ok";

fn probe_tool() -> ToolSpec {
    ToolSpec {
        name: "echo_probe".into(),
        description: "Return the provided marker string verbatim.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "value": { "type": "string" }
            },
            "required": ["value"],
            "additionalProperties": false
        }),
    }
}

fn mode_name(mode: ToolCallMode) -> &'static str {
    match mode {
        ToolCallMode::Native => "native",
        ToolCallMode::Text => "text",
        ToolCallMode::Grammar => "grammar",
    }
}

async fn run_case(
    provider: &Provider,
    req: ChatRequest,
    reasoning: ReasoningFormat,
) -> Result<(u64, ResponseBuilder), ProviderError> {
    let started = Instant::now();
    let cancel = CancellationToken::new();
    let mut rx = provider.stream(req, reasoning, cancel);
    let mut builder = ResponseBuilder::default();
    while let Some(ev) = rx.recv().await {
        match ev {
            Ok(event) => builder.push(&event),
            Err(err) => return Err(err),
        }
    }
    let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok((elapsed, builder))
}

fn valid_probe_args(raw: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| v.get("value").and_then(|x| x.as_str()).map(str::to_owned))
        .is_some_and(|v| v == PROBE_MARKER)
}

fn detect_text_format(text: &str) -> Option<&'static str> {
    let candidates = [
        (TextFormat::Hermes, "hermes"),
        (TextFormat::Qwen3Xml, "qwen3-xml"),
        (TextFormat::Glm, "glm"),
        (TextFormat::Deepseek, "deepseek"),
        (TextFormat::Kimi, "kimi"),
        (TextFormat::JsonBlock, "json-block"),
    ];
    for (fmt, label) in candidates {
        if let Some((_, calls)) = toolparse::extract(text, fmt, &["echo_probe"])
            && calls
                .iter()
                .any(|c| c.name == "echo_probe" && valid_probe_args(&c.arguments))
        {
            return Some(label);
        }
    }
    None
}

/// Probe `model` on `provider` with a tiny synthetic tool call to verify whether structured
/// `tool_calls`, JSON arguments, or text fallback tags work reliably.
pub async fn run_tool_probe(
    provider: &Provider,
    model: &str,
    configured_mode: ToolCallMode,
    reasoning: ReasoningFormat,
) -> Result<ToolProbeReport, ProviderError> {
    let configured = mode_name(configured_mode).to_owned();
    let tool = probe_tool();
    let mut cases = Vec::new();

    let native_req = ChatRequest {
        model: model.to_owned(),
        messages: vec![
            Message::System {
                content:
                    "You are running a tool-calling diagnostic. Call `echo_probe` immediately."
                        .into(),
            },
            Message::User {
                content: format!("Call echo_probe with value=\"{PROBE_MARKER}\"."),
                images: Vec::new(),
            },
        ],
        tools: vec![tool.clone()],
        max_tokens: Some(192),
        tool_mode: Some(ToolCallMode::Native),
        ..ChatRequest::default()
    };

    let (native_ms, native_builder) = run_case(provider, native_req, reasoning).await?;
    let native_msg = native_builder.into_message();
    let (native_passed, native_detail, native_malformed, inline_text_format) = match &native_msg {
        Message::Assistant {
            content,
            reasoning: r,
            tool_calls,
            ..
        } => {
            if let Some(tc) = tool_calls.iter().find(|c| c.name == "echo_probe") {
                if valid_probe_args(&tc.arguments) {
                    (
                        true,
                        "returned valid structured tool_call for echo_probe".to_owned(),
                        false,
                        None,
                    )
                } else {
                    (
                        false,
                        "returned echo_probe with malformed JSON arguments".to_owned(),
                        true,
                        None,
                    )
                }
            } else {
                let mut combined = content.clone();
                if let Some(reasoning_text) = r {
                    combined.push('\n');
                    combined.push_str(reasoning_text);
                }
                if let Some(fmt) = detect_text_format(&combined) {
                    (
                        false,
                        format!(
                            "emitted text-tagged `{fmt}` tool call instead of native tool_calls"
                        ),
                        false,
                        Some(fmt),
                    )
                } else {
                    (
                        false,
                        "did not emit a structured tool_call".to_owned(),
                        false,
                        None,
                    )
                }
            }
        }
        _ => (false, "unexpected probe response".to_owned(), false, None),
    };

    cases.push(ToolProbeCase {
        name: "structured_call".into(),
        passed: native_passed,
        latency_ms: native_ms,
        detail: native_detail,
    });

    if native_passed {
        return Ok(ToolProbeReport {
            model: model.to_owned(),
            configured_mode: configured,
            recommended_mode: "native".into(),
            recommended_text_format: None,
            cases,
            summary: format!("Native structured tool calls verified in {native_ms} ms."),
        });
    }

    if let Some(fmt) = inline_text_format {
        cases.push(ToolProbeCase {
            name: "text_fallback".into(),
            passed: true,
            latency_ms: 0,
            detail: format!("detected valid `{fmt}` markup from model output"),
        });
        return Ok(ToolProbeReport {
            model: model.to_owned(),
            configured_mode: configured,
            recommended_mode: "text".into(),
            recommended_text_format: Some(fmt.into()),
            cases,
            summary: format!("Model emitted `{fmt}` text tool calls; use mode = \"text\"."),
        });
    }

    if native_malformed {
        return Ok(ToolProbeReport {
            model: model.to_owned(),
            configured_mode: configured,
            recommended_mode: "grammar".into(),
            recommended_text_format: None,
            cases,
            summary: "Model emitted echo_probe with invalid JSON; try mode = \"grammar\" or check server flags."
                .into(),
        });
    }

    // Native structured calls produced neither a tool call nor inline markup: run one explicit
    // Hermes-tag fallback probe to verify whether text-mode tool parsing succeeds.
    let text_req = ChatRequest {
        model: model.to_owned(),
        messages: vec![
            Message::System {
                content: "When invoking a tool, output ONLY a `<tool_call>{\"name\":\"...\",\"arguments\":{...}}</tool_call>` block."
                    .into(),
            },
            Message::User {
                content: format!("Call echo_probe with value=\"{PROBE_MARKER}\"."),
                images: Vec::new(),
            },
        ],
        tools: vec![tool],
        max_tokens: Some(192),
        tool_mode: Some(ToolCallMode::Text),
        ..ChatRequest::default()
    };

    let (text_ms, text_builder) = run_case(provider, text_req, reasoning).await?;
    let text_msg = text_builder.into_message();
    let detected = match &text_msg {
        Message::Assistant { content, .. } => detect_text_format(content),
        _ => None,
    };
    if let Some(fmt) = detected {
        cases.push(ToolProbeCase {
            name: "text_fallback".into(),
            passed: true,
            latency_ms: text_ms,
            detail: format!("parsed `{fmt}` tool call in {text_ms} ms"),
        });
        Ok(ToolProbeReport {
            model: model.to_owned(),
            configured_mode: configured,
            recommended_mode: "text".into(),
            recommended_text_format: Some(fmt.into()),
            cases,
            summary: format!("Text fallback (`{fmt}`) succeeded; native tool_calls did not fire."),
        })
    } else {
        cases.push(ToolProbeCase {
            name: "text_fallback".into(),
            passed: false,
            latency_ms: text_ms,
            detail:
                "neither native tool_calls nor text-tag fallback produced a valid echo_probe call"
                    .into(),
        });
        Ok(ToolProbeReport {
            model: model.to_owned(),
            configured_mode: configured,
            recommended_mode: mode_name(configured_mode).into(),
            recommended_text_format: None,
            cases,
            summary:
                "Tool probe failed in both native and text modes; check server tool-calling flags."
                    .into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quack_config::ProviderFile;
    use quack_config::settings::NetworkSettings;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_provider(url: &str) -> Provider {
        let file: ProviderFile =
            serde_json::from_value(serde_json::json!({ "base_url": url })).expect("file");
        Provider::new("mock", file, &NetworkSettings::default(), &|_| None).expect("provider")
    }

    #[tokio::test]
    async fn probes_native_and_text_fallback_modes() {
        let server = MockServer::start().await;
        let sse = concat!(
            "data: {\"id\":\"1\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"type\":\"function\",\"function\":{\"name\":\"echo_probe\",\"arguments\":\"{\\\"value\\\":\\\"quack-ok\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"1\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(sse),
            )
            .mount(&server)
            .await;

        let provider = test_provider(&server.uri());
        let report = run_tool_probe(
            &provider,
            "test-model",
            ToolCallMode::Native,
            ReasoningFormat::Auto,
        )
        .await
        .expect("probe");
        assert_eq!(report.recommended_mode, "native");
        assert!(report.cases[0].passed);

        let text_server = MockServer::start().await;
        let hermes_sse = concat!(
            "data: {\"id\":\"2\",\"choices\":[{\"delta\":{\"content\":\"<tool_call>{\\\"name\\\":\\\"echo_probe\\\",\\\"arguments\\\":{\\\"value\\\":\\\"quack-ok\\\"}}</tool_call>\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"2\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(hermes_sse),
            )
            .mount(&text_server)
            .await;

        let provider = test_provider(&text_server.uri());
        let report = run_tool_probe(
            &provider,
            "test-model",
            ToolCallMode::Native,
            ReasoningFormat::Auto,
        )
        .await
        .expect("probe");
        assert_eq!(report.recommended_mode, "text");
        assert_eq!(report.recommended_text_format.as_deref(), Some("hermes"));
    }
}
