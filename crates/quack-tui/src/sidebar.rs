//! Read-only session details; the conversation keeps at least 104 columns.

use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use quack_protocol::{LoadState, LspServerInfo, LspServerState};

use crate::app::{App, PlanState};
use crate::theme::{meter, tokens};
use crate::ui::{plan_mark, truncate};
use crate::wrap::wrap;

const MIN_WIDTH: u16 = 140;
const MIN_HEIGHT: u16 = 24;
const WIDTH: u16 = 36;
/// The widest the sidebar grows on large terminals; the conversation keeps the rest.
const MAX_WIDTH: u16 = 48;

/// The sidebar's width at a screen width: 36 columns from 140, then one of every four
/// extra columns, up to 48, so wide details stop wrapping without crowding the
/// conversation past its 104-column minimum.
fn width(screen: u16) -> u16 {
    (WIDTH + screen.saturating_sub(MIN_WIDTH) / 4).min(MAX_WIDTH)
}

pub(crate) fn areas(screen: Rect, enabled: bool) -> (Rect, Option<Rect>) {
    if !enabled || screen.width < MIN_WIDTH || screen.height < MIN_HEIGHT {
        return (screen, None);
    }
    let [main, sidebar] = Layout::horizontal([
        Constraint::Min(104),
        Constraint::Length(width(screen.width)),
    ])
    .areas(screen);
    (main, Some(sidebar))
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct View {
    pub(crate) area: Rect,
    pub(crate) height: usize,
    pub(crate) max_scroll: usize,
}

#[derive(Debug)]
pub(crate) struct Sidebar {
    rows: Vec<Line<'static>>,
    /// Whether the sidebar rendered the session's plan; when true, the main pane omits
    /// its one-line plan strip unless `/todo` expanded it.
    pub(crate) shows_plan: bool,
}

impl Sidebar {
    pub(crate) fn draw(self, f: &mut Frame<'_>, app: &App, area: Rect) {
        let block = sidebar_block(app);
        let inner = block.inner(area);
        let height = usize::from(inner.height);
        let max_scroll = if app.startup_status_visible() {
            0
        } else {
            self.rows.len().saturating_sub(height)
        };
        let offset = app.sidebar_scroll.get().min(max_scroll);
        app.sidebar_scroll.set(offset);
        app.sidebar_view.set(View {
            area,
            height,
            max_scroll,
        });
        let visible: Vec<_> = self.rows.into_iter().skip(offset).take(height).collect();
        f.render_widget(Paragraph::new(visible).block(block), area);
        if max_scroll > 0 {
            let mut state = ScrollbarState::new(max_scroll + 1)
                .position(offset)
                .viewport_content_length(height);
            f.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None)
                    .thumb_symbol(app.glyphs.scroll[0])
                    .thumb_style(app.theme.dim)
                    .track_symbol(Some(app.glyphs.scroll[1]))
                    .track_style(app.theme.border),
                Rect::new(area.x, inner.y, area.width, inner.height),
                &mut state,
            );
        }
    }
}

fn sidebar_block(app: &App) -> Block<'static> {
    Block::new()
        .borders(Borders::LEFT)
        .border_set(app.glyphs.border)
        .border_style(app.theme.border)
        .padding(Padding::new(2, 1, 1, 0))
}

