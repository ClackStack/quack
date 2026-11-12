//! The shipped profiles: each loads without warnings, names its source, offers its own tool
//! names, and its calls reach the internal tools.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use quack_config::{LoadOptions, Loaded, load_with};
use quack_core::protocol::{Event, InputItem, Op, TurnOverrides};
use quack_core::{Engine, EngineHandle};
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn defaults_bundle() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../defaults")
}

fn sse(chunk: Value) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(format!("data: {chunk}\n\ndata: [DONE]\n\n"))
}

fn configure(root: &Path, base_url: &str, profile: &str) -> Loaded {
    configure_models(
        root,
        base_url,
        &format!("[{{ id = \"tiny\", profile = \"{profile}\" }}]"),
    )
}

fn configure_models(root: &Path, base_url: &str, models: &str) -> Loaded {
    let user = root.join("user");
    std::fs::create_dir_all(user.join("providers.d")).expect("mkdir");
    std::fs::write(
        user.join("providers.d").join("mock.toml"),
        format!("base_url = \"{base_url}/v1\"\nmodels = {models}\n"),
    )
    .expect("provider");
    std::fs::write(
        user.join("config.toml"),
        "[permissions]\nmode = \"full-auto\"\n",
    )
    .expect("config");
    let mut vars: HashMap<String, OsString> = HashMap::new();
    vars.insert("QUACK_CONFIG_DIR".into(), user.into_os_string());
    for (k, sub) in [
        ("QUACK_SYSTEM_DIR", "system"),
        ("QUACK_DATA_DIR", "data"),
        ("QUACK_CACHE_DIR", "cache"),
    ] {
        vars.insert(k.into(), root.join(sub).into_os_string());
    }
    vars.insert(
        "QUACK_DEFAULTS_DIR".into(),
        defaults_bundle().into_os_string(),
    );
    let ws = root.join("ws");
    std::fs::create_dir_all(&ws).expect("mkdir");
    load_with(&move |k| vars.get(k).cloned(), &LoadOptions::new(ws)).expect("load")
}

async fn next(engine: &mut EngineHandle) -> Event {
    tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
        .await
        .expect("event within 10s")
        .expect("channel open")
}

async fn turn(engine: &mut EngineHandle, text: &str) {
    engine
        .submit(Op::UserTurn {
            items: vec![InputItem::text(text)],
            overrides: TurnOverrides::default(),
        })
        .await;
    loop {
        match next(engine).await {
            Event::TurnComplete { .. } => return,
            Event::TurnAborted { reason, .. } => panic!("aborted: {reason}"),
            Event::Error { error } => panic!("engine error: {}", error.message),
            _ => {}
        }
    }
}

fn shipped() -> Vec<String> {
    let mut ids: Vec<String> = std::fs::read_dir(defaults_bundle().join("profiles.d"))
        .expect("profiles.d")
        .flatten()
        .filter(|e| e.path().join("profile.toml").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort();
    ids
}

/// The request's tools: name → parameter names.
fn tools(body: &Value) -> HashMap<String, Vec<String>> {
    body["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|t| {
            let f = &t["function"];
            let params = f["parameters"]["properties"]
                .as_object()
                .map(|p| p.keys().cloned().collect())
                .unwrap_or_default();
            (f["name"].as_str().unwrap_or_default().to_owned(), params)
        })
        .collect()
}

