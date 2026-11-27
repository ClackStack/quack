//! One client per (server id, project root), started the first time a covered file is
//! touched. Clients that crash restart up to [`MAX_RESTARTS`] times.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quack_config::LspServerFile;
use quack_sandbox::Command;
use serde_json::Value;
use tokio::sync::{OnceCell, watch};

use crate::client::{Client, Launch, WatchChangeType};
use crate::diag::{self, Diagnostic, Limits, Severity, Snapshot};

pub const MAX_RESTARTS: u32 = 3;
/// How long a tool waits for a server that is still starting.
pub const TOOL_WAIT: Duration = Duration::from_secs(20);

/// A server file and whether its program was found.
#[derive(Debug, Clone)]
pub struct ServerSpec {
    pub id: String,
    pub file: LspServerFile,
    /// The program's full path, or why it can't be used.
    pub program: Result<PathBuf, String>,
    env: BTreeMap<String, OsString>,
}

impl ServerSpec {
    /// Find the program and resolve the environment. Runs `cmd` secrets, so call it off
    /// the async runtime.
    pub fn resolve(id: String, file: LspServerFile, root: &Path) -> Self {
        let get = |k: &str| std::env::var_os(k);
        let mut env = quack_shell::inherited_env(None);
        let mut problem = None;
        for (k, secret) in &file.env {
            match secret.resolve(&get) {
                Ok(v) => {
                    env.insert(k.clone(), v.into());
                }
                Err(e) => problem = Some(format!("env `{k}`: {e}")),
            }
        }
        let search = env.get("PATH").cloned();
        let program = match problem {
            Some(p) => Err(p),
            None => quack_shell::resolve_program(&file.command, search.as_deref(), root)
                .map_err(|_| format!("`{}` is not installed (not on PATH)", file.command)),
        };
        Self {
            id,
            file,
            program,
            env,
        }
    }

    pub fn installed(&self) -> bool {
        self.program.is_ok()
    }
}

/// Builds a server's command from its program and arguments, e.g. inside the OS sandbox.
/// `Err` refuses to start it.
pub type Prepare =
    Arc<dyn Fn(&ServerSpec, &Path, &[String]) -> Result<Command, String> + Send + Sync>;

type Started = Result<Arc<Client>, String>;
type Cell = Arc<OnceCell<Started>>;

struct Slot {
    cell: Cell,
    restarts: u32,
}

/// What `/lsp` shows for one client or one unused server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticEntry {
    pub server: String,
    pub path: PathBuf,
    pub diagnostic: Diagnostic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub id: String,
    pub command: String,
    pub extensions: Vec<String>,
    /// `not installed`, `idle`, `starting`, `ready`, `indexing`, `failed` or `exited`.
    pub state: String,
    pub detail: String,
    pub root: Option<PathBuf>,
    pub errors: usize,
    pub warnings: usize,
    pub log: Vec<String>,
}

pub struct LspManager {
    specs: Vec<ServerSpec>,
    root: PathBuf,
    slots: Mutex<HashMap<(usize, PathBuf), Slot>>,
    changed: Arc<watch::Sender<u64>>,
    prepare: Prepare,
}