pub(crate) fn build(app: &App, area: Rect) -> Sidebar {
    let t = &app.theme;
    let inner = sidebar_block(app).inner(area);
    let width = usize::from(inner.width);
    let height = usize::from(inner.height);
    let ascii = app.info.ui.ascii;
    let mut rows = Vec::new();
    let heading = t.dim.add_modifier(Modifier::BOLD);
    // Until the user starts working, the sidebar is the startup panel's home: the
    // diagnostics stay put instead of flashing over the transcript.
    if app.startup_status_visible() {
        draw_startup(app, &mut rows, heading, width, ascii);
        return Sidebar {
            rows,
            shows_plan: false,
        };
    }
    field(
        &mut rows,
        Line::styled("Session", heading),
        app.session_title.as_deref().unwrap_or("New session"),
        Style::new().add_modifier(Modifier::BOLD),
        width,
        ascii,
    );
    rows.push(Line::raw(""));
    rows.push(Line::styled("Working folder", heading));
    // Keep the project folder visible at the end of a long path.
    let cwd = &app.info.cwd;
    let folder = if cwd.width() > width
        && let Some(at) = cwd.rfind(['/', '\\'])
        && !cwd[at + 1..].is_empty()
        && cwd[at + 1..].width() <= width
    {
        vec![tail(&cwd[..=at], width, ascii), cwd[at + 1..].to_owned()]
    } else {
        wrap(cwd, width)
    };
    let skip = folder.len().saturating_sub(2);
    for (i, line) in folder.into_iter().skip(skip).enumerate() {
        rows.push(Line::raw(if i == 0 && skip > 0 {
            let dots = if ascii { "..." } else { "…" };
            tail(&format!("{dots}{line}"), width, ascii)
        } else {
            line
        }));
    }
    if let Some(branch) = &app.info.branch {
        rows.push(Line::from(vec![
            Span::styled("Git branch  ", t.dim),
            Span::styled(clip(branch, width.saturating_sub(12), ascii), t.magenta),
        ]));
    }
    rows.push(Line::raw(""));
    let mut model_heading = vec![Span::styled("Model", heading)];
    if let Some(thinking) = app.thinking_label() {
        let separator = if ascii { " - " } else { " · " };
        model_heading.push(Span::styled(format!("{separator}{thinking}"), t.dim));
    }
    field(
        &mut rows,
        Line::from(model_heading),
        app.info.model.as_deref().unwrap_or("Select with /models"),
        if app.info.model.is_some() {
            Style::new()
        } else {
            t.warn
        },
        width,
        ascii,
    );
    if app.info.profile != "empty" {
        rows.push(Line::from(vec![
            Span::styled("Profile  ", t.dim),
            Span::raw(clip(&app.info.profile, width.saturating_sub(9), ascii)),
        ]));
    }
    rows.push(Line::raw(""));
    rows.push(Line::styled("Context", heading));
    match app.context {
        Some((used, max)) if max > 0 => {
            let percent = u64::from(used) * 100 / u64::from(max);
            let summary = format!("{} / {} tokens ({percent}%)", tokens(used), tokens(max));
            rows.push(Line::raw(clip(&summary, width, ascii)));
            let threshold = app.compact_at.unwrap_or(max).max(1);
            let style = if u64::from(used) * 100 >= u64::from(threshold) * 85 {
                t.error
            } else if u64::from(used) * 100 >= u64::from(threshold) * 60 {
                t.warn
            } else {
                t.accent
            };
            let fraction = used as f32 / max as f32;
            rows.push(Line::styled(meter(fraction, width, ascii), style));
            if let Some(at) = app.compact_at {
                rows.push(Line::styled(
                    format!("Summarize at {} tokens", tokens(at)),
                    t.dim,
                ));
            }
        }
        Some((used, _)) => {
            rows.push(Line::raw(format!("{} tokens in context", tokens(used))));
            rows.push(Line::styled("Window size unknown", t.dim));
        }
        None => rows.push(Line::styled("Usage not reported yet", t.dim)),
    }
    let sep = if ascii { " - " } else { " · " };
    let metrics = match (app.speed, app.cache_hit_percent) {
        (Some(speed), Some(pct)) => Some(format!("{speed:.0} tokens/s{sep}{pct}% cache hit")),
        (Some(speed), None) => Some(format!("{speed:.0} tokens/s")),
        (None, Some(pct)) => Some(format!("{pct}% cache hit")),
        (None, None) => None,
    };
    if let Some(metrics) = metrics {
        rows.push(Line::styled(clip(&metrics, width, ascii), t.dim));
    }
    let mut details = Vec::new();
    if !app.agents.is_empty() {
        details.push(Line::raw(""));
        details.push(Line::styled("Sub-agents", heading));
        draw_agents(app, &mut details, width, ascii);
    }
    let active_lsp: Vec<&LspServerInfo> = app
        .lsp_servers
        .iter()
        .filter(|s| !matches!(s.state, LspServerState::Idle | LspServerState::NotInstalled))
        .collect();
    if !active_lsp.is_empty() {
        details.push(Line::raw(""));
        details.push(Line::styled("LSP", heading));
        draw_lsp(app, &active_lsp, &mut details, width, ascii);
    }
    if !app.skills.is_empty() || !app.loaded_skills.is_empty() {
        details.push(Line::raw(""));
        details.push(Line::styled("Skills", heading));
        draw_skills(app, &mut details, width, ascii);
    }
    let shows_plan = if !app.plan.is_empty() {
        let remaining = height.saturating_sub(rows.len());
        // Give the lists below the plan room before expanding it. Long lists scroll
        // instead of squeezing out the compact plan.
        let plan_room = remaining
            .saturating_sub(details.len())
            .max(remaining.min(3));
        draw_plan(app, &mut rows, heading, width, plan_room, ascii)
    } else {
        false
    };
    rows.extend(details);
    Sidebar { rows, shows_plan }
}

