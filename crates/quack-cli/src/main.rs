//! The `quack` binary: parses arguments, loads config synchronously (tiny files
//! only), starts the engine and hands it to a client.

#![allow(clippy::print_stdout, clippy::print_stderr)]

mod cli;
mod commands;
mod errors;
mod logging;
mod mcp;
mod setup;
mod update;

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use clap::Parser;
use quack_config::LoadOptions;
use quack_core::{Engine, Resume};
use quack_exec::exit;

use cli::{CatalogCmd, Cli, Cmd, ConfigCmd, KeyringCmd, SessionsCmd};

fn main() -> ExitCode {
    let started = Instant::now();
    let cli = Cli::parse();
    let mut cwd = match cli.global.cd.clone().map_or_else(std::env::current_dir, Ok) {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("error: cannot determine working directory: {e}");
            return code(exit::CONFIG_ERROR);
        }
    };
    if let Some(name) = &cli.global.worktree {
        match quack_store::worktree::create_or_open(&cwd, name, true) {
            Ok(wt) => {
                cwd = wt.path;
            }
            Err(e) => {
                eprintln!("error: --worktree {name}: {e}");
                return code(exit::CONFIG_ERROR);
            }
        }
    }
    let mut overrides = cli.global.overrides.clone();
    if let Some(p) = &cli.global.profile {
        overrides.push(format!("model.profile={}", toml_string(p)));
    }
    if let Some(key) = cli
        .global
        .cache_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        overrides.push(format!("context.cache_key={}", toml_string(key)));
    }
    let mode = match (cli.global.permission_mode, cli.global.yolo) {
        (_, true) => Some(cli::PermissionModeArg::Yolo),
        (m, false) => m,
    };
    // Plan mode is an overlay on the configured mode, set once the engine runs.
    let legacy_plan = matches!(mode, Some(cli::PermissionModeArg::Plan));
    let plan = legacy_plan && cli.global.work_mode.is_none();
    if let Some(mode) = mode.filter(|_| !legacy_plan) {
        overrides.push(format!("permissions.mode=\"{}\"", mode.as_str()));
    }
    if let Some(mode) = cli.global.work_mode {
        overrides.push(format!("permissions.work_mode=\"{}\"", mode.as_str()));
    }
    let opts = LoadOptions {
        cwd,
        model: cli.global.model.clone(),
        overrides,
        mcp: Vec::new(),
    };
    let loaded = match quack_config::load(&opts) {
        Ok(l) => l,
        Err(e) => {
            errors::config(&e);
            return code(exit::CONFIG_ERROR);
        }
    };
    let config_loaded = started.elapsed();
    let _log_guard = logging::init(&loaded.dirs.log_dir());
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "starting");

    let resume = match (&cli.global.resume, cli.global.continue_) {
        (Some(id), _) => Resume::Id(id.clone()),
        (None, true) => Resume::Latest,
        (None, false) => Resume::New,
    };
    let command = match (cli.command, cli.print) {
        (Some(cmd), _) => cmd,
        (None, Some(prompt)) => Cmd::Exec(cli::ExecArgs {
            prompt,
            output_format: cli::Format::Text,
        }),
        (None, None) => Cmd::Tui,
    };

    match command {
        Cmd::Config { cmd } => code(match cmd {
            ConfigCmd::Check => commands::config_check(&loaded),
            ConfigCmd::Show { origin } => commands::config_show(&loaded, origin),
            ConfigCmd::Schema { dir } => commands::config_schema(&loaded, dir),
            ConfigCmd::Paths => commands::config_paths(&loaded),
        }),
        Cmd::Mcp { cmd } => match cmd {
            cli::McpCmd::Login { id, no_browser } => block_on(mcp::login(&loaded, id, no_browser)),
            cli::McpCmd::Logout { id } => block_on(mcp::logout(&loaded, id)),
            other => code(mcp::run(&loaded, other)),
        },
        Cmd::Doctor => block_on(async move {
            let checks = quack_core::doctor::run(Arc::new(loaded)).await;
            commands::doctor(&checks)
        }),
        Cmd::Setup => block_on(async move {
            let stdin = std::io::stdin();
            let mut io = setup::Io {
                input: &mut stdin.lock(),
                out: &mut std::io::stdout(),
                secret: &commands::read_secret,
                browser: &quack_core::setup::open_sign_in_browser,
            };
            setup::run(Arc::new(loaded), &mut io).await
        }),
        Cmd::Catalog {
            cmd: CatalogCmd::Refresh { out },
        } => block_on(commands::catalog_refresh(&loaded, out)),
        Cmd::Update => block_on(update::run(&loaded)),
        Cmd::Sessions { cmd } => code(match cmd {
            None => commands::sessions(&loaded),
            Some(SessionsCmd::Prune {
                older_than,
                all,
                dry_run,
            }) => commands::sessions_prune(&loaded, older_than.as_deref(), all, dry_run),
        }),
        Cmd::Trust { decision } => code(commands::trust(&loaded, decision.map(Into::into))),
        Cmd::Keyring { cmd } => code(match cmd {
            KeyringCmd::Set { name } => commands::keyring_set(&name),
            KeyringCmd::Delete { name } => commands::keyring_delete(&name),
        }),
        #[cfg(feature = "auth-chatgpt")]
        Cmd::Login {
            provider,
            new,
            no_browser,
        } => block_on(commands::login(&loaded, provider, new, no_browser)),
        #[cfg(feature = "auth-chatgpt")]
        Cmd::Logout { provider } => block_on(commands::logout(&loaded, provider)),
        Cmd::Eval(args) => block_on(commands::eval(&loaded, opts, args)),
        Cmd::Gui(args) => code(launch_gui(
            &loaded.dirs.workspace_root,
            cli.global.cd.is_some(),
            cli.global.resume.as_deref(),
            cli.global.continue_,
            &args,
        )),
        #[cfg(feature = "acp")]
        Cmd::Acp => block_on(async move {
            // Each ACP session loads the config of its own folder, with this command
            // line's model and settings.
            let spawn = move |start: quack_acp::SessionStart| {
                let mut o = opts.clone();
                o.cwd = start.cwd;
                o.mcp = start.mcp;
                let loaded = quack_config::load(&o)
                    .map_err(|e| quack_acp::SessionError::Other(e.to_string()))?;
                let engine =
                    Engine::spawn_with_bridge(Arc::new(loaded), start.resume, start.client_bridge);
                if plan {
                    let _ = engine.ops.try_send(plan_op());
                }
                Ok(engine)
            };
            let stdin = tokio::io::stdin();
            let stdout = tokio::io::stdout();
            match quack_acp::serve(stdin, stdout, Arc::new(spawn)).await {
                Ok(()) => exit::OK,
                Err(e) => {
                    tracing::error!(%e, "acp connection failed");
                    exit::AGENT_ERROR
                }
            }
        }),
        Cmd::Tui => block_on(async move {
            let config = Arc::new(loaded);
            let engine = Engine::spawn_with(config.clone(), resume);
            if plan {
                engine.submit(plan_op()).await;
            }
            let trace = cli
                .global
                .trace_startup
                .map(|path| quack_tui::StartupTrace {
                    started,
                    config_loaded,
                    path,
                });
            match quack_tui::run(engine, config, trace).await {
                Ok(resume_id) => {
                    if let Some(id) = resume_id {
                        println!("resume with: quack -s {id}");
                    }
                    exit::OK
                }
                Err(e) => {
                    eprintln!("error: terminal: {e}");
                    exit::AGENT_ERROR
                }
            }
        }),
        Cmd::Exec(args) => {
            let prompt = match commands::read_prompt(&args.prompt) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("error: reading prompt from stdin: {e}");
                    return code(exit::AGENT_ERROR);
                }
            };
            block_on(async move {
                let engine = Engine::spawn_with(Arc::new(loaded), resume);
                if plan {
                    engine.submit(plan_op()).await;
                }
                quack_exec::run(engine, prompt, args.output_format.into()).await
            })
        }
    }
}

