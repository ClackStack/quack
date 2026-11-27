//! Config hot-reload, trust decisions, hooks setup, and full reloads.

use super::*;

impl Engine {
    /// Apply scoped config-folder changes detected by `watch.rs`:
    /// - Pure-data extensions (`skills`, `commands`, `hooks`, `profiles`, `agents`, `themes`,
    ///   `grammars`) hot-reload automatically while preserving session approvals and servers.
    /// - Per-server `mcp.d` / `lsp.d` changes restart only the affected server(s).
    /// - Core `config.toml` / `providers.d` / `bundles` changes still prompt `/reload`.
    pub(super) async fn apply_watch(&mut self, ev: crate::watch::WatchEvent) {
        if ev.core_changed {
            self.emit(Event::Notice {
                message: "config files changed · /reload to apply them".into(),
            });
        }
        let mut reloaded: Vec<String> = Vec::new();
        if ev.data_kinds.contains(&"hooks") {
            self.load_hooks().await;
            reloaded.push("hooks".into());
        }
        let need_setup_refresh = ev.data_kinds.iter().any(|k| {
            matches!(
                *k,
                "skills" | "commands" | "profiles" | "agents" | "grammars"
            )
        });
        if need_setup_refresh && let Some(c) = &mut self.convo {
            let old = c.setup.clone();
            let cfg = self.extension_config.clone();
            let tx = self.tx.clone();
            if let Ok(fresh) =
                tokio::task::spawn_blocking(move || startup::refresh_extensions(&old, &cfg, &tx))
                    .await
            {
                let prompt_changed = fresh.system != c.setup.system
                    || fresh.effective_session_context() != c.setup.effective_session_context()
                    || fresh.surface.tools != c.setup.surface.tools;
                self.skills_changed |= fresh.vars.skills != c.setup.vars.skills;
                c.setup = Arc::new(fresh);
                if prompt_changed {
                    c.responses_session = Arc::default();
                }
                for &k in &ev.data_kinds {
                    if k != "hooks" && !reloaded.iter().any(|r| r == k) {
                        reloaded.push(k.to_owned());
                    }
                }
            }
        } else if ev.data_kinds.contains(&"themes") {
            reloaded.push("themes".into());
        }

        if !ev.lsp_changed.is_empty() {
            self.lsp.stop_servers(&ev.lsp_changed).await;
            reloaded.push(format!("lsp ({})", ev.lsp_changed.join(", ")));
        }

        if !ev.mcp_changed.is_empty() {
            let mut need_full_mcp = false;
            for id in &ev.mcp_changed {
                if self.mcp.manager.restart(id).await.is_err() {
                    need_full_mcp = true;
                    break;
                }
            }
            if need_full_mcp {
                let _ =
                    tokio::time::timeout(Duration::from_secs(3), self.mcp.manager.shutdown()).await;
                self.start_mcp().await;
                if let Some(c) = &mut self.convo {
                    let mut setup =
                        startup::refresh_extensions(&c.setup, &self.extension_config, &self.tx);
                    setup.mcp = self.mcp.clone();
                    c.setup = Arc::new(setup);
                }
            }
            reloaded.push(format!("mcp ({})", ev.mcp_changed.join(", ")));
        }

        if !reloaded.is_empty() {
            self.emit(Event::ConfigReloaded);
            self.emit(Event::Notice {
                message: format!("hot-reloaded {}", reloaded.join(", ")),
            });
        }
    }

    pub(super) async fn trust_project(
        &mut self,
        decision: String,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        let saved = quack_config::TrustDecision::parse(&decision)
            .ok_or_else(|| {
                format!("unknown decision `{decision}`; use trusted, settings-only or ignored")
            })
            .and_then(|d| self.config.trust_project(d));
        match saved {
            Ok(_) => {
                self.queued.push_back(Queued::Reload(None));
                self.next_queued(done, reload).await;
            }
            Err(message) => self.emit(Event::Error {
                error: EngineError {
                    kind: EngineErrorKind::Config,
                    message: format!("cannot save the trust decision: {message}"),
                },
            }),
        }
    }

