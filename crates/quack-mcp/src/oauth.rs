//! OAuth 2.1 + PKCE client for remote streamable-HTTP MCP servers
//! (RFC 9728 Protected Resource Metadata, RFC 8414 Authorization Server Metadata,
//! RFC 7591 Dynamic Client Registration, RFC 8707 Resource Indicators).
//!
//! Credentials live in `<data_dir>/mcp-auth/<server_id>.json` with `0600` permissions on
//! Unix. Every outbound discovery, registration, token, and revocation call goes through
//! `NetworkSettings::check_egress`, `[policy.mcp]` host checks, and the session's
//! `EgressLedger`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use quack_config::settings::{AllowList, NetworkSettings};
use quack_config::{McpOAuth, McpServerFile, is_loopback};
use quack_protocol::EgressKind;
use quack_providers::reqwest::{self, StatusCode};
use quack_providers::{EgressDraft, EgressLedger};
use ring::rand::SecureRandom;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CALLBACK_PATH: &str = "/oauth/callback";
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);
const EARLY_REFRESH_SECS: u64 = 60;
const STALE_LOCK: Duration = Duration::from_secs(60);

/// Terminal OAuth refresh error codes that require a fresh browser sign-in.
const TERMINAL_REFRESH: &[&str] = &[
    "invalid_grant",
    "invalid_token",
    "invalid_client",
    "unauthorized_client",
    "expired_token",
];

/// Saved OAuth registration and tokens for one remote MCP server.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct McpCredentials {
    pub server_url: String,
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub revocation_endpoint: Option<String>,
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    /// Unix epoch seconds, or `0` when the server returned no expiry.
    #[serde(default)]
    pub expires_at: u64,
    #[serde(default)]
    pub scopes: Vec<String>,
}

impl std::fmt::Debug for McpCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpCredentials")
            .field("server_url", &self.server_url)
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .field("expires_at", &self.expires_at)
            .field("scopes", &self.scopes)
            .field("signed_in", &self.has_tokens())
            .finish_non_exhaustive()
    }
}

impl McpCredentials {
    pub fn has_tokens(&self) -> bool {
        !self.access_token.is_empty() || !self.refresh_token.is_empty()
    }
}

/// Summary returned after a successful interactive OAuth sign-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpSignedIn {
    pub server: String,
    pub issuer: String,
    pub scopes: Vec<String>,
    pub expires_at: u64,
}

