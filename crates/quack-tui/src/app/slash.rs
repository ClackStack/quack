//! Slash command dispatch, modal openers, and settings persistence.

use super::*;

impl App {
    #[cfg(test)]
    pub(crate) fn skip_setup_offer(&mut self) {
        self.setup_offered = true;
    }

    /// Tests: the user has started working, so the startup diagnostics have stepped aside.
    #[cfg(test)]
    pub(crate) fn dismiss_startup(&mut self) {
        self.startup_dismissed = true;
    }

    /// Open `/setup` with what is known, and list the hosted services again.
    pub(super) fn open_setup(&mut self) {
        self.setup.settings = self.info.settings.clone();
        self.modal = Some(modal::setup(&self.setup, &self.theme));
        self.ops.push(Op::ListServices);
    }

    pub(super) fn open_hosted_setup(&mut self) {
        self.open_setup();
        if let Some(modal) = self.modal.take() {
            self.modal = Some(modal.with_tab(1));
        }
    }

    /// Redraw an open `/setup` with new findings, keeping the user's place.
    pub(super) fn refresh_setup(&mut self) {
        self.refresh_modal("Setup", |app| {
            modal::setup(&app.setup, &app.theme).keep_place(app.modal.as_ref())
        });
    }

    pub(super) fn set_mode(&mut self, mode: &str) {
        match mode {
            "plan" => self.ops.push(Op::SetWorkMode {
                mode: WorkMode::Plan,
            }),
            "edit" => self.ops.push(Op::SetWorkMode {
                mode: WorkMode::Edit,
            }),
            "yolo" => {
                if self.info.permission_mode != "yolo" {
                    self.mode_before_yolo = self.info.permission_mode.clone();
                }
                self.ops.push(Op::SetPermissionMode {
                    mode: PermissionMode::Yolo,
                });
            }
            _ => match mode.parse::<PermissionMode>() {
                Ok(mode) => self.ops.push(Op::SetPermissionMode { mode }),
                Err(()) => self.note(format!(
                    "unknown mode `{mode}`; use plan/edit for work mode or ask/allow for approvals"
                )),
            },
        }
    }

    /// Toggle approvals between Ask and Allow (or leave Yolo), keeping Plan/Edit and the input text intact.
    pub(super) fn cycle_approvals(&mut self) {
        if self.info.permission_mode == "yolo" {
            self.toggle_yolo("off");
        } else if self.info.permission_mode == "allow" {
            self.set_mode("ask");
        } else {
            self.set_mode("allow");
        }
    }

    /// `/yolo`: switch to yolo, or back to the mode it was turned on from.
    pub(super) fn toggle_yolo(&mut self, args: &str) {
        let on = match args.trim() {
            "on" => true,
            "off" => false,
            _ => self.info.permission_mode != "yolo",
        };
        if on {
            if self.info.permission_mode != "yolo" {
                self.mode_before_yolo = self.info.permission_mode.clone();
            }
            self.set_mode("yolo");
        } else {
            let back = match self.mode_before_yolo.as_str() {
                "" | "yolo" => "ask".to_owned(),
                m => m.to_owned(),
            };
            self.set_mode(&back);
        }
    }

    /// `/sandbox`: turn the OS sandbox off for this session, or back on.
    pub(super) fn toggle_sandbox(&mut self, args: &str) {
        let off = match args.trim() {
            "off" => true,
            "on" => false,
            "" => !self.sandbox_off,
            other => {
                return self.note(format!(
                    "unknown argument `{other}`; use /sandbox on or /sandbox off"
                ));
            }
        };
        self.ops.push(Op::SetSandbox { off });
    }

    /// Leave planning without submitting an implementation turn.
    pub(super) fn end_plan(&mut self) {
        self.set_mode("edit");
    }

    /// The engine preserves approvals and performs any planner handoff.
    pub(super) fn accept_plan(&mut self) {
        self.review = None;
        self.startup_dismissed = true;
        self.transcript
            .push(Entry::User("Carry out the plan above.".into()));
        self.ops.push(Op::AcceptPlan {
            mode: String::new(),
        });
    }

    /// Shift-Tab changes the work mode without changing approvals.
    pub(super) fn cycle_mode(&mut self) {
        self.set_mode(match self.info.work_mode {
            WorkMode::Edit => "plan",
            WorkMode::Plan => "edit",
        });
    }

