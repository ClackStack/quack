//! Data models and state types owned by `App`.

use super::*;

/// One configuration source shown by `/config`, with its file when it has one.
#[derive(Debug, Clone)]
pub struct ConfigLayer {
    pub layer: String,
    pub status: String,
    pub path: Option<String>,
}

/// Static facts the UI shows, captured once at startup.
#[derive(Debug, Clone)]
pub struct AppInfo {
    pub version: String,
    pub cwd: String,
    /// The checked-out git branch, or a short commit when detached.
    pub branch: Option<String>,
    pub model: Option<String>,
    pub default_model: Option<String>,
    pub small_model: Option<String>,
    pub default_locked: bool,
    pub small_locked: bool,
    pub profile: String,
    pub permission_mode: String,
    pub work_mode: WorkMode,
    pub ui: UiSettings,
    pub no_color: bool,
    /// Effective settings for `/config`.
    pub settings: Vec<SettingRow>,
    /// Canonical-display path to `.quack/config.toml` for matching saved-setting origins.
    pub project_config: String,
    /// Trust state of this workspace's `.quack/`.
    pub project_status: ProjectStatus,
    /// Configuration sources for `/config`.
    pub layers: Vec<ConfigLayer>,
    /// `(list, rule)` from `[permissions]`, e.g. `("allow", "shell:git status*")`.
    pub rules: Vec<(String, String)>,
    /// `grammars/` folders of the active layers, lowest first, for highlighting.
    pub grammar_dirs: Vec<std::path::PathBuf>,
    /// Installed bundles, for `/bundles`.
    pub bundles: Vec<quack_config::Bundle>,
    /// `[bundles] disabled` as configured, including names not installed.
    pub bundles_disabled: Vec<String>,
    /// Configured hooks from `hooks.d`, for `/hooks`.
    pub hooks: Vec<quack_config::Item<quack_config::HookFile>>,
    /// `[hooks] disabled` as configured, including names not installed.
    pub hooks_disabled: Vec<String>,
    /// Configured and bundled MCP servers, for `/mcp`.
    pub mcp_items: Vec<quack_config::McpSummary>,
    /// Prompt history loaded for this project at startup.
    pub history: Vec<String>,
    /// Where new prompt history entries for this project are appended.
    pub history_path: Option<std::path::PathBuf>,
}

impl AppInfo {
    pub fn from_config(config: &Loaded) -> Self {
        let root = config.dirs.workspace_root.display().to_string();
        let home =
            std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default();
        let cwd = match root.strip_prefix(&home) {
            Some(rest) if !home.is_empty() => format!("~{rest}"),
            _ => root,
        };
        let history_path = quack_core::history_path(config);
        let history = quack_core::load_history(&history_path);
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            branch: git_branch(&config.dirs.workspace_root),
            cwd,
            model: Some(config.settings.model.default.clone()).filter(|m| !m.is_empty()),
            default_model: Some(config.settings.model.default.clone()).filter(|m| !m.is_empty()),
            small_model: Some(config.settings.model.small.clone()).filter(|m| !m.is_empty()),
            default_locked: config.settings.policy.locks("model.default"),
            small_locked: config.settings.policy.locks("model.small"),
            profile: "empty".into(),
            permission_mode: config.settings.permissions.mode.label().to_owned(),
            work_mode: config.settings.permissions.work_mode,
            ui: config.settings.ui.clone(),
            no_color: std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()),
            settings: config.setting_rows(),
            project_config: config
                .dirs
                .project
                .join("config.toml")
                .display()
                .to_string(),
            project_status: config.project,
            grammar_dirs: config
                .item_layers()
                .into_iter()
                .map(|(_, d)| d.join("grammars"))
                .collect(),
            bundles: config.bundles.clone(),
            bundles_disabled: config.settings.bundles.disabled.clone(),
            hooks: quack_config::load_hooks(config),
            hooks_disabled: config.settings.hooks.disabled.clone(),
            mcp_items: config.mcp_summaries(),
            history,
            history_path: Some(history_path),
            rules: {
                let p = &config.settings.permissions;
                [("deny", &p.deny), ("ask", &p.ask), ("allow", &p.allow)]
                    .into_iter()
                    .flat_map(|(k, list)| list.iter().map(move |r| (k.to_owned(), r.clone())))
                    .collect()
            },
            layers: config
                .layers
                .iter()
                .map(|l| {
                    let status = match &l.status {
                        LayerStatus::Loaded => "loaded".to_owned(),
                        LayerStatus::Missing => "no config.toml".to_owned(),
                        LayerStatus::Failed => "failed to parse (see config check)".to_owned(),
                        LayerStatus::Skipped(why) => format!("skipped: {why}"),
                    };
                    let path = l
                        .dir
                        .as_ref()
                        .map(|d| d.join("config.toml").display().to_string());
                    ConfigLayer {
                        layer: l.kind.label().to_owned(),
                        status,
                        path,
                    }
                })
                .collect(),
        }
    }
}