#[derive(Debug, Clone)]
struct DiscoveredAuth {
    resource: String,
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: Option<String>,
    revocation_endpoint: Option<String>,
    scopes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ProtectedResourceMetadata {
    #[serde(default)]
    resource: Option<String>,
    #[serde(default)]
    authorization_servers: Vec<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AuthServerMetadata {
    #[serde(default)]
    issuer: Option<String>,
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    registration_endpoint: Option<String>,
    #[serde(default)]
    revocation_endpoint: Option<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
    #[serde(default)]
    code_challenge_methods_supported: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RegistrationResponse {
    client_id: String,
    #[serde(default)]
    client_secret: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    scope: Option<String>,
}

/// OAuth 2.1 client bound to a single configured remote MCP server.
pub struct McpOAuthClient {
    server_id: String,
    server_url: String,
    oauth: McpOAuth,
    dir: PathBuf,
    network: NetworkSettings,
    policy: AllowList,
    ledger: Option<Arc<EgressLedger>>,
}

impl std::fmt::Debug for McpOAuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpOAuthClient")
            .field("server_id", &self.server_id)
            .field("server_url", &self.server_url)
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

/// Fast local check for whether a server has saved OAuth credentials (no network calls).
pub fn has_saved_token(data_dir: &Path, server_id: &str) -> bool {
    let path = credentials_path(data_dir, server_id);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<McpCredentials>(&s).ok())
        .is_some_and(|c| c.has_tokens())
}

fn credentials_path(data_dir: &Path, server_id: &str) -> PathBuf {
    data_dir.join("mcp-auth").join(format!("{server_id}.json"))
}

impl McpOAuthClient {
    pub fn from_server(
        server_id: impl Into<String>,
        file: &McpServerFile,
        data_dir: &Path,
        network: NetworkSettings,
        policy: AllowList,
        ledger: Option<Arc<EgressLedger>>,
    ) -> Result<Self, String> {
        let server_id = server_id.into();
        if file.transport()? != "http" {
            return Err(format!(
                "MCP server `{server_id}` uses stdio; OAuth only applies to remote HTTP servers"
            ));
        }
        let oauth = file.oauth.clone().unwrap_or_default();
        if !oauth.enabled {
            return Err(format!("OAuth is disabled for MCP server `{server_id}`"));
        }
        Ok(Self {
            server_id,
            server_url: file.url.trim_end_matches('/').to_owned(),
            oauth,
            dir: data_dir.join("mcp-auth"),
            network,
            policy,
            ledger,
        })
    }

    fn file(&self) -> PathBuf {
        self.dir.join(format!("{}.json", self.server_id))
    }

    fn lock_file(&self) -> PathBuf {
        self.dir.join(format!("{}.lock", self.server_id))
    }

    /// Saved credentials on disk, if any.
    pub fn load(&self) -> Option<McpCredentials> {
        let text = std::fs::read_to_string(self.file()).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn save(&self, creds: &McpCredentials) -> Result<(), String> {
        let text = serde_json::to_string_pretty(creds).map_err(|e| e.to_string())?;
        write_private(&self.file(), &text)
    }

    fn build_client(&self) -> Result<reqwest::Client, String> {
        let env = |k: &str| std::env::var_os(k);
        quack_providers::http_client_observed(&self.network, &env, self.ledger.clone())
    }

    fn check_url(&self, url: &str) -> Result<(), String> {
        let valid_scheme =
            url.starts_with("https://") || (url.starts_with("http://") && is_loopback(url));
        if !valid_scheme {
            let msg = format!(
                "refusing non-HTTPS OAuth endpoint `{url}` (only `https://` or loopback `http://` is allowed)"
            );
            self.record_blocked(url, &msg);
            return Err(msg);
        }
        if let Err(why) = self.network.check_egress(url) {
            self.record_blocked(url, &why);
            return Err(why);
        }
        if !self.policy.allows(&self.server_id, Some(url)) {
            let msg = format!(
                "OAuth endpoint `{url}` is blocked by `[policy.mcp]` for `{}`",
                self.server_id
            );
            self.record_blocked(url, &msg);
            return Err(msg);
        }
        Ok(())
    }

    fn record_blocked(&self, url: &str, reason: &str) {
        if let Some(ledger) = &self.ledger {
            ledger.record(
                EgressDraft::new(EgressKind::BlockedEgress, url, "mcp-oauth")
                    .blocked(reason.to_owned()),
            );
        }
    }

    fn record_http(&self, url: &str, status: Option<u16>, error: Option<String>) {
        if let Some(ledger) = &self.ledger {
            let mut draft = EgressDraft::new(EgressKind::McpHttp, url, "mcp-oauth").status(status);
            if let Some(err) = error {
                draft = draft.blocked(err);
            }
            ledger.record(draft);
        }
    }

    /// Return a valid access token for the next MCP HTTP request, refreshing it when
    /// it is about to expire.
    pub async fn bearer(&self) -> Result<String, String> {
        let not_signed_in = || {
            format!(
                "not signed in to MCP server `{}` · run `quack mcp login {}` or `/mcp login {}`",
                self.server_id, self.server_id, self.server_id
            )
        };
        let creds = self
            .load()
            .filter(|c| c.server_url == self.server_url && c.has_tokens())
            .ok_or_else(not_signed_in)?;

        if token_fresh(&creds) {
            return Ok(creds.access_token);
        }
        if creds.refresh_token.is_empty() {
            return Err(format!(
                "OAuth token for MCP server `{}` has expired · run `quack mcp login {}`",
                self.server_id, self.server_id
            ));
        }

        let _lock = FileLock::acquire(self.lock_file()).await?;
        let creds = self
            .load()
            .filter(|c| c.server_url == self.server_url && c.has_tokens())
            .ok_or_else(not_signed_in)?;
        if token_fresh(&creds) {
            return Ok(creds.access_token);
        }

        let refreshed = self.refresh(creds).await?;
        self.save(&refreshed)
            .map_err(|e| format!("cannot save refreshed MCP OAuth token: {e}"))?;
        Ok(refreshed.access_token)
    }

    async fn refresh(&self, mut creds: McpCredentials) -> Result<McpCredentials, String> {
        self.check_url(&creds.token_endpoint)?;
        let client = self.build_client()?;
        let mut form_pairs: Vec<(&str, &str)> = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", &creds.refresh_token),
            ("client_id", &creds.client_id),
            ("resource", &self.server_url),
        ];
        let secret = self.resolve_client_secret(&creds)?;
        if let Some(sec) = secret.as_deref() {
            form_pairs.push(("client_secret", sec));
        }
        let resp = client
            .post(&creds.token_endpoint)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .header(reqwest::header::ACCEPT, "application/json")
            .body(form_encode(&form_pairs))
            .send()
            .await
            .map_err(|e| {
                let msg = format!("cannot reach {}: {e}", creds.token_endpoint);
                self.record_http(&creds.token_endpoint, None, Some(msg.clone()));
                msg
            })?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            let code = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v["error"].as_str().map(str::to_owned))
                .unwrap_or_default();
            self.record_http(
                &creds.token_endpoint,
                Some(status.as_u16()),
                Some(format!("token refresh failed ({status})")),
            );
            if TERMINAL_REFRESH.contains(&code.as_str()) {
                creds.access_token.clear();
                creds.refresh_token.clear();
                creds.expires_at = 0;
                let _ = self.save(&creds);
                return Err(format!(
                    "OAuth sign-in for `{}` expired or was revoked ({code}) · run `quack mcp login {}`",
                    self.server_id, self.server_id
                ));
            }
            return Err(format!(
                "refreshing OAuth token for `{}` failed ({status}): {}",
                self.server_id,
                error_detail(&body)
            ));
        }
        self.record_http(&creds.token_endpoint, Some(status.as_u16()), None);
        let tok: TokenResponse = serde_json::from_str(&body)
            .map_err(|e| format!("unexpected OAuth token response: {e}"))?;
        creds.access_token = tok.access_token;
        if let Some(r) = tok.refresh_token.filter(|r| !r.is_empty()) {
            creds.refresh_token = r;
        }
        creds.expires_at = tok.expires_in.map_or(0, |secs| now_secs() + secs);
        if let Some(scope) = tok.scope {
            creds.scopes = scope.split_whitespace().map(str::to_owned).collect();
        }
        Ok(creds)
    }

