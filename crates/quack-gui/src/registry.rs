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
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use quack_config::LoadOptions;
use quack_core::{Engine, EngineHandle, Resume};
use quack_protocol::{Event, Op, SessionId};
use serde::Serialize;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::appinfo::AppInfo;
use crate::coalesce::DeltaCoalescer;
use crate::recent::RecentFolders;

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
    Event { event: Event },
    Info { info: AppInfo },
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
            payload: Payload::Event { event },
        }
    }

    pub fn info(tab: TabId, info: AppInfo) -> Self {
        Self {
            tab,
            payload: Payload::Info { info },
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

/// One open tab: the ops half of its engine, a stop signal for the forwarder, and the
/// forwarder itself (which owns the engine and awaits its shutdown at the end).
struct Tab {
    session: watch::Receiver<SessionId>,
    cwd: PathBuf,
    workspace: PathBuf,
    ops: mpsc::Sender<Op>,
    stop: mpsc::UnboundedSender<()>,
    forwarder: JoinHandle<()>,
}

/// The open tabs, one engine each.
pub struct Registry {
    tabs: HashMap<TabId, Tab>,
    next_tab: TabId,
    load: Loader,
    recent: RecentFolders,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("tabs", &self.tabs.keys().collect::<Vec<_>>())
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

    /// A registry with a custom config loader (tests).
    pub fn with_loader(load: Loader) -> Self {
        Self {
            tabs: HashMap::new(),
            next_tab: 0,
            load,
            recent: RecentFolders::load(),
        }
    }

    /// A registry with a custom loader *and* a custom home for the recent-folders
    /// file, so tests never read or write the developer's real user data dir.
    pub fn with_parts(load: Loader, recent_dir: PathBuf) -> Self {
        Self {
            tabs: HashMap::new(),
            next_tab: 0,
            load,
            recent: RecentFolders::at(recent_dir),
        }
    }

    /// Recently opened folders, most recent first.
    pub fn recent_folders(&self, limit: usize) -> Vec<crate::recent::RecentFolder> {
        self.recent.list(limit)
    }

    /// Load `cwd`'s config, spawn its engine, and stream its events into `sink` from
    /// now on. The first [`Payload::Info`] message carries the `AppInfo` snapshot; the
    /// events seen while opening replay first.
    pub async fn open(
        &mut self,
        cwd: PathBuf,
        resume: Resume,
        sink: Arc<dyn Sink>,
    ) -> Result<TabId, OpenError> {
        self.tabs.retain(|_, t| !t.forwarder.is_finished());
        let opts = LoadOptions::new(cwd.clone());
        let load = self.load.clone();
        let loaded = tokio::task::spawn_blocking(move || load(opts))
            .await
            .map_err(|e| OpenError::Config(e.to_string()))?
            .map_err(OpenError::Config)?;
        // One live engine per workspace: session switching must never create two
        // writers to the same log (or competing edits to the same working tree).
        // Check BEFORE spawning: resume appends a record as part of startup.
        let workspace = workspace_key(&loaded.dirs.workspace_root);
        if let Some((&tab, _)) = self.tabs.iter().find(|(_, t)| t.workspace == workspace) {
            return Err(OpenError::AlreadyOpen { tab });
        }
        let mut engine = Engine::spawn_with(Arc::new(loaded), resume);
        let mut startup = Vec::new();
        // The initial session is always announced, even when no model is ready.
        // Forward resume/replay asynchronously; do not wait for a warning's wording
        // or for a second SessionConfigured that may never arrive.
        let opened = tokio::time::timeout(OPEN_TIMEOUT, async {
            loop {
                match engine.events.recv().await {
                    Ok(ev) => {
                        if let Event::SessionConfigured { session } = &ev {
                            let session = session.clone();
                            startup.push(ev);
                            break Ok(session);
                        }
                        startup.push(ev);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        break Err(format!("lost {n} startup events · reopen the folder"));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break Err("the engine stopped".to_owned());
                    }
                }
            }
        })
        .await
        .unwrap_or_else(|_| Err("the session did not open within a minute".to_owned()));
        let session = match opened {
            Ok(session) => session,
            Err(e) => {
                engine.shutdown().await;
                return Err(OpenError::Engine(e));
            }
        };
        if let Some(existing) = self.tab_of(&session.id) {
            engine.shutdown().await;
            return Err(OpenError::AlreadyOpen { tab: existing });
        }
        let tab = self.next_tab;
        self.next_tab += 1;
        sink.send(TabMessage::info(
            tab,
            AppInfo::from_config(&engine.config()),
        ));
        let (session_tx, session_rx) = watch::channel(session.id);
        let (forwarder, ops, stop) = spawn_forwarder(tab, engine, sink, startup, session_tx);
        self.tabs.insert(
            tab,
            Tab {
                session: session_rx,
                cwd: cwd.clone(),
                workspace,
                ops,
                stop,
                forwarder,
            },
        );
        self.recent.touch(cwd);
        Ok(tab)
    }

    /// Send a serialized [`Op`] to a tab's engine; `Err` says why not, as a message for
    /// the webview.
    pub async fn send_op(&self, tab: TabId, op: serde_json::Value) -> Result<(), String> {
        let op: Op = serde_json::from_value(op)
            .map_err(|e| format!("not a protocol op: {e} · regenerate protocol.ts"))?;
        let ops = self
            .tabs
            .get(&tab)
            .map(|t| t.ops.clone())
            .ok_or_else(|| format!("no open tab {tab}"))?;
        ops.send(op)
            .await
            .map_err(|_| format!("tab {tab} has stopped"))?;
        Ok(())
    }

    /// The tab showing `session`, to refuse a second engine on the same JSONL log.
    pub fn tab_of(&self, session: &SessionId) -> Option<TabId> {
        self.tabs
            .iter()
            .find(|(_, t)| *t.session.borrow() == *session)
            .map(|(tab, _)| *tab)
    }

    /// The folder a tab was opened on.
    pub fn cwd(&self, tab: TabId) -> Option<&PathBuf> {
        self.tabs.get(&tab).map(|t| &t.cwd)
    }

    /// Close a tab: stop its forwarder, which flushes pending deltas and awaits
    /// `engine.shutdown()` (MCP/LSP servers and `session_end` hooks included)
    /// before announcing [`Payload::Closed`].
    pub async fn close(&mut self, tab: TabId) {
        if let Some(t) = self.tabs.remove(&tab) {
            drop(t.ops);
            drop(t.stop);
            let _ = t.forwarder.await;
        }
    }

    /// Close every tab, awaiting each engine's shutdown; the app quit path calls this.
    pub async fn close_all(&mut self) {
        let ids: Vec<TabId> = self.tabs.keys().copied().collect();
        for tab in ids {
            self.close(tab).await;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

/// Own the engine, replay the events seen while opening, and stream coalesced events
/// into the sink until the tab is closed or the engine stops. Returns the forwarder
/// task, a clone of its ops channel, and the stop signal.
fn spawn_forwarder(
    tab: TabId,
    mut engine: EngineHandle,
    sink: Arc<dyn Sink>,
    startup: Vec<Event>,
    session: watch::Sender<SessionId>,
) -> (JoinHandle<()>, mpsc::Sender<Op>, mpsc::UnboundedSender<()>) {
    let (ops, mut ops_rx) = mpsc::channel::<Op>(16);
    let (stop_tx, mut stop_rx) = mpsc::unbounded_channel::<()>();
    let task = tokio::spawn(async move {
        let mut coalescer = DeltaCoalescer::default();
        for ev in startup {
            sink.send(TabMessage::event(tab, ev));
        }
        let mut tick = tokio::time::interval(FLUSH_TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    for ev in coalescer.flush() {
                        sink.send(TabMessage::event(tab, ev));
                    }
                }
                _ = stop_rx.recv() => break,
                op = ops_rx.recv() => match op {
                    Some(op) => {
                        if matches!(op, Op::Shutdown) {
                            break;
                        }
                        let _ = engine.submit(op).await;
                    }
                    None => break,
                },
                ev = engine.events.recv() => match ev {
                    Ok(ev) => {
                        if let Event::SessionConfigured { session: info }
                            | Event::SessionResumed { session: info, .. } = &ev
                        {
                            session.send_replace(info.id.clone());
                        }
                        // StartupComplete also follows Op::ReloadConfig, and by then
                        // engine.config() is the fresh one; ConfigReloaded fires before.
                        if matches!(ev, Event::StartupComplete) {
                            let info = AppInfo::from_config(&engine.config());
                            sink.send(TabMessage::info(tab, info));
                        }
                        if coalescer.on(&ev) {
                            for merged in coalescer.flush() {
                                sink.send(TabMessage::event(tab, merged));
                            }
                            sink.send(TabMessage::event(tab, ev));
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        for merged in coalescer.flush() {
                            sink.send(TabMessage::event(tab, merged));
                        }
                        engine.shutdown().await;
                        sink.send(TabMessage::closed(tab, Some(format!(
                            "Lost {n} engine events · reopen the folder to restore the session"
                        ))));
                        return;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
            }
        }
        for merged in coalescer.flush() {
            sink.send(TabMessage::event(tab, merged));
        }
        engine.shutdown().await;
        sink.send(TabMessage::closed(tab, None));
    });
    (task, ops, stop_tx)
}

/// Resolve symlinks before comparing workspace ownership; compare case-insensitively
/// on the platforms where workspace paths are normally case-insensitive.
fn workspace_key(path: &std::path::Path) -> PathBuf {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if cfg!(any(windows, target_os = "macos")) {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path
    }
}