impl std::fmt::Debug for LspManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LspManager")
            .field("specs", &self.specs)
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl LspManager {
    pub fn new(specs: Vec<ServerSpec>, root: PathBuf, prepare: Option<Prepare>) -> Arc<Self> {
        Arc::new(Self {
            specs,
            root,
            slots: Mutex::default(),
            changed: Arc::new(watch::channel(0).0),
            prepare: prepare.unwrap_or_else(|| {
                Arc::new(|_, program, args| {
                    Ok(quack_sandbox::unsandboxed(program.as_os_str(), args))
                })
            }),
        })
    }

    pub fn empty() -> Arc<Self> {
        Self::new(Vec::new(), PathBuf::new(), None)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Bumped when a server starts, stops, or reports diagnostics or progress.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    pub fn specs(&self) -> &[ServerSpec] {
        &self.specs
    }

    /// Installed servers, for the tool description: `(id, extensions)`.
    pub fn installed(&self) -> Vec<(&str, &[String])> {
        self.specs
            .iter()
            .filter(|s| s.installed())
            .map(|s| (s.id.as_str(), s.file.extensions.as_slice()))
            .collect()
    }

    /// The installed server covering this file.
    fn spec_for(&self, path: &Path) -> Option<usize> {
        let ext = path.extension()?.to_string_lossy();
        self.specs
            .iter()
            .position(|s| s.installed() && s.file.covers(&ext))
    }

    pub fn covers(&self, path: &Path) -> bool {
        self.spec_for(path).is_some()
    }

    /// The nearest folder at or above the file holding one of the server's root markers,
    /// not going above the workspace root; else the workspace root.
    ///
    /// For Cargo projects (`Cargo.toml` marker), if a nested member crate's `Cargo.toml`
    /// does not declare `[workspace]`, continue walking up to `self.root` for the nearest
    /// enclosing Cargo workspace root so one `rust-analyzer` instance serves the whole
    /// workspace instead of spawning one per member crate.
    fn project_root(&self, spec: &ServerSpec, path: &Path) -> PathBuf {
        let markers = &spec.file.root_markers;
        let inside = path.starts_with(&self.root);
        let wants_cargo = markers.iter().any(|m| m == "Cargo.toml");
        let mut dir = path.parent();
        let mut first_match: Option<PathBuf> = None;
        while let Some(d) = dir {
            if !markers.is_empty() && has_marker(d, markers) {
                if !wants_cargo || has_non_cargo_marker(d, markers) || is_cargo_workspace_root(d) {
                    return d.to_path_buf();
                }
                if first_match.is_none() {
                    first_match = Some(d.to_path_buf());
                }
            }
            if inside && d == self.root {
                break;
            }
            dir = d.parent();
        }
        if let Some(root) = first_match {
            return root;
        }
        if inside {
            self.root.clone()
        } else {
            path.parent()
                .map_or_else(|| self.root.clone(), Path::to_path_buf)
        }
    }

    /// The client for a file, starting it if needed. `Ok(None)` when no installed server
    /// covers the file.
    pub async fn client(&self, path: &Path) -> Result<Option<Arc<Client>>, String> {
        let Some(idx) = self.spec_for(path) else {
            return Ok(None);
        };
        let root = self.project_root(&self.specs[idx], path);
        self.get(idx, root).await.map(Some)
    }

    /// The client for a file if it is already running; never starts one.
    pub fn running(&self, path: &Path) -> Option<Arc<Client>> {
        let idx = self.spec_for(path)?;
        let root = self.project_root(&self.specs[idx], path);
        let slots = self.slots.lock().ok()?;
        match slots.get(&(idx, root))?.cell.get() {
            Some(Ok(c)) if c.alive() => Some(c.clone()),
            _ => None,
        }
    }

    /// Every running client.
    pub fn clients(&self) -> Vec<Arc<Client>> {
        let Ok(slots) = self.slots.lock() else {
            return Vec::new();
        };
        slots
            .values()
            .filter_map(|s| match s.cell.get() {
                Some(Ok(c)) if c.alive() => Some(c.clone()),
                _ => None,
            })
            .collect()
    }

    async fn get(&self, idx: usize, root: PathBuf) -> Result<Arc<Client>, String> {
        loop {
            let cell = {
                let mut slots = self.slots.lock().map_err(|e| e.to_string())?;
                slots
                    .entry((idx, root.clone()))
                    .or_insert_with(|| Slot {
                        cell: Arc::default(),
                        restarts: 0,
                    })
                    .cell
                    .clone()
            };
            let result = cell
                .get_or_init(|| async {
                    self.bump();
                    let r = self.start(idx, &root).await;
                    if let Err(e) = &r {
                        tracing::warn!(server = %self.specs[idx].id, error = %e, "language server did not start");
                    }
                    r
                })
                .await
                .clone();
            self.bump();
            let client = result?;
            if client.alive() {
                return Ok(client);
            }
            // It exited: start a new one, unless it keeps crashing.
            let restarts = {
                let mut slots = self.slots.lock().map_err(|e| e.to_string())?;
                let slot = slots
                    .get_mut(&(idx, root.clone()))
                    .ok_or("the server was removed")?;
                if Arc::ptr_eq(&slot.cell, &cell) {
                    if slot.restarts >= MAX_RESTARTS {
                        return Err(format!(
                            "{} exited {} times; /lsp shows its log",
                            self.specs[idx].id,
                            slot.restarts + 1
                        ));
                    }
                    slot.restarts += 1;
                    slot.cell = Arc::default();
                }
                slot.restarts
            };
            tokio::time::sleep(Duration::from_millis(250 * u64::from(restarts))).await;
        }
    }

    async fn start(&self, idx: usize, root: &Path) -> Result<Arc<Client>, String> {
        let spec = &self.specs[idx];
        let program = spec.program.clone()?;
        let mut cmd = (self.prepare)(spec, &program, &spec.file.args)?;
        cmd.current_dir(root).env_clear().envs(&spec.env);
        let file = spec.file.clone();
        let launch = Launch {
            id: spec.id.clone(),
            command: cmd,
            root: root.to_path_buf(),
            initialization_options: toml_to_json(&spec.file.initialization_options),
            settings: toml_to_json(&spec.file.settings),
            language_ids: Box::new(move |ext| file.language_id(ext)),
        };
        Client::start(launch, self.changed.clone()).await
    }

    fn bump(&self) {
        self.changed.send_modify(|n| *n += 1);
    }

    /// The file was read: start its server in the background and open the file there,
    /// so the server is warm by the time the file is edited.
    pub fn touch(self: &Arc<Self>, path: &Path) {
        if !self.covers(path) {
            return;
        }
        let this = self.clone();
        let path = path.to_path_buf();
        tokio::spawn(async move {
            if let Ok(Some(c)) = this.client(&path).await {
                let _ = c.sync(&path).await;
            }
        });
    }

    /// Diagnostics before an edit, to compare with afterwards. `None` when no server
    /// covers the file.
    pub async fn before_edit(&self, path: &Path) -> Option<Snapshot> {
        self.spec_for(path)?;
        let Some(c) = self.running(path) else {
            return Some(Snapshot::new());
        };
        // Pick up changes made outside the tools (shell commands, the user).
        let _ = c.sync(path).await;
        Some(c.snapshot())
    }

    /// After an edit: send the new text, wait for diagnostics to settle, and describe
    /// what is new compared with `before`.
    pub async fn after_edit(
        &self,
        path: &Path,
        before: &Snapshot,
        display: &(dyn Fn(&Path) -> String + Sync),
    ) -> Option<String> {
        let idx = self.spec_for(path)?;
        let d = &self.specs[idx].file.diagnostics;
        let max_wait = Duration::from_millis(d.max_wait_ms);
        let client = match tokio::time::timeout(max_wait, self.client(path)).await {
            Ok(Ok(Some(c))) => c,
            Ok(Err(e)) => return Some(format!("diagnostics unavailable: {e}")),
            Ok(Ok(None)) => return None,
            Err(_) => {
                return Some(format!(
                    "diagnostics unavailable: {} is still starting",
                    self.specs[idx].id
                ));
            }
        };
        if let Err(e) = client
            .changed(path, Duration::from_millis(d.settle_ms), max_wait)
            .await
        {
            return Some(format!("diagnostics unavailable: {e}"));
        }
        let limits = Limits {
            min_severity: Severity::parse(&d.min_severity),
            max_items: d.max_items.max(1),
        };
        let mut text = diag::report(path, before, &client.snapshot(), limits, display);
        let busy = client.progress();
        if !busy.is_empty() {
            let note = format!(
                "(the language server is still busy: {}; more diagnostics may follow — \
                 the lsp tool's diagnostics operation shows them later)",
                busy.join(", ")
            );
            text = Some(match text {
                Some(t) => format!("{t}\n{note}"),
                None => note,
            });
        }
        text
    }

    /// For `/lsp`: each running client, then each server with none running.
    pub fn status(&self) -> Vec<Status> {
        let slots: Vec<(usize, PathBuf, Option<Started>, u32)> = self
            .slots
            .lock()
            .map(|s| {
                s.iter()
                    .map(|((i, r), slot)| (*i, r.clone(), slot.cell.get().cloned(), slot.restarts))
                    .collect()
            })
            .unwrap_or_default();
        let mut out = Vec::new();
        for (idx, spec) in self.specs.iter().enumerate() {
            let base = Status {
                id: spec.id.clone(),
                command: std::iter::once(spec.file.command.as_str())
                    .chain(spec.file.args.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join(" "),
                extensions: spec.file.extensions.clone(),
                state: String::new(),
                detail: spec.file.description.clone(),
                root: None,
                errors: 0,
                warnings: 0,
                log: Vec::new(),
            };
            let mine: Vec<_> = slots.iter().filter(|s| s.0 == idx).collect();
            if mine.is_empty() {
                out.push(Status {
                    state: if spec.installed() {
                        "idle"
                    } else {
                        "not installed"
                    }
                    .into(),
                    detail: match &spec.program {
                        Ok(p) => p.display().to_string(),
                        Err(e) => e.clone(),
                    },
                    ..base
                });
                continue;
            }
            for (_, root, cell, restarts) in mine {
                let mut s = Status {
                    root: Some(root.clone()),
                    ..base.clone()
                };
                match cell {
                    None => s.state = "starting".into(),
                    Some(Err(e)) => {
                        s.state = "failed".into();
                        s.detail.clone_from(e);
                    }
                    Some(Ok(c)) => {
                        let busy = c.progress();
                        s.state = if !c.alive() {
                            "exited"
                        } else if busy.is_empty() {
                            "ready"
                        } else {
                            "indexing"
                        }
                        .into();
                        if !busy.is_empty() {
                            s.detail = busy.join(", ");
                        } else if *restarts > 0 {
                            s.detail = format!("restarted {restarts} time(s)");
                        }
                        for list in c.snapshot().values() {
                            for d in list {
                                match d.severity {
                                    Severity::Error => s.errors += 1,
                                    Severity::Warning => s.warnings += 1,
                                    _ => {}
                                }
                            }
                        }
                        s.log = c.log();
                    }
                }
                out.push(s);
            }
        }
        out
    }

    /// Collect all cached diagnostics across running servers, ordered Error -> Hint.
    pub fn diagnostics(&self) -> Vec<DiagnosticEntry> {
        let cells: Vec<(String, Arc<Client>)> = {
            let Ok(slots) = self.slots.lock() else {
                return Vec::new();
            };
            slots
                .iter()
                .filter_map(|((idx, _), slot)| {
                    let spec = self.specs.get(*idx)?;
                    let client = match slot.cell.get() {
                        Some(Ok(c)) => c.clone(),
                        _ => return None,
                    };
                    Some((spec.id.clone(), client))
                })
                .collect()
        };
        let mut out = Vec::new();
        for (server, client) in cells {
            for (path, list) in client.snapshot() {
                for diagnostic in list {
                    out.push(DiagnosticEntry {
                        server: server.clone(),
                        path: path.clone(),
                        diagnostic,
                    });
                }
            }
        }
        out.sort_by(|a, b| {
            a.diagnostic
                .severity
                .cmp(&b.diagnostic.severity)
                .then_with(|| a.path.cmp(&b.path))
                .then_with(|| a.diagnostic.line.cmp(&b.diagnostic.line))
                .then_with(|| a.diagnostic.column.cmp(&b.diagnostic.column))
        });
        out
    }

    /// Forward external workspace file changes (for example after a mutating `shell`
    /// command) to every running language server covering those paths. Never lazily starts
    /// stopped servers.
    pub async fn notify_watched_files(&self, changes: &[(PathBuf, WatchChangeType)]) {
        if changes.is_empty() {
            return;
        }
        for client in self.clients() {
            let relevant: Vec<(PathBuf, WatchChangeType)> = changes
                .iter()
                .filter(|(p, _)| p.starts_with(&client.root) || p.starts_with(&self.root))
                .cloned()
                .collect();
            if !relevant.is_empty() {
                let _ = client.did_change_watched_files(&relevant).await;
            }
        }
    }

    /// Stop only the running clients whose server id is in `ids`; unchanged servers keep
    /// running and warm.
    pub async fn stop_servers(&self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        let matching_idx: Vec<usize> = self
            .specs
            .iter()
            .enumerate()
            .filter(|(_, s)| ids.iter().any(|id| id == &s.id))
            .map(|(i, _)| i)
            .collect();
        let cells: Vec<Cell> = self
            .slots
            .lock()
            .map(|mut slots| {
                let keys: Vec<_> = slots
                    .keys()
                    .filter(|(idx, _)| matching_idx.contains(idx))
                    .cloned()
                    .collect();
                keys.into_iter()
                    .filter_map(|k| slots.remove(&k).map(|s| s.cell))
                    .collect()
            })
            .unwrap_or_default();
        let mut set = tokio::task::JoinSet::new();
        for c in cells.into_iter().filter_map(|cell| match cell.get() {
            Some(Ok(client)) => Some(client.clone()),
            _ => None,
        }) {
            set.spawn(async move { c.stop().await });
        }
        while set.join_next().await.is_some() {}
        self.bump();
    }

    /// Stop every server.
    pub async fn shutdown(&self) {
        let cells: Vec<Cell> = self
            .slots
            .lock()
            .map(|mut s| s.drain().map(|(_, slot)| slot.cell).collect())
            .unwrap_or_default();
        let stops = cells.into_iter().filter_map(|c| match c.get() {
            Some(Ok(client)) => Some(client.clone()),
            _ => None,
        });
        let stops: Vec<_> = stops.collect();
        let mut set = tokio::task::JoinSet::new();
        for c in stops {
            set.spawn(async move { c.stop().await });
        }
        while set.join_next().await.is_some() {}
        self.bump();
    }
}

fn has_marker(dir: &Path, markers: &[String]) -> bool {
    markers.iter().any(|m| {
        if m.contains('*') {
            std::fs::read_dir(dir).is_ok_and(|entries| {
                entries
                    .filter_map(Result::ok)
                    .any(|e| quack_config::glob_match(m, &e.file_name().to_string_lossy()))
            })
        } else {
            dir.join(m).exists()
        }
    })
}

fn has_non_cargo_marker(dir: &Path, markers: &[String]) -> bool {
    markers
        .iter()
        .filter(|m| m.as_str() != "Cargo.toml")
        .any(|m| {
            if m.contains('*') {
                std::fs::read_dir(dir).is_ok_and(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .any(|e| quack_config::glob_match(m, &e.file_name().to_string_lossy()))
                })
            } else {
                dir.join(m).exists()
            }
        })
}

fn is_cargo_workspace_root(dir: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
        return false;
    };
    text.parse::<toml::Table>()
        .is_ok_and(|table| table.contains_key("workspace"))
}

