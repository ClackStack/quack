//! The built-in tools against a temporary workspace.

use std::sync::Arc;
use std::time::Duration;

use quack_shell::ShellKind;
use quack_tools::{ReadTracker, Registry, ToolCtx};
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn ctx(root: &std::path::Path) -> ToolCtx {
    ToolCtx {
        root: root.to_path_buf(),
        shell: ShellKind::detect("auto", None),
        default_timeout: Duration::from_secs(10),
        max_timeout: Duration::from_secs(30),
        cancel: CancellationToken::new(),
        on_output: None,
        ask: None,
        reads: Arc::new(ReadTracker::default()),
        memory: None,
        skills: Default::default(),
        lsp: None,
        sandbox: Default::default(),
        plan: Default::default(),
        agents: Default::default(),
        spawn: None,
        edit_format: quack_config::settings::EditFormat::StrReplace,
        vision: false,
        outline_above: 0,
        grammars: quack_tools::grammar::Grammars::builtin(),
        bg: Arc::default(),
        web_fetch: None,
        client_bridge: None,
    }
}

async fn call(
    reg: &Registry,
    ctx: &ToolCtx,
    name: &str,
    args: serde_json::Value,
) -> quack_tools::ToolOutput {
    reg.get(name).expect("tool exists").run(args, ctx).await
}

#[tokio::test]
async fn write_read_edit_search() {
    let dir = tempfile::tempdir().expect("tempdir");
    let reg = Registry::builtin();
    let c = ctx(dir.path());

    let w = call(
        &reg,
        &c,
        "write",
        json!({"path": "src/lib.rs", "content": "fn a() {}\nfn b() {}\n"}),
    )
    .await;
    assert!(w.ok, "{w:?}");
    assert_eq!(w.summary, "+2 -0");

    let r = call(
        &reg,
        &c,
        "read",
        json!({"file_path": "src/lib.rs", "offset": "2"}),
    )
    .await;
    assert!(r.model.contains("   2\tfn b() {}"), "{}", r.model);

    let e = call(
        &reg,
        &c,
        "edit",
        json!({"path": "src/lib.rs", "old_string": "fn b() {}", "new_string": "fn c() {}"}),
    )
    .await;
    assert!(e.ok, "{e:?}");
    assert!(
        std::fs::read_to_string(dir.path().join("src").join("lib.rs"))
            .expect("read")
            .contains("fn c()")
    );

    // Changed behind the model's back: the next edit is refused until it reads again.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(
        dir.path().join("src").join("lib.rs"),
        "fn a() {}\nfn c() {}\n// external\n",
    )
    .expect("write");
    let stale = call(
        &reg,
        &c,
        "edit",
        json!({"path": "src/lib.rs", "old_string": "fn a", "new_string": "fn z"}),
    )
    .await;
    assert!(
        !stale.ok && stale.model.contains("changed on disk"),
        "{stale:?}"
    );

    let g = call(&reg, &c, "glob", json!({"pattern": "*.rs"})).await;
    assert_eq!(g.model, "src/lib.rs");
    let gr = call(&reg, &c, "grep", json!({"pattern": "fn [ac]"})).await;
    assert!(gr.model.contains("src/lib.rs:1: fn a() {}"), "{}", gr.model);
    let l = call(&reg, &c, "list_dir", json!({})).await;
    assert!(l.model.contains("  src/\n    lib.rs"), "{}", l.model);
}

#[tokio::test]
async fn edit_batch_rejections_leave_the_file_unchanged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let reg = Registry::builtin();
    let c = ctx(dir.path());
    let path = dir.path().join("a.txt");
    let before = "first\nsecond\n";
    std::fs::write(&path, before).expect("write fixture");

    let valid = json!({"path": "a.txt", "edits": [
        {"old_string": "first", "new_string": "FIRST"},
        {"old_string": "second", "new_string": "SECOND"}
    ]});
    let unread = call(&reg, &c, "edit", valid.clone()).await;
    assert!(
        !unread.ok && unread.model.contains("has not been read"),
        "{unread:?}"
    );
    assert!(call(&reg, &c, "read", json!({"path": "a.txt"})).await.ok);

    let mut mixed = valid.clone();
    mixed["old_string"] = json!("first");
    let mut unmatched = valid.clone();
    unmatched["edits"][1]["old_string"] = json!("missing");
    let mut missing_new = valid.clone();
    missing_new["edits"][1] = json!({"old_string": "second"});
    for (args, reason) in [
        (mixed, "not both"),
        (json!({"path": "a.txt", "edits": []}), "non-empty array"),
        (missing_new, "missing required string argument `new_string`"),
        (unmatched, "Edit 2: `old_string` was not found"),
    ] {
        let out = call(&reg, &c, "edit", args).await;
        assert!(!out.ok && out.model.contains(reason), "{out:?}");
        assert_eq!(
            std::fs::read_to_string(&path).expect("unchanged file"),
            before
        );
    }

    let external = "first\nsecond\nexternal change\n";
    std::fs::write(&path, external).expect("external write");
    let stale = call(&reg, &c, "edit", valid).await;
    assert!(
        !stale.ok && stale.model.contains("changed on disk"),
        "{stale:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("external file"),
        external
    );
}

