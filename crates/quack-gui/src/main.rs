//! The Tauri 2 desktop shell (docs/plan/15-desktop-client.md): a thin wrapper over the
//! pure-Rust bridge. Compiled only behind `--features desktop`; the webview talks to
//! the engines through these commands and one `tauri::ipc::Channel` per tab:
//!
//! ```text
//! invoke("open_folder", { cwd, resume, events }) → tab
//! invoke("send_op",     { tab, op })             // op is the tagged JSON of a protocol Op
//! invoke("close_tab",   { tab })                 // awaits the engine's shutdown
//! invoke("recent_folders", { limit })
//! ```

use std::sync::Arc;

use quack_core::Resume;
use quack_gui::{OpenError, Registry, Sink, TabMessage};
use serde::Deserialize;
use tauri::ipc::Channel;
use tauri::{Manager, RunEvent, State};

/// What the webview asked a new tab to open with (`Resume` is a `quack-core` type, so
/// it crosses the IPC as this small tagged JSON).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "resume", rename_all = "snake_case")]
enum ResumeDto {
    New,
    Latest,
    Id { id: String },
}

impl From<ResumeDto> for Resume {
    fn from(dto: ResumeDto) -> Self {
        match dto {
            ResumeDto::New => Self::New,
            ResumeDto::Latest => Self::Latest,
            ResumeDto::Id { id } => Self::Id(id),
        }
    }
}

/// Forwards bridge messages to the tab's channel; [`Channel::send`] posts
/// asynchronously and never blocks the engine task.
struct ChannelSink(Channel<TabMessage>);

impl Sink for ChannelSink {
    fn send(&self, message: TabMessage) {
        if let Err(e) = self.0.send(message) {
            tracing::warn!(%e, "could not push a tab message to the webview");
        }
    }
}

/// A tokio mutex because commands hold the guard across `await`s (opens wait for the
/// engine's session to come up).
type RegistryState = tokio::sync::Mutex<Registry>;

#[tauri::command]
async fn open_folder(
    state: State<'_, RegistryState>,
    cwd: String,
    resume: Option<ResumeDto>,
    events: Channel<TabMessage>,
) -> Result<u64, OpenError> {
    let resume = resume.map(Into::into).unwrap_or(Resume::New);
    let mut registry = state.lock().await;
    registry
        .open(
            std::path::PathBuf::from(cwd),
            resume,
            Arc::new(ChannelSink(events)),
        )
        .await
}

#[tauri::command]
async fn send_op(
    state: State<'_, RegistryState>,
    tab: u64,
    op: serde_json::Value,
) -> Result<(), String> {
    state.lock().await.send_op(tab, op).await
}

#[tauri::command]
async fn close_tab(state: State<'_, RegistryState>, tab: u64) -> Result<(), String> {
    state.lock().await.close(tab).await;
    Ok(())
}

#[tauri::command]
async fn recent_folders(
    state: State<'_, RegistryState>,
    limit: Option<u32>,
) -> Result<Vec<quack_gui::RecentFolder>, String> {
    Ok(state
        .lock()
        .await
        .recent_folders(limit.unwrap_or(10).max(1) as usize))
}

fn main() {
    // Launched from Finder/Dock, the PATH is the system default; give the engines the
    // login shell's so npx, cargo and friends are found (macOS/Linux).
    quack_gui::path_env::apply_login_shell_path();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .manage(RegistryState::new(Registry::new()))
        .invoke_handler(tauri::generate_handler![
            open_folder,
            send_op,
            close_tab,
            recent_folders
        ])
        .build(tauri::generate_context!())
        .expect("the tauri application builds")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                // Engines must finish their shutdown: child processes, MCP/LSP servers
                // and session_end hooks.
                let state: State<RegistryState> = app.state();
                tauri::async_runtime::block_on(async {
                    state.lock().await.close_all().await;
                });
            }
        });
}
