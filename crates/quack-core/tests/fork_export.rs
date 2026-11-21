//! `/fork` goes on in a copy of the session and leaves the original as it was; `/export`
//! writes the transcript.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{sse_text, turn, workspace};
use quack_core::protocol::{Event, ExportFormat, Op, ReplayItem};
use quack_core::{Engine, EngineHandle, Resume};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer};

/// Events until `want` matches one; panics on an error.
async fn until<T>(engine: &mut EngineHandle, want: impl Fn(&Event) -> Option<T>) -> T {
    loop {
        let e = tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
            .await
            .expect("event within 10s")
            .expect("channel open");
        if let Event::Error { error } = &e {
            panic!("engine error: {}", error.message);
        }
        if let Some(t) = want(&e) {
            return t;
        }
    }
}

async fn resumed(root: &std::path::Path, url: &str, id: &str) -> Vec<ReplayItem> {
    let loaded = workspace(root, url, "[\"tiny\"]", "");
    let mut engine = Engine::spawn_with(Arc::new(loaded), Resume::Id(id.to_owned()));
    let items = until(&mut engine, |e| match e {
        Event::SessionResumed { items, .. } => Some(items.to_vec()),
        _ => None,
    })
    .await;
    engine.shutdown().await;
    items
}

#[tokio::test]
async fn fork_branches_and_export_writes_the_transcript() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse_text("Sure."))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = workspace(tmp.path(), &server.uri(), "[\"tiny\"]", "");
    let ws = loaded.dirs.workspace_root.clone();
    let mut engine = Engine::spawn(Arc::new(loaded));

    // Nothing to fork before the first message.
    engine.submit(Op::ForkSession).await;
    let warning = until(&mut engine, |e| match e {
        Event::Warning { message } if message.contains("fork") => Some(message.clone()),
        _ => None,
    })
    .await;
    assert!(warning.contains("nothing to fork yet"), "{warning}");

    turn(&mut engine, "first").await;
    engine.submit(Op::ForkSession).await;
    let forked = until(&mut engine, |e| match e {
        Event::SessionConfigured { session } => Some(session.id.0.clone()),
        _ => None,
    })
    .await;
    let notice = until(&mut engine, |e| match e {
        Event::Notice { message } => Some(message.clone()),
        _ => None,
    })
    .await;
    assert!(notice.contains("forked"), "{notice}");

    turn(&mut engine, "second").await;
    engine
        .submit(Op::Export {
            format: ExportFormat::Markdown,
            path: Some("out.md".into()),
        })
        .await;
    until(&mut engine, |e| match e {
        Event::Notice { message } if message.contains("exported") => Some(()),
        _ => None,
    })
    .await;
    engine.shutdown().await;

    let md = std::fs::read_to_string(ws.join("out.md")).expect("exported");
    assert!(md.starts_with("# first (fork)\n"), "{md}");
    assert!(
        md.contains("## You\n\nfirst\n")
            && md.contains("## You\n\nsecond\n")
            && md.contains("## quack\n\nSure.\n"),
        "{md}"
    );

    // The fork holds both turns; the original only the first.
    let (_, sessions) =
        quack_core::list_sessions(&workspace(tmp.path(), &server.uri(), "[\"tiny\"]", ""));
    assert_eq!(sessions.len(), 2, "{sessions:?}");
    let users = |items: &[ReplayItem]| {
        items
            .iter()
            .filter(|i| matches!(i, ReplayItem::User { .. }))
            .count()
    };
    let parent = sessions
        .iter()
        .find(|s| s.id != forked)
        .map(|s| s.id.clone())
        .expect("original listed");
    assert_eq!(users(&resumed(tmp.path(), &server.uri(), &forked).await), 2);
    assert_eq!(users(&resumed(tmp.path(), &server.uri(), &parent).await), 1);
}

