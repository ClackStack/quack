//! Turning `tools/call`, `prompts/get` and `resources/read` results into text for the
//! model (docs/plan/09-mcp.md#results).
//!
//! Works on the result's JSON so it reads every content shape the spec revisions use.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

static SPILL_SEQ: AtomicU64 = AtomicU64::new(1);

/// Text for the model and whether the server reported an error.
#[cfg(test)]
pub(crate) fn render(result: &Value, max_chars: usize) -> (bool, String) {
    render_spilled(result, max_chars, None, "")
}

/// Like [`render`], writing the full untruncated output under `spill_dir` when it exceeds
/// `max_chars` and citing the saved path in the truncation marker.
pub(crate) fn render_spilled(
    result: &Value,
    max_chars: usize,
    spill_dir: Option<&Path>,
    label: &str,
) -> (bool, String) {
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut parts: Vec<String> = result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(block)
        .collect();
    if parts.is_empty()
        && let Some(s) = result.get("structuredContent").filter(|v| !v.is_null())
    {
        parts.push(s.to_string());
    }
    let text = if parts.is_empty() {
        "(no output)".to_owned()
    } else {
        parts.join("\n")
    };
    (is_error, cut_spilled(&text, max_chars, spill_dir, label))
}

/// One content block as text.
fn block(c: &Value) -> String {
    let field = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or_default();
    match field("type") {
        "text" => field("text").to_owned(),
        "image" | "audio" => format!(
            "[{} ({}, {} bytes) not shown: this model cannot see it]",
            field("type"),
            field("mimeType"),
            field("data").len() * 3 / 4
        ),
        "resource" => resource(&c.get("resource").cloned().unwrap_or_default()),
        "resource_link" => format!("[link: {} {}]", field("uri"), field("name")),
        other => format!("[{other} content not shown]"),
    }
}

/// One resource's contents: text under its URI, binary as a note.
fn resource(r: &Value) -> String {
    let uri = r.get("uri").and_then(Value::as_str).unwrap_or_default();
    match r.get("text").and_then(Value::as_str) {
        Some(text) => format!("[resource {uri}]\n{text}"),
        None => format!("[binary resource {uri} not shown]"),
    }
}

/// A `resources/read` result: each part under its URI.
#[cfg(test)]
pub(crate) fn contents(result: &Value, max_chars: usize) -> String {
    contents_spilled(result, max_chars, None, "")
}