/// List every available skill, marking the ones already loaded. Keep loaded skills
/// that are no longer in the catalog (for example after a reload).
fn draw_skills(app: &App, rows: &mut Vec<Line<'static>>, width: usize, ascii: bool) {
    let mut loaded: HashSet<&str> = app.loaded_skills.iter().map(String::as_str).collect();
    let mut push = |name, is_loaded| {
        let (icon, style) = if is_loaded {
            (app.glyphs.ok, app.theme.ok)
        } else {
            (app.glyphs.pending, app.theme.dim)
        };
        let room = width.saturating_sub(icon.width() + 1);
        rows.push(Line::from(vec![
            Span::styled(format!("{icon} "), style),
            Span::raw(clip(name, room, ascii)),
        ]));
    };
    for skill in &app.skills {
        let name = skill.name.as_str();
        push(name, loaded.remove(name));
    }
    for name in &app.loaded_skills {
        if loaded.remove(name.as_str()) {
            push(name, true);
        }
    }
}

/// Active/loaded language servers (excluding idle and uninstalled ones).
fn draw_lsp(
    app: &App,
    servers: &[&LspServerInfo],
    rows: &mut Vec<Line<'static>>,
    width: usize,
    ascii: bool,
) {
    const MAX_VISIBLE: usize = 3;
    let t = &app.theme;
    let total = servers.len();
    let shown = if total > MAX_VISIBLE {
        MAX_VISIBLE - 1
    } else {
        total
    };
    let (err_mark, warn_mark) = if ascii { ("E", "W") } else { ("✗", "⚠") };
    for s in servers.iter().take(shown) {
        let (icon, icon_style) = match s.state {
            LspServerState::Starting => (app.spinner(), t.accent),
            LspServerState::Indexing => (app.spinner(), t.warn),
            LspServerState::Ready => (app.glyphs.ok, t.ok),
            LspServerState::Failed | LspServerState::Exited => (app.glyphs.fail, t.error),
            LspServerState::Idle | LspServerState::NotInstalled => (app.glyphs.pending, t.dim),
        };
        let label = if s.root.is_empty() || s.root == "." {
            s.id.clone()
        } else {
            format!("{} ({})", s.id, s.root)
        };
        let mut tail_spans: Vec<Span<'static>> = Vec::new();
        let mut tail_width = 0usize;
        if s.errors > 0 || s.warnings > 0 {
            tail_spans.push(Span::raw("  "));
            tail_width += 2;
            if s.errors > 0 {
                let text = format!("{err_mark}{}", s.errors);
                tail_width += text.width();
                tail_spans.push(Span::styled(text, t.error));
            }
            if s.errors > 0 && s.warnings > 0 {
                tail_spans.push(Span::raw(" "));
                tail_width += 1;
            }
            if s.warnings > 0 {
                let text = format!("{warn_mark}{}", s.warnings);
                tail_width += text.width();
                tail_spans.push(Span::styled(text, t.warn));
            }
        } else {
            let state_text = s.state.label();
            let state_style = match s.state {
                LspServerState::Failed | LspServerState::Exited => t.error,
                LspServerState::Indexing => t.warn,
                _ => t.dim,
            };
            tail_spans.push(Span::raw("  "));
            tail_spans.push(Span::styled(state_text, state_style));
            tail_width += 2 + state_text.width();
        }
        let lead = icon.width() + 1;
        let name_room = width.saturating_sub(lead + tail_width);
        let mut spans = vec![
            Span::styled(format!("{icon} "), icon_style),
            Span::raw(clip(&label, name_room, ascii)),
        ];
        spans.extend(tail_spans);
        rows.push(Line::from(spans));
    }
    let hidden = total.saturating_sub(shown);
    if hidden > 0 {
        rows.push(Line::styled(format!("  +{hidden} more"), t.dim));
    }
}

