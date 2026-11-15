//! The engine-per-tab registry (docs/plan/15-desktop-client.md#the-bridge).
//!
//! Opening a tab loads the folder's config and spawns its engine (the ACP pattern),
//! waits for the initial session announcement, and hands the engine to a
//! forwarder task that pushes coalesced [`TabMessage`]s into the tab's [`Sink`] — on
//! desktop, a wrapper around a `tauri::ipc::Channel`. The registry refuses a second
//! live engine for a workspace before spawning it, so two engines never write to one
//! JSONL log, and a close awaits `engine.shutdown()` so MCP/LSP servers and
//! `session_end` hooks finish.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quack_config::LoadOptions;
use quack_core::{Engine, EngineHandle, Resume};
use quack_protocol::{Event, InputItem, Op, SessionId};
use serde::Serialize;
use tokio::sync::{mpsc, oneshot, watch};

use crate::appinfo::AppInfo;
use crate::coalesce::DeltaCoalescer;
use crate::recent::{RecentFolders, expand_tilde_with, workspace_key};
use crate::tabs::{
    OpenTabsStore, ParsedLaunch, SavedTab, StartupPayload, merge_startup_targets, parse_argv,
};

/// A syntax symbol returned to the composer's `@file#symbol` picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceSymbol {
    pub name: String,
    pub kind: String,
    pub start: usize,
    pub end: usize,
    pub signature: String,
}

/// How long opening a tab may take: config, the model, and replaying a saved log.
const OPEN_TIMEOUT: Duration = Duration::from_secs(60);

/// The coalescing tick: 60 fps, matching the TUI's frame loop.
const FLUSH_TICK: Duration = Duration::from_millis(16);

pub type TabId = u64;

/// Where a tab's messages go. Desktop passes a wrapper around a per-tab
/// `tauri::ipc::Channel`; tests collect into an mpsc channel.
pub trait Sink: Send + Sync + 'static {
    fn send(&self, message: TabMessage);
}

impl<F: Fn(TabMessage) + Send + Sync + 'static> Sink for F {
    fn send(&self, message: TabMessage) {
        self(message)
    }
}

/// One message across the IPC boundary: the tab it belongs to and its payload,
/// serialized as `{"tab":…,"event":{…}}`, `{"tab":…,"info":{…}}` or
/// `{"tab":…,"closed":{…}}`.
#[derive(Debug, Clone, Serialize)]
pub struct TabMessage {
    pub tab: TabId,
    #[serde(flatten)]
    pub payload: Payload,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Payload {
    Event { event: Box<Event> },
    Info { info: Box<AppInfo> },
    History { history: Vec<String> },
    Closed { closed: Closed },
}

#[derive(Debug, Clone, Serialize)]
pub struct Closed {
    /// Why the tab's engine stopped, when it said.
    pub reason: Option<String>,
}

impl TabMessage {
    pub fn event(tab: TabId, event: Event) -> Self {
        Self {
            tab,
            payload: Payload::Event {
                event: Box::new(event),
            },
        }
    }

    pub fn info(tab: TabId, info: AppInfo) -> Self {
        Self {
            tab,
            payload: Payload::Info {
                info: Box::new(info),
            },
        }
    }

    pub fn history(tab: TabId, history: Vec<String>) -> Self {
        Self {
            tab,
            payload: Payload::History { history },
        }
    }

    pub fn closed(tab: TabId, reason: Option<String>) -> Self {
        Self {
            tab,
            payload: Payload::Closed {
                closed: Closed { reason },
            },
        }
    }
}

/// Why a folder did not open.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum OpenError {
    Config(String),
    Engine(String),
    /// The workspace is already open in that tab; focus it instead of spawning again.
    AlreadyOpen {
        tab: TabId,
    },
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(e) => write!(f, "could not load the config: {e}"),
            Self::Engine(e) => write!(f, "the engine did not start: {e}"),
            Self::AlreadyOpen { tab } => {
                write!(f, "this workspace is already open in tab {tab}")
            }
        }
    }
}

impl std::error::Error for OpenError {}

