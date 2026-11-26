//! MCP in the agent loop (docs/plan/09-mcp.md): the tool adapter, deferred tools behind
//! `tool_search`, the resource tools, prompts as slash commands, and the startup row.
//!
//! Servers connect in the background. Each request offers the tools of the servers that
//! are ready at that moment, so a slow server never holds up a turn. When their
//! definitions would take more than a tenth of the context budget, only `tool_search` is
//! offered, plus the tools it has turned on.

use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use quack_config::{ItemState, Loaded, load_mcp};
use quack_mcp::{McpManager, ServerSpec, ServerState, ToolInfo, model_tool_name};
use quack_protocol::{
    ApprovalRequest, CommandInfo, Event, LoadState, McpServerInfo, McpServerState, Subsystem,
};
use quack_tools::{Subject, Tool, ToolCtx, ToolKind, ToolOutput};
use serde_json::{Value, json};
use tokio::sync::broadcast;

/// Budget used to decide deferral when the model's window is unknown.
const FALLBACK_BUDGET: u32 = 32_000;
/// Results `tool_search` returns (and turns on) at most.
const SEARCH_RESULTS: usize = 8;

/// Read `mcp.d` and create the manager. Files that fail to parse become warnings.
pub(crate) fn load(config: &Loaded) -> (Vec<ServerSpec>, Vec<String>) {
    let mut warnings = Vec::new();
    let specs = load_mcp(config)
        .into_iter()
        .filter_map(|item| match item.state {
            ItemState::Loaded(file) => Some(ServerSpec { id: item.id, file }),
            ItemState::Disabled(_) => None,
            ItemState::Failed(d) => {
                warnings.push(format!("mcp.d: {}", d.one_line()));
                None
            }
        })
        .collect();
    (specs, warnings)
}

/// The session's MCP servers and the deferred tools turned on so far.
#[derive(Debug)]
pub(crate) struct Mcp {
    pub manager: Arc<McpManager>,
    /// Model names of deferred tools `tool_search` turned on, in that order.
    activated: Mutex<Vec<String>>,
    /// Server order pinned as servers first appear in a session: servers ready at the first
    /// request appear in deterministic `id` order, while any slow server that connects later
    /// appends at the tail instead of shifting earlier tool definitions.
    server_order: Mutex<Vec<String>>,
    /// What [`Mcp::offered`] last returned, with the inputs it was built from: requests
    /// ask for the offered set on every model call and every tool dispatch, and
    /// rebuilding it clones every ready server's tool list.
    cache: Mutex<OfferCache>,
}

#[derive(Default)]
struct OfferCache {
    revision: u64,
    activated: Vec<String>,
    budget: Option<u32>,
    tools: Vec<Arc<dyn Tool>>,
}

impl std::fmt::Debug for OfferCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OfferCache")
            .field("revision", &self.revision)
            .field("activated", &self.activated)
            .field("budget", &self.budget)
            .field("tools", &self.tools.len())
            .finish()
    }
}

impl Mcp {
    pub fn new(manager: Arc<McpManager>) -> Arc<Self> {
        Arc::new(Self {
            manager,
            activated: Mutex::new(Vec::new()),
            server_order: Mutex::new(Vec::new()),
            cache: Mutex::new(OfferCache::default()),
        })
    }

    fn adapters(&self) -> Vec<McpTool> {
        let mut tools = self.manager.tools();
        if let Ok(mut order) = self.server_order.lock() {
            for t in &tools {
                if !order.iter().any(|id| id == &t.server) {
                    order.push(t.server.clone());
                }
            }
            tools.sort_by_key(|t| {
                order
                    .iter()
                    .position(|id| id == &t.server)
                    .unwrap_or(usize::MAX)
            });
        }
        tools
            .into_iter()
            .map(|info| McpTool {
                name: model_tool_name(&info.server, &info.name),
                info,
                manager: self.manager.clone(),
            })
            .collect()
    }

