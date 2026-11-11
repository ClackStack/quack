//! Sign in with ChatGPT against a mock issuer and API: the browser flow, token renewal,
//! and that nothing is sent before a sign-in.
#![cfg(feature = "auth-chatgpt")]

use std::path::Path;
use std::sync::{Arc, Mutex};

use quack_config::ProviderFile;
use quack_config::provider::ReasoningFormat;
use quack_config::settings::NetworkSettings;
use quack_providers::login::Credentials;
use quack_providers::{ChatRequest, ErrorKind, Message, Provider, ResponseBuilder};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const PLAN_SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";

fn provider(server: &MockServer, data: &Path) -> Provider {
    let file: ProviderFile = toml::from_str(&format!(
        "api = \"openai-responses\"\nbase_url = \"{0}/v1\"\n\
         auth = {{ type = \"chatgpt\", issuer = \"{0}\" }}\nmodels = [\"m\"]\n",
        server.uri()
    ))
    .expect("provider toml");
    let data = data.as_os_str().to_owned();
    let env = move |k: &str| (k == "QUACK_DATA_DIR").then(|| data.clone());
    Provider::new("chatgpt", file, &NetworkSettings::default(), &env).expect("provider")
}

fn save(data: &Path, c: &Credentials) {
    let dir = data.join("auth");
    std::fs::create_dir_all(&dir).expect("dir");
    std::fs::write(
        dir.join("chatgpt.json"),
        serde_json::to_string(c).expect("json"),
    )
    .expect("write");
}

fn saved(data: &Path) -> Credentials {
    let text = std::fs::read_to_string(data.join("auth").join("chatgpt.json")).expect("read");
    serde_json::from_str(&text).expect("parse")
}

fn signed_in(expires_at: u64) -> Credentials {
    Credentials {
        client_id: "oaiapp_1".into(),
        subject: "user-1".into(),
        email: "a@example.com".into(),
        access_token: "a1".into(),
        refresh_token: "r1".into(),
        id_token: "id".into(),
        expires_at,
        scopes: PLAN_SCOPES.split(' ').map(str::to_owned).collect(),
    }
}

const SSE: &str = "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"delta\":\"Hi\"}\n\n\
data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":1}}}\n\n";

async fn ask(p: &Provider) -> Result<String, quack_providers::ProviderError> {
    let req = ChatRequest {
        model: "m".into(),
        messages: vec![Message::User {
            content: "hi".into(),
            images: Vec::new(),
        }],
        ..ChatRequest::default()
    };
    let mut rx = p.stream(req, ReasoningFormat::Auto, CancellationToken::new());
    let mut b = ResponseBuilder::default();
    while let Some(ev) = rx.recv().await {
        b.push(&ev?);
    }
    match b.into_message() {
        Message::Assistant { content, .. } => Ok(content),
        _ => Ok(String::new()),
    }
}

#[tokio::test]
async fn nothing_is_sent_before_a_sign_in() {
    let server = MockServer::start().await;
    let data = tempfile::tempdir().expect("tempdir");
    let p = provider(&server, data.path());
    assert!(
        p.not_ready()
            .is_some_and(|w| w.contains("quack login chatgpt"))
    );
    let e = p.list_models().await.expect_err("not signed in");
    assert_eq!(e.kind, ErrorKind::Auth);
    let e = ask(&p).await.expect_err("not signed in");
    assert!(e.message.contains("quack login chatgpt"), "{}", e.message);
    assert!(
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
    );
}

#[tokio::test]
async fn an_expiring_token_is_renewed_once_and_used() {
    let server = MockServer::start().await;
    let data = tempfile::tempdir().expect("tempdir");
    save(data.path(), &signed_in(0));
    Mock::given(method("POST"))
        .and(path("/api/accounts/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=r1"))
        .and(body_string_contains("client_id=oaiapp_1"))
        .and(body_string_contains(format!(
            "resource={}",
            form_escape(&format!("{}/v1", server.uri()))
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "a2",
            "refresh_token": "r2",
            "expires_in": 3600,
            "scope": PLAN_SCOPES,
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer a2"))
        .and(body_string_contains("\"store\":false"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(SSE),
        )
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer a2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "models": [
                { "slug": "gpt-x", "display_name": "GPT X", "visibility": "list" },
                { "slug": "internal", "visibility": "hide" },
            ]
        })))
        .mount(&server)
        .await;

    let p = provider(&server, data.path());
    assert_eq!(p.not_ready(), None);
    assert_eq!(ask(&p).await.expect("first"), "Hi");
    assert_eq!(ask(&p).await.expect("second"), "Hi");
    let ids: Vec<String> = p
        .list_models()
        .await
        .expect("models")
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(ids, ["gpt-x"]);
    // The rotated refresh token was saved.
    let c = saved(data.path());
    assert_eq!(
        (c.access_token.as_str(), c.refresh_token.as_str()),
        ("a2", "r2")
    );
}

