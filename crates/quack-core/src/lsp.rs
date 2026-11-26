//! Language servers in the session (docs/plan/10-lsp.md): reading `lsp.d`, the startup
//! row, and `/lsp`.

use std::sync::Arc;

use quack_config::{ItemState, Loaded, load_lsp};
use quack_lsp::{LspManager, Prepare, ServerSpec};
use quack_protocol::{Event, LoadState, LspServerInfo, Subsystem};
use tokio::sync::broadcast;

/// Read `lsp.d` and look for each server's program. Blocking.
pub(crate) fn load(config: &Loaded) -> (Vec<ServerSpec>, Vec<String>) {
    let root = &config.dirs.workspace_root;
    let mut warnings = Vec::new();
    let specs = load_lsp(config)
        .into_iter()
        .filter_map(|item| match item.state {
            ItemState::Loaded(file) if file.enabled => {
                Some(ServerSpec::resolve(item.id, file, root))
            }
            ItemState::Loaded(_) | ItemState::Disabled(_) => None,
            ItemState::Failed(d) => {
                warnings.push(format!("lsp.d: {}", d.one_line()));
                None
            }
        })
        .collect();
    (specs, warnings)
}

pub(crate) fn manager(config: &Loaded, specs: Vec<ServerSpec>) -> Arc<LspManager> {
    let prepare: Option<Prepare> = crate::sandbox::server_wrapper(config);
    LspManager::new(specs, config.dirs.workspace_root.clone(), prepare)
}

/// Keep the startup row for language servers, the live server list, and the error and
/// warning counts up to date.
pub(crate) fn report(manager: Arc<LspManager>, tx: broadcast::Sender<Event>) {
    let mut rx = manager.subscribe();
    tokio::spawn(async move {
        let mut last = None;
        let mut last_servers: Option<Vec<LspServerInfo>> = None;
        let mut counts = (0u32, 0u32);
        loop {
            let state = startup_state(&manager);
            if last.as_ref() != Some(&state) {
                let _ = tx.send(Event::StartupProgress {
                    subsystem: Subsystem::Lsp,
                    state: state.clone(),
                });
                last = Some(state);
            }
            let servers = info(&manager);
            let now = servers.iter().fold((0u32, 0u32), |(e, w), s| {
                (e.saturating_add(s.errors), w.saturating_add(s.warnings))
            });
            if last_servers.as_ref() != Some(&servers) {
                let _ = tx.send(Event::LspServers {
                    servers: servers.clone(),
                });
                last_servers = Some(servers);
            }
            if now != counts {
                counts = now;
                let _ = tx.send(Event::Diagnostics {
                    errors: now.0,
                    warnings: now.1,
                });
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    });
}

fn startup_state(manager: &LspManager) -> LoadState {
    let specs = manager.specs();
    if specs.is_empty() {
        return LoadState::Disabled {
            reason: "no servers in lsp.d".into(),
        };
    }
    let installed: Vec<&str> = specs
        .iter()
        .filter(|s| s.installed())
        .map(|s| s.id.as_str())
        .collect();
    if installed.is_empty() {
        return LoadState::Disabled {
            reason: format!("none of {} servers installed · /lsp", specs.len()),
        };
    }
    let status = manager.status();
    let failed: Vec<String> = status
        .iter()
        .filter(|s| s.state == "failed")
        .map(|s| format!("{}: {}", s.id, s.detail))
        .collect();
    let running = status
        .iter()
        .filter(|s| matches!(s.state.as_str(), "ready" | "indexing"))
        .count();
    let summary = format!(
        "{} installed ({}) · {running} running · start when a file is opened",
        installed.len(),
        installed.join(", ")
    );
    if failed.is_empty() {
        LoadState::Ready { detail: summary }
    } else {
        LoadState::NeedsAttention {
            detail: format!("{summary} · {}", failed.join(" · ")),
        }
    }
}

/// `/lsp`: each server with its state.
pub(crate) fn info(manager: &LspManager) -> Vec<LspServerInfo> {
    let root = manager.root();
    manager
        .status()
        .into_iter()
        .map(|s| LspServerInfo {
            id: s.id,
            command: s.command,
            extensions: s.extensions,
            state: s
                .state
                .parse()
                .unwrap_or(quack_protocol::LspServerState::Idle),
            detail: s.detail,
            root: s
                .root
                .map(|r| quack_tools::paths::display(root, &r))
                .unwrap_or_default(),
            errors: u32::try_from(s.errors).unwrap_or(u32::MAX),
            warnings: u32::try_from(s.warnings).unwrap_or(u32::MAX),
            log: s.log,
        })
        .collect()
}