    /// Whether tools are behind `tool_search` for this budget.
    fn deferred(tools: &[McpTool], budget: Option<u32>) -> bool {
        let tokens: u32 = tools.iter().map(McpTool::tokens).sum();
        tokens > budget.unwrap_or(FALLBACK_BUDGET) / 10
    }

    /// The MCP tools to offer with the next request: the servers' tools (or
    /// `tool_search` and what it turned on), then the resource tools when any ready
    /// server lists resources. Cached until the servers, the turned-on tools or the
    /// budget change.
    pub fn offered(self: &Arc<Self>, budget: Option<u32>) -> Vec<Arc<dyn Tool>> {
        let revision = self.manager.revision();
        let activated = self.activated.lock().map(|a| a.clone()).unwrap_or_default();
        if let Ok(cache) = self.cache.lock()
            && cache.revision == revision
            && cache.budget == budget
            && cache.activated == activated
        {
            return cache.tools.clone();
        }
        let tools = self.build_offered(budget, &activated);
        if let Ok(mut cache) = self.cache.lock() {
            *cache = OfferCache {
                revision,
                activated,
                budget,
                tools: tools.clone(),
            };
        }
        tools
    }

    fn build_offered(
        self: &Arc<Self>,
        budget: Option<u32>,
        activated: &[String],
    ) -> Vec<Arc<dyn Tool>> {
        let tools = self.adapters();
        let mut out: Vec<Arc<dyn Tool>> = Vec::new();
        let has_resources = !self.manager.resources().is_empty();
        if !Self::deferred(&tools, budget) {
            out.extend(tools.into_iter().map(|t| Arc::new(t) as Arc<dyn Tool>));
            if has_resources {
                out.push(Arc::new(ListResources {
                    manager: self.manager.clone(),
                }));
                out.push(Arc::new(ReadResource {
                    manager: self.manager.clone(),
                }));
            }
        } else {
            // Keep static MCP tools (`tool_search` and resource tools) ahead of dynamically
            // activated tools so activating a match only appends at the tail of `tools[]`.
            out.push(Arc::new(ToolSearch {
                mcp: Arc::clone(self),
            }));
            if has_resources {
                out.push(Arc::new(ListResources {
                    manager: self.manager.clone(),
                }));
                out.push(Arc::new(ReadResource {
                    manager: self.manager.clone(),
                }));
            }
            for name in activated {
                if let Some(t) = tools.iter().find(|t| t.name == *name) {
                    out.push(Arc::new(t.clone()));
                }
            }
        }
        out
    }

    /// The offered tool with this model-facing name.
    pub fn find(self: &Arc<Self>, name: &str, budget: Option<u32>) -> Option<Arc<dyn Tool>> {
        self.offered(budget).into_iter().find(|t| t.name() == name)
    }

    /// `/mcp`: each server with its state and the cost of its tools.
    pub fn info(&self, budget: Option<u32>) -> Vec<McpServerInfo> {
        let tools = self.adapters();
        let deferred = Self::deferred(&tools, budget);
        self.manager
            .status()
            .into_iter()
            .map(|s| {
                let (state, error) = match s.state {
                    ServerState::Starting => (McpServerState::Starting, String::new()),
                    ServerState::Ready => (McpServerState::Ready, String::new()),
                    ServerState::Failed(e) => (McpServerState::Failed, e),
                    ServerState::Stopped => (McpServerState::Stopped, String::new()),
                };
                McpServerInfo {
                    tool_tokens: tools
                        .iter()
                        .filter(|t| t.info.server == s.id)
                        .map(McpTool::tokens)
                        .sum(),
                    id: s.id,
                    transport: s.transport,
                    target: s.target,
                    description: s.description,
                    state,
                    error,
                    tools: s.tools,
                    prompts: s.prompts,
                    resources: s.resources,
                    deferred,
                    log: s.log,
                }
            })
            .collect()
    }
}

