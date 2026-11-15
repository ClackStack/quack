//! Persistent open-tab state (`session-tabs.json`) and launch-argument merging for the
//! desktop client.
//!
//! Pure data and filesystem logic with no Tauri dependency so CLI path forwarding,
//! tab restoration, and single-instance deduplication stay unit-testable everywhere.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::recent::{expand_tilde_with, workspace_key};

const FILE: &str = "session-tabs.json";

/// One saved open workspace tab in `session-tabs.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTab {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub active: bool,
}

/// One workspace open/focus target resolved from startup state or single-instance args.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchTarget {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<String>,
    #[serde(default)]
    pub continue_latest: bool,
    #[serde(default)]
    pub focus: bool,
}

/// Parsed CLI/single-instance launch arguments for `quack-gui`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedLaunch {
    pub no_restore: bool,
    pub targets: Vec<LaunchTarget>,
}

/// Initial tabs the frontend should open when connecting on startup.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartupPayload {
    pub targets: Vec<LaunchTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_index: Option<usize>,
}

/// Persisted list of open tabs under `<user data>/gui/session-tabs.json`.
#[derive(Debug)]
pub struct OpenTabsStore {
    dir: PathBuf,
    tabs: Vec<SavedTab>,
}

impl OpenTabsStore {
    /// Load from `<user data>/gui/session-tabs.json`, or start empty.
    pub fn load() -> Self {
        Self::at(quack_config::user_data_dir(&|k| std::env::var_os(k)).join("gui"))
    }

    /// Load from an explicit directory (tests).
    pub fn at(dir: PathBuf) -> Self {
        Self::at_with(dir, &|k| std::env::var_os(k))
    }

    fn at_with(dir: PathBuf, env: &dyn Fn(&str) -> Option<OsString>) -> Self {
        let tabs = std::fs::read(dir.join(FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<SavedTab>>(&bytes).ok())
            .map(|raw| dedupe_saved(&raw, env))
            .unwrap_or_default();
        Self { dir, tabs }
    }

    /// All persisted tabs in display order.
    pub fn saved(&self) -> &[SavedTab] {
        &self.tabs
    }

    /// Saved tabs whose directories currently exist on disk.
    pub fn existing_saved(&self) -> Vec<SavedTab> {
        self.tabs
            .iter()
            .filter(|t| Path::new(&t.path).is_dir())
            .cloned()
            .collect()
    }

    /// Replace the persisted list with the current open tabs and write to disk.
    pub fn save(&mut self, incoming: &[SavedTab]) {
        self.save_with(incoming, &|k| std::env::var_os(k));
    }

    fn save_with(&mut self, incoming: &[SavedTab], env: &dyn Fn(&str) -> Option<OsString>) {
        self.tabs = dedupe_saved(incoming, env);
        if let Err(e) = std::fs::create_dir_all(&self.dir) {
            tracing::warn!(%e, "could not create the gui data directory");
            return;
        }
        match serde_json::to_vec_pretty(&self.tabs) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(self.dir.join(FILE), bytes) {
                    tracing::warn!(%e, "could not save session-tabs.json");
                }
            }
            Err(e) => tracing::warn!(%e, "could not serialize session-tabs.json"),
        }
    }
}

/// Parse full `argv` (including `argv[0]`), resolving relative paths against `base_cwd`.
pub fn parse_argv<S: AsRef<str>>(argv: &[S], base_cwd: Option<&Path>) -> ParsedLaunch {
    let args = argv.get(1..).unwrap_or(&[]);
    parse_launch_args_with(args, base_cwd, &|k| std::env::var_os(k))
}