#[tokio::test]
async fn a_revoked_sign_in_asks_for_a_new_one() {
    let server = MockServer::start().await;
    let data = tempfile::tempdir().expect("tempdir");
    save(data.path(), &signed_in(0));
    Mock::given(method("POST"))
        .and(path("/api/accounts/oauth/token"))
        .respond_with(
            ResponseTemplate::new(400).set_body_json(json!({ "error": "refresh_token_reused" })),
        )
        .mount(&server)
        .await;
    let p = provider(&server, data.path());
    let e = ask(&p).await.expect_err("revoked");
    assert_eq!(e.kind, ErrorKind::Auth);
    assert!(e.message.contains("quack login chatgpt"), "{}", e.message);
    let c = saved(data.path());
    assert!(!c.signed_in());
    // The registration stays for the next sign-in.
    assert_eq!(c.client_id, "oaiapp_1");
}

/// What the browser saw, for the token endpoint to answer with.
#[derive(Default)]
struct Seen {
    nonce: String,
    challenge: String,
}

/// The token endpoint: checks the PKCE verifier and returns an ID token signed with `key`.
struct TokenEndpoint {
    key: Arc<ring::signature::RsaKeyPair>,
    issuer: String,
    seen: Arc<Mutex<Seen>>,
    scopes: &'static str,
}

impl Respond for TokenEndpoint {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body = String::from_utf8_lossy(&req.body).into_owned();
        let field = |k: &str| {
            body.split('&')
                .find_map(|p| p.strip_prefix(&format!("{k}=")))
                .unwrap_or_default()
                .to_owned()
        };
        let seen = self.seen.lock().expect("lock");
        let verifier = field("code_verifier");
        let digest = ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes());
        if field("code") != "c1"
            || field("client_id") != "oaiapp_1"
            || b64(digest.as_ref()) != seen.challenge
        {
            return ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"}));
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_secs();
        let claims = json!({
            "iss": self.issuer,
            "aud": "oaiapp_1",
            "sub": "user-1",
            "email": "a@example.com",
            "exp": now + 3600,
            "nonce": seen.nonce,
        });
        ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "a1",
            "refresh_token": "r1",
            "id_token": jwt(&self.key, &claims),
            "expires_in": 3600,
            "scope": self.scopes,
        }))
    }
}

#[tokio::test]
async fn sign_in_registers_checks_the_id_token_and_signs_out() {
    setup_sign_in(true).await;
}

#[tokio::test]
async fn setup_sign_in_requires_consent_to_use_the_plan() {
    setup_sign_in(false).await;
}

async fn setup_sign_in(allow_plan: bool) {
    let server = MockServer::start().await;
    let data = tempfile::tempdir().expect("tempdir");
    let key = Arc::new(ring::signature::RsaKeyPair::from_pkcs8(&KEY_PKCS8_DER).expect("key"));
    let (n, e) = (KEY_N.to_vec(), KEY_E.to_vec());
    Mock::given(method("GET"))
        .and(path("/.well-known/openid-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jwks_uri": format!("{}/jwks", server.uri()),
            "revocation_endpoint": format!("{}/revoke", server.uri()),
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "keys": [{ "kty": "RSA", "kid": "k1", "n": b64(&n), "e": b64(&e) }]
        })))
        .mount(&server)
        .await;
    let seen = Arc::new(Mutex::new(Seen::default()));
    Mock::given(method("POST"))
        .and(path("/api/accounts/oauth/token"))
        .respond_with(TokenEndpoint {
            key,
            issuer: server.uri(),
            seen: seen.clone(),
            scopes: if allow_plan {
                PLAN_SCOPES
            } else {
                "openid profile email offline_access"
            },
        })
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/revoke"))
        .and(body_string_contains("token=r1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let p = provider(&server, data.path());
    let login = p.login().expect("login");
    let browser = seen.clone();
    let done = p
        .sign_in(move |url| {
            // Play the browser: read the request, come back to the callback.
            let q: Vec<(String, String)> = url
                .split_once('?')
                .map(|(_, q)| q)
                .unwrap_or_default()
                .split('&')
                .filter_map(|p| p.split_once('='))
                .map(|(k, v)| (k.to_owned(), unescape(v)))
                .collect();
            let get = |k: &str| {
                q.iter()
                    .find(|(n, _)| n == k)
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default()
            };
            assert_eq!(get("client_id"), "dynamic_agent_client");
            assert_eq!(get("agent_name_hint"), "quack");
            assert_eq!(get("code_challenge_method"), "S256");
            assert!(get("scope").contains("chatgpt.tokens.use.direct"));
            assert!(get("ext_agent_host_id").starts_with("urn:uuid:"));
            {
                let mut s = browser.lock().expect("lock");
                s.nonce = get("nonce");
                s.challenge = get("code_challenge");
            }
            let back = format!(
                "{}?code=c1&state={}&client_id=oaiapp_1",
                get("redirect_uri"),
                get("state")
            );
            tokio::spawn(async move {
                let _ = reqwest::get(back).await;
            });
        })
        .await;
    if allow_plan {
        assert!(done.is_ok(), "{done:?}");
    } else {
        assert!(
            done.expect_err("consent denied")
                .contains("did not allow plan usage")
        );
    }
    assert_eq!(p.subscription_ready(), allow_plan);
    assert_eq!(p.not_ready().is_none(), allow_plan);
    let c = saved(data.path());
    assert_eq!(c.email, "a@example.com");
    assert_eq!(
        (c.client_id.as_str(), c.subject.as_str()),
        ("oaiapp_1", "user-1")
    );
    assert!(c.signed_in());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(data.path().join("auth").join("chatgpt.json"))
            .expect("meta")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    assert_eq!(login.sign_out().await, Ok(true));
    let c = saved(data.path());
    assert!(!c.signed_in());
    assert_eq!(c.client_id, "oaiapp_1");
}

fn form_escape(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s
                .get(i + 1..i + 3)
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).expect("utf8")
}

