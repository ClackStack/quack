//! Session persistence, replay, export, forking, worktrees, and undo/rewind.

use super::*;

impl Engine {
    /// Load the requested session, if any, and reopen its log for appending. Returns its
    /// history and plan checklist; `None` when there is nothing to load or it failed (with
    /// a warning).
    pub(super) fn open_saved(
        &mut self,
        resume: &Resume,
        setup: &Setup,
    ) -> Option<(Vec<Message>, String)> {
        let entry = match session::find(&self.config, resume) {
            Ok(Some(e)) => e,
            Ok(None) => return None,
            Err(e) => {
                self.resume_failed(format!("cannot resume: {e}"));
                return None;
            }
        };
        let ps = session::project_sessions(&self.config);
        let records = match ps.read(&entry) {
            Ok(r) => r,
            Err(e) => {
                self.resume_failed(format!("cannot read session {}: {e}", entry.id));
                return None;
            }
        };
        let session::Replayed {
            history,
            items,
            turns,
            plan,
            planner_start,
            session_context,
        } = session::replay(&records);
        self.saved_plan_start = planner_start;
        self.saved_session_context = session_context;
        setup.set_session_context(self.saved_session_context.clone());
        self.saved_mode = session::modes(&records);
        self.reasoning_efforts = session::reasoning_efforts(&records);
        self.stats = Arc::new(Mutex::new(crate::stats::Stats::from_log(&records)));
        let egress = session::egress_entries(&records);
        self.ledger.reset(&egress);
        self.reset_secret_allowlist();
        self.log = None;
        match ps.reopen(entry.clone()) {
            Ok(log) => {
                let log = Arc::new(Mutex::new(log));
                session::record(
                    Some(&log),
                    "resume",
                    serde_json::json!({ "model": setup.model_ref, "version": env!("CARGO_PKG_VERSION") }),
                );
                self.log = Some(log);
            }
            Err(e) => self.warn(format!(
                "session log is read-only ({e}); new turns will not be saved"
            )),
        }
        self.sync_ledger_log();
        self.session.id = SessionId(entry.id.clone());
        self.turns = turns;
        self.title_tasks = JoinSet::new();
        self.pending_title = None;
        self.set_title(Some(entry.title));
        self.configure_reasoning(setup);
        self.emit(Event::SessionResumed {
            session: Box::new(self.session.clone()),
            items: items.into(),
        });
        let (entries, stats) = self.ledger.snapshot();
        self.emit(Event::EgressLedger { entries, stats });
        Some((history, plan))
    }