    /// Run a `/command args` line.
    pub(crate) fn execute(&mut self, text: &str) {
        // Any command counts as starting work: the startup diagnostics step aside.
        self.startup_dismissed = true;
        let rest = text.trim().trim_start_matches('/');
        let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        match commands::find(name) {
            Some(cmd) => self.run_command(cmd, args.trim()),
            // The engine expands `/<skill> args` into the skill's instructions, and fetches
            // MCP prompts.
            None if self.skills.iter().any(|s| s.name == name)
                || self.custom.iter().any(|c| c.name == name)
                || self.mcp_prompts.iter().any(|c| c.name == name) =>
            {
                let attachments = std::mem::take(&mut self.attachments);
                self.send(text.trim().to_owned(), attachments);
            }
            None => self.note(format!("unknown command /{name}; /help lists commands")),
        }
    }

    pub(super) fn run_command(&mut self, cmd: &Command, args: &str) {
        self.popup_selected = 0;
        match cmd.name {
            "models" if args.starts_with("default-menu ") => {
                let reference = args["default-menu ".len()..].trim();
                if !reference.is_empty() {
                    self.default_picker_model = Some(reference.to_owned());
                    self.modal = Some(modal::model_defaults(
                        reference,
                        self.model_roles(),
                        self.write_scope,
                        &self.theme,
                    ));
                }
            }
            "models"
                if {
                    let rest = take_scope_flag(args, self.write_scope).1;
                    rest == "default" || rest.starts_with("default ")
                } =>
            {
                let (scope, rest) = take_scope_flag(args, self.write_scope);
                let after = rest.strip_prefix("default").unwrap_or_default().trim();
                let (scope, after) = split_scope_flag(after, scope);
                let (role, reference) = match after.split_once(char::is_whitespace) {
                    Some((r @ ("large" | "small" | "both"), target)) => (r, target.trim()),
                    _ => ("large", after),
                };
                let (scope, reference) = split_scope_flag(reference, scope);
                self.save_model_default(role, reference, scope);
            }
            "models"
                if {
                    let rest = take_scope_flag(args, self.write_scope).1;
                    rest == "small" || rest.starts_with("small ")
                } =>
            {
                let (scope, rest) = take_scope_flag(args, self.write_scope);
                let after = rest.strip_prefix("small").unwrap_or_default().trim();
                let (scope, reference) = split_scope_flag(after, scope);
                self.save_model_default("small", reference, scope);
            }
            "models" if !args.is_empty() => match args.split_once('/') {
                Some((provider, model)) if !provider.is_empty() && !model.is_empty() => {
                    self.note(format!("switching to {args}…"));
                    // A model that declares thinking levels asks for one once switched.
                    self.pick_thinking = self
                        .catalog
                        .iter()
                        .find(|p| p.id == provider)
                        .and_then(|p| {
                            p.models
                                .iter()
                                .find(|m| m.alias.as_deref().unwrap_or(&m.id) == model)
                        })
                        .filter(|m| !m.reasoning_efforts.is_empty())
                        .map(|_| args.to_owned());
                    self.ops.push(Op::SetModel {
                        provider: ProviderId(provider.to_owned()),
                        model: ModelId(model.to_owned()),
                    });
                }
                _ => self.note("usage: /models <provider>/<model>".into()),
            },
            "instructions" => {
                self.modal = Some(modal::instructions(None, &self.theme));
                self.ops.push(Op::ListInstructions);
            }
            "profiles"
                if take_scope_flag(args, self.write_scope)
                    .1
                    .starts_with("default ") =>
            {
                let (scope, rest) = take_scope_flag(args, self.write_scope);
                let after = rest["default ".len()..].trim();
                let (scope, id) = split_scope_flag(after, scope);
                if id.is_empty() {
                    self.note("usage: /profiles default [--project|--user] <id>".into());
                } else {
                    self.save_setting("model.profile", &toml_string(id), scope);
                }
            }
            "profiles" if !args.is_empty() => {
                self.note(format!("switching to the {args} profile…"));
                self.ops.push(Op::SetProfile {
                    id: args.to_owned(),
                });
            }
            "profiles" => {
                self.modal = Some(modal::profiles(None, self.write_scope, &self.theme));
                self.ops.push(Op::ListProfiles);
            }
            "models" => {
                self.modal = Some(modal::models(
                    None,
                    self.model_roles(),
                    self.info.ui.ascii,
                    self.write_scope,
                    &self.theme,
                ));
                self.ops.push(Op::ListModels);
            }
            "thinking" if args.is_empty() => self.open_thinking(),
            "thinking" if args.starts_with("level ") => {
                self.ops.push(Op::SetReasoningEffort {
                    effort: Some(args["level ".len()..].trim().to_owned()),
                });
            }
            "thinking" if matches!(args, "show" | "on" | "hide" | "off") => {
                self.toggle_reasoning(Some(matches!(args, "show" | "on")));
            }
            "thinking" => {
                self.ops.push(Op::SetReasoningEffort {
                    effort: (args != "default").then(|| args.to_owned()),
                });
            }
            "resume" if !args.is_empty() => {
                self.resuming = true;
                self.ops.push(Op::ResumeSession {
                    id: SessionId(args.to_owned()),
                });
            }
            "resume" => {
                self.modal = Some(modal::sessions(None, &self.theme));
                self.ops.push(Op::ListSessions);
            }
            "new" => {
                if self.turn_running {
                    return self.note("wait for the current turn to finish first".into());
                }
                self.transcript.clear();
                self.agents.clear();
                self.plan.clear();
                self.plan_expanded = false;
                self.context = None;
                self.growth.clear();
                self.starting_new = true;
                self.session_saved = false;
                self.ops.push(Op::NewSession);
                self.note("new session; the previous one is saved (/resume)".into());
            }
            "fork" if args.starts_with("--worktree") => {
                let name = args
                    .strip_prefix("--worktree")
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                self.ops.push(Op::ForkWorktree { name });
            }
            "fork" => self.ops.push(Op::ForkSession),
            "export" => {
                let (first, rest) = args.split_once(char::is_whitespace).unwrap_or((args, ""));
                let (format, path) = match first {
                    "md" | "markdown" => (ExportFormat::Markdown, rest.trim()),
                    "json" => (ExportFormat::Json, rest.trim()),
                    // A path alone: its extension picks the format.
                    path if path.to_ascii_lowercase().ends_with(".json") => {
                        (ExportFormat::Json, args)
                    }
                    _ => (ExportFormat::Markdown, args),
                };
                self.ops.push(Op::Export {
                    format,
                    path: (!path.is_empty()).then(|| path.to_owned()),
                });
            }
            "title" if args.is_empty() => self.note("usage: /title <text>".into()),
            "title" => {
                self.ops.push(Op::SetTitle {
                    title: args.to_owned(),
                });
                self.note(format!("session renamed to “{args}”"));
            }
            "config" => {
                let (scope, rest) = take_scope_flag(args, self.write_scope);
                if rest.is_empty() {
                    self.write_scope = scope;
                    self.modal = Some(modal::config(
                        &self.info.settings,
                        &self.info.layers,
                        self.write_scope,
                        self.info.project_status,
                        &self.theme,
                    ));
                } else {
                    let after_set = rest.strip_prefix("set").map(str::trim_start);
                    let parsed = after_set.and_then(|r| {
                        let (scope, r) = take_scope_flag(r, scope);
                        r.split_once(char::is_whitespace)
                            .map(|(k, v)| (scope, k, v))
                    });
                    match parsed {
                        Some((scope, key, value)) if !value.trim().is_empty() => {
                            self.save_setting(key, value.trim(), scope);
                        }
                        _ => {
                            self.note("usage: /config set [--project|--user] <key> <value>".into())
                        }
                    }
                }
            }
            "scope" => match args.trim() {
                "" => {
                    self.write_scope = toggle_write_scope(self.write_scope);
                    self.note(self.scope_note());
                }
                "user" | "global" => {
                    self.write_scope = WriteScope::User;
                    self.note(self.scope_note());
                }
                "project" | ".quack" => {
                    self.write_scope = WriteScope::Project;
                    self.note(self.scope_note());
                }
                _ => self.note("usage: /scope [user|project]".into()),
            },
            "memory" if args.is_empty() => {
                self.modal = Some(modal::memory(None, 0, &self.theme));
                self.ops.push(Op::ListMemory);
            }
            "memory" => match args
                .strip_prefix("delete")
                .map(str::trim)
                .and_then(|r| r.split_once('/'))
            {
                Some((scope, name)) if !name.is_empty() => {
                    self.ops.push(Op::DeleteMemory {
                        scope: scope.to_owned(),
                        name: name.to_owned(),
                    });
                    self.note(format!("deleted {scope} note {name}"));
                }
                _ => self.note("usage: /memory, or /memory delete <global|project>/<name>".into()),
            },
            "key" if !args.trim().is_empty() => {
                let id = args.trim();
                self.modal = None;
                self.prompt = Some(Prompt::new_key(id, self.key_sources.get(id)));
            }
            "providers" if !args.is_empty() => self.provider_command(args),
            "providers" | "key" => self.open_providers(),
            "mcp" => {
                let (scope, mcp_args) = take_scope_flag(args, self.write_scope);
                if mcp_args.is_empty() {
                    self.write_scope = scope;
                    self.modal = Some(modal::mcp(
                        None,
                        &self.info.mcp_items,
                        self.write_scope,
                        &self.theme,
                    ));
                    self.ops.push(Op::ListMcp);
                } else if let Some(sub) = mcp_args
                    .strip_prefix("add")
                    .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
                {
                    let (scope, kind) = split_scope_flag(sub, scope);
                    match kind {
                        "" => self.prompt = Some(Prompt::mcp_transport(scope)),
                        "http" | "remote" => self.prompt = Some(Prompt::mcp_http_url(scope)),
                        "stdio" | "local" => self.prompt = Some(Prompt::mcp_stdio_cmd(scope)),
                        _ => self.note(
                            "usage: /mcp [add [--project|--user] [http|stdio] | enable <id> | disable <id> | restart <id>]"
                                .into(),
                        ),
                    }
                } else if let Some(id) = mcp_args.strip_prefix("enable ") {
                    let id = id.trim().to_owned();
                    self.note(format!("enabling MCP server {id}…"));
                    self.ops.push(Op::EnableMcp { id });
                } else if let Some(id) = mcp_args.strip_prefix("disable ") {
                    let id = id.trim().to_owned();
                    self.note(format!("disabling MCP server {id}…"));
                    self.ops.push(Op::DisableMcp { id });
                } else if let Some(id) = mcp_args.strip_prefix("restart ") {
                    let id = id.trim().to_owned();
                    self.note(format!("restarting MCP server {id}…"));
                    self.ops.push(Op::RestartMcp { id });
                } else {
                    self.note(
                        "usage: /mcp [add [--project|--user] [http|stdio] | enable <id> | disable <id> | restart <id>]"
                            .into(),
                    );
                }
            }
            "lsp" => {
                self.modal = Some(modal::lsp(None, &self.theme));
                self.ops.push(Op::ListLsp);
            }
            "init" => {
                self.note("asking the model to write AGENTS.md…".into());
                self.ops.push(Op::InitInstructions);
            }
            "setup" if args.starts_with("use ") => {
                let mut parts = args["use ".len()..].splitn(2, ' ');
                let found = match &self.setup.local {
                    modal::Scan::Found(list) => list.as_slice(),
                    _ => &[],
                };
                let server = parts
                    .next()
                    .and_then(|i| i.parse::<usize>().ok())
                    .and_then(|i| found.get(i));
                match (server, parts.next()) {
                    (Some(s), Some(model)) => self.ops.push(Op::AddProvider {
                        id: s.id.clone(),
                        base_url: s.base_url.clone(),
                        api: s.api.clone(),
                        model: model.to_owned(),
                    }),
                    _ => self.note("pick a server and model from /setup".into()),
                }
            }
            "setup" if args.starts_with("login ") => {
                let id = args["login ".len()..].trim().to_owned();
                self.signing_in = Some(id.clone());
                self.setup.sign_in_error = None;
                self.browser = None;
                self.modal = Some(modal::sign_in(&id, None, &self.theme));
                self.ops.push(Op::SignIn { id: Some(id) });
            }
            "setup" if args == "url" => self.prompt = Some(Prompt::server_url()),
            "setup" if args.starts_with("key ") => {
                let id = args["key ".len()..].trim();
                let service = self.setup.services.iter().flatten().find(|s| s.id == id);
                self.prompt = Some(Prompt::service_key(service, id));
            }
            "setup" if args.starts_with("models ") => {
                let id = args["models ".len()..].trim();
                self.modal = Some(
                    modal::models(
                        None,
                        self.model_roles(),
                        self.info.ui.ascii,
                        self.write_scope,
                        &self.theme,
                    )
                    .with_filter(&format!("{id}/")),
                );
                self.ops.push(Op::ListModels);
            }
            "setup" => {
                // Asking for /setup is asking to look; the first-run offer waits for "scan".
                self.setup.local = modal::Scan::Looking;
                self.ops.push(Op::DetectServers);
                self.open_setup();
            }
            "undo" => self.ops.push(Op::Undo {
                checkpoint: None,
                force: args == "--force",
            }),
            "skills" => self.modal = Some(modal::skills(&self.skills, &self.theme)),
            "bundles" => {
                let (scope, rest) = take_scope_flag(args, self.write_scope);
                if rest.is_empty() {
                    self.write_scope = scope;
                    self.modal = Some(modal::bundles(
                        &self.info.bundles,
                        self.write_scope,
                        &self.theme,
                    ));
                } else {
                    match rest
                        .strip_prefix("toggle ")
                        .map(|r| split_scope_flag(r, scope))
                    {
                        Some((scope, id)) if self.info.bundles.iter().any(|b| b.id == id) => {
                            let off = !self.info.bundles_disabled.iter().any(|d| d == id);
                            let list = toggle_disabled(&self.info.bundles_disabled, id, off);
                            self.modal = None;
                            self.ops.push(Op::SaveSetting {
                                key: "bundles.disabled".into(),
                                value: list,
                                scope,
                            });
                            self.ops.push(Op::ReloadConfig);
                            self.note(format!(
                                "bundle `{id}` turned {}; reloading",
                                if off { "off" } else { "on" }
                            ));
                        }
                        _ => self.note(
                            "usage: /bundles, or /bundles toggle [--project|--user] <name>".into(),
                        ),
                    }
                }
            }
            "tasks" => self.modal = Some(modal::tasks(&self.agents, &self.theme)),
            "todo" => {
                self.plan_expanded = !self.plan_expanded;
                if self.plan.is_empty() {
                    self.note(
                        "no plan yet; the model makes one with its `todo` tool for multi-step work"
                            .into(),
                    );
                }
            }
            "commands" => {
                let all: Vec<_> = self
                    .custom
                    .iter()
                    .chain(&self.mcp_prompts)
                    .cloned()
                    .collect();
                self.modal = Some(modal::custom_commands(&all, &self.theme));
            }
            "doctor" => {
                self.modal = Some(modal::doctor(None, &self.theme));
                self.ops.push(Op::RunDoctor);
            }
            "probe" => {
                self.modal = Some(modal::tool_probe(None, &self.theme));
                self.ops.push(Op::ProbeToolCalling);
            }
            "gui" => self.request_gui_handoff(quack_core::gui::find_binary().is_some()),
            "copy" => {
                let last = self.transcript.iter().rev().find_map(|e| match e {
                    Entry::Assistant(t) if !t.trim().is_empty() => Some(t.clone()),
                    _ => None,
                });
                let Some(text) = last else {
                    return self.note("nothing to copy yet".into());
                };
                let (what, text) = match args {
                    "code" => match last_code_block(&text) {
                        Some(code) => ("the last code block", code),
                        None => return self.note("the last response has no code block".into()),
                    },
                    _ => ("the last response", text),
                };
                self.note(format!(
                    "copied {what} ({} characters) through the terminal",
                    text.chars().count()
                ));
                self.clipboard = Some(text);
            }
            "permissions" if args.is_empty() => {
                self.modal = Some(modal::permissions(
                    &self.info.permission_mode,
                    &self.info.rules,
                    &self.theme,
                ));
            }
            "permissions" => self.set_mode(args),
            "edit" => {
                self.set_mode("edit");
                if !args.is_empty() {
                    let attachments = std::mem::take(&mut self.attachments);
                    self.send(args.to_owned(), attachments);
                }
            }
            "yolo" => self.toggle_yolo(args),
            "sandbox" => self.toggle_sandbox(args),
            "plan" => match args {
                "" => self.set_mode("plan"),
                "accept" | "go" => {
                    if self.info.work_mode != WorkMode::Plan {
                        return self.note("not in plan mode; /plan starts it".into());
                    }
                    self.accept_plan();
                }
                "off" | "exit" => self.end_plan(),
                task => {
                    self.set_mode("plan");
                    let attachments = std::mem::take(&mut self.attachments);
                    self.send(task.to_owned(), attachments);
                }
            },
            "context" => {
                self.modal = Some(modal::context(
                    None,
                    &self.growth,
                    self.info.ui.ascii,
                    &self.theme,
                ));
                self.ops.push(Op::GetContext);
            }
            "stats" => {
                self.modal = Some(modal::stats(None, self.info.ui.ascii, &self.theme));
                self.ops.push(Op::GetStats);
            }
            "compact" => self.ops.push(Op::Compact {
                focus: Some(args.to_owned()).filter(|a| !a.is_empty()),
            }),
            "rewind" if args.is_empty() => self.open_rewind(),
            "rewind" => {
                let mut words = args.split_whitespace();
                let back = words.next().and_then(|n| n.parse::<u32>().ok());
                let mut what = "both";
                let mut force = false;
                for w in words {
                    match w {
                        "--force" => force = true,
                        w => what = w,
                    }
                }
                match (back, what) {
                    (Some(back), "both" | "conversation" | "files") if back > 0 => {
                        self.ops.push(Op::Rewind {
                            back,
                            conversation: what != "files",
                            files: what != "conversation",
                            force,
                        });
                    }
                    _ => self.note("usage: /rewind [n] [both|conversation|files] [--force]".into()),
                }
            }
            "reload" => {
                self.note("reloading config…".into());
                self.ops.push(Op::ReloadConfig);
            }
            "trust" => match args {
                "" => self.modal = Some(modal::trust(self.trust.as_ref(), &self.theme)),
                "later" => {
                    self.note("skipping the project config for now; /trust to decide".into())
                }
                decision @ ("trusted" | "settings-only" | "ignored") => {
                    self.note(format!("project config: {decision}; reloading…"));
                    self.ops.push(Op::TrustProject {
                        decision: decision.to_owned(),
                    });
                }
                _ => self.note("usage: /trust [trusted|settings-only|ignored]".into()),
            },
            "hooks" if take_scope_flag(args, self.write_scope).1.is_empty() => {
                let (scope, _) = take_scope_flag(args, self.write_scope);
                self.write_scope = scope;
                self.modal = Some(modal::hooks(
                    &self.info.hooks,
                    &self.hook_runs,
                    self.write_scope,
                    &self.theme,
                ));
            }
            "hooks" if args.starts_with("configure ") || args.starts_with("test ") => {
                if let Some((action, id)) = args.split_once(' ') {
                    self.open_hook_setup(id.trim());
                    self.hook_test_after_inspect = action == "test";
                }
            }
            "hooks"
                if args.starts_with("keep-payload ") || args.starts_with("replace-payload ") =>
            {
                if let Some((choice, id)) = args.split_once(' ') {
                    self.ops.push(Op::EnableHook {
                        id: id.to_owned(),
                        replace_payload: Some(choice == "replace-payload"),
                    });
                }
            }
            "hooks" => match args
                .strip_prefix("toggle ")
                .map(|r| split_scope_flag(r, self.write_scope))
            {
                Some((scope, id))
                    if self
                        .info
                        .hooks
                        .iter()
                        .any(|h| h.id == id && h.layer != quack_config::LayerKind::System) =>
                {
                    // On means the engine enables the hook wherever it is disabled: it
                    // copies one shipped with `enabled = false` into the user layer, and
                    // drops the id from `[hooks] disabled`.
                    let on = self
                        .info
                        .hooks
                        .iter()
                        .find(|h| h.id == id)
                        .is_some_and(|h| h.loaded().is_some_and(|f| f.enabled))
                        && !self.info.hooks_disabled.iter().any(|d| d == id);
                    self.modal = None;
                    if on {
                        let list = toggle_disabled(&self.info.hooks_disabled, id, true);
                        self.ops.push(Op::SaveSetting {
                            key: "hooks.disabled".into(),
                            value: list,
                            scope,
                        });
                        self.ops.push(Op::ReloadConfig);
                        self.note(format!("hook `{id}` turned off; reloading"));
                    } else {
                        self.ops.push(Op::EnableHook {
                            id: id.to_owned(),
                            replace_payload: None,
                        });
                    }
                }
                _ => self.note("usage: /hooks, or /hooks toggle [--project|--user] <name>".into()),
            },
            "theme" => match split_scope_flag(args, self.write_scope) {
                (scope, "") => {
                    self.write_scope = scope;
                    self.modal = Some(modal::theme(
                        &self.info.ui.theme,
                        self.write_scope,
                        &self.theme,
                    ));
                }
                (scope, mode @ ("dark" | "light" | "auto")) => {
                    self.save_setting("ui.theme", &toml_string(mode), scope);
                }
                _ => self.note("usage: /theme [--project|--user] [dark|light|auto]".into()),
            },
            "keys" => self.modal = Some(modal::help(&self.theme).with_tab(1)),
            "help" => self.modal = Some(modal::help(&self.theme)),
            "quit" => self.quit = true,
            name => self.note(format!(
                "/{name} is planned for {}",
                cmd.pending.unwrap_or("a later milestone")
            )),
        }
    }

