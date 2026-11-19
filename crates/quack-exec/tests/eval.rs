//! `quack eval` against a mock model: a passing task, a failing one and a skipped one.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;

use quack_config::LoadOptions;
use quack_exec::eval::{EvalOptions, Report, RunErrorKind, load_tasks, run_task, table};
use serde_json::json;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sse(chunk: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(format!("data: {chunk}\n\ndata: [DONE]\n\n"))
}

fn task(suite: &Path, id: &str, toml: &str, files: &[(&str, &str)]) {
    let dir = suite.join(id);
    std::fs::create_dir_all(dir.join("repo")).expect("mkdir");
    std::fs::write(dir.join("task.toml"), toml).expect("task");
    for (name, text) in files {
        std::fs::write(dir.join("repo").join(name), text).expect("file");
    }
}

#[tokio::test]
async fn runs_checks_and_reports() {
    let server = MockServer::start().await;
    // The edit task: read the file, edit it, then answer, each reporting its prompt size.
    // The scripted calls mount before the catch-all answer: the first match wins.
    let args = json!({"path":"a.txt","old_string":"six","new_string":"seven"}).to_string();
    Mock::given(method("POST"))
        .and(body_string_contains("Change six"))
        .respond_with(sse(json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c0","function":{"name":"read","arguments":"{\"path\":\"a.txt\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":100,"completion_tokens":8}})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("\"role\":\"tool\""))
        .and(body_string_contains("Change six"))
        .respond_with(sse(json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"edit","arguments":args}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":120,"completion_tokens":9}})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("\"role\":\"tool\""))
        .respond_with(sse(json!({
            "choices":[{"delta":{"content":"Done, it is 7."},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":150,"completion_tokens":5}
        })))
        .mount(&server)
        .await;
    // The asking task: a question nobody is there to answer, then the answer.
    let ask = json!({"question": "Which file?", "options": ["a.txt", "b.txt"]}).to_string();
    Mock::given(method("POST"))
        .and(body_string_contains("Ask me first"))
        .respond_with(sse(json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"q1","function":{"name":"ask_user","arguments":ask}}]},"finish_reason":"tool_calls"}]})))
        .mount(&server)
        .await;
    // Anything else: a wrong answer.
    Mock::given(method("POST"))
        .respond_with(sse(
            json!({"choices":[{"delta":{"content":"No idea."},"finish_reason":"stop"}]}),
        ))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let suite = tmp.path().join("tasks");
    task(
        &suite,
        "edit-one",
        "category = \"single-edit\"\nprompt = \"Change six to seven in a.txt.\"\n\
         [[check]]\nfile = \"a.txt\"\nequals = \"seven\"\n[[check]]\nanswer_contains = \"7\"\n\
         [[check]]\ntool_called = \"edit\"\n",
        &[("a.txt", "six\n")],
    );
    task(
        &suite,
        "answer",
        "category = \"navigate\"\nprompt = \"What is the port?\"\n[[check]]\nanswer_contains = \"8080\"\n",
        &[("config.toml", "port = 8080\n")],
    );
    // Unattended: the question is skipped at once and the model goes on.
    task(
        &suite,
        "asks",
        "category = \"navigate\"\nprompt = \"Ask me first.\"\n[[check]]\nanswer_contains = \"7\"\n\
         [[check]]\ntool_called = \"ask_user\"\n",
        &[],
    );
    task(
        &suite,
        "needs-tool",
        "category = \"shell\"\nprompt = \"x\"\nrequires = [\"no-such-program-here\"]\n",
        &[],
    );

    let user = tmp.path().join("user");
    std::fs::create_dir_all(user.join("providers.d")).expect("mkdir");
    std::fs::write(
        user.join("providers.d").join("mock.toml"),
        format!(
            "base_url = \"{}/v1\"\nmodels = [{{ id = \"tiny\", sampling = {{ temperature = 0.3, top_k = 40 }} }}]\n",
            server.uri()
        ),
    )
    .expect("provider");
    let mut env: HashMap<String, OsString> = HashMap::new();
    env.insert("QUACK_CONFIG_DIR".into(), user.into_os_string());
    env.insert(
        "QUACK_SYSTEM_DIR".into(),
        tmp.path().join("system").into_os_string(),
    );
    env.insert(
        "QUACK_CACHE_DIR".into(),
        tmp.path().join("cache").into_os_string(),
    );
    env.insert("QUACK_DEFAULTS_DIR".into(), "none".into());
    // A cache directory under a checkout must not redirect edits out of the task copy.
    std::fs::create_dir(tmp.path().join(".git")).expect("enclosing Git root");
    std::fs::write(tmp.path().join("a.txt"), "leave this alone\n").expect("outside task");
    let opts = EvalOptions {
        load: LoadOptions::default(),
        repeat: 1,
        keep: false,
        planner: None,
        scratch: tmp.path().join("scratch"),
        env,
    };
    let tasks = load_tasks(&suite, &[]).expect("tasks");
    assert_eq!(tasks.len(), 4);
    let mut results = Vec::new();
    for t in &tasks {
        results.push(run_task(t, &opts, 0).await);
    }
    let by = |id: &str| results.iter().find(|r| r.task == id).expect("result");
    let edit = by("edit-one");
    assert!(edit.passed, "{edit:?}");
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("a.txt")).expect("untouched file"),
        "leave this alone\n"
    );
    assert_eq!((edit.steps, edit.tool_calls, edit.edit_failures), (3, 2, 0));
    // How full the context got: the largest of the three requests.
    assert_eq!(
        (edit.prompt_tokens, edit.peak_prompt, edit.pruned),
        (370, 150, 0)
    );
    assert_eq!(edit.model, "mock/tiny");
    // What the requests carried is recorded with the result.
    assert_eq!(
        edit.sampling,
        [("temperature".to_owned(), 0.3), ("top_k".to_owned(), 40.0)]
    );
    let answer = by("answer");
    assert!(!answer.passed);
    assert_eq!(answer.failed_checks.len(), 1, "{answer:?}");
    assert_eq!(answer.unreported_requests, Some(1));
    assert!(answer.cost.is_none());
    assert!(by("needs-tool").skipped.is_some());
    let asks = by("asks");
    assert!(asks.passed, "{asks:?}");
    // Workspaces are cleaned up.
    assert!(std::fs::read_dir(tmp.path().join("scratch")).map_or(true, |mut d| d.next().is_none()));

    let report = Report {
        harness: "test".into(),
        date: "2026-10-18".into(),
        suite: "tasks".into(),
        model: "mock/tiny".into(),
        profile: "empty".into(),
        planner: None,
        window: None,
        sampling: Report::sampling_of(&edit.sampling),
        revision: None,
        planned_trials: Some(4),
        results,
    };
    assert_eq!(report.score(), (2, 3));
    let text = table(&report);
    assert!(
        text.contains("sampling: temperature 0.3, top_k 40 · 2/3 passed"),
        "{text}"
    );
    assert!(load_tasks(&suite, &["nope".into()]).is_err());
}

