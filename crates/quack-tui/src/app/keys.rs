//! Terminal key, mouse, paste, prompt, approval, and choice-bar input handling.

use super::*;

impl App {
    /// Route terminal input to the active UI, acting on key Press/Repeat but never Release.
    pub fn on_terminal(&mut self, event: TermEvent) {
        match event {
            // Windows reports Release events too; act on Press and Repeat only.
            TermEvent::Key(key) if key.kind != KeyEventKind::Release => self.on_key(key),
            TermEvent::Paste(text) => {
                self.selection = None;
                if let Some(form) = &mut self.hook_form {
                    form.paste(&text);
                } else if let Some(p) = &mut self.prompt {
                    p.text
                        .push_str(text.lines().next().unwrap_or_default().trim());
                    p.error = None;
                } else {
                    match &mut self.modal {
                        Some(m) => m.paste(&text),
                        None => {
                            // A dropped image file arrives as its path; attach it.
                            if !self.attach_pasted(&text) {
                                self.input.insert_str(&text);
                            }
                        }
                    }
                }
                self.dirty = true;
            }
            TermEvent::Resize(..) => {
                // Rows wrap differently now, so the selected cells hold other text.
                self.selection = None;
                self.dirty = true;
            }
            TermEvent::Mouse(m) if self.modal.is_none() && self.hook_form.is_none() => match m.kind
            {
                MouseEventKind::ScrollUp => self.scroll_up(WHEEL_ROWS),
                MouseEventKind::ScrollDown => self.scroll_down(WHEEL_ROWS),
                MouseEventKind::Down(MouseButton::Left) => self.press(m.column, m.row),
                MouseEventKind::Drag(MouseButton::Left) => self.drag(m.column, m.row),
                MouseEventKind::Up(MouseButton::Left) => self.release(),
                _ => {}
            },
            _ => {}
        }
    }

    /// Columns the input text wraps at: the box as last drawn, less the prompt.
    pub(super) fn input_width(&self) -> usize {
        match self.input_view.get().0.width {
            // Not drawn yet: nothing wraps.
            0 => usize::MAX,
            w => usize::from(w.saturating_sub(2)),
        }
    }

    /// A click in the input box moves the cursor there.
    pub(super) fn click_input(&mut self, x: u16, y: u16) {
        let (area, first) = self.input_view.get();
        if !area.contains((x, y).into()) {
            return;
        }
        let layout = self.input.layout(self.input_width());
        let row = first + usize::from(y - area.y);
        let col = usize::from((x - area.x).saturating_sub(2));
        self.input.place(&layout, row, col);
        self.dirty = true;
    }

    /// The button went down: a click on a status-bar mode pill toggles it in place, a click
    /// in the input places the cursor, and either pane starts a selection there.
    pub(super) fn press(&mut self, x: u16, y: u16) {
        self.selection = None;
        self.dirty = true;
        let at = (x, y).into();
        if self.work_mode_area.get().contains(at) {
            self.cycle_mode();
        } else if self.permission_area.get().contains(at) {
            self.cycle_approvals();
        } else if self.input_view.get().0.contains(at) {
            self.click_input(x, y);
            self.selection = Some(Selection::new(Pane::Input, self.input_cell(x, y)));
        } else if !self.transcript.is_empty() && self.transcript_area.get().contains(at) {
            self.selection = Some(Selection::new(Pane::Transcript, self.transcript_cell(x, y)));
        }
    }

    pub(super) fn drag(&mut self, x: u16, y: u16) {
        let Some(pane) = self.selection.map(|s| s.pane) else {
            return;
        };
        let head = match pane {
            Pane::Transcript => {
                // Past the top or bottom edge, the view scrolls along.
                let area = self.transcript_area.get();
                if y < area.y {
                    self.scroll_up(1);
                } else if y >= area.bottom() {
                    self.scroll_down(1);
                }
                self.transcript_cell(x, y)
            }
            Pane::Input => self.input_cell(x, y),
        };
        if let Some(s) = &mut self.selection {
            s.head = head;
            // A shaky click that stays in its cell is still a click.
            s.dragged |= s.head != s.anchor;
        }
        self.dirty = true;
    }

    /// The button came up: a drag copies what it selected, which stays marked until the
    /// next key or click.
    pub(super) fn release(&mut self) {
        match self.selection {
            Some(sel) if sel.dragged => self.copy_selection(&sel),
            _ => self.selection = None,
        }
        self.dirty = true;
    }

