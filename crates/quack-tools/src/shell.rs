//! `shell`: run one command in the configured shell.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use quack_protocol::ApprovalRequest;
use quack_shell::{RunOptions, run};
use serde_json::{Value, json};

use crate::paths;
use crate::registry::{
    SandboxState, Subject, Tool, ToolCtx, ToolKind, ToolOutput, arg_str, opt_str, opt_u64,
};
use crate::text::trim_middle;

/// Model-facing output budget, in bytes: head and tail are kept. Full output beyond this
/// is saved to a temp log file whose path is included so the model can inspect the middle.
const HEAD: usize = 4 * 1024;
const TAIL: usize = 8 * 1024;
const CMD: &[&str] = &["command", "cmd", "script"];
static SPILL_SEQ: AtomicU64 = AtomicU64::new(0);

/// Save full truncated shell output under the system temp dir (never inside the workspace,
/// so git status and diffs stay clean). Returns the path if written.
fn spill_output(raw: &str) -> Option<std::path::PathBuf> {
    let dir = std::env::temp_dir().join("quack-shell");
    std::fs::create_dir_all(&dir).ok()?;
    let seq = SPILL_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    let path = dir.join(format!("cmd-{}-{seq}.log", std::process::id()));
    std::fs::write(&path, raw).ok()?;
    Some(path)
}

#[derive(Debug)]
pub struct Shell;

/// The policy to run this call under; `None` runs it outside the sandbox.
fn policy(args: &Value, ctx: &ToolCtx) -> Option<std::sync::Arc<quack_sandbox::Policy>> {
    match &ctx.sandbox {
        SandboxState::On(p) if !escapes(args) => Some(p.clone()),
        _ => None,
    }
}

/// The model asked to run outside the sandbox (`"sandbox": false`).
fn escapes(args: &Value) -> bool {
    match args.get("sandbox") {
        Some(Value::Bool(b)) => !b,
        Some(Value::String(s)) => s.eq_ignore_ascii_case("false"),
        _ => false,
    }
}

/// Whether this call runs outside a sandbox that is configured.
fn outside(args: &Value, ctx: &ToolCtx) -> bool {
    match &ctx.sandbox {
        SandboxState::Off => false,
        SandboxState::On(_) => escapes(args),
        SandboxState::Unavailable(_) => true,
    }
}

