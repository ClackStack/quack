//! quack's own requests, the ones that aren't to a provider or MCP server: the model catalog
//! and the version check. Each one passes the egress gate (`Settings::check_fetch`), and so
//! does every redirect, so a gated URL can't lead anywhere the gate would refuse. They carry
//! no identifier and no credentials.

use std::time::Duration;

use quack_config::{Env, Settings};
use reqwest::header::{ETAG, IF_NONE_MATCH, LOCATION};
use reqwest::{StatusCode, Url};

const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REDIRECTS: usize = 5;

/// One request.
#[derive(Debug, Clone, Copy)]
pub struct Get<'a> {
    pub url: &'a str,
    /// The tag of the copy already here: an unchanged resource answers [`Fetched::NotModified`].
    pub etag: Option<&'a str>,
    /// Follow redirects; otherwise the first one is the answer.
    pub follow: bool,
    /// Refuse a larger body.
    pub max_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetched {
    Body {
        bytes: Vec<u8>,
        etag: Option<String>,
    },
    NotModified,
    /// Where a redirect pointed, when not following them.
    Moved(String),
}

/// The client [`get`] needs, with the user's proxy and CA settings.
pub fn client(settings: &Settings, env: Env<'_>) -> Result<reqwest::Client, String> {
    crate::provider::unredirected_client(&settings.network, env)
}

pub async fn get(
    client: &reqwest::Client,
    settings: &Settings,
    req: Get<'_>,
) -> Result<Fetched, String> {
    let mut url = Url::parse(req.url).map_err(|e| format!("`{}`: {e}", req.url))?;
    for _ in 0..=MAX_REDIRECTS {
        settings.check_fetch(url.as_str())?;
        let mut request = client.get(url.clone()).timeout(TIMEOUT);
        if let Some(tag) = req.etag {
            request = request.header(IF_NONE_MATCH, tag);
        }
        let mut resp = request.send().await.map_err(|e| failed(&url, &e))?;
        let status = resp.status();
        if status == StatusCode::NOT_MODIFIED {
            return Ok(Fetched::NotModified);
        }
        if status.is_redirection() {
            let target = resp
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|l| url.join(l).ok())
                .ok_or_else(|| format!("{url} redirected without saying where"))?;
            if !req.follow {
                return Ok(Fetched::Moved(target.into()));
            }
            url = target;
            continue;
        }
        if !status.is_success() {
            return Err(format!("{url} answered {status}"));
        }
        let etag = resp
            .headers()
            .get(ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let mut bytes = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| failed(&url, &e))? {
            if bytes.len() + chunk.len() > req.max_bytes {
                return Err(format!("{url} sent more than {} bytes", req.max_bytes));
            }
            bytes.extend_from_slice(&chunk);
        }
        return Ok(Fetched::Body { bytes, etag });
    }
    Err(format!("{} redirected too many times", req.url))
}

fn failed(url: &Url, e: &reqwest::Error) -> String {
    format!(
        "could not reach {}: {e}",
        url.host_str().unwrap_or("the server")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn fetch(s: &Settings, req: Get<'_>) -> Result<Fetched, String> {
        let c = client(s, &|_| None).expect("client");
        get(&c, s, req).await
    }

    fn req(url: &str) -> Get<'_> {
        Get {
            url,
            etag: None,
            follow: true,
            max_bytes: 1024,
        }
    }

    fn offline() -> Settings {
        let mut s = Settings::default();
        s.network.offline = true;
        s
    }

    async fn moved(server: &MockServer, from: &str, to: &str) {
        Mock::given(path(from))
            .respond_with(ResponseTemplate::new(302).insert_header("location", to))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn sends_the_tag_and_reports_no_change() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header("if-none-match", "\"a\""))
            .respond_with(ResponseTemplate::new(304))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("etag", "\"a\"")
                    .set_body_string("hello"),
            )
            .mount(&server)
            .await;
        let s = Settings::default();
        let url = server.uri();
        assert_eq!(
            fetch(&s, req(&url)).await,
            Ok(Fetched::Body {
                bytes: b"hello".to_vec(),
                etag: Some("\"a\"".into())
            })
        );
        let again = Get {
            etag: Some("\"a\""),
            ..req(&url)
        };
        assert_eq!(fetch(&s, again).await, Ok(Fetched::NotModified));
    }

    #[tokio::test]
    async fn follows_redirects_or_reports_them() {
        let server = MockServer::start().await;
        moved(&server, "/a", "/b").await;
        Mock::given(path("/b"))
            .respond_with(ResponseTemplate::new(200).set_body_string("here"))
            .mount(&server)
            .await;
        let s = Settings::default();
        let url = format!("{}/a", server.uri());
        let got = fetch(&s, req(&url)).await.expect("followed");
        assert!(matches!(got, Fetched::Body { bytes, .. } if bytes == b"here"));
        let once = Get {
            follow: false,
            ..req(&url)
        };
        assert_eq!(
            fetch(&s, once).await,
            Ok(Fetched::Moved(format!("{}/b", server.uri())))
        );
    }

    #[tokio::test]
    async fn offline_stops_the_request_before_it_leaves() {
        let s = offline();
        let err = fetch(&s, req("https://feed.example/api.json")).await;
        assert!(err.expect_err("blocked").contains("egress_allow"));
    }

    #[tokio::test]
    async fn a_redirect_cannot_step_around_the_gate() {
        let server = MockServer::start().await;
        moved(&server, "/a", "https://elsewhere.example/feed").await;
        let url = format!("{}/a", server.uri());
        let err = fetch(&offline(), req(&url)).await;
        assert!(err.expect_err("blocked").contains("elsewhere.example"));
    }

    #[tokio::test]
    async fn policy_hosts_limit_these_requests_too() {
        let mut s = Settings::default();
        s.policy.providers.hosts = vec!["llm.corp.example".into()];
        let err = fetch(&s, req("https://feed.example/api.json")).await;
        assert!(err.expect_err("blocked").contains("policy"));
        assert!(s.check_fetch("https://llm.corp.example/x").is_ok());
    }

    #[tokio::test]
    async fn refuses_an_oversized_body_and_an_error_status() {
        let server = MockServer::start().await;
        Mock::given(path("/big"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(2000)))
            .mount(&server)
            .await;
        Mock::given(path("/gone"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let s = Settings::default();
        let big = format!("{}/big", server.uri());
        let gone = format!("{}/gone", server.uri());
        assert!(fetch(&s, req(&big)).await.is_err());
        assert!(fetch(&s, req(&gone)).await.is_err());
    }
}
