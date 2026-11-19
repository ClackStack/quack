//! Charts drawn in a modal's details pane, between its lines of text: bars, sparklines,
//! meters, figure tiles and a line over time. Each is built with its glyphs already
//! chosen (`ascii`), so drawing needs only the area.

use std::collections::VecDeque;

use ratatui::Frame;
use ratatui::buffer::CellWidth;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::symbols::{self, Marker};
use ratatui::text::{Line, Span, StyledGrapheme};
use ratatui::widgets::{
    Axis, Bar, BarChart, Chart, Dataset, GraphType, LineGauge, Paragraph, Sparkline,
};
use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;

/// Labels to the left of sparklines and meters are this wide.
const LABEL: u16 = 11;

/// A meter's value column, after the gauge.
const VALUE: u16 = 17;

const ASCII_BARS: symbols::bar::Set<'static> = symbols::bar::Set {
    full: "#",
    seven_eighths: "#",
    three_quarters: "=",
    five_eighths: "=",
    half: "-",
    three_eighths: "-",
    one_quarter: "_",
    one_eighth: "_",
    empty: " ",
};

#[derive(Debug, Clone)]
pub(crate) struct BarRow {
    pub label: String,
    pub value: u64,
    /// Shown inside the bar, e.g. `3m30s · 35%`.
    pub text: String,
    pub style: Style,
}

#[derive(Debug, Clone)]
pub(crate) enum Visual {
    /// Big figures side by side, each over a dim caption.
    Tiles(Vec<(String, String, Style)>),
    /// One horizontal bar per row, longest for the largest value.
    Bars {
        rows: Vec<BarRow>,
        ascii: bool,
        /// Text on a filled bar (the theme's pill text colour).
        on_bar: Style,
    },
    /// A labelled sparkline with a caption under the label (e.g. the peak).
    Spark {
        label: String,
        caption: String,
        data: Vec<u64>,
        style: Style,
        height: u16,
        ascii: bool,
    },
    /// A labelled line gauge with its value after it.
    Meter {
        label: String,
        text: String,
        ratio: f64,
        style: Style,
        ascii: bool,
    },
    /// A line over steps (turns, requests), with an optional dashed limit across it.
    Trend {
        points: Vec<(f64, f64)>,
        style: Style,
        limit: Option<(f64, Style)>,
        /// Labels for the bottom and top of the y axis, and the ends of the x axis.
        y: [String; 2],
        x: [String; 2],
        height: u16,
    },
}

impl Visual {
    /// Rows the chart wants.
    pub(crate) fn height(&self) -> u16 {
        match self {
            Self::Tiles(_) => 2,
            Self::Bars { rows, .. } => u16::try_from(rows.len()).unwrap_or(u16::MAX),
            Self::Spark { height, .. } => (*height).max(2),
            Self::Meter { .. } => 1,
            Self::Trend { height, .. } => *height,
        }
    }

    pub(crate) fn draw(&self, f: &mut Frame<'_>, area: Rect, theme: &Theme) {
        match self {
            Self::Tiles(tiles) => draw_tiles(f, area, tiles, theme),
            Self::Bars {
                rows,
                ascii,
                on_bar,
            } => draw_bars(f, area, rows, *ascii, *on_bar, theme),
            Self::Spark {
                label,
                caption,
                data,
                style,
                ascii,
                ..
            } => {
                let [l, chart] = labelled(area);
                f.render_widget(
                    Paragraph::new(vec![
                        Line::styled(label.clone(), theme.dim),
                        Line::styled(caption.clone(), *style),
                    ]),
                    l,
                );
                // The newest values, as many as fit.
                let shown = &data[data.len().saturating_sub(usize::from(chart.width))..];
                let mut s = Sparkline::default().data(shown).style(*style);
                if *ascii {
                    s = s.bar_set(ASCII_BARS);
                }
                f.render_widget(s, chart);
            }
            Self::Meter {
                label,
                text,
                ratio,
                style,
                ascii,
            } => {
                // Label, gauge, value: the gauges of a stack line up whatever the values.
                let [l, rest] = labelled(area);
                let [gauge, value] =
                    Layout::horizontal([Constraint::Min(1), Constraint::Length(VALUE)]).areas(rest);
                f.render_widget(Paragraph::new(Line::styled(label.clone(), theme.dim)), l);
                f.render_widget(
                    Paragraph::new(Line::styled(format!(" {text}"), *style)),
                    value,
                );
                let (on, off) = if *ascii {
                    ("=", "-")
                } else {
                    (symbols::line::THICK_HORIZONTAL, symbols::line::HORIZONTAL)
                };
                f.render_widget(
                    LineGauge::default()
                        .ratio(ratio.clamp(0.0, 1.0))
                        .label("")
                        .filled_symbol(on)
                        .unfilled_symbol(off)
                        .filled_style(*style)
                        .unfilled_style(theme.border),
                    gauge,
                );
            }
            Self::Trend {
                points,
                style,
                limit,
                y,
                x,
                ..
            } => draw_trend(f, area, points, *style, *limit, y, x, theme),
        }
    }
}

