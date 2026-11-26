//! Turn execution, compaction, context pruning, and prompt attachments.

use super::*;

impl Engine {
    pub(super) async fn user_turn(
        &mut self,
        items: Vec<InputItem>,
        done: &mpsc::Sender<(TurnId, Vec<Message>)>,
    ) {
        if self.reloading {
            self.queue_turn(items, "the config reload");
        } else if self.switching {
            self.queue_turn(items, "the planner model");
        } else if !self.started {
            self.queue_turn(items, "startup");
        } else if self.running.is_some() {
            self.queue_turn(items, "the current turn");
        } else {
            self.start_turn(items, done.clone()).await;
        }
    }

    pub(super) fn interrupt(&mut self) {
        if let Some((_, cancel)) = &self.running {
            cancel.cancel();
        }
        self.queued.clear();
    }

    pub(super) fn answer_questions(
        &mut self,
        call_id: CallId,
        answers: Vec<quack_protocol::Answer>,
        stop_asking: bool,
    ) {
        let waiting = self
            .questions
            .lock()
            .ok()
            .and_then(|mut p| p.remove(&call_id.0));
        match waiting {
            Some(tx) => {
                let _ = tx.send(quack_tools::Answers {
                    answers,
                    stop_asking,
                });
            }
            None => self.emit(Event::Warning {
                message: format!("no pending question for call {}", call_id.0),
            }),
        }
    }

    pub(super) fn answer_approval(
        &mut self,
        call_id: CallId,
        decision: quack_protocol::ApprovalDecision,
    ) {
        let waiting = self
            .approvals
            .lock()
            .ok()
            .and_then(|mut p| p.remove(&call_id.0));
        match waiting {
            Some(tx) => {
                let _ = tx.send(decision);
            }
            None => self.emit(Event::Warning {
                message: format!("no pending approval for call {}", call_id.0),
            }),
        }
    }

