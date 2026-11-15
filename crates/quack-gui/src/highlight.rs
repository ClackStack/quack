//! Syntax highlighting for markdown code blocks using the built-in tree-sitter grammars.

use quack_tools::grammar::{Grammars, language_for_tag};
use serde::Serialize;

/// A contiguous run of text within a code block with an optional highlight capture name.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HighlightSpan {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
}

/// Highlights `code` using the built-in tree-sitter grammar matching `lang`.
///
/// Returns `None` when no compiled grammar matches `lang` or when the crate was built
/// without `lang-common` (for example in `--no-default-features` CI builds). Adjacent
/// runs that resolve to the same `kind` are merged to keep serialized payloads compact.
pub fn highlight_code(lang: &str, code: &str) -> Option<Vec<HighlightSpan>> {
    if code.is_empty() || !Grammars::compiled_in() {
        return None;
    }
    let canonical = language_for_tag(lang)?;
    let runs = Grammars::builtin().highlight(canonical, code)?;
    let mut out: Vec<HighlightSpan> = Vec::with_capacity(runs.len());
    for (range, kind) in runs {
        let slice = code.get(range)?;
        if slice.is_empty() {
            continue;
        }
        if let Some(last) = out.last_mut()
            && last.kind == kind
        {
            last.text.push_str(slice);
        } else {
            out.push(HighlightSpan {
                text: slice.to_owned(),
                kind,
            });
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_none_for_unknown_or_empty_input() {
        assert_eq!(highlight_code("", "let x = 1;"), None);
        assert_eq!(highlight_code("unknownlang", "abc"), None);
        assert_eq!(highlight_code("rust", ""), None);
    }

    #[cfg(feature = "lang-common")]
    #[test]
    fn highlights_rust_including_non_ascii_comments() {
        let code = "fn main() { let s = \"café — →\"; } // résumé\n";
        let spans = highlight_code("rust", code).expect("rust grammar should highlight");
        let joined: String = spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(joined, code);
        assert!(
            spans
                .iter()
                .any(|s| s.kind == Some("keyword") && s.text == "fn")
        );
        assert!(spans.iter().any(|s| s.kind == Some("comment")));
    }
}