/// A label column and the rest.
fn labelled(area: Rect) -> [Rect; 2] {
    Layout::horizontal([Constraint::Length(LABEL), Constraint::Min(1)]).areas(area)
}

fn draw_tiles(f: &mut Frame<'_>, area: Rect, tiles: &[(String, String, Style)], theme: &Theme) {
    if tiles.is_empty() {
        return;
    }
    let cols = Layout::horizontal(vec![Constraint::Fill(1); tiles.len()])
        .spacing(1)
        .split(area);
    for ((value, caption, style), col) in tiles.iter().zip(cols.iter()) {
        f.render_widget(
            Paragraph::new(vec![
                Line::styled(value.clone(), style.add_modifier(Modifier::BOLD)),
                Line::styled(caption.clone(), theme.dim),
            ]),
            *col,
        );
    }
}

fn draw_bars(
    f: &mut Frame<'_>,
    area: Rect,
    rows: &[BarRow],
    ascii: bool,
    on_bar: Style,
    theme: &Theme,
) {
    let label_w = rows.iter().map(|r| r.label.width()).max().unwrap_or(0);
    let bars: Vec<Bar<'_>> = rows
        .iter()
        .map(|r| {
            // The text reads on the filled bar, and in the bar's colour past its end.
            let on = match r.style.fg {
                Some(c) => on_bar.bg(c),
                None => on_bar.add_modifier(Modifier::REVERSED),
            };
            Bar::with_label(
                Line::styled(format!("{:<label_w$}", r.label), theme.dim),
                r.value,
            )
            .text_value(format!(" {} ", r.text))
            .style(r.style)
            .value_style(on)
        })
        .collect();
    let mut chart = BarChart::horizontal(bars).bar_width(1).bar_gap(0);
    if ascii {
        chart = chart.bar_set(ASCII_BARS);
    }
    f.render_widget(chart, area);
}

#[expect(clippy::too_many_arguments)]
fn draw_trend(
    f: &mut Frame<'_>,
    area: Rect,
    points: &[(f64, f64)],
    style: Style,
    limit: Option<(f64, Style)>,
    y: &[String; 2],
    x: &[String; 2],
    theme: &Theme,
) {
    let x_max = points.last().map_or(1.0, |p| p.0).max(1.0);
    let x_min = points.first().map_or(0.0, |p| p.0).min(x_max - 1.0);
    let top = points
        .iter()
        .map(|p| p.1)
        .chain(limit.map(|l| l.0))
        .fold(0.0_f64, f64::max);
    let top = if top > 0.0 { top } else { 1.0 };
    let line: Vec<(f64, f64)> = limit
        .map(|(v, _)| vec![(x_min, v), (x_max, v)])
        .unwrap_or_default();
    let mut sets = Vec::new();
    if let Some((_, limit_style)) = limit {
        sets.push(
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(limit_style)
                .data(&line),
        );
    }
    sets.push(
        Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(style)
            .data(points),
    );
    let axis = |labels: &[String; 2]| {
        Axis::default()
            .style(theme.border)
            .labels(labels.iter().map(|l| Span::styled(l.clone(), theme.dim)))
    };
    let chart = Chart::new(sets)
        .x_axis(axis(x).bounds([x_min, x_max]))
        .y_axis(axis(y).bounds([0.0, top * 1.05]))
        .legend_position(None);
    f.render_widget(chart, area);
}

/// Draw `lines` with `charts` set between them (`(index, chart)`: before line `index`),
/// wrapped to `area`. Charts that no longer fit are left out; text is cut at the bottom.
pub(crate) fn draw_detail(
    f: &mut Frame<'_>,
    area: Rect,
    lines: &[Line<'static>],
    charts: &[(usize, Visual)],
    theme: &Theme,
) {
    let width = area.width;
    let mut y = area.y;
    let mut start = 0;
    let text = |f: &mut Frame<'_>, from: usize, to: usize, y: &mut u16| {
        let wrapped: Vec<Line<'_>> = lines[from..to]
            .iter()
            .flat_map(|line| wrap_line(line, width))
            .collect();
        let h = u16::try_from(wrapped.len())
            .unwrap_or(u16::MAX)
            .min(area.bottom().saturating_sub(*y));
        if h > 0 {
            f.render_widget(
                Paragraph::new(wrapped),
                Rect::new(area.x, *y, area.width, h),
            );
        }
        *y += h;
    };
    for (at, chart) in charts {
        let at = (*at).min(lines.len());
        text(f, start, at, &mut y);
        start = at;
        let h = chart.height();
        if y + h <= area.bottom() {
            chart.draw(f, Rect::new(area.x, y, area.width, h), theme);
            y += h;
        }
    }
    text(f, start, lines.len(), &mut y);
}