/// Turns that fit in `room` at the typical growth of the last few turns. Compactions show
/// up as shrinking turns and are left out.
pub(crate) fn turns_left(growth: &[u32], room: u32) -> Option<u32> {
    let mut steps: Vec<u32> = growth
        .windows(2)
        .rev()
        .take(6)
        .filter_map(|w| w[1].checked_sub(w[0]).filter(|d| *d > 0))
        .collect();
    if steps.is_empty() {
        return None;
    }
    steps.sort_unstable();
    Some(room / steps[steps.len() / 2])
}

/// The branch in `.git/HEAD` of `dir` or a parent; worktrees and submodules point at
/// their git dir with a `gitdir:` file.
pub(crate) fn git_branch(dir: &std::path::Path) -> Option<String> {
    let git = dir
        .ancestors()
        .map(|d| d.join(".git"))
        .find(|g| g.exists())?;
    let git = if git.is_file() {
        let link = std::fs::read_to_string(&git).ok()?;
        let target = link.strip_prefix("gitdir:")?.trim();
        git.parent()?.join(target)
    } else {
        git
    };
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    Some(match head.strip_prefix("ref: refs/heads/") {
        Some(branch) => branch.to_owned(),
        None => head.chars().take(7).collect(),
    })
}

/// A row of the popup: a built-in command, custom command, MCP prompt, skill, or `@file`.
#[derive(Debug, Clone)]
pub(crate) struct PopupItem {
    pub prefix: char,
    pub name: String,
    pub args: String,
    pub description: String,
    /// Shown after the description, e.g. a milestone, `skill`, or `file`.
    pub hint: Option<&'static str>,
    /// `None` for skills, custom commands, MCP prompts, and `@file` items.
    pub command: Option<&'static Command>,
}

/// What `popup_matches` was last computed from; any difference means a recompute.
#[derive(Debug, Default, PartialEq)]
pub(super) struct PopupKey {
    pub(super) text: String,
    pub(super) modal_open: bool,
    pub(super) dismissed: bool,
    /// Bumped when the files, skills, custom commands or MCP prompts change.
    pub(super) epoch: u64,
}

/// The last popup match list, and what it was computed from.
#[derive(Debug, Default)]
pub(super) struct PopupCache {
    pub(super) key: PopupKey,
    pub(super) items: Vec<PopupItem>,
}

impl PopupItem {
    pub(super) fn command(c: &'static Command) -> Self {
        Self {
            prefix: '/',
            name: c.name.to_owned(),
            args: c.args.to_owned(),
            description: c.description.to_owned(),
            hint: c.pending,
            command: Some(c),
        }
    }
}

/// A hook execution reported during this session (`Event::HookRan`), for `/hooks`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HookRun {
    pub id: String,
    pub on: String,
    pub outcome: String,
    pub ms: u64,
    pub message: Option<String>,
    pub context_tokens: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Entry {
    User(String),
    Assistant(String),
    Reasoning(String),
    Tool(Box<ToolEntry>),
    System(String),
    Warning(String),
    Error(String),
    /// What a finished turn took, shown as a dim rule.
    TurnSummary(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolEntry {
    pub call_id: CallId,
    pub name: String,
    pub summary: String,
    /// `None` while running.
    pub ok: Option<bool>,
    pub result: String,
    /// Live output while running; the full diff or output once done.
    pub detail: String,
    pub started: Option<Instant>,
    pub took: Option<Duration>,
}

/// A plan item's state, from the `todo` tool's `[x]`/`[~]`/`[ ]` marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlanState {
    Done,
    Active,
    Pending,
}

/// Plan items from the `todo` tool's rendered checklist.
pub(crate) fn parse_plan(text: &str) -> Vec<(PlanState, String)> {
    text.lines()
        .filter_map(|l| {
            let (mark, rest) = (l.get(..3)?, l.get(3..)?.trim());
            let state = match mark {
                "[x]" => PlanState::Done,
                "[~]" => PlanState::Active,
                "[ ]" => PlanState::Pending,
                _ => return None,
            };
            Some((state, rest.to_owned()))
        })
        .collect()
}

/// Something a sub-agent did: text it wrote, or a tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentItem {
    Text(String),
    Tool(ToolEntry),
}