    pub(super) fn open_rewind(&mut self) {
        if self.turn_running {
            return self.note("wait for the current turn to finish before rewinding".into());
        }
        self.modal = Some(modal::rewind(None, &self.theme));
        self.ops.push(Op::ListRewindPoints);
    }

    pub(super) fn open_providers(&mut self) {
        self.connection_picker_provider = None;
        self.modal = Some(modal::providers(
            None,
            None,
            self.info.model.as_deref(),
            &self.theme,
        ));
        self.ops.push(Op::ListModels);
        self.ops.push(Op::ListProviderConnections);
    }

    pub(super) fn open_providers_selected(&mut self, id: &str) {
        self.modal = Some(
            modal::providers(
                self.catalog_loaded.then_some(self.catalog.as_slice()),
                self.provider_connections.as_deref(),
                self.info.model.as_deref(),
                &self.theme,
            )
            .select_matching(|item| modal::provider_label_id(&item.label) == id),
        );
    }

    pub(super) fn refresh_providers(&mut self, current: Option<&str>) {
        let selected_id = self
            .modal
            .as_ref()
            .filter(|m| m.title == "Providers")
            .and_then(|m| m.selected_item())
            .map(|item| modal::provider_label_id(&item.label).to_owned());
        self.refresh_modal("Providers", |app| {
            let modal = modal::providers(
                app.catalog_loaded.then_some(app.catalog.as_slice()),
                app.provider_connections.as_deref(),
                current,
                &app.theme,
            )
            .keep_place(app.modal.as_ref());
            match selected_id.as_deref() {
                Some(id) => {
                    modal.select_matching(|item| modal::provider_label_id(&item.label) == id)
                }
                None => modal,
            }
        });
    }

