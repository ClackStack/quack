//! Black-box regressions for the shipped Python handlers. All assertions are Rust.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn script(name: &str, file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../defaults/hooks.d")
        .join(name)
        .join(file)
}

fn python() -> Option<PathBuf> {
    let path = std::env::var_os("PATH");
    ["python3", "python", "py"]
        .iter()
        .find_map(|name| quack_shell::resolve_program(name, path.as_deref(), Path::new(".")).ok())
}

async fn run(file: &Path, input: Value, secret: Option<&str>) -> Value {
    let python = python().expect("Python available");
    let mut env = quack_shell::inherited_env(None);
    if let Some(secret) = secret {
        env.insert("TOKEN".into(), secret.into());
    }
    let result = quack_shell::exec(
        &python,
        &["-B".into(), file.display().to_string()],
        quack_shell::ExecOptions {
            cwd: file.parent().expect("script directory").into(),
            env,
            stdin: serde_json::to_vec(&input).expect("input JSON"),
            full_stdout: true,
            timeout: Duration::from_secs(10),
            cancel: CancellationToken::new(),
        },
    )
    .await
    .expect("run script");
    assert_eq!(result.exit_code, Some(0), "{}", result.stderr);
    if result.stdout.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&result.stdout).expect("hook JSON")
    }
}

fn connection(kind: &str, web: &str, api: &str, auth: &str) -> Value {
    json!({"id": "test", "enabled": true, "values": {
        "kind": kind, "web_url": web, "api_url": api, "auth": auth,
        "user": "user", "timeout": "2"
    }, "credentials": {"token": "TOKEN"}})
}

async fn review(server: &MockServer, endpoint: &str, title: &str, diff: &str) {
    Mock::given(method("GET"))
        .and(path(endpoint))
        .and(header("accept", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"title": title})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(endpoint))
        .and(header("accept", "application/vnd.github.diff"))
        .respond_with(ResponseTemplate::new(200).set_body_string(diff))
        .mount(server)
        .await;
}

