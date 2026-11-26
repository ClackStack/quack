//! Pruning against a mock server: an old whole-file read keeps only its outline.

mod common;

use std::sync::Arc;

use common::{bodies, sse_call, sse_text, turn};
use quack_core::Engine;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer};

#[tokio::test]
async fn an_old_read_is_replaced_by_its_outline() {
    if !quack_tools::outline::supported(std::path::Path::new("a.rs")) {
        return; // built without grammars
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("read the file"))
        .and(body_string_contains("\"role\":\"tool\""))
        .respond_with(sse_text("Read it."))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("read the file"))
        .respond_with(sse_call(
            "c1",
            "read",
            serde_json::json!({"path": "big.rs"}),
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse_text("ok"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("user")).expect("mkdir");
    // Prune before every request; reads return whole files.
    std::fs::write(
        tmp.path().join("user").join("config.toml"),
        "[context]\ncache_strategy = \"aggressive\"\n[tools]\nread_outline_tokens = 0\n",
    )
    .expect("config");
    let loaded = common::workspace(
        tmp.path(),
        &server.uri(),
        r#"[{ id = "tiny", context_window = 200000 }]"#,
        "",
    );
    let mut src = String::new();
    for i in 0..60 {
        src.push_str(&format!("pub fn f{i}() -> u32 {{\n"));
        for k in 0..5 {
            src.push_str(&format!("    let v{k} = {i} * {k};\n"));
        }
        src.push_str("    0\n}\n\n");
    }
    std::fs::write(loaded.dirs.workspace_root.join("big.rs"), &src).expect("write");
    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(&mut engine, "read the file").await;
    let first = bodies(&server).await.pop().expect("request");
    assert!(first.contains("let v4 = 50 * 4"), "the whole file was read");
    for n in 0..5 {
        turn(&mut engine, &format!("question {n}")).await;
    }
    let last = bodies(&server).await.pop().expect("request");
    assert!(last.contains("only its outline is kept"), "{last}");
    assert!(last.contains("pub fn f50() -> u32 {"), "{last}");
    assert!(!last.contains("let v4 = 50 * 4"), "{last}");
    engine.shutdown().await;
}

#[tokio::test]
async fn auto_cache_strategy_prunes_eagerly_when_prefix_cache_is_false() {
    if !quack_tools::outline::supported(std::path::Path::new("a.rs")) {
        return; // built without grammars
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("read the file"))
        .and(body_string_contains("\"role\":\"tool\""))
        .respond_with(sse_text("Read it."))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("read the file"))
        .respond_with(sse_call(
            "c1",
            "read",
            serde_json::json!({"path": "big.rs"}),
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse_text("ok"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("user")).expect("mkdir");
    // Default cache_strategy = "auto", no manual "aggressive" override.
    std::fs::write(
        tmp.path().join("user").join("config.toml"),
        "[tools]\nread_outline_tokens = 0\n",
    )
    .expect("config");
    let loaded = common::workspace(
        tmp.path(),
        &server.uri(),
        r#"[{ id = "tiny", context_window = 200000, serving = { prefix_cache = false } }]"#,
        "",
    );
    let mut src = String::new();
    for i in 0..60 {
        src.push_str(&format!("pub fn f{i}() -> u32 {{\n"));
        for k in 0..5 {
            src.push_str(&format!("    let v{k} = {i} * {k};\n"));
        }
        src.push_str("    0\n}\n\n");
    }
    std::fs::write(loaded.dirs.workspace_root.join("big.rs"), &src).expect("write");
    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(&mut engine, "read the file").await;
    for n in 0..5 {
        turn(&mut engine, &format!("question {n}")).await;
    }
    let last = bodies(&server).await.pop().expect("request");
    assert!(
        last.contains("only its outline is kept"),
        "auto cache_strategy should prune eagerly when serving.prefix_cache = false: {last}"
    );
    assert!(!last.contains("let v4 = 50 * 4"), "{last}");
    engine.shutdown().await;
}

#[tokio::test]
async fn auto_cache_strategy_preserves_prefix_when_slow_local_server_has_prefix_cache() {
    if !quack_tools::outline::supported(std::path::Path::new("a.rs")) {
        return; // built without grammars
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("read the file"))
        .and(body_string_contains("\"role\":\"tool\""))
        .respond_with(sse_text("Read it."))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("read the file"))
        .respond_with(sse_call(
            "c1",
            "read",
            serde_json::json!({"path": "big.rs"}),
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse_text("ok"))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(tmp.path().join("user")).expect("mkdir");
    std::fs::write(
        tmp.path().join("user").join("config.toml"),
        "[tools]\nread_outline_tokens = 0\n",
    )
    .expect("config");
    // Slow prefill with prefix_cache = true (typical local GPU): auto must NOT eager-prune
    // and force slow re-prefills on every step.
    let loaded = common::workspace(
        tmp.path(),
        &server.uri(),
        r#"[{ id = "tiny", context_window = 200000, serving = { prefix_cache = true, prefill_tps = 120 } }]"#,
        "",
    );
    let mut src = String::new();
    for i in 0..60 {
        src.push_str(&format!("pub fn f{i}() -> u32 {{\n"));
        for k in 0..5 {
            src.push_str(&format!("    let v{k} = {i} * {k};\n"));
        }
        src.push_str("    0\n}\n\n");
    }
    std::fs::write(loaded.dirs.workspace_root.join("big.rs"), &src).expect("write");
    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(&mut engine, "read the file").await;
    for n in 0..5 {
        turn(&mut engine, &format!("question {n}")).await;
    }
    let last = bodies(&server).await.pop().expect("request");
    assert!(
        !last.contains("only its outline is kept"),
        "slow local server with prefix_cache = true must keep its prefix intact: {last}"
    );
    assert!(last.contains("let v4 = 50 * 4"), "{last}");
    engine.shutdown().await;
}

#[tokio::test]
async fn threshold_pruning_skips_tiny_reclaim_and_compacts_instead() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains("You summarize a coding session"))
        .respond_with(sse_text("SUMMARY-TEXT"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("read small"))
        .and(body_string_contains("\"role\":\"tool\""))
        .respond_with(sse_text("Read small."))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("read small"))
        .respond_with(sse_call(
            "c1",
            "read",
            serde_json::json!({"path": "small.txt"}),
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse_text("ok"))
        .mount(&server)
        .await;

    // Budget = 17,000, threshold = 13,600 (80%). Reading a ~4,500-byte file (~1,285 tokens)
    // is prunable by rule 3 (LargeOutput > 3,500 chars), but reclaiming ~1k tokens would
    // leave usage above 85% of threshold (~11,560) once the large turns push past 13,600.
    // The engine must skip the half-measure prune and compact directly instead of busting
    // the prefix cache for a tiny reclaim.
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = common::workspace(
        tmp.path(),
        &server.uri(),
        r#"[{ id = "tiny", context_window = 20000 }]"#,
        "",
    );
    let small_txt = (0..120)
        .map(|i| format!("line {i:03}: a small prunable output row\n"))
        .collect::<String>();
    std::fs::write(loaded.dirs.workspace_root.join("small.txt"), &small_txt).expect("write");
    let mut engine = Engine::spawn(Arc::new(loaded));
    turn(&mut engine, "read small").await;
    // Push the read past PROTECT_MESSAGES (8) and cross threshold with large user turns.
    let big = "x".repeat(16_000);
    for n in 0..3 {
        turn(&mut engine, &format!("filler {n}")).await;
    }
    let events = turn(&mut engine, &format!("FIRST {big}")).await;
    let events = if events
        .iter()
        .any(|e| matches!(e, quack_core::protocol::Event::Compacted { .. }))
    {
        events
    } else {
        turn(&mut engine, &format!("SECOND {big}")).await
    };
    let events = if events
        .iter()
        .any(|e| matches!(e, quack_core::protocol::Event::Compacted { .. }))
    {
        events
    } else {
        turn(&mut engine, &format!("THIRD {big}")).await
    };
    assert!(
        events
            .iter()
            .any(|e| matches!(e, quack_core::protocol::Event::Compacted { .. })),
        "should compact directly rather than thrash on a tiny prune: {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, quack_core::protocol::Event::ContextPruned { .. })),
        "should not emit a wasteful prune right before compacting: {events:?}"
    );
    engine.shutdown().await;
}