/// The session's plan: progress summary, meter, and steps. Fits within `remaining` rows
/// below Context without pushing metadata off-screen; returns `false` when vertical space
/// is too tight so the main pane keeps its one-line plan strip.
fn draw_plan(
    app: &App,
    rows: &mut Vec<Line<'static>>,
    heading: Style,
    width: usize,
    remaining: usize,
    ascii: bool,
) -> bool {
    if remaining < 3 || width == 0 {
        return false;
    }
    let t = &app.theme;
    let total = app.plan.len();
    let done = app
        .plan
        .iter()
        .filter(|(s, _)| *s == PlanState::Done)
        .count();
    let percent = done * 100 / total.max(1);
    let count_style = if done == total { t.ok } else { t.dim };

    rows.push(Line::raw(""));
    if remaining == 3 {
        let fraction = done as f32 / total.max(1) as f32;
        let compact = meter(fraction, total.min(8), ascii);
        rows.push(Line::from(vec![
            Span::styled("Plan", heading),
            Span::raw(" "),
            Span::styled(compact, t.ok),
            Span::styled(format!(" {done}/{total}"), count_style),
        ]));
        let (state, text) = &app.plan[plan_focus(&app.plan)];
        push_plan_step_single(app, rows, *state, text, width, ascii);
        return true;
    }

    let separator = if ascii { " - " } else { " · " };
    rows.push(Line::from(vec![
        Span::styled("Plan", heading),
        Span::styled(separator, t.dim),
        Span::styled(format!("{done}/{total} ({percent}%)"), count_style),
    ]));
    let river = t.plan_river(&app.glyphs, done, total);
    rows.push(Line::from(Vec::from(river)));

    let step_room = remaining - 3;
    let wrapped: Vec<Vec<String>> = app
        .plan
        .iter()
        .map(|(state, text)| {
            let lead = plan_mark(app, *state).0.width() + 1;
            wrap_step(text, width.saturating_sub(lead), ascii)
        })
        .collect();
    let wrapped_rows: usize = wrapped.iter().map(Vec::len).sum();
    if wrapped_rows <= step_room {
        for ((state, _), lines) in app.plan.iter().zip(wrapped) {
            push_plan_step_wrapped(app, rows, *state, lines);
        }
        return true;
    }

    if total <= step_room {
        for (state, text) in &app.plan {
            push_plan_step_single(app, rows, *state, text, width, ascii);
        }
        return true;
    }

    let (visible, show_overflow) = if step_room >= 2 {
        (step_room - 1, true)
    } else {
        (1, false)
    };
    let focus = plan_focus(&app.plan);
    let leading_done = app
        .plan
        .iter()
        .take_while(|(s, _)| *s == PlanState::Done)
        .count();
    let max_start = total.saturating_sub(visible);
    let min_for_focus = focus.saturating_add(1).saturating_sub(visible);
    let start = leading_done.max(min_for_focus).min(max_start);
    let end = (start + visible).min(total);
    for (state, text) in &app.plan[start..end] {
        push_plan_step_single(app, rows, *state, text, width, ascii);
    }
    if show_overflow {
        let hidden_above = start;
        let hidden_below = total.saturating_sub(end);
        let note = match (hidden_above, hidden_below) {
            (0, b) => format!("  +{b} more"),
            (a, 0) => format!("  +{a} earlier"),
            (a, b) => {
                let sep = if ascii { ", " } else { " · " };
                format!("  +{a} earlier{sep}+{b} more")
            }
        };
        rows.push(Line::styled(clip(&note, width, ascii), t.dim));
    }
    true
}

