//! One user turn: stream the model, run its tool calls, loop until it answers
//! (docs/plan/04-agent-loop-and-tools.md#turn-lifecycle).

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};

use quack_config::hooks::HookEvent;
use quack_config::settings::{NetworkSettings, SecretGuardMode};
use quack_protocol::{
    ApprovalDecision, ApprovalRequest, CallId, EgressKind, EngineError, EngineErrorKind, Event,
    Question, ToolCallInfo, ToolResultSummary, TurnId, Usage, WriteScope,
};
use quack_providers::{
    ChatRequest, EgressDraft, EgressLedger, ErrorKind, FinishReason, Message, ResponseBuilder,
    StreamEvent, ToolCall, ToolSpec, tokens, toolparse,
};
use quack_store::checkpoint::Checkpoints;
use quack_tools::{Answers, Decision, Permissions, Subject, Tool, ToolCtx, ToolKind, ToolOutput};
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use quack_context::compact;

use crate::hooks::{self, Hooks, Sink, Target, Verdict};
use crate::session::{self, SharedLog};
use crate::startup::Setup;
use crate::stats::Stats;

fn hash_str(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Identical consecutive calls (or consecutive failures on the same target) before the
/// model is told to change approach.
const REPEAT_LIMIT: usize = 3;

/// Last reminders appended to history for append-only providers, so unchanged reminders
/// are emitted only when they change or after a compaction instead of every step.
#[derive(Debug, Default)]
pub(crate) struct ReminderState {
    working_files: Option<String>,
    plan: Option<String>,
}

/// Tracks which conversation history and prompt context have already been vetted for remote
/// Secret Shield egress. Left unadvanced on loopback (`secret_guard_local = false`) so a
/// mid-session switch from a local model to a cloud provider scans all prior turns.
#[derive(Debug, Default)]
pub(crate) struct EgressGuardState {
    scanned_upto: usize,
    system_checked: Option<(String, u64, Option<String>)>,
    context_checked: Option<(String, u64, Option<String>)>,
}

/// Repeated calls and consecutive target failures this turn, tracked by name and argument
/// hash so the arguments (whole file contents for edits) are never cloned. `record`
/// returns the length of the trailing run of the call and its total count in the turn.
#[derive(Default)]
struct RecentCalls {
    counts: HashMap<(String, u64), usize>,
    last: Option<(String, u64)>,
    trailing: usize,
    target_failures: HashMap<(String, String), usize>,
}

impl RecentCalls {
    fn record(&mut self, name: &str, arguments: &str) -> (usize, usize) {
        let mut hasher = DefaultHasher::new();
        arguments.hash(&mut hasher);
        let key = (name.to_owned(), hasher.finish());
        let total = *self.counts.entry(key.clone()).or_insert(0) + 1;
        self.counts.insert(key.clone(), total);
        self.trailing = if self.last.as_ref() == Some(&key) {
            self.trailing + 1
        } else {
            1
        };
        self.last = Some(key);
        (self.trailing, total)
    }

    fn record_outcome(&mut self, name: &str, target: Option<&str>, ok: bool) -> usize {
        let Some(target) = target else {
            return 0;
        };
        let key = (name.to_owned(), target.to_owned());
        if ok {
            self.target_failures.remove(&key);
            0
        } else {
            let count = self.target_failures.entry(key).or_insert(0);
            *count += 1;
            *count
        }
    }
}

pub(crate) type Approvals = Arc<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>>;
/// `ask_user` calls waiting for the user's answers, by call id.
pub(crate) type Questions = Arc<Mutex<HashMap<String, oneshot::Sender<Answers>>>>;
/// Turns paused at `agent.max_steps`, waiting for the user's go-ahead, by turn id.
pub(crate) type Continues = Arc<Mutex<HashMap<TurnId, oneshot::Sender<bool>>>>;

pub(crate) struct TurnCtx {
    pub turn: TurnId,
    pub setup: Arc<Setup>,
    pub perms: Arc<Mutex<Permissions>>,
    pub nested: Arc<quack_instructions::agents_md::Nested>,
    pub loaded_skills: Arc<Mutex<session::LoadedSkills>>,
    pub approvals: Approvals,
    pub questions: Questions,
    pub continues: Continues,
    pub tx: broadcast::Sender<Event>,
    pub cancel: CancellationToken,
    pub log: Option<SharedLog>,
    /// The session's own log. For a sub-agent, whose messages go to its own `log`, its
    /// usage is also recorded here (`agent_usage`), so `/stats` counts it after a resume.
    pub main_log: Option<SharedLog>,
    /// Pre-images of edited files, for `/undo`.
    pub checkpoints: Option<Arc<Checkpoints>>,
    /// The session's usage, fed by `emit`.
    pub stats: Arc<Mutex<Stats>>,
    pub hooks: Arc<Hooks>,
    /// The session id, for hooks.
    pub session: String,
    /// The sub-agent run this turn is (`explore-2`); `None` for the session's own turns.
    pub agent: Option<String>,
    /// The sub-agent definition id (`explore`), shared across runs of the same agent type
    /// so their `prompt_cache_key` routes to the same warm prefix.
    pub agent_type: Option<String>,
    /// Model calls before the turn stops.
    pub max_steps: usize,
    /// Sub-agent setups, built on first use.
    pub pool: Arc<crate::agent::Pool>,
    /// One approval or question on screen at a time, while sub-agents run side by side.
    pub prompt_lock: Arc<tokio::sync::Mutex<()>>,
    /// Network settings governing pre-egress Secret Shield.
    pub network: NetworkSettings,
    /// Session-scoped outbound ledger.
    pub ledger: Arc<EgressLedger>,
    /// Runtime session-level secret fingerprint allowlist shared across turns.
    pub secret_allowlist: Arc<Mutex<Vec<String>>>,
    /// Connection-local Responses state and the last response ID for diagnostics.
    pub responses_session: Arc<quack_providers::ResponsesSession>,
    /// Last plan / working-files reminders written into append-only history.
    pub last_reminders: Arc<Mutex<ReminderState>>,
    /// Remote Secret Shield vetting state across turns and model switches.
    pub egress_guard: Arc<Mutex<EgressGuardState>>,
    /// Enterprise audit log writer, when `[audit] path` is configured.
    pub audit: Option<Arc<crate::audit::AuditLog>>,
    /// Optional host-client bridge (e.g. ACP editor `fs/*` and `terminal/*`).
    pub client_bridge: Option<Arc<dyn quack_tools::ClientBridge>>,
}

/// What the user sent, ready for the model, and what they typed (which hooks see).
pub(crate) struct Prompt {
    pub text: String,
    pub typed: String,
    /// Images the user attached, encoded and ready for the wire.
    pub images: Vec<quack_protocol::Image>,
    /// Context from `session_start` hooks, for the session's first prompt.
    pub start: Option<tokio::task::JoinHandle<Vec<String>>>,
}

impl TurnCtx {
    pub(crate) fn start_request(&self) {
        session::record(self.log.as_ref(), "request_start", json!({}));
        if let Some(agent) = &self.agent {
            session::record(
                self.main_log.as_ref(),
                "agent_request_start",
                json!({ "agent": agent }),
            );
        }
    }

    /// Count every attempted request, including summaries and discarded replies. A missing
    /// usage report stays null in the log so collectors can detect incomplete accounting.
    pub(crate) fn record_request(&self, usage: Option<&Usage>, took: std::time::Duration) {
        self.record_request_for(&self.setup, usage, took);
    }

    pub(crate) fn record_request_for(
        &self,
        setup: &Setup,
        usage: Option<&Usage>,
        took: std::time::Duration,
    ) {
        let cost = usage.and_then(|u| {
            setup.model.cost.as_ref().map(|c| {
                c.of(
                    u.prompt_tokens,
                    u.cached_prompt_tokens.unwrap_or(0),
                    u.cache_write_tokens.unwrap_or(0),
                    u.completion_tokens,
                )
            })
        });
        session::record_usage(self.log.as_ref(), usage, cost);
        if let Some(agent) = &self.agent {
            session::record(
                self.main_log.as_ref(),
                "agent_usage",
                json!({ "agent": agent, "usage": usage, "cost": cost }),
            );
        }
        if let Ok(mut stats) = self.stats.lock() {
            stats.request(usage, cost, took);
        }
    }

    /// Clear tracked reminder state (for example after compaction removed earlier reminders).
    pub(crate) fn reset_reminders(&self) {
        if let Ok(mut state) = self.last_reminders.lock() {
            state.working_files = None;
            state.plan = None;
        }
        if let Ok(mut guard) = self.egress_guard.lock() {
            guard.scanned_upto = 0;
        }
    }

    /// For an append-only provider, append plan and working-file reminders to the step's
    /// last tool result only when they changed since last emitted, and leave them in
    /// history so the next request strictly extends this one without mutating its tail.
    fn keep_plan(&self, content: &mut String, last: bool, is_todo: bool) {
        if !last || !self.setup.provider.append_only() {
            return;
        }
        let wf = working_files_reminder(self);
        let todo_name = self.setup.surface.name_of("todo").unwrap_or("todo");
        let plan = self.setup.tool_ctx.plan.reminder_for(todo_name);
        let Ok(mut state) = self.last_reminders.lock() else {
            return;
        };
        if wf != state.working_files {
            if let Some(text) = &wf {
                content.push_str("\n\n");
                content.push_str(text);
            }
            state.working_files = wf;
        }
        if plan != state.plan {
            if let Some(text) = &plan {
                if is_todo && let Some(rest) = content.strip_prefix("Plan updated:\n") {
                    let suffix = rest
                        .find("\n\n")
                        .map(|i| rest[i..].to_owned())
                        .unwrap_or_default();
                    *content = format!("Plan updated.{suffix}");
                }
                content.push_str("\n\n");
                content.push_str(text);
            }
            state.plan = plan;
        }
    }

    /// Add to history and the session log together, so the log is the source of truth.
    fn push(&self, history: &mut Vec<Message>, m: Message) {
        session::record_message(self.log.as_ref(), &m);
        history.push(m);
        if !crate::secrets::should_skip_guard(&self.network, self.setup.provider.base_url())
            && let Ok(mut guard) = self.egress_guard.lock()
        {
            guard.scanned_upto = history.len();
        }
    }

    /// Scan `system`, `session_context` (`AGENTS.md` + memory index), and any previously
    /// unscanned `history` (such as turns run on a local loopback model before switching to
    /// a remote host, or a resumed session) before sending a request to a remote provider.
    async fn guard_session_and_history(&self, history: &mut [Message]) -> Result<(), ()> {
        let base_url = self.setup.provider.base_url();
        if crate::secrets::should_skip_guard(&self.network, base_url) {
            return Ok(());
        }

        let sys_hash = hash_str(&self.setup.system);
        let need_sys = !self.setup.system.is_empty()
            && self.egress_guard.lock().ok().and_then(|g| {
                g.system_checked
                    .as_ref()
                    .map(|(u, h, _)| (u == base_url, *h))
            }) != Some((true, sys_hash));
        if need_sys {
            let ask_id = self.key(&format!("egress-system-{}", self.turn.0));
            let clean = self
                .guard_egress("system_prompt", self.setup.system.clone(), &ask_id)
                .await?;
            let override_text = (clean != self.setup.system).then_some(clean);
            if let Ok(mut g) = self.egress_guard.lock() {
                g.system_checked = Some((base_url.to_owned(), sys_hash, override_text));
            }
        }

        let session_ctx = self.setup.effective_session_context();
        let ctx_hash = hash_str(&session_ctx);
        let need_ctx = !session_ctx.is_empty()
            && self.egress_guard.lock().ok().and_then(|g| {
                g.context_checked
                    .as_ref()
                    .map(|(u, h, _)| (u == base_url, *h))
            }) != Some((true, ctx_hash));
        if need_ctx {
            let ask_id = self.key(&format!("egress-context-{}", self.turn.0));
            let clean = self
                .guard_egress("session_context", session_ctx.clone(), &ask_id)
                .await?;
            let override_text = (clean != session_ctx).then_some(clean);
            if let Ok(mut g) = self.egress_guard.lock() {
                g.context_checked = Some((base_url.to_owned(), ctx_hash, override_text));
            }
        }

        let start = self
            .egress_guard
            .lock()
            .map(|g| g.scanned_upto.min(history.len()))
            .unwrap_or(0);
        for (idx, msg) in history.iter_mut().enumerate().skip(start) {
            let ask_id = self.key(&format!("egress-history-{}-{idx}", self.turn.0));
            match msg {
                Message::User { content, .. } if !content.is_empty() => {
                    *content = self
                        .guard_egress("history:user", std::mem::take(content), &ask_id)
                        .await?;
                }
                Message::Tool { content, .. } if !content.is_empty() => {
                    *content = self
                        .guard_egress("history:tool", std::mem::take(content), &ask_id)
                        .await?;
                }
                Message::Assistant {
                    content, sealed, ..
                } if !content.is_empty() => {
                    let original = content.clone();
                    let clean = self
                        .guard_egress("history:assistant", original.clone(), &ask_id)
                        .await?;
                    if clean != original {
                        *content = clean;
                        *sealed = None;
                    }
                }
                Message::Compaction {
                    content,
                    session_context,
                    ..
                } => {
                    if !content.is_empty() {
                        *content = self
                            .guard_egress("history:compaction", std::mem::take(content), &ask_id)
                            .await?;
                    }
                    if let Some(ctx_text) = session_context.take()
                        && !ctx_text.is_empty()
                    {
                        let ask_ctx =
                            self.key(&format!("egress-history-ctx-{}-{idx}", self.turn.0));
                        *session_context = Some(
                            self.guard_egress("history:session_context", ctx_text, &ask_ctx)
                                .await?,
                        );
                    }
                }
                Message::System { content } if !content.is_empty() => {
                    *content = self
                        .guard_egress("history:system", std::mem::take(content), &ask_id)
                        .await?;
                }
                _ => {}
            }
        }
        if let Ok(mut g) = self.egress_guard.lock() {
            g.scanned_upto = history.len();
        }
        Ok(())
    }

    fn egress_overrides(&self) -> (Option<String>, Option<String>) {
        let base_url = self.setup.provider.base_url();
        let Ok(g) = self.egress_guard.lock() else {
            return (None, None);
        };
        let sys = g
            .system_checked
            .as_ref()
            .filter(|(u, _, _)| u == base_url)
            .and_then(|(_, _, text)| text.clone());
        let ctx = g
            .context_checked
            .as_ref()
            .filter(|(u, _, _)| u == base_url)
            .and_then(|(_, _, text)| text.clone());
        (sys, ctx)
    }

    pub(crate) fn emit(&self, e: Event) {
        if let Ok(mut s) = self.stats.lock() {
            s.observe(&e);
        }
        // A sub-agent's events go to clients wrapped, except the approvals they answer.
        let e = match &self.agent {
            Some(id) if !matches!(e, Event::ApprovalRequest { .. }) => Event::Agent {
                id: id.clone(),
                inner: Box::new(e),
            },
            _ => e,
        };
        let _ = self.tx.send(e);
    }

    /// Park a reply channel under `key`, show `request`, and wait for the answer or
    /// the turn's end; see [`wait_for_answer`].
    pub(crate) async fn wait_for_answer<T, K: Clone + Eq + Hash>(
        &self,
        pending: &Mutex<HashMap<K, oneshot::Sender<T>>>,
        key: K,
        request: Event,
    ) -> Option<T> {
        wait_for_answer(
            &self.prompt_lock,
            &self.cancel,
            &self.stats,
            pending,
            key,
            request,
            |e| self.emit(e),
        )
        .await
    }

    /// A call id as clients and the approval table see it. A sub-agent's model numbers its
    /// calls from scratch, so its ids get the run's id in front.
    fn key(&self, id: &str) -> String {
        match &self.agent {
            Some(a) => format!("{a}/{id}"),
            None => id.to_owned(),
        }
    }

    /// Emit the corrected estimate of what the next request will cost.
    pub(crate) fn context_estimate(&self, history: &[Message]) {
        self.emit(Event::ContextUpdate {
            used: estimate(&self.setup, history),
            budget: self.setup.budget.unwrap_or(0),
            window: self.setup.model.context_window,
            cache_hit_tokens: None,
            cache_hit_percent: self.stats.lock().ok().and_then(|s| s.cache_hit_percent()),
            threshold: crate::context::threshold(&self.setup),
        });
    }

    /// Run the hooks for `event` that match `target`, reporting through this turn.
    pub(crate) async fn hooks(
        &self,
        event: HookEvent,
        fields: Value,
        target: Target<'_>,
    ) -> hooks::Outcome {
        if !self.hooks.any(event) {
            return hooks::Outcome::default();
        }
        let emit = |e: Event| self.emit(e);
        let sink = Sink {
            emit: &emit,
            log: self.log.as_ref(),
            session: &self.session,
        };
        self.hooks
            .run(event, fields, target, &sink, &self.cancel)
            .await
    }

    /// `turn_end` hooks, in the background: nothing waits for them.
    fn turn_end(&self, fields: Value) {
        if self.agent.is_some() || !self.hooks.any(HookEvent::TurnEnd) {
            return;
        }
        let (hooks, tx, log) = (self.hooks.clone(), self.tx.clone(), self.log.clone());
        let session = self.session.clone();
        tokio::spawn(async move {
            let emit = move |e: Event| {
                let _ = tx.send(e);
            };
            let sink = Sink {
                emit: &emit,
                log: log.as_ref(),
                session: &session,
            };
            let cancel = CancellationToken::new();
            hooks
                .run(HookEvent::TurnEnd, fields, Target::None, &sink, &cancel)
                .await;
        });
    }

    fn error(&self, kind: EngineErrorKind, message: impl Into<String>) {
        self.emit(Event::Error {
            error: EngineError {
                kind,
                message: message.into(),
            },
        });
    }

    async fn guard_egress(&self, location: &str, text: String, ask_id: &str) -> Result<String, ()> {
        if crate::secrets::should_skip_guard(&self.network, self.setup.provider.base_url()) {
            return Ok(text);
        }
        let mut active = self.setup.provider.secret_values();
        active.extend(self.setup.mcp.manager.secret_values());
        let mut allowlist = self.network.secret_allowlist.clone();
        if let Ok(extra) = self.secret_allowlist.lock() {
            allowlist.extend(extra.iter().cloned());
        }
        let report = crate::secrets::scan_text(&text, location, &active, &allowlist);
        if report.findings.is_empty() {
            return Ok(text);
        }
        let kinds: Vec<&str> = report.findings.iter().map(|f| f.kind.as_str()).collect();
        let fingerprints: Vec<&str> = report
            .findings
            .iter()
            .map(|f| f.fingerprint.as_str())
            .collect();
        session::record(
            self.log.as_ref(),
            "secret_scan",
            json!({
                "location": location,
                "count": report.redaction_count,
                "kinds": kinds,
                "fingerprints": fingerprints,
            }),
        );
        match self.network.secret_guard {
            SecretGuardMode::Off => Ok(text),
            SecretGuardMode::Redact => {
                self.setup
                    .provider
                    .add_pending_redactions(report.redaction_count);
                self.emit(Event::Notice {
                    message: format!(
                        "Secret Shield redacted {} credential(s) in {location}.",
                        report.redaction_count
                    ),
                });
                Ok(report.redacted)
            }
            SecretGuardMode::Ask => {
                let answer = self
                    .wait_for_answer(
                        &self.approvals,
                        ask_id.to_owned(),
                        Event::ApprovalRequest {
                            call_id: CallId(ask_id.to_owned()),
                            request: ApprovalRequest::EgressSecrets {
                                provider: self.setup.provider.id.clone(),
                                host: crate::secrets::host_of(self.setup.provider.base_url()),
                                findings: report.findings.clone(),
                            },
                            always: Vec::new(),
                        },
                    )
                    .await;
                session::record(
                    self.log.as_ref(),
                    "approval",
                    json!({ "call_id": ask_id, "decision": answer }),
                );
                match answer {
                    Some(ApprovalDecision::RedactAndSend) => {
                        self.setup
                            .provider
                            .add_pending_redactions(report.redaction_count);
                        Ok(report.redacted)
                    }
                    Some(ApprovalDecision::AllowOnce) => Ok(text),
                    Some(ApprovalDecision::AllowSession)
                    | Some(ApprovalDecision::AlwaysAllow { .. }) => {
                        if let Ok(mut extra) = self.secret_allowlist.lock() {
                            for f in &report.findings {
                                if !extra.contains(&f.fingerprint) {
                                    extra.push(f.fingerprint.clone());
                                }
                            }
                        }
                        Ok(text)
                    }
                    None
                    | Some(ApprovalDecision::Deny)
                    | Some(ApprovalDecision::DenyWithFeedback { .. }) => {
                        self.ledger.record(
                            EgressDraft::new(
                                EgressKind::BlockedEgress,
                                self.setup.provider.base_url(),
                                "secret-guard",
                            )
                            .blocked(format!("blocked by secret_guard in {location}")),
                        );
                        Err(())
                    }
                }
            }
        }
    }
}

async fn apply_tool_guard(ctx: &TurnCtx, call: &ToolCall, content: String) -> String {
    let ask_id = ctx.key(&format!("egress-{}", call.id));
    let location = format!("tool:{}", call.name);
    match ctx.guard_egress(&location, content, &ask_id).await {
        Ok(clean) => clean,
        Err(()) => format!(
            "[Blocked by Secret Shield: output of `{}` contained unredacted credential(s) and sending was denied.]",
            call.name
        ),
    }
}

/// Sent once per turn after an empty reply.
const EMPTY_NUDGE: &str =
    "Your last reply was empty. Continue: call a tool if you need one, otherwise answer.";

const LENGTH_NUDGE: &str = "Your last response reached its output-token limit and was discarded. \
    None of its tool calls ran. Continue the task with a shorter response; split large writes \
    into smaller edits and keep reasoning concise.";

enum StepEnd {
    Done,
    Aborted(String),
    /// The server said the request is too long for the model.
    Overflow(String),
    /// The reply stalled or the connection dropped before it finished; nothing went into the
    /// history, so the step can be sent again.
    Dropped(String),
    Truncated,
}

/// Step-level retries after a dropped reply, on top of the provider's own retries (which stop
/// once a reply has started).
const MAX_DROPPED_RETRIES: u32 = 2;

/// Result of running one turn (or sub-agent turn).
pub(crate) struct TurnOutcome {
    pub history: Vec<Message>,
    /// Completed assistant steps produced during this turn (unaffected by compaction).
    pub steps: usize,
    /// Whether the turn ended because it stopped at `max_steps`.
    pub step_limited: bool,
}

/// Run the turn; returns the updated history and outcome metadata.
pub(crate) async fn run(ctx: TurnCtx, mut history: Vec<Message>, prompt: Prompt) -> TurnOutcome {
    ctx.emit(Event::TurnStarted { turn: ctx.turn });
    let mut extra = match prompt.start {
        Some(start) => start.await.unwrap_or_else(|e| {
            tracing::error!("the session-start hooks task failed: {e}");
            Vec::new()
        }),
        None => Vec::new(),
    };
    // A sub-agent's prompt comes from the model, not the user.
    let hooked = if ctx.agent.is_some() {
        hooks::Outcome::default()
    } else {
        ctx.hooks(
            HookEvent::UserPrompt,
            json!({ "prompt": prompt.typed }),
            Target::Prompt(&prompt.typed),
        )
        .await
    };
    if let Some((id, reason)) = hooked.denied() {
        let reason = if reason.is_empty() {
            String::new()
        } else {
            format!(": {reason}")
        };
        ctx.emit(Event::Warning {
            message: format!("hook `{id}` blocked this prompt{reason}; nothing was sent"),
        });
        ctx.emit(Event::TurnAborted {
            turn: ctx.turn,
            reason: format!("blocked by hook `{id}`"),
        });
        ctx.turn_end(json!({ "turn": ctx.turn.0, "outcome": "blocked" }));
        return TurnOutcome {
            history,
            steps: 0,
            step_limited: false,
        };
    }
    extra.extend(hooked.context);
    // Hook context rides on the user message, after the cached prefix, and is kept with
    // it, so a resumed session sends the same request.
    let content = if extra.is_empty() {
        prompt.text
    } else {
        format!("{}\n\n{}", prompt.text, extra.join("\n\n"))
    };
    if ctx.guard_session_and_history(&mut history).await.is_err() {
        ctx.emit(Event::Warning {
            message:
                "Blocked outbound session context or prior history containing sensitive credentials; nothing was sent."
                    .into(),
        });
        ctx.emit(Event::TurnAborted {
            turn: ctx.turn,
            reason: "blocked by Secret Shield".into(),
        });
        ctx.turn_end(json!({ "turn": ctx.turn.0, "outcome": "blocked" }));
        return TurnOutcome {
            history,
            steps: 0,
            step_limited: false,
        };
    }
    let ask_id = ctx.key(&format!("egress-user-{}", ctx.turn.0));
    let content = match ctx.guard_egress("user_prompt", content, &ask_id).await {
        Ok(clean) => clean,
        Err(()) => {
            ctx.emit(Event::Warning {
                message:
                    "Blocked outbound prompt containing sensitive credentials; nothing was sent."
                        .into(),
            });
            ctx.emit(Event::TurnAborted {
                turn: ctx.turn,
                reason: "blocked by Secret Shield".into(),
            });
            ctx.turn_end(json!({ "turn": ctx.turn.0, "outcome": "blocked" }));
            return TurnOutcome {
                history,
                steps: 0,
                step_limited: false,
            };
        }
    };
    ctx.push(
        &mut history,
        Message::User {
            content,
            images: prompt.images,
        },
    );
    let mut total = Usage::default();
    let mut recent = RecentCalls::default();
    let mut nudged = false;
    let mut overflowed = false;
    let mut dropped = 0;
    let mut truncated = 0;
    let mut completed_steps = 0usize;
    let mut step_limited = false;

    let end = 'steps: {
        let mut steps = 0usize;
        loop {
            if steps >= ctx.max_steps {
                match continue_at_limit(&ctx).await {
                    Continue::Proceed => steps = 0,
                    Continue::Stop => {
                        step_limited = true;
                        let msg = match ctx.agent_type.as_deref() {
                            Some(agent) => format!(
                                "Stopped after {} model calls · increase max_steps in agents.d/{agent}.toml.",
                                ctx.max_steps
                            ),
                            None if ctx.agent.is_some() => format!(
                                "Stopped after {} model calls · increase max_steps in agents.d.",
                                ctx.max_steps
                            ),
                            None => format!(
                                "Stopped after {} model calls · increase agent.max_steps or send a message to continue.",
                                ctx.max_steps
                            ),
                        };
                        ctx.error(EngineErrorKind::Internal, msg);
                        break 'steps StepEnd::Aborted("step limit reached".into());
                    }
                    Continue::Interrupted => break 'steps StepEnd::Aborted("interrupted".into()),
                }
            }
            steps += 1;
            let estimated = crate::context::fit(&ctx, &mut history).await;
            let (msg, usage) = match stream_step(&ctx, &history).await {
                Ok(r) => {
                    dropped = 0;
                    truncated = 0;
                    r
                }
                Err(StepEnd::Truncated) => {
                    if truncated < MAX_DROPPED_RETRIES {
                        truncated += 1;
                        ctx.emit(Event::Warning {
                            message: "the reply reached its output-token limit; asking for a shorter response".into(),
                        });
                        ctx.push(
                            &mut history,
                            Message::Nudge {
                                content: LENGTH_NUDGE.into(),
                            },
                        );
                        continue;
                    }
                    ctx.error(EngineErrorKind::Provider,
                        "The model repeatedly reached its output-token limit · increase the model's max_output or use a model with a larger output budget.");
                    break 'steps StepEnd::Aborted("output-token limit reached".into());
                }
                Err(StepEnd::Aborted(reason)) => break 'steps StepEnd::Aborted(reason),
                Err(StepEnd::Done) => break 'steps StepEnd::Done,
                Err(StepEnd::Dropped(message)) => {
                    if dropped < MAX_DROPPED_RETRIES {
                        dropped += 1;
                        ctx.emit(Event::Warning {
                            message: format!("{message} · sending the step again"),
                        });
                        continue;
                    }
                    ctx.error(EngineErrorKind::Provider, message);
                    break 'steps StepEnd::Aborted("provider error".into());
                }
                Err(StepEnd::Overflow(message)) => {
                    // Compact once and retry; a second overflow ends the turn.
                    let compacted = !overflowed
                        && crate::context::compact_now(
                            &ctx,
                            &mut history,
                            compact::KEEP_TURNS,
                            None,
                            "the server said the request is too long",
                        )
                        .await
                        .is_ok();
                    overflowed = true;
                    if compacted {
                        continue;
                    }
                    ctx.error(
                        EngineErrorKind::Provider,
                        format!(
                            "{message} · the conversation is too long for this model; \
                             try /compact or /new"
                        ),
                    );
                    break 'steps StepEnd::Aborted("context overflow".into());
                }
            };
            if let Some(u) = usage {
                if let Ok(mut e) = ctx.setup.estimator.lock() {
                    e.observe(estimated, u.prompt_tokens);
                }
                total.prompt_tokens = total.prompt_tokens.saturating_add(u.prompt_tokens);
                total.completion_tokens =
                    total.completion_tokens.saturating_add(u.completion_tokens);
                if let Some(c) = u.cached_prompt_tokens {
                    total.cached_prompt_tokens =
                        Some(total.cached_prompt_tokens.unwrap_or(0).saturating_add(c));
                }
                if let Some(w) = u.cache_write_tokens {
                    total.cache_write_tokens =
                        Some(total.cache_write_tokens.unwrap_or(0).saturating_add(w));
                }
                if let Some(r) = u.reasoning_tokens {
                    total.reasoning_tokens =
                        Some(total.reasoning_tokens.unwrap_or(0).saturating_add(r));
                }
                ctx.emit(Event::ContextUpdate {
                    used: u.prompt_tokens + u.completion_tokens,
                    budget: ctx.setup.budget.unwrap_or(0),
                    window: ctx.setup.model.context_window,
                    cache_hit_tokens: u.cached_prompt_tokens,
                    cache_hit_percent: ctx.stats.lock().ok().and_then(|s| s.cache_hit_percent()),
                    threshold: crate::context::threshold(&ctx.setup),
                });
            }
            let empty =
                matches!(&msg, Message::Assistant { content, .. } if content.trim().is_empty());
            let no_calls =
                matches!(&msg, Message::Assistant { tool_calls, .. } if tool_calls.is_empty());
            let reported = usage.is_some();
            // An empty reply (common with small models after a tool result) gets one nudge.
            // The empty message itself is dropped: some chat templates reject it.
            if no_calls && empty && !nudged {
                nudged = true;
                ctx.emit(Event::Warning {
                    message: "the model returned an empty response; asking it to continue".into(),
                });
                ctx.push(
                    &mut history,
                    Message::Nudge {
                        content: EMPTY_NUDGE.into(),
                    },
                );
                continue;
            }
            ctx.push(&mut history, msg);
            completed_steps += 1;
            if !reported {
                // Servers that do not report usage still get a live context figure.
                ctx.context_estimate(&history);
            }
            // Move the calls out of the history entry rather than cloning them: the
            // arguments hold whole file contents for write/edit calls. They are put
            // back once every call has its result, so the history is unchanged.
            let at = history.len() - 1;
            let calls = match history.last_mut() {
                Some(Message::Assistant { tool_calls, .. }) => std::mem::take(tool_calls),
                _ => Vec::new(),
            };
            if calls.is_empty() {
                if empty {
                    ctx.emit(Event::Warning {
                        message: "the model returned an empty response".into(),
                    });
                }
                break 'steps StepEnd::Done;
            }
            let interrupted = run_tool_calls(&ctx, &mut history, &mut recent, calls, at).await;
            if interrupted {
                break 'steps StepEnd::Aborted("interrupted".into());
            }
            // Tool output can be large; show its weight before the next request.
            ctx.context_estimate(&history);
        }
    };

    match end {
        StepEnd::Done => {
            ctx.emit(Event::TurnComplete {
                turn: ctx.turn,
                usage: total,
            });
            ctx.turn_end(json!({ "turn": ctx.turn.0, "outcome": "complete", "usage": total }));
        }
        StepEnd::Aborted(reason) | StepEnd::Overflow(reason) | StepEnd::Dropped(reason) => {
            ctx.turn_end(json!({ "turn": ctx.turn.0, "outcome": "aborted", "reason": reason }));
            ctx.emit(Event::TurnAborted {
                turn: ctx.turn,
                reason,
            });
        }
        StepEnd::Truncated => unreachable!("truncated replies are handled inside the step loop"),
    }
    TurnOutcome {
        history,
        steps: completed_steps,
        step_limited,
    }
}