#[tokio::test]
async fn every_shipped_profile_loads_cleanly() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(
            json!({"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]}),
        ))
        .mount(&server)
        .await;
    for profile in shipped() {
        let file: quack_config::profile::ProfileFile = toml::from_str(
            &std::fs::read_to_string(
                defaults_bundle()
                    .join("profiles.d")
                    .join(&profile)
                    .join("profile.toml"),
            )
            .expect("read"),
        )
        .expect("parse");
        assert!(!file.source.trim().is_empty(), "{profile}: no source");

        let tmp = tempfile::tempdir().expect("tempdir");
        let mut engine = Engine::spawn(Arc::new(configure(tmp.path(), &server.uri(), &profile)));
        let mut warnings = Vec::new();
        loop {
            match next(&mut engine).await {
                Event::Warning { message } => warnings.push(message),
                Event::StartupComplete => break,
                _ => {}
            }
        }
        let ours: Vec<&String> = warnings.iter().filter(|w| w.contains("profile")).collect();
        assert!(ours.is_empty(), "{profile}: {ours:?}");
        turn(&mut engine, &format!("hello {profile}")).await;
        engine.shutdown().await;

        let body: Value = server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter_map(|r| serde_json::from_slice::<Value>(&r.body).ok())
            .rfind(|b| b.to_string().contains(&format!("hello {profile}")))
            .expect("request");
        let offered = tools(&body);
        // Skills, language servers, and vision are capability-gated.
        for t in file
            .tools
            .iter()
            .filter(|t| !matches!(t.maps_to.as_deref(), Some("skill" | "lsp" | "view_image")))
        {
            assert!(
                offered.contains_key(&t.name),
                "{profile}: `{}` not offered",
                t.name
            );
            for p in t.params.keys() {
                assert!(
                    offered[&t.name].contains(p),
                    "{profile}: `{}` has no parameter `{p}`: {:?}",
                    t.name,
                    offered[&t.name]
                );
            }
        }
        let system = body["messages"][0]["content"].as_str().expect("system");
        assert!(
            !system.contains("{{") && !system.contains("{%"),
            "{profile}"
        );
    }
}

#[test]
fn codex_matches_only_the_current_openai_model_ids() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = configure(tmp.path(), "http://localhost:1", "auto");
    let (profiles, warnings) = quack_profiles::load_profiles(&loaded);
    assert!(warnings.is_empty(), "{warnings:?}");
    let codex = profiles.iter().find(|p| p.id == "codex").expect("codex");
    assert_eq!(codex.file.matching.priority, 50);
    assert_eq!(
        codex.file.matching.model,
        ["gpt-6.1-sol", "gpt-6-astra", "gpt-6-luna", "gpt-6.1-terra"]
    );
    for model in ["gpt-6.1-sol", "gpt-6-astra", "gpt-6-luna", "gpt-6.1-terra"] {
        assert_eq!(
            quack_profiles::select(&profiles, "auto", &[model])
                .expect("selection")
                .id,
            "codex",
            "{model}"
        );
        assert_eq!(
            quack_profiles::select(&profiles, "generic", &[model])
                .expect("override")
                .id,
            "generic",
            "{model}"
        );
    }
    for model in [
        "tiny",
        "gpt-oss-120b",
        "gpt-6",
        "sol",
        "astra",
        "luna",
        "terra",
        "gpt-5.6-terra",
        "gpt-6.1-terra-extra",
        "gpt-6.1-sol-extra",
    ] {
        assert_eq!(
            quack_profiles::select(&profiles, "auto", &[model])
                .expect("selection")
                .id,
            "generic",
            "{model}"
        );
    }
    assert_eq!(
        quack_profiles::select(&profiles, "codex", &["gpt-oss-120b"])
            .expect("opt-in")
            .id,
        "codex"
    );
}

