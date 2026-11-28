//! `quack mcp` — list, add, enable, disable, and remove MCP servers cleanly from the CLI.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};

use quack_config::{
    Loaded, McpSummary, NewMcpServer, NewMcpTransport, Secret, check_mcp_id,
    collect_secret_env_vars, derive_mcp_id_from_command, derive_mcp_id_from_url,
    split_command_words,
};

use crate::cli::{McpAddArgs, McpCmd};

pub(crate) struct McpIo<'a> {
    pub input: &'a mut dyn BufRead,
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
    pub env_exists: &'a dyn Fn(&str) -> bool,
}

pub(crate) fn run(loaded: &Loaded, cmd: McpCmd) -> i32 {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    let env_exists = |name: &str| std::env::var_os(name).is_some();
    let mut io = McpIo {
        input: &mut input,
        out: &mut out,
        err: &mut err,
        env_exists: &env_exists,
    };
    run_with_io(loaded, cmd, &mut io)
}

pub(crate) fn run_with_io(loaded: &Loaded, cmd: McpCmd, io: &mut McpIo<'_>) -> i32 {
    let result = match cmd {
        McpCmd::List => cmd_list(loaded, io),
        McpCmd::Add(args) => cmd_add(loaded, args, io),
        McpCmd::Enable { id } => cmd_enable(loaded, &id, io),
        McpCmd::Disable { id } => cmd_disable(loaded, &id, io),
        McpCmd::Remove { id, project } => cmd_remove(loaded, &id, project, io),
        McpCmd::Login { .. } | McpCmd::Logout { .. } => {
            Err("login/logout run asynchronously via `quack mcp login|logout`".into())
        }
    };
    match result {
        Ok(()) => 0,
        Err(msg) => {
            let _ = writeln!(io.err, "error: {msg}");
            1
        }
    }
}

pub(crate) async fn login(loaded: &Loaded, id: String, no_browser: bool) -> i32 {
    let client = match quack_core::mcp_oauth_client(loaded, &id, None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return quack_exec::exit::CONFIG_ERROR;
        }
    };
    let open = |url: &str| {
        eprintln!(
            "Sign in to MCP server `{id}` in your browser. If it does not open, visit:\n\n  {url}\n"
        );
        if !no_browser {
            quack_core::setup::open_sign_in_browser(url);
        }
    };
    match client.sign_in(open).await {
        Ok(done) => {
            if done.scopes.is_empty() {
                eprintln!("signed in to MCP server `{}`.", done.server);
            } else {
                eprintln!(
                    "signed in to MCP server `{}` (scopes: {}).",
                    done.server,
                    done.scopes.join(" ")
                );
            }
            quack_exec::exit::OK
        }
        Err(e) => {
            eprintln!("error: {e}");
            quack_exec::exit::CONFIG_ERROR
        }
    }
}

pub(crate) async fn logout(loaded: &Loaded, id: String) -> i32 {
    let client = match quack_core::mcp_oauth_client(loaded, &id, None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return quack_exec::exit::CONFIG_ERROR;
        }
    };
    match client.sign_out().await {
        Ok(true) => {
            eprintln!("signed out of MCP server `{id}`.");
            quack_exec::exit::OK
        }
        Ok(false) => {
            eprintln!(
                "removed local OAuth credentials for `{id}`, but the server did not confirm revocation."
            );
            quack_exec::exit::OK
        }
        Err(e) => {
            eprintln!("error: {e}");
            quack_exec::exit::CONFIG_ERROR
        }
    }
}

