//! Work modes, approval policies, sandbox toggle, and planner/editor handoff.

use super::*;

impl Engine {
    pub(super) fn set_mode(&mut self, mode: PermissionMode) {
        match mode {
            PermissionMode::Plan => self.set_work_mode(WorkMode::Plan),
            PermissionMode::Edit => self.set_work_mode(WorkMode::Edit),
            mode => {
                if let Some(policy) = mode.policy() {
                    self.set_approval_policy(policy);
                }
            }
        }
    }

    pub(super) fn set_work_mode(&mut self, mode: WorkMode) {
        let Some(c) = &self.convo else {
            return self.warn("no model is ready; the mode applies once one is".into());
        };
        if mode == WorkMode::Edit
            && self.config.settings.permissions.work_mode == WorkMode::Plan
            && self.config.settings.policy.locks("permissions.work_mode")
        {
            self.warn(
                "`permissions.work_mode` is locked by the system policy; staying in plan".into(),
            );
            return self.emit_mode();
        }
        let changed = if let Ok(mut p) = c.perms.lock() {
            let changed = p.work_mode != mode;
            p.work_mode = mode;
            changed
        } else {
            false
        };
        if changed {
            match mode {
                WorkMode::Plan => self.start_planner(),
                WorkMode::Edit => {
                    self.saved_plan_start = None;
                    if let Some(c) = &mut self.convo {
                        c.plan_start = None;
                    }
                    self.restore_editor();
                }
            }
        }
        self.emit_mode();
    }

    pub(super) fn set_approval_policy(&mut self, requested: ApprovalPolicy) {
        let Some(c) = &self.convo else {
            return self.warn("no model is ready; the approvals apply once one is".into());
        };
        let settings = &self.config.settings;
        let mode = if self.mode_locked() && requested != settings.permissions.mode {
            self.warn(format!(
                "`permissions.mode` is locked by the system policy; staying in {}",
                settings.permissions.mode.label()
            ));
            settings.permissions.mode
        } else {
            let capped = settings
                .policy
                .max_permission_mode
                .map_or(requested, |cap| requested.min(cap));
            if capped != requested {
                self.warn(format!(
                    "{} is not allowed by policy; using {}",
                    requested.label(),
                    capped.label()
                ));
            }
            capped
        };
        if let Ok(mut p) = c.perms.lock() {
            p.mode = mode;
        }
        self.emit_mode();
    }

    pub(super) fn restore_modes(&mut self) {
        if let Some((work_mode, approvals)) = self.saved_mode.take() {
            let cli_sets = |key: &str| {
                self.config.origins.iter().any(|(path, origin)| {
                    *origin == quack_config::Origin::Cli
                        && (path == key || key.starts_with(&format!("{path}.")))
                })
            };
            let work_mode = if cli_sets("permissions.work_mode") {
                self.config.settings.permissions.work_mode
            } else {
                work_mode
            };
            let approvals = if cli_sets("permissions.mode") {
                self.config.settings.permissions.mode
            } else {
                approvals
            };
            // Restore the restriction before emitting or recording either setting.
            // A locked Plan setting also narrows a previously saved Edit session.
            let mode = if self.config.settings.permissions.work_mode == WorkMode::Plan
                && self.config.settings.policy.locks("permissions.work_mode")
            {
                WorkMode::Plan
            } else {
                work_mode
            };
            if mode == WorkMode::Edit {
                self.saved_plan_start = None;
            }
            if let Some(c) = &mut self.convo {
                if let Ok(mut p) = c.perms.lock() {
                    p.work_mode = mode;
                }
                if mode == WorkMode::Plan && c.plan_start.is_none() {
                    c.plan_start = self.saved_plan_start;
                } else if mode == WorkMode::Edit {
                    c.plan_start = None;
                }
            }
            self.set_approval_policy(approvals);
        }
    }

    pub(super) fn mode_locked(&self) -> bool {
        self.config.settings.policy.locks("permissions.mode")
    }

    pub(super) fn sandbox_locked(&self) -> bool {
        self.config.settings.policy.locks("sandbox.mode")
    }

    /// `/sandbox on|off`: run shell commands without the OS sandbox this session, whatever
    /// the permission mode, or go back to `[sandbox] mode`.
    pub(super) fn set_sandbox(&mut self, off: bool) {
        let Some(c) = &self.convo else {
            return self.warn("no model is ready; the sandbox setting applies once one is".into());
        };
        if off && self.sandbox_locked() {
            self.warn("`sandbox.mode` is locked by the system policy; the sandbox stays on".into());
            return self.emit(Event::Sandbox { off: false });
        }
        if let Ok(mut p) = c.perms.lock() {
            p.sandbox_off = off;
        }
        if !off {
            match &c.setup.tool_ctx.sandbox {
                quack_tools::SandboxState::On(_) => {}
                quack_tools::SandboxState::Off => self.warn(format!(
                    "[sandbox] mode = \"{}\", so commands still run without a sandbox · set it in config.toml",
                    self.config.settings.sandbox.mode.label()
                )),
                quack_tools::SandboxState::Unavailable(why) => self.warn(format!(
                    "the sandbox is not available ({why}); every shell command asks first"
                )),
            }
        }
        self.emit(Event::Sandbox { off });
    }