/// Run one step's tool calls in order, putting each result into the history as it
/// finishes, then put the calls back into the assistant message they were taken from
/// (`at`), so the history is unchanged. Read-only sub-agents that start together run
/// side by side. Returns whether the turn was interrupted; every call still gets a
/// result, so the next request stays valid.
async fn run_tool_calls(
    ctx: &TurnCtx,
    history: &mut Vec<Message>,
    recent: &mut RecentCalls,
    calls: Vec<ToolCall>,
    at: usize,
) -> bool {
    let mut interrupted = false;
    let mut i = 0;
    // Which calls start a read-only sub-agent, decided once: the scan below would
    // otherwise re-parse each call's arguments every time a batch is measured.
    let side_by_side: Vec<bool> = calls.iter().map(|c| parallel(ctx, c)).collect();
    while i < calls.len() {
        if ctx.cancel.is_cancelled() {
            // Every tool call needs a result, or the next request is invalid.
            for c in &calls[i..] {
                ctx.push(
                    history,
                    Message::Tool {
                        call_id: c.id.clone(),
                        content: "Not run: interrupted by the user.".into(),
                        images: Vec::new(),
                    },
                );
            }
            interrupted = true;
            break;
        }
        // Read-only sub-agents and allowed read-only tools started together run side by side.
        let together = side_by_side[i..].iter().take_while(|p| **p).count();
        if together > 1 {
            let batch = &calls[i..i + together];
            let repeats: Vec<(usize, usize)> = batch
                .iter()
                .map(|c| recent.record(&c.name, &c.arguments))
                .collect();
            let outs = futures::future::join_all(batch.iter().map(|c| run_call(ctx, c, at))).await;
            let end = i + together == calls.len();
            for (n, ((call, (trailing, _)), out)) in batch.iter().zip(repeats).zip(outs).enumerate()
            {
                let target = failure_target(&ctx.setup, call);
                let target_fails = recent.record_outcome(&call.name, target.as_deref(), out.ok);
                let ToolOutput {
                    model: mut content,
                    images,
                    ..
                } = out;
                if trailing >= REPEAT_LIMIT {
                    content.push_str(&format!(
                        "\n\nNote: this exact call has now been made {trailing} times in a row. \
                         Try a different approach."
                    ));
                } else if target_fails >= REPEAT_LIMIT
                    && let Some(target) = target
                {
                    content.push_str(&format!(
                        "\n\nNote: `{}` on `{target}` has failed {target_fails} times in a row. \
                         Stop retrying small variations; re-read the file or diagnose the root error first.",
                        call.name
                    ));
                }
                content = apply_tool_guard(ctx, call, content).await;
                let is_todo = ctx.setup.surface.name_of("todo") == Some(call.name.as_str());
                ctx.keep_plan(&mut content, end && n + 1 == together, is_todo);
                ctx.push(
                    history,
                    Message::Tool {
                        call_id: call.id.clone(),
                        content,
                        images,
                    },
                );
            }
            i += together;
            continue;
        }
        let call = &calls[i];
        i += 1;
        let (repeats, same_count) = recent.record(&call.name, &call.arguments);
        let mut out = run_call(ctx, call, at).await;
        let target = failure_target(&ctx.setup, call);
        let target_fails = recent.record_outcome(&call.name, target.as_deref(), out.ok);
        if repeats >= REPEAT_LIMIT {
            out.model.push_str(&format!(
                "\n\nNote: this exact call has now been made {repeats} times in a row. \
                 Try a different approach."
            ));
        } else if same_count >= 2 && edits(&ctx.setup, call) {
            out.model.push_str(&format!(
                "\n\nNote: this exact edit has already been attempted in this turn ({same_count} times total). \
                 Repeating identical changes without modifying arguments will not fix the problem. \
                 Read the file to check its actual state or try a different fix."
            ));
        } else if target_fails >= REPEAT_LIMIT
            && let Some(target) = target
        {
            out.model.push_str(&format!(
                "\n\nNote: `{}` on `{target}` has failed {target_fails} times in a row. \
                 Stop retrying small variations; re-read the file or diagnose the root error first.",
                call.name
            ));
        }
        out.model = apply_tool_guard(ctx, call, out.model).await;
        let is_todo = ctx.setup.surface.name_of("todo") == Some(call.name.as_str());
        ctx.keep_plan(&mut out.model, i == calls.len(), is_todo);
        ctx.push(
            history,
            Message::Tool {
                call_id: call.id.clone(),
                content: out.model,
                images: out.images,
            },
        );
    }
    if let Some(Message::Assistant { tool_calls, .. }) = history.get_mut(at) {
        *tool_calls = calls;
    }
    interrupted
}

