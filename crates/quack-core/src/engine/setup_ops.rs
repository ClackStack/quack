//! Providers, models, profiles, memory, MCP management, and diagnostics.

use super::*;

impl Engine {
    pub(super) fn save_memory(&self, note: SaveMemoryArgs) {
        let store = crate::env::memory_store(&self.config);
        let result = (|| -> Result<(), String> {
            let target = Scope::parse(&note.scope)
                .ok_or_else(|| format!("unknown scope `{}`", note.scope))?;
            let prev = match (&note.previous_scope, &note.previous_name) {
                (Some(ps), Some(pn)) => Some((
                    Scope::parse(ps).ok_or_else(|| format!("unknown scope `{ps}`"))?,
                    pn.as_str(),
                )),
                (None, None) => None,
                _ => {
                    return Err(
                        "previous_scope and previous_name must both be set when moving a note"
                            .into(),
                    );
                }
            };
            if let Some((ps, pn)) = prev
                && (ps, pn) != (target, note.name.as_str())
                && store.exists(target, &note.name)
            {
                return Err(format!(
                    "a {} note named `{}` already exists",
                    target.label(),
                    note.name
                ));
            }
            store.write(
                target,
                &note.name,
                &note.description,
                &note.kind,
                &note.body,
            )?;
            if let Some((ps, pn)) = prev
                && (ps, pn) != (target, note.name.as_str())
            {
                let _ = store.delete(ps, pn);
            }
            Ok(())
        })();
        match result {
            Ok(()) => self.list_memory(),
            Err(e) => self.warn(format!("cannot save note: {e}")),
        }
    }

    pub(super) fn list_memory(&self) {
        let store = crate::env::memory_store(&self.config);
        let mut notes = Vec::new();
        for scope in [Scope::Project, Scope::Global] {
            for (name, description) in store.list(scope) {
                notes.push(MemoryNote {
                    scope: scope.label().into(),
                    path: store
                        .dir(scope)
                        .join("notes")
                        .join(format!("{name}.md"))
                        .display()
                        .to_string(),
                    text: store.read(scope, &name).unwrap_or_default(),
                    name,
                    description,
                });
            }
        }
        self.emit(Event::MemoryNotes {
            notes,
            index_fill: u32::try_from(store.index_fill()).unwrap_or(u32::MAX),
        });
    }

    pub(super) async fn save_setting(&mut self, key: &str, value: &str, scope: &WriteScope) {
        let layer = match scope {
            WriteScope::User => LayerKind::User,
            WriteScope::Project => LayerKind::Project,
            WriteScope::Session => {
                return self.warn("settings are saved to the user or project layer".into());
            }
        };
        let config = Arc::clone(&self.config);
        let key_owned = key.to_owned();
        let value_owned = value.to_owned();
        let outcome = tokio::task::spawn_blocking(move || {
            let fresh = config.reload().map_err(|e| e.to_string())?;
            let (path, note) = fresh.save_setting(layer, &key_owned, &value_owned)?;
            let updated = fresh.reload().map_err(|e| e.to_string())?;
            Ok::<_, String>((updated, path, note))
        })
        .await
        .map_err(|e| format!("failed to save setting: {e}"))
        .and_then(|r| r);
        match outcome {
            Ok((updated, path, note)) => {
                self.config = Arc::new(updated);
                self.config_tx.send_replace(self.config.clone());
                self.emit(Event::SettingSaved {
                    key: key.to_owned(),
                    value: value.to_owned(),
                    path: path.display().to_string(),
                    note,
                });
            }
            Err(message) => self.emit(Event::Error {
                error: EngineError {
                    kind: EngineErrorKind::Config,
                    message: format!("cannot save {key}: {message}"),
                },
            }),
        }
    }

