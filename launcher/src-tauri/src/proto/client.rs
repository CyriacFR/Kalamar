//! One community server as this launcher talks to it: its API base URL, the session (tokens, refreshed and rotated
//! as `api.md` "Tokens" says), the cached meta document and the server's clock. Every call goes through
//! [`Client::call`], which adds the token, refreshes it once on `401 token_expired`, waits on `429` and forgets a
//! session the server revoked.

use crate::proto::http::{self, Body, CallError, Reply};
use crate::proto::meta::{self, MetaCheck};
use crate::proto::tokens::{self, Session};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the page may know of a session (never a token).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionInfo {
    pub logged_in: bool,
    pub provider: Option<String>,
    pub user_id: Option<String>,
    pub display_name: Option<String>,
}

/// Told when a session ends without the player asking (revoked, refresh refused).
pub type SessionLost = Arc<dyn Fn(&str, &str) + Send + Sync>;

pub struct Client {
    pub server_id: String,
    pub api: String,
    /// `online\<server id>` in the launcher's data folder.
    pub dir: PathBuf,
    http: reqwest::Client,
    lang: Mutex<String>,
    session: Mutex<Option<Session>>,
    refresh_gate: tokio::sync::Mutex<()>,
    meta: Mutex<Option<(Value, MetaCheck)>>,
    /// The address the last meta came from (where the server is, for `relayping::allowed`).
    peer: Mutex<Option<std::net::SocketAddr>>,
    /// Server clock minus local clock, seconds.
    offset_s: AtomicI64,
    on_lost: Mutex<Option<SessionLost>>,
    /// What the player did from this launcher on this server during this run (`consent`): every successful request is
    /// noted there, and the match runner reads it before it starts the game.
    pub consent: Arc<crate::proto::consent::Consent>,
}

fn now_unix() -> i64 {
    crate::util::now_secs() as i64
}

impl Client {
    pub fn new(server_id: &str, api: &str, dir: PathBuf, lang: &str) -> Result<Client, String> {
        let session = tokens::load(&dir);
        Ok(Client {
            server_id: server_id.to_string(),
            api: api.trim_end_matches('/').to_string(),
            http: http::build_client(Duration::from_secs(20))?,
            lang: Mutex::new(lang.to_string()),
            session: Mutex::new(session),
            refresh_gate: tokio::sync::Mutex::new(()),
            meta: Mutex::new(None),
            peer: Mutex::new(None),
            offset_s: AtomicI64::new(0),
            on_lost: Mutex::new(None),
            consent: Arc::new(crate::proto::consent::Consent::new()),
            dir,
        })
    }

    pub fn set_lang(&self, lang: &str) {
        *self.lang.lock().unwrap_or_else(|p| p.into_inner()) = lang.to_string();
    }

    pub fn on_session_lost(&self, f: SessionLost) {
        *self.on_lost.lock().unwrap_or_else(|p| p.into_inner()) = Some(f);
    }