/// Loads the config for a folder. The default reads the process environment; tests
/// inject an explicit variable map so they never read the developer's real config.
pub type Loader = Arc<dyn Fn(LoadOptions) -> Result<quack_config::Loaded, String> + Send + Sync>;

/// A reserved workspace remains owned while starting, running, or shutting down.
/// The task owns its engine independently of the command that opened it.
struct Tab {
    session: watch::Receiver<Option<SessionId>>,
    cwd: PathBuf,
    root: PathBuf,
    workspace_key: PathBuf,
    history_path: Arc<std::sync::RwLock<PathBuf>>,
    ops: mpsc::Sender<Op>,
    stop: watch::Sender<bool>,
    done: watch::Receiver<bool>,
}

impl Tab {
    fn finished(&self) -> bool {
        *self.done.borrow() || self.done.has_changed().is_err()
    }

    fn stop(&self) {
        self.stop.send_replace(true);
    }

    async fn stopped(&self) {
        let mut done = self.done.clone();
        while !*done.borrow_and_update() {
            if done.changed().await.is_err() {
                break;
            }
        }
    }
}

#[derive(Default)]
struct Tabs {
    entries: HashMap<TabId, Arc<Tab>>,
    next: TabId,
    client: u64,
}

/// The open tabs, one engine each. The map lock covers bookkeeping only; config
/// loading, startup, channel backpressure and shutdown never hold it across awaits.
pub struct Registry {
    tabs: Mutex<Tabs>,
    load: Loader,
    recent: Mutex<RecentFolders>,
    session_tabs: Mutex<OpenTabsStore>,
    initial_launch: Mutex<Option<ParsedLaunch>>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tabs = self.tabs.lock().expect("tab map lock");
        f.debug_struct("Registry")
            .field("tabs", &tabs.entries.keys().collect::<Vec<_>>())
            .field("client", &tabs.client)
            .finish_non_exhaustive()
    }
}

impl Registry {
    /// A registry that loads configs from the process environment.
    pub fn new() -> Self {
        let load: Loader =
            Arc::new(|opts: LoadOptions| quack_config::load(&opts).map_err(|e| e.to_string()));
        Self::with_loader(load)
    }

    /// Load defaults from an optional native resource directory when present, falling back
    /// to the shared on-disk defaults locations otherwise. An explicit `QUACK_DEFAULTS_DIR`
    /// (including `none`) still takes precedence.
    pub fn with_resource_dir(resource_dir: PathBuf) -> Self {
        let load: Loader = Arc::new(move |opts| {
            load_from_resources(&opts, &resource_dir, &|key| std::env::var_os(key))
        });
        Self::with_loader(load)
    }

    /// A registry with a custom config loader (tests).
    pub fn with_loader(load: Loader) -> Self {
        Self {
            tabs: Mutex::default(),
            load,
            recent: Mutex::new(RecentFolders::load()),
            session_tabs: Mutex::new(OpenTabsStore::load()),
            initial_launch: Mutex::new(None),
        }
    }

    /// Isolate both config and recent-folder persistence from the real user data.
    pub fn with_parts(load: Loader, recent_dir: PathBuf) -> Self {
        Self {
            tabs: Mutex::default(),
            load,
            recent: Mutex::new(RecentFolders::at(recent_dir.clone())),
            session_tabs: Mutex::new(OpenTabsStore::at(recent_dir)),
            initial_launch: Mutex::new(None),
        }
    }

    /// Capture the desktop process's launch arguments once during startup.
    pub fn set_initial_argv(&self, argv: &[String], base_cwd: Option<&Path>) {
        let parsed = parse_argv(argv, base_cwd);
        *self.initial_launch.lock().expect("initial launch lock") = Some(parsed);
    }