    pub(super) async fn enable_hook(
        &mut self,
        id: String,
        replace_payload: Option<bool>,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        let config = self.config.clone();
        let requested = id.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            config
                .reload()
                .map_err(|e| e.to_string())?
                .enable_hook_with_payload(&requested, replace_payload)
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        match outcome {
            Ok(quack_config::HookEnableOutcome::Enabled(outcome)) => {
                self.queued.push_back(Queued::Reload(Some((
                    id,
                    outcome.path.display().to_string(),
                ))));
                self.next_queued(done, reload).await;
            }
            Ok(quack_config::HookEnableOutcome::ChoosePayload(path)) => {
                self.emit(Event::HookPayloadChoice {
                    id,
                    path: path.display().to_string(),
                })
            }
            Err(message) => self.emit(Event::Error {
                error: EngineError {
                    kind: EngineErrorKind::Config,
                    message,
                },
            }),
        }
    }

    pub(super) fn inspect_hook_setup(&mut self, id: String, request: u64) {
        let config = self.config.clone();
        let tx = self.tx.clone();
        self.hook_tasks.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let fresh = config.reload().map_err(|_| {
                    "Cannot reload the configuration; fix its errors first.".to_owned()
                })?;
                crate::hook_setup::inspect(&fresh, &id)
            })
            .await
            .unwrap_or_else(|_| Err("Cannot inspect the hook configuration.".into()));
            let event = match result {
                Ok(info) => Event::HookSetup {
                    request,
                    info: Box::new(info),
                },
                Err(message) => Event::HookSetupFailed { request, message },
            };
            let _ = tx.send(event);
        });
    }

    pub(super) async fn queue_hook_save(
        &mut self,
        op: Op,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        if let Op::SaveHookSetup { request, .. } = &op {
            self.emit(Event::HookSetupPending { request: *request });
        }
        self.queued.push_back(Queued::HookSave(Box::new(op)));
        self.next_queued(done, reload).await;
    }

    pub(super) fn check_hook_setup(
        &mut self,
        id: String,
        request: u64,
        revision: String,
        connection: quack_protocol::HookConnectionDraft,
        url: String,
    ) {
        let config = self.config.clone();
        let tx = self.tx.clone();
        let cancel = CancellationToken::new();
        self.hook_checks.insert(request, cancel.clone());
        self.hook_tasks.spawn(async move {
            let root = config.dirs.workspace_root.clone();
            let prepared = tokio::task::spawn_blocking(move || {
                crate::hook_setup::check_binding(&config, &id, &revision, &connection, &url)
                    .map(|(file, dir)| (file, dir, url))
            })
            .await;
            let result = match prepared {
                Ok(Ok((file, dir, url))) => {
                    crate::hooks::check(file, dir, root, url, cancel.clone()).await
                }
                Ok(Err(error)) => Err(error),
                Err(_) => Err("Cannot prepare the access check.".into()),
            };
            let (status, message) = result.unwrap_or_else(|message| ("invalid".into(), message));
            let _ = tx.send(Event::HookCheckResult {
                request,
                status,
                message,
            });
            cancel.cancel();
        });
    }

    pub(super) fn cancel_hook_check(&mut self, request: u64) {
        if let Some(cancel) = self.hook_checks.remove(&request) {
            cancel.cancel();
        }
    }

    pub(super) fn begin_hook_save(&mut self, op: Op, reload: mpsc::Sender<Reload>) {
        let Op::SaveHookSetup {
            id,
            request,
            revision,
            connections,
            enable,
            replace_payload,
        } = op
        else {
            return;
        };
        self.reloading = true;
        let config = self.config.clone();
        self.hook_tasks.spawn(async move {
            let name = id.clone();
            let result = tokio::task::spawn_blocking(move || {
                let note = crate::hook_setup::save(
                    &config,
                    &name,
                    &revision,
                    &connections,
                    enable,
                    replace_payload,
                )?;
                let fresh = config.reload().map_err(|_| {
                    "The hook was saved but could not be reloaded; run /reload.".to_owned()
                })?;
                Ok((Box::new(fresh), note))
            })
            .await
            .unwrap_or_else(|_| Err("Cannot save the hook configuration.".into()));
            let _ = reload
                .send(Reload::HookSaved {
                    request,
                    id,
                    result,
                })
                .await;
        });
    }

    pub(super) async fn apply_config_edit<T: Send + 'static>(
        &mut self,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
        op: impl FnOnce(&Loaded) -> Result<T, String> + Send + 'static,
        on_ok: impl FnOnce(&mut Self, T),
    ) {
        let config = self.config.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let reloaded = config.reload().map_err(|e| e.to_string())?;
            op(&reloaded)
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        match outcome {
            Ok(value) => {
                on_ok(self, value);
                self.queued.push_back(Queued::Reload(None));
                self.next_queued(done, reload).await;
            }
            Err(message) => self.emit(Event::Error {
                error: EngineError {
                    kind: EngineErrorKind::Config,
                    message,
                },
            }),
        }
    }

    pub(super) async fn apply_mcp_change(
        &mut self,
        id: String,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
        op: impl FnOnce(&Loaded) -> Result<String, String> + Send + 'static,
    ) {
        self.apply_config_edit(done, reload, op, move |engine, message| {
            engine.emit(Event::McpChanged { id, message });
        })
        .await;
    }

    /// Read the config files again off the async threads.
    pub(super) fn begin_reload(
        &mut self,
        reload: mpsc::Sender<Reload>,
        hook: Option<(String, String)>,
    ) {
        self.reloading = true;
        let config = self.config.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || config.reload())
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = reload.send(Reload::Read(result.map(Box::new), hook)).await;
        });
    }

    /// The new config is read: restart MCP servers on it and run startup again, keeping
    /// the model in use.
    pub(super) async fn on_reread(
        &mut self,
        result: Result<Box<Loaded>, String>,
        hook: Option<(String, String)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        let config = match result {
            Ok(c) => Arc::new(*c),
            Err(message) => {
                self.reloading = false;
                return self.emit(Event::Error {
                    error: EngineError {
                        kind: EngineErrorKind::Config,
                        message: format!("cannot reload the config: {message}"),
                    },
                });
            }
        };
        self.imports.on_reload();
        self.replace_config(config);
        let _ = tokio::time::timeout(
            Duration::from_secs(3),
            futures::future::join(self.mcp.manager.shutdown(), self.lsp.shutdown()),
        )
        .await;
        self.start_mcp().await;
        self.start_lsp().await;
        self.load_hooks().await;
        if let Some((id, path)) = hook {
            if self.hooks.contains(&id) {
                self.emit(Event::HookEnabled {
                    id,
                    path,
                    note: None,
                });
            } else {
                self.emit(Event::Error {
                    error: EngineError {
                        kind: EngineErrorKind::Config,
                        message: format!("hook `{id}` is still disabled or failed to load; check /hooks and the config layers"),
                    },
                });
            }
        }
        self.emit(Event::ConfigReloaded);
        // The session's own model, also while the planner is in use.
        let keep = self.convo.as_ref().map(|c| {
            let s = c.editor.as_ref().unwrap_or(&c.setup);
            (s.provider.id.clone(), s.model.id.clone())
        });
        let tx = self.tx.clone();
        let config = self.extension_config.clone();
        let mcp = self.mcp.clone();
        let lsp = self.lsp.clone();
        let reload = reload.clone();
        tokio::spawn(async move {
            let outcome = startup::run(config, tx, mcp, lsp, keep).await;
            let _ = reload.send(Reload::Ready(Box::new(outcome))).await;
        });
    }

    /// Startup after a reload finished: the session continues on the new setup, with its
    /// history and permission mode. Approvals given for the session are dropped, since the
    /// rules they were given under may have changed.
    pub(super) fn on_reloaded(&mut self, outcome: StartupOutcome) {
        let Some(setup) = outcome.setup else {
            self.problem = outcome.problem;
            if self.convo.is_some() {
                self.warn(
                    "the reloaded config has no usable model; keeping the previous one".into(),
                );
            }
            return;
        };
        self.problem = None;
        self.session.model = Some(setup.model_ref.clone());
        self.session.profile = setup.profile.id.clone();
        setup.provider.set_ledger(self.ledger.clone());
        if let Ok(mut list) = self.secret_allowlist.lock() {
            for fp in &self.config.settings.network.secret_allowlist {
                if !list.contains(fp) {
                    list.push(fp.clone());
                }
            }
        }
        self.configure_reasoning(&setup);
        let mut perms = setup.permissions.clone();
        let (history, loaded_skills) = match self.convo.take() {
            Some(c) => {
                self.saved_plan_start = c.plan_start;
                let old_setup = c.editor.as_ref().unwrap_or(&c.setup);
                if let Some(saved) = &self.saved_session_context
                    && setup.profile.id == old_setup.profile.id
                    && setup.tool_ctx.root == old_setup.tool_ctx.root
                {
                    setup.set_session_context(Some(saved.clone()));
                    self.skills_changed |= setup.vars.skills != old_setup.vars.skills;
                }
                if let Ok(old) = c.perms.lock() {
                    // A locked mode keeps the configured one from `setup`.
                    if !self.mode_locked() {
                        perms.mode = self
                            .config
                            .settings
                            .policy
                            .max_permission_mode
                            .map_or(old.mode, |cap| old.mode.min(cap));
                    }
                    if !self.config.settings.policy.locks("permissions.work_mode")
                        || old.work_mode == WorkMode::Plan
                    {
                        perms.work_mode = old.work_mode;
                    }
                    // A reload that locks `sandbox.mode` turns the sandbox back on.
                    perms.sandbox_off = old.sandbox_off && !self.sandbox_locked();
                    if old.sandbox_off && !perms.sandbox_off {
                        self.warn("`sandbox.mode` is now locked by the system policy; the sandbox is back on".into());
                        self.emit(Event::Sandbox { off: false });
                    }
                }
                (c.history.unwrap_or_default(), c.loaded_skills)
            }
            None => (Vec::new(), Arc::default()),
        };
        let plan_start = (perms.work_mode == WorkMode::Plan)
            .then_some(self.saved_plan_start)
            .flatten();
        self.convo = Some(Conversation {
            perms: Arc::new(Mutex::new(perms)),
            nested: nested(&self.config, &setup, &history),
            loaded_skills,
            setup: Arc::new(setup),
            history: Some(history),
            editor: None,
            plan_start,
            responses_session: Arc::default(),
            last_reminders: Arc::default(),
            egress_guard: Arc::default(),
        });
        session::record(
            self.log.as_ref(),
            "reload",
            serde_json::json!({ "model": self.session.model }),
        );
        self.emit(Event::SessionConfigured {
            session: Box::new(self.session.clone()),
        });
        self.emit_mode();
        if self
            .convo
            .as_ref()
            .is_some_and(|c| c.perms.lock().is_ok_and(|p| p.work_mode == WorkMode::Plan))
        {
            self.start_planner();
        }
    }

    pub(super) fn reset_secret_allowlist(&self) {
        if let Ok(mut list) = self.secret_allowlist.lock() {
            *list = self.config.settings.network.secret_allowlist.clone();
        }
    }

    /// Add the config's `[network] secret_allowlist` to the live list; a reload may
    /// have grown it, and duplicates are skipped.
    pub(super) fn merge_secret_allowlist(&self) {
        if let Ok(mut list) = self.secret_allowlist.lock() {
            for fp in &self.config.settings.network.secret_allowlist {
                if !list.contains(fp) {
                    list.push(fp.clone());
                }
            }
        }
    }
}

#[derive(Debug)]
pub(super) enum Reload {
    Read(Result<Box<Loaded>, String>, Option<(String, String)>),
    Ready(Box<StartupOutcome>),
    HookSaved {
        request: u64,
        id: String,
        result: Result<(Box<Loaded>, Option<String>), String>,
    },
}

pub(super) fn wire_ledger(
    ledger: &Arc<EgressLedger>,
    tx: &broadcast::Sender<Event>,
    log_slot: &Arc<Mutex<Option<SharedLog>>>,
) {
    let tx = tx.clone();
    let log_slot = Arc::clone(log_slot);
    ledger.set_hook(Arc::new(move |entry| {
        if let Ok(guard) = log_slot.lock()
            && let Some(log) = guard.as_ref()
        {
            session::record(Some(log), "egress", serde_json::json!(entry));
        }
        let _ = tx.send(Event::EgressRecorded {
            entry: entry.clone(),
        });
    }));
}