/// Keep the startup row for MCP and the clients' list of prompts up to date as servers
/// connect.
pub(crate) fn report(manager: Arc<McpManager>, tx: broadcast::Sender<Event>) {
    let mut rx = manager.subscribe();
    tokio::spawn(async move {
        let mut last = None;
        let mut last_prompts = Vec::new();
        loop {
            let state = startup_state(&manager);
            if last.as_ref() != Some(&state) {
                let _ = tx.send(Event::StartupProgress {
                    subsystem: Subsystem::Mcp,
                    state: state.clone(),
                });
                last = Some(state);
            }
            let prompts = prompt_commands(&manager);
            if prompts != last_prompts {
                let _ = tx.send(Event::McpPrompts {
                    prompts: prompts.clone(),
                });
                last_prompts = prompts;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    });
}

/// The ready servers' prompts as slash commands.
fn prompt_commands(manager: &McpManager) -> Vec<CommandInfo> {
    manager
        .prompts()
        .into_iter()
        .map(|p| CommandInfo {
            name: p.command(),
            description: p.description.clone(),
            args: p.usage(),
            source: format!("mcp: {}", p.server),
            path: String::new(),
        })
        .collect()
}

/// A prompt the user runs as `/<server>:<prompt> args`: the server, the prompt and its
/// arguments. `None` when the line is not an MCP prompt, `Some(Err)` when its arguments
/// don't fit.
pub(crate) type PromptCall = (String, String, serde_json::Map<String, Value>);

pub(crate) fn prompt_call(manager: &McpManager, line: &str) -> Option<Result<PromptCall, String>> {
    let rest = line.trim().strip_prefix('/')?;
    let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    if !name.contains(':') {
        return None;
    }
    let p = manager
        .prompts()
        .into_iter()
        .find(|p| p.command() == name)?;
    Some(
        p.parse_args(args)
            .map(|a| (p.server.clone(), p.name.clone(), a))
            .map_err(|e| format!("/{name}: {e}")),
    )
}

fn startup_state(manager: &McpManager) -> LoadState {
    let status = manager.status();
    if status.is_empty() {
        return LoadState::Disabled {
            reason: "no servers in mcp.d".into(),
        };
    }
    let ready = status
        .iter()
        .filter(|s| s.state == ServerState::Ready)
        .count();
    let tools: usize = status
        .iter()
        .filter(|s| s.state == ServerState::Ready)
        .map(|s| s.tools.len())
        .sum();
    let starting: Vec<&str> = status
        .iter()
        .filter(|s| s.state == ServerState::Starting)
        .map(|s| s.id.as_str())
        .collect();
    let failed: Vec<String> = status
        .iter()
        .filter_map(|s| match &s.state {
            ServerState::Failed(e) => Some(format!("{}: {e}", s.id)),
            _ => None,
        })
        .collect();
    let summary = format!(
        "{ready} of {} ready · {tools} tool{}",
        status.len(),
        if tools == 1 { "" } else { "s" }
    );
    if !starting.is_empty() {
        LoadState::Running {
            detail: format!("{summary} · connecting {}", starting.join(", ")),
        }
    } else if !failed.is_empty() {
        LoadState::NeedsAttention {
            detail: format!("{summary} · {}", failed.join(" · ")),
        }
    } else {
        LoadState::Ready { detail: summary }
    }
}

/// One MCP tool as the agent loop sees it.
#[derive(Debug, Clone)]
struct McpTool {
    info: ToolInfo,
    /// `mcp__<server>__<tool>`.
    name: String,
    manager: Arc<McpManager>,
}

impl McpTool {
    fn description(&self) -> String {
        let d = self.info.description.trim();
        if d.is_empty() {
            format!(
                "`{}` from the {} MCP server.",
                self.info.name, self.info.server
            )
        } else {
            format!("{d} (MCP server: {})", self.info.server)
        }
    }

    /// Estimated tokens of the definition (about 3.5 bytes per token).
    fn tokens(&self) -> u32 {
        let bytes = self.name.len() + self.description().len() + self.info.schema.to_string().len();
        u32::try_from(bytes * 2 / 7).unwrap_or(u32::MAX)
    }
}

impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> ToolKind {
        if self.info.read_only {
            ToolKind::ReadOnly
        } else {
            ToolKind::Mutating
        }
    }

    fn description(&self, _ctx: &ToolCtx) -> String {
        McpTool::description(self)
    }

    fn schema(&self) -> Value {
        self.info.schema.clone()
    }

    fn subject(&self, _args: &Value, _ctx: &ToolCtx) -> Result<Subject, String> {
        Ok(Subject::Mcp {
            server: self.info.server.clone(),
            tool: self.info.name.clone(),
            read_only: self.info.read_only,
            auto_approve: self.info.auto_approve,
        })
    }

    fn summary(&self, args: &Value, _ctx: &ToolCtx) -> String {
        let args = args.to_string();
        let args = if args == "{}" {
            String::new()
        } else if args.chars().count() > 60 {
            format!(" {}…", args.chars().take(60).collect::<String>())
        } else {
            format!(" {args}")
        };
        format!("{} · {}{args}", self.info.server, self.info.name)
    }

    fn preview(&self, args: &Value, _ctx: &ToolCtx) -> Result<ApprovalRequest, String> {
        Ok(ApprovalRequest::Tool {
            name: format!("{}/{} (MCP)", self.info.server, self.info.name),
            arguments: serde_json::to_string_pretty(args).unwrap_or_default(),
        })
    }

    fn run<'a>(&'a self, args: Value, ctx: &'a ToolCtx) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let out = self
                .manager
                .call(&self.info.server, &self.info.name, args, &ctx.cancel)
                .await;
            // One line, so JSON results read as `{ "a": 1, … }` rather than `{`.
            let first: String = out
                .text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(100)
                .collect();
            ToolOutput {
                ok: out.ok,
                images: Vec::new(),
                summary: if out.ok {
                    first
                } else {
                    format!("failed: {first}")
                },
                detail: Some(out.text.clone()),
                model: out.text,
            }
        })
    }
}

