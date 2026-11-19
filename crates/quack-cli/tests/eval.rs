//! Checkpointing and stop conditions through the real CLI, with no live provider.

use std::path::PathBuf;
use std::process::Command;

use quack_exec::eval::Report;
use serde_json::json;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn sse(chunk: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(format!("data: {chunk}\n\ndata: [DONE]\n\n"))
}

struct Checkpoint(PathBuf);

impl Respond for Checkpoint {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let report: Report = serde_json::from_slice(&std::fs::read(&self.0).expect("checkpoint"))
            .expect("valid JSON while the next trial is running");
        assert_eq!(report.planned_trials, Some(3));
        assert_eq!(report.results.len(), 1);
        assert_eq!(report.results[0].task, "a");
        assert!(report.results[0].cost.is_some());
        assert!(
            !report.results[0].passed,
            "the priced verifier miss did not stop the run"
        );
        sse(json!({"choices":[{"delta":{"content":"expected"},"finish_reason":"stop"}]}))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoints_a_priced_failure_and_stops_after_an_unpriced_trial() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let root = scratch.path();
    let server = MockServer::start().await;
    let user = root.join("config");
    std::fs::create_dir_all(user.join("providers.d")).expect("providers.d");
    std::fs::write(
        user.join("providers.d").join("mock.toml"),
        format!(
            "base_url=\"{}/v1\"\nmodels=[{{id=\"tiny\",cost={{input=1.0,output=1.0}}}}]\n",
            server.uri()
        ),
    )
    .expect("provider file");
    for (id, prompt) in [
        ("a", "First trial."),
        ("b", "Second trial."),
        ("c", "Never run this."),
    ] {
        let task = root.join("tasks").join(id);
        std::fs::create_dir_all(task.join("repo")).expect("task repo");
        std::fs::write(task.join("task.toml"), format!(
            "category=\"navigate\"\nprompt=\"{prompt}\"\ntimeout_secs=10\n[[check]]\nanswer_contains=\"expected\"\n"
        )).expect("task file");
    }
    std::fs::write(root.join("baseline.json"), "earlier report").expect("prior result");
    let checkpoint = root.join("baseline-2.json");
    Mock::given(method("POST"))
        .and(body_string_contains("First trial."))
        .respond_with(sse(json!({
            "choices":[{"delta":{"content":"wrong"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":80,"completion_tokens":2}
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("Second trial."))
        .respond_with(Checkpoint(checkpoint.clone()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(body_string_contains("Never run this."))
        .respond_with(ResponseTemplate::new(400))
        .expect(0)
        .mount(&server)
        .await;

    let mut command = Command::new(env!("CARGO_BIN_EXE_quack"));
    command.current_dir(root).env_clear();
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", system_root);
    }
    let output = command
        .env("QUACK_DEFAULTS_DIR", "none")
        .env("QUACK_CONFIG_DIR", user)
        .env("QUACK_SYSTEM_DIR", root.join("system"))
        .env("QUACK_CACHE_DIR", root.join("cache"))
        .env("QUACK_DATA_DIR", root.join("data"))
        .args([
            "eval",
            "-m",
            "mock/tiny",
            "--suite",
            "tasks",
            "--out",
            "baseline.json",
            "--revision",
            "test-build",
            "--stop-on-incomplete",
        ])
        .output()
        .expect("start quack");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("stopped after an incomplete trial"),
        "{stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("baseline.json")).expect("prior result"),
        "earlier report"
    );
    let report: Report = serde_json::from_slice(&std::fs::read(checkpoint).expect("checkpoint"))
        .expect("valid final checkpoint");
    assert_eq!(report.revision.as_deref(), Some("test-build"));
    assert_eq!(report.planned_trials, Some(3));
    assert_eq!(report.results.len(), 2);
    assert!(report.results[1].passed);
    assert_eq!(report.results[1].unreported_requests, Some(1));
    assert!(report.results[1].cost.is_none());
}