#[cfg(feature = "lang-common")]
#[tokio::test]
async fn edit_syntax_warning() {
    let dir = tempfile::tempdir().expect("tempdir");
    let reg = Registry::builtin();
    let c = ctx(dir.path());

    let w = call(
        &reg,
        &c,
        "write",
        json!({"path": "src/lib.rs", "content": "fn clean() {\n    let x = 1;\n}\n"}),
    )
    .await;
    assert!(w.ok, "{w:?}");
    assert!(
        !w.model
            .contains("Warning: Tree-sitter detected a syntax error")
    );

    let e = call(
        &reg,
        &c,
        "edit",
        json!({"path": "src/lib.rs", "old_string": "    let x = 1;", "new_string": "    let x = ;"}),
    )
    .await;
    assert!(e.ok, "{e:?}");
    assert!(
        e.model
            .contains("Warning: Tree-sitter detected a syntax error at line 2"),
        "{}",
        e.model
    );
}

#[tokio::test]
async fn shell_reports_exit_code() {
    let dir = tempfile::tempdir().expect("tempdir");
    let reg = Registry::builtin();
    let c = ctx(dir.path());
    let out = call(&reg, &c, "shell", json!({"command": "echo hello"})).await;
    assert!(out.ok, "{out:?}");
    assert!(out.model.starts_with("exit code 0\nhello"), "{}", out.model);
}

#[cfg(feature = "lang-common")]
#[tokio::test]
async fn index_and_outlined_reads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let reg = Registry::builtin();
    let mut c = ctx(dir.path());
    let mut src = String::from("pub struct Big;\n\nimpl Big {\n");
    for i in 0..200 {
        src.push_str(&format!("    pub fn f{i}(&self) -> u32 {{\n"));
        for k in 0..6 {
            src.push_str(&format!("        let v{k} = {i} * {k} + self.offset();\n"));
        }
        src.push_str(&format!("        {i}\n    }}\n\n"));
    }
    src.push_str("}\n");
    std::fs::create_dir_all(dir.path().join("src")).expect("mkdir");
    std::fs::write(dir.path().join("src").join("big.rs"), &src).expect("write");
    std::fs::write(
        dir.path().join("src").join("small.py"),
        "def g():\n    return 1\n",
    )
    .expect("write");

    let i = call(&reg, &c, "index", json!({"path": "src/big.rs"})).await;
    assert!(i.ok, "{i:?}");
    assert!(
        i.model.starts_with("src/big.rs (2004 lines, rust)"),
        "{}",
        i.model
    );
    assert!(i.model.contains("pub fn f7(&self) -> u32 {"), "{}", i.model);
    let d = call(&reg, &c, "index", json!({})).await;
    assert!(
        d.model.contains("src/small.py (2 lines): fn g"),
        "{}",
        d.model
    );
    assert!(
        d.model
            .contains("src/big.rs (2004 lines): type Big, impl Big"),
        "{}",
        d.model
    );

    // Off: plain lines, capped at the default 500-line chunk so non-outlined files don't
    // dump thousands of lines at once. On: the outline, unless a range or symbol is asked for.
    let whole = call(&reg, &c, "read", json!({"path": "src/big.rs"})).await;
    assert!(whole.model.contains("pub fn f48"), "{}", whole.summary);
    assert!(
        whole.model.contains("lines 1-500 of 2004"),
        "{}",
        whole.model
    );
    let tail = call(
        &reg,
        &c,
        "read",
        json!({"path": "src/big.rs", "offset": 1990, "limit": 20}),
    )
    .await;
    assert!(tail.model.contains("pub fn f199"), "{}", tail.summary);
    c.outline_above = 2000;
    let o = call(&reg, &c, "read", json!({"path": "src/big.rs"})).await;
    assert!(o.model.contains("so this is its outline"), "{}", o.model);
    assert!(o.model.len() < src.len() / 2);
    let part = call(
        &reg,
        &c,
        "read",
        json!({"path": "src/big.rs", "offset": 4, "limit": 3}),
    )
    .await;
    assert!(part.model.contains("pub fn f0"), "{}", part.model);
    let sym = call(
        &reg,
        &c,
        "read",
        json!({"path": "src/big.rs", "symbol": "Big::f150"}),
    )
    .await;
    assert!(
        sym.model.contains("pub fn f150") && sym.model.contains("let v5 = 150 * 5"),
        "{}",
        sym.model
    );
    assert!(!sym.model.contains("f151"), "{}", sym.model);
    let small = call(&reg, &c, "read", json!({"path": "src/small.py"})).await;
    assert!(small.model.contains("return 1"), "{}", small.model);
    let missing = call(
        &reg,
        &c,
        "read",
        json!({"path": "src/big.rs", "symbol": "nope"}),
    )
    .await;
    assert!(
        !missing.ok && missing.model.contains("index"),
        "{}",
        missing.model
    );
}

