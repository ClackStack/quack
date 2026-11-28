//! `lsp`: definitions, references, hover, symbols and diagnostics from language servers,
//! and `rename`, which changes files. Offered only when a server in `lsp.d` is installed.
//!
//! A rename asks the server in `prepare`, before the permission check, and keeps each
//! file's new text in the arguments (`RENAMED`). The call is then an edit of those files
//! like any other: edit rules, plan mode, approval with the diff, and an undo point each.

use futures::future::BoxFuture;
use quack_lsp::ops::{self, Operation, Query};
use quack_protocol::ApprovalRequest;
use serde_json::{Value, json};

use std::path::PathBuf;

use crate::patch;
use crate::paths;
use crate::registry::{Subject, Tool, ToolCtx, ToolKind, ToolOutput, opt_str, opt_u64};
use crate::text::TextFile;

const OP: &[&str] = &["operation", "op", "action", "command"];
const PATH: &[&str] = &["path", "file_path", "filename", "file"];
const SYMBOL: &[&str] = &["symbol", "name", "query"];
const NEW_NAME: &[&str] = &["new_name", "newName", "to"];
const APPLY: &[&str] = &["apply", "code_action", "title"];
const KIND: &[&str] = &["kind", "only"];
/// Where `prepare` keeps a rename or code-action edit's result. Not part of the schema; a
/// value the model sends is replaced.
const RENAMED: &str = "__quack_renamed";

fn is_rename(args: &Value) -> bool {
    opt_str(args, OP).is_some_and(|o| o.trim().eq_ignore_ascii_case("rename"))
}

fn is_code_action(args: &Value) -> bool {
    opt_str(args, OP).is_some_and(|o| {
        matches!(
            o.trim().to_ascii_lowercase().as_str(),
            "code_action" | "code_actions" | "quickfix" | "fix" | "actions"
        )
    })
}

