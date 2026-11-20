//! Shared modal framework (docs/plan/06-tui.md#modal-framework): title, optional tabs,
//! an always-focused filter line, a list pane, a details pane and a key footer.
//! Every configuration command (`/models`, `/resume`, `/config`, …) is built on this.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Clear, List, ListItem, ListState, Padding, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState,
};
use unicode_width::UnicodeWidthStr;

use quack_config::ProjectStatus;
use quack_protocol::WriteScope;

use crate::charts::{BarRow, Visual};
use crate::commands::{COMMANDS, KEYS};
use crate::fuzzy;
use crate::theme::{Glyphs, Theme};
use crate::ui::truncate;

/// Below this size the modal goes full-screen with a single column.
const MIN_SPLIT: (u16, u16) = (80, 24);

mod extensions;
mod inspectors;
mod pickers;
mod providers;

pub(crate) use extensions::{
    bundles, custom_commands, hook_payload, hooks, instructions, lsp, mcp, profiles, skills,
};
pub(crate) use inspectors::{context, doctor, stats, tool_probe};
pub(crate) use pickers::{
    ModelRoles, TrustPrompt, config, help, memory, model_defaults, models, permissions, rewind,
    sessions, tasks, theme, thinking, trust,
};
pub(crate) use providers::{
    Scan, SetupView, key_source, provider_connection, provider_label_id, providers, setup, sign_in,
};

#[derive(Debug, Clone)]
pub(crate) struct ModalItem {
    pub label: String,
    /// Short right-aligned text in the list (state, milestone, size).
    pub hint: String,
    pub detail: Vec<Line<'static>>,
    /// Text inserted into the input when the item is activated, if any.
    pub action: Option<String>,
    /// Run `action` as a command immediately instead of inserting it.
    pub run: bool,
    /// Command run by the modal's secondary key (Ctrl-S), if any.
    pub alt: Option<String>,
    /// Charts in the details pane, each before the `detail` line it names.
    pub charts: Vec<(usize, Visual)>,
}

#[derive(Debug, Clone)]
pub(crate) struct ModalTab {
    pub title: String,
    pub items: Vec<ModalItem>,
}

#[derive(Debug)]
pub(crate) struct Modal {
    pub title: String,
    tabs: Vec<ModalTab>,
    tab: usize,
    filter: String,
    filtered: Vec<usize>,
    selected: usize,
    /// Optional section header per item for each tab (`groups[tab][item]`).
    groups: Vec<Vec<Option<&'static str>>>,
    /// Tab whose item actions copy text while keeping the modal open.
    copy_tab: Option<usize>,
    /// True when `^l` toggles between user and project (`.quack/`) write scope.
    scoped: bool,
    footer: Vec<(&'static str, &'static str)>,
}

/// The secondary action key, shown in the footer when a modal has one.
const ALT_KEY: &str = "^s";
/// The scope-toggle key, shown in the footer of modals that save settings/MCP.
const SCOPE_KEY: &str = "^l";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ModalOutcome {
    None,
    Close,
    Copy(String),
    /// Close and put this text in the input.
    Insert(String),
    /// Close and run this command.
    Run(String),
    /// Toggle between user and project (`.quack/`) write scope and rebuild.
    ToggleScope,
}

impl Modal {
    pub fn new(title: impl Into<String>, tabs: Vec<ModalTab>) -> Self {
        let mut m = Self {
            title: title.into(),
            tabs,
            tab: 0,
            filter: String::new(),
            filtered: Vec::new(),
            selected: 0,
            groups: Vec::new(),
            copy_tab: None,
            scoped: false,
            footer: vec![
                ("↑↓", "move"),
                ("tab", "switch tab"),
                ("⏎", "use"),
                ("esc", "close"),
            ],
        };
        m.refilter();
        m
    }