/// Finds deferred MCP tools and turns them on for the rest of the session.
#[derive(Debug)]
struct ToolSearch {
    mcp: Arc<Mcp>,
}

impl ToolSearch {
    fn search(&self, query: &str) -> Vec<McpTool> {
        let words: Vec<String> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 1)
            .map(str::to_lowercase)
            .collect();
        let mut scored: Vec<(usize, McpTool)> = self
            .mcp
            .adapters()
            .into_iter()
            .filter_map(|t| {
                let name = format!("{} {}", t.info.server, t.info.name).to_lowercase();
                let desc = t.info.description.to_lowercase();
                // A word in the name counts more than one in the description.
                let score: usize = words
                    .iter()
                    .map(|w| {
                        3 * usize::from(name.contains(w.as_str()))
                            + usize::from(desc.contains(w.as_str()))
                    })
                    .sum();
                (score > 0).then_some((score, t))
            })
            .collect();
        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
        scored
            .into_iter()
            .take(SEARCH_RESULTS)
            .map(|(_, t)| t)
            .collect()
    }
}

impl Tool for ToolSearch {
    fn name(&self) -> &str {
        "tool_search"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::ReadOnly
    }

    fn description(&self, _ctx: &ToolCtx) -> String {
        // Keep the description static over configured servers (never live ready state or
        // tool counts) so reconnecting or late-connecting servers never mutate the schema
        // and invalidate the model's prompt cache.
        let list: Vec<String> = self
            .mcp
            .manager
            .status()
            .into_iter()
            .map(|s| {
                if s.description.is_empty() {
                    s.id
                } else {
                    format!("{} ({})", s.id, s.description)
                }
            })
            .collect();
        if list.is_empty() {
            "Find tools on the connected MCP servers by what they do; the matches become \
             available for your next steps."
                .to_owned()
        } else {
            format!(
                "Find tools on the connected MCP servers by what they do; the matches become \
                 available for your next steps. Configured servers: {}.",
                list.join("; ")
            )
        }
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Words describing the tool, e.g. \"create issue\"" }
            },
            "required": ["query"]
        })
    }

    fn subject(&self, _args: &Value, _ctx: &ToolCtx) -> Result<Subject, String> {
        Ok(Subject::ToolSearch)
    }

    fn summary(&self, args: &Value, _ctx: &ToolCtx) -> String {
        format!(
            "tool_search {}",
            args.get("query")
                .and_then(Value::as_str)
                .unwrap_or_default()
        )
    }

    fn preview(&self, args: &Value, ctx: &ToolCtx) -> Result<ApprovalRequest, String> {
        Ok(ApprovalRequest::Tool {
            name: "tool_search".into(),
            arguments: self.summary(args, ctx),
        })
    }

    fn run<'a>(&'a self, args: Value, _ctx: &'a ToolCtx) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let query = args
                .get("query")
                .or_else(|| args.get("q"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let found = self.search(query);
            if found.is_empty() {
                let ready: Vec<String> = self
                    .mcp
                    .manager
                    .status()
                    .into_iter()
                    .filter(|s| s.state == ServerState::Ready)
                    .map(|s| {
                        if s.description.is_empty() {
                            format!("{} ({} tools)", s.id, s.tools.len())
                        } else {
                            format!("{} ({} tools: {})", s.id, s.tools.len(), s.description)
                        }
                    })
                    .collect();
                let servers = if ready.is_empty() {
                    "No MCP servers are currently ready.".to_owned()
                } else {
                    format!("Ready servers: {}.", ready.join("; "))
                };
                return ToolOutput::ok(
                    format!("No tools match `{query}`. Try other words. {servers}"),
                    "no matching tools",
                );
            }
            if let Ok(mut active) = self.mcp.activated.lock() {
                for t in &found {
                    if !active.contains(&t.name) {
                        active.push(t.name.clone());
                    }
                }
            }
            let lines: Vec<String> = found
                .iter()
                .map(|t| {
                    let d: String = t.info.description.chars().take(160).collect();
                    format!("- {}: {d}", t.name)
                })
                .collect();
            ToolOutput::ok(
                format!(
                    "These tools are now available; call them directly:\n{}",
                    lines.join("\n")
                ),
                match found.len() {
                    1 => "1 tool turned on".to_owned(),
                    n => format!("{n} tools turned on"),
                },
            )
        })
    }
}

