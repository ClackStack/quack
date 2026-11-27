//! Extension modals (/bundles, /hooks, /skills, /instructions, /profiles, /commands, /mcp, /lsp).

use super::*;

/// The `/skills` modal: installed skills; ⏎ puts `/<name> ` in the input.
pub(crate) fn bundles(bundles: &[quack_config::Bundle], scope: WriteScope, theme: &Theme) -> Modal {
    let target = scope_file(scope);
    let flag = scope_flag(scope);
    let items = if bundles.is_empty() {
        vec![placeholder(
            "no bundles",
            "A bundle is a folder with a bundle.toml that groups skills, commands and \
             mcp.d, hooks.d or profiles.d files. Copy one into bundles/ in your config folder \
             (`quack config paths` shows where) or a trusted project's .quack/.",
            theme,
        )]
    } else {
        bundles
            .iter()
            .map(|b| {
                let name = if b.file.name.is_empty() {
                    b.id.clone()
                } else {
                    b.file.name.clone()
                };
                let (state, style) = if b.enabled {
                    ("on", theme.ok)
                } else {
                    ("off", theme.dim)
                };
                let mut detail = vec![Line::styled(name.clone(), style), Line::raw("")];
                if !b.file.description.is_empty() {
                    detail.push(Line::raw(b.file.description.clone()));
                    detail.push(Line::raw(""));
                }
                detail.push(field(theme, "state", state.to_owned()));
                if !b.file.version.is_empty() {
                    detail.push(field(theme, "version", b.file.version.clone()));
                }
                detail.push(field(theme, "holds", b.contents.clone()));
                detail.push(field(theme, "layer", b.layer.to_string()));
                detail.push(field(theme, "folder", b.dir.display().to_string()));
                detail.push(Line::raw(""));
                detail.push(Line::styled(
                    format!(
                        "⏎ turn it {} (saved to [bundles] disabled in {target})",
                        if b.enabled { "off" } else { "on" }
                    ),
                    theme.dim,
                ));
                ModalItem {
                    label: name,
                    hint: format!("{state} · {}", b.layer),
                    detail,
                    action: Some(format!("/bundles toggle{flag} {}", b.id)),
                    run: true,
                    alt: None,
                    charts: Vec::new(),
                }
            })
            .collect()
    };
    Modal::new(
        "Bundles",
        vec![ModalTab {
            title: "Bundles".into(),
            items,
        }],
    )
    .with_scope(scope)
}

/// The `/hooks` modal: configured hooks from `hooks.d` and their recent runs.
pub(crate) fn hook_payload(id: &str, path: &str, theme: &Theme) -> Modal {
    let items = [
        ("Keep existing folder", "keep-payload"),
        ("Replace with bundled folder", "replace-payload"),
    ]
    .into_iter()
    .map(|(label, action)| {
        let mut item = placeholder(
            label,
            &format!("An existing payload for `{id}` was found at {path}."),
            theme,
        );
        item.action = Some(format!("/hooks {action} {id}"));
        item.run = true;
        item
    })
    .collect();
    Modal::new(
        "Hook payload",
        vec![ModalTab {
            title: "Hook payload".into(),
            items,
        }],
    )
}

