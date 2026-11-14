//! M1 invariants (docs/plan/13-testing-and-evals.md#invariant-tests-must-never-regress):
//! removal of default files, no egress, and resume fidelity.

use std::collections::HashMap;
use std::ffi::OsString;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quack_config::{LoadOptions, Loaded, load_with};
use quack_core::protocol::{
    ApprovalDecision, Event, InputItem, LoadState, Op, Subsystem, TurnOverrides,
};
use quack_core::{Engine, EngineHandle, Resume};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sse(chunks: &[&str]) -> ResponseTemplate {
    let body: String = chunks
        .iter()
        .map(|c| format!("data: {c}\n\n"))
        .collect::<String>()
        + "data: [DONE]\n\n";
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

async fn answering_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse(&[
            r#"{"choices":[{"delta":{"content":"Hi."},"finish_reason":"stop"}]}"#,
        ]))
        .mount(&server)
        .await;
    server
}

fn defaults_bundle() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../defaults")
}

/// An isolated configuration with an optional user-layer `mock` provider, an optional
/// defaults bundle and extra `config.toml` text. `None` provider is the shape of a fresh
/// install: the bundle alone, nothing of the user's own.
fn load_config(
    root: &Path,
    provider: Option<&str>,
    defaults: Option<&Path>,
    config: &str,
) -> Loaded {
    let user = root.join("user");
    std::fs::create_dir_all(user.join("providers.d")).expect("mkdir");
    if let Some(base_url) = provider {
        std::fs::write(
            user.join("providers.d").join("mock.toml"),
            format!("base_url = \"{base_url}/v1\"\nmodels = [\"tiny\"]\n"),
        )
        .expect("provider");
    }
    std::fs::write(user.join("config.toml"), config).expect("config");
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
        defaults.map_or_else(|| "none".into(), |d| d.as_os_str().to_owned()),
    );
    let ws = root.join("ws");
    std::fs::create_dir_all(&ws).expect("mkdir");
    load_with(&move |k| vars.get(k).cloned(), &LoadOptions::new(ws)).expect("load")
}

/// An isolated configuration with a user-layer `mock` provider.
fn configure(root: &Path, base_url: &str, defaults: Option<&Path>, config: &str) -> Loaded {
    load_config(root, Some(base_url), defaults, config)
}

async fn next(rx: &mut tokio::sync::broadcast::Receiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("event within 10s")
        .expect("channel open")
}

/// Final state of every subsystem, plus warnings and errors, up to `StartupComplete`.
#[derive(Debug, Default)]
struct Startup {
    states: HashMap<Subsystem, LoadState>,
    warnings: Vec<String>,
    errors: Vec<String>,
}

async fn startup(engine: &mut EngineHandle) -> Startup {
    let mut s = Startup::default();
    loop {
        match next(&mut engine.events).await {
            Event::StartupProgress { subsystem, state } => {
                s.states.insert(subsystem, state);
            }
            Event::Warning { message } => s.warnings.push(message),
            Event::Error { error } => s.errors.push(error.message),
            Event::StartupComplete => return s,
            _ => {}
        }
    }
}

/// Run one turn, approving whatever is asked.
async fn turn(engine: &mut EngineHandle, text: &str) {
    engine
        .submit(Op::UserTurn {
            items: vec![InputItem::text(text)],
            overrides: TurnOverrides::default(),
        })
        .await;
    loop {
        match next(&mut engine.events).await {
            Event::ApprovalRequest { call_id, .. } => {
                engine
                    .submit(Op::ApprovalResponse {
                        call_id,
                        decision: ApprovalDecision::AllowOnce,
                    })
                    .await;
            }
            Event::TurnComplete { .. } => return,
            Event::TurnAborted { reason, .. } => panic!("aborted: {reason}"),
            Event::Error { error } => panic!("engine error: {}", error.message),
            _ => {}
        }
    }
}