/// A sub-agent run (`task`), from its events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentRun {
    /// The run, e.g. `explore-2`.
    pub id: String,
    pub agent: String,
    pub description: String,
    pub model: String,
    /// The `task` call that started it.
    pub call_id: CallId,
    /// `None` while running.
    pub ok: Option<bool>,
    pub summary: String,
    pub started: Instant,
    pub took: Option<Duration>,
    pub items: Vec<AgentItem>,
}

/// A streaming delta goes onto the last entry when it is the same kind, or starts a new
/// one with `new`. Shared by the transcript and sub-agent runs; `new` decides how a fresh
/// entry shapes the text (the transcript trims leading whitespace, agents keep it).
pub(super) fn append_delta<T>(
    items: &mut Vec<T>,
    delta: &str,
    as_text: impl Fn(&mut T) -> Option<&mut String>,
    new: impl FnOnce(&str) -> T,
) {
    match items.last_mut().and_then(as_text) {
        Some(buf) => buf.push_str(delta),
        None => items.push(new(delta)),
    }
}

impl AgentRun {
    pub(super) fn on_event(&mut self, e: Event) {
        match e {
            Event::AssistantDelta { text } => append_delta(
                &mut self.items,
                &text,
                |i| match i {
                    AgentItem::Text(t) => Some(t),
                    _ => None,
                },
                |t| AgentItem::Text(t.to_owned()),
            ),
            Event::AssistantRewrite { text } => {
                if let Some(AgentItem::Text(t)) = self.items.last_mut() {
                    *t = text;
                }
            }
            Event::ToolCallBegin { call } => self.items.push(AgentItem::Tool(ToolEntry {
                call_id: call.call_id,
                name: call.name,
                summary: call.summary,
                ok: None,
                result: String::new(),
                detail: String::new(),
                started: Some(Instant::now()),
                took: None,
            })),
            Event::ToolCallEnd { call_id, result } => {
                if let Some(t) = self.items.iter_mut().rev().find_map(|i| match i {
                    AgentItem::Tool(t) if t.call_id == call_id => Some(t),
                    _ => None,
                }) {
                    t.ok = Some(result.ok);
                    t.result = result.summary;
                    t.detail = result.detail.unwrap_or_default();
                    t.took = t.started.map(|s| s.elapsed());
                }
            }
            Event::Warning { message } => self.items.push(AgentItem::Text(format!("! {message}"))),
            Event::SessionResumeFailed { reason } => {
                self.items.push(AgentItem::Text(format!("! {reason}")))
            }
            Event::Error { error } => {
                self.items
                    .push(AgentItem::Text(format!("! {}", error.message)));
            }
            _ => {}
        }
    }

    /// Its calls so far.
    pub(crate) fn steps(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i, AgentItem::Tool(_)))
            .count()
    }
}

/// An `ask_user` call's questions, answered one at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Asking {
    pub call_id: CallId,
    pub questions: Vec<Question>,
    pub current: usize,
    /// The highlighted row; the options' count is the "own answer" row.
    pub selected: usize,
    pub answers: Vec<Answer>,
}

impl Asking {
    pub(super) fn new(call_id: CallId, questions: Vec<Question>) -> Self {
        let mut a = Self {
            call_id,
            answers: vec![Answer::Skipped; questions.len()],
            questions,
            current: 0,
            selected: 0,
        };
        a.select_default();
        a
    }

    pub(crate) fn question(&self) -> &Question {
        &self.questions[self.current]
    }

    /// The recommended option, else the first (or the own-answer row when there are none).
    pub(super) fn select_default(&mut self) {
        self.selected = self.question().recommended.unwrap_or(0);
    }
}

/// What the plan review bar offers once a plan-mode turn ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewChoice {
    Accept,
    KeepPlanning,
}

/// A one-line question `/setup` or `/mcp add` asks.
pub(crate) struct Prompt {
    pub title: String,
    pub label: String,
    pub help: Vec<String>,
    /// Draw the text as dots.
    pub secret: bool,
    pub text: String,
    /// Why the last answer was not taken.
    pub error: Option<String>,
    /// Active write scope when adding an MCP server (`^l` toggles).
    pub scope: Option<WriteScope>,
    pub(super) then: PromptThen,
}

