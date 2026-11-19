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

/// The `/help` modal: commands and keys.
pub(crate) fn help(theme: &Theme) -> Modal {
    let field = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(format!("{k:<10}"), theme.dim),
            Span::raw(v),
        ])
    };
    let commands = COMMANDS
        .iter()
        .map(|c| {
            let mut detail = vec![
                Line::styled(format!("/{} {}", c.name, c.args), theme.accent),
                Line::raw(""),
                Line::raw(c.description),
                Line::raw(""),
            ];
            if !c.aliases.is_empty() {
                let aliases: Vec<String> = c.aliases.iter().map(|a| format!("/{a}")).collect();
                detail.push(field("aliases", aliases.join(", ")));
            }
            detail.push(field(
                "status",
                c.pending
                    .map_or_else(|| "available".into(), |m| format!("planned for {m}")),
            ));
            ModalItem {
                label: format!("/{}", c.name),
                hint: c.pending.unwrap_or_default().to_owned(),
                detail,
                action: Some(format!(
                    "/{}{}",
                    c.name,
                    if c.args.is_empty() { "" } else { " " }
                )),
                run: false,
                alt: None,
                charts: Vec::new(),
            }
        })
        .collect();
    let keys = KEYS
        .iter()
        .map(|(k, what)| ModalItem {
            label: (*k).to_owned(),
            hint: String::new(),
            detail: vec![
                Line::styled(*k, theme.accent),
                Line::raw(""),
                Line::raw(*what),
            ],
            action: None,
            run: false,
            alt: None,
            charts: Vec::new(),
        })
        .collect();
    Modal::new(
        "Help",
        vec![
            ModalTab {
                title: "Commands".into(),
                items: commands,
            },
            ModalTab {
                title: "Keys".into(),
                items: keys,
            },
        ],
    )
}

/// The current model's declared thinking levels, plus a reset to its configured default.
pub(crate) fn thinking(
    model: Option<&str>,
    levels: &[String],
    current: Option<&str>,
    theme: &Theme,
) -> Modal {
    let mut items = Vec::new();
    if model.is_some() && !levels.is_empty() {
        for level in std::iter::once("default").chain(levels.iter().map(String::as_str)) {
            let description = if level == "default" {
                "Restore the level configured in the model's provider file."
            } else {
                "Use this thinking level for this model in the current session."
            };
            items.push(ModalItem {
                label: level.to_owned(),
                hint: if current == Some(level) || (current.is_none() && level == "default") {
                    "current".into()
                } else {
                    String::new()
                },
                detail: vec![
                    Line::styled(model.unwrap_or_default().to_owned(), theme.accent),
                    Line::raw(""),
                    Line::raw(description),
                    Line::raw(""),
                    Line::raw("Lower levels favor speed and fewer reasoning tokens."),
                    Line::raw("Higher levels allow more time for difficult tasks."),
                    Line::raw("Changes apply between turns and are saved with the session."),
                ],
                action: Some(if level == "default" {
                    "/thinking default".into()
                } else {
                    // Names such as `off` may also be visibility commands.
                    format!("/thinking level {level}")
                }),
                run: true,
                alt: None,
                charts: Vec::new(),
            });
        }
    } else {
        items.push(ModalItem {
            label: "No selectable thinking levels".into(),
            hint: String::new(),
            detail: vec![Line::raw(match model {
                Some(model) => format!(
                    "{model} does not declare thinking levels. Run `quack catalog refresh` to fetch \
                     the catalog's, or add reasoning.efforts to a user-layer copy of the provider file."
                ),
                None => "Select a model with /models first.".into(),
            })],
            action: None,
            run: false,
            alt: None,
            charts: Vec::new(),
        });
    }
    let mut modal = Modal::new(
        "Thinking",
        vec![ModalTab {
            title: "Levels".into(),
            items,
        }],
    );
    modal.footer.retain(|(key, _)| *key != "tab");
    if let Some(level) = current
        && let Some(index) = levels.iter().position(|e| e == level)
    {
        modal.selected = index + 1;
    }
    modal
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ModelRoles<'a> {
    pub current: Option<&'a str>,
    pub default: Option<&'a str>,
    pub small: Option<&'a str>,
    pub default_locked: bool,
    pub small_locked: bool,
    pub project_status: ProjectStatus,
}

/// The `/models` modal. `catalog` is `None` while the engine is still listing models.
pub(crate) fn models(
    catalog: Option<&[quack_protocol::ProviderModels]>,
    roles: ModelRoles<'_>,
    ascii: bool,
    scope: WriteScope,
    theme: &Theme,
) -> Modal {
    let target = scope_file(scope);
    let field = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(format!("{k:<10}"), theme.dim),
            Span::raw(v),
        ])
    };
    let mut items = Vec::new();
    match catalog {
        None => items.push(ModalItem {
            label: "listing models…".into(),
            hint: String::new(),
            detail: vec![Line::styled(
                "Asking each provider for its models.",
                theme.dim,
            )],
            action: None,
            run: false,
            alt: None,
            charts: Vec::new(),
        }),
        Some(providers) => {
            // Context windows are drawn against the largest on offer.
            let widest = providers
                .iter()
                .filter(|p| p.available)
                .flat_map(|p| p.models.iter().filter_map(|m| m.context_window))
                .max()
                .unwrap_or(0);
            // The bundle's providers that are not set up are listed in /providers.
            for p in providers.iter().filter(|p| p.available || !p.bundled) {
                if !p.available {
                    items.push(ModalItem {
                        label: format!("{} (unavailable)", p.id),
                        hint: String::new(),
                        detail: vec![
                            Line::styled(p.id.clone(), theme.accent),
                            Line::raw(""),
                            Line::raw(p.note.clone()),
                        ],
                        action: None,
                        run: false,
                        alt: None,
                        charts: Vec::new(),
                    });
                    continue;
                }
                for m in &p.models {
                    let name = m.alias.clone().unwrap_or_else(|| m.id.clone());
                    let reference = format!("{}/{}", p.id, name);
                    let raw_ref = format!("{}/{}", p.id, m.id);
                    let matches_ref = |target: Option<&str>| {
                        target == Some(reference.as_str()) || target == Some(raw_ref.as_str())
                    };
                    let is_current = matches_ref(roles.current);
                    let is_default = matches_ref(roles.default);
                    let is_small = matches_ref(roles.small);
                    let role_tag = match (is_default, is_small) {
                        (true, true) => "both",
                        (true, false) => "large",
                        (false, true) => "small",
                        (false, false) => "",
                    };
                    let ctx = m
                        .context_window
                        .map_or_else(String::new, crate::theme::tokens);
                    let hint = match (role_tag.is_empty(), ctx.is_empty()) {
                        (true, _) => ctx,
                        (false, true) => role_tag.into(),
                        (false, false) => format!("{role_tag} · {ctx}"),
                    };
                    let mut detail = vec![
                        Line::styled(reference.clone(), theme.accent),
                        Line::raw(""),
                        field("provider", p.id.clone()),
                        field("model id", m.id.clone()),
                        field(
                            "context",
                            m.context_window
                                .map_or_else(|| "unknown".into(), |c| format!("{c} tokens")),
                        ),
                    ];
                    if let Some(price) = &m.price {
                        detail.push(field("price", price.clone()));
                    }
                    if !m.reasoning_efforts.is_empty() {
                        detail.push(field("thinking", m.reasoning_efforts.join(" · ")));
                    }
                    if is_default || is_small {
                        detail.push(field(
                            "default",
                            match (is_default, is_small) {
                                (true, true) => "large + small".into(),
                                (true, false) => "large (model.default)".into(),
                                (false, true) => "small (model.small)".into(),
                                (false, false) => String::new(),
                            },
                        ));
                    }
                    detail.push(Line::raw(""));
                    let mut charts = Vec::new();
                    if let Some(c) = m.context_window.filter(|_| widest > 0) {
                        charts.push((
                            detail.len(),
                            Visual::Meter {
                                label: "window".into(),
                                text: if c == widest {
                                    "largest here".into()
                                } else {
                                    format!(
                                        "{}% of largest",
                                        u64::from(c) * 100 / u64::from(widest)
                                    )
                                },
                                ratio: f64::from(c) / f64::from(widest),
                                style: theme.accent,
                                ascii,
                            },
                        ));
                        detail.push(Line::raw(""));
                    }
                    detail.push(if is_current {
                        Line::styled("in use", theme.ok)
                    } else {
                        Line::styled("⏎ switch to this model for the session", theme.dim)
                    });
                    if !m.reasoning_efforts.is_empty() {
                        detail.push(Line::styled(
                            if is_current {
                                "/thinking reopens the level picker"
                            } else {
                                "quack then asks for a thinking level (speed vs cost)"
                            },
                            theme.dim,
                        ));
                    }
                    detail.push(Line::styled(
                        format!("^s set default: large, small, or both ({target})"),
                        theme.dim,
                    ));
                    items.push(ModalItem {
                        label: if is_current {
                            format!("{reference} ●")
                        } else {
                            reference.clone()
                        },
                        hint,
                        detail,
                        action: Some(format!("/models {reference}")),
                        run: true,
                        alt: Some(format!("/models default-menu {reference}")),
                        charts,
                    });
                }
            }
            if items.is_empty() {
                items.push(placeholder(
                    "no models ready",
                    "/setup adds one: a server on this machine, a hosted service with its \
                     key, or any server by its URL. /providers lists what is configured.",
                    theme,
                ));
            }
        }
    }
    Modal::new(
        "Models",
        vec![ModalTab {
            title: "Models".into(),
            items,
        }],
    )
    .with_alt("save as default")
    .with_scope(scope)
}

