//! `doctor` against a mock provider.

mod common;

use std::sync::Arc;

use quack_core::doctor;
use quack_core::protocol::CheckStatus;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn reports_each_provider_and_the_local_checks() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"data": [{"id": "tiny"}, {"id": "big"}]})),
        )
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = common::workspace(tmp.path(), &server.uri(), "[]", "");
    let checks = doctor::run(Arc::new(loaded)).await;
    let get = |name: &str| {
        checks
            .iter()
            .find(|c| c.name.trim() == name)
            .unwrap_or_else(|| panic!("no {name} check in {checks:?}"))
    };
    assert_eq!(get("config").status, CheckStatus::Ok);
    assert_eq!(get("providers").status, CheckStatus::Ok);
    assert!(
        get("mock").detail.starts_with("2 models"),
        "{:?}",
        get("mock")
    );
    assert_eq!(get("sessions").status, CheckStatus::Ok);
    assert_eq!(get("defaults").status, CheckStatus::Info);
    let names: Vec<&str> = checks.iter().map(|c| c.name.trim()).collect();
    assert_eq!(&names[..4], ["config", "defaults", "providers", "mock"]);
}

#[tokio::test]
async fn a_provider_that_pins_a_stale_model_list_is_flagged() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": []})))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let defaults = tmp.path().join("defaults");
    std::fs::create_dir_all(defaults.join("providers.d")).expect("mkdir");
    std::fs::write(
        defaults.join("providers.d").join("charm.toml"),
        "name = \"Charm\"\nbase_url = \"https://hyper.example/v1\"\n\
         api_key = { env = \"QUACK_TEST_UNSET_KEY\" }\n\
         [[models]]\nid = \"glm-5.3\"\n[[models]]\nid = \"kimi-k3\"\n",
    )
    .expect("bundle file");
    // A full copy from an older bundle: the list it pins has moved on.
    let user = tmp.path().join("user").join("providers.d");
    std::fs::create_dir_all(&user).expect("mkdir");
    std::fs::write(
        user.join("charm.toml"),
        "base_url = \"https://hyper.example/v1\"\n\
         api_key = { keyring = \"charm\" }\n\
         [[models]]\nid = \"glm-5.3\"\n",
    )
    .expect("user copy");
    let loaded = common::workspace_in(tmp.path(), &server.uri(), "[]", "", Some(&defaults));
    let checks = doctor::run(Arc::new(loaded)).await;
    let drift = checks
        .iter()
        .find(|c| c.name == "  charm (override)")
        .unwrap_or_else(|| panic!("no drift check in {checks:?}"));
    assert_eq!(drift.status, CheckStatus::Warn);
    assert!(
        drift.detail.contains("adds kimi-k3") && drift.detail.contains("follow the bundle"),
        "{drift:?}"
    );
}