/// The next request's prompt tokens: the byte estimate corrected by reported usage.
pub(crate) fn estimate(s: &Setup, history: &[Message]) -> u32 {
    scale(s, raw_estimate(s, history))
}

/// The estimator's correction of a raw byte estimate, when one has been fitted.
pub(crate) fn scale(s: &Setup, raw: u32) -> u32 {
    s.estimator.lock().map_or(raw, |e| e.scale(raw))
}

/// Rough token count of `bytes` of text or JSON: about 3.5 bytes per token.
pub(crate) fn bytes_to_tokens(bytes: usize) -> u32 {
    u32::try_from(bytes * 2 / 7).unwrap_or(u32::MAX)
}

/// Rough byte size of `tokens`: about 3.5 bytes per token.
pub(crate) fn tokens_to_bytes(tokens: usize) -> usize {
    tokens * 7 / 2
}

/// Uncorrected token estimate for native compaction items (`Message::Compaction.sealed.items`).
/// Counts the encrypted compaction payload's byte-scaled weight plus any retained items
/// without counting unsent fallback text or wrapper metadata.
pub(crate) fn sealed_compaction_tokens(items: &[Value]) -> u32 {
    items.iter().fold(0u32, |acc, item| {
        let item_tokens = if item["type"] == "compaction" {
            item["encrypted_content"]
                .as_str()
                .map_or(0, |s| bytes_to_tokens(s.len()))
        } else {
            tokens::estimate(&item.to_string())
        };
        acc.saturating_add(item_tokens).saturating_add(4)
    })
}

