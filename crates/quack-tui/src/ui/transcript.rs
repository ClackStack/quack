//! Transcript rendering, caching, tool cards, and sub-agent rows.

use super::*;

/// Diff and output lines shown under a finished tool call.
pub(super) const TOOL_DETAIL_LINES: usize = 24;
/// Live output lines shown while a tool runs, and for failures.
pub(super) const TOOL_LIVE_LINES: usize = 6;
/// Output lines shown under a successful command.
pub(super) const TOOL_OK_LINES: usize = 3;

pub(super) fn assistant_lead(app: &App, is_head: bool) -> Vec<Span<'static>> {
    let lead_w = 2 + app.glyphs.assistant.width();
    if !is_head {
        return vec![Span::raw(" ".repeat(lead_w))];
    }
    let (body_style, beak_style) = app.theme.duck();
    if let Some(body) = app.glyphs.assistant.strip_suffix('>') {
        vec![
            Span::raw(" "),
            Span::styled(body.to_owned(), body_style),
            Span::styled(">", beak_style),
            Span::raw(" "),
        ]
    } else {
        vec![
            Span::raw(" "),
            Span::styled(app.glyphs.assistant, body_style),
            Span::raw(" "),
        ]
    }
}

/// The transcript's rows, and for each the columns its lead (glyph and indent) takes,
/// which copying a selection leaves out.
pub(crate) fn transcript_rows_with_gutters(
    app: &App,
    width: usize,
) -> (Vec<Line<'static>>, Vec<usize>) {
    let pieces = transcript_pieces(app, width);
    let mut rows = Vec::new();
    let mut gutters = Vec::new();
    for piece in pieces {
        gutters.extend(std::iter::repeat_n(
            piece.gutter,
            piece.rows.len() + usize::from(piece.blank),
        ));
        rows.extend(piece.rows.iter().cloned());
        if piece.blank {
            rows.push(Line::raw(""));
        }
    }
    (rows, gutters)
}

/// The transcript's rendered rows, kept per entry so a frame only lays out entries whose
/// text changed (streaming deltas, finished tools) or whose wrap width did. Drawing
/// writes it, like the `Cell` view state in `App`.
#[derive(Debug, Default)]
pub(crate) struct TranscriptCache {
    /// The wrap width the cached rows were laid out for.
    width: usize,
    /// One slot per transcript entry, in order. `None` for entries that are re-rendered
    /// every frame (running tools, whose spinner and elapsed time move).
    slots: Vec<Option<Slot>>,
}

#[derive(Debug, Clone)]
pub(super) struct Slot {
    /// Fingerprint of everything the entry's rows depend on; any change re-renders it.
    fp: u64,
    rows: Arc<Vec<Line<'static>>>,
}

/// One entry's share of the transcript: its rendered rows, the gutter its lead takes
/// (what copying a selection leaves out), and whether a blank row follows it.
pub(super) struct Piece {
    rows: Arc<Vec<Line<'static>>>,
    gutter: usize,
    blank: bool,
}

/// Everything a tool entry's rows depend on beyond its own fields: sub-agent runs are
/// drawn under their `task` call and grow while the sub-agent works.
pub(super) fn agents_fingerprint(app: &App) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    app.agents.len().hash(&mut h);
    for a in &app.agents {
        a.id.hash(&mut h);
        a.ok.hash(&mut h);
        a.summary.len().hash(&mut h);
        a.items.len().hash(&mut h);
    }
    h.finish()
}

/// A cheap identity for an entry's rendering: the width, the entry's own content, and
/// the pieces of `App` state it is drawn from (highlighted code blocks, the reasoning
/// mode, whether the last reasoning line is live, sub-agent runs under tool calls).
pub(super) fn entry_fingerprint(
    entry: &Entry,
    width: usize,
    last: bool,
    show_reasoning: u8,
    turn_running: bool,
    highlights: u64,
    agents: u64,
) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    width.hash(&mut h);
    match entry {
        Entry::User(s) => {
            0u8.hash(&mut h);
            s.hash(&mut h);
        }
        Entry::TurnSummary(s) => {
            1u8.hash(&mut h);
            s.hash(&mut h);
        }
        Entry::System(s) => {
            2u8.hash(&mut h);
            s.hash(&mut h);
        }
        Entry::Warning(s) => {
            3u8.hash(&mut h);
            s.hash(&mut h);
        }
        Entry::Error(s) => {
            4u8.hash(&mut h);
            s.hash(&mut h);
        }
        Entry::Assistant(s) => {
            5u8.hash(&mut h);
            s.hash(&mut h);
            highlights.hash(&mut h);
        }
        Entry::Reasoning(s) => {
            6u8.hash(&mut h);
            s.hash(&mut h);
            show_reasoning.hash(&mut h);
            (turn_running && last).hash(&mut h);
        }
        Entry::Tool(t) => {
            7u8.hash(&mut h);
            t.name.hash(&mut h);
            t.summary.hash(&mut h);
            t.ok.hash(&mut h);
            t.result.hash(&mut h);
            t.detail.hash(&mut h);
            t.took.hash(&mut h);
            agents.hash(&mut h);
        }
    }
    h.finish()
}