    /// Run interactive OAuth 2.1 + PKCE sign-in in the browser.
    /// `open` receives the authorization URL to open or display to the user.
    pub async fn sign_in(&self, open: impl FnOnce(&str)) -> Result<McpSignedIn, String> {
        self.check_url(&self.server_url)?;
        let client = self.build_client()?;
        let discovered = self.discover(&client).await?;

        let listener = match self.oauth.redirect_port {
            Some(port) => match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                Ok(l) => l,
                Err(_) => tokio::net::TcpListener::bind(("127.0.0.1", 0))
                    .await
                    .map_err(|e| format!("cannot listen on 127.0.0.1: {e}"))?,
            },
            None => tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .map_err(|e| format!("cannot listen on 127.0.0.1: {e}"))?,
        };
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");

        let (client_id, client_secret) = self
            .resolve_or_register_client(&client, &discovered, &redirect_uri)
            .await?;

        let state = random_token()?;
        let verifier = random_token()?;
        let challenge =
            b64url(ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes()).as_ref());

        let scope_str = discovered.scopes.join(" ");
        let mut params = vec![
            ("response_type", "code"),
            ("client_id", client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
            ("resource", discovered.resource.as_str()),
        ];
        if !scope_str.is_empty() {
            params.push(("scope", scope_str.as_str()));
        }
        let sep = if discovered.authorization_endpoint.contains('?') {
            '&'
        } else {
            '?'
        };
        let auth_url = format!(
            "{}{sep}{}",
            discovered.authorization_endpoint,
            form_encode(&params)
        );
        open(&auth_url);

        let query = tokio::time::timeout(SIGN_IN_TIMEOUT, callback(&listener, &state))
            .await
            .map_err(|_| "MCP OAuth sign-in timed out after 5 minutes".to_owned())??;
        let get = |k: &str| query.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        if let Some(err) = get("error") {
            let desc = get("error_description").unwrap_or_default();
            return Err(match err {
                "access_denied" => "MCP OAuth sign-in was declined in the browser".to_owned(),
                other if desc.is_empty() => format!("MCP OAuth sign-in failed: {other}"),
                other => format!("MCP OAuth sign-in failed ({other}): {desc}"),
            });
        }
        let code = get("code").ok_or("MCP OAuth callback returned no authorization code")?;

        self.check_url(&discovered.token_endpoint)?;
        let mut token_form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri.as_str()),
            ("client_id", client_id.as_str()),
            ("code_verifier", verifier.as_str()),
            ("resource", discovered.resource.as_str()),
        ];
        if let Some(sec) = client_secret.as_deref() {
            token_form.push(("client_secret", sec));
        }

        let resp = client
            .post(&discovered.token_endpoint)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .header(reqwest::header::ACCEPT, "application/json")
            .body(form_encode(&token_form))
            .send()
            .await
            .map_err(|e| {
                let msg = format!("cannot reach {}: {e}", discovered.token_endpoint);
                self.record_http(&discovered.token_endpoint, None, Some(msg.clone()));
                msg
            })?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            self.record_http(
                &discovered.token_endpoint,
                Some(status.as_u16()),
                Some(format!("token exchange failed ({status})")),
            );
            return Err(format!(
                "MCP OAuth token exchange failed ({status}): {}",
                error_detail(&body)
            ));
        }
        self.record_http(&discovered.token_endpoint, Some(status.as_u16()), None);
        let tok: TokenResponse = serde_json::from_str(&body)
            .map_err(|e| format!("unexpected OAuth token response: {e}"))?;
        let scopes: Vec<String> = tok.scope.map_or_else(
            || discovered.scopes.clone(),
            |s| s.split_whitespace().map(str::to_owned).collect(),
        );
        let expires_at = tok.expires_in.map_or(0, |secs| now_secs() + secs);
        let creds = McpCredentials {
            server_url: self.server_url.clone(),
            issuer: discovered.issuer.clone(),
            authorization_endpoint: discovered.authorization_endpoint,
            token_endpoint: discovered.token_endpoint,
            revocation_endpoint: discovered.revocation_endpoint,
            client_id,
            // Only persist dynamically issued client secrets; config-provided secrets stay
            // in the config's `Secret` source (env/keyring/cmd).
            client_secret: if self.oauth.client_secret.is_some() {
                None
            } else {
                client_secret
            },
            access_token: tok.access_token,
            refresh_token: tok.refresh_token.unwrap_or_default(),
            expires_at,
            scopes: scopes.clone(),
        };
        self.save(&creds)
            .map_err(|e| format!("cannot save MCP OAuth credentials: {e}"))?;
        Ok(McpSignedIn {
            server: self.server_id.clone(),
            issuer: discovered.issuer,
            scopes,
            expires_at,
        })
    }

    /// Revoke saved tokens when a revocation endpoint is known, then remove the local file.
    /// Returns `Ok(true)` if revoked (or not signed in), `Ok(false)` if local credentials were
    /// removed without server revocation confirmation.
    pub async fn sign_out(&self) -> Result<bool, String> {
        let Some(creds) = self.load() else {
            return Ok(true);
        };
        let mut revoked = true;
        if let Some(url) = creds.revocation_endpoint.as_deref()
            && !url.is_empty()
            && self.check_url(url).is_ok()
            && let Ok(client) = self.build_client()
        {
            let token = if !creds.refresh_token.is_empty() {
                (&creds.refresh_token, "refresh_token")
            } else {
                (&creds.access_token, "access_token")
            };
            if !token.0.is_empty() {
                let mut pairs = vec![
                    ("token", token.0.as_str()),
                    ("token_type_hint", token.1),
                    ("client_id", creds.client_id.as_str()),
                ];
                let secret = self.resolve_client_secret(&creds).ok().flatten();
                if let Some(sec) = secret.as_deref() {
                    pairs.push(("client_secret", sec));
                }
                match client
                    .post(url)
                    .header(
                        reqwest::header::CONTENT_TYPE,
                        "application/x-www-form-urlencoded",
                    )
                    .body(form_encode(&pairs))
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        self.record_http(url, Some(resp.status().as_u16()), None);
                    }
                    Ok(resp) => {
                        self.record_http(
                            url,
                            Some(resp.status().as_u16()),
                            Some("revocation failed".into()),
                        );
                        revoked = false;
                    }
                    Err(e) => {
                        self.record_http(url, None, Some(e.to_string()));
                        revoked = false;
                    }
                }
            }
        }
        match std::fs::remove_file(self.file()) {
            Ok(()) => Ok(revoked),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(revoked),
            Err(e) => Err(format!("cannot remove MCP OAuth credentials: {e}")),
        }
    }

    async fn discover(&self, client: &reqwest::Client) -> Result<DiscoveredAuth, String> {
        let mut resource = self.server_url.clone();
        let mut challenge_scopes: Vec<String> = Vec::new();
        let mut prm_scopes: Vec<String> = Vec::new();

        let issuer = if let Some(explicit) = self
            .oauth
            .authorization_server
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            explicit.trim_end_matches('/').to_owned()
        } else {
            let (prm, header_scopes) = self.fetch_protected_resource_metadata(client).await?;
            challenge_scopes = header_scopes;
            if let Some(meta) = prm {
                if let Some(r) = meta.resource.filter(|s| !s.is_empty()) {
                    resource = r;
                }
                prm_scopes = meta.scopes_supported;
                if let Some(first) = meta
                    .authorization_servers
                    .into_iter()
                    .find(|s| !s.is_empty())
                {
                    first.trim_end_matches('/').to_owned()
                } else {
                    split_origin_and_path(&self.server_url)?.0
                }
            } else {
                // Fallback for pre-RFC-9728 MCP servers: authorization server at the same origin.
                split_origin_and_path(&self.server_url)?.0
            }
        };

        self.check_url(&issuer)?;
        let as_meta = self.fetch_auth_server_metadata(client, &issuer).await?;

        if !as_meta.code_challenge_methods_supported.is_empty()
            && !as_meta
                .code_challenge_methods_supported
                .iter()
                .any(|m| m.eq_ignore_ascii_case("S256"))
        {
            return Err(format!(
                "authorization server `{issuer}` does not advertise PKCE S256 support"
            ));
        }
        self.check_url(&as_meta.authorization_endpoint)?;
        self.check_url(&as_meta.token_endpoint)?;
        if let Some(reg) = &as_meta.registration_endpoint {
            self.check_url(reg)?;
        }
        if let Some(rev) = &as_meta.revocation_endpoint {
            self.check_url(rev)?;
        }

        let scopes = if !self.oauth.scopes.is_empty() {
            self.oauth.scopes.clone()
        } else if !challenge_scopes.is_empty() {
            challenge_scopes
        } else if !prm_scopes.is_empty() {
            prm_scopes
        } else {
            as_meta.scopes_supported
        };

        Ok(DiscoveredAuth {
            resource,
            issuer: as_meta
                .issuer
                .filter(|s| !s.is_empty())
                .unwrap_or(issuer)
                .trim_end_matches('/')
                .to_owned(),
            authorization_endpoint: as_meta.authorization_endpoint,
            token_endpoint: as_meta.token_endpoint,
            registration_endpoint: as_meta.registration_endpoint,
            revocation_endpoint: as_meta.revocation_endpoint,
            scopes,
        })
    }

    async fn fetch_protected_resource_metadata(
        &self,
        client: &reqwest::Client,
    ) -> Result<(Option<ProtectedResourceMetadata>, Vec<String>), String> {
        let mut challenge_scopes = Vec::new();
        let (origin, path) = split_origin_and_path(&self.server_url)?;
        let mut candidates = Vec::new();
        if !path.is_empty() && path != "/" {
            candidates.push(format!(
                "{origin}/.well-known/oauth-protected-resource{path}"
            ));
        }
        candidates.push(format!("{origin}/.well-known/oauth-protected-resource"));

        for url in &candidates {
            if let Some(meta) = self
                .try_get_json::<ProtectedResourceMetadata>(client, url)
                .await?
            {
                return Ok((Some(meta), challenge_scopes));
            }
        }

        // Probe the MCP endpoint itself for a 401 WWW-Authenticate header pointing at
        // `resource_metadata="..."` (RFC 9728 §5.1).
        self.check_url(&self.server_url)?;
        if let Ok(resp) = client
            .get(&self.server_url)
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            )
            .send()
            .await
        {
            let status = resp.status();
            self.record_http(&self.server_url, Some(status.as_u16()), None);
            if status == StatusCode::UNAUTHORIZED
                && let Some(raw) = resp
                    .headers()
                    .get(reqwest::header::WWW_AUTHENTICATE)
                    .and_then(|h| h.to_str().ok())
            {
                let params = parse_www_authenticate(raw);
                if let Some(scope) = params.get("scope") {
                    challenge_scopes = scope.split_whitespace().map(str::to_owned).collect();
                }
                if let Some(meta_url) = params.get("resource_metadata")
                    && let Some(meta) = self
                        .try_get_json::<ProtectedResourceMetadata>(client, meta_url)
                        .await?
                {
                    return Ok((Some(meta), challenge_scopes));
                }
            }
        }
        Ok((None, challenge_scopes))
    }

    async fn fetch_auth_server_metadata(
        &self,
        client: &reqwest::Client,
        issuer: &str,
    ) -> Result<AuthServerMetadata, String> {
        let (origin, path) = split_origin_and_path(issuer)?;
        let mut candidates = Vec::new();
        if !path.is_empty() && path != "/" {
            candidates.push(format!(
                "{origin}/.well-known/oauth-authorization-server{path}"
            ));
            candidates.push(format!("{origin}/.well-known/openid-configuration{path}"));
            candidates.push(format!("{issuer}/.well-known/openid-configuration"));
        } else {
            candidates.push(format!("{origin}/.well-known/oauth-authorization-server"));
            candidates.push(format!("{origin}/.well-known/openid-configuration"));
        }
        for url in &candidates {
            if let Some(meta) = self.try_get_json::<AuthServerMetadata>(client, url).await? {
                return Ok(meta);
            }
        }
        Err(format!(
            "could not discover OAuth metadata for `{issuer}` (tried {})",
            candidates.join(", ")
        ))
    }

    async fn try_get_json<T: for<'de> Deserialize<'de>>(
        &self,
        client: &reqwest::Client,
        url: &str,
    ) -> Result<Option<T>, String> {
        self.check_url(url)?;
        let resp = match client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                self.record_http(url, None, Some(e.to_string()));
                return Err(format!("cannot reach {url}: {e}"));
            }
        };
        let status = resp.status();
        self.record_http(url, Some(status.as_u16()), None);
        if status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED {
            return Ok(None);
        }
        if !status.is_success() {
            return Ok(None);
        }
        let body = resp.text().await.unwrap_or_default();
        match serde_json::from_str::<T>(&body) {
            Ok(parsed) => Ok(Some(parsed)),
            Err(_) => Ok(None),
        }
    }

    async fn resolve_or_register_client(
        &self,
        client: &reqwest::Client,
        discovered: &DiscoveredAuth,
        redirect_uri: &str,
    ) -> Result<(String, Option<String>), String> {
        if let Some(id) = self
            .oauth
            .client_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let secret = self.resolve_config_secret()?;
            return Ok((id.to_owned(), secret));
        }

        if let Some(reg_url) = discovered.registration_endpoint.as_deref() {
            self.check_url(reg_url)?;
            let payload = json!({
                "client_name": "quack",
                "redirect_uris": [redirect_uri],
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none",
            });
            let resp = client
                .post(reg_url)
                .header(reqwest::header::ACCEPT, "application/json")
                .json(&payload)
                .send()
                .await
                .map_err(|e| {
                    let msg = format!("OAuth dynamic client registration failed at {reg_url}: {e}");
                    self.record_http(reg_url, None, Some(msg.clone()));
                    msg
                })?;
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                self.record_http(
                    reg_url,
                    Some(status.as_u16()),
                    Some(format!("DCR failed ({status})")),
                );
                return Err(format!(
                    "OAuth dynamic client registration failed ({status}): {} · set `oauth.client_id` in mcp.d/{}.toml if the server requires a pre-registered client",
                    error_detail(&body),
                    self.server_id
                ));
            }
            self.record_http(reg_url, Some(status.as_u16()), None);
            let reg: RegistrationResponse = serde_json::from_str(&body)
                .map_err(|e| format!("unexpected dynamic client registration response: {e}"))?;
            return Ok((reg.client_id, reg.client_secret));
        }

        if let Some(saved) = self
            .load()
            .filter(|c| c.issuer == discovered.issuer && !c.client_id.is_empty())
        {
            return Ok((saved.client_id, saved.client_secret));
        }

        Err(format!(
            "authorization server `{}` does not advertise `registration_endpoint` · set `oauth.client_id` in mcp.d/{}.toml",
            discovered.issuer, self.server_id
        ))
    }

    fn resolve_config_secret(&self) -> Result<Option<String>, String> {
        let Some(secret) = &self.oauth.client_secret else {
            return Ok(None);
        };
        let env = |k: &str| std::env::var_os(k);
        secret
            .resolve(&env)
            .map(Some)
            .map_err(|e| format!("oauth.client_secret: {e}"))
    }

    fn resolve_client_secret(&self, creds: &McpCredentials) -> Result<Option<String>, String> {
        if let Some(sec) = self.resolve_config_secret()? {
            return Ok(Some(sec));
        }
        Ok(creds.client_secret.clone().filter(|s| !s.is_empty()))
    }
}

