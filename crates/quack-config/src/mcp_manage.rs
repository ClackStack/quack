//! Inspecting, enabling, disabling, adding, and removing `mcp.d/*.toml` server files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::diagnostic::read_text;
use crate::edit::{add_to_list, write_atomic};
use crate::enable::{FileOff, defaults_dest, file_off, flip_enabled_toml, remove_bare_overrides};
use crate::items::{Item, ItemState, load_items, load_items_except, url_host};
use crate::load::{Loaded, ProjectStatus};
use crate::mcp::McpServerFile;
use crate::paths::LayerKind;
use crate::secret::Secret;
use crate::trust::TrustDecision;

/// Result of removing an MCP server configuration file.
/// (See [`Loaded::mcp_summaries`], [`Loaded::enable_mcp`], [`Loaded::disable_mcp`],
/// [`Loaded::add_mcp_server`], [`Loaded::remove_mcp`].)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpSummary {
    pub id: String,
    pub layer: LayerKind,
    pub path: PathBuf,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    pub parse_error: Option<String>,
    pub description: String,
    /// `"stdio"` or `"http"`, or empty if unknown/broken.
    pub transport: String,
    /// Endpoint URL or `command arg1 arg2` preview, or empty if unknown/broken.
    pub target: String,
    /// Environment variable names referenced by `{ env = "..." }`.
    pub env_vars: Vec<String>,
}

/// Result of enabling an MCP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpEnabled {
    /// File that now holds the enabled definition.
    pub file: PathBuf,
    /// Set when the file was copied from the defaults bundle.
    pub copied_from: Option<PathBuf>,
    /// Set when `[mcp] disabled` in user/project config no longer lists the server.
    pub unlisted: bool,
    /// Short human summary of what changed.
    pub action: String,
    /// Environment variable names referenced by `{ env = "..." }` in `env` or `headers`.
    pub env_vars: Vec<String>,
}

/// Result of disabling an MCP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpDisabled {
    /// File edited (either `mcp.d/<id>.toml` or `config.toml`).
    pub path: PathBuf,
    /// Short human summary of what changed.
    pub action: String,
}

/// Transport-specific fields for [`NewMcpServer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewMcpTransport {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, Secret>,
        cwd: Option<String>,
        sandbox: Option<bool>,
    },
    Http {
        url: String,
        headers: BTreeMap<String, Secret>,
    },
}

/// Specification for writing a new `mcp.d/<id>.toml` server file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMcpServer {
    pub id: String,
    pub description: Option<String>,
    pub transport: NewMcpTransport,
    pub auto_approve: Vec<String>,
    pub startup_timeout_secs: Option<u64>,
    /// Write to `.quack/mcp.d/` in the workspace instead of the user config directory.
    pub project: bool,
    /// Replace an existing file in the target directory.
    pub overwrite: bool,
}

/// Validate a safe `<id>.toml` stem (`[A-Za-z0-9_-]+`).
pub fn check_mcp_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("MCP server id cannot be empty".into());
    }
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(format!(
            "`{id}` is not a valid id (use ASCII letters, digits, `-`, or `_`)"
        ));
    }
    Ok(())
}

/// Suggest a valid MCP server id from an endpoint URL (e.g. `https://mcp.context7.com/mcp` -> `"context7"`).
pub fn derive_mcp_id_from_url(url: &str) -> Option<String> {
    let no_scheme = url
        .split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let host = no_scheme.split(':').next().unwrap_or_default();
    let host = host
        .strip_prefix("mcp.")
        .or_else(|| host.strip_prefix("api."))
        .or_else(|| host.strip_prefix("www."))
        .unwrap_or(host);
    let label = host.split('.').next().unwrap_or(host);
    sanitize_mcp_id(label)
}

/// Suggest a valid MCP server id from a stdio command and arguments.
pub fn derive_mcp_id_from_command(command: &str, args: &[String]) -> Option<String> {
    let exe = Path::new(command)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(command);
    if matches!(exe, "npx" | "uvx" | "bunx" | "pnpm" | "dnx")
        && let Some(pkg) = args.iter().find(|a| !a.starts_with('-'))
    {
        let (scope, pkg_part) = pkg.split_once('/').map_or((None, pkg.as_str()), |(s, p)| {
            (Some(s.trim_start_matches('@')), p)
        });
        let without_ver = pkg_part.split('@').next().unwrap_or(pkg_part);
        let clean = without_ver
            .strip_prefix("mcp-server-")
            .or_else(|| without_ver.strip_prefix("server-"))
            .or_else(|| without_ver.strip_suffix("-mcp"))
            .unwrap_or(without_ver);
        let candidate = if matches!(clean, "mcp" | "server" | "mcp-server") {
            scope.unwrap_or(clean)
        } else {
            clean
        };
        if let Some(id) = sanitize_mcp_id(candidate) {
            return Some(id);
        }
    }
    sanitize_mcp_id(exe)
}

fn sanitize_mcp_id(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    (!out.is_empty()).then_some(out)
}