/// Uncorrected token estimate for one history message.
/// - Images use `Image::tokens` instead of base64 byte length.
/// - Assistant messages skip `sealed` so encrypted reasoning ciphertext and duplicate text
///   copies do not inflate the budget estimate.
/// - Compaction messages count `sealed.items` when native compaction is active and fallback
///   `content` otherwise, matching what `prepare_history` sends on the wire.
pub(crate) fn message_tokens(m: &Message, native_compaction: bool) -> u32 {
    let image_tokens = |images: &[quack_protocol::Image]| -> u32 {
        images.iter().fold(0u32, |acc, img| {
            acc.saturating_add(quack_protocol::Image::tokens(img))
        })
    };
    match m {
        Message::System { content } | Message::Nudge { content } => {
            tokens::estimate(content).saturating_add(4)
        }
        Message::User { content, images } => tokens::estimate(content)
            .saturating_add(image_tokens(images))
            .saturating_add(4),
        Message::Compaction {
            content,
            sealed,
            session_context,
        } => {
            let body = if native_compaction {
                sealed_compaction_tokens(&sealed.items)
            } else {
                tokens::estimate(content).saturating_add(4)
            };
            body.saturating_add(session_context.as_deref().map_or(0, tokens::estimate))
        }
        Message::Assistant {
            content,
            reasoning,
            tool_calls,
            sealed: _,
        } => {
            let calls = tool_calls.iter().fold(0u32, |acc, c| {
                acc.saturating_add(tokens::estimate(&c.name))
                    .saturating_add(tokens::estimate(&c.arguments))
                    .saturating_add(8)
            });
            tokens::estimate(content)
                .saturating_add(reasoning.as_deref().map_or(0, tokens::estimate))
                .saturating_add(calls)
                .saturating_add(4)
        }
        Message::Tool {
            call_id: _,
            content,
            images,
        } => tokens::estimate(content)
            .saturating_add(image_tokens(images))
            .saturating_add(8),
    }
}

pub(crate) fn history_tokens(history: &[Message], native_compaction: bool) -> u32 {
    history.iter().fold(0u32, |acc, m| {
        acc.saturating_add(message_tokens(m, native_compaction))
    })
}

/// Uncorrected estimate of a request's prompt: system prompt, history and tool definitions.
/// Image payloads are counted as their rough token cost, not their base64 bytes, which
/// would count a screenshot as tens of thousands of tokens.
pub(crate) fn raw_estimate(s: &Setup, history: &[Message]) -> u32 {
    let history = history_tokens(history, s.provider.native_compaction());
    // The tool block is constant for the setup, so its byte size is computed once.
    let tools: usize = *s.tool_block_bytes.get_or_init(|| {
        tool_specs(s)
            .iter()
            .map(|t| t.description.len() + t.name.len() + t.parameters.to_string().len())
            .sum()
    });
    tokens::estimate(&s.system)
        .saturating_add(tokens::estimate(&s.effective_session_context()))
        .saturating_add(history)
        .saturating_add(bytes_to_tokens(tools))
}

/// The model entry's sampling settings over the profile's defaults.
pub(crate) fn sampling(s: &Setup) -> quack_config::provider::Sampling {
    let mut sampling = s.profile.file.defaults.sampling.clone();
    let m = &s.model.sampling;
    sampling.temperature = m.temperature.or(sampling.temperature);
    sampling.top_p = m.top_p.or(sampling.top_p);
    sampling.top_k = m.top_k.or(sampling.top_k);
    sampling.min_p = m.min_p.or(sampling.min_p);
    sampling.repetition_penalty = m.repetition_penalty.or(sampling.repetition_penalty);
    sampling
}

/// A request to the session's model with its sampling and extra parameters; with the
/// session's tools when `tools` is set.
pub(crate) fn request(s: &Setup, mut messages: Vec<Message>, tools: bool) -> ChatRequest {
    if !s.tool_ctx.vision {
        without_images(&mut messages);
    }
    ChatRequest {
        model: s.model.id.clone(),
        messages,
        tools: if tools { tool_specs(s) } else { Vec::new() },
        context_window: s.model.context_window,
        max_tokens: s.model.max_output,
        sampling: sampling(s),
        reasoning_effort: Some(s.reasoning_effort()).filter(|e| !e.is_empty()),
        preserve_reasoning: s.model.reasoning.preserve_in_history,
        reasoning_wire: s.model.reasoning.wire,
        parallel_tool_calls: (!s.model.tool_calling.parallel).then_some(false),
        prompt_cache: s.provider.file.prompt_cache_for(&s.model),
        comparison_response_id: None,
        extra_body: serde_json::to_value(&s.model.extra_body)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default(),
        cache_key: None,
        tool_mode: Some(s.model.tool_calling.mode),
        utility: false,
    }
}

/// The lowest supported reasoning effort for utility calls (title generation, text
/// compaction) so they don't burn full session thinking tokens.
pub(crate) fn lowest_effort(model: &quack_config::provider::ModelEntry) -> Option<String> {
    let efforts = &model.reasoning.efforts;
    if efforts.is_empty() {
        return None;
    }
    for wanted in ["none", "off", "minimal", "low"] {
        if let Some(found) = efforts.iter().find(|e| e.eq_ignore_ascii_case(wanted)) {
            return Some(found.clone());
        }
    }
    efforts.first().cloned()
}

/// A lightweight tool-free request (title generation, text compaction) with minimal
/// reasoning effort and no reasoning summary/context or thinking-config overhead.
pub(crate) fn utility_request(s: &Setup, messages: Vec<Message>) -> ChatRequest {
    let mut req = request(s, messages, false);
    req.utility = true;
    req.reasoning_effort = lowest_effort(&s.model);
    quack_providers::strip_utility_extra(&mut req.extra_body);
    req
}

/// Images in the history from a model that took them (before a `/models` switch, a
/// resume on another model, or a planner handoff) become a note, since a model without
/// vision would have the whole request rejected. The history itself keeps them.
fn without_images(messages: &mut [Message]) {
    for m in messages {
        if let Message::User { content, images }
        | Message::Tool {
            content, images, ..
        } = m
            && !images.is_empty()
        {
            let n = images.len();
            images.clear();
            if !content.is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(&format!(
                "[{n} image{} not sent: this model takes no images]",
                if n == 1 { "" } else { "s" }
            ));
        }
    }
}

