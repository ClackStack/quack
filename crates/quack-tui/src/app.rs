use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crossterm::event::{
    Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use quack_config::settings::{ShowReasoning, StartupPanel, UiSettings};
use quack_config::{LayerStatus, Loaded, ProjectStatus, SettingRow};
use quack_protocol::{
    Answer, ApprovalDecision, ApprovalRequest, CallId, Event, ExportFormat, HookOutcome, InputItem,
    KeySource, LoadState, ModelId, Op, PermissionMode, ProviderId, Question, ReplayItem,
    SecretText, SessionId, Subsystem, TurnId, TurnOverrides, WorkMode, WriteScope,
};
use ratatui::layout::Rect;

use crate::commands::{self, COMMANDS, Command};
use crate::fuzzy;
use crate::hook_form::{Action as HookAction, HookForm};
use crate::input::Input;
use crate::modal::{self, Modal, ModalOutcome};
use crate::select::{self, Pane, Selection};
use crate::theme::{Glyphs, Theme};

const DOUBLE_CTRL_C: Duration = Duration::from_millis(1500);
const DOUBLE_ESC: Duration = Duration::from_millis(600);
/// Spinner advances every N frames (16 ms each).
const SPIN_EVERY: u64 = 5;
/// Welcome duck advances animation every N frames (16 ms each).
const DUCK_ANIM_EVERY: u64 = 8;
/// Transcript rows one mouse-wheel notch scrolls.
const WHEEL_ROWS: usize = 3;
/// Frames (16 ms each) the status bar says a selection was copied.
const COPIED_FRAMES: u64 = 125;

mod events;
mod keys;
mod slash;
#[cfg(test)]
pub(crate) mod tests;
mod types;

#[cfg(test)]
pub(crate) use tests::info as test_info;

use slash::toggle_write_scope;
pub(crate) use types::{
    AgentItem, AgentRun, Approval, Asking, Attachment, Entry, HookRun, PlanState, PopupItem,
    Prompt, ReviewChoice, StepLimit, ToolEntry, git_branch, parse_plan, turns_left,
};
pub use types::{AppInfo, ConfigLayer};
use types::{PopupCache, PopupKey, PromptThen, append_delta};

#[derive(Debug)]
pub struct App {
    pub(crate) info: AppInfo,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) reasoning_efforts: Vec<String>,
    pub(crate) theme: Theme,
    pub(crate) glyphs: Glyphs,
    pub(crate) startup: Vec<(Subsystem, LoadState)>,
    pub(crate) startup_done: bool,
    pub(crate) transcript: Vec<Entry>,
    /// Highlighted code blocks, filled as assistant text arrives; drawing only reads it.
    pub(crate) highlights: crate::highlight::Highlights,
    pub(crate) input: Input,
    /// Images attached to the next message (pasted or dropped paths), oldest first.
    pub(crate) attachments: Vec<Attachment>,
    pub(crate) modal: Option<Modal>,
    /// Transcript rows scrolled up from the bottom. Drawing clamps it and keeps the view
    /// still while new rows arrive below, so it lives in a `Cell`.
    pub(crate) scroll: Cell<usize>,
    /// `(rows, height)` of the transcript as last drawn.
    pub(crate) view: Cell<(usize, usize)>,
    /// The input box's text area and its first visible row, as last drawn.
    pub(crate) input_view: Cell<(Rect, usize)>,
    /// The transcript's area as last drawn, scrollbar column included.
    pub(crate) transcript_area: Cell<Rect>,
    /// Status-bar work-mode pill (`edit` / `plan`), clickable to toggle without touching input.
    pub(crate) work_mode_area: Cell<Rect>,
    /// Status-bar approvals pill (`ask` / `allow` / `YOLO`), clickable to toggle without touching input.
    pub(crate) permission_area: Cell<Rect>,
    /// Rendered transcript rows per entry, so drawing only lays out new or changed text
    /// (streaming deltas, finished tools); see `ui::transcript_pieces`. Drawing only
    /// writes here, like the `Cell`s above.
    pub(crate) transcript_cache: RefCell<crate::ui::TranscriptCache>,
    /// The popup's match list, rebuilt only when the input or its sources change.
    popup_cache: RefCell<Option<PopupCache>>,
    /// Bumped whenever the popup's sources (files, skills, custom or MCP commands) change.
    popup_epoch: u64,
    /// Text being selected with the mouse, or selected and copied.
    pub(crate) selection: Option<Selection>,
    /// The frame a selection was copied on, and how many chars it had.
    copied: Option<(u64, usize)>,
    /// How reasoning shows right now: `ui.show_reasoning`, flipped by Ctrl-T.
    pub(crate) show_reasoning: ShowReasoning,
    pub(crate) popup_selected: usize,
    popup_dismissed: bool,
    pub(crate) turn_running: bool,
    pub(crate) queued: bool,
    pub(crate) approval: Option<Approval>,
    /// Questions from the model waiting for answers.
    pub(crate) asking: Option<Asking>,
    /// The step-limit bar, while a turn waits for its go-ahead.
    pub(crate) step_limit: Option<StepLimit>,
    /// The plan review bar, with its highlighted choice.
    pub(crate) review: Option<usize>,
    /// Work mode when the running turn started, so switching into Plan mode mid-turn
    /// does not turn a finishing Edit turn into a plan review.
    turn_work_mode: Option<WorkMode>,
    /// Whether the running turn is a manual `/compact` rather than a model turn.
    turn_compacting: bool,
    /// Whether the checklist changed during the running turn.
    turn_plan_updated: bool,
    /// True once a plan was accepted or carried out in Edit mode, until a Plan-mode
    /// turn writes new unfinished steps to the checklist.
    plan_stale_for_review: bool,
    /// `(used, max)` tokens from the last response.
    pub(crate) context: Option<(u32, u32)>,
    /// Where compaction starts, in tokens.
    pub(crate) compact_at: Option<u32>,
    /// Context used at the end of each turn, oldest first: how fast it grows.
    pub(crate) growth: Vec<u32>,
    /// Generation speed of the last response, tokens per second.
    pub(crate) speed: Option<f32>,
    /// Session-wide prompt cache hit percentage (`cached / prompt`, 0..=100).
    pub(crate) cache_hit_percent: Option<u8>,
    /// Skills loaded into the current session, in load order.
    pub(crate) loaded_skills: Vec<String>,
    /// Latest language server snapshot pushed by the engine.
    pub(crate) lsp_servers: Vec<quack_protocol::LspServerInfo>,
    /// Errors and warnings the language servers report.
    pub(crate) diagnostics: (u32, u32),
    /// When the running turn started, and how many tools it has called.
    pub(crate) turn_started: Option<Instant>,
    turn_tools: u32,
    /// The model's current plan, from its last `todo` call.
    pub(crate) plan: Vec<(PlanState, String)>,
    /// `/todo`: the whole plan above the input instead of its one-line summary.
    pub(crate) plan_expanded: bool,
    /// Sub-agent runs in this session, oldest first (`/tasks`).
    pub(crate) agents: Vec<AgentRun>,
    /// The user started working (sent a turn, resumed a session, accepted a plan), so the
    /// startup diagnostics step aside: the sidebar returns to session details and the
    /// panel in the transcript area goes.
    startup_dismissed: bool,
    /// A `/resume` is in flight: clear the transcript when it lands.
    resuming: bool,
    /// The mode `/yolo` turns back to.
    mode_before_yolo: String,
    /// `/sandbox off`: shell commands run without the OS sandbox this session.
    pub(crate) sandbox_off: bool,
    /// Text for the terminal's clipboard (OSC 52), written by the event loop.
    clipboard: Option<String>,
    browser: Option<String>,
    /// The session title, for the terminal window and the sidebar.
    pub(crate) session_title: Option<String>,
    /// Current engine session id, for the resume hint on exit.
    session_id: Option<String>,
    /// True once the current session exists on disk (resumed or at least one turn started).
    session_saved: bool,
    /// A `/new` session switch is in flight; clear `session_saved` when `SessionConfigured` lands.
    starting_new: bool,
    /// Installed skills, runnable as `/<name>`.
    skills: Vec<quack_protocol::SkillInfo>,
    /// Custom slash commands, minus any that a built-in command's name hides.
    custom: Vec<quack_protocol::CommandInfo>,
    /// Prompts of the ready MCP servers, runnable as `/<server>:<prompt>`.
    mcp_prompts: Vec<quack_protocol::CommandInfo>,
    /// Workspace relative file paths for `@file` completion.
    files: Vec<String>,
    /// Hook runs recorded this session (`/hooks`).
    pub(crate) hook_runs: Vec<HookRun>,
    pub(crate) hook_form: Option<HookForm>,
    hook_request: u64,
    hook_inspect: Option<u64>,
    hook_reapply: Option<Vec<quack_protocol::HookConnectionDraft>>,
    hook_test_after_inspect: bool,
    /// What `/setup` has found so far.
    setup: modal::SetupView,
    /// A question from `/setup`, drawn over everything else.
    pub(crate) prompt: Option<Prompt>,
    /// `/setup` sent a key or URL and waits for the provider to be added.
    setup_adding: bool,
    /// `/key` sent a new key and waits for it to be stored.
    rekeying: bool,
    signing_in: Option<String>,
    /// Where each provider's key is read from, as last listed.
    key_sources: BTreeMap<String, KeySource>,
    /// The model catalog as last listed, to know a chosen model's thinking levels.
    catalog: Vec<quack_protocol::ProviderModels>,
    catalog_loaded: bool,
    provider_connections: Option<Vec<quack_protocol::ProviderConnection>>,
    /// Last profile list returned by the engine, so `^l` can rebuild `/profiles` in place.
    profiles_cache: Option<(String, bool, Vec<quack_protocol::ProfileInfo>)>,
    /// Last MCP server list returned by the engine, so `^l` can rebuild `/mcp` in place.
    mcp_servers_cache: Option<Vec<quack_protocol::McpServerInfo>>,
    /// Active target layer when saving settings/MCP in the TUI (`^l` / `/scope` toggles).
    pub(crate) write_scope: WriteScope,
    /// Model selected for `^s` in `/models`, so `^l` and `esc` in the role picker know it.
    default_picker_model: Option<String>,
    /// Provider whose connection picker is open, so `esc` returns to `/providers` on it.
    connection_picker_provider: Option<String>,
    /// A `/models <provider>/<model>` selection whose thinking-level picker is still owed
    /// once the engine confirms the switch.
    pick_thinking: Option<String>,
    /// `/setup` opened on its own once this run, because no model was ready.
    setup_offered: bool,
    /// The plan-mode explanation note has already been shown once this run.
    plan_note_shown: bool,
    /// A `.quack/` waiting for a trust decision.
    trust: Option<modal::TrustPrompt>,
    frame: u64,
    dirty: bool,
    quit: bool,
    gui_handoff: bool,
    ops: Vec<Op>,
    last_ctrl_c: Option<Instant>,
    /// For Esc Esc, which opens the rewind picker.
    last_esc: Option<Instant>,
}