fn toml_to_json(t: &toml::Table) -> Value {
    if t.is_empty() {
        return Value::Null;
    }
    serde_json::to_value(t).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rust_spec() -> ServerSpec {
        let file: LspServerFile = toml::from_str(
            r#"
            command = "rust-analyzer"
            extensions = ["rs"]
            root_markers = ["Cargo.toml", "rust-project.json"]
            "#,
        )
        .expect("lsp server file");
        ServerSpec {
            id: "rust".into(),
            file,
            program: Ok(PathBuf::from("rust-analyzer")),
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn cargo_member_resolves_to_enclosing_workspace_root() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/a\", \"crates/b\"]\n",
        )
        .expect("root cargo");
        let a_src = root.join("crates/a/src");
        let b_src = root.join("crates/b/src");
        std::fs::create_dir_all(&a_src).expect("mkdir a");
        std::fs::create_dir_all(&b_src).expect("mkdir b");
        std::fs::write(
            root.join("crates/a/Cargo.toml"),
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\n",
        )
        .expect("a cargo");
        std::fs::write(
            root.join("crates/b/Cargo.toml"),
            "[package]\nname = \"b\"\nversion = \"0.1.0\"\n",
        )
        .expect("b cargo");

        let spec = rust_spec();
        let mgr = LspManager::new(vec![spec.clone()], root.to_path_buf(), None);
        assert_eq!(mgr.project_root(&spec, &a_src.join("lib.rs")), root);
        assert_eq!(mgr.project_root(&spec, &b_src.join("lib.rs")), root);
    }

    #[test]
    fn nested_standalone_workspace_and_single_crate_keep_their_own_roots() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/a\"]\nexclude = [\"tools/sub\"]\n",
        )
        .expect("root cargo");
        let sub_src = root.join("tools/sub/crates/x/src");
        std::fs::create_dir_all(&sub_src).expect("mkdir sub");
        std::fs::write(
            root.join("tools/sub/Cargo.toml"),
            "[workspace]\nmembers = [\"crates/x\"]\n",
        )
        .expect("sub workspace");
        std::fs::write(
            root.join("tools/sub/crates/x/Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
        )
        .expect("x cargo");

        let spec = rust_spec();
        let mgr = LspManager::new(vec![spec.clone()], root.to_path_buf(), None);
        assert_eq!(
            mgr.project_root(&spec, &sub_src.join("lib.rs")),
            root.join("tools/sub")
        );

        // Single crate with no enclosing `[workspace]` keeps its crate directory.
        let single_tmp = tempfile::tempdir().expect("single");
        let crate_dir = single_tmp.path().join("app");
        std::fs::create_dir_all(crate_dir.join("src")).expect("mkdir app");
        std::fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        )
        .expect("app cargo");
        let single_mgr = LspManager::new(vec![spec.clone()], single_tmp.path().to_path_buf(), None);
        assert_eq!(
            single_mgr.project_root(&spec, &crate_dir.join("src/main.rs")),
            crate_dir
        );
    }
}