/// The profile's tools, then the MCP tools offered right now.
fn tool_specs(s: &Setup) -> Vec<ToolSpec> {
    let mut specs: Vec<ToolSpec> = s
        .surface
        .tools
        .iter()
        .map(|t| ToolSpec {
            name: t.name.clone(),
            description: t.description.clone(),
            parameters: t.schema.clone(),
        })
        .collect();
    let mcp = if s.mcp_tools {
        s.mcp.offered(s.budget)
    } else {
        Vec::new()
    };
    specs.extend(mcp.into_iter().map(|t| ToolSpec {
        name: t.name().to_owned(),
        description: t.description(&s.tool_ctx),
        parameters: t.schema(),
    }));
    specs
}

/// Every tool name the model may call right now: the surface's names plus the MCP
/// tools offered at the moment. Building only the names avoids the descriptions and
/// schemas of [`tool_specs`], which the unknown-tool path needs on every model call.
fn tool_names(s: &Setup) -> Vec<String> {
    let mut names: Vec<String> = s.surface.tools.iter().map(|t| t.name.clone()).collect();
    if s.mcp_tools {
        names.extend(
            s.mcp
                .offered(s.budget)
                .into_iter()
                .map(|t| t.name().to_owned()),
        );
    }
    names
}

pub(crate) fn reasoning_format(s: &Setup) -> quack_config::provider::ReasoningFormat {
    s.profile
        .file
        .defaults
        .reasoning_format
        .unwrap_or(s.model.reasoning.format)
}

/// Put the session context (`context.md`: workspace, date, instructions) at the start of
/// the first user message. Keeping it out of the system prompt keeps the system prompt and
/// the tool block, which chat templates render after it, the same in every session and
/// project, so the server's prompt cache reuses them. The history keeps what the user typed.
fn with_session_context_text(ctx_text: &str, messages: &mut [Message]) {
    if ctx_text.is_empty() {
        return;
    }
    match messages
        .iter_mut()
        .find(|m| matches!(m, Message::User { .. } | Message::Compaction { .. }))
    {
        Some(Message::User { content, .. }) => {
            *content = format!("{ctx_text}\n\n{content}");
        }
        Some(Message::Compaction {
            session_context, ..
        }) => {
            *session_context = Some(ctx_text.to_owned());
        }
        _ => {}
    }
}

/// The files the session changed, as shown to the model, first change first.
pub(crate) fn changed_files(ctx: &TurnCtx) -> Vec<String> {
    let mut files = Vec::new();
    for c in ctx.checkpoints.iter().flat_map(|cp| cp.changes()) {
        let shown = quack_tools::paths::display(&ctx.setup.tool_ctx.root, &c.path);
        if !files.contains(&shown) {
            files.push(shown);
        }
    }
    files
}

fn working_files_reminder(ctx: &TurnCtx) -> Option<String> {
    let files = changed_files(ctx);
    if files.is_empty() {
        return None;
    }
    Some(format!(
        "[Modified files in this session: {}]",
        files.join(", ")
    ))
}

pub(crate) fn history_messages(s: &Setup, history: &[Message]) -> Vec<Message> {
    history_messages_with(s, None, None, history)
}

fn history_messages_with(
    s: &Setup,
    system_override: Option<&str>,
    context_override: Option<&str>,
    history: &[Message],
) -> Vec<Message> {
    let mut messages = Vec::with_capacity(history.len() + 1);
    let system = system_override.unwrap_or(&s.system);
    if !system.is_empty() {
        messages.push(Message::System {
            content: system.to_owned(),
        });
    }
    messages.extend_from_slice(history);
    let effective_ctx;
    let ctx_text = match context_override {
        Some(c) => c,
        None => {
            effective_ctx = s.effective_session_context();
            &effective_ctx
        }
    };
    with_session_context_text(ctx_text, &mut messages);
    messages
}

pub(crate) fn cache_key(ctx: &TurnCtx) -> String {
    let base = ctx
        .setup
        .context
        .cache_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .unwrap_or(&ctx.session);
    match ctx.agent_type.as_deref().or(ctx.agent.as_deref()) {
        Some(agent) => format!("{base}/{agent}"),
        None => base.to_owned(),
    }
}

/// One model call. Streams deltas to clients and returns the assembled message.
async fn stream_step(
    ctx: &TurnCtx,
    history: &[Message],
) -> Result<(Message, Option<Usage>), StepEnd> {
    let s = &ctx.setup;
    let (sys_override, ctx_override) = ctx.egress_overrides();
    let messages =
        history_messages_with(s, sys_override.as_deref(), ctx_override.as_deref(), history);
    // Plan and modified-files reminders are already written into history on state changes
    // (`TurnCtx::keep_plan`) so earlier messages never mutate between steps.
    let mut req = request(s, messages, true);
    // Sub-agents share one key per agent type so sibling/repeated runs reuse the warm prefix.
    req.cache_key = Some(cache_key(ctx));
    if req.prompt_cache.diagnostics {
        req.comparison_response_id = ctx.responses_session.last_response_id();
    }
    let fingerprint = quack_providers::fingerprint(&req);
    ctx.start_request();
    let mut rx = s.provider.stream_in(
        req,
        reasoning_format(s),
        ctx.cancel.child_token(),
        ctx.responses_session.clone(),
    );
    let mut b = ResponseBuilder::default();
    let sent = std::time::Instant::now();
    let mut first: Option<std::time::Instant> = None;
    let format = toolparse::TextFormat::parse(&s.model.tool_calling.text_format);
    let mut gate = toolparse::TextGate::new(format);
    let result = async {
        loop {
            let item = tokio::select! {
                () = ctx.cancel.cancelled() => None,
                item = rx.recv() => Some(item),
            };
            let Some(item) = item else {
                // Interrupted. The partial text is not kept in history: an assistant message cut
                // mid-sentence confuses some chat templates. The session log will record it.
                return Err(StepEnd::Aborted("interrupted".into()));
            };
            let Some(item) = item else { break };
            match item {
                Ok(ev) => {
                    if first.is_none()
                        && matches!(
                            ev,
                            StreamEvent::TextDelta(_)
                                | StreamEvent::ReasoningDelta(_)
                                | StreamEvent::ToolCallStart { .. }
                        )
                    {
                        first = Some(std::time::Instant::now());
                    }
                    match &ev {
                        StreamEvent::TextDelta(t) => {
                            let text = gate.feed(t);
                            if !text.is_empty() {
                                ctx.emit(Event::AssistantDelta { text });
                            }
                        }
                        StreamEvent::ReasoningDelta(t) => {
                            ctx.emit(Event::ReasoningDelta { text: t.clone() })
                        }
                        _ => {}
                    }
                    b.push(&ev);
                }
                Err(e) => {
                    if e.kind == ErrorKind::ContextLengthExceeded {
                        return Err(StepEnd::Overflow(format!("{}: {e}", s.provider.id)));
                    }
                    if matches!(e.kind, ErrorKind::Timeout | ErrorKind::Network) {
                        return Err(StepEnd::Dropped(format!("{}: {e}", s.provider.id)));
                    }
                    let hint = match e.kind {
                        ErrorKind::Auth => " · check the provider's api_key",
                        _ => "",
                    };
                    ctx.error(
                        EngineErrorKind::Provider,
                        format!("{}: {e}{hint}", s.provider.id),
                    );
                    return Err(StepEnd::Aborted("provider error".into()));
                }
            }
        }
        Ok(())
    }
    .await;
    ctx.record_request(b.usage.as_ref(), sent.elapsed());
    result?;
    match &b.finish {
        None => {
            return Err(StepEnd::Dropped(format!(
                "{}: the stream ended without a finish reason",
                s.provider.id
            )));
        }
        Some(FinishReason::Length) => return Err(StepEnd::Truncated),
        Some(FinishReason::ContentFilter) => {
            ctx.error(
                EngineErrorKind::Provider,
                "The provider filtered the response · check the provider's response policy.",
            );
            return Err(StepEnd::Aborted("response filtered".into()));
        }
        _ => {}
    }
    let tail = gate.finish();
    if !tail.is_empty() {
        ctx.emit(Event::AssistantDelta { text: tail });
    }
    let usage = b.usage;
    if let Some(resp_id) = b.response_id.take() {
        ctx.responses_session.set_response_id(resp_id);
    }
    let mut msg = b.into_message();
    if let Some(first) = first {
        emit_speed(ctx, sent, first, usage, &msg);
    }
    if let Message::Assistant {
        tool_calls,
        content,
        sealed,
        ..
    } = &mut msg
    {
        if let Some(sd) = sealed {
            sd.provider.clone_from(&s.provider.id);
            sd.model.clone_from(&s.model.id);
            sd.fingerprint = fingerprint;
        }
        // Fallback chain: a model that printed its tool call as text still gets it run.
        let names = tool_names(s);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let promoted = tool_calls.is_empty()
            && toolparse::extract(content, format, &names).is_some_and(|(rest, calls)| {
                tracing::debug!(count = calls.len(), "promoted text tool calls");
                *content = rest;
                *tool_calls = calls;
                true
            });
        if promoted {
            // The reply no longer matches what the server sent.
            *sealed = None;
        }
        if promoted || gate.hid() {
            // Promoted: show the text without the calls. Otherwise it only looked like a
            // call, so show what the gate held back.
            ctx.emit(Event::AssistantRewrite {
                text: content.clone(),
            });
        }
        dedupe_ids(tool_calls, history);
    }
    Ok((msg, usage))
}

/// Report time to first token and generation speed. Output tokens come from the server's
/// usage, or are estimated from the reply when it reports none.
fn emit_speed(
    ctx: &TurnCtx,
    sent: std::time::Instant,
    first: std::time::Instant,
    usage: Option<Usage>,
    msg: &Message,
) {
    let generating = first.elapsed().as_secs_f32();
    let out = usage
        .map(|u| u.completion_tokens)
        .unwrap_or_else(|| match msg {
            Message::Assistant {
                content,
                reasoning,
                tool_calls,
                ..
            } => {
                let calls: usize = tool_calls.iter().map(|c| c.arguments.len()).sum();
                tokens::estimate(content)
                    + reasoning.as_deref().map_or(0, tokens::estimate)
                    + bytes_to_tokens(calls)
            }
            _ => 0,
        });
    // Very short replies give meaningless rates.
    let tokens_per_sec = (out >= 8 && generating > 0.05).then(|| out as f32 / generating);
    ctx.emit(Event::Speed {
        ttft_ms: u32::try_from((first - sent).as_millis()).unwrap_or(u32::MAX),
        tokens_per_sec,
    });
}

/// Servers that omit ids get `call_N`, which can collide across steps.
fn dedupe_ids(calls: &mut [ToolCall], history: &[Message]) {
    // Borrowed from the history; only the current batch of ids needs owning.
    let used: HashSet<&str> = history
        .iter()
        .filter_map(|m| match m {
            Message::Tool { call_id, .. } => Some(call_id.as_str()),
            _ => None,
        })
        .collect();
    let mut added: Vec<String> = Vec::new();
    for c in calls {
        let mut n = 1;
        let base = c.id.clone();
        while used.contains(c.id.as_str()) || added.iter().any(|id| id == &c.id) {
            c.id = format!("{base}_{n}");
            n += 1;
        }
        added.push(c.id.clone());
    }
}

