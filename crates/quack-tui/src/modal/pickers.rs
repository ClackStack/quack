//! Picker modals (help, models, thinking, tasks, sessions, config, memory, permissions, rewind, trust, theme).

use super::*;

/// The `/help` modal: commands and keys.
pub(crate) fn help(theme: &Theme) -> Modal {
    let field = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(format!("{k:<10}"), theme.dim),
            Span::raw(v),
        ])
    };
    let commands = COMMANDS
        .iter()
        .map(|c| {
            let mut detail = vec![
                Line::styled(format!("/{} {}", c.name, c.args), theme.accent),
                Line::raw(""),
                Line::raw(c.description),
                Line::raw(""),
            ];
            if !c.aliases.is_empty() {
                let aliases: Vec<String> = c.aliases.iter().map(|a| format!("/{a}")).collect();
                detail.push(field("aliases", aliases.join(", ")));
            }
            detail.push(field(
                "status",
                c.pending
                    .map_or_else(|| "available".into(), |m| format!("planned for {m}")),
            ));
            let requires_arg = c.args.starts_with('<');
            ModalItem {
                label: format!("/{}", c.name),
                hint: c.pending.unwrap_or_default().to_owned(),
                detail,
                action: Some(format!(
                    "/{}{}",
                    c.name,
                    if requires_arg { " " } else { "" }
                )),
                run: !requires_arg,
                alt: None,
                charts: Vec::new(),
            }
        })
        .collect();
    let keys = KEYS
        .iter()
        .map(|(k, what)| ModalItem {
            label: (*k).to_owned(),
            hint: String::new(),
            detail: vec![
                Line::styled(*k, theme.accent),
                Line::raw(""),
                Line::raw(*what),
            ],
            action: None,
            run: false,
            alt: None,
            charts: Vec::new(),
        })
        .collect();
    Modal::new(
        "Help",
        vec![
            ModalTab {
                title: "Commands".into(),
                items: commands,
            },
            ModalTab {
                title: "Keys".into(),
                items: keys,
            },
        ],
    )
}

/// The current model's declared thinking levels, plus a reset to its configured default.
pub(crate) fn thinking(
    model: Option<&str>,
    levels: &[String],
    current: Option<&str>,
    theme: &Theme,
) -> Modal {
    let mut items = Vec::new();
    if model.is_some() && !levels.is_empty() {
        for level in std::iter::once("default").chain(levels.iter().map(String::as_str)) {
            let description = if level == "default" {
                "Restore the level configured in the model's provider file."
            } else {
                "Use this thinking level for this model in the current session."
            };
            items.push(ModalItem {
                label: level.to_owned(),
                hint: if current == Some(level) || (current.is_none() && level == "default") {
                    "current".into()
                } else {
                    String::new()
                },
                detail: vec![
                    Line::styled(model.unwrap_or_default().to_owned(), theme.accent),
                    Line::raw(""),
                    Line::raw(description),
                    Line::raw(""),
                    Line::raw("Lower levels favor speed and fewer reasoning tokens."),
                    Line::raw("Higher levels allow more time for difficult tasks."),
                    Line::raw("Changes apply between turns and are saved with the session."),
                ],
                action: Some(if level == "default" {
                    "/thinking default".into()
                } else {
                    // Names such as `off` may also be visibility commands.
                    format!("/thinking level {level}")
                }),
                run: true,
                alt: None,
                charts: Vec::new(),
            });
        }
    } else {
        items.push(ModalItem {
            label: "No selectable thinking levels".into(),
            hint: String::new(),
            detail: vec![Line::raw(match model {
                Some(model) => format!(
                    "{model} does not declare thinking levels. Run `quack catalog refresh` to fetch \
                     the catalog's, or add reasoning.efforts to a user-layer copy of the provider file."
                ),
                None => "Select a model with /models first.".into(),
            })],
            action: None,
            run: false,
            alt: None,
            charts: Vec::new(),
        });
    }
    let mut modal = Modal::new(
        "Thinking",
        vec![ModalTab {
            title: "Levels".into(),
            items,
        }],
    );
    modal.footer.retain(|(key, _)| *key != "tab");
    if let Some(level) = current
        && let Some(index) = levels.iter().position(|e| e == level)
    {
        modal.selected = index + 1;
    }
    modal
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ModelRoles<'a> {
    pub current: Option<&'a str>,
    pub default: Option<&'a str>,
    pub small: Option<&'a str>,
    pub default_locked: bool,
    pub small_locked: bool,
    pub project_status: ProjectStatus,
}

