//! Engine event handling and background syntax highlighting updates.

use super::*;

impl App {
    /// Apply one engine event and schedule any transcript highlighting it requires.
    pub fn on_engine(&mut self, event: Event) {
        self.dirty = true;
        // Assistant text changed: the last entry, or any of them.
        let code = match &event {
            Event::AssistantDelta { .. } => Some(false),
            Event::AssistantRewrite { .. } | Event::SessionResumed { .. } => Some(true),
            _ => None,
        };
        if let Event::ModelCatalog { providers, .. } = &event {
            for p in providers {
                match &p.key {
                    Some(key) => self.key_sources.insert(p.id.clone(), key.clone()),
                    None => self.key_sources.remove(&p.id),
                };
            }
        }
        self.on_engine_event(event);
        if let Some(all) = code {
            self.highlight_code(all);
        }
    }

    /// Highlight the closed code blocks of assistant messages: the last one, or all.
    pub(super) fn highlight_code(&mut self, all: bool) {
        let skip = if all {
            0
        } else {
            self.transcript.len().saturating_sub(1)
        };
        for entry in &self.transcript[skip..] {
            if let Entry::Assistant(text) = entry {
                self.highlights.scan(text);
            }
        }
    }

    /// A highlight finished on the blocking thread.
    pub fn on_highlighted(&mut self, done: (u64, Option<crate::highlight::Runs>)) {
        self.highlights.done(done);
        self.dirty = true;
    }

    /// Where highlights done off-thread arrive; the event loop takes it once.
    pub fn take_highlight_results(
        &mut self,
    ) -> Option<tokio::sync::mpsc::UnboundedReceiver<(u64, Option<crate::highlight::Runs>)>> {
        self.highlights.take_results()
    }

    /// Rebuild a modal with `title` if it is still open; late responses leave other UI alone.
    pub(super) fn refresh_modal(
        &mut self,
        title: &str,
        build: impl FnOnce(&Self) -> Modal,
    ) -> bool {
        if !self.modal.as_ref().is_some_and(|m| m.title == title) {
            return false;
        }
        self.modal = Some(build(self));
        true
    }