    /// Compute initial tabs to open on webview startup, merging restored tabs with launch paths.
    pub fn startup_targets_for(&self, client: u64) -> Result<StartupPayload, String> {
        self.check_client(client)?;
        let restore_tabs = (self.load)(LoadOptions::default())
            .map(|cfg| cfg.settings.ui.gui_restore_tabs)
            .unwrap_or(true);
        let launch = self
            .initial_launch
            .lock()
            .expect("initial launch lock")
            .take()
            .unwrap_or_default();
        let saved = self
            .session_tabs
            .lock()
            .expect("session tabs lock")
            .existing_saved();
        Ok(merge_startup_targets(
            &saved,
            &launch.targets,
            restore_tabs,
            launch.no_restore,
        ))
    }

    /// Persist the current open workspace tabs to `session-tabs.json`.
    pub fn save_open_tabs_for(&self, client: u64, tabs: &[SavedTab]) -> Result<(), String> {
        self.check_client(client)?;
        self.session_tabs
            .lock()
            .expect("session tabs lock")
            .save(tabs);
        Ok(())
    }

    /// Recently opened folders, most recent first.
    pub fn recent_folders(&self, limit: usize) -> Vec<crate::recent::RecentFolder> {
        self.recent.lock().expect("recent folders lock").list(limit)
    }

    /// Start a new webview connection. Invalidates stale commands immediately,
    /// invalidates pending opens and awaits all previous engines before returning.
    pub async fn connect(&self) -> Result<u64, String> {
        let (client, tabs) = self.retire();
        for tab in tabs {
            tab.stopped().await;
        }
        self.check_client(client)?;
        Ok(client)
    }

    /// A full webview navigation discarded its channels. Stop tasks immediately;
    /// the next connection (or app exit) awaits their completed shutdown.
    pub fn disconnect(&self) {
        self.retire();
    }

    /// Reject commands from a discarded UI document.
    pub fn check_client(&self, client: u64) -> Result<(), String> {
        if self.tabs.lock().expect("tab map lock").client == client {
            Ok(())
        } else {
            Err("This desktop connection was replaced · reload the window".to_owned())
        }
    }

    /// Open using the current connection (headless bridge callers).
    pub async fn open(
        &self,
        cwd: PathBuf,
        resume: Resume,
        sink: Arc<dyn Sink>,
    ) -> Result<TabId, OpenError> {
        let client = self.tabs.lock().expect("tab map lock").client;
        self.open_for(client, cwd, resume, sink).await
    }

    /// Load and reserve a workspace, then await its task's first session snapshot.
    /// Stale config loads cannot publish an engine after a webview reconnect.
    pub async fn open_for(
        &self,
        client: u64,
        cwd: PathBuf,
        resume: Resume,
        sink: Arc<dyn Sink>,
    ) -> Result<TabId, OpenError> {
        self.check_client(client).map_err(OpenError::Engine)?;
        let cwd = expand_tilde_with(cwd, &|k| std::env::var_os(k));
        if cwd.as_os_str().is_empty() || !cwd.is_dir() {
            return Err(OpenError::Config(format!(
                "folder does not exist: {} · choose an existing folder",
                cwd.display()
            )));
        }
        let opts = LoadOptions::new(cwd.clone());
        let load = self.load.clone();
        let loaded = tokio::task::spawn_blocking(move || load(opts))
            .await
            .map_err(|e| OpenError::Config(e.to_string()))?
            .map_err(OpenError::Config)?;
        let workspace = workspace_key(&loaded.dirs.workspace_root);
        let recent_root = if !loaded.dirs.workspace_root.as_os_str().is_empty()
            && loaded.dirs.workspace_root.is_dir()
        {
            loaded.dirs.workspace_root.clone()
        } else {
            cwd.clone()
        };
        // A closing tab keeps its reservation until shutdown has finished. Reopen
        // waits here without blocking operations on unrelated tabs.
        let (id, tab, started) = loop {
            let closing = {
                let mut tabs = self.tabs.lock().expect("tab map lock");
                if tabs.client != client {
                    return Err(OpenError::Engine(
                        "This desktop connection was replaced · reopen the folder".to_owned(),
                    ));
                }
                tabs.entries.retain(|_, tab| !tab.finished());
                if let Some((&id, tab)) = tabs
                    .entries
                    .iter()
                    .find(|(_, tab)| tab.workspace_key == workspace)
                {
                    if !*tab.stop.borrow() {
                        return Err(OpenError::AlreadyOpen { tab: id });
                    }
                    Some(tab.clone())
                } else {
                    let id = tabs.next;
                    tabs.next += 1;
                    let (tab, started) = spawn_tab(
                        id,
                        Arc::new(loaded),
                        resume,
                        sink,
                        cwd.clone(),
                        recent_root.clone(),
                        workspace,
                    );
                    tabs.entries.insert(id, tab.clone());
                    break (id, tab, started);
                }
            };
            if let Some(tab) = closing {
                tab.stopped().await;
            }
        };
        if let Err(error) = started
            .await
            .unwrap_or_else(|_| Err("the engine startup task stopped".to_owned()))
        {
            tab.stop();
            tab.stopped().await;
            return Err(OpenError::Engine(error));
        }
        self.check_client(client).map_err(OpenError::Engine)?;
        self.recent
            .lock()
            .expect("recent folders lock")
            .touch(recent_root);
        Ok(id)
    }

