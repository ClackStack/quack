//! The desktop client's bridge (docs/plan/15-desktop-client.md): engines for webview
//! tabs, with no Tauri dependency.
//!
//! This crate is a fourth client of the Op/Event protocol, like the TUI, `exec` and
//! ACP. The library half is pure Rust so `cargo test --workspace`,
//! `cargo clippy --workspace` and the MSRV check run on every OS without `webkit2gtk`
//! or a built frontend; the Tauri 2 shell lives behind the `desktop` feature
//! (`src/main.rs`) and the SPA in `ui/`.
//!
//! - [`registry`]: one engine per tab, opened on a folder, with a duplicate-session
//!   guard and a clean shutdown on close.
//! - [`coalesce`]: the 16 ms delta coalescer that merges streaming text events before
//!   they cross the IPC boundary.
//! - [`appinfo`]: the `AppInfo` snapshot the webview is sent at tab start and after a
//!   config reload, the same static facts the TUI gathers in `AppInfo::from_config`.
//! - [`recent`]: recently opened folders, persisted in the user data dir.
//! - [`path_env`]: the login-shell `PATH` fix for launching from Finder/Dock.

pub mod appinfo;
pub mod coalesce;
#[cfg(feature = "desktop")]
pub mod path_env;
pub mod recent;
pub mod registry;

pub use appinfo::AppInfo;
pub use coalesce::DeltaCoalescer;
pub use recent::{RecentFolder, RecentFolders};
pub use registry::{OpenError, Payload, Registry, Sink, TabId, TabMessage};
