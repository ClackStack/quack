//! `[tools] edit_format`: the model gets the chosen edit tool only, multi-file calls change
//! every file, and `/undo` puts all of them back.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::{bodies, sse_call, sse_text, turn};
use quack_core::protocol::{Event, LoadState, Op, Subsystem};
use quack_core::{Engine, EngineHandle};
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer};

async fn engine(root: &Path, server: &MockServer, format: &str) -> EngineHandle {
    std::fs::create_dir_all(root.join("user")).expect("mkdir");
    std::fs::write(
        root.join("user").join("config.toml"),
        format!("[tools]\nedit_format = \"{format}\"\n[permissions]\nmode = \"full-auto\"\n"),
    )
    .expect("config");
    let loaded = common::workspace(root, &server.uri(), r#"["tiny"]"#, "");
    Engine::spawn(Arc::new(loaded))
}

async fn profile_startup(profile: &str, format: &str, mode: &str) -> LoadState {
    let tmp = tempfile::tempdir().expect("tempdir");
    let server = MockServer::start().await;
    let user = tmp.path().join("user");
    std::fs::create_dir_all(&user).expect("mkdir");
    std::fs::write(
        user.join("config.toml"),
        format!(
            "[model]\nprofile = \"{profile}\"\n[tools]\nedit_format = \"{format}\"\n[permissions]\nmode = \"{mode}\"\n"
        ),
    )
    .expect("config");
    if profile == "codex" {
        let dir = user.join("profiles.d").join(profile);
        std::fs::create_dir_all(&dir).expect("profile dir");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../defaults/profiles.d/codex");
        for file in ["profile.toml", "system.md"] {
            std::fs::copy(source.join(file), dir.join(file)).expect("profile file");
        }
    } else if profile == "reader" {
        let dir = user.join("profiles.d").join(profile);
        std::fs::create_dir_all(&dir).expect("profile dir");
        std::fs::write(
            dir.join("profile.toml"),
            "source = \"test\"\nexclusive = true\n[[tools]]\nname = \"read\"\n",
        )
        .expect("profile");
        std::fs::write(dir.join("system.md"), "Read only.").expect("prompt");
    }
    let loaded = common::workspace(tmp.path(), &server.uri(), r#"["tiny"]"#, "");
    let mut engine = Engine::spawn(Arc::new(loaded));
    let mut profile_state = None;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
            .await
            .expect("startup event")
            .expect("open");
        match event {
            Event::StartupProgress {
                subsystem: Subsystem::Profile,
                state,
            } => profile_state = Some(state),
            Event::StartupComplete => break,
            _ => {}
        }
    }
    engine.shutdown().await;
    assert!(
        bodies(&server).await.is_empty(),
        "startup must not call a model"
    );
    profile_state.expect("profile startup state")
}

#[tokio::test]
async fn exclusive_editing_profile_rejects_incompatible_formats_before_permission_filtering() {
    for mode in ["read-only", "ask", "auto-edit", "full-auto", "yolo"] {
        for format in [
            "auto",
            "apply_patch",
            "str_replace",
            "hashline",
            "search_replace_blocks",
            "whole_file",
        ] {
            let state = profile_startup("codex", format, mode).await;
            if matches!(format, "auto" | "apply_patch") {
                assert!(
                    matches!(state, LoadState::Ready { .. }),
                    "{mode}/{format}: {state:?}"
                );
            } else {
                let LoadState::Failed { error } = state else {
                    panic!("{mode}/{format}: {state:?}");
                };
                assert!(error.contains("codex"), "{error}");
                assert!(
                    error.contains(&format!("tools.edit_format=\"{format}\"")),
                    "{error}"
                );
                assert!(error.contains("tools.edit_format=auto"), "{error}");
                assert!(error.contains("compatible profile"), "{error}");
            }
        }
    }
}

#[tokio::test]
async fn generic_and_intentionally_read_only_profiles_accept_all_edit_formats() {
    for profile in ["generic", "reader"] {
        for format in [
            "auto",
            "apply_patch",
            "str_replace",
            "hashline",
            "search_replace_blocks",
            "whole_file",
        ] {
            let state = profile_startup(profile, format, "full-auto").await;
            assert!(
                matches!(state, LoadState::Ready { .. }),
                "{profile}/{format}: {state:?}"
            );
        }
    }
}

/// Script a turn of tool calls: the first request gets `calls[0]`, each result the next
/// call, and the last result ends the turn.
async fn reply_with_calls(server: &MockServer, calls: &[(&str, Value)]) {
    let (first, rest) = calls.split_first().expect("at least one call");
    Mock::given(method("POST"))
        .respond_with(sse_call("c1", first.0, first.1.clone()))
        .up_to_n_times(1)
        .mount(server)
        .await;
    for (name, args) in rest {
        Mock::given(method("POST"))
            .and(body_string_contains("\"role\":\"tool\""))
            .respond_with(sse_call("c1", name, args.clone()))
            .up_to_n_times(1)
            .mount(server)
            .await;
    }
    Mock::given(method("POST"))
        .and(body_string_contains("\"role\":\"tool\""))
        .respond_with(sse_text("done"))
        .mount(server)
        .await;
}

fn tool_names(body: &str) -> Vec<String> {
    let v: Value = serde_json::from_str(body).expect("json");
    v["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["function"]["name"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn apply_patch_changes_several_files_and_undo_restores_them() {
    let server = MockServer::start().await;
    let patch = "*** Begin Patch\n*** Update File: a.txt\n one\n-two\n+TWO\n three\n\
                 *** Delete File: b.txt\n*** Add File: sub/c.txt\n+new\n*** End Patch";
    reply_with_calls(
        &server,
        &[
            ("read", json!({ "path": "a.txt" })),
            ("read", json!({ "path": "b.txt" })),
            ("apply_patch", json!({ "patch": patch })),
        ],
    )
    .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let ws = tmp.path().join("ws");
    let mut engine = engine(tmp.path(), &server, "apply_patch").await;
    std::fs::write(ws.join("a.txt"), "one\r\ntwo\r\nthree\r\n").expect("a");
    std::fs::write(ws.join("b.txt"), "bye\n").expect("b");

    turn(&mut engine, "change them").await;
    let first = bodies(&server).await.remove(0);
    let names = tool_names(&first);
    assert!(names.contains(&"apply_patch".to_owned()), "{names:?}");
    assert!(!names.contains(&"edit".to_owned()), "{names:?}");
    // Line endings are kept.
    assert_eq!(
        std::fs::read_to_string(ws.join("a.txt")).expect("a"),
        "one\r\nTWO\r\nthree\r\n"
    );
    assert!(!ws.join("b.txt").exists());
    assert_eq!(
        std::fs::read_to_string(ws.join("sub").join("c.txt")).expect("c"),
        "new\n"
    );
    let result = bodies(&server).await.pop().expect("second request");
    assert!(result.contains("Edited a.txt"), "{result}");

    engine
        .submit(Op::Undo {
            checkpoint: None,
            force: false,
        })
        .await;
    let mut restored = loop {
        let e = tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
            .await
            .expect("event")
            .expect("open");
        if let Event::FilesRestored {
            restored,
            conflicts,
        } = e
        {
            assert!(conflicts.is_empty(), "{conflicts:?}");
            break restored;
        }
    };
    restored.sort();
    assert_eq!(restored, ["a.txt", "b.txt", "sub/c.txt"]);
    assert_eq!(
        std::fs::read_to_string(ws.join("a.txt")).expect("a"),
        "one\r\ntwo\r\nthree\r\n"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("b.txt")).expect("b"),
        "bye\n"
    );
    assert!(!ws.join("sub").join("c.txt").exists());
    engine.shutdown().await;
}

#[tokio::test]
async fn edit_blocks_edit_two_files_in_one_call() {
    let server = MockServer::start().await;
    let blocks = "a.txt\n<<<<<<< SEARCH\ntwo\n=======\n2\n>>>>>>> REPLACE\n\n\
                  b.txt\n<<<<<<< SEARCH\nbye\n=======\nhello\n>>>>>>> REPLACE\n";
    reply_with_calls(
        &server,
        &[
            ("read", json!({ "path": "a.txt" })),
            ("read", json!({ "path": "b.txt" })),
            ("edit_blocks", json!({ "blocks": blocks })),
        ],
    )
    .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let ws = tmp.path().join("ws");
    let mut engine = engine(tmp.path(), &server, "search_replace_blocks").await;
    std::fs::write(ws.join("a.txt"), "one\ntwo\n").expect("a");
    std::fs::write(ws.join("b.txt"), "bye\n").expect("b");

    turn(&mut engine, "change them").await;
    let names = tool_names(&bodies(&server).await.remove(0));
    assert!(names.contains(&"edit_blocks".to_owned()), "{names:?}");
    assert!(!names.contains(&"edit".to_owned()), "{names:?}");
    assert_eq!(
        std::fs::read_to_string(ws.join("a.txt")).expect("a"),
        "one\n2\n"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("b.txt")).expect("b"),
        "hello\n"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn whole_file_offers_only_write() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse_text("ok"))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut engine = engine(tmp.path(), &server, "whole_file").await;
    turn(&mut engine, "hi").await;
    let body = bodies(&server).await.remove(0);
    let names = tool_names(&body);
    assert!(names.contains(&"write".to_owned()), "{names:?}");
    for edit in ["edit", "edit_hashline", "apply_patch", "edit_blocks"] {
        assert!(!names.contains(&edit.to_owned()), "{names:?}");
    }
    assert!(body.contains("complete new content"), "{body}");
    engine.shutdown().await;
}

