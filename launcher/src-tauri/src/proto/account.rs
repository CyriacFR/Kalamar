//! The player's own data on a server: `GET /v1/me/export` (everything the server holds, one JSON document) and
//! `DELETE /v1/me?confirm=erase` (the account is erased there). Both belong to the protocol's core, so every server
//! has them. The page never sees a token or chooses a path: the document is written where the player says in the
//! system's own "save as" dialog (`glue::proto_export`), and its content is never logged.

use super::client::Client;
use super::http::{Body, CallError};
use serde_json::Value;
use std::path::Path;

/// The largest export read back. A real one is a few hundred kilobytes; this only stops a server from filling the
/// memory or the disk.
pub const MAX_EXPORT: usize = 32 << 20;

/// The host of an API address, reduced to what a file name may carry (`a-z`, digits, `.` and `-`).
fn host_for_file(api: &str) -> String {
    let host = reqwest::Url::parse(api).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    let clean: String = host.to_ascii_lowercase().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-').take(64).collect();
    let clean = clean.trim_matches(|c| c == '.' || c == '-').to_string();
    if clean.is_empty() { "server".into() } else { clean }
}

/// `kalamar-export-<server host>-<UTC date>.json`, the name the "save as" dialog proposes.
pub fn export_file_name(api: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    format!("kalamar-export-{}-{}.json", host_for_file(api), now.format("%Y-%m-%d"))
}

