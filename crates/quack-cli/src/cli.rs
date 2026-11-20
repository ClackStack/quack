use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "quack",
    version,
    about = "A coding agent for open-weight models"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub command: Option<Cmd>,
    #[command(flatten)]
    pub global: Global,
    /// Run one prompt headless and print the answer (same as `exec`).
    #[arg(short = 'p', long = "print", value_name = "PROMPT")]
    pub print: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct Global {
    /// Run as if started in this directory.
    #[arg(short = 'C', long = "cd", global = true, value_name = "DIR")]
    pub cd: Option<PathBuf>,
    /// Create or reopen a git worktree under `.quack/worktrees/<NAME>` on branch `quack/<NAME>`.
    #[arg(long, global = true, value_name = "NAME")]
    pub worktree: Option<String>,
    /// Model as <provider>/<model>; overrides [model].default.
    #[arg(short = 'm', long, global = true)]
    pub model: Option<String>,
    /// Profile for this run, e.g. `qwen-code`; shorthand for -c model.profile=ID.
    #[arg(long, global = true, value_name = "ID")]
    pub profile: Option<String>,
    /// Override a setting, e.g. -c ui.ascii=true. Repeatable.
    #[arg(short = 'c', long = "config", global = true, value_name = "KEY=VALUE")]
    pub overrides: Vec<String>,
    /// Resume the most recent session in this project.
    #[arg(long = "continue", global = true, conflicts_with = "resume")]
    pub continue_: bool,
    /// Resume a saved session by id (or unique prefix); `quack sessions` lists them.
    #[arg(short = 's', long, global = true, value_name = "ID")]
    pub resume: Option<String>,
    /// Ask before unapproved actions, or allow them within the selected work mode and sandbox.
    #[arg(
        long = "approvals",
        visible_alias = "permission-mode",
        global = true,
        value_enum
    )]
    pub permission_mode: Option<PermissionModeArg>,
    /// Plan reads/searches and proposes tests; Edit may implement the task.
    #[arg(long = "mode", global = true, value_enum)]
    pub work_mode: Option<WorkModeArg>,
    /// Allow actions without approval or sandboxing; Plan restrictions still apply.
    #[arg(long, global = true, conflicts_with = "permission_mode")]
    pub yolo: bool,
    /// Draw the first frame, write startup timings as JSON to FILE, and exit.
    #[arg(long, hide = true, value_name = "FILE")]
    pub trace_startup: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum PermissionModeArg {
    #[value(hide = true)]
    ReadOnly,
    Ask,
    #[value(hide = true)]
    AutoEdit,
    Allow,
    #[value(hide = true)]
    FullAuto,
    /// No approvals and no sandbox; only the built-in deny list and deny rules apply.
    #[value(hide = true)]
    Yolo,
    /// Plan mode: look around and propose a plan; nothing changes until it is accepted.
    #[value(hide = true)]
    Plan,
}