#[tokio::test]
async fn a_planner_plans_and_the_model_carries_it_out() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains(r#""model":"big""#))
        .respond_with(sse(
            json!({"choices":[{"delta":{"content":"1. Change six to seven in a.txt."},"finish_reason":"stop"}]}),
        ))
        .mount(&server)
        .await;
    let args = json!({"path":"a.txt","old_string":"six","new_string":"seven"}).to_string();
    Mock::given(method("POST"))
        .and(body_string_contains("[Plan handoff]"))
        .respond_with(sse(json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c0","function":{"name":"read","arguments":"{\"path\":\"a.txt\"}"}}]},"finish_reason":"tool_calls"}]})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("[Plan handoff]"))
        .and(body_string_contains("\"role\":\"tool\""))
        .respond_with(sse(json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"edit","arguments":args}}]},"finish_reason":"tool_calls"}]})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("[Plan handoff]"))
        .and(body_string_contains("\"role\":\"tool\""))
        .respond_with(sse(
            json!({"choices":[{"delta":{"content":"Done, it is 7."},"finish_reason":"stop"}]}),
        ))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let suite = tmp.path().join("tasks");
    task(
        &suite,
        "edit-one",
        "category = \"single-edit\"\nprompt = \"Change six to seven in a.txt.\"\n\
         [[check]]\nfile = \"a.txt\"\nequals = \"seven\"\n",
        &[("a.txt", "six\n")],
    );
    let user = tmp.path().join("user");
    std::fs::create_dir_all(user.join("providers.d")).expect("mkdir");
    std::fs::write(
        user.join("providers.d").join("mock.toml"),
        format!(
            "base_url = \"{}/v1\"\nmodels = [\"tiny\", \"big\"]\n",
            server.uri()
        ),
    )
    .expect("provider");
    let mut env: HashMap<String, OsString> = HashMap::new();
    env.insert("QUACK_CONFIG_DIR".into(), user.into_os_string());
    for (k, sub) in [("QUACK_SYSTEM_DIR", "system"), ("QUACK_CACHE_DIR", "cache")] {
        env.insert(k.into(), tmp.path().join(sub).into_os_string());
    }
    env.insert("QUACK_DEFAULTS_DIR".into(), "none".into());
    let mut load = LoadOptions::default();
    load.overrides.push("model.default=\"mock/tiny\"".into());
    let opts = EvalOptions {
        load,
        repeat: 1,
        keep: false,
        planner: Some("mock/big".into()),
        scratch: tmp.path().join("scratch"),
        env,
    };
    let tasks = load_tasks(&suite, &[]).expect("tasks");
    let r = run_task(&tasks[0], &opts, 0).await;
    assert!(r.passed, "{r:?}");
    assert_eq!(
        (r.steps, r.tool_calls),
        (4, 2),
        "one planning step, three to carry it out"
    );
    assert_eq!(
        r.model, "mock/tiny",
        "the result names the model that did the work"
    );
}

