//! The HTTPS side of the protocol (`api.md`, "Conventions"): one request, its headers, the size of what is read back,
//! and the error model. TLS is rustls with the operating system's certificate store (platform verifier); certificate
//! checks are never turned off. Redirects: at most 3, never to a non-https URL (plain http only between loopback URLs).

use crate::proto::url as purl;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

/// The highest 1.x version this launcher implements (1.1: activities, series, contract 2, presentation).
pub const PROTOCOL_VERSION: &str = "1.1";
pub const USER_AGENT: &str = concat!("Kalamar/", env!("CARGO_PKG_VERSION"));
/// What the launcher reads of an ordinary answer (meta, lists, match objects).
pub const MAX_ANSWER: usize = 2 << 20;

/// Installs the TLS crypto provider once (ring).
pub fn install_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// The HTTP client: platform certificate verification, the protocol's redirect rules, a timeout per request.
pub fn build_client(timeout: Duration) -> Result<reqwest::Client, String> {
    install_crypto();
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        let downgrade = attempt.previous().last().is_some_and(|p| p.scheme() == "https") && attempt.url().scheme() != "https";
        if attempt.previous().len() > 3 {
            attempt.error("more than 3 redirects")
        } else if downgrade || !purl::allowed_url(attempt.url()) {
            attempt.error("redirect to a non-https URL refused")
        } else {
            attempt.follow()
        }
    });
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(policy)
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())
}

/// An error answer of the service (`overview.md` 6).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ApiError {
    pub status: u16,
    pub code: String,
    pub message: String,
    pub field: Option<String>,
    pub retry_after_s: Option<u64>,
    /// `details` of the error object when there is one (for example `requirements` of `not_eligible`).
    pub details: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallError {
    /// No answer: DNS, connection, TLS (a certificate the system does not trust), timeout.
    Transport(String),
    Api(ApiError),
    /// An answer this launcher cannot use (too large, not JSON where JSON is expected).
    Body(String),
}

impl CallError {
    /// A stable code: the service's `code`, `network` or `bad_answer`.
    pub fn code(&self) -> &str {
        match self {
            CallError::Transport(_) => "network",
            CallError::Api(e) => &e.code,
            CallError::Body(_) => "bad_answer",
        }
    }
    pub fn message(&self) -> String {
        match self {
            CallError::Transport(m) | CallError::Body(m) => m.clone(),
            CallError::Api(e) => format!("{} {}: {}", e.status, e.code, e.message),
        }
    }
    pub fn status(&self) -> u16 {
        match self {
            CallError::Api(e) => e.status,
            _ => 0,
        }
    }
    pub fn api(&self) -> Option<&ApiError> {
        match self {
            CallError::Api(e) => Some(e),
            _ => None,
        }
    }
}

/// The code of an error answer: the body's `error.code`, else one derived from the status (unknown codes are handled by
/// their status class).
pub fn parse_error(status: u16, body: &Value, retry_after_header: Option<u64>) -> ApiError {
    let e = body.get("error");
    let s = |k: &str| e.and_then(|e| e.get(k)).and_then(Value::as_str).map(str::to_string);
    let code = s("code").filter(|c| !c.is_empty() && c.len() <= 64).unwrap_or_else(|| match status {
        400 => "invalid_json".into(),
        401 => "unauthorized".into(),
        403 => "forbidden".into(),
        404 => "not_found".into(),
        409 => "conflict".into(),
        413 => "payload_too_large".into(),
        422 => "validation_failed".into(),
        429 => "rate_limited".into(),
        503 => "unavailable".into(),
        s if s >= 500 => "internal".into(),
        s => format!("http_{s}"),
    });
    ApiError {
        status,
        message: s("message").unwrap_or_else(|| format!("HTTP {status}")).chars().take(300).collect(),
        field: s("field"),
        retry_after_s: e.and_then(|e| e.get("retry_after_s")).and_then(Value::as_u64).or(retry_after_header),
        details: e.and_then(|e| e.get("details")).cloned().unwrap_or(Value::Null),
        code,
    }
}