/// Lists the resources of the connected MCP servers. Reads the lists the servers sent
/// when they connected, so it contacts nothing.
#[derive(Debug)]
struct ListResources {
    manager: Arc<McpManager>,
}

impl Tool for ListResources {
    fn name(&self) -> &str {
        "list_mcp_resources"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::ReadOnly
    }

    fn description(&self, _ctx: &ToolCtx) -> String {
        "List the resources (documents, records, files) the connected MCP servers offer, \
         to read with read_mcp_resource."
            .into()
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "server": { "type": "string", "description": "Only this server's resources" }
            }
        })
    }

    fn subject(&self, _args: &Value, _ctx: &ToolCtx) -> Result<Subject, String> {
        Ok(Subject::ToolSearch)
    }

    fn summary(&self, args: &Value, _ctx: &ToolCtx) -> String {
        match args.get("server").and_then(Value::as_str) {
            Some(s) if !s.is_empty() => format!("list_mcp_resources {s}"),
            _ => "list_mcp_resources".into(),
        }
    }

    fn preview(&self, args: &Value, ctx: &ToolCtx) -> Result<ApprovalRequest, String> {
        Ok(ApprovalRequest::Tool {
            name: "list_mcp_resources".into(),
            arguments: self.summary(args, ctx),
        })
    }

    fn run<'a>(&'a self, args: Value, _ctx: &'a ToolCtx) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let only = args
                .get("server")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            let found: Vec<_> = self
                .manager
                .resources()
                .into_iter()
                .filter(|r| only.is_none_or(|s| r.server == s))
                .collect();
            if found.is_empty() {
                return ToolOutput::ok(
                    match only {
                        Some(s) => format!("The MCP server `{s}` lists no resources."),
                        None => "No connected MCP server lists resources.".into(),
                    },
                    "no resources",
                );
            }
            let lines: Vec<String> = found
                .iter()
                .map(|r| {
                    let mut line = format!("- {} {}", r.server, r.uri);
                    let mut about: Vec<&str> = Vec::new();
                    if r.template {
                        about.push("template: fill in the {placeholders}");
                    }
                    for s in [&r.name, &r.mime_type] {
                        if !s.is_empty() && *s != r.uri {
                            about.push(s);
                        }
                    }
                    if !about.is_empty() {
                        line.push_str(&format!(" ({})", about.join(", ")));
                    }
                    let d: String = r.description.chars().take(160).collect();
                    if !d.is_empty() {
                        line.push_str(&format!(": {d}"));
                    }
                    line
                })
                .collect();
            let n = found.len();
            ToolOutput::ok(
                format!("server uri (details): description\n{}", lines.join("\n")),
                format!("{n} resource{}", if n == 1 { "" } else { "s" }),
            )
        })
    }
}