/// Picker opened by `^s` in `/models`: save the model as large (`model.default`), small
/// (`model.small`), or both.
pub(crate) fn model_defaults(
    reference: &str,
    roles: ModelRoles<'_>,
    scope: WriteScope,
    theme: &Theme,
) -> Modal {
    let target = scope_file(scope);
    let flag = scope_flag(scope);
    let status_note = (scope == WriteScope::Project)
        .then(|| project_status_note(roles.project_status))
        .flatten();
    let is_default = roles.default == Some(reference);
    let is_small = roles.small == Some(reference);
    let both_locked = roles.default_locked || roles.small_locked;

    let build_choice = |label: &str,
                        role: &str,
                        current: bool,
                        locked: bool,
                        current_value: Option<&str>,
                        summary: &str,
                        usage: &str| {
        let mut detail = vec![
            Line::styled(reference.to_owned(), theme.accent),
            Line::raw(""),
            Line::raw(summary.to_owned()),
            Line::raw(usage.to_owned()),
            Line::raw(""),
        ];
        if let Some(cur) = current_value.filter(|c| !c.is_empty()) {
            detail.push(field(theme, "current", cur.to_owned()));
            detail.push(Line::raw(""));
        }
        let action = if locked {
            detail.push(Line::styled("locked by the system policy", theme.warn));
            None
        } else {
            detail.push(Line::styled(format!("⏎ save to {target}"), theme.dim));
            if let Some(msg) = status_note {
                detail.push(Line::styled(msg, theme.warn));
            }
            Some(format!("/models default{flag} {role} {reference}"))
        };
        ModalItem {
            label: label.to_owned(),
            hint: if locked {
                "locked".into()
            } else if current {
                "current".into()
            } else {
                String::new()
            },
            detail,
            run: action.is_some(),
            action,
            alt: None,
            charts: Vec::new(),
        }
    };

    let mut items = vec![
        build_choice(
            "Large (default model)",
            "large",
            is_default,
            roles.default_locked,
            roles.default,
            &format!("Save as the main default model (`model.default`) in {target}."),
            "Used when starting new sessions.",
        ),
        build_choice(
            "Small (helper model)",
            "small",
            is_small,
            roles.small_locked,
            roles.small,
            &format!("Save as the small helper model (`model.small`) in {target}."),
            "Used immediately for session titles, the explore sub-agent, and cheap subtasks.",
        ),
        build_choice(
            "Both (large & small)",
            "both",
            is_default && is_small,
            both_locked,
            None,
            &format!(
                "Save as both the large default (`model.default`) and small helper (`model.small`) in {target}."
            ),
            "Uses this model for both main turns and helper subtasks.",
        ),
    ];

    if is_default && !roles.default_locked {
        items.push(ModalItem {
            label: "Clear large default".into(),
            hint: "unset".into(),
            detail: vec![
                Line::styled(reference.to_owned(), theme.accent),
                Line::raw(""),
                Line::raw(format!("Clear `model.default` in {target}.")),
            ],
            action: Some(format!("/models default{flag} large clear")),
            run: true,
            alt: None,
            charts: Vec::new(),
        });
    }
    if is_small && !roles.small_locked {
        items.push(ModalItem {
            label: "Clear small default".into(),
            hint: "unset".into(),
            detail: vec![
                Line::styled(reference.to_owned(), theme.accent),
                Line::raw(""),
                Line::raw(format!(
                    "Clear `model.small` in {target} so helper tasks use the active model."
                )),
            ],
            action: Some(format!("/models default{flag} small clear")),
            run: true,
            alt: None,
            charts: Vec::new(),
        });
    }

    let mut modal = Modal::new(
        "Default role",
        vec![ModalTab {
            title: "Role".into(),
            items,
        }],
    )
    .with_scope(scope);
    modal.footer.retain(|(key, _)| *key != "tab");
    modal
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

/// The `/tasks` modal: each sub-agent run of this session, newest first, with what it did
/// and its report.
pub(crate) fn tasks(runs: &[crate::app::AgentRun], theme: &Theme) -> Modal {
    use crate::app::AgentItem;
    let items = if runs.is_empty() {
        vec![placeholder(
            "no sub-agents yet",
            "The model starts sub-agents with its task tool, from the agents in agents.d: \
             explore for read-only searches (several at once), general for self-contained \
             changes. Each works with a fresh context and only its report comes back.",
            theme,
        )]
    } else {
        runs.iter()
            .rev()
            .map(|r| {
                let took = r.took.unwrap_or_else(|| r.started.elapsed());
                let state = match r.ok {
                    None => format!("running · {} steps", r.steps()),
                    Some(true) => r.summary.clone(),
                    Some(false) => format!("failed · {}", r.summary),
                };
                let mut detail = vec![
                    Line::styled(format!("{}: {}", r.agent, r.description), theme.accent),
                    Line::raw(""),
                    field(theme, "run", r.id.clone()),
                    field(theme, "model", r.model.clone()),
                    field(
                        theme,
                        "state",
                        format!("{state} · {}", crate::theme::elapsed(took)),
                    ),
                    Line::raw(""),
                ];
                for item in &r.items {
                    match item {
                        AgentItem::Tool(t) => {
                            let (mark, style) = match t.ok {
                                None => ("…", theme.accent),
                                Some(true) => ("✓", theme.ok),
                                Some(false) => ("✗", theme.error),
                            };
                            let result = if t.result.is_empty() {
                                String::new()
                            } else {
                                format!(" · {}", t.result)
                            };
                            detail.push(Line::from(vec![
                                Span::styled(format!("{mark} "), style),
                                Span::raw(t.summary.clone()),
                                Span::styled(result, theme.dim),
                            ]));
                        }
                        AgentItem::Text(text) => {
                            detail.push(Line::raw(""));
                            detail.extend(text.trim().lines().map(|l| Line::raw(l.to_owned())));
                            detail.push(Line::raw(""));
                        }
                    }
                }
                ModalItem {
                    label: format!("{} · {}", r.id, r.description),
                    hint: match r.ok {
                        None => "running".into(),
                        Some(true) => "done".into(),
                        Some(false) => "failed".into(),
                    },
                    detail,
                    action: None,
                    run: false,
                    alt: None,
                    charts: Vec::new(),
                }
            })
            .collect()
    };
    Modal::new(
        "Tasks",
        vec![ModalTab {
            title: "Sub-agents".into(),
            items,
        }],
    )
}

/// The `/resume` modal. `sessions` is `None` while the engine is listing them.
pub(crate) fn sessions(
    sessions: Option<&[quack_protocol::SessionSummary]>,
    theme: &Theme,
) -> Modal {
    let items = match sessions {
        None => vec![placeholder(
            "listing sessions…",
            "Reading this project's sessions.",
            theme,
        )],
        Some([]) => vec![placeholder(
            "no saved sessions",
            "Sessions are saved as you work; there are none for this project yet.",
            theme,
        )],
        Some(list) => list
            .iter()
            .map(|s| {
                let title = if s.title.is_empty() {
                    "(untitled)".to_owned()
                } else {
                    s.title.clone()
                };
                let mut detail = vec![
                    Line::styled(title.clone(), theme.accent),
                    Line::raw(""),
                    field(theme, "id", s.id.0.clone()),
                    field(theme, "started", s.started.clone()),
                    field(theme, "updated", s.age.clone()),
                    field(theme, "turns", s.turns.to_string()),
                    field(theme, "model", s.model.clone()),
                    Line::raw(""),
                ];
                detail.push(if s.current {
                    Line::styled("this session", theme.ok)
                } else {
                    Line::styled("⏎ resume it (the current session stays saved)", theme.dim)
                });
                ModalItem {
                    label: if s.current {
                        format!("{title} ●")
                    } else {
                        title
                    },
                    hint: format!("{} · {}t", s.age, s.turns),
                    detail,
                    action: (!s.current).then(|| format!("/resume {}", s.id)),
                    run: true,
                    alt: None,
                    charts: Vec::new(),
                }
            })
            .collect(),
    };
    Modal::new(
        "Sessions",
        vec![ModalTab {
            title: "Sessions".into(),
            items,
        }],
    )
}

/// The `/config` modal: every setting with its value and where it came from.
pub(crate) fn config(
    rows: &[quack_config::SettingRow],
    files: &[crate::app::ConfigLayer],
    scope: WriteScope,
    project_status: ProjectStatus,
    theme: &Theme,
) -> Modal {
    let target = scope_file(scope);
    let flag = scope_flag(scope);
    let status_note = (scope == WriteScope::Project)
        .then(|| project_status_note(project_status))
        .flatten();
    let settings = rows
        .iter()
        .map(|r| {
            let mut detail = vec![Line::styled(r.key.clone(), theme.accent), Line::raw("")];
            if !r.description.is_empty() {
                detail.push(Line::raw(r.description.clone()));
                detail.push(Line::raw(""));
            }
            detail.push(field(theme, "value", r.value.clone()));
            detail.push(field(theme, "from", r.origin.clone()));
            detail.push(Line::raw(""));
            let (action, run) = if r.locked {
                detail.push(Line::styled("locked by the system policy", theme.warn));
                (None, false)
            } else if r.is_bool {
                let flipped = if r.value == "true" { "false" } else { "true" };
                detail.push(Line::styled(
                    format!("⏎ set to {flipped} in {target}"),
                    theme.dim,
                ));
                (Some(format!("/config set{flag} {} {flipped}", r.key)), true)
            } else {
                detail.push(Line::styled(
                    format!("⏎ edit the value, then press ⏎ to save it to {target}"),
                    theme.dim,
                ));
                let prefill = if r.value == "\"\"" {
                    format!("/config set{flag} {} ", r.key)
                } else {
                    format!("/config set{flag} {} {}", r.key, r.value)
                };
                (Some(prefill), false)
            };
            if !r.locked
                && let Some(msg) = status_note
            {
                detail.push(Line::styled(msg, theme.warn));
            }
            let mut hint: String = r.value.chars().take(24).collect();
            if r.value.chars().count() > 24 {
                hint.push('…');
            }
            ModalItem {
                label: r.key.clone(),
                hint,
                detail,
                action,
                run,
                alt: None,
                charts: Vec::new(),
            }
        })
        .collect();
    let layers = files
        .iter()
        .map(|file| {
            let (hint, style) = match file.status.as_str() {
                "loaded" => ("loaded", theme.ok),
                "no config.toml" => ("missing", theme.dim),
                s if s.starts_with("failed") => ("failed", theme.warn),
                _ => ("skipped", theme.dim),
            };
            let mut detail = vec![
                Line::styled(file.layer.clone(), theme.accent),
                Line::styled(file.status.clone(), style),
                Line::raw(""),
            ];
            if let Some(path) = &file.path {
                detail.push(Line::raw(path.clone()));
            } else {
                detail.push(Line::styled("No file path for this layer", theme.dim));
            }
            ModalItem {
                label: file.layer.clone(),
                hint: hint.into(),
                detail,
                action: file.path.clone(),
                run: false,
                alt: None,
                charts: Vec::new(),
            }
        })
        .collect();
    let mut modal = Modal::new(
        "Config",
        vec![
            ModalTab {
                title: "Settings".into(),
                items: settings,
            },
            ModalTab {
                title: "Layers".into(),
                items: layers,
            },
        ],
    );
    modal.copy_tab = Some(1);
    modal.with_scope(scope)
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

/// Chart helpers shared by the `/stats` sections.
fn stat_k(n: u64) -> String {
    crate::theme::tokens(u32::try_from(n).unwrap_or(u32::MAX))
}

fn stat_time(ms: u64) -> String {
    crate::theme::elapsed(std::time::Duration::from_millis(ms))
}

fn stat_pct(n: u64, of: u64) -> u64 {
    n * 100 / of.max(1)
}

fn stat_ratio(n: u64, of: u64) -> f64 {
    n as f64 / of.max(1) as f64
}

fn stat_bars(rows: Vec<BarRow>, ascii: bool, theme: &Theme) -> Visual {
    Visual::Bars {
        rows,
        ascii,
        on_bar: theme.pill(theme.accent).bg(ratatui::style::Color::Reset),
    }
}

fn stat_spark(
    label: &str,
    caption: String,
    data: Vec<u64>,
    style: Style,
    height: u16,
    ascii: bool,
) -> Visual {
    Visual::Spark {
        label: label.to_owned(),
        caption,
        data,
        style,
        height,
        ascii,
    }
}

fn stat_meter(label: &str, text: String, ratio: f64, style: Style, ascii: bool) -> Visual {
    Visual::Meter {
        label: label.to_owned(),
        text,
        ratio,
        style,
        ascii,
    }
}

fn stat_row(label: &str, value: u64, text: String, style: Style) -> BarRow {
    BarRow {
        label: label.to_owned(),
        value,
        text,
        style,
    }
}

fn stat_series(
    r: &quack_protocol::StatsReport,
    f: impl Fn(&quack_protocol::RequestStat) -> u64,
) -> Vec<u64> {
    r.history.iter().map(f).collect()
}

fn stat_peak(v: &[u64]) -> u64 {
    v.iter().copied().max().unwrap_or(0)
}

/// The `/stats` modal. `report` is `None` until the engine answers.
pub(crate) fn stats(
    report: Option<&quack_protocol::StatsReport>,
    ascii: bool,
    theme: &Theme,
) -> Modal {
    let tabs = |items| {
        Modal::new(
            "Stats",
            vec![ModalTab {
                title: "Stats".into(),
                items,
            }],
        )
    };
    let Some(r) = report else {
        return tabs(vec![placeholder(
            "counting…",
            "Adding up this session.",
            theme,
        )]);
    };
    let since = if r.resumed {
        "Resumed: tokens and cost include earlier runs; times and tools start at the resume."
    } else {
        "Since this session started."
    };
    let calls: u32 = r.tools.iter().map(|t| t.calls).sum();
    let failed: u32 = r.tools.iter().map(|t| t.failed).sum();
    let sent = stat_series(r, |q| u64::from(q.prompt));
    let received = stat_series(r, |q| u64::from(q.completion));
    let overview = stat_overview(r, since, &sent, calls, failed, ascii, theme);
    let tokens = stat_tokens(r, sent, received, ascii, theme);
    let (cost_hint, cost) = stat_cost(r, ascii, theme);
    let timing = stat_time_section(r, ascii, theme);
    let tools = stat_tools(r, calls, failed, ascii, theme);
    tabs(vec![
        overview.item("Overview", format!("{} turns", r.turns)),
        tokens.item("Tokens", stat_k(r.prompt_tokens + r.completion_tokens)),
        cost.item("Cost", cost_hint),
        timing.item("Time", stat_time(r.open_ms)),
        tools.item("Tools", format!("{calls} calls")),
    ])
}

/// Overview: the headline figures, then how well the cache and the clock went.
fn stat_overview(
    r: &quack_protocol::StatsReport,
    since: &'static str,
    sent: &[u64],
    calls: u32,
    failed: u32,
    ascii: bool,
    theme: &Theme,
) -> Detail {
    let mut overview = Detail::default();
    overview
        .line(Line::styled(r.model.clone(), theme.accent))
        .line(Line::styled(since, theme.dim))
        .gap();
    let mut tiles = vec![
        (r.turns.to_string(), "turns".to_owned(), theme.user),
        (stat_k(r.prompt_tokens), "sent".to_owned(), theme.accent),
        (
            stat_k(r.completion_tokens),
            "received".to_owned(),
            theme.magenta,
        ),
    ];
    if let Some(c) = r.cost {
        tiles.push((dollars(c), "cost".to_owned(), theme.ok));
    }
    tiles.push((stat_time(r.open_ms), "open".to_owned(), theme.warn));
    overview.chart(Visual::Tiles(tiles));
    if r.cache_reported > 0 {
        overview.meter(stat_meter(
            "cached",
            format!("{}% of sent", stat_pct(r.cached_tokens, r.prompt_tokens)),
            stat_ratio(r.cached_tokens, r.prompt_tokens),
            theme.ok,
            ascii,
        ));
    }
    overview.meter(stat_meter(
        "model",
        format!("{}% of the time", stat_pct(r.model_ms, r.open_ms)),
        stat_ratio(r.model_ms, r.open_ms),
        theme.accent,
        ascii,
    ));
    if calls > 0 {
        let ok = calls - failed;
        overview.meter(stat_meter(
            "tools",
            format!("{ok} of {calls} ok"),
            stat_ratio(u64::from(ok), u64::from(calls)),
            if failed > 0 { theme.warn } else { theme.ok },
            ascii,
        ));
    }
    overview.gap();
    if sent.len() >= 2 {
        overview.chart(stat_spark(
            "sent",
            "by request".into(),
            sent.to_vec(),
            theme.accent,
            3,
            ascii,
        ));
    }
    overview
}

/// Tokens sent, cached and received, with their per-request history.
fn stat_tokens(
    r: &quack_protocol::StatsReport,
    sent: Vec<u64>,
    received: Vec<u64>,
    ascii: bool,
    theme: &Theme,
) -> Detail {
    let mut tokens = Detail::default();
    tokens.line(Line::styled("Tokens", theme.accent)).gap();
    let fresh = r.prompt_tokens.saturating_sub(r.cached_tokens);
    let mut rows = vec![stat_row(
        "sent",
        fresh,
        if r.cache_reported > 0 {
            format!("{} fresh", stat_k(fresh))
        } else {
            stat_k(fresh)
        },
        theme.accent,
    )];
    if r.cache_reported > 0 {
        rows.push(stat_row(
            "cached",
            r.cached_tokens,
            format!(
                "{} · {}%",
                stat_k(r.cached_tokens),
                stat_pct(r.cached_tokens, r.prompt_tokens)
            ),
            theme.ok,
        ));
    }
    rows.push(stat_row(
        "received",
        r.completion_tokens,
        stat_k(r.completion_tokens),
        theme.magenta,
    ));
    tokens.chart(stat_bars(rows, ascii, theme));
    if sent.len() >= 2 {
        tokens.chart(stat_spark(
            "sent",
            format!("peak {}", stat_k(stat_peak(&sent))),
            sent,
            theme.accent,
            3,
            ascii,
        ));
        tokens.chart(stat_spark(
            "received",
            format!("peak {}", stat_k(stat_peak(&received))),
            received,
            theme.magenta,
            2,
            ascii,
        ));
    }
    tokens
        .line(field(theme, "requests", r.requests.to_string()))
        .gap();
    tokens.line(Line::styled(
        "Sent counts every request's whole prompt, so it grows faster than the \
         context: that is what a hosted service bills.",
        theme.dim,
    ));
    if r.cache_reported == 0 && r.requests > 0 {
        tokens.line(Line::styled(
            "The server did not say how much of the prompt it had cached.",
            theme.dim,
        ));
    }
    if r.unreported > 0 {
        tokens.line(Line::styled(
            format!(
                "{} request(s) came back without usage and are not counted.",
                r.unreported
            ),
            theme.warn,
        ));
    }
    tokens
}

/// Cost figures, and the total climbing request by request.
fn stat_cost(r: &quack_protocol::StatsReport, ascii: bool, theme: &Theme) -> (String, Detail) {
    let cost_hint = r.cost.map_or_else(|| "–".to_owned(), dollars);
    let mut cost = Detail::default();
    cost.line(Line::styled("Cost", theme.accent)).gap();
    match r.cost {
        Some(c) => {
            let mut tiles = vec![(dollars(c), "total".to_owned(), theme.ok)];
            if r.turns > 0 {
                tiles.push((
                    dollars(c / f64::from(r.turns)),
                    "per turn".to_owned(),
                    theme.accent,
                ));
            }
            let priced = r.requests.saturating_sub(r.unpriced + r.unreported);
            if priced > 0 {
                tiles.push((
                    dollars(c / f64::from(priced)),
                    "per request".to_owned(),
                    theme.magenta,
                ));
            }
            cost.chart(Visual::Tiles(tiles));
            let each: Vec<f64> = r.history.iter().map(|q| q.cost.unwrap_or(0.0)).collect();
            if each.len() >= 2 {
                // Requests older than the history start the line where they left it.
                let mut total = (c - each.iter().sum::<f64>()).max(0.0);
                let points: Vec<(f64, f64)> = each
                    .iter()
                    .enumerate()
                    .map(|(i, x)| {
                        total += x;
                        ((i + 1) as f64, total)
                    })
                    .collect();
                let n = points.len();
                cost.chart(if ascii {
                    // Micro-dollars, so the sparkline has whole numbers to scale.
                    let data = points.iter().map(|p| (p.1 * 1e6) as u64).collect();
                    stat_spark("total", "by request".into(), data, theme.ok, 4, true)
                } else {
                    Visual::Trend {
                        points,
                        style: theme.ok,
                        limit: None,
                        y: ["$0".into(), dollars(c)],
                        x: ["1".into(), format!("{n} requests")],
                        height: 7,
                    }
                });
            }
            if r.unpriced > 0 {
                cost.line(Line::styled(
                    format!(
                        "{} request(s) to models without a price are left out.",
                        r.unpriced
                    ),
                    theme.warn,
                ));
            }
        }
        None => {
            cost.line(Line::raw("No price set for this model."))
                .gap()
                .line(Line::styled(
                    "To see one, add the price per million tokens to the model in its \
                     provider file:",
                    theme.dim,
                ))
                .gap()
                .line(Line::styled(
                    "cost = { input = 0.3, output = 1.2 }",
                    theme.accent,
                ));
        }
    }
    (cost_hint, cost)
}

/// Where time went, then how quick the model was request by request.
fn stat_time_section(r: &quack_protocol::StatsReport, ascii: bool, theme: &Theme) -> Detail {
    let other = r
        .open_ms
        .saturating_sub(r.model_ms + r.tools_ms + r.approval_ms);
    let total = r.open_ms.max(1);
    let mut timing = Detail::default();
    timing
        .line(Line::styled("Where the time went", theme.accent))
        .gap();
    let rows = [
        ("the model", r.model_ms, theme.accent),
        ("tools", r.tools_ms, theme.magenta),
        ("your approval", r.approval_ms, theme.warn),
        ("between turns", other, theme.dim),
    ]
    .into_iter()
    .filter(|(name, ms, _)| *ms > 0 || *name == "the model")
    .map(|(name, ms, style)| {
        stat_row(
            name,
            ms,
            format!("{} · {}%", stat_time(ms), stat_pct(ms, total)),
            style,
        )
    })
    .collect();
    timing.chart(stat_bars(rows, ascii, theme));
    let latency: Vec<u64> = r
        .history
        .iter()
        .filter_map(|q| q.ttft_ms.map(u64::from))
        .collect();
    let speeds: Vec<u64> = r
        .history
        .iter()
        .filter_map(|q| q.tokens_per_sec.map(|t| t.round() as u64))
        .collect();
    if let (Some((median, _)), true) = (r.ttft_ms, latency.len() >= 2) {
        timing.chart(stat_spark(
            "latency",
            format!("{} typ.", stat_time(u64::from(median))),
            latency,
            theme.warn,
            2,
            ascii,
        ));
    }
    if let (Some(tps), true) = (r.tokens_per_sec, speeds.len() >= 2) {
        timing.chart(stat_spark(
            "speed",
            format!("{tps:.0} t/s"),
            speeds,
            theme.ok,
            2,
            ascii,
        ));
    }
    if let Some((median, slowest)) = r.ttft_ms {
        timing.line(field(
            theme,
            "latency",
            format!(
                "{} to first token · {} slowest",
                stat_time(u64::from(median)),
                stat_time(u64::from(slowest))
            ),
        ));
    }
    if let Some(tps) = r.tokens_per_sec {
        timing.line(field(theme, "speed", format!("{tps:.0} tokens/s typical")));
    }
    let sampling = if r.sampling.is_empty() {
        "the server's defaults".to_owned()
    } else {
        let parts: Vec<String> = r.sampling.iter().map(|(k, v)| format!("{k} {v}")).collect();
        parts.join(" · ")
    };
    timing.line(field(theme, "sampling", sampling));
    timing
}

/// The busiest tools as bars in their transcript colours.
fn stat_tools(
    r: &quack_protocol::StatsReport,
    calls: u32,
    failed: u32,
    ascii: bool,
    theme: &Theme,
) -> Detail {
    let mut tools = Detail::default();
    tools.line(Line::styled("Tools", theme.accent)).gap();
    if r.tools.is_empty() {
        tools.line(Line::styled("No tools called yet.", theme.dim));
    } else {
        let rows = r
            .tools
            .iter()
            .take(TOOL_BARS)
            .map(|t| {
                let text = if t.failed > 0 {
                    format!("{} · {} failed · {}", t.calls, t.failed, stat_time(t.ms))
                } else {
                    format!("{} · {}", t.calls, stat_time(t.ms))
                };
                stat_row(&t.name, u64::from(t.calls), text, theme.tool(&t.name))
            })
            .collect();
        tools.chart(stat_bars(rows, ascii, theme));
        if r.tools.len() > TOOL_BARS {
            let rest = &r.tools[TOOL_BARS..];
            tools.line(Line::styled(
                format!(
                    "and {} more, {} calls",
                    rest.len(),
                    rest.iter().map(|t| t.calls).sum::<u32>()
                ),
                theme.dim,
            ));
            tools.gap();
        }
        tools.chart(stat_meter(
            "succeeded",
            format!("{} of {calls}", calls - failed),
            stat_ratio(u64::from(calls - failed), u64::from(calls)),
            if failed > 0 { theme.warn } else { theme.ok },
            ascii,
        ));
    }
    if r.compactions + r.pruned > 0 {
        tools
            .line(field(
                theme,
                "compacted",
                format!("{} time(s)", r.compactions),
            ))
            .line(field(
                theme,
                "pruned",
                format!("{} old result(s)", r.pruned),
            ));
    }
    tools
}

/// `$0.0042`, `$0.312`, `$12.40`.
fn dollars(x: f64) -> String {
    if x < 0.1 {
        format!("${x:.4}")
    } else if x < 10.0 {
        format!("${x:.3}")
    } else {
        format!("${x:.2}")
    }
}

/// Width of the `/context` band, in cells.
const BAND: usize = 32;

/// The `/context` modal. `report` is `None` until the engine answers; `growth` is the
/// context used at the end of each turn so far.
///
/// The overview is one band: every part of the request in its own colour and fill, the
/// free room after it, and a mark where compaction starts. Below it, what is left in
/// turns, how fast it has been growing, and what would help.
pub(crate) fn context(
    report: Option<&quack_protocol::ContextReport>,
    growth: &[u32],
    ascii: bool,
    theme: &Theme,
) -> Modal {
    let tabs = |items| {
        Modal::new(
            "Context",
            vec![ModalTab {
                title: "Context".into(),
                items,
            }],
        )
    };
    let Some(r) = report else {
        return tabs(vec![placeholder(
            "measuring…",
            "Estimating what the next request will hold.",
            theme,
        )]);
    };
    let k = crate::theme::tokens;
    let share = |n: u32, of: u32| u64::from(n) * 100 / u64::from(of.max(1));
    let fills: &[&str] = if ascii {
        &["#", "%", "=", "+", "*", "~"]
    } else {
        &["█", "▓", "▒", "▚", "▞", "░"]
    };
    let colours = [
        theme.accent,
        theme.magenta,
        theme.ok,
        theme.warn,
        theme.user,
        theme.error,
    ];
    let paint = |i: usize| (fills[i % fills.len()], colours[i % colours.len()]);

    let mut over = vec![Line::styled(r.model.clone(), theme.accent), Line::raw("")];
    if let Some(n) = &r.note {
        over.push(Line::styled(n.clone(), theme.warn));
        over.push(Line::raw(""));
    }
    let line = r.threshold.or(r.budget).filter(|l| *l > 0);
    if let Some(b) = r.budget.filter(|b| *b > 0) {
        over.push(band(r, b, &paint, ascii, theme));
        let mark = if ascii { "|" } else { "┊" };
        over.push(Line::styled(
            match r.threshold {
                Some(t) => format!("{mark} compacts at {}", k(t)),
                None => "compacts only when the server says it is too long".into(),
            },
            theme.dim,
        ));
        over.push(Line::styled(
            match r.window {
                Some(w) => format!("budget {} of a {} window", k(b), k(w)),
                None => format!("budget {}", k(b)),
            },
            theme.dim,
        ));
        over.push(Line::raw(""));
    }
    if let Some(line) = line {
        let room = line.saturating_sub(r.used);
        let pace = crate::app::turns_left(growth, room);
        let left = match pace {
            _ if room == 0 => "Past the line: the next request is pruned, then compacted.".into(),
            Some(n) => format!(
                "{} left before compaction: about {n} more turn{} at this pace.",
                k(room),
                if n == 1 { "" } else { "s" }
            ),
            None => format!("{} left before compaction.", k(room)),
        };
        over.push(Line::raw(left));
    }
    let mut charts = Vec::new();
    if growth.len() >= 2 && ascii {
        over.push(Line::from(vec![
            Span::styled("by turn   ", theme.dim),
            Span::styled(sparkline(growth, ascii), theme.accent),
        ]));
    } else if growth.len() >= 2 {
        // Context at the end of each turn, rising towards the dashed compaction line.
        over.push(Line::raw(""));
        let points: Vec<(f64, f64)> = growth
            .iter()
            .enumerate()
            .map(|(i, v)| ((i + 1) as f64, f64::from(*v)))
            .collect();
        let top = growth.iter().copied().chain(line).max().unwrap_or(0);
        charts.push((
            over.len(),
            Visual::Trend {
                points,
                style: theme.accent,
                limit: line.map(|l| (f64::from(l), theme.warn)),
                y: ["0".into(), k(top)],
                x: ["turn 1".into(), format!("turn {}", growth.len())],
                height: 6,
            },
        ));
    }
    over.push(Line::raw(""));
    over.push(Line::styled(advice(r), theme.dim));

    let mut items = vec![ModalItem {
        label: "Overview".into(),
        hint: match r.window.or(r.budget).filter(|m| *m > 0) {
            Some(max) => format!("{}/{}", k(r.used), k(max)),
            None => k(r.used),
        },
        detail: over,
        action: None,
        run: false,
        alt: None,
        charts,
    }];
    let budget = r.budget.unwrap_or(0);
    items.extend(r.segments.iter().enumerate().map(|(i, seg)| {
        let (fill, colour) = paint(i);
        let mut detail = vec![
            Line::from(vec![
                Span::styled(format!("{fill} "), colour),
                Span::styled(seg.name.clone(), theme.accent),
            ]),
            Line::raw(""),
            field(theme, "tokens", format!("~{}", seg.tokens)),
            Line::raw(""),
        ];
        // Its share of what is used and of the budget, in its band colour.
        let meter = |label: &str, of: u32| Visual::Meter {
            label: label.to_owned(),
            text: format!("{}%", share(seg.tokens, of)),
            ratio: f64::from(seg.tokens) / f64::from(of.max(1)),
            style: colour,
            ascii,
        };
        let mut charts = vec![(detail.len(), meter("of used", r.used))];
        if budget > 0 {
            charts.push((detail.len(), meter("of budget", budget)));
        }
        detail.extend([Line::raw(""), Line::raw(seg.detail.clone())]);
        ModalItem {
            // The fill matches the band, so the list doubles as its legend.
            label: format!("{fill} {}", seg.name),
            hint: k(seg.tokens),
            detail,
            action: None,
            run: false,
            alt: None,
            charts,
        }
    }));
    tabs(items)
}

/// The parts of the request side by side, then the free room, with the compaction mark.
fn band(
    r: &quack_protocol::ContextReport,
    budget: u32,
    paint: &dyn Fn(usize) -> (&'static str, Style),
    ascii: bool,
    theme: &Theme,
) -> Line<'static> {
    let cells = |n: u32| (u64::from(n) * BAND as u64 / u64::from(budget)) as usize;
    let mut spans = Vec::new();
    let mut used = 0;
    for (i, seg) in r.segments.iter().enumerate() {
        // Every part gets at least a cell, so a small one is still visible.
        let n = cells(seg.tokens)
            .max(usize::from(seg.tokens > 0))
            .min(BAND - used);
        if n == 0 {
            continue;
        }
        let (fill, colour) = paint(i);
        spans.push(Span::styled(fill.repeat(n), colour));
        used += n;
    }
    let free = if ascii { "." } else { "·" };
    let mark = r.threshold.map(cells).filter(|m| *m < BAND);
    for cell in used..BAND {
        if Some(cell) == mark {
            spans.push(Span::styled(if ascii { "|" } else { "┊" }, theme.warn));
        } else {
            spans.push(Span::styled(free, theme.border));
        }
    }
    Line::from(spans)
}

/// Context at the end of each turn, as block heights.
fn sparkline(values: &[u32], ascii: bool) -> String {
    let bars: &[char] = if ascii {
        &['_', '.', '-', '=', '#']
    } else {
        &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█']
    };
    let recent = &values[values.len().saturating_sub(BAND)..];
    let max = recent.iter().copied().max().unwrap_or(0).max(1);
    recent
        .iter()
        .map(|v| bars[(u64::from(*v) * (bars.len() as u64 - 1) / u64::from(max)) as usize])
        .collect()
}

/// One suggestion, from what is taking the room.
fn advice(r: &quack_protocol::ContextReport) -> String {
    let tokens = |name: &str| {
        r.segments
            .iter()
            .find(|s| s.name == name)
            .map_or(0, |s| s.tokens)
    };
    let fixed = tokens("System prompt") + tokens("Tool definitions");
    let results = tokens("Tool results");
    let pct = |n: u32, of: u32| u64::from(n) * 100 / u64::from(of.max(1));
    match r.budget {
        Some(b) if b > 0 && pct(fixed, b) >= 30 => format!(
            "The system prompt and tool definitions take {}% of the budget before \
             anything is said. A smaller profile or fewer MCP servers leaves more room.",
            pct(fixed, b)
        ),
        _ if r.used > 0 && pct(results, r.used) >= 50 => format!(
            "Tool results are {}% of it. Old ones are pruned on their own; \
             /compact [focus] summarizes earlier turns now.",
            pct(results, r.used)
        ),
        _ => "/compact [focus] summarizes earlier turns now; /new starts afresh.".into(),
    }
}

/// The `/memory` modal. `notes` is `None` while the engine reads them. From 80% of the
/// index cap (`fill`), the tab title says so.
pub(crate) fn memory(
    notes: Option<&[quack_protocol::MemoryNote]>,
    fill: u32,
    theme: &Theme,
) -> Modal {
    let items = match notes {
        None => vec![placeholder(
            "reading notes…",
            "Reading memory notes.",
            theme,
        )],
        Some([]) => vec![placeholder(
            "no notes yet",
            "The model saves notes with the memory tool when something will matter in later \
             sessions. Notes are markdown files you can edit.",
            theme,
        )],
        Some(list) => list
            .iter()
            .map(|n| {
                let mut detail = vec![
                    Line::styled(n.name.clone(), theme.accent),
                    Line::raw(""),
                    Line::raw(n.description.clone()),
                    Line::raw(""),
                    field(theme, "scope", n.scope.clone()),
                    field(theme, "file", n.path.clone()),
                    Line::raw(""),
                ];
                // The body, without the front matter shown above.
                let body = n
                    .text
                    .strip_prefix("---")
                    .and_then(|r| r.split_once("\n---"))
                    .map_or(n.text.as_str(), |(_, b)| b);
                detail.extend(body.trim().lines().map(|l| Line::raw(l.to_owned())));
                ModalItem {
                    label: n.name.clone(),
                    hint: n.scope.clone(),
                    detail,
                    action: None,
                    run: false,
                    alt: Some(format!("/memory delete {}/{}", n.scope, n.name)),
                    charts: Vec::new(),
                }
            })
            .collect(),
    };
    Modal::new(
        "Memory",
        vec![ModalTab {
            title: if fill >= 80 {
                format!("Notes · index {fill}% full")
            } else {
                "Notes".into()
            },
            items,
        }],
    )
    .with_alt("delete")
}

/// The `/permissions` modal: Ask/Allow approvals and the configured rules.
pub(crate) fn permissions(current: &str, rules: &[(String, String)], theme: &Theme) -> Modal {
    const MODES: &[(&str, &str)] = &[
        (
            "ask",
            "Ask before unapproved tools. Plan/Edit stays as selected; Plan only reads and searches.",
        ),
        (
            "allow",
            "Automatically approve tools within the current work mode and sandbox. Explicit ask/deny rules still apply; leaving the sandbox asks.",
        ),
    ];
    let modes = MODES
        .iter()
        .map(|(name, what)| ModalItem {
            label: if *name == current {
                format!("{name} ●")
            } else {
                (*name).to_owned()
            },
            hint: String::new(),
            detail: vec![
                Line::styled(*name, theme.accent),
                Line::raw(""),
                Line::raw(*what),
                Line::raw(""),
                Line::styled(
                    if *name == current {
                        "the current approval policy"
                    } else {
                        "⏎ select approvals, keeping Plan/Edit"
                    },
                    theme.dim,
                ),
            ],
            action: Some(format!("/permissions {name}")),
            run: true,
            alt: None,
            charts: Vec::new(),
        })
        .collect();
    let rule_items = if rules.is_empty() {
        vec![placeholder(
            "no rules",
            "Add allow, ask and deny rules under [permissions] in config.toml, e.g. \
             allow = [\"shell:cargo test*\"]. /config shows where each setting comes from.",
            theme,
        )]
    } else {
        rules
            .iter()
            .map(|(list, rule)| ModalItem {
                label: rule.clone(),
                hint: list.clone(),
                detail: vec![
                    Line::styled(rule.clone(), theme.accent),
                    Line::raw(""),
                    field(theme, "list", list.clone()),
                    Line::raw(""),
                    Line::styled(
                        "Order: Plan restrictions, built-in deny list, deny, ask, session approvals/allow, then the approval default.",
                        theme.dim,
                    ),
                ],
                action: None,
                run: false,
                alt: None,
                charts: Vec::new(),
            })
            .collect()
    };
    Modal::new(
        "Permissions",
        vec![
            ModalTab {
                title: match current {
                    "ask" | "allow" => "Approvals".into(),
                    "yolo" => "Approvals · yolo".into(),
                    _ => format!("Approvals · {current} (legacy)"),
                },
                items: modes,
            },
            ModalTab {
                title: "Rules".into(),
                items: rule_items,
            },
        ],
    )
}

/// The `/rewind` picker. `points` is `None` while the engine lists them.
pub(crate) fn rewind(points: Option<&[quack_protocol::RewindPoint]>, theme: &Theme) -> Modal {
    let items = match points {
        None => vec![placeholder("listing messages…", "", theme)],
        Some([]) => vec![placeholder(
            "no messages yet",
            "Once you have sent a message, /rewind can go back to before it.",
            theme,
        )],
        Some(points) => points
            .iter()
            .map(|p| {
                let first = p.text.lines().find(|l| !l.trim().is_empty()).unwrap_or_default();
                let mut detail = vec![Line::styled(format!("{} back", p.back), theme.accent), Line::raw("")];
                detail.extend(p.text.lines().take(12).map(|l| Line::raw(l.to_owned())));
                detail.push(Line::raw(""));
                if p.files.is_empty() {
                    detail.push(Line::styled("No files changed by write or edit since then.", theme.dim));
                } else {
                    detail.push(Line::styled("Files put back:", theme.dim));
                    detail.extend(p.files.iter().map(|f| Line::raw(format!("  {f}"))));
                }
                detail.push(Line::raw(""));
                detail.push(Line::styled(
                    format!(
                        "⏎ conversation and files · ^s conversation only · /rewind {} files for files only. \
                         Changes made by shell commands are not tracked.",
                        p.back
                    ),
                    theme.dim,
                ));
                ModalItem {
                    label: first.chars().take(100).collect(),
                    hint: match p.files.len() {
                        0 => String::new(),
                        1 => "1 file".into(),
                        n => format!("{n} files"),
                    },
                    detail,
                    action: Some(format!("/rewind {} both", p.back)),
                    run: true,
                    alt: Some(format!("/rewind {} conversation", p.back)),
                    charts: Vec::new(),
                }
            })
            .collect(),
    };
    Modal::new(
        "Rewind",
        vec![ModalTab {
            title: "Messages".into(),
            items,
        }],
    )
    .with_alt("conversation only")
}

/// The `/trust` modal: what the workspace's `.quack/` holds and the three choices.
pub(crate) fn trust(pending: Option<&TrustPrompt>, theme: &Theme) -> Modal {
    const CHOICES: &[(&str, &str, &str)] = &[
        (
            "trusted",
            "Trust",
            "Load everything: settings, providers, MCP servers, hooks and language servers. \
             MCP servers and hooks start programs from this repo's files.",
        ),
        (
            "settings-only",
            "Trust settings only",
            "Load config.toml, but nothing that starts a program or adds a provider.",
        ),
        (
            "ignored",
            "Ignore",
            "Never load this repo's .quack/, and don't ask again.",
        ),
    ];
    let mut about = match pending {
        Some(p) => vec![
            Line::styled(
                if p.changed {
                    "Changed since you trusted it"
                } else {
                    "Project config found"
                },
                theme.warn,
            ),
            Line::raw(""),
            field(theme, "folder", p.path.clone()),
            field(theme, "holds", p.summary.clone()),
        ],
        None => vec![
            Line::styled("This workspace's .quack/", theme.accent),
            Line::raw(""),
            Line::raw("No decision is waiting. Choosing again replaces the stored one."),
        ],
    };
    about.push(Line::raw(""));
    about.push(Line::styled(
        "Decisions are kept in trust.toml in the user data folder. If the files change, \
         you are asked again.",
        theme.dim,
    ));
    let mut items: Vec<ModalItem> = CHOICES
        .iter()
        .map(|(id, label, what)| {
            let mut detail = vec![
                Line::styled(*label, theme.accent),
                Line::raw(""),
                Line::raw(*what),
                Line::raw(""),
            ];
            detail.extend(about.iter().cloned());
            ModalItem {
                label: (*label).to_owned(),
                hint: String::new(),
                detail,
                action: Some(format!("/trust {id}")),
                run: true,
                alt: None,
                charts: Vec::new(),
            }
        })
        .collect();
    items.push(ModalItem {
        label: "Decide later".into(),
        hint: String::new(),
        detail: vec![
            Line::styled("Decide later", theme.accent),
            Line::raw(""),
            Line::raw("Keep skipping it for now. /trust brings this back."),
        ],
        action: Some("/trust later".into()),
        run: true,
        alt: None,
        charts: Vec::new(),
    });
    Modal::new(
        "Trust",
        vec![ModalTab {
            title: "Project".into(),
            items,
        }],
    )
}

/// A `.quack/` waiting for a decision (`Event::ProjectTrust`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrustPrompt {
    pub path: String,
    pub summary: String,
    pub changed: bool,
}

/// The `/doctor` modal. `checks` is `None` while they run.
pub(crate) fn doctor(checks: Option<&[quack_protocol::DoctorCheck]>, theme: &Theme) -> Modal {
    use quack_protocol::CheckStatus;
    let items = match checks {
        None => vec![placeholder(
            "checking…",
            "Running environment checks and contacting each configured provider.",
            theme,
        )],
        Some(list) => {
            let paint = |status: &CheckStatus| match status {
                CheckStatus::Ok => ("ok", theme.ok),
                CheckStatus::Info => ("info", theme.dim),
                CheckStatus::Warn => ("warn", theme.warn),
                CheckStatus::Fail => ("FAIL", theme.error),
            };
            let count = |s: CheckStatus| list.iter().filter(|c| c.status == s).count();
            let (ok, warn, fail) = (
                count(CheckStatus::Ok),
                count(CheckStatus::Warn),
                count(CheckStatus::Fail),
            );
            let mut summary = Detail::default();
            summary
                .line(Line::styled("Environment checks", theme.accent))
                .gap()
                .chart(Visual::Tiles(vec![
                    (ok.to_string(), "passed".into(), theme.ok),
                    (warn.to_string(), "warnings".into(), theme.warn),
                    (fail.to_string(), "failed".into(), theme.error),
                ]))
                .line(Line::styled(
                    match (fail, warn) {
                        (0, 0) => "Everything checks out.",
                        (0, _) => "Nothing is broken; the warnings say what could be better.",
                        _ => "Pick a failed check to see what to do about it.",
                    },
                    theme.dim,
                ));
            let hint = match (fail, warn) {
                (0, 0) => "all ok".to_owned(),
                (0, w) => format!("{w} warn"),
                (f, _) => format!("{f} failed"),
            };
            let mut items = vec![summary.item("Summary", hint)];
            items.extend(list.iter().map(|c| {
                let (hint, style) = paint(&c.status);
                let mut detail = vec![
                    Line::styled(c.name.trim().to_owned(), theme.accent),
                    Line::raw(""),
                    Line::from(Span::styled(format!(" {hint} "), theme.pill(style))),
                    Line::raw(""),
                ];
                detail.extend(c.detail.lines().map(|l| Line::raw(l.to_owned())));
                ModalItem {
                    label: c.name.clone(),
                    hint: hint.to_owned(),
                    detail,
                    action: None,
                    run: false,
                    alt: None,
                    charts: Vec::new(),
                }
            }));
            items
        }
    };
    Modal::new(
        "Doctor",
        vec![ModalTab {
            title: "Checks".into(),
            items,
        }],
    )
}

/// The `/probe` modal: synthetic tool-calling capability probe report.
pub(crate) fn tool_probe(report: Option<&quack_protocol::ToolProbeReport>, theme: &Theme) -> Modal {
    let items = match report {
        None => vec![placeholder(
            "probing…",
            "Sending minimal synthetic probe calls to verify tool-calling and reasoning compatibility.",
            theme,
        )],
        Some(report) => {
            let passed = report.cases.iter().filter(|c| c.passed).count();
            let total = report.cases.len();
            let mut summary = Detail::default();
            summary
                .line(Line::styled(
                    format!("Model: {}", report.model),
                    theme.accent,
                ))
                .gap()
                .chart(Visual::Tiles(vec![
                    (
                        format!("{passed}/{total}"),
                        "passed".into(),
                        if passed == total {
                            theme.ok
                        } else if passed > 0 {
                            theme.warn
                        } else {
                            theme.error
                        },
                    ),
                    (
                        report.configured_mode.clone(),
                        "configured".into(),
                        theme.dim,
                    ),
                    (
                        report.recommended_mode.clone(),
                        "recommended".into(),
                        theme.accent,
                    ),
                ]));
            if let Some(fmt) = &report.recommended_text_format {
                summary.line(Line::raw(format!("Text fallback parser: {fmt}")));
                summary.gap();
            }
            for line in report.summary.lines() {
                summary.line(Line::styled(line.to_owned(), theme.dim));
            }
            let mut items = vec![summary.item("Summary", report.recommended_mode.clone())];
            items.extend(report.cases.iter().map(|c| {
                let (badge, style) = if c.passed {
                    ("pass", theme.ok)
                } else {
                    ("FAIL", theme.error)
                };
                let mut detail = vec![
                    Line::styled(c.name.clone(), theme.accent),
                    Line::raw(""),
                    Line::from(Span::styled(
                        format!(" {badge} · {} ms ", c.latency_ms),
                        theme.pill(style),
                    )),
                    Line::raw(""),
                ];
                detail.extend(c.detail.lines().map(|l| Line::raw(l.to_owned())));
                ModalItem {
                    label: c.name.clone(),
                    hint: format!("{badge} · {} ms", c.latency_ms),
                    detail,
                    action: None,
                    run: false,
                    alt: None,
                    charts: Vec::new(),
                }
            }));
            items
        }
    };
    Modal::new(
        "Tool Probe",
        vec![ModalTab {
            title: "Probe".into(),
            items,
        }],
    )
}

/// The `/providers` modal: each provider file with its state. `None` while probing.
/// Where a key is read from, as the end of "read from …".
pub(crate) fn key_source(key: &quack_protocol::KeySource) -> String {
    use quack_protocol::KeySource;
    match key {
        KeySource::Keyring { name } => format!("the OS credential store (`{name}`)"),
        KeySource::Env { var } => format!("the {var} environment variable"),
        KeySource::File { path } => format!("the file {path}"),
        KeySource::Cmd => "a command in its provider file".into(),
        KeySource::Value => "its provider file".into(),
    }
}

pub(crate) fn provider_label_id(label: &str) -> &str {
    label.strip_suffix(" ●").unwrap_or(label)
}

pub(crate) fn providers(
    catalog: Option<&[quack_protocol::ProviderModels]>,
    connections: Option<&[quack_protocol::ProviderConnection]>,
    current: Option<&str>,
    theme: &Theme,
) -> Modal {
    let items = match catalog {
        None => match connections.filter(|list| !list.is_empty()) {
            Some(list) => list
                .iter()
                .map(|conn| {
                    let in_use = current.is_some_and(|c| c.starts_with(&format!("{}/", conn.id)));
                    let mut detail = vec![
                        Line::styled(conn.id.clone(), theme.accent),
                        Line::raw(""),
                        Line::styled("checking models…", theme.dim),
                    ];
                    if in_use {
                        detail.push(Line::styled("the current model is from here", theme.dim));
                    }
                    detail.push(Line::raw(""));
                    detail.push(field(theme, "HTTP", provider_http_version(conn).into()));
                    detail.push(field(theme, "proxy", provider_proxy(conn).into()));
                    detail.push(Line::raw(""));
                    detail.push(Line::styled("⏎ or ^s edit connection settings", theme.dim));
                    let edit = format!("/providers advanced {}", conn.id);
                    ModalItem {
                        label: if in_use {
                            format!("{} ●", conn.id)
                        } else {
                            conn.id.clone()
                        },
                        hint: "checking…".into(),
                        detail,
                        run: true,
                        action: Some(edit.clone()),
                        alt: Some(edit),
                        charts: Vec::new(),
                    }
                })
                .collect(),
            None => vec![placeholder(
                "checking providers…",
                "Asking each provider for its models.",
                theme,
            )],
        },
        Some([]) => vec![placeholder(
            "no providers",
            "/setup adds one: a server on this machine, a hosted service with its key, or \
             any server by its URL. Or add a TOML file with a base_url to providers.d in \
             your config folder (`quack config paths` shows where).",
            theme,
        )],
        Some(list) => list
            .iter()
            .map(|p| {
                let in_use = current.is_some_and(|c| c.starts_with(&format!("{}/", p.id)));
                let conn = connections.and_then(|cs| cs.iter().find(|c| c.id == p.id));
                let mut detail = vec![
                    Line::styled(p.id.clone(), theme.accent),
                    Line::raw(""),
                    if p.available {
                        Line::styled("usable", theme.ok)
                    } else if p.bundled {
                        Line::styled("offered by the defaults bundle; not set up", theme.dim)
                    } else {
                        Line::styled("unavailable", theme.warn)
                    },
                ];
                if !p.note.is_empty() {
                    detail.push(Line::raw(p.note.clone()));
                }
                if in_use {
                    detail.push(Line::styled("the current model is from here", theme.dim));
                }
                if p.key.is_some() || conn.is_some() || connections.is_none() {
                    detail.push(Line::raw(""));
                }
                if let Some(key) = &p.key {
                    detail.push(field(theme, "key", key_source(key)));
                }
                if let Some(conn) = conn {
                    detail.push(field(theme, "HTTP", provider_http_version(conn).into()));
                    detail.push(field(theme, "proxy", provider_proxy(conn).into()));
                } else if connections.is_none() {
                    detail.push(field(theme, "network", "loading…".into()));
                }
                match (&p.key, conn) {
                    (Some(_), Some(_)) => {
                        detail.push(Line::raw(""));
                        detail.push(Line::styled(
                            "⏎ replace its key · ^s connection settings",
                            theme.dim,
                        ));
                    }
                    (Some(_), None) => {
                        detail.push(Line::styled(
                            "⏎ replace its key, for example after rotating it",
                            theme.dim,
                        ));
                    }
                    (None, Some(_)) => {
                        detail.push(Line::raw(""));
                        detail.push(Line::styled("⏎ or ^s edit connection settings", theme.dim));
                    }
                    (None, None) => {}
                }
                if !p.models.is_empty() {
                    detail.push(Line::raw(""));
                    detail.extend(p.models.iter().take(30).map(|m| {
                        Line::raw(format!(
                            "  {}{}",
                            m.alias
                                .as_ref()
                                .map_or_else(|| m.id.clone(), |a| format!("{a} ({})", m.id)),
                            m.context_window.map_or_else(String::new, |c| format!(
                                " · {}",
                                crate::theme::tokens(c)
                            ))
                        ))
                    }));
                    if p.models.len() > 30 {
                        detail.push(Line::styled(
                            format!("  … {} more in /models", p.models.len() - 30),
                            theme.dim,
                        ));
                    }
                }
                let edit_connection = conn.map(|_| format!("/providers advanced {}", p.id));
                let action = if p.key.is_some() {
                    Some(format!("/key {}", p.id))
                } else {
                    edit_connection.clone()
                };
                ModalItem {
                    label: if in_use {
                        format!("{} ●", p.id)
                    } else {
                        p.id.clone()
                    },
                    hint: if p.available {
                        format!("{} models", p.models.len())
                    } else if p.bundled {
                        "not set up".into()
                    } else {
                        "unavailable".into()
                    },
                    detail,
                    run: action.is_some(),
                    action,
                    alt: edit_connection,
                    charts: Vec::new(),
                }
            })
            .collect(),
    };
    Modal::new(
        "Providers",
        vec![ModalTab {
            title: "Providers".into(),
            items,
        }],
    )
    .with_alt("connection")
}

fn provider_http_version(p: &quack_protocol::ProviderConnection) -> &'static str {
    use quack_protocol::ProviderHttpVersion::*;
    match p.http_version {
        Inherit if p.inherited_http1_only => "Inherited · HTTP/1.1 only",
        Inherit => "Inherited · Negotiate",
        Http1Only => "HTTP/1.1 only",
        Negotiate => "Negotiate",
    }
}

