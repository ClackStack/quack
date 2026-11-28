//! Internal tools, the tool registry and the permission engine.
//! See docs/plan/04-agent-loop-and-tools.md and 11-security.md.

mod ask;
mod blocks;
pub mod client_bridge;
mod edit;
pub mod grammar;
mod hashline;
mod image;
mod index;
mod lsp;
mod memory;
pub mod outline;
mod patch;
pub mod paths;
pub mod permissions;
#[cfg(feature = "lang-common")]
mod ps_split;
mod read;
mod registry;
mod search;
mod shell;
mod shell_split;
mod skill;
mod task;
mod text;
mod todo;
pub mod web_fetch;
mod write;

pub use ask::{Answers, AskRequest, Asker};
pub use client_bridge::{ClientBridge, TerminalOutput};
pub use permissions::{Decision, Permissions, is_read_only_shell};
pub use registry::{
    ReadTracker, Registry, SandboxState, Subject, Tool, ToolCtx, ToolKind, ToolOutput,
};
pub use search::workspace_files;
pub use skill::expand as expand_skill;
pub use task::{AgentInfo, SpawnRequest, Spawner, TaskReport, TaskRequest};
pub use todo::Plan;
pub use web_fetch::{WebFetchResponse, WebFetcher};