/// A 2x1 PNG header, enough for the probe.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, b'I', b'H', b'D', b'R', 0, 0, 0,
    2, 0, 0, 0, 1, 8, 2, 0, 0, 0, 0x7D, 0x0C, 0xC0, 0xFA,
];

#[tokio::test]
async fn view_image_needs_vision_and_sends_the_pixels() {
    let dir = tempfile::tempdir().expect("tempdir");
    let reg = Registry::builtin();
    let mut c = ctx(dir.path());
    std::fs::write(dir.path().join("pic.png"), PNG).expect("png");

    // Without vision the tool is not offered at all.
    assert!(!reg.get("view_image").expect("tool").available(&c));

    c.vision = true;
    let v = call(&reg, &c, "view_image", json!({"path": "pic.png"})).await;
    assert!(v.ok, "{v:?}");
    assert_eq!(v.model, "[image: pic.png 1KB 2x1]");
    assert_eq!(v.images.len(), 1);
    assert_eq!(v.images[0].media_type, "image/png");
    assert_eq!(v.images[0].width, Some(2));

    // A non-image is refused, with the path in the message.
    std::fs::write(dir.path().join("not.txt"), b"plain").expect("txt");
    let bad = call(&reg, &c, "view_image", json!({"path": "not.txt"})).await;
    assert!(!bad.ok && bad.model.contains("not.txt"), "{bad:?}");
    assert!(bad.images.is_empty());
}

#[tokio::test]
async fn grep_supports_files_count_context_and_limit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let reg = Registry::builtin();
    let c = ctx(dir.path());
    std::fs::write(
        dir.path().join("a.txt"),
        "alpha\nMATCH one\nbeta\ngamma\ndelta\nMATCH two\nepsilon\n",
    )
    .expect("a");
    std::fs::write(dir.path().join("b.txt"), "MATCH three\n").expect("b");

    let files = call(
        &reg,
        &c,
        "grep",
        json!({"pattern": "MATCH", "output_mode": "files"}),
    )
    .await;
    assert!(files.ok, "{files:?}");
    assert_eq!(files.model, "a.txt\nb.txt\n");

    let counts = call(
        &reg,
        &c,
        "grep",
        json!({"pattern": "MATCH", "output_mode": "count"}),
    )
    .await;
    assert!(counts.ok, "{counts:?}");
    assert_eq!(counts.model, "a.txt: 2\nb.txt: 1\n");

    let with_ctx = call(
        &reg,
        &c,
        "grep",
        json!({"pattern": "MATCH", "path": "a.txt", "context": 1}),
    )
    .await;
    assert!(with_ctx.ok, "{with_ctx:?}");
    assert!(
        with_ctx.model.contains("a.txt-1- alpha\n")
            && with_ctx.model.contains("a.txt:2: MATCH one\n")
            && with_ctx.model.contains("a.txt-3- beta\n")
            && with_ctx.model.contains("--\n")
            && with_ctx.model.contains("a.txt:6: MATCH two\n"),
        "{}",
        with_ctx.model
    );

    let limited = call(&reg, &c, "grep", json!({"pattern": "MATCH", "limit": 1})).await;
    assert!(limited.ok, "{limited:?}");
    assert!(
        limited.model.contains("2 more matches"),
        "{}",
        limited.model
    );
}