/// One entry's rows and gutter. `blank` says whether a separating blank row follows it
/// (tool rows ask the caller, who knows the neighbours).
pub(super) fn render_entry(
    app: &App,
    entry: &Entry,
    width: usize,
    live: bool,
) -> (Arc<Vec<Line<'static>>>, usize, bool) {
    let t = &app.theme;
    let body_width = width.saturating_sub(3);
    match entry {
        Entry::User(s) => {
            // A tinted band, so each turn's start is easy to find when scrolling.
            let mut rows = Vec::new();
            for (i, row) in wrap(s, body_width).into_iter().enumerate() {
                let lead = if i == 0 {
                    format!(" {} ", app.glyphs.prompt)
                } else {
                    "   ".into()
                };
                let pad = width.saturating_sub(3 + row.width());
                rows.push(Line::from(vec![
                    Span::styled(lead, t.accent.patch(t.user_bg)),
                    Span::styled(row, t.user.patch(t.user_bg)),
                    Span::styled(" ".repeat(pad), t.user_bg),
                ]));
            }
            (Arc::new(rows), 3, true)
        }
        Entry::TurnSummary(s) => {
            let rule = app.glyphs.rule;
            let text = format!("{rule}{rule} {s} ");
            let fill = width.saturating_sub(3 + text.width()).min(8);
            let rows = vec![Line::styled(
                format!("   {text}{}", rule.repeat(fill)),
                t.dim,
            )];
            (Arc::new(rows), 0, true)
        }
        Entry::Assistant(s) if s.trim().is_empty() => (Arc::new(Vec::new()), 3, false),
        Entry::Assistant(s) => {
            let lead_w = 2 + app.glyphs.assistant.width();
            let body_w = width.saturating_sub(lead_w);
            let mut rows = Vec::new();
            for (i, row) in
                crate::markdown::render(s, body_w, t, app.info.ui.ascii, Some(&app.highlights))
                    .into_iter()
                    .enumerate()
            {
                let mut spans = assistant_lead(app, i == 0);
                spans.extend(row);
                rows.push(Line::from(spans));
            }
            (Arc::new(rows), lead_w, true)
        }
        Entry::Reasoning(s) => match app.show_reasoning {
            ShowReasoning::Hidden => (Arc::new(Vec::new()), 3, false),
            ShowReasoning::Collapsed => {
                let words = s.split_whitespace().count();
                let italic = t.dim.add_modifier(Modifier::ITALIC);
                let what = if live {
                    format!("thinking… {words} words")
                } else {
                    format!("thought for {words} words")
                };
                lead_rows(
                    app.glyphs.think,
                    t.magenta,
                    italic,
                    &format!("{what} · ctrl-t shows"),
                    width,
                )
            }
            ShowReasoning::Expanded => lead_rows(
                app.glyphs.think,
                t.magenta,
                t.dim.add_modifier(Modifier::ITALIC),
                s.trim(),
                width,
            ),
        },
        Entry::Tool(tool) => {
            let mut rows = Vec::new();
            tool_rows(app, tool, body_width, &mut rows);
            (Arc::new(rows), 3, false)
        }
        Entry::System(s) => lead_rows(" ", t.dim, t.dim, s, width),
        Entry::Warning(s) => lead_rows("!", t.warn, t.warn, s, width),
        Entry::Error(s) => lead_rows(app.glyphs.fail, t.error, t.error, s, width),
    }
}