/// Lenient argument parsing: empty means `{}`, a JSON string holding an object is unwrapped.
pub(crate) fn parse_args(raw: &str) -> Result<Value, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    let v: Value = serde_json::from_str(raw)
        .or_else(|_| serde_json::from_str(&format!("{raw}}}")))
        .map_err(|e| format!("arguments are not valid JSON ({e}): {raw}"))?;
    match v {
        Value::Object(_) => Ok(v),
        Value::String(s) => parse_args(&s),
        other => Err(format!("arguments must be a JSON object, got {other}")),
    }
}

fn begin(ctx: &TurnCtx, call: &ToolCall, summary: &str) {
    ctx.emit(Event::ToolCallBegin {
        call: ToolCallInfo {
            call_id: CallId(ctx.key(&call.id)),
            name: call.name.clone(),
            summary: summary.to_owned(),
        },
    });
}

fn finish(ctx: &TurnCtx, call: &ToolCall, summary: &str, out: ToolOutput) -> ToolOutput {
    let mut data = serde_json::json!({
        "call_id": call.id,
        "summary": summary,
        "ok": out.ok,
        "result": out.summary,
    });
    // ReplayItem::Tool.detail: the full diff or output, for a collapsible card on resume.
    // Left out when there is none, so older readers of the log stay unaffected.
    if let Some(detail) = &out.detail {
        data["detail"] = Value::String(detail.clone());
    }
    session::record(ctx.log.as_ref(), "tool", data);
    ctx.emit(Event::ToolCallEnd {
        call_id: CallId(ctx.key(&call.id)),
        result: ToolResultSummary {
            ok: out.ok,
            summary: out.summary.clone(),
            detail: out.detail.clone(),
        },
    });
    out
}

/// A call that fails before it reaches a tool.
fn reject(ctx: &TurnCtx, call: &ToolCall, message: String) -> ToolOutput {
    begin(ctx, call, &call.name);
    finish(ctx, call, &call.name, ToolOutput::error(message))
}

/// What happens when a turn reaches `agent.max_steps`.
enum Continue {
    Proceed,
    Stop,
    /// The turn was interrupted while waiting for the user.
    Interrupted,
}

/// At `max_steps` model calls, ask the user whether the turn should keep going
/// (`Op::ContinueTurn`); continuing grants another `max_steps` calls. Sub-agents never
/// ask: they stop at their limit and the model that called them decides what next.
async fn continue_at_limit(ctx: &TurnCtx) -> Continue {
    if ctx.agent.is_some() {
        return Continue::Stop;
    }
    let answer = ctx
        .wait_for_answer(
            &ctx.continues,
            ctx.turn,
            Event::StepLimitReached {
                turn: ctx.turn,
                limit: u32::try_from(ctx.max_steps).unwrap_or(u32::MAX),
            },
        )
        .await;
    session::record(
        ctx.log.as_ref(),
        "step_limit",
        json!({
            "turn": ctx.turn.0,
            "limit": ctx.max_steps,
            "continued": answer,
        }),
    );
    match answer {
        Some(true) => Continue::Proceed,
        Some(false) => Continue::Stop,
        None => Continue::Interrupted,
    }
}

/// Show the user an `ask_user` call's questions and wait for the answers; `None` when
/// the turn is interrupted.
async fn ask(ctx: &TurnCtx, call_id: &CallId, questions: Vec<Question>) -> Option<Answers> {
    let answer = ctx
        .wait_for_answer(
            &ctx.questions,
            call_id.0.clone(),
            Event::Questions {
                call_id: call_id.clone(),
                questions: questions.clone(),
            },
        )
        .await;
    session::record(
        ctx.log.as_ref(),
        "questions",
        json!({
            "call_id": call_id.0,
            "questions": questions,
            "answers": answer.as_ref().map(|a| &a.answers),
            "stop_asking": answer.as_ref().is_some_and(|a| a.stop_asking),
        }),
    );
    answer
}

/// The shared half of asking the user something: one question at a time through
/// `prompt_lock`, the reply channel parked under `key` before the request is shown,
/// the wait counted in the stats, and the parked channel removed again either way.
/// `None` when the asking side was interrupted.
pub(crate) async fn wait_for_answer<T, K: Clone + Eq + Hash>(
    prompt_lock: &tokio::sync::Mutex<()>,
    cancel: &CancellationToken,
    stats: &Mutex<Stats>,
    pending: &Mutex<HashMap<K, oneshot::Sender<T>>>,
    key: K,
    request: Event,
    emit: impl Fn(Event),
) -> Option<T> {
    let _one_at_a_time = prompt_lock.lock().await;
    let (otx, orx) = oneshot::channel();
    if let Ok(mut pending) = pending.lock() {
        pending.insert(key.clone(), otx);
    }
    emit(request);
    let asked = std::time::Instant::now();
    let answer = tokio::select! {
        () = cancel.cancelled() => None,
        a = orx => a.ok(),
    };
    if let Ok(mut st) = stats.lock() {
        st.waited(asked.elapsed());
    }
    if let Ok(mut pending) = pending.lock() {
        pending.remove(&key);
    }
    answer
}

/// The internal tool and arguments behind a call, through the profile's names.
pub(crate) fn internal_call(s: &Setup, call: &ToolCall) -> Option<(String, Value)> {
    let args = parse_args(&call.arguments).ok()?;
    let (internal, mapped) = s.surface.map_call(&call.name, &args)?;
    Some((internal, mapped.unwrap_or(args)))
}

/// Whether a call changes files, whatever the profile calls the tool.
fn edits(s: &Setup, call: &ToolCall) -> bool {
    internal_call(s, call)
        .and_then(|(name, args)| s.registry.get(&name)?.subject(&args, &s.tool_ctx).ok())
        .is_some_and(|sub| matches!(sub, Subject::Edit(_) | Subject::Edits(_)))
}

/// Target key for consecutive-failure doom-loop detection: file path(s) for edits, or
/// the leading command tokens for shell commands.
fn failure_target(s: &Setup, call: &ToolCall) -> Option<String> {
    let (name, args) = internal_call(s, call)?;
    let subject = s.registry.get(&name)?.subject(&args, &s.tool_ctx).ok()?;
    match subject {
        Subject::Edit(p) => Some(quack_tools::paths::display(&s.tool_ctx.root, &p)),
        Subject::Edits(ps) if !ps.is_empty() => {
            let shown: Vec<String> = ps
                .iter()
                .take(3)
                .map(|p| quack_tools::paths::display(&s.tool_ctx.root, p))
                .collect();
            Some(shown.join(", "))
        }
        Subject::Shell(cmd) | Subject::ShellOutsideSandbox(cmd) => {
            let prefix: Vec<&str> = cmd.split_whitespace().take(2).collect();
            (!prefix.is_empty()).then(|| prefix.join(" "))
        }
        _ => None,
    }
}

/// Whether `call` may run side by side with adjacent parallelizable calls:
/// - a read-only sub-agent (`task` with `read_only = true`), or
/// - an allowed `ToolKind::ReadOnly` tool that does not require interactive approval.
fn parallel(ctx: &TurnCtx, call: &ToolCall) -> bool {
    let s = &ctx.setup;
    let Some((internal, args)) = internal_call(s, call) else {
        return false;
    };
    let Some(tool) = s.registry.get(&internal) else {
        return false;
    };
    let Ok(subject) = tool.subject(&args, &s.tool_ctx) else {
        return false;
    };
    if internal == "task" {
        return subject == Subject::Agent { read_only: true };
    }
    // Keep `skill` serialized so duplicate calls to the same skill in one step deduplicate.
    if internal == "skill" {
        return false;
    }
    tool.kind() == ToolKind::ReadOnly
        && ctx
            .perms
            .lock()
            .is_ok_and(|p| matches!(p.check(&subject), Decision::Allow))
}

/// Run one tool call through mapping, permissions and approval.
async fn run_call(ctx: &TurnCtx, call: &ToolCall, history_index: usize) -> ToolOutput {
    let s = &ctx.setup;
    let call_id = CallId(ctx.key(&call.id));
    let plan_version = s.tool_ctx.plan.version();
    let (tool, mut args) = match map_and_find_tool(s, call) {
        Ok(found) => found,
        Err(e) => return reject(ctx, call, e),
    };

    let mut tctx = s.tool_ctx.clone();
    tctx.cancel = ctx.cancel.child_token();
    tctx.client_bridge = ctx.client_bridge.clone();
    // Yolo and `/sandbox off` run commands outside the sandbox; in Plan mode, any active
    // sandbox is tightened to a strict read-only policy (no writable paths, no network).
    if let Ok(p) = ctx.perms.lock() {
        if p.sandbox_off || p.mode == quack_config::settings::PermissionMode::Yolo {
            tctx.sandbox = quack_tools::SandboxState::Off;
        } else if p.work_mode == quack_config::settings::WorkMode::Plan
            && matches!(tctx.sandbox, quack_tools::SandboxState::On(_))
        {
            tctx.sandbox =
                quack_tools::SandboxState::On(Arc::new(quack_sandbox::Policy::default()));
        }
    }
    let summary = tool.summary(&args, &tctx);
    begin(ctx, call, &summary);
    let done = |out: ToolOutput| finish(ctx, call, &summary, out);

    args = match prepare(tool.as_ref(), args, &tctx).await {
        Ok(a) => a,
        Err(e) => return done(ToolOutput::error(e)),
    };
    let subject = match tool.subject(&args, &tctx) {
        Ok(sub) => sub,
        Err(e) => return done(ToolOutput::error(e)),
    };
    let (args, subject, decided_by) =
        match permit(ctx, &tool, call, &call_id, &tctx, args, subject).await {
            Ok(passed) => passed,
            Err(denied) => {
                if let Some(audit) = &ctx.audit {
                    let subj = crate::audit::subject_label(&denied.subject, &tctx.root);
                    audit.record(crate::audit::AuditEntry {
                        session_id: &ctx.session,
                        agent: ctx.agent.as_deref(),
                        call_id: &call_id.0,
                        tool: tool.name(),
                        subject: &subj,
                        decision: denied.decision,
                        decided_by: &denied.decided_by,
                        ok: None,
                        args: audit.include_content().then_some(&denied.args),
                        summary: Some(&denied.message),
                    });
                }
                return done(ToolOutput::error(denied.message));
            }
        };
    let audit_subj = ctx
        .audit
        .as_ref()
        .map(|_| crate::audit::subject_label(&subject, &tctx.root));
    let audit_args = ctx
        .audit
        .as_ref()
        .filter(|a| a.include_content())
        .map(|_| args.clone());
    let out = execute(
        ctx,
        &tool,
        &call_id,
        tctx,
        args,
        subject,
        plan_version,
        history_index,
    )
    .await;
    if let (Some(audit), Some(subj)) = (&ctx.audit, &audit_subj) {
        audit.record(crate::audit::AuditEntry {
            session_id: &ctx.session,
            agent: ctx.agent.as_deref(),
            call_id: &call_id.0,
            tool: tool.name(),
            subject: subj,
            decision: crate::audit::AuditDecision::Allow,
            decided_by: &decided_by,
            ok: Some(out.ok),
            args: audit_args.as_ref(),
            summary: Some(&out.summary),
        });
    }
    done(out)
}

