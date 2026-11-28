//! Optional bridge allowing a host client (such as an ACP editor) to handle file I/O
//! and foreground terminal commands so the agent sees unsaved buffers and runs commands
//! inside the editor's integrated terminal.

use std::path::Path;
use std::time::Duration;

use futures::future::BoxFuture;
use tokio_util::sync::CancellationToken;

/// Output of a foreground command run inside a client-hosted terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalOutput {
    pub output: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub cancelled: bool,
}

/// Optional host-client delegation hooks injected into [`crate::ToolCtx`].
///
/// Returning `None` from any method means the client does not handle that capability
/// (or it is disabled by config), so the tool falls back to local disk / shell execution.
pub trait ClientBridge: Send + Sync + std::fmt::Debug {
    /// Read a text file through the host client (e.g. ACP `fs/read_text_file`).
    fn read_text_file<'a>(
        &'a self,
        _path: &'a Path,
    ) -> BoxFuture<'a, Option<Result<String, String>>> {
        Box::pin(async { None })
    }

    /// Write a text file through the host client (e.g. ACP `fs/write_text_file`).
    fn write_text_file<'a>(
        &'a self,
        _path: &'a Path,
        _content: &'a str,
    ) -> BoxFuture<'a, Option<Result<(), String>>> {
        Box::pin(async { None })
    }

    /// Run a foreground command in the host client's terminal (e.g. ACP `terminal/*`).
    fn run_terminal<'a>(
        &'a self,
        _command: &'a str,
        _cwd: &'a Path,
        _timeout: Duration,
        _cancel: CancellationToken,
    ) -> BoxFuture<'a, Option<Result<TerminalOutput, String>>> {
        Box::pin(async { None })
    }
}