/// A TOML string literal (a JSON string is one).
fn toml_string(s: &str) -> String {
    serde_json::Value::String(s.to_owned()).to_string()
}

fn plan_op() -> quack_core::protocol::Op {
    quack_core::protocol::Op::SetPermissionMode {
        mode: quack_core::protocol::PermissionMode::Plan,
    }
}

fn block_on(fut: impl Future<Output = i32>) -> ExitCode {
    match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => code(rt.block_on(fut)),
        Err(e) => {
            eprintln!("error: cannot start async runtime: {e}");
            code(exit::AGENT_ERROR)
        }
    }
}

fn launch_gui(
    workspace_root: &std::path::Path,
    has_cd: bool,
    resume: Option<&str>,
    continue_latest: bool,
    args: &cli::GuiArgs,
) -> i32 {
    let Some(bin) = quack_core::gui::find_binary() else {
        eprintln!(
            "error: quack-gui binary not found in PATH or beside quack\n  help: build the desktop shell with `cargo build -p quack-gui --features desktop`"
        );
        return exit::AGENT_ERROR;
    };
    let mut paths: Vec<std::path::PathBuf> = args
        .paths
        .iter()
        .map(|p| {
            let joined = if p.is_relative() {
                workspace_root.join(p)
            } else {
                p.clone()
            };
            std::path::absolute(&joined).unwrap_or(joined)
        })
        .collect();
    if paths.is_empty() && (has_cd || resume.is_some() || continue_latest) {
        paths.push(
            std::path::absolute(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf()),
        );
    }
    let spawn_args = quack_core::gui::build_args(&paths, resume, continue_latest, args.no_restore);
    if let Err(e) = quack_core::gui::spawn(&bin, &spawn_args, Some(workspace_root)) {
        eprintln!("error: could not launch {}: {e}", bin.display());
        return exit::AGENT_ERROR;
    }
    exit::OK
}

fn code(c: i32) -> ExitCode {
    ExitCode::from(u8::try_from(c).unwrap_or(1))
}