pub(crate) fn hooks(
    hooks: &[quack_config::Item<quack_config::HookFile>],
    runs: &[crate::app::HookRun],
    scope: WriteScope,
    theme: &Theme,
) -> Modal {
    use quack_config::{ItemState, LayerKind};
    let target = scope_file(scope);
    let flag = scope_flag(scope);
    let items = if hooks.is_empty() {
        vec![placeholder(
            "no hooks",
            "Add a TOML file to hooks.d in your config folder (`quack config paths` shows \
             where) or a trusted project's .quack/hooks.d, for example\n\n\
             event = \"pre_tool\"\nmatch = { tool = \"shell\" }\ncommand = [\"python3\", \"check.py\"]",
            theme,
        )]
    } else {
        hooks
            .iter()
            .map(|h| {
                let hook_runs: Vec<&crate::app::HookRun> =
                    runs.iter().filter(|r| r.id == h.id).collect();
                let last = hook_runs.last().copied();
                let mut detail = Vec::new();
                let (hint, can_toggle, enabled) = match &h.state {
                    ItemState::Loaded(f) if f.enabled => {
                        detail.push(Line::styled(h.id.clone(), theme.ok));
                        detail.push(Line::raw(""));
                        if !f.description.is_empty() {
                            detail.push(Line::raw(f.description.clone()));
                            detail.push(Line::raw(""));
                        }
                        detail.push(field(theme, "state", "on".into()));
                        detail.push(field(theme, "event", f.event.label().into()));
                        detail.push(field(theme, "command", f.command.join(" ")));
                        if !f.matcher.tool.is_empty() {
                            detail.push(field(theme, "tool", f.matcher.tool.clone()));
                        }
                        if !f.matcher.pattern.is_empty() {
                            detail.push(field(theme, "pattern", f.matcher.pattern.clone()));
                        }
                        if !f.matcher.prompt.is_empty() {
                            detail.push(field(theme, "prompt", f.matcher.prompt.clone()));
                        }
                        let hint = match last {
                            Some(r) => format!("{} · {} ms", f.event.label(), r.ms),
                            None => format!("{} · {}", f.event.label(), h.layer.label()),
                        };
                        (hint, h.layer != LayerKind::System, true)
                    }
                    ItemState::Loaded(f) => {
                        detail.push(Line::styled(h.id.clone(), theme.dim));
                        detail.push(Line::raw(""));
                        detail.push(field(theme, "state", "disabled in file".into()));
                        detail.push(field(theme, "event", f.event.label().into()));
                        detail.push(field(theme, "command", f.command.join(" ")));
                        ("off".into(), h.layer != LayerKind::System, false)
                    }
                    ItemState::Disabled(why) => {
                        detail.push(Line::styled(h.id.clone(), theme.dim));
                        detail.push(Line::raw(""));
                        detail.push(field(theme, "state", format!("off ({why})")));
                        ("off".into(), h.layer != LayerKind::System, false)
                    }
                    ItemState::Failed(d) => {
                        detail.push(Line::styled(h.id.clone(), theme.error));
                        detail.push(Line::raw(""));
                        detail.push(Line::styled(d.one_line(), theme.error));
                        ("failed".into(), false, false)
                    }
                };
                let action = if can_toggle {
                    if enabled {
                        format!(
                            "{}disables by adding `{}` to [hooks] disabled in {target}.",
                            if matches!(&h.state, ItemState::Loaded(f) if f.setup.is_none()) {
                                "Enter or Ctrl-S "
                            } else {
                                "Enter configures; Ctrl-S "
                            },
                            h.id,
                        )
                    } else if matches!(&h.state, ItemState::Loaded(f) if f.setup.is_none()) {
                        "Enter or Ctrl-S enables this hook.".to_owned()
                    } else {
                        "Enter configures the hook; Ctrl-S enables it.".to_owned()
                    }
                } else {
                    String::new()
                };
                detail.push(field(theme, "layer", h.layer.label().into()));
                detail.push(field(theme, "file", h.path.display().to_string()));
                if !hook_runs.is_empty() {
                    detail.push(Line::raw(""));
                    detail.push(Line::styled("recent runs", theme.dim));
                    let skip = hook_runs.len().saturating_sub(10);
                    for r in hook_runs.into_iter().skip(skip) {
                        let msg = r
                            .message
                            .as_deref()
                            .map(|m| format!(" · {m}"))
                            .unwrap_or_default();
                        detail.push(Line::raw(format!(
                            "  {} → {} ({} ms){msg}",
                            r.on, r.outcome, r.ms
                        )));
                    }
                }
                if can_toggle {
                    detail.push(Line::raw(""));
                    detail.push(Line::styled(action, theme.dim));
                } else if h.layer == LayerKind::System {
                    detail.push(Line::raw(""));
                    detail.push(Line::styled("Enforced by the system layer.", theme.dim));
                }
                ModalItem {
                    label: h.id.clone(),
                    hint,
                    detail,
                    action: can_toggle.then(|| {
                        if matches!(&h.state, ItemState::Loaded(f) if f.setup.is_none()) {
                            format!("/hooks toggle{flag} {}", h.id)
                        } else {
                            format!("/hooks configure {}", h.id)
                        }
                    }),
                    run: can_toggle,
                    alt: can_toggle.then(|| format!("/hooks toggle{flag} {}", h.id)),
                    charts: Vec::new(),
                }
            })
            .collect()
    };
    Modal::new(
        "Hooks",
        vec![ModalTab {
            title: "Hooks".into(),
            items,
        }],
    )
    .with_alt("enable/disable")
    .with_scope(scope)
}