fn provider_proxy(p: &quack_protocol::ProviderConnection) -> &'static str {
    if p.no_proxy {
        "Direct connection"
    } else {
        "Use configured proxy settings"
    }
}

pub(crate) fn provider_connection(p: &quack_protocol::ProviderConnection, theme: &Theme) -> Modal {
    use quack_protocol::ProviderHttpVersion::*;
    let option = |label: &str, setting: &str, selected, detail: &str| ModalItem {
        label: label.into(),
        hint: if selected {
            "current".into()
        } else {
            String::new()
        },
        detail: vec![
            Line::styled(p.id.clone(), theme.accent),
            Line::raw(""),
            field(theme, "HTTP", provider_http_version(p).into()),
            field(theme, "proxy", provider_proxy(p).into()),
            Line::raw(""),
            Line::raw(detail.to_owned()),
            Line::raw(""),
            Line::styled("⏎ save setting · esc back to providers", theme.dim),
        ],
        action: Some(format!("/providers {setting} {}", p.id)),
        run: true,
        alt: None,
        charts: Vec::new(),
    };
    Modal::grouped(
        format!("Connection: {}", p.id),
        vec![
            (
                "HTTP version",
                vec![
                    option(
                        if p.inherited_http1_only {
                            "Inherit (HTTP/1.1)"
                        } else {
                            "Inherit (Negotiate)"
                        },
                        "http-inherit",
                        p.http_version == Inherit,
                        "Remove this file's HTTP version override.",
                    ),
                    option(
                        "HTTP/1.1 only",
                        "http-1",
                        p.http_version == Http1Only,
                        "Try this when HTTP/2 connections stall behind a VPN or proxy.",
                    ),
                    option(
                        "Negotiate",
                        "http-negotiate",
                        p.http_version == Negotiate,
                        "Let quack and the server negotiate the HTTP version.",
                    ),
                ],
            ),
            (
                "Proxy",
                vec![
                    option(
                        "Configured proxy",
                        "proxy-inherit",
                        !p.no_proxy,
                        "Use the provider, global, environment or system proxy settings.",
                    ),
                    option(
                        "Direct connection",
                        "proxy-direct",
                        p.no_proxy,
                        "Bypass proxies for this provider. Use this when your VPN routes to the provider directly.",
                    ),
                ],
            ),
        ],
    )
}