#[tokio::test]
async fn a_shared_base_hidden_checks_and_failure_labels() {
    let server = MockServer::start().await;
    // The model reads the overlay's file, edits it, and answers; the bug is in the base's.
    // The scripted calls mount before the catch-all answer: the first match wins.
    Mock::given(method("POST"))
        .and(body_string_contains("Fix the bug"))
        .respond_with(sse(json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c0","function":{"name":"read","arguments":"{\"path\":\"app.txt\"}"}}]},"finish_reason":"tool_calls"}]})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let args = json!({"path":"app.txt","old_string":"v1","new_string":"v2"}).to_string();
    Mock::given(method("POST"))
        .and(body_string_contains("\"role\":\"tool\""))
        .and(body_string_contains("v1"))
        .respond_with(sse(json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"edit","arguments":args}}]},"finish_reason":"tool_calls"}]})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("\"role\":\"tool\""))
        .respond_with(sse(
            json!({"choices":[{"delta":{"content":"Fixed."},"finish_reason":"stop"}]}),
        ))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let suite = tmp.path().join("hard");
    let base = suite.join("_projects").join("shop");
    std::fs::create_dir_all(&base).expect("mkdir");
    std::fs::write(base.join("lib.txt"), "bug\n").expect("base");
    std::fs::write(base.join("app.txt"), "v0\n").expect("base");
    task(
        &suite,
        "fix-lib",
        "category = \"fix-test\"\nprompt = \"Fix the bug.\"\nbase = \"../_projects/shop\"\n\
         [[check]]\nfile = \"lib.txt\"\nequals = \"fixed\"\n\
         [[check]]\nfile = \"revealed.txt\"\nequals = \"yes\"\n",
        // The overlay replaces one of the base's files.
        &[("app.txt", "v1\n")],
    );
    let dir = suite.join("fix-lib");
    std::fs::create_dir_all(dir.join("hidden")).expect("mkdir");
    std::fs::write(dir.join("hidden").join("revealed.txt"), "yes\n").expect("hidden");
    std::fs::create_dir_all(dir.join("solution")).expect("mkdir");
    std::fs::write(dir.join("solution").join("lib.txt"), "fixed\n").expect("solution");

    let user = tmp.path().join("user");
    std::fs::create_dir_all(user.join("providers.d")).expect("mkdir");
    std::fs::write(
        user.join("providers.d").join("mock.toml"),
        format!("base_url = \"{}/v1\"\nmodels = [\"tiny\"]\n", server.uri()),
    )
    .expect("provider");
    let mut env: HashMap<String, OsString> = HashMap::new();
    env.insert("QUACK_CONFIG_DIR".into(), user.into_os_string());
    for (k, sub) in [("QUACK_SYSTEM_DIR", "system"), ("QUACK_CACHE_DIR", "cache")] {
        env.insert(k.into(), tmp.path().join(sub).into_os_string());
    }
    env.insert("QUACK_DEFAULTS_DIR".into(), "none".into());
    let opts = EvalOptions {
        load: LoadOptions::default(),
        repeat: 1,
        keep: false,
        planner: None,
        scratch: tmp.path().join("scratch"),
        env,
    };
    let tasks = load_tasks(&suite, &[]).expect("tasks");
    assert_eq!(tasks.len(), 1, "the projects folder is not a task");
    assert_eq!(tasks[0].solution_paths(), ["lib.txt"]);
    let r = run_task(&tasks[0], &opts, 0).await;
    assert!(!r.passed, "{r:?}");
    assert_eq!(
        r.changed,
        ["app.txt"],
        "hidden files are not the model's changes"
    );
    assert_eq!(
        r.failed_checks,
        ["lib.txt is not \"fixed\""],
        "the hidden file is there for the checks"
    );
    assert_eq!(r.failure.map(|f| f.label()), Some("wrong-files"));
    let json = serde_json::to_value(&r).expect("json");
    assert_eq!(json["failure"], "wrong-files");
}

fn mock_options(root: &Path, server: &MockServer, models: &str) -> EvalOptions {
    let user = root.join("user");
    std::fs::create_dir_all(user.join("providers.d")).expect("providers.d");
    std::fs::write(
        user.join("providers.d").join("mock.toml"),
        format!("base_url = \"{}/v1\"\nmodels = {models}\n", server.uri()),
    )
    .expect("provider file");
    let env = [
        ("QUACK_CONFIG_DIR", user.into_os_string()),
        ("QUACK_SYSTEM_DIR", root.join("system").into_os_string()),
        ("QUACK_CACHE_DIR", root.join("cache").into_os_string()),
        ("QUACK_DEFAULTS_DIR", "none".into()),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v))
    .collect();
    EvalOptions {
        load: LoadOptions::default(),
        repeat: 1,
        keep: false,
        planner: None,
        scratch: root.join("scratch"),
        env,
    }
}