fn cmd_list(loaded: &Loaded, io: &mut McpIo<'_>) -> Result<(), String> {
    let items = loaded.mcp_summaries();
    if items.is_empty() {
        writeln_str(
            io.out,
            "No MCP servers configured and no bundled examples found.",
        )?;
        writeln_str(io.out, "Add one with: quack mcp add")?;
        return Ok(());
    }

    let enabled: Vec<&McpSummary> = items.iter().filter(|i| i.enabled).collect();
    let disabled: Vec<&McpSummary> = items
        .iter()
        .filter(|i| !i.enabled && i.parse_error.is_none())
        .collect();
    let failed: Vec<&McpSummary> = items.iter().filter(|i| i.parse_error.is_some()).collect();

    if !enabled.is_empty() {
        writeln_str(io.out, "Enabled MCP servers:")?;
        for item in &enabled {
            print_summary_row(io, item, "ok")?;
        }
    } else {
        writeln_str(io.out, "Enabled MCP servers: none")?;
    }

    if !disabled.is_empty() {
        if !enabled.is_empty() {
            writeln_str(io.out, "")?;
        }
        writeln_str(io.out, "Disabled / bundled examples:")?;
        for item in &disabled {
            print_summary_row(io, item, "off")?;
        }
    }

    if !failed.is_empty() {
        writeln_str(io.out, "")?;
        writeln_str(io.out, "Invalid MCP server files:")?;
        for item in &failed {
            print_summary_row(io, item, "err")?;
        }
    }

    writeln_str(io.out, "")?;
    writeln_str(
        io.out,
        "Enable an example: quack mcp enable <id>   |   Add a server: quack mcp add",
    )?;
    Ok(())
}

fn print_summary_row(io: &mut McpIo<'_>, item: &McpSummary, tag: &str) -> Result<(), String> {
    let transport = if item.transport.is_empty() {
        "-"
    } else {
        item.transport.as_str()
    };
    let target = if let Some(err) = &item.parse_error {
        err.as_str()
    } else if item.target.is_empty() {
        "-"
    } else {
        item.target.as_str()
    };
    writeln_str(
        io.out,
        &format!(
            "  {tag:<4} {:<14} {:<8} {:<6} {target}",
            item.id,
            item.layer.label(),
            transport
        ),
    )?;
    if !item.description.is_empty() {
        writeln_str(io.out, &format!("       {}", item.description))?;
    }
    let missing_env: Vec<&str> = item
        .env_vars
        .iter()
        .map(String::as_str)
        .filter(|v| !(io.env_exists)(v))
        .collect();
    if item.enabled && !missing_env.is_empty() {
        writeln_str(
            io.out,
            &format!("       needs env: {}", missing_env.join(", ")),
        )?;
    }
    Ok(())
}

fn cmd_enable(loaded: &Loaded, id: &str, io: &mut McpIo<'_>) -> Result<(), String> {
    let out = loaded.enable_mcp(id)?;
    writeln_str(io.out, &format!("{} ({})", out.action, out.file.display()))?;
    print_missing_env_note(io, &out.env_vars)?;
    writeln_str(
        io.out,
        "Run `quack doctor` or open `/mcp` inside quack to verify it connects.",
    )?;
    Ok(())
}

fn cmd_disable(loaded: &Loaded, id: &str, io: &mut McpIo<'_>) -> Result<(), String> {
    let out = loaded.disable_mcp(id)?;
    writeln_str(io.out, &format!("{} ({})", out.action, out.path.display()))?;
    Ok(())
}

fn cmd_remove(loaded: &Loaded, id: &str, project: bool, io: &mut McpIo<'_>) -> Result<(), String> {
    let removed = loaded.remove_mcp(id, project.then_some(true))?;
    writeln_str(io.out, &format!("removed {}", removed.display()))?;
    Ok(())
}

fn cmd_add(loaded: &Loaded, args: McpAddArgs, io: &mut McpIo<'_>) -> Result<(), String> {
    if args.url.is_some() && !args.command.is_empty() {
        return Err(
            "set either --url (remote HTTP) or -- <command>... (local stdio), not both".into(),
        );
    }

    let interactive = args.url.is_none() && args.command.is_empty();
    let spec = if interactive {
        prompt_new_mcp(args, io)?
    } else {
        build_spec_from_args(args)?
    };

    let path = loaded.add_mcp_server(&spec)?;
    writeln_str(
        io.out,
        &format!("added MCP server `{}` -> {}", spec.id, path.display()),
    )?;

    let env_vars = match &spec.transport {
        NewMcpTransport::Stdio { env, .. } => collect_secret_env_vars(env.values()),
        NewMcpTransport::Http { headers, .. } => collect_secret_env_vars(headers.values()),
    };
    print_missing_env_note(io, &env_vars)?;
    writeln_str(
        io.out,
        "Run `quack doctor` or open `/mcp` inside quack to check its connection.",
    )?;
    Ok(())
}

