//! Inspector modals (/stats, /context, /doctor, /probe).

use super::*;

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
    if r.cache_write_tokens > 0 {
        tokens.line(field(
            theme,
            "cache write",
            format!(
                "{} · {}%",
                stat_k(r.cache_write_tokens),
                stat_pct(r.cache_write_tokens, r.prompt_tokens)
            ),
        ));
    }
    if r.reasoning_tokens > 0 {
        tokens.line(field(
            theme,
            "reasoning",
            format!(
                "{} · {}% of out",
                stat_k(r.reasoning_tokens),
                stat_pct(r.reasoning_tokens, r.completion_tokens)
            ),
        ));
    }
    if r.cache_write_tokens > 0 || r.reasoning_tokens > 0 {
        tokens.gap();
    }
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