/// The `/models` modal. `catalog` is `None` while the engine is still listing models.
pub(crate) fn models(
    catalog: Option<&[quack_protocol::ProviderModels]>,
    roles: ModelRoles<'_>,
    ascii: bool,
    scope: WriteScope,
    theme: &Theme,
) -> Modal {
    let target = scope_file(scope);
    let field = |k: &str, v: String| {
        Line::from(vec![
            Span::styled(format!("{k:<10}"), theme.dim),
            Span::raw(v),
        ])
    };
    let mut items = Vec::new();
    match catalog {
        None => items.push(ModalItem {
            label: "listing models…".into(),
            hint: String::new(),
            detail: vec![Line::styled(
                "Asking each provider for its models.",
                theme.dim,
            )],
            action: None,
            run: false,
            alt: None,
            charts: Vec::new(),
        }),
        Some(providers) => {
            // Context windows are drawn against the largest on offer.
            let widest = providers
                .iter()
                .filter(|p| p.available)
                .flat_map(|p| p.models.iter().filter_map(|m| m.context_window))
                .max()
                .unwrap_or(0);
            // The bundle's providers that are not set up are listed in /providers.
            for p in providers.iter().filter(|p| p.available || !p.bundled) {
                if !p.available {
                    items.push(ModalItem {
                        label: format!("{} (unavailable)", p.id),
                        hint: String::new(),
                        detail: vec![
                            Line::styled(p.id.clone(), theme.accent),
                            Line::raw(""),
                            Line::raw(p.note.clone()),
                        ],
                        action: None,
                        run: false,
                        alt: None,
                        charts: Vec::new(),
                    });
                    continue;
                }
                for m in &p.models {
                    let name = m.alias.clone().unwrap_or_else(|| m.id.clone());
                    let reference = format!("{}/{}", p.id, name);
                    let raw_ref = format!("{}/{}", p.id, m.id);
                    let matches_ref = |target: Option<&str>| {
                        target == Some(reference.as_str()) || target == Some(raw_ref.as_str())
                    };
                    let is_current = matches_ref(roles.current);
                    let is_default = matches_ref(roles.default);
                    let is_small = matches_ref(roles.small);
                    let role_tag = match (is_default, is_small) {
                        (true, true) => "both",
                        (true, false) => "large",
                        (false, true) => "small",
                        (false, false) => "",
                    };
                    let ctx = m
                        .context_window
                        .map_or_else(String::new, crate::theme::tokens);
                    let hint = match (role_tag.is_empty(), ctx.is_empty()) {
                        (true, _) => ctx,
                        (false, true) => role_tag.into(),
                        (false, false) => format!("{role_tag} · {ctx}"),
                    };
                    let mut detail = vec![
                        Line::styled(reference.clone(), theme.accent),
                        Line::raw(""),
                        field("provider", p.id.clone()),
                        field("model id", m.id.clone()),
                        field(
                            "context",
                            m.context_window
                                .map_or_else(|| "unknown".into(), |c| format!("{c} tokens")),
                        ),
                    ];
                    if let Some(price) = &m.price {
                        detail.push(field("price", price.clone()));
                    }
                    if !m.reasoning_efforts.is_empty() {
                        detail.push(field("thinking", m.reasoning_efforts.join(" · ")));
                    }
                    if is_default || is_small {
                        detail.push(field(
                            "default",
                            match (is_default, is_small) {
                                (true, true) => "large + small".into(),
                                (true, false) => "large (model.default)".into(),
                                (false, true) => "small (model.small)".into(),
                                (false, false) => String::new(),
                            },
                        ));
                    }
                    detail.push(Line::raw(""));
                    let mut charts = Vec::new();
                    if let Some(c) = m.context_window.filter(|_| widest > 0) {
                        charts.push((
                            detail.len(),
                            Visual::Meter {
                                label: "window".into(),
                                text: if c == widest {
                                    "largest here".into()
                                } else {
                                    format!(
                                        "{}% of largest",
                                        u64::from(c) * 100 / u64::from(widest)
                                    )
                                },
                                ratio: f64::from(c) / f64::from(widest),
                                style: theme.accent,
                                ascii,
                            },
                        ));
                        detail.push(Line::raw(""));
                    }
                    detail.push(if is_current {
                        Line::styled("in use", theme.ok)
                    } else {
                        Line::styled("⏎ switch to this model for the session", theme.dim)
                    });
                    if !m.reasoning_efforts.is_empty() {
                        detail.push(Line::styled(
                            if is_current {
                                "/thinking reopens the level picker"
                            } else {
                                "quack then asks for a thinking level (speed vs cost)"
                            },
                            theme.dim,
                        ));
                    }
                    detail.push(Line::styled(
                        format!("^s set default: large, small, or both ({target})"),
                        theme.dim,
                    ));
                    items.push(ModalItem {
                        label: if is_current {
                            format!("{reference} ●")
                        } else {
                            reference.clone()
                        },
                        hint,
                        detail,
                        action: Some(format!("/models {reference}")),
                        run: true,
                        alt: Some(format!("/models default-menu {reference}")),
                        charts,
                    });
                }
            }
            if items.is_empty() {
                items.push(placeholder(
                    "no models ready",
                    "/setup adds one: a server on this machine, a hosted service with its \
                     key, or any server by its URL. /providers lists what is configured.",
                    theme,
                ));
            }
        }
    }
    Modal::new(
        "Models",
        vec![ModalTab {
            title: "Models".into(),
            items,
        }],
    )
    .with_alt("save as default")
    .with_scope(scope)
}

