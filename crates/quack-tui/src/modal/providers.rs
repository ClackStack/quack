//! Provider, connection, setup, and sign-in modals.

use super::*;

/// The `/providers` modal: each provider file with its state. `None` while probing.
/// Where a key is read from, as the end of "read from …".
pub(crate) fn key_source(key: &quack_protocol::KeySource) -> String {
    use quack_protocol::KeySource;
    match key {
        KeySource::Keyring { name } => format!("the OS credential store (`{name}`)"),
        KeySource::Env { var } => format!("the {var} environment variable"),
        KeySource::File { path } => format!("the file {path}"),
        KeySource::Cmd => "a command in its provider file".into(),
        KeySource::Value => "its provider file".into(),
    }
}

pub(crate) fn provider_label_id(label: &str) -> &str {
    label.strip_suffix(" ●").unwrap_or(label)
}

pub(crate) fn providers(
    catalog: Option<&[quack_protocol::ProviderModels]>,
    connections: Option<&[quack_protocol::ProviderConnection]>,
    current: Option<&str>,
    theme: &Theme,
) -> Modal {
    let items = match catalog {
        None => match connections.filter(|list| !list.is_empty()) {
            Some(list) => list
                .iter()
                .map(|conn| {
                    let in_use = current.is_some_and(|c| c.starts_with(&format!("{}/", conn.id)));
                    let mut detail = vec![
                        Line::styled(conn.id.clone(), theme.accent),
                        Line::raw(""),
                        Line::styled("checking models…", theme.dim),
                    ];
                    if in_use {
                        detail.push(Line::styled("the current model is from here", theme.dim));
                    }
                    detail.push(Line::raw(""));
                    detail.push(field(theme, "HTTP", provider_http_version(conn).into()));
                    detail.push(field(theme, "proxy", provider_proxy(conn).into()));
                    detail.push(Line::raw(""));
                    detail.push(Line::styled("⏎ or ^s edit connection settings", theme.dim));
                    let edit = format!("/providers advanced {}", conn.id);
                    ModalItem {
                        label: if in_use {
                            format!("{} ●", conn.id)
                        } else {
                            conn.id.clone()
                        },
                        hint: "checking…".into(),
                        detail,
                        run: true,
                        action: Some(edit.clone()),
                        alt: Some(edit),
                        charts: Vec::new(),
                    }
                })
                .collect(),
            None => vec![placeholder(
                "checking providers…",
                "Asking each provider for its models.",
                theme,
            )],
        },
        Some([]) => vec![placeholder(
            "no providers",
            "/setup adds one: a server on this machine, a hosted service with its key, or \
             any server by its URL. Or add a TOML file with a base_url to providers.d in \
             your config folder (`quack config paths` shows where).",
            theme,
        )],
        Some(list) => list
            .iter()
            .map(|p| {
                let in_use = current.is_some_and(|c| c.starts_with(&format!("{}/", p.id)));
                let conn = connections.and_then(|cs| cs.iter().find(|c| c.id == p.id));
                let mut detail = vec![
                    Line::styled(p.id.clone(), theme.accent),
                    Line::raw(""),
                    if p.available {
                        Line::styled("usable", theme.ok)
                    } else if p.bundled {
                        Line::styled("offered by the defaults bundle; not set up", theme.dim)
                    } else {
                        Line::styled("unavailable", theme.warn)
                    },
                ];
                if !p.note.is_empty() {
                    detail.push(Line::raw(p.note.clone()));
                }
                if in_use {
                    detail.push(Line::styled("the current model is from here", theme.dim));
                }
                if p.key.is_some() || conn.is_some() || connections.is_none() {
                    detail.push(Line::raw(""));
                }
                if let Some(key) = &p.key {
                    detail.push(field(theme, "key", key_source(key)));
                }
                if let Some(conn) = conn {
                    detail.push(field(theme, "HTTP", provider_http_version(conn).into()));
                    detail.push(field(theme, "proxy", provider_proxy(conn).into()));
                } else if connections.is_none() {
                    detail.push(field(theme, "network", "loading…".into()));
                }
                match (&p.key, conn) {
                    (Some(_), Some(_)) => {
                        detail.push(Line::raw(""));
                        detail.push(Line::styled(
                            "⏎ replace its key · ^s connection settings",
                            theme.dim,
                        ));
                    }
                    (Some(_), None) => {
                        detail.push(Line::styled(
                            "⏎ replace its key, for example after rotating it",
                            theme.dim,
                        ));
                    }
                    (None, Some(_)) => {
                        detail.push(Line::raw(""));
                        detail.push(Line::styled("⏎ or ^s edit connection settings", theme.dim));
                    }
                    (None, None) => {}
                }
                if !p.models.is_empty() {
                    detail.push(Line::raw(""));
                    detail.extend(p.models.iter().take(30).map(|m| {
                        Line::raw(format!(
                            "  {}{}",
                            m.alias
                                .as_ref()
                                .map_or_else(|| m.id.clone(), |a| format!("{a} ({})", m.id)),
                            m.context_window.map_or_else(String::new, |c| format!(
                                " · {}",
                                crate::theme::tokens(c)
                            ))
                        ))
                    }));
                    if p.models.len() > 30 {
                        detail.push(Line::styled(
                            format!("  … {} more in /models", p.models.len() - 30),
                            theme.dim,
                        ));
                    }
                }
                let edit_connection = conn.map(|_| format!("/providers advanced {}", p.id));
                let action = if p.key.is_some() {
                    Some(format!("/key {}", p.id))
                } else {
                    edit_connection.clone()
                };
                ModalItem {
                    label: if in_use {
                        format!("{} ●", p.id)
                    } else {
                        p.id.clone()
                    },
                    hint: if p.available {
                        format!("{} models", p.models.len())
                    } else if p.bundled {
                        "not set up".into()
                    } else {
                        "unavailable".into()
                    },
                    detail,
                    run: action.is_some(),
                    action,
                    alt: edit_connection,
                    charts: Vec::new(),
                }
            })
            .collect(),
    };
    Modal::new(
        "Providers",
        vec![ModalTab {
            title: "Providers".into(),
            items,
        }],
    )
    .with_alt("connection")
}