#[tokio::test]
async fn grep_can_match_literal_metacharacters_case_insensitively() {
    let dir = tempfile::tempdir().expect("tempdir");
    let reg = Registry::builtin();
    let c = ctx(dir.path());
    std::fs::write(dir.path().join("a.txt"), "foo.bar()\nfooXbar\nFOO.BAR()\n").expect("fixture");

    let literal = call(
        &reg,
        &c,
        "grep",
        json!({"pattern": "foo.bar()", "literal": true, "case_insensitive": true}),
    )
    .await;
    assert_eq!(literal.model, "a.txt:1: foo.bar()\na.txt:3: FOO.BAR()\n");

    let regex = call(&reg, &c, "grep", json!({"pattern": "foo.bar()"})).await;
    assert_eq!(regex.model, "a.txt:1: foo.bar()\na.txt:2: fooXbar\n");
}

#[tokio::test]
async fn grep_keeps_matches_visible_when_clipping_long_unicode_lines() {
    let dir = tempfile::tempdir().expect("tempdir");
    let reg = Registry::builtin();
    let c = ctx(dir.path());
    let left = "é".repeat(330);
    let right = "界".repeat(330);
    std::fs::write(
        dir.path().join("a.txt"),
        format!("needle{right}\n{left}needle{right}\n{left}needle\n"),
    )
    .expect("fixture");

    let out = call(&reg, &c, "grep", json!({"pattern": "needle"})).await;
    let lines: Vec<_> = out.model.lines().collect();
    assert_eq!(lines.len(), 3, "{out:?}");
    for (i, (left_clipped, right_clipped)) in [(false, true), (true, true), (true, false)]
        .into_iter()
        .enumerate()
    {
        let snippet = lines[i]
            .strip_prefix(&format!("a.txt:{}: ", i + 1))
            .expect("file and line prefix");
        assert!(snippet.contains("needle"), "{snippet}");
        assert_eq!(snippet.starts_with('…'), left_clipped, "{snippet}");
        assert_eq!(snippet.ends_with('…'), right_clipped, "{snippet}");
        assert!(
            snippet.trim_matches('…').chars().count() <= 300,
            "{snippet}"
        );
    }
}

#[tokio::test]
async fn search_reports_bad_paths_instead_of_no_matches() {
    let dir = tempfile::tempdir().expect("tempdir");
    let reg = Registry::builtin();
    let c = ctx(dir.path());
    for tool in ["grep", "glob"] {
        let out = call(
            &reg,
            &c,
            tool,
            json!({"pattern": "needle", "path": "missing"}),
        )
        .await;
        assert!(
            !out.ok && out.model.contains("Cannot search `missing`"),
            "{out:?}"
        );
    }

    std::fs::write(dir.path().join("a.txt"), "needle\n").expect("fixture");
    let out = call(
        &reg,
        &c,
        "glob",
        json!({"pattern": "*.txt", "path": "a.txt"}),
    )
    .await;
    assert!(!out.ok && out.model.contains("not a directory"), "{out:?}");
}

#[tokio::test]
async fn grep_reports_size_skips_with_or_without_hits() {
    let dir = tempfile::tempdir().expect("tempdir");
    let reg = Registry::builtin();
    let c = ctx(dir.path());
    std::fs::write(dir.path().join("a.txt"), "needle\n").expect("fixture");
    std::fs::File::create(dir.path().join("oversized.log"))
        .expect("large file")
        .set_len(5 * 1024 * 1024 + 1)
        .expect("set file length");

    for (pattern, expected) in [
        ("needle", "a.txt:1: needle"),
        ("missing", "No matches in searched files."),
    ] {
        let out = call(&reg, &c, "grep", json!({"pattern": pattern})).await;
        assert!(out.ok && out.model.starts_with(expected), "{out:?}");
        assert!(
            out.model
                .contains("Search incomplete (files over 5 MiB: 1)"),
            "{out:?}"
        );
    }
    let out = call(
        &reg,
        &c,
        "grep",
        json!({"pattern": "missing", "glob": "*.txt"}),
    )
    .await;
    assert!(out.ok && out.model == "No matches.", "{out:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn search_reports_unreadable_files_and_directories() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let reg = Registry::builtin();
    let c = ctx(dir.path());
    let blocked_dir = dir.path().join("blocked");
    let blocked_file = dir.path().join("blocked.txt");
    std::fs::create_dir(&blocked_dir).expect("create directory");
    std::fs::write(&blocked_file, "needle").expect("fixture");
    let permissions = |path, mode| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("permissions");
    };
    permissions(&blocked_dir, 0o000);
    permissions(&blocked_file, 0o000);
    let no_dir_access = std::fs::read_dir(&blocked_dir).is_err();
    let no_file_access = std::fs::read(&blocked_file).is_err();
    let grep = call(&reg, &c, "grep", json!({"pattern": "needle"})).await;
    let glob = call(&reg, &c, "glob", json!({"pattern": "*.rs"})).await;
    permissions(&blocked_dir, 0o700);
    permissions(&blocked_file, 0o600);

    // A privileged test runner may be able to read mode-000 files and directories.
    if no_dir_access {
        assert!(grep.model.contains("traversal errors: 1"), "{grep:?}");
        assert!(glob.model.contains("traversal errors: 1"), "{glob:?}");
    }
    if no_file_access {
        assert!(grep.model.contains("unreadable files: 1"), "{grep:?}");
    }
}

