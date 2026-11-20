//! Precision `@file`, `@file#symbol`, `@file:Lstart-Lend`, and `InputItem::File` context
//! attachment expansion. Slices are attached to the user's message so system prompt + tool
//! prefix stay cached.

use std::collections::HashSet;
use std::path::Path;

use quack_tools::grammar::Grammars;
use quack_tools::outline;
use quack_tools::paths;

const MAX_SLICES: usize = 6;
const MAX_TOTAL_BYTES: usize = 48 * 1024;
const MAX_SLICE_BYTES: usize = 16 * 1024;
const MAX_LINE_SPAN: usize = 400;
const MAX_INLINE_FILE_LINES: usize = 200;

/// Expanded context block and non-fatal resolution warnings for one user prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MentionExpansion {
    pub attachments: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Target {
    Symbol {
        path: String,
        symbol: String,
    },
    Lines {
        path: String,
        start: usize,
        end: usize,
    },
    File {
        path: String,
    },
}

/// Parse `@file`, `@file#symbol`, and `@file:Lstart-Lend` tokens from `prompt` and resolve
/// bounded workspace slices.
#[cfg(test)]
pub fn expand(root: &Path, grammars: &Grammars, prompt: &str) -> MentionExpansion {
    expand_with_files(root, grammars, prompt, &[])
}

/// Expand inline `@...` mentions in `prompt` alongside explicit `InputItem::File` paths.
/// Unparseable tokens or non-existent plain `@mentions` are ignored; explicit `InputItem::File`
/// entries report a warning if missing or outside the workspace.
pub fn expand_with_files(
    root: &Path,
    grammars: &Grammars,
    prompt: &str,
    explicit_files: &[String],
) -> MentionExpansion {
    let mut block = String::new();
    let mut warnings = Vec::new();
    let mut seen = HashSet::new();
    let mut slice_count = 0usize;
    let mut total_bytes = 0usize;

    let mut work: Vec<(Target, bool)> = Vec::new();
    for raw in explicit_files {
        let trimmed = raw.trim().trim_start_matches('@');
        if trimmed.is_empty() {
            continue;
        }
        let target = parse_structured_target(trimmed).unwrap_or_else(|| Target::File {
            path: trimmed.to_owned(),
        });
        work.push((target, true));
    }
    for raw in prompt.split_whitespace() {
        let Some(token) = strip_wrappers(raw) else {
            continue;
        };
        if let Some(target) = parse_structured_target(token) {
            work.push((target, false));
        } else if looks_like_workspace_file(root, token) {
            work.push((
                Target::File {
                    path: token.to_owned(),
                },
                false,
            ));
        }
    }

    for (target, explicit) in work {
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
                if !explicit && !looks_like_file_path(root, &path) {
                    continue;
                }
                match outline::extract_symbol_slice(root, grammars, &path, &symbol, remaining) {
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
                if !explicit && !looks_like_file_path(root, &path) {
                    continue;
                }
                match extract_line_slice(root, &path, start, end, remaining) {
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
            Target::File { path } => match extract_file_preview(root, grammars, &path, remaining) {
                Ok(chunk) => {
                    if !block.is_empty() {
                        block.push('\n');
                    }
                    total_bytes += chunk.len();
                    slice_count += 1;
                    block.push_str(&chunk);
                }
                Err(err) => {
                    if explicit || err.contains("escapes") || err.contains("outside") {
                        warnings.push(format!("@{path}: {err}"));
                    }
                }
            },
        }
    }

    MentionExpansion {
        attachments: block,
        warnings,
    }
}

fn append_fallback(block: &mut String, target: &Target) {
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
        Target::File { path } => block.push_str(&format!(
            "[attachment budget reached for @{path}; call read(path=\"{path}\") or index(\"{path}\")]\n"
        )),
    }
}

fn strip_wrappers(token: &str) -> Option<&str> {
    let trimmed = token.trim_start_matches(['(', '[', '{', '"', '\'', '`']);
    let body = trimmed.strip_prefix('@')?;
    let cleaned = body.trim_end_matches([',', '.', ';', ')', ']', '}', '"', '\'', '`']);
    (!cleaned.is_empty()).then_some(cleaned)
}

