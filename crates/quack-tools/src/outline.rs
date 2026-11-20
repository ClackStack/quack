//! File outlines: definitions with their line ranges, nested, from tree-sitter and each
//! grammar's `tags.scm` (docs/plan/05-context-management.md#index-tool-maki-style-skeleton).
//!
//! Grammars are compiled in behind the `lang-common` feature. Without it, or for a
//! language it lacks, there is no outline and callers fall back to plain reads.

use std::path::Path;

use crate::grammar::Grammars;
use crate::paths;
use crate::text::{MAX_EDIT_BYTES, TextFile, io_msg, looks_binary};
#[cfg(feature = "lang-common")]
pub(crate) use imp::builtin_tags;

/// One definition: a function, type, module, `impl` block, …
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    /// The grammar's word for it: `function`, `class`, `method`, `module`, `impl`, …
    pub kind: String,
    /// 1-based, inclusive.
    pub start: usize,
    pub end: usize,
    /// The definition's first line, trimmed and shortened.
    pub signature: String,
    /// How many definitions enclose this one.
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outline {
    pub language: &'static str,
    pub lines: usize,
    pub symbols: Vec<Symbol>,
}

impl Outline {
    /// The outline as the model sees it:
    ///
    /// ```text
    /// src/session.rs (412 lines, rust)
    ///   14-31    pub struct Session {
    ///   33-180   impl Session
    ///     35-60    pub fn new(cfg: &Config) -> Result<Self>
    /// ```
    pub fn render(&self, shown: &str) -> String {
        let mut out = format!("{shown} ({} lines, {})\n", self.lines, self.language);
        let width = self.lines.to_string().len() * 2 + 1;
        for s in &self.symbols {
            let indent = "  ".repeat(s.depth + 1);
            let range = format!("{}-{}", s.start, s.end);
            out.push_str(&format!("{indent}{range:<width$}  {}\n", s.signature));
        }
        out
    }

    /// Top-level symbols on one line, for directory listings.
    pub fn summary(&self, max_chars: usize) -> String {
        let mut out = String::new();
        for s in self.symbols.iter().filter(|s| s.depth == 0) {
            let item = format!("{} {}", s.kind_word(), s.name);
            if out.len() + item.len() + 2 > max_chars {
                out.push_str(", …");
                break;
            }
            if !out.is_empty() {
                out.push_str(", ");
            }
            out.push_str(&item);
        }
        out
    }

    /// The symbol a name points at: `name`, or `Parent::name` / `Parent.name` for one
    /// inside another (a C++ `Parent::name` defined outside its class counts too). The
    /// longest match wins, so a definition beats its prototype, then the innermost.
    pub fn find(&self, query: &str) -> Option<&Symbol> {
        let parts: Vec<&str> = query.split([':', '.']).filter(|p| !p.is_empty()).collect();
        let (last, parents) = parts.split_last()?;
        // Symbols are sorted by range start (longest first on a tie), so every encloser
        // of `s` is already on `open`, outermost first. One sweep gives every symbol
        // O(depth) containment instead of an O(n) scan.
        let mut open: Vec<&Symbol> = Vec::new();
        let mut best: Option<&Symbol> = None;
        for s in &self.symbols {
            while open.last().is_some_and(|p| p.end < s.end) {
                open.pop();
            }
            // Its path, minus the last token (`own`): the names of every encloser,
            // outermost first, then the leading tokens of `s.name`.
            let mut s_parts = s.name.split([':', '.']);
            let own = s_parts
                .next_back()
                .expect("str::split yields at least one element");
            let mut above = open
                .iter()
                .flat_map(|p| p.name.split([':', '.']))
                .chain(s_parts);
            if own == *last && parents.iter().all(|p| above.any(|a| a == *p)) {
                let key = (s.end - s.start, s.depth);
                match best {
                    None => best = Some(s),
                    Some(b) if (b.end - b.start, b.depth) < key => best = Some(s),
                    Some(_) => {}
                }
            }
            open.push(s);
        }
        best
    }
}