    pub(super) fn answer_continue(&mut self, turn: TurnId, proceed: bool) {
        let waiting = self.continues.lock().ok().and_then(|mut p| p.remove(&turn));
        match waiting {
            Some(tx) => {
                let _ = tx.send(proceed);
            }
            None => self.emit(Event::Warning {
                message: format!("turn {} is not paused at its step limit", turn.0),
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn turn_ctx(
        &self,
        turn: TurnId,
        setup: Arc<Setup>,
        perms: Arc<Mutex<Permissions>>,
        nested: Arc<Nested>,
        loaded_skills: Arc<Mutex<session::LoadedSkills>>,
        cancel: CancellationToken,
        pool: Arc<crate::agent::Pool>,
        responses_session: Arc<quack_providers::ResponsesSession>,
        last_reminders: Arc<Mutex<turn::ReminderState>>,
        egress_guard: Arc<Mutex<turn::EgressGuardState>>,
    ) -> TurnCtx {
        TurnCtx {
            turn,
            setup,
            perms,
            nested,
            loaded_skills,
            approvals: self.approvals.clone(),
            questions: self.questions.clone(),
            tx: self.tx.clone(),
            cancel,
            log: self.log.clone(),
            main_log: self.log.clone(),
            checkpoints: None,
            stats: self.stats.clone(),
            hooks: self.hooks.clone(),
            session: self.session.id.0.clone(),
            agent: None,
            agent_type: None,
            max_steps: self.config.settings.agent.max_steps.get() as usize,
            pool,
            prompt_lock: self.prompt_lock.clone(),
            continues: self.continues.clone(),
            network: self.config.settings.network.clone(),
            ledger: self.ledger.clone(),
            secret_allowlist: self.secret_allowlist.clone(),
            responses_session,
            last_reminders,
            egress_guard,
        }
    }

    /// Run `/compact` like a turn: it owns the history until it is done and Esc stops it.
    pub(super) fn start_compact(
        &mut self,
        focus: Option<String>,
        done: mpsc::Sender<(TurnId, Vec<Message>)>,
    ) {
        let pool = self.pool();
        let Some(convo) = &mut self.convo else {
            return self.warn("no model is ready to write a summary".into());
        };
        let Some(mut history) = convo.history.take() else {
            return;
        };
        self.next_turn += 1;
        let turn = TurnId(self.next_turn);
        let cancel = CancellationToken::new();
        self.running = Some((turn, cancel.clone()));
        let setup = convo.setup.clone();
        let perms = convo.perms.clone();
        let nested = convo.nested.clone();
        let loaded_skills = convo.loaded_skills.clone();
        let responses_session = convo.responses_session.clone();
        let last_reminders = convo.last_reminders.clone();
        let egress_guard = convo.egress_guard.clone();
        let ctx = TurnCtx {
            checkpoints: None,
            ..self.turn_ctx(
                turn,
                setup,
                perms,
                nested,
                loaded_skills,
                cancel,
                pool,
                responses_session,
                last_reminders,
                egress_guard,
            )
        };
        tokio::spawn(async move {
            ctx.emit(Event::TurnStarted { turn });
            let result =
                crate::context::compact_now(&ctx, &mut history, 1, focus.as_deref(), "/compact")
                    .await;
            ctx.emit(match result {
                Ok(()) => Event::TurnComplete {
                    turn,
                    usage: Default::default(),
                },
                Err(reason) => Event::TurnAborted { turn, reason },
            });
            let _ = done.send((turn, history)).await;
        });
    }

    pub(super) async fn load_images(&mut self, items: &[InputItem]) -> Vec<quack_protocol::Image> {
        let Some(setup) = self.convo.as_ref().map(|c| c.setup.clone()) else {
            return Vec::new();
        };
        let cwd = self.config.dirs.cwd.clone();
        let paths: Vec<String> = items
            .iter()
            .filter_map(|item| match item {
                InputItem::Image { path } => Some(path.clone()),
                _ => None,
            })
            .collect();
        // Attached images load here, so the turn itself stays synchronous.
        let mut images = Vec::new();
        for path in paths {
            let setup = setup.clone();
            let cwd = cwd.clone();
            let loaded = tokio::task::spawn_blocking(move || load_image(&setup, &cwd, &path))
                .await
                .unwrap_or_else(|e| {
                    tracing::error!("the image loading task failed: {e}");
                    Err("the image loading task failed".into())
                });
            match loaded {
                Ok(img) => images.push(img),
                Err(e) => self.warn(e),
            }
        }
        images
    }

    pub(super) async fn start_turn(
        &mut self,
        items: Vec<InputItem>,
        done: mpsc::Sender<(TurnId, Vec<Message>)>,
    ) {
        self.next_turn += 1;
        let turn = TurnId(self.next_turn);
        let prompt = items
            .iter()
            .filter_map(|i| match i {
                InputItem::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let files: Vec<String> = items
            .iter()
            .filter_map(|i| match i {
                InputItem::File { path } => Some(path.clone()),
                _ => None,
            })
            .collect();
        let images = self.load_images(&items).await;

        // `/<server>:<prompt> args` runs an MCP prompt, unless a skill or custom command
        // has that name. Arguments are checked here; the prompt is fetched in the turn.
        let shadowed = |setup: &Setup| {
            let name = prompt
                .strip_prefix('/')
                .and_then(|r| r.split_whitespace().next())
                .unwrap_or_default();
            setup.tool_ctx.skills.iter().any(|s| s.name == name)
                || setup.commands.iter().any(|c| c.name == name)
        };
        let mcp_prompt = match &self.convo {
            Some(c) if !shadowed(&c.setup) => crate::mcp::prompt_call(&self.mcp.manager, &prompt),
            _ => None,
        };
        let mcp_prompt = match mcp_prompt {
            Some(Err(message)) => {
                self.emit(Event::TurnStarted { turn });
                self.emit(Event::Error {
                    error: EngineError {
                        kind: EngineErrorKind::Config,
                        message: message.clone(),
                    },
                });
                self.emit(Event::TurnAborted {
                    turn,
                    reason: message,
                });
                return;
            }
            Some(Ok(call)) => Some(call),
            None => None,
        };

        let Some(convo) = &mut self.convo else {
            let message = match &self.problem {
                Some(p) => format!("No model is ready: {p}"),
                None => "No model is ready.".to_owned(),
            };
            self.emit(Event::TurnStarted { turn });
            self.emit(Event::Error {
                error: EngineError {
                    kind: EngineErrorKind::Config,
                    message,
                },
            });
            self.emit(Event::TurnAborted {
                turn,
                reason: "no model".into(),
            });
            return;
        };
        let Some(history) = convo.history.take() else {
            return;
        };
        let planner_start = if convo.editor.is_some() && convo.plan_start.is_none() {
            convo.plan_start = Some(history.len());
            convo.plan_start
        } else {
            None
        };
        let history_len = history.len();
        let setup = convo.setup.clone();
        let perms = convo.perms.clone();
        let nested = convo.nested.clone();
        let loaded_skills = convo.loaded_skills.clone();
        let responses_session = convo.responses_session.clone();
        let last_reminders = convo.last_reminders.clone();
        let egress_guard = convo.egress_guard.clone();
        // `expand_skill` borrows the typed prompt; when it expands a skill the
        // expansion replaces the request body, so the original is kept once, not twice.
        let typed = prompt;
        let expansion = if mcp_prompt.is_some() {
            PromptExpansion::default()
        } else {
            expand_skill(&setup, &typed)
        };
        let PromptExpansion {
            text: expanded,
            skill: loaded_skill,
            error: skill_error,
        } = expansion;
        let notes = (!self.notes.is_empty()).then(|| std::mem::take(&mut self.notes).join("\n"));
        if let Some(e) = skill_error {
            self.warn(e);
        }
        // Plan mode asks for a plan at the end of the message, after the cached prefix.
        let plan = perms
            .lock()
            .is_ok_and(|p| p.work_mode == WorkMode::Plan)
            .then(|| plan_note(&setup));
        // A correction or lasting preference: remind the model to save it.
        let remember = setup
            .surface
            .name_of("memory")
            .filter(|_| {
                plan.is_none()
                    && self.config.settings.memory.nudge
                    && setup.tool_ctx.memory.is_some()
                    && crate::nudge::lasting(&typed)
            })
            .map(crate::nudge::note);
        let compose = move |body: String| {
            let body = match notes {
                Some(n) => format!("{n}\n\n{body}"),
                None => body,
            };
            let body = match remember {
                Some(r) => format!("{body}\n\n{r}"),
                None => body,
            };
            match plan {
                Some(p) => format!("{body}\n\n{p}"),
                None => body,
            }
        };
        let manager = self.mcp.manager.clone();
        self.ensure_log().await;
        if let Some(skill_name) = loaded_skill {
            session::record(
                self.log.as_ref(),
                "skill",
                serde_json::json!({ "name": skill_name }),
            );
            let updated = loaded_skills
                .lock()
                .ok()
                .and_then(|mut s| s.record(history_len, skill_name).then(|| s.names()));
            if let Some(skills) = updated {
                self.emit(Event::LoadedSkills { skills });
            }
        }
        if let Some(at) = planner_start {
            session::record(
                self.log.as_ref(),
                "planner_start",
                serde_json::json!({ "at": at }),
            );
        }
        self.turns += 1;
        let untitled = self.title.is_none();
        if untitled {
            // A stand-in until the small model's title arrives (or for good without one).
            let first = typed.lines().map(str::trim).find(|l| !l.is_empty());
            self.set_title(first.map(|l| title::shorten(l, title::MAX_CHARS)));
        }
        if untitled && self.turns == 1 && !typed.trim().is_empty() && self.log.is_some() {
            if self.title_targets_local(&setup) {
                self.pending_title = Some((setup.clone(), typed.clone()));
            } else {
                self.generate_title(&setup, &typed);
            }
        }
        let cancel = CancellationToken::new();
        self.running = Some((turn, cancel.clone()));
        let pool = self.pool();
        let ctx = TurnCtx {
            checkpoints: self.checkpoints(),
            ..self.turn_ctx(
                turn,
                setup,
                perms,
                nested,
                loaded_skills,
                cancel,
                pool,
                responses_session,
                last_reminders,
                egress_guard,
            )
        };
        let start = self.start_context.take();
        self.spawn_turn(
            ctx,
            manager,
            TurnLaunch {
                done,
                history,
                mcp_prompt,
                typed,
                expanded,
                files,
                images,
                start,
            },
            compose,
        );
    }

    pub(super) fn spawn_turn(
        &self,
        ctx: TurnCtx,
        manager: Arc<McpManager>,
        launch: TurnLaunch,
        compose: impl FnOnce(String) -> String + Send + 'static,
    ) {
        let TurnLaunch {
            done,
            history,
            mcp_prompt,
            typed,
            expanded,
            files,
            images,
            start,
        } = launch;
        let turn = ctx.turn;
        let is_mcp_prompt = mcp_prompt.is_some();
        tokio::spawn(async move {
            let body = match mcp_prompt {
                None => expanded.unwrap_or_else(|| typed.clone()),
                Some((server, name, args)) => {
                    match manager.get_prompt(&server, &name, args).await {
                        Ok(text) => text,
                        Err(e) => {
                            let reason = format!("/{server}:{name}: {e}");
                            ctx.emit(Event::TurnStarted { turn });
                            ctx.emit(Event::Error {
                                error: EngineError {
                                    kind: EngineErrorKind::Config,
                                    message: reason.clone(),
                                },
                            });
                            ctx.emit(Event::TurnAborted { turn, reason });
                            let _ = done.send((turn, history)).await;
                            return;
                        }
                    }
                }
            };
            let body = if !is_mcp_prompt {
                let mention_setup = ctx.setup.clone();
                let mention_tx = ctx.tx.clone();
                let raw_body = body.clone();
                let explicit_files = files;
                let expansion = tokio::task::spawn_blocking(move || {
                    crate::mentions::expand_with_files(
                        &mention_setup.tool_ctx.root,
                        &mention_setup.tool_ctx.grammars,
                        &raw_body,
                        &explicit_files,
                    )
                })
                .await
                .unwrap_or_default();
                for message in expansion.warnings {
                    let _ = mention_tx.send(Event::Warning { message });
                }
                if expansion.attachments.is_empty() {
                    body
                } else if body.is_empty() {
                    expansion.attachments
                } else {
                    format!("{body}\n\n{}", expansion.attachments)
                }
            } else {
                body
            };
            let prompt = turn::Prompt {
                text: compose(body),
                typed,
                images,
                start,
            };
            let history = turn::run(ctx, history, prompt).await.history;
            let _ = done.send((turn, history)).await;
        });
    }

    /// Queue a turn that cannot start yet and tell the user what it waits for.
    pub(super) fn queue_turn(&mut self, items: Vec<InputItem>, waiting_for: &str) {
        self.emit(Event::TurnQueued {
            waiting_for: waiting_for.into(),
        });
        self.queued.push_back(Queued::Turn(items));
    }

    pub(super) fn prune_context(&mut self, indices: Option<Vec<usize>>) {
        let Some((report, ctx_report)) = (match &mut self.convo {
            None => {
                self.warn("no model is ready, so there is no context to prune".into());
                return;
            }
            Some(c) => {
                let Some(history) = c.history.as_mut() else {
                    return;
                };
                let report = crate::context::prune_selected(
                    &c.setup,
                    history,
                    indices.as_deref(),
                    self.log.as_ref(),
                    &self.tx,
                );
                let last_cache_hit = self.stats.lock().ok().and_then(|s| s.last_cache_hit());
                let ctx_report = crate::context::report(&c.setup, Some(history), last_cache_hit);
                Some((report, ctx_report))
            }
        }) else {
            return;
        };
        if report.items.is_empty() {
            self.emit(Event::ContextPruned { report });
        }
        self.emit(Event::ContextReport {
            report: Box::new(ctx_report),
        });
    }
}

/// Load an image the user attached to a turn. Relative paths resolve against the
/// working directory, the way the model's do.
pub(super) fn load_image(
    setup: &Setup,
    cwd: &std::path::Path,
    path: &str,
) -> Result<quack_protocol::Image, String> {
    if !setup.tool_ctx.vision {
        return Err(format!(
            "{path}: this model takes no images (set `vision = true` on it to attach them)"
        ));
    }
    let resolved = std::path::Path::new(path);
    let resolved = if resolved.is_absolute() {
        resolved.to_path_buf()
    } else {
        cwd.join(resolved)
    };
    quack_protocol::Image::from_path(&resolved)
}

#[derive(Debug, Default)]
pub(super) struct PromptExpansion {
    pub(super) text: Option<String>,
    pub(super) skill: Option<String>,
    pub(super) error: Option<String>,
}

/// `/<skill> [args]` becomes the skill's instructions followed by the request. Anything
/// else (including unknown `/words`) is sent as typed, so the expansion is `None` and
/// the caller keeps the prompt it passed in.
pub(super) fn expand_skill(setup: &Setup, prompt: &str) -> PromptExpansion {
    let Some(rest) = prompt.strip_prefix('/') else {
        return PromptExpansion::default();
    };
    let (name, args) = rest
        .split_once(char::is_whitespace)
        .map_or((rest, ""), |(n, a)| (n, a.trim()));
    let Some(skill) = setup.tool_ctx.skills.iter().find(|s| s.name == name) else {
        // A custom command: its prompt with the arguments filled in.
        return match quack_instructions::commands::expand(&setup.commands, prompt) {
            Some(Ok(text)) => PromptExpansion {
                text: Some(text),
                ..Default::default()
            },
            Some(Err(e)) => PromptExpansion {
                error: Some(format!("/{name}: {e}")),
                ..Default::default()
            },
            None => PromptExpansion::default(),
        };
    };
    match quack_tools::expand_skill(skill) {
        Ok(text) => {
            let request = if args.is_empty() {
                "Use this skill now."
            } else {
                args
            };
            PromptExpansion {
                text: Some(format!("{text}\n\n{request}")),
                skill: Some(skill.name.clone()),
                error: None,
            }
        }
        Err(e) => PromptExpansion {
            error: Some(format!("skill /{name}: {e}")),
            ..Default::default()
        },
    }
}