    pub(super) fn emit_mode(&self) {
        let Some((work_mode, approvals)) = self
            .convo
            .as_ref()
            .and_then(|c| c.perms.lock().ok().map(|p| (p.work_mode, p.mode)))
        else {
            return;
        };
        session::record(
            self.log.as_ref(),
            "mode",
            serde_json::json!({
                "work_mode": work_mode, "approvals": approvals,
            }),
        );
        // Retain the legacy notification for older clients.
        let mode = if work_mode == WorkMode::Plan {
            PermissionMode::Plan
        } else {
            PermissionMode::from(approvals)
        };
        self.emit(Event::PermissionMode { mode });
        self.emit(Event::ModeChanged {
            work_mode,
            approvals,
        });
    }

    /// Plan mode began: plan with the `[model] planner`, when one is set and it is not
    /// the model in use already.
    pub(super) fn start_planner(&mut self) {
        let planner = self.config.settings.model.planner.clone();
        let Some(c) = &self.convo else { return };
        if self.switching
            || planner.is_empty()
            || c.editor.is_some()
            || planner == c.setup.model_ref
        {
            return;
        }
        let Some((provider, model)) = planner.split_once('/') else {
            return self.warn(format!(
                "model.planner `{planner}` is not <provider>/<model>; planning with {}",
                c.setup.model_ref
            ));
        };
        self.switching = true;
        self.switch_to(Role::Planner, provider.to_owned(), model.to_owned());
    }

    /// The planner's setup is ready. The session's own setup waits in `editor` until plan
    /// mode ends; the todo list is shared, so the plan carries over.
    pub(super) fn on_planner(&mut self, result: Result<Setup, String>) {
        self.switching = false;
        let mut setup = match result {
            Ok(s) => s,
            Err(e) => {
                return self.warn(format!(
                    "cannot plan with {}: {e}; planning with the session's model",
                    self.config.settings.model.planner
                ));
            }
        };
        let Some(c) = &mut self.convo else { return };
        // Plan mode ended while the planner was loading.
        if c.editor.is_some() || !c.perms.lock().is_ok_and(|p| p.work_mode == WorkMode::Plan) {
            return;
        }
        c.reset_transient();
        setup.provider.set_ledger(self.ledger.clone());
        setup.tool_ctx.plan = c.setup.tool_ctx.plan.clone();
        c.editor = Some(std::mem::replace(&mut c.setup, Arc::new(setup)));
        c.plan_start = c.plan_start.or_else(|| self.saved_plan_start.take());
        self.announce_model("planner");
    }

    /// Plan mode ended: go back to the session's own model. Returns where the planner's
    /// messages start in the history, when it wrote any.
    pub(super) fn restore_editor(&mut self) -> Option<usize> {
        let c = self.convo.as_mut()?;
        let editor = c.editor.take()?;
        c.setup = editor;
        c.reset_transient();
        let start = c.plan_start.take();
        self.announce_model("session");
        start
    }

    /// Tell the clients and the log which model the conversation now uses.
    pub(super) fn announce_model(&mut self, role: &str) {
        let Some(setup) = self.convo.as_ref().map(|c| c.setup.clone()) else {
            return;
        };
        self.session.model = Some(setup.model_ref.clone());
        self.session.profile = setup.profile.id.clone();
        setup.provider.set_ledger(self.ledger.clone());
        self.merge_secret_allowlist();
        self.configure_reasoning(&setup);
        session::record(
            self.log.as_ref(),
            "model_switch",
            serde_json::json!({ "to": setup.model_ref, "profile": setup.profile.id, "role": role }),
        );
        self.emit(Event::SessionConfigured {
            session: Box::new(self.session.clone()),
        });
    }