/// Split a shell-style command line respecting single quotes, double quotes, and backslashes.
pub fn split_command_words(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            None if c == '\\' => {
                if let Some(next) = chars.next() {
                    cur.push(next);
                }
            }
            None => cur.push(c),
        }
    }
    if quote.is_some() {
        return Err("unclosed quote in command".into());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    Ok(out)
}

/// Environment variable names referenced via `{ env = "NAME" }` in any secret list.
pub fn collect_secret_env_vars<'a>(secrets: impl IntoIterator<Item = &'a Secret>) -> Vec<String> {
    let mut vars = Vec::new();
    for secret in secrets {
        if let Secret::Env { env, .. } = secret
            && !vars.iter().any(|existing| existing == env)
        {
            vars.push(env.clone());
        }
    }
    vars.sort();
    vars
}

/// Environment variable names referenced via `{ env = "NAME" }` in a server's `env` or `headers`.
pub fn collect_mcp_env_vars(file: &McpServerFile) -> Vec<String> {
    collect_secret_env_vars(file.env.values().chain(file.headers.values()))
}

impl Loaded {
    /// Summarize every configured and bundled MCP server, sorted by id, including
    /// transport/target/description for disabled bundled examples.
    pub fn mcp_summaries(&self) -> Vec<McpSummary> {
        let mut out: Vec<McpSummary> = crate::load_mcp(self)
            .into_iter()
            .map(|item| summarize_item(self, item))
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Enable an MCP server by id: copy a bundled `enabled = false` example into user
    /// `mcp.d/<id>.toml` with `enabled = true`, enable a user/project file in place,
    /// remove bare `enabled = false` overrides, and remove `id` from `[mcp] disabled`.
    pub fn enable_mcp(&self, id: &str) -> Result<McpEnabled, String> {
        const KEY: &str = "mcp.disabled";
        check_mcp_id(id)?;
        let mut item = crate::load_mcp(self)
            .into_iter()
            .find(|i| i.id == id)
            .ok_or_else(|| {
                format!("no MCP server `{id}` found — add one with `quack mcp add {id} ...`")
            })?;

        let mut overrides = Vec::new();
        let off = loop {
            if item.layer == LayerKind::System {
                return Err(format!(
                    "MCP server `{id}` is enforced by the system layer and cannot be changed"
                ));
            }
            if let ItemState::Failed(d) = &item.state {
                return Err(format!(
                    "MCP server `{id}` does not load: {}; fix its file first",
                    d.one_line()
                ));
            }
            let off = file_off::<McpServerFile>(&item.path)?;
            if !matches!(off, FileOff::Bare) {
                break off;
            }
            if !matches!(item.layer, LayerKind::User | LayerKind::Project) {
                return Err(format!(
                    "MCP server `{id}` is switched off by {}; edit that file to enable it",
                    item.path.display()
                ));
            }
            overrides.push(item.path);
            item = load_items_except::<McpServerFile>(&self.item_layers(), "mcp.d", &overrides)
                .into_iter()
                .find(|m| m.id == id)
                .ok_or_else(|| {
                    format!(
                        "no MCP server definition remains after removing the overrides for `{id}`"
                    )
                })?;
        };

        let text = read_text(&item.path)
            .map_err(|e| format!("cannot read {}: {e}", item.path.display()))?;
        let parsed: McpServerFile = toml::from_str(&text)
            .map_err(|e| format!("{} does not load: {e}", item.path.display()))?;
        parsed.transport()?;
        let url = (!parsed.url.is_empty()).then_some(parsed.url.as_str());
        if !self.settings.policy.mcp.allows(id, url) {
            return Err(format!("MCP server `{id}` is blocked by `[policy] mcp`"));
        }

        if self.settings.mcp.disabled.iter().any(|d| d == id) {
            self.ensure_disabled_mutable(KEY, "MCP server", id)?;
        }

        let flag_outcome = if matches!(off, FileOff::Flag) {
            let out = flip_enabled_toml::<McpServerFile>(&item.path)?;
            match item.layer {
                LayerKind::Defaults => {
                    let dest =
                        defaults_dest(&self.dirs.user_config, "mcp.d", &item.path, &overrides)?;
                    write_atomic(&dest, &out)?;
                    Some((
                        dest,
                        Some(item.path.clone()),
                        format!("copied example `{id}` to user config and enabled it"),
                    ))
                }
                _ => {
                    if item.layer == LayerKind::Project {
                        require_trusted_project(self)?;
                    }
                    write_atomic(&item.path, &out)?;
                    let scope = if item.layer == LayerKind::Project {
                        " in project config"
                    } else {
                        ""
                    };
                    Some((item.path.clone(), None, format!("enabled `{id}`{scope}")))
                }
            }
        } else {
            None
        };
        remove_bare_overrides(
            &overrides,
            flag_outcome.as_ref().map(|(path, _, _)| path.as_path()),
        )?;

        let mut project_changed = (item.layer == LayerKind::Project && flag_outcome.is_some())
            || overrides.iter().any(|p| p.starts_with(&self.dirs.project));
        let unlisted = self.unlist_disabled(
            KEY,
            "MCP server",
            id,
            |s| &s.mcp.disabled,
            &mut project_changed,
        )?;
        let env_vars = collect_mcp_env_vars(&parsed);

        match flag_outcome {
            Some((file, copied_from, action)) => Ok(McpEnabled {
                file,
                copied_from,
                unlisted,
                action,
                env_vars,
            }),
            None if !overrides.is_empty() => Ok(McpEnabled {
                file: item.path,
                copied_from: None,
                unlisted,
                action: format!("removed disabled override and enabled `{id}`"),
                env_vars,
            }),
            None if unlisted => Ok(McpEnabled {
                file: item.path,
                copied_from: None,
                unlisted: true,
                action: format!("enabled `{id}`"),
                env_vars,
            }),
            None => Err(format!("MCP server `{id}` is already enabled")),
        }
    }

    /// Disable an MCP server by id without deleting its config: sets `enabled = false` in
    /// user/project `mcp.d/<id>.toml`, or adds `id` to `[mcp] disabled` for defaults/system.
    pub fn disable_mcp(&self, id: &str) -> Result<McpDisabled, String> {
        check_mcp_id(id)?;
        let item = crate::load_mcp(self)
            .into_iter()
            .find(|i| i.id == id)
            .ok_or_else(|| format!("no MCP server `{id}` is configured"))?;

        match &item.state {
            ItemState::Disabled(reason) => {
                return Ok(McpDisabled {
                    path: item.path,
                    action: format!("`{id}` is already disabled ({reason})"),
                });
            }
            ItemState::Loaded(file) if !file.enabled => {
                return Ok(McpDisabled {
                    path: item.path,
                    action: format!("`{id}` is already disabled"),
                });
            }
            _ => {}
        }

        if matches!(item.layer, LayerKind::User | LayerKind::Project)
            && !matches!(item.state, ItemState::Failed(_))
        {
            if item.layer == LayerKind::Project {
                require_trusted_project(self)?;
            }
            let updated = set_enabled_in_toml(&item.path, false)?;
            write_atomic(&item.path, &updated)?;
            if item.layer == LayerKind::Project {
                self.trust_project(TrustDecision::Trusted)?;
            }
            return Ok(McpDisabled {
                path: item.path,
                action: format!("disabled `{id}`"),
            });
        }

        if self.settings.policy.locks("mcp.disabled") {
            return Err("`mcp.disabled` is locked by the system policy".into());
        }
        let cfg = self
            .layer_file(LayerKind::User)
            .ok_or("the user layer has no directory")?;
        add_to_list(&cfg, "mcp.disabled", &[id.to_owned()])?;
        Ok(McpDisabled {
            path: cfg,
            action: format!("added `{id}` to [mcp] disabled"),
        })
    }

    /// Write a new `<id>.toml` server definition to user `mcp.d/` or (with `project = true`)
    /// `.quack/mcp.d/`, validating the schema, `[policy.mcp]`, and project trust.
    pub fn add_mcp_server(&self, spec: &NewMcpServer) -> Result<PathBuf, String> {
        check_mcp_id(&spec.id)?;
        let system_layers: Vec<_> = self
            .item_layers()
            .into_iter()
            .filter(|(k, _)| *k == LayerKind::System)
            .collect();
        if load_items_except::<McpServerFile>(&system_layers, "mcp.d", &[])
            .iter()
            .any(|i| i.id == spec.id)
        {
            return Err(format!(
                "MCP server `{}` is enforced by the system layer and cannot be replaced",
                spec.id
            ));
        }
        validate_new_spec(self, spec)?;
        if self.settings.mcp.disabled.iter().any(|d| d == &spec.id) {
            self.ensure_disabled_mutable("mcp.disabled", "MCP server", &spec.id)?;
        }

        let dir = if spec.project {
            match self.project {
                ProjectStatus::Trusted | ProjectStatus::None => self.dirs.project.join("mcp.d"),
                ProjectStatus::DisabledByPolicy => {
                    return Err("project config is disabled by the system policy".into());
                }
                _ => {
                    return Err(
                        ".quack/ exists and is not trusted — run `quack trust` first, or omit `--project`"
                            .into(),
                    );
                }
            }
        } else {
            self.dirs.user_config.join("mcp.d")
        };

        let path = dir.join(format!("{}.toml", spec.id));
        if path.exists() && !spec.overwrite {
            return Err(format!(
                "{} already exists (pass --force to overwrite or pick a different id)",
                path.display()
            ));
        }

        let toml_text = render_new_mcp_toml(spec)?;
        write_atomic(&path, &toml_text)?;

        if spec.project {
            self.trust_project(TrustDecision::Trusted)?;
        }
        let mut project_changed = false;
        let _ = self.unlist_disabled(
            "mcp.disabled",
            "MCP server",
            &spec.id,
            |s| &s.mcp.disabled,
            &mut project_changed,
        )?;
        Ok(path)
    }

    /// Create or update `<layer>/mcp.d/<id>.toml` in the user or project layer,
    /// remove `<id>` from `[mcp] disabled`, and record/refresh project trust when needed.
    pub fn save_mcp_server(
        &self,
        layer: LayerKind,
        mut spec: NewMcpServer,
    ) -> Result<PathBuf, String> {
        spec.id = spec.id.trim().to_owned();
        match layer {
            LayerKind::User => spec.project = false,
            LayerKind::Project => {
                if !matches!(self.project, ProjectStatus::Trusted | ProjectStatus::None) {
                    let message = if self.project == ProjectStatus::DisabledByPolicy {
                        "project config is disabled by the system policy"
                    } else {
                        "trust this project before saving project-level MCP servers"
                    };
                    return Err(message.into());
                }
                spec.project = true;
            }
            _ => return Err("MCP servers can only be saved to the user or project layer".into()),
        }
        spec.overwrite = true;
        self.add_mcp_server(&spec)
    }

    /// Remove an MCP server TOML file from a writable layer (`user` or trusted `project`).
    pub fn remove_mcp(&self, id: &str, project_only: Option<bool>) -> Result<PathBuf, String> {
        check_mcp_id(id)?;
        let file_name = format!("{id}.toml");
        let project_path = self.dirs.project.join("mcp.d").join(&file_name);
        let user_path = self.dirs.user_config.join("mcp.d").join(&file_name);

        if project_only != Some(false)
            && self.project == ProjectStatus::Trusted
            && project_path.is_file()
        {
            std::fs::remove_file(&project_path)
                .map_err(|e| format!("cannot remove {}: {e}", project_path.display()))?;
            self.trust_project(TrustDecision::Trusted)?;
            return Ok(project_path);
        }

        if project_only != Some(true) && user_path.is_file() {
            std::fs::remove_file(&user_path)
                .map_err(|e| format!("cannot remove {}: {e}", user_path.display()))?;
            return Ok(user_path);
        }

        if project_only == Some(true) {
            if project_path.exists() && self.project != ProjectStatus::Trusted {
                return Err(
                    "project config is not trusted — run `quack trust` before modifying .quack/mcp.d/"
                        .into(),
                );
            }
            return Err(format!(
                "no project MCP server file `{}`",
                project_path.display()
            ));
        }

        if let Some(item) = crate::load_mcp(self).into_iter().find(|i| i.id == id) {
            return match item.layer {
                LayerKind::Defaults => Err(format!(
                    "`{id}` is a bundled example (not a user file); use `quack mcp disable {id}` to keep it disabled"
                )),
                LayerKind::System => Err(format!(
                    "`{id}` is configured in the system layer ({})",
                    item.path.display()
                )),
                LayerKind::Project => Err(
                    "project config is not trusted — run `quack trust` before removing project files"
                        .into(),
                ),
                LayerKind::User => Err(format!("cannot locate writable file for `{id}`")),
            };
        }

        Err(format!("no MCP server `{id}` found"))
    }
}

fn summarize_item(loaded: &Loaded, item: Item<McpServerFile>) -> McpSummary {
    let mut summary = McpSummary {
        id: item.id.clone(),
        layer: item.layer,
        path: item.path.clone(),
        enabled: false,
        disabled_reason: None,
        parse_error: None,
        description: String::new(),
        transport: String::new(),
        target: String::new(),
        env_vars: Vec::new(),
    };
    match &item.state {
        ItemState::Loaded(file) => {
            summary.enabled = true;
            fill_summary_from_file(&mut summary, file);
        }
        ItemState::Disabled(reason) => {
            summary.disabled_reason = Some(reason.clone());
            let parsed = read_text(&item.path)
                .ok()
                .and_then(|t| toml::from_str::<McpServerFile>(&t).ok());
            let file = match parsed {
                Some(f) if f.transport().is_ok() => Some(f),
                _ => find_lower_valid_mcp(loaded, &item.id, item.layer).and_then(|base| {
                    read_text(&base.path)
                        .ok()
                        .and_then(|t| toml::from_str::<McpServerFile>(&t).ok())
                }),
            };
            if let Some(f) = file {
                fill_summary_from_file(&mut summary, &f);
            }
        }
        ItemState::Failed(err) => {
            summary.parse_error = Some(err.to_string());
        }
    }
    summary
}

fn fill_summary_from_file(summary: &mut McpSummary, file: &McpServerFile) {
    summary.description = file.description.clone();
    summary.transport = file.transport().unwrap_or_default().to_owned();
    summary.target = if !file.url.is_empty() {
        file.url.clone()
    } else if !file.command.is_empty() {
        if file.args.is_empty() {
            file.command.clone()
        } else {
            format!("{} {}", file.command, file.args.join(" "))
        }
    } else {
        String::new()
    };
    summary.env_vars = collect_mcp_env_vars(file);
}

fn find_lower_valid_mcp(
    loaded: &Loaded,
    id: &str,
    below: LayerKind,
) -> Option<Item<McpServerFile>> {
    let lower_layers: Vec<(LayerKind, PathBuf)> = loaded
        .item_layers()
        .into_iter()
        .filter(|(k, _)| *k < below)
        .collect();
    load_items::<McpServerFile>(&lower_layers, "mcp.d")
        .into_iter()
        .find(|i| {
            i.id == id
                && !matches!(i.state, ItemState::Failed(_))
                && read_text(&i.path)
                    .ok()
                    .and_then(|t| toml::from_str::<McpServerFile>(&t).ok())
                    .is_some_and(|f| f.transport().is_ok())
        })
}

fn require_trusted_project(loaded: &Loaded) -> Result<(), String> {
    if loaded.project != ProjectStatus::Trusted {
        return Err(
            "project config is not trusted — run `quack trust` before editing .quack/mcp.d/".into(),
        );
    }
    Ok(())
}

fn set_enabled_in_toml(path: &Path, enabled: bool) -> Result<String, String> {
    let text = read_text(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| {
        format!("{}: not valid TOML: {}", path.display(), e.message())
    })?;
    doc["enabled"] = toml_edit::value(enabled);
    let out = doc.to_string();
    let parsed: McpServerFile =
        toml::from_str(&out).map_err(|e| format!("{}: {}", path.display(), e.message()))?;
    parsed.transport()?;
    Ok(out)
}

fn validate_new_spec(loaded: &Loaded, spec: &NewMcpServer) -> Result<(), String> {
    let url_opt = match &spec.transport {
        NewMcpTransport::Stdio { command, .. } => {
            if command.trim().is_empty() {
                return Err("command cannot be empty for a stdio MCP server".into());
            }
            None
        }
        NewMcpTransport::Http { url, .. } => {
            let trimmed = url.trim();
            if !(trimmed.starts_with("http://") || trimmed.starts_with("https://"))
                || trimmed.chars().any(char::is_whitespace)
                || url_host(trimmed).is_empty()
            {
                return Err(format!(
                    "`{trimmed}` is not a valid http:// or https:// MCP endpoint URL"
                ));
            }
            Some(trimmed)
        }
    };
    if !loaded.settings.policy.mcp.allows(&spec.id, url_opt) {
        return Err(format!(
            "MCP server `{}` is not allowed by `[policy.mcp]`",
            spec.id
        ));
    }
    Ok(())
}

fn render_new_mcp_toml(spec: &NewMcpServer) -> Result<String, String> {
    let mut doc = toml_edit::DocumentMut::new();
    if let Some(desc) = spec.description.as_deref().filter(|d| !d.trim().is_empty()) {
        doc["description"] = toml_edit::value(desc.trim());
    }
    match &spec.transport {
        NewMcpTransport::Stdio {
            command,
            args,
            env,
            cwd,
            sandbox,
        } => {
            doc["command"] = toml_edit::value(command.trim());
            if !args.is_empty() {
                let mut arr = toml_edit::Array::new();
                for a in args {
                    arr.push(a.as_str());
                }
                doc["args"] = toml_edit::value(arr);
            }
            if let Some(cwd) = cwd.as_deref().filter(|c| !c.trim().is_empty()) {
                doc["cwd"] = toml_edit::value(cwd.trim());
            }
            if let Some(sb) = sandbox {
                doc["sandbox"] = toml_edit::value(*sb);
            }
            if !env.is_empty() {
                doc["env"] = toml_edit::value(secret_inline_table(env)?);
            }
        }
        NewMcpTransport::Http { url, headers } => {
            doc["url"] = toml_edit::value(url.trim());
            if !headers.is_empty() {
                doc["headers"] = toml_edit::value(secret_inline_table(headers)?);
            }
        }
    }
    if let Some(secs) = spec.startup_timeout_secs {
        doc["startup_timeout_secs"] = toml_edit::value(secs as i64);
    }
    if !spec.auto_approve.is_empty() {
        let mut tools = toml_edit::Table::new();
        let mut arr = toml_edit::Array::new();
        for pattern in &spec.auto_approve {
            arr.push(pattern.as_str());
        }
        tools["auto_approve"] = toml_edit::value(arr);
        doc["tools"] = toml_edit::Item::Table(tools);
    }

    let rendered = format!("# MCP server: {}\n{}", spec.id, doc);
    let parsed: McpServerFile = toml::from_str(&rendered)
        .map_err(|e| format!("generated invalid MCP TOML: {}", e.message()))?;
    parsed.transport()?;
    Ok(rendered)
}

fn secret_inline_table(
    entries: &BTreeMap<String, Secret>,
) -> Result<toml_edit::InlineTable, String> {
    let mut tbl = toml_edit::InlineTable::new();
    for (k, v) in entries {
        let key = k.trim();
        if key.is_empty() {
            return Err("environment/header key must not be empty".into());
        }
        tbl.insert(key, secret_to_toml_value(v));
    }
    Ok(tbl)
}

fn secret_to_toml_value(secret: &Secret) -> toml_edit::Value {
    match secret {
        Secret::Plain(v) => toml_edit::Value::from(v.as_str()),
        Secret::Env { env, format } => {
            let mut tbl = toml_edit::InlineTable::new();
            tbl.insert("env", toml_edit::Value::from(env.as_str()));
            if let Some(fmt) = format {
                tbl.insert("format", toml_edit::Value::from(fmt.as_str()));
            }
            toml_edit::Value::InlineTable(tbl)
        }
        Secret::Keyring { keyring } => {
            let mut tbl = toml_edit::InlineTable::new();
            tbl.insert("keyring", toml_edit::Value::from(keyring.as_str()));
            toml_edit::Value::InlineTable(tbl)
        }
        Secret::File { file } => {
            let mut tbl = toml_edit::InlineTable::new();
            tbl.insert("file", toml_edit::Value::from(file.as_str()));
            toml_edit::Value::InlineTable(tbl)
        }
        Secret::Cmd { cmd } => {
            let mut tbl = toml_edit::InlineTable::new();
            let mut arr = toml_edit::Array::new();
            for part in cmd {
                arr.push(part.as_str());
            }
            tbl.insert("cmd", toml_edit::Value::Array(arr));
            toml_edit::Value::InlineTable(tbl)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::{LoadOptions, load_with};
    use std::collections::HashMap;
    use std::ffi::OsString;

    struct Fixture {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        vars: HashMap<String, OsString>,
    }

    impl Fixture {
        fn new() -> Self {
            let tmp = tempfile::tempdir().expect("tempdir");
            let root = tmp.path().to_path_buf();
            std::fs::create_dir_all(root.join("repo")).expect("repo");
            let mut vars = HashMap::new();
            vars.insert(
                "QUACK_DEFAULTS_DIR".into(),
                root.join("defaults").into_os_string(),
            );
            vars.insert(
                "QUACK_SYSTEM_DIR".into(),
                root.join("system").into_os_string(),
            );
            vars.insert(
                "QUACK_CONFIG_DIR".into(),
                root.join("user").into_os_string(),
            );
            vars.insert("QUACK_DATA_DIR".into(), root.join("data").into_os_string());
            Self {
                _tmp: tmp,
                root,
                vars,
            }
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.root.join(rel);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, text).expect("write");
        }

        fn load(&self) -> Loaded {
            let opts = LoadOptions::new(self.root.join("repo"));
            let vars = self.vars.clone();
            load_with(&move |k| vars.get(k).cloned(), &opts).expect("load")
        }
    }

    #[test]
    fn enable_defaults_copies_preserving_comments_and_reports_env_vars() {
        let f = Fixture::new();
        f.write(
            "defaults/mcp.d/roost.toml",
            "# Roost search server\n\
             enabled = false\n\
             description = \"Roost docs\"\n\
             url = \"https://roost.internal:8484/mcp\"\n\
             headers = { Authorization = { env = \"ROOST_TOKEN\", format = \"Bearer {}\" } }\n",
        );

        let out = f.load().enable_mcp("roost").expect("enable");
        assert_eq!(out.file, f.root.join("user/mcp.d/roost.toml"));
        assert_eq!(out.env_vars, ["ROOST_TOKEN"]);

        let saved = std::fs::read_to_string(&out.file).expect("read");
        assert!(saved.contains("# Roost search server"));
        assert!(saved.contains("enabled = true"));

        let items = crate::load_mcp(&f.load());
        let roost = items.iter().find(|i| i.id == "roost").expect("roost");
        assert_eq!(roost.layer, LayerKind::User);
        assert!(matches!(&roost.state, ItemState::Loaded(m) if m.enabled));
    }

    #[test]
    fn disable_and_re_enable_user_server_round_trips() {
        let f = Fixture::new();
        let spec = NewMcpServer {
            id: "context7".into(),
            description: Some("Docs".into()),
            transport: NewMcpTransport::Http {
                url: "https://mcp.context7.com/mcp".into(),
                headers: BTreeMap::new(),
            },
            auto_approve: vec!["*".into()],
            startup_timeout_secs: None,
            project: false,
            overwrite: false,
        };
        let path = f.load().add_mcp_server(&spec).expect("add");
        assert_eq!(path, f.root.join("user/mcp.d/context7.toml"));

        let disabled = f.load().disable_mcp("context7").expect("disable");
        assert_eq!(disabled.path, path);
        let items = crate::load_mcp(&f.load());
        let c7 = items.iter().find(|i| i.id == "context7").expect("c7");
        assert!(matches!(&c7.state, ItemState::Disabled(r) if r.contains("enabled = false")));

        f.load().enable_mcp("context7").expect("re-enable");
        let items = crate::load_mcp(&f.load());
        let c7 = items.iter().find(|i| i.id == "context7").expect("c7");
        assert!(matches!(&c7.state, ItemState::Loaded(m) if m.enabled));
    }

    #[test]
    fn add_project_server_trusts_clean_repo_and_refuses_untrusted_repo() {
        let f = Fixture::new();
        let spec = NewMcpServer {
            id: "local".into(),
            description: None,
            transport: NewMcpTransport::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "@playwright/mcp@latest".into()],
                env: BTreeMap::from([(
                    "API_KEY".into(),
                    Secret::Env {
                        env: "MY_KEY".into(),
                        format: None,
                    },
                )]),
                cwd: None,
                sandbox: None,
            },
            auto_approve: vec![],
            startup_timeout_secs: Some(60),
            project: true,
            overwrite: false,
        };

        // Clean workspace (no .quack/ yet): adding with --project writes and trusts it.
        let path = f.load().add_mcp_server(&spec).expect("add in clean repo");
        assert_eq!(path, f.root.join("repo/.quack/mcp.d/local.toml"));
        let reloaded = f.load();
        assert_eq!(reloaded.project, ProjectStatus::Trusted);
        let items = crate::load_mcp(&reloaded);
        assert!(
            items
                .iter()
                .any(|i| i.id == "local" && i.layer == LayerKind::Project)
        );

        // Remove the project file cleanly and preserve trust.
        let removed = reloaded.remove_mcp("local", Some(true)).expect("remove");
        assert_eq!(removed, path);
        assert!(!path.exists());

        // Existing untrusted .quack/ must NOT be auto-trusted by add --project.
        let f2 = Fixture::new();
        f2.write(
            "repo/.quack/config.toml",
            "[permissions]\nallow = [\"Bash(*)\"]\n",
        );
        let err = f2
            .load()
            .add_mcp_server(&spec)
            .expect_err("untrusted repo refused");
        assert!(err.contains("not trusted"), "{err}");
    }