/// An `edit` on a file the model never read is refused; after a `read` the same edit
/// goes through.
#[tokio::test]
async fn blind_edit_is_refused_until_the_file_is_read() {
    let server = MockServer::start().await;
    let edit = json!({"path": "a.txt", "old_string": "two", "new_string": "2"});
    reply_with_calls(
        &server,
        &[
            ("edit", edit.clone()),
            ("read", json!({ "path": "a.txt" })),
            ("edit", edit),
        ],
    )
    .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let ws = tmp.path().join("ws");
    let mut engine = engine(tmp.path(), &server, "str_replace").await;
    std::fs::write(ws.join("a.txt"), "one\ntwo\n").expect("a");

    turn(&mut engine, "change it").await;
    let all = bodies(&server).await;
    assert!(all[1].contains("has not been read yet"), "{}", all[1]);
    assert_eq!(
        std::fs::read_to_string(ws.join("a.txt")).expect("a"),
        "one\n2\n"
    );
    engine.shutdown().await;
}

/// `write` over an existing file the model never read is refused; creating a new file
/// with `write` needs no read.
#[tokio::test]
async fn overwriting_an_unread_file_is_refused() {
    let server = MockServer::start().await;
    let write = json!({"path": "a.txt", "content": "fresh\n"});
    reply_with_calls(
        &server,
        &[
            ("write", write.clone()),
            ("read", json!({ "path": "a.txt" })),
            ("write", write),
        ],
    )
    .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let ws = tmp.path().join("ws");
    let mut engine = engine(tmp.path(), &server, "str_replace").await;
    std::fs::write(ws.join("a.txt"), "old\n").expect("a");

    turn(&mut engine, "replace it").await;
    let all = bodies(&server).await;
    assert!(all[1].contains("has not been read yet"), "{}", all[1]);
    assert_eq!(
        std::fs::read_to_string(ws.join("a.txt")).expect("a"),
        "fresh\n"
    );
    engine.shutdown().await;
}