fn parse_structured_target(token: &str) -> Option<Target> {
    if let Some((path, symbol)) = token.split_once('#')
        && !path.is_empty()
        && !symbol.is_empty()
    {
        return Some(Target::Symbol {
            path: path.to_owned(),
            symbol: symbol.to_owned(),
        });
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
                    path: path.to_owned(),
                    start,
                    end: end.min(start + MAX_LINE_SPAN - 1),
                });
            }
        } else if let Ok(line) = line_spec.parse::<usize>()
            && line >= 1
        {
            return Some(Target::Lines {
                path: path.to_owned(),
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

fn looks_like_workspace_file(root: &Path, rel: &str) -> bool {
    if rel.contains("://") || !looks_like_file_path(root, rel) {
        return false;
    }
    let resolved = paths::resolve(root, rel);
    if !paths::inside(root, &resolved) {
        return false;
    }
    // Accept existing regular files and symlinks (so escaping symlinks are rejected with a warning).
    std::fs::symlink_metadata(&resolved)
        .map(|meta| !meta.is_dir())
        .unwrap_or(false)
}

fn extract_file_preview(
    root: &Path,
    grammars: &Grammars,
    rel_path: &str,
    max_bytes: usize,
) -> Result<String, String> {
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
    let meta = std::fs::metadata(&canon_file).map_err(|e| format!("{shown}: {e}"))?;
    if meta.is_dir() {
        return Err(format!("{shown} is a directory"));
    }
    let text = std::fs::read_to_string(&canon_file).map_err(|e| format!("{shown}: {e}"))?;
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Ok(format!(
            "<attached_context path=\"{shown}\" lines=\"0-0\">\n(empty file)\n</attached_context>\n"
        ));
    }

    if lines.len() <= MAX_INLINE_FILE_LINES {
        let mut body = String::new();
        let mut truncated = false;
        let mut actual_end = 0usize;
        for (idx, line) in lines.iter().enumerate() {
            let lineno = idx + 1;
            let row = format!("{lineno:>5}: {line}\n");
            if body.len() + row.len() > max_bytes && !body.is_empty() {
                truncated = true;
                break;
            }
            body.push_str(&row);
            actual_end = lineno;
        }
        return Ok(format!(
            "<attached_context path=\"{shown}\" lines=\"1-{actual_end}\"{}>\n{body}</attached_context>\n",
            if truncated { " truncated=\"true\"" } else { "" }
        ));
    }

    if let Some(out) = grammars.outline(&canon_file, &text)
        && !out.symbols.is_empty()
    {
        let mut rendered = out.render(&shown);
        let mut truncated = false;
        if rendered.len() > max_bytes {
            let mut end = max_bytes.min(rendered.len());
            while end > 0 && !rendered.is_char_boundary(end) {
                end -= 1;
            }
            rendered.truncate(end);
            if !rendered.ends_with('\n') {
                rendered.push('\n');
            }
            truncated = true;
        }
        return Ok(format!(
            "<attached_context path=\"{shown}\" mode=\"outline\" lines=\"1-{}\"{}>\n{rendered}[File has {} lines; call read(path=\"{shown}\", offset=..., limit=...) for specific sections]\n</attached_context>\n",
            lines.len(),
            if truncated { " truncated=\"true\"" } else { "" },
            lines.len(),
        ));
    }

    let mut body = String::new();
    let mut actual_end = 0usize;
    for (idx, line) in lines.iter().take(MAX_INLINE_FILE_LINES).enumerate() {
        let lineno = idx + 1;
        let row = format!("{lineno:>5}: {line}\n");
        if body.len() + row.len() > max_bytes && !body.is_empty() {
            break;
        }
        body.push_str(&row);
        actual_end = lineno;
    }
    Ok(format!(
        "<attached_context path=\"{shown}\" lines=\"1-{actual_end}\" truncated=\"true\">\n{body}[Showing lines 1-{actual_end} of {}; call read(path=\"{shown}\", offset={}, limit=...) for more]\n</attached_context>\n",
        lines.len(),
        actual_end + 1
    ))
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

        let plain = expand(dir.path(), &grammars, "inspect @link.rs");
        assert!(!plain.warnings.is_empty());
        assert!(!plain.attachments.contains("pub fn secret"));
    }

    #[test]
    fn expands_plain_file_mentions_and_explicit_files_with_outline_fallback() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        std::fs::write(root.join("src/small.rs"), "pub fn tiny() -> u32 { 7 }\n").expect("write");

        let mut big = String::new();
        for i in 0..220 {
            big.push_str(&format!("pub fn f_{i}() -> usize {{ {i} }}\n"));
        }
        std::fs::write(root.join("src/big.rs"), big).expect("write");
        let grammars = Grammars::builtin();

        let plain = expand(
            root,
            &grammars,
            "Review @src/small.rs and ping @someone about @missing/nope.rs",
        );
        assert!(plain.warnings.is_empty(), "warnings: {:?}", plain.warnings);
        assert!(plain.attachments.contains("path=\"src/small.rs\""));
        assert!(plain.attachments.contains("pub fn tiny()"));

        let explicit = expand_with_files(
            root,
            &grammars,
            "inspect",
            &["src/big.rs".to_owned(), "src/does_not_exist.rs".to_owned()],
        );
        assert_eq!(explicit.warnings.len(), 1);
        assert!(explicit.warnings[0].contains("src/does_not_exist.rs"));
        if outline::supported(Path::new("src/big.rs")) {
            assert!(explicit.attachments.contains("mode=\"outline\""));
            assert!(explicit.attachments.contains("f_0"));
        } else {
            assert!(explicit.attachments.contains("truncated=\"true\""));
        }
    }
}