fn b64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(char::from(A[(n >> (18 - 6 * i)) as usize & 63]));
        }
    }
    out
}

fn jwt(key: &ring::signature::RsaKeyPair, claims: &Value) -> String {
    let head = b64(json!({"alg": "RS256", "kid": "k1"}).to_string().as_bytes());
    let body = b64(claims.to_string().as_bytes());
    let signed = format!("{head}.{body}");
    let mut sig = vec![0; key.public().modulus_len()];
    key.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        signed.as_bytes(),
        &mut sig,
    )
    .expect("sign");
    format!("{signed}.{}", b64(&sig))
}

// A throwaway 2048-bit RSA key standing in for the IdP's signing key: the PKCS#8
// DER that ring loads (ring cannot generate RSA keys), with the modulus and
// exponent that the JWKS reply publishes beside it.
const KEY_N: [u8; 256] = [
    155, 125, 236, 64, 92, 151, 51, 218, 255, 189, 66, 66, 90, 57, 68, 217, 38, 93, 140, 44, 168,
    22, 142, 231, 180, 107, 65, 178, 127, 214, 59, 211, 206, 99, 179, 122, 111, 228, 49, 21, 188,
    202, 193, 157, 228, 28, 250, 201, 201, 157, 236, 204, 160, 146, 40, 223, 15, 42, 216, 78, 20,
    212, 150, 65, 25, 205, 109, 132, 53, 128, 20, 254, 125, 179, 78, 223, 207, 0, 83, 210, 62, 42,
    88, 167, 207, 127, 245, 41, 48, 49, 113, 68, 46, 110, 66, 69, 149, 86, 226, 24, 154, 176, 202,
    28, 126, 175, 185, 250, 107, 154, 38, 161, 215, 210, 2, 31, 234, 128, 212, 129, 103, 35, 242,
    177, 148, 147, 77, 250, 129, 245, 237, 29, 45, 249, 88, 129, 76, 221, 255, 177, 10, 160, 254,
    179, 238, 4, 177, 4, 179, 209, 158, 28, 206, 62, 202, 115, 116, 19, 10, 28, 243, 92, 148, 3,
    135, 41, 184, 240, 242, 246, 128, 204, 126, 11, 239, 198, 233, 38, 166, 235, 102, 187, 112, 80,
    154, 197, 171, 212, 254, 145, 254, 244, 206, 91, 72, 150, 106, 99, 85, 72, 32, 251, 211, 8, 42,
    38, 17, 197, 19, 5, 129, 173, 112, 232, 7, 155, 129, 23, 158, 148, 185, 119, 160, 131, 248, 70,
    27, 126, 28, 65, 158, 127, 9, 80, 2, 223, 222, 204, 138, 144, 186, 17, 101, 70, 210, 223, 154,
    221, 70, 244, 99, 140, 149, 142, 40, 151,
];

const KEY_E: [u8; 3] = [1, 0, 1];

