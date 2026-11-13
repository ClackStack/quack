//! `write`: create or overwrite a file.

use futures::future::BoxFuture;
use quack_config::settings::EditFormat;
use quack_protocol::ApprovalRequest;
use serde_json::{Value, json};

use crate::paths;
use crate::registry::{Subject, Tool, ToolCtx, ToolKind, ToolOutput, arg_str};
use crate::text::{TextFile, io_msg, unified_diff};

const PATH: &[&str] = &["path", "file_path", "filename", "file"];
const CONTENT: &[&str] = &["content", "contents", "text"];

#[derive(Debug)]
pub struct Write;

/// The planned change: path, old text (if any), and the bytes to write.
struct Plan {
    path: std::path::PathBuf,
    shown: String,
    before: Option<String>,
    after: String,
    encoded: String,
}

fn plan(args: &Value, ctx: &ToolCtx) -> Result<Plan, String> {
    let path = paths::resolve(&ctx.root, arg_str(args, PATH)?);
    let shown = paths::display(&ctx.root, &path);
    let content = arg_str(args, CONTENT)?.to_owned();
    if path.is_dir() {
        return Err(format!("{shown} is a directory"));
    }
    // `after` is the diffable text; a new file is written exactly as given, CRLFs and all.
    let crlf = content.contains("\r\n");
    if path.exists() {
        let after = if crlf {
            content.replace("\r\n", "\n")
        } else {
            content
        };
        let file = TextFile::load(&path)?;
        ctx.reads.editable(&path, &shown)?;
        Ok(Plan {
            encoded: file.encode(&after),
            path,
            shown,
            before: Some(file.text),
            after,
        })
    } else {
        let (after, encoded) = if crlf {
            (content.replace("\r\n", "\n"), content)
        } else {
            let after = content.clone();
            (after, content)
        };
        Ok(Plan {
            encoded,
            path,
            shown,
            before: None,
            after,
        })
    }
}

impl Tool for Write {
    fn name(&self) -> &str {
        "write"
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Mutating
    }

    fn description(&self, ctx: &ToolCtx) -> String {
        match ctx.edit_format {
            EditFormat::WholeFile => {
                "Create a file, or change one by writing its complete new content. Parent \
                 directories are created. Read a file before rewriting it, and keep every line \
                 you are not changing exactly as it was; overwriting a file that was never \
                 read fails."
            }
            EditFormat::ApplyPatch => {
                "Create a new file or replace a file's entire content. Parent directories are \
                 created. Prefer `apply_patch` for changing part of an existing file; \
                 overwriting a file that was never read fails."
            }
            EditFormat::SearchReplaceBlocks => {
                "Create a new file or replace a file's entire content. Parent directories are \
                 created. Prefer `edit_blocks` for changing part of an existing file; \
                 overwriting a file that was never read fails."
            }
            EditFormat::Hashline => {
                "Create a new file or replace a file's entire content. Parent directories are \
                 created. Prefer `edit_hashline` for changing part of an existing file; \
                 overwriting a file that was never read fails."
            }
            EditFormat::StrReplace | EditFormat::Auto => {
                "Create a new file or replace a file's entire content. Parent directories are \
                 created. Prefer `edit` for changing part of an existing file; overwriting a \
                 file that was never read fails."
            }
        }
        .into()
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path, relative to the workspace root or absolute" },
                "content": { "type": "string", "description": "The complete file content" }
            },
            "required": ["path", "content"]
        })
    }

    fn subject(&self, args: &Value, ctx: &ToolCtx) -> Result<Subject, String> {
        Ok(Subject::Edit(paths::resolve(
            &ctx.root,
            arg_str(args, PATH)?,
        )))
    }

    fn summary(&self, args: &Value, ctx: &ToolCtx) -> String {
        let path = arg_str(args, PATH)
            .map(|p| paths::display(&ctx.root, &paths::resolve(&ctx.root, p)))
            .unwrap_or_default();
        format!("write {path}")
    }

    fn preview(&self, args: &Value, ctx: &ToolCtx) -> Result<ApprovalRequest, String> {
        let p = plan(args, ctx)?;
        Ok(ApprovalRequest::Edit {
            diff: unified_diff(&p.shown, p.before.as_deref().unwrap_or(""), &p.after),
            path: p.shown,
        })
    }

    fn run<'a>(&'a self, args: Value, ctx: &'a ToolCtx) -> BoxFuture<'a, ToolOutput> {
        let ctx = ctx.clone();
        Box::pin(crate::registry::blocking(move || {
            let p = plan(&args, &ctx)?;
            if let Some(parent) = p.path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io_msg(parent, &e))?;
            }
            std::fs::write(&p.path, &p.encoded).map_err(|e| io_msg(&p.path, &e))?;
            let c = crate::text::commit(
                &ctx,
                &p.path,
                &p.shown,
                p.before.as_deref(),
                &p.after,
                "change",
                false,
            );
            crate::hashline::saw_whole(&ctx, &p.path, &p.after);
            let verb = if p.before.is_some() {
                "Updated"
            } else {
                "Created"
            };
            let mut msg = format!("{verb} {} ({} lines).", p.shown, p.after.lines().count());
            if let Some(warning) = &c.warning {
                msg.push_str("\n\n");
                msg.push_str(warning);
            }
            Ok(ToolOutput::ok(msg, format!("+{} -{}", c.add, c.del)).with_detail(c.detail))
        }))
    }
}