pub(crate) fn compat(folders: Option<&[quack_protocol::CompatFolder]>, theme: &Theme) -> Modal {
    use quack_protocol::CompatKind;
    let mut items: Vec<_> = folders
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, folder)| {
            let path = std::path::Path::new(&folder.path);
            let label = path
                .parent()
                .and_then(|p| p.parent())
                .and_then(|base| path.strip_prefix(base).ok())
                .unwrap_or(path)
                .display()
                .to_string();
            let kind = match folder.kind {
                CompatKind::Skills => "skills",
                CompatKind::Commands => "commands",
            };
            let project = folder.scope == WriteScope::Project;
            let origin = if project { "project" } else { "user" };
            let save = if folder.can_save {
                if project {
                    "^s load and save in .quack/config.toml"
                } else {
                    "^s load and save in user config.toml"
                }
            } else {
                "Saving is unavailable for this project."
            };
            ModalItem {
                label,
                hint: format!("{origin} {kind}"),
                detail: vec![
                    field(theme, "folder", folder.path.clone()),
                    Line::raw(""),
                    Line::raw("Only load prompt files you trust."),
                    Line::raw("Matching names can replace existing ones."),
                    Line::raw(""),
                    Line::raw("⏎ load for this run without saving"),
                    Line::raw(save),
                ],
                action: Some(format!("/compat load {i}")),
                run: true,
                alt: folder.can_save.then(|| format!("/compat save {i}")),
                charts: Vec::new(),
            }
        })
        .collect();
    if items.is_empty() {
        items.push(if folders.is_none() {
            placeholder(
                "Looking for folders",
                "Startup is still checking for other agents’ folders.",
                theme,
            )
        } else {
            placeholder(
                "No new folders",
                "Use /skills or /commands to see what is already loaded.",
                theme,
            )
        });
    }
    let modal = Modal::new(
        "Other agents’ folders",
        vec![ModalTab {
            title: "Folders".into(),
            items,
        }],
    );
    if folders.is_some_and(|folders| folders.iter().any(|f| f.can_save)) {
        modal.with_alt("load and save")
    } else {
        modal
    }
}

pub(crate) fn skills(skills: &[quack_protocol::SkillInfo], theme: &Theme) -> Modal {
    let items = if skills.is_empty() {
        vec![placeholder(
            "no skills",
            "Put a skill folder with a SKILL.md in .agents/skills (this project), \
             ~/.agents/skills or skills/ in your config folder.",
            theme,
        )]
    } else {
        skills
            .iter()
            .map(|s| ModalItem {
                label: s.name.clone(),
                hint: s.source.split(' ').next().unwrap_or_default().to_owned(),
                detail: vec![
                    Line::styled(format!("/{}", s.name), theme.accent),
                    Line::raw(""),
                    Line::raw(s.description.clone()),
                    Line::raw(""),
                    field(theme, "from", s.source.clone()),
                    field(theme, "folder", s.dir.clone()),
                    Line::raw(""),
                    Line::styled(
                        "⏎ run it with a request; the model also loads it when relevant",
                        theme.dim,
                    ),
                ],
                action: Some(format!("/{} ", s.name)),
                run: false,
                alt: None,
                charts: Vec::new(),
            })
            .collect()
    };
    Modal::new(
        "Skills",
        vec![ModalTab {
            title: "Skills".into(),
            items,
        }],
    )
}

