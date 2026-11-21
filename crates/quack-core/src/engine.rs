use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quack_config::hooks::HookEvent;
use quack_config::{LayerKind, Loaded, NewMcpServer, NewMcpTransport, Secret};
use quack_lsp::LspManager;
use quack_mcp::{McpManager, ServerAsk};
use quack_protocol::PermissionMode;
use quack_protocol::{
    ApprovalPolicy, CallId, EngineError, EngineErrorKind, Event, ExportFormat, InputItem,
    InstructionFile, LspDiagnosticItem, McpSecretValue, Op, SessionId, SessionInfo, SessionSummary,
    TurnId, WorkMode, WriteScope,
};
use quack_providers::{EgressLedger, Message, Provider, StreamEvent};
use quack_tools::Permissions;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

use crate::handoff;
use crate::hooks::{Hooks, Sink, Target};
use crate::session::{self, Resume, SharedLog};
use crate::startup::{self, Setup, StartupOutcome};
use crate::turn::{self, Approvals, Questions, TurnCtx};
use quack_instructions::agents_md::Nested;
use quack_protocol::MemoryNote;
use quack_store::checkpoint::Checkpoints;
use quack_store::memory::Scope;

const EVENT_CAPACITY: usize = 4096;

/// Appended to user messages in plan mode when the profile has no `plan.md`.
const PLAN_PROMPT: &str = "[Plan mode] Do not change anything yet. Explore with read-only \
tools first. If a decision would change the plan in a big way (scope, approach, a trade-off, \
an unclear requirement), ask the user with ask_user: put the questions in one call, give \
options with your recommendation first, and don't ask what the code or docs can answer. Ask \
only a few, and none when the task is clear. Then record the plan with the todo tool (concrete \
steps, all pending) and reply with a short summary: the decisions made, which files change, \
and how to check the result. The user will accept the plan or ask for changes.";
const OP_CAPACITY: usize = 64;

/// A client's connection to a running engine.
mod modes;
mod reload_ops;
mod session_ops;
mod setup_ops;
mod title;
mod turn_ops;

use modes::plan_note;
use reload_ops::{Reload, wire_ledger};
use session_ops::new_session_id;
use setup_ops::{AddMcpFields, SavedMcpFields};

#[derive(Debug)]
pub struct EngineHandle {
    /// Sends client operations to the engine, usually through [`submit`](Self::submit).
    pub ops: mpsc::Sender<Op>,
    /// Receives the engine's event stream; clients wait on it to update their state.
    pub events: broadcast::Receiver<Event>,
    tx: broadcast::Sender<Event>,
    config: watch::Receiver<Arc<Loaded>>,
    task: JoinHandle<()>,
}

impl EngineHandle {
    /// Another receiver for the same event stream (e.g. a logger next to the UI).
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    /// The config in use; it changes after `Event::ConfigReloaded`.
    pub fn config(&self) -> Arc<Loaded> {
        self.config.borrow().clone()
    }

    pub async fn submit(&self, op: Op) -> bool {
        self.ops.send(op).await.is_ok()
    }

    /// Ask the engine to stop and wait for it.
    pub async fn shutdown(self) {
        let _ = self.ops.send(Op::Shutdown).await;
        let _ = self.task.await;
    }
}

#[derive(Debug)]
struct RewindOpts {
    conversation: bool,
    files: bool,
    force: bool,
}

#[derive(Debug)]
struct SaveMemoryArgs {
    scope: String,
    name: String,
    description: String,
    kind: String,
    body: String,
    previous_scope: Option<String>,
    previous_name: Option<String>,
}

struct TurnLaunch {
    done: mpsc::Sender<(TurnId, Vec<Message>)>,
    history: Vec<Message>,
    mcp_prompt: Option<crate::mcp::PromptCall>,
    typed: String,
    expanded: Option<String>,
    files: Vec<String>,
    images: Vec<quack_protocol::Image>,
    start: Option<JoinHandle<Vec<String>>>,
}

/// Work that waits for startup or the running turn.
#[derive(Debug)]
enum Queued {
    Turn(Vec<InputItem>),
    /// Resume a saved session, or start a new one with `None`.
    Switch(Option<SessionId>),
    /// `/compact`, with an optional focus.
    Compact(Option<String>),
    /// `/prune` or manual context trim.
    Prune(Option<Vec<usize>>),
    /// `/reload`, or a trust decision.
    Reload(Option<(String, String)>),
    /// `/plan accept`, into this mode.
    Accept(String),
    /// `/fork`.
    Fork,
    /// `/fork --worktree [name]`.
    ForkWorktree(Option<String>),
    HookSave(Box<Op>),
    ReasoningEffort(Option<String>),
}