fn token_fresh(creds: &McpCredentials) -> bool {
    !creds.access_token.is_empty()
        && (creds.expires_at == 0 || creds.expires_at > now_secs() + EARLY_REFRESH_SECS)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn split_origin_and_path(url: &str) -> Result<(String, String), String> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| format!("invalid URL `{url}`"))?;
    let without_frag = rest.split(['?', '#']).next().unwrap_or(rest);
    let (authority, path) = match without_frag.find('/') {
        Some(idx) => (&without_frag[..idx], &without_frag[idx..]),
        None => (without_frag, ""),
    };
    if authority.is_empty() {
        return Err(format!("invalid URL `{url}`: missing host"));
    }
    let clean_path = path.trim_end_matches('/');
    Ok((format!("{scheme}://{authority}"), clean_path.to_owned()))
}

/// Parse `Bearer key="value", key2=value2` parameters from a `WWW-Authenticate` header.
pub(crate) fn parse_www_authenticate(header: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let trimmed = header.trim();
    let rest = if trimmed.len() >= 7 && trimmed[..7].eq_ignore_ascii_case("bearer ") {
        &trimmed[7..]
    } else {
        trimmed
    };
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b',') {
            i += 1;
        }
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' && bytes[i] != b',' {
            i += 1;
        }
        let key = rest[key_start..i].trim().to_ascii_lowercase();
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let value = if i < bytes.len() && bytes[i] == b'"' {
            i += 1;
            let mut val = String::new();
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                }
                val.push(char::from(bytes[i]));
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'"' {
                i += 1;
            }
            val
        } else {
            let val_start = i;
            while i < bytes.len() && bytes[i] != b',' {
                i += 1;
            }
            rest[val_start..i].trim().to_owned()
        };
        if !key.is_empty() {
            out.insert(key, value);
        }
    }
    out
}