/// The `/skills` modal: installed skills; ⏎ puts `/<name> ` in the input.
pub(crate) fn bundles(bundles: &[quack_config::Bundle], scope: WriteScope, theme: &Theme) -> Modal {
    let target = scope_file(scope);
    let flag = scope_flag(scope);
    let items = if bundles.is_empty() {
        vec![placeholder(
            "no bundles",
            "A bundle is a folder with a bundle.toml that groups skills, commands and \
             mcp.d, hooks.d or profiles.d files. Copy one into bundles/ in your config folder \
             (`quack config paths` shows where) or a trusted project's .quack/.",
            theme,
        )]
    } else {
        bundles
            .iter()
            .map(|b| {
                let name = if b.file.name.is_empty() {
                    b.id.clone()
                } else {
                    b.file.name.clone()
                };
                let (state, style) = if b.enabled {
                    ("on", theme.ok)
                } else {
                    ("off", theme.dim)
                };
                let mut detail = vec![Line::styled(name.clone(), style), Line::raw("")];
                if !b.file.description.is_empty() {
                    detail.push(Line::raw(b.file.description.clone()));
                    detail.push(Line::raw(""));
                }
                detail.push(field(theme, "state", state.to_owned()));
                if !b.file.version.is_empty() {
                    detail.push(field(theme, "version", b.file.version.clone()));
                }
                detail.push(field(theme, "holds", b.contents.clone()));
                detail.push(field(theme, "layer", b.layer.to_string()));
                detail.push(field(theme, "folder", b.dir.display().to_string()));
                detail.push(Line::raw(""));
                detail.push(Line::styled(
                    format!(
                        "⏎ turn it {} (saved to [bundles] disabled in {target})",
                        if b.enabled { "off" } else { "on" }
                    ),
                    theme.dim,
                ));
                ModalItem {
                    label: name,
                    hint: format!("{state} · {}", b.layer),
                    detail,
                    action: Some(format!("/bundles toggle{flag} {}", b.id)),
                    run: true,
                    alt: None,
                    charts: Vec::new(),
                }
            })
            .collect()
    };
    Modal::new(
        "Bundles",
        vec![ModalTab {
            title: "Bundles".into(),
            items,
        }],
    )
    .with_scope(scope)
}