impl PermissionModeArg {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Ask => "ask",
            Self::AutoEdit => "auto-edit",
            Self::Allow => "allow",
            Self::FullAuto => "full-auto",
            Self::Yolo => "yolo",
            Self::Plan => "plan",
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum WorkModeArg {
    Plan,
    Edit,
}

impl WorkModeArg {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Edit => "edit",
        }
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum Cmd {
    /// Full-screen terminal UI (the default).
    Tui,
    /// Run one prompt headless.
    Exec(ExecArgs),
    /// Inspect and validate configuration.
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// List saved sessions for this project.
    Sessions,
    /// Check the environment: config, directories, terminal.
    Doctor,
    /// Add a provider (a server on this machine, a hosted service, or a URL) and pick the
    /// default model.
    Setup,
    /// List, add, enable, or disable MCP servers.
    Mcp {
        #[command(subcommand)]
        cmd: McpCmd,
    },
    /// The catalog of model prices and context windows.
    Catalog {
        #[command(subcommand)]
        cmd: CatalogCmd,
    },
    /// Say whether a new version is out and how to get it. Changes nothing.
    Update,
    /// Show or decide whether this repo's `.quack/` loads. Without a decision,
    /// prints what it holds and the current state.
    Trust {
        #[arg(value_enum)]
        decision: Option<TrustArg>,
    },
    /// Store or remove API keys in the OS credential store, for `api_key = { keyring = "…" }`.
    Keyring {
        #[command(subcommand)]
        cmd: KeyringCmd,
    },
    /// Sign in to a provider that uses a sign-in instead of a key (`[auth]`, such as
    /// ChatGPT). Opens the browser.
    #[cfg(feature = "auth-chatgpt")]
    Login {
        /// The provider id; may be left out when only one provider has a sign-in.
        provider: Option<String>,
        /// Sign in to another account instead of the saved one.
        #[arg(long)]
        new: bool,
        /// Print the address instead of opening the browser.
        #[arg(long)]
        no_browser: bool,
    },
    /// Sign out of a provider's sign-in and revoke its token.
    #[cfg(feature = "auth-chatgpt")]
    Logout { provider: Option<String> },
    /// Run the eval task suite with the selected model and profile.
    Eval(EvalArgs),
    /// Launch the desktop GUI (or forward folders to the running instance).
    Gui(GuiArgs),
    /// Run as an Agent Client Protocol agent on stdio, for editors such as Zed.
    #[cfg(feature = "acp")]
    Acp,
}

#[derive(Debug, Args)]
pub(crate) struct GuiArgs {
    /// Workspace folders to open or focus in the desktop UI.
    #[arg(value_name = "PATH")]
    pub paths: Vec<PathBuf>,
    /// Open only the requested folders instead of restoring previous tabs.
    #[arg(long = "no-restore")]
    pub no_restore: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ExecArgs {
    /// The prompt; `-` reads it from stdin.
    pub prompt: String,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub output_format: Format,
}

#[derive(Debug, Args)]
pub(crate) struct EvalArgs {
    /// The task suite: a folder of task folders.
    #[arg(long, default_value = "evals/tasks")]
    pub suite: PathBuf,
    /// Only these tasks (folder names), comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub tasks: Vec<String>,
    /// Run each task this many times.
    #[arg(long, default_value_t = 1)]
    pub repeat: u32,
    /// Where to write the JSON results. Default:
    /// `evals/results/<date>/<model>-<profile>.json` next to the suite.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Record the source revision of the binary and defaults used for the run.
    #[arg(long)]
    pub revision: Option<String>,
    /// Checkpoint and stop on an error, skip, step limit, unknown setup or unknown trial cost.
    #[arg(long)]
    pub stop_on_incomplete: bool,
    /// Keep each run's workspace (under the cache directory) to look at afterwards.
    #[arg(long)]
    pub keep: bool,
    /// Plan each task with this `<provider>/<model>` in plan mode, then accept the plan
    /// and let the `-m` model carry it out (`[model] planner`).
    #[arg(long)]
    pub planner: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum Format {
    Text,
    Json,
    StreamJson,
}

impl From<Format> for quack_exec::OutputFormat {
    fn from(f: Format) -> Self {
        match f {
            Format::Text => Self::Text,
            Format::Json => Self::Json,
            Format::StreamJson => Self::StreamJson,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum TrustArg {
    /// Load everything, including MCP servers and hooks that start programs.
    Trusted,
    /// Load `config.toml` only.
    SettingsOnly,
    /// Never load it, and don't ask again.
    Ignored,
}

impl From<TrustArg> for quack_config::TrustDecision {
    fn from(t: TrustArg) -> Self {
        match t {
            TrustArg::Trusted => Self::Trusted,
            TrustArg::SettingsOnly => Self::SettingsOnly,
            TrustArg::Ignored => Self::Ignored,
        }
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum McpCmd {
    /// List configured MCP servers and bundled disabled examples.
    List,
    /// Add a local stdio or remote HTTP MCP server (interactive without `--url` / `--`).
    Add(McpAddArgs),
    /// Enable a bundled example or disabled MCP server by id.
    Enable {
        /// Server id (see `quack mcp list`).
        id: String,
    },
    /// Disable an MCP server without deleting its config.
    Disable {
        /// Server id.
        id: String,
    },
    /// Remove a user or project MCP server TOML file.
    Remove {
        /// Server id.
        id: String,
        /// Remove only from workspace `.quack/mcp.d/`.
        #[arg(long)]
        project: bool,
    },
}

#[derive(Debug, clap::Args)]
pub(crate) struct McpAddArgs {
    /// Server id (`[A-Za-z0-9_-]+`); derived from the URL or command if omitted.
    pub id: Option<String>,
    /// Remote streamable HTTP endpoint (`https://...` or `http://...`).
    #[arg(long)]
    pub url: Option<String>,
    /// HTTP header (`Name: Value`, `Name=env:VAR`, or `Name=Bearer {env:VAR}`). Repeatable.
    #[arg(long = "header", value_name = "HEADER")]
    pub headers: Vec<String>,
    /// Stdio env var (`KEY=VALUE`, `KEY=env:HOST_VAR`, or `KEY` forwarded from host env).
    #[arg(short = 'e', long = "env", value_name = "ENV")]
    pub envs: Vec<String>,
    /// Short description shown in `quack mcp list` and `/mcp`.
    #[arg(long)]
    pub description: Option<String>,
    /// Tool name globs to auto-approve (`*`, `read_*`, ...). Repeatable or comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub auto_approve: Vec<String>,
    /// Startup timeout in seconds.
    #[arg(long)]
    pub startup_timeout_secs: Option<u64>,
    /// Run a local stdio server inside the OS sandbox.
    #[arg(long)]
    pub sandbox: bool,
    /// Working directory for a stdio server.
    #[arg(long)]
    pub cwd: Option<String>,
    /// Write to `.quack/mcp.d/` in the workspace instead of user config.
    #[arg(long)]
    pub project: bool,
    /// Overwrite an existing file with the same id.
    #[arg(long)]
    pub force: bool,
    /// Local command and arguments for a stdio server (after `--`), e.g. `-- npx -y @playwright/mcp@latest`.
    #[arg(last = true, value_name = "COMMAND")]
    pub command: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum KeyringCmd {
    /// Store a secret. Reads it hidden from the terminal, or from stdin when piped.
    Set {
        /// `<service>/<user>`, or `<user>` under the `quack` service.
        name: String,
    },
    /// Remove a stored secret.
    Delete { name: String },
}

#[derive(Debug, Subcommand)]
pub(crate) enum CatalogCmd {
    /// Download the catalog from `catalog.url` now, keeping the configured providers.
    Refresh {
        /// Write it here instead of the cache directory.
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum ConfigCmd {
    /// Validate every layer and item file; non-zero exit on errors.
    Check,
    /// Print the effective settings.
    Show {
        /// Annotate each value with the layer it came from.
        #[arg(long)]
        origin: bool,
    },
    /// Write JSON Schema files for editor completion.
    Schema {
        /// Target directory (default: <user config>/schemas).
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Print the resolved directories.
    Paths,
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn planning_and_automatic_approvals_can_be_selected_together() {
        let cli = Cli::try_parse_from(["quack", "--mode", "plan", "--approvals", "allow"])
            .expect("independent flags");
        assert!(matches!(cli.global.work_mode, Some(WorkModeArg::Plan)));
        assert!(matches!(
            cli.global.permission_mode,
            Some(PermissionModeArg::Allow)
        ));
        let cli = Cli::try_parse_from(["quack", "--mode", "plan", "--yolo"])
            .expect("planning with legacy yolo");
        assert!(cli.global.yolo);
        assert!(matches!(cli.global.work_mode, Some(WorkModeArg::Plan)));
    }

    #[test]
    fn old_permission_flags_still_parse() {
        let cli = Cli::try_parse_from(["quack", "--permission-mode", "full-auto"])
            .expect("legacy flag and policy");
        assert!(matches!(
            cli.global.permission_mode,
            Some(PermissionModeArg::FullAuto)
        ));
        for legacy in ["auto-edit", "read-only", "plan", "yolo"] {
            Cli::try_parse_from(["quack", "--permission-mode", legacy])
                .expect("legacy policy still accepted");
        }
    }

    #[test]
    fn help_offers_only_ask_and_allow_for_approvals() {
        use clap::CommandFactory;

        let help = Cli::command().render_long_help().to_string();
        let approvals = help
            .split("--approvals")
            .nth(1)
            .expect("approval flag")
            .split("--mode")
            .next()
            .expect("approval help");
        assert!(
            approvals.contains("ask") && approvals.contains("allow"),
            "{approvals}"
        );
        for legacy in ["auto-edit", "read-only", "full-auto", "yolo", "plan"] {
            assert!(!approvals.contains(legacy), "{approvals}");
        }
    }

    #[test]
    fn short_s_parses_as_resume() {
        let cli = Cli::try_parse_from(["quack", "-s", "0123456789ab"]).expect("-s flag");
        assert_eq!(cli.global.resume.as_deref(), Some("0123456789ab"));
    }

    #[test]
    fn gui_subcommand_parses_paths_and_no_restore() {
        let cli = Cli::try_parse_from(["quack", "-s", "abc123", "gui", "--no-restore", "/tmp/a"])
            .expect("gui args");
        assert_eq!(cli.global.resume.as_deref(), Some("abc123"));
        match cli.command {
            Some(Cmd::Gui(args)) => {
                assert!(args.no_restore);
                assert_eq!(args.paths, vec![PathBuf::from("/tmp/a")]);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
