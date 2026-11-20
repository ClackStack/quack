//! Scoped config-folder watching (docs/plan/01-configuration.md#hot-reload).
//!
//! Polls the system, user and project config folders off the async threads and hashes
//! each subsystem independently:
//! - Pure-data extensions (`skills/`, `commands/`, `hooks.d/`, `profiles.d/`, `agents.d/`,
//!   `themes/`, `grammars/`) are hot-reloaded automatically by the engine without dropping
//!   session approvals or restarting MCP/LSP servers.
//! - Per-server `mcp.d/<id>.toml` and `lsp.d/<id>.toml` changes identify the exact server
//!   id(s) so only the affected server(s) restart.
//! - Policy/settings/provider edits (`config.toml`, `providers.d/`, `bundles/`) emit a
//!   `/reload` notice so permissions, approvals and sandbox changes stay explicit.

use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use quack_config::Loaded;
use tokio::sync::{mpsc, watch};

const EVERY: Duration = Duration::from_secs(2);

/// Folders inside a config folder that hold no configuration.
const SKIP: &[&str] = &["memory", "worktrees", "schemas"];

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct WatchEvent {
    /// Core config (`config.toml`, `providers.d/`, `bundles/`): prompt `/reload`.
    pub core_changed: bool,
    /// Pure-data extension folders that changed (safe to auto hot-reload).
    pub data_kinds: Vec<&'static str>,
    /// Individual MCP server ids under `mcp.d/` that changed, were added, or were removed.
    pub mcp_changed: Vec<String>,
    /// Individual LSP server ids under `lsp.d/` that changed, were added, or were removed.
    pub lsp_changed: Vec<String>,
}

impl WatchEvent {
    pub(crate) fn is_empty(&self) -> bool {
        !self.core_changed
            && self.data_kinds.is_empty()
            && self.mcp_changed.is_empty()
            && self.lsp_changed.is_empty()
    }