fn error_detail(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        if let Some(desc) = v["error_description"].as_str().filter(|s| !s.is_empty()) {
            return desc.to_owned();
        }
        if let Some(err) = v["error"].as_str().filter(|s| !s.is_empty()) {
            return err.to_owned();
        }
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        "empty response".to_owned()
    } else {
        trimmed.chars().take(200).collect()
    }
}

async fn callback(
    listener: &tokio::net::TcpListener,
    state: &str,
) -> Result<Vec<(String, String)>, String> {
    loop {
        let (mut stream, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 2048];
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 * 1024 {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        let head = String::from_utf8_lossy(&buf);
        let target = head
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("GET "))
            .and_then(|l| l.split_whitespace().next())
            .unwrap_or_default();
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let pairs = parse_query(query);
        let ours = path == CALLBACK_PATH && pairs.iter().any(|(k, v)| k == "state" && v == state);
        let (status, text) = if !ours {
            ("404 Not Found", "Not found.")
        } else if pairs.iter().any(|(k, _)| k == "error") {
            (
                "200 OK",
                "MCP sign-in did not finish. You can close this tab.",
            )
        } else {
            (
                "200 OK",
                "MCP sign-in succeeded. You can close this tab and return to quack.",
            )
        };
        let page = format!("<!doctype html><meta charset=utf-8><title>quack</title><p>{text}</p>");
        let reply = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{page}",
            page.len()
        );
        let _ = stream.write_all(reply.as_bytes()).await;
        let _ = stream.shutdown().await;
        if ours {
            return Ok(pairs);
        }
    }
}