/// Asks the server for the export and returns it as the bytes of a JSON file (indented, for a person to read: the data
/// is the answer's, nothing is added or removed). An answer that is not a JSON object is refused.
pub async fn export_document(client: &Client) -> Result<Vec<u8>, CallError> {
    let r = client.call_limited("GET", "/v1/me/export", Body::None, true, MAX_EXPORT).await?;
    if !r.body.is_object() {
        return Err(CallError::Body("the export is not a JSON object".into()));
    }
    let mut bytes = serde_json::to_vec_pretty(&r.body).map_err(|e| CallError::Body(format!("the export cannot be written: {e}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Writes the export where the player chose (through a temporary file beside it, so that a failure leaves no half
/// file under the chosen name).
pub fn write_export(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| "no file name".to_string())?;
    let tmp = path.with_file_name(format!("{name}.part"));
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// The name typed in the confirmation is the account's display name (surrounding spaces aside). An empty name never
/// matches: an account whose name is unknown here cannot be erased by pressing a button.
pub fn name_matches(typed: &str, display_name: &str) -> bool {
    let (t, d) = (typed.trim(), display_name.trim());
    !d.is_empty() && t == d
}

/// Erases the account on the server, then forgets the session here: the server has already ended every session of the
/// account, so no logout request follows. A refusal (a match in play, a rate limit, a lost network) changes nothing.
pub async fn erase(client: &Client) -> Result<(), CallError> {
    let r = client.call("DELETE", "/v1/me?confirm=erase", Body::None, true).await?;
    let _: Value = r.body;
    client.forget_session();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, TcpListener};
    use std::sync::{Arc, Mutex};

    /// A stand-in service on loopback: it notes every request line and its `Authorization` header, answers the export
    /// with `export`, and the erasure with `erase_status` (204, or an error of the protocol's shape).
    struct Api {
        port: u16,
        seen: Arc<Mutex<Vec<(String, String, String)>>>,
    }

    fn start(export: String, erase_status: u16) -> Api {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { continue };
                let mut buf = Vec::new();
                let mut b = [0u8; 4096];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut b) {
                        Ok(0) | Err(_) => break,
                        Ok(k) => buf.extend_from_slice(&b[..k]),
                    }
                }
                let head = String::from_utf8_lossy(&buf).to_string();
                let mut first = head.lines().next().unwrap_or("").split(' ');
                let (method, path) = (first.next().unwrap_or("").to_string(), first.next().unwrap_or("").to_string());
                let auth = head.lines().find_map(|l| l.strip_prefix("authorization: ").or_else(|| l.strip_prefix("Authorization: "))).unwrap_or("").to_string();
                s2.lock().unwrap().push((method.clone(), path.clone(), auth));
                let (status, text) = match (method.as_str(), path.as_str()) {
                    ("GET", "/v1/me/export") => (200, export.clone()),
                    ("DELETE", "/v1/me?confirm=erase") if erase_status == 204 => (204, String::new()),
                    ("DELETE", "/v1/me?confirm=erase") => (erase_status, json!({"error": {"code": "conflict", "message": "a match is in play"}}).to_string()),
                    _ => (404, json!({"error": {"code": "not_found", "message": "no such route"}}).to_string()),
                };
                let resp = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len());
                let _ = s.write_all(resp.as_bytes());
            }
        });
        Api { port, seen }
    }

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("s3l-account-{tag}-{}", crate::util::random_hex(4)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn logged_in(api: &Api, tag: &str) -> Client {
        let c = Client::new("srv", &format!("http://127.0.0.1:{}", api.port), tmp(tag), "en").unwrap();
        c.accept_session(&json!({"access_token": "acc-token", "refresh_token": "ref-token", "me": {"id": "u_1", "display_name": "Ana"}}), "guest").unwrap();
        c
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    #[test]
    fn the_proposed_file_name_carries_the_host_and_the_utc_date_and_nothing_a_path_could_use() {
        let day = chrono::DateTime::parse_from_rfc3339("2026-11-14T23:59:59Z").unwrap().with_timezone(&chrono::Utc);
        assert_eq!(export_file_name("https://Play.Example.org/api", day), "kalamar-export-play.example.org-2026-11-14.json");
        assert_eq!(export_file_name("http://127.0.0.1:8080/api", day), "kalamar-export-127.0.0.1-2026-11-14.json");
        assert_eq!(export_file_name("not an address", day), "kalamar-export-server-2026-11-14.json");
        let odd = export_file_name("https://xn--caf-dma.example/api", day);
        assert!(!odd.contains(['/', '\\', ':', ' ']) && !odd.contains(".."), "{odd}");
    }

    #[test]
    fn the_confirmation_wants_the_exact_display_name() {
        assert!(name_matches("Ana", "Ana"));
        assert!(name_matches("  Ana ", "Ana"));
        assert!(!name_matches("ana", "Ana"));
        assert!(!name_matches("An", "Ana"));
        assert!(!name_matches("", ""));
        assert!(!name_matches("  ", " "));
    }

    #[test]
    fn the_export_is_asked_with_the_session_and_written_as_a_json_file() {
        let api = start(json!({"account": {"id": "u_1", "display_name": "Ana"}, "matches": []}).to_string(), 204);
        let c = logged_in(&api, "export");
        let bytes = rt().block_on(export_document(&c)).unwrap();
        let back: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back["account"]["id"], "u_1");
        assert_eq!(api.seen.lock().unwrap().as_slice(), [("GET".to_string(), "/v1/me/export".to_string(), "Bearer acc-token".to_string())]);
        let dir = tmp("write");
        let file = dir.join("kalamar-export-test.json");
        write_export(&file, &bytes).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
        assert!(!dir.join("kalamar-export-test.json.part").exists());
        assert!(c.session().is_some(), "an export leaves the session alone");
    }

    #[test]
    fn an_export_that_is_not_an_object_is_refused() {
        let api = start("[1, 2, 3]".into(), 204);
        let c = logged_in(&api, "notobject");
        assert!(matches!(rt().block_on(export_document(&c)), Err(CallError::Body(_))));
    }

    #[test]
    fn erasing_asks_with_the_confirmation_and_forgets_the_session_without_a_logout_request() {
        let api = start("{}".into(), 204);
        let c = logged_in(&api, "erase");
        rt().block_on(erase(&c)).unwrap();
        assert!(c.session().is_none());
        assert!(super::super::tokens::load(&c.dir).is_none(), "the stored tokens are deleted");
        assert_eq!(api.seen.lock().unwrap().as_slice(), [("DELETE".to_string(), "/v1/me?confirm=erase".to_string(), "Bearer acc-token".to_string())]);
    }

    #[test]
    fn a_refused_erasure_keeps_the_session() {
        let api = start("{}".into(), 409);
        let c = logged_in(&api, "refused");
        let e = rt().block_on(erase(&c)).unwrap_err();
        assert_eq!(e.status(), 409);
        assert!(c.session().is_some());
    }

    #[test]
    fn nothing_is_sent_without_a_session() {
        let api = start("{}".into(), 204);
        let c = Client::new("srv", &format!("http://127.0.0.1:{}", api.port), tmp("nosession"), "en").unwrap();
        assert_eq!(rt().block_on(export_document(&c)).unwrap_err().status(), 401);
        assert_eq!(rt().block_on(erase(&c)).unwrap_err().status(), 401);
        assert!(api.seen.lock().unwrap().is_empty());
    }
}
