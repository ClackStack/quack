//! The Tauri 2 desktop shell (docs/plan/15-desktop-client.md): a thin wrapper over the
//! pure-Rust bridge. Compiled only behind `--features desktop`; the webview talks to
//! the engines through these commands and one `tauri::ipc::Channel` per tab:
//!
//! ```text
//! invoke("connect") → client                    // retires the previous UI's engines
//! invoke("open_folder", { client, cwd, resume, events }) → tab
//! invoke("send_op", { client, tab, op })         // the tagged JSON of a protocol Op
//! invoke("close_tab", { client, tab })           // awaits engine shutdown
//! invoke("recent_folders", { client, limit })
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

#[tauri::command]
async fn connect(state: State<'_, Registry>) -> Result<u64, String> {
    state.connect().await
}

#[tauri::command]
async fn open_folder(
    state: State<'_, Registry>,
    client: u64,
    cwd: String,
    resume: Option<ResumeDto>,
    events: Channel<TabMessage>,
) -> Result<u64, OpenError> {
    let resume = resume.map(Into::into).unwrap_or(Resume::New);
    state
        .open_for(
            client,
            std::path::PathBuf::from(cwd),
            resume,
            Arc::new(ChannelSink(events)),
        )
        .await
}

#[tauri::command]
async fn send_op(
    state: State<'_, Registry>,
    client: u64,
    tab: u64,
    op: serde_json::Value,
) -> Result<(), String> {
    state.send_op_for(client, tab, op).await
}

#[tauri::command]
async fn close_tab(state: State<'_, Registry>, client: u64, tab: u64) -> Result<(), String> {
    state.close_for(client, tab).await
}

#[tauri::command]
async fn recent_folders(
    state: State<'_, Registry>,
    client: u64,
    limit: Option<u32>,
) -> Result<Vec<quack_gui::RecentFolder>, String> {
    state.check_client(client)?;
    Ok(state.recent_folders(limit.unwrap_or(10).max(1) as usize))
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
        .setup(|app| {
            let resource_dir = app.path().resource_dir()?;
            app.manage(Registry::with_resource_dir(resource_dir));
            Ok(())
        })
        .on_page_load(|webview, payload| {
            if webview.label() == "main"
                && payload.event() == tauri::webview::PageLoadEvent::Started
                && let Some(registry) = webview.try_state::<Registry>()
            {
                registry.disconnect();
            }
        })
        .invoke_handler(tauri::generate_handler![
            connect,
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
                let state: State<Registry> = app.state();
                tauri::async_runtime::block_on(async {
                    state.close_all().await;
                });
            }
        });
}