impl Symbol {
    fn kind_word(&self) -> &str {
        match self.kind.as_str() {
            "function" | "method" => "fn",
            "class" | "struct" | "union" | "enum" => "type",
            "interface" => "trait",
            "module" => "mod",
            other => other,
        }
    }
}

/// The language for a path, by extension.
pub fn language_of(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "go" => "go",
        "sh" | "bash" => "bash",
        "c" => "c",
        "cc" | "cpp" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "h++" | "ipp" | "tpp" | "inl" => {
            "cpp"
        }
        // C or C++: the C++ grammar reads both.
        "h" => "c/c++",
        _ => return None,
    })
}

/// Whether this build can outline `path` (its language, and grammars compiled in).
pub fn supported(path: &Path) -> bool {
    cfg!(feature = "lang-common") && language_of(path).is_some()
}

/// The outline of `text` with the built-in queries, if its language has a grammar in this
/// build. Tools use [`Grammars::outline`], which follows `grammars/` overrides.
pub fn outline(path: &Path, text: &str) -> Option<Outline> {
    Grammars::builtin().outline(path, text)
}

impl Grammars {
    /// The outline of `text`, if its language has a grammar in this build.
    pub fn outline(&self, path: &Path, text: &str) -> Option<Outline> {
        outline_with(self, path, text)
    }
}

/// Where an edit from `before` to `after` breaks the syntax: the first line (1-based) of
/// a parse error in `after`, when `before` parsed cleanly. `None` otherwise, or when the
/// file's language has no grammar in this build.
pub(crate) fn new_syntax_error(path: &Path, before: &str, after: &str) -> Option<usize> {
    let language = language_of(path)?;
    if !imp::error_lines(language, before)?.is_empty() {
        return None;
    }
    imp::error_lines(language, after)?.first().copied()
}

fn outline_key(language: &'static str, text: &str) -> (&'static str, usize, u64, u64) {
    let mut h1: u64 = 0xcbf2_9ce4_8422_2325;
    let mut h2: u64 = 0x1000_0000_01b3_9e37;
    for &b in text.as_bytes() {
        h1 ^= u64::from(b);
        h1 = h1.wrapping_mul(0x0100_0000_01b3);
        h2 = h2.wrapping_mul(31).wrapping_add(u64::from(b));
    }
    (language, text.len(), h1, h2)
}

fn outline_with(g: &Grammars, path: &Path, text: &str) -> Option<Outline> {
    let language = language_of(path)?;
    let key = outline_key(language, text);
    if let Some(cached) = g.cached_outline(key) {
        return Some(cached);
    }
    let mut symbols = imp::symbols(g, language, text)?;
    symbols.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    // Several patterns can match one definition.
    symbols.dedup_by(|b, a| a.start == b.start && a.end == b.end && a.name == b.name);
    // Depth: how many earlier symbols enclose each one.
    let mut open: Vec<usize> = Vec::new();
    for s in &mut symbols {
        // Sorted by start, longest first: the top encloses this one unless it ends sooner.
        while open.last().is_some_and(|&end| end < s.end) {
            open.pop();
        }
        s.depth = open.len();
        open.push(s.end);
    }
    let outline = Outline {
        language,
        lines: text.lines().count(),
        symbols,
    };
    g.store_outline(key, outline.clone());
    Some(outline)
}

/// Extracted snippet for a symbol inside a workspace file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolSlice {
    /// Forward-slash relative path inside the workspace.
    pub path: String,
    pub symbol: Symbol,
    pub text: String,
    pub truncated: bool,
}