#[test]
fn kimi_code_matches_the_kimi_k3_family_and_the_plan_models() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = configure(tmp.path(), "http://localhost:1", "auto");
    let (profiles, warnings) = quack_profiles::load_profiles(&loaded);
    assert!(warnings.is_empty(), "{warnings:?}");
    let kimi = profiles
        .iter()
        .find(|p| p.id == "kimi-code")
        .expect("kimi-code");
    assert_eq!(kimi.file.matching.priority, 50);
    // Every Kimi model the bundled provider files ship must auto-select the
    // profile, whatever the provider names it (moonshot/charm-hyper `kimi-k3*`,
    // fireworks' accounts-prefixed id, the Kimi Code plan's short ids). The
    // multi-family providers keep their other models on their own profiles.
    for provider in ["moonshot", "charm-hyper", "fireworks", "kimi-code"] {
        let path = defaults_bundle()
            .join("providers.d")
            .join(format!("{provider}.toml"));
        let file: quack_config::provider::ProviderFile =
            toml::from_str(&std::fs::read_to_string(path).expect("provider file"))
                .expect("provider TOML");
        for model in &file.models {
            let id = if model.id.contains("kimi") || provider == "kimi-code" {
                "kimi-code"
            } else if model.id.contains("glm-") {
                "zcode"
            } else if model.id.contains("deepseek") {
                "dsh"
            } else {
                "generic"
            };
            assert_eq!(
                quack_profiles::select(&profiles, "auto", &[&model.id])
                    .expect("selection")
                    .id,
                id,
                "{provider}/{}",
                model.id
            );
        }
    }
    // Prefixed and aliased K3 ids from other providers (openrouter-style) too.
    for model in [
        "moonshotai/kimi-k3",
        "kimi-k3.1-pro",
        "accounts/fireworks/routers/kimi-k3-fast",
        "kimi-for-coding",
    ] {
        assert_eq!(
            quack_profiles::select(&profiles, "auto", &[model])
                .expect("selection")
                .id,
            "kimi-code",
            "{model}"
        );
        assert_eq!(
            quack_profiles::select(&profiles, "generic", &[model])
                .expect("override")
                .id,
            "generic",
            "{model}"
        );
    }
    // K2 was trained on the retired kimi-cli names, and moving-target aliases
    // are left to `generic` on purpose.
    for model in [
        "kimi-k2",
        "kimi-k2.5",
        "kimi-latest",
        "kimi-fast-latest",
        "tiny",
    ] {
        assert_eq!(
            quack_profiles::select(&profiles, "auto", &[model])
                .expect("selection")
                .id,
            "generic",
            "{model}"
        );
    }
    assert_eq!(
        quack_profiles::select(&profiles, "kimi-code", &["tiny"])
            .expect("opt-in")
            .id,
        "kimi-code"
    );
}

#[test]
fn current_openai_provider_models_have_conservative_context_and_vision() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = configure(tmp.path(), "http://localhost:1", "auto");
    let (profiles, warnings) = quack_profiles::load_profiles(&loaded);
    assert!(warnings.is_empty(), "{warnings:?}");
    for provider in ["openai", "chatgpt"] {
        let path = defaults_bundle()
            .join("providers.d")
            .join(format!("{provider}.toml"));
        let file: quack_config::provider::ProviderFile =
            toml::from_str(&std::fs::read_to_string(path).expect("provider file"))
                .expect("provider TOML");
        assert_eq!(file.models.len(), 4, "{provider}");
        for model in file.models {
            let window = if model.id == "gpt-6.1-terra" {
                500000
            } else {
                272000
            };
            assert_eq!(
                model.context_window,
                Some(window),
                "{provider}/{}",
                model.id
            );
            assert!(model.vision, "{provider}/{}", model.id);
            assert_eq!(
                quack_profiles::select(&profiles, &model.profile, &[&model.id])
                    .expect("selection")
                    .id,
                "codex",
                "{provider}/{}",
                model.id
            );
        }
    }
}

#[test]
fn removing_codex_leaves_the_generic_fallback() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut loaded = configure(tmp.path(), "http://localhost:1", "auto");
    let bundle = tmp.path().join("defaults");
    let codex = bundle.join("profiles.d").join("codex");
    std::fs::create_dir_all(&codex).expect("mkdir");
    for name in ["profile.toml", "system.md"] {
        std::fs::copy(
            defaults_bundle().join("profiles.d/codex").join(name),
            codex.join(name),
        )
        .expect("copy Codex");
    }
    loaded.dirs.defaults = Some(bundle);
    let (profiles, warnings) = quack_profiles::load_profiles(&loaded);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        quack_profiles::select(&profiles, "auto", &["gpt-6.1-sol"])
            .expect("selection")
            .id,
        "codex"
    );
    std::fs::remove_dir_all(codex).expect("remove Codex");
    let (profiles, warnings) = quack_profiles::load_profiles(&loaded);
    assert!(warnings.is_empty(), "{warnings:?}");
    for model in ["gpt-6.1-sol", "gpt-6-astra", "gpt-6-luna", "gpt-6.1-terra"] {
        assert_eq!(
            quack_profiles::select(&profiles, "auto", &[model])
                .expect("fallback")
                .id,
            "generic",
            "{model}"
        );
    }
}