const KEY_PKCS8_DER: [u8; 1218] = [
    48, 130, 4, 190, 2, 1, 0, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 1, 5, 0, 4, 130, 4,
    168, 48, 130, 4, 164, 2, 1, 0, 2, 130, 1, 1, 0, 155, 125, 236, 64, 92, 151, 51, 218, 255, 189,
    66, 66, 90, 57, 68, 217, 38, 93, 140, 44, 168, 22, 142, 231, 180, 107, 65, 178, 127, 214, 59,
    211, 206, 99, 179, 122, 111, 228, 49, 21, 188, 202, 193, 157, 228, 28, 250, 201, 201, 157, 236,
    204, 160, 146, 40, 223, 15, 42, 216, 78, 20, 212, 150, 65, 25, 205, 109, 132, 53, 128, 20, 254,
    125, 179, 78, 223, 207, 0, 83, 210, 62, 42, 88, 167, 207, 127, 245, 41, 48, 49, 113, 68, 46,
    110, 66, 69, 149, 86, 226, 24, 154, 176, 202, 28, 126, 175, 185, 250, 107, 154, 38, 161, 215,
    210, 2, 31, 234, 128, 212, 129, 103, 35, 242, 177, 148, 147, 77, 250, 129, 245, 237, 29, 45,
    249, 88, 129, 76, 221, 255, 177, 10, 160, 254, 179, 238, 4, 177, 4, 179, 209, 158, 28, 206, 62,
    202, 115, 116, 19, 10, 28, 243, 92, 148, 3, 135, 41, 184, 240, 242, 246, 128, 204, 126, 11,
    239, 198, 233, 38, 166, 235, 102, 187, 112, 80, 154, 197, 171, 212, 254, 145, 254, 244, 206,
    91, 72, 150, 106, 99, 85, 72, 32, 251, 211, 8, 42, 38, 17, 197, 19, 5, 129, 173, 112, 232, 7,
    155, 129, 23, 158, 148, 185, 119, 160, 131, 248, 70, 27, 126, 28, 65, 158, 127, 9, 80, 2, 223,
    222, 204, 138, 144, 186, 17, 101, 70, 210, 223, 154, 221, 70, 244, 99, 140, 149, 142, 40, 151,
    2, 3, 1, 0, 1, 2, 130, 1, 0, 11, 121, 230, 198, 245, 78, 127, 152, 73, 73, 201, 171, 38, 7,
    150, 10, 68, 231, 160, 161, 231, 34, 11, 24, 20, 219, 217, 180, 119, 244, 242, 227, 118, 23,
    43, 182, 252, 173, 37, 230, 73, 209, 217, 42, 115, 28, 164, 113, 226, 60, 146, 174, 163, 167,
    69, 246, 248, 162, 155, 230, 95, 105, 53, 81, 198, 186, 191, 187, 131, 75, 89, 193, 82, 187,
    186, 53, 144, 140, 160, 163, 34, 205, 115, 79, 167, 99, 245, 79, 61, 236, 88, 68, 196, 252,
    132, 77, 168, 113, 10, 44, 240, 181, 221, 164, 238, 192, 191, 132, 57, 14, 222, 141, 17, 101,
    193, 156, 99, 109, 112, 193, 192, 64, 9, 248, 139, 83, 183, 236, 136, 188, 183, 169, 133, 10,
    131, 68, 7, 234, 11, 246, 220, 129, 194, 229, 223, 226, 221, 170, 53, 33, 28, 194, 233, 51, 76,
    251, 120, 23, 76, 94, 251, 208, 169, 199, 63, 124, 71, 99, 202, 13, 152, 193, 40, 5, 89, 10,
    85, 134, 81, 12, 72, 192, 224, 12, 170, 13, 128, 186, 18, 151, 94, 9, 157, 121, 225, 126, 239,
    151, 28, 27, 223, 131, 170, 152, 224, 94, 211, 224, 146, 94, 97, 189, 93, 42, 168, 4, 248, 193,
    140, 237, 144, 151, 97, 149, 45, 155, 21, 106, 42, 147, 50, 29, 107, 159, 62, 154, 31, 106, 71,
    102, 103, 218, 85, 38, 198, 179, 248, 95, 23, 168, 161, 170, 167, 156, 105, 161, 2, 129, 129,
    0, 217, 190, 152, 234, 189, 79, 122, 177, 97, 135, 238, 154, 177, 105, 9, 52, 53, 101, 65, 96,
    9, 5, 16, 200, 161, 246, 19, 141, 169, 71, 174, 252, 24, 251, 19, 68, 114, 202, 117, 167, 76,
    185, 14, 69, 3, 63, 193, 43, 45, 52, 250, 249, 28, 231, 117, 144, 130, 134, 61, 38, 103, 119,
    207, 194, 11, 99, 213, 201, 127, 168, 62, 117, 97, 214, 191, 186, 41, 212, 19, 200, 217, 196,
    126, 99, 89, 167, 83, 238, 250, 2, 218, 228, 178, 54, 165, 102, 203, 212, 205, 71, 1, 140, 142,
    62, 125, 131, 88, 94, 131, 148, 18, 30, 209, 244, 21, 242, 111, 127, 81, 145, 86, 164, 73, 192,
    45, 77, 157, 71, 2, 129, 129, 0, 182, 207, 106, 78, 170, 62, 159, 152, 154, 77, 168, 117, 110,
    50, 110, 237, 209, 63, 48, 73, 165, 125, 84, 29, 128, 203, 110, 48, 187, 8, 239, 199, 224, 43,
    29, 254, 11, 251, 102, 54, 255, 7, 67, 17, 119, 206, 157, 7, 9, 113, 252, 65, 94, 144, 252,
    133, 114, 176, 58, 92, 124, 117, 33, 58, 23, 77, 170, 84, 48, 137, 118, 9, 112, 65, 233, 240,
    245, 205, 197, 126, 36, 116, 237, 73, 50, 48, 208, 124, 42, 111, 31, 238, 48, 43, 1, 246, 243,
    150, 63, 16, 187, 230, 22, 186, 182, 53, 222, 171, 151, 1, 71, 115, 231, 139, 209, 92, 139,
    224, 160, 91, 5, 86, 192, 141, 24, 3, 130, 49, 2, 129, 128, 95, 124, 218, 10, 119, 34, 151,
    167, 154, 33, 198, 93, 72, 87, 203, 149, 125, 164, 253, 218, 99, 154, 57, 22, 3, 16, 170, 218,
    128, 213, 210, 194, 243, 33, 135, 253, 136, 105, 136, 214, 233, 42, 110, 59, 141, 133, 51, 236,
    255, 82, 48, 141, 118, 67, 122, 166, 248, 214, 109, 146, 157, 229, 72, 210, 203, 179, 232, 89,
    9, 6, 122, 253, 118, 254, 121, 237, 229, 209, 255, 43, 33, 116, 174, 8, 91, 148, 169, 150, 47,
    69, 121, 64, 185, 118, 133, 106, 99, 16, 219, 68, 188, 69, 43, 75, 244, 54, 217, 85, 249, 85,
    193, 236, 138, 30, 4, 81, 224, 219, 112, 165, 207, 221, 20, 10, 150, 211, 102, 153, 2, 129,
    129, 0, 128, 126, 160, 25, 116, 254, 153, 141, 89, 0, 54, 229, 123, 180, 133, 86, 49, 218, 249,
    100, 250, 166, 114, 38, 94, 174, 30, 224, 175, 198, 163, 75, 217, 156, 84, 243, 137, 150, 134,
    141, 39, 139, 2, 48, 88, 6, 233, 70, 213, 243, 193, 188, 108, 236, 250, 34, 14, 176, 34, 5,
    122, 17, 150, 110, 79, 140, 238, 169, 72, 136, 134, 104, 49, 248, 146, 168, 37, 14, 34, 191,
    252, 126, 23, 127, 120, 128, 233, 58, 45, 85, 120, 155, 34, 78, 72, 142, 61, 63, 33, 74, 240,
    215, 212, 168, 30, 133, 20, 227, 133, 59, 135, 238, 199, 20, 246, 238, 214, 254, 231, 151, 178,
    211, 252, 121, 240, 36, 31, 49, 2, 129, 129, 0, 217, 47, 118, 98, 49, 107, 7, 39, 187, 109, 42,
    37, 108, 92, 98, 230, 204, 186, 225, 33, 77, 93, 128, 38, 253, 156, 177, 36, 190, 110, 72, 30,
    140, 146, 66, 151, 6, 224, 251, 215, 158, 118, 43, 51, 52, 240, 10, 179, 182, 138, 157, 188,
    143, 177, 95, 223, 116, 202, 115, 129, 180, 35, 206, 227, 29, 104, 246, 96, 129, 206, 210, 230,
    222, 203, 108, 202, 192, 233, 175, 35, 151, 154, 138, 167, 250, 47, 170, 131, 189, 175, 29,
    137, 101, 134, 180, 252, 231, 104, 28, 11, 64, 151, 168, 149, 128, 193, 221, 35, 230, 36, 218,
    92, 27, 19, 2, 135, 182, 157, 171, 207, 135, 229, 168, 120, 210, 11, 114, 96,
];