/// List outlined definitions in `rel_path` matching `query` (up to `limit`).
/// Verifies canonicalized workspace containment so symlinks cannot escape `root`.
pub fn list_symbols(
    root: &Path,
    grammars: &Grammars,
    rel_path: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<Symbol>, String> {
    if !cfg!(feature = "lang-common") {
        return Ok(Vec::new());
    }
    let (_, _, outline) = load_checked_outline(root, grammars, rel_path)?;
    let q = query.trim().to_ascii_lowercase();
    let mut ancestors: Vec<String> = Vec::new();
    let mut matches: Vec<(u8, Symbol)> = outline
        .symbols
        .into_iter()
        .filter_map(|mut sym| {
            ancestors.truncate(sym.depth);
            let qualified =
                if !ancestors.is_empty() && !sym.name.contains("::") && !sym.name.contains('.') {
                    let mut qname = ancestors.join("::");
                    qname.push_str("::");
                    qname.push_str(&sym.name);
                    qname
                } else {
                    sym.name.clone()
                };
            let leaf = sym.name.clone();
            ancestors.push(leaf);
            sym.name = qualified;
            if q.is_empty() {
                return Some((0, sym));
            }
            let lower = sym.name.to_ascii_lowercase();
            let short = lower.rsplit("::").next().unwrap_or(lower.as_str());
            if short == q || lower == q {
                Some((0, sym))
            } else if short.starts_with(&q) || lower.starts_with(&q) {
                Some((1, sym))
            } else if lower.contains(&q) {
                Some((2, sym))
            } else {
                None
            }
        })
        .collect();
    matches.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.start.cmp(&b.1.start)));
    Ok(matches
        .into_iter()
        .take(limit)
        .map(|(_, sym)| sym)
        .collect())
}

/// Extract a single symbol's exact line range from `rel_path`, bounded at `max_bytes`.
pub fn extract_symbol_slice(
    root: &Path,
    grammars: &Grammars,
    rel_path: &str,
    symbol_query: &str,
    max_bytes: usize,
) -> Result<SymbolSlice, String> {
    if !cfg!(feature = "lang-common") {
        return Err("syntax outlines require the lang-common feature".into());
    }
    let (shown, text, outline) = load_checked_outline(root, grammars, rel_path)?;
    let symbol = outline
        .find(symbol_query)
        .cloned()
        .ok_or_else(|| format!("no definition `{symbol_query}` in {shown}"))?;
    let lines: Vec<&str> = text.lines().collect();
    let start_idx = symbol.start.saturating_sub(1).min(lines.len());
    let end_idx = symbol.end.min(lines.len());
    let mut body = String::new();
    let mut truncated = false;
    for (offset, line) in lines[start_idx..end_idx].iter().enumerate() {
        let lineno = start_idx + offset + 1;
        let formatted = format!("{lineno:>5}: {line}\n");
        if body.len() + formatted.len() > max_bytes && !body.is_empty() {
            truncated = true;
            break;
        }
        body.push_str(&formatted);
    }
    Ok(SymbolSlice {
        path: shown,
        symbol,
        text: body,
        truncated,
    })
}

fn load_checked_outline(
    root: &Path,
    grammars: &Grammars,
    rel_path: &str,
) -> Result<(String, String, Outline), String> {
    let resolved = paths::resolve(root, rel_path);
    let shown = paths::display(root, &resolved);
    if !paths::inside(root, &resolved) {
        return Err(format!("{shown} is outside the workspace"));
    }
    let canon_root = std::fs::canonicalize(root).map_err(|e| io_msg(root, &e))?;
    let canon_path = std::fs::canonicalize(&resolved).map_err(|e| io_msg(&resolved, &e))?;
    if !paths::inside(&canon_root, &canon_path) {
        return Err(format!("{shown} resolves outside the workspace"));
    }
    let meta = std::fs::metadata(&canon_path).map_err(|e| io_msg(&resolved, &e))?;
    if meta.is_dir() {
        return Err(format!("{shown} is a directory"));
    }
    if meta.len() > MAX_EDIT_BYTES {
        return Err(format!(
            "{shown} is {} bytes; too large for symbol extraction",
            meta.len()
        ));
    }
    let bytes = std::fs::read(&canon_path).map_err(|e| io_msg(&resolved, &e))?;
    if looks_binary(&bytes) {
        return Err(format!("{shown} is a binary file"));
    }
    let file = TextFile::from_bytes(&bytes).ok_or_else(|| format!("{shown} is not UTF-8 text"))?;
    let outline = grammars
        .outline(&resolved, &file.text)
        .ok_or_else(|| format!("no syntax outline available for {shown}"))?;
    Ok((shown, file.text, outline))
}