/// The `/theme` modal: switch between `auto`, `dark` and `light`.
pub(crate) fn theme(current: &str, scope: WriteScope, theme: &Theme) -> Modal {
    let target = scope_file(scope);
    let flag = scope_flag(scope);
    const THEMES: [(&str, &str); 3] = [
        (
            "auto",
            "Follows the terminal's background (`COLORFGBG` when set, otherwise dark).",
        ),
        (
            "dark",
            "High-contrast palette for dark terminal backgrounds.",
        ),
        (
            "light",
            "High-contrast palette for light terminal backgrounds.",
        ),
    ];
    let items = THEMES
        .iter()
        .map(|(id, about)| {
            let active = *id == current;
            ModalItem {
                label: (*id).to_owned(),
                hint: if active {
                    "in use".into()
                } else {
                    String::new()
                },
                detail: vec![
                    Line::styled(*id, theme.accent),
                    Line::raw(""),
                    Line::raw(*about),
                    Line::raw(""),
                    Line::styled(
                        if active {
                            "in use".into()
                        } else {
                            format!("⏎ use this theme (saved to [ui] theme in {target})")
                        },
                        if active { theme.ok } else { theme.dim },
                    ),
                ],
                action: Some(format!("/theme{flag} {id}")),
                run: true,
                alt: None,
                charts: Vec::new(),
            }
        })
        .collect();
    Modal::new(
        "Theme",
        vec![ModalTab {
            title: "Themes".into(),
            items,
        }],
    )
    .with_scope(scope)
}