    /// Build a single-tab modal whose items are visually separated into named sections.
    pub fn grouped(
        title: impl Into<String>,
        sections: Vec<(&'static str, Vec<ModalItem>)>,
    ) -> Self {
        let mut items = Vec::new();
        let mut item_groups = Vec::new();
        for (heading, section_items) in sections {
            for item in section_items {
                items.push(item);
                item_groups.push(Some(heading));
            }
        }
        let title = title.into();
        let mut m = Self::new(title.clone(), vec![ModalTab { title, items }]);
        m.groups = vec![item_groups];
        m.refilter();
        m
    }

    /// Name the secondary action (Ctrl-S) in the footer.
    pub fn with_alt(mut self, what: &'static str) -> Self {
        self.footer.insert(self.footer.len() - 1, (ALT_KEY, what));
        self
    }

    /// Enable the Ctrl-L user/project (`.quack/`) scope toggle and show it in the footer.
    pub fn with_scope(mut self, scope: WriteScope) -> Self {
        self.scoped = true;
        let label = match scope {
            WriteScope::Project => "scope: .quack",
            WriteScope::User | WriteScope::Session => "scope: user",
        };
        let pos = self
            .footer
            .iter()
            .position(|(k, _)| *k == ALT_KEY || *k == "⏎")
            .unwrap_or_else(|| self.footer.len().saturating_sub(1));
        self.footer.insert(pos, (SCOPE_KEY, label));
        self
    }

    /// Keep the tab, filter and selection of the modal this one replaces, so a refresh
    /// that arrives while the user looks around does not move them.
    pub fn keep_place(mut self, old: Option<&Modal>) -> Self {
        if let Some(old) = old.filter(|o| o.title == self.title) {
            self.tab = old.tab.min(self.tabs.len().saturating_sub(1));
            self.filter = old.filter.clone();
            self.selected = old.selected;
            self.refilter();
        }
        self
    }

    /// Start with this text in the filter line.
    pub fn with_filter(mut self, text: &str) -> Self {
        self.filter = text.to_owned();
        self.refilter();
        self
    }

    /// Select the first visible item matching `pred`, if any.
    pub fn select_matching(mut self, mut pred: impl FnMut(&ModalItem) -> bool) -> Self {
        if let Some(pos) = self
            .filtered
            .iter()
            .position(|i| self.items().get(*i).is_some_and(&mut pred))
        {
            self.selected = pos;
        }
        self
    }

    /// Open on `tab`.
    pub fn with_tab(mut self, tab: usize) -> Self {
        self.tab = tab.min(self.tabs.len().saturating_sub(1));
        self.refilter();
        self
    }

    fn items(&self) -> &[ModalItem] {
        self.tabs.get(self.tab).map_or(&[], |t| t.items.as_slice())
    }

    fn item_group(&self, item_idx: usize) -> Option<&'static str> {
        self.groups
            .get(self.tab)
            .and_then(|g| g.get(item_idx))
            .copied()
            .flatten()
    }

    fn has_groups(&self) -> bool {
        self.groups
            .get(self.tab)
            .is_some_and(|g| g.iter().any(Option::is_some))
    }

    fn refilter(&mut self) {
        if self.has_groups() {
            let candidates: Vec<String> = self
                .items()
                .iter()
                .enumerate()
                .map(|(i, item)| match self.item_group(i) {
                    Some(g) => format!("{g} {}", item.label),
                    None => item.label.clone(),
                })
                .collect();
            self.filtered = fuzzy::filter(&self.filter, candidates.iter().map(String::as_str));
            // Keep items in their section order so group headers never split or repeat.
            self.filtered.sort_unstable();
        } else {
            self.filtered =
                fuzzy::filter(&self.filter, self.items().iter().map(|i| i.label.as_str()));
        }
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
    }

    pub fn selected_item(&self) -> Option<&ModalItem> {
        self.filtered
            .get(self.selected)
            .and_then(|i| self.items().get(*i))
    }

    pub fn paste(&mut self, text: &str) {
        self.filter
            .push_str(text.lines().next().unwrap_or_default());
        self.refilter();
    }