/// Which surface owns the next key (`App::focus` computes it, `App::on_key` dispatches on
/// it). The order is the priority: a key goes to the topmost surface that is showing,
/// with the two bars (`StepLimit`, `Review`) letting unused keys fall through to `Base`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// The hook-setup form, which reads every key and turns Ctrl-C into Esc.
    HookForm,
    /// A tool-approval request.
    Approval,
    /// The engine's questions, with the input live for an answer in the user's own words.
    Asking,
    /// The step-limit bar.
    StepLimit,
    /// A one-line prompt from `/setup`.
    Prompt,
    /// An open modal (picker) dialog.
    Modal,
    /// The plan-review bar.
    Review,
    /// The transcript and input box.
    Base,
}

impl App {
    /// Build all UI state from startup facts, ready for the initial dirty draw.
    pub fn new(info: AppInfo) -> Self {
        let input = Input::with_history(info.history.clone());
        Self {
            reasoning_effort: None,
            reasoning_efforts: Vec::new(),
            theme: Theme::resolve(&info.ui.theme, info.no_color),
            glyphs: Glyphs::new(info.ui.ascii),
            startup: Subsystem::ALL
                .iter()
                .map(|s| (*s, LoadState::Pending))
                .collect(),
            startup_done: false,
            transcript: Vec::new(),
            highlights: crate::highlight::Highlights::new(std::sync::Arc::new(
                quack_core::grammar::Grammars::new(info.grammar_dirs.clone()),
            )),
            input,
            attachments: Vec::new(),
            modal: None,
            scroll: Cell::new(0),
            view: Cell::new((0, 0)),
            input_view: Cell::new((Rect::default(), 0)),
            transcript_area: Cell::new(Rect::default()),
            work_mode_area: Cell::new(Rect::default()),
            permission_area: Cell::new(Rect::default()),
            transcript_cache: RefCell::new(crate::ui::TranscriptCache::default()),
            popup_cache: RefCell::new(None),
            popup_epoch: 0,
            selection: None,
            copied: None,
            show_reasoning: info.ui.show_reasoning,
            popup_selected: 0,
            popup_dismissed: false,
            turn_running: false,
            queued: false,
            approval: None,
            asking: None,
            step_limit: None,
            review: None,
            turn_work_mode: None,
            turn_compacting: false,
            turn_plan_updated: false,
            plan_stale_for_review: false,
            context: None,
            compact_at: None,
            growth: Vec::new(),
            speed: None,
            cache_hit_percent: None,
            loaded_skills: Vec::new(),
            lsp_servers: Vec::new(),
            diagnostics: (0, 0),
            turn_started: None,
            turn_tools: 0,
            plan: Vec::new(),
            plan_expanded: false,
            agents: Vec::new(),
            startup_dismissed: false,
            resuming: false,
            mode_before_yolo: String::new(),
            sandbox_off: false,
            clipboard: None,
            browser: None,
            session_title: None,
            session_id: None,
            session_saved: false,
            starting_new: false,
            skills: Vec::new(),
            custom: Vec::new(),
            mcp_prompts: Vec::new(),
            files: Vec::new(),
            hook_runs: Vec::new(),
            hook_form: None,
            hook_request: 0,
            hook_inspect: None,
            hook_reapply: None,
            hook_test_after_inspect: false,
            setup: modal::SetupView::default(),
            prompt: None,
            setup_adding: false,
            rekeying: false,
            signing_in: None,
            key_sources: BTreeMap::new(),
            catalog: Vec::new(),
            catalog_loaded: false,
            provider_connections: None,
            profiles_cache: None,
            mcp_servers_cache: None,
            write_scope: WriteScope::User,
            default_picker_model: None,
            connection_picker_provider: None,
            pick_thinking: None,
            setup_offered: false,
            plan_note_shown: false,
            trust: None,
            frame: 0,
            dirty: true,
            quit: false,
            gui_handoff: false,
            ops: Vec::new(),
            last_ctrl_c: None,
            last_esc: None,
            info,
        }
    }