fn build_spec_from_args(args: McpAddArgs) -> Result<NewMcpServer, String> {
    let mut headers = BTreeMap::new();
    for raw in &args.headers {
        let (k, v) = parse_header(raw)?;
        headers.insert(k, v);
    }
    let mut env = BTreeMap::new();
    for raw in &args.envs {
        let (k, v) = parse_env(raw)?;
        env.insert(k, v);
    }

    let (derived_id, transport) = if let Some(url) = args.url {
        if !env.is_empty() || args.sandbox || args.cwd.is_some() {
            return Err("--env, --sandbox, and --cwd only apply to local stdio servers".into());
        }
        let d = derive_mcp_id_from_url(&url);
        (d, NewMcpTransport::Http { url, headers })
    } else {
        if !headers.is_empty() {
            return Err("--header only applies to remote HTTP servers (`--url`)".into());
        }
        let mut iter = args.command.into_iter();
        let command = iter
            .next()
            .filter(|c| !c.trim().is_empty())
            .ok_or("missing command after `--`")?;
        let cmd_args: Vec<String> = iter.collect();
        let d = derive_mcp_id_from_command(&command, &cmd_args);
        (
            d,
            NewMcpTransport::Stdio {
                command,
                args: cmd_args,
                env,
                cwd: args.cwd,
                sandbox: args.sandbox.then_some(true),
            },
        )
    };

    let id = match args.id {
        Some(given) => given,
        None => derived_id.ok_or(
            "could not derive a server id from the target; pass one explicitly (`quack mcp add <id> ...`)",
        )?,
    };
    check_mcp_id(&id)?;

    Ok(NewMcpServer {
        id,
        description: args.description,
        transport,
        auto_approve: args
            .auto_approve
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect(),
        startup_timeout_secs: args.startup_timeout_secs,
        project: args.project,
        overwrite: args.force,
    })
}

fn prompt_new_mcp(args: McpAddArgs, io: &mut McpIo<'_>) -> Result<NewMcpServer, String> {
    writeln_str(io.out, "Add an MCP server")?;
    writeln_str(io.out, "  1. Local command (stdio: npx, uvx, binary)")?;
    writeln_str(io.out, "  2. Remote URL (HTTP endpoint)")?;
    let kind = ask(io, "Transport [1/2]: ")?;
    let is_http = match kind.trim().to_ascii_lowercase().as_str() {
        "2" | "http" | "remote" | "url" => true,
        "1" | "stdio" | "local" | "cmd" => false,
        other => {
            return Err(format!(
                "expected `1` (local stdio) or `2` (remote HTTP), got `{other}`"
            ));
        }
    };

    let (derived_id, mut transport) = if is_http {
        let url = ask(io, "MCP server URL (https://...): ")?;
        if url.trim().is_empty() {
            return Err("URL cannot be empty".into());
        }
        let mut headers = BTreeMap::new();
        for raw in &args.headers {
            let (k, v) = parse_header(raw)?;
            headers.insert(k, v);
        }
        let bearer_env = ask(
            io,
            "Bearer token env var (optional, e.g. ROOST_TOKEN; Enter to skip): ",
        )?;
        if !bearer_env.trim().is_empty() {
            headers.insert(
                "Authorization".into(),
                Secret::Env {
                    env: bearer_env.trim().to_owned(),
                    format: Some("Bearer {}".into()),
                },
            );
        }
        let d = derive_mcp_id_from_url(url.trim());
        (
            d,
            NewMcpTransport::Http {
                url: url.trim().to_owned(),
                headers,
            },
        )
    } else {
        let line = ask(
            io,
            "Command and args (e.g. npx -y @playwright/mcp@latest): ",
        )?;
        let parts = split_command_words(&line)?;
        let mut iter = parts.into_iter();
        let command = iter.next().ok_or("command cannot be empty")?;
        let cmd_args: Vec<String> = iter.collect();
        let mut env = BTreeMap::new();
        for raw in &args.envs {
            let (k, v) = parse_env(raw)?;
            env.insert(k, v);
        }
        let env_line = ask(
            io,
            "Env vars (comma-separated VAR or VAR=VALUE, Enter to skip): ",
        )?;
        for entry in env_line.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (k, v) = parse_env(entry)?;
            env.insert(k, v);
        }
        let d = derive_mcp_id_from_command(&command, &cmd_args);
        (
            d,
            NewMcpTransport::Stdio {
                command,
                args: cmd_args,
                env,
                cwd: args.cwd,
                sandbox: args.sandbox.then_some(true),
            },
        )
    };

    // Preserve any `--header` flags passed even before interactive prompts.
    let _ = &mut transport;

    let id = match args.id {
        Some(given) => given,
        None => {
            let prompt = match &derived_id {
                Some(d) => format!("Server id [{d}]: "),
                None => "Server id (letters, digits, `-`, `_`): ".into(),
            };
            let raw = ask(io, &prompt)?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                derived_id.ok_or("server id cannot be empty")?
            } else {
                trimmed.to_owned()
            }
        }
    };
    check_mcp_id(&id)?;

    let description = match args.description {
        Some(d) => Some(d),
        None => {
            let raw = ask(io, "Description (optional, Enter to skip): ")?;
            (!raw.trim().is_empty()).then(|| raw.trim().to_owned())
        }
    };

    let auto_approve = if !args.auto_approve.is_empty() {
        args.auto_approve
    } else {
        let choice = ask(
            io,
            "Auto-approve tools? [n] Ask each time (default), [r] Read-only, [a] All (*): ",
        )?;
        match choice.trim().to_ascii_lowercase().as_str() {
            "" | "n" | "no" | "ask" => Vec::new(),
            "r" | "read" | "readonly" | "read-only" => vec![
                "list_*".into(),
                "search_*".into(),
                "get_*".into(),
                "read_*".into(),
            ],
            "a" | "all" | "*" => vec!["*".into()],
            other => {
                return Err(format!("expected `n`, `r`, or `a`, got `{other}`"));
            }
        }
    };

    Ok(NewMcpServer {
        id,
        description,
        transport,
        auto_approve,
        startup_timeout_secs: args.startup_timeout_secs,
        project: args.project,
        overwrite: args.force,
    })
}