#[cfg(feature = "lang-common")]
mod imp {
    #[cfg(test)]
    use tree_sitter::Query;
    use tree_sitter::{Parser, QueryCursor, StreamingIterator};

    use super::{Grammars, Symbol};

    const SIGNATURE_CHARS: usize = 100;

    /// A definition's first line, trimmed and shortened. When the name is not on it (a
    /// return type or macro on its own line, `typedef struct {`), the lines up to the
    /// name's, or the first and the name's line around `…` when the name is further down.
    fn signature(text: &str, start: usize, name: std::ops::Range<usize>) -> String {
        let first = text[start..].lines().next().unwrap_or_default();
        let line = if name.start < start + first.len() {
            first.trim().to_owned()
        } else {
            let line_end = text[name.end..]
                .find('\n')
                .map_or(text.len(), |i| name.end + i);
            let span = &text[start..line_end];
            if span.lines().count() <= 3 {
                span.split_whitespace().collect::<Vec<_>>().join(" ")
            } else {
                let name_line = span.lines().last().unwrap_or_default();
                format!("{} … {}", first.trim(), name_line.trim())
            }
        };
        if line.chars().count() > SIGNATURE_CHARS {
            let cut: String = line.chars().take(SIGNATURE_CHARS).collect();
            format!("{cut}…")
        } else {
            line
        }
    }

    /// `impl` blocks are references in the Rust query; they hold the methods, so they are
    /// shown as definitions.
    const RUST_EXTRA: &str = "(impl_item type: (_) @name) @definition.impl\n";

    /// Python: decorated functions and classes span their decorators.
    const PYTHON_EXTRA: &str = r#"
(decorated_definition definition: (function_definition name: (identifier) @name)) @definition.function
(decorated_definition definition: (class_definition name: (identifier) @name)) @definition.class
"#;

    /// TypeScript / TSX: type aliases and enums are missing from the upstream tags query.
    const TS_EXTRA: &str = r#"
(type_alias_declaration name: (type_identifier) @name) @definition.type
(enum_declaration name: (identifier) @name) @definition.enum
"#;

    // The C and C++ grammars' own `tags.scm` capture a function's declarator (its name and
    // parameters) rather than the definition, so every function would be one line long;
    // these capture the whole definition, and add namespaces, methods and macros.

    /// Definitions shared by C and C++.
    const C_COMMON: &str = r#"
(struct_specifier name: (type_identifier) @name body: (_)) @definition.struct
(union_specifier name: (type_identifier) @name body: (_)) @definition.union
(enum_specifier name: (type_identifier) @name body: (_)) @definition.enum
(type_definition declarator: (type_identifier) @name) @definition.type
(preproc_function_def name: (identifier) @name) @definition.macro
"#;

    const C_TAGS: &str = r#"
(function_definition declarator: [
  (function_declarator declarator: (identifier) @name)
  (pointer_declarator declarator: (function_declarator declarator: (identifier) @name))
  (pointer_declarator declarator: (pointer_declarator
    declarator: (function_declarator declarator: (identifier) @name)))
]) @definition.function
"#;

