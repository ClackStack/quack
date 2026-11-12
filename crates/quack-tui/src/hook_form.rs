//! A keyboard-driven editor for bundle-defined hook connections.

use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use quack_protocol::{
    HookConnection, HookConnectionDraft, HookCredentialChange, HookField, HookSetupInfo, SecretText,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::Line,
    widgets::{Block, Clear, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::theme::{Glyphs, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Connections,
    Presets,
    Fields,
    Input,
    Sample,
    Discard,
    Remove,
    Help,
}

pub(crate) enum Action {
    None,
    Close,
    Reload,
    ReloadDraft,
    Save {
        enable: Option<bool>,
        replace_payload: Option<bool>,
    },
    Check {
        connection: HookConnectionDraft,
        url: String,
    },
    Cancel(u64),
}

pub(crate) struct HookForm {
    pub info: HookSetupInfo,
    pub drafts: Vec<HookConnectionDraft>,
    page: Page,
    selected: usize,
    field: usize,
    preset: usize,
    buffer: String,
    source_input: bool,
    advanced: bool,
    dirty: bool,
    pub pending: Option<u64>,
    checking: bool,
    pub status: String,
    replace_payload: Option<bool>,
    help_scroll: u16,
}

impl std::fmt::Debug for HookForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookForm")
            .field("id", &self.info.id)
            .finish_non_exhaustive()
    }
}

impl HookForm {
    pub fn new(info: HookSetupInfo) -> Self {
        let drafts = info
            .connections
            .iter()
            .map(|c| HookConnectionDraft {
                connection: c.connection.clone(),
                credentials: BTreeMap::new(),
            })
            .collect();
        let status = info.problem.clone().unwrap_or_else(|| {
            "Saved settings. Access has not been checked in this session.".into()
        });
        Self {
            info,
            drafts,
            page: Page::Connections,
            selected: 0,
            field: 0,
            preset: 0,
            buffer: String::new(),
            source_input: false,
            advanced: false,
            dirty: false,
            pending: None,
            checking: false,
            status,
            replace_payload: None,
            help_scroll: 0,
        }
    }

    fn fields(&self) -> Vec<&HookField> {
        self.info
            .descriptor
            .fields
            .iter()
            .filter(|f| self.advanced || !f.advanced)
            .collect()
    }

    fn current_field(&self) -> Option<&HookField> {
        self.fields().get(self.field).copied()
    }

    pub fn waiting(&mut self, request: u64, checking: bool) {
        self.pending = Some(request);
        self.checking = checking;
        self.status = if checking {
            "Checking access… Escape cancels the request."
        } else {
            "Saving… Waiting for the current turn to finish if necessary."
        }
        .into();
    }

    pub fn result(&mut self, status: &str, message: String) {
        self.pending = None;
        self.checking = false;
        self.status = format!("{status}: {message}");
    }

    pub fn paste(&mut self, value: &str) {
        if matches!(self.page, Page::Input | Page::Sample) {
            self.buffer
                .push_str(value.lines().next().unwrap_or_default().trim());
        }
    }

    pub fn restore_drafts(&mut self, drafts: Vec<HookConnectionDraft>) {
        self.drafts = drafts;
        self.dirty = true;
        self.status =
            "Saved configuration reloaded. Your draft was reapplied; review it before saving."
                .into();
    }