/// Reads one resource from an MCP server.
#[derive(Debug)]
struct ReadResource {
    manager: Arc<McpManager>,
}

impl ReadResource {
    /// The server for a call: as given, else the one that lists this URI, else the only
    /// server with resources.
    fn server(&self, args: &Value) -> Result<(String, String), String> {
        let uri = args
            .get("uri")
            .and_then(Value::as_str)
            .filter(|u| !u.is_empty())
            .ok_or("`uri` is required")?
            .to_owned();
        if let Some(s) = args
            .get("server")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            return Ok((s.to_owned(), uri));
        }
        let resources = self.manager.resources();
        if let Some(r) = resources.iter().find(|r| r.uri == uri) {
            return Ok((r.server.clone(), uri));
        }
        let mut servers: Vec<&str> = resources.iter().map(|r| r.server.as_str()).collect();
        servers.dedup();
        match servers.as_slice() {
            [one] => Ok(((*one).to_owned(), uri)),
            _ => Err(format!(
                "say which server: {} (list_mcp_resources shows what each has)",
                servers.join(", ")
            )),
        }
    }
}

impl Tool for ReadResource {
    fn name(&self) -> &str {
        "read_mcp_resource"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::ReadOnly
    }

    fn description(&self, _ctx: &ToolCtx) -> String {
        "Read a resource from a connected MCP server by its URI. list_mcp_resources shows \
         what each server offers; for a URI template, fill in its {placeholders}."
            .into()
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "server": { "type": "string", "description": "The MCP server" },
                "uri": { "type": "string", "description": "The resource URI" }
            },
            "required": ["server", "uri"]
        })
    }

    fn subject(&self, args: &Value, _ctx: &ToolCtx) -> Result<Subject, String> {
        let (server, _) = self.server(args)?;
        // Reading what the server offers to read: allowed unless a rule says otherwise
        // (`mcp:<server>/resources/read`).
        Ok(Subject::Mcp {
            server,
            tool: "resources/read".into(),
            read_only: true,
            auto_approve: true,
        })
    }

    fn summary(&self, args: &Value, _ctx: &ToolCtx) -> String {
        match self.server(args) {
            Ok((server, uri)) => format!("{server} · {uri}"),
            Err(_) => "read_mcp_resource".into(),
        }
    }

    fn preview(&self, args: &Value, ctx: &ToolCtx) -> Result<ApprovalRequest, String> {
        Ok(ApprovalRequest::Tool {
            name: "read_mcp_resource".into(),
            arguments: self.summary(args, ctx),
        })
    }

    fn run<'a>(&'a self, args: Value, _ctx: &'a ToolCtx) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let (server, uri) = match self.server(&args) {
                Ok(found) => found,
                Err(e) => return ToolOutput::error(e),
            };
            let out = self.manager.read_resource(&server, &uri).await;
            if !out.ok {
                return ToolOutput::error(out.text);
            }
            let chars = out.text.chars().count();
            ToolOutput {
                ok: true,
                images: Vec::new(),
                summary: format!("{chars} characters"),
                detail: Some(out.text.clone()),
                model: out.text,
            }
        })
    }
}