    #[test]
    fn policy_blocks_disallowed_mcp_servers() {
        let f = Fixture::new();
        f.write(
            "system/config.toml",
            "[policy.mcp]\nallowed = [\"roost\"]\nhosts = [\"roost.internal\"]\n",
        );
        let spec = NewMcpServer {
            id: "evil".into(),
            description: None,
            transport: NewMcpTransport::Http {
                url: "https://evil.example/mcp".into(),
                headers: BTreeMap::new(),
            },
            auto_approve: vec![],
            startup_timeout_secs: None,
            project: false,
            overwrite: false,
        };
        let err = f.load().add_mcp_server(&spec).expect_err("blocked");
        assert!(err.contains("policy.mcp"), "{err}");
    }

    #[test]
    fn enabling_clears_bare_disabled_stub_over_defaults() {
        let f = Fixture::new();
        f.write(
            "defaults/mcp.d/context7.toml",
            "enabled = false\nurl = \"https://mcp.context7.com/mcp\"\n",
        );
        f.write("user/mcp.d/context7.toml", "enabled = false\n");

        let out = f.load().enable_mcp("context7").expect("enable");
        let text = std::fs::read_to_string(&out.file).expect("read");
        assert!(text.contains("https://mcp.context7.com/mcp"), "{text}");
        assert!(text.contains("enabled = true"), "{text}");
    }