/// Picker opened by `^s` in `/models`: save the model as large (`model.default`), small
/// (`model.small`), or both.
pub(crate) fn model_defaults(
    reference: &str,
    roles: ModelRoles<'_>,
    scope: WriteScope,
    theme: &Theme,
) -> Modal {
    let target = scope_file(scope);
    let flag = scope_flag(scope);
    let status_note = (scope == WriteScope::Project)
        .then(|| project_status_note(roles.project_status))
        .flatten();
    let is_default = roles.default == Some(reference);
    let is_small = roles.small == Some(reference);
    let both_locked = roles.default_locked || roles.small_locked;

    let build_choice = |label: &str,
                        role: &str,
                        current: bool,
                        locked: bool,
                        current_value: Option<&str>,
                        summary: &str,
                        usage: &str| {
        let mut detail = vec![
            Line::styled(reference.to_owned(), theme.accent),
            Line::raw(""),
            Line::raw(summary.to_owned()),
            Line::raw(usage.to_owned()),
            Line::raw(""),
        ];
        if let Some(cur) = current_value.filter(|c| !c.is_empty()) {
            detail.push(field(theme, "current", cur.to_owned()));
            detail.push(Line::raw(""));
        }
        let action = if locked {
            detail.push(Line::styled("locked by the system policy", theme.warn));
            None
        } else {
            detail.push(Line::styled(format!("⏎ save to {target}"), theme.dim));
            if let Some(msg) = status_note {
                detail.push(Line::styled(msg, theme.warn));
            }
            Some(format!("/models default{flag} {role} {reference}"))
        };
        ModalItem {
            label: label.to_owned(),
            hint: if locked {
                "locked".into()
            } else if current {
                "current".into()
            } else {
                String::new()
            },
            detail,
            run: action.is_some(),
            action,
            alt: None,
            charts: Vec::new(),
        }
    };

    let mut items = vec![
        build_choice(
            "Large (default model)",
            "large",
            is_default,
            roles.default_locked,
            roles.default,
            &format!("Save as the main default model (`model.default`) in {target}."),
            "Used when starting new sessions.",
        ),
        build_choice(
            "Small (helper model)",
            "small",
            is_small,
            roles.small_locked,
            roles.small,
            &format!("Save as the small helper model (`model.small`) in {target}."),
            "Used immediately for session titles, the explore sub-agent, and cheap subtasks.",
        ),
        build_choice(
            "Both (large & small)",
            "both",
            is_default && is_small,
            both_locked,
            None,
            &format!(
                "Save as both the large default (`model.default`) and small helper (`model.small`) in {target}."
            ),
            "Uses this model for both main turns and helper subtasks.",
        ),
    ];

    if is_default && !roles.default_locked {
        items.push(ModalItem {
            label: "Clear large default".into(),
            hint: "unset".into(),
            detail: vec![
                Line::styled(reference.to_owned(), theme.accent),
                Line::raw(""),
                Line::raw(format!("Clear `model.default` in {target}.")),
            ],
            action: Some(format!("/models default{flag} large clear")),
            run: true,
            alt: None,
            charts: Vec::new(),
        });
    }
    if is_small && !roles.small_locked {
        items.push(ModalItem {
            label: "Clear small default".into(),
            hint: "unset".into(),
            detail: vec![
                Line::styled(reference.to_owned(), theme.accent),
                Line::raw(""),
                Line::raw(format!(
                    "Clear `model.small` in {target} so helper tasks use the active model."
                )),
            ],
            action: Some(format!("/models default{flag} small clear")),
            run: true,
            alt: None,
            charts: Vec::new(),
        });
    }

    let mut modal = Modal::new(
        "Default role",
        vec![ModalTab {
            title: "Role".into(),
            items,
        }],
    )
    .with_scope(scope);
    modal.footer.retain(|(key, _)| *key != "tab");
    modal
}