#[tokio::test]
async fn codex_exposes_task_and_gates_view_image_on_vision() {
    for vision in [false, true] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(sse(
                json!({"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]}),
            ))
            .mount(&server)
            .await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let models = format!("[{{ id = \"gpt-6.1-sol\", vision = {vision} }}]");
        let mut engine = Engine::spawn(Arc::new(configure_models(
            tmp.path(),
            &server.uri(),
            &models,
        )));
        turn(&mut engine, "check tools").await;
        engine.shutdown().await;
        let requests = server.received_requests().await.expect("requests");
        let body: Value =
            serde_json::from_slice(&requests.last().expect("request").body).expect("request JSON");
        let offered = tools(&body);
        assert!(offered.contains_key("task"), "{offered:?}");
        assert!(
            offered.contains_key("apply_patch"),
            "automatic Codex: {offered:?}"
        );
        assert!(offered.contains_key("update_plan"), "{offered:?}");
        assert_eq!(offered.contains_key("view_image"), vision, "{offered:?}");
        for name in [
            "spawn_agent",
            "wait",
            "send_input",
            "write_stdin",
            "exec_command",
        ] {
            assert!(!offered.contains_key(name), "{offered:?}");
        }
    }
}

#[tokio::test]
async fn codex_shell_takes_an_argument_list() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("\"role\":\"tool\""))
        .respond_with(sse(
            json!({"choices":[{"delta":{"content":"done"},"finish_reason":"stop"}]}),
        ))
        .mount(&server)
        .await;
    let args = json!({"command": ["bash", "-lc", "echo argv-ok"]}).to_string();
    Mock::given(method("POST"))
        .respond_with(sse(json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"shell","arguments":args}}]},"finish_reason":"tool_calls"}]})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut engine = Engine::spawn(Arc::new(configure(tmp.path(), &server.uri(), "codex")));
    turn(&mut engine, "run it").await;
    engine.shutdown().await;
    let bodies: Vec<Value> = server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter_map(|r| serde_json::from_slice::<Value>(&r.body).ok())
        .collect();
    let offered = tools(&bodies[0]);
    let mut names: Vec<&str> = offered.keys().map(String::as_str).collect();
    names.sort_unstable();
    // Exclusive: only its own tools (memory and skill are offered when available).
    assert!(
        names.iter().all(|n| [
            "apply_patch",
            "ask_user",
            "memory",
            "read",
            "shell",
            "skill",
            "task",
            "update_plan",
            "view_image"
        ]
        .contains(n)),
        "{names:?}"
    );
    assert_eq!(
        bodies[0]["tools"]
            .as_array()
            .and_then(|t| t.iter().find(|t| t["function"]["name"] == "shell"))
            .map(|t| t["function"]["parameters"]["properties"]["command"]["type"].clone()),
        Some(json!("array"))
    );
    let last = bodies.last().expect("second request").to_string();
    assert!(last.contains("argv-ok"), "{last}");
}