/// Run one turn that is expected to fail: collect the engine's error messages instead of
/// panicking on them, until the turn ends one way or the other.
async fn turn_errors(engine: &mut EngineHandle, text: &str) -> Vec<String> {
    engine
        .submit(Op::UserTurn {
            items: vec![InputItem::text(text)],
            overrides: TurnOverrides::default(),
        })
        .await;
    let mut errors = Vec::new();
    loop {
        match next(&mut engine.events).await {
            Event::ApprovalRequest { call_id, .. } => {
                engine
                    .submit(Op::ApprovalResponse {
                        call_id,
                        decision: ApprovalDecision::AllowOnce,
                    })
                    .await;
            }
            Event::TurnComplete { .. } | Event::TurnAborted { .. } => return errors,
            Event::Error { error } => errors.push(error.message),
            _ => {}
        }
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("read_dir").flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("copy");
        }
    }
}

fn toml_files(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |d| {
        d.flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
            .count()
    })
}

/// Every bundled model entry's thinking levels are consistent: a pinned default effort is
/// one of the picker's levels, no level is listed twice, and no level is empty.
#[test]
fn bundled_reasoning_efforts_are_consistent() {
    let providers = defaults_bundle().join("providers.d");
    let mut checked = 0;
    for entry in std::fs::read_dir(&providers)
        .expect("providers.d")
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_some_and(|x| x != "toml") {
            continue;
        }
        let file: quack_config::ProviderFile =
            toml::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
        for model in &file.models {
            let reasoning = &model.reasoning;
            let shown = format!("{} {}", path.display(), model.id);
            if !reasoning.effort.is_empty() {
                assert!(
                    reasoning.efforts.contains(&reasoning.effort),
                    "{shown}: effort {:?} is not one of its efforts {:?}",
                    reasoning.effort,
                    reasoning.efforts
                );
            }
            let mut seen = std::collections::BTreeSet::new();
            for effort in &reasoning.efforts {
                assert!(!effort.is_empty(), "{shown}: empty effort level");
                assert!(seen.insert(effort), "{shown}: effort {effort} listed twice");
            }
            checked += 1;
        }
    }
    assert!(
        checked > 20,
        "expected the bundle's model entries, got {checked}"
    );
}

/// Everything in the defaults bundle a user or company might delete, relative to it.
fn removable(defaults: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(defaults).expect("defaults").flatten() {
        let name = PathBuf::from(entry.file_name());
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            out.extend(
                std::fs::read_dir(entry.path())
                    .expect("read_dir")
                    .flatten()
                    .map(|e| name.join(e.file_name())),
            );
        }
        out.push(name);
    }
    out.sort();
    out
}

#[tokio::test]
async fn removing_any_default_file_leaves_startup_clean() {
    let server = answering_server().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let bundle = defaults_bundle();

    let run = |defaults: PathBuf, root: PathBuf| {
        let uri = server.uri();
        async move {
            let loaded = configure(&root, &uri, Some(&defaults), "");
            assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
            let mut engine = Engine::spawn(Arc::new(loaded));
            let s = startup(&mut engine).await;
            engine.shutdown().await;
            s
        }
    };
    let providers = |s: &Startup| match s.states.get(&Subsystem::Config) {
        Some(LoadState::Ready { detail }) => detail
            .split(" · ")
            .find_map(|p| p.strip_suffix(" providers")?.parse::<usize>().ok())
            .expect("provider count"),
        other => panic!("config: {other:?}"),
    };

    let baseline = run(bundle.clone(), tmp.path().join("baseline")).await;
    assert_eq!(
        providers(&baseline),
        toml_files(&bundle.join("providers.d")) + 1
    );

    let items = removable(&bundle);
    assert!(items.len() > 10, "{items:?}");
    for (n, item) in items.iter().enumerate() {
        let defaults = tmp.path().join(format!("defaults-{n}"));
        copy_dir(&bundle, &defaults);
        let gone = defaults.join(item);
        if gone.is_dir() {
            std::fs::remove_dir_all(&gone).expect("remove");
        } else {
            std::fs::remove_file(&gone).expect("remove");
        }
        let s = run(defaults.clone(), tmp.path().join(format!("run-{n}"))).await;
        let shown = item.display();

        assert!(s.errors.is_empty(), "{shown}: {:?}", s.errors);
        assert_eq!(s.warnings, baseline.warnings, "{shown}");
        for (subsystem, state) in &s.states {
            assert!(
                !matches!(state, LoadState::Failed { .. }),
                "{shown}: {subsystem:?} {state:?}"
            );
        }
        assert!(
            matches!(s.states.get(&Subsystem::Provider), Some(LoadState::Ready { detail }) if detail.starts_with("mock/tiny")),
            "{shown}: {:?}",
            s.states.get(&Subsystem::Provider)
        );
        // Only the removed item is missing: every remaining provider file loads.
        assert_eq!(
            providers(&s),
            toml_files(&defaults.join("providers.d")) + 1,
            "{shown}"
        );
    }
}