    pub fn on_key(&mut self, key: KeyEvent) -> ModalOutcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.refilter();
            }
            KeyCode::Esc => return ModalOutcome::Close,
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Char('p') if ctrl => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down => self.move_down(),
            KeyCode::Char('n') if ctrl => self.move_down(),
            KeyCode::Char('s') if ctrl => {
                if let Some(cmd) = self.selected_item().and_then(|i| i.alt.clone()) {
                    return ModalOutcome::Run(cmd);
                }
            }
            KeyCode::Char('l') if ctrl && self.scoped => return ModalOutcome::ToggleScope,
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(10),
            KeyCode::PageDown => {
                self.selected = (self.selected + 10).min(self.filtered.len().saturating_sub(1));
            }
            KeyCode::Tab | KeyCode::BackTab if self.tabs.len() > 1 => {
                let n = self.tabs.len();
                self.tab = if key.code == KeyCode::Tab {
                    (self.tab + 1) % n
                } else {
                    (self.tab + n - 1) % n
                };
                self.selected = 0;
                self.refilter();
            }
            KeyCode::Enter => {
                return match self.selected_item() {
                    Some(ModalItem {
                        action: Some(text),
                        run,
                        ..
                    }) => {
                        if self.copy_tab == Some(self.tab) {
                            ModalOutcome::Copy(text.clone())
                        } else if *run {
                            ModalOutcome::Run(text.clone())
                        } else {
                            ModalOutcome::Insert(text.clone())
                        }
                    }
                    _ => ModalOutcome::None,
                };
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.refilter();
            }
            KeyCode::Char(c) if !ctrl => {
                self.filter.push(c);
                self.selected = 0;
                self.refilter();
            }
            _ => {}
        }
        ModalOutcome::None
    }

    fn move_down(&mut self) {
        self.selected = (self.selected + 1).min(self.filtered.len().saturating_sub(1));
    }

    pub fn draw(&self, f: &mut Frame<'_>, theme: &Theme, glyphs: &Glyphs) {
        let screen = f.area();
        let narrow = screen.width < MIN_SPLIT.0 || screen.height < MIN_SPLIT.1;
        let area = if narrow {
            screen
        } else {
            centered(screen, 90, 85)
        };
        f.render_widget(Clear, area);
        let block = Block::bordered()
            .border_set(glyphs.border)
            .border_style(theme.accent)
            .title(Span::styled(
                format!(" {} ", self.title),
                theme.pill(theme.accent),
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let tabs_h = u16::from(self.tabs.len() > 1);
        // Room to breathe under the title and between the rows, unless the screen is small.
        let gap = u16::from(!narrow);
        let [_, tabs_area, _, filter_area, _, body, footer] = Layout::vertical([
            Constraint::Length(gap),
            Constraint::Length(tabs_h),
            Constraint::Length(gap * tabs_h),
            Constraint::Length(1),
            Constraint::Length(gap),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .areas(inner);

        if tabs_h > 0 {
            let mut spans = vec![Span::raw(" ")];
            for (i, tab) in self.tabs.iter().enumerate() {
                let style = if i == self.tab {
                    theme.pill(theme.accent)
                } else {
                    theme.dim
                };
                spans.push(Span::styled(format!(" {} ", tab.title), style));
                spans.push(Span::raw(" "));
            }
            f.render_widget(Paragraph::new(Line::from(spans)), tabs_area);
        }
        let total = self.items().len();
        let count = if self.filter.is_empty() {
            format!("{total} ")
        } else {
            format!("{} of {total} ", self.filtered.len())
        };
        let mut filter_line = vec![Span::styled(format!(" {} ", glyphs.prompt), theme.accent)];
        if self.filter.is_empty() {
            filter_line.push(Span::styled("type to filter", theme.dim));
        } else {
            filter_line.push(Span::styled(self.filter.clone(), theme.user));
        }
        f.render_widget(Paragraph::new(Line::from(filter_line)), filter_area);
        f.render_widget(
            Paragraph::new(Line::styled(count, theme.dim).right_aligned()),
            filter_area,
        );
        let prompt_w = u16::try_from(glyphs.prompt.width() + 2).unwrap_or(3);
        let cursor_x = filter_area.x + prompt_w + u16::try_from(self.filter.width()).unwrap_or(0);
        f.set_cursor_position((
            cursor_x.min(filter_area.right().saturating_sub(1)),
            filter_area.y,
        ));

        let copying = self.copy_tab == Some(self.tab);
        let (list_area, detail_area) = if narrow {
            let [l, d] = Layout::vertical([
                Constraint::Percentage(if copying { 45 } else { 60 }),
                Constraint::Percentage(if copying { 55 } else { 40 }),
            ])
            .areas(body);
            (l, d)
        } else {
            let [l, d] = Layout::horizontal([
                Constraint::Percentage(if copying { 35 } else { 45 }),
                Constraint::Percentage(if copying { 65 } else { 55 }),
            ])
            .spacing(u16::from(copying && !narrow))
            .areas(body);
            (l, d)
        };
        self.draw_list(f, list_area, gap, theme, glyphs);
        self.draw_details(f, detail_area, gap, theme, glyphs);

        let keys: Vec<(&str, &str)> = self
            .footer
            .iter()
            .filter(|(k, _)| *k != "tab" || self.tabs.len() > 1)
            .map(|&(key, label)| {
                if copying && self.title.starts_with("Sign in:") && key == "⏎" {
                    (key, "copy URL")
                } else if copying && key == "⏎" {
                    (key, "copy path")
                } else {
                    (key, label)
                }
            })
            .collect();
        f.render_widget(Paragraph::new(crate::ui::key_hints(theme, &keys)), footer);
    }

    /// `top`: blank rows above the first item.
    fn draw_list(&self, f: &mut Frame<'_>, area: Rect, top: u16, theme: &Theme, glyphs: &Glyphs) {
        let block = Block::bordered()
            .border_set(glyphs.border)
            .border_style(theme.border)
            .padding(Padding::top(top));
        let inner = block.inner(area);
        let width = usize::from(inner.width);
        let height = usize::from(inner.height);
        let grouped = self.has_groups();
        let make_row = |item: &ModalItem, on: bool| -> ListItem<'static> {
            let soft = if on { theme.sel_text } else { theme.dim };
            let hint = truncate(&item.hint, width / 2);
            let label = truncate(&item.label, width.saturating_sub(hint.width() + 4));
            let hits = if label == item.label {
                fuzzy::positions(&self.filter, &item.label)
            } else {
                Vec::new()
            };
            let pad = width.saturating_sub(label.width() + hint.width() + 3);
            let mut spans = vec![
                Span::styled(if on { glyphs.bar } else { " " }, theme.accent),
                Span::raw(" "),
            ];
            let base = if on { theme.selected } else { Style::new() };
            spans.extend(theme.highlight(&label, &hits, base));
            spans.push(Span::raw(" ".repeat(pad)));
            spans.push(Span::styled(hint, soft));
            ListItem::new(Line::from(spans))
        };
        let (rows, selected_row, total_rows, scroll_pos) = if grouped {
            let mut rows: Vec<ListItem<'_>> = Vec::new();
            let mut selected_row = None;
            let mut prev_group: Option<&str> = None;
            for (pos, &i) in self.filtered.iter().enumerate() {
                let Some(item) = self.items().get(i) else {
                    continue;
                };
                let group = self.item_group(i);
                if group != prev_group {
                    if !rows.is_empty() {
                        rows.push(ListItem::new(Line::raw("")));
                    }
                    if let Some(title) = group {
                        let label = format!("  {title} ");
                        let rule_len = width.saturating_sub(label.width() + 1);
                        let mut spans =
                            vec![Span::styled(label, theme.dim.add_modifier(Modifier::BOLD))];
                        if rule_len > 0 {
                            spans.push(Span::styled(glyphs.rule.repeat(rule_len), theme.border));
                        }
                        rows.push(ListItem::new(Line::from(spans)));
                    }
                    prev_group = group;
                }
                let on = pos == self.selected;
                if on {
                    selected_row = Some(rows.len());
                }
                rows.push(make_row(item, on));
            }
            let total_rows = rows.len();
            let scroll_pos = selected_row.unwrap_or(0);
            (rows, selected_row, total_rows, scroll_pos)
        } else {
            // Only the rows that can be visible are built. A fresh `ListState`
            // starts at offset 0 and ratatui slides the window to keep `selected`
            // on screen, so with single-line rows the visible range is
            // `first..last` below and rows outside it are never drawn.
            // (`selected` is kept below `filtered.len()` by `refilter` and the
            // movement keys, so the clamp only guards the empty list.)
            let len = self.filtered.len();
            let shown = self.selected.min(len.saturating_sub(1));
            let first = shown.saturating_sub(height.saturating_sub(1));
            let last = (first + height).min(len);
            let mut rows: Vec<ListItem<'_>> = Vec::with_capacity(last - first);
            let mut selected_row = None;
            for (n, i) in self.filtered[first..last].iter().enumerate() {
                let Some(item) = self.items().get(*i) else {
                    continue;
                };
                let on = first + n == self.selected;
                if on {
                    selected_row = Some(rows.len());
                }
                rows.push(make_row(item, on));
            }
            (rows, selected_row, self.filtered.len(), self.selected)
        };
        let empty = rows.is_empty();
        // The selected row's position in `rows`, or the last row, matching how
        // ratatui clamps an out-of-range selection.
        let selected = (!empty).then(|| selected_row.unwrap_or(rows.len() - 1));
        let list = List::new(rows).block(block).highlight_style(theme.selected);
        let mut state = ListState::default().with_selected(selected);
        f.render_stateful_widget(list, area, &mut state);
        if total_rows > height {
            let mut sb = ScrollbarState::new(total_rows).position(scroll_pos);
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None)
                    .thumb_symbol(glyphs.scroll[0])
                    .thumb_style(theme.accent)
                    .track_symbol(Some(glyphs.scroll[1]))
                    .track_style(theme.border),
                area.inner(Margin::new(0, 1)),
                &mut sb,
            );
        }
    }

    fn draw_details(
        &self,
        f: &mut Frame<'_>,
        area: Rect,
        top: u16,
        theme: &Theme,
        glyphs: &Glyphs,
    ) {
        let block = Block::bordered()
            .border_set(glyphs.border)
            .border_style(theme.border)
            .padding(Padding::new(1, 1, top, 0));
        let inner = block.inner(area);
        f.render_widget(block, area);
        match self.selected_item() {
            Some(i) if self.copy_tab == Some(self.tab) => {
                let lines: Vec<Line<'_>> = i
                    .detail
                    .iter()
                    .flat_map(|line| {
                        crate::wrap::wrap(&line.to_string(), usize::from(inner.width))
                            .into_iter()
                            .map(|text| Line::styled(text, line.style))
                    })
                    .collect();
                f.render_widget(Paragraph::new(lines), inner);
            }
            Some(i) => crate::charts::draw_detail(f, inner, &i.detail, &i.charts, theme),
            None => f.render_widget(Paragraph::new(Line::styled("no matches", theme.dim)), inner),
        }
    }
}