/// Wrap one line exactly as a Paragraph with `Wrap { trim: false }`, returning its rows.
///
/// Paragraph does not expose its reflow output. Replaying its word-wrapping state machine here
/// lets `draw_detail` use the same rows both for chart placement and the final draw, including
/// symbols wider than one cell and styles on only part of a line.
fn wrap_line<'a>(line: &'a Line<'a>, width: u16) -> Vec<Line<'a>> {
    let mut rows: Vec<Vec<StyledGrapheme<'_>>> = Vec::new();
    let mut pending_line: Vec<StyledGrapheme<'_>> = Vec::new();
    let mut pending_word: Vec<StyledGrapheme<'_>> = Vec::new();
    let mut pending_whitespace: VecDeque<StyledGrapheme<'_>> = VecDeque::new();
    let mut line_width = 0_u16;
    let mut word_width = 0_u16;
    let mut whitespace_width = 0_u16;
    let mut non_whitespace_previous = false;
    let alignment = line.alignment.unwrap_or(Alignment::Left);

    for grapheme in line.styled_graphemes(Style::default()) {
        let is_whitespace = grapheme.is_whitespace();
        let symbol_width = grapheme.symbol.cell_width();
        // Ratatui ignores symbols wider than the whole line.
        if symbol_width > width {
            continue;
        }
        let word_found = non_whitespace_previous && is_whitespace;
        let untrimmed_overflow =
            pending_line.is_empty() && word_width + whitespace_width + symbol_width > width;
        if word_found || untrimmed_overflow {
            pending_line.extend(pending_whitespace.drain(..));
            line_width += whitespace_width;
            pending_line.append(&mut pending_word);
            line_width += word_width;
            pending_whitespace.clear();
            whitespace_width = 0;
            word_width = 0;
        }
        let line_full = line_width >= width;
        let pending_word_overflow =
            symbol_width > 0 && line_width + whitespace_width + word_width >= width;
        if line_full || pending_word_overflow {
            let mut remaining_width = width.saturating_sub(line_width);
            rows.push(std::mem::take(&mut pending_line));
            line_width = 0;
            // Drop trailing whitespace that fits the just-closed row.
            while let Some(grapheme) = pending_whitespace.front() {
                let w = grapheme.symbol.cell_width();
                if w > remaining_width {
                    break;
                }
                whitespace_width -= w;
                remaining_width -= w;
                pending_whitespace.pop_front();
            }
            if is_whitespace && pending_whitespace.is_empty() {
                continue;
            }
        }
        if is_whitespace {
            whitespace_width += symbol_width;
            pending_whitespace.push_back(grapheme);
        } else {
            word_width += symbol_width;
            pending_word.push(grapheme);
        }
        non_whitespace_previous = !is_whitespace;
    }
    pending_line.extend(pending_whitespace);
    pending_line.append(&mut pending_word);
    if !pending_line.is_empty() {
        rows.push(pending_line);
    }
    if rows.is_empty() {
        rows.push(Vec::new());
    }
    rows.into_iter()
        .map(|row| Line {
            spans: row
                .into_iter()
                .map(|g| Span::styled(g.symbol, g.style))
                .collect(),
            style: Style::default(),
            alignment: Some(alignment),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Color;
    use ratatui::widgets::{Widget, Wrap};

    use super::*;

    #[test]
    fn prewrapped_lines_match_paragraph_trim_false() {
        let lines = vec![
            Line::from(vec![
                Span::styled("  alpha beta", Style::new().fg(Color::Red)),
                Span::styled(" gamma abcdefghij δε", Style::new().fg(Color::Blue)),
            ]),
            Line::raw("界 x abc  ").style(Style::new().fg(Color::Green)),
            Line::raw("   "),
            Line::raw(""),
            Line::raw("한n tune word").right_aligned(),
        ];
        for width in 1..=16 {
            let area = Rect::new(0, 0, width, 64);
            let mut expected = Buffer::empty(area);
            Paragraph::new(lines.clone())
                .wrap(Wrap { trim: false })
                .render(area, &mut expected);
            let wrapped: Vec<_> = lines.iter().flat_map(|l| wrap_line(l, width)).collect();
            let mut actual = Buffer::empty(area);
            Paragraph::new(wrapped).render(area, &mut actual);
            assert_eq!(actual, expected, "width {width}");
        }
    }
}