    pub(super) fn copy_selection(&mut self, sel: &Selection) {
        let text = match sel.pane {
            Pane::Transcript => {
                let width = usize::from(self.transcript_area.get().width.saturating_sub(1));
                let (lines, gutters) = crate::ui::transcript_rows_with_gutters(self, width);
                let rows: Vec<(String, usize)> = lines
                    .iter()
                    .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
                    .zip(gutters)
                    .collect();
                select::transcript_text(&rows, sel)
            }
            Pane::Input => {
                let (from, to) = sel.bounds();
                let layout = self.input.layout(self.input_width());
                self.input.slice(&layout, from, to)
            }
        };
        if !text.is_empty() {
            self.copied = Some((self.frame, text.chars().count()));
            self.clipboard = Some(text);
        }
    }

    /// The transcript cell under `(x, y)`, held to the transcript's rows.
    pub(super) fn transcript_cell(&self, x: u16, y: u16) -> (usize, usize) {
        let area = self.transcript_area.get();
        let (total, height) = self.view.get();
        // The first row on screen, as drawing worked it out.
        let first = total
            .saturating_sub(self.scroll.get())
            .saturating_sub(height);
        let y = y.clamp(area.y, area.bottom().saturating_sub(1));
        let row = (first + usize::from(y - area.y)).min(total.saturating_sub(1));
        (row, usize::from(x.saturating_sub(area.x)))
    }

    /// The input cell under `(x, y)`, held to the box; columns leave out the prompt.
    pub(super) fn input_cell(&self, x: u16, y: u16) -> (usize, usize) {
        let (area, first) = self.input_view.get();
        let y = y.clamp(area.y, area.bottom().saturating_sub(1));
        let col = usize::from(x.saturating_sub(area.x)).saturating_sub(2);
        (first + usize::from(y - area.y), col)
    }

    pub(super) fn open_hook_setup(&mut self, id: &str) {
        self.hook_request += 1;
        self.hook_inspect = Some(self.hook_request);
        self.hook_form = None;
        self.hook_reapply = None;
        self.hook_test_after_inspect = false;
        self.modal = Some(Modal::new("Hook setup", Vec::new()));
        self.ops.push(Op::InspectHookSetup {
            id: id.into(),
            request: self.hook_request,
        });
    }

    pub(super) fn on_hook_key(&mut self, key: KeyEvent) {
        let Some(mut form) = self.hook_form.take() else {
            return;
        };
        let key = if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
        {
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
        } else {
            key
        };
        match form.key(key) {
            HookAction::None => {}
            HookAction::Close => {
                self.modal = Some(modal::hooks(
                    &self.info.hooks,
                    &self.hook_runs,
                    self.write_scope,
                    &self.theme,
                ));
                return;
            }
            HookAction::Reload => {
                self.hook_reapply = None;
                self.open_hook_setup(&form.info.id);
                return;
            }
            HookAction::ReloadDraft => {
                let drafts = form.drafts;
                self.open_hook_setup(&form.info.id);
                self.hook_reapply = Some(drafts);
                return;
            }
            HookAction::Cancel(request) => self.ops.push(Op::CancelHookCheck { request }),
            HookAction::Save {
                enable,
                replace_payload,
            } => {
                self.hook_request += 1;
                self.ops.push(Op::SaveHookSetup {
                    id: form.info.id.clone(),
                    request: self.hook_request,
                    revision: form.info.revision.clone(),
                    connections: form.drafts.clone(),
                    enable,
                    replace_payload,
                });
                form.waiting(self.hook_request, false);
            }
            HookAction::Check { connection, url } => {
                self.hook_request += 1;
                self.ops.push(Op::CheckHookSetup {
                    id: form.info.id.clone(),
                    request: self.hook_request,
                    revision: form.info.revision.clone(),
                    connection,
                    url,
                });
                form.waiting(self.hook_request, true);
            }
        }
        self.hook_form = Some(form);
    }

    /// Which surface owns the next key, in `on_key`'s priority order. Each variant names
    /// the `on_*_key` handler it dispatches to; `Base` is the transcript-and-input view.
    ///
    /// `Ctrl-C` deliberately bypasses this order: only the hook form sees it first (as
    /// Esc); everywhere else a dragged selection turns it into a copy, and otherwise it
    /// interrupts or quits whatever `ctrl_c` finds on screen. See `on_key`.
    pub(super) fn focus(&self) -> Focus {
        if self.hook_form.is_some() {
            Focus::HookForm
        } else if self.approval.is_some() {
            Focus::Approval
        } else if self.asking.is_some() {
            Focus::Asking
        } else if self.step_limit.is_some() {
            Focus::StepLimit
        } else if self.prompt.is_some() {
            Focus::Prompt
        } else if self.modal.is_some() {
            Focus::Modal
        } else if self.review.is_some() {
            Focus::Review
        } else {
            Focus::Base
        }
    }