struct PermitDenied {
    message: String,
    decision: crate::audit::AuditDecision,
    decided_by: String,
    args: Value,
    subject: Subject,
}

/// The permission side of a call: the pre-tool hook (which can deny the call or
/// rewrite its arguments), the permission decision, the approval ask when the rules
/// say `Ask`, and the recheck for plan mode entered while the ask was on screen.
/// `Ok` carries the arguments, subject, and who decided (`rule`, `user`, `hook:...`);
/// `Err` carries the denial message and audit metadata.
async fn permit(
    ctx: &TurnCtx,
    tool: &Arc<dyn Tool>,
    call: &ToolCall,
    call_id: &CallId,
    tctx: &ToolCtx,
    mut args: Value,
    mut subject: Subject,
) -> Result<(Value, Subject, String), Box<PermitDenied>> {
    use crate::audit::AuditDecision;
    let deny = |message: String, decision, decided_by: String, args, subject| {
        Box::new(PermitDenied {
            message,
            decision,
            decided_by,
            args,
            subject,
        })
    };
    let root = &ctx.setup.tool_ctx.root;
    let mut hooked: Option<(Verdict, String)> = None;
    if ctx.hooks.any(HookEvent::PreTool) {
        let target = hooks::call_target(&subject, &args, root);
        let h = ctx
            .hooks(
                HookEvent::PreTool,
                json!({ "tool": tool.name(), "args": args, "target": target }),
                Target::Call {
                    tool: tool.name(),
                    target: &target,
                },
            )
            .await;
        if let Some((id, reason)) = h.denied() {
            let reason = if reason.is_empty() {
                "no reason given"
            } else {
                reason
            };
            return Err(deny(
                format!("blocked by hook `{id}`: {reason}"),
                AuditDecision::Deny,
                format!("hook:{id}"),
                args,
                subject,
            ));
        }
        if let Some(updated) = h.updated_args.clone() {
            args = match prepare(tool.as_ref(), updated, tctx).await {
                Ok(a) => a,
                Err(e) => {
                    return Err(deny(e, AuditDecision::Deny, "hook".into(), args, subject));
                }
            };
            subject = match tool.subject(&args, tctx) {
                Ok(s) => s,
                Err(e) => {
                    return Err(deny(e, AuditDecision::Deny, "hook".into(), args, subject));
                }
            };
        }
        hooked = h.verdict;
    }
    let read_tools: Vec<&str> = ["read", "grep", "glob", "list_dir", "index", "lsp"]
        .into_iter()
        .filter_map(|t| ctx.setup.surface.name_of(t))
        .collect();
    let rule_decision = ctx
        .perms
        .lock()
        .map(|p| p.check_with_tools(&subject, Some(&read_tools)))
        .unwrap_or(Decision::Ask);
    // A deny rule beats a hook's allow, and a command outside the sandbox always asks.
    let (decision, mut decided_by) = match (&hooked, rule_decision) {
        (_, deny @ Decision::Deny(_)) => (deny, "rule".to_owned()),
        (Some((Verdict::Ask, id)), _) => (Decision::Ask, format!("hook:{id}")),
        (Some((Verdict::Allow, id)), Decision::Ask)
            if !matches!(subject, Subject::ShellOutsideSandbox(_)) =>
        {
            (Decision::Allow, format!("hook:{id}"))
        }
        (_, d) => (d, "rule".to_owned()),
    };
    match decision {
        Decision::Allow => {}
        Decision::Deny(reason) => {
            return Err(deny(
                format!("permission denied: {reason}"),
                AuditDecision::Deny,
                decided_by,
                args,
                subject,
            ));
        }
        Decision::Ask => {
            // If the call would fail anyway (e.g. old_string not found), skip asking.
            let request = match tool.preview(&args, tctx) {
                Ok(r) => r,
                Err(e) => {
                    return Err(deny(e, AuditDecision::Deny, decided_by, args, subject));
                }
            };
            let always = ctx
                .perms
                .lock()
                .map(|p| p.rules_for(&subject))
                .unwrap_or_default();
            let answer = ctx
                .wait_for_answer(
                    &ctx.approvals,
                    call_id.0.clone(),
                    Event::ApprovalRequest {
                        call_id: call_id.clone(),
                        request,
                        always: always.clone(),
                    },
                )
                .await;
            session::record(
                ctx.log.as_ref(),
                "approval",
                serde_json::json!({ "call_id": call.id, "decision": answer }),
            );
            decided_by = "user".to_owned();
            match answer {
                None => {
                    return Err(deny(
                        "interrupted by the user".into(),
                        AuditDecision::Interrupted,
                        decided_by,
                        args,
                        subject,
                    ));
                }
                Some(ApprovalDecision::AllowOnce | ApprovalDecision::RedactAndSend) => {}
                Some(ApprovalDecision::AllowSession) => {
                    if let Ok(mut p) = ctx.perms.lock() {
                        p.allow_session(subject.clone());
                    }
                }
                Some(ApprovalDecision::AlwaysAllow { scope }) => {
                    if let Ok(mut p) = ctx.perms.lock() {
                        p.allow_session(subject.clone());
                    }
                    save_always(ctx, &scope, &always);
                }
                Some(ApprovalDecision::Deny) => {
                    return Err(deny(
                        "the user declined this action. Ask them how to proceed if unsure.".into(),
                        AuditDecision::Deny,
                        decided_by,
                        args,
                        subject,
                    ));
                }
                Some(ApprovalDecision::DenyWithFeedback { feedback }) => {
                    return Err(deny(
                        format!("the user declined this action and said: {feedback}"),
                        AuditDecision::Deny,
                        decided_by,
                        args,
                        subject,
                    ));
                }
            }
        }
    }

    // The user may have entered Plan while an approval dialog was open. Recheck
    // restrictions before any checkpoint or tool execution, without asking twice.
    if let Ok(p) = ctx.perms.lock()
        && let Decision::Deny(reason) = p.check_with_tools(&subject, Some(&read_tools))
    {
        return Err(deny(
            format!("permission denied: {reason}"),
            AuditDecision::Deny,
            "rule".into(),
            args,
            subject,
        ));
    }
    Ok((args, subject, decided_by))
}

/// The execution side of a call: undo points and diagnostics before the run, the run
/// itself — with the questions it asks and the sub-agents it starts answered along
/// the way — then the undo records, post-edit diagnostics, nested AGENTS.md context
/// and the post-tool hook. Returns the raw output; the caller finishes it.
#[allow(clippy::too_many_arguments)]
async fn execute(
    ctx: &TurnCtx,
    tool: &Arc<dyn Tool>,
    call_id: &CallId,
    mut tctx: ToolCtx,
    args: Value,
    subject: Subject,
    plan_version: u64,
    history_index: usize,
) -> ToolOutput {
    let s = &ctx.setup;
    let root = &s.tool_ctx.root;
    let (otx, mut orx) = mpsc::unbounded_channel::<String>();
    tctx.on_output = Some(otx);
    let tx = ctx.tx.clone();
    let id = call_id.clone();
    let forward = tokio::spawn(async move {
        while let Some(chunk) = orx.recv().await {
            let _ = tx.send(Event::ToolCallOutputDelta {
                call_id: id.clone(),
                chunk,
            });
        }
    });
    // An undo point for each file the call changes.
    let checkpoints: Vec<_> = match &ctx.checkpoints {
        Some(cp) => subject
            .edited()
            .iter()
            .filter_map(|p| match cp.before(ctx.turn.0, &call_id.0, p) {
                Ok(seq) => Some((cp, seq, p)),
                Err(e) => {
                    tracing::warn!(error = %e, path = %p.display(), "no undo point for this edit");
                    None
                }
            })
            .collect(),
        None => Vec::new(),
    };
    // Diagnostics before an edit (single or multi-file), to report only what the edit changed.
    let lsp = tctx.lsp.clone();
    let mut lsp_before = Vec::new();
    if let Some(m) = &lsp {
        for p in subject.edited() {
            if let Some(snap) = m.before_edit(p).await {
                lsp_before.push((p.clone(), snap));
            }
        }
    }
    // Shadow git snapshot before a mutating shell command so `/undo`, `/rewind`, and LSP
    // `workspace/didChangeWatchedFiles` track files created, modified, or deleted by shell.
    let shell_snapshot = match &subject {
        Subject::Shell(cmd) | Subject::ShellOutsideSandbox(cmd)
            if !quack_tools::is_read_only_shell(cmd) =>
        {
            let shadow_dir = ctx
                .checkpoints
                .as_ref()
                .map(|cp| cp.shadow_git_dir())
                .or_else(|| {
                    lsp.as_ref()
                        .is_some_and(|m| !m.clients().is_empty())
                        .then(|| {
                            std::env::temp_dir()
                                .join(format!("quack-shadow-{}", std::process::id()))
                        })
                });
            if let Some(shadow_dir) = shadow_dir {
                let ws = root.clone();
                let sdir = shadow_dir.clone();
                tokio::task::spawn_blocking(move || quack_store::snapshot::capture_tree(&sdir, &ws))
                    .await
                    .ok()
                    .flatten()
                    .map(|tree| (shadow_dir, tree))
            } else {
                None
            }
        }
        _ => None,
    };
    let post_args = ctx.hooks.any(HookEvent::PostTool).then(|| args.clone());
    let (atx, mut arx) = mpsc::unbounded_channel();
    tctx.ask = Some(atx);
    let (stx, mut srx) = mpsc::unbounded_channel();
    tctx.spawn = Some(stx);
    // Deduplicate `skill` calls when the skill's verbatim instructions are already in context.
    let dedup_skill = match &subject {
        Subject::Skill(name)
            if ctx
                .loaded_skills
                .lock()
                .ok()
                .is_some_and(|s| s.is_verbatim(name)) =>
        {
            Some(name.clone())
        }
        _ => None,
    };
    // The call runs until done; questions it asks on the way go to the user, and
    // sub-agents it starts run here.
    let mut out = if let Some(name) = &dedup_skill {
        ToolOutput::ok(
            format!(
                "Skill `{name}` is already loaded in this conversation; follow the instructions \
                 already in context."
            ),
            format!("skill {name} already loaded"),
        )
    } else {
        let run = tool.run(args, &tctx);
        tokio::pin!(run);
        loop {
            tokio::select! {
                o = &mut run => break o,
                Some((questions, reply)) = arx.recv() => {
                    // Dropping `reply` when interrupted lets the call see the cancellation.
                    if let Some(a) = ask(ctx, call_id, questions).await {
                        let _ = reply.send(a);
                    }
                }
                Some((request, reply)) = srx.recv() => {
                    let _ = reply.send(crate::agent::run(ctx, &call_id.0, request).await);
                }
            }
        }
    };
    if s.tool_ctx.plan.version() != plan_version {
        let checklist = s.tool_ctx.plan.checklist();
        // The session's plan is restored from the log on resume; a sub-agent's is its own.
        if ctx.agent.is_none() {
            session::record(ctx.log.as_ref(), "plan", json!({ "checklist": checklist }));
        }
        ctx.emit(Event::Plan { checklist });
    }
    if out.ok
        && dedup_skill.is_none()
        && let Subject::Skill(name) = &subject
    {
        if ctx.agent.is_none() {
            session::record(ctx.log.as_ref(), "skill", json!({ "name": name }));
        }
        let updated = ctx
            .loaded_skills
            .lock()
            .ok()
            .and_then(|mut s| s.record(history_index, name.clone()).then(|| s.names()));
        if ctx.agent.is_none()
            && let Some(skills) = updated
        {
            ctx.emit(Event::LoadedSkills { skills });
        }
    }
    drop(tctx);
    let _ = forward.await;
    for (cp, seq, p) in checkpoints {
        if let Err(e) = cp.after(seq, p, out.ok) {
            tracing::warn!(error = %e, "could not record the edit for undo");
        }
    }
    if let Some((shadow_dir, before_tree)) = shell_snapshot {
        let ws = root.clone();
        let changes = tokio::task::spawn_blocking(move || {
            quack_store::snapshot::diff_after(&shadow_dir, &ws, &before_tree)
        })
        .await
        .unwrap_or_default();
        if !changes.is_empty() {
            if let Some(cp) = &ctx.checkpoints {
                for change in &changes {
                    if let Err(e) = cp.record_snapshot(
                        ctx.turn.0,
                        &call_id.0,
                        &change.path,
                        change.before.as_deref(),
                    ) {
                        tracing::warn!(
                            error = %e,
                            path = %change.path.display(),
                            "could not record shell change for undo"
                        );
                    }
                }
            }
            if let Some(m) = &lsp {
                let watched: Vec<(std::path::PathBuf, quack_lsp::WatchChangeType)> = changes
                    .iter()
                    .map(|c| {
                        let kind = match c.kind {
                            quack_store::snapshot::FileChangeKind::Created => {
                                quack_lsp::WatchChangeType::Created
                            }
                            quack_store::snapshot::FileChangeKind::Changed => {
                                quack_lsp::WatchChangeType::Changed
                            }
                            quack_store::snapshot::FileChangeKind::Deleted => {
                                quack_lsp::WatchChangeType::Deleted
                            }
                        };
                        (c.path.clone(), kind)
                    })
                    .collect();
                m.notify_watched_files(&watched).await;
            }
        }
    }
    if out.ok
        && let Some(m) = &lsp
    {
        match &subject {
            Subject::Read(p) => m.touch(p),
            Subject::Edit(_) | Subject::Edits(_) => {
                let root = ctx.setup.tool_ctx.root.clone();
                let display = move |f: &std::path::Path| quack_tools::paths::display(&root, f);
                let mut reports = Vec::new();
                let mut total_new: u32 = 0;
                for (p, before) in &lsp_before {
                    let report = tokio::select! {
                        r = m.after_edit(p, before, &display) => r,
                        () = ctx.cancel.cancelled() => None,
                    };
                    if let Some(text) = report {
                        if let Some(n) = text
                            .strip_prefix("diagnostics (")
                            .and_then(|t| t.split(' ').next())
                            .and_then(|n| n.parse::<u32>().ok())
                        {
                            total_new = total_new.saturating_add(n);
                        }
                        reports.push(text);
                    }
                }
                // Any edited path without a pre-edit snapshot still notifies its server.
                for p in subject.edited() {
                    if !lsp_before.iter().any(|(bp, _)| bp == p) {
                        m.touch(p);
                    }
                }
                if !reports.is_empty() {
                    if total_new > 0 {
                        out.summary
                            .push_str(&format!(" · {total_new} new diagnostic(s)"));
                    }
                    let combined = reports.join("\n\n");
                    out.model.push_str("\n\n");
                    out.model.push_str(&combined);
                    out.detail = Some(match out.detail.take() {
                        Some(d) => format!("{d}\n{combined}"),
                        None => combined,
                    });
                }
            }
            _ => {}
        }
    }
    // AGENTS.md files below the root arrive with the first result that touches them
    // (including multi-file edits such as `apply_patch` and `edit_blocks`).
    if out.ok {
        let mut extras = Vec::new();
        match &subject {
            Subject::Read(p) | Subject::Edit(p) => {
                if let Some(extra) = ctx.nested.attach(p) {
                    extras.push(extra);
                }
            }
            Subject::Edits(ps) => {
                for p in ps {
                    if let Some(extra) = ctx.nested.attach(p) {
                        extras.push(extra);
                    }
                }
            }
            _ => {}
        }
        for extra in extras {
            out.model.push_str("\n\n");
            out.model.push_str(&extra);
        }
    }
    if let Some(args) = post_args
        && !ctx.cancel.is_cancelled()
    {
        let target = hooks::call_target(&subject, &args, root);
        let h = ctx
            .hooks(
                HookEvent::PostTool,
                json!({
                    "tool": tool.name(),
                    "args": args,
                    "target": target,
                    "ok": out.ok,
                    "output": out.model,
                }),
                Target::Call {
                    tool: tool.name(),
                    target: &target,
                },
            )
            .await;
        if let Some(text) = h.context_text() {
            out.model.push_str("\n\n");
            out.model.push_str(&text);
        }
    }
    out
}