/// The conversation once a model is ready.
#[derive(Debug)]
struct Conversation {
    setup: Arc<Setup>,
    perms: Arc<Mutex<Permissions>>,
    /// Nested AGENTS.md files already shown in this session.
    nested: Arc<Nested>,
    /// `None` while a turn task owns it.
    history: Option<Vec<Message>>,
    /// The session's own setup, while plan mode uses the `[model] planner`'s.
    editor: Option<Arc<Setup>>,
    /// History length at the planner's first turn: its messages come after it.
    plan_start: Option<usize>,
    /// Last server response id seen in this conversation (Responses cache diagnostics).
    last_response_id: Arc<Mutex<Option<String>>>,
    /// Last plan / working-files reminders written into append-only history.
    last_reminders: Arc<Mutex<turn::ReminderState>>,
    /// Remote Secret Shield vetting state across turns and model switches.
    egress_guard: Arc<Mutex<turn::EgressGuardState>>,
}

impl Conversation {
    /// Clear transient per-session / per-model tracking when switching sessions, rewinding,
    /// or changing models.
    fn reset_transient(&mut self) {
        self.last_response_id = Arc::default();
        self.last_reminders = Arc::default();
        self.egress_guard = Arc::default();
    }
}

/// Which slot a model switch fills: the session's model, or the planner in plan mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Session,
    Planner,
}

/// Nested instruction tracking for a session: the startup chain, plus any nested file
/// already attached to a tool result in `history` (after a resume).
fn nested(config: &Loaded, setup: &Setup, history: &[Message]) -> Arc<Nested> {
    let n = Nested::new(
        &config.dirs.workspace_root,
        crate::env::rules(config),
        setup.instruction_dirs.iter().cloned(),
    );
    for m in history {
        if let Message::Tool { content, .. } = m {
            n.mark_shown(content);
        }
    }
    Arc::new(n)
}

#[derive(Debug)]
pub struct Engine {
    config: Arc<Loaded>,
    config_tx: watch::Sender<Arc<Loaded>>,
    /// A reload is reading files or rebuilding the session; turns wait for it.
    reloading: bool,
    /// The planner's setup is being built; turns wait for it.
    switching: bool,
    tx: broadcast::Sender<Event>,
    session: SessionInfo,
    reasoning_efforts: BTreeMap<String, String>,
    started: bool,
    problem: Option<String>,
    convo: Option<Conversation>,
    next_turn: u64,
    queued: VecDeque<Queued>,
    running: Option<(TurnId, CancellationToken)>,
    approvals: Approvals,
    questions: Questions,
    continues: turn::Continues,
    resume: Resume,
    /// Created at the first turn (no empty sessions) or reopened on resume.
    log: Option<SharedLog>,
    turns: u32,
    title: Option<String>,
    title_tasks: JoinSet<(SessionId, Result<String, String>)>,
    switch_tx: Option<mpsc::Sender<(Role, Result<Setup, String>)>>,
    /// A mode set before startup finished, applied once a model is ready.
    pending_mode: Option<PermissionMode>,
    pending_work_mode: Option<WorkMode>,
    pending_approvals: Option<ApprovalPolicy>,
    saved_mode: Option<(WorkMode, ApprovalPolicy)>,
    saved_plan_start: Option<usize>,
    /// A `/sandbox` setting sent before startup finished.
    pending_sandbox: Option<bool>,
    /// Deferred scoped config-watcher changes that arrived while a turn was running.
    pending_watch: Option<crate::watch::WatchEvent>,
    /// Told to the model at the start of the next turn, e.g. that the user undid an edit.
    notes: Vec<String>,
    /// MCP servers, shared by every model the session switches to.
    mcp: Arc<crate::mcp::Mcp>,
    /// Language servers, likewise.
    lsp: Arc<LspManager>,
    /// This session's usage, for `/stats`.
    stats: Arc<Mutex<crate::stats::Stats>>,
    /// `hooks.d`, read at startup and on `/reload`.
    hooks: Arc<Hooks>,
    /// The `session_start` hooks running, for the session's first prompt.
    start_context: Option<JoinHandle<Vec<String>>>,
    /// Sub-agent setups for the current config (`pool()`).
    pool: Option<Arc<crate::agent::Pool>>,
    /// Sub-agent runs in this session, for their ids.
    agent_runs: Arc<std::sync::atomic::AtomicU64>,
    /// Where MCP servers' forms and sampling requests arrive, and how many have.
    asks: mpsc::Sender<ServerAsk>,
    asked: u64,
    /// The pending browser sign-in, aborted on cancel, replacement or shutdown.
    sign_in: Option<JoinHandle<()>>,
    hook_tasks: JoinSet<()>,
    hook_checks: BTreeMap<u64, CancellationToken>,
    /// Live session outbound/blocked network ledger.
    ledger: Arc<EgressLedger>,
    /// Active session log handle shared with `ledger`'s recording hook.
    ledger_log: Arc<Mutex<Option<SharedLog>>>,
    /// SHA-256 secret fingerprints explicitly allowed for the active session.
    secret_allowlist: Arc<Mutex<Vec<String>>>,
    /// One user question at a time: every turn and sampling request shares this.
    prompt_lock: Arc<tokio::sync::Mutex<()>>,
}