/// The `/hooks` modal: configured hooks from `hooks.d` and their recent runs.
pub(crate) fn hook_payload(id: &str, path: &str, theme: &Theme) -> Modal {
    let items = [
        ("Keep existing folder", "keep-payload"),
        ("Replace with bundled folder", "replace-payload"),
    ]
    .into_iter()
    .map(|(label, action)| {
        let mut item = placeholder(
            label,
            &format!("An existing payload for `{id}` was found at {path}."),
            theme,
        );
        item.action = Some(format!("/hooks {action} {id}"));
        item.run = true;
        item
    })
    .collect();
    Modal::new(
        "Hook payload",
        vec![ModalTab {
            title: "Hook payload".into(),
            items,
        }],
    )
}

pub(crate) fn hooks(
    hooks: &[quack_config::Item<quack_config::HookFile>],
    runs: &[crate::app::HookRun],
    scope: WriteScope,
    theme: &Theme,
) -> Modal {
    use quack_config::{ItemState, LayerKind};
    let target = scope_file(scope);
    let flag = scope_flag(scope);
    let items = if hooks.is_empty() {
        vec![placeholder(
            "no hooks",
            "Add a TOML file to hooks.d in your config folder (`quack config paths` shows \
             where) or a trusted project's .quack/hooks.d, for example\n\n\
             event = \"pre_tool\"\nmatch = { tool = \"shell\" }\ncommand = [\"python3\", \"check.py\"]",
            theme,
        )]
    } else {
        hooks
            .iter()
            .map(|h| {
                let hook_runs: Vec<&crate::app::HookRun> =
                    runs.iter().filter(|r| r.id == h.id).collect();
                let last = hook_runs.last().copied();
                let mut detail = Vec::new();
                let (hint, can_toggle, enabled) = match &h.state {
                    ItemState::Loaded(f) if f.enabled => {
                        detail.push(Line::styled(h.id.clone(), theme.ok));
                        detail.push(Line::raw(""));
                        if !f.description.is_empty() {
                            detail.push(Line::raw(f.description.clone()));
                            detail.push(Line::raw(""));
                        }
                        detail.push(field(theme, "state", "on".into()));
                        detail.push(field(theme, "event", f.event.label().into()));
                        detail.push(field(theme, "command", f.command.join(" ")));
                        if !f.matcher.tool.is_empty() {
                            detail.push(field(theme, "tool", f.matcher.tool.clone()));
                        }
                        if !f.matcher.pattern.is_empty() {
                            detail.push(field(theme, "pattern", f.matcher.pattern.clone()));
                        }
                        if !f.matcher.prompt.is_empty() {
                            detail.push(field(theme, "prompt", f.matcher.prompt.clone()));
                        }
                        let hint = match last {
                            Some(r) => format!("{} · {} ms", f.event.label(), r.ms),
                            None => format!("{} · {}", f.event.label(), h.layer.label()),
                        };
                        (hint, h.layer != LayerKind::System, true)
                    }
                    ItemState::Loaded(f) => {
                        detail.push(Line::styled(h.id.clone(), theme.dim));
                        detail.push(Line::raw(""));
                        detail.push(field(theme, "state", "disabled in file".into()));
                        detail.push(field(theme, "event", f.event.label().into()));
                        detail.push(field(theme, "command", f.command.join(" ")));
                        ("off".into(), h.layer != LayerKind::System, false)
                    }
                    ItemState::Disabled(why) => {
                        detail.push(Line::styled(h.id.clone(), theme.dim));
                        detail.push(Line::raw(""));
                        detail.push(field(theme, "state", format!("off ({why})")));
                        ("off".into(), h.layer != LayerKind::System, false)
                    }
                    ItemState::Failed(d) => {
                        detail.push(Line::styled(h.id.clone(), theme.error));
                        detail.push(Line::raw(""));
                        detail.push(Line::styled(d.one_line(), theme.error));
                        ("failed".into(), false, false)
                    }
                };
                let action = if can_toggle {
                    if enabled {
                        format!(
                            "{}disables by adding `{}` to [hooks] disabled in {target}.",
                            if matches!(&h.state, ItemState::Loaded(f) if f.setup.is_none()) {
                                "Enter or Ctrl-S "
                            } else {
                                "Enter configures; Ctrl-S "
                            },
                            h.id,
                        )
                    } else if matches!(&h.state, ItemState::Loaded(f) if f.setup.is_none()) {
                        "Enter or Ctrl-S enables this hook.".to_owned()
                    } else {
                        "Enter configures the hook; Ctrl-S enables it.".to_owned()
                    }
                } else {
                    String::new()
                };
                detail.push(field(theme, "layer", h.layer.label().into()));
                detail.push(field(theme, "file", h.path.display().to_string()));
                if !hook_runs.is_empty() {
                    detail.push(Line::raw(""));
                    detail.push(Line::styled("recent runs", theme.dim));
                    let skip = hook_runs.len().saturating_sub(10);
                    for r in hook_runs.into_iter().skip(skip) {
                        let msg = r
                            .message
                            .as_deref()
                            .map(|m| format!(" · {m}"))
                            .unwrap_or_default();
                        detail.push(Line::raw(format!(
                            "  {} → {} ({} ms){msg}",
                            r.on, r.outcome, r.ms
                        )));
                    }
                }
                if can_toggle {
                    detail.push(Line::raw(""));
                    detail.push(Line::styled(action, theme.dim));
                } else if h.layer == LayerKind::System {
                    detail.push(Line::raw(""));
                    detail.push(Line::styled("Enforced by the system layer.", theme.dim));
                }
                ModalItem {
                    label: h.id.clone(),
                    hint,
                    detail,
                    action: can_toggle.then(|| {
                        if matches!(&h.state, ItemState::Loaded(f) if f.setup.is_none()) {
                            format!("/hooks toggle{flag} {}", h.id)
                        } else {
                            format!("/hooks configure {}", h.id)
                        }
                    }),
                    run: can_toggle,
                    alt: can_toggle.then(|| format!("/hooks toggle{flag} {}", h.id)),
                    charts: Vec::new(),
                }
            })
            .collect()
    };
    Modal::new(
        "Hooks",
        vec![ModalTab {
            title: "Hooks".into(),
            items,
        }],
    )
    .with_alt("enable/disable")
    .with_scope(scope)
}

pub(crate) fn skills(skills: &[quack_protocol::SkillInfo], theme: &Theme) -> Modal {
    let items = if skills.is_empty() {
        vec![placeholder(
            "no skills",
            "Put a skill folder with a SKILL.md in .agents/skills (this project), \
             ~/.agents/skills or skills/ in your config folder.",
            theme,
        )]
    } else {
        skills
            .iter()
            .map(|s| ModalItem {
                label: s.name.clone(),
                hint: s.source.split(' ').next().unwrap_or_default().to_owned(),
                detail: vec![
                    Line::styled(format!("/{}", s.name), theme.accent),
                    Line::raw(""),
                    Line::raw(s.description.clone()),
                    Line::raw(""),
                    field(theme, "from", s.source.clone()),
                    field(theme, "folder", s.dir.clone()),
                    Line::raw(""),
                    Line::styled(
                        "⏎ run it with a request; the model also loads it when relevant",
                        theme.dim,
                    ),
                ],
                action: Some(format!("/{} ", s.name)),
                run: false,
                alt: None,
                charts: Vec::new(),
            })
            .collect()
    };
    Modal::new(
        "Skills",
        vec![ModalTab {
            title: "Skills".into(),
            items,
        }],
    )
}

/// The `/instructions` modal: the AGENTS.md files the model has, in the system prompt or
/// attached when it first worked in their folder. `None` while asking the engine.
pub(crate) fn instructions(
    files: Option<&[quack_protocol::InstructionFile]>,
    theme: &Theme,
) -> Modal {
    let items = match files {
        None => vec![placeholder("reading…", "", theme)],
        Some([]) => vec![placeholder(
            "no instruction files",
            "None was found. quack reads AGENTS.md (and the other names in \
             `[instructions] files`) from your config folder, then from each folder between \
             the git root and the working directory. Files in subfolders are added when the \
             model first works there. /init writes one for this project.",
            theme,
        )],
        Some(files) => files
            .iter()
            .map(|f| {
                let mut detail = vec![
                    Line::styled(f.source.clone(), theme.accent),
                    Line::raw(""),
                    field(
                        theme,
                        "where",
                        if f.in_prompt {
                            "in the system prompt".into()
                        } else {
                            "added to a tool result in its folder".into()
                        },
                    ),
                    field(theme, "size", format!("~{} tokens", f.tokens)),
                    Line::raw(""),
                ];
                detail.extend(f.text.lines().map(|l| Line::raw(l.to_owned())));
                ModalItem {
                    label: f.source.clone(),
                    hint: crate::theme::tokens(f.tokens),
                    detail,
                    action: None,
                    run: false,
                    alt: None,
                    charts: Vec::new(),
                }
            })
            .collect(),
    };
    Modal::new(
        "Instructions",
        vec![ModalTab {
            title: "Instructions".into(),
            items,
        }],
    )
}

/// What `/profiles` shows: the profile in use, whether it was chosen for the session,
/// and every installed profile.
pub(crate) type ProfileList<'a> = (&'a str, bool, &'a [quack_protocol::ProfileInfo]);

