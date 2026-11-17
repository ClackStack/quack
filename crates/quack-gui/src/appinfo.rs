//! The `AppInfo` snapshot (docs/plan/15-desktop-client.md#the-bridge): the static facts
//! a webview needs about its engine's config, sent at tab start and after
//! `StartupComplete` (which also follows `Op::ReloadConfig`). Mirrors the TUI's
//! `AppInfo::from_config` without the terminal-only parts (grammar dirs);
//! the rest of the same data reaches the GUI through `Op`s and `Event`s.

use std::path::Path;

use quack_config::hooks::OnError;
use quack_config::settings::SandboxMode;
use quack_config::{HookFile, ItemState, LayerKind, Loaded, McpServerFile, ProjectStatus};
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
    /// `<provider>/<model>` configured as `model.default`, if any.
    pub default_model: Option<String>,
    /// Whether `model.default` is locked by system policy.
    pub default_locked: bool,
    /// `<provider>/<model>` configured as `model.small`, if any.
    pub small_model: Option<String>,
    /// Whether `model.small` is locked by system policy.
    pub small_locked: bool,
    /// `<provider>/<model>` used in Plan mode, if configured.
    pub planner_model: Option<String>,
    /// Whether `model.planner` is locked by system policy.
    pub planner_locked: bool,
    /// The approval policy label, e.g. `ask` or `allow`.
    pub permission_mode: String,
    /// The work mode label: `plan` or `edit`.
    pub work_mode: String,
    /// `theme` and motion settings the webview honors (dark/light and reduced motion).
    pub ui: UiSnapshot,
    /// Outbound destination & background check summary.
    pub egress: EgressSnapshot,
    /// OS sandbox configuration & runtime support summary.
    pub sandbox: SandboxSnapshot,
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
    /// Configured hooks from `hooks.d/*.toml` across active layers.
    pub hooks: Vec<HookRow>,
    /// `[hooks] disabled` as configured, including names not installed.
    pub hooks_disabled: Vec<String>,
    /// Configured MCP servers from `mcp.d/*.toml` across active layers.
    pub mcp: Vec<McpRow>,
    /// `[mcp] disabled` as configured, including names not installed.
    pub mcp_disabled: Vec<String>,
    /// Project trust status for `.quack/`.
    pub project_status: String,
    /// Resolved configuration, memory, and session directories.
    pub paths: PathsSnapshot,
    /// Recent submitted prompts for this workspace (`history.jsonl`), oldest first.
    pub history: Vec<String>,
}