    #[test]
    fn enable_mcp_copies_defaults_and_unlists_disabled() {
        let f = Fixture::new();
        f.write(
            "defaults/mcp.d/github.toml",
            "# GitHub MCP\nenabled = false\nurl = \"https://api.githubcopilot.com/mcp/\"\n",
        );
        f.write(
            "user/config.toml",
            "[mcp]\ndisabled = [\"other\", \"github\"]\n",
        );
        let loaded = f.load();

        let done = loaded.enable_mcp("github").expect("enable");
        assert_eq!(
            done.copied_from,
            Some(f.root.join("defaults/mcp.d/github.toml"))
        );
        assert!(done.unlisted);
        let copy = std::fs::read_to_string(f.root.join("user/mcp.d/github.toml")).expect("copy");
        assert!(copy.starts_with("# GitHub MCP\n"), "{copy}");
        assert!(copy.contains("enabled = true"), "{copy}");

        let reloaded = f.load();
        assert_eq!(reloaded.settings.mcp.disabled, ["other"]);
        let servers = crate::load_mcp(&reloaded);
        assert_eq!(servers.len(), 1);
        assert!(servers[0].loaded().is_some());
    }

    #[test]
    fn enable_mcp_refuses_system_and_policy_blocked_servers() {
        let f = Fixture::new();
        f.write(
            "system/mcp.d/corp.toml",
            "enabled = false\nurl = \"https://corp.internal/mcp\"\n",
        );
        let err = f.load().enable_mcp("corp").expect_err("system");
        assert!(err.contains("system layer"), "{err}");

        let f2 = Fixture::new();
        f2.write(
            "defaults/mcp.d/github.toml",
            "enabled = false\nurl = \"https://api.githubcopilot.com/mcp/\"\n",
        );
        f2.write(
            "system/config.toml",
            "[policy.mcp]\nallowed = [\"local-*\"]\n",
        );
        let err2 = f2.load().enable_mcp("github").expect_err("policy");
        assert!(err2.contains("policy"), "{err2}");
    }

