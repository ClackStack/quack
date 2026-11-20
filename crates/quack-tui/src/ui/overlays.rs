//! Popups, prompt modals, approval cards, and choice bars.

use super::*;

/// `/setup`'s question box: a URL or a key, over everything else. A key is drawn as dots.
pub(super) fn draw_prompt(f: &mut Frame<'_>, app: &App) {
    let Some(p) = &app.prompt else {
        return;
    };
    let t = &app.theme;
    let screen = f.area();
    let width = screen.width.saturating_sub(4).min(72);
    let inner_w = usize::from(width.saturating_sub(4));
    // A blank row under the title.
    let mut lines: Vec<Line<'static>> = vec![Line::raw("")];
    for h in &p.help {
        lines.extend(wrap(h, inner_w).into_iter().map(|l| Line::styled(l, t.dim)));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(p.label.clone(), t.accent));
    let shown = if p.secret {
        let dot = if app.info.ui.ascii { "*" } else { "•" };
        dot.repeat(p.text.chars().count())
    } else {
        p.text.clone()
    };
    // Keep the end of a long answer in view, where the cursor is.
    let room = inner_w.saturating_sub(3);
    let mut tail: String = shown.chars().rev().take(room).collect();
    tail = tail.chars().rev().collect();
    let field_row = lines.len();
    lines.push(Line::from(vec![
        Span::styled("› ", t.accent),
        Span::raw(tail.clone()),
    ]));
    if let Some(e) = &p.error {
        lines.extend(
            wrap(e, inner_w)
                .into_iter()
                .map(|l| Line::styled(l, t.error)),
        );
    }
    let height = (u16::try_from(lines.len()).unwrap_or(u16::MAX) + 2).min(screen.height);
    let area = Rect {
        x: screen.x + (screen.width - width) / 2,
        y: screen.y + screen.height.saturating_sub(height) / 2,
        width,
        height,
    };
    let mut prompt_hints = vec![("⏎", p.enter_does())];
    if let Some(scope) = p.scope {
        prompt_hints.push((
            "^l",
            match scope {
                WriteScope::Project => "scope: .quack",
                WriteScope::User | WriteScope::Session => "scope: user",
            },
        ));
    }
    prompt_hints.extend([("^u", "clear"), ("esc", "back")]);
    let block = Block::bordered()
        .border_set(app.glyphs.border)
        .border_style(t.accent)
        .padding(ratatui::widgets::Padding::horizontal(1))
        .title(Span::styled(format!(" {} ", p.title), t.pill(t.accent)))
        .title_bottom(key_hints(t, &prompt_hints).right_aligned());
    let inner = block.inner(area);
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(lines).block(block), area);
    let x = inner.x + 2 + u16::try_from(tail.width()).unwrap_or(0);
    let y = inner.y + u16::try_from(field_row).unwrap_or(0);
    if y < inner.bottom() {
        f.set_cursor_position((x.min(inner.right().saturating_sub(1)), y));
    }
}