    pub(super) fn on_engine_event(&mut self, event: Event) {
        match event {
            Event::StartupProgress { subsystem, state } => {
                if let Some(row) = self.startup.iter_mut().find(|(s, _)| *s == subsystem) {
                    row.1 = state;
                }
            }
            Event::StartupComplete => {
                self.startup_done = true;
                // A first run with nothing set up: show how to add a provider instead of
                // waiting for the user to find /setup. `info.model` starts as `[model]
                // default`, so a configured model whose server is down does not count.
                if self.info.model.is_none()
                    && !self.setup_offered
                    && self.modal.is_none()
                    && self.prompt.is_none()
                    && self.trust.is_none()
                {
                    self.setup_offered = true;
                    self.open_setup();
                }
            }
            Event::ConfigReloaded => {
                self.startup_done = false;
                self.startup_dismissed = false;
                self.trust = None;
                self.note("config reloaded".into());
                if self.modal.as_ref().is_some_and(|m| m.title == "Providers") {
                    self.ops.push(Op::ListProviderConnections);
                }
                self.refresh_modal("Config", |app| {
                    modal::config(
                        &app.info.settings,
                        &app.info.layers,
                        app.write_scope,
                        app.info.project_status,
                        &app.theme,
                    )
                    .keep_place(app.modal.as_ref())
                });
                self.refresh_modal("Hooks", |app| {
                    modal::hooks(&app.info.hooks, &app.hook_runs, app.write_scope, &app.theme)
                        .keep_place(app.modal.as_ref())
                });
                if self.refresh_modal("MCP", |app| {
                    modal::mcp(None, &app.info.mcp_items, app.write_scope, &app.theme)
                        .keep_place(app.modal.as_ref())
                }) {
                    self.mcp_servers_cache = None;
                    self.ops.push(Op::ListMcp);
                }
            }
            Event::ProjectTrust {
                path,
                summary,
                changed,
            } => {
                let prompt = modal::TrustPrompt {
                    path,
                    summary,
                    changed,
                };
                // Offer the choice once it arrives, unless the user is busy with something.
                if self.modal.is_none() && self.approval.is_none() {
                    self.modal = Some(modal::trust(Some(&prompt), &self.theme));
                }
                self.trust = Some(prompt);
            }
            Event::TurnQueued { waiting_for } => {
                self.queued = true;
                self.note(format!("queued, waiting for {waiting_for}"));
            }
            Event::SessionConfigured { session } => {
                self.session_id = Some(session.id.0);
                if std::mem::take(&mut self.starting_new) {
                    self.session_saved = false;
                }
                self.info.model = session.model;
                self.info.profile = session.profile;
                self.reasoning_effort = session.reasoning_effort;
                self.reasoning_efforts = session.reasoning_efforts;
                if self.modal.as_ref().is_some_and(|m| m.title == "Thinking") {
                    self.open_thinking();
                }
                // Selecting a model that declares thinking levels asks for one right away:
                // the level changes both how the model works and what it costs.
                if let Some(expected) = self.pick_thinking.take()
                    && self.info.model.as_deref() == Some(expected.as_str())
                    && !self.reasoning_efforts.is_empty()
                    && self.modal.is_none()
                {
                    self.open_thinking();
                }
            }
            Event::SessionTitle { title } => {
                self.session_title = title;
            }
            Event::ModelCatalog { providers, current }
                if self.modal.as_ref().is_some_and(|m| m.title == "Providers") =>
            {
                self.catalog = providers;
                self.catalog_loaded = true;
                self.refresh_providers(current.as_deref());
            }
            Event::ProviderConnections { connections } => {
                self.provider_connections = Some(connections);
                self.refresh_providers(self.info.model.clone().as_deref());
            }
            Event::ProviderConnectionSaved { connection, path } => {
                self.note(format!(
                    "saved {} connection settings in {path}",
                    connection.id
                ));
                match &mut self.provider_connections {
                    Some(list) => {
                        if let Some(item) = list.iter_mut().find(|p| p.id == connection.id) {
                            *item = connection;
                        } else {
                            list.push(connection);
                        }
                    }
                    None => self.provider_connections = Some(vec![connection]),
                }
                self.refresh_providers(self.info.model.clone().as_deref());
                if self.modal.as_ref().is_some_and(|m| m.title == "Providers") {
                    self.ops.push(Op::ListModels);
                }
            }
            Event::ModelCatalog { providers, current } => {
                self.refresh_modal("Models", |app| {
                    let mut roles = app.model_roles();
                    roles.current = current.as_deref();
                    modal::models(
                        Some(&providers),
                        roles,
                        app.info.ui.ascii,
                        app.write_scope,
                        &app.theme,
                    )
                    .keep_place(app.modal.as_ref())
                });
                self.catalog = providers;
                self.catalog_loaded = true;
            }
            Event::Instructions { files } => {
                self.refresh_modal("Instructions", |app| {
                    modal::instructions(Some(&files), &app.theme)
                });
            }
            Event::Profiles {
                active,
                forced,
                profiles,
            } => {
                self.refresh_modal("Profiles", |app| {
                    modal::profiles(
                        Some((&active, forced, &profiles)),
                        app.write_scope,
                        &app.theme,
                    )
                    .keep_place(app.modal.as_ref())
                });
                self.profiles_cache = Some((active, forced, profiles));
            }
            Event::Skills { skills } => {
                self.skills = skills;
                self.popup_epoch = self.popup_epoch.wrapping_add(1);
            }
            Event::Commands { commands } => {
                let (hidden, kept): (Vec<_>, Vec<_>) = commands
                    .into_iter()
                    .partition(|c| commands::find(&c.name).is_some());
                for c in hidden {
                    self.transcript.push(Entry::Warning(format!(
                        "/{} in {} is not used: a built-in command has that name",
                        c.name, c.path
                    )));
                }
                self.custom = kept;
                self.popup_epoch = self.popup_epoch.wrapping_add(1);
            }
            Event::McpPrompts { prompts } => {
                self.mcp_prompts = prompts;
                self.popup_epoch = self.popup_epoch.wrapping_add(1);
            }
            Event::FilesRestored {
                restored,
                conflicts,
            } => {
                if !restored.is_empty() {
                    self.note(format!(
                        "undone: {} (the model is told at the start of your next message)",
                        restored.join(", ")
                    ));
                }
                if !conflicts.is_empty() {
                    self.transcript.push(Entry::Warning(format!(
                        "not undone, changed since the edit: {}. /undo --force overwrites them",
                        conflicts.join(", ")
                    )));
                }
            }
            Event::RewindPoints { points } => {
                self.refresh_modal("Rewind", |app| {
                    modal::rewind(Some(&points), &app.theme)
                });
            }
            Event::Rewound { back, text } => {
                // Cut the transcript at the same user message, counted from the end.
                let at = self
                    .transcript
                    .iter()
                    .enumerate()
                    .rev()
                    .filter(|(_, e)| matches!(e, Entry::User(_)))
                    .nth(back.saturating_sub(1) as usize)
                    .map(|(i, _)| i);
                if let Some(i) = at {
                    self.transcript.truncate(i);
                }
                self.context = None;
                self.growth.clear();
                if self.input.is_empty() {
                    self.input.set(&text);
                }
                self.note("rewound; the message is back in the input to edit or send again".into());
            }
            Event::McpServers { servers } => {
                self.refresh_modal("MCP", |app| {
                    modal::mcp(
                        Some(&servers),
                        &app.info.mcp_items,
                        app.write_scope,
                        &app.theme,
                    )
                    .keep_place(app.modal.as_ref())
                });
                self.mcp_servers_cache = Some(servers);
            }
            Event::McpChanged { message, .. } => {
                self.note(message);
            }
            Event::LspServers { servers } => {
                self.refresh_modal("LSP", |app| modal::lsp(Some(&servers), &app.theme));
            }
            Event::ServersDetected { servers } => {
                self.setup.local = modal::Scan::Found(servers);
                self.refresh_setup();
            }
            Event::SignInUrl { id, url } if self.signing_in.as_deref() == Some(&id) => {
                self.modal = Some(modal::sign_in(&id, Some(&url), &self.theme));
                self.browser = Some(url);
            }
            Event::SignInResult { id, error } if self.signing_in.as_deref() == Some(&id) => {
                self.signing_in = None;
                self.browser = None;
                if let Some(error) = error {
                    self.setup.sign_in_error = Some((id.clone(), error.clone()));
                    self.open_hosted_setup();
                    if let Some(service) = self.setup.services.iter().flatten().find(|s| s.id == id)
                        && let Some(modal) = self.modal.take()
                    {
                        self.modal = Some(modal.with_filter(&service.name));
                    }
                    self.note(format!("Sign-in failed: {error}"));
                } else {
                    self.ops.push(Op::ListServices);
                    self.note(format!("Signed in to {id}. Choose a model to finish setup."));
                    self.execute(&format!("/setup models {id}"));
                }
            }
            Event::Services { services } => {
                self.key_sources.extend(
                    services
                        .iter()
                        .filter_map(|s| Some((s.id.clone(), s.key.clone()?))),
                );
                self.setup.services = Some(services);
                self.refresh_setup();
            }
            Event::ProviderAdded {
                id, path, note, ..
            } if self.rekeying => {
                self.rekeying = false;
                self.note(match (note, path) {
                    (Some(note), _) => format!("stored the new {id} key · {note}"),
                    (None, Some(path)) => format!(
                        "stored the new {id} key in the OS credential store; {path} now reads \
                         it from there"
                    ),
                    (None, None) => format!("replaced the {id} key"),
                });
            }
            Event::ProviderAdded { id, path, note, .. } if self.setup_adding => {
                // Its models follow; the user picks one.
                self.setup_adding = false;
                let mut said = match path {
                    Some(path) => format!("added provider {id} ({path}); pick a model"),
                    None => format!("using the existing provider {id}; pick a model"),
                };
                if let Some(note) = note {
                    said.push_str(&format!(" · {note}"));
                }
                self.note(said);
                self.prompt = None;
                self.modal = Some(modal::models(
                    None,
                    self.model_roles(),
                    self.info.ui.ascii,
                    self.write_scope,
                    &self.theme,
                ));
            }
            Event::ProviderAdded { id, path, .. } => self.note(match path {
                Some(path) => format!("added provider {id} ({path}); switching…"),
                None => format!("using the existing provider {id}; switching…"),
            }),
            Event::DoctorReport { checks } => {
                self.refresh_modal("Doctor", |app| modal::doctor(Some(&checks), &app.theme));
            }
            Event::ToolProbeResult { report } => {
                self.refresh_modal("Tool Probe", |app| {
                    modal::tool_probe(Some(&report), &app.theme)
                });
            }
            Event::MemoryNotes { notes, index_fill } => {
                self.refresh_modal("Memory", |app| {
                    modal::memory(Some(&notes), index_fill, &app.theme)
                });
            }
            Event::ContextReport { report } => {
                self.refresh_modal("Context", |app| {
                    modal::context(Some(&report), &app.growth, app.info.ui.ascii, &app.theme)
                });
            }
            Event::Stats { report } => {
                self.refresh_modal("Stats", |app| {
                    modal::stats(Some(&report), app.info.ui.ascii, &app.theme)
                });
            }
            Event::SessionList { sessions } => {
                self.refresh_modal("Sessions", |app| {
                    modal::sessions(Some(&sessions), &app.theme)
                });
            }
            Event::SettingSaved {
                key,
                value,
                path,
                note,
            } => self.on_setting_saved(&key, &value, &path, note),
            Event::HookSetup { request, info } => {
                if self.hook_inspect == Some(request) && self.modal.as_ref().is_some_and(|m| m.title == "Hook setup") {
                    self.hook_inspect = None;
                    self.modal = None;
                    let mut form = HookForm::new(*info);
                    if let Some(drafts) = self.hook_reapply.take() { form.restore_drafts(drafts); }
                    if self.hook_test_after_inspect {
                        if form.drafts.len() == 1 { form.key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL)); }
                        else { form.status = "Select a connection, then press Ctrl-T to check a review or issue URL.".into(); }
                    }
                    self.hook_form = Some(form);
                }
            }
            Event::HookSetupSaved { request, info, note } => {
                if self.hook_form.as_ref().is_some_and(|f| f.pending == Some(request)) {
                    let enabled = info.enabled;
                    let problem = info.problem.clone();
                    let mut form = HookForm::new(info);
                    form.status = if let Some(problem) = problem {
                        format!("Settings saved, but the hook needs attention: {problem}")
                    } else {
                        let state = if enabled { "Saved and applied. The next prompt uses these settings." }
                            else { "Saved. The hook remains disabled; Ctrl-E enables it." };
                        note.map_or_else(|| state.to_owned(), |note| format!("{state} {note}"))
                    };
                    self.hook_form = Some(form);
                }
            }
            Event::HookSetupPending { request } => {
                if let Some(form) = self.hook_form.as_mut().filter(|f| f.pending == Some(request)) {
                    form.status = "Saving… Applies after any running turn finishes.".into();
                }
            }
            Event::HookSetupFailed { request, message } => {
                if let Some(form) = self.hook_form.as_mut().filter(|f| f.pending == Some(request)) {
                    form.result("Could not save", message);
                } else if self.hook_inspect == Some(request) {
                    self.hook_inspect = None;
                    self.modal = Some(modal::hooks(
                        &self.info.hooks,
                        &self.hook_runs,
                        self.write_scope,
                        &self.theme,
                    ));
                    self.note(message);
                }
            }
            Event::HookCheckResult { request, status, message } => {
                if let Some(form) = self.hook_form.as_mut().filter(|f| f.pending == Some(request)) {
                    form.result(&status.replace('_', " "), message);
                }
            }
            Event::HookPayloadChoice { id, path } => {
                self.modal = Some(modal::hook_payload(&id, &path, &self.theme));
            }
            Event::HookEnabled { id, path, note } => {
                let mut text = format!("hook `{id}` enabled ({path})");
                if let Some(n) = note {
                    text.push_str(": ");
                    text.push_str(&n);
                }
                self.note(text);
            }
            Event::SessionResumed { session, items } => {
                if std::mem::take(&mut self.resuming) {
                    self.transcript.clear();
                    self.agents.clear();
                    self.context = None;
                    self.growth.clear();
                }
                self.startup_dismissed = true;
                self.starting_new = false;
                self.session_saved = true;
                let short: String = session.id.0.chars().take(12).collect();
                self.session_id = Some(session.id.0);
                self.note(format!("resumed session {short} ({} entries)", items.len()));
                for item in items {
                    self.transcript.push(match item {
                        ReplayItem::User { text } => Entry::User(text),
                        ReplayItem::Assistant { text } => Entry::Assistant(text),
                        ReplayItem::Tool {
                            summary,
                            ok,
                            result,
                            detail,
                        } => Entry::Tool(Box::new(ToolEntry {
                            call_id: CallId(String::new()),
                            name: summary.split_whitespace().next().unwrap_or_default().to_owned(),
                            summary,
                            ok: Some(ok),
                            result,
                            detail: detail.unwrap_or_default(),
                            started: None,
                            took: None,
                        })),
                        ReplayItem::Reasoning { text } => Entry::Reasoning(text),
                        ReplayItem::Compacted => {
                            Entry::System("── earlier conversation summarized ──".into())
                        }
                    });
                }
            }
            Event::TurnStarted { .. } => {
                if self.info.model.is_some() {
                    self.session_saved = true;
                }
                self.queued = false;
                self.turn_running = true;
                self.turn_started = Some(Instant::now());
                self.turn_tools = 0;
            }
            Event::AssistantDelta { text } => append_delta(
                &mut self.transcript,
                &text,
                |e| match e {
                    Entry::Assistant(buf) => Some(buf),
                    _ => None,
                },
                |t| Entry::Assistant(t.trim_start().to_owned()),
            ),
            Event::AssistantRewrite { text } => {
                let last = self
                    .transcript
                    .iter()
                    .rposition(|e| matches!(e, Entry::Assistant(_)));
                if let Some(i) = last {
                    self.transcript[i] = Entry::Assistant(text);
                }
            }
            Event::ReasoningDelta { text } => append_delta(
                &mut self.transcript,
                &text,
                |e| match e {
                    Entry::Reasoning(buf) => Some(buf),
                    _ => None,
                },
                |t| Entry::Reasoning(t.trim_start().to_owned()),
            ),
            Event::ToolCallBegin { call } => {
                self.turn_tools += 1;
                self.transcript.push(Entry::Tool(Box::new(ToolEntry {
                    call_id: call.call_id,
                    name: call.name,
                    summary: call.summary,
                    ok: None,
                    result: String::new(),
                    detail: String::new(),
                    started: Some(Instant::now()),
                    took: None,
                })));
            }
            Event::ToolCallOutputDelta { call_id, chunk } => {
                if let Some(t) = self.tool_mut(&call_id) {
                    t.detail.push_str(&chunk);
                }
            }
            Event::ToolCallEnd { call_id, result } => {
                let mut plan = None;
                if let Some(t) = self.tool_mut(&call_id) {
                    t.ok = Some(result.ok);
                    t.result = result.summary;
                    t.detail = result.detail.unwrap_or_default();
                    t.took = t.started.map(|s| s.elapsed());
                    if t.name == "todo" && result.ok {
                        plan = Some(parse_plan(&t.detail));
                    }
                }
                if let Some(plan) = plan {
                    self.plan = plan;
                }
            }
            Event::ApprovalRequest {
                call_id,
                request,
                always,
            } => {
                let project_ready = matches!(
                    self.info.project_status,
                    ProjectStatus::Trusted | ProjectStatus::SettingsOnly
                );
                self.approval = Some(Approval {
                    call_id,
                    request,
                    always,
                    always_scope: if project_ready {
                        WriteScope::Project
                    } else {
                        WriteScope::User
                    },
                });
            }
            Event::Plan { checklist } => self.plan = parse_plan(&checklist),
            Event::AgentStarted {
                id,
                agent,
                description,
                call_id,
                model,
            } => self.agents.push(AgentRun {
                id,
                agent,
                description,
                model,
                call_id,
                ok: None,
                summary: String::new(),
                started: Instant::now(),
                took: None,
                items: Vec::new(),
            }),
            Event::Agent { id, inner } => {
                if let Some(a) = self.agents.iter_mut().rev().find(|a| a.id == id) {
                    a.on_event(*inner);
                }
            }
            Event::AgentFinished { id, ok, summary } => {
                if let Some(a) = self.agents.iter_mut().rev().find(|a| a.id == id) {
                    a.ok = Some(ok);
                    a.summary = summary;
                    a.took = Some(a.started.elapsed());
                }
            }
            Event::Questions { call_id, questions } if !questions.is_empty() => {
                self.asking = Some(Asking::new(call_id, questions));
            }
            Event::StepLimitReached { turn, limit } => {
                self.note(format!(
                    "the turn reached its limit of {limit} model calls"
                ));
                self.step_limit = Some(StepLimit {
                    turn,
                    limit,
                    selected: 0,
                });
            }
            Event::ContextUpdate {
                used,
                budget,
                window,
                threshold,
                ..
            } => {
                let max = window.filter(|w| *w > 0).unwrap_or(budget);
                self.context = Some((used, max));
                self.compact_at = threshold;
            }
            Event::Speed { tokens_per_sec, .. } => {
                if tokens_per_sec.is_some() {
                    self.speed = tokens_per_sec;
                }
            }
            Event::ContextPruned { report } => self.note(format!(
                "pruned {} old tool result{} (~{} tokens)",
                report.items.len(),
                if report.items.len() == 1 { "" } else { "s" },
                report.removed_tokens
            )),
            Event::Compacting { reason } => {
                self.note(format!("summarizing earlier conversation ({reason})…"))
            }
            Event::Compacted {
                summary_tokens,
                replaced,
            } => self.note(format!(
                "── earlier conversation summarized: {replaced} messages → ~{summary_tokens} tokens ──"
            )),
            Event::TurnComplete { usage, .. } => {
                self.turn_running = false;
                if let Some((used, _)) = self.context {
                    self.growth.push(used);
                }
                self.approval = None;
                if let Some(started) = self.turn_started.take() {
                    let mut parts = vec![crate::theme::elapsed(started.elapsed())];
                    match self.turn_tools {
                        0 => {}
                        1 => parts.push("1 tool".into()),
                        n => parts.push(format!("{n} tools")),
                    }
                    if usage.completion_tokens > 0 {
                        parts.push(format!(
                            "{} tokens out",
                            crate::theme::tokens(usage.completion_tokens)
                        ));
                    }
                    if let Some(tps) = self.speed {
                        parts.push(format!("{tps:.0} t/s"));
                    }
                    self.transcript.push(Entry::TurnSummary(parts.join(" · ")));
                }
                self.asking = None;
                self.step_limit = None;
                if self.info.work_mode == WorkMode::Plan {
                    self.review = Some(0);
                }
            }
            Event::ModeChanged { work_mode, approvals } => {
                if work_mode == WorkMode::Plan
                    && self.info.work_mode != WorkMode::Plan
                    && !self.plan_note_shown
                {
                    self.plan_note_shown = true;
                    self.note("plan mode: read and search only; the model will propose changes and list tests for your review before running them · /edit leaves".into());
                }
                let mode = approvals.label();
                if mode == "yolo" && self.info.permission_mode != "yolo" {
                    self.note("yolo: approvals are automatic and the sandbox is off; work-mode restrictions and deny rules still apply · /yolo turns it off".into());
                } else if mode != "yolo" && self.info.permission_mode == "yolo" {
                    self.note(format!("yolo off · {mode} approvals"));
                }
                if work_mode != WorkMode::Plan {
                    self.review = None;
                }
                self.info.work_mode = work_mode;
                self.info.permission_mode = mode.to_owned();
            }
            Event::Sandbox { off } => {
                if off && !self.sandbox_off {
                    self.note(format!(
                        "sandbox off: shell commands run without the OS sandbox; {} mode still decides what asks · /sandbox on turns it back on",
                        self.info.permission_mode
                    ));
                } else if !off && self.sandbox_off {
                    self.note("sandbox on".into());
                }
                self.sandbox_off = off;
            }
            Event::TurnAborted { reason, .. } => {
                self.turn_running = false;
                self.turn_started = None;
                self.approval = None;
                self.asking = None;
                self.step_limit = None;
                self.note(reason);
            }
            Event::HookRan {
                id,
                on,
                outcome,
                ms,
                message,
                context_tokens,
            } => {
                self.hook_runs.push(HookRun {
                    id: id.clone(),
                    on: on.clone(),
                    outcome: outcome.label().to_owned(),
                    ms,
                    message: message.clone(),
                    context_tokens,
                });
                self.refresh_modal("Hooks", |app| {
                    modal::hooks(&app.info.hooks, &app.hook_runs, app.write_scope, &app.theme)
                        .keep_place(app.modal.as_ref())
                });
                // Quiet unless the hook changed something; failures arrive as warnings.
                let did = match outcome {
                    HookOutcome::Context => format!(
                        "added ~{} tokens of context",
                        crate::theme::tokens(context_tokens)
                    ),
                    HookOutcome::Denied => "denied".to_owned(),
                    HookOutcome::Asked => "asked for approval".to_owned(),
                    HookOutcome::Allowed => "allowed".to_owned(),
                    HookOutcome::Updated => "changed the arguments".to_owned(),
                    HookOutcome::Ok | HookOutcome::Failed if message.is_some() => "said".to_owned(),
                    HookOutcome::Ok | HookOutcome::Failed => {
                        if !self.modal.as_ref().is_some_and(|m| m.title == "Hooks") {
                            self.dirty = false;
                        }
                        return;
                    }
                };
                let said = message.map(|m| format!(": {m}")).unwrap_or_default();
                self.note(format!("hook {id} ({on}) {did}{said} · {ms} ms"));
            }
            Event::Diagnostics { errors, warnings } => self.diagnostics = (errors, warnings),
            Event::Notice { message } => self.transcript.push(Entry::System(message)),
            Event::UpdateAvailable { current, latest } => self.transcript.push(Entry::System(
                format!("quack {latest} is out (you have {current}) · `quack update` says how"),
            )),
            Event::Warning { message } => self.transcript.push(Entry::Warning(message)),
            Event::SessionResumeFailed { reason } => self.transcript.push(Entry::Warning(reason)),
            Event::Error { error } => {
                if std::mem::take(&mut self.setup_adding) {
                    self.note("run /setup to try again".into());
                }
                self.rekeying = false;
                self.transcript.push(Entry::Error(error.message));
            }
            _ => self.dirty = false,
        }
    }

    pub(super) fn on_setting_saved(
        &mut self,
        key: &str,
        value: &str,
        path: &str,
        note: Option<String>,
    ) {
        // Settings the UI owns apply now; the rest on the next start.
        let parsed_model = (value.trim() != "null")
            .then(|| value.trim().trim_matches('"').to_owned())
            .filter(|s| !s.is_empty());
        let live = match key {
            "ui.ascii" => {
                self.info.ui.ascii = value == "true";
                self.glyphs = Glyphs::new(self.info.ui.ascii);
                true
            }
            "ui.reduced_motion" => {
                self.info.ui.reduced_motion = value == "true";
                true
            }
            "ui.terminal_title" => {
                self.info.ui.terminal_title = value == "true";
                true
            }
            "ui.sidebar" => {
                self.info.ui.sidebar = value == "true";
                self.selection = None;
                true
            }
            "ui.theme" => {
                self.info.ui.theme = value.trim_matches('"').to_owned();
                self.theme = Theme::resolve(&self.info.ui.theme, self.info.no_color);
                true
            }
            "model.default" => {
                self.info.default_model = parsed_model;
                false
            }
            "model.small" => {
                self.info.small_model = parsed_model;
                true
            }
            _ => false,
        };
        let layer = if !self.info.project_config.is_empty() && path == self.info.project_config {
            if self.info.project_status == ProjectStatus::None && value.trim() != "null" {
                self.info.project_status = ProjectStatus::SettingsOnly;
            }
            "project"
        } else {
            "user"
        };
        if let Some(row) = self.info.settings.iter_mut().find(|r| r.key == key) {
            if value.trim() == "null" {
                row.value = "unset (/reload)".into();
                row.origin = format!("removed from {layer}: {path}");
            } else {
                row.value = value.to_owned();
                row.origin = format!("{layer}: {path}");
            }
        }
        let when = if live {
            ""
        } else {
            " (takes effect on the next start)"
        };
        let mut text = format!("saved {key} = {value} to {path}{when}");
        if let Some(n) = note {
            text.push_str("; ");
            text.push_str(&n);
        }
        self.note(text);
    }
}