/// Encode internal arguments using the shipped profile's advertised names and line bases.
fn profile_call(
    file: &quack_config::profile::ProfileFile,
    internal: &str,
    args: Value,
) -> (String, Value) {
    use quack_config::profile::Transform;

    let Some(mapping) = file
        .tools
        .iter()
        .find(|t| t.maps_to.as_deref().unwrap_or(&t.name) == internal)
    else {
        assert!(
            !file.exclusive,
            "exclusive profile does not expose {internal}"
        );
        return (internal.to_owned(), args);
    };
    let args = args
        .as_object()
        .expect("call arguments")
        .iter()
        .filter(|(key, _)| !mapping.fixed.contains_key(*key))
        .map(|(key, value)| {
            let Some((name, param)) = mapping.params.iter().find(|(_, p)| p.to == *key) else {
                return (key.clone(), value.clone());
            };
            let value = match param.transform {
                Some(Transform::ZeroToOneBased) => {
                    json!(value.as_u64().expect("line number") - 1)
                }
                Some(Transform::OneToZeroBased) => {
                    json!(value.as_u64().expect("line number") + 1)
                }
                Some(Transform::AbsToWorkspace) | None => value.clone(),
                other => panic!("unsupported regression argument transform: {other:?}"),
            };
            (name.clone(), value)
        })
        .collect();
    (mapping.name.clone(), Value::Object(args))
}

#[tokio::test]
async fn every_shipped_profile_can_edit_after_a_tracked_read() {
    use quack_config::settings::EditFormat;

    for profile in shipped() {
        let server = MockServer::start().await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let config = configure(tmp.path(), &server.uri(), &profile);
        let (profiles, warnings) = quack_profiles::load_profiles(&config);
        assert!(warnings.is_empty(), "{profile}: {warnings:?}");
        let file = &profiles
            .iter()
            .find(|p| p.id == profile)
            .expect("shipped profile")
            .file;
        // Exercise each profile's default edit format, not a config override.
        let (editor, edit_args) = match file.defaults.edit_format {
            Some(EditFormat::ApplyPatch) => (
                "apply_patch",
                json!({
                    "patch": "*** Begin Patch\n*** Update File: a.txt\n@@\n-one\n+two\n*** End Patch"
                }),
            ),
            None | Some(EditFormat::Auto | EditFormat::StrReplace) => (
                "edit",
                json!({"path": "a.txt", "old_string": "one", "new_string": "two"}),
            ),
            other => panic!("{profile}: unsupported default edit format: {other:?}"),
        };
        let (editor, edit_args) = profile_call(file, editor, edit_args);
        // Include a range so Qwen's zero-based offset and Kimi's aliases are exercised.
        let (reader, read_args) = profile_call(
            file,
            "read",
            json!({"path": "a.txt", "offset": 1, "limit": 1}),
        );
        let mut replies = std::collections::VecDeque::new();
        for (id, name, args) in [
            ("blind", &editor, edit_args.clone()),
            ("read", &reader, read_args.clone()),
            ("edited", &editor, edit_args.clone()),
        ] {
            replies.push_back(sse(json!({"choices":[{"delta":{"tool_calls":[{
                "index":0,"id":id,"function":{"name":name,"arguments":args.to_string()}
            }]},"finish_reason":"tool_calls"}]})));
        }
        replies.push_back(sse(
            json!({"choices":[{"delta":{"content":"done"},"finish_reason":"stop"}]}),
        ));
        let replies = std::sync::Mutex::new(replies);
        Mock::given(method("POST"))
            .respond_with(move |_: &wiremock::Request| {
                replies
                    .lock()
                    .expect("script lock")
                    .pop_front()
                    .expect("unexpected extra request")
            })
            .expect(4)
            .mount(&server)
            .await;
        let path = tmp.path().join("ws").join("a.txt");
        std::fs::write(&path, "one\n").expect("file");
        let mut engine = Engine::spawn(Arc::new(config));
        turn(&mut engine, "change a.txt").await;
        engine.shutdown().await;

        let bodies: Vec<Value> = server
            .received_requests()
            .await
            .expect("requests")
            .iter()
            .filter(|r| r.method == "POST")
            .map(|r| serde_json::from_slice(&r.body).expect("request JSON"))
            .collect();
        assert_eq!(bodies.len(), 4, "{profile}");
        for body in &bodies {
            let offered = tools(body);
            if file.defaults.edit_format == Some(EditFormat::ApplyPatch) {
                for name in ["edit", "edit_hashline", "edit_blocks", "write"] {
                    assert!(!offered.contains_key(name), "{profile}: {offered:?}");
                }
            }
            for (name, args) in [(&reader, &read_args), (&editor, &edit_args)] {
                let params = offered
                    .get(name)
                    .unwrap_or_else(|| panic!("{profile}: {name} not offered: {offered:?}"));
                for key in args.as_object().expect("arguments").keys() {
                    assert!(
                        params.contains(key),
                        "{profile}: {name} does not advertise {key}: {params:?}"
                    );
                }
            }
        }
        let results: HashMap<&str, &str> = bodies[3]["messages"]
            .as_array()
            .expect("messages")
            .iter()
            .filter(|m| m["role"] == "tool")
            .map(|m| {
                (
                    m["tool_call_id"].as_str().expect("call id"),
                    m["content"].as_str().expect("tool content"),
                )
            })
            .collect();
        assert_eq!(results.len(), 3, "{profile}: {results:?}");
        assert!(
            results["blind"].contains("has not been read yet"),
            "{profile}: {results:?}"
        );
        assert!(results["read"].contains("one"), "{profile}: {results:?}");
        assert!(
            results["edited"].contains("Edited a.txt"),
            "{profile}: {results:?}"
        );
        assert_eq!(
            std::fs::read_to_string(path).expect("edited file"),
            "two\n",
            "{profile}"
        );
    }
}