    /// C++: also used for `.h`, so prototypes count (a header is mostly prototypes).
    /// `Foo::bar` defined outside its class keeps its qualified name.
    const CPP_TAGS: &str = r#"
(class_specifier name: (type_identifier) @name body: (_)) @definition.class
(namespace_definition name: [(namespace_identifier) (nested_namespace_specifier)] @name) @definition.module
(function_definition declarator: [
  (function_declarator declarator: [(identifier) (field_identifier) (destructor_name) (operator_name)] @name)
  (pointer_declarator declarator: (function_declarator declarator: [(identifier) (field_identifier)] @name))
  (reference_declarator (function_declarator declarator: [(identifier) (field_identifier) (operator_name)] @name))
]) @definition.function
(function_definition declarator: [
  (function_declarator declarator: (qualified_identifier) @name)
  (pointer_declarator declarator: (function_declarator declarator: (qualified_identifier) @name))
  (reference_declarator (function_declarator declarator: (qualified_identifier) @name))
]) @definition.method
(declaration declarator: [
  (function_declarator declarator: (identifier) @name)
  (pointer_declarator declarator: (function_declarator declarator: (identifier) @name))
]) @definition.function
(field_declaration declarator: [
  (function_declarator declarator: [(field_identifier) (destructor_name) (operator_name)] @name)
  (pointer_declarator declarator: (function_declarator declarator: (field_identifier) @name))
  (reference_declarator (function_declarator declarator: [(field_identifier) (operator_name)] @name))
]) @definition.method
(declaration declarator: (function_declarator declarator: (destructor_name) @name)) @definition.method
"#;

    /// Go: functions, methods with their receiver type, interface methods, and types.
    const GO_TAGS: &str = r#"
(function_declaration
  name: (identifier) @name) @definition.function
(method_declaration
  receiver: (parameter_list
    (parameter_declaration
      type: [
        (type_identifier) @receiver
        (pointer_type (type_identifier) @receiver)
        (pointer_type (generic_type type: (type_identifier) @receiver))
        (generic_type type: (type_identifier) @receiver)
      ]))
  name: (field_identifier) @name) @definition.method
(method_declaration
  name: (field_identifier) @name) @definition.method
(method_elem
  name: (field_identifier) @name) @definition.method
(type_spec
  name: (type_identifier) @name) @definition.type
"#;

    /// Shell scripts: functions.
    const BASH_TAGS: &str = "(function_definition name: (word) @name) @definition.function\n";