/// Parse launch flags and positional paths (excluding `argv[0]`).
pub fn parse_launch_args_with<S: AsRef<str>>(
    args: &[S],
    base_cwd: Option<&Path>,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> ParsedLaunch {
    let mut no_restore = false;
    let mut resume: Option<String> = None;
    let mut continue_latest = false;
    let mut cd: Option<String> = None;
    let mut raw_paths: Vec<String> = Vec::new();
    let mut after_sep = false;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_ref();
        if after_sep {
            raw_paths.push(arg.to_owned());
            i += 1;
            continue;
        }
        match arg {
            "--" => after_sep = true,
            "--no-restore" => no_restore = true,
            "--continue" => continue_latest = true,
            "-s" | "--resume" => {
                if let Some(val) = args.get(i + 1) {
                    resume = Some(val.as_ref().to_owned());
                    i += 1;
                }
            }
            "-C" | "--cd" => {
                if let Some(val) = args.get(i + 1) {
                    cd = Some(val.as_ref().to_owned());
                    i += 1;
                }
            }
            _ if arg.starts_with("--resume=") => {
                let val = &arg["--resume=".len()..];
                if !val.is_empty() {
                    resume = Some(val.to_owned());
                }
            }
            _ if arg.starts_with("--cd=") => {
                let val = &arg["--cd=".len()..];
                if !val.is_empty() {
                    cd = Some(val.to_owned());
                }
            }
            _ if arg.starts_with('-') => {
                // Skip OS/framework flags such as macOS `-psn_*` or WebKit flags.
            }
            _ => raw_paths.push(arg.to_owned()),
        }
        i += 1;
    }

    if raw_paths.is_empty() {
        if let Some(dir) = cd {
            raw_paths.push(dir);
        } else if (resume.is_some() || continue_latest)
            && let Some(base) = base_cwd
            && !base.as_os_str().is_empty()
        {
            raw_paths.push(base.display().to_string());
        }
    }

    let mut targets: Vec<LaunchTarget> = Vec::new();
    let mut seen: HashMap<PathBuf, usize> = HashMap::new();
    for raw in raw_paths {
        let Some(cleaned) = resolve_target_path(&raw, base_cwd, env) else {
            continue;
        };
        let key = workspace_key(&cleaned);
        if let Some(&idx) = seen.get(&key) {
            if let Some(id) = resume.clone() {
                targets[idx].resume = Some(id);
                targets[idx].continue_latest = false;
            } else if continue_latest {
                targets[idx].continue_latest = true;
            }
            continue;
        }
        seen.insert(key, targets.len());
        targets.push(LaunchTarget {
            path: cleaned.display().to_string(),
            resume: resume.clone(),
            continue_latest: continue_latest && resume.is_none(),
            focus: false,
        });
    }

    if let Some(last) = targets.last_mut() {
        last.focus = true;
    }

    ParsedLaunch {
        no_restore,
        targets,
    }
}

/// Merge persisted tabs with explicit launch targets, focusing newly requested paths.
pub fn merge_startup_targets(
    saved: &[SavedTab],
    explicit: &[LaunchTarget],
    restore_tabs: bool,
    no_restore: bool,
) -> StartupPayload {
    if !restore_tabs || no_restore {
        let mut out = explicit.to_vec();
        let active_index = if out.is_empty() {
            None
        } else {
            let idx = out.iter().rposition(|t| t.focus).unwrap_or(out.len() - 1);
            for (i, item) in out.iter_mut().enumerate() {
                item.focus = i == idx;
            }
            Some(idx)
        };
        return StartupPayload {
            targets: out,
            active_index,
        };
    }

    let mut out: Vec<LaunchTarget> = Vec::with_capacity(saved.len() + explicit.len());
    let mut seen: HashMap<PathBuf, usize> = HashMap::new();
    let mut saved_active: Option<usize> = None;

    for tab in saved {
        let path = PathBuf::from(&tab.path);
        if path.as_os_str().is_empty() {
            continue;
        }
        let key = workspace_key(&path);
        if seen.contains_key(&key) {
            continue;
        }
        let idx = out.len();
        seen.insert(key, idx);
        if tab.active && saved_active.is_none() {
            saved_active = Some(idx);
        }
        out.push(LaunchTarget {
            path: tab.path.clone(),
            resume: tab.session_id.clone(),
            continue_latest: false,
            focus: false,
        });
    }

    let mut explicit_focus: Option<usize> = None;
    for req in explicit {
        let path = PathBuf::from(&req.path);
        if path.as_os_str().is_empty() {
            continue;
        }
        let key = workspace_key(&path);
        if let Some(&idx) = seen.get(&key) {
            if req.resume.is_some() {
                out[idx].resume = req.resume.clone();
                out[idx].continue_latest = false;
            } else if req.continue_latest {
                out[idx].continue_latest = true;
                out[idx].resume = None;
            }
            if req.focus || explicit_focus.is_none() {
                explicit_focus = Some(idx);
            }
        } else {
            let idx = out.len();
            seen.insert(key, idx);
            out.push(req.clone());
            if req.focus || explicit_focus.is_none() {
                explicit_focus = Some(idx);
            }
        }
    }

    let active_index = explicit_focus
        .or(saved_active)
        .or_else(|| (!out.is_empty()).then_some(0));
    if let Some(idx) = active_index {
        for (i, item) in out.iter_mut().enumerate() {
            item.focus = i == idx;
        }
    }

    StartupPayload {
        targets: out,
        active_index,
    }
}