/// The `/instructions` modal: the AGENTS.md files the model has, in the system prompt or
/// attached when it first worked in their folder. `None` while asking the engine.
pub(crate) fn instructions(
    files: Option<&[quack_protocol::InstructionFile]>,
    theme: &Theme,
) -> Modal {
    let items = match files {
        None => vec![placeholder("reading…", "", theme)],
        Some([]) => vec![placeholder(
            "no instruction files",
            "None was found. quack reads AGENTS.md (and the other names in \
             `[instructions] files`) from your config folder, then from each folder between \
             the git root and the working directory. Files in subfolders are added when the \
             model first works there. /init writes one for this project.",
            theme,
        )],
        Some(files) => files
            .iter()
            .map(|f| {
                let mut detail = vec![
                    Line::styled(f.source.clone(), theme.accent),
                    Line::raw(""),
                    field(
                        theme,
                        "where",
                        if f.in_prompt {
                            "in the system prompt".into()
                        } else {
                            "added to a tool result in its folder".into()
                        },
                    ),
                    field(theme, "size", format!("~{} tokens", f.tokens)),
                    Line::raw(""),
                ];
                detail.extend(f.text.lines().map(|l| Line::raw(l.to_owned())));
                ModalItem {
                    label: f.source.clone(),
                    hint: crate::theme::tokens(f.tokens),
                    detail,
                    action: None,
                    run: false,
                    alt: None,
                    charts: Vec::new(),
                }
            })
            .collect(),
    };
    Modal::new(
        "Instructions",
        vec![ModalTab {
            title: "Instructions".into(),
            items,
        }],
    )
}

/// What `/profiles` shows: the profile in use, whether it was chosen for the session,
/// and every installed profile.
pub(crate) type ProfileList<'a> = (&'a str, bool, &'a [quack_protocol::ProfileInfo]);

/// The `/profiles` modal: each profile previewed for the current model. `None` while
/// asking the engine.
pub(crate) fn profiles(list: Option<ProfileList<'_>>, scope: WriteScope, theme: &Theme) -> Modal {
    let target = scope_file(scope);
    let flag = scope_flag(scope);
    let Some((active, forced, profiles)) = list else {
        return Modal::new(
            "Profiles",
            vec![ModalTab {
                title: "Profiles".into(),
                items: vec![placeholder("rendering profiles…", "", theme)],
            }],
        )
        .with_alt("make default")
        .with_scope(scope);
    };
    let mut items: Vec<ModalItem> = profiles
        .iter()
        .map(|p| {
            let hint = match (p.id == active, p.matches) {
                (true, _) => "in use".into(),
                (false, true) => "auto pick".into(),
                _ => String::new(),
            };
            let mut detail = vec![
                Line::styled(p.id.clone(), theme.accent),
                Line::raw(""),
                Line::raw(p.description.clone()),
                Line::raw(""),
                field(theme, "source", p.source.clone()),
                field(theme, "edits", p.edit_format.replace('_', " ")),
                field(theme, "tools", p.tools.join(", ")),
                Line::raw(""),
                Line::styled(
                    format!("⏎ use for session · ^s set default ({target})"),
                    theme.dim,
                ),
                Line::raw(""),
                Line::styled("System prompt, rendered for this model", theme.dim),
            ];
            detail.extend(p.system.lines().map(|l| Line::raw(l.to_owned())));
            ModalItem {
                label: p.id.clone(),
                hint,
                detail,
                action: Some(format!("/profiles {}", p.id)),
                run: true,
                alt: Some(format!("/profiles default{flag} {}", p.id)),
                charts: Vec::new(),
            }
        })
        .collect();
    items.push(ModalItem {
        label: "auto".into(),
        hint: if forced { String::new() } else { "on".into() },
        detail: vec![
            Line::styled("auto", theme.accent),
            Line::raw(""),
            Line::raw(
                "Each model's own profile: the one its provider file names, else the \
                 best `[match]` for its id, else `generic`.",
            ),
            Line::raw(""),
            Line::styled("⏎ go back to it for this session", theme.dim),
        ],
        action: Some("/profiles auto".into()),
        run: true,
        alt: None,
        charts: Vec::new(),
    });
    Modal::new(
        "Profiles",
        vec![ModalTab {
            title: "Profiles".into(),
            items,
        }],
    )
    .with_alt("make default")
    .with_scope(scope)
}

