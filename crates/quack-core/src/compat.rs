//! Discover other agents' extension folders without loading them.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use quack_config::settings::ProjectConfigPolicy;
use quack_config::{LayerKind, Loaded, ProjectStatus};
use quack_instructions::compat::Compat;
use quack_protocol::{CompatFolder, CompatKind, Event, LoadState, Subsystem, WriteScope};
use tokio::sync::broadcast;

#[derive(Debug)]
pub(crate) struct Candidate {
    pub folder: CompatFolder,
    setting: String,
}

pub(crate) fn key(kind: CompatKind) -> &'static str {
    match kind {
        CompatKind::Skills => "skills.compat_paths",
        CompatKind::Commands => "commands.compat_paths",
    }
}

fn project_enabled(config: &Loaded) -> bool {
    config.settings.policy.project_config != ProjectConfigPolicy::Never
        && config.project != ProjectStatus::Ignored
}

pub(crate) fn discover(config: &Loaded) -> Vec<Candidate> {
    let layers: Vec<_> = config.item_layers().into_iter().map(|(_, d)| d).collect();
    let list = Compat::load(&layers);
    let mut found = Vec::new();
    for (kind, folders) in [
        (CompatKind::Skills, list.skills),
        (CompatKind::Commands, list.commands),
    ] {
        if config.settings.policy.locks(key(kind))
            || (folders.project.is_empty() && folders.user.is_empty())
        {
            continue;
        }
        let roots = match kind {
            CompatKind::Skills => crate::env::skill_roots(config),
            CompatKind::Commands => crate::env::command_roots(config),
        };
        let mut seen: HashSet<_> = roots
            .into_iter()
            .filter_map(|(_, p)| p.canonicalize().ok())
            .collect();
        if project_enabled(config) {
            let can_save = matches!(
                config.project,
                ProjectStatus::None | ProjectStatus::Trusted | ProjectStatus::SettingsOnly
            ) && !config.dirs.project.is_symlink();
            let base = &config.dirs.workspace_root;
            if let Ok(real_base) = base.canonicalize() {
                for setting in folders.project {
                    if let Some(candidate) = candidate(
                        base,
                        &real_base,
                        &setting,
                        kind,
                        WriteScope::Project,
                        can_save,
                        &mut seen,
                    ) {
                        found.push(candidate);
                    }
                }
            }
        }
        if let Some(home) = &config.dirs.home
            && let Ok(real_home) = home.canonicalize()
        {
            for setting in folders.user {
                if let Some(relative) = setting
                    .strip_prefix("~/")
                    .or_else(|| setting.strip_prefix("~\\"))
                    && let Some(mut candidate) = candidate(
                        home,
                        &real_home,
                        relative,
                        kind,
                        WriteScope::User,
                        true,
                        &mut seen,
                    )
                {
                    candidate.setting = setting;
                    found.push(candidate);
                }
            }
        }
    }
    found
}

fn candidate(
    base: &Path,
    real_base: &Path,
    relative: &str,
    kind: CompatKind,
    scope: WriteScope,
    can_save: bool,
    seen: &mut HashSet<PathBuf>,
) -> Option<Candidate> {
    let rel = Path::new(relative);
    if relative.is_empty() || !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    let path = base.join(rel.components().collect::<PathBuf>());
    let real = path.canonicalize().ok()?;
    if real == real_base || !real.starts_with(real_base) || !real.is_dir() || !seen.insert(real) {
        return None;
    }
    Some(Candidate {
        folder: CompatFolder {
            kind,
            path: path.to_str()?.to_owned(),
            scope,
            can_save,
        },
        setting: relative.to_owned(),
    })
}

impl Candidate {
    pub(crate) fn save(&self, config: &Loaded) -> Result<PathBuf, String> {
        let layer = if self.folder.scope == WriteScope::Project {
            LayerKind::Project
        } else {
            LayerKind::User
        };
        config.save_compat_path(layer, key(self.folder.kind), &self.setting)
    }

    pub(crate) fn is_configured(&self, config: &Loaded) -> bool {
        match self.folder.kind {
            CompatKind::Skills => config.settings.skills.compat_paths.contains(&self.setting),
            CompatKind::Commands => config
                .settings
                .commands
                .compat_paths
                .contains(&self.setting),
        }
    }
}

#[derive(Debug)]
struct Import {
    workspace: PathBuf,
    candidate: Candidate,
    persisted: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Imports(Vec<Import>);

impl Imports {
    pub(crate) fn enable(&mut self, config: &Loaded, candidate: Candidate, persisted: bool) {
        self.0.push(Import {
            workspace: config.dirs.workspace_root.clone(),
            candidate,
            persisted,
        });
    }

    pub(crate) fn on_reload(&mut self) {
        // On-disk settings decide whether a saved path still applies after a reload.
        self.0.retain(|entry| !entry.persisted);
    }

    pub(crate) fn config(&self, base: &Arc<Loaded>) -> Arc<Loaded> {
        let mut effective = None;
        for Import {
            workspace,
            candidate,
            ..
        } in &self.0
        {
            let folder = &candidate.folder;
            if base.settings.policy.locks(key(folder.kind))
                || (folder.scope == WriteScope::Project
                    && (workspace != &base.dirs.workspace_root || !project_enabled(base)))
            {
                continue;
            }
            if candidate.is_configured(base) {
                continue;
            }
            let settings = &mut effective.get_or_insert_with(|| (**base).clone()).settings;
            let paths = match folder.kind {
                CompatKind::Skills => &mut settings.skills.compat_paths,
                CompatKind::Commands => &mut settings.commands.compat_paths,
            };
            if !paths.contains(&folder.path) {
                paths.push(folder.path.clone());
            }
        }
        effective.map_or_else(|| base.clone(), Arc::new)
    }
}

pub(crate) fn report(tx: &broadcast::Sender<Event>, candidates: Vec<Candidate>) {
    let folders: Vec<_> = candidates.into_iter().map(|c| c.folder).collect();
    let state = match folders.len() {
        0 => LoadState::Ready {
            detail: "no new folders".into(),
        },
        1 => LoadState::NeedsAttention {
            detail: "1 folder".into(),
        },
        n => LoadState::NeedsAttention {
            detail: format!("{n} folders"),
        },
    };
    let _ = tx.send(Event::CompatFolders { folders });
    let _ = tx.send(Event::StartupProgress {
        subsystem: Subsystem::Compat,
        state,
    });
}