impl Engine {
    /// Spawn the engine on the current tokio runtime. Startup runs in the background;
    /// the returned handle is subscribed before any event is sent, so nothing is missed.
    pub fn spawn(config: Arc<Loaded>) -> EngineHandle {
        Self::spawn_with(config, Resume::New)
    }

    /// Like [`spawn`](Self::spawn), resuming a saved session.
    pub fn spawn_with(config: Arc<Loaded>, resume: Resume) -> EngineHandle {
        let (tx, events) = broadcast::channel(EVENT_CAPACITY);
        let (ops, ops_rx) = mpsc::channel(OP_CAPACITY);
        let (config_tx, config_rx) = watch::channel(config.clone());
        let (asks, asks_rx) = mpsc::channel(8);
        let session = SessionInfo {
            id: new_session_id(),
            cwd: config.dirs.workspace_root.display().to_string(),
            model: Some(config.settings.model.default.clone()).filter(|m| !m.is_empty()),
            profile: "empty".into(),
            reasoning_effort: None,
            reasoning_efforts: Vec::new(),
        };
        let ledger = EgressLedger::new();
        let ledger_log: Arc<Mutex<Option<SharedLog>>> = Arc::default();
        let secret_allowlist =
            Arc::new(Mutex::new(config.settings.network.secret_allowlist.clone()));
        let prompt_lock = Arc::default();
        wire_ledger(&ledger, &tx, &ledger_log);
        let engine = Self {
            config,
            config_tx,
            reloading: false,
            switching: false,
            tx: tx.clone(),
            session,
            reasoning_efforts: BTreeMap::new(),
            started: false,
            problem: None,
            convo: None,
            next_turn: 0,
            queued: VecDeque::new(),
            running: None,
            approvals: Approvals::default(),
            questions: Questions::default(),
            continues: turn::Continues::default(),
            resume,
            log: None,
            turns: 0,
            title: None,
            title_tasks: JoinSet::new(),
            switch_tx: None,
            pending_mode: None,
            pending_work_mode: None,
            pending_approvals: None,
            saved_mode: None,
            saved_plan_start: None,
            pending_sandbox: None,
            pending_watch: None,
            notes: Vec::new(),
            stats: Arc::default(),
            hooks: Arc::default(),
            start_context: None,
            pool: None,
            agent_runs: Arc::default(),
            asks,
            asked: 0,
            prompt_lock,
            sign_in: None,
            hook_tasks: JoinSet::new(),
            hook_checks: BTreeMap::new(),
            mcp: crate::mcp::Mcp::new(McpManager::new(
                Vec::new(),
                PathBuf::new(),
                Default::default(),
            )),
            lsp: LspManager::empty(),
            ledger,
            ledger_log,
            secret_allowlist,
        };
        let task = tokio::spawn(engine.run(ops_rx, asks_rx));
        EngineHandle {
            ops,
            events,
            tx,
            config: config_rx,
            task,
        }
    }

    fn emit(&self, event: Event) {
        // No receivers just means no client is listening right now.
        let _ = self.tx.send(event);
    }

    /// Change the session title, telling clients when it differs (an empty title is none).
    fn set_title(&mut self, title: Option<String>) {
        let title = title.filter(|t| !t.is_empty());
        if title != self.title {
            self.title = title;
            self.emit(Event::SessionTitle {
                title: self.title.clone(),
            });
        }
    }