/// The `/commands` modal: custom slash commands from the `commands/` folders.
pub(crate) fn custom_commands(commands: &[quack_protocol::CommandInfo], theme: &Theme) -> Modal {
    let items = if commands.is_empty() {
        vec![placeholder(
            "no custom commands",
            "Put a markdown prompt in commands/ in your config folder (or .quack/commands in \
             a trusted project): commands/review.md is /review, commands/test/flaky.md is \
             /test:flaky. {{ args }} in the file becomes what you type after the name; \
             front matter can give a `description` and `args`. Prompts of connected MCP \
             servers show here too, as /<server>:<prompt>.",
            theme,
        )]
    } else {
        commands
            .iter()
            .map(|c| ModalItem {
                label: format!("/{}", c.name),
                hint: c.source.clone(),
                detail: [
                    Line::styled(
                        format!("/{} {}", c.name, c.args).trim_end().to_owned(),
                        theme.accent,
                    ),
                    Line::raw(""),
                    Line::raw(c.description.clone()),
                    Line::raw(""),
                    field(theme, "from", c.source.clone()),
                ]
                .into_iter()
                // MCP prompts have no file.
                .chain((!c.path.is_empty()).then(|| field(theme, "file", c.path.clone())))
                .chain([
                    Line::raw(""),
                    Line::styled("⏎ type it with a request", theme.dim),
                ])
                .collect(),
                action: Some(format!("/{} ", c.name)),
                run: false,
                alt: None,
                charts: Vec::new(),
            })
            .collect()
    };
    Modal::new(
        "Commands",
        vec![ModalTab {
            title: "Commands".into(),
            items,
        }],
    )
}

/// The `/mcp` modal: live servers, disabled/bundled examples, and quick-add actions.
/// `servers` is `None` while asking the engine.
pub(crate) fn mcp(
    servers: Option<&[quack_protocol::McpServerInfo]>,
    catalog: &[quack_config::McpSummary],
    scope: WriteScope,
    theme: &Theme,
) -> Modal {
    let items = match servers {
        None => vec![placeholder("checking servers…", "", theme)],
        Some(list) => {
            let mut out: Vec<ModalItem> = list
                .iter()
                .map(|s| {
                    let style = match s.state {
                        quack_protocol::McpServerState::Ready => theme.ok,
                        quack_protocol::McpServerState::Failed => theme.error,
                        _ => theme.warn,
                    };
                    let mut detail = vec![Line::styled(s.id.clone(), style), Line::raw("")];
                    if !s.description.is_empty() {
                        detail.push(Line::raw(s.description.clone()));
                        detail.push(Line::raw(""));
                    }
                    detail.push(field(theme, "state", s.state.label().to_owned()));
                    if !s.error.is_empty() {
                        detail.push(Line::styled(s.error.clone(), theme.error));
                    }
                    detail.push(field(theme, s.transport.as_str(), s.target.clone()));
                    if s.state == quack_protocol::McpServerState::Ready {
                        detail.push(field(
                            theme,
                            "tools",
                            format!(
                                "{} · ~{} tokens{}",
                                s.tools.len(),
                                s.tool_tokens,
                                if s.deferred {
                                    " · found with tool_search (too large to send every time)"
                                } else {
                                    ""
                                }
                            ),
                        ));
                        detail.push(Line::raw(""));
                        detail.extend(s.tools.iter().map(|t| Line::raw(format!("  {t}"))));
                        if !s.prompts.is_empty() {
                            detail.push(Line::raw(""));
                            detail.push(field(theme, "prompts", s.prompts.len().to_string()));
                            detail.extend(
                                s.prompts
                                    .iter()
                                    .map(|p| Line::raw(format!("  /{}:{p}", s.id))),
                            );
                        }
                        if !s.resources.is_empty() {
                            detail.push(Line::raw(""));
                            detail.push(field(theme, "resources", s.resources.len().to_string()));
                            detail.extend(s.resources.iter().map(|r| Line::raw(format!("  {r}"))));
                        }
                    }
                    if !s.log.is_empty() {
                        detail.push(Line::raw(""));
                        detail.push(Line::styled("recent output", theme.dim));
                        let skip = s.log.len().saturating_sub(12);
                        detail.extend(
                            s.log
                                .iter()
                                .skip(skip)
                                .map(|l| Line::styled(format!("  {l}"), theme.dim)),
                        );
                    }
                    detail.push(Line::raw(""));
                    detail.push(Line::styled(
                        "⏎ disable this server · ^s restart",
                        theme.dim,
                    ));
                    ModalItem {
                        label: s.id.clone(),
                        hint: if s.state == quack_protocol::McpServerState::Ready {
                            format!("{} tools", s.tools.len())
                        } else {
                            s.state.label().to_owned()
                        },
                        detail,
                        action: Some(format!("/mcp disable {}", s.id)),
                        run: true,
                        alt: Some(format!("/mcp restart {}", s.id)),
                        charts: Vec::new(),
                    }
                })
                .collect();
            out.extend(mcp_catalog_and_add_items(list, catalog, scope, theme));
            out
        }
    };
    Modal::new(
        "MCP",
        vec![ModalTab {
            title: "Servers".into(),
            items,
        }],
    )
    .with_alt("restart")
    .with_scope(scope)
}