/// The `/profiles` modal: each profile previewed for the current model. `None` while
/// asking the engine.
pub(crate) fn profiles(list: Option<ProfileList<'_>>, scope: WriteScope, theme: &Theme) -> Modal {
    let target = scope_file(scope);
    let flag = scope_flag(scope);
    let Some((active, forced, profiles)) = list else {
        return Modal::new(
            "Profiles",
            vec![ModalTab {
                title: "Profiles".into(),
                items: vec![placeholder("rendering profiles…", "", theme)],
            }],
        )
        .with_alt("make default")
        .with_scope(scope);
    };
    let mut items: Vec<ModalItem> = profiles
        .iter()
        .map(|p| {
            let hint = match (p.id == active, p.matches) {
                (true, _) => "in use".into(),
                (false, true) => "auto pick".into(),
                _ => String::new(),
            };
            let mut detail = vec![
                Line::styled(p.id.clone(), theme.accent),
                Line::raw(""),
                Line::raw(p.description.clone()),
                Line::raw(""),
                field(theme, "source", p.source.clone()),
                field(theme, "edits", p.edit_format.replace('_', " ")),
                field(theme, "tools", p.tools.join(", ")),
                Line::raw(""),
                Line::styled(
                    format!("⏎ use for session · ^s set default ({target})"),
                    theme.dim,
                ),
                Line::raw(""),
                Line::styled("System prompt, rendered for this model", theme.dim),
            ];
            detail.extend(p.system.lines().map(|l| Line::raw(l.to_owned())));
            ModalItem {
                label: p.id.clone(),
                hint,
                detail,
                action: Some(format!("/profiles {}", p.id)),
                run: true,
                alt: Some(format!("/profiles default{flag} {}", p.id)),
                charts: Vec::new(),
            }
        })
        .collect();
    items.push(ModalItem {
        label: "auto".into(),
        hint: if forced { String::new() } else { "on".into() },
        detail: vec![
            Line::styled("auto", theme.accent),
            Line::raw(""),
            Line::raw(
                "Each model's own profile: the one its provider file names, else the \
                 best `[match]` for its id, else `generic`.",
            ),
            Line::raw(""),
            Line::styled("⏎ go back to it for this session", theme.dim),
        ],
        action: Some("/profiles auto".into()),
        run: true,
        alt: None,
        charts: Vec::new(),
    });
    Modal::new(
        "Profiles",
        vec![ModalTab {
            title: "Profiles".into(),
            items,
        }],
    )
    .with_alt("make default")
    .with_scope(scope)
}

/// The `/commands` modal: custom slash commands from the `commands/` folders.
pub(crate) fn custom_commands(commands: &[quack_protocol::CommandInfo], theme: &Theme) -> Modal {
    let items = if commands.is_empty() {
        vec![placeholder(
            "no custom commands",
            "Put a markdown prompt in commands/ in your config folder (or .quack/commands in \
             a trusted project): commands/review.md is /review, commands/test/flaky.md is \
             /test:flaky. {{ args }} in the file becomes what you type after the name; \
             front matter can give a `description` and `args`. Prompts of connected MCP \
             servers show here too, as /<server>:<prompt>.",
            theme,
        )]
    } else {
        commands
            .iter()
            .map(|c| ModalItem {
                label: format!("/{}", c.name),
                hint: c.source.clone(),
                detail: [
                    Line::styled(
                        format!("/{} {}", c.name, c.args).trim_end().to_owned(),
                        theme.accent,
                    ),
                    Line::raw(""),
                    Line::raw(c.description.clone()),
                    Line::raw(""),
                    field(theme, "from", c.source.clone()),
                ]
                .into_iter()
                // MCP prompts have no file.
                .chain((!c.path.is_empty()).then(|| field(theme, "file", c.path.clone())))
                .chain([
                    Line::raw(""),
                    Line::styled("⏎ type it with a request", theme.dim),
                ])
                .collect(),
                action: Some(format!("/{} ", c.name)),
                run: false,
                alt: None,
                charts: Vec::new(),
            })
            .collect()
    };
    Modal::new(
        "Commands",
        vec![ModalTab {
            title: "Commands".into(),
            items,
        }],
    )
}

/// The `/mcp` modal: live servers, disabled/bundled examples, and quick-add actions.
/// `servers` is `None` while asking the engine.
pub(crate) fn mcp(
    servers: Option<&[quack_protocol::McpServerInfo]>,
    catalog: &[quack_config::McpSummary],
    scope: WriteScope,
    theme: &Theme,
) -> Modal {
    let items = match servers {
        None => vec![placeholder("checking servers…", "", theme)],
        Some(list) => {
            let mut out: Vec<ModalItem> = list
                .iter()
                .map(|s| {
                    let style = match s.state {
                        quack_protocol::McpServerState::Ready => theme.ok,
                        quack_protocol::McpServerState::Failed => theme.error,
                        _ => theme.warn,
                    };
                    let mut detail = vec![Line::styled(s.id.clone(), style), Line::raw("")];
                    if !s.description.is_empty() {
                        detail.push(Line::raw(s.description.clone()));
                        detail.push(Line::raw(""));
                    }
                    detail.push(field(theme, "state", s.state.label().to_owned()));
                    if !s.error.is_empty() {
                        detail.push(Line::styled(s.error.clone(), theme.error));
                    }
                    detail.push(field(theme, s.transport.as_str(), s.target.clone()));
                    if s.state == quack_protocol::McpServerState::Ready {
                        detail.push(field(
                            theme,
                            "tools",
                            format!(
                                "{} · ~{} tokens{}",
                                s.tools.len(),
                                s.tool_tokens,
                                if s.deferred {
                                    " · found with tool_search (too large to send every time)"
                                } else {
                                    ""
                                }
                            ),
                        ));
                        detail.push(Line::raw(""));
                        detail.extend(s.tools.iter().map(|t| Line::raw(format!("  {t}"))));
                        if !s.prompts.is_empty() {
                            detail.push(Line::raw(""));
                            detail.push(field(theme, "prompts", s.prompts.len().to_string()));
                            detail.extend(
                                s.prompts
                                    .iter()
                                    .map(|p| Line::raw(format!("  /{}:{p}", s.id))),
                            );
                        }
                        if !s.resources.is_empty() {
                            detail.push(Line::raw(""));
                            detail.push(field(theme, "resources", s.resources.len().to_string()));
                            detail.extend(s.resources.iter().map(|r| Line::raw(format!("  {r}"))));
                        }
                    }
                    if !s.log.is_empty() {
                        detail.push(Line::raw(""));
                        detail.push(Line::styled("recent output", theme.dim));
                        let skip = s.log.len().saturating_sub(12);
                        detail.extend(
                            s.log
                                .iter()
                                .skip(skip)
                                .map(|l| Line::styled(format!("  {l}"), theme.dim)),
                        );
                    }
                    detail.push(Line::raw(""));
                    detail.push(Line::styled(
                        "⏎ disable this server · ^s restart",
                        theme.dim,
                    ));
                    ModalItem {
                        label: s.id.clone(),
                        hint: if s.state == quack_protocol::McpServerState::Ready {
                            format!("{} tools", s.tools.len())
                        } else {
                            s.state.label().to_owned()
                        },
                        detail,
                        action: Some(format!("/mcp disable {}", s.id)),
                        run: true,
                        alt: Some(format!("/mcp restart {}", s.id)),
                        charts: Vec::new(),
                    }
                })
                .collect();
            out.extend(mcp_catalog_and_add_items(list, catalog, scope, theme));
            out
        }
    };
    Modal::new(
        "MCP",
        vec![ModalTab {
            title: "Servers".into(),
            items,
        }],
    )
    .with_alt("restart")
    .with_scope(scope)
}

fn mcp_catalog_and_add_items(
    active: &[quack_protocol::McpServerInfo],
    catalog: &[quack_config::McpSummary],
    scope: WriteScope,
    theme: &Theme,
) -> Vec<ModalItem> {
    let flag = scope_flag(scope);
    let mcp_dir = match scope {
        WriteScope::Project => ".quack/mcp.d/",
        WriteScope::User | WriteScope::Session => "user mcp.d/",
    };
    let mut items = Vec::new();
    for entry in catalog {
        if active.iter().any(|s| s.id == entry.id) {
            continue;
        }
        let hint = if entry.parse_error.is_some() {
            "error".to_owned()
        } else if entry.layer == quack_config::LayerKind::Defaults {
            "example · off".to_owned()
        } else {
            "off".to_owned()
        };
        let mut detail = Vec::new();
        if !entry.description.is_empty() {
            detail.push(Line::raw(entry.description.clone()));
            detail.push(Line::raw(""));
        }
        if !entry.transport.is_empty() {
            detail.push(field(theme, "transport", entry.transport.clone()));
        }
        if !entry.target.is_empty() {
            detail.push(field(theme, "target", entry.target.clone()));
        }
        detail.push(field(
            theme,
            "source",
            format!("{} ({})", entry.layer.label(), entry.path.display()),
        ));
        if !entry.env_vars.is_empty() {
            detail.push(field(theme, "env vars", entry.env_vars.join(", ")));
        }
        if let Some(reason) = &entry.disabled_reason {
            detail.push(field(theme, "state", reason.clone()));
        }
        if let Some(err) = &entry.parse_error {
            detail.push(Line::raw(""));
            detail.push(Line::styled(format!("parse error: {err}"), theme.error));
        } else {
            detail.push(Line::raw(""));
            detail.push(Line::styled(
                if entry.layer == quack_config::LayerKind::Defaults {
                    "⏎ copy example to user config, enable, and connect now"
                } else {
                    "⏎ enable and connect now"
                },
                theme.dim,
            ));
        }
        items.push(ModalItem {
            label: entry.id.clone(),
            hint,
            detail,
            action: entry
                .parse_error
                .is_none()
                .then(|| format!("/mcp enable {}", entry.id)),
            run: entry.parse_error.is_none(),
            alt: None,
            charts: Vec::new(),
        });
    }

    items.push(ModalItem {
        label: "+ Add remote MCP server (HTTP)".into(),
        hint: "url".into(),
        detail: vec![
            Line::styled("Connect a streamable HTTP MCP endpoint by URL", theme.accent),
            Line::raw(""),
            Line::styled(
                format!(
                    "Prompts for the URL (e.g. https://mcp.context7.com/mcp), server id, and optional Bearer auth env var or token. Saves to {mcp_dir}."
                ),
                theme.dim,
            ),
        ],
        action: Some(format!("/mcp add{flag} http")),
        run: true,
        alt: None,
        charts: Vec::new(),
    });
    items.push(ModalItem {
        label: "+ Add local MCP server (stdio)".into(),
        hint: "command".into(),
        detail: vec![
            Line::styled(
                "Add a local MCP server process run over stdio",
                theme.accent,
            ),
            Line::raw(""),
            Line::styled(
                format!(
                    "Prompts for the command line (e.g. npx -y @playwright/mcp@latest) and server id. Saves to {mcp_dir}."
                ),
                theme.dim,
            ),
        ],
        action: Some(format!("/mcp add{flag} stdio")),
        run: true,
        alt: None,
        charts: Vec::new(),
    });
    items
}

/// The `/lsp` modal: each language server with its state and diagnostics. `None` while
/// asking the engine.
pub(crate) fn lsp(servers: Option<&[quack_protocol::LspServerInfo]>, theme: &Theme) -> Modal {
    let items = match servers {
        None => vec![placeholder("checking servers…", "", theme)],
        Some([]) => vec![placeholder(
            "no language servers",
            "Add a TOML file to lsp.d in your config folder (`quack config paths` shows \
             where), for example\n\ncommand = \"gopls\"\nextensions = [\"go\"]\n\nThe defaults \
             bundle has files for the main languages. quack never installs a server.",
            theme,
        )],
        Some(list) => list
            .iter()
            .map(|s| {
                let style = match s.state {
                    quack_protocol::LspServerState::Ready
                    | quack_protocol::LspServerState::Idle => theme.ok,
                    quack_protocol::LspServerState::Failed
                    | quack_protocol::LspServerState::Exited => theme.error,
                    quack_protocol::LspServerState::NotInstalled => theme.dim,
                    _ => theme.warn,
                };
                let mut detail = vec![Line::styled(s.id.clone(), style), Line::raw("")];
                detail.push(field(theme, "state", s.state.label().to_owned()));
                if !s.detail.is_empty() {
                    let st = if s.state == quack_protocol::LspServerState::Failed {
                        theme.error
                    } else {
                        theme.dim
                    };
                    detail.push(Line::styled(s.detail.clone(), st));
                }
                detail.push(field(theme, "command", s.command.clone()));
                detail.push(field(
                    theme,
                    "files",
                    format!(".{}", s.extensions.join(" .")),
                ));
                if !s.root.is_empty() {
                    detail.push(field(theme, "project", s.root.clone()));
                }
                if matches!(
                    s.state,
                    quack_protocol::LspServerState::Ready
                        | quack_protocol::LspServerState::Indexing
                ) {
                    detail.push(field(
                        theme,
                        "problems",
                        format!("{} errors · {} warnings", s.errors, s.warnings),
                    ));
                }
                if s.state == quack_protocol::LspServerState::Idle {
                    detail.push(Line::raw(""));
                    detail.push(Line::styled(
                        "Starts when the agent reads or edits a file it covers.",
                        theme.dim,
                    ));
                }
                if !s.log.is_empty() {
                    detail.push(Line::raw(""));
                    detail.push(Line::styled("recent output", theme.dim));
                    let skip = s.log.len().saturating_sub(12);
                    detail.extend(
                        s.log
                            .iter()
                            .skip(skip)
                            .map(|l| Line::styled(format!("  {l}"), theme.dim)),
                    );
                }
                let label = if s.root.is_empty() || s.root == "." {
                    s.id.clone()
                } else {
                    format!("{} · {}", s.id, s.root)
                };
                ModalItem {
                    label,
                    hint: match s.state {
                        quack_protocol::LspServerState::Ready if s.errors + s.warnings > 0 => {
                            format!("✗{} ⚠{}", s.errors, s.warnings)
                        }
                        other => other.label().to_owned(),
                    },
                    detail,
                    action: None,
                    run: false,
                    alt: None,
                    charts: Vec::new(),
                }
            })
            .collect(),
    };
    Modal::new(
        "LSP",
        vec![ModalTab {
            title: "Servers".into(),
            items,
        }],
    )
}