    /// `/setup`: write the provider file, then switch to the model like `/models` does.
    pub(super) fn add_provider(
        &mut self,
        id: &str,
        base_url: &str,
        api: Option<&str>,
        model: String,
    ) {
        if self.running.is_some() {
            return self
                .warn("wait for the current turn to finish before adding a provider".into());
        }
        let added =
            match crate::setup::add_provider(&self.config, id, base_url, api, "Added by setup.") {
                Ok(a) => a,
                Err(message) => {
                    return self.emit(Event::Error {
                        error: EngineError {
                            kind: EngineErrorKind::Config,
                            message: format!("cannot add the provider: {message}"),
                        },
                    });
                }
            };
        self.emit(Event::ProviderAdded {
            id: added.id.clone(),
            path: added.path.map(|p| p.display().to_string()),
            note: added.note,
        });
        let reference = format!("{}/{model}", added.id);
        match crate::setup::default_model_if_unset(&self.config, &reference) {
            Ok(Some(path)) => self.emit(Event::SettingSaved {
                key: "model.default".into(),
                value: serde_json::Value::from(reference).to_string(),
                path: path.display().to_string(),
                note: None,
            }),
            Ok(None) => {}
            Err(e) => self.warn(format!("cannot save the default model: {e}")),
        }
        self.switch_to(Role::Session, added.id, model);
    }