#[tokio::test]
async fn every_nonexclusive_profile_offers_the_explicit_edit_format() {
    use quack_config::settings::EditFormat;

    let formats = [
        (EditFormat::StrReplace, "edit"),
        (EditFormat::Hashline, "edit_hashline"),
        (EditFormat::ApplyPatch, "apply_patch"),
        (EditFormat::SearchReplaceBlocks, "edit_blocks"),
        (EditFormat::WholeFile, "write"),
    ];
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(
            json!({"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]}),
        ))
        .mount(&server)
        .await;
    for profile in shipped() {
        for (format, editor) in formats {
            let tmp = tempfile::tempdir().expect("tempdir");
            let mut loaded = configure(tmp.path(), &server.uri(), &profile);
            let (profiles, warnings) = quack_profiles::load_profiles(&loaded);
            assert!(warnings.is_empty(), "{profile}: {warnings:?}");
            let file = &profiles
                .iter()
                .find(|p| p.id == profile)
                .expect("shipped profile")
                .file;
            // Exclusive profiles deliberately limit overrides; edit_formats.rs tests that guard.
            if file.exclusive {
                continue;
            }
            loaded.settings.tools.edit_format = format;
            let mut engine = Engine::spawn(Arc::new(loaded));
            turn(&mut engine, "check the edit tools").await;
            engine.shutdown().await;

            let requests = server.received_requests().await.expect("requests");
            let body: Value = serde_json::from_slice(&requests.last().expect("request").body)
                .expect("request JSON");
            let offered = tools(&body);
            // Resolve aliases from the shipped mappings, including the tracked reader and writer.
            for internal in ["read", editor, "write"] {
                let (name, _) = profile_call(file, internal, json!({}));
                assert!(
                    offered.contains_key(&name),
                    "{profile}/{format:?}: {internal} ({name}) not offered: {offered:?}"
                );
            }
            // An explicit override must replace the default editor, not add another editor.
            for (_, other) in formats {
                if other == editor || other == "write" {
                    continue;
                }
                let (name, _) = profile_call(file, other, json!({}));
                assert!(
                    !offered.contains_key(&name),
                    "{profile}/{format:?}: inactive {name} offered: {offered:?}"
                );
            }
            let (name, _) = profile_call(file, editor, json!({}));
            let system = body["messages"][0]["content"].as_str().expect("system");
            assert!(
                system.contains(&name),
                "{profile}/{format:?}: prompt does not mention active editor {name}"
            );
            assert!(
                !system.contains("{{") && !system.contains("{%"),
                "{profile}/{format:?}: unrendered prompt"
            );
        }
    }
}

#[tokio::test]
async fn a_repeated_edit_is_noted_under_the_profiles_own_name() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("\"role\":\"tool\""))
        .respond_with(sse(
            json!({"choices":[{"delta":{"content":"done"},"finish_reason":"stop"}]}),
        ))
        .mount(&server)
        .await;
    // cc-style's `Edit` is the internal `edit`; the same failing edit twice in one reply.
    let args =
        json!({"file_path": "missing.txt", "old_string": "a", "new_string": "b"}).to_string();
    let call = |id: &str, index: u32| json!({"index": index, "id": id, "function": {"name": "Edit", "arguments": args}});
    Mock::given(method("POST"))
        .respond_with(sse(json!({"choices":[{"delta":{"tool_calls":[call("c1", 0), call("c2", 1)]},"finish_reason":"tool_calls"}]})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut engine = Engine::spawn(Arc::new(configure(tmp.path(), &server.uri(), "cc-style")));
    turn(&mut engine, "edit it").await;
    engine.shutdown().await;
    let last: Value = server
        .received_requests()
        .await
        .unwrap_or_default()
        .last()
        .and_then(|r| serde_json::from_slice(&r.body).ok())
        .expect("second request");
    let results: Vec<&str> = last["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter(|m| m["role"] == "tool")
        .filter_map(|m| m["content"].as_str())
        .collect();
    assert_eq!(results.len(), 2, "{results:?}");
    assert!(
        !results[0].contains("already been attempted"),
        "{}",
        results[0]
    );
    assert!(
        results[1].contains("already been attempted"),
        "{}",
        results[1]
    );
}

#[tokio::test]
async fn profiles_are_listed_previewed_and_switched() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(
            json!({"choices":[{"delta":{"content":"ok"},"finish_reason":"stop"}]}),
        ))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut engine = Engine::spawn(Arc::new(configure(tmp.path(), &server.uri(), "auto")));
    loop {
        if let Event::StartupComplete = next(&mut engine).await {
            break;
        }
    }
    engine.submit(Op::ListProfiles).await;
    let (active, forced, profiles) = loop {
        if let Event::Profiles {
            active,
            forced,
            profiles,
        } = next(&mut engine).await
        {
            break (active, forced, profiles);
        }
    };
    assert_eq!((active.as_str(), forced), ("generic", false));
    assert_eq!(profiles.len(), shipped().len());
    let qwen = profiles
        .iter()
        .find(|p| p.id == "qwen-code")
        .expect("qwen-code");
    assert!(
        qwen.tools.contains(&"read_file".to_owned()),
        "{:?}",
        qwen.tools
    );
    assert!(
        qwen.system.contains("todo_write"),
        "rendered with its own tool names"
    );
    let codex = profiles.iter().find(|p| p.id == "codex").expect("codex");
    assert_eq!(codex.edit_format, "apply_patch");
    let generic = profiles
        .iter()
        .find(|p| p.id == "generic")
        .expect("generic");
    assert_eq!(generic.edit_format, "str_replace");
    assert!(generic.matches);

    engine
        .submit(Op::SetProfile {
            id: "qwen-code".into(),
        })
        .await;
    let session = loop {
        if let Event::SessionConfigured { session } = next(&mut engine).await {
            break session;
        }
    };
    assert_eq!(session.profile, "qwen-code");
    turn(&mut engine, "after the switch").await;
    // A reload keeps the chosen profile.
    engine.submit(Op::ReloadConfig).await;
    let session = loop {
        if let Event::SessionConfigured { session } = next(&mut engine).await {
            break session;
        }
    };
    assert_eq!(session.profile, "qwen-code");
    engine.shutdown().await;
    let body: Value = server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter_map(|r| serde_json::from_slice::<Value>(&r.body).ok())
        .rfind(|b| b.to_string().contains("after the switch"))
        .expect("request");
    assert!(tools(&body).contains_key("read_file"));
}