#[derive(Debug)]
pub(super) enum PromptThen {
    ServerUrl,
    ServerKey {
        url: String,
    },
    ServiceKey {
        id: String,
    },
    /// `/key`: replace the key of a provider that may be in use.
    NewKey {
        id: String,
    },
    McpTransport,
    McpHttpUrl,
    McpHttpId {
        url: String,
    },
    McpHttpAuth {
        id: String,
        url: String,
    },
    McpStdioCmd,
    McpStdioId {
        command: String,
        args: Vec<String>,
    },
}

/// Never shows the text: it may be a key.
impl std::fmt::Debug for Prompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Prompt")
            .field("title", &self.title)
            .field("then", &self.then)
            .finish_non_exhaustive()
    }
}

impl Prompt {
    /// What Enter does: send the answer, or ask the next question.
    pub(crate) fn enter_does(&self) -> &'static str {
        match self.then {
            PromptThen::ServerUrl
            | PromptThen::McpTransport
            | PromptThen::McpHttpUrl
            | PromptThen::McpHttpId { .. }
            | PromptThen::McpStdioCmd => "next",
            PromptThen::NewKey { .. } => "replace",
            _ => "add",
        }
    }

    pub(super) fn server_url() -> Self {
        Self {
            title: "Add a server · 1 of 2".into(),
            label: "Base URL".into(),
            help: vec![
                "The address of any OpenAI-compatible server, usually ending in /v1, for \
                 example http://gpu-box:8000/v1."
                    .into(),
            ],
            secret: false,
            text: String::new(),
            error: None,
            scope: None,
            then: PromptThen::ServerUrl,
        }
    }

    /// What happens to a saved key where no Secret Service is running.
    const KEY_STORE_HELP: &str = "Without a Secret Service (headless Linux, WSL, a server), \
         the key is kept in a file only your user can read instead, and the provider file \
         points at it.";

    pub(super) fn server_key(url: String) -> Self {
        Self {
            title: "Add a server · 2 of 2".into(),
            label: "API key".into(),
            help: vec![
                format!(
                    "The key for {url}, if it needs one. Leave it empty for a server on your own network that takes none."
                ),
                "It goes to the OS credential store, not into the provider file.".into(),
                Self::KEY_STORE_HELP.into(),
            ],
            secret: true,
            text: String::new(),
            error: None,
            scope: None,
            then: PromptThen::ServerKey { url },
        }
    }

    pub(super) fn new_key(id: &str, source: Option<&KeySource>) -> Self {
        let mut help = vec![format!(
            "Paste the new API key for `{id}`, for example after rotating it at the provider."
        )];
        help.push(match source {
            Some(KeySource::Keyring { name }) => format!(
                "It replaces the key stored in the OS credential store as `{name}`; the \
                 provider file stays as it is."
            ),
            Some(source) => format!(
                "Its key is read from {} now. The new one goes to the OS credential store \
                 instead, and the provider file in your config folder is pointed at it.",
                modal::key_source(source)
            ),
            None => "It goes to the OS credential store, and the provider file in your \
                     config folder is pointed at it."
                .into(),
        });
        if let Some(KeySource::Env { var }) = source {
            help.push(format!(
                "To keep using {var}, press esc, change it in your shell and start quack again."
            ));
        }
        help.push(Self::KEY_STORE_HELP.into());
        help.push("This session sends the new key from its next request.".into());
        Self {
            title: format!("New {id} key"),
            label: "API key".into(),
            help,
            secret: true,
            text: String::new(),
            error: None,
            scope: None,
            then: PromptThen::NewKey { id: id.to_owned() },
        }
    }

    pub(super) fn service_key(service: Option<&quack_protocol::ServiceInfo>, id: &str) -> Self {
        let name = service.map_or(id, |s| s.name.as_str());
        let mut help = vec![format!(
            "Paste your {name} API key. quack keeps it in the OS credential store under \
             `{id}`; the provider file in your config folder only names it."
        )];
        if let Some(env) = service.and_then(|s| s.env.as_ref()) {
            help.push(format!(
                "To use the {env} environment variable instead, press esc, set it in your shell and start quack again."
            ));
        }
        help.push(Self::KEY_STORE_HELP.into());
        Self {
            title: format!("{name} API key"),
            label: "API key".into(),
            help,
            secret: true,
            text: String::new(),
            error: None,
            scope: None,
            then: PromptThen::ServiceKey { id: id.to_owned() },
        }
    }

    pub(super) fn mcp_transport(scope: WriteScope) -> Self {
        Self {
            title: "Add an MCP server".into(),
            label: "Transport (1 = remote HTTP, 2 = local command)".into(),
            help: vec![
                "Choose `1` / `http` for a remote endpoint URL (e.g. Context7, Roost), or \
                 `2` / `stdio` for a local process (e.g. npx, uvx)."
                    .into(),
            ],
            secret: false,
            text: String::new(),
            error: None,
            scope: Some(scope),
            then: PromptThen::McpTransport,
        }
    }

    pub(super) fn mcp_http_url(scope: WriteScope) -> Self {
        Self {
            title: "Add remote MCP server · 1 of 3".into(),
            label: "Endpoint URL".into(),
            help: vec![
                "Must start with http:// or https://, e.g. https://mcp.context7.com/mcp".into(),
            ],
            secret: false,
            text: String::new(),
            error: None,
            scope: Some(scope),
            then: PromptThen::McpHttpUrl,
        }
    }

    pub(super) fn mcp_http_id(url: String, scope: WriteScope) -> Self {
        let suggested = quack_config::derive_mcp_id_from_url(&url);
        let hint = match &suggested {
            Some(s) => format!(
                "Leave empty to use `{s}`, or enter a custom id (letters, digits, `-`, `_`)."
            ),
            None => "Enter a short id using ASCII letters, digits, `-`, or `_`.".into(),
        };
        Self {
            title: "Add remote MCP server · 2 of 3".into(),
            label: "Server id".into(),
            help: vec![hint],
            secret: false,
            text: suggested.unwrap_or_default(),
            error: None,
            scope: Some(scope),
            then: PromptThen::McpHttpId { url },
        }
    }

    pub(super) fn mcp_http_auth(id: String, url: String, scope: WriteScope) -> Self {
        Self {
            title: "Add remote MCP server · 3 of 3".into(),
            label: "Bearer token or env:VAR (optional)".into(),
            help: vec![
                "Type env:VAR_NAME (for example env:ROOST_TOKEN), paste a Bearer token to store \
                 in the OS credential store / key file, or press Enter for no auth header."
                    .into(),
            ],
            secret: true,
            text: String::new(),
            error: None,
            scope: Some(scope),
            then: PromptThen::McpHttpAuth { id, url },
        }
    }

    pub(super) fn mcp_stdio_cmd(scope: WriteScope) -> Self {
        Self {
            title: "Add local MCP server · 1 of 2".into(),
            label: "Command and args".into(),
            help: vec!["Example: npx -y @playwright/mcp@latest   or   uvx mcp-server-fetch".into()],
            secret: false,
            text: String::new(),
            error: None,
            scope: Some(scope),
            then: PromptThen::McpStdioCmd,
        }
    }

    pub(super) fn mcp_stdio_id(command: String, args: Vec<String>, scope: WriteScope) -> Self {
        let suggested = quack_config::derive_mcp_id_from_command(&command, &args);
        let hint = match &suggested {
            Some(s) => format!(
                "Leave empty to use `{s}`, or enter a custom id (letters, digits, `-`, `_`)."
            ),
            None => "Enter a short id using ASCII letters, digits, `-`, or `_`.".into(),
        };
        Self {
            title: "Add local MCP server · 2 of 2".into(),
            label: "Server id".into(),
            help: vec![hint],
            secret: false,
            text: suggested.unwrap_or_default(),
            error: None,
            scope: Some(scope),
            then: PromptThen::McpStdioId { command, args },
        }
    }
}

/// A tool call waiting for the user's decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Approval {
    pub call_id: CallId,
    pub request: ApprovalRequest,
    /// The rules `A` (always allow) saves; empty when it isn't offered.
    pub always: Vec<String>,
    /// Where `A` saves the rule (`.quack/` when trusted/settings-only, or user config).
    pub always_scope: WriteScope,
}

/// A turn paused at `agent.max_steps`, waiting for the user's go-ahead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StepLimit {
    pub turn: TurnId,
    pub limit: u32,
    /// The highlighted choice: continue, or stop here.
    pub selected: usize,
}

/// An image attached to the next message: the path the engine loads, and the size its
/// header gave, for the chip.
#[derive(Debug, Clone)]
pub(crate) struct Attachment {
    pub path: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}