pub(super) fn draw_popup(f: &mut Frame<'_>, app: &App, above: Rect) {
    let matches = app.popup_matches();
    if matches.is_empty() {
        return;
    }
    let t = &app.theme;
    let g = &app.glyphs;
    let is_files = matches.first().is_some_and(|i| i.prefix == '@');
    let query = app.popup_query().to_owned();
    let width = above.width;
    let inner_w = usize::from(width.saturating_sub(2));
    let selected = app.popup_selected.min(matches.len() - 1);

    let label = |i: &PopupItem| format!("{}{}", i.prefix, i.name);
    let max_col = if is_files {
        inner_w.saturating_sub(4).max(10)
    } else {
        30
    };
    let col = matches
        .iter()
        .map(|i| {
            label(i).width()
                + if i.args.is_empty() {
                    0
                } else {
                    i.args.width() + 1
                }
        })
        .max()
        .unwrap_or(0)
        .clamp(10, max_col);
    let tag = |i: &PopupItem| match (i.prefix, i.command, i.hint) {
        ('@', _, _) => "",
        (_, None, _) => "skill",
        (_, Some(_), Some(_)) => "soon",
        _ => "",
    };
    let room = |i: &PopupItem| {
        let tag_w = if tag(i).is_empty() {
            0
        } else {
            tag(i).width() + 2
        };
        inner_w.saturating_sub((2 + col + 2).max(2 + label(i).width() + 1) + tag_w + 1)
    };
    // Under the list: what the selected row could not show (the rest of its description,
    // aliases, when it lands), sized for the tallest so the popup doesn't jump.
    let lead_w = 2 + g.assistant.width();
    let about_w = inner_w.saturating_sub(lead_w).max(10);
    let about = |i: &PopupItem| {
        let mut parts = Vec::new();
        if i.description.width() > room(i) {
            parts.push(i.description.clone());
        }
        if let Some(c) = i.command
            && !c.aliases.is_empty()
        {
            let also: Vec<String> = c.aliases.iter().map(|a| format!("/{a}")).collect();
            parts.push(format!("also {}", also.join(", ")));
        }
        if let Some(m) = i.command.and_then(|c| c.pending) {
            parts.push(format!("coming in {m}"));
        }
        if parts.is_empty() {
            return Vec::new();
        }
        let mut lines = wrap(&parts.join(" · "), about_w);
        lines.truncate(2);
        lines
    };
    let rows = matches.len().min(MAX_POPUP_ROWS);
    let first = selected.saturating_sub(rows - 1);
    let about_h = matches
        .iter()
        .skip(first)
        .take(rows)
        .map(|i| about(i).len())
        .max()
        .unwrap_or(0);
    let mut about_lines = about(&matches[selected]);
    about_lines.resize(about_h, String::new());

    let extra = if about_h > 0 { about_h + 1 } else { 0 };
    let height = u16::try_from(rows + extra + 2).unwrap_or(u16::MAX);
    if above.height < height {
        return;
    }
    let area = Rect {
        x: above.x,
        y: above.bottom() - height,
        width,
        height,
    };

    let mut lines: Vec<Line<'_>> = matches
        .iter()
        .enumerate()
        .skip(first)
        .take(rows)
        .map(|(i, item)| {
            let on = i == selected;
            let row = if on { t.selected } else { Style::new() };
            let soft = if on { t.sel_text } else { t.dim };
            let tag = tag(item);
            let tag_style = if item.command.is_none() {
                t.magenta
            } else {
                soft
            };
            let name = label(item);
            let hits: Vec<usize> = fuzzy::positions(&query, &item.name)
                .into_iter()
                .map(|p| p + 1)
                .collect();
            let mut spans = vec![
                Span::styled(if on { g.bar } else { " " }, row.patch(t.accent)),
                Span::styled(" ", row),
            ];
            spans.extend(t.highlight(
                &name,
                &hits,
                row.patch(t.accent).add_modifier(Modifier::BOLD),
            ));
            let mut used = 2 + name.width();
            if !item.args.is_empty() {
                let args = truncate(&item.args, col.saturating_sub(name.width() + 1).max(3));
                used += args.width() + 1;
                spans.push(Span::styled(format!(" {args}"), row.patch(soft)));
            }
            let pad = (2 + col + 2).saturating_sub(used);
            spans.push(Span::styled(" ".repeat(pad), row));
            used += pad;
            let tag_w = if tag.is_empty() { 0 } else { tag.width() + 2 };
            let desc_style = if on {
                t.selected.remove_modifier(Modifier::BOLD)
            } else if item.hint.is_some() && item.command.is_some() {
                t.dim
            } else {
                Style::new()
            };
            let desc = truncate(&item.description, room(item));
            used += desc.width();
            spans.push(Span::styled(desc, desc_style));
            let fill = inner_w.saturating_sub(used + tag_w);
            spans.push(Span::styled(" ".repeat(fill), row));
            if !tag.is_empty() {
                spans.push(Span::styled(format!("{tag} "), row.patch(tag_style)));
                spans.push(Span::styled(" ", row));
            }
            Line::from(spans)
        })
        .collect();
    if about_h > 0 {
        lines.push(Line::styled(g.rule.repeat(inner_w), t.border));
    }
    for (n, l) in about_lines.into_iter().enumerate() {
        let mut spans = assistant_lead(app, n == 0 && !l.is_empty());
        spans.push(Span::styled(l, t.sel_text));
        lines.push(Line::from(spans));
    }

    let count = if matches.len() > rows {
        format!(" {} of {} ", selected + 1, matches.len())
    } else {
        format!(" {} ", matches.len())
    };
    let keys = key_hints(
        t,
        &[
            ("↑↓", "move"),
            ("tab", "complete"),
            ("⏎", if is_files { "insert" } else { "run" }),
            ("esc", "close"),
        ],
    );
    let title = if is_files { " files " } else { " commands " };
    f.render_widget(Clear, area);
    let block = Block::bordered()
        .border_set(g.border)
        .border_style(t.border)
        .title_top(Line::from(Span::styled(title, t.pill(t.accent))))
        .title_top(Line::styled(count, t.dim).right_aligned())
        .title_bottom(keys.right_aligned());
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// `key what · key what`, for popup and modal footers.
pub(crate) fn key_hints(t: &crate::theme::Theme, keys: &[(&str, &str)]) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (i, (key, what)) in keys.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", t.border));
        }
        spans.push(Span::styled(
            (*key).to_owned(),
            t.accent.add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {what}"), t.dim));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

/// Bottom overlay asking to allow a tool call.
pub(super) fn draw_approval(f: &mut Frame<'_>, app: &App, above: Rect) {
    let Some(approval) = &app.approval else {
        return;
    };
    let t = &app.theme;
    let (title, body): (String, Vec<Line<'_>>) = match &approval.request {
        ApprovalRequest::Shell {
            command,
            cwd,
            outside_sandbox,
        } => (
            match (cwd.as_str(), outside_sandbox) {
                (".", false) => " Run this command? ".to_owned(),
                (".", true) => " Run this command outside the sandbox? ".to_owned(),
                (_, false) => format!(" Run this command in {cwd}? "),
                (_, true) => format!(" Run this command in {cwd}, outside the sandbox? "),
            },
            command
                .lines()
                .map(|l| Line::from(vec![Span::styled("$ ", t.accent), Span::raw(l.to_owned())]))
                .collect(),
        ),
        ApprovalRequest::Edit { path, diff } => (
            format!(" Apply this change to {path}? "),
            diff.lines()
                .skip(2)
                .map(|l| {
                    let style = if l.starts_with('+') {
                        t.ok
                    } else if l.starts_with('-') {
                        t.error
                    } else if l.starts_with("@@") {
                        t.accent
                    } else {
                        t.dim
                    };
                    Line::styled(l.replace('\t', "    "), style)
                })
                .collect(),
        ),
        ApprovalRequest::Tool { name, arguments } => (
            format!(" Allow {name}? "),
            vec![Line::styled(arguments.clone(), t.dim)],
        ),
        ApprovalRequest::EgressSecrets {
            provider,
            host,
            findings,
        } => (
            format!(" Potential secret bound for {provider} ({host}) "),
            findings
                .iter()
                .map(|f| {
                    Line::from(vec![
                        Span::styled(format!(" {} ", f.kind), t.error),
                        Span::raw(format!(" {} · ", f.location)),
                        Span::styled(f.preview.clone(), t.accent),
                        Span::styled(format!(" ({})", f.fingerprint), t.dim),
                    ])
                })
                .collect(),
        ),
    };
    let is_secrets = matches!(approval.request, ApprovalRequest::EgressSecrets { .. });
    // Borders, a blank row under the title and one above the keys, the rules `A` saves,
    // and the keys.
    let always = !approval.always.is_empty();
    let extra = u16::from(always);
    let max_body = usize::from(above.height.saturating_sub(6 + extra)).max(1);
    let shown = body.len().min(max_body);
    let height = u16::try_from(shown).unwrap_or(0) + 5 + extra;
    let area = Rect {
        x: above.x,
        y: above.bottom().saturating_sub(height),
        width: above.width,
        height: height.min(above.height),
    };
    let mut lines: Vec<Line<'_>> = vec![Line::raw("")];
    lines.extend(body.into_iter().take(shown));
    lines.push(Line::raw(""));
    if always {
        let target = match approval.always_scope {
            WriteScope::Project => ".quack",
            WriteScope::User | WriteScope::Session => "user",
        };
        let toggle = if matches!(
            app.info.project_status,
            ProjectStatus::Trusted | ProjectStatus::SettingsOnly
        ) {
            " · ^l scope"
        } else {
            ""
        };
        lines.push(Line::styled(
            format!(
                "always allow ({target}) saves {}{toggle}",
                approval.always.join(", ")
            ),
            t.dim,
        ));
    }
    let mut keys = Vec::new();
    if is_secrets {
        keys.extend([
            Span::styled(" r ", t.pill(t.ok)),
            Span::raw(" redact & send  "),
        ]);
    }
    keys.extend([
        Span::styled(" y ", t.pill(t.ok)),
        Span::raw(" allow  "),
        Span::styled(" a ", t.pill(t.accent)),
        Span::raw(" this session  "),
    ]);
    if always {
        keys.extend([
            Span::styled(" A ", t.pill(t.accent)),
            Span::raw(" always  "),
        ]);
    }
    keys.extend([
        Span::styled(" n ", t.pill(t.error)),
        Span::raw(" deny  "),
        Span::styled("ctrl-c interrupts", t.dim),
    ]);
    lines.push(Line::from(keys));
    f.render_widget(Clear, area);
    let block = Block::bordered()
        .title(Span::styled(title, t.pill(t.warn)))
        .border_set(app.glyphs.border)
        .border_style(t.warn)
        .padding(ratatui::widgets::Padding::horizontal(1));
    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// The question or plan review bar, built before layout so it can take its rows.
pub(super) struct Choices<'a> {
    title: String,
    /// The question, or the plan; it gives way first when rows are short.
    body: Vec<Line<'a>>,
    /// The choices and the keys line.
    rows: Vec<Line<'a>>,
}

impl Choices<'_> {
    /// Rows wanted, borders included.
    pub(super) fn height(&self) -> usize {
        let spacer = usize::from(!self.body.is_empty());
        self.body.len() + spacer + self.rows.len() + 2
    }
}

/// Numbered choices above the input: the model's questions (`ask_user`), or the plan
/// review once a plan-mode turn ends. The input stays live for typed answers.
pub(super) fn choices(app: &App, width: usize) -> Option<Choices<'_>> {
    if app.approval.is_some() {
        return None;
    }
    let t = &app.theme;
    let (arrows, left) = if app.info.ui.ascii {
        ("up/down", "left")
    } else {
        ("↑↓", "←")
    };
    // (label, description, recommended)
    let mut lead: Vec<Line<'_>> = Vec::new();
    let mut options: Vec<(String, Option<String>, bool)> = Vec::new();
    let (title, body, selected, own, keys) = if let Some(a) = &app.asking {
        let q = a.question();
        let n = a.questions.len();
        let count = if n > 1 {
            format!("{}/{n}", a.current + 1)
        } else {
            String::new()
        };
        let title = match (q.header.is_empty(), count.is_empty()) {
            (true, true) => " Question ".to_owned(),
            (true, false) => format!(" Question {count} "),
            (false, true) => format!(" {} ", q.header),
            (false, false) => format!(" {} · {count} ", q.header),
        };
        // The question is part of the choices, never cut.
        let body: Vec<Line<'_>> = Vec::new();
        lead = wrap(&q.question, width.saturating_sub(1))
            .into_iter()
            .map(|l| Line::styled(format!(" {l}"), Style::new().add_modifier(Modifier::BOLD)))
            .collect();
        for (i, o) in q.options.iter().enumerate() {
            options.push((
                o.label.clone(),
                o.description.clone(),
                q.recommended == Some(i),
            ));
        }
        let own = if app.input.is_empty() {
            "Type your own answer below"
        } else {
            "Your own answer (Enter sends it)"
        };
        let back = if a.current > 0 {
            format!("  {left} back")
        } else {
            String::new()
        };
        let keys = format!(
            " {arrows} choose  enter confirm  esc skip{back}  ctrl-s stop asking  ctrl-c interrupts"
        );
        (title, body, a.selected, Some(own), keys)
    } else if let Some(s) = &app.step_limit {
        lead = wrap(
            &format!(
                "The turn made {} model calls, the limit set by agent.max_steps.",
                s.limit
            ),
            width.saturating_sub(1),
        )
        .into_iter()
        .map(|l| Line::styled(format!(" {l}"), Style::new().add_modifier(Modifier::BOLD)))
        .collect();
        options.push((
            format!("Keep going for {} more", s.limit),
            Some("the conversation and tool state stay as they are".into()),
            true,
        ));
        options.push((
            "Stop here".into(),
            Some("nothing is lost; send a message to continue".into()),
            false,
        ));
        let keys = format!(" {arrows} choose  enter confirm  esc stop  or type a redirect");
        (
            " Keep going? ".to_owned(),
            Vec::new(),
            s.selected,
            None,
            keys,
        )
    } else {
        let selected = app.review?;
        let body: Vec<Line<'_>> = app
            .plan
            .iter()
            .map(|(state, text)| {
                let (mark, style) = plan_mark(app, *state);
                Line::from(vec![
                    Span::styled(format!(" {mark} "), style),
                    Span::raw(truncate(text, width.saturating_sub(4))),
                ])
            })
            .collect();
        for (_, label) in app.review_choices() {
            options.push((label, None, false));
        }
        let keys =
            format!(" {arrows} choose  enter confirm  esc keep planning  or type what to change");
        (
            " Ready to carry out this plan? ".to_owned(),
            body,
            selected,
            None,
            keys,
        )
    };

    let mut rows = lead;
    for (i, (label, description, recommended)) in options.iter().enumerate() {
        let on = i == selected;
        let lead = if on { app.glyphs.bar } else { " " };
        let style = if on {
            t.accent.add_modifier(Modifier::BOLD)
        } else {
            Style::new()
        };
        let mut spans = vec![
            Span::styled(lead.to_owned(), t.accent),
            Span::styled(format!("{}. {label}", i + 1), style),
        ];
        if *recommended {
            spans.push(Span::styled(" (recommended)", t.ok));
        }
        if let Some(d) = description {
            let used: usize = spans.iter().map(|s| s.content.width()).sum();
            let room = width.saturating_sub(used + 3);
            if room > 8 {
                spans.push(Span::styled(format!(" · {}", truncate(d, room)), t.dim));
            }
        }
        rows.push(Line::from(spans));
    }
    if let Some(own) = own {
        let on = selected == options.len();
        let lead = if on { app.glyphs.bar } else { " " };
        let style = if on { t.accent } else { t.dim };
        rows.push(Line::from(vec![
            Span::styled(lead.to_owned(), t.accent),
            Span::styled(format!("{}. {own}", options.len() + 1), style),
        ]));
    }
    rows.push(Line::styled(truncate(&keys, width), t.dim));
    Some(Choices { title, body, rows })
}

pub(super) fn draw_choices(f: &mut Frame<'_>, app: &App, area: Rect, c: Choices<'_>) {
    if area.height == 0 {
        return;
    }
    let t = &app.theme;
    // The choices first; a long body (a plan) gives way, keeping its first lines.
    let room = usize::from(area.height)
        .saturating_sub(2 + c.rows.len() + 1)
        .min(c.body.len());
    let mut lines: Vec<Line<'_>> = c.body.into_iter().take(room).collect();
    if !lines.is_empty() {
        lines.push(Line::raw(""));
    }
    lines.extend(c.rows);
    let title = c.title;
    let block = Block::bordered()
        .title(Span::styled(title, t.pill(t.accent)))
        .border_set(app.glyphs.border)
        .border_style(t.accent);
    f.render_widget(Paragraph::new(lines).block(block), area);
}