/// The common shape: a glyph lead on the first row, indented wrap after, blank below.
pub(super) fn lead_rows(
    prefix: &str,
    prefix_style: Style,
    body_style: Style,
    text: &str,
    width: usize,
) -> (Arc<Vec<Line<'static>>>, usize, bool) {
    let mut rows = Vec::new();
    let body_width = width.saturating_sub(3);
    for (i, row) in wrap(text, body_width).into_iter().enumerate() {
        let lead = if i == 0 {
            format!(" {prefix} ")
        } else {
            "   ".into()
        };
        rows.push(Line::from(vec![
            Span::styled(lead, prefix_style),
            Span::styled(row, body_style),
        ]));
    }
    (Arc::new(rows), 3, true)
}

/// The transcript's entries rendered at `width`, served from `App`'s layout cache: only
/// entries whose fingerprint changed are laid out again, so a streaming delta re-renders
/// one message, not the whole transcript. Running tools render fresh every frame.
pub(super) fn transcript_pieces(app: &App, width: usize) -> Vec<Piece> {
    let agents = agents_fingerprint(app);
    let highlights = app.highlights.version();
    let show_reasoning = match app.show_reasoning {
        ShowReasoning::Hidden => 0u8,
        ShowReasoning::Collapsed => 1,
        ShowReasoning::Expanded => 2,
    };
    let len = app.transcript.len();
    let mut pieces = Vec::with_capacity(len);
    {
        let mut cache = app.transcript_cache.borrow_mut();
        if cache.width != width {
            cache.width = width;
            cache.slots.clear();
        }
        cache.slots.truncate(len);
        cache.slots.resize_with(len, || None);
        for (i, entry) in app.transcript.iter().enumerate() {
            let fp = entry_fingerprint(
                entry,
                width,
                i + 1 == len,
                show_reasoning,
                app.turn_running,
                highlights,
                agents,
            );
            let last = i + 1 == len;
            let rows = if matches!(entry, Entry::Tool(t) if t.ok.is_none()) {
                // Live output, spinner and elapsed time: render it fresh each frame.
                cache.slots[i] = None;
                render_entry(app, entry, width, app.turn_running && last).0
            } else if let Some(slot) = cache.slots[i].as_ref().filter(|s| s.fp == fp) {
                slot.rows.clone()
            } else {
                let rows = render_entry(app, entry, width, app.turn_running && last).0;
                cache.slots[i] = Some(Slot {
                    fp,
                    rows: rows.clone(),
                });
                rows
            };
            let (gutter, blank) = match entry {
                Entry::Tool(_) => (
                    3,
                    // Consecutive tool calls read as one block.
                    !matches!(app.transcript.get(i + 1), Some(Entry::Tool(_)) | None),
                ),
                Entry::User(_) | Entry::System(_) | Entry::Warning(_) | Entry::Error(_) => {
                    (3, true)
                }
                Entry::TurnSummary(_) => (0, true),
                Entry::Assistant(s) => (2 + app.glyphs.assistant.width(), !s.trim().is_empty()),
                Entry::Reasoning(_) => (3, app.show_reasoning != ShowReasoning::Hidden),
            };
            pieces.push(Piece {
                rows,
                gutter,
                blank,
            });
        }
    }
    pieces
}