    pub fn key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if let Some(request) = self.pending {
            if key.code == KeyCode::Esc && self.checking {
                self.pending = None;
                self.status = "Access check cancelled; settings were not changed.".into();
                return Action::Cancel(request);
            }
            return Action::None;
        }
        if matches!(self.page, Page::Input | Page::Sample) {
            return self.input_key(key);
        }
        if self.page == Page::Discard {
            match key.code {
                KeyCode::Char('d') => return Action::Close,
                KeyCode::Char('r') => return Action::Reload,
                KeyCode::Esc | KeyCode::Enter => self.page = Page::Connections,
                _ => {}
            }
            return Action::None;
        }
        if self.page == Page::Remove {
            match key.code {
                KeyCode::Char('d') => {
                    self.drafts.remove(self.selected);
                    self.selected = self.selected.min(self.drafts.len().saturating_sub(1));
                    self.changed();
                    self.page = Page::Connections;
                }
                KeyCode::Esc | KeyCode::Enter => self.page = Page::Connections,
                _ => {}
            }
            return Action::None;
        }
        if self.page == Page::Help {
            match key.code {
                KeyCode::Esc => self.page = Page::Fields,
                KeyCode::Down | KeyCode::PageDown => {
                    self.help_scroll = self.help_scroll.saturating_add(1)
                }
                KeyCode::Up | KeyCode::PageUp => {
                    self.help_scroll = self.help_scroll.saturating_sub(1)
                }
                KeyCode::Home => self.help_scroll = 0,
                _ => {}
            }
            return Action::None;
        }
        if ctrl {
            match key.code {
                KeyCode::Char('s') if self.info.writable => {
                    return Action::Save {
                        enable: None,
                        replace_payload: self.replace_payload,
                    };
                }
                KeyCode::Char('e') if self.info.writable => {
                    return Action::Save {
                        enable: Some(true),
                        replace_payload: self.replace_payload,
                    };
                }
                KeyCode::Char('t') if self.selected < self.drafts.len() => {
                    self.page = Page::Sample;
                    self.buffer.clear();
                }
                KeyCode::Char('a') if self.info.writable => {
                    self.page = Page::Presets;
                    self.preset = 0;
                }
                KeyCode::Char('d') if self.info.writable && self.selected < self.drafts.len() => {
                    self.drafts[self.selected].connection.enabled =
                        !self.drafts[self.selected].connection.enabled;
                    self.changed();
                }
                KeyCode::Char('r')
                    if self.page == Page::Fields
                        && self.current_field().is_some_and(|f| f.kind == "secret") =>
                {
                    if let Some(field) = self.current_field().map(|f| f.id.clone()) {
                        self.drafts[self.selected]
                            .credentials
                            .insert(field, HookCredentialChange::Remove);
                        self.changed();
                    }
                }
                KeyCode::Char('x')
                    if self.page == Page::Fields
                        && self.current_field().is_some_and(|f| f.kind == "secret") =>
                {
                    self.source_input = true;
                    self.buffer = "env:".into();
                    self.page = Page::Input;
                }
                _ => {}
            }
            return Action::None;
        }
        match key.code {
            KeyCode::Esc if self.page == Page::Connections => {
                if self.dirty {
                    self.page = Page::Discard;
                } else {
                    return Action::Close;
                }
            }
            KeyCode::Esc => self.page = Page::Connections,
            KeyCode::Char('r') if self.page == Page::Connections => {
                if self.dirty {
                    self.page = Page::Discard;
                } else {
                    return Action::Reload;
                }
            }
            KeyCode::Char('R') if self.page == Page::Connections => return Action::ReloadDraft,
            KeyCode::Char('a') if self.page == Page::Connections && self.info.writable => {
                self.page = Page::Presets
            }
            KeyCode::Char('k') if self.page == Page::Connections => {
                self.replace_payload = Some(false);
                self.status = "Keep existing payload selected for the next save.".into();
            }
            KeyCode::Char('p') if self.page == Page::Connections => {
                self.replace_payload = Some(true);
                self.status = "Replace existing payload selected for the next save.".into();
            }
            KeyCode::Delete
                if self.page == Page::Connections
                    && self.selected < self.drafts.len()
                    && self.info.writable =>
            {
                self.page = Page::Remove
            }
            KeyCode::Char('v') if self.page == Page::Fields => {
                self.advanced = !self.advanced;
                self.field = 0;
            }
            KeyCode::Char('?') if self.page == Page::Fields => {
                self.page = Page::Help;
                self.help_scroll = 0;
            }
            KeyCode::Up | KeyCode::BackTab => self.move_selection(-1),
            KeyCode::Down | KeyCode::Tab => self.move_selection(1),
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
                if self.page == Page::Fields && self.info.writable =>
            {
                self.cycle(key.code != KeyCode::Left)
            }
            KeyCode::Enter => match self.page {
                Page::Connections if self.selected < self.drafts.len() => {
                    self.page = Page::Fields;
                    self.field = 0;
                }
                Page::Presets if self.info.writable => self.add_connection(),
                Page::Fields if self.info.writable => {
                    if let Some(field) = self.current_field() {
                        self.buffer = if field.kind == "secret" {
                            String::new()
                        } else {
                            self.drafts[self.selected]
                                .connection
                                .values
                                .get(&field.id)
                                .cloned()
                                .unwrap_or_default()
                        };
                        self.source_input = false;
                        self.page = Page::Input;
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Action::None
    }

    fn move_selection(&mut self, direction: isize) {
        let (current, length) = match self.page {
            Page::Connections => (&mut self.selected, self.drafts.len()),
            Page::Presets => (&mut self.preset, self.info.descriptor.presets.len()),
            Page::Fields => {
                let len = self.fields().len();
                (&mut self.field, len)
            }
            _ => return,
        };
        if length > 0 {
            *current = current.saturating_add_signed(direction).min(length - 1);
        }
    }

    fn add_connection(&mut self) {
        let Some(preset) = self.info.descriptor.presets.get(self.preset) else {
            return;
        };
        let mut values: BTreeMap<String, String> = self
            .info
            .descriptor
            .fields
            .iter()
            .filter(|f| f.kind != "secret")
            .map(|f| (f.id.clone(), f.default.clone()))
            .collect();
        values.extend(preset.values.clone());
        values.insert("label".into(), preset.label.clone());
        let mut number = 1;
        while self
            .drafts
            .iter()
            .any(|d| d.connection.id == format!("connection-{number}"))
            || self
                .info
                .connections
                .iter()
                .any(|c| c.connection.id == format!("connection-{number}"))
        {
            number += 1;
        }
        self.drafts.push(HookConnectionDraft {
            connection: HookConnection {
                id: format!("connection-{number}"),
                preset: preset.id.clone(),
                enabled: true,
                values,
            },
            credentials: BTreeMap::new(),
        });
        self.selected = self.drafts.len() - 1;
        self.derive_fields();
        self.page = Page::Fields;
        self.field = 0;
        self.changed();
    }

    fn derive_fields(&mut self) {
        let conn = &mut self.drafts[self.selected].connection;
        if let Some(preset) = self
            .info
            .descriptor
            .presets
            .iter()
            .find(|p| p.id == conn.preset)
        {
            for (field, template) in &preset.derived {
                let mut value = template.clone();
                for (key, setting) in &conn.values {
                    value = value.replace(&format!("{{{key}}}"), setting.trim_end_matches('/'));
                }
                conn.values.insert(field.clone(), value);
            }
        }
    }

    fn changed(&mut self) {
        self.dirty = true;
        self.status = "Unsaved changes. Previous access verification is stale.".into();
    }

    fn cycle(&mut self, forwards: bool) {
        let Some(field) = self.current_field().cloned() else {
            return;
        };
        let conn = &mut self.drafts[self.selected].connection;
        let choices = self
            .info
            .descriptor
            .presets
            .iter()
            .find(|p| p.id == conn.preset)
            .and_then(|p| p.choices.get(&field.id))
            .unwrap_or(&field.choices);
        let choices = if field.kind == "boolean" {
            vec!["false".to_owned(), "true".to_owned()]
        } else {
            choices.clone()
        };
        if choices.is_empty() {
            return;
        }
        let at = choices
            .iter()
            .position(|v| conn.values.get(&field.id) == Some(v))
            .unwrap_or(0);
        let next = if forwards {
            (at + 1) % choices.len()
        } else {
            (at + choices.len() - 1) % choices.len()
        };
        conn.values.insert(field.id, choices[next].clone());
        self.changed();
    }

    fn input_key(&mut self, key: KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Tab | KeyCode::BackTab if self.page == Page::Input => {
                let action = self.input_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                if self.page == Page::Fields {
                    self.move_selection(if key.code == KeyCode::Tab { 1 } else { -1 });
                }
                return action;
            }
            KeyCode::Esc => {
                self.buffer.clear();
                self.page = if self.page == Page::Sample {
                    Page::Connections
                } else {
                    Page::Fields
                };
            }
            KeyCode::Backspace => {
                self.buffer.pop();
            }
            KeyCode::Char('u') if ctrl => self.buffer.clear(),
            KeyCode::Char(c) if !ctrl => self.buffer.push(c),
            KeyCode::Enter if self.page == Page::Sample => {
                let url = std::mem::take(&mut self.buffer);
                self.page = Page::Connections;
                return Action::Check {
                    connection: self.drafts[self.selected].clone(),
                    url,
                };
            }
            KeyCode::Enter => {
                let Some(field) = self.current_field().cloned() else {
                    return Action::None;
                };
                let text = std::mem::take(&mut self.buffer);
                if field.kind == "secret" {
                    let change = if self.source_input {
                        let Some((source, location)) = text.split_once(':') else {
                            self.status = "Use env:VARIABLE, file:PATH, keyring:NAME, or cmd:[\"program\",\"argument\"].".into();
                            self.buffer = text;
                            return Action::None;
                        };
                        HookCredentialChange::External {
                            source: source.into(),
                            location: SecretText(location.into()),
                        }
                    } else if text.is_empty() {
                        HookCredentialChange::Keep
                    } else {
                        HookCredentialChange::Replace {
                            value: SecretText(text),
                        }
                    };
                    self.drafts[self.selected]
                        .credentials
                        .insert(field.id, change);
                } else {
                    let derive = field.credential_scope
                        && self
                            .info
                            .descriptor
                            .presets
                            .iter()
                            .find(|p| p.id == self.drafts[self.selected].connection.preset)
                            .is_some_and(|p| {
                                p.derived
                                    .values()
                                    .any(|v| v.contains(&format!("{{{}}}", field.id)))
                            });
                    self.drafts[self.selected]
                        .connection
                        .values
                        .insert(field.id, text.trim().into());
                    if derive {
                        self.derive_fields();
                    }
                }
                self.changed();
                self.page = Page::Fields;
            }
            _ => {}
        }
        Action::None
    }

    fn shown(&self, field: &HookField) -> String {
        let draft = &self.drafts[self.selected];
        if field.kind != "secret" {
            return draft
                .connection
                .values
                .get(&field.id)
                .cloned()
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "(empty)".into());
        }
        match draft
            .credentials
            .get(&field.id)
            .unwrap_or(&HookCredentialChange::Keep)
        {
            HookCredentialChange::Remove => "Remove on save".into(),
            HookCredentialChange::Replace { .. } => "New credential entered (masked)".into(),
            HookCredentialChange::External { source, .. } => format!("Use {source} source on save"),
            HookCredentialChange::Keep => self
                .info
                .connections
                .iter()
                .find(|c| c.connection.id == draft.connection.id)
                .and_then(|c| c.credentials.get(&field.id))
                .map(|source| match source {
                    quack_protocol::KeySource::Keyring { name } => {
                        format!("OS credential store: {name}")
                    }
                    quack_protocol::KeySource::File { path } => format!("File: {path}"),
                    quack_protocol::KeySource::Env { var } => {
                        format!("Environment variable: {var}")
                    }
                    quack_protocol::KeySource::Cmd => "Read from a command".into(),
                    quack_protocol::KeySource::Value => "Stored in hook file".into(),
                })
                .unwrap_or_else(|| "No credential configured".into()),
        }
    }

    pub fn draw(&self, frame: &mut Frame<'_>, theme: &Theme, glyphs: &Glyphs, ascii: bool) {
        let screen = frame.area();
        let width = screen.width.min(86);
        let area = Rect {
            x: screen.x + (screen.width - width) / 2,
            width,
            ..screen
        };
        frame.render_widget(Clear, area);
        let block = Block::bordered()
            .border_set(glyphs.border)
            .border_style(theme.accent)
            .title(format!(" Configure {} ", self.info.id));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let [body, status, footer] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(3),
            Constraint::Length(4),
        ])
        .areas(inner);
        let mut lines = Vec::new();
        let mut cursor = None;
        let name = |d: &HookConnectionDraft| {
            d.connection
                .values
                .get("label")
                .cloned()
                .unwrap_or_else(|| d.connection.id.clone())
        };
        match self.page {
            Page::Connections => {
                lines.push(Line::styled(
                    format!(
                        "{} · {} connection(s)",
                        if self.info.enabled {
                            "Enabled"
                        } else {
                            "Disabled"
                        },
                        self.drafts.len()
                    ),
                    theme.accent,
                ));
                lines.push(Line::raw(
                    "Enter edits a connection. A adds another service.",
                ));
                if self.drafts.is_empty() {
                    lines.push(Line::raw(
                        "No connections yet. Press A to choose a service.",
                    ));
                }
                let room = usize::from(body.height.saturating_sub(4)).max(1);
                let start = self.selected.saturating_sub(room - 1);
                for (index, draft) in self.drafts.iter().enumerate().skip(start).take(room) {
                    lines.push(Line::styled(
                        format!(
                            "{} {} [{}]",
                            if index == self.selected { ">" } else { " " },
                            name(draft),
                            if draft.connection.enabled {
                                "on"
                            } else {
                                "off"
                            }
                        ),
                        if index == self.selected {
                            theme.selected
                        } else {
                            theme.dim
                        },
                    ));
                }
                if self.status.contains("payload") {
                    lines.push(Line::styled(
                        "K keeps the existing payload; P replaces it on save.",
                        theme.dim,
                    ));
                }
            }
            Page::Presets => {
                lines.push(Line::styled("Choose a service", theme.accent));
                let room = usize::from(body.height.saturating_sub(4)).max(1);
                let start = self.preset.saturating_sub(room - 1);
                for (index, preset) in self
                    .info
                    .descriptor
                    .presets
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take(room)
                {
                    lines.push(Line::styled(
                        format!(
                            "{} {}",
                            if index == self.preset { ">" } else { " " },
                            preset.label
                        ),
                        if index == self.preset {
                            theme.selected
                        } else {
                            theme.dim
                        },
                    ));
                }
            }
            Page::Fields | Page::Input => {
                if let Some(draft) = self.drafts.get(self.selected) {
                    lines.push(Line::styled(name(draft), theme.accent));
                    if let Some(field) = self.current_field() {
                        lines.push(Line::styled(
                            format!(
                                "Field {} of {} · {}",
                                self.field + 1,
                                self.fields().len(),
                                if self.advanced {
                                    "all settings"
                                } else {
                                    "basic settings"
                                }
                            ),
                            theme.dim,
                        ));
                        lines.push(Line::raw(""));
                        lines.push(Line::styled(field.label.clone(), theme.accent));
                        let mut value = if self.page == Page::Input {
                            if field.kind == "secret" {
                                if ascii { "*" } else { "•" }.repeat(self.buffer.chars().count())
                            } else {
                                self.buffer.clone()
                            }
                        } else {
                            self.shown(field)
                        };
                        if self.page == Page::Input {
                            let room = usize::from(body.width.saturating_sub(3));
                            value = value
                                .chars()
                                .rev()
                                .take(room)
                                .collect::<String>()
                                .chars()
                                .rev()
                                .collect();
                            cursor = Some((
                                body.x + 2 + u16::try_from(value.width()).unwrap_or(0),
                                body.y + 4,
                            ));
                        }
                        lines.push(Line::raw(format!("> {value}")));
                        lines.push(Line::raw(""));
                        if self.source_input && self.page == Page::Input {
                            lines.push(Line::raw(
                                "env:VARIABLE, file:PATH, keyring:NAME, cmd:[\"program\",\"arg\"]",
                            ));
                        } else {
                            lines.push(Line::raw(field.help.clone()));
                        }
                        if field.kind == "secret" {
                            lines.push(Line::styled("Empty keeps the saved credential. Ctrl-R removes it; Ctrl-X selects an external source.", theme.dim));
                        }
                        if let Some(preset) = self
                            .info
                            .descriptor
                            .presets
                            .iter()
                            .find(|p| p.id == draft.connection.preset)
                        {
                            lines.push(Line::styled(preset.help.clone(), theme.dim));
                        }
                    }
                }
            }
            Page::Sample => {
                lines.push(Line::styled(
                    "Check access using these unsaved settings",
                    theme.accent,
                ));
                lines.push(Line::raw(
                    "Paste a review or issue URL on this connection's server.",
                ));
                lines.push(Line::raw(format!("> {}", self.buffer)));
                cursor = Some((
                    body.x + 2 + u16::try_from(self.buffer.width()).unwrap_or(0),
                    body.y + 2,
                ));
                lines.push(Line::raw(
                    "Checks do not save credentials, enable hooks, or call a model.",
                ));
            }
            Page::Discard => {
                lines.push(Line::styled("Discard unsaved changes?", theme.warn));
                lines.push(Line::raw(
                    "D discards and closes. R reloads saved settings. Enter or Escape keeps editing.",
                ));
            }
            Page::Remove => {
                lines.push(Line::styled(
                    "Remove this connection from the draft?",
                    theme.warn,
                ));
                lines.push(Line::raw(
                    "D removes it; save to apply. Enter or Escape keeps it.",
                ));
            }
            Page::Help => {
                if let Some(field) = self.current_field() {
                    lines.push(Line::styled(field.label.clone(), theme.accent));
                    lines.push(Line::raw(field.help.clone()));
                    if let Some(preset) = self.drafts.get(self.selected).and_then(|d| {
                        self.info
                            .descriptor
                            .presets
                            .iter()
                            .find(|p| p.id == d.connection.preset)
                    }) {
                        lines.push(Line::raw(""));
                        lines.push(Line::raw(preset.help.clone()));
                    }
                    lines.push(Line::raw(""));
                    lines.push(Line::raw(format!(
                        "Configuration file: {}",
                        self.info.source
                    )));
                    lines.push(Line::raw("Credentials are saved in your user credential store. Editing a project hook writes credential references to the trusted project file."));
                }
            }
        }
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((
                if self.page == Page::Help {
                    self.help_scroll
                } else {
                    0
                },
                0,
            )),
            body,
        );
        frame.render_widget(
            Paragraph::new(self.status.clone())
                .style(theme.warn)
                .wrap(Wrap { trim: false }),
            status,
        );
        let footer_text = if self.page == Page::Help {
            "Up/Down scroll help · Esc back"
        } else if self.page == Page::Input {
            "Enter accepts · Tab next field · Ctrl-U clears · Esc back"
        } else if self.page == Page::Sample {
            "Enter checks access · Ctrl-U clears · Esc back"
        } else {
            "Tab/Shift-Tab move · Enter edit · Esc back\nCtrl-S Save · Ctrl-E Save and enable · Ctrl-T Check\nA Add · Ctrl-D on/off · Del remove · V advanced\n? help · R reload · Shift-R reapply draft"
        };
        frame.render_widget(
            Paragraph::new(footer_text)
                .style(theme.dim)
                .wrap(Wrap { trim: false }),
            footer,
        );
        if let Some((x, y)) = cursor.filter(|(_, y)| *y < body.bottom()) {
            frame.set_cursor_position((x.min(body.right().saturating_sub(1)), y));
        }
    }
}

