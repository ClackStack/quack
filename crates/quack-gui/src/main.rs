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
//! invoke("open_url", { url })                    // a sign-in address from the engine
//! ```

use std::path::Path;
use std::sync::Arc;

use quack_core::Resume;
use quack_gui::{OpenError, Registry, SavedTab, Sink, StartupPayload, TabMessage, WorkspaceSymbol};
use serde::Deserialize;
use tauri::ipc::Channel;
use tauri::{Emitter, Manager, RunEvent, State};

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

#[tauri::command]
async fn input_history(
    state: State<'_, Registry>,
    client: u64,
    tab: u64,
) -> Result<Vec<String>, String> {
    state.input_history_for(client, tab).await
}

/// Opens an external browser URL handed over by the engine (subscription sign-in or
/// the configured release page). Restricts to `http://`/`https://` URLs; no arbitrary
/// `fetch` or shell access is granted to the webview.
#[tauri::command]
fn open_url(app: tauri::AppHandle, url: String) -> Result<(), String> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("refusing to open a non-http(s) URL".into());
    }
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_workspace_files(
    state: State<'_, Registry>,
    client: u64,
    tab: u64,
    limit: Option<u32>,
) -> Result<Vec<String>, String> {
    state
        .list_files_for(client, tab, limit.unwrap_or(5000).clamp(1, 20_000) as usize)
        .await
}

#[tauri::command]
async fn list_workspace_symbols(
    state: State<'_, Registry>,
    client: u64,
    tab: u64,
    path: String,
    query: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<WorkspaceSymbol>, String> {
    state
        .list_symbols_for(
            client,
            tab,
            path,
            query,
            limit.unwrap_or(50).clamp(1, 200) as usize,
        )
        .await
}

#[tauri::command]
fn startup_targets(state: State<'_, Registry>, client: u64) -> Result<StartupPayload, String> {
    state.startup_targets_for(client)
}

#[tauri::command]
fn save_open_tabs(
    state: State<'_, Registry>,
    client: u64,
    tabs: Vec<SavedTab>,
) -> Result<(), String> {
    state.save_open_tabs_for(client, &tabs)
}

#[tauri::command]
async fn reveal_path(
    app: tauri::AppHandle,
    state: State<'_, Registry>,
    client: u64,
    tab: u64,
    path: String,
    open_file: Option<bool>,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let resolved = state.resolve_workspace_path_for(client, tab, Path::new(&path))?;
    if open_file.unwrap_or(false) {
        app.opener()
            .open_path(resolved.display().to_string(), None::<&str>)
            .map_err(|e| e.to_string())
    } else {
        app.opener()
            .reveal_item_in_dir(resolved)
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
async fn highlight_code(lang: String, code: String) -> Option<Vec<quack_gui::HighlightSpan>> {
    tokio::task::spawn_blocking(move || quack_gui::highlight_code(&lang, &code))
        .await
        .ok()
        .flatten()
}

#[tauri::command]
fn stage_pasted_image(
    state: State<'_, Registry>,
    client: u64,
    tab: u64,
    bytes: Vec<u8>,
) -> Result<String, String> {
    state.save_pasted_image_for(client, tab, &bytes)
}

#[tauri::command]
fn notify_attention(app: tauri::AppHandle, title: String, body: String) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::debug!(%e, "desktop attention notification skipped");
    }
}

fn main() {
    // Launched from Finder/Dock, the PATH is the system default; give the engines the
    // login shell's so npx, cargo and friends are found (macOS/Linux).
    quack_gui::path_env::apply_login_shell_path();
    // Create the isolated sandbox desktop before Tauri opens windows on WinSta0.
    #[cfg(windows)]
    let _ = quack_sandbox::available();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            let parsed = quack_gui::tabs::parse_argv(&argv, Some(Path::new(&cwd)));
            if !parsed.targets.is_empty() {
                let _ = app.emit("quack://open-request", &parsed.targets);
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let resource_dir = app.path().resource_dir()?;
            let registry = Registry::with_resource_dir(resource_dir);
            let argv: Vec<String> = std::env::args().collect();
            let cwd = std::env::current_dir().ok();
            registry.set_initial_argv(&argv, cwd.as_deref());
            app.manage(registry);
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
            recent_folders,
            startup_targets,
            save_open_tabs,
            input_history,
            open_url,
            list_workspace_files,
            list_workspace_symbols,
            reveal_path,
            highlight_code,
            stage_pasted_image,
            notify_attention
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