pub(super) fn tool_rows(app: &App, tool: &ToolEntry, width: usize, rows: &mut Vec<Line<'static>>) {
    let t = &app.theme;
    let (icon, style) = match tool.ok {
        None => (app.spinner(), t.accent),
        Some(true) => (app.glyphs.ok, t.ok),
        Some(false) => (app.glyphs.fail, t.error),
    };
    // How long it took (or has been running), when that is long enough to matter.
    let time = match (tool.took, tool.started) {
        (Some(d), _) => Some(d),
        (None, Some(s)) if tool.ok.is_none() => Some(s.elapsed()),
        _ => None,
    }
    .filter(|d| d.as_secs() >= 1)
    .map(|d| format!("  {}", crate::theme::elapsed(d)))
    .unwrap_or_default();
    let summary = truncate(&tool.summary, width.saturating_sub(2 + time.width()));
    // The tool's name stands out in its colour; the rest is its argument.
    let (verb, rest) = match summary.split_once(' ') {
        Some((v, r)) => (v.to_owned(), format!(" {r}")),
        None => (summary.clone(), String::new()),
    };
    let mut head = vec![
        Span::styled(format!(" {icon} "), style),
        Span::styled(verb, t.tool(&tool.name).add_modifier(Modifier::BOLD)),
        Span::raw(rest),
    ];
    if !tool.result.is_empty() && !tool.summary.contains(&tool.result) {
        let room = width.saturating_sub(summary.width() + 3 + time.width());
        let result_style = if tool.ok == Some(false) {
            t.error
        } else {
            t.dim
        };
        head.push(Span::styled(
            format!(" · {}", truncate(&tool.result, room)),
            result_style,
        ));
    }
    head.push(Span::styled(time, t.dim));
    rows.push(Line::from(head));

    if tool.name == "todo" && tool.ok == Some(true) {
        for (state, text) in crate::app::parse_plan(&tool.detail) {
            let (mark, style) = plan_mark(app, state);
            let text_style = match state {
                PlanState::Done => t.dim.add_modifier(Modifier::CROSSED_OUT),
                PlanState::Active => Style::new().add_modifier(Modifier::BOLD),
                PlanState::Pending => Style::new(),
            };
            rows.push(Line::from(vec![
                Span::styled(format!("     {mark} "), style),
                Span::styled(truncate(&text, width.saturating_sub(6)), text_style),
            ]));
        }
        return;
    }

    if tool.name == "ask_user" && tool.ok == Some(true) {
        let mut q = "";
        for l in tool.detail.lines() {
            if let Some(rest) = l.strip_prefix("Q: ") {
                q = rest;
            } else if let Some(a) = l.strip_prefix("A: ") {
                let q = truncate(q, width.saturating_sub(10) / 2);
                let used = q.width() + 9;
                rows.push(Line::from(vec![
                    Span::styled(format!("     {q} → "), t.dim),
                    Span::raw(truncate(a, width.saturating_sub(used))),
                ]));
            }
        }
        return;
    }

    if let Some(run) = app.agents.iter().rev().find(|a| a.call_id == tool.call_id) {
        agent_rows(app, run, tool, width, rows);
        return;
    }

    let is_diff = tool.detail.starts_with("--- ");
    let lines: Vec<&str> = tool.detail.lines().collect();
    let shown: Vec<&str> = if tool.ok.is_none() {
        lines[lines.len().saturating_sub(TOOL_LIVE_LINES)..].to_vec()
    } else if is_diff {
        // Skip the ---/+++ header; hunks carry the information.
        lines
            .iter()
            .skip(2)
            .take(TOOL_DETAIL_LINES)
            .copied()
            .collect()
    } else if tool.ok == Some(false) {
        lines[lines.len().saturating_sub(TOOL_LIVE_LINES)..].to_vec()
    } else {
        // Successful command output: the tail is usually what matters.
        lines[lines.len().saturating_sub(TOOL_OK_LINES)..].to_vec()
    };
    let hidden = if is_diff {
        lines.len().saturating_sub(2 + shown.len())
    } else {
        0
    };
    for l in shown {
        let style = if !is_diff {
            t.dim
        } else if l.starts_with('+') {
            t.ok
        } else if l.starts_with('-') {
            t.error
        } else if l.starts_with("@@") {
            t.accent
        } else {
            t.dim
        };
        let gutter = if is_diff { app.glyphs.gutter } else { " " };
        rows.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(gutter, style),
            Span::styled(
                truncate(&l.replace('\t', "    "), width.saturating_sub(3)),
                style,
            ),
        ]));
    }
    if hidden > 0 {
        rows.push(Line::styled(
            format!("     … {hidden} more diff lines"),
            t.dim,
        ));
    }
}

