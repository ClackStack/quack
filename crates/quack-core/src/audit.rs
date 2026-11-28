//! Local JSONL audit log for enterprise tool-execution tracking
//! (docs/plan/11-security.md#audit-log-enterprise).
//!
//! Appends one JSON object per tool decision/execution when `[audit] path` is set.
//! File contents and model text are excluded unless `audit.include_content = true`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use quack_config::Loaded;
use quack_tools::Subject;
use serde_json::{Value, json};

/// Whether a tool call was permitted, denied, or interrupted before running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuditDecision {
    Allow,
    Deny,
    Interrupted,
}

impl AuditDecision {
    fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Interrupted => "interrupted",
        }
    }
}

/// One tool decision/execution record written to the audit JSONL file.
pub(crate) struct AuditEntry<'a> {
    pub session_id: &'a str,
    pub agent: Option<&'a str>,
    pub call_id: &'a str,
    pub tool: &'a str,
    pub subject: &'a str,
    pub decision: AuditDecision,
    pub decided_by: &'a str,
    /// `Some(true|false)` after execution, `None` if denied or interrupted before running.
    pub ok: Option<bool>,
    /// Populated only when `audit.include_content` is enabled.
    pub args: Option<&'a Value>,
    pub summary: Option<&'a str>,
}

/// Append-only JSONL audit writer configured from `[audit]`.
#[derive(Debug)]
pub(crate) struct AuditLog {
    path: PathBuf,
    include_content: bool,
    user: String,
    workspace: PathBuf,
    file: Mutex<Option<File>>,
}

impl AuditLog {
    /// Build an audit logger from the loaded settings, or `None` when `[audit] path` is empty.
    pub(crate) fn from_config(config: &Loaded) -> Option<Arc<Self>> {
        let path = resolve_audit_path(config)?;
        Some(Arc::new(Self {
            path,
            include_content: config.settings.audit.include_content,
            user: current_user(),
            workspace: config.dirs.workspace_root.clone(),
            file: Mutex::new(None),
        }))
    }

    pub(crate) fn include_content(&self) -> bool {
        self.include_content
    }

    /// Append one JSONL record. Failures warn in the application log and never abort a turn.
    pub(crate) fn record(&self, entry: AuditEntry<'_>) {
        let ts = quack_store::time::rfc3339(quack_store::time::unix_now());
        let exit_status = match (entry.decision, entry.ok) {
            (AuditDecision::Deny, _) => "denied",
            (AuditDecision::Interrupted, _) => "interrupted",
            (AuditDecision::Allow, Some(true)) => "ok",
            (AuditDecision::Allow, Some(false)) => "error",
            (AuditDecision::Allow, None) => "error",
        };
        let mut obj = json!({
            "v": 1,
            "ts": ts,
            "user": self.user,
            "session_id": entry.session_id,
            "workspace": self.workspace.display().to_string(),
            "call_id": entry.call_id,
            "tool": entry.tool,
            "subject": entry.subject,
            "decision": entry.decision.as_str(),
            "decided_by": entry.decided_by,
            "ok": entry.ok,
            "exit_status": exit_status,
        });
        if let Some(agent) = entry.agent {
            obj["agent"] = Value::String(agent.to_owned());
        }
        if self.include_content {
            if let Some(args) = entry.args {
                obj["args"] = args.clone();
            }
            if let Some(summary) = entry.summary {
                obj["summary"] = Value::String(summary.to_owned());
            }
        }
        let Ok(mut line) = serde_json::to_string(&obj) else {
            return;
        };
        line.push('\n');
        if let Err(e) = self.append_line(&line) {
            tracing::warn!(
                path = %self.path.display(),
                error = %e,
                "failed to write audit log record"
            );
        }
    }

    fn append_line(&self, line: &str) -> std::io::Result<()> {
        let mut guard = self
            .file
            .lock()
            .map_err(|_| std::io::Error::other("audit log mutex poisoned"))?;
        if guard.is_none() {
            if let Some(parent) = self.path.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent)?;
            }
            let file = open_append(&self.path)?;
            *guard = Some(file);
        }
        if let Some(file) = guard.as_mut() {
            file.write_all(line.as_bytes())?;
            file.flush()?;
        }
        Ok(())
    }
}

/// Resolve `[audit] path` (expanding `~/` and resolving relative paths against the workspace).
pub(crate) fn resolve_audit_path(config: &Loaded) -> Option<PathBuf> {
    let raw = config.settings.audit.path.trim();
    if raw.is_empty() {
        return None;
    }
    let expanded = quack_config::expand_home(raw, &|k| std::env::var_os(k));
    Some(if expanded.is_absolute() {
        expanded
    } else {
        config.dirs.workspace_root.join(expanded)
    })
}