pub(crate) struct AddMcpInput {
    pub id: String,
    pub url: String,
    pub command: String,
    pub args: Vec<String>,
    pub bearer_env: Option<String>,
    pub bearer_token: Option<String>,
    pub project: bool,
}

/// Enable an MCP server on disk and return its path plus a summary with any unset env vars.
pub(crate) fn enable_mcp_in_config_detail(
    config: &Loaded,
    id: &str,
    env_exists: &dyn Fn(&str) -> bool,
) -> Result<(std::path::PathBuf, String), String> {
    let out = config.enable_mcp(id)?;
    let missing: Vec<&str> = out
        .env_vars
        .iter()
        .map(String::as_str)
        .filter(|v| !env_exists(v))
        .collect();
    let msg = if missing.is_empty() {
        format!("{} ({}); reloading…", out.action, out.file.display())
    } else {
        format!(
            "{} ({}) — set {} in your environment if needed; reloading…",
            out.action,
            out.file.display(),
            missing.join(", ")
        )
    };
    Ok((out.file, msg))
}

/// Enable an MCP server on disk and summarize what changed plus any unset env vars.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn enable_mcp_in_config(
    config: &Loaded,
    id: &str,
    env_exists: &dyn Fn(&str) -> bool,
) -> Result<String, String> {
    enable_mcp_in_config_detail(config, id, env_exists).map(|(_, msg)| msg)
}

/// Disable an MCP server on disk without removing its config.
pub(crate) fn disable_mcp_in_config(config: &Loaded, id: &str) -> Result<String, String> {
    let out = config.disable_mcp(id)?;
    Ok(format!(
        "{} ({}); reloading…",
        out.action,
        out.path.display()
    ))
}

/// Write a new user-layer `mcp.d/<id>.toml` file and store any bearer secret safely.
pub(crate) fn add_mcp_in_config(config: &Loaded, input: AddMcpInput) -> Result<String, String> {
    use std::collections::BTreeMap;

    use quack_config::{NewMcpServer, NewMcpTransport, Secret};

    let transport = if !input.url.trim().is_empty() {
        let mut headers = BTreeMap::new();
        if let Some(env_var) = input
            .bearer_env
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            headers.insert(
                "Authorization".into(),
                Secret::Env {
                    env: env_var.to_owned(),
                    format: Some("Bearer {}".into()),
                },
            );
        } else if let Some(tok) = input
            .bearer_token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let secret = store_mcp_bearer(config, &input.id, tok)?;
            headers.insert("Authorization".into(), secret);
        }
        NewMcpTransport::Http {
            url: input.url.trim().to_owned(),
            headers,
        }
    } else {
        NewMcpTransport::Stdio {
            command: input.command.trim().to_owned(),
            args: input.args,
            env: BTreeMap::new(),
            cwd: None,
            sandbox: None,
        }
    };

    let spec = NewMcpServer {
        id: input.id.clone(),
        description: None,
        transport,
        auto_approve: Vec::new(),
        startup_timeout_secs: None,
        project: input.project,
        overwrite: false,
    };
    let path = config.add_mcp_server(&spec)?;
    Ok(format!(
        "added MCP server `{}` ({}); reloading…",
        input.id,
        path.display()
    ))
}