/// Under a `task` call: what its sub-agent is doing, then the start of its report.
pub(super) fn agent_rows(
    app: &App,
    run: &crate::app::AgentRun,
    tool: &ToolEntry,
    width: usize,
    rows: &mut Vec<Line<'static>>,
) {
    use crate::app::AgentItem;
    let t = &app.theme;
    let row = |rows: &mut Vec<Line<'static>>, text: &str, style: Style| {
        rows.push(Line::from(vec![
            Span::styled("     › ", t.dim),
            Span::styled(truncate(text, width.saturating_sub(7)), style),
        ]));
    };
    if tool.ok.is_none() {
        let start = run.items.len().saturating_sub(TOOL_LIVE_LINES);
        for item in &run.items[start..] {
            match item {
                AgentItem::Tool(c) => {
                    let line = if c.result.is_empty() {
                        c.summary.clone()
                    } else {
                        format!("{} · {}", c.summary, c.result)
                    };
                    let style = if c.ok == Some(false) { t.error } else { t.dim };
                    row(rows, &line, style);
                }
                AgentItem::Text(text) => {
                    if let Some(l) = text.lines().rev().find(|l| !l.trim().is_empty()) {
                        row(rows, l.trim(), t.dim.add_modifier(Modifier::ITALIC));
                    }
                }
            }
        }
        return;
    }
    let report: Vec<&str> = tool
        .detail
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    for l in report.iter().take(TOOL_OK_LINES) {
        row(rows, l.trim(), t.dim);
    }
    let more = report.len().saturating_sub(TOOL_OK_LINES);
    let more = if more > 0 {
        format!("{more} more lines · ")
    } else {
        String::new()
    };
    rows.push(Line::styled(
        format!("       {more}/tasks shows what it did"),
        t.dim,
    ));
}

pub(super) fn draw_transcript(f: &mut Frame<'_>, app: &App, area: Rect) {
    app.transcript_area.set(area);
    if app.transcript.is_empty() {
        f.render_widget(Paragraph::new(welcome(app, area)), area);
        return;
    }
    // The last column is kept for the scrollbar. Rows come from the per-entry layout
    // cache, and only the visible slice is cloned into the paragraph.
    let width = usize::from(area.width.saturating_sub(1));
    let pieces = transcript_pieces(app, width);
    let height = usize::from(area.height);
    let total: usize = pieces
        .iter()
        .map(|p| p.rows.len() + usize::from(p.blank))
        .sum();
    let max = total.saturating_sub(height);
    // Scrolled up or selecting text while a reply streams in: move with the new rows so the
    // view holds still instead of sliding out from under the cursor.
    let (drawn, _) = app.view.get();
    let mut scroll = app.scroll.get();
    let pinned = scroll > 0 || app.selection.is_some_and(|s| s.pane == Pane::Transcript);
    if pinned && drawn > 0 && total > drawn {
        scroll = scroll.saturating_add(total - drawn);
    }
    let scroll = scroll.min(max);
    app.scroll.set(scroll);
    app.view.set((total, height));

    let end = total - scroll;
    let start = end.saturating_sub(height);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(end.saturating_sub(start));
    // Row widths, collected only while a selection is painted over the rows.
    let want_widths = app.selection.is_some_and(|s| s.pane == Pane::Transcript);
    let mut widths = Vec::new();
    let mut r = 0usize;
    for piece in &pieces {
        for line in piece.rows.iter() {
            if want_widths {
                widths.push(line.width());
            }
            if r >= start && r < end {
                lines.push(line.clone());
            }
            r += 1;
        }
        if piece.blank {
            if want_widths {
                widths.push(0);
            }
            if r >= start && r < end {
                lines.push(Line::raw(""));
            }
            r += 1;
        }
    }
    f.render_widget(Paragraph::new(lines), area);
    if let Some(sel) = app.selection.filter(|s| s.pane == Pane::Transcript) {
        let text = Rect {
            width: area.width.saturating_sub(1),
            ..area
        };
        crate::select::paint(f.buffer_mut(), text, start, &sel, |r| {
            widths.get(r).copied().unwrap_or(0)
        });
    }

    if max > 0 {
        let mut state = ScrollbarState::new(max)
            .position(max - scroll)
            .viewport_content_length(height);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_symbol(app.glyphs.scroll[0])
                .thumb_style(app.theme.dim)
                .track_symbol(Some(app.glyphs.scroll[1]))
                .track_style(app.theme.border),
            area,
            &mut state,
        );
    }
    if scroll > 0 {
        // What is below, and the way back to it, on the bottom row.
        let arrow = if app.info.ui.ascii { "v" } else { "↓" };
        let hint = format!(" {arrow} {scroll} more lines · ctrl-end jumps to the latest ");
        let w = u16::try_from(hint.width())
            .unwrap_or(u16::MAX)
            .min(area.width);
        let at = Rect {
            x: area.right().saturating_sub(w + 1),
            y: area.bottom() - 1,
            width: w,
            height: 1,
        };
        f.render_widget(
            Paragraph::new(Span::styled(hint, app.theme.pill(app.theme.accent))),
            at,
        );
    }
}