    pub(super) fn on_key(&mut self, key: KeyEvent) {
        self.dirty = true;
        if let Focus::HookForm = self.focus() {
            // The hook form takes every key, including Ctrl-C (as Esc); no selection is
            // released on the way in.
            return self.on_hook_key(key);
        }
        // Any key lets go of a selection. Ctrl-C, out of habit, copies it again rather than
        // interrupting or quitting — this wins over every focus below.
        let selection = self.selection.take().filter(|s| s.dragged);
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return match selection {
                Some(sel) => self.copy_selection(&sel),
                None => self.ctrl_c(),
            };
        }
        match self.focus() {
            // These take every key; ones they don't use are swallowed.
            Focus::Approval => self.on_approval_key(key),
            Focus::Asking => self.on_asking_key(key),
            Focus::Prompt => self.on_prompt_key(key),
            Focus::Modal => self.on_modal_key(key),
            // Bars over the live transcript: a key they don't use falls through to the
            // base handler, so typing stays possible.
            Focus::StepLimit => {
                if !self.on_step_limit_key(key) {
                    self.on_base_key(key);
                }
            }
            Focus::Review => {
                if !self.on_review_key(key) {
                    self.on_base_key(key);
                }
            }
            // HookForm was handled above; unreachable here.
            Focus::HookForm | Focus::Base => self.on_base_key(key),
        }
    }

    /// A key for the open modal. Every key is consumed, even one the modal ignores.
    pub(super) fn on_modal_key(&mut self, key: KeyEvent) {
        let Some(modal) = &mut self.modal else {
            return;
        };
        match modal.on_key(key) {
            ModalOutcome::None => {}
            ModalOutcome::Close => {
                let was_default_role = self
                    .modal
                    .as_ref()
                    .is_some_and(|m| m.title == "Default role");
                let was_connection = self
                    .modal
                    .as_ref()
                    .is_some_and(|m| m.title.starts_with("Connection: "));
                self.modal = None;
                if was_default_role && self.default_picker_model.take().is_some() {
                    self.modal = Some(modal::models(
                        self.catalog_loaded.then_some(self.catalog.as_slice()),
                        self.model_roles(),
                        self.info.ui.ascii,
                        self.write_scope,
                        &self.theme,
                    ));
                } else if was_connection && let Some(id) = self.connection_picker_provider.take() {
                    self.open_providers_selected(&id);
                } else if self.signing_in.take().is_some() {
                    self.browser = None;
                    self.ops.push(Op::SignIn { id: None });
                    self.open_hosted_setup();
                }
            }
            ModalOutcome::Copy(text) => {
                self.copied = Some((self.frame, text.chars().count()));
                self.clipboard = Some(text);
            }
            ModalOutcome::Insert(text) => {
                self.modal = None;
                let existing = self.input.text();
                if text.starts_with('/')
                    && text.ends_with(' ')
                    && !existing.is_empty()
                    && !existing.starts_with('/')
                {
                    let combined = format!("{text}{existing}");
                    self.input.set(&combined);
                } else {
                    self.input.set(&text);
                }
                self.popup_dismissed = true;
            }
            ModalOutcome::Run(text) => {
                self.modal = None;
                self.execute(&text);
            }
            ModalOutcome::ToggleScope => {
                self.write_scope = toggle_write_scope(self.write_scope);
                if let Some(old) = self.modal.take() {
                    self.refresh_scoped_modal(&old);
                }
            }
        }
    }

    /// Keys for the transcript and input box: editing, history, scrolling, popups.
    pub(super) fn on_base_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let popup = self.popup_matches();
        match key.code {
            KeyCode::Esc if !popup.is_empty() => self.popup_dismissed = true,
            KeyCode::Esc if self.turn_running || self.queued => self.ops.push(Op::Interrupt),
            KeyCode::Esc => {
                if self
                    .last_esc
                    .take()
                    .is_some_and(|t| t.elapsed() < DOUBLE_ESC)
                {
                    self.open_rewind();
                } else {
                    self.last_esc = Some(Instant::now());
                }
            }
            KeyCode::Enter
                if key
                    .modifiers
                    .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
            {
                self.input.insert('\n');
            }
            KeyCode::Char('j') if ctrl => self.input.insert('\n'),
            KeyCode::Enter => {
                let idx = self.popup_selected.min(popup.len().saturating_sub(1));
                self.submit(popup.get(idx).cloned());
            }
            KeyCode::BackTab => self.cycle_mode(),
            KeyCode::Char('y') if ctrl => self.cycle_approvals(),
            KeyCode::Tab => {
                let idx = self.popup_selected.min(popup.len().saturating_sub(1));
                if let Some(item) = popup.get(idx) {
                    if item.prefix == '@' {
                        self.input.complete_at(&item.name);
                        self.popup_selected = 0;
                    } else {
                        self.input.set(&format!("/{} ", item.name));
                    }
                }
            }
            _ if self.scroll_key(key) => {}
            KeyCode::Char('t') if ctrl => self.toggle_reasoning(None),
            KeyCode::Up if !popup.is_empty() => {
                self.popup_selected = self.popup_selected.saturating_sub(1);
            }
            KeyCode::Down if !popup.is_empty() => {
                self.popup_selected = (self.popup_selected + 1).min(popup.len() - 1);
            }
            KeyCode::Up => {
                if !self.input.up(self.input_width()) {
                    self.input.history_prev();
                }
            }
            KeyCode::Down => {
                if !self.input.down(self.input_width()) {
                    self.input.history_next();
                }
            }
            KeyCode::Left => self.input.left(),
            KeyCode::Right => self.input.right(),
            KeyCode::Home => self.input.home(),
            KeyCode::End => self.input.end(),
            KeyCode::Char('a') if ctrl => self.input.home(),
            KeyCode::Char('e') if ctrl => self.input.end(),
            KeyCode::Char('u') if ctrl => self.input.clear(),
            KeyCode::Char('p') if ctrl => self.modal = Some(modal::help(&self.theme)),
            KeyCode::Backspace => {
                if self.input.is_empty() && self.attachments.pop().is_some() {
                    self.dirty = true;
                } else {
                    self.edited(Input::backspace);
                }
            }
            KeyCode::Delete => self.edited(Input::delete),
            KeyCode::Char(c) if !ctrl => {
                self.input.insert(c);
                self.popup_dismissed = false;
                self.popup_selected = 0;
            }
            _ => {}
        }
    }

    /// Transcript scrolling keys, shared by the prompt and the approval bar. Returns
    /// whether `key` was one.
    pub(super) fn scroll_key(&mut self, key: KeyEvent) -> bool {
        let page = self.view.get().1.saturating_sub(2).max(1);
        let jump = key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        match key.code {
            KeyCode::PageUp => self.scroll_up(page),
            KeyCode::PageDown => self.scroll_down(page),
            KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => self.scroll_up(1),
            KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => self.scroll_down(1),
            // Drawing clamps this to the first row.
            KeyCode::Home if jump => self.scroll.set(usize::MAX),
            KeyCode::End if jump => self.scroll.set(0),
            _ => return false,
        }
        self.dirty = true;
        true
    }

    pub(super) fn scroll_up(&mut self, rows: usize) {
        let (total, height) = self.view.get();
        let max = total.saturating_sub(height);
        self.scroll.set((self.scroll.get() + rows).min(max));
        self.dirty = true;
    }

    pub(super) fn scroll_down(&mut self, rows: usize) {
        self.scroll.set(self.scroll.get().saturating_sub(rows));
        self.dirty = true;
    }

    /// Show (`Some(true)`), hide (`Some(false)`) or flip the model's reasoning. Hiding goes
    /// back to `ui.show_reasoning`, or to a one-line summary when that is `expanded`.
    pub(super) fn toggle_reasoning(&mut self, show: Option<bool>) {
        let show = show.unwrap_or(self.show_reasoning != ShowReasoning::Expanded);
        self.show_reasoning = match (show, self.info.ui.show_reasoning) {
            (true, _) => ShowReasoning::Expanded,
            (false, ShowReasoning::Expanded) => ShowReasoning::Collapsed,
            (false, configured) => configured,
        };
        // Without any reasoning on screen the change is invisible; say what happened.
        if !self
            .transcript
            .iter()
            .any(|e| matches!(e, Entry::Reasoning(_)))
        {
            let state = if show { "shown" } else { "hidden" };
            self.note(format!(
                "thinking will be {state} (Ctrl-T); nothing to show yet, \
                 and only some models send it"
            ));
        }
        self.dirty = true;
    }

    pub(super) fn on_approval_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if key.code == KeyCode::Char('l')
            && ctrl
            && matches!(
                self.info.project_status,
                ProjectStatus::Trusted | ProjectStatus::SettingsOnly
            )
            && let Some(a) = self.approval.as_mut().filter(|a| !a.always.is_empty())
        {
            a.always_scope = toggle_write_scope(a.always_scope);
            return;
        }
        let decision = match key.code {
            KeyCode::Char('r' | 'R')
                if self.approval.as_ref().is_some_and(|a| {
                    matches!(a.request, ApprovalRequest::EgressSecrets { .. })
                }) =>
            {
                ApprovalDecision::RedactAndSend
            }
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => ApprovalDecision::AllowOnce,
            KeyCode::Char('A') if self.approval.as_ref().is_some_and(|a| !a.always.is_empty()) => {
                let scope = self
                    .approval
                    .as_ref()
                    .map_or(WriteScope::User, |a| a.always_scope);
                ApprovalDecision::AlwaysAllow { scope }
            }
            KeyCode::Char('a' | 'A') => ApprovalDecision::AllowSession,
            KeyCode::Char('n' | 'N') | KeyCode::Esc => ApprovalDecision::Deny,
            _ if self.scroll_key(key) => return,
            _ => return,
        };
        if let Some(a) = self.approval.take() {
            self.ops.push(Op::ApprovalResponse {
                call_id: a.call_id,
                decision,
            });
        }
    }

    /// Keys while the model's questions are showing. The input stays live for an answer in
    /// the user's own words.
    pub(super) fn on_prompt_key(&mut self, key: KeyEvent) {
        let Some(p) = &mut self.prompt else {
            return;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => {
                let back_setup = matches!(
                    p.then,
                    PromptThen::ServerUrl
                        | PromptThen::ServerKey { .. }
                        | PromptThen::ServiceKey { .. }
                );
                let back_mcp = matches!(
                    p.then,
                    PromptThen::McpTransport
                        | PromptThen::McpHttpUrl
                        | PromptThen::McpHttpId { .. }
                        | PromptThen::McpHttpAuth { .. }
                        | PromptThen::McpStdioCmd
                        | PromptThen::McpStdioId { .. }
                );
                self.prompt = None;
                if back_setup {
                    self.open_setup();
                } else if back_mcp {
                    self.modal = Some(modal::mcp(
                        self.mcp_servers_cache.as_deref(),
                        &self.info.mcp_items,
                        self.write_scope,
                        &self.theme,
                    ));
                    self.ops.push(Op::ListMcp);
                }
            }
            KeyCode::Enter => self.answer_prompt(),
            KeyCode::Backspace => {
                p.text.pop();
            }
            KeyCode::Char('u') if ctrl => p.text.clear(),
            KeyCode::Char('l') if ctrl => {
                if let Some(s) = &mut p.scope {
                    *s = toggle_write_scope(*s);
                }
            }
            KeyCode::Char(c) if !ctrl => {
                p.text.push(c);
                p.error = None;
            }
            _ => {}
        }
    }

    /// Enter in a `/setup` prompt: check the answer, then ask the next question or send it.
    pub(super) fn answer_prompt(&mut self) {
        let Some(p) = self.prompt.take() else {
            return;
        };
        let text = p.text.trim().to_owned();
        let retry = |mut p: Prompt, error: &str| {
            p.error = Some(error.to_owned());
            Some(p)
        };
        let mcp_scope = p.scope.unwrap_or(self.write_scope);
        let scope_label = if mcp_scope == WriteScope::Project {
            "project .quack/"
        } else {
            "user config"
        };
        self.prompt = match &p.then {
            PromptThen::ServerUrl if text.is_empty() => {
                retry(p, "type the server's URL, or press esc to go back")
            }
            PromptThen::ServerUrl
                if !(text.starts_with("http://") || text.starts_with("https://")) =>
            {
                retry(p, "the URL must start with http:// or https://")
            }
            PromptThen::ServerUrl => Some(Prompt::server_key(text)),
            PromptThen::ServerKey { url } => {
                self.note(format!("adding {url}…"));
                self.setup_adding = true;
                self.ops.push(Op::AddServer {
                    base_url: url.clone(),
                    key: (!text.is_empty()).then_some(SecretText(text)),
                });
                None
            }
            PromptThen::ServiceKey { .. } if text.is_empty() => {
                retry(p, "paste the key, or press esc to go back")
            }
            PromptThen::NewKey { .. } if text.is_empty() => {
                retry(p, "paste the new key, or press esc to keep the old one")
            }
            PromptThen::NewKey { id } => {
                self.note(format!("storing the new {id} key…"));
                self.rekeying = true;
                self.ops.push(Op::SetProviderKey {
                    id: id.clone(),
                    key: SecretText(text),
                });
                None
            }
            PromptThen::ServiceKey { id } => {
                self.note(format!("storing the {id} key…"));
                self.setup_adding = true;
                self.ops.push(Op::SetProviderKey {
                    id: id.clone(),
                    key: SecretText(text),
                });
                None
            }
            PromptThen::McpTransport => match text.to_ascii_lowercase().as_str() {
                "1" | "http" | "remote" | "url" => Some(Prompt::mcp_http_url(mcp_scope)),
                "2" | "stdio" | "local" | "cmd" => Some(Prompt::mcp_stdio_cmd(mcp_scope)),
                _ => retry(p, "enter `1` (remote HTTP) or `2` (local command)"),
            },
            PromptThen::McpHttpUrl if text.is_empty() => {
                retry(p, "enter an http:// or https:// URL, or press esc")
            }
            PromptThen::McpHttpUrl
                if !(text.starts_with("http://") || text.starts_with("https://")) =>
            {
                retry(p, "the URL must start with http:// or https://")
            }
            PromptThen::McpHttpUrl => Some(Prompt::mcp_http_id(text, mcp_scope)),
            PromptThen::McpHttpId { url } => {
                let id = if text.is_empty() {
                    quack_config::derive_mcp_id_from_url(url).unwrap_or_default()
                } else {
                    text
                };
                match quack_config::check_mcp_id(&id) {
                    Ok(()) => Some(Prompt::mcp_http_auth(id, url.clone(), mcp_scope)),
                    Err(e) => retry(p, &e),
                }
            }
            PromptThen::McpHttpAuth { id, url } => {
                if let Some(env_name) = text
                    .strip_prefix("env:")
                    .or_else(|| text.strip_prefix("ENV:"))
                {
                    let var = env_name.trim().to_owned();
                    if var.is_empty() {
                        retry(p, "enter an environment variable name after `env:`")
                    } else {
                        self.note(format!("adding MCP server `{id}` ({scope_label})…"));
                        self.ops.push(Op::AddMcp {
                            id: id.clone(),
                            url: url.clone(),
                            command: String::new(),
                            args: Vec::new(),
                            bearer_env: Some(var),
                            bearer_token: None,
                            scope: Some(mcp_scope),
                        });
                        None
                    }
                } else {
                    self.note(format!("adding MCP server `{id}` ({scope_label})…"));
                    self.ops.push(Op::AddMcp {
                        id: id.clone(),
                        url: url.clone(),
                        command: String::new(),
                        args: Vec::new(),
                        bearer_env: None,
                        bearer_token: (!text.is_empty()).then_some(SecretText(text)),
                        scope: Some(mcp_scope),
                    });
                    None
                }
            }
            PromptThen::McpStdioCmd if text.is_empty() => retry(
                p,
                "enter a command line (e.g. npx -y @playwright/mcp@latest)",
            ),
            PromptThen::McpStdioCmd => match quack_config::split_command_words(&text) {
                Ok(parts) if !parts.is_empty() => {
                    let mut iter = parts.into_iter();
                    let command = iter.next().unwrap_or_default();
                    let args = iter.collect();
                    Some(Prompt::mcp_stdio_id(command, args, mcp_scope))
                }
                Ok(_) => retry(p, "enter a command line"),
                Err(e) => retry(p, &e),
            },
            PromptThen::McpStdioId { command, args } => {
                let id = if text.is_empty() {
                    quack_config::derive_mcp_id_from_command(command, args).unwrap_or_default()
                } else {
                    text
                };
                match quack_config::check_mcp_id(&id) {
                    Ok(()) => {
                        self.note(format!("adding MCP server `{id}` ({scope_label})…"));
                        self.ops.push(Op::AddMcp {
                            id,
                            url: String::new(),
                            command: command.clone(),
                            args: args.clone(),
                            bearer_env: None,
                            bearer_token: None,
                            scope: Some(mcp_scope),
                        });
                        None
                    }
                    Err(e) => retry(p, &e),
                }
            }
        };
    }

    pub(super) fn on_asking_key(&mut self, key: KeyEvent) {
        if self.scroll_key(key) {
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let typing = !self.input.is_empty();
        let Some(a) = &mut self.asking else { return };
        let options = a.question().options.len();
        match key.code {
            KeyCode::Char('s') if ctrl => {
                self.input.clear();
                self.finish_asking(true);
            }
            KeyCode::Up => a.selected = a.selected.saturating_sub(1),
            KeyCode::Down => a.selected = (a.selected + 1).min(options),
            KeyCode::Char(c @ '1'..='9') if !typing && !ctrl => {
                let i = usize::from(c as u8 - b'1');
                if i < options {
                    self.answer(Answer::Picked { index: i });
                }
            }
            KeyCode::Enter if typing => {
                let text = self.input.submit();
                self.answer(Answer::Text {
                    text: text.trim().to_owned(),
                });
            }
            KeyCode::Enter if a.selected < options => {
                let index = a.selected;
                self.answer(Answer::Picked { index });
            }
            KeyCode::Esc => {
                self.input.clear();
                self.answer(Answer::Skipped);
            }
            KeyCode::Left if !typing && a.current > 0 => {
                a.current -= 1;
                a.select_default();
            }
            KeyCode::Char(c) if !ctrl => {
                // Typing picks the own-answer row.
                a.selected = options;
                self.input.insert(c);
            }
            KeyCode::Backspace => self.input.backspace(),
            KeyCode::Delete => self.input.delete(),
            KeyCode::Left => self.input.left(),
            KeyCode::Right => self.input.right(),
            KeyCode::Home => self.input.home(),
            KeyCode::End => self.input.end(),
            _ => {}
        }
    }

    /// Answer the current question and go on to the next, or send them all.
    pub(super) fn answer(&mut self, answer: Answer) {
        let Some(a) = &mut self.asking else { return };
        a.answers[a.current] = answer;
        a.current += 1;
        if a.current < a.questions.len() {
            a.select_default();
        } else {
            self.finish_asking(false);
        }
    }

    pub(super) fn finish_asking(&mut self, stop_asking: bool) {
        if let Some(a) = self.asking.take() {
            self.ops.push(Op::AnswerQuestions {
                call_id: a.call_id,
                answers: a.answers,
                stop_asking,
            });
        }
    }

    /// Implementing preserves approvals; the review never broadens permissions.
    pub(crate) fn review_choices(&self) -> Vec<(ReviewChoice, String)> {
        vec![
            (
                ReviewChoice::Accept,
                format!("Implement plan · {} approvals", self.info.permission_mode),
            ),
            (ReviewChoice::KeepPlanning, "Keep planning".into()),
        ]
    }

    /// Keys for the plan review bar. Returns whether it used the key; typing goes on to
    /// the input, and Enter with text sends it as the revision.
    pub(super) fn on_review_key(&mut self, key: KeyEvent) -> bool {
        let Some(selected) = self.review else {
            return false;
        };
        let choices = self.review_choices();
        let typing = !self.input.is_empty();
        let pick = match key.code {
            KeyCode::Up if key.modifiers.is_empty() => {
                self.review = Some(selected.saturating_sub(1));
                return true;
            }
            KeyCode::Down if key.modifiers.is_empty() => {
                self.review = Some((selected + 1).min(choices.len() - 1));
                return true;
            }
            KeyCode::Char(c @ '1'..='9') if !typing && key.modifiers.is_empty() => {
                let i = usize::from(c as u8 - b'1');
                if i >= choices.len() {
                    return false;
                }
                i
            }
            KeyCode::Enter if !typing => selected,
            KeyCode::Esc if !typing => choices.len() - 1,
            _ => return false,
        };
        self.review = None;
        match choices[pick].0 {
            ReviewChoice::Accept => self.accept_plan(),
            ReviewChoice::KeepPlanning => {
                self.note("still planning · say what to change, or /plan off to leave".into());
            }
        }
        true
    }

    /// Keys for the step-limit bar: keep going, or stop here. Returns whether it used
    /// the key; typing goes on to the input, and submitting a message stops the turn
    /// first, so it reads as "no, do this instead".
    pub(super) fn on_step_limit_key(&mut self, key: KeyEvent) -> bool {
        let Some(s) = &self.step_limit else {
            return false;
        };
        let typing = !self.input.is_empty();
        let proceed = match key.code {
            KeyCode::Up if key.modifiers.is_empty() => {
                self.step_limit = Some(StepLimit {
                    selected: s.selected.saturating_sub(1),
                    ..s.clone()
                });
                return true;
            }
            KeyCode::Down if key.modifiers.is_empty() => {
                self.step_limit = Some(StepLimit {
                    selected: (s.selected + 1).min(1),
                    ..s.clone()
                });
                return true;
            }
            KeyCode::Char(c @ '1'..='2') if !typing && key.modifiers.is_empty() => c == '1',
            KeyCode::Enter if !typing => s.selected == 0,
            KeyCode::Esc if !typing => false,
            _ => return false,
        };
        let turn = s.turn;
        self.step_limit = None;
        self.ops.push(Op::ContinueTurn { turn, proceed });
        true
    }

    pub(super) fn edited(&mut self, f: fn(&mut Input)) {
        f(&mut self.input);
        self.popup_dismissed = false;
        self.popup_selected = 0;
    }
    pub(super) fn ctrl_c(&mut self) {
        if self.signing_in.take().is_some() {
            self.browser = None;
            self.ops.push(Op::SignIn { id: None });
            self.open_hosted_setup();
            return;
        }
        if self.modal.take().is_some() {
            return;
        }
        if self.approval.is_some() || self.asking.is_some() {
            self.approval = None;
            self.asking = None;
            self.ops.push(Op::Interrupt);
            return;
        }
        if self.step_limit.take().is_some() {
            self.ops.push(Op::Interrupt);
            return;
        }
        if self.review.take().is_some() {
            return;
        }
        if !self.input.is_empty() {
            self.input.clear();
        } else if self.turn_running || self.queued {
            self.ops.push(Op::Interrupt);
        } else if self
            .last_ctrl_c
            .is_some_and(|t| t.elapsed() < DOUBLE_CTRL_C)
        {
            self.quit = true;
        } else {
            self.last_ctrl_c = Some(Instant::now());
            self.note("press Ctrl-C again to quit".into());
        }
    }

    pub(super) fn submit(&mut self, popup_choice: Option<PopupItem>) {
        match popup_choice {
            Some(item) if item.prefix == '@' => {
                self.input.complete_at(&item.name);
                self.popup_selected = 0;
                return;
            }
            Some(PopupItem {
                command: Some(cmd), ..
            }) => {
                self.input.clear();
                return self.run_command(cmd, "");
            }
            // A skill: let the user add a request first.
            Some(item) => {
                self.input.set(&format!("/{} ", item.name));
                self.popup_dismissed = true;
                return;
            }
            None => {}
        }
        let text = self.input.submit();
        let attachments = std::mem::take(&mut self.attachments);
        if text.trim().is_empty() && attachments.is_empty() {
            return;
        }
        // A message sent while the turn is paused at its step limit stops it first,
        // so what the user typed reads as "no, do this instead".
        if let Some(s) = self.step_limit.take() {
            self.ops.push(Op::ContinueTurn {
                turn: s.turn,
                proceed: false,
            });
        }
        if !text.trim().is_empty()
            && let Some(path) = &self.info.history_path
        {
            let _ = quack_core::append_history(path, &text);
        }
        self.scroll.set(0);
        if text.starts_with('/') {
            // Commands that send a turn (skills, `/plan <task>`) take the attachments;
            // the rest leave them waiting for the next message.
            self.attachments = attachments;
            return self.execute(&text);
        }
        self.send(text, attachments);
    }

    pub(super) fn send(&mut self, text: String, attachments: Vec<Attachment>) {
        self.review = None;
        self.startup_dismissed = true;
        let shown = if text.trim().is_empty() {
            format!(
                "attached: {}",
                attachments
                    .iter()
                    .map(|a| a.path.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            text.clone()
        };
        self.transcript.push(Entry::User(shown));
        let mut items: Vec<InputItem> = attachments
            .into_iter()
            .map(|a| InputItem::Image { path: a.path })
            .collect();
        if !text.trim().is_empty() {
            items.push(InputItem::text(text));
        }
        self.ops.push(Op::UserTurn {
            items,
            overrides: TurnOverrides::default(),
        });
    }

    /// A paste that is nothing but image file paths attaches them instead of inserting
    /// text: that is what dropping a file on the terminal sends. Anything else is pasted.
    /// Only the headers are read here; the engine encodes the files when the turn is sent.
    pub(super) fn attach_pasted(&mut self, text: &str) -> bool {
        let Some(paths) = dropped_images(text) else {
            return false;
        };
        for (attachment, len) in paths {
            if len > quack_protocol::image::MAX_RAW_BYTES as u64 {
                self.note(format!(
                    "{}: the image is {} KB; the limit is {} KB, so downscale it first",
                    attachment.path,
                    len / 1024,
                    quack_protocol::image::MAX_RAW_BYTES / 1024
                ));
                continue;
            }
            self.attachments.push(attachment);
        }
        true
    }
}

/// The image files a paste names, as attachments with their byte lengths, when it names nothing
/// else. Terminals send a dropped file as its path: quoted or backslash-escaped when it
/// has spaces, or as a `file://` URL.
pub(super) fn dropped_images(text: &str) -> Option<Vec<(Attachment, u64)>> {
    let image = |p: &str| {
        let path = std::path::Path::new(p);
        let (_, width, height, len) = quack_protocol::image::probe_file(path)?;
        let attachment = Attachment {
            path: p.to_owned(),
            width,
            height,
        };
        Some((attachment, len))
    };
    let whole = text.trim();
    // One path with bare spaces, as some terminals paste it.
    if let Some(one) = image(&file_url(whole)) {
        return Some(vec![one]);
    }
    let words = split_paths(whole);
    if words.is_empty() {
        return None;
    }
    words.iter().map(|w| image(&file_url(w))).collect()
}

/// A `file://` URL as a path, with its percent escapes decoded; anything else as is.
pub(super) fn file_url(s: &str) -> String {
    let Some(rest) = s.strip_prefix("file://") else {
        return s.to_owned();
    };
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Split a paste into paths the way a shell would read dropped files: whitespace
/// separates, quotes group, and outside Windows a backslash escapes the next character.
pub(super) fn split_paths(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                started = true;
            }
            (None, '\\') if !cfg!(windows) => {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (None, c) => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}