pub enum Body {
    None,
    Json(Value),
    /// Bytes sent as they are, with `Content-Type: application/json` (the game's status and result files, verbatim).
    Raw(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub status: u16,
    /// `Null` for an empty answer (204).
    pub body: Value,
    /// The address the answer came from (the server, or a proxy), when known.
    pub peer: Option<std::net::SocketAddr>,
}

/// One request. `max` bounds what is read back. 2xx is success; anything else is an [`ApiError`].
pub async fn send(client: &reqwest::Client, method: &str, url: &str, bearer: Option<&str>, body: &Body, lang: &str, max: usize) -> Result<Reply, CallError> {
    let parsed = reqwest::Url::parse(url).map_err(|e| CallError::Transport(format!("{url}: {e}")))?;
    if !purl::allowed_url(&parsed) {
        return Err(CallError::Transport(format!("{url}: only https (or http to this computer) is allowed")));
    }
    let m = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| CallError::Transport(format!("bad method {method}")))?;
    let mut rb = client.request(m, parsed).header("Kalamar-Protocol", PROTOCOL_VERSION).header("Accept", "application/json");
    if !lang.is_empty() {
        rb = rb.header("Accept-Language", if lang == "en" { "en".to_string() } else { format!("{lang}, en;q=0.5") });
    }
    if let Some(t) = bearer {
        rb = rb.header("Authorization", format!("Bearer {t}"));
    }
    rb = match body {
        Body::None => rb,
        Body::Json(v) => rb.header("Content-Type", "application/json").body(v.to_string()),
        Body::Raw(b) => rb.header("Content-Type", "application/json").body(b.clone()),
    };
    let mut resp = rb.send().await.map_err(|e| CallError::Transport(describe(&e)))?;
    let status = resp.status().as_u16();
    let peer = resp.remote_addr();
    let retry_after = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok());
    if resp.content_length().is_some_and(|n| n as usize > max) {
        return Err(CallError::Body(format!("the answer is larger than {max} bytes")));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| CallError::Transport(describe(&e)))? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > max {
            return Err(CallError::Body(format!("the answer is larger than {max} bytes")));
        }
    }
    let json = if bytes.iter().all(u8::is_ascii_whitespace) { Ok(Value::Null) } else { serde_json::from_slice::<Value>(&bytes) };
    if (200..300).contains(&status) {
        return json.map(|body| Reply { status, body, peer }).map_err(|e| CallError::Body(format!("the answer is not JSON: {e}")));
    }
    Err(CallError::Api(parse_error(status, &json.unwrap_or(Value::Null), retry_after)))
}

/// A transport error in one sentence, with its cause chain (a refused certificate says so).
pub fn describe(e: &(dyn std::error::Error + 'static)) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    let mut n = 0;
    while let Some(c) = src {
        let t = c.to_string();
        if !s.contains(&t) {
            s.push_str(": ");
            s.push_str(&t);
        }
        src = c.source();
        n += 1;
        if n > 6 {
            break;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn error_answers() {
        let e = parse_error(409, &json!({"error": {"code": "room_full", "message": "the room is full", "field": null, "retry_after_s": null}}), None);
        assert_eq!((e.code.as_str(), e.message.as_str(), e.field.clone(), e.retry_after_s), ("room_full", "the room is full", None, None));
        let e = parse_error(422, &json!({"error": {"code": "validation_failed", "message": "bad", "field": "/options/password"}}), None);
        assert_eq!(e.field.as_deref(), Some("/options/password"));
        let e = parse_error(429, &Value::Null, Some(7));
        assert_eq!((e.code.as_str(), e.retry_after_s), ("rate_limited", Some(7)));
        assert_eq!(parse_error(502, &json!("x"), None).code, "internal");
        assert_eq!(parse_error(418, &Value::Null, None).code, "http_418");
        let e = parse_error(403, &json!({"error": {"code": "not_eligible", "message": "m", "details": {"requirements": [{"id": "x"}]}}}), None);
        assert_eq!(e.details["requirements"][0]["id"], "x");
    }

    #[test]
    fn plain_http_is_refused_outside_the_loopback() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let c = build_client(Duration::from_secs(2)).unwrap();
        let r = rt.block_on(send(&c, "GET", "http://example.invalid/v1/meta", None, &Body::None, "fr", MAX_ANSWER));
        assert!(matches!(r, Err(CallError::Transport(m)) if m.contains("only https")));
    }
}