    async fn run(mut self, mut ops: mpsc::Receiver<Op>, mut asks: mpsc::Receiver<ServerAsk>) {
        self.emit(Event::SessionConfigured {
            session: Box::new(self.session.clone()),
        });
        self.start_mcp().await;
        self.start_lsp().await;
        self.load_hooks().await;
        let (watch_tx, mut watch_rx) = mpsc::channel::<crate::watch::WatchEvent>(8);
        crate::watch::spawn(self.config_tx.subscribe(), watch_tx);
        let mut startup = tokio::spawn(startup::run(
            self.config.clone(),
            self.tx.clone(),
            self.mcp.clone(),
            self.lsp.clone(),
            None,
        ));
        let (done_tx, mut done_rx) = mpsc::channel::<(TurnId, Vec<Message>)>(8);
        let (switch_tx, mut switch_rx) = mpsc::channel::<(Role, Result<Setup, String>)>(2);
        self.switch_tx = Some(switch_tx);
        let (reload_tx, mut reload_rx) = mpsc::channel::<Reload>(2);

        loop {
            tokio::select! {
                outcome = &mut startup, if !self.started => {
                    self.started = true;
                    let outcome = outcome.unwrap_or_else(|e| {
                        tracing::error!("the startup task failed: {e}");
                        Default::default()
                    });
                    self.on_started(outcome);
                    self.emit(Event::StartupComplete);
                    crate::checks::background(self.config.clone(), self.tx.clone());
                    self.next_queued(&done_tx, &reload_tx).await;
                }
                Some((role, result)) = switch_rx.recv() => {
                    match role {
                        Role::Session => self.on_switched(result),
                        Role::Planner => self.on_planner(result),
                    }
                    self.next_queued(&done_tx, &reload_tx).await;
                }
                Some(step) = reload_rx.recv() => {
                    match step {
                        Reload::Read(result, hook) => self.on_reread(result, hook, &reload_tx).await,
                        Reload::Ready(outcome) => {
                            self.on_reloaded(*outcome);
                            self.reloading = false;
                            self.emit(Event::StartupComplete);
                            crate::checks::background(self.config.clone(), self.tx.clone());
                        }
                        Reload::HookSaved { request, id, result } => {
                            self.reloading = false;
                            match result {
                                Ok((config, note)) => {
                                    self.config = Arc::new(*config);
                                    self.config_tx.send_replace(self.config.clone());
                                    self.load_hooks().await;
                                    self.emit(Event::ConfigReloaded);
                                    match crate::hook_setup::inspect(&self.config, &id) {
                                        Ok(info) if info.enabled && !self.hooks.contains(&id) => self.emit(Event::HookSetupFailed { request, message: "The settings were saved, but the hook did not load. Check /hooks and /doctor before using it.".into() }),
                                        Ok(info) => self.emit(Event::HookSetupSaved { request, info, note }),
                                        Err(message) => self.emit(Event::HookSetupFailed { request, message }),
                                    }
                                }
                                Err(message) => self.emit(Event::HookSetupFailed { request, message }),
                            }
                        }
                    }
                    self.next_queued(&done_tx, &reload_tx).await;
                }
                Some((turn, history)) = done_rx.recv() => {
                    if let Some(c) = &mut self.convo {
                        c.history = Some(history);
                    }
                    self.sync_log();
                    if self.running.as_ref().is_some_and(|(t, _)| *t == turn) {
                        self.running = None;
                    }
                    if let Some(ev) = self.pending_watch.take() {
                        self.apply_watch(ev).await;
                    }
                    self.next_queued(&done_tx, &reload_tx).await;
                }
                Some(ev) = watch_rx.recv() => {
                    if self.running.is_some() || self.reloading {
                        match &mut self.pending_watch {
                            Some(pending) => pending.merge(ev),
                            None => self.pending_watch = Some(ev),
                        }
                    } else {
                        self.apply_watch(ev).await;
                    }
                }
                result = self.title_tasks.join_next(), if !self.title_tasks.is_empty() => {
                    if let Some(Ok((session, title))) = result
                        && self.session.id == session
                    {
                        match title {
                            Ok(title) => self.save_title(title),
                            Err(why) => {
                                tracing::warn!(model = %self.config.settings.model.small, "no session title: {why}");
                                self.warn(format!(
                                    "model.small ({}) gave no session title: {why}",
                                    self.config.settings.model.small.trim()
                                ));
                            }
                        }
                    }
                }
                Some(ask) = asks.recv() => self.on_server_ask(ask),
                _ = self.hook_tasks.join_next(), if !self.hook_tasks.is_empty() => {
                    self.hook_checks.retain(|_, cancel| !cancel.is_cancelled());
                }
                op = ops.recv() => {
                    let Some(op) = op else { break };
                    if !self.handle(op, &done_tx, &reload_tx).await {
                        break;
                    }
                }
            }
        }
        if let Some((_, cancel)) = self.running.take() {
            cancel.cancel();
        }
        for cancel in self.hook_checks.values() {
            cancel.cancel();
        }
        self.hook_tasks.abort_all();
        if let Some(task) = self.sign_in.take() {
            task.abort();
            let _ = task.await;
        }
        startup.abort();
        if let Some(end) = self.session_hooks(HookEvent::SessionEnd, "quit") {
            let _ = end.await;
        }
        let _ = tokio::time::timeout(
            Duration::from_secs(3),
            futures::future::join(self.mcp.manager.shutdown(), self.lsp.shutdown()),
        )
        .await;
        self.sync_log();
        self.log = None;
        self.sync_ledger_log();
        self.emit(Event::ShutdownComplete);
    }

    /// Read `hooks.d`.
    async fn load_hooks(&mut self) {
        let config = self.config.clone();
        let (hooks, warnings) = tokio::task::spawn_blocking(move || crate::hooks::load(&config))
            .await
            .unwrap_or_default();
        for w in warnings {
            self.warn(w);
        }
        self.hooks = Arc::new(hooks);
    }

    /// Start the `session_start` or `session_end` hooks in the background. `source` says
    /// why: `startup`, `resume`, `new`, `quit` or `switch`.
    fn session_hooks(&self, event: HookEvent, source: &str) -> Option<JoinHandle<Vec<String>>> {
        if !self.hooks.any(event) {
            return None;
        }
        let (hooks, tx) = (self.hooks.clone(), self.tx.clone());
        let session = self.session.id.0.clone();
        let fields = serde_json::json!({ "source": source });
        Some(tokio::spawn(async move {
            let emit = move |e: Event| {
                let _ = tx.send(e);
            };
            let sink = Sink {
                emit: &emit,
                log: None,
                session: &session,
            };
            let cancel = CancellationToken::new();
            hooks
                .run(event, fields, Target::None, &sink, &cancel)
                .await
                .context
        }))
    }

