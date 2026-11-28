//! SSRF-protected HTTP fetch implementation for the opt-in `web_fetch` tool
//! (docs/plan/11-security.md#network-egress-policy).
//!
//! Enforces:
//! - `http://` and `https://` schemes only (no userinfo credentials in URLs)
//! - `Settings::check_fetch` (`network.offline`, `egress_allow`, system policy host lists)
//! - Private/loopback/link-local/metadata/CGNAT/reserved IPv4 & IPv6 blocking unless
//!   `tools.web_fetch.allow_private = true`
//! - DNS resolution before connecting + `resolve_to_addrs` pinning (when no proxy is active)
//!   to defend against DNS rebinding
//! - Per-hop redirect re-validation (max 5 hops)
//! - Bounded response body + HTML-to-readable-text extraction
//! - Full `EgressLedger` recording for both allowed and blocked hops

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use quack_config::Settings;
use quack_protocol::EgressKind;
use reqwest::Url;
use reqwest::header::{CONTENT_TYPE, LOCATION};
use tokio_util::sync::CancellationToken;

use crate::ledger::{EgressDraft, EgressLedger};

const MAX_REDIRECTS: usize = 5;

/// Result of a completed SSRF-checked fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFetchOutcome {
    pub final_url: String,
    pub status: u16,
    pub content_type: String,
    pub body: String,
    pub truncated: bool,
}

/// Whether `ip` belongs to a non-public, loopback, private, link-local, metadata, CGNAT,
/// multicast, documentation, or reserved network range.
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => is_blocked_v6(v6),
    }
}

fn is_blocked_v4(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        // 0.0.0.0/8 ("this network")
        || a == 0
        // 100.64.0.0/10 (RFC 6598 CGNAT / shared space, e.g. Tailscale & cloud internal)
        || (a == 100 && (b & 0xc0) == 64)
        // 198.18.0.0/15 (RFC 2544 benchmarking)
        || (a == 198 && (b & 0xfe) == 18)
        // 240.0.0.0/4 (reserved / Class E)
        || a >= 240
}

fn is_blocked_v6(ip: Ipv6Addr) -> bool {
    let segs = ip.segments();
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_blocked_v4(v4);
    }
    // IPv4-compatible `::/96` (other than :: and ::1 which are caught below)
    if segs[0..6] == [0, 0, 0, 0, 0, 0] && (segs[6] != 0 || segs[7] > 1) {
        let [a, b] = segs[6].to_be_bytes();
        let [c, d] = segs[7].to_be_bytes();
        return is_blocked_v4(Ipv4Addr::new(a, b, c, d));
    }
    // NAT64 well-known prefix `64:ff9b::/96` and local `64:ff9b:1::/48`
    if segs[0] == 0x0064 && segs[1] == 0xff9b {
        if segs[2..6] == [0, 0, 0, 0] {
            let [a, b] = segs[6].to_be_bytes();
            let [c, d] = segs[7].to_be_bytes();
            return is_blocked_v4(Ipv4Addr::new(a, b, c, d));
        }
        if segs[2] == 1 {
            return true;
        }
    }
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        // Unique Local Addresses `fc00::/7`
        || (segs[0] & 0xfe00) == 0xfc00
        // Link-Local Unicast `fe80::/10`
        || (segs[0] & 0xffc0) == 0xfe80
        // Documentation `2001:db8::/32`
        || (segs[0] == 0x2001 && segs[1] == 0x0db8)
        // Discard-Only `100::/64`
        || (segs[0] == 0x0100 && segs[1] == 0 && segs[2] == 0 && segs[3] == 0)
}