fn resolve_target_path(
    raw: &str,
    base_cwd: Option<&Path>,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let expanded = expand_tilde_with(PathBuf::from(trimmed), env);
    let joined = if expanded.is_relative() {
        match base_cwd {
            Some(base) if !base.as_os_str().is_empty() => base.join(expanded),
            _ => std::env::current_dir().ok()?.join(expanded),
        }
    } else {
        expanded
    };
    let cleaned = clean_lexical(&joined);
    if cleaned.as_os_str().is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn clean_lexical(path: &Path) -> PathBuf {
    let mut stack: Vec<Component<'_>> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match stack.last() {
                Some(Component::Normal(_)) => {
                    stack.pop();
                }
                Some(Component::RootDir) | Some(Component::Prefix(_)) => {}
                _ => stack.push(comp),
            },
            _ => stack.push(comp),
        }
    }
    stack.iter().collect()
}

fn dedupe_saved(entries: &[SavedTab], env: &dyn Fn(&str) -> Option<OsString>) -> Vec<SavedTab> {
    let mut out: Vec<SavedTab> = Vec::with_capacity(entries.len());
    let mut seen: HashMap<PathBuf, usize> = HashMap::new();
    let mut has_active = false;
    for entry in entries {
        let Some(cleaned) = crate::recent::normalize_recent_path(Path::new(&entry.path), env)
        else {
            continue;
        };
        let key = workspace_key(&cleaned);
        if let Some(&idx) = seen.get(&key) {
            if out[idx].session_id.is_none() {
                out[idx].session_id = entry.session_id.clone();
            }
            if entry.active && !has_active {
                out[idx].active = true;
                has_active = true;
            }
            continue;
        }
        let active = entry.active && !has_active;
        if active {
            has_active = true;
        }
        seen.insert(key, out.len());
        out.push(SavedTab {
            path: cleaned.display().to_string(),
            session_id: entry.session_id.clone(),
            active,
        });
    }
    if !has_active && let Some(first) = out.first_mut() {
        first.active = true;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_env(home: &Path) -> impl Fn(&str) -> Option<OsString> {
        let h = home.as_os_str().to_os_string();
        move |k| match k {
            "HOME" | "USERPROFILE" => Some(h.clone()),
            _ => None,
        }
    }

    #[test]
    fn parse_argv_handles_flags_relative_paths_and_tilde() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("work").join("repo");
        let home = dir.path().join("home");
        let env = test_env(&home);

        let parsed = parse_launch_args_with(
            &[
                "-psn_0_123",
                "--no-restore",
                "--resume",
                "abc123",
                ".",
                "../sibling",
                "~/projects/app",
            ],
            Some(&base),
            &env,
        );
        assert!(parsed.no_restore);
        assert_eq!(parsed.targets.len(), 3);
        assert_eq!(parsed.targets[0].path, base.display().to_string());
        assert_eq!(parsed.targets[0].resume.as_deref(), Some("abc123"));
        assert!(!parsed.targets[0].focus);
        assert_eq!(
            parsed.targets[1].path,
            dir.path()
                .join("work")
                .join("sibling")
                .display()
                .to_string()
        );
        assert_eq!(
            parsed.targets[2].path,
            home.join("projects").join("app").display().to_string()
        );
        assert!(parsed.targets[2].focus);
    }

    #[test]
    fn parse_argv_uses_base_cwd_when_resume_given_without_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let env = test_env(dir.path());
        let parsed = parse_launch_args_with(&["-s", "deadbeef"], Some(dir.path()), &env);
        assert_eq!(parsed.targets.len(), 1);
        assert_eq!(parsed.targets[0].path, dir.path().display().to_string());
        assert_eq!(parsed.targets[0].resume.as_deref(), Some("deadbeef"));
        assert!(parsed.targets[0].focus);
    }

    #[test]
    fn merge_startup_targets_appends_and_focuses_new_folder_when_restoring() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ws1 = dir.path().join("ws1").display().to_string();
        let ws2 = dir.path().join("ws2").display().to_string();
        let ws3 = dir.path().join("ws3").display().to_string();
        let saved = vec![
            SavedTab {
                path: ws1.clone(),
                session_id: Some("s1".into()),
                active: false,
            },
            SavedTab {
                path: ws2.clone(),
                session_id: Some("s2".into()),
                active: true,
            },
        ];
        let explicit = vec![LaunchTarget {
            path: ws3.clone(),
            resume: None,
            continue_latest: false,
            focus: true,
        }];
        let merged = merge_startup_targets(&saved, &explicit, true, false);
        assert_eq!(merged.active_index, Some(2));
        assert_eq!(merged.targets.len(), 3);
        assert!(!merged.targets[0].focus);
        assert!(!merged.targets[1].focus);
        assert!(merged.targets[2].focus);
        assert_eq!(merged.targets[2].path, ws3);
    }

    #[test]
    fn merge_startup_targets_dedupes_existing_folder_and_focuses_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ws1 = dir.path().join("ws1");
        let ws2 = dir.path().join("ws2");
        let saved = vec![
            SavedTab {
                path: ws1.display().to_string(),
                session_id: Some("s1".into()),
                active: false,
            },
            SavedTab {
                path: ws2.display().to_string(),
                session_id: Some("s2".into()),
                active: true,
            },
        ];
        let explicit = vec![LaunchTarget {
            path: ws1.display().to_string(),
            resume: Some("override-id".into()),
            continue_latest: false,
            focus: true,
        }];
        let merged = merge_startup_targets(&saved, &explicit, true, false);
        assert_eq!(merged.targets.len(), 2);
        assert_eq!(merged.active_index, Some(0));
        assert!(merged.targets[0].focus);
        assert_eq!(merged.targets[0].resume.as_deref(), Some("override-id"));
    }

    #[test]
    fn merge_startup_targets_respects_no_restore_flag_or_setting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let saved = vec![SavedTab {
            path: dir.path().join("old").display().to_string(),
            session_id: None,
            active: true,
        }];
        let explicit = vec![LaunchTarget {
            path: dir.path().join("new").display().to_string(),
            resume: None,
            continue_latest: false,
            focus: true,
        }];

        let only_explicit = merge_startup_targets(&saved, &explicit, true, true);
        assert_eq!(only_explicit.targets.len(), 1);
        assert_eq!(only_explicit.active_index, Some(0));

        let disabled = merge_startup_targets(&saved, &[], false, false);
        assert!(disabled.targets.is_empty());
        assert_eq!(disabled.active_index, None);
    }

    #[test]
    fn store_persists_and_reloads_session_tabs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir_all(&a).expect("a");
        std::fs::create_dir_all(&b).expect("b");

        let mut store = OpenTabsStore::at(dir.path().to_path_buf());
        store.save(&[
            SavedTab {
                path: a.display().to_string(),
                session_id: Some("111111".into()),
                active: false,
            },
            SavedTab {
                path: b.display().to_string(),
                session_id: None,
                active: true,
            },
        ]);

        let reloaded = OpenTabsStore::at(dir.path().to_path_buf());
        assert_eq!(reloaded.saved().len(), 2);
        assert_eq!(reloaded.saved()[0].session_id.as_deref(), Some("111111"));
        assert!(reloaded.saved()[1].active);
        assert_eq!(reloaded.existing_saved().len(), 2);
    }
}
