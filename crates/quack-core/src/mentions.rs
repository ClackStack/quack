//! Precision `@file#symbol` and `@file:Lstart-Lend` context attachment expansion.
//! Slices are attached to the user's message so system prompt + tool prefix stay cached.

use std::collections::HashSet;
use std::path::Path;

use quack_tools::grammar::Grammars;
use quack_tools::outline;
use quack_tools::paths;

const MAX_SLICES: usize = 6;
const MAX_TOTAL_BYTES: usize = 48 * 1024;
const MAX_SLICE_BYTES: usize = 16 * 1024;
const MAX_LINE_SPAN: usize = 400;

/// Expanded context block and non-fatal resolution warnings for one user prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MentionExpansion {
    pub attachments: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Target<'a> {
    Symbol {
        path: &'a str,
        symbol: &'a str,
    },
    Lines {
        path: &'a str,
        start: usize,
        end: usize,
    },
}

/// Parse `@file#symbol` and `@file:Lstart-Lend` tokens from `prompt` and resolve bounded
/// workspace slices. Unparseable tokens or non-existent paths outside known extensions are ignored.
pub fn expand(root: &Path, grammars: &Grammars, prompt: &str) -> MentionExpansion {
    let mut block = String::new();
    let mut warnings = Vec::new();
    let mut seen = HashSet::new();
    let mut slice_count = 0usize;
    let mut total_bytes = 0usize;

    for raw in prompt.split_whitespace() {
        let Some(token) = strip_wrappers(raw) else {
            continue;
        };
        let Some(target) = parse_target(token) else {
            continue;
        };
        if !seen.insert(target.clone()) {
            continue;
        }

        if slice_count >= MAX_SLICES || total_bytes >= MAX_TOTAL_BYTES {
            append_fallback(&mut block, &target);
            continue;
        }

        let remaining = (MAX_TOTAL_BYTES - total_bytes).min(MAX_SLICE_BYTES);
        match target {
            Target::Symbol { path, symbol } => {
                if !looks_like_file_path(root, path) {
                    continue;
                }
                match outline::extract_symbol_slice(root, grammars, path, symbol, remaining) {
                    Ok(slice) => {
                        let header = format!(
                            "<attached_context path=\"{}\" symbol=\"{}\" lines=\"{}-{}\"{}>\n",
                            slice.path,
                            symbol,
                            slice.symbol.start,
                            slice.symbol.end,
                            if slice.truncated {
                                " truncated=\"true\""
                            } else {
                                ""
                            }
                        );
                        let footer = "</attached_context>\n";
                        let chunk = format!("{header}{}{footer}", slice.text);
                        if !block.is_empty() {
                            block.push('\n');
                        }
                        total_bytes += chunk.len();
                        slice_count += 1;
                        block.push_str(&chunk);
                    }
                    Err(err) => {
                        warnings.push(format!("@{path}#{symbol}: {err}"));
                    }
                }
            }
            Target::Lines { path, start, end } => {
                if !looks_like_file_path(root, path) {
                    continue;
                }
                match extract_line_slice(root, path, start, end, remaining) {
                    Ok((shown, text, actual_end, truncated)) => {
                        let header = format!(
                            "<attached_context path=\"{}\" lines=\"{}-{}\"{}>\n",
                            shown,
                            start,
                            actual_end,
                            if truncated { " truncated=\"true\"" } else { "" }
                        );
                        let footer = "</attached_context>\n";
                        let chunk = format!("{header}{text}{footer}");
                        if !block.is_empty() {
                            block.push('\n');
                        }
                        total_bytes += chunk.len();
                        slice_count += 1;
                        block.push_str(&chunk);
                    }
                    Err(err) => warnings.push(format!("@{path}:L{start}-{end}: {err}")),
                }
            }
        }
    }

    MentionExpansion {
        attachments: block,
        warnings,
    }
}

fn append_fallback(block: &mut String, target: &Target<'_>) {
    if !block.is_empty() {
        block.push('\n');
    }
    match target {
        Target::Symbol { path, symbol } => block.push_str(&format!(
            "[attachment budget reached for @{path}#{symbol}; call read(path=\"{path}\") or index(\"{path}\")]\n"
        )),
        Target::Lines { path, start, end } => block.push_str(&format!(
            "[attachment budget reached for @{path}:L{start}-{end}; call read(path=\"{path}\", offset={start}, limit={})]\n",
            end.saturating_sub(*start).max(1)
        )),
    }
}

fn strip_wrappers(token: &str) -> Option<&str> {
    let trimmed = token.trim_start_matches(['(', '[', '{', '"', '\'', '`']);
    let body = trimmed.strip_prefix('@')?;
    let cleaned = body.trim_end_matches([',', '.', ';', ')', ']', '}', '"', '\'', '`']);
    (!cleaned.is_empty()).then_some(cleaned)
}