/// Like [`contents`], saving untruncated text under `spill_dir` when it exceeds `max_chars`.
pub(crate) fn contents_spilled(
    result: &Value,
    max_chars: usize,
    spill_dir: Option<&Path>,
    label: &str,
) -> String {
    let parts: Vec<String> = result
        .get("contents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(resource)
        .collect();
    if parts.is_empty() {
        return "(empty resource)".to_owned();
    }
    cut_spilled(&parts.join("\n"), max_chars, spill_dir, label)
}

/// A `prompts/get` result as one message. A prompt that is a single user message reads
/// as typed; other roles are marked, since they all go to the model as the user's turn.
pub(crate) fn prompt(result: &Value) -> String {
    let messages: Vec<(&str, String)> = result
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|m| {
            let role = m.get("role").and_then(Value::as_str).unwrap_or("user");
            (role, block(m.get("content").unwrap_or(&Value::Null)))
        })
        .collect();
    if messages.iter().all(|(role, _)| *role == "user") {
        return messages
            .into_iter()
            .map(|(_, text)| text)
            .collect::<Vec<_>>()
            .join("\n\n");
    }
    messages
        .into_iter()
        .map(|(role, text)| format!("[{role}]\n{text}"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Keep the start and the end of long output.
#[cfg(test)]
pub(crate) fn cut(text: &str, max_chars: usize) -> String {
    cut_spilled(text, max_chars, None, "")
}

/// Keep the start and the end of long output, spilling the complete text to `spill_dir`
/// when provided so the model can inspect it with `read` or `grep`.
pub(crate) fn cut_spilled(
    text: &str,
    max_chars: usize,
    spill_dir: Option<&Path>,
    label: &str,
) -> String {
    let total = text.chars().count();
    if total <= max_chars {
        return text.to_owned();
    }
    let head = max_chars * 3 / 5;
    let tail = max_chars - head;
    let start: String = text.chars().take(head).collect();
    let end: String = text.chars().skip(total - tail).collect();
    let dropped = total - head - tail;
    if let Some(saved) = spill_dir.and_then(|dir| write_spill(dir, label, text)) {
        format!(
            "{start}\n[… {dropped} characters cut; full output saved to {} …]\n{end}",
            saved.display()
        )
    } else {
        format!("{start}\n[… {dropped} characters cut …]\n{end}")
    }
}

fn write_spill(dir: &Path, label: &str, text: &str) -> Option<std::path::PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    let safe: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    let stem = if safe.is_empty() { "mcp" } else { &safe };
    let seq = SPILL_SEQ.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!("{stem}-{}-{seq}.txt", std::process::id()));
    std::fs::write(&path, text).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_each_content_kind() {
        let r = serde_json::json!({
            "content": [
                {"type": "text", "text": "hello"},
                {"type": "image", "data": "AAAA", "mimeType": "image/png"},
                {"type": "resource", "resource": {"uri": "file:///a", "text": "body"}},
                {"type": "resource_link", "uri": "file:///b", "name": "b"}
            ],
            "isError": true
        });
        let (err, text) = render(&r, 10_000);
        assert!(err);
        assert_eq!(
            text,
            "hello\n[image (image/png, 3 bytes) not shown: this model cannot see it]\n[resource file:///a]\nbody\n[link: file:///b b]"
        );
        let (_, text) = render(
            &serde_json::json!({"content": [], "structuredContent": {"n": 1}}),
            100,
        );
        assert_eq!(text, "{\"n\":1}");
    }

    #[test]
    fn prompts_and_resource_contents() {
        let one = serde_json::json!({"messages": [
            {"role": "user", "content": {"type": "text", "text": "Review this."}},
            {"role": "user", "content": {"type": "resource",
                "resource": {"uri": "file:///a.rs", "text": "fn a() {}"}}}
        ]});
        assert_eq!(
            prompt(&one),
            "Review this.\n\n[resource file:///a.rs]\nfn a() {}"
        );
        let mixed = serde_json::json!({"messages": [
            {"role": "user", "content": {"type": "text", "text": "Q"}},
            {"role": "assistant", "content": {"type": "text", "text": "A"}}
        ]});
        assert_eq!(prompt(&mixed), "[user]\nQ\n\n[assistant]\nA");
        let read = serde_json::json!({"contents": [
            {"uri": "mem://notes", "text": "hello"},
            {"uri": "mem://logo", "blob": "AAAA"}
        ]});
        assert_eq!(
            contents(&read, 1000),
            "[resource mem://notes]\nhello\n[binary resource mem://logo not shown]"
        );
    }

    #[test]
    fn long_output_keeps_both_ends() {
        let text: String = (0..100).map(|i| format!("{i:03}")).collect();
        let short = cut(&text, 50);
        assert!(short.starts_with("000001"));
        assert!(short.ends_with("099"));
        assert!(short.contains("250 characters cut"));
    }

    #[test]
    fn long_output_spills_full_text_to_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let text: String = (0..100).map(|i| format!("{i:03}")).collect();
        let short = cut_spilled(&text, 50, Some(dir.path()), "fx/echo");
        assert!(short.contains("full output saved to"), "{short}");
        let files: Vec<_> = std::fs::read_dir(dir.path())
            .expect("read_dir")
            .filter_map(Result::ok)
            .collect();
        assert_eq!(files.len(), 1);
        let saved = std::fs::read_to_string(files[0].path()).expect("read");
        assert_eq!(saved, text);
    }
}