    /// `/export`: the transcript so far, from the session log. Reading and rendering
    /// can be the whole session in text, so it stays off the async runtime.
    pub(super) async fn export(&mut self, format: ExportFormat, path: Option<&str>) {
        self.sync_log();
        let Some(entry) = self
            .log
            .as_ref()
            .and_then(|l| l.lock().ok().map(|l| l.entry.clone()))
        else {
            return self
                .warn("nothing to export yet: the session starts with its first message".into());
        };
        let ps = session::project_sessions(&self.config);
        let id = self.session.id.0.clone();
        let title = self.title.clone();
        let cwd = self.session.cwd.clone();
        let name = path
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_owned);
        let written = tokio::task::spawn_blocking(move || {
            let records = ps.read(&entry)?;
            let items = session::replay(&records).items;
            let started = quack_store::time::date(entry.started);
            let header = crate::export::Header {
                id: &id,
                title: title.as_deref(),
                model: &entry.model,
                started: &started,
            };
            let text = crate::export::render(format, &header, &items);
            let name = name.unwrap_or_else(|| crate::export::file_name(&header, format));
            let file = std::path::Path::new(&cwd)
                .join(quack_config::expand_home(&name, &|k| std::env::var_os(k)));
            std::fs::write(&file, text).map(|()| file)
        })
        .await
        .unwrap_or_else(|e| {
            tracing::error!("the export task failed: {e}");
            Err(std::io::Error::other("the export task failed"))
        });
        match written {
            Ok(file) => self.emit(Event::Notice {
                message: format!("exported the transcript to {}", file.display()),
            }),
            Err(e) => self.warn(format!("cannot read or write the transcript: {e}")),
        }
    }

    /// `/fork`: go on in a new session that starts as a copy of this one, with its undo
    /// points. The original is left as it is; `session_start` names it as `parent`.
    pub(super) async fn fork_session(&mut self) {
        self.sync_log();
        let Some((entry, dir)) = self
            .log
            .as_ref()
            .and_then(|l| l.lock().ok().map(|l| (l.entry.clone(), l.dir())))
        else {
            return self
                .warn("nothing to fork yet: the session starts with its first message".into());
        };
        let ps = session::project_sessions(&self.config);
        let id = new_session_id();
        let task_id = id.clone();
        let cwd = self.session.cwd.clone();
        let profile = self.session.profile.clone();
        let forked = tokio::task::spawn_blocking(move || {
            ps.read(&entry).and_then(|records| {
                let mut log = ps.create(&task_id.0, &entry.model)?;
                log.append(
                    "session_start",
                    serde_json::json!({
                        "version": env!("CARGO_PKG_VERSION"),
                        "os": std::env::consts::OS,
                        "cwd": cwd,
                        "model": entry.model,
                        "profile": profile,
                        "parent": entry.id,
                        "at": records.len(),
                    }),
                )?;
                for r in records
                    .iter()
                    .filter(|r| !matches!(r.kind.as_str(), "session_start" | "resume"))
                {
                    log.copy(r)?;
                }
                copy_dir(&dir.join("checkpoints"), &log.dir().join("checkpoints"))?;
                Ok(log)
            })
        })
        .await
        .unwrap_or_else(|e| {
            tracing::error!("the session fork task failed: {e}");
            Err(std::io::Error::other("the session fork task failed"))
        });
        let log = match forked {
            Ok(log) => log,
            Err(e) => return self.warn(format!("cannot fork this session: {e}")),
        };
        let parent = std::mem::replace(&mut self.session.id, id);
        if let Some(c) = &mut self.convo {
            c.responses_session = Arc::default();
        }
        self.log = Some(Arc::new(Mutex::new(log)));
        self.sync_ledger_log();
        let title = self.title.as_ref().map(|t| format!("{t} (fork)"));
        self.title_tasks = JoinSet::new();
        self.pending_title = None;
        self.set_title(title);
        self.sync_log();
        self.emit(Event::SessionConfigured {
            session: Box::new(self.session.clone()),
        });
        let short: String = parent.0.chars().take(8).collect();
        self.emit(Event::Notice {
            message: format!(
                "forked: this is a new session, and the original is unchanged (/resume {short})"
            ),
        });
    }

    /// Create a git worktree under `.quack/worktrees/<name>` on branch `quack/<name>`,
    /// copy the current session log (when present) into the worktree's project history,
    /// and reload the engine rooted in that worktree while keeping history and model.
    pub(super) async fn fork_worktree(
        &mut self,
        name: Option<String>,
        reload: mpsc::Sender<Reload>,
    ) {
        self.sync_log();
        let id = new_session_id();
        let short_id: String = id.0.chars().take(8).collect();
        let wt_name = match name {
            Some(n) if !n.trim().is_empty() => n.trim().to_owned(),
            _ => format!("wt-{short_id}"),
        };
        let repo_root = self.config.dirs.workspace_root.clone();
        let cfg = self.config.clone();
        let existing_log = self
            .log
            .as_ref()
            .and_then(|l| l.lock().ok().map(|l| (l.entry.clone(), l.dir())));
        let old_ps = session::project_sessions(&self.config);
        let profile = self.session.profile.clone();
        let task_id = id.clone();
        let task_name = wt_name.clone();

        let created = tokio::task::spawn_blocking(move || {
            let wt = quack_store::worktree::create_or_open(&repo_root, &task_name, false)?;
            let loaded = cfg
                .reload_in_cwd(wt.path.clone())
                .map_err(|e| format!("cannot load worktree config: {e}"))?;
            let new_ps = session::project_sessions(&loaded);
            let new_log = match existing_log {
                Some((entry, dir)) => {
                    let records = old_ps
                        .read(&entry)
                        .map_err(|e| format!("cannot read session log: {e}"))?;
                    let mut log = new_ps
                        .create(&task_id.0, &entry.model)
                        .map_err(|e| format!("cannot create worktree session: {e}"))?;
                    log.append(
                        "session_start",
                        serde_json::json!({
                            "version": env!("CARGO_PKG_VERSION"),
                            "os": std::env::consts::OS,
                            "cwd": wt.path.display().to_string(),
                            "model": entry.model,
                            "profile": profile,
                            "parent": entry.id,
                            "at": records.len(),
                            "worktree": wt.path.display().to_string(),
                            "branch": wt.branch,
                        }),
                    )
                    .map_err(|e| format!("cannot write worktree session_start: {e}"))?;
                    for r in records.iter().filter(|r| {
                        !matches!(
                            r.kind.as_str(),
                            "session_start" | "resume" | "session_context"
                        )
                    }) {
                        log.copy(r)
                            .map_err(|e| format!("cannot copy session record: {e}"))?;
                    }
                    let _ = copy_dir(&dir.join("checkpoints"), &log.dir().join("checkpoints"));
                    Some(log)
                }
                None => None,
            };
            Ok::<_, String>((wt, loaded, new_log))
        })
        .await
        .unwrap_or_else(|e| Err(format!("worktree fork task failed: {e}")));

        let (wt, loaded, new_log) = match created {
            Ok(v) => v,
            Err(e) => return self.warn(format!("cannot create worktree: {e}")),
        };

        let wt_path = wt.path.display().to_string();
        let wt_branch = wt.branch.clone();
        self.session.id = id.clone();
        self.session.cwd = wt_path.clone();
        self.saved_session_context = None;
        self.log = new_log.map(|l| Arc::new(Mutex::new(l)));
        self.sync_ledger_log();
        let title = self
            .title
            .as_ref()
            .map(|t| format!("{t} ({wt_name})"))
            .or_else(|| Some(format!("worktree {wt_name}")));
        self.title_tasks = JoinSet::new();
        self.pending_title = None;
        self.set_title(title);
        self.sync_log();

        self.emit(Event::WorktreeCreated {
            path: wt_path.clone(),
            branch: wt_branch.clone(),
            session_id: id,
        });
        self.emit(Event::Notice {
            message: format!(
                "forked into worktree {wt_path} on branch {wt_branch} (original repo unchanged)"
            ),
        });

        self.reloading = true;
        self.on_reread(Ok(Box::new(loaded)), None, &reload).await;
    }

    /// Replace the conversation with a saved session or an empty one, keeping the model.
    pub(super) fn switch_session(&mut self, target: Option<SessionId>) {
        // The new session starts outside plan mode, on the session's own model.
        self.restore_editor();
        let Some(setup) = self.convo.as_ref().map(|c| c.setup.clone()) else {
            return self.warn("no model is ready; sessions open once one is".into());
        };
        if target.as_ref() == Some(&self.session.id) {
            return self.warn("already in this session".into());
        }
        let source = if target.is_some() { "resume" } else { "new" };
        if let Some(end) = self.session_hooks(HookEvent::SessionEnd, "switch") {
            drop(end); // runs on; nothing waits for it
        }
        let (history, plan) = match target {
            Some(id) => {
                self.sync_log();
                match self.open_saved(&Resume::Id(id.0), &setup) {
                    Some(h) => h,
                    None => return,
                }
            }
            None => {
                self.sync_log();
                self.log = None;
                self.sync_ledger_log();
                self.ledger.reset(&[]);
                self.reset_secret_allowlist();
                self.reasoning_efforts.clear();
                self.saved_mode = None;
                self.saved_plan_start = None;
                self.saved_session_context = None;
                setup.set_session_context(None);
                self.session.id = new_session_id();
                self.stats = Arc::default();
                self.turns = 0;
                self.title_tasks = JoinSet::new();
                self.pending_title = None;
                self.set_title(None);
                self.emit(Event::EgressLedger {
                    entries: Vec::new(),
                    stats: Default::default(),
                });
                (Vec::new(), String::new())
            }
        };
        if let Some(c) = &mut self.convo {
            // Session approvals and the plan belong to the session they were made in.
            c.perms = Arc::new(Mutex::new(c.setup.permissions.clone()));
            c.nested = nested(&self.config, &c.setup, &history);
            c.history = Some(history);
            c.setup.tool_ctx.plan.restore(&plan);
            c.reset_transient();
        }
        self.pool = None;
        if let Some(setup) = self.convo.as_ref().map(|c| c.setup.clone()) {
            self.configure_reasoning(&setup);
        }
        self.restore_modes();
        self.emit(Event::Plan { checklist: plan });
        self.emit_mode();
        if self.in_plan_mode() {
            self.start_planner();
        }
        self.emit(Event::SessionConfigured {
            session: Box::new(self.session.clone()),
        });
        self.start_context = self.session_hooks(HookEvent::SessionStart, source);
    }

    pub(super) fn list_sessions(&self) {
        let now = quack_store::time::unix_now();
        let sessions = session::project_sessions(&self.config)
            .list()
            .into_iter()
            .map(|e| SessionSummary {
                current: e.id == self.session.id.0,
                id: SessionId(e.id),
                title: e.title,
                model: e.model,
                turns: e.turns,
                age: quack_store::time::age(e.updated, now),
                started: quack_store::time::date(e.started),
            })
            .collect();
        self.emit(Event::SessionList { sessions });
    }

    pub(super) fn checkpoints(&self) -> Option<Arc<Checkpoints>> {
        let dir = self.log.as_ref()?.lock().ok()?.dir().join("checkpoints");
        Some(Arc::new(Checkpoints::open(dir)))
    }

    pub(super) fn undo(&mut self, force: bool) {
        if self.running.is_some() {
            return self.warn("wait for the current turn to finish before /undo".into());
        }
        let Some(cp) = self.checkpoints() else {
            return self.warn("nothing to undo in this session".into());
        };
        let last = cp.last_call();
        if last.is_empty() {
            return self.warn("nothing to undo in this session yet".into());
        }
        let root = &self.config.dirs.workspace_root;
        let show = |p: &PathBuf| {
            p.strip_prefix(root)
                .unwrap_or(p)
                .display()
                .to_string()
                .replace('\\', "/")
        };
        match cp.restore(&last, force) {
            Ok(r) => {
                let restored: Vec<String> = r.restored.iter().map(show).collect();
                if !restored.is_empty() {
                    self.notes.push(format!(
                        "[The user undid your last change to {}; the file is back to how it was before that change.]",
                        restored.join(", ")
                    ));
                }
                self.emit(Event::FilesRestored {
                    restored,
                    conflicts: r.conflicts.iter().map(show).collect(),
                });
            }
            Err(e) => self.warn(format!("undo failed: {e}")),
        }
    }

    /// The user messages still in the history, most recent first, with the tool calls
    /// made after each.
    pub(super) fn rewind_points(&self) -> Vec<Point> {
        let Some(history) = self.convo.as_ref().and_then(|c| c.history.as_deref()) else {
            return Vec::new();
        };
        let changes = self
            .checkpoints()
            .map(|cp| cp.changes())
            .unwrap_or_default();
        let root = &self.config.dirs.workspace_root;
        let mut points = Vec::new();
        let mut calls: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (index, m) in history.iter().enumerate().rev() {
            match m {
                Message::Assistant { tool_calls, .. } => {
                    calls.extend(tool_calls.iter().map(|c| c.id.clone()));
                }
                Message::User { content, .. }
                    if !content.starts_with(quack_context::compact::SUMMARY_MARKER) =>
                {
                    let mut files: Vec<String> = Vec::new();
                    for c in changes
                        .iter()
                        .filter(|c| calls.contains(c.call_id.as_str()))
                    {
                        let shown = c
                            .path
                            .strip_prefix(root)
                            .unwrap_or(&c.path)
                            .display()
                            .to_string()
                            .replace('\\', "/");
                        if !files.contains(&shown) {
                            files.push(shown);
                        }
                    }
                    points.push(Point {
                        point: quack_protocol::RewindPoint {
                            back: u32::try_from(points.len() + 1).unwrap_or(u32::MAX),
                            text: content.clone(),
                            files,
                        },
                        index,
                        calls: calls.clone(),
                    });
                }
                _ => {}
            }
        }
        points
    }

    /// `/rewind`: put files back and/or cut the conversation back to before a user message.
    /// When files changed again since, nothing happens without `force`.
    pub(super) fn rewind(&mut self, back: u32, opts: RewindOpts) {
        let RewindOpts {
            conversation,
            files,
            force,
        } = opts;
        if self.running.is_some() {
            return self.warn("wait for the current turn to finish before /rewind".into());
        }
        let Some(point) = self
            .rewind_points()
            .into_iter()
            .find(|p| p.point.back == back)
        else {
            return self.warn(format!("no user message {back} back to rewind to"));
        };
        let root = self.config.dirs.workspace_root.clone();
        let show = |p: &PathBuf| {
            p.strip_prefix(&root)
                .unwrap_or(p)
                .display()
                .to_string()
                .replace('\\', "/")
        };
        let mut restored = Vec::new();
        if files && let Some(cp) = self.checkpoints() {
            let changes: Vec<_> = cp
                .changes()
                .into_iter()
                .filter(|c| point.calls.contains(c.call_id.as_str()))
                .collect();
            let conflicts = cp.conflicts(&changes);
            if !force && !conflicts.is_empty() {
                self.emit(Event::FilesRestored {
                    restored: Vec::new(),
                    conflicts: conflicts.iter().map(show).collect(),
                });
                return self.warn(format!(
                    "nothing rewound: files changed since; /rewind {back} --force overwrites them"
                ));
            }
            match cp.restore(&changes, force) {
                Ok(r) => {
                    restored = r.restored.iter().map(show).collect();
                    self.emit(Event::FilesRestored {
                        restored: restored.clone(),
                        conflicts: r.conflicts.iter().map(show).collect(),
                    });
                }
                Err(e) => return self.warn(format!("rewind failed: {e}")),
            }
        }
        if !conversation {
            if !restored.is_empty() {
                self.notes.push(format!(
                    "[The user put {} back to how they were before their message “{}”.]",
                    restored.join(", "),
                    point.point.text.lines().next().unwrap_or_default()
                ));
            }
            return;
        }
        let Some(c) = &mut self.convo else { return };
        c.reset_transient();
        let Some(history) = &mut c.history else {
            return;
        };
        let dropped = history[point.index..]
            .iter()
            .filter(|m| matches!(m, Message::User { .. }))
            .count();
        history.truncate(point.index);
        c.nested = nested(&self.config, &c.setup, history);
        self.turns = self
            .turns
            .saturating_sub(u32::try_from(dropped).unwrap_or(u32::MAX));
        session::record(
            self.log.as_ref(),
            "rewind",
            serde_json::json!({ "to": point.index }),
        );
        self.sync_log();
        self.emit(Event::Rewound {
            back,
            text: point.point.text,
        });
    }

    /// Create the session log on the first turn.
    pub(super) async fn ensure_log(&mut self) {
        if self.log.is_some() {
            return;
        }
        let ps = session::project_sessions(&self.config);
        let model = self.session.model.clone().unwrap_or_default();
        let id = self.session.id.0.clone();
        let task_model = model.clone();
        let created = tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&ps.dir).and_then(|()| ps.create(&id, &task_model))
        })
        .await
        .unwrap_or_else(|e| {
            tracing::error!("the session log creation task failed: {e}");
            Err(std::io::Error::other(
                "the session log creation task failed",
            ))
        });
        match created {
            Ok(log) => {
                let log = Arc::new(Mutex::new(log));
                session::record(
                    Some(&log),
                    "session_start",
                    serde_json::json!({
                        "version": env!("CARGO_PKG_VERSION"),
                        "os": std::env::consts::OS,
                        "cwd": self.session.cwd,
                        "model": model,
                        "profile": self.session.profile,
                    }),
                );
                self.log = Some(log);
                if let Some(ctx) = self
                    .convo
                    .as_ref()
                    .map(|c| c.setup.effective_session_context())
                    .filter(|c| !c.is_empty())
                {
                    session::record(
                        self.log.as_ref(),
                        "session_context",
                        serde_json::json!({ "text": ctx }),
                    );
                    self.saved_session_context = Some(ctx.clone());
                    if let Some(c) = &self.convo {
                        c.setup.set_session_context(Some(ctx));
                    }
                }
                self.sync_ledger_log();
                let (pre_entries, _) = self.ledger.snapshot();
                for entry in &pre_entries {
                    session::record(self.log.as_ref(), "egress", serde_json::json!(entry));
                }
                self.emit_mode();
                for (model, effort) in &self.reasoning_efforts {
                    session::record(
                        self.log.as_ref(),
                        "reasoning_effort",
                        serde_json::json!({ "model": model, "effort": effort }),
                    );
                }
            }
            Err(e) => self.warn(format!(
                "cannot save this session ({e}); continuing without a log"
            )),
        }
    }

    pub(super) fn sync_log(&mut self) {
        if let Some(log) = &self.log
            && let Ok(mut l) = log.lock()
            && let Err(e) = l.sync(self.title.as_deref(), self.turns)
        {
            tracing::warn!(error = %e, "could not sync session log");
        }
    }

    pub(super) fn set_session_title(&mut self, title: String) {
        self.title_tasks = JoinSet::new();
        self.pending_title = None;
        self.save_title(title.trim().to_owned());
    }

    pub(super) fn save_title(&mut self, title: String) {
        session::record(
            self.log.as_ref(),
            "title",
            serde_json::json!({ "title": title }),
        );
        self.set_title(Some(title));
        self.sync_log();
    }

    pub(super) fn title_targets_local(&self, current: &Setup) -> bool {
        let Some((provider, _)) = self.config.settings.model.small.trim().split_once('/') else {
            return false;
        };
        if current.provider.id == provider {
            return quack_config::is_loopback(current.provider.base_url());
        }
        quack_config::load_providers(&self.config)
            .into_iter()
            .find(|item| item.id == provider)
            .and_then(|item| match item.state {
                quack_config::ItemState::Loaded(file) => {
                    Some(quack_config::is_loopback(&file.base_url))
                }
                _ => None,
            })
            .unwrap_or(false)
    }

    pub(super) fn generate_title(&mut self, current: &Arc<Setup>, prompt: &str) {
        let Some((provider, model)) = self.config.settings.model.small.trim().split_once('/')
        else {
            return;
        };
        let job = title::Job {
            config: self.config.clone(),
            mcp: self.mcp.clone(),
            lsp: self.lsp.clone(),
            ledger: self.ledger.clone(),
            current: current.clone(),
            provider: provider.to_owned(),
            model: model.to_owned(),
            prompt: prompt
                .trim()
                .chars()
                .take(title::MAX_PROMPT_CHARS)
                .collect(),
        };
        let session = self.session.id.clone();
        self.title_tasks
            .spawn(async move { (session, title::generate(job).await) });
    }

    pub(super) fn sync_ledger_log(&self) {
        if let Ok(mut guard) = self.ledger_log.lock() {
            *guard = self.log.clone();
        }
    }
}

/// A rewind point and where it is in the history.
#[derive(Debug)]
pub(super) struct Point {
    pub(super) point: quack_protocol::RewindPoint,
    /// Index of the user message in the history.
    index: usize,
    /// Tool calls made after it.
    calls: std::collections::HashSet<String>,
}

/// Copy a folder's files, recursively; a missing folder copies nothing.
pub(super) fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(from) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    std::fs::create_dir_all(to)?;
    for e in entries {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            std::fs::copy(e.path(), target)?;
        }
    }
    Ok(())
}

pub(super) fn new_session_id() -> SessionId {
    SessionId(quack_store::session::new_session_id())
}
