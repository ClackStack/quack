//! Header, startup panel, input box, status bar, plan strip, and welcome screen.

use super::*;

pub(super) fn draw_header(f: &mut Frame<'_>, app: &App, area: Rect, sidebar: bool) {
    let t = &app.theme;
    let version = format!(" {}", app.info.version);
    if sidebar {
        let mut spans = vec![Span::raw(" ")];
        spans.extend(t.gradient_spans("quack"));
        spans.push(Span::styled(version, t.dim));
        f.render_widget(Line::from(spans), area);
        return;
    }
    let branch = app
        .info
        .branch
        .as_ref()
        .map(|b| format!("{}{b} ", app.glyphs.branch))
        .unwrap_or_default();
    let room = usize::from(area.width)
        .saturating_sub(" quack".width() + version.width() + 3 + branch.width() + 1);
    let cwd = &app.info.cwd;
    // Keep the end of a long path: the project folder is the useful part.
    let cwd = if cwd.width() > room && room > 1 {
        let tail: String = cwd
            .chars()
            .rev()
            .take(room - 1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("…{tail}")
    } else {
        cwd.clone()
    };
    let mut spans = vec![Span::raw(" ")];
    spans.extend(t.gradient_spans("quack"));
    spans.push(Span::styled(version, t.dim));
    spans.push(Span::styled(format!(" · {cwd}"), t.dim));
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    let pad = usize::from(area.width).saturating_sub(used + branch.width());
    if !branch.is_empty() && pad > 0 {
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(branch, t.magenta));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

pub(super) fn draw_panel(f: &mut Frame<'_>, app: &App, area: Rect) {
    let t = &app.theme;
    let g = &app.glyphs;
    let lines: Vec<Line<'_>> = app
        .startup
        .iter()
        .map(|(subsystem, state)| {
            let (icon, style, text): (&str, Style, &str) = match state {
                LoadState::Pending => (g.pending, t.dim, ""),
                LoadState::Running { detail } => (app.spinner(), t.accent, detail),
                LoadState::Ready { detail } => (g.ok, t.ok, detail),
                LoadState::NeedsAttention { detail } => (g.attention, t.warn, detail),
                LoadState::Failed { error } => (g.fail, t.error, error),
                LoadState::Disabled { reason } => (g.disabled, t.dim, reason),
            };
            Line::from(vec![
                Span::styled(format!(" {icon} "), style),
                Span::raw(format!("{:<13}", subsystem.label())),
                Span::styled(
                    text,
                    if matches!(state, LoadState::Disabled { .. }) {
                        t.dim
                    } else {
                        Style::new()
                    },
                ),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), area);
}

/// Columns for input text: the box's borders and the prompt take four.
pub(super) fn input_width(area_width: u16) -> usize {
    usize::from(area_width.saturating_sub(4))
}

/// The attached-image chips above the input, one per image: `name WxH`.
pub(super) fn draw_attachments(f: &mut Frame<'_>, app: &App, area: Rect) {
    let t = &app.theme;
    let mut spans: Vec<Span<'_>> = vec![Span::raw(" ")];
    for (n, a) in app.attachments.iter().enumerate() {
        if n > 0 {
            spans.push(Span::styled(" · ", t.dim));
        }
        let dims = match (a.width, a.height) {
            (Some(w), Some(h)) => format!(" {w}x{h}"),
            _ => String::new(),
        };
        let name = std::path::Path::new(&a.path)
            .file_name()
            .map_or_else(|| a.path.as_str().into(), |s| s.to_string_lossy());
        spans.push(Span::styled(format!("[image] {name}{dims}"), t.accent));
    }
    spans.push(Span::styled("  (backspace removes)", t.dim));
    f.render_widget(Line::from(spans), area);
}

pub(super) fn draw_input(f: &mut Frame<'_>, app: &App, area: Rect) {
    let block = Block::bordered()
        .border_set(app.glyphs.border)
        .border_style(app.theme.border);
    let inner = block.inner(area);
    let layout = app.input.layout(input_width(area.width));
    let (cur_row, cur_col) = layout.cursor;
    let visible = usize::from(inner.height).max(1);
    let first = cur_row.saturating_sub(visible - 1);
    app.input_view.set((inner, first));
    let widths: Vec<usize> = layout.rows.iter().map(|r| r.width()).collect();
    let lines: Vec<Line<'_>> = layout
        .rows
        .into_iter()
        .enumerate()
        .skip(first)
        .take(visible)
        .map(|(i, l)| {
            let lead = if i == 0 {
                format!("{} ", app.glyphs.prompt)
            } else {
                "  ".into()
            };
            Line::from(vec![Span::styled(lead, app.theme.accent), Span::raw(l)])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(block), area);
    if let Some(sel) = app.selection.filter(|s| s.pane == Pane::Input) {
        let text = Rect {
            x: inner.x + 2,
            width: inner.width.saturating_sub(2),
            ..inner
        };
        crate::select::paint(f.buffer_mut(), text, first, &sel, |r| {
            widths.get(r).copied().unwrap_or(0)
        });
    }
    if app.modal.is_none() && app.hook_form.is_none() {
        let x = inner.x + 2 + u16::try_from(cur_col).unwrap_or(0);
        let y = inner.y + u16::try_from(cur_row - first).unwrap_or(0);
        f.set_cursor_position((x.min(inner.right().saturating_sub(1)), y));
    }
}

pub(super) fn draw_status(
    f: &mut Frame<'_>,
    app: &App,
    area: Rect,
    session_sidebar: bool,
    startup_shown: bool,
) {
    let t = &app.theme;
    let mode = &app.info.permission_mode;
    let label = if mode == "yolo" {
        "YOLO"
    } else {
        mode.as_str()
    };
    let work_pill = format!(" {} ", app.info.work_mode.label());
    let perm_pill = format!(" {label} ");
    let work_w = u16::try_from(work_pill.width()).unwrap_or(0);
    let perm_w = u16::try_from(perm_pill.width()).unwrap_or(0);
    app.work_mode_area.set(Rect {
        x: area.x.saturating_add(1),
        y: area.y,
        width: work_w,
        height: area.height.min(1),
    });
    app.permission_area.set(Rect {
        x: area.x.saturating_add(2).saturating_add(work_w),
        y: area.y,
        width: perm_w,
        height: area.height.min(1),
    });
    let mut left: Vec<Span<'_>> = vec![
        Span::raw(" "),
        Span::styled(work_pill, t.pill(t.mode(app.info.work_mode.label()))),
        Span::raw(" "),
        Span::styled(perm_pill, t.pill(t.mode(mode))),
        Span::raw(" "),
    ];
    // Yolo already says the sandbox is off.
    if app.sandbox_off && mode != "yolo" {
        left.push(Span::styled("no sandbox", t.warn));
        left.push(Span::styled(" · ", t.dim));
    }
    let thinking = app.thinking_label();
    match &app.info.model {
        Some(m) => left.push(Span::raw(if thinking.is_some() {
            // Keep the effort visible beside long model names, including on resize.
            std::borrow::Cow::Owned(truncate(m, usize::from(area.width) / 2))
        } else {
            std::borrow::Cow::Borrowed(m.as_str())
        })),
        None => left.push(Span::styled("no model", t.warn)),
    }
    if let Some(thinking) = thinking {
        left.push(Span::styled(format!(" · {thinking}"), t.dim));
    }
    if !session_sidebar {
        if app.info.profile != "empty" {
            left.push(Span::styled(format!(" · {}", app.info.profile), t.dim));
        }
        if let Some((used, max)) = app.context {
            let k = crate::theme::tokens;
            left.push(Span::styled(" · ", t.dim));
            if max > 0 {
                let line = app.compact_at.unwrap_or(max).max(1);
                let frac = used as f32 / line as f32;
                let style = if frac >= 0.85 {
                    t.error
                } else if frac >= 0.6 {
                    t.warn
                } else {
                    t.dim
                };
                left.push(Span::styled(format!("{}/{}", k(used), k(max)), style));
            } else {
                left.push(Span::styled(format!("{} in context", k(used)), t.dim));
            }
        }
        if let Some(tps) = app.speed {
            left.push(Span::styled(format!(" · {tps:.0} t/s"), t.dim));
        }
    }
    let mut right: Vec<Span<'_>> = Vec::new();
    if app.approval.is_some() {
        right.push(Span::styled("waiting for approval", t.warn));
    } else if app.step_limit.is_some() {
        right.push(Span::styled("waiting to continue", t.warn));
    } else if app.turn_running {
        let clock = app
            .turn_started
            .map(|s| s.elapsed())
            .filter(|d| d.as_secs() >= 1)
            .map(|d| format!(" · {}", crate::theme::elapsed(d)))
            .unwrap_or_default();
        right.push(Span::styled(
            format!("{} {}{clock}", app.spinner(), activity(app)),
            t.accent,
        ));
        right.push(Span::styled(" · esc to interrupt", t.dim));
    } else if app.queued {
        right.push(Span::styled(format!("{} queued", app.spinner()), t.warn));
    } else if !app.startup_done {
        right.push(Span::styled(format!("{} starting", app.spinner()), t.dim));
    }
    if let Some(n) = app.copied() {
        let s = if n == 1 { "" } else { "s" };
        right.push(Span::styled(format!("  copied {n} char{s}"), t.ok));
    }
    let (errors, warnings) = app.diagnostics;
    if errors > 0 || warnings > 0 {
        let (e, w) = if app.info.ui.ascii {
            ("E", "W")
        } else {
            ("✗", "⚠")
        };
        right.push(Span::raw("  "));
        if errors > 0 {
            right.push(Span::styled(format!("{e}{errors}"), t.error));
        }
        if errors > 0 && warnings > 0 {
            right.push(Span::raw(" "));
        }
        if warnings > 0 {
            right.push(Span::styled(format!("{w}{warnings}"), t.warn));
        }
    }
    let issues = app.startup_issues();
    if issues > 0 && !app.panel_visible() && !startup_shown {
        right.push(Span::styled(
            format!("  ! {issues} startup issue(s)"),
            t.warn,
        ));
    }
    right.push(Span::styled("  /help ", t.dim));
    let right_w: usize = right.iter().map(|s| s.content.width()).sum();
    let left_w: usize = left.iter().map(|s| s.content.width()).sum();
    let pad = usize::from(area.width).saturating_sub(left_w + right_w);
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(pad)));
    spans.extend(right);
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// What the model is doing right now, for the status bar.
pub(super) fn activity(app: &App) -> String {
    match app.transcript.last() {
        Some(Entry::Reasoning(_)) => "thinking".into(),
        Some(Entry::Assistant(_)) => "writing".into(),
        Some(Entry::Tool(t)) if t.ok.is_none() => format!("running {}", t.name),
        _ => "working".into(),
    }
}

pub(crate) fn plan_mark(app: &App, state: PlanState) -> (&'static str, Style) {
    let [done, active, pending] = app.glyphs.todo;
    match state {
        PlanState::Done => (done, app.theme.ok),
        PlanState::Active => (active, app.theme.warn),
        PlanState::Pending => (pending, app.theme.dim),
    }
}

/// The plan strip shows while the model's plan has unfinished items.
pub(super) fn plan_open(app: &App) -> bool {
    app.plan.iter().any(|(s, _)| *s != PlanState::Done)
}

/// One row above the input: progress through the plan and the step in hand.
pub(super) fn draw_plan(f: &mut Frame<'_>, app: &App, area: Rect) {
    let t = &app.theme;
    let total = app.plan.len();
    let done = app
        .plan
        .iter()
        .filter(|(s, _)| *s == PlanState::Done)
        .count();
    let (state, step) = app
        .plan
        .iter()
        .find(|(s, _)| *s == PlanState::Active)
        .or_else(|| app.plan.iter().find(|(s, _)| *s == PlanState::Pending))
        .map(|(s, text)| (*s, text.as_str()))
        .unwrap_or((PlanState::Pending, ""));
    let (mark, mark_style) = plan_mark(app, state);
    let meter = crate::theme::meter(
        done as f32 / total.max(1) as f32,
        total.min(10),
        app.info.ui.ascii,
    );
    let lead = vec![
        Span::raw(" "),
        Span::styled(" plan ", t.pill(t.ok)),
        Span::raw(" "),
        Span::styled(meter, t.ok),
        Span::styled(format!(" {done}/{total}  "), t.dim),
        Span::styled(format!("{mark} "), mark_style),
    ];
    let used: usize = lead.iter().map(|s| s.content.width()).sum();
    let mut spans = lead;
    spans.push(Span::raw(truncate(
        step,
        usize::from(area.width).saturating_sub(used + 1),
    )));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// `/todo`: every step, under the one-line summary. Done steps scroll off first when the
/// plan is longer than the room.
pub(super) fn draw_plan_list(f: &mut Frame<'_>, app: &App, area: Rect) {
    let [head, list] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    draw_plan(f, app, head);
    let room = usize::from(list.height);
    let skip = app
        .plan
        .iter()
        .take_while(|(s, _)| *s == PlanState::Done)
        .count()
        .min(app.plan.len().saturating_sub(room));
    let width = usize::from(list.width).saturating_sub(6);
    let lines: Vec<Line<'static>> = app
        .plan
        .iter()
        .skip(skip)
        .take(room)
        .map(|(state, text)| {
            let (mark, style) = plan_mark(app, *state);
            let text_style = match state {
                PlanState::Done => app.theme.dim,
                PlanState::Active => style,
                PlanState::Pending => Style::default(),
            };
            Line::from(vec![
                Span::raw("   "),
                Span::styled(format!("{mark} "), style),
                Span::styled(truncate(text, width), text_style),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), list);
}

/// The empty transcript: a duck, and the few things worth knowing first.
pub(super) fn welcome(app: &App, area: Rect) -> Vec<Line<'static>> {
    let t = &app.theme;
    let hint = |key: &str, what: &str| {
        vec![
            Span::styled(key.to_owned(), t.accent),
            Span::styled(format!(" {what}"), t.dim),
        ]
    };
    let mut title = t.gradient_spans("quack");
    title.push(Span::styled(
        "  a coding agent for open-weight models",
        t.dim,
    ));
    let model = match &app.info.model {
        Some(m) => vec![
            Span::styled("model ", t.dim),
            Span::raw(m.clone()),
            Span::styled(" · /models to switch", t.dim),
        ],
        None => vec![
            Span::styled("no model yet", t.warn),
            Span::styled(" · /setup adds a provider", t.dim),
        ],
    };
    let right: Vec<Vec<Span<'static>>> = vec![
        title,
        vec![],
        model,
        [
            hint("/", "commands  "),
            hint("shift+tab", "modes  "),
            hint("/plan", "plan first"),
        ]
        .concat(),
        [
            hint("/help", "everything  "),
            hint("esc esc", "rewind  "),
            hint("/yolo", "no prompts"),
        ]
        .concat(),
    ];
    // Too narrow or short for the duck: just the tips.
    if area.width < 64 || area.height < 7 {
        let mut lines = vec![Line::raw("")];
        lines.extend(right.into_iter().skip(2).map(|mut r| {
            r.insert(0, Span::raw("   "));
            Line::from(r)
        }));
        return lines;
    }
    let (body, beak) = t.duck();

    let step = if app.info.ui.reduced_motion {
        0
    } else {
        (app.frame() / 8) % 16
    };

    let (shift, eye, beak_lower, tail, wave_idx) = match step {
        // 0: default / static pose
        0 => (0, "(o )", "_>", " /  ", 0),
        1 => (0, "(o )", "_>", " /  ", 1),
        2 => (0, "(o )", "_>", " |  ", 2),
        3 => (0, "(o )", "_>", " /  ", 3),
        // 4..9: glide forward & quack
        4 => (1, "(o )", "_>", " / ", 0),
        5 => (1, "(o )", "_>", " | ", 1),
        6 => (1, "(o )", "_>", " / ", 2),
        7 => (1, "(o )", " >", " / ", 3), // quack!
        8 => (1, "(o )", " >", " / ", 0), // quack!
        9 => (1, "(o )", "_>", " / ", 1),
        // 10..15: glide back, blink & rest
        10 => (0, "(o )", "_>", " |  ", 2),
        11 => (0, "(o )", "_>", " /  ", 3),
        12 => (0, "(o )", "_>", " /  ", 0),
        13 => (0, "(- )", "_>", " /  ", 1), // blink!
        14 => (0, "(o )", "_>", " /  ", 2),
        _ => (0, "(o )", "_>", " /  ", 3),
    };

    let water = if app.info.ui.ascii {
        match wave_idx {
            1 | 3 => "~-~-~-~-~-",
            2 => "-~-~-~-~-~",
            _ => "~~~~~~~~~~",
        }
    } else {
        match wave_idx {
            1 | 3 => "~≈~≈~≈~≈~≈",
            2 => "≈~≈~≈~≈~≈~",
            _ => "~~~~~~~~~~",
        }
    };

    let duck: [Vec<Span<'static>>; 5] = if shift == 0 {
        [
            vec![Span::styled("      __      ", body)],
            vec![
                Span::styled("    ", body),
                Span::styled("<", beak),
                Span::styled(format!("{eye}___  "), body),
            ],
            vec![
                Span::styled("     ( .", body),
                Span::styled(beak_lower, beak),
                Span::styled(tail, body),
            ],
            vec![Span::styled("      `---'   ", body)],
            vec![
                Span::styled("   ", t.accent),
                Span::styled(format!("{water} "), t.accent),
            ],
        ]
    } else {
        [
            vec![Span::styled("       __     ", body)],
            vec![
                Span::styled("     ", body),
                Span::styled("<", beak),
                Span::styled(format!("{eye}___ "), body),
            ],
            vec![
                Span::styled("      ( .", body),
                Span::styled(beak_lower, beak),
                Span::styled(tail, body),
            ],
            vec![Span::styled("       `---'  ", body)],
            vec![
                Span::styled("   ", t.accent),
                Span::styled(format!("{water} "), t.accent),
            ],
        ]
    };
    let mut lines = vec![Line::raw("")];
    for (d, r) in duck.into_iter().zip(right) {
        let mut spans = d;
        if !r.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.extend(r);
        lines.push(Line::from(spans));
    }
    lines
}