struct FileLock(PathBuf);

impl FileLock {
    async fn acquire(path: PathBuf) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        for _ in 0..200 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(_) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > STALE_LOCK);
                    if stale {
                        let _ = std::fs::remove_file(&path);
                    } else {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
                Err(e) => return Err(format!("cannot lock {}: {e}", path.display())),
            }
        }
        Err(format!(
            "another quack process is refreshing MCP OAuth credentials; remove {} if none is",
            path.display()
        ))
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn write_private(path: &Path, text: &str) -> Result<(), String> {
    let dir = path.parent().ok_or("no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts.open(&tmp).map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut f, text.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())?;
    drop(f);
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn random_token() -> Result<String, String> {
    let mut b = [0u8; 32];
    ring::rand::SystemRandom::new()
        .fill(&mut b)
        .map_err(|_| "no random source")?;
    Ok(b64url(&b))
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn b64url(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(char::from(B64[(n >> (18 - 6 * i)) as usize & 63]));
        }
    }
    out
}

fn form_encode(pairs: &[(&str, &str)]) -> String {
    let enc = |s: &str| {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                    char::from(b).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect::<String>()
    };
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn parse_query(query: &str) -> Vec<(String, String)> {
    let dec = |s: &str| {
        let s = s.replace('+', " ");
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
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
        String::from_utf8_lossy(&out).into_owned()
    };
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (dec(k), dec(v))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn parses_www_authenticate_header() {
        let map = parse_www_authenticate(
            r#"Bearer realm="mcp", resource_metadata="https://mcp.example/.well-known/oauth-protected-resource", scope="read write""#,
        );
        assert_eq!(
            map.get("resource_metadata").map(String::as_str),
            Some("https://mcp.example/.well-known/oauth-protected-resource")
        );
        assert_eq!(map.get("scope").map(String::as_str), Some("read write"));
    }

    #[test]
    fn splits_origin_and_path_cleanly() {
        assert_eq!(
            split_origin_and_path("https://mcp.example:8443/v1/mcp/").expect("split"),
            ("https://mcp.example:8443".into(), "/v1/mcp".into())
        );
        assert_eq!(
            split_origin_and_path("https://auth.example").expect("split"),
            ("https://auth.example".into(), String::new())
        );
    }

    #[test]
    fn debug_never_exposes_tokens() {
        let creds = McpCredentials {
            access_token: "super-secret-access".into(),
            refresh_token: "super-secret-refresh".into(),
            client_secret: Some("super-secret-client".into()),
            ..McpCredentials::default()
        };
        let dbg = format!("{creds:?}");
        assert!(!dbg.contains("super-secret"), "{dbg}");
    }

    #[tokio::test]
    async fn full_pkce_dcr_sign_in_and_refresh_flow() {
        let server = MockServer::start().await;
        let uri = server.uri();
        let mcp_url = format!("{uri}/mcp");

        Mock::given(method("GET"))
            .and(path("/.well-known/oauth-protected-resource/mcp"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "resource": mcp_url,
                "authorization_servers": [uri],
                "scopes_supported": ["mcp:read", "mcp:write"]
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/.well-known/oauth-authorization-server"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": uri,
                "authorization_endpoint": format!("{uri}/authorize"),
                "token_endpoint": format!("{uri}/token"),
                "registration_endpoint": format!("{uri}/register"),
                "revocation_endpoint": format!("{uri}/revoke"),
                "code_challenge_methods_supported": ["S256"]
            })))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/register"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "client_id": "dyn-client-42"
            })))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "tok-initial",
                "refresh_token": "ref-1",
                "expires_in": 10,
                "scope": "mcp:read"
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "tok-refreshed",
                "refresh_token": "ref-2",
                "expires_in": 3600,
                "scope": "mcp:read"
            })))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/revoke"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().expect("tempdir");
        let file: McpServerFile =
            toml::from_str(&format!("url = \"{mcp_url}\"\n[oauth]\nenabled = true\n"))
                .expect("parse");
        let ledger = Arc::new(EgressLedger::default());
        let oauth = McpOAuthClient::from_server(
            "demo",
            &file,
            tmp.path(),
            NetworkSettings::default(),
            AllowList::default(),
            Some(ledger.clone()),
        )
        .expect("client");

        assert!(!has_saved_token(tmp.path(), "demo"));
        assert!(oauth.bearer().await.is_err());

        // Simulate browser completing the loopback callback as soon as `open` gets the auth URL.
        let signed = oauth
            .sign_in(|auth_url| {
                let (_, q) = auth_url.split_once('?').expect("query");
                let pairs = parse_query(q);
                let get = |k: &str| {
                    pairs
                        .iter()
                        .find(|(n, _)| n == k)
                        .map(|(_, v)| v.clone())
                        .expect(k)
                };
                assert_eq!(get("client_id"), "dyn-client-42");
                assert_eq!(get("code_challenge_method"), "S256");
                assert_eq!(get("resource"), mcp_url);
                let redirect = get("redirect_uri");
                let state = get("state");
                tokio::spawn(async move {
                    let cb = format!("{redirect}?code=auth-code-99&state={state}");
                    let client = reqwest::Client::builder()
                        .no_proxy()
                        .build()
                        .expect("cb client");
                    let _ = client.get(cb).send().await;
                });
            })
            .await
            .expect("sign_in");

        assert_eq!(signed.server, "demo");
        assert_eq!(signed.scopes, vec!["mcp:read"]);
        assert!(has_saved_token(tmp.path(), "demo"));

        // Because expires_in was 10s (< EARLY_REFRESH_SECS), bearer() triggers refresh immediately.
        let tok = oauth.bearer().await.expect("refreshed bearer");
        assert_eq!(tok, "tok-refreshed");

        // Second call uses the fresh cached/disk token without another refresh.
        let tok2 = oauth.bearer().await.expect("fresh bearer");
        assert_eq!(tok2, "tok-refreshed");

        assert!(oauth.sign_out().await.expect("sign_out"));
        assert!(!has_saved_token(tmp.path(), "demo"));
        assert!(!ledger.snapshot().0.is_empty());
    }

    #[tokio::test]
    async fn blocks_disallowed_policy_host_during_discovery() {
        let server = MockServer::start().await;
        let mcp_url = format!("{}/mcp", server.uri());
        let tmp = tempfile::tempdir().expect("tempdir");
        let file: McpServerFile = toml::from_str(&format!(
            "url = \"{mcp_url}\"\n[oauth]\nauthorization_server = \"https://evil.example\"\n"
        ))
        .expect("parse");
        let policy = AllowList {
            allowed: Vec::new(),
            hosts: vec!["127.0.0.1".into()],
        };
        let ledger = Arc::new(EgressLedger::default());
        let oauth = McpOAuthClient::from_server(
            "demo",
            &file,
            tmp.path(),
            NetworkSettings::default(),
            policy,
            Some(ledger.clone()),
        )
        .expect("client");
        let err = oauth
            .sign_in(|_| {})
            .await
            .expect_err("policy should block");
        assert!(err.contains("policy.mcp"), "{err}");
        assert!(
            ledger
                .snapshot()
                .0
                .iter()
                .any(|e| e.kind == EgressKind::BlockedEgress)
        );
    }
}