/// Fetch `raw_url` with full SSRF, egress policy, redirect, timeout, and size enforcement.
pub async fn fetch(
    settings: &Settings,
    env: &(dyn Fn(&str) -> Option<std::ffi::OsString> + Sync),
    raw_url: &str,
    max_chars: usize,
    cancel: CancellationToken,
    ledger: Option<&EgressLedger>,
) -> Result<WebFetchOutcome, String> {
    if settings.network.offline {
        return Err("web_fetch is unavailable when network.offline = true".into());
    }
    let cfg = &settings.tools.web_fetch;
    let timeout = Duration::from_secs(cfg.timeout_secs.max(1));
    let max_bytes = cfg.max_bytes.max(1024);
    let mut url = parse_http_url(raw_url)?;

    let (base_builder, has_proxy) = crate::provider::unredirected_builder(&settings.network, env)?;
    let shared_client = if has_proxy {
        Some(
            base_builder
                .build()
                .map_err(|e| format!("HTTP client: {e}"))?,
        )
    } else {
        None
    };

    for hop in 0..=MAX_REDIRECTS {
        let block_kind = if hop == 0 {
            EgressKind::BlockedEgress
        } else {
            EgressKind::BlockedRedirect
        };
        let record_block = |why: &str| {
            if let Some(ledger) = ledger {
                ledger.record(
                    EgressDraft::new(block_kind, url.to_string(), "web_fetch")
                        .blocked(why.to_owned()),
                );
            }
        };

        if let Err(why) = settings.check_fetch(url.as_str()) {
            record_block(&why);
            return Err(why);
        }

        let host = url
            .host_str()
            .ok_or_else(|| format!("`{url}` has no host"))?
            .to_owned();
        let port = url
            .port_or_known_default()
            .ok_or_else(|| format!("`{url}` has no known port"))?;

        let client = if let Some(ref c) = shared_client {
            // When routing through an HTTP proxy, block literal private IPs directly;
            // DNS resolution happens on the proxy.
            if !cfg.allow_private
                && let Ok(ip) = host.trim_matches(['[', ']']).parse::<IpAddr>()
                && is_blocked_ip(ip)
            {
                let why = format!(
                    "blocked by SSRF protection: `{ip}` is a private, loopback, or metadata address (set tools.web_fetch.allow_private = true to allow)"
                );
                record_block(&why);
                return Err(why);
            }
            c.clone()
        } else {
            let addrs = resolve_and_check(&host, port, cfg.allow_private)
                .await
                .inspect_err(|why| {
                    record_block(why);
                })?;
            let (mut b, _) = crate::provider::unredirected_builder(&settings.network, env)?;
            if host.trim_matches(['[', ']']).parse::<IpAddr>().is_err() {
                b = b.resolve_to_addrs(&host, &addrs);
            }
            b.build().map_err(|e| format!("HTTP client: {e}"))?
        };

        let send = client
            .get(url.clone())
            .header(
                reqwest::header::ACCEPT,
                "text/markdown, text/plain;q=0.9, text/html;q=0.8, application/json;q=0.7, */*;q=0.1",
            )
            .timeout(timeout)
            .send();
        let mut resp = tokio::select! {
            () = cancel.cancelled() => return Err("cancelled".into()),
            r = send => r.map_err(|e| format!("could not fetch {host}: {e}"))?,
        };
        let status = resp.status();

        if status.is_redirection() {
            let target = resp
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|l| url.join(l).ok())
                .ok_or_else(|| format!("{url} redirected without a valid Location header"))?;
            let target = parse_http_url(target.as_str()).inspect_err(|why| {
                record_block(why);
            })?;
            if let Some(ledger) = ledger {
                ledger.record(
                    EgressDraft::new(EgressKind::Fetch, url.to_string(), "web_fetch")
                        .status(Some(status.as_u16())),
                );
            }
            url = target;
            continue;
        }

        let content_type = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();

        let mut bytes = Vec::new();
        let mut byte_truncated = false;
        loop {
            let chunk = tokio::select! {
                () = cancel.cancelled() => return Err("cancelled".into()),
                c = resp.chunk() => c.map_err(|e| format!("reading {host}: {e}"))?,
            };
            let Some(chunk) = chunk else { break };
            if bytes.len() + chunk.len() > max_bytes {
                let take = max_bytes.saturating_sub(bytes.len());
                bytes.extend_from_slice(&chunk[..take]);
                byte_truncated = true;
                break;
            }
            bytes.extend_from_slice(&chunk);
        }

        if let Some(ledger) = ledger {
            let mut draft = EgressDraft::new(EgressKind::Fetch, url.to_string(), "web_fetch")
                .received(bytes.len() as u64)
                .status(Some(status.as_u16()));
            if !status.is_success() {
                draft = draft.blocked(format!("HTTP {status}"));
            }
            ledger.record(draft);
        }

        let raw_text = String::from_utf8_lossy(&bytes);
        let text = if is_html(&content_type, &raw_text) {
            html_to_text(&raw_text)
        } else {
            raw_text.into_owned()
        };
        let (body, char_truncated) = truncate_chars(&text, max_chars);
        return Ok(WebFetchOutcome {
            final_url: url.into(),
            status: status.as_u16(),
            content_type,
            body,
            truncated: byte_truncated || char_truncated,
        });
    }

    Err(format!(
        "`{raw_url}` redirected more than {MAX_REDIRECTS} times"
    ))
}