    /// Read `lsp.d` and find the servers' programs; they start when a file is opened.
    async fn start_lsp(&mut self) {
        let config = self.config.clone();
        let (specs, warnings) = tokio::task::spawn_blocking(move || crate::lsp::load(&config))
            .await
            .unwrap_or_default();
        for w in warnings {
            self.warn(w);
        }
        self.lsp = crate::lsp::manager(&self.config, specs);
        crate::lsp::report(self.lsp.clone(), self.tx.clone());
    }

    /// Read `mcp.d` and connect the servers in the background.
    async fn start_mcp(&mut self) {
        let config = self.config.clone();
        let (specs, warnings) = tokio::task::spawn_blocking(move || crate::mcp::load(&config))
            .await
            .unwrap_or_default();
        for w in warnings {
            self.warn(w);
        }
        let manager = McpManager::with_sandbox(
            specs,
            self.config.dirs.workspace_root.clone(),
            self.config.settings.network.clone(),
            crate::sandbox::server_policy(&self.config),
        );
        manager.set_asker(self.asks.clone());
        manager.set_ledger(self.ledger.clone());
        manager.start();
        crate::mcp::report(manager.clone(), self.tx.clone());
        self.mcp = crate::mcp::Mcp::new(manager);
    }

    /// An MCP server asks the user to fill in a form, or the model for a reply. Both wait
    /// in the background; an interrupt of the running turn ends them.
    fn on_server_ask(&mut self, ask: ServerAsk) {
        self.asked += 1;
        let call_id = CallId(format!("mcp-ask-{}", self.asked));
        let cancel = self
            .running
            .as_ref()
            .map_or_else(CancellationToken::new, |(_, c)| c.clone());
        match ask {
            ServerAsk::Elicit {
                server,
                form,
                mut reply,
            } => {
                let (otx, orx) = tokio::sync::oneshot::channel();
                if let Ok(mut pending) = self.questions.lock() {
                    pending.insert(call_id.0.clone(), otx);
                }
                self.emit(Event::Questions {
                    call_id: call_id.clone(),
                    questions: crate::mcp_ask::questions(&server, &form),
                });
                let (questions, tx) = (self.questions.clone(), self.tx.clone());
                tokio::spawn(async move {
                    let answers =
                        crate::mcp_ask::answers(&questions, &call_id.0, orx, &cancel, &mut reply)
                            .await;
                    let (result, why) = crate::mcp_ask::elicited(&form, answers);
                    if let Some(message) = why {
                        let _ = tx.send(Event::Warning { message });
                    }
                    let _ = reply.send(result);
                });
            }
            ServerAsk::Sample {
                server,
                request,
                mut reply,
            } => {
                let Some(c) = &self.convo else {
                    let _ = reply.send(Err("no model is ready yet".into()));
                    return;
                };
                let sampler = crate::mcp_ask::Sampler {
                    tx: self.tx.clone(),
                    approvals: self.approvals.clone(),
                    perms: c.perms.clone(),
                    setup: c.setup.clone(),
                    stats: self.stats.clone(),
                    log: self.log.clone(),
                    prompt_lock: self.prompt_lock.clone(),
                    cancel,
                    call_id,
                };
                tokio::spawn(async move {
                    let result = tokio::select! {
                        r = sampler.run(&server, request) => r,
                        () = reply.closed() => return,
                    };
                    let _ = reply.send(result);
                });
            }
        }
    }

    fn on_started(&mut self, outcome: StartupOutcome) {
        self.problem = outcome.problem;
        let Some(setup) = outcome.setup else {
            let has_resume_target = match &self.resume {
                Resume::New => false,
                Resume::Latest => session::project_sessions(&self.config).latest().is_some(),
                Resume::Id(_) => true,
            };
            if has_resume_target {
                self.resume_failed("cannot resume: no model is ready".into());
            }
            return;
        };
        setup.provider.set_ledger(self.ledger.clone());
        self.session.model = Some(setup.model_ref.clone());
        self.session.profile = setup.profile.id.clone();
        let resume = std::mem::take(&mut self.resume);
        let resumed = resume != Resume::New;
        let (history, plan) = self.open_saved(&resume, &setup).unwrap_or_default();
        self.configure_reasoning(&setup);
        if !plan.is_empty() {
            setup.tool_ctx.plan.restore(&plan);
            self.emit(Event::Plan { checklist: plan });
        }
        self.emit(Event::SessionConfigured {
            session: Box::new(self.session.clone()),
        });
        self.convo = Some(Conversation {
            perms: Arc::new(Mutex::new(setup.permissions.clone())),
            nested: nested(&self.config, &setup, &history),
            setup: Arc::new(setup),
            history: Some(history),
            editor: None,
            plan_start: None,
            last_response_id: Arc::default(),
            last_reminders: Arc::default(),
            egress_guard: Arc::default(),
        });
        self.restore_modes();
        if let Some(policy) = self.pending_approvals.take() {
            self.set_approval_policy(policy);
        }
        match self.pending_mode.take() {
            Some(mode) => self.set_mode(mode),
            None => self.emit_mode(),
        }
        if let Some(mode) = self.pending_work_mode.take() {
            self.set_work_mode(mode);
        }
        if self.in_plan_mode() {
            self.start_planner();
        }
        if let Some(off) = self.pending_sandbox.take() {
            self.set_sandbox(off);
        }
        let source = if resumed && self.log.is_some() {
            "resume"
        } else {
            "startup"
        };
        self.start_context = self.session_hooks(HookEvent::SessionStart, source);
    }