    #[test]
    fn save_mcp_server_writes_stdio_and_http_with_project_trust() {
        let f = Fixture::new();
        let path = f
            .load()
            .save_mcp_server(
                LayerKind::User,
                NewMcpServer {
                    id: "everything".into(),
                    description: Some("Everything demo".into()),
                    transport: NewMcpTransport::Stdio {
                        command: "npx".into(),
                        args: vec![
                            "-y".into(),
                            "@modelcontextprotocol/server-everything".into(),
                        ],
                        env: BTreeMap::from([("LOG_LEVEL".into(), Secret::Plain("info".into()))]),
                        cwd: None,
                        sandbox: Some(true),
                    },
                    auto_approve: Vec::new(),
                    startup_timeout_secs: None,
                    project: false,
                    overwrite: false,
                },
            )
            .expect("save user stdio");
        assert_eq!(path, f.root.join("user/mcp.d/everything.toml"));

        f.write("repo/.quack/config.toml", "");
        let loaded = f.load();
        let err = loaded
            .save_mcp_server(
                LayerKind::Project,
                NewMcpServer {
                    id: "docs".into(),
                    description: None,
                    transport: NewMcpTransport::Http {
                        url: "https://docs.example.com/mcp".into(),
                        headers: BTreeMap::new(),
                    },
                    auto_approve: Vec::new(),
                    startup_timeout_secs: None,
                    project: false,
                    overwrite: false,
                },
            )
            .expect_err("untrusted project rejected");
        assert!(err.contains("trust"), "{err}");

        loaded.trust_project(TrustDecision::Trusted).expect("trust");
        let proj_path = f
            .load()
            .save_mcp_server(
                LayerKind::Project,
                NewMcpServer {
                    id: "docs".into(),
                    description: Some("Docs MCP".into()),
                    transport: NewMcpTransport::Http {
                        url: "https://docs.example.com/mcp".into(),
                        headers: BTreeMap::from([(
                            "Authorization".into(),
                            Secret::Env {
                                env: "DOCS_TOKEN".into(),
                                format: Some("Bearer {}".into()),
                            },
                        )]),
                    },
                    auto_approve: Vec::new(),
                    startup_timeout_secs: None,
                    project: false,
                    overwrite: false,
                },
            )
            .expect("save trusted project http");
        assert_eq!(proj_path, f.root.join("repo/.quack/mcp.d/docs.toml"));

        let reloaded = f.load();
        assert_eq!(reloaded.project, ProjectStatus::Trusted);
        let servers = crate::load_mcp(&reloaded);
        let docs = servers.iter().find(|s| s.id == "docs").expect("docs");
        let doc_file = docs.loaded().expect("loaded docs");
        assert_eq!(doc_file.url, "https://docs.example.com/mcp");
        assert_eq!(
            doc_file.headers.get("Authorization"),
            Some(&Secret::Env {
                env: "DOCS_TOKEN".into(),
                format: Some("Bearer {}".into()),
            })
        );
    }