/// The model asked to run in the background (`"background": true`).
fn is_background(args: &Value) -> bool {
    match args.get("background") {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

/// Optional background job control action (`"output"`, `"stop"`, `"list"`).
fn job_action(args: &Value) -> Option<&str> {
    opt_str(args, &["job_action", "action"])
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn format_bg_status(st: &quack_shell::BgStatus) -> String {
    let state = if st.running {
        format!("running ({}s)", st.elapsed_secs)
    } else {
        match st.exit_code {
            Some(c) => format!("exited (code {c}, {}s)", st.elapsed_secs),
            None => format!("stopped ({}s)", st.elapsed_secs),
        }
    };
    let mut out = format!("job {} · {state}\ncommand: {}", st.id, st.command);
    if st.dropped_bytes > 0 {
        out.push_str(&format!(
            "\n[{} earlier bytes evicted from ring buffer]",
            st.dropped_bytes
        ));
    }
    if st.output.trim().is_empty() {
        out.push_str("\n(no output)");
    } else {
        let distilled = distill_output(&st.output);
        let trimmed = trim_middle(&distilled, HEAD, TAIL);
        out.push('\n');
        out.push_str(&trimmed);
    }
    out
}

impl Tool for Shell {
    fn name(&self) -> &str {
        "shell"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Mutating
    }

    fn description(&self, ctx: &ToolCtx) -> String {
        let shell = ctx.shell.label();
        let os = std::env::consts::OS;
        let sandbox = match &ctx.sandbox {
            SandboxState::On(p) => format!(
                " Commands run in a sandbox: they can read anything but write only in the \
                 workspace and temp folders{}. If the sandbox blocks a command you need, run \
                 it again with `sandbox: false`; the user is asked to approve that.",
                if p.network { "" } else { ", with no network" }
            ),
            _ => String::new(),
        };
        format!(
            "Run a command with {shell} on {os} in the workspace root and return its exit code and \
             combined output. Write commands for {shell}. There is no stdin, so interactive \
             commands fail; pass non-interactive flags. Default timeout {}s, maximum {}s. \
             Pass `background: true` to start a long-running server/watcher (returns a `job_id`), \
             then inspect or stop it with `job_action: \"output\" | \"stop\" | \"list\"` and `job_id`.{sandbox}",
            ctx.default_timeout.as_secs(),
            ctx.max_timeout.as_secs()
        )
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The command to run" },
                "timeout_secs": { "type": "integer", "description": "Timeout in seconds" },
                "sandbox": { "type": "boolean", "description": "Set false only to rerun a command the sandbox blocked" },
                "background": { "type": "boolean", "description": "Start the command in the background and return a job_id" },
                "job_action": { "type": "string", "enum": ["output", "stop", "list"], "description": "Inspect, stop, or list background jobs started in this session" },
                "job_id": { "type": "string", "description": "Background job id returned by `background: true` (for output/stop)" },
                "tail_bytes": { "type": "integer", "description": "Max tail bytes to return for `job_action: \"output\"`" }
            },
            "required": []
        })
    }

    fn subject(&self, args: &Value, ctx: &ToolCtx) -> Result<Subject, String> {
        if let Some(action) = job_action(args) {
            return match action {
                "list" | "output" | "stop" => Ok(Subject::Read(ctx.root.clone())),
                other => Err(format!(
                    "unknown job_action `{other}`; use `output`, `stop`, or `list`"
                )),
            };
        }
        let command = arg_str(args, CMD)?.trim().to_owned();
        Ok(if outside(args, ctx) {
            Subject::ShellOutsideSandbox(command)
        } else {
            Subject::Shell(command)
        })
    }

    fn summary(&self, args: &Value, _ctx: &ToolCtx) -> String {
        if let Some(action) = job_action(args) {
            return match opt_str(args, &["job_id", "id"]) {
                Some(id) => format!("shell {action} {id}"),
                None => format!("shell {action}"),
            };
        }
        let prefix = if is_background(args) {
            "shell (bg) "
        } else {
            "shell "
        };
        format!("{prefix}{}", arg_str(args, CMD).unwrap_or_default().trim())
    }

    fn preview(&self, args: &Value, ctx: &ToolCtx) -> Result<ApprovalRequest, String> {
        if let Some(action) = job_action(args) {
            return Ok(ApprovalRequest::Tool {
                name: "shell".into(),
                arguments: match opt_str(args, &["job_id", "id"]) {
                    Some(id) => format!("{action} {id}"),
                    None => action.to_owned(),
                },
            });
        }
        Ok(ApprovalRequest::Shell {
            command: arg_str(args, CMD)?.trim().to_owned(),
            cwd: paths::display(&ctx.root, &ctx.root),
            outside_sandbox: outside(args, ctx),
        })
    }

    fn run<'a>(&'a self, args: Value, ctx: &'a ToolCtx) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            if let Some(action) = job_action(&args) {
                return match action {
                    "list" => {
                        let jobs = ctx.bg.list();
                        if jobs.is_empty() {
                            ToolOutput::ok(
                                "no background jobs in this session",
                                "0 background jobs",
                            )
                        } else {
                            let lines: Vec<String> = jobs
                                .iter()
                                .map(|j| {
                                    let st = if j.running {
                                        format!("running ({}s)", j.elapsed_secs)
                                    } else {
                                        match j.exit_code {
                                            Some(c) => format!("exited({c})"),
                                            None => "stopped".to_owned(),
                                        }
                                    };
                                    format!("{} · {st} · {}", j.id, j.command)
                                })
                                .collect();
                            ToolOutput::ok(
                                lines.join("\n"),
                                format!("{} background job(s)", jobs.len()),
                            )
                        }
                    }
                    "output" => {
                        let Some(id) = opt_str(&args, &["job_id", "id"]) else {
                            return ToolOutput::error(
                                "`job_id` is required for `job_action: \"output\"`",
                            );
                        };
                        let tail = opt_u64(&args, &["tail_bytes"])
                            .and_then(|n| usize::try_from(n).ok())
                            .map(|n| n.clamp(256, 64 * 1024));
                        match ctx.bg.output(id, tail) {
                            Ok(st) => {
                                let text = format_bg_status(&st);
                                ToolOutput {
                                    ok: true,
                                    summary: format!(
                                        "{} · {}",
                                        st.id,
                                        if st.running { "running" } else { "exited" }
                                    ),
                                    detail: Some(st.output),
                                    model: text,
                                    images: Vec::new(),
                                }
                            }
                            Err(e) => ToolOutput::error(e),
                        }
                    }
                    "stop" => {
                        let Some(id) = opt_str(&args, &["job_id", "id"]) else {
                            return ToolOutput::error(
                                "`job_id` is required for `job_action: \"stop\"`",
                            );
                        };
                        match ctx.bg.stop(id) {
                            Ok(st) => {
                                let text = format_bg_status(&st);
                                ToolOutput {
                                    ok: true,
                                    summary: format!("stopped {}", st.id),
                                    detail: Some(st.output),
                                    model: text,
                                    images: Vec::new(),
                                }
                            }
                            Err(e) => ToolOutput::error(e),
                        }
                    }
                    other => ToolOutput::error(format!(
                        "unknown job_action `{other}`; use `output`, `stop`, or `list`"
                    )),
                };
            }

            let command = match arg_str(&args, CMD) {
                Ok(c) => c.trim().to_owned(),
                Err(e) => return ToolOutput::error(e),
            };
            if is_background(&args) {
                let id =
                    match ctx
                        .bg
                        .start(&ctx.shell, &command, ctx.root.clone(), policy(&args, ctx))
                    {
                        Ok(id) => id,
                        Err(e) => return ToolOutput::error(e),
                    };
                // Give fast-failing commands a brief window to emit an error or startup banner.
                tokio::time::sleep(Duration::from_millis(150)).await;
                return match ctx.bg.output(&id, None) {
                    Ok(st) => {
                        let ok = st.running || st.exit_code == Some(0);
                        let summary = if st.running {
                            format!("started {id} in background")
                        } else {
                            format!("{id} exited quickly ({:?})", st.exit_code)
                        };
                        ToolOutput {
                            ok,
                            model: format_bg_status(&st),
                            images: Vec::new(),
                            summary,
                            detail: Some(st.output),
                        }
                    }
                    Err(e) => ToolOutput::error(e),
                };
            }
            let timeout = opt_u64(&args, &["timeout_secs", "timeout"])
                .map(Duration::from_secs)
                .unwrap_or(ctx.default_timeout)
                .min(ctx.max_timeout);
            let opts = RunOptions {
                cwd: ctx.root.clone(),
                timeout,
                cancel: ctx.cancel.clone(),
                on_output: ctx.on_output.clone(),
                sandbox: policy(&args, ctx),
            };
            let sandboxed = opts.sandbox.is_some();
            let r = match run(&ctx.shell, &command, opts).await {
                Ok(r) => r,
                Err(e) => {
                    return ToolOutput::error(format!(
                        "could not start {}: {e}",
                        ctx.shell.program().display()
                    ));
                }
            };
            let status = if r.timed_out {
                format!(
                    "timed out after {}s (process tree killed)",
                    timeout.as_secs()
                )
            } else if r.cancelled {
                "interrupted by the user".to_owned()
            } else {
                match r.exit_code {
                    Some(c) => format!("exit code {c}"),
                    None => "killed by a signal".to_owned(),
                }
            };
            let mut body = r.output.clone();
            if r.dropped_bytes > 0 {
                body.push_str(&format!(
                    "\n… [{} more bytes not captured]",
                    r.dropped_bytes
                ));
            }
            let distilled = distill_output(&body);
            let truncated = distilled.len() > HEAD + TAIL;
            let trimmed = trim_middle(&distilled, HEAD, TAIL);
            let mut model = if trimmed.trim().is_empty() {
                format!("{status}\n(no output)")
            } else {
                format!("{status}\n{trimmed}")
            };
            let lines = r.output.lines().count();
            if truncated && let Some(saved) = spill_output(&body) {
                model.push_str(&format!(
                    "\n[full output ({} bytes, {lines} lines) saved to {}]",
                    body.len(),
                    saved.display()
                ));
            }
            let ok = r.exit_code == Some(0);
            if sandboxed && !ok && quack_sandbox::looks_blocked(&r.output) {
                model.push_str(
                    "\n\nThis command ran in the sandbox (writes only in the workspace and temp \
                     folders; network as configured). If the sandbox caused the failure and the \
                     command is needed, run it again with `sandbox: false` to ask the user.",
                );
            }
            ToolOutput {
                ok,
                model,
                images: Vec::new(),
                summary: format!("{status} · {lines} lines"),
                detail: Some(body),
            }
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
enum LineKind {
    PassTest,
    Compiling,
    Other,
}