    fn warn(&self, message: String) {
        self.emit(Event::Warning { message });
    }

    /// A resume was asked for and failed; unlike [`Event::Warning`], clients can tell
    /// this apart from any other warning without matching the prose.
    fn resume_failed(&self, reason: String) {
        self.emit(Event::SessionResumeFailed { reason });
    }

    /// The sub-agent pool, made again when a reload replaced the config or the servers.
    fn pool(&mut self) -> Arc<crate::agent::Pool> {
        match &self.pool {
            Some(p) if p.current(&self.config, &self.mcp, &self.lsp) => p.clone(),
            _ => {
                let p = Arc::new(crate::agent::Pool::new(
                    self.config.clone(),
                    self.mcp.clone(),
                    self.lsp.clone(),
                    self.agent_runs.clone(),
                ));
                self.pool = Some(p.clone());
                p
            }
        }
    }

    async fn queue_reasoning_effort(
        &mut self,
        effort: Option<String>,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        self.queued.push_back(Queued::ReasoningEffort(effort));
        self.next_queued(done, reload).await;
    }

    async fn queue_fork(
        &mut self,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        self.queued.push_back(Queued::Fork);
        self.next_queued(done, reload).await;
    }

    async fn queue_fork_worktree(
        &mut self,
        name: Option<String>,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        self.queued.push_back(Queued::ForkWorktree(name));
        self.next_queued(done, reload).await;
    }

    async fn queue_compact(
        &mut self,
        focus: Option<String>,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        if !self.started || self.running.is_some() {
            self.emit(Event::TurnQueued {
                waiting_for: if self.started {
                    "the current turn".into()
                } else {
                    "startup".into()
                },
            });
        }
        self.queued.push_back(Queued::Compact(focus));
        self.next_queued(done, reload).await;
    }

    fn send_rewind_points(&self) {
        let points = self.rewind_points().into_iter().map(|p| p.point).collect();
        self.emit(Event::RewindPoints { points });
    }

    async fn queue_prune(
        &mut self,
        indices: Option<Vec<usize>>,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        if !self.started || self.running.is_some() {
            self.emit(Event::TurnQueued {
                waiting_for: if self.started {
                    "the current turn".into()
                } else {
                    "startup".into()
                },
            });
        }
        self.queued.push_back(Queued::Prune(indices));
        self.next_queued(done, reload).await;
    }

    async fn queue_accept_plan(
        &mut self,
        mode: String,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        self.queued.push_back(Queued::Accept(mode));
        self.next_queued(done, reload).await;
    }

    async fn queue_reload(
        &mut self,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        self.queued.push_back(Queued::Reload(None));
        self.next_queued(done, reload).await;
    }