fn plan_focus(plan: &[(PlanState, String)]) -> usize {
    plan.iter()
        .position(|(s, _)| *s == PlanState::Active)
        .or_else(|| plan.iter().position(|(s, _)| *s == PlanState::Pending))
        .unwrap_or_else(|| plan.len().saturating_sub(1))
}

fn plan_text_style(app: &App, state: PlanState, mark_style: Style) -> Style {
    match state {
        PlanState::Done => app.theme.dim.add_modifier(Modifier::CROSSED_OUT),
        PlanState::Active => mark_style.add_modifier(Modifier::BOLD),
        PlanState::Pending => Style::new(),
    }
}

fn wrap_step(text: &str, room: usize, ascii: bool) -> Vec<String> {
    if room == 0 {
        return vec![String::new()];
    }
    let mut lines = wrap(text, room).into_iter();
    let Some(first) = lines.next() else {
        return vec![String::new()];
    };
    let mut out = vec![first];
    if let Some(mut second) = lines.next() {
        if lines.next().is_some() {
            let dots = if ascii { "..." } else { "…" };
            second = clip(&format!("{second}{dots}"), room, ascii);
        }
        out.push(second);
    }
    out
}

fn push_plan_step_wrapped(
    app: &App,
    rows: &mut Vec<Line<'static>>,
    state: PlanState,
    lines: Vec<String>,
) {
    let (mark, mark_style) = plan_mark(app, state);
    let text_style = plan_text_style(app, state, mark_style);
    let indent = " ".repeat(mark.width() + 1);
    let mut iter = lines.into_iter();
    rows.push(Line::from(vec![
        Span::styled(format!("{mark} "), mark_style),
        Span::styled(iter.next().unwrap_or_default(), text_style),
    ]));
    for rest in iter {
        rows.push(Line::from(vec![
            Span::raw(indent.clone()),
            Span::styled(rest, text_style),
        ]));
    }
}

fn push_plan_step_single(
    app: &App,
    rows: &mut Vec<Line<'static>>,
    state: PlanState,
    text: &str,
    width: usize,
    ascii: bool,
) {
    let (mark, mark_style) = plan_mark(app, state);
    let text_style = plan_text_style(app, state, mark_style);
    let room = width.saturating_sub(mark.width() + 1);
    rows.push(Line::from(vec![
        Span::styled(format!("{mark} "), mark_style),
        Span::styled(clip(text, room, ascii), text_style),
    ]));
}

/// The session's sub-agent runs: the running ones live with their step count, the
/// finished ones as their result. Newest last; `/tasks` has the detail.
fn draw_agents(app: &App, rows: &mut Vec<Line<'static>>, width: usize, ascii: bool) {
    let t = &app.theme;
    for a in &app.agents {
        let (icon, style) = match a.ok {
            None => (app.spinner().to_owned(), t.accent),
            Some(true) => (app.glyphs.ok.to_owned(), t.ok),
            Some(false) => (app.glyphs.fail.to_owned(), t.error),
        };
        let room = width.saturating_sub(2);
        rows.push(Line::from(vec![
            Span::styled(format!("{icon} "), style),
            Span::styled(clip(&a.description, room, ascii), style),
        ]));
        let took = a.took.unwrap_or_else(|| a.started.elapsed());
        let detail = match a.ok {
            None => format!("{} · {} steps", a.agent, a.steps()),
            Some(true) => format!("{} · {}", a.agent, a.summary),
            Some(false) => format!("{} · failed · {}", a.agent, a.summary),
        };
        rows.push(Line::styled(
            format!(
                "  {}",
                clip(
                    &format!("{detail} · {}", crate::theme::elapsed(took)),
                    width,
                    ascii
                )
            ),
            t.dim,
        ));
    }
}