    /// Each language's built-in definition queries, joined in order.
    pub(crate) fn builtin_tags(name: &str) -> Option<&'static [&'static str]> {
        Some(match name {
            "rust" => &[tree_sitter_rust::TAGS_QUERY, RUST_EXTRA],
            "python" => &[PYTHON_EXTRA, tree_sitter_python::TAGS_QUERY],
            "javascript" => &[tree_sitter_javascript::TAGS_QUERY],
            "typescript" | "tsx" => &[
                tree_sitter_javascript::TAGS_QUERY,
                tree_sitter_typescript::TAGS_QUERY,
                TS_EXTRA,
            ],
            "go" => &[GO_TAGS],
            "c" => &[C_COMMON, C_TAGS],
            "cpp" => &[C_COMMON, CPP_TAGS],
            "bash" => &[BASH_TAGS],
            _ => return None,
        })
    }

    /// Each language's queries that do not compile as a whole (the registry would quietly
    /// fall back to the last one).
    #[cfg(test)]
    pub(super) fn query_errors() -> Vec<String> {
        crate::grammar::LANGUAGES
            .iter()
            .filter_map(|name| {
                let language = crate::grammar::language(name)?;
                Query::new(&language, &builtin_tags(name)?.concat())
                    .err()
                    .map(|e| format!("{name}: {e}"))
            })
            .collect()
    }

    /// The lines (1-based, sorted) where the parse has an error or a missing node.
    pub(super) fn error_lines(language: &str, text: &str) -> Option<Vec<usize>> {
        let name = if language == "c/c++" { "cpp" } else { language };
        let mut parser = Parser::new();
        parser.set_language(&crate::grammar::language(name)?).ok()?;
        let tree = parser.parse(text, None)?;
        let mut out = Vec::new();
        let mut stack = vec![tree.root_node()];
        let mut cursor = tree.walk();
        while let Some(node) = stack.pop() {
            if node.is_error() || node.is_missing() {
                out.push(node.start_position().row + 1);
            } else if node.has_error() {
                stack.extend(node.children(&mut cursor));
            }
        }
        out.sort_unstable();
        Some(out)
    }

    pub(super) fn symbols(g: &Grammars, language: &str, text: &str) -> Option<Vec<Symbol>> {
        let (lang, query) = g.tags(language)?;
        let mut parser = Parser::new();
        parser.set_language(lang).ok()?;
        let tree = parser.parse(text, None)?;
        let names = query.capture_names();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), text.as_bytes());
        let mut out = Vec::new();
        // One symbol per definition and pattern: a C++ parse confused by macros can give a
        // node several names (an initializer list read as declarators); the first is kept.
        // Different patterns may still name one node (Rust's `impl Trait for Type`).
        let mut seen = std::collections::HashSet::new();
        let mut qualified = std::collections::HashSet::new();
        while let Some(m) = matches.next() {
            let mut def = None;
            let mut name = None;
            let mut receiver = None;
            for c in m.captures() {
                let capture = names[c.index as usize];
                if capture == "name" {
                    name = Some(c.node);
                } else if capture == "receiver" {
                    receiver = Some(c.node);
                } else if let Some(kind) = capture
                    .strip_prefix("definition.")
                    .or_else(|| (capture == "reference.implementation").then_some("impl"))
                {
                    def = Some((kind, c.node));
                }
            }
            let (Some((kind, node)), Some(name_node)) = (def, name) else {
                continue;
            };
            if qualified.contains(&node.id()) {
                continue;
            }
            if node
                .parent()
                .is_some_and(|p| p.kind() == "decorated_definition")
            {
                continue;
            }
            let Ok(name) = name_node.utf8_text(text.as_bytes()) else {
                continue;
            };
            let (name, is_qualified) =
                if let Some(r) = receiver.and_then(|n| n.utf8_text(text.as_bytes()).ok()) {
                    (format!("{r}::{name}"), true)
                } else {
                    (name.to_owned(), false)
                };
            if !seen.insert((node.id(), m.pattern_index)) {
                continue;
            }
            if is_qualified {
                qualified.insert(node.id());
            }
            let (start, end) = (node.start_position(), node.end_position());
            out.push(Symbol {
                name,
                kind: kind.to_owned(),
                start: start.row + 1,
                // A node that takes its line's newline (`#define`) ends on the line before.
                end: if end.column == 0 && end.row > start.row {
                    end.row
                } else {
                    end.row + 1
                },
                signature: signature(text, node.start_byte(), name_node.byte_range()),
                depth: 0,
            });
        }
        Some(out)
    }
}

#[cfg(not(feature = "lang-common"))]
mod imp {
    use super::{Grammars, Symbol};

    pub(super) fn symbols(_g: &Grammars, _language: &str, _text: &str) -> Option<Vec<Symbol>> {
        None
    }

    pub(super) fn error_lines(_language: &str, _text: &str) -> Option<Vec<usize>> {
        None
    }
}

#[cfg(all(test, feature = "lang-common"))]
mod tests {
    use super::*;

    const RUST: &str = "use std::io;\n\npub struct Session {\n    id: u32,\n}\n\nimpl Session {\n    pub fn new() -> Self {\n        Self { id: 0 }\n    }\n\n    fn run(&self) {}\n}\n\nmod tests {\n    fn t() {}\n}\n";

    #[test]
    fn rust_outline_nests_methods_in_impls() {
        let o = outline(Path::new("src/s.rs"), RUST).expect("outline");
        let rows: Vec<(usize, usize, &str, usize)> = o
            .symbols
            .iter()
            .map(|s| (s.start, s.end, s.name.as_str(), s.depth))
            .collect();
        assert_eq!(
            rows,
            [
                (3, 5, "Session", 0),
                (7, 13, "Session", 0),
                (8, 10, "new", 1),
                (12, 12, "run", 1),
                (15, 17, "tests", 0),
                (16, 16, "t", 1),
            ]
        );
        let text = o.render("src/s.rs");
        assert!(text.starts_with("src/s.rs (17 lines, rust)\n"), "{text}");
        assert!(text.contains("    8-10   pub fn new() -> Self {"), "{text}");
        assert_eq!(o.find("Session::run").map(|s| s.start), Some(12));
        assert_eq!(o.find("tests.t").map(|s| s.start), Some(16));
        assert_eq!(o.find("nope"), None);
        assert_eq!(o.summary(200), "type Session, impl Session, mod tests");
    }