fn provider_http_version(p: &quack_protocol::ProviderConnection) -> &'static str {
    use quack_protocol::ProviderHttpVersion::*;
    match p.http_version {
        Inherit if p.inherited_http1_only => "Inherited · HTTP/1.1 only",
        Inherit => "Inherited · Negotiate",
        Http1Only => "HTTP/1.1 only",
        Negotiate => "Negotiate",
    }
}

fn provider_proxy(p: &quack_protocol::ProviderConnection) -> &'static str {
    if p.no_proxy {
        "Direct connection"
    } else {
        "Use configured proxy settings"
    }
}

pub(crate) fn provider_connection(p: &quack_protocol::ProviderConnection, theme: &Theme) -> Modal {
    use quack_protocol::ProviderHttpVersion::*;
    let option = |label: &str, setting: &str, selected, detail: &str| ModalItem {
        label: label.into(),
        hint: if selected {
            "current".into()
        } else {
            String::new()
        },
        detail: vec![
            Line::styled(p.id.clone(), theme.accent),
            Line::raw(""),
            field(theme, "HTTP", provider_http_version(p).into()),
            field(theme, "proxy", provider_proxy(p).into()),
            Line::raw(""),
            Line::raw(detail.to_owned()),
            Line::raw(""),
            Line::styled("⏎ save setting · esc back to providers", theme.dim),
        ],
        action: Some(format!("/providers {setting} {}", p.id)),
        run: true,
        alt: None,
        charts: Vec::new(),
    };
    Modal::grouped(
        format!("Connection: {}", p.id),
        vec![
            (
                "HTTP version",
                vec![
                    option(
                        if p.inherited_http1_only {
                            "Inherit (HTTP/1.1)"
                        } else {
                            "Inherit (Negotiate)"
                        },
                        "http-inherit",
                        p.http_version == Inherit,
                        "Remove this file's HTTP version override.",
                    ),
                    option(
                        "HTTP/1.1 only",
                        "http-1",
                        p.http_version == Http1Only,
                        "Try this when HTTP/2 connections stall behind a VPN or proxy.",
                    ),
                    option(
                        "Negotiate",
                        "http-negotiate",
                        p.http_version == Negotiate,
                        "Let quack and the server negotiate the HTTP version.",
                    ),
                ],
            ),
            (
                "Proxy",
                vec![
                    option(
                        "Configured proxy",
                        "proxy-inherit",
                        !p.no_proxy,
                        "Use the provider, global, environment or system proxy settings.",
                    ),
                    option(
                        "Direct connection",
                        "proxy-direct",
                        p.no_proxy,
                        "Bypass proxies for this provider. Use this when your VPN routes to the provider directly.",
                    ),
                ],
            ),
        ],
    )
}