pub(crate) fn centered(area: Rect, pct_w: u16, pct_h: u16) -> Rect {
    let w = area.width * pct_w / 100;
    let h = area.height * pct_h / 100;
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn scope_file(scope: WriteScope) -> &'static str {
    match scope {
        WriteScope::Project => ".quack/config.toml",
        WriteScope::User | WriteScope::Session => "user config.toml",
    }
}

fn scope_flag(scope: WriteScope) -> &'static str {
    match scope {
        WriteScope::Project => " --project",
        WriteScope::User | WriteScope::Session => "",
    }
}

fn project_status_note(status: ProjectStatus) -> Option<&'static str> {
    match status {
        ProjectStatus::DisabledByPolicy => Some("project config is disabled by the system policy"),
        ProjectStatus::Untrusted | ProjectStatus::Changed | ProjectStatus::Ignored => {
            Some("note: .quack/ is not trusted yet — run `/trust` to load project settings")
        }
        ProjectStatus::None | ProjectStatus::Trusted | ProjectStatus::SettingsOnly => None,
    }
}

fn field(theme: &Theme, k: &str, v: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{k:<10}"), theme.dim),
        Span::raw(v),
    ])
}

fn placeholder(label: &str, detail: &str, theme: &Theme) -> ModalItem {
    ModalItem {
        label: label.to_owned(),
        hint: String::new(),
        detail: vec![Line::styled(detail.to_owned(), theme.dim)],
        action: None,
        run: false,
        alt: None,
        charts: Vec::new(),
    }
}