    #[test]
    fn other_languages_parse() {
        let py = "@app.get(\"/items\")\ndef get_items():\n    pass\n\nclass A:\n    def f(self):\n        pass\n\ndef g():\n    return 1\n";
        let o = outline(Path::new("a.py"), py).expect("python");
        assert_eq!(o.find("get_items").map(|s| s.start), Some(1));
        assert_eq!(o.find("A.f").map(|s| (s.start, s.depth)), Some((6, 1)));
        let go = "package main\n\nfunc main() {\n}\n\ntype T struct{}\n\nfunc (t T) M() {}\n";
        let o = outline(Path::new("a.go"), go).expect("go");
        assert!(
            o.find("main").is_some() && o.find("T.M").is_some() && o.find("M").is_some(),
            "{o:?}"
        );
        let ts = "export class C {\n  m(): void {}\n}\ninterface I { x: number }\ntype Config = { a: string }\nenum Mode { Fast, Safe }\nfunction f() {}\n";
        let o = outline(Path::new("a.ts"), ts).expect("ts");
        assert!(
            o.find("C.m").is_some()
                && o.find("Config").is_some()
                && o.find("Mode").is_some()
                && o.find("f").is_some(),
            "{o:?}"
        );
        let js = "function a() {}\nconst b = () => 1;\nclass K { z() {} }\n";
        let o = outline(Path::new("a.js"), js).expect("js");
        assert!(o.find("a").is_some() && o.find("K.z").is_some(), "{o:?}");
        assert!(outline(Path::new("a.txt"), "x").is_none());
    }

    #[test]
    fn every_query_compiles() {
        assert_eq!(imp::query_errors(), Vec::<String>::new());
    }

    #[test]
    fn go_outline_resolves_methods_by_receiver() {
        let go = "package main\n\ntype Scope struct {\n    name string\n}\n\nfunc (s *Scope) Define(name string) {\n    s.name = name\n}\n\nfunc (s Scope) Lookup(name string) string {\n    return s.name\n}\n\ntype Definer interface {\n    Define(name string)\n}\n";
        let o = outline(Path::new("scope.go"), go).expect("go");
        assert_eq!(o.find("Scope.Define").map(|s| s.start), Some(7));
        assert_eq!(o.find("Scope::Define").map(|s| s.start), Some(7));
        assert_eq!(o.find("Define").map(|s| s.start), Some(7));
        assert_eq!(o.find("Scope.Lookup").map(|s| s.start), Some(11));
        assert_eq!(o.find("Definer.Define").map(|s| s.start), Some(16));
        assert_eq!(o.find("Missing.Define"), None);
    }

    fn rows(o: &Outline) -> Vec<(usize, usize, &str, &str, usize)> {
        o.symbols
            .iter()
            .map(|s| (s.start, s.end, s.kind.as_str(), s.name.as_str(), s.depth))
            .collect()
    }

    const C: &str = "#include <stdio.h>\n\
\n\
#define MAX(a, b) ((a) > (b) ? (a) : (b))\n\
#define LIMIT 10\n\
\n\
struct point {\n\
    int x, y;\n\
};\n\
\n\
typedef enum { RED, GREEN } color;\n\
\n\
static int add(int a, int b);\n\
\n\
static int add(int a, int b) {\n\
    return a + b;\n\
}\n\
\n\
char *name(struct point *p) {\n\
    (void)p;\n\
    return \"p\";\n\
}\n";