/// Parse a call's arguments and find the tool behind its name: the surface's mapped
/// internal tool, or an offered MCP tool. `Err` is the message the call is rejected with.
fn map_and_find_tool(s: &Setup, call: &ToolCall) -> Result<(Arc<dyn Tool>, Value), String> {
    let args = parse_args(&call.arguments)?;
    match s.surface.map_call(&call.name, &args) {
        Some((internal, mapped)) => match s.registry.get(&internal) {
            Some(tool) => Ok((tool, mapped.unwrap_or(args))),
            None => Err(format!("tool `{internal}` is not available")),
        },
        None => match s.mcp.find(&call.name, s.budget).filter(|_| s.mcp_tools) {
            Some(tool) => Ok((tool, args)),
            None => Err(format!(
                "unknown tool `{}`; available tools: {}",
                call.name,
                tool_names(s).join(", ")
            )),
        },
    }
}

/// A tool's work before the permission check, stopped by an interrupt.
async fn prepare(tool: &dyn Tool, args: Value, tctx: &ToolCtx) -> Result<Value, String> {
    tokio::select! {
        r = tool.prepare(args, tctx) => r,
        () = tctx.cancel.cancelled() => Err("interrupted by the user".into()),
    }
}

/// "Always allow": save the call's rules to config, so later sessions allow it too.
pub(crate) fn save_always(ctx: &TurnCtx, scope: &WriteScope, rules: &[String]) {
    if rules.is_empty() || *scope == WriteScope::Session {
        return;
    }
    let project = *scope == WriteScope::Project;
    match ctx.pool.config.save_allow_rules(project, rules) {
        Ok(path) => ctx.emit(Event::SettingSaved {
            key: "permissions.allow".into(),
            value: rules.join(", "),
            path: path.display().to_string(),
            note: None,
        }),
        Err(e) => ctx.emit(Event::Warning {
            message: format!("allowed for this session only; cannot save the rule: {e}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_become_a_note_for_a_model_without_vision() {
        let img = || quack_protocol::Image {
            media_type: "image/png".into(),
            data: "eHk=".into(),
            width: Some(2),
            height: Some(1),
        };
        let mut messages = vec![
            Message::User {
                content: String::new(),
                images: vec![img(), img()],
            },
            Message::Tool {
                call_id: "c1".into(),
                content: "[image: pic.png]".into(),
                images: vec![img()],
            },
        ];
        without_images(&mut messages);
        let Message::User { content, images } = &messages[0] else {
            panic!("user message")
        };
        assert!(images.is_empty());
        assert_eq!(content, "[2 images not sent: this model takes no images]");
        let Message::Tool {
            content, images, ..
        } = &messages[1]
        else {
            panic!("tool message")
        };
        assert!(images.is_empty());
        assert_eq!(
            content,
            "[image: pic.png]\n\n[1 image not sent: this model takes no images]"
        );
    }

    #[test]
    fn lenient_args() {
        assert_eq!(parse_args(""), Ok(serde_json::json!({})));
        assert_eq!(
            parse_args(r#""{\"a\":1}""#),
            Ok(serde_json::json!({"a": 1}))
        );
        assert_eq!(parse_args(r#"{"a":1"#), Ok(serde_json::json!({"a": 1})));
        assert!(parse_args("[1]").is_err());
    }

    #[test]
    fn duplicate_ids_are_renamed() {
        let history = vec![Message::Tool {
            call_id: "call_0".into(),
            content: String::new(),
            images: Vec::new(),
        }];
        let mut calls = vec![ToolCall {
            id: "call_0".into(),
            name: "read".into(),
            arguments: "{}".into(),
        }];
        dedupe_ids(&mut calls, &history);
        assert_eq!(calls[0].id, "call_0_1");
    }

    #[test]
    fn message_tokens_skips_assistant_sealed_ciphertext_and_picks_wire_compaction_body() {
        let huge_blob = "A".repeat(70_000);
        let asst_plain = Message::Assistant {
            content: "Look at src/lib.rs".into(),
            reasoning: Some("Check the parser".into()),
            tool_calls: vec![ToolCall {
                id: "c1".into(),
                name: "read".into(),
                arguments: r#"{"path":"src/lib.rs"}"#.into(),
            }],
            sealed: None,
        };
        let asst_sealed = Message::Assistant {
            content: "Look at src/lib.rs".into(),
            reasoning: Some("Check the parser".into()),
            tool_calls: vec![ToolCall {
                id: "c1".into(),
                name: "read".into(),
                arguments: r#"{"path":"src/lib.rs"}"#.into(),
            }],
            sealed: Some(quack_providers::Sealed {
                provider: "openai".into(),
                model: "gpt-6.2-sol".into(),
                fingerprint: "fp".into(),
                items: vec![
                    json!({ "type": "reasoning", "encrypted_content": huge_blob }),
                    json!({ "type": "message", "text": "Look at src/lib.rs", "phase": "commentary" }),
                ],
            }),
        };
        assert_eq!(
            message_tokens(&asst_plain, true),
            message_tokens(&asst_sealed, true),
            "sealed ciphertext and duplicate text must not inflate assistant estimates"
        );

        let compaction = Message::Compaction {
            content: "fallback ".repeat(5_000),
            sealed: quack_providers::Sealed {
                provider: "openai".into(),
                model: "gpt-6.2-sol".into(),
                fingerprint: "fp".into(),
                items: vec![json!({
                    "type": "compaction",
                    "encrypted_content": "B".repeat(3_500)
                })],
            },
            session_context: None,
        };
        // Native mode counts the wire compaction item (~1000 tokens), not the 45KB fallback text.
        let native_est = message_tokens(&compaction, true);
        assert!(
            (950..=1_100).contains(&native_est),
            "expected ~1004 native tokens, got {native_est}"
        );
        // Non-native mode counts the portable fallback text (~12,857 tokens), not sealed.
        let fallback_est = message_tokens(&compaction, false);
        assert!(
            fallback_est > 10_000,
            "expected fallback text tokens, got {fallback_est}"
        );

        // Multi-byte UTF-8 text alongside a large base64 image must estimate cleanly.
        let utf8_with_image = Message::User {
            content: "é".repeat(100),
            images: vec![quack_protocol::Image {
                media_type: "image/png".into(),
                data: "X".repeat(50_000),
                width: Some(512),
                height: Some(512),
            }],
        };
        let est = message_tokens(&utf8_with_image, true);
        assert!(est < 2_000, "image base64 must not inflate estimate: {est}");
    }
}