    /// Whether the user asked the event loop to exit.
    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// Whether exit should hand this session to the desktop GUI instead.
    pub fn wants_gui_handoff(&self) -> bool {
        self.gui_handoff
    }

    fn request_gui_handoff(&mut self, gui_found: bool) {
        if gui_found {
            self.gui_handoff = true;
            self.quit = true;
        } else {
            self.note(
                "quack-gui binary was not found in PATH or beside quack (build with: cargo build -p quack-gui --features desktop)"
                    .into(),
            );
        }
    }

    /// Short id of the current session if it has been saved on disk, for a resume hint on exit.
    pub fn resume_id(&self) -> Option<String> {
        self.session_saved
            .then_some(self.session_id.as_deref())
            .flatten()
            .filter(|id| !id.is_empty())
            .map(|id| id.chars().take(12).collect())
    }

    /// Return the redraw flag and clear it; later input or events set it again.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Drain a URL waiting to be opened in the browser, opening it only once.
    pub fn take_browser(&mut self) -> Option<String> {
        self.browser.take()
    }

    /// Drain text waiting for the system clipboard, copying it only once.
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clipboard.take()
    }

    /// Git metadata is read off-thread by the event loop, including external checkouts.
    pub(crate) fn set_branch(&mut self, branch: Option<String>) {
        if self.info.branch != branch {
            self.info.branch = branch;
            self.dirty = true;
        }
    }

    /// Chars in the selection just copied, while the status bar says so.
    pub(crate) fn copied(&self) -> Option<usize> {
        self.copied
            .filter(|(at, _)| self.frame.wrapping_sub(*at) < COPIED_FRAMES)
            .map(|(_, n)| n)
    }

    /// The terminal window's title: the duck from the reply marker, then the session title
    /// (`quack` until there is one), marked while a turn runs and when something waits for
    /// the user. None with `ui.terminal_title` off.
    pub fn window_title(&self) -> Option<String> {
        if !self.info.ui.terminal_title {
            return None;
        }
        let g = &self.glyphs;
        let state = if self.approval.is_some() || self.asking.is_some() || self.step_limit.is_some()
        {
            format!("{} ", g.attention)
        } else if self.turn_running {
            format!("{} ", g.think)
        } else {
            String::new()
        };
        let title = self.session_title.as_deref().unwrap_or("quack");
        Some(format!("{state}{} {title}", g.assistant))
    }

    /// Drain engine operations accumulated by UI actions, in submission order.
    pub fn take_ops(&mut self) -> Vec<Op> {
        std::mem::take(&mut self.ops)
    }

    pub(crate) fn spinner(&self) -> &'static str {
        if self.info.ui.reduced_motion {
            return self.glyphs.pending;
        }
        let frames = self.glyphs.spinner;
        frames[(self.frame / SPIN_EVERY) as usize % frames.len()]
    }

    pub(crate) fn frame(&self) -> u64 {
        self.frame
    }

    pub(crate) fn startup_issues(&self) -> usize {
        self.startup
            .iter()
            .filter(|(_, s)| {
                matches!(
                    s,
                    LoadState::Failed { .. } | LoadState::NeedsAttention { .. }
                )
            })
            .count()
    }

    /// The sidebar's startup section: shown from the first frame until the user starts
    /// working, so the diagnostics never flash in and out. While it is up, the panel in
    /// the transcript area stays away.
    pub(crate) fn startup_status_visible(&self) -> bool {
        match self.info.ui.startup_panel {
            StartupPanel::Always => true,
            StartupPanel::Never => false,
            StartupPanel::Auto => !self.startup_dismissed,
        }
    }

    /// The panel above the transcript: narrow terminals have no sidebar, so only real
    /// problems earn its rows. It never appears just because startup is slow, and it
    /// stays until the user starts working.
    pub(crate) fn panel_visible(&self) -> bool {
        match self.info.ui.startup_panel {
            StartupPanel::Always => true,
            StartupPanel::Never => false,
            StartupPanel::Auto => !self.startup_dismissed && self.startup_issues() > 0,
        }
    }

    /// Workspace relative file paths for `@file` completion, loaded off-thread.
    pub fn set_files(&mut self, files: Vec<String>) {
        self.files = files;
        self.popup_epoch = self.popup_epoch.wrapping_add(1);
        if self.input.at_mention().is_some() {
            self.dirty = true;
        }
    }

    pub(crate) fn popup_query(&self) -> &str {
        let text = self.input.text();
        if text.starts_with('/') && !text.contains(char::is_whitespace) {
            return text.get(1..).unwrap_or_default();
        }
        self.input.at_mention().map_or("", |(_, _, q)| q)
    }

    /// Commands matching the input while it is a bare `/word`, or workspace files matching
    /// an `@file` token at the cursor.
    pub(crate) fn popup_matches(&self) -> Vec<PopupItem> {
        // Both the draw path and every key press ask for the list; only recompute it when
        // the input text or one of its sources changed since the last time.
        let key = PopupKey {
            text: self.input.text().to_owned(),
            modal_open: self.modal.is_some(),
            dismissed: self.popup_dismissed,
            epoch: self.popup_epoch,
        };
        if let Some(cache) = self.popup_cache.borrow().as_ref()
            && cache.key == key
        {
            return cache.items.clone();
        }
        let items = self.compute_popup_matches();
        *self.popup_cache.borrow_mut() = Some(PopupCache {
            key,
            items: items.clone(),
        });
        items
    }

    fn compute_popup_matches(&self) -> Vec<PopupItem> {
        let text = self.input.text();
        if self.modal.is_some() || self.popup_dismissed {
            return Vec::new();
        }
        if !text.starts_with('/') || text.contains(char::is_whitespace) {
            let Some((_, _, query)) = self.input.at_mention() else {
                return Vec::new();
            };
            return fuzzy::filter(query, self.files.iter().map(String::as_str))
                .into_iter()
                .take(50)
                .map(|i| PopupItem {
                    prefix: '@',
                    name: self.files[i].clone(),
                    args: String::new(),
                    description: String::new(),
                    hint: Some("file"),
                    command: None,
                })
                .collect();
        }
        let query = &text[1..];
        let mut hits: Vec<&Command> = fuzzy::filter(query, COMMANDS.iter().map(|c| c.name))
            .into_iter()
            .map(|i| &COMMANDS[i])
            .collect();
        if !query.is_empty() {
            let mut insert_at = hits
                .iter()
                .rposition(|c| c.name.starts_with(query))
                .map_or(0, |i| i + 1);
            for cmd in COMMANDS {
                if !cmd.aliases.iter().any(|a| a.starts_with(query)) {
                    continue;
                }
                if let Some(pos) = hits.iter().position(|h| h.name == cmd.name) {
                    if pos >= insert_at {
                        hits.remove(pos);
                        hits.insert(insert_at, cmd);
                        insert_at += 1;
                    }
                } else {
                    hits.insert(insert_at, cmd);
                    insert_at += 1;
                }
            }
        }
        if let Some(alias) = COMMANDS.iter().find(|c| c.aliases.contains(&query)) {
            hits.retain(|c| c.name != alias.name);
            hits.insert(0, alias);
        }
        let mut items: Vec<PopupItem> = hits.into_iter().map(PopupItem::command).collect();
        items.extend(
            fuzzy::filter(query, self.custom.iter().map(|c| c.name.as_str()))
                .into_iter()
                .map(|i| PopupItem {
                    prefix: '/',
                    name: self.custom[i].name.clone(),
                    args: self.custom[i].args.clone(),
                    description: self.custom[i].description.clone(),
                    hint: Some("custom"),
                    command: None,
                }),
        );
        items.extend(
            fuzzy::filter(query, self.mcp_prompts.iter().map(|c| c.name.as_str()))
                .into_iter()
                .map(|i| PopupItem {
                    prefix: '/',
                    name: self.mcp_prompts[i].name.clone(),
                    args: self.mcp_prompts[i].args.clone(),
                    description: self.mcp_prompts[i].description.clone(),
                    hint: Some("mcp"),
                    command: None,
                }),
        );
        items.extend(
            fuzzy::filter(query, self.skills.iter().map(|s| s.name.as_str()))
                .into_iter()
                .map(|i| PopupItem {
                    prefix: '/',
                    name: self.skills[i].name.clone(),
                    args: "[request]".into(),
                    description: self.skills[i].description.clone(),
                    hint: Some("skill"),
                    command: None,
                }),
        );
        items
    }

    /// Advance frame-driven animation and mark a redraw only when something visible changes.
    pub fn on_tick(&mut self) {
        self.frame = self.frame.wrapping_add(1);
        let animating = self.turn_running
            || self.queued
            || self
                .startup
                .iter()
                .any(|(_, s)| matches!(s, LoadState::Running { .. }))
            || self.lsp_servers.iter().any(|s| {
                matches!(
                    s.state,
                    quack_protocol::LspServerState::Starting
                        | quack_protocol::LspServerState::Indexing
                )
            });
        if animating && !self.info.ui.reduced_motion && self.frame.is_multiple_of(SPIN_EVERY) {
            self.dirty = true;
        }
        if self
            .copied
            .is_some_and(|(at, _)| self.frame.wrapping_sub(at) == COPIED_FRAMES)
        {
            self.dirty = true;
        }
        // The welcome duck animates on startup while the transcript is empty.
        if self.transcript.is_empty()
            && !self.info.ui.reduced_motion
            && self.frame.is_multiple_of(DUCK_ANIM_EVERY)
        {
            self.dirty = true;
        }
        // The elapsed-time clock in the status bar.
        if self.turn_running && self.frame.is_multiple_of(60) {
            self.dirty = true;
        }
    }

    /// Whether the terminal should capture the mouse (`ui.mouse`), for wheel scrolling and input clicks.
    pub fn wants_mouse(&self) -> bool {
        self.info.ui.mouse
    }

    fn tool_mut(&mut self, id: &CallId) -> Option<&mut ToolEntry> {
        self.transcript.iter_mut().rev().find_map(|e| match e {
            Entry::Tool(t) if &t.call_id == id => Some(t.as_mut()),
            _ => None,
        })
    }

    pub(crate) fn thinking_label(&self) -> Option<String> {
        self.info.model.as_ref()?;
        match self.reasoning_effort.as_deref() {
            Some(effort) => Some(format!("{effort} thinking")),
            None if !self.reasoning_efforts.is_empty() => Some("default thinking".into()),
            None => None,
        }
    }

    /// Settings from a reloaded config. What the engine reports (model, profile, mode)
    /// stays as it is until the engine says otherwise.
    pub fn reconfigure(&mut self, info: AppInfo) {
        if info.ui.sidebar != self.info.ui.sidebar {
            self.selection = None;
        }
        if info.ui.show_reasoning != self.info.ui.show_reasoning {
            self.show_reasoning = info.ui.show_reasoning;
        }
        let keep = (
            self.info.model.take(),
            std::mem::take(&mut self.info.profile),
            std::mem::take(&mut self.info.permission_mode),
            self.info.work_mode,
        );
        self.info = info;
        (
            self.info.model,
            self.info.profile,
            self.info.permission_mode,
            self.info.work_mode,
        ) = keep;
        self.theme = Theme::resolve(&self.info.ui.theme, self.info.no_color);
        self.glyphs = Glyphs::new(self.info.ui.ascii);
        // Query overrides may have changed.
        self.highlights
            .reset(std::sync::Arc::new(quack_core::grammar::Grammars::new(
                self.info.grammar_dirs.clone(),
            )));
        self.highlight_code(true);
        self.dirty = true;
    }

    fn note(&mut self, text: String) {
        self.transcript.push(Entry::System(text));
    }

    pub(crate) fn model_roles(&self) -> modal::ModelRoles<'_> {
        modal::ModelRoles {
            current: self.info.model.as_deref(),
            default: self.info.default_model.as_deref(),
            small: self.info.small_model.as_deref(),
            default_locked: self.info.default_locked,
            small_locked: self.info.small_locked,
            project_status: self.info.project_status,
        }
    }
}