fn print_missing_env_note(io: &mut McpIo<'_>, env_vars: &[String]) -> Result<(), String> {
    let missing: Vec<&str> = env_vars
        .iter()
        .map(String::as_str)
        .filter(|v| !(io.env_exists)(v))
        .collect();
    if !missing.is_empty() {
        writeln_str(
            io.out,
            &format!(
                "Note: set {} in your environment before running quack (or switch to keyring/file in the TOML).",
                missing.join(", ")
            ),
        )?;
    }
    Ok(())
}

fn ask(io: &mut McpIo<'_>, prompt: &str) -> Result<String, String> {
    write!(io.out, "{prompt}").map_err(|e| e.to_string())?;
    io.out.flush().map_err(|e| e.to_string())?;
    let mut line = String::new();
    let n = io.input.read_line(&mut line).map_err(|e| e.to_string())?;
    if n == 0 && line.is_empty() {
        return Err("unexpected end of input".into());
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}

fn writeln_str(out: &mut dyn Write, text: &str) -> Result<(), String> {
    writeln!(out, "{text}").map_err(|e| e.to_string())
}

/// Parse `Name: Value` or `Name=Value`, supporting `env:VAR`, `Bearer {env:VAR}`,
/// `keyring:NAME`, and `file:PATH` secret references.
pub(crate) fn parse_header(raw: &str) -> Result<(String, Secret), String> {
    let (name, val) = split_key_value(raw, true)
        .ok_or_else(|| format!("header `{raw}` must be `Name: Value` or `Name=Value`"))?;
    if name.is_empty() {
        return Err(format!("header `{raw}` has an empty name"));
    }
    let secret = parse_secret_value(val)?;
    Ok((name.to_owned(), secret))
}

/// Parse `KEY=VALUE` or `KEY` (forward `KEY` from host environment).
pub(crate) fn parse_env(raw: &str) -> Result<(String, Secret), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("empty env specification".into());
    }
    if let Some((name, val)) = split_key_value(trimmed, false) {
        if name.is_empty() {
            return Err(format!("env `{raw}` has an empty variable name"));
        }
        Ok((name.to_owned(), parse_secret_value(val)?))
    } else {
        Ok((
            trimmed.to_owned(),
            Secret::Env {
                env: trimmed.to_owned(),
                format: None,
            },
        ))
    }
}