/// Format a permission [`Subject`] into a stable audit label.
pub(crate) fn subject_label(subject: &Subject, root: &Path) -> String {
    let rel = |p: &Path| quack_tools::paths::display(root, p);
    match subject {
        Subject::Read(p) => format!("read:{}", rel(p)),
        Subject::Edit(p) => format!("edit:{}", rel(p)),
        Subject::Edits(ps) => {
            let names: Vec<String> = ps.iter().map(|p| rel(p)).collect();
            format!("edit:{}", names.join(","))
        }
        Subject::Shell(cmd) => format!("shell:{cmd}"),
        Subject::ShellOutsideSandbox(cmd) => format!("shell-outside-sandbox:{cmd}"),
        Subject::Memory { write: true } => "memory:write".into(),
        Subject::Memory { write: false } => "memory:read".into(),
        Subject::Skill(name) => format!("skill:{name}"),
        Subject::ToolSearch => "mcp:tool_search".into(),
        Subject::Plan => "plan".into(),
        Subject::Agent { read_only: true } => "agent:read-only".into(),
        Subject::Agent { read_only: false } => "agent:mutating".into(),
        Subject::Mcp { server, tool, .. } => format!("mcp:{server}/{tool}"),
        Subject::WebFetch(url) => format!("web_fetch:{url}"),
    }
}

fn current_user() -> String {
    for var in ["USER", "USERNAME", "LOGNAME"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim();
            if !v.is_empty() {
                return v.to_owned();
            }
        }
    }
    "unknown".into()
}

fn open_append(path: &Path) -> std::io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        OpenOptions::new().create(true).append(true).open(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_jsonl_and_omits_content_by_default() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let log_path = tmp.path().join("logs").join("audit.jsonl");
        let logger = AuditLog {
            path: log_path.clone(),
            include_content: false,
            user: "alice".into(),
            workspace: tmp.path().to_path_buf(),
            file: Mutex::new(None),
        };
        let args = json!({ "path": "secret.txt", "content": "super-secret" });
        logger.record(AuditEntry {
            session_id: "sess-1",
            agent: None,
            call_id: "call_1",
            tool: "write",
            subject: "edit:secret.txt",
            decision: AuditDecision::Allow,
            decided_by: "user",
            ok: Some(true),
            args: Some(&args),
            summary: Some("wrote secret.txt"),
        });
        logger.record(AuditEntry {
            session_id: "sess-1",
            agent: Some("explore-1"),
            call_id: "explore-1/call_2",
            tool: "shell",
            subject: "shell:rm -rf /tmp/x",
            decision: AuditDecision::Deny,
            decided_by: "hook:block-rm",
            ok: None,
            args: Some(&args),
            summary: Some("blocked"),
        });

        let text = std::fs::read_to_string(&log_path).expect("read audit log");
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).expect("valid jsonl"))
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["user"], "alice");
        assert_eq!(lines[0]["decision"], "allow");
        assert_eq!(lines[0]["decided_by"], "user");
        assert_eq!(lines[0]["exit_status"], "ok");
        assert!(lines[0].get("args").is_none());
        assert!(lines[0].get("summary").is_none());
        assert!(!text.contains("super-secret"));

        assert_eq!(lines[1]["agent"], "explore-1");
        assert_eq!(lines[1]["decision"], "deny");
        assert_eq!(lines[1]["decided_by"], "hook:block-rm");
        assert_eq!(lines[1]["exit_status"], "denied");
    }

    #[test]
    fn includes_content_when_opted_in() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let log_path = tmp.path().join("audit.jsonl");
        let logger = AuditLog {
            path: log_path.clone(),
            include_content: true,
            user: "bob".into(),
            workspace: tmp.path().to_path_buf(),
            file: Mutex::new(None),
        };
        let args = json!({ "path": "a.rs" });
        logger.record(AuditEntry {
            session_id: "sess-2",
            agent: None,
            call_id: "c1",
            tool: "read",
            subject: "read:a.rs",
            decision: AuditDecision::Allow,
            decided_by: "rule",
            ok: Some(false),
            args: Some(&args),
            summary: Some("not found"),
        });
        let text = std::fs::read_to_string(&log_path).expect("read");
        let v: Value = serde_json::from_str(text.trim()).expect("json");
        assert_eq!(v["args"]["path"], "a.rs");
        assert_eq!(v["summary"], "not found");
        assert_eq!(v["exit_status"], "error");
    }
}