    /// `/setup`'s key and URL steps: write the provider in the background, then list its
    /// models alone so the user can pick one. The `live` providers (in use, with the same
    /// id) take the provider's new key.
    pub(super) fn connect(
        &self,
        live: Vec<Provider>,
        add: impl FnOnce(&Loaded) -> Result<crate::setup::Added, String> + Send + 'static,
    ) {
        let config = self.config.clone();
        let tx = self.tx.clone();
        let current = self.session.model.clone();
        tokio::spawn(async move {
            let cfg = config.clone();
            let added = tokio::task::spawn_blocking(move || add(&cfg))
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            let added = match added {
                Ok(a) => a,
                Err(message) => {
                    let _ = tx.send(Event::Error {
                        error: EngineError {
                            kind: EngineErrorKind::Config,
                            message: format!("cannot add the provider: {message}"),
                        },
                    });
                    return;
                }
            };
            if !live.is_empty() {
                let (cfg, id) = (config.clone(), added.id.clone());
                let fresh = tokio::task::spawn_blocking(move || fresh_provider(&cfg, &id))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string()));
                match fresh {
                    Ok(fresh) => live.iter().for_each(|p| p.take_key(&fresh)),
                    Err(e) => {
                        let _ = tx.send(Event::Warning {
                            message: format!(
                                "the key is stored, but this session cannot read it ({e}) · \
                                 restart quack to use it"
                            ),
                        });
                    }
                }
            }
            let _ = tx.send(Event::ProviderAdded {
                id: added.id.clone(),
                path: added.path.map(|p| p.display().to_string()),
                note: added.note,
            });
            let providers = crate::models::catalog_of(config, Some(&added.id)).await;
            let _ = tx.send(Event::ModelCatalog { providers, current });
        });
    }

    /// Build the setup for `provider/model` in the background; `on_switched` (or
    /// `on_planner`) gets it.
    pub(super) fn switch_to(&self, role: Role, provider: String, model: String) {
        if let Some(done) = self.switch_tx.clone() {
            let config = self.config.clone();
            let tx = self.tx.clone();
            let mcp = self.mcp.clone();
            let lsp = self.lsp.clone();
            tokio::spawn(async move {
                let r = startup::switch(config, tx, mcp, lsp, provider, model).await;
                let _ = done.send((role, r)).await;
            });
        }
    }

    /// `/instructions`: the files in the system prompt, then the ones attached to tool
    /// results (seen in the history when no turn holds it).
    pub(super) fn list_instructions(&self) {
        let file = |(source, text): (String, String), in_prompt| InstructionFile {
            tokens: quack_providers::tokens::estimate(&text),
            source,
            in_prompt,
            text,
        };
        let mut files: Vec<InstructionFile> = Vec::new();
        if let Some(c) = &self.convo {
            files.extend(
                quack_instructions::agents_md::blocks(&c.setup.vars.instructions)
                    .into_iter()
                    .map(|b| file(b, true)),
            );
            for m in c.history.iter().flatten() {
                if let Message::Tool { content, .. } = m {
                    for b in quack_instructions::agents_md::blocks(content) {
                        if !files.iter().any(|f| f.source == b.0) {
                            files.push(file(b, false));
                        }
                    }
                }
            }
        }
        self.emit(Event::Instructions { files });
    }

    /// A warning, for an `Op` handler that then carries on with the next op.
    pub(super) fn warn_continue(&self, message: &str) -> bool {
        self.warn(message.to_owned());
        true
    }

    /// `/profiles <id>`: rebuild the session's setup with that profile for the same model.
    /// It goes into the command-line overrides, so a reload keeps it.
    pub(super) fn set_profile(&mut self, id: String) {
        if self.running.is_some() {
            return self
                .warn("wait for the current turn to finish before switching profiles".into());
        }
        let Some((provider, model)) = self
            .convo
            .as_ref()
            .map(|c| (c.setup.provider.id.clone(), c.setup.model.id.clone()))
        else {
            return self.warn("no model is ready yet".into());
        };
        let value = if id == "auto" { String::new() } else { id };
        let mut config = (*self.config).clone();
        config.settings.model.profile.clone_from(&value);
        config
            .opts
            .overrides
            .retain(|o| !o.starts_with("model.profile="));
        config.opts.overrides.push(format!(
            "model.profile={}",
            serde_json::Value::String(value)
        ));
        let config = Arc::new(config);
        self.config = config.clone();
        self.config_tx.send_replace(config);
        self.switch_to(Role::Session, provider, model);
    }

    /// Apply this session's selection without changing the prompt or tool surface. A
    /// model this session has not set falls back to the level remembered from earlier
    /// runs; anything the model no longer declares is ignored.
    pub(super) fn configure_reasoning(&mut self, setup: &Setup) {
        let key = setup.reasoning_key();
        let levels = &setup.model.reasoning.efforts;
        let remembered = quack_config::reasoning_efforts(&self.config.dirs.user_data);
        let selected = self
            .reasoning_efforts
            .get(&key)
            .or_else(|| remembered.get(&key))
            .filter(|e| levels.contains(e))
            .cloned();
        if let Ok(mut effort) = setup.reasoning_effort.write() {
            *effort = selected;
        }
        self.session.reasoning_effort = Some(setup.reasoning_effort()).filter(|e| !e.is_empty());
        self.session.reasoning_efforts = setup.model.reasoning.efforts.clone();
    }

    pub(super) fn set_reasoning_effort(&mut self, effort: Option<String>) {
        let Some(setup) = self.convo.as_ref().map(|c| c.setup.clone()) else {
            return self.warn("No model is ready yet; select one with /models first.".into());
        };
        let levels = &setup.model.reasoning.efforts;
        if levels.is_empty() {
            return self.warn(format!(
                "{} has no selectable thinking levels; run `quack catalog refresh` for the \
                 catalog's, or declare reasoning.efforts in its provider file.",
                setup.model_ref
            ));
        }
        if let Some(selected) = &effort
            && !levels.contains(selected)
        {
            return self.warn(format!(
                "Thinking level `{selected}` is unavailable for {}; use {} or default.",
                setup.model_ref,
                levels.join(", ")
            ));
        }
        let model = setup.reasoning_key();
        let mut remembered = quack_config::reasoning_efforts(&self.config.dirs.user_data);
        match &effort {
            Some(selected) => {
                self.reasoning_efforts
                    .insert(model.clone(), selected.clone());
                remembered.insert(model.clone(), selected.clone());
            }
            None => {
                self.reasoning_efforts.remove(&model);
                remembered.remove(&model);
            }
        }
        if let Err(e) =
            quack_config::save_reasoning_efforts(&self.config.dirs.user_data, &remembered)
        {
            self.warn(format!(
                "cannot remember the thinking level for next time: {e}"
            ));
        }
        self.configure_reasoning(&setup);
        session::record(
            self.log.as_ref(),
            "reasoning_effort",
            serde_json::json!({ "model": model, "effort": effort }),
        );
        self.pool = None;
        self.emit(Event::SessionConfigured {
            session: Box::new(self.session.clone()),
        });
        self.emit(Event::Notice {
            message: format!(
                "Thinking for {}: {}.",
                setup.model_ref,
                self.session
                    .reasoning_effort
                    .as_deref()
                    .unwrap_or("model default")
            ),
        });
    }

    pub(super) fn on_switched(&mut self, result: Result<Setup, String>) {
        let mut setup = match result {
            Ok(s) => s,
            Err(e) => {
                self.emit(Event::Error {
                    error: EngineError {
                        kind: EngineErrorKind::Provider,
                        message: format!("cannot switch model: {e}"),
                    },
                });
                return;
            }
        };
        self.session.model = Some(setup.model_ref.clone());
        self.session.profile = setup.profile.id.clone();
        setup.provider.set_ledger(self.ledger.clone());
        self.merge_secret_allowlist();
        self.configure_reasoning(&setup);
        if self.config.settings.model.remember_last
            && let Err(e) =
                quack_config::save_last_model(&self.config.dirs.user_data, &setup.model_ref)
        {
            self.warn(format!("cannot remember the model for next time: {e}"));
        }
        session::record(
            self.log.as_ref(),
            "model_switch",
            serde_json::json!({ "to": setup.model_ref, "profile": setup.profile.id }),
        );
        match &mut self.convo {
            // Keep the history, the plan and the session's approvals.
            Some(c) => {
                setup.tool_ctx.plan = c.setup.tool_ctx.plan.clone();
                if let Some(saved) = &self.saved_session_context
                    && setup.profile.id == c.setup.profile.id
                    && setup.tool_ctx.root == c.setup.tool_ctx.root
                {
                    setup.set_session_context(Some(saved.clone()));
                }
                c.setup = Arc::new(setup);
                c.reset_transient();
            }
            None => {
                let resume = std::mem::take(&mut self.resume);
                let (history, plan) = self.open_saved(&resume, &setup).unwrap_or_default();
                self.configure_reasoning(&setup);
                if !plan.is_empty() {
                    setup.tool_ctx.plan.restore(&plan);
                    self.emit(Event::Plan { checklist: plan });
                }
                self.convo = Some(Conversation {
                    perms: Arc::new(Mutex::new(setup.permissions.clone())),
                    nested: nested(&self.config, &setup, &history),
                    setup: Arc::new(setup),
                    history: Some(history),
                    editor: None,
                    plan_start: None,
                    responses_session: Arc::default(),
                    last_reminders: Arc::default(),
                    egress_guard: Arc::default(),
                });
                self.restore_modes();
                self.problem = None;
            }
        }
        self.emit(Event::SessionConfigured {
            session: Box::new(self.session.clone()),
        });
    }

    pub(super) fn list_models(&self) {
        let config = self.config.clone();
        let tx = self.tx.clone();
        let current = self.session.model.clone();
        tokio::spawn(async move {
            let providers = crate::models::catalog(config).await;
            let _ = tx.send(Event::ModelCatalog { providers, current });
        });
    }

    pub(super) fn list_profiles(&self) -> bool {
        let Some(setup) = self.convo.as_ref().map(|c| c.setup.clone()) else {
            return self.warn_continue("no model is ready yet; profiles are shown for one");
        };
        let config = self.config.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let event =
                tokio::task::spawn_blocking(move || startup::profiles(&config, &setup)).await;
            if let Ok(event) = event {
                let _ = tx.send(event);
            }
        });
        true
    }

    pub(super) fn set_model(
        &self,
        provider: quack_protocol::ProviderId,
        model: quack_protocol::ModelId,
    ) {
        if self.running.is_some() {
            self.warn("wait for the current turn to finish before switching models".into());
        } else {
            self.switch_to(Role::Session, provider.0, model.0);
        }
    }

    pub(super) fn run_doctor(&self) {
        let config = self.config.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let checks = crate::doctor::run(config).await;
            let _ = tx.send(Event::DoctorReport { checks });
        });
    }

    pub(super) async fn init_instructions(
        &mut self,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) -> bool {
        let prompt = init_prompt(&self.config);
        Box::pin(self.handle(
            Op::UserTurn {
                items: vec![InputItem::text(prompt)],
                overrides: Default::default(),
            },
            done,
            reload,
        ))
        .await
    }

    pub(super) fn restart_mcp(&self, id: String) {
        let mcp = self.mcp.clone();
        let tx = self.tx.clone();
        let budget = self.convo.as_ref().and_then(|c| c.setup.budget);
        tokio::spawn(async move {
            if let Err(message) = mcp.manager.restart(&id).await {
                let _ = tx.send(Event::Warning { message });
            }
            let _ = tx.send(Event::McpServers {
                servers: mcp.info(budget),
            });
        });
    }

    pub(super) async fn enable_mcp(
        &mut self,
        id: String,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        let target = id.clone();
        self.apply_config_edit(
            done,
            reload,
            move |cfg| {
                crate::mcp::enable_mcp_in_config_detail(cfg, &target, &|v| {
                    std::env::var_os(v).is_some()
                })
            },
            move |engine, (path, message)| {
                engine.emit(Event::McpEnabled {
                    id: id.clone(),
                    path: path.display().to_string(),
                });
                engine.emit(Event::McpChanged { id, message });
            },
        )
        .await;
    }

    pub(super) async fn disable_mcp(
        &mut self,
        id: String,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        let target = id.clone();
        self.apply_mcp_change(id, done, reload, move |cfg| {
            crate::mcp::disable_mcp_in_config(cfg, &target)
        })
        .await;
    }

    pub(super) async fn add_mcp(
        &mut self,
        fields: AddMcpFields,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        let AddMcpFields {
            id,
            url,
            command,
            args,
            bearer_env,
            bearer_token,
            scope,
        } = fields;
        let project = match scope {
            None | Some(WriteScope::User) => Some(false),
            Some(WriteScope::Project) => Some(true),
            Some(WriteScope::Session) => None,
        };
        if let Some(project) = project {
            let target = id.clone();
            self.apply_mcp_change(id, done, reload, move |cfg| {
                crate::mcp::add_mcp_in_config(
                    cfg,
                    crate::mcp::AddMcpInput {
                        id: target,
                        url,
                        command,
                        args,
                        bearer_env,
                        bearer_token: bearer_token.map(|s| s.0),
                        project,
                    },
                )
            })
            .await;
        } else {
            self.warn("MCP servers are saved to the user or project layer".into());
        }
    }

    pub(super) async fn save_mcp_server(
        &mut self,
        scope: WriteScope,
        fields: SavedMcpFields,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        let layer = match scope {
            WriteScope::User => Some(LayerKind::User),
            WriteScope::Project => Some(LayerKind::Project),
            WriteScope::Session => {
                self.warn("MCP servers are saved to the user or project layer".into());
                None
            }
        };
        if let Some(layer) = layer {
            let saved_id = fields.id.clone();
            let spec = build_saved_mcp_server(fields);
            match spec {
                Ok(spec) => {
                    self.apply_config_edit(
                        done,
                        reload,
                        move |cfg| cfg.save_mcp_server(layer, spec),
                        move |engine, path| {
                            engine.emit(Event::McpServerSaved {
                                id: saved_id,
                                path: path.display().to_string(),
                            });
                        },
                    )
                    .await;
                }
                Err(message) => self.emit(Event::Error {
                    error: EngineError {
                        kind: EngineErrorKind::Config,
                        message,
                    },
                }),
            }
        }
    }

    pub(super) fn detect_servers(&self) {
        let config = self.config.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let servers = crate::setup::detect(config).await;
            let _ = tx.send(Event::ServersDetected { servers });
        });
    }

    pub(super) async fn start_sign_in(&mut self, id: Option<String>) {
        if let Some(task) = self.sign_in.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(id) = id {
            let config = self.config.clone();
            let tx = self.tx.clone();
            self.sign_in = Some(tokio::spawn(async move {
                let result = match crate::setup::sign_in_provider(&config, &id) {
                    Ok(provider) => {
                        provider
                            .sign_in(|url| {
                                let _ = tx.send(Event::SignInUrl {
                                    id: id.clone(),
                                    url: url.to_owned(),
                                });
                            })
                            .await
                    }
                    Err(error) => Err(error),
                };
                let _ = tx.send(Event::SignInResult {
                    id,
                    error: result.err(),
                });
            }));
        }
    }

    pub(super) fn list_services(&self) {
        let config = self.config.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let services =
                tokio::task::spawn_blocking(move || crate::setup::service_infos(&config))
                    .await
                    .unwrap_or_default();
            let _ = tx.send(Event::Services { services });
        });
    }

    pub(super) fn set_provider_key(&self, id: String, key: quack_protocol::SecretText) {
        // The session (and the planner, or the editor it stands in for) sends the
        // new key from its next request, a running turn's included.
        let live = self
            .convo
            .iter()
            .flat_map(|c| [Some(&c.setup), c.editor.as_ref()])
            .flatten()
            .map(|s| s.provider.clone())
            .filter(|p| p.id == id)
            .collect();
        self.connect(live, move |config| {
            crate::setup::set_provider_key(config, &id, &key.0).map(|saved| crate::setup::Added {
                id,
                path: saved.path,
                note: saved.note,
            })
        });
    }

    pub(super) fn list_provider_connections(&self) {
        let config = self.config.clone();
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            let connections = crate::setup::provider_connections(&config);
            let _ = tx.send(Event::ProviderConnections { connections });
        });
    }

    pub(super) async fn save_provider_connection(
        &mut self,
        id: String,
        http_version: Option<quack_protocol::ProviderHttpVersion>,
        no_proxy: Option<bool>,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
        reload: &mpsc::Sender<Reload>,
    ) {
        self.apply_config_edit(
            done,
            reload,
            move |config| {
                crate::setup::save_provider_connection(config, &id, http_version, no_proxy)
            },
            |engine, (connection, path)| {
                engine.emit(Event::ProviderConnectionSaved {
                    connection,
                    path: path.display().to_string(),
                });
            },
        )
        .await;
    }

    pub(super) fn delete_memory(&mut self, scope: String, name: String) {
        let store = crate::env::memory_store(&self.config);
        let result = Scope::parse(&scope)
            .ok_or_else(|| format!("unknown scope `{scope}`"))
            .and_then(|s| store.delete(s, &name));
        match result {
            Ok(()) => self.list_memory(),
            Err(e) => self.warn(format!("cannot delete note: {e}")),
        }
    }

    pub(super) fn context_report(&self) {
        let last_cache_hit = self.stats.lock().ok().and_then(|s| s.last_cache_hit());
        match &self.convo {
            Some(c) => self.emit(Event::ContextReport {
                report: Box::new(crate::context::report(
                    &c.setup,
                    c.history.as_deref(),
                    last_cache_hit,
                )),
            }),
            None => self.warn("no model is ready, so there is no context yet".into()),
        }
    }

    pub(super) fn stats_report(&self) {
        let model = self
            .convo
            .as_ref()
            .map(|c| c.setup.model_ref.clone())
            .or_else(|| self.session.model.clone())
            .unwrap_or_default();
        let mut report = self
            .stats
            .lock()
            .map(|s| s.report(&model, self.turns))
            .unwrap_or_default();
        if let Some(c) = &self.convo {
            let s = turn::sampling(&c.setup);
            report.sampling = [
                ("temperature", s.temperature),
                ("top_p", s.top_p),
                ("top_k", s.top_k.map(f64::from)),
                ("min_p", s.min_p),
                ("repetition_penalty", s.repetition_penalty),
            ]
            .into_iter()
            .filter_map(|(k, v)| Some((k.to_owned(), v?)))
            .collect();
        }
        self.emit(Event::Stats {
            report: Box::new(report),
        });
    }

    pub(super) fn probe_tool_calling(&self) {
        let Some(c) = &self.convo else {
            return self.warn("no model is ready to probe".into());
        };
        let provider = c.setup.provider.clone();
        let model = c.setup.model.id.clone();
        let mode = c.setup.model.tool_calling.mode;
        let reasoning = c.setup.model.reasoning.format;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            match quack_providers::probe::run_tool_probe(&provider, &model, mode, reasoning).await {
                Ok(report) => {
                    let _ = tx.send(Event::ToolProbeResult { report });
                }
                Err(e) => {
                    let _ = tx.send(Event::Warning {
                        message: format!("tool probe failed: {e}"),
                    });
                }
            }
        });
    }

    pub(super) fn list_lsp_diagnostics(&self, server_id: Option<&str>) {
        let root = &self.config.dirs.workspace_root;
        let items = self
            .lsp
            .diagnostics()
            .into_iter()
            .filter(|entry| server_id.is_none_or(|id| entry.server == id))
            .map(|entry| LspDiagnosticItem {
                server: entry.server,
                path: quack_tools::paths::display(root, &entry.path),
                line: entry.diagnostic.line.saturating_add(1),
                column: entry.diagnostic.column.saturating_add(1),
                severity: entry.diagnostic.severity.label().to_owned(),
                message: entry.diagnostic.message,
                code: entry.diagnostic.code,
            })
            .collect();
        self.emit(Event::LspDiagnostics { items });
    }
}

