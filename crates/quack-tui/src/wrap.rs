use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Details of a wrapping call which differ between plain text and markdown rows.
#[derive(Clone, Copy)]
pub(crate) struct WrapSpec<'a> {
    /// Prefix on the first row, and on every continuation row.
    pub first: &'a str,
    pub rest: &'a str,
    pub prefix_style: Style,
    /// Discard spaces at the start of a row after the prefix.
    pub trim_leading: bool,
    pub trim_wrapped: bool,
    pub trim_broken: bool,
    pub trim_final: bool,
    /// Keep a final empty row instead of dropping it after another row.
    pub keep_empty_final: bool,
    /// The plain-text variant hard-breaks any word wider than the whole row.
    pub break_long_words_anywhere: bool,
    /// Permit a zero-width break before a character wider than the row.
    pub allow_empty_break: bool,
    /// Preserve the table-cell accounting of a parting chunk on a broken word.
    pub count_chunk_twice: bool,
}

impl WrapSpec<'_> {
    fn prefix(self, text: &str) -> Vec<(String, Style)> {
        // An empty prefix carries no style and must not make a row non-empty.
        if text.is_empty() {
            Vec::new()
        } else {
            vec![(text.to_owned(), self.prefix_style)]
        }
    }
}

fn trim_end_spans(spans: &mut Vec<(String, Style)>) {
    while let Some((text, _)) = spans.last_mut() {
        let trimmed = text.trim_end();
        if trimmed.is_empty() {
            spans.pop();
        } else {
            if trimmed.len() != text.len() {
                *text = trimmed.to_owned();
            }
            break;
        }
    }
}

/// Wrap styled `spans` to `width`, breaking at spaces and then between characters.
pub(crate) fn wrap_styled(
    spans: &[(String, Style)],
    width: usize,
    spec: WrapSpec<'_>,
) -> Vec<Vec<(String, Style)>> {
    let width = width.max(1);
    let prefix_width = spec.first.width();
    let rest_width = spec.rest.width();
    let mut lines: Vec<Vec<(String, Style)>> = Vec::new();
    let mut row = spec.prefix(spec.first);
    let mut used = prefix_width;
    let mut has_text = false;

    for (text, style) in spans {
        for word in text.split_inclusive(' ') {
            let w = word.width();
            let visible = w - usize::from(word.ends_with(' '));
            if used + visible > width && has_text {
                if spec.trim_wrapped {
                    trim_end_spans(&mut row);
                }
                lines.push(std::mem::take(&mut row));
                row = spec.prefix(spec.rest);
                used = rest_width;
                has_text = false;
            }
            let word = if spec.trim_leading && !has_text {
                word.trim_start()
            } else {
                word
            };
            if word.is_empty() {
                continue;
            }
            let word_width = word.width();
            let hard_break = if spec.break_long_words_anywhere {
                word_width > width
            } else {
                used + word_width > width && !has_text
            };
            if hard_break {
                let mut chunk = String::new();
                for ch in word.chars() {
                    let cw = ch.width().unwrap_or(0);
                    if used + cw > width && (!chunk.is_empty() || spec.allow_empty_break) {
                        if !chunk.is_empty() || spec.allow_empty_break {
                            row.push((std::mem::take(&mut chunk), *style));
                        }
                        if spec.trim_broken {
                            trim_end_spans(&mut row);
                        }
                        lines.push(std::mem::take(&mut row));
                        row = spec.prefix(spec.rest);
                        used = rest_width;
                    }
                    chunk.push(ch);
                    used += cw;
                }
                if spec.count_chunk_twice {
                    used += chunk.width();
                }
                row.push((chunk, *style));
            } else {
                row.push((word.to_owned(), *style));
                used += word_width;
            }
            has_text = true;
        }
    }
    if spec.trim_final {
        trim_end_spans(&mut row);
    }
    if spec.keep_empty_final || !row.is_empty() || lines.is_empty() {
        lines.push(row);
    }
    lines
}

/// Wrap `text` to `width` columns, breaking at spaces where possible.
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let spec = WrapSpec {
            first: "",
            rest: "",
            prefix_style: Style::new(),
            trim_leading: false,
            trim_wrapped: true,
            trim_broken: false,
            trim_final: true,
            keep_empty_final: true,
            break_long_words_anywhere: true,
            allow_empty_break: true,
            count_chunk_twice: false,
        };
        let spans = [(line.to_owned(), Style::new())];
        out.extend(
            wrap_styled(&spans, width, spec)
                .into_iter()
                .map(|row| row.into_iter().map(|(text, _)| text).collect::<String>()),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::wrap;

    #[test]
    fn wraps_words_and_long_tokens() {
        assert_eq!(
            wrap("hello brave new world", 11),
            ["hello brave", "new world"]
        );
        assert_eq!(wrap("abcdefgh", 3), ["abc", "def", "gh"]);
        assert_eq!(wrap("a\n\nb", 5), ["a", "", "b"]);
    }
}