fn parse_target(token: &str) -> Option<Target<'_>> {
    if let Some((path, symbol)) = token.split_once('#')
        && !path.is_empty()
        && !symbol.is_empty()
    {
        return Some(Target::Symbol { path, symbol });
    }
    if let Some((path, line_spec)) = token.rsplit_once(":L")
        && !path.is_empty()
    {
        if let Some((a, b)) = line_spec.split_once('-') {
            let start: usize = a.parse().ok()?;
            let end_str = b.strip_prefix('L').unwrap_or(b);
            let end: usize = end_str.parse().ok()?;
            if start >= 1 && end >= start {
                return Some(Target::Lines {
                    path,
                    start,
                    end: end.min(start + MAX_LINE_SPAN - 1),
                });
            }
        } else if let Ok(line) = line_spec.parse::<usize>()
            && line >= 1
        {
            return Some(Target::Lines {
                path,
                start: line,
                end: line,
            });
        }
    }
    None
}

fn looks_like_file_path(root: &Path, rel: &str) -> bool {
    rel.contains('/')
        || rel.contains('\\')
        || rel.contains('.')
        || paths::resolve(root, rel).exists()
}

fn extract_line_slice(
    root: &Path,
    rel_path: &str,
    start: usize,
    end: usize,
    max_bytes: usize,
) -> Result<(String, String, usize, bool), String> {
    let resolved = paths::resolve(root, rel_path);
    let shown = paths::display(root, &resolved);
    if !paths::inside(root, &resolved) {
        return Err(format!("{shown} is outside the workspace"));
    }
    let canon_root = std::fs::canonicalize(root).map_err(|e| format!("{shown}: {e}"))?;
    let canon_file = std::fs::canonicalize(&resolved).map_err(|e| format!("{shown}: {e}"))?;
    if !paths::inside(&canon_root, &canon_file) {
        return Err(format!("{shown} escapes the workspace"));
    }
    let text = std::fs::read_to_string(&canon_file).map_err(|e| format!("{shown}: {e}"))?;
    let lines: Vec<&str> = text.lines().collect();
    if start > lines.len() {
        return Err(format!("line {start} past EOF ({} lines)", lines.len()));
    }
    let start_idx = start - 1;
    let end_idx = end.min(lines.len());
    let mut out = String::new();
    let mut truncated = false;
    let mut actual_end = start;
    for (offset, line) in lines[start_idx..end_idx].iter().enumerate() {
        let lineno = start_idx + offset + 1;
        let row = format!("{lineno:>5}: {line}\n");
        if out.len() + row.len() > max_bytes && !out.is_empty() {
            truncated = true;
            break;
        }
        out.push_str(&row);
        actual_end = lineno;
    }
    Ok((shown, out, actual_end, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_symbol_and_line_slice_and_caps_slices() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        std::fs::write(
            root.join("src/lib.rs"),
            "pub fn alpha() -> i32 {\n    1\n}\n\npub fn beta() -> i32 {\n    2\n}\n",
        )
        .expect("write");
        let grammars = Grammars::builtin();

        let exp = expand(
            root,
            &grammars,
            "Compare @src/lib.rs#alpha with @src/lib.rs:L5-L7 please",
        );
        if outline::supported(Path::new("src/lib.rs")) {
            assert!(exp.warnings.is_empty(), "warnings: {:?}", exp.warnings);
            assert!(exp.attachments.contains("symbol=\"alpha\""));
            assert!(exp.attachments.contains("pub fn alpha()"));
        }
        assert!(exp.attachments.contains("lines=\"5-7\""));
        assert!(exp.attachments.contains("pub fn beta()"));

        // Seven distinct slices hits MAX_SLICES (6) and emits fallback hint on #7.
        let many = (1..=7)
            .map(|n| format!("@src/lib.rs:L{n}"))
            .collect::<Vec<_>>()
            .join(" ");
        let capped = expand(root, &grammars, &many);
        assert!(capped.attachments.contains("attachment budget reached"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("outside");
        std::fs::write(outside.path().join("secret.rs"), "pub fn secret() {}\n").expect("write");
        std::os::unix::fs::symlink(outside.path().join("secret.rs"), dir.path().join("link.rs"))
            .expect("symlink");
        let grammars = Grammars::builtin();
        let exp = expand(dir.path(), &grammars, "inspect @link.rs:L1-L1");
        assert!(!exp.warnings.is_empty());
        assert!(!exp.attachments.contains("pub fn secret"));
    }
}
