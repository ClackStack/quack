//! Read-only session details; the conversation keeps at least 104 columns.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use quack_protocol::LoadState;

use crate::app::App;
use crate::theme::{meter, tokens};
use crate::ui::truncate;
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

pub(crate) fn draw(f: &mut Frame<'_>, app: &App, area: Rect) {
    let t = &app.theme;
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_set(app.glyphs.border)
        .border_style(t.border)
        .padding(Padding::new(2, 1, 1, 0));
    let width = usize::from(block.inner(area).width);
    let ascii = app.info.ui.ascii;
    let mut rows = Vec::new();
    let heading = t.dim.add_modifier(Modifier::BOLD);
    // Until the user starts working, the sidebar is the startup panel's home: the
    // diagnostics stay put instead of flashing over the transcript.
    if app.startup_status_visible() {
        draw_startup(app, &mut rows, heading, width, ascii);
        f.render_widget(Paragraph::new(rows).block(block), area);
        return;
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
    if !app.agents.is_empty() {
        rows.push(Line::raw(""));
        rows.push(Line::styled("Sub-agents", heading));
        draw_agents(app, &mut rows, width, ascii);
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
            #[allow(clippy::cast_precision_loss)]
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
    if let Some(speed) = app.speed {
        rows.push(Line::styled(format!("{speed:.0} tokens/s"), t.dim));
    }
    f.render_widget(Paragraph::new(rows).block(block), area);
}

/// The session's sub-agent runs: the running ones live with their step count, the
/// finished ones as their result. Newest last; only the last few finished runs stay,
/// so a long session keeps room for the rest of the panel. `/tasks` has the detail.
fn draw_agents(app: &App, rows: &mut Vec<Line<'static>>, width: usize, ascii: bool) {
    let t = &app.theme;
    /// How many finished runs stay listed after new ones push them down.
    const KEPT_DONE: usize = 2;
    let done = app.agents.iter().filter(|a| a.ok.is_some()).count();
    let first_done = done.saturating_sub(KEPT_DONE);
    for (i, a) in app.agents.iter().enumerate() {
        if a.ok.is_some() && i < first_done {
            continue;
        }
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
    if first_done > 0 {
        let dots = if ascii { "..." } else { "…" };
        rows.push(Line::styled(
            format!("  +{first_done} earlier{dots}"),
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