#[cfg(feature = "lang-common")]
#[tokio::test]
async fn edit_symbol_scoping_and_hashline_outline_insert_guards() {
    let dir = tempfile::tempdir().expect("tempdir");
    let reg = Registry::builtin();
    let mut c = ctx(dir.path());
    let src = "fn first() -> i32 {\n    let x = 1;\n    x\n}\n\nfn second() -> i32 {\n    let x = 1;\n    x\n}\n";
    std::fs::write(dir.path().join("dup.rs"), src).expect("dup.rs");

    assert!(call(&reg, &c, "read", json!({"path": "dup.rs"})).await.ok);
    // Duplicate snippet fails without symbol, succeeds when scoped to `second`.
    let ambig = call(
        &reg,
        &c,
        "edit",
        json!({"path": "dup.rs", "old_string": "    let x = 1;", "new_string": "    let x = 2;"}),
    )
    .await;
    assert!(!ambig.ok && ambig.model.contains("2 places"), "{ambig:?}");
    let scoped = call(
        &reg,
        &c,
        "edit",
        json!({"path": "dup.rs", "symbol": "second", "old_string": "    let x = 1;", "new_string": "    let x = 2;"}),
    )
    .await;
    assert!(scoped.ok, "{scoped:?}");
    let after = std::fs::read_to_string(dir.path().join("dup.rs")).expect("read");
    assert!(
        after.contains("fn first() -> i32 {\n    let x = 1;")
            && after.contains("fn second() -> i32 {\n    let x = 2;"),
        "{after}"
    );

    // Hashline mode: outline-only read must NOT mark unseen body lines as shown, and
    // blind inserts after unseen lines must also be refused before editing.
    c.edit_format = quack_config::settings::EditFormat::Hashline;
    c.outline_above = 10;
    let outlined = call(&reg, &c, "read", json!({"path": "dup.rs"})).await;
    assert!(
        outlined.ok && outlined.model.contains("so this is its outline"),
        "{outlined:?}"
    );
    let blind_replace = call(
        &reg,
        &c,
        "edit_hashline",
        json!({"path": "dup.rs", "edits": [{"start": 2, "end": 2, "content": "    let x = 9;"}]}),
    )
    .await;
    assert!(
        !blind_replace.ok && blind_replace.model.contains("did not show"),
        "{blind_replace:?}"
    );
    let blind_insert = call(
        &reg,
        &c,
        "edit_hashline",
        json!({"path": "dup.rs", "edits": [{"action": "insert", "after": 7, "content": "    let y = 3;"}]}),
    )
    .await;
    assert!(
        !blind_insert.ok && blind_insert.model.contains("did not show"),
        "{blind_insert:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn shell_background_start_output_list_and_stop() {
    let dir = tempfile::tempdir().expect("tempdir");
    let c = ctx(dir.path());
    let reg = Registry::builtin();

    let started = call(
        &reg,
        &c,
        "shell",
        json!({"command": "echo server_up; sleep 30", "background": true}),
    )
    .await;
    assert!(started.ok, "{started:?}");
    assert!(started.model.contains("job bg-1"), "{}", started.model);
    assert!(started.model.contains("server_up"), "{}", started.model);

    let listed = call(&reg, &c, "shell", json!({"job_action": "list"})).await;
    assert!(listed.ok && listed.model.contains("bg-1"), "{listed:?}");

    let output = call(
        &reg,
        &c,
        "shell",
        json!({"job_action": "output", "job_id": "bg-1"}),
    )
    .await;
    assert!(output.ok && output.model.contains("running"), "{output:?}");

    let stopped = call(
        &reg,
        &c,
        "shell",
        json!({"job_action": "stop", "job_id": "bg-1"}),
    )
    .await;
    assert!(
        stopped.ok && stopped.model.contains("stopped"),
        "{stopped:?}"
    );
}
