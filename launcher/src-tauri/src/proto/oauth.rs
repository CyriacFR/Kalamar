//! Login with an `oauth_loopback` provider (`api.md`, "OAuth loopback providers"; RFC 7636 PKCE, RFC 8252 loopback
//! redirect): the launcher makes a PKCE pair, listens on one of the server's loopback redirects, asks the server for
//! the authorization URL, opens it in the system browser, takes the code from the redirect (checking `state`), and
//! hands code and verifier to the server. The provider's tokens and secret never reach the launcher.

use crate::proto::client::Client;
use crate::proto::http::{Body, CallError};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub fn b64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        let k = c.len() + 1;
        for i in 0..k {
            out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// `(code_verifier, code_challenge)`: 43 characters from 32 random bytes, and BASE64URL-NOPAD(SHA-256(verifier)).
pub fn pkce() -> Result<(String, String), String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    let verifier = b64url(&b);
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    Ok((verifier, challenge))
}

/// `http://127.0.0.1:<port>/<path>` or `http://[::1]:<port>/<path>`: (bind address, path).
pub fn parse_redirect(uri: &str) -> Option<(String, String)> {
    let rest = uri.strip_prefix("http://")?;
    let (auth, path) = rest.split_once('/').map(|(a, p)| (a, format!("/{p}"))).unwrap_or((rest, "/".into()));
    let (host, port) = auth.rsplit_once(':')?;
    let port: u16 = port.parse().ok().filter(|p| *p > 0)?;
    let bind = match host {
        "127.0.0.1" => format!("127.0.0.1:{port}"),
        "[::1]" => format!("[::1]:{port}"),
        _ => return None,
    };
    if path.contains(['?', '#']) {
        return None;
    }
    Some((bind, path))
}

/// The query parameters of the redirect's request line (`GET /callback?code=...&state=... HTTP/1.1`).
pub fn parse_callback(request_line: &str, path: &str) -> Option<Vec<(String, String)>> {
    let mut parts = request_line.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let target = parts.next()?;
    let (p, q) = target.split_once('?').unwrap_or((target, ""));
    if p != path {
        return None;
    }
    Some(q.split('&').filter(|kv| !kv.is_empty()).map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (unescape(k), unescape(v))
    }).collect())
}

fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = (b[i] == b'%' && i + 3 <= b.len()).then(|| std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())).flatten();
        match (b[i], hex) {
            (b'+', _) => out.push(b' '),
            (b'%', Some(x)) => {
                out.push(x);
                i += 2;
            }
            (c, _) => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Opens a URL in the system browser (no shell involved: the URL is one argument).
pub fn open_browser(url: &str) -> Result<(), String> {
    let u = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
    // https (plain http only to this computer, for a test server's stand-in provider); never another scheme.
    if !crate::proto::url::allowed_url(&u) {
        return Err("the authorization URL is not https".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // By its full path in System32: a bare name would be looked up in the launcher's folder first.
        std::process::Command::new(crate::winutil::system_exe("rundll32.exe")).arg("url.dll,FileProtocolHandler").arg(u.as_str()).creation_flags(0x0800_0000).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("xdg-open").arg(u.as_str()).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

const PAGE_OK: &str = "<!doctype html><meta charset=utf-8><title>Kalamar</title><body style=\"font-family:sans-serif;background:#0b0920;color:#fffdf6;padding:3rem\"><h1>Kalamar</h1><p>Login received: you can close this tab and go back to the launcher.</p>";
const PAGE_BAD: &str = "<!doctype html><meta charset=utf-8><title>Kalamar</title><body style=\"font-family:sans-serif;background:#0b0920;color:#fffdf6;padding:3rem\"><h1>Kalamar</h1><p>The login did not complete. Go back to the launcher.</p>";

/// The whole flow. `open` opens the authorization URL (the system browser; a test follows it itself). Returns `me`.
pub async fn login(client: &Client, provider: &Value, open: &(dyn Fn(&str) -> Result<(), String> + Send + Sync), timeout: Duration) -> Result<Value, CallError> {
    let id = provider.get("id").and_then(Value::as_str).unwrap_or_default();
    if !crate::proto::meta::valid_id(id) || provider.get("kind").and_then(Value::as_str) != Some("oauth_loopback") {
        return Err(CallError::Body("not an oauth_loopback provider".into()));
    }
    let redirects: Vec<String> = provider.get("loopback_redirects").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
    // 1-2. PKCE pair; a listener on the first redirect whose port is free.
    let (verifier, challenge) = pkce().map_err(CallError::Transport)?;
    let mut bound = None;
    for r in &redirects {
        if let Some((addr, path)) = parse_redirect(r)
            && let Ok(l) = TcpListener::bind(&addr).await
        {
            bound = Some((l, r.clone(), path));
            break;
        }
    }
    let (listener, redirect_uri, path) = bound.ok_or_else(|| CallError::Transport("none of the provider's loopback ports is free on this computer".into()))?;
    let begin = client
        .call("POST", &format!("/v1/auth/{id}/begin"), Body::Json(json!({"redirect_uri": redirect_uri, "code_challenge": challenge, "code_challenge_method": "S256"})), false)
        .await?;
    let authorize = begin.body.get("authorize_url").and_then(Value::as_str).ok_or_else(|| CallError::Body("no authorize_url".into()))?.to_string();
    let state = begin.body.get("state").and_then(Value::as_str).ok_or_else(|| CallError::Body("no state".into()))?.to_string();
    // 3. The browser; then the redirect.
    open(&authorize).map_err(CallError::Transport)?;
    let code = tokio::time::timeout(timeout, wait_code(&listener, &path, &state))
        .await
        .map_err(|_| CallError::Transport("the login was not finished in the browser in time".into()))?
        .map_err(CallError::Transport)?;
    drop(listener);
    // 4. Complete.
    let r = client.call("POST", &format!("/v1/auth/{id}/complete"), Body::Json(json!({"state": state, "code": code, "code_verifier": verifier})), false).await?;
    client.accept_session(&r.body, id)
}

async fn wait_code(listener: &TcpListener, path: &str, state: &str) -> Result<String, String> {
    loop {
        let (mut sock, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 8192];
        let n = tokio::time::timeout(Duration::from_secs(5), sock.read(&mut buf)).await.ok().and_then(|r| r.ok()).unwrap_or(0);
        let text = String::from_utf8_lossy(&buf[..n]).to_string();
        let line = text.lines().next().unwrap_or("");
        let Some(params) = parse_callback(line, path) else {
            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue; // a favicon request, a stray connection
        };
        let get = |k: &str| params.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        let ok = get("state").as_deref() == Some(state) && get("code").is_some_and(|c| !c.is_empty());
        let page = if ok { PAGE_OK } else { PAGE_BAD };
        let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len()).as_bytes()).await;
        let _ = sock.shutdown().await;
        if get("state").as_deref() != Some(state) {
            return Err("the browser came back with another login's state".into());
        }
        if let Some(e) = get("error") {
            return Err(format!("the provider refused the login: {e}"));
        }
        return get("code").filter(|c| !c.is_empty()).ok_or_else(|| "no code in the redirect".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc7636() {
        // RFC 7636, appendix B.
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(b64url(&Sha256::digest(v.as_bytes())), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(b64url(b"f"), "Zg");
        assert_eq!(b64url(b"fo"), "Zm8");
        assert_eq!(b64url(b"foo"), "Zm9v");
        let (verifier, challenge) = pkce().unwrap();
        assert_eq!(verifier.len(), 43);
        assert!(verifier.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        assert_eq!(challenge, b64url(&Sha256::digest(verifier.as_bytes())));
    }

    #[test]
    fn redirects_and_callbacks() {
        assert_eq!(parse_redirect("http://127.0.0.1:53682/callback"), Some(("127.0.0.1:53682".into(), "/callback".into())));
        assert_eq!(parse_redirect("http://[::1]:53683/cb"), Some(("[::1]:53683".into(), "/cb".into())));
        assert_eq!(parse_redirect("http://localhost:53682/callback"), None);
        assert_eq!(parse_redirect("https://127.0.0.1:53682/callback"), None);
        assert_eq!(parse_redirect("http://10.0.0.1:53682/callback"), None);
        let p = parse_callback("GET /callback?code=abc%2Fd&state=st_1 HTTP/1.1", "/callback").unwrap();
        assert_eq!(p, vec![("code".to_string(), "abc/d".to_string()), ("state".to_string(), "st_1".to_string())]);
        assert!(parse_callback("GET /favicon.ico HTTP/1.1", "/callback").is_none());
        assert!(parse_callback("POST /callback?code=a HTTP/1.1", "/callback").is_none());
        assert_eq!(unescape("a%zz"), "a%zz");
        assert_eq!(unescape("%41%"), "A%");
    }
}