/// The `/tasks` modal: each sub-agent run of this session, newest first, with what it did
/// and its report.
pub(crate) fn tasks(runs: &[crate::app::AgentRun], theme: &Theme) -> Modal {
    use crate::app::AgentItem;
    let items = if runs.is_empty() {
        vec![placeholder(
            "no sub-agents yet",
            "The model starts sub-agents with its task tool, from the agents in agents.d: \
             explore for read-only searches (several at once), general for self-contained \
             changes. Each works with a fresh context and only its report comes back.",
            theme,
        )]
    } else {
        runs.iter()
            .rev()
            .map(|r| {
                let took = r.took.unwrap_or_else(|| r.started.elapsed());
                let state = match r.ok {
                    None => format!("running · {} steps", r.steps()),
                    Some(true) => r.summary.clone(),
                    Some(false) => format!("failed · {}", r.summary),
                };
                let mut detail = vec![
                    Line::styled(format!("{}: {}", r.agent, r.description), theme.accent),
                    Line::raw(""),
                    field(theme, "run", r.id.clone()),
                    field(theme, "model", r.model.clone()),
                    field(
                        theme,
                        "state",
                        format!("{state} · {}", crate::theme::elapsed(took)),
                    ),
                    Line::raw(""),
                ];
                for item in &r.items {
                    match item {
                        AgentItem::Tool(t) => {
                            let (mark, style) = match t.ok {
                                None => ("…", theme.accent),
                                Some(true) => ("✓", theme.ok),
                                Some(false) => ("✗", theme.error),
                            };
                            let result = if t.result.is_empty() {
                                String::new()
                            } else {
                                format!(" · {}", t.result)
                            };
                            detail.push(Line::from(vec![
                                Span::styled(format!("{mark} "), style),
                                Span::raw(t.summary.clone()),
                                Span::styled(result, theme.dim),
                            ]));
                        }
                        AgentItem::Text(text) => {
                            detail.push(Line::raw(""));
                            detail.extend(text.trim().lines().map(|l| Line::raw(l.to_owned())));
                            detail.push(Line::raw(""));
                        }
                    }
                }
                ModalItem {
                    label: format!("{} · {}", r.id, r.description),
                    hint: match r.ok {
                        None => "running".into(),
                        Some(true) => "done".into(),
                        Some(false) => "failed".into(),
                    },
                    detail,
                    action: None,
                    run: false,
                    alt: None,
                    charts: Vec::new(),
                }
            })
            .collect()
    };
    Modal::new(
        "Tasks",
        vec![ModalTab {
            title: "Sub-agents".into(),
            items,
        }],
    )
}