#[tokio::test]
async fn explicit_and_automatic_profiles_reach_the_wire_and_the_report() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(sse(json!({
            "choices":[{"delta":{"content":"Done."},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":100,"completion_tokens":8,"prompt_tokens_details":{"cached_tokens":40}}
        })))
        .expect(3)
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let suite = tmp.path().join("tasks");
    task(
        &suite,
        "answer",
        "category = \"navigate\"\nprompt = \"Say done.\"\n[[check]]\nanswer_contains = \"done\"\n",
        &[],
    );
    let tasks = load_tasks(&suite, &[]).expect("tasks");
    let mut opts = mock_options(
        tmp.path(),
        &server,
        r#"[{id = "accounts/fireworks/models/glm-5p3-flash", reasoning = {effort = "low"}, cost = {input = 0.15, cached_input = 0.03, output = 0.5}}]"#,
    );
    // Use the shipped profiles with only a loopback provider, no bundled services.
    let profile_src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("defaults")
        .join("profiles.d");
    let user_profiles = tmp.path().join("user").join("profiles.d");
    for id in ["generic", "zcode"] {
        quack_exec::eval::copy_dir(&profile_src.join(id), &user_profiles.join(id))
            .expect("copy profile");
    }
    for (attempt, (requested, effective, read, path)) in [
        ("generic", "generic", "read", "path"),
        ("zcode", "zcode", "Read", "file_path"),
        ("auto", "zcode", "Read", "file_path"),
    ]
    .into_iter()
    .enumerate()
    {
        opts.load.overrides = vec![
            format!("model.profile={requested}"),
            "context.window=16384".into(),
        ];
        let result = run_task(&tasks[0], &opts, attempt as u32).await;
        assert!(result.passed && !result.incomplete(), "{result:?}");
        let row = serde_json::to_value(&result).expect("result JSON");
        assert_eq!(row["attempt"], attempt + 1);
        assert_eq!(row["model"], "mock/accounts/fireworks/models/glm-5p3-flash");
        assert_eq!(row["profile"], effective);
        assert_eq!(row["edit_format"], "str_replace");
        assert_eq!(row["reasoning_effort"], "low");
        assert_eq!(row["window"], 16384);
        assert_eq!(row["unreported_requests"], 0);
        assert_eq!(row["cache_reported_requests"], 1);

        let requests = server.received_requests().await.expect("recorded requests");
        let body: serde_json::Value = requests
            .iter()
            .filter(|r| r.method.as_str() == "POST")
            .nth(attempt)
            .expect("model call")
            .body_json()
            .expect("request JSON");
        assert_eq!(body["reasoning_effort"], "low");
        assert_eq!(body["model"], "accounts/fireworks/models/glm-5p3-flash");
        let tools = body["tools"].as_array().expect("tool surface");
        let reader = tools
            .iter()
            .find(|tool| tool["function"]["name"] == read)
            .expect("reader");
        assert!(
            reader["function"]["parameters"]["properties"]
                .get(path)
                .is_some()
        );
    }
}

#[tokio::test]
async fn provider_error_retains_observed_cost_without_pricing_the_trial() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_string_contains(r#""role":"tool""#))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error":{"message":"request rejected"}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(sse(json!({
            "choices":[{"delta":{"tool_calls":[{"index":0,"id":"c0","function":{"name":"read","arguments":"{\"path\":\"a.txt\"}"}}]},"finish_reason":"tool_calls"}],
            "usage":{"prompt_tokens":100,"completion_tokens":8}
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let suite = tmp.path().join("tasks");
    task(
        &suite,
        "answer",
        "category = \"navigate\"\nprompt = \"Read a.txt.\"\n",
        &[("a.txt", "fixture")],
    );
    let tasks = load_tasks(&suite, &[]).expect("tasks");
    let opts = mock_options(
        tmp.path(),
        &server,
        r#"[{id = "tiny", cost = {input = 1.0, output = 2.0}}]"#,
    );
    let result = run_task(&tasks[0], &opts, 0).await;
    assert!(!result.passed && result.incomplete(), "{result:?}");
    assert_eq!(result.error_kind, Some(RunErrorKind::Provider));
    assert_eq!(result.failure.map(|f| f.label()), Some("error"));
    assert!(result.cost.is_none());
    assert!(result.observed_cost.is_some_and(|cost| cost > 0.0));
    assert!(result.failed_checks.is_empty());
}