/// What `/setup` knows so far.
#[derive(Debug, Default)]
pub(crate) struct SetupView {
    pub local: Scan,
    /// The hosted services in providers.d; `None` while listing.
    pub services: Option<Vec<quack_protocol::ServiceInfo>>,
    /// The last subscription sign-in failure, shown beside that service for retry.
    pub sign_in_error: Option<(String, String)>,
    /// The settings when `/setup` opened, for the opt-in checks.
    pub settings: Vec<quack_config::SettingRow>,
}

/// Looking for model servers on this machine, which happens only when the user asks.
#[derive(Debug, Default)]
pub(crate) enum Scan {
    #[default]
    NotYet,
    Looking,
    Found(Vec<quack_protocol::DetectedServer>),
}

/// The `/setup` modal: local servers, hosted API keys and subscriptions, and any
/// server by its URL.
pub(crate) fn setup(view: &SetupView, theme: &Theme) -> Modal {
    const ELSEWHERE: &str = "For a hosted service, or a server on another machine or \
         port, press tab for the other ways to add one.";
    const LOOKS_AT: &str = "Checks the providers in providers.d that point at this machine \
         (Ollama, llama.cpp, vLLM, LM Studio and SGLang with the defaults bundle). Nothing \
         leaves this machine.";
    let item =
        |label: &str, hint: &str, detail: Vec<Line<'static>>, action: Option<String>| ModalItem {
            label: label.to_owned(),
            hint: hint.to_owned(),
            detail,
            run: action.is_some(),
            action,
            alt: None,
            charts: Vec::new(),
        };
    let local = match &view.local {
        Scan::NotYet => vec![item(
            "find servers on this machine",
            "",
            vec![
                Line::styled(LOOKS_AT, theme.dim),
                Line::raw(""),
                Line::styled("⏎ look now", theme.dim),
                Line::raw(""),
                Line::styled(ELSEWHERE, theme.dim),
            ],
            Some("/setup scan".into()),
        )],
        Scan::Looking => vec![placeholder("looking for servers…", LOOKS_AT, theme)],
        Scan::Found(list) if list.is_empty() => vec![item(
            "no servers found",
            "",
            vec![
                Line::styled(
                    "None of the servers on this machine in providers.d answered. Start \
                     one, for example `ollama serve` or `llama-server --jinja -m \
                     <model.gguf>`, then look again.",
                    theme.dim,
                ),
                Line::raw(""),
                Line::styled("⏎ look again", theme.dim),
                Line::raw(""),
                Line::styled(ELSEWHERE, theme.dim),
            ],
            Some("/setup scan".into()),
        )],
        Scan::Found(list) => list
            .iter()
            .enumerate()
            .flat_map(|(i, s)| {
                s.models.iter().map(move |m| {
                    let mut detail = vec![
                        Line::styled(format!("{} · {}", s.name, m.id), theme.accent),
                        Line::raw(""),
                        field(theme, "server", s.base_url.clone()),
                        field(
                            theme,
                            "context",
                            m.context_window
                                .map_or_else(|| "unknown".into(), |c| format!("{c} tokens")),
                        ),
                        Line::raw(""),
                    ];
                    if !s.note.is_empty() {
                        detail.push(Line::raw(s.note.clone()));
                        detail.push(Line::raw(""));
                    }
                    detail.push(match &s.configured {
                        Some(id) => Line::styled(
                            format!("⏎ use it (already set up as provider `{id}`)"),
                            theme.dim,
                        ),
                        None => Line::styled(
                            format!(
                                "⏎ add providers.d/{}.toml to your config folder and use this \
                                 model; it becomes the default if none is set",
                                s.id
                            ),
                            theme.dim,
                        ),
                    });
                    detail.push(Line::raw(""));
                    detail.push(Line::styled(ELSEWHERE, theme.dim));
                    item(
                        &format!("{} · {}", s.name, m.id),
                        &m.context_window
                            .map_or_else(String::new, crate::theme::tokens),
                        detail,
                        Some(format!("/setup use {i} {}", m.id)),
                    )
                })
            })
            .collect(),
    };
    let hosted = match view.services.as_deref() {
        None => vec![placeholder(
            "listing services…",
            "Reading the hosted services in providers.d.",
            theme,
        )],
        Some([]) => vec![placeholder(
            "no hosted services configured",
            "providers.d has no hosted services with a subscription or API key. The defaults bundle ships \
             a list of them; an install without them (e.g. `--plucked`) has none. Add a service \
             under Any server with its OpenAI-compatible URL instead.",
            theme,
        )],
        Some(list) => list
            .iter()
            .map(|s| {
                let mut detail = vec![
                    Line::styled(s.name.clone(), theme.accent),
                    Line::raw(""),
                    field(theme, "provider", s.id.clone()),
                    field(theme, "group", s.group.clone()),
                    field(theme, "url", s.base_url.clone()),
                    Line::raw(""),
                ];
                if s.subscription {
                    if let Some((id, error)) = &view.sign_in_error
                        && id == &s.id
                    {
                        detail.push(Line::styled(error.clone(), theme.warn));
                        detail.push(Line::raw("Press Enter to try signing in again."));
                        detail.push(Line::raw(""));
                    }
                    detail.push(Line::styled(
                        if s.ready {
                            "Signed in with plan usage allowed."
                        } else {
                            "Sign in with your subscription in the browser. Allow plan usage to continue."
                        }, theme.dim,
                    ));
                    detail.push(Line::raw("Credentials are saved privately in your data folder."));
                    detail.push(Line::raw("⏎ continue · ^s sign in again"));
                    let failed = view.sign_in_error.as_ref().is_some_and(|(id, _)| id == &s.id);
                    let next = if s.ready && !failed { "models" } else { "login" };
                    return ModalItem {
                        alt: Some(format!("/setup login {}", s.id)),
                        ..item(
                            &s.name, if s.ready { "ready" } else { "needs sign-in" }, detail,
                            Some(format!("/setup {next} {}", s.id)),
                        )
                    };
                }
                let action = if s.ready {
                    detail.push(Line::styled(
                        match &s.key {
                            Some(key) => format!("Its key is read from {}.", key_source(key)),
                            None => "Its key is set.".into(),
                        },
                        theme.ok,
                    ));
                    detail.push(Line::raw(""));
                    detail.push(Line::styled("⏎ pick one of its models", theme.dim));
                    detail.push(Line::styled(
                        format!("{ALT_KEY} replace its key, for example after rotating it"),
                        theme.dim,
                    ));
                    format!("/setup models {}", s.id)
                } else {
                    detail.push(Line::styled(
                        "⏎ enter its API key; quack keeps it in the OS credential store, \
                         and the provider file in your config folder only names it",
                        theme.dim,
                    ));
                    detail.push(Line::styled(
                        "Without a Secret Service (WSL, a server) it is kept in a file only \
                         your user can read instead.",
                        theme.dim,
                    ));
                    if let Some(env) = &s.env {
                        detail.push(Line::raw(""));
                        detail.push(Line::styled(
                            format!("Or set {env} in your shell and start quack again."),
                            theme.dim,
                        ));
                    }
                    format!("/setup key {}", s.id)
                };
                ModalItem {
                    alt: s.ready.then(|| format!("/key {}", s.id)),
                    ..item(
                        &s.name,
                        if s.ready { "ready" } else { "needs a key" },
                        detail,
                        Some(action),
                    )
                }
            })
            .collect(),
    };
    let other = vec![
        item(
            "add a server by its URL",
            "",
            vec![
                Line::styled("Any OpenAI-compatible server", theme.accent),
                Line::raw(""),
                Line::raw(
                    "A model server on another machine or port, or a hosted service that is \
                     not listed. quack asks for its base URL (for example \
                     http://gpu-box:8000/v1) and then its API key, if it needs one.",
                ),
                Line::raw(""),
                Line::styled(
                    "The key goes to the OS credential store; the provider file written to \
                     your config folder only names it. Without a Secret Service (WSL, a \
                     server) it is kept in a file only your user can read instead.",
                    theme.dim,
                ),
                Line::raw(""),
                Line::styled("⏎ start", theme.dim),
            ],
            Some("/setup url".into()),
        ),
        item(
            "write a provider file yourself",
            "",
            vec![
                Line::raw(
                    "A provider is a TOML file in providers.d in your config folder. Two lines \
                     are enough:",
                ),
                Line::raw(""),
                Line::styled("  base_url = \"http://gpu-box:8000/v1\"", theme.accent),
                Line::styled("  api_key = { env = \"MY_KEY\" }", theme.accent),
                Line::raw(""),
                Line::styled(
                    "`quack config paths` shows where the folder is. Run /reload after \
                     adding one.",
                    theme.dim,
                ),
            ],
            None,
        ),
    ];
    let mut tabs = vec![
        ModalTab {
            title: "This machine".into(),
            items: local,
        },
        ModalTab {
            title: "Hosted services".into(),
            items: hosted,
        },
        ModalTab {
            title: "Any server".into(),
            items: other,
        },
    ];
    let checks = setup_checks(&view.settings, theme);
    if !checks.is_empty() {
        tabs.push(ModalTab {
            title: "Checks".into(),
            items: checks,
        });
    }
    Modal::new("Setup", tabs)
}

/// The opt-in background checks that have somewhere to go. An install without those URLs
/// (e.g. `--plucked`) has none, and the tab stays away.
fn setup_checks(settings: &[quack_config::SettingRow], theme: &Theme) -> Vec<ModalItem> {
    const CHECKS: [(&str, &str, &str, &str); 2] = [
        (
            "model prices",
            "catalog.refresh",
            "catalog.url",
            "Downloads prices and context windows for models their provider doesn't \
             describe, at most once a day. It only ever fills in those numbers.",
        ),
        (
            "new versions",
            "updates.check",
            "updates.url",
            "Asks for the latest version number at most once a day and tells you here when \
             there is a new one. quack never updates itself.",
        ),
    ];
    let row = |key: &str| settings.iter().find(|r| r.key == key);
    if row("network.offline").is_some_and(|r| r.value == "true") {
        return Vec::new();
    }
    CHECKS
        .iter()
        .filter_map(|(label, key, url_key, about)| {
            let url = row(url_key)?.value.trim_matches('"').to_owned();
            let setting = row(key)?;
            if url.is_empty() {
                return None;
            }
            let on = setting.value == "true";
            let host = quack_config::url_host(&url).to_owned();
            let mut detail = vec![
                Line::styled((*label).to_owned(), theme.accent),
                Line::raw(""),
                Line::raw((*about).to_owned()),
                Line::raw(""),
                field(theme, "contacts", host),
                field(theme, "setting", (*key).to_owned()),
                Line::raw(""),
            ];
            let (hint, action) = if setting.locked {
                detail.push(Line::styled("Locked by the system policy.", theme.dim));
                (if on { "on, locked" } else { "off, locked" }, None)
            } else {
                detail.push(Line::styled(
                    format!(
                        "⏎ turn it {}, from the next start",
                        if on { "off" } else { "on" }
                    ),
                    theme.dim,
                ));
                (
                    if on { "on" } else { "off" },
                    Some(format!("/config set {key} {}", !on)),
                )
            };
            Some(ModalItem {
                label: (*label).to_owned(),
                hint: hint.to_owned(),
                detail,
                run: action.is_some(),
                action,
                alt: None,
                charts: Vec::new(),
            })
        })
        .collect()
}

/// The URL remains copyable when no browser is available. Leaving cancels the listener.
pub(crate) fn sign_in(id: &str, url: Option<&str>, theme: &Theme) -> Modal {
    let link = ModalItem {
        label: if url.is_some() {
            "copy sign-in URL"
        } else {
            "starting sign-in…"
        }
        .into(),
        hint: "waiting for browser".into(),
        detail: vec![
            Line::styled(
                "Continue in your browser and allow plan usage.",
                theme.accent,
            ),
            Line::raw("If it did not open, copy this URL into your browser:"),
            Line::raw(url.unwrap_or("Waiting for a sign-in URL…").to_owned()),
            Line::raw("Press Enter to copy the URL, or Escape to cancel sign-in."),
        ],
        action: url.map(str::to_owned),
        run: false,
        alt: None,
        charts: Vec::new(),
    };
    let mut modal = Modal::new(
        format!("Sign in: {id}"),
        vec![ModalTab {
            title: "Browser".into(),
            items: vec![link],
        }],
    );
    modal.copy_tab = Some(0);
    modal
}

#[cfg(test)]
mod tests {
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