/// The `/resume` modal. `sessions` is `None` while the engine is listing them.
pub(crate) fn sessions(
    sessions: Option<&[quack_protocol::SessionSummary]>,
    theme: &Theme,
) -> Modal {
    let items = match sessions {
        None => vec![placeholder(
            "listing sessions…",
            "Reading this project's sessions.",
            theme,
        )],
        Some([]) => vec![placeholder(
            "no saved sessions",
            "Sessions are saved as you work; there are none for this project yet.",
            theme,
        )],
        Some(list) => list
            .iter()
            .map(|s| {
                let title = if s.title.is_empty() {
                    "(untitled)".to_owned()
                } else {
                    s.title.clone()
                };
                let mut detail = vec![
                    Line::styled(title.clone(), theme.accent),
                    Line::raw(""),
                    field(theme, "id", s.id.0.clone()),
                    field(theme, "started", s.started.clone()),
                    field(theme, "updated", s.age.clone()),
                    field(theme, "turns", s.turns.to_string()),
                    field(theme, "model", s.model.clone()),
                    Line::raw(""),
                ];
                detail.push(if s.current {
                    Line::styled("this session", theme.ok)
                } else {
                    Line::styled("⏎ resume it (the current session stays saved)", theme.dim)
                });
                ModalItem {
                    label: if s.current {
                        format!("{title} ●")
                    } else {
                        title
                    },
                    hint: format!("{} · {}t", s.age, s.turns),
                    detail,
                    action: (!s.current).then(|| format!("/resume {}", s.id)),
                    run: true,
                    alt: None,
                    charts: Vec::new(),
                }
            })
            .collect(),
    };
    Modal::new(
        "Sessions",
        vec![ModalTab {
            title: "Sessions".into(),
            items,
        }],
    )
}

/// The `/config` modal: every setting with its value and where it came from.
pub(crate) fn config(
    rows: &[quack_config::SettingRow],
    files: &[crate::app::ConfigLayer],
    scope: WriteScope,
    project_status: ProjectStatus,
    theme: &Theme,
) -> Modal {
    let target = scope_file(scope);
    let flag = scope_flag(scope);
    let status_note = (scope == WriteScope::Project)
        .then(|| project_status_note(project_status))
        .flatten();
    let settings = rows
        .iter()
        .map(|r| {
            let mut detail = vec![Line::styled(r.key.clone(), theme.accent), Line::raw("")];
            if !r.description.is_empty() {
                detail.push(Line::raw(r.description.clone()));
                detail.push(Line::raw(""));
            }
            detail.push(field(theme, "value", r.value.clone()));
            detail.push(field(theme, "from", r.origin.clone()));
            detail.push(Line::raw(""));
            let (action, run) = if r.locked {
                detail.push(Line::styled("locked by the system policy", theme.warn));
                (None, false)
            } else if r.is_bool {
                let flipped = if r.value == "true" { "false" } else { "true" };
                detail.push(Line::styled(
                    format!("⏎ set to {flipped} in {target}"),
                    theme.dim,
                ));
                (Some(format!("/config set{flag} {} {flipped}", r.key)), true)
            } else if r.key == "permissions.work_mode" {
                let flipped = if r.value.trim_matches('"') == "plan" {
                    "\"edit\""
                } else {
                    "\"plan\""
                };
                detail.push(Line::styled(
                    format!("⏎ set to {flipped} in {target} and apply now"),
                    theme.dim,
                ));
                (Some(format!("/config set{flag} {} {flipped}", r.key)), true)
            } else if r.key == "permissions.mode" {
                let flipped = if r.value.trim_matches('"') == "allow" {
                    "\"ask\""
                } else {
                    "\"allow\""
                };
                detail.push(Line::styled(
                    format!("⏎ set to {flipped} in {target} and apply now"),
                    theme.dim,
                ));
                (Some(format!("/config set{flag} {} {flipped}", r.key)), true)
            } else {
                detail.push(Line::styled(
                    format!("⏎ edit the value, then press ⏎ to save it to {target}"),
                    theme.dim,
                ));
                let prefill = if r.value == "\"\"" {
                    format!("/config set{flag} {} ", r.key)
                } else {
                    format!("/config set{flag} {} {}", r.key, r.value)
                };
                (Some(prefill), false)
            };
            if !r.locked
                && let Some(msg) = status_note
            {
                detail.push(Line::styled(msg, theme.warn));
            }
            let mut hint: String = r.value.chars().take(24).collect();
            if r.value.chars().count() > 24 {
                hint.push('…');
            }
            ModalItem {
                label: r.key.clone(),
                hint,
                detail,
                action,
                run,
                alt: None,
                charts: Vec::new(),
            }
        })
        .collect();
    let layers = files
        .iter()
        .map(|file| {
            let (hint, style) = match file.status.as_str() {
                "loaded" => ("loaded", theme.ok),
                "no config.toml" => ("missing", theme.dim),
                s if s.starts_with("failed") => ("failed", theme.warn),
                _ => ("skipped", theme.dim),
            };
            let mut detail = vec![
                Line::styled(file.layer.clone(), theme.accent),
                Line::styled(file.status.clone(), style),
                Line::raw(""),
            ];
            if let Some(path) = &file.path {
                detail.push(Line::raw(path.clone()));
            } else {
                detail.push(Line::styled("No file path for this layer", theme.dim));
            }
            ModalItem {
                label: file.layer.clone(),
                hint: hint.into(),
                detail,
                action: file.path.clone(),
                run: false,
                alt: None,
                charts: Vec::new(),
            }
        })
        .collect();
    let mut modal = Modal::new(
        "Config",
        vec![
            ModalTab {
                title: "Settings".into(),
                items: settings,
            },
            ModalTab {
                title: "Layers".into(),
                items: layers,
            },
        ],
    );
    modal.copy_tab = Some(1);
    modal.with_scope(scope)
}