fn mcp_catalog_and_add_items(
    active: &[quack_protocol::McpServerInfo],
    catalog: &[quack_config::McpSummary],
    scope: WriteScope,
    theme: &Theme,
) -> Vec<ModalItem> {
    let flag = scope_flag(scope);
    let mcp_dir = match scope {
        WriteScope::Project => ".quack/mcp.d/",
        WriteScope::User | WriteScope::Session => "user mcp.d/",
    };
    let mut items = Vec::new();
    for entry in catalog {
        if active.iter().any(|s| s.id == entry.id) {
            continue;
        }
        let hint = if entry.parse_error.is_some() {
            "error".to_owned()
        } else if entry.layer == quack_config::LayerKind::Defaults {
            "example · off".to_owned()
        } else {
            "off".to_owned()
        };
        let mut detail = Vec::new();
        if !entry.description.is_empty() {
            detail.push(Line::raw(entry.description.clone()));
            detail.push(Line::raw(""));
        }
        if !entry.transport.is_empty() {
            detail.push(field(theme, "transport", entry.transport.clone()));
        }
        if !entry.target.is_empty() {
            detail.push(field(theme, "target", entry.target.clone()));
        }
        detail.push(field(
            theme,
            "source",
            format!("{} ({})", entry.layer.label(), entry.path.display()),
        ));
        if !entry.env_vars.is_empty() {
            detail.push(field(theme, "env vars", entry.env_vars.join(", ")));
        }
        if let Some(reason) = &entry.disabled_reason {
            detail.push(field(theme, "state", reason.clone()));
        }
        if let Some(err) = &entry.parse_error {
            detail.push(Line::raw(""));
            detail.push(Line::styled(format!("parse error: {err}"), theme.error));
        } else {
            detail.push(Line::raw(""));
            detail.push(Line::styled(
                if entry.layer == quack_config::LayerKind::Defaults {
                    "⏎ copy example to user config, enable, and connect now"
                } else {
                    "⏎ enable and connect now"
                },
                theme.dim,
            ));
        }
        items.push(ModalItem {
            label: entry.id.clone(),
            hint,
            detail,
            action: entry
                .parse_error
                .is_none()
                .then(|| format!("/mcp enable {}", entry.id)),
            run: entry.parse_error.is_none(),
            alt: None,
            charts: Vec::new(),
        });
    }

    items.push(ModalItem {
        label: "+ Add remote MCP server (HTTP)".into(),
        hint: "url".into(),
        detail: vec![
            Line::styled("Connect a streamable HTTP MCP endpoint by URL", theme.accent),
            Line::raw(""),
            Line::styled(
                format!(
                    "Prompts for the URL (e.g. https://mcp.context7.com/mcp), server id, and optional Bearer auth env var or token. Saves to {mcp_dir}."
                ),
                theme.dim,
            ),
        ],
        action: Some(format!("/mcp add{flag} http")),
        run: true,
        alt: None,
        charts: Vec::new(),
    });
    items.push(ModalItem {
        label: "+ Add local MCP server (stdio)".into(),
        hint: "command".into(),
        detail: vec![
            Line::styled(
                "Add a local MCP server process run over stdio",
                theme.accent,
            ),
            Line::raw(""),
            Line::styled(
                format!(
                    "Prompts for the command line (e.g. npx -y @playwright/mcp@latest) and server id. Saves to {mcp_dir}."
                ),
                theme.dim,
            ),
        ],
        action: Some(format!("/mcp add{flag} stdio")),
        run: true,
        alt: None,
        charts: Vec::new(),
    });
    items
}

