//! Opt-in SSRF-protected HTTP fetch tool (`web_fetch`).
//!
//! Disabled by default (`[tools.web_fetch] enabled = false`) and unregistered whenever
//! `network.offline = true`. The HTTP/SSRF implementation is injected via [`WebFetcher`]
//! so `quack-tools` stays free of HTTP dependencies.

use std::fmt;

use futures::future::BoxFuture;
use quack_protocol::ApprovalRequest;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::registry::{Subject, Tool, ToolCtx, ToolKind, ToolOutput, arg_str, opt_u64};

/// Response returned by a [`WebFetcher`] implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFetchResponse {
    pub final_url: String,
    pub status: u16,
    pub content_type: String,
    pub body: String,
    pub truncated: bool,
}

/// Engine-provided HTTP fetcher carrying SSRF checks, egress policy, and ledger recording.
pub trait WebFetcher: Send + Sync + fmt::Debug {
    fn fetch<'a>(
        &'a self,
        url: &'a str,
        max_chars: usize,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<WebFetchResponse, String>>;
}

pub(crate) struct WebFetch;

const DEFAULT_MAX_CHARS: usize = 24_000;
const HARD_MAX_CHARS: usize = 100_000;

impl Tool for WebFetch {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::ReadOnly
    }

    fn available(&self, ctx: &ToolCtx) -> bool {
        ctx.web_fetch.is_some()
    }

    fn description(&self, _ctx: &ToolCtx) -> String {
        "Fetch a public HTTP or HTTPS URL and return its text content. HTML is converted to \
         readable text. Blocks private, loopback, link-local, and metadata IP addresses unless \
         configured with tools.web_fetch.allow_private = true."
            .into()
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The http:// or https:// URL to fetch."
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Maximum characters of body text to return (default 24000, max 100000)."
                }
            },
            "required": ["url"]
        })
    }

    fn subject(&self, args: &Value, _ctx: &ToolCtx) -> Result<Subject, String> {
        let url = arg_str(args, &["url", "uri"])?.trim();
        if url.is_empty() {
            return Err("`url` cannot be empty".into());
        }
        Ok(Subject::WebFetch(url.to_owned()))
    }

    fn summary(&self, args: &Value, _ctx: &ToolCtx) -> String {
        let url = arg_str(args, &["url", "uri"]).unwrap_or("?");
        format!("web_fetch {url}")
    }

    fn preview(&self, args: &Value, _ctx: &ToolCtx) -> Result<ApprovalRequest, String> {
        let url = arg_str(args, &["url", "uri"])?.trim();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(format!(
                "web_fetch only allows http:// or https:// URLs (got `{url}`)"
            ));
        }
        Ok(ApprovalRequest::Tool {
            name: "web_fetch".into(),
            arguments: url.to_owned(),
        })
    }

    fn run<'a>(&'a self, args: Value, ctx: &'a ToolCtx) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let Some(fetcher) = &ctx.web_fetch else {
                return ToolOutput::error(
                    "web_fetch is disabled (enable [tools.web_fetch] enabled = true and ensure network.offline = false)",
                );
            };
            let url = match arg_str(&args, &["url", "uri"]) {
                Ok(u) => u.trim().to_owned(),
                Err(e) => return ToolOutput::error(e),
            };
            let max_chars = opt_u64(&args, &["max_chars", "max_length"])
                .and_then(|n| usize::try_from(n).ok())
                .map_or(DEFAULT_MAX_CHARS, |n| n.clamp(256, HARD_MAX_CHARS));
            match fetcher.fetch(&url, max_chars, ctx.cancel.clone()).await {
                Ok(resp) => {
                    let mut header = format!(
                        "URL: {}\nStatus: {}\nContent-Type: {}",
                        resp.final_url, resp.status, resp.content_type
                    );
                    if resp.truncated {
                        header.push_str("\n[Output truncated]");
                    }
                    let model = format!("{header}\n\n{}", resp.body);
                    let summary = format!("{} · {} chars", resp.status, resp.body.len());
                    ToolOutput::ok(model, summary)
                }
                Err(e) => ToolOutput::error(e),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::test_ctx;

    #[derive(Debug)]
    struct StubFetcher;

    impl WebFetcher for StubFetcher {
        fn fetch<'a>(
            &'a self,
            url: &'a str,
            _max_chars: usize,
            _cancel: CancellationToken,
        ) -> BoxFuture<'a, Result<WebFetchResponse, String>> {
            let url = url.to_owned();
            Box::pin(async move {
                Ok(WebFetchResponse {
                    final_url: url,
                    status: 200,
                    content_type: "text/plain".into(),
                    body: "hello world".into(),
                    truncated: false,
                })
            })
        }
    }

    #[tokio::test]
    async fn hidden_when_disabled_and_runs_when_enabled() {
        let mut ctx = test_ctx();
        assert!(!WebFetch.available(&ctx));
        ctx.web_fetch = Some(std::sync::Arc::new(StubFetcher));
        assert!(WebFetch.available(&ctx));

        assert!(
            WebFetch
                .preview(&json!({"url": "file:///etc/passwd"}), &ctx)
                .is_err()
        );
        let out = WebFetch
            .run(json!({"url": "https://example.com/docs"}), &ctx)
            .await;
        assert!(out.ok, "{out:?}");
        assert!(out.model.contains("hello world"));
    }
}