/// A forward proxy that records every destination and only passes requests to `allowed`.
struct RecordingProxy {
    url: String,
    seen: Arc<Mutex<Vec<String>>>,
}

impl RecordingProxy {
    async fn start(allowed: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("addr"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let log = log.clone();
                tokio::spawn(async move {
                    let _ = forward(client, allowed, &log).await;
                });
            }
        });
        Self { url, seen }
    }

    fn destinations(&self) -> Vec<String> {
        let mut d = self.seen.lock().map(|s| s.clone()).unwrap_or_default();
        d.sort();
        d.dedup();
        d
    }
}

/// Handle one proxied request. Plain-HTTP requests to `allowed` are passed on with
/// `connection: close`; anything else (including every `CONNECT`) is refused.
async fn forward(
    mut client: TcpStream,
    allowed: SocketAddr,
    log: &Mutex<Vec<String>>,
) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        let n = client.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut request = lines.next().unwrap_or_default().splitn(3, ' ');
    let (verb, target, version) = (
        request.next().unwrap_or_default(),
        request.next().unwrap_or_default(),
        request.next().unwrap_or_default(),
    );
    let (authority, origin_form) = match target.strip_prefix("http://") {
        Some(rest) => match rest.split_once('/') {
            Some((a, p)) => (a.to_owned(), format!("/{p}")),
            None => (rest.to_owned(), "/".to_owned()),
        },
        None => (target.to_owned(), String::new()),
    };
    if let Ok(mut l) = log.lock() {
        l.push(format!("{verb} {authority}"));
    }
    if verb == "CONNECT" || authority != allowed.to_string() {
        client
            .write_all(b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .await?;
        return Ok(());
    }

    let headers: Vec<&str> = lines
        .filter(|l| {
            let l = l.to_ascii_lowercase();
            !l.is_empty() && !l.starts_with("connection:") && !l.starts_with("proxy-connection:")
        })
        .collect();
    let length = headers
        .iter()
        .find_map(|h| {
            let (k, v) = h.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    while buf.len() < head_end + length {
        let n = client.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let mut upstream = TcpStream::connect(allowed).await?;
    let mut out = format!("{verb} {origin_form} {version}\r\n");
    for h in headers {
        out.push_str(h);
        out.push_str("\r\n");
    }
    out.push_str("connection: close\r\n\r\n");
    upstream.write_all(out.as_bytes()).await?;
    upstream.write_all(&buf[head_end..]).await?;
    tokio::io::copy(&mut upstream, &mut client).await?;
    client.shutdown().await
}

fn reply_text(t: &str) -> ResponseTemplate {
    sse(&[&format!(
        r#"{{"choices":[{{"delta":{{"content":"{t}"}},"finish_reason":"stop"}}]}}"#
    )])
}

fn reply_call(name: &str, args: &str) -> ResponseTemplate {
    sse(&[&format!(
        r#"{{"choices":[{{"delta":{{"tool_calls":[{{"index":0,"id":"c1","function":{{"name":"{name}","arguments":"{args}"}}}}]}},"finish_reason":"tool_calls"}}]}}"#
    )])
}

/// A provider that walks a fixed script: three plain replies, then a `read` tool call and
/// the continuation after its result, then a `task` sub-agent call (whose nested turn makes
/// a request of its own in the middle) and the final answer. Each mock answers once
/// (`up_to_n_times`), so wiremock's first-match rule walks the script in order; the
/// continuation mocks also require a tool result in the history, proving the tool round
/// trips really happened.
async fn scripted_server() -> MockServer {
    let server = MockServer::start().await;
    let script: Vec<(bool, ResponseTemplate)> = vec![
        (false, reply_text("One.")),
        (false, reply_text("Two.")),
        (false, reply_text("Three.")),
        (false, reply_call("read", r#"{\"path\":\"note.txt\"}"#)),
        (true, reply_text("Read it.")),
        (
            false,
            reply_call("task", r#"{\"agent\":\"explore\",\"prompt\":\"look\"}"#),
        ),
        (false, reply_text("Found.")),
        (true, reply_text("Done.")),
    ];
    for (after_tool, reply) in script {
        let mut matcher = Mock::given(method("POST")).and(path("/v1/chat/completions"));
        if after_tool {
            matcher = matcher.and(body_string_contains("\"role\":\"tool\""));
        }
        matcher
            .respond_with(reply)
            .up_to_n_times(1)
            .mount(&server)
            .await;
    }
    server
}

/// Startup plus plain turns, a tool-call turn and a sub-agent turn, with the defaults
/// bundle installed, reaches nothing but the configured provider, with `offline` on and
/// off. All provider traffic goes through a recording proxy, so any other destination
/// would show up.
#[tokio::test]
async fn no_egress_beyond_the_configured_provider() {
    let tmp = tempfile::tempdir().expect("tempdir");
    for offline in [false, true] {
        let server = scripted_server().await;
        let mock = *server.address();
        let proxy = RecordingProxy::start(mock).await;
        let loaded = configure(
            tmp.path(),
            &server.uri(),
            Some(&defaults_bundle()),
            &format!(
                "[network]\noffline = {offline}\nproxy = \"{}\"\n",
                proxy.url
            ),
        );
        std::fs::write(tmp.path().join("ws").join("note.txt"), "a note\n").expect("note");
        let mut engine = Engine::spawn(Arc::new(loaded));
        let s = startup(&mut engine).await;
        assert!(
            matches!(
                s.states.get(&Subsystem::Provider),
                Some(LoadState::Ready { .. })
            ),
            "{s:?}"
        );
        for text in ["one", "two", "three", "read the note", "delegate a look"] {
            turn(&mut engine, text).await;
        }
        let requests = server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|r| r.url.path() == "/v1/chat/completions")
            .count();
        engine.shutdown().await;
        // Three plain turns (3 requests), the read round trip (2) and the sub-agent (its
        // own request plus the parent's continuation, 3): the whole script ran. (Startup's
        // one `GET /v1/models`, which fills model metadata, is not a turn request.)
        assert_eq!(requests, 8, "the scripted exchange did not run in full");
        let allowed = format!("POST {mock}");
        let destinations = proxy.destinations();
        assert!(
            destinations.contains(&allowed),
            "turns went through the proxy: {destinations:?}"
        );
        assert!(
            destinations.iter().all(|d| d.ends_with(&mock.to_string())),
            "offline = {offline}: unexpected destinations {destinations:?}"
        );
    }
}

/// A provider whose endpoint answers with a redirect to another host cannot step around
/// offline mode: every redirect hop is checked against the same gate as the first
/// request, so the turn fails without anything being sent towards the redirect target.
#[tokio::test]
async fn a_provider_redirect_cannot_step_around_offline_mode() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", "http://elsewhere.example/v1/chat/completions"),
        )
        .mount(&server)
        .await;
    let mock = *server.address();
    let proxy = RecordingProxy::start(mock).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = configure(
        tmp.path(),
        &server.uri(),
        Some(&defaults_bundle()),
        &format!("[network]\noffline = true\nproxy = \"{}\"\n", proxy.url),
    );
    let mut engine = Engine::spawn(Arc::new(loaded));
    startup(&mut engine).await;
    let errors = turn_errors(&mut engine, "hello").await;
    assert!(
        errors
            .iter()
            .any(|e| e.contains("network.offline") && e.contains("elsewhere.example")),
        "the gate's refusal should surface, got {errors:?}"
    );
    engine.shutdown().await;
    let destinations = proxy.destinations();
    assert!(
        destinations.iter().all(|d| d.ends_with(&mock.to_string())),
        "the redirect target was contacted: {destinations:?}"
    );
}

/// A fresh install: the defaults bundle, no provider of the user's own, no keys. Startup
/// may probe the bundle's local servers (it needs a default model, and only servers on
/// this machine answer without a key), and the turn a user then starts reaches for the
/// same ones - but nothing a fresh install does ever leaves the machine.
#[tokio::test]
async fn a_fresh_install_stays_on_this_machine() {
    let black_hole: SocketAddr = "127.0.0.1:9".parse().expect("addr");
    let proxy = RecordingProxy::start(black_hole).await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = load_config(
        tmp.path(),
        None,
        Some(&defaults_bundle()),
        &format!("[network]\nproxy = \"{}\"\n", proxy.url),
    );
    let mut engine = Engine::spawn(Arc::new(loaded));
    startup(&mut engine).await;
    let startup_destinations = proxy.destinations();
    assert!(
        startup_destinations.iter().all(|d| d.contains("127.0.0.1")),
        "startup reached off this machine: {startup_destinations:?}"
    );
    let errors = turn_errors(&mut engine, "hello").await;
    assert!(
        !errors.is_empty(),
        "the turn should fail without a reachable provider"
    );
    engine.shutdown().await;
    let destinations = proxy.destinations();
    assert!(
        destinations.iter().all(|d| d.contains("127.0.0.1")),
        "a fresh install reached off this machine: {destinations:?}"
    );
}

/// The opt-in checks reach the configured host when turned on, and `offline` stops them
/// before anything leaves.
#[tokio::test]
async fn offline_stops_the_opt_in_checks() {
    let server = answering_server().await;
    let mock = *server.address();
    for offline in [false, true] {
        let proxy = RecordingProxy::start(mock).await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let loaded = configure(
            tmp.path(),
            &server.uri(),
            Some(&defaults_bundle()),
            &format!(
                "[network]\noffline = {offline}\nproxy = \"{}\"\n\
                 [catalog]\nrefresh = true\nurl = \"http://feed.example/api.json\"\n\
                 [updates]\ncheck = true\nurl = \"http://releases.example/latest\"\n",
                proxy.url
            ),
        );
        let mut engine = Engine::spawn(Arc::new(loaded));
        startup(&mut engine).await;
        // The state file is written after each check, so both have run once it says so.
        let state = tmp.path().join("cache").join("checks.json");
        for _ in 0..500 {
            let text = std::fs::read_to_string(&state).unwrap_or_default();
            if !text.contains("\"updates_checked\":0") && !text.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        engine.shutdown().await;
        let destinations = proxy.destinations();
        let reached = |host: &str| destinations.iter().any(|d| d.contains(host));
        assert_eq!(reached("feed.example"), !offline, "{destinations:?}");
        assert_eq!(reached("releases.example"), !offline, "{destinations:?}");
    }
}

/// A system policy's `hosts` disables every provider elsewhere, including the bundle's and
/// the user's own, so nothing else can be contacted or picked.
#[test]
fn policy_hosts_leave_only_allowed_servers() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let system = tmp.path().join("system");
    std::fs::create_dir_all(&system).expect("mkdir");
    std::fs::write(
        system.join("config.toml"),
        "[policy.providers]\nhosts = [\"127.0.0.1\"]\n",
    )
    .expect("policy");
    let loaded = configure(
        tmp.path(),
        "https://elsewhere.example",
        Some(&defaults_bundle()),
        "",
    );
    let items = quack_config::load_providers(&loaded);
    assert!(items.len() > 10, "the bundle loaded");
    for item in &items {
        match item.loaded() {
            Some(f) => assert!(
                quack_config::url_host(&f.base_url) == "127.0.0.1",
                "{} at {} is allowed",
                item.id,
                f.base_url
            ),
            None => assert!(
                matches!(&item.state, quack_config::ItemState::Disabled(why) if why.contains("policy")),
                "{}: {:?}",
                item.id,
                item.state
            ),
        }
    }
    assert!(items.iter().any(|i| i.id == "mock" && i.loaded().is_none()));
}

/// The request after a resume is byte-for-byte the request the live session would have
/// sent: same system prompt, tools, history (tool calls, results, reasoning), plan
/// reminder and params.
#[tokio::test]
async fn resume_reproduces_the_request_exactly() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("Created notes.txt"))
        .respond_with(sse(&[
            r#"{"choices":[{"delta":{"reasoning_content":"The file exists now."}}]}"#,
            r#"{"choices":[{"delta":{"content":"Done."},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":300,"completion_tokens":4}}"#,
        ]))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse(&[
            r#"{"choices":[{"delta":{"content":"Writing it."}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"write","arguments":"{\"path\":\"notes.txt\",\"content\":\"hi\\n\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"c2","function":{"name":"todo","arguments":"{\"items\":[{\"text\":\"write notes\",\"status\":\"done\"},{\"text\":\"answer questions\",\"status\":\"in_progress\"}]}"}}]},"finish_reason":"tool_calls"}]}"#,
        ]))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let spawn = |resume| {
        // The defaults bundle supplies the shipped `generic` profile and its system prompt.
        let loaded = configure(root, &server.uri(), Some(&defaults_bundle()), "");
        Engine::spawn_with(Arc::new(loaded), resume)
    };

    let mut live = spawn(Resume::New);
    turn(&mut live, "write notes").await;
    turn(&mut live, "second question").await;
    // Each turn ends with an fsync, so this is the state a crash or quit would leave.
    copy_dir(&root.join("data"), &root.join("saved"));
    turn(&mut live, "third question").await;
    live.shutdown().await;

    std::fs::remove_dir_all(root.join("data")).expect("remove");
    copy_dir(&root.join("saved"), &root.join("data"));
    std::fs::remove_file(root.join("ws").join("notes.txt")).expect("reset workspace");
    let mut resumed = spawn(Resume::Latest);
    turn(&mut resumed, "third question").await;
    resumed.shutdown().await;

    let bodies: Vec<serde_json::Value> = server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .filter(|r| String::from_utf8_lossy(&r.body).contains("third question"))
        .map(|r| serde_json::from_slice(&r.body).expect("json body"))
        .collect();
    assert_eq!(bodies.len(), 2, "{bodies:#?}");
    assert_eq!(bodies[0], bodies[1]);
    let messages = bodies[0]["messages"].as_array().expect("messages");
    assert_eq!(messages.first().map(|m| &m["role"]), Some(&"system".into()));
    assert!(
        messages.iter().any(|m| m["role"] == "tool"),
        "{messages:#?}"
    );
    assert!(bodies[0]["tools"].as_array().is_some_and(|t| !t.is_empty()));
    assert!(
        bodies[0].to_string().contains("[~] answer questions"),
        "the plan reminder rides on the request: {messages:#?}"
    );
}