    pub(super) fn provider_command(&mut self, args: &str) {
        use quack_protocol::ProviderHttpVersion::*;
        if matches!(args, "advanced" | "connection") {
            self.open_providers();
            return;
        }
        let (action, id) = args.split_once(' ').unwrap_or_default();
        let id = id.trim();
        let (http_version, no_proxy) = match action {
            "advanced" | "connection" => {
                if let Some(provider) = self
                    .provider_connections
                    .iter()
                    .flatten()
                    .find(|p| p.id == id)
                    .cloned()
                {
                    self.connection_picker_provider = Some(provider.id.clone());
                    self.modal = Some(modal::provider_connection(&provider, &self.theme));
                } else {
                    self.note("open /providers to choose a configured provider".into());
                }
                return;
            }
            "http-inherit" => (Some(Inherit), None),
            "http-1" => (Some(Http1Only), None),
            "http-negotiate" => (Some(Negotiate), None),
            "proxy-inherit" => (None, Some(false)),
            "proxy-direct" => (None, Some(true)),
            _ => return self.note("open /providers to edit connection settings".into()),
        };
        self.connection_picker_provider = None;
        self.open_providers_selected(id);
        self.ops.push(Op::SaveProviderConnection {
            id: id.into(),
            http_version,
            no_proxy,
        });
    }