/// Resolved directories used by this config and workspace (`quack config paths`, memory, sessions).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PathsSnapshot {
    pub defaults: Option<String>,
    pub system: String,
    pub user_config: String,
    pub user_data: String,
    pub user_cache: String,
    pub project: String,
    pub memory_global: String,
    pub memory_project: String,
    pub sessions_root: String,
    pub sessions_project: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UiSnapshot {
    /// `auto`, `dark`, `light`, or a theme name.
    pub theme: String,
    /// Replace animation with static indicators.
    pub reduced_motion: bool,
    /// Restore previously open workspace tabs on desktop startup.
    pub gui_restore_tabs: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EgressEndpoint {
    pub id: String,
    pub host: String,
    /// `true` when loopback (`localhost`, `127.*`, `[::1]`).
    pub local: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EgressSnapshot {
    pub offline: bool,
    pub offline_locked: bool,
    pub egress_allow: Vec<String>,
    pub providers: Vec<EgressEndpoint>,
    pub mcp_remote: Vec<EgressEndpoint>,
    pub http1_only: bool,
    pub has_proxy: bool,
    pub updates_check: bool,
    pub catalog_refresh: bool,
    pub telemetry_enabled: bool,
    pub secret_guard: String,
    pub secret_guard_local: bool,
    pub secret_allowlist: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SandboxSnapshot {
    pub mode: String,
    pub os_supported: bool,
    pub status_detail: String,
    pub writable: Vec<String>,
    pub locked: bool,
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

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HookRow {
    pub id: String,
    pub layer: String,
    pub file: String,
    /// `enabled`, `disabled`, or `failed`.
    pub state: String,
    pub detail: String,
    pub enabled: bool,
    pub can_toggle: bool,
    pub has_setup: bool,
    pub description: String,
    pub event: Option<String>,
    pub command: Vec<String>,
    pub matcher_tool: String,
    pub matcher_pattern: String,
    pub matcher_prompt: String,
    pub timeout_secs: Option<u64>,
    pub on_error: Option<String>,
    pub full_context: bool,
    pub connections_count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct McpRow {
    pub id: String,
    pub layer: String,
    pub file: String,
    /// `enabled`, `disabled`, or `failed`.
    pub state: String,
    pub detail: String,
    pub enabled: bool,
    pub can_toggle: bool,
    pub blocked_by_policy: bool,
    pub transport: Option<String>,
    pub target: String,
    pub description: String,
    pub sandbox: Option<bool>,
    pub startup_timeout_secs: Option<u64>,
    pub call_timeout_secs: Option<u64>,
    pub max_output_tokens: Option<u32>,
    pub auto_approve: Vec<String>,
    pub env_keys: Vec<String>,
    pub header_keys: Vec<String>,
}

impl AppInfo {
    pub fn from_config(config: &Loaded) -> Self {
        let cwd = match config
            .dirs
            .home
            .as_deref()
            .and_then(|home| config.dirs.workspace_root.strip_prefix(home).ok())
        {
            Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
            Some(rest) => Path::new("~").join(rest).display().to_string(),
            None => config.dirs.workspace_root.display().to_string(),
        };
        let permissions = &config.settings.permissions;
        Self {
            version: env!("CARGO_PKG_VERSION").to_owned(),
            branch: git_branch(&config.dirs.workspace_root),
            cwd,
            model: Some(config.settings.model.default.clone()).filter(|m| !m.is_empty()),
            default_model: Some(config.settings.model.default.clone()).filter(|m| !m.is_empty()),
            default_locked: config.settings.policy.locks("model.default"),
            small_model: Some(config.settings.model.small.clone()).filter(|m| !m.is_empty()),
            small_locked: config.settings.policy.locks("model.small"),
            planner_model: Some(config.settings.model.planner.clone()).filter(|m| !m.is_empty()),
            planner_locked: config.settings.policy.locks("model.planner"),
            permission_mode: permissions.mode.label().to_owned(),
            work_mode: permissions.work_mode.label().to_owned(),
            ui: UiSnapshot {
                theme: config.settings.ui.theme.clone(),
                reduced_motion: config.settings.ui.reduced_motion,
                gui_restore_tabs: config.settings.ui.gui_restore_tabs,
            },
            egress: egress_snapshot(config),
            sandbox: sandbox_snapshot(config),
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
            hooks: hooks_snapshot(config),
            hooks_disabled: config.settings.hooks.disabled.clone(),
            mcp: mcp_snapshot(config),
            mcp_disabled: config.settings.mcp.disabled.clone(),
            project_status: project_status_label(config.project).to_owned(),
            paths: paths_snapshot(config),
            history: quack_core::load_history(&quack_core::history_path(config)),
        }
    }
}

fn project_status_label(status: ProjectStatus) -> &'static str {
    match status {
        ProjectStatus::None => "none",
        ProjectStatus::Trusted => "trusted",
        ProjectStatus::SettingsOnly => "settings-only",
        ProjectStatus::Untrusted => "untrusted",
        ProjectStatus::Changed => "changed",
        ProjectStatus::Ignored => "ignored",
        ProjectStatus::DisabledByPolicy => "disabled-by-policy",
    }
}

fn paths_snapshot(config: &Loaded) -> PathsSnapshot {
    let d = &config.dirs;
    let (memory_global, memory_project) = quack_core::memory_dirs(config);
    let (sessions_root, sessions_project) = quack_core::sessions_dirs(config);
    PathsSnapshot {
        defaults: d.defaults.as_ref().map(|p| p.display().to_string()),
        system: d.system.display().to_string(),
        user_config: d.user_config.display().to_string(),
        user_data: d.user_data.display().to_string(),
        user_cache: d.user_cache.display().to_string(),
        project: d.project.display().to_string(),
        memory_global: memory_global.display().to_string(),
        memory_project: memory_project.display().to_string(),
        sessions_root: sessions_root.display().to_string(),
        sessions_project: sessions_project.display().to_string(),
    }
}

fn egress_snapshot(config: &Loaded) -> EgressSnapshot {
    let providers = quack_config::load_providers(config)
        .into_iter()
        .filter_map(|item| {
            let file = item.loaded()?;
            let id = file.id.clone().unwrap_or_else(|| item.id.clone());
            let host = quack_config::url_host(&file.base_url).to_owned();
            if host.is_empty() {
                return None;
            }
            Some(EgressEndpoint {
                id,
                host,
                local: quack_config::is_loopback(&file.base_url),
                enabled: file.enabled,
            })
        })
        .collect();
    let mcp_remote = quack_config::load_mcp(config)
        .into_iter()
        .filter_map(|item| {
            let file = item.loaded()?;
            if file.url.is_empty() {
                return None;
            }
            let id = file.id.clone().unwrap_or_else(|| item.id.clone());
            let host = quack_config::url_host(&file.url).to_owned();
            Some(EgressEndpoint {
                id,
                host,
                local: quack_config::is_loopback(&file.url),
                enabled: file.enabled,
            })
        })
        .collect();
    let net = &config.settings.network;
    let secret_guard = match net.secret_guard {
        quack_config::settings::SecretGuardMode::Off => "off",
        quack_config::settings::SecretGuardMode::Ask => "ask",
        quack_config::settings::SecretGuardMode::Redact => "redact",
    }
    .to_owned();
    EgressSnapshot {
        offline: net.offline,
        offline_locked: config.settings.policy.locks("network.offline"),
        egress_allow: net.egress_allow.clone(),
        providers,
        mcp_remote,
        http1_only: net.http1_only,
        has_proxy: !net.proxy.trim().is_empty(),
        updates_check: config.settings.updates.check,
        catalog_refresh: config.settings.catalog.refresh,
        telemetry_enabled: !config.settings.telemetry.otel_endpoint.trim().is_empty(),
        secret_guard,
        secret_guard_local: net.secret_guard_local,
        secret_allowlist: net.secret_allowlist.clone(),
    }
}

fn sandbox_snapshot(config: &Loaded) -> SandboxSnapshot {
    let sb = &config.settings.sandbox;
    let avail = quack_sandbox::available();
    let os_supported = avail.is_ok();
    let status_detail = match (sb.mode, avail) {
        (SandboxMode::Off, _) => "disabled (sandbox.mode = off)".to_owned(),
        (_, Ok(mech)) => format!("active ({mech})"),
        (_, Err(why)) => why,
    };
    SandboxSnapshot {
        mode: sb.mode.label().to_owned(),
        os_supported,
        status_detail,
        writable: sb.writable.clone(),
        locked: config.settings.policy.locks("sandbox.mode"),
    }
}

fn hooks_snapshot(config: &Loaded) -> Vec<HookRow> {
    let disabled_locked = config.settings.policy.locks("hooks.disabled");
    quack_config::load_hooks(config)
        .into_iter()
        .map(|item| {
            let file_path = item.path.display().to_string();
            let parsed = match &item.state {
                ItemState::Loaded(f) => Some(f.clone()),
                ItemState::Disabled(_) => quack_config::read_item_file::<HookFile>(&item.path),
                ItemState::Failed(_) => None,
            };
            let (state, detail, enabled) = match &item.state {
                ItemState::Loaded(f) if f.enabled => ("enabled", "on".to_owned(), true),
                ItemState::Loaded(_) => ("disabled", "disabled in file".to_owned(), false),
                ItemState::Disabled(why) => ("disabled", why.clone(), false),
                ItemState::Failed(d) => ("failed", d.one_line(), false),
            };
            let listed_and_locked =
                disabled_locked && config.settings.hooks.disabled.contains(&item.id);
            let can_toggle = item.layer != LayerKind::System
                && !matches!(item.state, ItemState::Failed(_))
                && !listed_and_locked
                && !(enabled && disabled_locked);
            let (
                has_setup,
                description,
                event,
                command,
                matcher_tool,
                matcher_pattern,
                matcher_prompt,
                timeout_secs,
                on_error,
                full_context,
                connections_count,
            ) = match parsed {
                Some(f) => (
                    f.setup.is_some(),
                    f.description,
                    Some(f.event.label().to_owned()),
                    f.command,
                    f.matcher.tool,
                    f.matcher.pattern,
                    f.matcher.prompt,
                    Some(f.timeout_secs),
                    Some(on_error_label(f.on_error).to_owned()),
                    f.full_context,
                    f.connections.as_ref().map_or(0, Vec::len),
                ),
                None => (
                    false,
                    String::new(),
                    None,
                    Vec::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    None,
                    None,
                    false,
                    0,
                ),
            };
            HookRow {
                id: item.id,
                layer: item.layer.label().to_owned(),
                file: file_path,
                state: state.to_owned(),
                detail,
                enabled,
                can_toggle,
                has_setup,
                description,
                event,
                command,
                matcher_tool,
                matcher_pattern,
                matcher_prompt,
                timeout_secs,
                on_error,
                full_context,
                connections_count,
            }
        })
        .collect()
}

fn on_error_label(mode: OnError) -> &'static str {
    match mode {
        OnError::Deny => "deny",
        OnError::Allow => "allow",
        OnError::Warn => "warn",
    }
}

fn mcp_snapshot(config: &Loaded) -> Vec<McpRow> {
    let disabled_locked = config.settings.policy.locks("mcp.disabled");
    quack_config::load_mcp(config)
        .into_iter()
        .map(|item| {
            let file_path = item.path.display().to_string();
            let parsed = match &item.state {
                ItemState::Loaded(f) => Some(f.clone()),
                ItemState::Disabled(_) => quack_config::read_item_file::<McpServerFile>(&item.path),
                ItemState::Failed(_) => None,
            };
            let (state, detail, enabled) = match &item.state {
                ItemState::Loaded(_) => ("enabled", "enabled".to_owned(), true),
                ItemState::Disabled(why) => ("disabled", why.clone(), false),
                ItemState::Failed(d) => ("failed", d.one_line(), false),
            };
            let policy_url = parsed
                .as_ref()
                .and_then(|f| (!f.url.is_empty()).then_some(f.url.as_str()));
            let blocked_by_policy = detail.contains("policy")
                || (parsed.is_some() && !config.settings.policy.mcp.allows(&item.id, policy_url));
            let listed_and_locked =
                disabled_locked && config.settings.mcp.disabled.contains(&item.id);
            let can_toggle = item.layer != LayerKind::System
                && !matches!(item.state, ItemState::Failed(_))
                && !blocked_by_policy
                && !listed_and_locked
                && !(enabled && disabled_locked);
            let (
                transport,
                target,
                description,
                sandbox,
                startup_timeout_secs,
                call_timeout_secs,
                max_output_tokens,
                auto_approve,
                env_keys,
                header_keys,
            ) = match parsed {
                Some(f) => {
                    let transport = f.transport().ok().map(str::to_owned);
                    let target = if !f.url.is_empty() {
                        f.url
                    } else if f.args.is_empty() {
                        f.command
                    } else {
                        format!("{} {}", f.command, f.args.join(" "))
                    };
                    (
                        transport,
                        target,
                        f.description,
                        f.sandbox,
                        Some(f.startup_timeout_secs),
                        Some(f.call_timeout_secs),
                        Some(f.max_output_tokens),
                        f.tools.auto_approve,
                        f.env.into_keys().collect(),
                        f.headers.into_keys().collect(),
                    )
                }
                None => (
                    None,
                    String::new(),
                    String::new(),
                    None,
                    None,
                    None,
                    None,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ),
            };
            McpRow {
                id: item.id,
                layer: item.layer.label().to_owned(),
                file: file_path,
                state: state.to_owned(),
                detail,
                enabled,
                can_toggle,
                blocked_by_policy,
                transport,
                target,
                description,
                sandbox,
                startup_timeout_secs,
                call_timeout_secs,
                max_output_tokens,
                auto_approve,
                env_keys,
                header_keys,
            }
        })
        .collect()
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

    #[test]
    fn cwd_collapses_only_configured_home_at_component_boundary() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().join("alice");
        let proj = home.join("dev").join("proj");
        let sibling = tmp.path().join("alice2").join("proj");
        std::fs::create_dir_all(&proj).expect("proj");
        std::fs::create_dir_all(&sibling).expect("sibling");

        let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let load = |ws: &Path, with_home: bool| {
            let vars = |key: &str| match key {
                "QUACK_CONFIG_DIR" => Some(tmp.path().join("user").into_os_string()),
                "QUACK_SYSTEM_DIR" => Some(tmp.path().join("system").into_os_string()),
                "QUACK_DATA_DIR" => Some(tmp.path().join("data").into_os_string()),
                "QUACK_CACHE_DIR" => Some(tmp.path().join("cache").into_os_string()),
                "QUACK_DEFAULTS_DIR" => Some("none".into()),
                k if with_home && k == home_var => Some(home.clone().into_os_string()),
                _ => None,
            };
            quack_config::load_with(&vars, &quack_config::LoadOptions::new(ws.to_path_buf()))
                .expect("load")
        };

        assert_eq!(
            AppInfo::from_config(&load(&proj, true)).cwd,
            Path::new("~")
                .join("dev")
                .join("proj")
                .display()
                .to_string()
        );
        assert_eq!(AppInfo::from_config(&load(&home, true)).cwd, "~");
        assert_eq!(
            AppInfo::from_config(&load(&sibling, true)).cwd,
            sibling.display().to_string()
        );
        let info = AppInfo::from_config(&load(&proj, false));
        assert_eq!(info.cwd, proj.display().to_string());
        assert!(info.ui.gui_restore_tabs);
        assert_eq!(info.egress.secret_guard, "ask");
        assert!(!info.egress.secret_guard_local);
    }
}