    pub(crate) fn merge(&mut self, other: WatchEvent) {
        self.core_changed |= other.core_changed;
        for kind in other.data_kinds {
            if !self.data_kinds.contains(&kind) {
                self.data_kinds.push(kind);
            }
        }
        for id in other.mcp_changed {
            if !self.mcp_changed.contains(&id) {
                self.mcp_changed.push(id);
            }
        }
        for id in other.lsp_changed {
            if !self.lsp_changed.contains(&id) {
                self.lsp_changed.push(id);
            }
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Snapshot {
    core: u64,
    skills: u64,
    commands: u64,
    hooks: u64,
    profiles: u64,
    agents: u64,
    themes: u64,
    grammars: u64,
    mcp: BTreeMap<String, u64>,
    lsp: BTreeMap<String, u64>,
}

impl Snapshot {
    fn diff(&self, next: &Self) -> WatchEvent {
        let mut ev = WatchEvent {
            core_changed: self.core != next.core,
            ..Default::default()
        };
        let pairs: [(&'static str, bool); 7] = [
            ("skills", self.skills != next.skills),
            ("commands", self.commands != next.commands),
            ("hooks", self.hooks != next.hooks),
            ("profiles", self.profiles != next.profiles),
            ("agents", self.agents != next.agents),
            ("themes", self.themes != next.themes),
            ("grammars", self.grammars != next.grammars),
        ];
        for (name, changed) in pairs {
            if changed {
                ev.data_kinds.push(name);
            }
        }
        ev.mcp_changed = diff_map(&self.mcp, &next.mcp);
        ev.lsp_changed = diff_map(&self.lsp, &next.lsp);
        ev
    }
}

fn diff_map(old: &BTreeMap<String, u64>, new: &BTreeMap<String, u64>) -> Vec<String> {
    let mut changed = Vec::new();
    for (k, v) in new {
        if old.get(k) != Some(v) {
            changed.push(k.clone());
        }
    }
    for k in old.keys() {
        if !new.contains_key(k) {
            changed.push(k.clone());
        }
    }
    changed.sort();
    changed
}

/// Watch the config folders until the engine (the sender of `config`) is gone.
pub(crate) fn spawn(mut config: watch::Receiver<Arc<Loaded>>, tx: mpsc::Sender<WatchEvent>) {
    tokio::spawn(async move {
        let mut roots = folders(&config.borrow_and_update());
        let mut baseline = snapshot(roots.clone()).await;
        let mut pending: Option<Snapshot> = None;
        let mut tick = tokio::time::interval(EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                changed = config.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    roots = folders(&config.borrow_and_update());
                    baseline = snapshot(roots.clone()).await;
                    pending = None;
                }
                _ = tick.tick() => {
                    let now = snapshot(roots.clone()).await;
                    if now == baseline {
                        pending = None;
                    } else if pending.as_ref() == Some(&now) {
                        let ev = baseline.diff(&now);
                        baseline = now;
                        pending = None;
                        if !ev.is_empty() && tx.send(ev).await.is_err() {
                            break;
                        }
                    } else {
                        pending = Some(now);
                    }
                }
            }
        }
    });
}

/// The config folders a user edits: system, user, and the project's `.quack/`.
fn folders(config: &Loaded) -> Vec<PathBuf> {
    let d = &config.dirs;
    vec![d.system.clone(), d.user_config.clone(), d.project.clone()]
}

async fn snapshot(roots: Vec<PathBuf>) -> Snapshot {
    tokio::task::spawn_blocking(move || snapshot_sync(&roots))
        .await
        .unwrap_or_default()
}

fn snapshot_sync(roots: &[PathBuf]) -> Snapshot {
    let mut core = DefaultHasher::new();
    let mut skills = DefaultHasher::new();
    let mut commands = DefaultHasher::new();
    let mut hooks = DefaultHasher::new();
    let mut profiles = DefaultHasher::new();
    let mut agents = DefaultHasher::new();
    let mut themes = DefaultHasher::new();
    let mut grammars = DefaultHasher::new();
    let mut mcp: BTreeMap<String, DefaultHasher> = BTreeMap::new();
    let mut lsp: BTreeMap<String, DefaultHasher> = BTreeMap::new();

    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for e in entries {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || name.ends_with(".tmp") || SKIP.contains(&name.as_ref()) {
                continue;
            }
            let path = e.path();
            let Ok(meta) = e.metadata() else {
                continue;
            };
            if !meta.is_dir() {
                stamp_file(&path, &meta, &mut core);
                continue;
            }
            match name.as_ref() {
                "skills" => walk(&path, 0, &mut skills),
                "commands" => walk(&path, 0, &mut commands),
                "hooks.d" => walk(&path, 0, &mut hooks),
                "profiles.d" => walk(&path, 0, &mut profiles),
                "agents.d" => walk(&path, 0, &mut agents),
                "themes" => walk(&path, 0, &mut themes),
                "grammars" => walk(&path, 0, &mut grammars),
                "mcp.d" => stamp_server_dir(&path, &mut mcp),
                "lsp.d" => stamp_server_dir(&path, &mut lsp),
                _ => walk(&path, 0, &mut core),
            }
        }
    }

    Snapshot {
        core: core.finish(),
        skills: skills.finish(),
        commands: commands.finish(),
        hooks: hooks.finish(),
        profiles: profiles.finish(),
        agents: agents.finish(),
        themes: themes.finish(),
        grammars: grammars.finish(),
        mcp: mcp.into_iter().map(|(k, h)| (k, h.finish())).collect(),
        lsp: lsp.into_iter().map(|(k, h)| (k, h.finish())).collect(),
    }
}

fn stamp_server_dir(dir: &Path, map: &mut BTreeMap<String, DefaultHasher>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for e in entries {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name.ends_with(".tmp") {
            continue;
        }
        let path = e.path();
        let Ok(meta) = e.metadata() else {
            continue;
        };
        if meta.is_file()
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            && path.extension().and_then(|s| s.to_str()) == Some("toml")
        {
            let h = map.entry(stem.to_owned()).or_default();
            stamp_file(&path, &meta, h);
        }
    }
}

fn stamp_file(path: &Path, meta: &std::fs::Metadata, h: &mut impl Hasher) {
    path.hash(h);
    meta.len().hash(h);
    meta.modified().ok().hash(h);
}

fn walk(dir: &Path, depth: usize, h: &mut impl Hasher) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for e in entries {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name.ends_with(".tmp") || SKIP.contains(&name.as_ref()) {
            continue;
        }
        let Ok(meta) = e.metadata() else {
            continue;
        };
        if meta.is_dir() {
            walk(&e.path(), depth + 1, h);
        } else {
            stamp_file(&e.path(), &meta, h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn partitions_changes_by_subsystem_and_server_id() {
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().to_path_buf();
        std::fs::create_dir_all(root.join("mcp.d")).expect("mkdir");
        std::fs::create_dir_all(root.join("skills/demo")).expect("mkdir");
        std::fs::write(root.join("config.toml"), "").expect("write");

        let base = snapshot(vec![root.clone()]).await;
        assert_eq!(snapshot(vec![root.clone()]).await, base);

        // Ignored folders / temp files produce no change.
        std::fs::create_dir_all(root.join("memory")).expect("mkdir");
        std::fs::write(root.join("memory/note.md"), "x").expect("write");
        std::fs::write(root.join("config.toml.tmp"), "x").expect("write");
        assert_eq!(snapshot(vec![root.clone()]).await, base);

        // Skill edit only flags `skills`, not `core` or servers.
        std::fs::write(
            root.join("skills/demo/SKILL.md"),
            "---\nname: demo\n---\nhi\n",
        )
        .expect("write");
        let after_skill = snapshot(vec![root.clone()]).await;
        let ev = base.diff(&after_skill);
        assert!(!ev.core_changed);
        assert_eq!(ev.data_kinds, vec!["skills"]);
        assert!(ev.mcp_changed.is_empty());

        // Single MCP server edit identifies only that server id.
        std::fs::write(root.join("mcp.d/gh.toml"), "command = \"gh\"").expect("write");
        let after_mcp = snapshot(vec![root.clone()]).await;
        let ev2 = after_skill.diff(&after_mcp);
        assert!(!ev2.core_changed);
        assert!(ev2.data_kinds.is_empty());
        assert_eq!(ev2.mcp_changed, vec!["gh"]);
    }
}