fn parse_http_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw.trim()).map_err(|e| format!("invalid URL `{raw}`: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "web_fetch only allows http:// and https:// URLs (got `{}`)",
            url.scheme()
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("web_fetch refuses URLs with embedded credentials".into());
    }
    if url.host_str().is_none() {
        return Err(format!("URL `{raw}` has no host"));
    }
    Ok(url)
}

async fn resolve_and_check(
    host: &str,
    port: u16,
    allow_private: bool,
) -> Result<Vec<SocketAddr>, String> {
    let bare = host.trim_matches(['[', ']']);
    if let Ok(ip) = bare.parse::<IpAddr>() {
        if !allow_private && is_blocked_ip(ip) {
            return Err(format!(
                "blocked by SSRF protection: `{ip}` is a private, loopback, or metadata address (set tools.web_fetch.allow_private = true to allow)"
            ));
        }
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    let resolved: Vec<SocketAddr> = tokio::net::lookup_host((bare, port))
        .await
        .map_err(|e| format!("could not resolve `{bare}`: {e}"))?
        .collect();
    if resolved.is_empty() {
        return Err(format!("DNS lookup for `{bare}` returned no addresses"));
    }
    if !allow_private {
        for addr in &resolved {
            if is_blocked_ip(addr.ip()) {
                return Err(format!(
                    "blocked by SSRF protection: `{bare}` resolved to private/loopback/metadata address `{}` (set tools.web_fetch.allow_private = true to allow)",
                    addr.ip()
                ));
            }
        }
    }
    Ok(resolved)
}

fn is_html(content_type: &str, body: &str) -> bool {
    let ct = content_type.to_ascii_lowercase();
    if ct.contains("text/html") || ct.contains("application/xhtml") {
        return true;
    }
    let start = body.trim_start();
    start.starts_with("<!DOCTYPE html")
        || start.starts_with("<!doctype html")
        || start.starts_with("<html")
}

/// Lightweight, dependency-free HTML-to-readable-text extractor: strips `<script>`/`<style>`
/// blocks, inserts newlines around block tags, decodes common HTML entities, and collapses
/// excess blank lines.
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let bytes = html.as_bytes();
    let mut i = 0;
    let mut skip_until: Option<&str> = None;

    while i < bytes.len() {
        if let Some(close_tag) = skip_until {
            if let Some(pos) = find_ascii_ci(&html[i..], close_tag) {
                i += pos + close_tag.len();
                skip_until = None;
            } else {
                break;
            }
            continue;
        }
        if bytes[i] == b'<' {
            // HTML comment `<!-- ... -->`
            if html[i..].starts_with("<!--") {
                if let Some(end) = html[i + 4..].find("-->") {
                    i += 4 + end + 3;
                    continue;
                }
                break;
            }
            if let Some(end) = html[i..].find('>') {
                let tag = &html[i + 1..i + end];
                let name = tag
                    .trim_start()
                    .trim_start_matches('/')
                    .split(|c: char| c.is_ascii_whitespace() || c == '/')
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let is_closing = tag.trim_start().starts_with('/');
                if !is_closing && matches!(name.as_str(), "script" | "style" | "noscript" | "svg") {
                    skip_until = Some(match name.as_str() {
                        "script" => "</script>",
                        "style" => "</style>",
                        "noscript" => "</noscript>",
                        _ => "</svg>",
                    });
                } else if matches!(
                    name.as_str(),
                    "p" | "div"
                        | "br"
                        | "li"
                        | "tr"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "section"
                        | "article"
                        | "header"
                        | "footer"
                        | "pre"
                        | "blockquote"
                        | "hr"
                ) {
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                } else if name == "td" || name == "th" {
                    out.push(' ');
                }
                i += end + 1;
                continue;
            }
        }
        let ch = html[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }

    let decoded = decode_entities(&out);
    let mut cleaned = String::with_capacity(decoded.len());
    let mut blank_run = 0usize;
    for raw_line in decoded.lines() {
        let line: String = raw_line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            blank_run += 1;
            if blank_run <= 1 && !cleaned.is_empty() {
                cleaned.push('\n');
            }
        } else {
            blank_run = 0;
            cleaned.push_str(&line);
            cleaned.push('\n');
        }
    }
    cleaned.trim().to_owned()
}