fn store_mcp_bearer(
    config: &Loaded,
    id: &str,
    token: &str,
) -> Result<quack_config::Secret, String> {
    let header_val = if token.to_ascii_lowercase().starts_with("bearer ") {
        token.to_owned()
    } else {
        format!("Bearer {token}")
    };
    let store_id = format!("mcp-{id}");
    match crate::setup::key_store(config, &store_id)? {
        (crate::setup::KeyStore::Keyring, _) => {
            quack_config::keyring_set(&store_id, &header_val).map_err(|e| e.to_string())?;
            Ok(quack_config::Secret::Keyring { keyring: store_id })
        }
        (crate::setup::KeyStore::File(path), _) => {
            crate::setup::write_key_file(&path, &header_val)?;
            Ok(quack_config::Secret::File {
                file: path.display().to_string(),
            })
        }
    }
}

#[cfg(test)]
mod manage_tests {
    use super::*;
    use quack_config::{LoadOptions, load_with};
    use std::collections::HashMap;
    use std::ffi::OsString;

    struct Fixture {
        _tmp: tempfile::TempDir,
        root: std::path::PathBuf,
        vars: HashMap<String, OsString>,
    }

    impl Fixture {
        fn new() -> Self {
            let tmp = tempfile::tempdir().expect("tempdir");
            let root = tmp.path().to_path_buf();
            std::fs::create_dir_all(root.join("repo")).expect("repo");
            let mut vars = HashMap::new();
            vars.insert(
                "QUACK_DEFAULTS_DIR".into(),
                root.join("defaults").into_os_string(),
            );
            vars.insert(
                "QUACK_CONFIG_DIR".into(),
                root.join("user").into_os_string(),
            );
            vars.insert("QUACK_DATA_DIR".into(), root.join("data").into_os_string());
            Self {
                _tmp: tmp,
                root,
                vars,
            }
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.root.join(rel);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, text).expect("write");
        }

        fn load(&self) -> Loaded {
            let opts = LoadOptions::new(self.root.join("repo"));
            let vars = self.vars.clone();
            load_with(&move |k| vars.get(k).cloned(), &opts).expect("load")
        }
    }

    #[test]
    fn enable_disable_and_add_mcp_helpers_work_and_remind_missing_env() {
        let f = Fixture::new();
        f.write(
            "defaults/mcp.d/roost.toml",
            "enabled = false\n\
             url = \"https://roost.internal:8484/mcp\"\n\
             headers = { Authorization = { env = \"ROOST_TOKEN\", format = \"Bearer {}\" } }\n",
        );

        let msg = enable_mcp_in_config(&f.load(), "roost", &|_| false).expect("enable");
        assert!(msg.contains("ROOST_TOKEN"), "{msg}");

        let msg = disable_mcp_in_config(&f.load(), "roost").expect("disable");
        assert!(msg.contains("disabled"), "{msg}");

        let added = add_mcp_in_config(
            &f.load(),
            AddMcpInput {
                id: "context7".into(),
                url: "https://mcp.context7.com/mcp".into(),
                command: String::new(),
                args: Vec::new(),
                bearer_env: Some("CONTEXT7_API_KEY".into()),
                bearer_token: None,
                project: false,
            },
        )
        .expect("add");
        assert!(added.contains("context7"), "{added}");
        let saved = std::fs::read_to_string(f.root.join("user/mcp.d/context7.toml")).expect("read");
        assert!(saved.contains("https://mcp.context7.com/mcp"), "{saved}");
        assert!(saved.contains("CONTEXT7_API_KEY"), "{saved}");

        let proj_added = add_mcp_in_config(
            &f.load(),
            AddMcpInput {
                id: "proj".into(),
                url: String::new(),
                command: "npx".into(),
                args: vec!["-y".into(), "proj-mcp".into()],
                bearer_env: None,
                bearer_token: None,
                project: true,
            },
        )
        .expect("add project");
        assert!(proj_added.contains("proj"), "{proj_added}");
        assert!(f.root.join("repo/.quack/mcp.d/proj.toml").is_file());
        assert_eq!(f.load().project, quack_config::ProjectStatus::Trusted);
    }
}