fn code_action_apply_arg(args: &Value) -> Option<&str> {
    opt_str(args, APPLY)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn is_edit_op(args: &Value) -> bool {
    is_rename(args) || (is_code_action(args) && code_action_apply_arg(args).is_some())
}

/// A rename's planned changes: each file with the text it was worked out from, so a
/// file changed while the user decided is noticed.
fn planned(args: &Value) -> Result<Vec<(PathBuf, String, String)>, String> {
    let list = args[RENAMED]
        .as_array()
        .ok_or("the rename was not worked out; try again")?;
    list.iter()
        .map(|c| {
            let get = |k: &str| c[k].as_str().map(str::to_owned);
            match (get("path"), get("before"), get("after")) {
                (Some(p), Some(b), Some(a)) => Ok((PathBuf::from(p), b, a)),
                _ => Err("the rename was not worked out; try again".to_owned()),
            }
        })
        .collect()
}

fn changes(args: &Value, ctx: &ToolCtx) -> Result<Vec<patch::Change>, String> {
    planned(args)?
        .into_iter()
        .map(|(path, before, after)| {
            let shown = paths::display(&ctx.root, &path);
            // The rename's content is the server's, not the model's, so it needs no
            // read first; only a file that changed mid-approval is refused.
            if !path.exists() {
                return Err(format!("{shown} does not exist"));
            }
            let file = TextFile::load(&path)?;
            if ctx.reads.is_stale(&path) {
                return Err(format!(
                    "{shown} changed on disk since you last read it; read it again before \
                     editing"
                ));
            }
            if file.text != before {
                return Err(format!(
                    "{shown} changed while the rename was being approved; run it again"
                ));
            }
            Ok(patch::Change {
                path,
                shown,
                before: Some(file),
                after: Some(after),
            })
        })
        .collect()
}

#[derive(Debug)]
pub struct Lsp;

fn operation(args: &Value) -> Result<Operation, String> {
    let name = opt_str(args, OP).ok_or("missing required string argument `operation`")?;
    Operation::parse(name).ok_or_else(|| {
        format!(
            "unknown operation `{name}`; use one of: {}, rename, code_action",
            Operation::NAMES.join(", ")
        )
    })
}

fn query(args: &Value, ctx: &ToolCtx) -> Query {
    let to_u32 = |k: &str| opt_u64(args, &[k]).and_then(|n| u32::try_from(n).ok());
    Query {
        path: opt_str(args, PATH).map(|p| paths::resolve(&ctx.root, p)),
        line: to_u32("line"),
        column: to_u32("column").or_else(|| to_u32("character")),
        symbol: opt_str(args, SYMBOL).map(str::to_owned),
    }
}

impl Tool for Lsp {
    fn name(&self) -> &str {
        "lsp"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::ReadOnly
    }

    fn available(&self, ctx: &ToolCtx) -> bool {
        ctx.lsp.as_ref().is_some_and(|m| !m.installed().is_empty())
    }

    fn description(&self, ctx: &ToolCtx) -> String {
        let langs: Vec<String> = ctx
            .lsp
            .as_ref()
            .map(|m| {
                m.installed()
                    .iter()
                    .map(|(_, ext)| format!(".{}", ext.join(" .")))
                    .collect()
            })
            .unwrap_or_default();
        format!(
            "Ask a language server about code ({} files). Operations: `definition` (where a \
             symbol is defined), `references` (where it is used), `hover` (its type and docs), \
             `symbols` (with `symbol`: search the workspace by name; with only `path`: the \
             file's outline), `diagnostics` (errors and warnings; for `path`, or everywhere). \
             For definition/references/hover give `path`, `line` and the `symbol` name on that \
             line. `rename` (with `new_name`) renames the symbol everywhere the server \
             knows of, as one edit for the user to approve. `code_action` lists quickfixes \
             and refactors at `path` (and optional `line`/`end_line`/`symbol`/`kind`); pass \
             `apply` with a number or title to apply one as an approved edit. Edits already \
             report new errors, so you rarely need `diagnostics`.",
            langs.join(" ")
        )
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": { "type": "string", "enum": [
                    "definition", "references", "hover", "symbols", "diagnostics", "rename", "code_action"
                ] },
                "path": { "type": "string", "description": "File path, relative to the workspace root or absolute" },
                "line": { "type": "integer", "description": "1-based line number" },
                "end_line": { "type": "integer", "description": "For code_action: optional 1-based inclusive end line" },
                "symbol": { "type": "string", "description": "Identifier on that line, or a name to search for" },
                "column": { "type": "integer", "description": "1-based column, instead of `symbol`" },
                "new_name": { "type": "string", "description": "For rename: the new name" },
                "kind": { "type": "string", "description": "For code_action: filter by kind (e.g. `quickfix`, `refactor`, `source.organizeImports`)" },
                "apply": { "type": "string", "description": "For code_action: 1-based index or title of the action to apply" }
            },
            "required": ["operation"]
        })
    }

    fn prepare<'a>(
        &'a self,
        mut args: Value,
        ctx: &'a ToolCtx,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            if let Some(o) = args.as_object_mut() {
                o.remove(RENAMED);
            }
            if !is_edit_op(&args) {
                return Ok(args);
            }
            let mgr = ctx
                .lsp
                .as_ref()
                .ok_or("no language servers are configured")?;
            let (label, encoding, files) = if is_rename(&args) {
                let new_name = opt_str(&args, NEW_NAME).unwrap_or_default();
                let rename = ops::rename(mgr, &query(&args, ctx), new_name).await?;
                ("rename", rename.encoding, rename.files)
            } else {
                let selection = code_action_apply_arg(&args).unwrap_or_default();
                let end_line = opt_u64(&args, &["end_line"]).and_then(|n| u32::try_from(n).ok());
                let kind = opt_str(&args, KIND);
                let applied =
                    ops::code_action_apply(mgr, &query(&args, ctx), end_line, kind, selection)
                        .await?;
                ("code action", applied.encoding, applied.files)
            };
            let mut planned = Vec::new();
            for (path, edits) in &files {
                let shown = paths::display(&ctx.root, path);
                if !paths::inside(&ctx.root, path) {
                    return Err(format!(
                        "the {label} would change {shown}, outside the workspace; quack does not do that"
                    ));
                }
                let file = TextFile::load(path)?;
                let after = ops::apply_edits(&file.text, edits, encoding)
                    .map_err(|e| format!("{shown}: {e}"))?;
                planned.push(json!({
                    "path": path.display().to_string(),
                    "before": file.text,
                    "after": after,
                }));
            }
            args[RENAMED] = Value::Array(planned);
            Ok(args)
        })
    }

    fn subject(&self, args: &Value, ctx: &ToolCtx) -> Result<Subject, String> {
        if is_edit_op(args) {
            let files: Vec<PathBuf> = planned(args)?.into_iter().map(|(p, _, _)| p).collect();
            return Ok(Subject::Edits(files));
        }
        if !is_code_action(args) {
            operation(args)?;
        }
        Ok(Subject::Read(match opt_str(args, PATH) {
            Some(p) => paths::resolve(&ctx.root, p),
            None => ctx.root.clone(),
        }))
    }

    fn summary(&self, args: &Value, ctx: &ToolCtx) -> String {
        let op = opt_str(args, OP).unwrap_or("?");
        let mut s = format!("lsp {op}");
        if let Some(sym) = opt_str(args, SYMBOL) {
            s.push_str(&format!(" {sym}"));
        }
        if let Some(to) = opt_str(args, NEW_NAME).filter(|_| is_rename(args)) {
            s.push_str(&format!(" → {to}"));
        }
        if let Some(apply) = code_action_apply_arg(args).filter(|_| is_code_action(args)) {
            s.push_str(&format!(" apply={apply}"));
        }
        if let Some(p) = opt_str(args, PATH) {
            s.push_str(&format!(
                " {}",
                paths::display(&ctx.root, &paths::resolve(&ctx.root, p))
            ));
            if let Some(l) = opt_u64(args, &["line"]) {
                s.push_str(&format!(":{l}"));
            }
        }
        s
    }

    fn preview(&self, args: &Value, ctx: &ToolCtx) -> Result<ApprovalRequest, String> {
        if is_edit_op(args) {
            return Ok(patch::preview(&changes(args, ctx)?));
        }
        Ok(ApprovalRequest::Tool {
            name: "lsp".into(),
            arguments: self.summary(args, ctx),
        })
    }

    fn run<'a>(&'a self, args: Value, ctx: &'a ToolCtx) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            if is_edit_op(&args) {
                return match changes(&args, ctx).and_then(|c| patch::write(&c, ctx)) {
                    Ok(out) => out,
                    Err(e) => ToolOutput::error(e),
                };
            }
            let Some(mgr) = &ctx.lsp else {
                return ToolOutput::error("no language servers are configured");
            };
            if is_code_action(&args) {
                let q = query(&args, ctx);
                let end_line = opt_u64(&args, &["end_line"]).and_then(|n| u32::try_from(n).ok());
                let kind = opt_str(&args, KIND).map(str::to_owned);
                let run = ops::code_actions(mgr, &q, end_line, kind.as_deref());
                let result = tokio::select! {
                    r = run => r.map(|list| ops::format_code_actions(&list)),
                    () = ctx.cancel.cancelled() => Err("interrupted by the user".into()),
                };
                return match result {
                    Ok(text) => {
                        let first = text.lines().next().unwrap_or_default().to_owned();
                        ToolOutput::ok(text, first)
                    }
                    Err(e) => ToolOutput::error(e),
                };
            }
            let op = match operation(&args) {
                Ok(op) => op,
                Err(e) => return ToolOutput::error(e),
            };
            let query = query(&args, ctx);
            let root = ctx.root.clone();
            let display = move |p: &std::path::Path| paths::display(&root, p);
            let run = ops::run(mgr, op, query, &display);
            let result = tokio::select! {
                r = run => r,
                () = ctx.cancel.cancelled() => Err("interrupted by the user".into()),
            };
            match result {
                Ok(text) => {
                    let first = text.lines().next().unwrap_or_default().to_owned();
                    ToolOutput::ok(text, first)
                }
                Err(e) => ToolOutput::error(e),
            }
        })
    }
}