    fn lang(&self) -> String {
        self.lang.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn session(&self) -> Option<Session> {
        self.session.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn session_info(&self) -> SessionInfo {
        match self.session() {
            Some(s) => SessionInfo { logged_in: true, provider: Some(s.provider), user_id: Some(s.user_id), display_name: Some(s.display_name) },
            None => SessionInfo { logged_in: false, provider: None, user_id: None, display_name: None },
        }
    }

    pub fn access_token(&self) -> Option<String> {
        self.session().map(|s| s.access_token)
    }

    fn set_session(&self, s: Option<Session>) {
        match &s {
            Some(s) => {
                let _ = tokens::save(&self.dir, s);
            }
            None => tokens::clear(&self.dir),
        }
        *self.session.lock().unwrap_or_else(|p| p.into_inner()) = s;
    }

    /// The server revoked the session or refused its refresh: it is forgotten, and the page is told.
    pub fn drop_session(&self, reason: &str) {
        if self.session().is_none() {
            return;
        }
        self.set_session(None);
        if let Some(f) = self.on_lost.lock().unwrap_or_else(|p| p.into_inner()).clone() {
            f(&self.server_id, reason);
        }
    }

    /// Keeps the name the server knows (`me` of an answer) in the stored session.
    pub fn remember_me(&self, me: &Value) {
        let mut g = self.session.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(s) = g.as_mut() {
            let name = me.get("display_name").and_then(Value::as_str).unwrap_or_default();
            let id = me.get("id").and_then(Value::as_str).unwrap_or_default();
            if (!name.is_empty() && s.display_name != name) || (!id.is_empty() && s.user_id != id) {
                if !name.is_empty() {
                    s.display_name = name.to_string();
                }
                if !id.is_empty() {
                    s.user_id = id.to_string();
                }
                let _ = tokens::save(&self.dir, s);
            }
        }
    }

    /// The server's clock now (Unix seconds), from the offset measured on the last meta.
    pub fn server_now(&self) -> i64 {
        now_unix() + self.offset_s.load(Ordering::Relaxed)
    }

    pub fn offset_s(&self) -> i64 {
        self.offset_s.load(Ordering::Relaxed)
    }

    /// The address the last meta came from, when known.
    pub fn peer(&self) -> Option<std::net::SocketAddr> {
        *self.peer.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn cached_meta(&self) -> Option<(Value, MetaCheck)> {
        self.meta.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// `GET /v1/meta` (public), checked and cached; also measures the server's clock.
    pub async fn fetch_meta(&self) -> Result<(Value, MetaCheck), CallError> {
        let r = http::send(&self.http, "GET", &crate::proto::url::join(&self.api, "/v1/meta"), None, &Body::None, &self.lang(), http::MAX_ANSWER).await?;
        let check = meta::check(&r.body);
        if let Some(t) = check.time.as_deref().and_then(tokens::parse_time) {
            self.offset_s.store(t - now_unix(), Ordering::Relaxed);
        }
        *self.peer.lock().unwrap_or_else(|p| p.into_inner()) = r.peer;
        let out = (r.body, check);
        *self.meta.lock().unwrap_or_else(|p| p.into_inner()) = Some(out.clone());
        Ok(out)
    }

    /// The declared areas (from the cached meta; empty before it is fetched).
    pub fn features(&self) -> Vec<String> {
        self.cached_meta().map(|(_, c)| c.features).unwrap_or_default()
    }

    /// A request to `<api><path>`. `auth`: with the session's token (refreshed first when it is about to expire, and
    /// once more on `401 token_expired`). Waits and retries on `429` (at most twice, as long as `Retry-After` asks).
    pub async fn call(&self, method: &str, path: &str, body: Body, auth: bool) -> Result<Reply, CallError> {
        self.call_limited(method, path, body, auth, http::MAX_ANSWER).await
    }

    pub async fn call_limited(&self, method: &str, path: &str, body: Body, auth: bool, max: usize) -> Result<Reply, CallError> {
        let url = crate::proto::url::join(&self.api, path);
        let mut refreshed = false;
        let mut waits = 0;
        loop {
            let token = if auth {
                let s = self.session().ok_or_else(|| CallError::Api(http::parse_error(401, &json!({"error": {"code": "unauthorized", "message": "not logged in"}}), None)))?;
                if !refreshed && s.access_expiring(self.server_now(), 30) {
                    refreshed = true;
                    let _ = self.refresh(&s.access_token).await;
                }
                self.access_token()
            } else {
                None
            };
            if auth && token.is_none() {
                return Err(CallError::Api(http::parse_error(401, &json!({"error": {"code": "session_revoked", "message": "the session ended"}}), None)));
            }
            match http::send(&self.http, method, &url, token.as_deref(), &body, &self.lang(), max).await {
                Err(CallError::Api(e)) if e.status == 401 && auth => {
                    if e.code == "token_expired" && !refreshed {
                        refreshed = true;
                        self.refresh(token.as_deref().unwrap_or_default()).await?;
                        continue;
                    }
                    self.drop_session(&e.code);
                    return Err(CallError::Api(e));
                }
                Err(CallError::Api(e)) if e.status == 429 && waits < 2 && e.retry_after_s.unwrap_or(1) <= 10 => {
                    waits += 1;
                    tokio::time::sleep(Duration::from_secs(e.retry_after_s.unwrap_or(1).max(1))).await;
                    continue;
                }
                Ok(r) => {
                    self.consent.note_request(method, path, &r.body);
                    return Ok(r);
                }
                other => return other,
            }
        }
    }

    /// `POST /v1/auth/refresh` with rotation: the new pair replaces the old one. `seen` is the access token the caller
    /// had: when another task refreshed meanwhile, nothing is sent. A refusal (401) ends the session; a network error
    /// keeps it for later.
    pub async fn refresh(&self, seen: &str) -> Result<(), CallError> {
        let _gate = self.refresh_gate.lock().await;
        let Some(s) = self.session() else {
            return Err(CallError::Api(http::parse_error(401, &json!({"error": {"code": "session_revoked", "message": "the session ended"}}), None)));
        };
        if s.access_token != seen {
            return Ok(());
        }
        let r = http::send(&self.http, "POST", &crate::proto::url::join(&self.api, "/v1/auth/refresh"), None, &Body::Json(json!({"refresh_token": s.refresh_token})), &self.lang(), http::MAX_ANSWER).await;
        match r {
            Ok(r) => {
                let next = Session::from_answer(&r.body, &s.provider).ok_or_else(|| CallError::Body("the refresh answer has no tokens".into()))?;
                self.set_session(Some(next));
                Ok(())
            }
            Err(CallError::Api(e)) if e.status == 401 || e.status == 400 => {
                self.drop_session(&e.code);
                Err(CallError::Api(e))
            }
            Err(e) => Err(e),
        }
    }

    /// `POST /v1/auth/{provider}/login` of a direct provider. Returns `me`.
    pub async fn login_direct(&self, provider: &str, fields: Value) -> Result<Value, CallError> {
        if !meta::valid_id(provider) {
            return Err(CallError::Body("unknown provider".into()));
        }
        let r = self.call("POST", &format!("/v1/auth/{provider}/login"), Body::Json(fields), false).await?;
        self.accept_session(&r.body, provider)
    }

    /// Stores the session of a login answer (direct or OAuth).
    pub fn accept_session(&self, answer: &Value, provider: &str) -> Result<Value, CallError> {
        let s = Session::from_answer(answer, provider).ok_or_else(|| CallError::Body("the login answer has no tokens".into()))?;
        self.set_session(Some(s));
        Ok(answer.get("me").cloned().unwrap_or(Value::Null))
    }

    /// `POST /v1/auth/logout` (best effort), then the session is forgotten here whatever the answer.
    pub async fn logout(&self, all: bool) {
        if self.session().is_some() {
            let _ = tokio::time::timeout(Duration::from_secs(8), self.call("POST", "/v1/auth/logout", Body::Json(json!({"all": all})), true)).await;
        }
        self.set_session(None);
    }

    /// Forgets the session here without asking the server anything and without the "session lost" notice: the
    /// account was just erased at the player's request, which ended its sessions on the server.
    pub fn forget_session(&self) {
        self.set_session(None);
    }

    /// `PUT /v1/me/game`: what this launcher and its engine can run (`engine::game_body`).
    pub async fn put_game(&self, body: Value) -> Result<(), CallError> {
        self.call_limited("PUT", "/v1/me/game", Body::Json(body), true, http::MAX_ANSWER).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_session_means_no_authenticated_call() {
        let d = std::env::temp_dir().join(format!("s3l-cl-{}", crate::util::random_hex(4)));
        let c = Client::new("0a1b2c3d", "https://play.example.invalid/api/", d.clone(), "fr").unwrap();
        assert_eq!(c.api, "https://play.example.invalid/api");
        assert!(!c.session_info().logged_in);
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let e = rt.block_on(c.call("GET", "/v1/me", Body::None, true)).unwrap_err();
        assert_eq!((e.status(), e.code()), (401, "unauthorized"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