/// One row per subsystem, the same states the transcript-area panel shows. A detail
/// that does not fit gets one continuation line; anything longer clips.
fn draw_startup(
    app: &App,
    rows: &mut Vec<Line<'static>>,
    heading: Style,
    width: usize,
    ascii: bool,
) {
    let t = &app.theme;
    rows.push(Line::styled("Startup", heading));
    for (subsystem, state) in &app.startup {
        let (icon, style, text): (&str, Style, &str) = match state {
            LoadState::Pending => (app.glyphs.pending, t.dim, ""),
            LoadState::Running { detail } => (app.spinner(), t.accent, detail),
            LoadState::Ready { detail } => (app.glyphs.ok, t.ok, detail),
            LoadState::NeedsAttention { detail } => (app.glyphs.attention, t.warn, detail),
            LoadState::Failed { error } => (app.glyphs.fail, t.error, error),
            LoadState::Disabled { reason } => (app.glyphs.disabled, t.dim, reason),
        };
        // Longest label is "instructions".
        let lead = icon.width() + 1 + 12 + 1;
        let room = width.saturating_sub(lead);
        let detail_style = if matches!(state, LoadState::Ready { .. } | LoadState::Disabled { .. })
        {
            t.dim
        } else {
            Style::new()
        };
        let mut lines = wrap(text, room).into_iter();
        let mut spans = vec![
            Span::styled(format!("{icon} "), style),
            Span::styled(format!("{:<12} ", subsystem.label()), style),
            Span::styled(lines.next().unwrap_or_default(), detail_style),
        ];
        let rest = lines.next();
        rows.push(Line::from(std::mem::take(&mut spans)));
        if let Some(second) = rest {
            let second = if lines.next().is_some() {
                let dots = if ascii { "..." } else { "…" };
                clip(&format!("{second}{dots}"), room, ascii)
            } else {
                second
            };
            rows.push(Line::from(vec![
                Span::raw(" ".repeat(lead)),
                Span::styled(second, detail_style),
            ]));
        }
    }
}

/// Bound wrapping so long titles, paths and model ids leave room for context usage.
fn field(
    rows: &mut Vec<Line<'static>>,
    label: Line<'static>,
    value: &str,
    style: Style,
    width: usize,
    ascii: bool,
) {
    rows.push(label);
    let lines = wrap(value, width);
    let mut lines = lines.into_iter();
    if let Some(first) = lines.next() {
        rows.push(Line::styled(first, style));
    }
    if let Some(mut second) = lines.next() {
        if lines.next().is_some() {
            let dots = if ascii { "..." } else { "…" };
            second = clip(&format!("{second} {dots}"), width, ascii);
        }
        rows.push(Line::styled(second, style));
    }
}

fn clip(text: &str, width: usize, ascii: bool) -> String {
    if ascii && text.width() > width {
        let clipped = truncate(text, width.saturating_sub(2));
        format!("{}...", clipped.trim_end_matches('…'))
    } else {
        truncate(text, width)
    }
}

fn tail(text: &str, width: usize, ascii: bool) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let dots = if ascii { "..." } else { "…" };
    let mut used = dots.width();
    let tail: String = text
        .chars()
        .rev()
        .take_while(|c| {
            used += c.width().unwrap_or(0);
            used <= width
        })
        .collect();
    format!("{dots}{}", tail.chars().rev().collect::<String>())
}