    pub(super) fn open_thinking(&mut self) {
        self.modal = Some(modal::thinking(
            self.info.model.as_deref(),
            &self.reasoning_efforts,
            self.reasoning_effort.as_deref(),
            &self.theme,
        ));
    }

    pub(super) fn save_setting(&mut self, key: &str, value: &str, scope: WriteScope) {
        self.ops.push(Op::SaveSetting {
            key: key.to_owned(),
            value: value.to_owned(),
            scope,
        });
    }

    pub(super) fn scope_note(&self) -> String {
        match self.write_scope {
            WriteScope::Project => {
                let status = match self.info.project_status {
                    ProjectStatus::DisabledByPolicy => " · disabled by policy",
                    ProjectStatus::Untrusted | ProjectStatus::Changed | ProjectStatus::Ignored => {
                        " · untrusted (run `/trust` to load)"
                    }
                    ProjectStatus::None | ProjectStatus::Trusted | ProjectStatus::SettingsOnly => {
                        ""
                    }
                };
                format!("write scope: project (.quack/){status}")
            }
            WriteScope::User | WriteScope::Session => "write scope: user config".into(),
        }
    }

    pub(super) fn save_model_default(&mut self, role: &str, reference: &str, scope: WriteScope) {
        let Some((provider, model)) = reference.split_once('/') else {
            self.note(
                "usage: /models default [large|small|both] [--user|--project] <provider>/<model>"
                    .into(),
            );
            return;
        };
        if provider.is_empty() || model.is_empty() {
            self.note(
                "usage: /models default [large|small|both] [--user|--project] <provider>/<model>"
                    .into(),
            );
            return;
        }
        let value = format!("\"{provider}/{model}\"");
        match role {
            "large" => {
                self.ops.push(Op::SaveSetting {
                    key: "model.default".into(),
                    value,
                    scope,
                });
            }
            "small" => {
                self.ops.push(Op::SaveSetting {
                    key: "model.small".into(),
                    value,
                    scope,
                });
            }
            "both" => {
                self.ops.push(Op::SaveSetting {
                    key: "model.default".into(),
                    value: value.clone(),
                    scope,
                });
                self.ops.push(Op::SaveSetting {
                    key: "model.small".into(),
                    value,
                    scope,
                });
            }
            _ => {
                self.note(
                    "usage: /models default [large|small|both] [--user|--project] <provider>/<model>"
                        .into(),
                );
            }
        }
    }