/// Detail lines with charts set between them.
#[derive(Default)]
struct Detail {
    lines: Vec<Line<'static>>,
    charts: Vec<(usize, Visual)>,
}

impl Detail {
    fn line(&mut self, l: Line<'static>) -> &mut Self {
        self.lines.push(l);
        self
    }

    fn gap(&mut self) -> &mut Self {
        self.line(Line::raw(""))
    }

    /// A chart here, then a blank line.
    fn chart(&mut self, v: Visual) -> &mut Self {
        self.meter(v).gap()
    }

    /// A chart here, with the next one straight under it: for a stack of meters.
    fn meter(&mut self, v: Visual) -> &mut Self {
        self.charts.push((self.lines.len(), v));
        self
    }

    fn item(self, label: &str, hint: String) -> ModalItem {
        ModalItem {
            label: label.to_owned(),
            hint,
            detail: self.lines,
            action: None,
            run: false,
            alt: None,
            charts: self.charts,
        }
    }
}

/// How many tools `/stats` draws bars for; the rest are counted below them.
const TOOL_BARS: usize = 8;

#[cfg(test)]
mod tests {
    use super::providers::setup_checks;
    use super::*;
    use crossterm::event::KeyEventKind;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Press)
    }

    fn row(key: &str, value: &str, locked: bool) -> quack_config::SettingRow {
        quack_config::SettingRow {
            key: key.into(),
            value: value.into(),
            origin: "default".into(),
            description: String::new(),
            is_bool: value == "true" || value == "false",
            locked,
        }
    }

    #[test]
    fn setup_offers_only_the_checks_with_somewhere_to_go() {
        let theme = Theme::resolve("dark", false);
        let settings = vec![
            row("catalog.refresh", "false", false),
            row("catalog.url", "\"https://feed.example/api.json\"", false),
            row("updates.check", "true", true),
            row("updates.url", "\"https://releases.example/latest\"", false),
        ];
        let items = setup_checks(&settings, &theme);
        assert_eq!(items[0].hint, "off");
        assert_eq!(
            items[0].action.as_deref(),
            Some("/config set catalog.refresh true")
        );
        let shown = format!("{:?}", items[0].detail);
        assert!(shown.contains("feed.example"), "{shown}");
        assert_eq!(items[1].hint, "on, locked");
        assert_eq!(items[1].action, None);

        let plucked = vec![
            row("catalog.refresh", "false", false),
            row("catalog.url", "\"\"", false),
        ];
        assert!(setup_checks(&plucked, &theme).is_empty());

        let mut offline = settings;
        offline.push(row("network.offline", "true", false));
        assert!(setup_checks(&offline, &theme).is_empty());
    }

    #[test]
    fn filter_tabs_and_activation() {
        let mut m = help(&Theme::resolve("dark", false));
        for c in "resu".chars() {
            m.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(m.selected_item().map(|i| i.label.as_str()), Some("/resume"));
        assert_eq!(
            m.on_key(key(KeyCode::Enter)),
            ModalOutcome::Insert("/resume ".into())
        );
        assert_eq!(
            m.on_key(key(KeyCode::Esc)),
            ModalOutcome::None,
            "first esc clears filter"
        );
        m.on_key(key(KeyCode::Tab));
        assert_eq!(m.selected_item().map(|i| i.label.as_str()), Some("Enter"));
        assert_eq!(m.on_key(key(KeyCode::Esc)), ModalOutcome::Close);
    }
}
