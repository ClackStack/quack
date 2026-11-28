//! `mcp.d/<id>.toml` schema (docs/plan/09-mcp.md#server-file-schema).
//!
//! A server on this machine needs only its command:
//!
//! ```toml
//! command = "npx"
//! args = ["-y", "@modelcontextprotocol/server-everything"]
//! ```
//!
//! A remote one needs only its URL:
//!
//! ```toml
//! url = "https://mcp.example.com/mcp"
//! headers = { Authorization = { env = "EXAMPLE_TOKEN", format = "Bearer {}" } }
//! ```

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::secret::Secret;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpServerFile {
    /// Defaults to the file stem.
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Shown in `/mcp`.
    #[serde(default)]
    pub description: String,

    /// Program to start (a server on this machine, over stdio). Found on `PATH`, including
    /// `.cmd` files on Windows.
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Extra environment variables for the server; values may be secrets.
    #[serde(default)]
    pub env: BTreeMap<String, Secret>,
    /// Working directory; default: the workspace root.
    #[serde(default)]
    pub cwd: String,
    /// Variables passed through from the harness's own environment. Default: a minimal set
    /// (`PATH`, `HOME`, temp and locale variables, and what Windows needs).
    #[serde(default)]
    pub inherit_env: Option<Vec<String>>,

    /// Streamable HTTP endpoint of a remote server (instead of `command`).
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, Secret>,
    /// OAuth 2.1 + PKCE sign-in for a remote HTTP server (`quack mcp login <id>`).
    #[serde(default)]
    pub oauth: Option<McpOAuth>,

    /// Run a local server inside the OS sandbox (see `[sandbox]` in config.toml).
    #[serde(default)]
    pub sandbox: Option<bool>,

    #[serde(default = "startup_timeout")]
    pub startup_timeout_secs: u64,
    #[serde(default = "call_timeout")]
    pub call_timeout_secs: u64,
    /// Longer results are cut, keeping the start and the end (and saving the full output
    /// to a file when a spill directory is configured).
    #[serde(default = "max_output")]
    pub max_output_tokens: u32,
    /// Controls whether this server's tools are deferred behind `tool_search`.
    #[serde(default)]
    pub defer: McpDefer,
    /// Which model answers this server's `sampling/createMessage` requests.
    #[serde(default)]
    pub sampling_model: McpSamplingModel,
    #[serde(default)]
    pub tools: McpTools,
}

/// OAuth 2.1 + PKCE configuration for a remote streamable-HTTP MCP server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpOAuth {
    /// Enable OAuth sign-in for this server (default true when `[oauth]` is present).
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Pre-registered OAuth client ID. Omit to use RFC 7591 Dynamic Client Registration
    /// when the authorization server advertises `registration_endpoint`.
    #[serde(default)]
    pub client_id: Option<String>,
    /// Optional client secret for confidential clients.
    #[serde(default)]
    pub client_secret: Option<Secret>,
    /// Scopes to request; defaults to scopes advertised by the resource/auth server.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Override the authorization server issuer URL instead of discovering it via
    /// RFC 9728 Protected Resource Metadata (`/.well-known/oauth-protected-resource`).
    #[serde(default)]
    pub authorization_server: Option<String>,
    /// Preferred loopback port on `127.0.0.1` for the OAuth redirect callback.
    #[serde(default)]
    pub redirect_port: Option<u16>,
}

impl Default for McpOAuth {
    fn default() -> Self {
        Self {
            enabled: true,
            client_id: None,
            client_secret: None,
            scopes: Vec::new(),
            authorization_server: None,
            redirect_port: None,
        }
    }
}

/// Per-server override for deferring MCP tool schemas behind `tool_search`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpDefer {
    /// Defer behind `tool_search` when MCP schemas exceed 10% of the context budget.
    #[default]
    Auto,
    /// Always keep this server's tools behind `tool_search`.
    Always,
    /// Always offer this server's tools directly in the model's tool list.
    Never,
}

/// Model used to serve a server's `sampling/createMessage` requests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpSamplingModel {
    /// Use the session's active model.
    #[default]
    Current,
    /// Use `[model] small` when configured, falling back to the session model.
    Small,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpTools {
    /// Only these tools (globs on the tool name). Empty: all.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Never these tools (globs).
    #[serde(default)]
    pub deny: Vec<String>,
    /// Run these without asking (globs; `["*"]` for all). Others ask, unless the permission
    /// mode or an `mcp:` allow rule says otherwise.
    #[serde(default)]
    pub auto_approve: Vec<String>,
}

impl McpServerFile {
    /// `stdio` or `http`, or why the file names neither or both.
    pub fn transport(&self) -> Result<&'static str, String> {
        match (self.command.is_empty(), self.url.is_empty()) {
            (false, true) => Ok("stdio"),
            (true, false) => Ok("http"),
            (true, true) => Err("set `command` (a local server) or `url` (a remote one)".into()),
            (false, false) => Err("set either `command` or `url`, not both".into()),
        }
    }

    /// True when this server uses OAuth 2.1 sign-in (`[oauth]` enabled on an HTTP server).
    pub fn uses_oauth(&self) -> bool {
        self.oauth.as_ref().is_some_and(|o| o.enabled)
    }
}

fn yes() -> bool {
    true
}

fn startup_timeout() -> u64 {
    20
}

fn call_timeout() -> u64 {
    120
}

fn max_output() -> u32 {
    8000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_files() {
        let f: McpServerFile =
            toml::from_str("command = \"npx\"\nargs = [\"-y\", \"server\"]\n").expect("parse");
        assert_eq!(f.transport(), Ok("stdio"));
        assert_eq!(f.call_timeout_secs, 120);
        let f: McpServerFile = toml::from_str(
            "url = \"https://x/mcp\"\nheaders = { Authorization = { env = \"T\", format = \"Bearer {}\" } }\n[tools]\nauto_approve = [\"get_*\"]\n",
        )
        .expect("parse");
        assert_eq!(f.transport(), Ok("http"));
        assert_eq!(f.tools.auto_approve, ["get_*"]);
        let f: McpServerFile = toml::from_str("description = \"x\"").expect("parse");
        assert!(f.transport().is_err());
        assert!(toml::from_str::<McpServerFile>("comand = \"x\"").is_err());
    }

    #[test]
    fn shipped_examples_parse() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../defaults/mcp.d");
        let mut n = 0;
        for e in std::fs::read_dir(&dir).expect("defaults/mcp.d") {
            let path = e.expect("entry").path();
            if path.extension().is_some_and(|x| x == "toml") {
                let text = std::fs::read_to_string(&path).expect("read");
                let f: McpServerFile =
                    toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                assert!(f.transport().is_ok(), "{}", path.display());
                assert!(!f.enabled, "{} must ship disabled", path.display());
                n += 1;
            }
        }
        assert!(n >= 4);
    }
}
