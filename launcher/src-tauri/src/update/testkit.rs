//! Test helpers (compiled for tests only): a minisign test key made by the tests themselves (never a real key), signed
//! manifests, and a small HTTP/1.1 server on the loopback interface that plays GitHub's release API and its download
//! hosts (ETag, ranges, redirects, a connection cut in the middle of a file).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// A key pair generated for one test run (unencrypted, in memory only).
pub struct TestKey {
    pk: minisign::PublicKey,
    sk: minisign::SecretKey,
}

impl TestKey {
    pub fn generate() -> TestKey {
        let kp = minisign::KeyPair::generate_unencrypted_keypair().expect("test key");
        TestKey { pk: kp.pk, sk: kp.sk }
    }

    pub fn public_b64(&self) -> String {
        self.pk.to_base64()
    }

    pub fn public(&self) -> minisign_verify::PublicKey {
        minisign_verify::PublicKey::from_base64(&self.public_b64()).expect("own key")
    }

    /// This key's bytes under another key id (a forged id).
    pub fn public_with_id_of(&self, id: &[u8]) -> minisign_verify::PublicKey {
        let mut b = self.pk.to_bytes();
        b[2..10].copy_from_slice(id);
        let k = minisign::PublicKey::from_bytes(&b).expect("bytes");
        minisign_verify::PublicKey::from_base64(&k.to_base64()).expect("forged key")
    }

    /// The key id a signature names.
    pub fn id_of(sig: &[u8]) -> Vec<u8> {
        minisign::SignatureBox::from_string(std::str::from_utf8(sig).expect("utf8")).expect("signature").keynum().to_vec()
    }

    /// The `.minisig` text of `data`, as `minisign -S` writes it (prehashed).
    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        let sb = minisign::sign(Some(&self.pk), &self.sk, std::io::Cursor::new(data), Some("timestamp:0\tfile:kalamar-release.json\thashed"), None).expect("sign");
        sb.into_string().into_bytes()
    }
}

/// A manifest as the release tool writes it (compact JSON): `(name, kind, size, sha256)` per file.
pub fn manifest_json(repo: &str, version: &str, channel: &str, files: &[(&str, &str, u64, &str)]) -> Vec<u8> {
    let files: Vec<_> = files.iter().map(|(n, k, s, h)| serde_json::json!({"name": n, "kind": k, "size": s, "sha256": h})).collect();
    serde_json::to_vec(&serde_json::json!({
        "format": 1, "product": "kalamar", "repo": repo, "version": version, "channel": channel,
        "published": "2026-11-12T10:00:00Z", "notes": format!("Kalamar {version}\n- test notes"), "files": files,
    }))
    .expect("json")
}

pub fn sha256_hex(data: &[u8]) -> String {
    crate::installer::bundle::sha256_hex(data)
}

#[derive(Clone, Default)]
pub struct Route {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub etag: Option<String>,
    /// Answers `Range: bytes=N-` with 206.
    pub ranges: bool,
    /// Sends only this many body bytes (with the full Content-Length) and closes: a connection cut mid-file. Used once.
    pub cut_after: Option<usize>,
}

impl Route {
    pub fn ok(body: impl Into<Vec<u8>>) -> Route {
        Route { status: 200, body: body.into(), ..Default::default() }
    }
    pub fn json(v: serde_json::Value) -> Route {
        Route { status: 200, body: v.to_string().into_bytes(), headers: vec![("Content-Type".into(), "application/json".into())], ..Default::default() }
    }
    pub fn redirect(to: &str) -> Route {
        Route { status: 302, headers: vec![("Location".into(), to.into())], ..Default::default() }
    }
    pub fn status(code: u16) -> Route {
        Route { status: code, ..Default::default() }
    }
}

#[derive(Debug, Clone)]
pub struct Seen {
    pub path: String,
    pub headers: Vec<(String, String)>,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// The loopback server. Routes are exact paths (query included); anything else is a 404.
#[derive(Clone)]
pub struct FakeServer {
    pub port: u16,
    routes: Arc<Mutex<HashMap<String, Route>>>,
    pub seen: Arc<Mutex<Vec<Seen>>>,
}

impl FakeServer {
    pub fn start() -> FakeServer {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = l.local_addr().expect("addr").port();
        let s = FakeServer { port, routes: Arc::default(), seen: Arc::default() };
        let s2 = s.clone();
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                let s3 = s2.clone();
                std::thread::spawn(move || s3.serve(c));
            }
        });
        s
    }

    pub fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn route(&self, path: &str, r: Route) {
        self.routes.lock().unwrap().insert(path.to_string(), r);
    }

    pub fn requests(&self, path: &str) -> Vec<Seen> {
        self.seen.lock().unwrap().iter().filter(|s| s.path == path).cloned().collect()
    }

    fn serve(&self, mut c: TcpStream) {
        let _ = c.set_read_timeout(Some(std::time::Duration::from_secs(5)));
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16 * 1024 {
            match c.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
        let text = String::from_utf8_lossy(&buf).to_string();
        let mut lines = text.split("\r\n");
        let first = lines.next().unwrap_or_default();
        let path = first.split(' ').nth(1).unwrap_or("/").to_string();
        let headers: Vec<(String, String)> =
            lines.take_while(|l| !l.is_empty()).filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
        let seen = Seen { path: path.clone(), headers };
        self.seen.lock().unwrap().push(seen.clone());
        let route = {
            let mut routes = self.routes.lock().unwrap();
            let r = routes.get(&path).cloned();
            // a cut happens once: the next request gets the whole file
            if let Some(r) = routes.get_mut(&path) {
                r.cut_after = None;
            }
            r
        };
        let Some(r) = route else {
            let _ = c.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            return;
        };
        let mut status = r.status;
        let mut extra: Vec<(String, String)> = r.headers.clone();
        let mut body = r.body.clone();
        if let Some(e) = &r.etag {
            extra.push(("ETag".into(), e.clone()));
            if seen.header("If-None-Match") == Some(e.as_str()) {
                status = 304;
                body.clear();
            }
        }
        if r.ranges && status == 200 {
            extra.push(("Accept-Ranges".into(), "bytes".into()));
            if let Some(start) = seen.header("Range").and_then(|v| v.strip_prefix("bytes=")).and_then(|v| v.strip_suffix('-')).and_then(|v| v.parse::<usize>().ok())
                && start < body.len()
            {
                status = 206;
                extra.push(("Content-Range".into(), format!("bytes {start}-{}/{}", body.len() - 1, body.len())));
                body = body[start..].to_vec();
            }
        }
        let mut head = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
        for (k, v) in &extra {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("\r\n");
        let _ = c.write_all(head.as_bytes());
        let send = r.cut_after.map(|n| n.min(body.len())).unwrap_or(body.len());
        let _ = c.write_all(&body[..send]);
        let _ = c.flush();
        if r.cut_after.is_some() {
            let _ = c.shutdown(std::net::Shutdown::Both);
        }
    }
}