/// What `/setup` knows so far.
#[derive(Debug, Default)]
pub(crate) struct SetupView {
    pub local: Scan,
    /// The hosted services in providers.d; `None` while listing.
    pub services: Option<Vec<quack_protocol::ServiceInfo>>,
    /// The last subscription sign-in failure, shown beside that service for retry.
    pub sign_in_error: Option<(String, String)>,
    /// The settings when `/setup` opened, for the opt-in checks.
    pub settings: Vec<quack_config::SettingRow>,
}

/// Looking for model servers on this machine, which happens only when the user asks.
#[derive(Debug, Default)]
pub(crate) enum Scan {
    #[default]
    NotYet,
    Looking,
    Found(Vec<quack_protocol::DetectedServer>),
}

/// The `/setup` modal: local servers, hosted API keys and subscriptions, and any
/// server by its URL.
pub(crate) fn setup(view: &SetupView, theme: &Theme) -> Modal {
    const ELSEWHERE: &str = "For a hosted service, or a server on another machine or \
         port, press tab for the other ways to add one.";
    const LOOKS_AT: &str = "Checks the providers in providers.d that point at this machine \
         (Ollama, llama.cpp, vLLM, LM Studio and SGLang with the defaults bundle). Nothing \
         leaves this machine.";
    let item =
        |label: &str, hint: &str, detail: Vec<Line<'static>>, action: Option<String>| ModalItem {
            label: label.to_owned(),
            hint: hint.to_owned(),
            detail,
            run: action.is_some(),
            action,
            alt: None,
            charts: Vec::new(),
        };
    let local = match &view.local {
        Scan::NotYet => vec![item(
            "find servers on this machine",
            "",
            vec![
                Line::styled(LOOKS_AT, theme.dim),
                Line::raw(""),
                Line::styled("⏎ look now", theme.dim),
                Line::raw(""),
                Line::styled(ELSEWHERE, theme.dim),
            ],
            Some("/setup scan".into()),
        )],
        Scan::Looking => vec![placeholder("looking for servers…", LOOKS_AT, theme)],
        Scan::Found(list) if list.is_empty() => vec![item(
            "no servers found",
            "",
            vec![
                Line::styled(
                    "None of the servers on this machine in providers.d answered. Start \
                     one, for example `ollama serve` or `llama-server --jinja -m \
                     <model.gguf>`, then look again.",
                    theme.dim,
                ),
                Line::raw(""),
                Line::styled("⏎ look again", theme.dim),
                Line::raw(""),
                Line::styled(ELSEWHERE, theme.dim),
            ],
            Some("/setup scan".into()),
        )],
        Scan::Found(list) => list
            .iter()
            .enumerate()
            .flat_map(|(i, s)| {
                s.models.iter().map(move |m| {
                    let mut detail = vec![
                        Line::styled(format!("{} · {}", s.name, m.id), theme.accent),
                        Line::raw(""),
                        field(theme, "server", s.base_url.clone()),
                        field(
                            theme,
                            "context",
                            m.context_window
                                .map_or_else(|| "unknown".into(), |c| format!("{c} tokens")),
                        ),
                        Line::raw(""),
                    ];
                    if !s.note.is_empty() {
                        detail.push(Line::raw(s.note.clone()));
                        detail.push(Line::raw(""));
                    }
                    detail.push(match &s.configured {
                        Some(id) => Line::styled(
                            format!("⏎ use it (already set up as provider `{id}`)"),
                            theme.dim,
                        ),
                        None => Line::styled(
                            format!(
                                "⏎ add providers.d/{}.toml to your config folder and use this \
                                 model; it becomes the default if none is set",
                                s.id
                            ),
                            theme.dim,
                        ),
                    });
                    detail.push(Line::raw(""));
                    detail.push(Line::styled(ELSEWHERE, theme.dim));
                    item(
                        &format!("{} · {}", s.name, m.id),
                        &m.context_window
                            .map_or_else(String::new, crate::theme::tokens),
                        detail,
                        Some(format!("/setup use {i} {}", m.id)),
                    )
                })
            })
            .collect(),
    };
    let hosted = match view.services.as_deref() {
        None => vec![placeholder(
            "listing services…",
            "Reading the hosted services in providers.d.",
            theme,
        )],
        Some([]) => vec![placeholder(
            "no hosted services configured",
            "providers.d has no hosted services with a subscription or API key. The defaults bundle ships \
             a list of them; an install without them (e.g. `--plucked`) has none. Add a service \
             under Any server with its OpenAI-compatible URL instead.",
            theme,
        )],
        Some(list) => list
            .iter()
            .map(|s| {
                let mut detail = vec![
                    Line::styled(s.name.clone(), theme.accent),
                    Line::raw(""),
                    field(theme, "provider", s.id.clone()),
                    field(theme, "group", s.group.clone()),
                    field(theme, "url", s.base_url.clone()),
                    Line::raw(""),
                ];
                if s.subscription {
                    if let Some((id, error)) = &view.sign_in_error
                        && id == &s.id
                    {
                        detail.push(Line::styled(error.clone(), theme.warn));
                        detail.push(Line::raw("Press Enter to try signing in again."));
                        detail.push(Line::raw(""));
                    }
                    detail.push(Line::styled(
                        if s.ready {
                            "Signed in with plan usage allowed."
                        } else {
                            "Sign in with your subscription in the browser. Allow plan usage to continue."
                        }, theme.dim,
                    ));
                    detail.push(Line::raw("Credentials are saved privately in your data folder."));
                    detail.push(Line::raw("⏎ continue · ^s sign in again"));
                    let failed = view.sign_in_error.as_ref().is_some_and(|(id, _)| id == &s.id);
                    let next = if s.ready && !failed { "models" } else { "login" };
                    return ModalItem {
                        alt: Some(format!("/setup login {}", s.id)),
                        ..item(
                            &s.name, if s.ready { "ready" } else { "needs sign-in" }, detail,
                            Some(format!("/setup {next} {}", s.id)),
                        )
                    };
                }
                let action = if s.ready {
                    detail.push(Line::styled(
                        match &s.key {
                            Some(key) => format!("Its key is read from {}.", key_source(key)),
                            None => "Its key is set.".into(),
                        },
                        theme.ok,
                    ));
                    detail.push(Line::raw(""));
                    detail.push(Line::styled("⏎ pick one of its models", theme.dim));
                    detail.push(Line::styled(
                        format!("{ALT_KEY} replace its key, for example after rotating it"),
                        theme.dim,
                    ));
                    format!("/setup models {}", s.id)
                } else {
                    detail.push(Line::styled(
                        "⏎ enter its API key; quack keeps it in the OS credential store, \
                         and the provider file in your config folder only names it",
                        theme.dim,
                    ));
                    detail.push(Line::styled(
                        "Without a Secret Service (WSL, a server) it is kept in a file only \
                         your user can read instead.",
                        theme.dim,
                    ));
                    if let Some(env) = &s.env {
                        detail.push(Line::raw(""));
                        detail.push(Line::styled(
                            format!("Or set {env} in your shell and start quack again."),
                            theme.dim,
                        ));
                    }
                    format!("/setup key {}", s.id)
                };
                ModalItem {
                    alt: s.ready.then(|| format!("/key {}", s.id)),
                    ..item(
                        &s.name,
                        if s.ready { "ready" } else { "needs a key" },
                        detail,
                        Some(action),
                    )
                }
            })
            .collect(),
    };
    let other = vec![
        item(
            "add a server by its URL",
            "",
            vec![
                Line::styled("Any OpenAI-compatible server", theme.accent),
                Line::raw(""),
                Line::raw(
                    "A model server on another machine or port, or a hosted service that is \
                     not listed. quack asks for its base URL (for example \
                     http://gpu-box:8000/v1) and then its API key, if it needs one.",
                ),
                Line::raw(""),
                Line::styled(
                    "The key goes to the OS credential store; the provider file written to \
                     your config folder only names it. Without a Secret Service (WSL, a \
                     server) it is kept in a file only your user can read instead.",
                    theme.dim,
                ),
                Line::raw(""),
                Line::styled("⏎ start", theme.dim),
            ],
            Some("/setup url".into()),
        ),
        item(
            "write a provider file yourself",
            "",
            vec![
                Line::raw(
                    "A provider is a TOML file in providers.d in your config folder. Two lines \
                     are enough:",
                ),
                Line::raw(""),
                Line::styled("  base_url = \"http://gpu-box:8000/v1\"", theme.accent),
                Line::styled("  api_key = { env = \"MY_KEY\" }", theme.accent),
                Line::raw(""),
                Line::styled(
                    "`quack config paths` shows where the folder is. Run /reload after \
                     adding one.",
                    theme.dim,
                ),
            ],
            None,
        ),
    ];
    let mut tabs = vec![
        ModalTab {
            title: "This machine".into(),
            items: local,
        },
        ModalTab {
            title: "Hosted services".into(),
            items: hosted,
        },
        ModalTab {
            title: "Any server".into(),
            items: other,
        },
    ];
    let checks = setup_checks(&view.settings, theme);
    if !checks.is_empty() {
        tabs.push(ModalTab {
            title: "Checks".into(),
            items: checks,
        });
    }
    Modal::new("Setup", tabs)
}

