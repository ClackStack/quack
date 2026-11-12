//! The `AppInfo` snapshot (docs/plan/15-desktop-client.md#the-bridge): the static facts
//! a webview needs about its engine's config, sent at tab start and after
//! `StartupComplete` (which also follows `Op::ReloadConfig`). Mirrors the TUI's
//! `AppInfo::from_config` without the terminal-only parts (history, hooks, grammar
//! dirs); the rest of the same data reaches the GUI through `Op`s and `Event`s.

use std::path::Path;

use quack_config::Loaded;
use serde::Serialize;

/// A serializable snapshot of `Loaded`, built once per tab and on reload.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AppInfo {
    pub version: String,
    /// The workspace root, with `$HOME` collapsed to `~`.
    pub cwd: String,
    /// The checked-out git branch, or a short commit when detached.
    pub branch: Option<String>,
    /// `<provider>/<model>` the session starts on.
    pub model: Option<String>,
    /// The approval policy label, e.g. `ask` or `allow`.
    pub permission_mode: String,
    /// The work mode label: `plan` or `edit`.
    pub work_mode: String,
    /// `theme` and motion settings the webview honors (dark/light and reduced motion).
    pub ui: UiSnapshot,
    /// Effective settings for the settings table (`Op::SaveSetting` edits them).
    pub settings: Vec<SettingRowView>,
    /// `(layer, status and file)` rows, lowest first.
    pub layers: Vec<LayerRow>,
    /// `(list, rule)` from `[permissions]`, e.g. `("allow", "shell:git status*")`.
    pub rules: Vec<RuleRow>,
    /// Installed bundles, enabled or not.
    pub bundles: Vec<BundleRow>,
    /// `[bundles] disabled` as configured, including names not installed.
    pub bundles_disabled: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UiSnapshot {
    /// `auto`, `dark`, `light`, or a theme name.
    pub theme: String,
    /// Replace animation with static indicators.
    pub reduced_motion: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SettingRowView {
    pub key: String,
    /// The effective value as TOML.
    pub value: String,
    pub origin: String,
    pub description: String,
    pub is_bool: bool,
    /// Locked by the system policy; cannot be changed here.
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LayerRow {
    /// The layer kind's label, e.g. `user`.
    pub layer: String,
    /// `loaded`, `no config.toml`, `failed to parse (see config check)` or `skipped: …`.
    pub status: String,
    /// The layer's `config.toml`, when the layer has a directory.
    pub file: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RuleRow {
    /// `deny`, `ask` or `allow`.
    pub list: String,
    pub rule: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BundleRow {
    pub id: String,
    /// The layer the bundle came from.
    pub layer: String,
    pub enabled: bool,
    /// What it holds, e.g. `2 skills, 1 MCP server`.
    pub contents: String,
}

impl AppInfo {
    pub fn from_config(config: &Loaded) -> Self {
        let root = config.dirs.workspace_root.display().to_string();
        let home =
            std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default();
        let cwd = match root.strip_prefix(&home) {
            Some(rest) if !home.is_empty() => format!("~{rest}"),
            _ => root,
        };
        let permissions = &config.settings.permissions;
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            branch: git_branch(&config.dirs.workspace_root),
            cwd,
            model: Some(config.settings.model.default.clone()).filter(|m| !m.is_empty()),
            permission_mode: permissions.mode.label().to_owned(),
            work_mode: permissions.work_mode.label().to_owned(),
            ui: UiSnapshot {
                theme: config.settings.ui.theme.clone(),
                reduced_motion: config.settings.ui.reduced_motion,
            },
            settings: config
                .setting_rows()
                .into_iter()
                .map(|row| SettingRowView {
                    key: row.key,
                    value: row.value,
                    origin: row.origin,
                    description: row.description,
                    is_bool: row.is_bool,
                    locked: row.locked,
                })
                .collect(),
            layers: config
                .layers
                .iter()
                .map(|l| LayerRow {
                    layer: l.kind.label().to_owned(),
                    status: match &l.status {
                        quack_config::LayerStatus::Loaded => "loaded".to_owned(),
                        quack_config::LayerStatus::Missing => "no config.toml".to_owned(),
                        quack_config::LayerStatus::Failed => {
                            "failed to parse (see config check)".to_owned()
                        }
                        quack_config::LayerStatus::Skipped(why) => format!("skipped: {why}"),
                    },
                    file: l
                        .dir
                        .as_ref()
                        .map(|d| d.join("config.toml").display().to_string()),
                })
                .collect(),
            rules: [
                ("deny", &permissions.deny),
                ("ask", &permissions.ask),
                ("allow", &permissions.allow),
            ]
            .into_iter()
            .flat_map(|(list, rules)| {
                rules.iter().map(move |rule| RuleRow {
                    list: list.to_owned(),
                    rule: rule.clone(),
                })
            })
            .collect(),
            bundles: config
                .bundles
                .iter()
                .map(|b| BundleRow {
                    id: b.id.clone(),
                    layer: b.layer.label().to_owned(),
                    enabled: b.enabled,
                    contents: b.contents.clone(),
                })
                .collect(),
            bundles_disabled: config.settings.bundles.disabled.clone(),
        }
    }
}

/// The branch in `.git/HEAD` of `dir` or a parent; worktrees and submodules point at
/// their git dir with a `gitdir:` file.
fn git_branch(dir: &Path) -> Option<String> {
    let git = dir
        .ancestors()
        .map(|d| d.join(".git"))
        .find(|g| g.exists())?;
    let git = if git.is_file() {
        let link = std::fs::read_to_string(&git).ok()?;
        let target = link.strip_prefix("gitdir:")?.trim();
        git.parent()?.join(target)
    } else {
        git
    };
    let head = std::fs::read_to_string(git.join("HEAD")).ok()?;
    let head = head.trim();
    Some(match head.strip_prefix("ref: refs/heads/") {
        Some(branch) => branch.to_owned(),
        None => head.chars().take(7).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_branch_of_a_normal_checkout() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".git")).expect("write .git");
        std::fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/tauri-gui\n")
            .expect("write HEAD");
        assert_eq!(git_branch(dir.path()).as_deref(), Some("tauri-gui"));
    }

    #[test]
    fn finds_the_branch_of_a_worktree_checkout() {
        let store = tempfile::tempdir().expect("tempdir");
        let wt = store.path().join("wt");
        std::fs::create_dir_all(&wt).expect("write gitdir");
        std::fs::write(wt.join("HEAD"), "ref: refs/heads/main\n").expect("write HEAD");
        let dir = tempfile::tempdir().expect("tempdir");
        let link = format!("gitdir: {}", wt.display());
        std::fs::write(dir.path().join(".git"), link).expect("write .git link");
        assert_eq!(git_branch(dir.path()).as_deref(), Some("main"));
    }

    #[test]
    fn falls_back_to_the_short_commit_when_detached() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".git")).expect("write .git");
        std::fs::write(dir.path().join(".git/HEAD"), "1a2b3c4d5e6f\n").expect("write HEAD");
        assert_eq!(git_branch(dir.path()).as_deref(), Some("1a2b3c4"));
    }
}