    /// Send a serialized Op without holding the registry lock over backpressure.
    pub async fn send_op(&self, tab: TabId, op: serde_json::Value) -> Result<(), String> {
        let client = self.tabs.lock().expect("tab map lock").client;
        self.send_op_for(client, tab, op).await
    }

    pub async fn send_op_for(
        &self,
        client: u64,
        tab: TabId,
        op: serde_json::Value,
    ) -> Result<(), String> {
        let op: Op = serde_json::from_value(op)
            .map_err(|e| format!("not a protocol op: {e} · regenerate protocol.ts"))?;
        let tab_state = self
            .get_tab(client, tab)?
            .ok_or_else(|| format!("no open tab {tab}"))?;
        if *tab_state.stop.borrow() || tab_state.finished() {
            return Err(format!("tab {tab} has stopped"));
        }
        let mut stop = tab_state.stop.subscribe();
        tokio::select! {
            biased;
            _ = stopping(&mut stop) => Err(format!("tab {tab} has stopped")),
            result = tab_state.ops.send(op) => result.map_err(|_| format!("tab {tab} has stopped")),
        }
    }

    fn get_tab(&self, client: u64, tab: TabId) -> Result<Option<Arc<Tab>>, String> {
        let tabs = self.tabs.lock().expect("tab map lock");
        if tabs.client != client {
            return Err("This desktop connection was replaced · reload the window".to_owned());
        }
        Ok(tabs.entries.get(&tab).cloned())
    }

    /// The tab showing this session, including session switches within a workspace.
    pub fn tab_of(&self, session: &SessionId) -> Option<TabId> {
        self.tabs
            .lock()
            .expect("tab map lock")
            .entries
            .iter()
            .find(|(_, tab)| !tab.finished() && tab.session.borrow().as_ref() == Some(session))
            .map(|(id, _)| *id)
    }

    pub fn cwd(&self, tab: TabId) -> Option<PathBuf> {
        self.tabs
            .lock()
            .expect("tab map lock")
            .entries
            .get(&tab)
            .map(|t| t.cwd.clone())
    }

    /// List workspace-relative files for `@` mentions off the async runtime.
    pub async fn list_files_for(
        &self,
        client: u64,
        tab: TabId,
        limit: usize,
    ) -> Result<Vec<String>, String> {
        let Some(tab) = self.get_tab(client, tab)? else {
            return Err("this workspace tab is closed".to_owned());
        };
        let root = tab_root(&tab);
        tokio::task::spawn_blocking(move || quack_tools::workspace_files(&root, limit))
            .await
            .map_err(|e| format!("could not list workspace files: {e}"))
    }