    /// Returns false when the engine should stop.
    async fn handle(
        &mut self,
        op: Op,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) -> bool {
        tracing::debug!(op = op_name(&op), "op");
        match op {
            Op::UserTurn { items, .. } => self.user_turn(items, done).await,
            Op::Interrupt => self.interrupt(),
            Op::AnswerQuestions {
                call_id,
                answers,
                stop_asking,
            } => self.answer_questions(call_id, answers, stop_asking),
            Op::ApprovalResponse { call_id, decision } => self.answer_approval(call_id, decision),
            Op::ContinueTurn { turn, proceed } => self.answer_continue(turn, proceed),
            Op::ListModels => self.list_models(),
            Op::ListProfiles => return self.list_profiles(),
            Op::SetProfile { id } => self.set_profile(id),
            Op::ListInstructions => self.list_instructions(),
            Op::SetModel { provider, model } => self.set_model(provider, model),
            Op::SetReasoningEffort { effort } => {
                self.queue_reasoning_effort(effort, done, reload).await
            }
            // A client may send these the moment it sees `TurnComplete`, before the turn
            // task has handed the history back, so they wait in the queue like turns do.
            Op::ResumeSession { id } => self.queue_switch(Some(id), done, reload).await,
            Op::NewSession => self.queue_switch(None, done, reload).await,
            Op::ForkSession => self.queue_fork(done, reload).await,
            Op::ForkWorktree { name } => self.queue_fork_worktree(name, done, reload).await,
            Op::Export { format, path } => self.export(format, path.as_deref()).await,
            Op::Compact { focus } => self.queue_compact(focus, done, reload).await,
            Op::ListSessions => self.list_sessions(),
            Op::ListMemory => self.list_memory(),
            Op::ListRewindPoints => self.send_rewind_points(),
            Op::Rewind {
                back,
                conversation,
                files,
                force,
            } => self.rewind(
                back,
                RewindOpts {
                    conversation,
                    files,
                    force,
                },
            ),
            Op::Undo { force, .. } => self.undo(force),
            Op::RunDoctor => self.run_doctor(),
            Op::CheckUpdates => {
                crate::checks::check_updates_now(self.config.clone(), self.tx.clone());
            }
            Op::InitInstructions => return self.init_instructions(done, reload).await,
            Op::ListLsp => self.emit(Event::LspServers {
                servers: crate::lsp::info(&self.lsp),
            }),
            Op::ListEgress => {
                let (entries, stats) = self.ledger.snapshot();
                self.emit(Event::EgressLedger { entries, stats });
            }
            Op::PruneContext { indices } => self.queue_prune(indices, done, reload).await,
            Op::ProbeToolCalling => self.probe_tool_calling(),
            Op::ListLspDiagnostics { server_id } => {
                self.list_lsp_diagnostics(server_id.as_deref());
            }
            Op::ListMcp => self.emit(Event::McpServers {
                servers: self
                    .mcp
                    .info(self.convo.as_ref().and_then(|c| c.setup.budget)),
            }),
            Op::RestartMcp { id } => self.restart_mcp(id),
            Op::EnableMcp { id } => self.enable_mcp(id, done, reload).await,
            Op::DisableMcp { id } => self.disable_mcp(id, done, reload).await,
            Op::AddMcp {
                id,
                url,
                command,
                args,
                bearer_env,
                bearer_token,
                scope,
            } => {
                self.add_mcp(
                    AddMcpFields {
                        id,
                        url,
                        command,
                        args,
                        bearer_env,
                        bearer_token,
                        scope,
                    },
                    done,
                    reload,
                )
                .await
            }
            Op::SaveMcpServer {
                id,
                scope,
                description,
                command,
                args,
                cwd,
                url,
                sandbox,
                env,
                headers,
            } => {
                self.save_mcp_server(
                    scope,
                    SavedMcpFields {
                        id,
                        description,
                        command,
                        args,
                        cwd,
                        url,
                        sandbox,
                        env,
                        headers,
                    },
                    done,
                    reload,
                )
                .await
            }
            Op::DetectServers => self.detect_servers(),
            Op::AddProvider {
                id,
                base_url,
                api,
                model,
            } => self.add_provider(&id, &base_url, api.as_deref(), model),
            Op::SignIn { id } => self.start_sign_in(id).await,
            Op::ListServices => self.list_services(),
            Op::SetProviderKey { id, key } => self.set_provider_key(id, key),
            Op::AddServer { base_url, key } => self.connect(Vec::new(), move |config| {
                crate::setup::add_server(config, &base_url, key.as_ref().map(|k| k.0.as_str()))
            }),
            Op::ListProviderConnections => self.list_provider_connections(),
            Op::SaveProviderConnection {
                id,
                http_version,
                no_proxy,
            } => {
                self.save_provider_connection(id, http_version, no_proxy, done, reload)
                    .await
            }
            Op::SetPermissionMode { mode } if !self.started => self.pending_mode = Some(mode),
            Op::SetPermissionMode { mode } => self.set_mode(mode),
            Op::SetWorkMode { mode } if !self.started => self.pending_work_mode = Some(mode),
            Op::SetWorkMode { mode } => self.set_work_mode(mode),
            Op::SetApprovalPolicy { policy } if !self.started => {
                self.pending_approvals = Some(policy)
            }
            Op::SetApprovalPolicy { policy } => self.set_approval_policy(policy),
            Op::SetSandbox { off } if !self.started => self.pending_sandbox = Some(off),
            Op::SetSandbox { off } => self.set_sandbox(off),
            Op::AcceptPlan { mode } => self.queue_accept_plan(mode, done, reload).await,
            Op::SaveMemory {
                scope,
                name,
                description,
                kind,
                body,
                previous_scope,
                previous_name,
            } => self.save_memory(SaveMemoryArgs {
                scope,
                name,
                description,
                kind,
                body,
                previous_scope,
                previous_name,
            }),
            Op::DeleteMemory { scope, name } => self.delete_memory(scope, name),
            Op::GetContext => self.context_report(),
            Op::GetStats => self.stats_report(),
            Op::SetTitle { title } => self.set_session_title(title),
            Op::SaveSetting { key, value, scope } => self.save_setting(&key, &value, &scope).await,
            Op::ReloadConfig => self.queue_reload(done, reload).await,
            Op::TrustProject { decision } => self.trust_project(decision, done, reload).await,
            Op::EnableHook {
                id,
                replace_payload,
            } => self.enable_hook(id, replace_payload, done, reload).await,
            Op::InspectHookSetup { id, request } => self.inspect_hook_setup(id, request),
            op @ Op::SaveHookSetup { .. } => self.queue_hook_save(op, done, reload).await,
            Op::CheckHookSetup {
                id,
                request,
                revision,
                connection,
                url,
            } => self.check_hook_setup(id, request, revision, connection, url),
            Op::CancelHookCheck { request } => self.cancel_hook_check(request),
            Op::Shutdown => return false,
        }
        true
    }