/// The bundle ships with both opt-in background checks off: a fresh install refreshes no
/// model catalog and looks for no updates until the user turns them on.
#[test]
fn the_bundle_ships_with_the_opt_in_checks_off() {
    let text = std::fs::read_to_string(defaults_bundle().join("config.toml")).expect("read");
    let settings: quack_config::Settings = toml::from_str(&text).expect("parse");
    assert!(
        !settings.catalog.refresh,
        "a fresh install would fetch {} in the background",
        settings.catalog.url
    );
    assert!(
        !settings.updates.check,
        "a fresh install would ask {} for updates in the background",
        settings.updates.url
    );
}

/// No remote (`http`/`https`) MCP server in the bundle ships enabled: the user opts in by
/// copying the file and turning it on, so a fresh install connects to no MCP endpoint.
#[test]
fn no_remote_mcp_server_ships_enabled() {
    let dir = defaults_bundle().join("mcp.d");
    let mut remote = 0;
    for entry in std::fs::read_dir(&dir).expect("mcp.d").flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|x| x != "toml") {
            continue;
        }
        let file: quack_config::McpServerFile =
            toml::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse");
        if file.url.starts_with("http") {
            remote += 1;
            assert!(
                !file.enabled,
                "{}: a remote MCP server ships enabled ({})",
                path.display(),
                file.url
            );
        }
    }
    assert!(
        remote > 0,
        "the bundle's remote MCP offers are gone; revisit this test"
    );
}