    /// `/plan accept`: leave plan mode for `mode` and carry out the plan. After a planner,
    /// or when `clear_context` is true, the planning messages give way to one brief, so the
    /// implementing model reads only that on top of the history it already has cached.
    pub(super) async fn accept_plan(
        &mut self,
        mode: &str,
        clear_context: bool,
        done: mpsc::Sender<(TurnId, Vec<Message>)>,
    ) {
        let approvals = (!mode.is_empty())
            .then(|| serde_json::from_value::<ApprovalPolicy>(serde_json::Value::from(mode)).ok())
            .flatten();
        if !mode.is_empty() && approvals.is_none() {
            return self.warn(format!(
                "cannot carry out the plan with `{mode}` approvals; use ask or allow"
            ));
        }
        if self.config.settings.permissions.work_mode == WorkMode::Plan
            && self.config.settings.policy.locks("permissions.work_mode")
        {
            return self.warn("`permissions.work_mode` is locked by the system policy; the plan cannot be implemented".into());
        }
        let Some(c) = &mut self.convo else {
            return self.warn("no model is ready yet".into());
        };
        if !c.perms.lock().is_ok_and(|p| p.work_mode == WorkMode::Plan) {
            return self.warn("not in plan mode; /plan starts it".into());
        }
        let mut skills_after_handoff = None;
        let same_model = c.editor.is_none();
        let brief = if c.editor.is_some() || clear_context {
            let start = c.plan_start.or_else(|| {
                clear_context.then(|| {
                    c.history
                        .as_ref()
                        .and_then(|h| h.iter().rposition(|m| matches!(m, Message::User { .. })))
                        .unwrap_or(0)
                })
            });
            match (start, c.history.as_mut()) {
                (Some(start), Some(history)) if start < history.len() => {
                    let planner = &c.setup;
                    let planner_ref = planner.model_ref.clone();
                    let start = start.min(history.len());
                    let note = plan_note(planner);
                    let ask = planner.surface.name_of("ask_user");
                    let todo = c
                        .editor
                        .as_deref()
                        .unwrap_or(planner)
                        .surface
                        .name_of("todo");
                    let text = handoff::brief(
                        &history[start..],
                        &note,
                        ask,
                        todo,
                        &planner_ref,
                        same_model,
                    );
                    history.truncate(start);
                    skills_after_handoff = c
                        .loaded_skills
                        .lock()
                        .ok()
                        .and_then(|mut s| s.truncate(start).then(|| s.names()));
                    if same_model {
                        c.reset_transient();
                    }
                    Some((start, planner_ref, text))
                }
                _ => None,
            }
        } else {
            None
        };
        if let Some(skills) = skills_after_handoff {
            self.emit(Event::LoadedSkills { skills });
        }
        if let Some(policy) = approvals {
            self.set_mode(PermissionMode::from(policy));
        }
        self.set_work_mode(WorkMode::Edit);
        let text = match brief {
            Some((start, planner, text)) => {
                session::record(
                    self.log.as_ref(),
                    "handoff",
                    serde_json::json!({ "to": start, "from": planner }),
                );
                text
            }
            None => "Carry out the plan above.".to_owned(),
        };
        self.start_turn(vec![InputItem::text(text)], done).await;
    }

    /// Whether the session is in plan mode (the planner swaps in for the session's model).
    pub(super) fn in_plan_mode(&self) -> bool {
        self.convo
            .as_ref()
            .is_some_and(|c| c.perms.lock().is_ok_and(|p| p.work_mode == WorkMode::Plan))
    }
}

/// The request appended to each message in plan mode: the profile's `plan.md` (or the
/// default), and when read-only sub-agents can run, a pointer to them.
pub(super) fn plan_note(setup: &Setup) -> String {
    let base = setup.profile.plan_prompt.as_deref().unwrap_or(PLAN_PROMPT);
    let read_tools: Vec<&str> = ["read", "grep", "glob", "list_dir", "index", "lsp"]
        .into_iter()
        .filter_map(|t| setup.surface.name_of(t))
        .collect();
    let tool_guidance = match (read_tools.is_empty(), setup.surface.name_of("shell")) {
        (false, Some(shell)) => format!(
            " Prefer dedicated read-only tools ({}) over `{shell}` when possible; `{shell}` in plan mode only runs read-only inspection commands (such as rg, grep, find, sed -n, awk, head, tail, cat, wc, sort, uniq, cut, jq, diff, git status/diff/log/show) with no file writes, redirects, builds, tests or interpreter scripts.",
            read_tools.join(", ")
        ),
        (false, None) => format!(
            " Use the read-only search and file tools ({}).",
            read_tools.join(", ")
        ),
        (true, Some(shell)) => format!(
            " `{shell}` in plan mode only runs read-only inspection commands (such as rg, grep, find, sed -n, awk, head, tail, cat, wc, sort, uniq, cut, jq, diff, git status/diff/log/show) with no file writes, redirects, builds, tests or interpreter scripts."
        ),
        (true, None) => String::new(),
    };
    let note = format!(
        "{base} Only read and search while planning.{tool_guidance} Do not run tests, builds, scripts, edits or memory writes, even when an allow rule exists. Present a list of the exact test commands you propose to run, with what each verifies, for the user to review before implementation. If no tests apply, explain why. Keep proposed test steps pending in the checklist; do not execute them until the user accepts the plan. If an earlier plan is already finished and there is follow-up work to propose, replace the checklist with the new pending follow-up steps. If the work is already finished and no further changes are needed, say so and do not leave pending steps in the checklist."
    );
    let explorers: Vec<&str> = setup
        .tool_ctx
        .agents
        .iter()
        .filter(|a| a.read_only)
        .map(|a| a.id.as_str())
        .collect();
    match setup.surface.name_of("task") {
        Some(task) if !explorers.is_empty() => format!(
            "{note} To explore widely without filling your context, hand searches to \
             read-only agents ({}) with {task}; several started in one reply run in parallel.",
            explorers.join(", ")
        ),
        _ => note,
    }
}