#[tokio::test]
async fn small_model_title_is_saved_and_restored() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_string_contains(
            r#""model":"small""#,
        ))
        .respond_with(sse_text("Login form validation"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_string_contains(
            r#""model":"tiny""#,
        ))
        .respond_with(sse_text("Sure."))
        .expect(1)
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut loaded = workspace(tmp.path(), &server.uri(), r#"["tiny", "small"]"#, "");
    loaded.settings.model.small = "mock/small".into();
    let loaded = Arc::new(loaded);
    let mut engine = Engine::spawn(loaded.clone());
    engine
        .submit(Op::UserTurn {
            items: vec![quack_core::protocol::InputItem::text(
                "Please fix the login form.",
            )],
            overrides: Default::default(),
        })
        .await;
    let title = "Login form validation";
    let is_title = |e: &Event| match e {
        Event::SessionTitle { title: Some(t) } if t == title => Some(()),
        _ => None,
    };
    let events = common::until_done(&mut engine).await;
    if !events.iter().any(|e| is_title(e).is_some()) {
        until(&mut engine, is_title).await;
    }
    engine.submit(Op::ListSessions).await;
    let sessions = until(&mut engine, |e| match e {
        Event::SessionList { sessions } => Some(sessions.clone()),
        _ => None,
    })
    .await;
    assert_eq!(sessions[0].title, title);
    engine.shutdown().await;

    let mut engine = Engine::spawn_with(loaded, Resume::Latest);
    until(&mut engine, is_title).await;
    engine.shutdown().await;
}

/// A reasoning small model thinks first and wraps its answer; the title is still found,
/// and the request leaves it room to think.
#[tokio::test]
async fn small_model_title_survives_reasoning_and_wrapping() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_string_contains(
            r#""model":"small""#,
        ))
        .and(wiremock::matchers::body_string_contains(
            r#""max_tokens":1024"#,
        ))
        .respond_with(common::sse_raw(&[
            serde_json::json!({"choices":[{"delta":{"reasoning_content":"The user wants the login form fixed."}}]}).to_string(),
            serde_json::json!({"choices":[{"delta":{"content":"\n**Title:** \"Fix login form.\"\n"},"finish_reason":"stop"}]}).to_string(),
        ]))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_string_contains(
            r#""model":"tiny""#,
        ))
        .respond_with(sse_text("Sure."))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut loaded = workspace(tmp.path(), &server.uri(), r#"["tiny", "small"]"#, "");
    loaded.settings.model.small = "mock/small".into();
    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(
        &mut engine,
        "Please fix the login form, it accepts empty passwords and more",
    )
    .await;
    until(&mut engine, |e| match e {
        Event::SessionTitle { title: Some(t) } if t == "Fix login form" => Some(()),
        _ => None,
    })
    .await;
    engine.shutdown().await;
}

/// A small model that can't give a title says why instead of failing silently, and the
/// first line of the prompt stays the title.
#[tokio::test]
async fn small_model_without_a_title_warns() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse_text("Sure."))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut loaded = workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    loaded.settings.model.small = "nowhere/small".into();
    let mut engine = Engine::spawn(Arc::new(loaded));
    let seen = turn(&mut engine, "Please fix the login form").await;
    let is_warning = |e: &Event| match e {
        Event::Warning { message } if message.contains("session title") => Some(message.clone()),
        _ => None,
    };
    let warning = match seen.iter().find_map(is_warning) {
        Some(w) => w,
        None => until(&mut engine, is_warning).await,
    };
    assert!(warning.contains("nowhere/small"), "{warning}");
    engine.submit(Op::ListSessions).await;
    let sessions = until(&mut engine, |e| match e {
        Event::SessionList { sessions } => Some(sessions.clone()),
        _ => None,
    })
    .await;
    assert_eq!(sessions[0].title, "Please fix the login form");
    engine.shutdown().await;
}

#[tokio::test]
async fn fork_worktree_creates_isolated_branch_and_continues_in_worktree() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(wiremock::matchers::body_string_contains(
            "write in worktree",
        ))
        .and(wiremock::matchers::body_string_contains(
            "\"role\":\"tool\"",
        ))
        .respond_with(sse_text("wrote in worktree"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(wiremock::matchers::body_string_contains(
            "write in worktree",
        ))
        .respond_with(common::sse_call(
            "c_wt",
            "write",
            serde_json::json!({"path": "isolated.txt", "content": "only in worktree\n"}),
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse_text("first ok"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = workspace(tmp.path(), &server.uri(), "[\"tiny\"]", "");
    let ws = loaded.dirs.workspace_root.clone();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&ws)
            .args(args)
            .env("GIT_AUTHOR_NAME", "quack")
            .env("GIT_AUTHOR_EMAIL", "quack@example.com")
            .env("GIT_COMMITTER_NAME", "quack")
            .env("GIT_COMMITTER_EMAIL", "quack@example.com")
            .status()
            .expect("git")
    };
    if !git(&["init", "-q"]).success() {
        return;
    }
    std::fs::write(ws.join("base.txt"), "base\n").expect("write");
    assert!(git(&["add", "base.txt"]).success());
    assert!(git(&["commit", "-q", "-m", "init"]).success());

    let mut engine = Engine::spawn(Arc::new(loaded));
    engine
        .submit(Op::SetPermissionMode {
            mode: quack_core::protocol::PermissionMode::Allow,
        })
        .await;
    turn(&mut engine, "initial message").await;

    engine
        .submit(Op::ForkWorktree {
            name: Some("feat-x".into()),
        })
        .await;
    let (wt_path, wt_branch) = until(&mut engine, |e| match e {
        Event::WorktreeCreated { path, branch, .. } => Some((path.clone(), branch.clone())),
        _ => None,
    })
    .await;
    assert_eq!(wt_branch, "quack/feat-x");
    assert!(wt_path.ends_with("feat-x"), "{wt_path}");
    // Wait for reload inside the new worktree to finish.
    until(&mut engine, |e| match e {
        Event::SessionConfigured { session } if session.cwd == wt_path => Some(()),
        _ => None,
    })
    .await;

    turn(&mut engine, "write in worktree").await;
    assert!(
        std::path::Path::new(&wt_path).join("isolated.txt").exists(),
        "expected isolated.txt inside worktree {wt_path}"
    );
    assert!(
        !ws.join("isolated.txt").exists(),
        "main workspace should remain untouched"
    );
    engine.shutdown().await;
}