#[tokio::test]
async fn configured_authentication_is_scoped_and_checks_fetch_metadata_and_diff() {
    if python().is_none() {
        return;
    }
    for (kind, auth, auth_header, auth_value, link, endpoint, diff_suffix, accept) in [
        (
            "github",
            "token",
            "authorization",
            "Bearer test-token",
            "/a/b/pull/1",
            "/repos/a/b/pulls/1",
            "",
            "application/vnd.github.diff",
        ),
        (
            "gitlab",
            "token",
            "private-token",
            "test-token",
            "/a/sub/b/-/merge_requests/01/diffs",
            "/projects/a%2Fsub%2Fb/merge_requests/1",
            "/raw_diffs",
            "text/plain",
        ),
        (
            "gitea",
            "token",
            "authorization",
            "token test-token",
            "/a/b/pulls/1",
            "/repos/a/b/pulls/1",
            ".diff",
            "text/plain",
        ),
        (
            "bitbucket",
            "basic",
            "authorization",
            "Basic dXNlcjp0ZXN0LXRva2Vu",
            "/a/b/pull-requests/1",
            "/repositories/a/b/pullrequests/1",
            "/diff",
            "text/plain",
        ),
        (
            "bitbucket-server",
            "token",
            "authorization",
            "Bearer test-token",
            "/projects/A/repos/b/pull-requests/1",
            "/projects/A/repos/b/pull-requests/1",
            ".diff",
            "text/plain",
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/prefix/api{endpoint}")))
            .and(header("accept", "application/json"))
            .and(header(auth_header, auth_value))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"title":"Private review"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/prefix/api{endpoint}{diff_suffix}")))
            .and(header("accept", accept))
            .and(header(auth_header, auth_value))
            .respond_with(ResponseTemplate::new(200).set_body_string("private diff"))
            .expect(1)
            .mount(&server)
            .await;
        let conn = connection(
            kind,
            &format!("{}/prefix", server.uri()),
            &format!("{}/prefix/api", server.uri()),
            auth,
        );
        let output = run(&script("forge-review", "review.py"), json!({"event":"check", "matches":[format!("{}/prefix{link}", server.uri())], "connections":[conn]}), Some("test-token")).await;
        assert_eq!(output["status"], "verified", "{kind}: {output}");
        assert!(!output.to_string().contains("test-token"));
        assert!(output.get("additional_context").is_none());
        server.verify().await;
    }
}

#[tokio::test]
async fn longest_prefix_wins_and_disabled_or_empty_lists_do_not_fall_back() {
    if python().is_none() {
        return;
    }
    let server = MockServer::start().await;
    review(&server, "/specific/repos/a/b/pulls/1", "Specific", "diff").await;
    let root = connection(
        "github",
        &server.uri(),
        &format!("{}/wrong", server.uri()),
        "public",
    );
    let specific = connection(
        "github",
        &format!("{}/prefix", server.uri()),
        &format!("{}/specific", server.uri()),
        "public",
    );
    let url = format!("{}/prefix/a/b/pull/1", server.uri());
    let output = run(
        &script("forge-review", "review.py"),
        json!({"matches":[url],"connections":[root,specific]}),
        None,
    )
    .await;
    assert!(
        output["additional_context"]
            .as_str()
            .expect("context")
            .contains("Specific")
    );
    let mut disabled = connection(
        "github",
        &format!("{}/prefix", server.uri()),
        &format!("{}/specific", server.uri()),
        "public",
    );
    disabled["enabled"] = json!(false);
    let before = server.received_requests().await.expect("requests").len();
    for connections in [json!([disabled]), json!([])] {
        assert!(
            run(
                &script("forge-review", "review.py"),
                json!({"matches":[url],"connections":connections}),
                None
            )
            .await
            .is_null()
        );
    }
    assert_eq!(
        server.received_requests().await.expect("requests").len(),
        before
    );
}

#[tokio::test]
async fn invalid_urls_do_not_fetch_and_review_limits_deduplicate_and_strip_queries() {
    if python().is_none() {
        return;
    }
    let server = MockServer::start().await;
    let web = format!("{}/team", server.uri());
    let api = format!("{}/api", server.uri());
    let conn = connection("github", &web, &api, "public");
    let mut matches = Vec::new();
    for part in ["..", "%2e%2e", "%2f", "%5c"] {
        matches.push(format!("{web}/a/{part}/pull/1"));
    }
    matches.extend([
        format!("{}/teammate/a/b/pull/1", server.uri()),
        format!("{web}/a/b/pull/0"),
        format!("{web}/a/b/pull/1garbage"),
        format!("{web}/a/b/pull/1?token=private-query#L1"),
        format!("{web}/a/b/pull/01/files"),
        format!("{web}/a/b/pull/2"),
        format!("{web}/a/b/pull/3"),
    ]);
    review(&server, "/api/repos/a/b/pulls/1", "First <title>", "diff").await;
    review(&server, "/api/repos/a/b/pulls/2", "Second", "diff").await;
    let output = run(
        &script("forge-review", "review.py"),
        json!({"matches":matches,"connections":[conn]}),
        None,
    )
    .await;
    let context = output["additional_context"].as_str().expect("context");
    assert!(context.contains("First &lt;title&gt;") && context.contains("Second"));
    assert!(!context.contains("private-query") && !context.contains("#L1"));
    assert!(context.contains("first 2 linked reviews"));
    assert_eq!(server.received_requests().await.expect("requests").len(), 4);
}

#[tokio::test]
async fn cross_origin_redirects_do_not_receive_credentials_and_diff_failures_are_partial() {
    if python().is_none() {
        return;
    }
    let server = MockServer::start().await;
    let other = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("accept", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"title":"Keep metadata"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(header("accept", "application/vnd.github.diff"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", format!("{}/stolen", other.uri())),
        )
        .mount(&server)
        .await;
    let conn = connection("github", &server.uri(), &server.uri(), "token");
    let url = format!("{}/a/b/pull/1", server.uri());
    let output = run(
        &script("forge-review", "review.py"),
        json!({"event":"check","matches":[url],"connections":[conn]}),
        Some("test-token"),
    )
    .await;
    assert_eq!(output["status"], "partial");
    assert!(
        other
            .received_requests()
            .await
            .expect("requests")
            .is_empty()
    );
}

#[tokio::test]
async fn check_errors_are_actionable_and_large_diffs_remain_complete() {
    if python().is_none() {
        return;
    }
    for (status, expected) in [
        (401, "authentication_failed"),
        (403, "authentication_failed"),
        (404, "resource_unavailable"),
        (429, "rate_limited"),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(status).set_body_string("server may echo test-token"),
            )
            .mount(&server)
            .await;
        let conn = connection("github", &server.uri(), &server.uri(), "token");
        let output = run(&script("forge-review", "review.py"), json!({"event":"check","matches":[format!("{}/a/b/pull/1",server.uri())],"connections":[conn]}), Some("test-token")).await;
        assert_eq!(output["status"], expected);
        assert!(!output.to_string().contains("test-token"));
    }
    let server = MockServer::start().await;
    let diff = format!("+<injected>\n{}END_OF_DIFF", "x".repeat(2 * 1024 * 1024));
    review(&server, "/repos/a/b/pulls/1", "Large", &diff).await;
    let conn = connection("github", &server.uri(), &server.uri(), "public");
    let output = run(
        &script("forge-review", "review.py"),
        json!({"matches":[format!("{}/a/b/pull/1",server.uri())],"connections":[conn]}),
        None,
    )
    .await;
    let context = output["additional_context"].as_str().expect("context");
    assert!(context.contains("+&lt;injected&gt;") && context.contains("END_OF_DIFF"));
    assert!(context.len() > 2 * 1024 * 1024);
    assert!(
        output["message"]
            .as_str()
            .expect("message")
            .contains("full diff was retained")
    );
}

#[tokio::test]
async fn jira_uses_scoped_connections_and_isolates_an_unavailable_credential() {
    if python().is_none() {
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/rest/api/2/issue/PAY-1"))
        .and(header("authorization", "Basic dXNlcjp0ZXN0LXRva2Vu"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"fields":{"summary":"Private issue","description":"<details>"}}),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;
    let mut conn = connection(
        "jira",
        &format!("{}/team", server.uri()),
        &format!("{}/api", server.uri()),
        "basic",
    );
    conn["values"]["max_issues"] = json!("5");
    let output = run(&script("jira","jira.py"),json!({"event":"check","matches":[format!("{}/team/browse/PAY-1",server.uri())],"connections":[conn]}),Some("test-token")).await;
    assert_eq!(output["status"], "verified");
    server.verify().await;
    let mut bad = connection("github", &server.uri(), &server.uri(), "token");
    bad["credentials"] = json!({});
    let good = connection(
        "github",
        &format!("{}/team", server.uri()),
        &format!("{}/api", server.uri()),
        "public",
    );
    review(
        &server,
        "/api/repos/a/b/pulls/2",
        "Working connection",
        "diff",
    )
    .await;
    let output = run(&script("forge-review","review.py"),json!({"matches":[format!("{}/a/b/pull/1",server.uri()),format!("{}/team/a/b/pull/2",server.uri())],"connections":[bad,good]}),None).await;
    let context = output["additional_context"].as_str().expect("context");
    assert!(context.contains("unavailable") && context.contains("Working connection"));
}

#[tokio::test]
async fn malformed_or_oversized_metadata_keeps_other_reviews_available() {
    if python().is_none() {
        return;
    }
    for payload in [
        "{".to_owned(),
        "[]".into(),
        r#"{"title":"bad","body":{}}"#.into(),
        " ".repeat(1024 * 1024 + 1),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/a/b/pulls/1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(payload))
            .mount(&server)
            .await;
        review(
            &server,
            "/repos/a/b/pulls/2",
            "Healthy second review",
            "diff",
        )
        .await;
        let conn = connection("github", &server.uri(), &server.uri(), "public");
        let output=run(&script("forge-review","review.py"),json!({"matches":[format!("{}/a/b/pull/1",server.uri()),format!("{}/a/b/pull/2",server.uri())],"connections":[conn]}),None).await;
        let context = output["additional_context"].as_str().expect("context");
        assert!(context.contains("unavailable") && context.contains("Healthy second review"));
        assert_eq!(server.received_requests().await.expect("requests").len(), 3);
    }
}

#[tokio::test]
async fn redirects_cannot_forward_credentials_to_another_installation_on_the_same_host() {
    if python().is_none() {
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/one/api/repos/a/b/pulls/1"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/two/api/private"))
        .mount(&server)
        .await;
    let conn = connection(
        "github",
        &format!("{}/one", server.uri()),
        &format!("{}/one/api", server.uri()),
        "token",
    );
    let output=run(&script("forge-review","review.py"),json!({"event":"check","matches":[format!("{}/one/a/b/pull/1",server.uri())],"connections":[conn]}),Some("test-token")).await;
    assert_ne!(output["status"], "verified");
    assert!(
        !server
            .received_requests()
            .await
            .expect("requests")
            .iter()
            .any(|r| r.url.path().starts_with("/two"))
    );
}