fn split_key_value(raw: &str, allow_colon: bool) -> Option<(&str, &str)> {
    let eq = raw.find('=');
    let col = allow_colon.then(|| raw.find(':')).flatten();
    let idx = match (eq, col) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) | (None, Some(a)) => a,
        (None, None) => return None,
    };
    let name = raw[..idx].trim();
    let val = raw[idx + 1..].trim();
    Some((name, val))
}

fn parse_secret_value(val: &str) -> Result<Secret, String> {
    if let Some(env_name) = val.strip_prefix("env:") {
        let env_name = env_name.trim();
        if env_name.is_empty() {
            return Err("empty variable name after `env:`".into());
        }
        return Ok(Secret::Env {
            env: env_name.to_owned(),
            format: None,
        });
    }
    if let Some(keyring_name) = val.strip_prefix("keyring:") {
        let keyring_name = keyring_name.trim();
        if keyring_name.is_empty() {
            return Err("empty entry name after `keyring:`".into());
        }
        return Ok(Secret::Keyring {
            keyring: keyring_name.to_owned(),
        });
    }
    if let Some(path) = val.strip_prefix("file:") {
        let path = path.trim();
        if path.is_empty() {
            return Err("empty path after `file:`".into());
        }
        return Ok(Secret::File {
            file: path.to_owned(),
        });
    }
    if let Some(start) = val.find("{env:")
        && let Some(end_rel) = val[start + 5..].find('}')
    {
        let env_name = val[start + 5..start + 5 + end_rel].trim();
        if env_name.is_empty() {
            return Err("empty variable name in `{env:...}`".into());
        }
        let format = format!("{}{{}}{}", &val[..start], &val[start + 5 + end_rel + 1..]);
        return Ok(Secret::Env {
            env: env_name.to_owned(),
            format: (format != "{}").then_some(format),
        });
    }
    Ok(Secret::Plain(val.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use quack_config::{LoadOptions, load_with};
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::io::Cursor;

    struct Fixture {
        _tmp: tempfile::TempDir,
        root: std::path::PathBuf,
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
    fn parses_headers_and_envs() {
        assert_eq!(
            parse_header("Authorization: Bearer {env:ROOST_TOKEN}").expect("parse"),
            (
                "Authorization".into(),
                Secret::Env {
                    env: "ROOST_TOKEN".into(),
                    format: Some("Bearer {}".into()),
                }
            )
        );
        assert_eq!(
            parse_env("OPENAI_KEY").expect("parse"),
            (
                "OPENAI_KEY".into(),
                Secret::Env {
                    env: "OPENAI_KEY".into(),
                    format: None,
                }
            )
        );
        assert_eq!(
            parse_env("TOKEN=keyring:quack/tok").expect("parse"),
            (
                "TOKEN".into(),
                Secret::Keyring {
                    keyring: "quack/tok".into(),
                }
            )
        );
    }

    #[test]
    fn derives_sensible_ids_from_url_and_command() {
        assert_eq!(
            derive_mcp_id_from_url("https://mcp.context7.com/mcp").as_deref(),
            Some("context7")
        );
        assert_eq!(
            derive_mcp_id_from_url("https://roost.internal:8484/mcp").as_deref(),
            Some("roost")
        );
        assert_eq!(
            derive_mcp_id_from_command("npx", &["-y".into(), "@upstash/context7-mcp".into()])
                .as_deref(),
            Some("context7")
        );
        assert_eq!(
            derive_mcp_id_from_command("npx", &["-y".into(), "@playwright/mcp@latest".into()])
                .as_deref(),
            Some("playwright")
        );
        assert_eq!(
            derive_mcp_id_from_command(
                "npx",
                &[
                    "-y".into(),
                    "@modelcontextprotocol/server-filesystem".into()
                ]
            )
            .as_deref(),
            Some("filesystem")
        );
        assert_eq!(
            derive_mcp_id_from_command("uvx", &["mcp-server-fetch".into()]).as_deref(),
            Some("fetch")
        );
    }

    #[test]
    fn non_interactive_add_list_disable_enable_remove_flow() {
        let f = Fixture::new();
        f.write(
            "defaults/mcp.d/roost.toml",
            "enabled = false\n\
             description = \"Roost search\"\n\
             url = \"https://roost.internal:8484/mcp\"\n\
             headers = { Authorization = { env = \"ROOST_TOKEN\", format = \"Bearer {}\" } }\n",
        );

        let run = |cmd: McpCmd, stdin: &str| -> (i32, String, String) {
            let mut input = Cursor::new(stdin.as_bytes());
            let mut out = Vec::new();
            let mut err = Vec::new();
            let env_exists = |_: &str| false;
            let mut io = McpIo {
                input: &mut input,
                out: &mut out,
                err: &mut err,
                env_exists: &env_exists,
            };
            let code = run_with_io(&f.load(), cmd, &mut io);
            (
                code,
                String::from_utf8(out).expect("utf8"),
                String::from_utf8(err).expect("utf8"),
            )
        };

        // List shows bundled example before enabling.
        let (code, out, err) = run(McpCmd::List, "");
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("roost"), "{out}");
        assert!(out.contains("Roost search"), "{out}");

        // Enable bundled roost and remind about ROOST_TOKEN.
        let (code, out, err) = run(McpCmd::Enable { id: "roost".into() }, "");
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("ROOST_TOKEN"), "{out}");

        // Add custom HTTP server via flags without an explicit id.
        let (code, out, err) = run(
            McpCmd::Add(McpAddArgs {
                id: None,
                url: Some("https://mcp.context7.com/mcp".into()),
                headers: vec![],
                envs: vec![],
                description: Some("Library docs".into()),
                auto_approve: vec!["*".into()],
                startup_timeout_secs: None,
                sandbox: false,
                cwd: None,
                project: false,
                force: false,
                command: vec![],
            }),
            "",
        );
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("`context7`"), "{out}");
        assert!(f.root.join("user/mcp.d/context7.toml").is_file());

        // Disable then remove context7.
        let (code, _, err) = run(
            McpCmd::Disable {
                id: "context7".into(),
            },
            "",
        );
        assert_eq!(code, 0, "{err}");
        let (code, _, err) = run(
            McpCmd::Remove {
                id: "context7".into(),
                project: false,
            },
            "",
        );
        assert_eq!(code, 0, "{err}");
        assert!(!f.root.join("user/mcp.d/context7.toml").exists());
    }

    #[test]
    fn interactive_wizard_adds_remote_and_local_servers() {
        let f = Fixture::new();
        let run_add = |stdin: &str| -> (i32, String, String) {
            let mut input = Cursor::new(stdin.as_bytes());
            let mut out = Vec::new();
            let mut err = Vec::new();
            let env_exists = |_: &str| false;
            let mut io = McpIo {
                input: &mut input,
                out: &mut out,
                err: &mut err,
                env_exists: &env_exists,
            };
            let code = run_with_io(
                &f.load(),
                McpCmd::Add(McpAddArgs {
                    id: None,
                    url: None,
                    headers: vec![],
                    envs: vec![],
                    description: None,
                    auto_approve: vec![],
                    startup_timeout_secs: None,
                    sandbox: false,
                    cwd: None,
                    project: false,
                    force: false,
                    command: vec![],
                }),
                &mut io,
            );
            (
                code,
                String::from_utf8(out).expect("utf8"),
                String::from_utf8(err).expect("utf8"),
            )
        };

        // Guided remote HTTP flow.
        let (code, out, err) =
            run_add("2\nhttps://roost.internal:8484/mcp\nROOST_TOKEN\n\nDoc search\nr\n");
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("ROOST_TOKEN"), "{out}");
        let roost = std::fs::read_to_string(f.root.join("user/mcp.d/roost.toml")).expect("read");
        assert!(roost.contains("https://roost.internal:8484/mcp"), "{roost}");
        assert!(roost.contains("ROOST_TOKEN"), "{roost}");
        assert!(roost.contains("auto_approve"), "{roost}");

        // Guided local stdio flow.
        let (code, _, err) =
            run_add("1\nnpx -y @playwright/mcp@latest\n\nplaywright\nBrowser\nn\n");
        assert_eq!(code, 0, "{err}");
        let pw = std::fs::read_to_string(f.root.join("user/mcp.d/playwright.toml")).expect("read");
        assert!(pw.contains("command = \"npx\""), "{pw}");
        assert!(pw.contains("@playwright/mcp@latest"), "{pw}");
    }
}