fn classify_line(line: &str) -> LineKind {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return LineKind::Other;
    }

    if trimmed.contains("FAILED")
        || trimmed.contains("FAIL")
        || trimmed.contains("Error")
        || trimmed.contains("error")
        || trimmed.contains("Traceback")
        || trimmed.contains("AssertionError")
        || trimmed.contains("failures:")
    {
        return LineKind::Other;
    }

    if (trimmed.starts_with("test ")
        && (trimmed.ends_with("... ok") || trimmed.ends_with("... ignored")))
        || trimmed.ends_with(" ... ok")
        || trimmed.ends_with(" PASSED")
        || trimmed.starts_with("PASSED ")
        || trimmed.starts_with("--- PASS:")
        || trimmed.starts_with("✓ ")
        || trimmed.starts_with("✔ ")
        || (trimmed.starts_with("PASS ") && !trimmed.contains("FAIL"))
        || (trimmed.starts_with("ok ") && !trimmed.contains("not ok"))
    {
        return LineKind::PassTest;
    }

    if trimmed.starts_with("Compiling ")
        || trimmed.starts_with("Checking ")
        || trimmed.starts_with("Downloading ")
        || trimmed.starts_with("Downloaded ")
        || trimmed.starts_with("Fresh ")
        || trimmed.starts_with("Updating ")
        || (trimmed.starts_with('[')
            && trimmed.contains(']')
            && (trimmed.contains("Building ") || trimmed.contains("Compiling ")))
    {
        return LineKind::Compiling;
    }

    LineKind::Other
}