    #[test]
    fn c_outline_spans_whole_definitions() {
        let o = outline(Path::new("src/geo.c"), C).expect("c");
        assert_eq!(o.language, "c");
        assert_eq!(
            rows(&o),
            [
                (3, 3, "macro", "MAX", 0),
                (6, 8, "struct", "point", 0),
                (10, 10, "type", "color", 0),
                (14, 16, "function", "add", 0),
                (18, 21, "function", "name", 0),
            ]
        );
        assert_eq!(o.find("name").map(|s| s.start), Some(18));
        assert_eq!(
            o.summary(200),
            "macro MAX, type point, type color, fn add, fn name"
        );
    }

    const CPP: &str = "namespace geo {\n\
\n\
class Shape {\n\
public:\n\
    virtual ~Shape();\n\
    double area() const;\n\
    int sides() const { return n_; }\n\
private:\n\
    int n_;\n\
};\n\
\n\
double Shape::area() const {\n\
    return 0.0;\n\
}\n\
\n\
}  // namespace geo\n\
\n\
template <typename T>\n\
T twice(T v) {\n\
    return v * 2;\n\
}\n";

    #[test]
    fn cpp_outline_has_namespaces_classes_and_methods() {
        let o = outline(Path::new("src/shape.cpp"), CPP).expect("cpp");
        assert_eq!(o.language, "cpp");
        assert_eq!(
            rows(&o),
            [
                (1, 16, "module", "geo", 0),
                (3, 10, "class", "Shape", 1),
                (5, 5, "method", "~Shape", 2),
                (6, 6, "method", "area", 2),
                (7, 7, "function", "sides", 2),
                (12, 14, "method", "Shape::area", 1),
                (19, 21, "function", "twice", 0),
            ]
        );
        // The out-of-line definition, by its qualified name or inside its namespace;
        // the definition over the declaration in the class.
        assert_eq!(o.find("Shape::area").map(|s| s.start), Some(12));
        assert_eq!(o.find("geo::Shape::area").map(|s| s.start), Some(12));
        assert_eq!(o.find("Shape.sides").map(|s| s.start), Some(7));
        assert_eq!(o.find("area").map(|s| s.start), Some(12));
        assert_eq!(o.find("Other::area"), None);
    }

    #[test]
    fn signatures_show_the_name() {
        let c = "typedef struct {\n    int x;\n    int y;\n    int z;\n} vec3;\n\nstatic int\nadd(int a, int b)\n{\n    return a + b;\n}\n";
        let o = outline(Path::new("v.c"), c).expect("c");
        let sigs: Vec<&str> = o.symbols.iter().map(|s| s.signature.as_str()).collect();
        assert_eq!(
            sigs,
            ["typedef struct { … } vec3;", "static int add(int a, int b)"]
        );
    }

    #[test]
    fn headers_read_as_c_or_cpp() {
        let h = "#ifndef GEO_H\n#define GEO_H\n\nstruct point { int x; };\nint add(int a, int b);\nchar *name(void);\n\n#endif\n";
        let o = outline(Path::new("include/geo.h"), h).expect("header");
        assert_eq!(o.language, "c/c++");
        let names: Vec<&str> = o.symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["point", "add", "name"]);
        assert_eq!(language_of(Path::new("a.HPP")), Some("cpp"));
        assert_eq!(language_of(Path::new("a.cc")), Some("cpp"));
    }

    #[test]
    fn list_and_extract_symbol_slice_checks_containment() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        std::fs::write(root.join("src/s.rs"), RUST).expect("write");
        let grammars = Grammars::builtin();

        let listed = list_symbols(root, &grammars, "src/s.rs", "new", 10).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Session::new");

        let slice =
            extract_symbol_slice(root, &grammars, "src/s.rs", "Session::new", 4096).expect("slice");
        assert_eq!(slice.path, "src/s.rs");
        assert!(!slice.truncated);
        assert!(slice.text.contains("pub fn new() -> Self"));

        assert!(list_symbols(root, &grammars, "../outside.rs", "", 10).is_err());
    }
}