    /// List syntax symbols in `rel_path` inside the tab's workspace for `@file#symbol`.
    pub async fn list_symbols_for(
        &self,
        client: u64,
        tab: TabId,
        rel_path: String,
        query: Option<String>,
        limit: usize,
    ) -> Result<Vec<WorkspaceSymbol>, String> {
        let Some(tab) = self.get_tab(client, tab)? else {
            return Err("this workspace tab is closed".to_owned());
        };
        let root = tab_root(&tab);
        let query = query.unwrap_or_default();
        tokio::task::spawn_blocking(move || {
            match quack_tools::outline::list_symbols(
                &root,
                &quack_tools::grammar::Grammars::builtin(),
                &rel_path,
                &query,
                limit,
            ) {
                Ok(syms) => Ok(syms
                    .into_iter()
                    .map(|s| WorkspaceSymbol {
                        name: s.name,
                        kind: s.kind,
                        start: s.start,
                        end: s.end,
                        signature: s.signature,
                    })
                    .collect()),
                Err(msg) if msg.starts_with("no syntax outline available") => Ok(Vec::new()),
                Err(msg) => Err(msg),
            }
        })
        .await
        .map_err(|e| format!("could not list symbols: {e}"))?
    }

    /// Read shared workspace prompt history (`history.jsonl`), oldest first.
    pub async fn input_history(&self, tab: TabId) -> Result<Vec<String>, String> {
        let client = self.tabs.lock().expect("tab map lock").client;
        self.input_history_for(client, tab).await
    }

    pub async fn input_history_for(&self, client: u64, tab: TabId) -> Result<Vec<String>, String> {
        let Some(tab) = self.get_tab(client, tab)? else {
            return Err("this workspace tab is closed".to_owned());
        };
        let path = read_history_path(&tab.history_path);
        tokio::task::spawn_blocking(move || quack_core::load_history(&path))
            .await
            .map_err(|e| format!("could not load input history: {e}"))
    }

    /// Resolve a user-clicked relative or workspace path, refusing anything outside
    /// the active tab's workspace root.
    pub fn resolve_workspace_path_for(
        &self,
        client: u64,
        tab: TabId,
        path: &Path,
    ) -> Result<PathBuf, String> {
        let Some(tab) = self.get_tab(client, tab)? else {
            return Err("this workspace tab is closed".to_owned());
        };
        if path.as_os_str().is_empty() {
            return Err("empty path".to_owned());
        }
        let root = std::fs::canonicalize(tab_root(&tab))
            .map_err(|e| format!("cannot locate workspace: {e}"))?;
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        let resolved = std::fs::canonicalize(&candidate)
            .map_err(|e| format!("cannot locate {}: {e}", path.display()))?;
        if !resolved.starts_with(&root) {
            return Err(format!("{} is outside this workspace", path.display()));
        }
        Ok(resolved)
    }

    /// Validate an image pasted from the clipboard (format + 3 MB cap) and stage it as a
    /// temporary file so the engine's `InputItem::Image` can load it natively.
    pub fn save_pasted_image_for(
        &self,
        client: u64,
        tab: TabId,
        bytes: &[u8],
    ) -> Result<String, String> {
        if self.get_tab(client, tab)?.is_none() {
            return Err("this workspace tab is closed".to_owned());
        }
        let image = quack_protocol::image::Image::from_bytes(bytes)?;
        let ext = match image.media_type.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            other => return Err(format!("unsupported image type: {other}")),
        };
        static NEXT_PASTE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let seq = NEXT_PASTE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join("quack-gui-paste");
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create paste folder: {e}"))?;
        let file = dir.join(format!("paste-{}-{stamp}-{seq}.{ext}", std::process::id()));
        std::fs::write(&file, bytes).map_err(|e| format!("cannot stage pasted image: {e}"))?;
        Ok(file.display().to_string())
    }

    /// Close a tab and await engine shutdown. The reservation remains until done.
    pub async fn close(&self, tab: TabId) {
        let client = self.tabs.lock().expect("tab map lock").client;
        let _ = self.close_for(client, tab).await;
    }

    pub async fn close_for(&self, client: u64, tab: TabId) -> Result<(), String> {
        if let Some(tab) = self.get_tab(client, tab)? {
            tab.stop();
            tab.stopped().await;
        }
        Ok(())
    }

    /// Invalidate opens and signal every task before waiting for any shutdown.
    fn retire(&self) -> (u64, Vec<Arc<Tab>>) {
        let mut tabs = self.tabs.lock().expect("tab map lock");
        tabs.client += 1;
        tabs.entries.retain(|_, tab| !tab.finished());
        let closing: Vec<_> = tabs.entries.values().cloned().collect();
        for tab in &closing {
            tab.stop();
        }
        (tabs.client, closing)
    }

    pub async fn close_all(&self) {
        let (_, tabs) = self.retire();
        for tab in tabs {
            tab.stopped().await;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tabs
            .lock()
            .expect("tab map lock")
            .entries
            .values()
            .all(|tab| tab.finished())
    }
}