    #[test]
    fn save_mcp_server_allows_clean_repo_and_blocks_policy_disabled() {
        let f = Fixture::new();
        let clean = f.load();
        assert_eq!(clean.project, ProjectStatus::None);
        let proj_path = clean
            .save_mcp_server(
                LayerKind::Project,
                NewMcpServer {
                    id: "clean".into(),
                    description: None,
                    transport: NewMcpTransport::Stdio {
                        command: "npx".into(),
                        args: vec!["-y".into(), "clean-mcp".into()],
                        cwd: None,
                        sandbox: None,
                        env: BTreeMap::new(),
                    },
                    auto_approve: Vec::new(),
                    startup_timeout_secs: None,
                    project: false,
                    overwrite: false,
                },
            )
            .expect("save on clean repo creates .quack and trusts");
        assert_eq!(proj_path, f.root.join("repo/.quack/mcp.d/clean.toml"));
        let reloaded = f.load();
        assert_eq!(reloaded.project, ProjectStatus::Trusted);
        assert!(
            crate::load_mcp(&reloaded)
                .iter()
                .any(|s| s.id == "clean" && s.loaded().is_some())
        );

        f.write(
            "system/config.toml",
            "[policy]\nproject_config = \"never\"\n",
        );
        let err = f
            .load()
            .save_mcp_server(
                LayerKind::Project,
                NewMcpServer {
                    id: "blocked".into(),
                    description: None,
                    transport: NewMcpTransport::Http {
                        url: "https://example.com/mcp".into(),
                        headers: BTreeMap::new(),
                    },
                    auto_approve: Vec::new(),
                    startup_timeout_secs: None,
                    project: false,
                    overwrite: false,
                },
            )
            .expect_err("blocked by policy");
        assert!(err.contains("policy"), "{err}");
    }
}