/// The opt-in background checks that have somewhere to go. An install without those URLs
/// (e.g. `--plucked`) has none, and the tab stays away.
pub(super) fn setup_checks(settings: &[quack_config::SettingRow], theme: &Theme) -> Vec<ModalItem> {
    const CHECKS: [(&str, &str, &str, &str); 2] = [
        (
            "model prices",
            "catalog.refresh",
            "catalog.url",
            "Downloads prices and context windows for models their provider doesn't \
             describe, at most once a day. It only ever fills in those numbers.",
        ),
        (
            "new versions",
            "updates.check",
            "updates.url",
            "Asks for the latest version number at most once a day and tells you here when \
             there is a new one. quack never updates itself.",
        ),
    ];
    let row = |key: &str| settings.iter().find(|r| r.key == key);
    if row("network.offline").is_some_and(|r| r.value == "true") {
        return Vec::new();
    }
    CHECKS
        .iter()
        .filter_map(|(label, key, url_key, about)| {
            let url = row(url_key)?.value.trim_matches('"').to_owned();
            let setting = row(key)?;
            if url.is_empty() {
                return None;
            }
            let on = setting.value == "true";
            let host = quack_config::url_host(&url).to_owned();
            let mut detail = vec![
                Line::styled((*label).to_owned(), theme.accent),
                Line::raw(""),
                Line::raw((*about).to_owned()),
                Line::raw(""),
                field(theme, "contacts", host),
                field(theme, "setting", (*key).to_owned()),
                Line::raw(""),
            ];
            let (hint, action) = if setting.locked {
                detail.push(Line::styled("Locked by the system policy.", theme.dim));
                (if on { "on, locked" } else { "off, locked" }, None)
            } else {
                detail.push(Line::styled(
                    format!(
                        "⏎ turn it {}, from the next start",
                        if on { "off" } else { "on" }
                    ),
                    theme.dim,
                ));
                (
                    if on { "on" } else { "off" },
                    Some(format!("/config set {key} {}", !on)),
                )
            };
            Some(ModalItem {
                label: (*label).to_owned(),
                hint: hint.to_owned(),
                detail,
                run: action.is_some(),
                action,
                alt: None,
                charts: Vec::new(),
            })
        })
        .collect()
}

/// The URL remains copyable when no browser is available. Leaving cancels the listener.
pub(crate) fn sign_in(id: &str, url: Option<&str>, theme: &Theme) -> Modal {
    let link = ModalItem {
        label: if url.is_some() {
            "copy sign-in URL"
        } else {
            "starting sign-in…"
        }
        .into(),
        hint: "waiting for browser".into(),
        detail: vec![
            Line::styled(
                "Continue in your browser and allow plan usage.",
                theme.accent,
            ),
            Line::raw("If it did not open, copy this URL into your browser:"),
            Line::raw(url.unwrap_or("Waiting for a sign-in URL…").to_owned()),
            Line::raw("Press Enter to copy the URL, or Escape to cancel sign-in."),
        ],
        action: url.map(str::to_owned),
        run: false,
        alt: None,
        charts: Vec::new(),
    };
    let mut modal = Modal::new(
        format!("Sign in: {id}"),
        vec![ModalTab {
            title: "Browser".into(),
            items: vec![link],
        }],
    );
    modal.copy_tab = Some(0);
    modal
}