fn tab_root(tab: &Tab) -> PathBuf {
    if tab.root.is_dir() {
        tab.root.clone()
    } else {
        tab.cwd.clone()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        // Dropping a bridge must not leave detached engines waiting on owned senders.
        for tab in self.tabs.get_mut().expect("tab map lock").entries.values() {
            tab.stop();
        }
    }
}

fn spawn_tab(
    id: TabId,
    loaded: Arc<quack_config::Loaded>,
    resume: Resume,
    sink: Arc<dyn Sink>,
    cwd: PathBuf,
    root: PathBuf,
    workspace_key: PathBuf,
) -> (Arc<Tab>, oneshot::Receiver<Result<(), String>>) {
    let (ops, ops_rx) = mpsc::channel(16);
    let (stop, stop_rx) = watch::channel(false);
    let (done, done_rx) = watch::channel(false);
    let (session, session_rx) = watch::channel(None);
    let (started, started_rx) = oneshot::channel();
    let history_path = Arc::new(std::sync::RwLock::new(quack_core::history_path(&loaded)));
    let tab = Arc::new(Tab {
        session: session_rx,
        cwd,
        root,
        workspace_key,
        history_path: Arc::clone(&history_path),
        ops,
        stop,
        done: done_rx,
    });
    let rt = TabRuntime {
        id,
        sink,
        session,
        history_path,
    };
    tokio::spawn(async move {
        let engine = Engine::spawn_with(loaded, resume);
        run_tab(rt, engine, ops_rx, stop_rx, started).await;
        done.send_replace(true);
    });
    (tab, started_rx)
}

struct TabRuntime {
    id: TabId,
    sink: Arc<dyn Sink>,
    session: watch::Sender<Option<SessionId>>,
    history_path: Arc<std::sync::RwLock<PathBuf>>,
}