    pub(super) fn refresh_scoped_modal(&mut self, old: &Modal) {
        let next = match old.title.as_str() {
            "Config" => Some(modal::config(
                &self.info.settings,
                &self.info.layers,
                self.write_scope,
                self.info.project_status,
                &self.theme,
            )),
            "Models" => Some(modal::models(
                self.catalog_loaded.then_some(self.catalog.as_slice()),
                self.model_roles(),
                self.info.ui.ascii,
                self.write_scope,
                &self.theme,
            )),
            "Default role" => self.default_picker_model.as_deref().map(|reference| {
                modal::model_defaults(reference, self.model_roles(), self.write_scope, &self.theme)
            }),
            "Profiles" => Some(modal::profiles(
                self.profiles_cache
                    .as_ref()
                    .map(|(active, forced, list)| (active.as_str(), *forced, list.as_slice())),
                self.write_scope,
                &self.theme,
            )),
            "Bundles" => Some(modal::bundles(
                &self.info.bundles,
                self.write_scope,
                &self.theme,
            )),
            "Theme" => Some(modal::theme(
                &self.info.ui.theme,
                self.write_scope,
                &self.theme,
            )),
            "Hooks" => Some(modal::hooks(
                &self.info.hooks,
                &self.hook_runs,
                self.write_scope,
                &self.theme,
            )),
            "MCP" => Some(modal::mcp(
                self.mcp_servers_cache.as_deref(),
                &self.info.mcp_items,
                self.write_scope,
                &self.theme,
            )),
            _ => None,
        };
        if let Some(m) = next {
            self.modal = Some(m.keep_place(Some(old)));
        }
    }
}