/// The `/memory` modal. `notes` is `None` while the engine reads them. From 80% of the
/// index cap (`fill`), the tab title says so.
pub(crate) fn memory(
    notes: Option<&[quack_protocol::MemoryNote]>,
    fill: u32,
    theme: &Theme,
) -> Modal {
    let items = match notes {
        None => vec![placeholder(
            "reading notes…",
            "Reading memory notes.",
            theme,
        )],
        Some([]) => vec![placeholder(
            "no notes yet",
            "The model saves notes with the memory tool when something will matter in later \
             sessions. Notes are markdown files you can edit.",
            theme,
        )],
        Some(list) => list
            .iter()
            .map(|n| {
                let mut detail = vec![
                    Line::styled(n.name.clone(), theme.accent),
                    Line::raw(""),
                    Line::raw(n.description.clone()),
                    Line::raw(""),
                    field(theme, "scope", n.scope.clone()),
                    field(theme, "file", n.path.clone()),
                    Line::raw(""),
                ];
                // The body, without the front matter shown above.
                let body = n
                    .text
                    .strip_prefix("---")
                    .and_then(|r| r.split_once("\n---"))
                    .map_or(n.text.as_str(), |(_, b)| b);
                detail.extend(body.trim().lines().map(|l| Line::raw(l.to_owned())));
                ModalItem {
                    label: n.name.clone(),
                    hint: n.scope.clone(),
                    detail,
                    action: None,
                    run: false,
                    alt: Some(format!("/memory delete {}/{}", n.scope, n.name)),
                    charts: Vec::new(),
                }
            })
            .collect(),
    };
    Modal::new(
        "Memory",
        vec![ModalTab {
            title: if fill >= 80 {
                format!("Notes · index {fill}% full")
            } else {
                "Notes".into()
            },
            items,
        }],
    )
    .with_alt("delete")
}