fn read_history_path(lock: &std::sync::RwLock<PathBuf>) -> PathBuf {
    lock.read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn user_turn_prompt(op: &Op) -> Option<String> {
    let Op::UserTurn { items, .. } = op else {
        return None;
    };
    let text: String = items
        .iter()
        .filter_map(|item| match item {
            InputItem::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then_some(text)
}

/// Return no watch borrow, so callers can select on this more than once.
async fn stopping(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|stop| *stop).await;
}

/// The task owns startup and shutdown, even if an IPC command is cancelled.
async fn run_tab(
    rt: TabRuntime,
    mut engine: EngineHandle,
    mut ops: mpsc::Receiver<Op>,
    mut stop: watch::Receiver<bool>,
    started: oneshot::Sender<Result<(), String>>,
) {
    let mut startup = Vec::new();
    let opening = tokio::time::timeout(OPEN_TIMEOUT, async {
        loop {
            tokio::select! {
                biased;
                _ = stopping(&mut stop) => return Err("the tab closed while opening".to_owned()),
                event = engine.events.recv() => match event {
                    Ok(event) => {
                        if let Event::SessionConfigured { session: info } = &event {
                            rt.session.send_replace(Some(info.id.clone()));
                            startup.push(event);
                            return Ok(());
                        }
                        startup.push(event);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => return Err(format!("lost {n} startup events · reopen the folder")),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return Err("the engine stopped".to_owned()),
                }
            }
        }
    }).await.unwrap_or_else(|_| Err("the session did not open within a minute".to_owned()));
    if let Err(error) = opening {
        engine.shutdown().await;
        let _ = started.send(Err(error));
        return;
    }
    rt.sink.send(TabMessage::info(
        rt.id,
        AppInfo::from_config(&engine.config()),
    ));
    for event in startup {
        rt.sink.send(TabMessage::event(rt.id, event));
    }
    if started.send(Ok(())).is_err() {
        engine.shutdown().await;
        rt.sink.send(TabMessage::closed(
            rt.id,
            Some("the open request was cancelled".to_owned()),
        ));
        return;
    }
    let mut coalescer = DeltaCoalescer::default();
    let mut tick = tokio::time::interval(FLUSH_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut reason = None;
    loop {
        tokio::select! {
            _ = stopping(&mut stop) => break,
            _ = tick.tick() => {
                for event in coalescer.flush() { rt.sink.send(TabMessage::event(rt.id, event)); }
            }
            op = ops.recv() => match op {
                Some(Op::Shutdown) | None => break,
                Some(op) => {
                    let prompt = user_turn_prompt(&op);
                    // A full engine queue must not prevent close/quit/reconnect.
                    let accepted = tokio::select! {
                        biased;
                        _ = stopping(&mut stop) => break,
                        accepted = engine.submit(op) => accepted,
                    };
                    if !accepted {
                        break;
                    }
                    if let Some(prompt) = prompt {
                        let path = read_history_path(&rt.history_path);
                        let history = tokio::select! {
                            biased;
                            _ = stopping(&mut stop) => break,
                            res = tokio::task::spawn_blocking(move || {
                                if let Err(e) = quack_core::append_history(&path, &prompt) {
                                    tracing::warn!(%e, path = %path.display(), "cannot append input history");
                                }
                                quack_core::load_history(&path)
                            }) => res,
                        };
                        if let Ok(history) = history {
                            rt.sink.send(TabMessage::history(rt.id, history));
                        }
                    }
                }
            },
            event = engine.events.recv() => match event {
                Ok(event) => {
                    if let Event::SessionConfigured { session: info }
                        | Event::SessionResumed { session: info, .. } = &event {
                        rt.session.send_replace(Some(info.id.clone()));
                    }
                    if matches!(event, Event::StartupComplete) {
                        let cfg = engine.config();
                        *rt.history_path
                            .write()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                            quack_core::history_path(&cfg);
                        rt.sink.send(TabMessage::info(rt.id, AppInfo::from_config(&cfg)));
                    }
                    if coalescer.on(&event) {
                        for merged in coalescer.flush() { rt.sink.send(TabMessage::event(rt.id, merged)); }
                        rt.sink.send(TabMessage::event(rt.id, event));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    reason = Some(format!("Lost {n} engine events · reopen the folder to restore the session"));
                    break;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }
    for merged in coalescer.flush() {
        rt.sink.send(TabMessage::event(rt.id, merged));
    }
    engine.shutdown().await;
    rt.sink.send(TabMessage::closed(rt.id, reason));
}

/// Keep native resource resolution local to this client, with no process-wide
/// environment mutation and the same override/reload behavior as other clients.
fn load_from_resources(
    opts: &LoadOptions,
    resource_dir: &Path,
    env: quack_config::Env<'_>,
) -> Result<quack_config::Loaded, String> {
    let defaults = resource_dir.join("defaults");
    quack_config::load_with(
        &|key| {
            env(key).filter(|value| !value.is_empty()).or_else(|| {
                (key == "QUACK_DEFAULTS_DIR" && defaults.is_dir())
                    .then(|| defaults.clone().into_os_string())
            })
        },
        opts,
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_resources_load_without_overriding_an_explicit_defaults_choice() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let resources = tmp.path().join("resources");
        let defaults = resources.join("defaults");
        std::fs::create_dir_all(&defaults).expect("defaults");
        std::fs::write(defaults.join("config.toml"), "[ui]\ntheme = \"dark\"\n")
            .expect("default settings");
        let opts = LoadOptions::new(tmp.path().join("workspace"));
        let vars = |key: &str| match key {
            "QUACK_CONFIG_DIR" => Some(tmp.path().join("user").into_os_string()),
            "QUACK_SYSTEM_DIR" => Some(tmp.path().join("system").into_os_string()),
            "QUACK_DATA_DIR" => Some(tmp.path().join("data").into_os_string()),
            "QUACK_CACHE_DIR" => Some(tmp.path().join("cache").into_os_string()),
            _ => None,
        };
        let loaded = load_from_resources(&opts, &resources, &vars).expect("load resources");
        assert_eq!(loaded.dirs.defaults, Some(defaults));
        assert_eq!(loaded.settings.ui.theme, "dark");
        let disabled = load_from_resources(&opts, &resources, &|key| {
            if key == "QUACK_DEFAULTS_DIR" {
                Some("none".into())
            } else {
                vars(key)
            }
        })
        .expect("load without defaults");
        assert_eq!(disabled.dirs.defaults, None);
        assert_eq!(
            loaded.reload().expect("reload resources").settings.ui.theme,
            "dark"
        );
    }

    #[test]
    fn expand_tilde_resolves_home_relative_paths() {
        let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let fake_home = if cfg!(windows) {
            "C:\\Users\\duck"
        } else {
            "/home/duck"
        };
        let env = |key: &str| (key == home_var).then(|| std::ffi::OsString::from(fake_home));
        assert_eq!(
            expand_tilde_with(PathBuf::from("~/pond/notes"), &env),
            PathBuf::from(fake_home).join("pond/notes")
        );
        assert_eq!(
            expand_tilde_with(PathBuf::from("~"), &env),
            PathBuf::from(fake_home)
        );
        assert_eq!(
            expand_tilde_with(PathBuf::from("~other/pond"), &env),
            PathBuf::from("~other/pond")
        );
    }

    #[tokio::test]
    async fn open_rejects_empty_or_missing_directories() {
        let registry = Registry::new();
        let sink: Arc<dyn Sink> = Arc::new(|_| {});
        let err = registry
            .open(PathBuf::new(), Resume::New, sink.clone())
            .await
            .expect_err("empty cwd must fail");
        assert!(matches!(err, OpenError::Config(_)), "{err:?}");
        let missing = std::env::temp_dir().join("quack-gui-does-not-exist-99999");
        let err = registry
            .open(missing, Resume::New, sink)
            .await
            .expect_err("missing dir must fail");
        assert!(matches!(err, OpenError::Config(_)), "{err:?}");
    }

    #[test]
    fn startup_targets_merges_and_persists_isolated_tabs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ws1 = dir.path().join("ws1");
        let ws2 = dir.path().join("ws2");
        std::fs::create_dir_all(&ws1).expect("ws1");
        std::fs::create_dir_all(&ws2).expect("ws2");
        let data = dir.path().join("data");
        let load: Loader = Arc::new(move |opts| {
            let vars = |k: &str| match k {
                "QUACK_DEFAULTS_DIR" => Some("none".into()),
                "QUACK_CONFIG_DIR" | "QUACK_SYSTEM_DIR" | "QUACK_DATA_DIR" | "QUACK_CACHE_DIR" => {
                    Some(data.clone().into_os_string())
                }
                _ => None,
            };
            quack_config::load_with(&vars, &opts).map_err(|e| e.to_string())
        });
        let reg = Registry::with_parts(load, dir.path().join("gui"));
        let client = reg.tabs.lock().expect("lock").client;
        reg.save_open_tabs_for(
            client,
            &[SavedTab {
                path: ws1.display().to_string(),
                session_id: Some("sess1".into()),
                active: true,
            }],
        )
        .expect("save");
        reg.set_initial_argv(
            &["quack-gui".into(), ws2.display().to_string()],
            Some(dir.path()),
        );
        let first = reg.startup_targets_for(client).expect("startup");
        assert_eq!(first.targets.len(), 2);
        assert_eq!(first.active_index, Some(1));
        assert!(first.targets[1].focus);

        // Second call (e.g. webview reload) uses saved tabs without replaying initial argv.
        let second = reg.startup_targets_for(client).expect("reload");
        assert_eq!(second.targets.len(), 1);
        assert_eq!(second.targets[0].path, ws1.display().to_string());
    }
}