/// Steps of a reload, reported back to the engine loop.
/// Provider `id` built again from its file as it is now, with its key read afresh.
pub(super) fn fresh_provider(config: &Loaded, id: &str) -> Result<Provider, String> {
    let file = quack_config::load_providers(config)
        .into_iter()
        .find(|i| i.id == id)
        .and_then(|i| i.loaded().cloned())
        .ok_or_else(|| format!("provider `{id}` did not load"))?;
    Provider::new(id, file, &config.settings.network, &|k| std::env::var_os(k))
        .map_err(|e| e.message)
}

/// What `/init` asks the model to do: merge other agents' files into AGENTS.md, update
/// an existing AGENTS.md, or write one from scratch.
pub(super) fn init_prompt(config: &Loaded) -> String {
    let root = &config.dirs.workspace_root;
    let others = crate::env::unmerged(config);
    let has_agents = root.join("AGENTS.md").is_file();
    let guide = "Keep it short and concrete: exact build, test and lint commands, the project \
                 layout, conventions that are not obvious from the code, and pitfalls. Do not \
                 invent anything you have not checked.";
    if !others.is_empty() {
        format!(
            "Consolidate this project's agent instructions into AGENTS.md at the workspace \
             root. Read {}{}. Write one AGENTS.md that keeps every instruction that still \
             applies, drops duplicates, and is organized under short headings; if two files \
             contradict each other, keep both points and tell me so I can decide. {guide} Do \
             not change or delete the other files. When you are done, list them and suggest \
             replacing each with a single line `@AGENTS.md`, so those tools keep reading the \
             same instructions.",
            others.join(", "),
            if has_agents {
                " and the existing AGENTS.md"
            } else {
                ""
            },
        )
    } else if has_agents {
        format!(
            "Review AGENTS.md at the workspace root against the repository as it is now, and \
             update it where it is wrong, missing something important, or out of date. {guide}"
        )
    } else {
        format!(
            "Create AGENTS.md at the workspace root: instructions for coding agents working in \
             this repository. Explore the repository first. {guide}"
        )
    }
}