pub(super) fn take_scope_flag(args: &str, default: WriteScope) -> (WriteScope, &str) {
    let s = args.trim();
    if let Some(rest) = s
        .strip_prefix("--project")
        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
    {
        (WriteScope::Project, rest.trim())
    } else if let Some(rest) = s
        .strip_prefix("--user")
        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
    {
        (WriteScope::User, rest.trim())
    } else {
        (default, s)
    }
}

/// Flip between the user and project write targets; session scope switches to project.
pub(super) fn toggle_write_scope(scope: WriteScope) -> WriteScope {
    match scope {
        WriteScope::Project => WriteScope::User,
        WriteScope::User | WriteScope::Session => WriteScope::Project,
    }
}

pub(super) fn split_scope_flag(args: &str, default: WriteScope) -> (WriteScope, &str) {
    let (scope, s) = take_scope_flag(args, default);
    if let Some(head) = s
        .strip_suffix("--project")
        .filter(|h| h.is_empty() || h.ends_with(char::is_whitespace))
    {
        (WriteScope::Project, head.trim())
    } else if let Some(head) = s
        .strip_suffix("--user")
        .filter(|h| h.is_empty() || h.ends_with(char::is_whitespace))
    {
        (WriteScope::User, head.trim())
    } else {
        (scope, s)
    }
}

/// The TOML `disabled` array after adding or removing `id`.
pub(super) fn toggle_disabled(disabled: &[String], id: &str, disable: bool) -> String {
    let mut disabled = disabled.to_vec();
    if disable {
        disabled.push(id.to_owned());
    } else {
        disabled.retain(|d| d != id);
    }
    let list = disabled
        .iter()
        .map(|d| toml_string(d))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{list}]")
}

/// A TOML string literal, so ids like `true` or `1.5` stay strings.
pub(super) fn toml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The contents of the last fenced code block in `text`.
pub(super) fn last_code_block(text: &str) -> Option<String> {
    let mut last = None;
    let mut current: Option<Vec<&str>> = None;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            match current.take() {
                Some(lines) => last = Some(lines.join("\n")),
                None => current = Some(Vec::new()),
            }
        } else if let Some(lines) = &mut current {
            lines.push(line);
        }
    }
    last
}