/// The `/lsp` modal: each language server with its state and diagnostics. `None` while
/// asking the engine.
pub(crate) fn lsp(servers: Option<&[quack_protocol::LspServerInfo]>, theme: &Theme) -> Modal {
    let items = match servers {
        None => vec![placeholder("checking servers…", "", theme)],
        Some([]) => vec![placeholder(
            "no language servers",
            "Add a TOML file to lsp.d in your config folder (`quack config paths` shows \
             where), for example\n\ncommand = \"gopls\"\nextensions = [\"go\"]\n\nThe defaults \
             bundle has files for the main languages. quack never installs a server.",
            theme,
        )],
        Some(list) => list
            .iter()
            .map(|s| {
                let style = match s.state {
                    quack_protocol::LspServerState::Ready
                    | quack_protocol::LspServerState::Idle => theme.ok,
                    quack_protocol::LspServerState::Failed
                    | quack_protocol::LspServerState::Exited => theme.error,
                    quack_protocol::LspServerState::NotInstalled => theme.dim,
                    _ => theme.warn,
                };
                let mut detail = vec![Line::styled(s.id.clone(), style), Line::raw("")];
                detail.push(field(theme, "state", s.state.label().to_owned()));
                if !s.detail.is_empty() {
                    let st = if s.state == quack_protocol::LspServerState::Failed {
                        theme.error
                    } else {
                        theme.dim
                    };
                    detail.push(Line::styled(s.detail.clone(), st));
                }
                detail.push(field(theme, "command", s.command.clone()));
                detail.push(field(
                    theme,
                    "files",
                    format!(".{}", s.extensions.join(" .")),
                ));
                if !s.root.is_empty() {
                    detail.push(field(theme, "project", s.root.clone()));
                }
                if matches!(
                    s.state,
                    quack_protocol::LspServerState::Ready
                        | quack_protocol::LspServerState::Indexing
                ) {
                    detail.push(field(
                        theme,
                        "problems",
                        format!("{} errors · {} warnings", s.errors, s.warnings),
                    ));
                }
                if s.state == quack_protocol::LspServerState::Idle {
                    detail.push(Line::raw(""));
                    detail.push(Line::styled(
                        "Starts when the agent reads or edits a file it covers.",
                        theme.dim,
                    ));
                }
                if !s.log.is_empty() {
                    detail.push(Line::raw(""));
                    detail.push(Line::styled("recent output", theme.dim));
                    let skip = s.log.len().saturating_sub(12);
                    detail.extend(
                        s.log
                            .iter()
                            .skip(skip)
                            .map(|l| Line::styled(format!("  {l}"), theme.dim)),
                    );
                }
                let label = if s.root.is_empty() || s.root == "." {
                    s.id.clone()
                } else {
                    format!("{} · {}", s.id, s.root)
                };
                ModalItem {
                    label,
                    hint: match s.state {
                        quack_protocol::LspServerState::Ready if s.errors + s.warnings > 0 => {
                            format!("✗{} ⚠{}", s.errors, s.warnings)
                        }
                        other => other.label().to_owned(),
                    },
                    detail,
                    action: None,
                    run: false,
                    alt: None,
                    charts: Vec::new(),
                }
            })
            .collect(),
    };
    Modal::new(
        "LSP",
        vec![ModalTab {
            title: "Servers".into(),
            items,
        }],
    )
}