fn find_ascii_ci(haystack: &str, needle: &str) -> Option<usize> {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || h.len() < n.len() {
        return None;
    }
    h.windows(n.len()).position(|w| w.eq_ignore_ascii_case(n))
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

fn truncate_chars(s: &str, max_chars: usize) -> (String, bool) {
    let mut iter = s.char_indices();
    if let Some((byte_idx, _)) = iter.nth(max_chars) {
        (s[..byte_idx].to_owned(), true)
    } else {
        (s.to_owned(), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::path;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn ssrf_blocks_private_loopback_metadata_and_mapped_ips() {
        for blocked in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.1",
            "192.168.1.10",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "240.0.0.1",
            "::1",
            "fe80::1",
            "fc00::1",
            "fd12:3456::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "64:ff9b::169.254.169.254",
        ] {
            let ip: IpAddr = blocked.parse().expect("parse ip");
            assert!(is_blocked_ip(ip), "expected {blocked} to be blocked");
        }
        for allowed in [
            "93.184.216.34",
            "1.1.1.1",
            "2606:2800:220:1:248:1893:25c8:1946",
        ] {
            let ip: IpAddr = allowed.parse().expect("parse ip");
            assert!(!is_blocked_ip(ip), "expected {allowed} to be allowed");
        }
    }

    #[tokio::test]
    async fn blocks_loopback_by_default_and_allows_when_opted_in() {
        let server = MockServer::start().await;
        Mock::given(path("/doc"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/html")
                    .set_body_string(
                        "<html><head><style>body{color:red}</style></head>\
                         <body><h1>Title</h1><script>evil()</script><p>Hello &amp; welcome</p></body></html>",
                    ),
            )
            .mount(&server)
            .await;

        let url = format!("{}/doc", server.uri());
        let mut settings = Settings::default();
        settings.tools.web_fetch.enabled = true;
        let ledger = EgressLedger::new();

        // Default allow_private = false blocks 127.0.0.1.
        let err = fetch(
            &settings,
            &|_| None,
            &url,
            10_000,
            CancellationToken::new(),
            Some(&ledger),
        )
        .await
        .expect_err("loopback blocked");
        assert!(err.contains("SSRF"), "{err}");
        assert_eq!(ledger.snapshot().0.len(), 1);
        assert_eq!(ledger.snapshot().0[0].kind, EgressKind::BlockedEgress);

        // Opting into allow_private succeeds and strips script/style tags.
        settings.tools.web_fetch.allow_private = true;
        let ok = fetch(
            &settings,
            &|_| None,
            &url,
            10_000,
            CancellationToken::new(),
            Some(&ledger),
        )
        .await
        .expect("allowed private fetch");
        assert_eq!(ok.status, 200);
        assert!(ok.body.contains("Title"));
        assert!(ok.body.contains("Hello & welcome"));
        assert!(!ok.body.contains("evil()"));
        assert!(!ok.body.contains("color:red"));
    }

    #[tokio::test]
    async fn offline_and_bad_schemes_are_refused() {
        let mut settings = Settings::default();
        settings.tools.web_fetch.enabled = true;
        settings.network.offline = true;
        let err = fetch(
            &settings,
            &|_| None,
            "https://example.com",
            1000,
            CancellationToken::new(),
            None,
        )
        .await
        .expect_err("offline blocked");
        assert!(err.contains("offline"), "{err}");

        settings.network.offline = false;
        assert!(
            fetch(
                &settings,
                &|_| None,
                "file:///etc/passwd",
                1000,
                CancellationToken::new(),
                None,
            )
            .await
            .is_err()
        );
        assert!(
            fetch(
                &settings,
                &|_| None,
                "https://user:pass@example.com/",
                1000,
                CancellationToken::new(),
                None,
            )
            .await
            .is_err()
        );
    }
}