pub(super) struct AddMcpFields {
    pub(super) id: String,
    pub(super) url: String,
    pub(super) command: String,
    pub(super) args: Vec<String>,
    pub(super) bearer_env: Option<String>,
    pub(super) bearer_token: Option<quack_protocol::SecretText>,
    pub(super) scope: Option<WriteScope>,
}

pub(super) struct SavedMcpFields {
    pub(super) id: String,
    pub(super) description: String,
    pub(super) command: String,
    pub(super) args: Vec<String>,
    pub(super) cwd: String,
    pub(super) url: String,
    pub(super) sandbox: Option<bool>,
    pub(super) env: BTreeMap<String, McpSecretValue>,
    pub(super) headers: BTreeMap<String, McpSecretValue>,
}

pub(super) fn build_saved_mcp_server(fields: SavedMcpFields) -> Result<NewMcpServer, String> {
    let command = fields.command.trim().to_owned();
    let url = fields.url.trim().to_owned();
    let transport = match (command.is_empty(), url.is_empty()) {
        (false, true) => {
            if !fields.headers.is_empty() {
                return Err("headers only apply to remote HTTP MCP servers".into());
            }
            NewMcpTransport::Stdio {
                command,
                args: fields.args,
                env: fields
                    .env
                    .into_iter()
                    .map(|(k, v)| (k, secret_from_mcp(v)))
                    .collect(),
                cwd: (!fields.cwd.trim().is_empty()).then(|| fields.cwd.trim().to_owned()),
                sandbox: fields.sandbox,
            }
        }
        (true, false) => {
            if !fields.env.is_empty() {
                return Err("env only applies to local stdio MCP servers".into());
            }
            NewMcpTransport::Http {
                url,
                headers: fields
                    .headers
                    .into_iter()
                    .map(|(k, v)| (k, secret_from_mcp(v)))
                    .collect(),
            }
        }
        (true, true) => return Err("set `command` (a local server) or `url` (a remote one)".into()),
        (false, false) => return Err("set either `command` or `url`, not both".into()),
    };
    let desc = fields.description.trim();
    Ok(NewMcpServer {
        id: fields.id,
        description: (!desc.is_empty()).then(|| desc.to_owned()),
        transport,
        auto_approve: Vec::new(),
        startup_timeout_secs: None,
        project: false,
        overwrite: true,
    })
}

pub(super) fn secret_from_mcp(value: McpSecretValue) -> Secret {
    match value {
        McpSecretValue::Plain { value } => Secret::Plain(value.0),
        McpSecretValue::Env { env, format } => Secret::Env { env, format },
        McpSecretValue::Keyring { keyring } => Secret::Keyring { keyring },
    }
}