/// The `/permissions` modal: Ask/Allow approvals and the configured rules.
pub(crate) fn permissions(current: &str, rules: &[(String, String)], theme: &Theme) -> Modal {
    const MODES: &[(&str, &str)] = &[
        (
            "ask",
            "Ask before unapproved tools. Plan/Edit stays as selected; Plan only reads and searches.",
        ),
        (
            "allow",
            "Automatically approve tools within the current work mode and sandbox. Explicit ask/deny rules still apply; leaving the sandbox asks.",
        ),
    ];
    let modes = MODES
        .iter()
        .map(|(name, what)| ModalItem {
            label: if *name == current {
                format!("{name} ●")
            } else {
                (*name).to_owned()
            },
            hint: String::new(),
            detail: vec![
                Line::styled(*name, theme.accent),
                Line::raw(""),
                Line::raw(*what),
                Line::raw(""),
                Line::styled(
                    if *name == current {
                        "the current approval policy"
                    } else {
                        "⏎ select approvals, keeping Plan/Edit"
                    },
                    theme.dim,
                ),
            ],
            action: Some(format!("/permissions {name}")),
            run: true,
            alt: None,
            charts: Vec::new(),
        })
        .collect();
    let rule_items = if rules.is_empty() {
        vec![placeholder(
            "no rules",
            "Add allow, ask and deny rules under [permissions] in config.toml, e.g. \
             allow = [\"shell:cargo test*\"]. /config shows where each setting comes from.",
            theme,
        )]
    } else {
        rules
            .iter()
            .map(|(list, rule)| ModalItem {
                label: rule.clone(),
                hint: list.clone(),
                detail: vec![
                    Line::styled(rule.clone(), theme.accent),
                    Line::raw(""),
                    field(theme, "list", list.clone()),
                    Line::raw(""),
                    Line::styled(
                        "Order: Plan restrictions, built-in deny list, deny, ask, session approvals/allow, then the approval default.",
                        theme.dim,
                    ),
                ],
                action: None,
                run: false,
                alt: None,
                charts: Vec::new(),
            })
            .collect()
    };
    Modal::new(
        "Permissions",
        vec![
            ModalTab {
                title: match current {
                    "ask" | "allow" => "Approvals".into(),
                    "yolo" => "Approvals · yolo".into(),
                    _ => format!("Approvals · {current} (legacy)"),
                },
                items: modes,
            },
            ModalTab {
                title: "Rules".into(),
                items: rule_items,
            },
        ],
    )
}

/// The `/rewind` picker. `points` is `None` while the engine lists them.
pub(crate) fn rewind(points: Option<&[quack_protocol::RewindPoint]>, theme: &Theme) -> Modal {
    let items = match points {
        None => vec![placeholder("listing messages…", "", theme)],
        Some([]) => vec![placeholder(
            "no messages yet",
            "Once you have sent a message, /rewind can go back to before it.",
            theme,
        )],
        Some(points) => points
            .iter()
            .map(|p| {
                let first = p.text.lines().find(|l| !l.trim().is_empty()).unwrap_or_default();
                let mut detail = vec![Line::styled(format!("{} back", p.back), theme.accent), Line::raw("")];
                detail.extend(p.text.lines().take(12).map(|l| Line::raw(l.to_owned())));
                detail.push(Line::raw(""));
                if p.files.is_empty() {
                    detail.push(Line::styled("No files changed by write or edit since then.", theme.dim));
                } else {
                    detail.push(Line::styled("Files put back:", theme.dim));
                    detail.extend(p.files.iter().map(|f| Line::raw(format!("  {f}"))));
                }
                detail.push(Line::raw(""));
                detail.push(Line::styled(
                    format!(
                        "⏎ conversation and files · ^s conversation only · /rewind {} files for files only. \
                         Changes made by shell commands are not tracked.",
                        p.back
                    ),
                    theme.dim,
                ));
                ModalItem {
                    label: first.chars().take(100).collect(),
                    hint: match p.files.len() {
                        0 => String::new(),
                        1 => "1 file".into(),
                        n => format!("{n} files"),
                    },
                    detail,
                    action: Some(format!("/rewind {} both", p.back)),
                    run: true,
                    alt: Some(format!("/rewind {} conversation", p.back)),
                    charts: Vec::new(),
                }
            })
            .collect(),
    };
    Modal::new(
        "Rewind",
        vec![ModalTab {
            title: "Messages".into(),
            items,
        }],
    )
    .with_alt("conversation only")
}