#[cfg(test)]
pub(crate) fn example() -> HookSetupInfo {
    let file: quack_config::HookFile =
        toml::from_str(include_str!("../../../defaults/hooks.d/forge-review.toml"))
            .expect("default hook");
    HookSetupInfo {
        id: "forge-review".into(),
        revision: "test-revision".into(),
        descriptor: file.descriptor().expect("descriptor"),
        connections: file
            .connections
            .expect("connections")
            .into_iter()
            .map(|b| quack_protocol::HookConnectionInfo {
                connection: b.connection,
                credentials: BTreeMap::new(),
            })
            .collect(),
        enabled: false,
        source: "defaults/hooks.d/forge-review.toml".into(),
        writable: true,
        problem: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn adding_connections_uses_the_descriptor_and_preserves_unique_ids() {
        let mut info = example();
        info.connections.clear();
        let mut form = HookForm::new(info);
        form.key(key(KeyCode::Char('a')));
        form.key(key(KeyCode::Enter));
        assert_eq!(
            form.drafts[0].connection.values["web_url"],
            "https://github.com"
        );
        form.key(key(KeyCode::Esc));
        form.key(key(KeyCode::Char('a')));
        form.key(key(KeyCode::Enter));
        assert_ne!(form.drafts[0].connection.id, form.drafts[1].connection.id);
        assert!(matches!(
            form.key(ctrl('s')),
            Action::Save { enable: None, .. }
        ));
        assert!(matches!(
            form.key(ctrl('e')),
            Action::Save {
                enable: Some(true),
                ..
            }
        ));
    }

    #[test]
    fn secrets_stay_masked_and_an_empty_answer_keeps_the_saved_source() {
        let mut form = HookForm::new(example());
        form.key(key(KeyCode::Enter));
        for _ in 0..3 {
            form.key(key(KeyCode::Tab));
        }
        assert_eq!(form.current_field().expect("field").kind, "secret");
        form.key(key(KeyCode::Enter));
        form.paste("private-test-token");
        assert!(!format!("{form:?}").contains("private-test-token"));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 20)).expect("terminal");
        terminal
            .draw(|f| form.draw(f, &Theme::resolve("dark", false), &Glyphs::new(true), true))
            .expect("render");
        let shown = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(!shown.contains("private-test-token"));
        assert!(shown.contains("*****"));
        form.key(key(KeyCode::Enter));
        assert!(matches!(
            form.drafts[0].credentials.get("token"),
            Some(HookCredentialChange::Replace { .. })
        ));
        form.key(key(KeyCode::Enter));
        form.key(key(KeyCode::Enter));
        assert!(matches!(
            form.drafts[0].credentials.get("token"),
            Some(HookCredentialChange::Keep)
        ));
    }

    #[test]
    fn cancel_check_and_discard_do_not_submit_a_save() {
        let mut form = HookForm::new(example());
        form.key(ctrl('t'));
        form.paste("https://github.com/a/b/pull/1");
        assert!(matches!(
            form.key(key(KeyCode::Enter)),
            Action::Check { .. }
        ));
        form.waiting(42, true);
        assert!(matches!(form.key(key(KeyCode::Esc)), Action::Cancel(42)));
        form.key(ctrl('d'));
        assert!(matches!(form.key(key(KeyCode::Esc)), Action::None));
        assert_eq!(form.page, Page::Discard);
        assert!(matches!(form.key(key(KeyCode::Char('d'))), Action::Close));
    }

    #[test]
    fn tab_accepts_an_edit_and_shift_tab_moves_back() {
        let mut form = HookForm::new(example());
        form.key(key(KeyCode::Enter));
        form.key(key(KeyCode::Enter));
        form.key(ctrl('u'));
        form.paste("Work account");
        form.key(key(KeyCode::Tab));
        assert_eq!(form.drafts[0].connection.values["label"], "Work account");
        assert_eq!(form.current_field().expect("field").id, "web_url");
        form.key(key(KeyCode::Enter));
        form.key(key(KeyCode::BackTab));
        assert_eq!(form.current_field().expect("field").id, "label");
    }

    #[test]
    fn a_new_connection_does_not_reuse_a_removed_saved_connections_identity() {
        let mut info = example();
        info.connections[0].connection.id = "connection-1".into();
        let mut form = HookForm::new(info);
        form.key(key(KeyCode::Delete));
        form.key(key(KeyCode::Char('d')));
        form.key(key(KeyCode::Char('a')));
        form.key(key(KeyCode::Enter));
        assert_eq!(
            form.drafts.last().expect("new connection").connection.id,
            "connection-2"
        );
    }
}
