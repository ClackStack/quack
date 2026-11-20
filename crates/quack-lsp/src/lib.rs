//! LSP client manager (docs/plan/10-lsp.md): language servers from `lsp.d`, started
//! lazily, diagnostics after edits, and the queries behind the `lsp` tool.

mod client;
pub mod diag;
mod manager;
pub mod ops;
pub mod pos;
mod rpc;

pub use client::{Client, WatchChangeType};
pub use manager::{
    DiagnosticEntry, LspManager, MAX_RESTARTS, Prepare, ServerSpec, Status, TOOL_WAIT,
};