    async fn queue_switch(
        &mut self,
        target: Option<SessionId>,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        self.queued.push_back(Queued::Switch(target));
        self.next_queued(done, reload).await;
    }

    async fn next_queued(
        &mut self,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        while self.started && self.running.is_none() && !self.reloading && !self.switching {
            match self.queued.pop_front() {
                Some(Queued::Turn(items)) => self.start_turn(items, done.clone()).await,
                Some(Queued::Switch(target)) => self.switch_session(target),
                Some(Queued::Compact(focus)) => self.start_compact(focus, done.clone()),
                Some(Queued::Prune(indices)) => self.prune_context(indices),
                Some(Queued::Reload(hook)) => self.begin_reload(reload.clone(), hook),
                Some(Queued::Accept(mode)) => self.accept_plan(&mode, done.clone()).await,
                Some(Queued::Fork) => self.fork_session().await,
                Some(Queued::ForkWorktree(name)) => self.fork_worktree(name, reload.clone()).await,
                Some(Queued::HookSave(op)) => self.begin_hook_save(*op, reload.clone()),
                Some(Queued::ReasoningEffort(effort)) => self.set_reasoning_effort(effort),
                None => break,
            }
        }
    }
}

fn op_name(op: &Op) -> &'static str {
    match op {
        Op::UserTurn { .. } => "user_turn",
        Op::Interrupt => "interrupt",
        Op::ApprovalResponse { .. } => "approval_response",
        Op::AnswerQuestions { .. } => "answer_questions",
        Op::ContinueTurn { .. } => "continue_turn",
        Op::SetModel { .. } => "set_model",
        Op::SetReasoningEffort { .. } => "set_reasoning_effort",
        Op::ListProfiles => "list_profiles",
        Op::ListInstructions => "list_instructions",
        Op::SetProfile { .. } => "set_profile",
        Op::ListModels => "list_models",
        Op::Compact { .. } => "compact",
        Op::Undo { .. } => "undo",
        Op::ResumeSession { .. } => "resume_session",
        Op::NewSession => "new_session",
        Op::ListSessions => "list_sessions",
        Op::GetContext => "get_context",
        Op::GetStats => "get_stats",
        Op::ListMemory => "list_memory",
        Op::SaveMemory { .. } => "save_memory",
        Op::RunDoctor => "run_doctor",
        Op::CheckUpdates => "check_updates",
        Op::DetectServers => "detect_servers",
        Op::ListMcp => "list_mcp",
        Op::ListLsp => "list_lsp",
        Op::InitInstructions => "init_instructions",
        Op::RestartMcp { .. } => "restart_mcp",
        Op::EnableMcp { .. } => "enable_mcp",
        Op::SaveMcpServer { .. } => "save_mcp_server",
        Op::DisableMcp { .. } => "disable_mcp",
        Op::AddMcp { .. } => "add_mcp",
        Op::AddProvider { .. } => "add_provider",
        Op::ListServices => "list_services",
        Op::SignIn { .. } => "sign_in",
        Op::SetProviderKey { .. } => "set_provider_key",
        Op::AddServer { .. } => "add_server",
        Op::ListProviderConnections => "list_provider_connections",
        Op::SaveProviderConnection { .. } => "save_provider_connection",
        Op::SetPermissionMode { .. } => "set_permission_mode",
        Op::SetWorkMode { .. } => "set_work_mode",
        Op::SetApprovalPolicy { .. } => "set_approval_policy",
        Op::SetSandbox { .. } => "set_sandbox",
        Op::AcceptPlan { .. } => "accept_plan",
        Op::DeleteMemory { .. } => "delete_memory",
        Op::SetTitle { .. } => "set_title",
        Op::SaveSetting { .. } => "save_setting",
        Op::ForkSession => "fork_session",
        Op::ForkWorktree { .. } => "fork_worktree",
        Op::Export { .. } => "export",
        Op::ListRewindPoints => "list_rewind_points",
        Op::Rewind { .. } => "rewind",
        Op::ReloadConfig => "reload_config",
        Op::TrustProject { .. } => "trust_project",
        Op::EnableHook { .. } => "enable_hook",
        Op::InspectHookSetup { .. } => "inspect_hook_setup",
        Op::SaveHookSetup { .. } => "save_hook_setup",
        Op::CheckHookSetup { .. } => "check_hook_setup",
        Op::CancelHookCheck { .. } => "cancel_hook_check",
        Op::ListEgress => "list_egress",
        Op::PruneContext { .. } => "prune_context",
        Op::ProbeToolCalling => "probe_tool_calling",
        Op::ListLspDiagnostics { .. } => "list_lsp_diagnostics",
        Op::Shutdown => "shutdown",
    }
}
