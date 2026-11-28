//! The engine: a tokio task that owns a session, receives [`Op`]s and emits [`Event`]s.
//!
//! Clients (TUI, headless exec, ACP) hold an [`EngineHandle`] and never call subsystems
//! directly (docs/plan/00-architecture.md#engine-protocol).

mod agent;
mod audit;
pub mod checks;
mod compat;
mod context;
pub mod doctor;
mod engine;
mod env;
mod export;
pub mod gui;
mod handoff;
mod hook_setup;
mod hooks;
mod lsp;
mod mcp;
mod mcp_ask;
mod mentions;
mod models;
mod nudge;
mod sandbox;
pub mod secrets;
mod session;
pub mod setup;
mod startup;
mod stats;
mod turn;
mod watch;

pub use engine::{Engine, EngineHandle};
pub use env::memory_dirs;
pub use mcp::oauth_client as mcp_oauth_client;
pub use session::{
    Resume, append_history, history_path, list_sessions, load_history, prune_sessions,
    sessions_dirs,
};

pub use quack_protocol as protocol;
/// Grammars and syntax highlighting, for clients that show code.
pub use quack_tools::grammar;
/// Workspace file listing for `@file` completion.
pub use quack_tools::workspace_files;
/// Optional host-client delegation hooks (e.g. ACP editor `fs/*` and `terminal/*`).
pub use quack_tools::{ClientBridge, TerminalOutput};