pub(crate) fn distill_output(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().collect();
    if lines.len() <= 10 {
        return raw.to_string();
    }

    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        match classify_line(lines[i]) {
            LineKind::PassTest => {
                let start = i;
                while i < lines.len() && matches!(classify_line(lines[i]), LineKind::PassTest) {
                    i += 1;
                }
                let count = i - start;
                if count >= 3 {
                    out.push(format!("[... {count} passed tests omitted ...]"));
                } else {
                    for line in &lines[start..i] {
                        out.push((*line).to_string());
                    }
                }
            }
            LineKind::Compiling => {
                let start = i;
                while i < lines.len() && matches!(classify_line(lines[i]), LineKind::Compiling) {
                    i += 1;
                }
                let count = i - start;
                if count >= 4 {
                    out.push(format!("[... compiled {count} packages ...]"));
                } else {
                    for line in &lines[start..i] {
                        out.push((*line).to_string());
                    }
                }
            }
            LineKind::Other => {
                out.push(lines[i].to_string());
                i += 1;
            }
        }
    }
    let mut result = out.join("\n");
    if raw.ends_with('\n') {
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distills_passing_tests() {
        let mut raw = String::from("running 20 tests\n");
        for i in 1..=15 {
            raw.push_str(&format!("test test_{i} ... ok\n"));
        }
        raw.push_str("test test_fail ... FAILED\n");
        raw.push_str("failures:\n    test_fail\n");
        raw.push_str("test result: FAILED. 15 passed; 1 failed\n");

        let distilled = distill_output(&raw);
        assert!(
            distilled.contains("[... 15 passed tests omitted ...]"),
            "{distilled}"
        );
        assert!(
            distilled.contains("test test_fail ... FAILED"),
            "{distilled}"
        );
        assert!(
            distilled.contains("failures:\n    test_fail"),
            "{distilled}"
        );
    }

    #[test]
    fn preserves_short_output() {
        let raw = "test a ... ok\ntest b ... ok\n";
        assert_eq!(distill_output(raw), raw);
    }

    #[test]
    fn distills_js_tests_and_build_progress_and_spills_large_output() {
        let mut raw = String::new();
        for i in 1..=6 {
            raw.push_str(&format!("Checking crate_{i} v0.1.0\n"));
        }
        for i in 1..=5 {
            raw.push_str(&format!("✔ passes spec {i}\n"));
        }
        raw.push_str("FAIL src/app.test.ts\n");
        let distilled = distill_output(&raw);
        assert!(
            distilled.contains("[... compiled 6 packages ...]"),
            "{distilled}"
        );
        assert!(
            distilled.contains("[... 5 passed tests omitted ...]"),
            "{distilled}"
        );
        assert!(distilled.contains("FAIL src/app.test.ts"), "{distilled}");

        let big = "line\n".repeat(4000);
        let saved = spill_output(&big).expect("spills to temp file");
        assert_eq!(std::fs::read_to_string(&saved).expect("read spill"), big);
        let _ = std::fs::remove_file(saved);
    }
}