/// The `/trust` modal: what the workspace's `.quack/` holds and the three choices.
pub(crate) fn trust(pending: Option<&TrustPrompt>, theme: &Theme) -> Modal {
    const CHOICES: &[(&str, &str, &str)] = &[
        (
            "trusted",
            "Trust",
            "Load everything: settings, providers, MCP servers, hooks and language servers. \
             MCP servers and hooks start programs from this repo's files.",
        ),
        (
            "settings-only",
            "Trust settings only",
            "Load config.toml, but nothing that starts a program or adds a provider.",
        ),
        (
            "ignored",
            "Ignore",
            "Never load this repo's .quack/, and don't ask again.",
        ),
    ];
    let mut about = match pending {
        Some(p) => vec![
            Line::styled(
                if p.changed {
                    "Changed since you trusted it"
                } else {
                    "Project config found"
                },
                theme.warn,
            ),
            Line::raw(""),
            field(theme, "folder", p.path.clone()),
            field(theme, "holds", p.summary.clone()),
        ],
        None => vec![
            Line::styled("This workspace's .quack/", theme.accent),
            Line::raw(""),
            Line::raw("No decision is waiting. Choosing again replaces the stored one."),
        ],
    };
    about.push(Line::raw(""));
    about.push(Line::styled(
        "Decisions are kept in trust.toml in the user data folder. If the files change, \
         you are asked again.",
        theme.dim,
    ));
    let mut items: Vec<ModalItem> = CHOICES
        .iter()
        .map(|(id, label, what)| {
            let mut detail = vec![
                Line::styled(*label, theme.accent),
                Line::raw(""),
                Line::raw(*what),
                Line::raw(""),
            ];
            detail.extend(about.iter().cloned());
            ModalItem {
                label: (*label).to_owned(),
                hint: String::new(),
                detail,
                action: Some(format!("/trust {id}")),
                run: true,
                alt: None,
                charts: Vec::new(),
            }
        })
        .collect();
    items.push(ModalItem {
        label: "Decide later".into(),
        hint: String::new(),
        detail: vec![
            Line::styled("Decide later", theme.accent),
            Line::raw(""),
            Line::raw("Keep skipping it for now. /trust brings this back."),
        ],
        action: Some("/trust later".into()),
        run: true,
        alt: None,
        charts: Vec::new(),
    });
    Modal::new(
        "Trust",
        vec![ModalTab {
            title: "Project".into(),
            items,
        }],
    )
}

/// A `.quack/` waiting for a decision (`Event::ProjectTrust`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrustPrompt {
    pub path: String,
    pub summary: String,
    pub changed: bool,
}

/// The `/theme` modal: switch between `auto`, `dark` and `light`.
pub(crate) fn theme(current: &str, scope: WriteScope, theme: &Theme) -> Modal {
    let target = scope_file(scope);
    let flag = scope_flag(scope);
    const THEMES: [(&str, &str); 3] = [
        (
            "auto",
            "Follows the terminal's background (`COLORFGBG` when set, otherwise dark).",
        ),
        (
            "dark",
            "High-contrast palette for dark terminal backgrounds.",
        ),
        (
            "light",
            "High-contrast palette for light terminal backgrounds.",
        ),
    ];
    let items = THEMES
        .iter()
        .map(|(id, about)| {
            let active = *id == current;
            ModalItem {
                label: (*id).to_owned(),
                hint: if active {
                    "in use".into()
                } else {
                    String::new()
                },
                detail: vec![
                    Line::styled(*id, theme.accent),
                    Line::raw(""),
                    Line::raw(*about),
                    Line::raw(""),
                    Line::styled(
                        if active {
                            "in use".into()
                        } else {
                            format!("⏎ use this theme (saved to [ui] theme in {target})")
                        },
                        if active { theme.ok } else { theme.dim },
                    ),
                ],
                action: Some(format!("/theme{flag} {id}")),
                run: true,
                alt: None,
                charts: Vec::new(),
            }
        })
        .collect();
    Modal::new(
        "Theme",
        vec![ModalTab {
            title: "Themes".into(),
            items,
        }],
    )
    .with_scope(scope)
}
