//! The event WebSocket (`events.md`): one per server while the player is logged in. Authenticated with the bearer
//! token on the upgrade (a ticket from `POST /v1/events/ticket` when that is refused, for proxies that drop the
//! header); answers the server's pings; considers the connection dead after 3 heartbeats of silence; reconnects with
//! exponential backoff (1 s doubling to 30 s, with jitter) and resumes the stream (`resume` + `last_seq`). When the
//! server cannot resume, the owner is told to refetch `GET /v1/state`. A server that closes the connection because of
//! this client's own frames (1008: the reference server allows 20 a second, 100 at once, since 1.1.12) is not answered
//! with a reconnection after 1 s: the wait keeps doubling until a connection ends otherwise.

use crate::proto::client::Client;
use crate::proto::http::{self, Body};
use crate::proto::url as purl;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// The largest event this launcher reads (the protocol's events are much smaller).
pub const MAX_EVENT: usize = 64 * 1024;

/// Where the stream stands, for the page.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StreamInfo {
    /// `connecting`, `open`, `retrying` (after a failure, waiting `retry_in_s`), `stopped` (logged out, revoked,
    /// banned, replaced or stopped by the launcher).
    pub state: String,
    pub resumed: bool,
    pub retry_in_s: Option<u64>,
    /// Why it stopped or the last failure, in English for the details line.
    pub reason: Option<String>,
    pub last_seq: u64,
}

impl StreamInfo {
    fn new(state: &str) -> Self {
        StreamInfo { state: state.into(), resumed: false, retry_in_s: None, reason: None, last_seq: 0 }
    }
}

/// What the stream delivers to its owner.
pub trait Handler: Send + Sync {
    /// An event (`{"seq", "type", "at", "data"}`), in order, each once.
    fn event(&self, ev: &Value);
    /// The stream (re)started without the events missed meanwhile: refetch `GET /v1/state`.
    fn resync(&self);
    fn state(&self, info: &StreamInfo);
}

pub struct Stream {
    stop: watch::Sender<bool>,
    kick: watch::Sender<u64>,
    info: Arc<Mutex<StreamInfo>>,
}

impl Stream {
    pub fn info(&self) -> StreamInfo {
        self.info.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
    pub fn stop(&self) {
        let _ = self.stop.send(true);
    }
    /// Drops the current connection; the stream reconnects and resumes (the page's "reconnect", and tests).
    pub fn reconnect(&self) {
        self.kick.send_modify(|n| *n += 1);
    }
    pub fn stopped(&self) -> bool {
        self.info().state == "stopped"
    }
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

type Ws = tokio_tungstenite::WebSocketStream<Box<dyn Io>>;

enum ConnectError {
    /// The upgrade was refused with this HTTP status and protocol code.
    Refused(u16, String, Option<u64>),
    Other(String),
}

async fn connect(client: &Client, query: &str, bearer: Option<&str>) -> Result<Ws, ConnectError> {
    http::install_crypto();
    let url = purl::websocket_url(&client.api, &format!("/v1/events{query}"));
    let mut req = url.as_str().into_client_request().map_err(|e| ConnectError::Other(e.to_string()))?;
    {
        let h = req.headers_mut();
        h.insert("kalamar-protocol", http::PROTOCOL_VERSION.parse().map_err(|_| ConnectError::Other("header".into()))?);
        h.insert("user-agent", http::USER_AGENT.parse().map_err(|_| ConnectError::Other("header".into()))?);
        if let Some(t) = bearer {
            h.insert("authorization", format!("Bearer {t}").parse().map_err(|_| ConnectError::Other("token".into()))?);
        }
    }
    let tls = url.starts_with("wss://");
    let host = req.uri().host().ok_or_else(|| ConnectError::Other("no host".into()))?.trim_matches(['[', ']']).to_string();
    if !tls && !purl::is_loopback(&host) {
        return Err(ConnectError::Other("plain ws:// is only allowed to this computer".into()));
    }
    let port = req.uri().port_u16().unwrap_or(if tls { 443 } else { 80 });
    let tcp = tokio::time::timeout(Duration::from_secs(10), tokio::net::TcpStream::connect((host.as_str(), port)))
        .await
        .map_err(|_| ConnectError::Other("connection timed out".into()))?
        .map_err(|e| ConnectError::Other(e.to_string()))?;
    let io: Box<dyn Io> = if tls {
        use rustls_platform_verifier::ConfigVerifierExt;
        let cfg = rustls::ClientConfig::with_platform_verifier().map_err(|e| ConnectError::Other(e.to_string()))?;
        let name = rustls::pki_types::ServerName::try_from(host.clone()).map_err(|e| ConnectError::Other(e.to_string()))?;
        let s = tokio_rustls::TlsConnector::from(Arc::new(cfg)).connect(name, tcp).await.map_err(|e| ConnectError::Other(format!("TLS: {e}")))?;
        Box::new(s)
    } else {
        Box::new(tcp)
    };
    // An event is at most 64 KiB (`MAX_EVENT`): a larger frame or message ends the connection as it arrives, instead of
    // being received whole (the library's defaults are 16 and 64 MiB) and dropped afterwards.
    let limits = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(MAX_EVENT)).max_frame_size(Some(MAX_EVENT));
    match tokio::time::timeout(Duration::from_secs(10), tokio_tungstenite::client_async_with_config(req, io, Some(limits))).await {
        Err(_) => Err(ConnectError::Other("the upgrade timed out".into())),
        Ok(Ok((ws, _))) => Ok(ws),
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(r))) => {
            let status = r.status().as_u16();
            let body = r.body().as_ref().and_then(|b| serde_json::from_slice::<Value>(b).ok()).unwrap_or(Value::Null);
            let retry = r.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
            let e = http::parse_error(status, &body, retry);
            Err(ConnectError::Refused(status, e.code, e.retry_after_s))
        }
        Ok(Err(e)) => Err(ConnectError::Other(e.to_string())),
    }
}

fn jitter_ms(max: u64) -> u64 {
    let mut b = [0u8; 2];
    let _ = getrandom::fill(&mut b);
    u16::from_le_bytes(b) as u64 % max.max(1)
}

/// Starts the stream on the current tokio runtime. It runs until [`Stream::stop`] or until the session ends.
pub fn spawn(client: Arc<Client>, handler: Arc<dyn Handler>) -> Stream {
    let (tx, rx) = watch::channel(false);
    let (kick, kick_rx) = watch::channel(0u64);
    let info = Arc::new(Mutex::new(StreamInfo::new("connecting")));
    let task_info = info.clone();
    tokio::spawn(run(client, handler, rx, kick_rx, task_info));
    Stream { stop: tx, kick, info }
}

fn set(info: &Mutex<StreamInfo>, handler: &dyn Handler, f: impl FnOnce(&mut StreamInfo)) {
    let snapshot = {
        let mut g = info.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut g);
        g.clone()
    };
    handler.state(&snapshot);
}

async fn run(client: Arc<Client>, handler: Arc<dyn Handler>, mut stop: watch::Receiver<bool>, mut kick: watch::Receiver<u64>, info: Arc<Mutex<StreamInfo>>) {
    let mut resume: Option<(String, u64)> = None;
    let mut backoff = 1u64;
    let mut use_ticket = false;
    let mut refreshed = false;
    let mut first = true;
    let stopped = |info: &Mutex<StreamInfo>, handler: &dyn Handler, reason: &str| {
        set(info, handler, |i| {
            i.state = "stopped".into();
            i.reason = Some(reason.into());
            i.retry_in_s = None;
        })
    };
    loop {
        if *stop.borrow() {
            stopped(&info, handler.as_ref(), "stopped");
            return;
        }
        let Some(token) = client.access_token() else {
            stopped(&info, handler.as_ref(), "logged_out");
            return;
        };
        set(&info, handler.as_ref(), |i| {
            i.state = "connecting".into();
            i.retry_in_s = None;
        });
        let mut q: Vec<String> = Vec::new();
        if let Some((s, n)) = &resume {
            q.push(format!("resume={}&last_seq={n}", purl::query_escape(s)));
        }
        let bearer = if use_ticket {
            match client.call("POST", "/v1/events/ticket", Body::Json(json!({})), true).await {
                Ok(r) => match r.body.get("ticket").and_then(Value::as_str) {
                    Some(t) => {
                        q.push(format!("ticket={}", purl::query_escape(t)));
                        None
                    }
                    None => Some(token.clone()),
                },
                Err(e) if e.status() == 401 => {
                    stopped(&info, handler.as_ref(), "logged_out");
                    return;
                }
                Err(_) => Some(token.clone()),
            }
        } else {
            Some(token.clone())
        };
        let query = if q.is_empty() { String::new() } else { format!("?{}", q.join("&")) };
        let failure: String;
        match connect(&client, &query, bearer.as_deref()).await {
            Ok(ws) => {
                refreshed = false;
                match session(&client, ws, handler.as_ref(), &info, &mut stop, &mut kick, &mut resume, first).await {
                    End::Stop(reason) => {
                        stopped(&info, handler.as_ref(), &reason);
                        return;
                    }
                    End::Lost(why) => {
                        first = false;
                        backoff = 1;
                        failure = why;
                    }
                    // The server cut this connection for its own frames: the wait keeps growing (no reset to 1 s).
                    End::Throttled(why) => {
                        first = false;
                        failure = why;
                    }
                }
            }
            Err(ConnectError::Refused(401, code, _)) if code == "token_expired" && !refreshed => {
                refreshed = true;
                let _ = client.refresh(&token).await;
                continue;
            }
            Err(ConnectError::Refused(401, code, _)) => {
                if !use_ticket && code == "unauthorized" {
                    // A proxy may drop the Authorization header of an upgrade: try a ticket once.
                    use_ticket = true;
                    continue;
                }
                client.drop_session(&code);
                stopped(&info, handler.as_ref(), &code);
                return;
            }
            Err(ConnectError::Refused(403, code, _)) => {
                stopped(&info, handler.as_ref(), &code);
                return;
            }
            Err(ConnectError::Refused(status, code, retry)) => {
                failure = format!("{status} {code}");
                if let Some(r) = retry {
                    backoff = backoff.max(r.min(300));
                }
            }
            Err(ConnectError::Other(m)) => failure = m,
        }
        let wait_ms = backoff * 1000 + jitter_ms(backoff * 300);
        set(&info, handler.as_ref(), |i| {
            i.state = "retrying".into();
            i.retry_in_s = Some(wait_ms.div_ceil(1000));
            i.reason = Some(failure.clone());
        });
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(wait_ms)) => {}
            _ = stop.changed() => {}
        }
        backoff = (backoff * 2).min(30);
    }
}

enum End {
    /// Do not reconnect (the reason is shown).
    Stop(String),
    /// Reconnect with backoff (the reason is shown while waiting).
    Lost(String),
    /// Reconnect with a backoff that keeps growing: the server closed the connection because of this client's
    /// frames (1008, "policy violation"; the reference server's limit of 1.1.12 on acks and pongs).
    Throttled(String),
}

/// The server's close frame as the reason shown while reconnecting: its code and its text (cleaned, 64 characters at
/// most; it comes from the server).
fn closed(frame: Option<&tokio_tungstenite::tungstenite::protocol::CloseFrame>) -> End {
    let Some(f) = frame else { return End::Lost("the server closed the connection".into()) };
    let code = u16::from(f.code);
    let text: String = f.reason.as_str().chars().filter(|c| !c.is_control()).take(64).collect();
    let why = if text.trim().is_empty() {
        format!("the server closed the connection ({code})")
    } else {
        format!("the server closed the connection ({code}: {})", text.trim())
    };
    if code == 1008 { End::Throttled(why) } else { End::Lost(why) }
}

#[allow(clippy::too_many_arguments)]
async fn session(client: &Client, ws: Ws, handler: &dyn Handler, info: &Mutex<StreamInfo>, stop: &mut watch::Receiver<bool>, kick: &mut watch::Receiver<u64>, resume: &mut Option<(String, u64)>, first: bool) -> End {
    let (mut sink, mut source) = ws.split();
    // The first frame is `hello`.
    let hello = match tokio::time::timeout(Duration::from_secs(10), source.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => serde_json::from_str::<Value>(t.as_str()).unwrap_or(Value::Null),
        _ => return End::Lost("no hello".into()),
    };
    if hello.get("type").and_then(Value::as_str) != Some("hello") {
        return End::Lost("the first frame is not hello".into());
    }
    let stream_id = hello.get("stream").and_then(Value::as_str).unwrap_or_default().to_string();
    let resumed = hello.get("resumed").and_then(Value::as_bool).unwrap_or(false);
    let heartbeat = hello.get("heartbeat_s").and_then(Value::as_u64).filter(|h| (1..=600).contains(h)).unwrap_or(20);
    let mut last_seq = if resumed { resume.as_ref().map(|(_, n)| *n).unwrap_or(0) } else { 0 };
    *resume = Some((stream_id.clone(), last_seq));
    set(info, handler, |i| {
        i.state = "open".into();
        i.resumed = resumed;
        i.retry_in_s = None;
        i.reason = None;
        i.last_seq = last_seq;
    });
    if !resumed {
        // A new stream: whatever happened while it was down (or before the first connection) is in GET /v1/state.
        let _ = first;
        handler.resync();
    }
    let dead_after = Duration::from_secs(heartbeat * 3);
    loop {
        let frame = tokio::select! {
            f = tokio::time::timeout(dead_after, source.next()) => f,
            _ = stop.changed() => {
                let _ = sink.send(Message::Close(None)).await;
                return End::Stop("stopped".into());
            }
            _ = kick.changed() => {
                let _ = sink.send(Message::Close(None)).await;
                return End::Lost("reconnecting".into());
            }
        };
        let msg = match frame {
            Err(_) => return End::Lost("no frame for 3 heartbeats".into()),
            Ok(None) => return End::Lost("the server closed the connection".into()),
            Ok(Some(Err(e))) => return End::Lost(e.to_string()),
            Ok(Some(Ok(m))) => m,
        };
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Ping(p) => {
                let _ = sink.send(Message::Pong(p)).await;
                continue;
            }
            Message::Close(frame) => return closed(frame.as_ref()),
            _ => continue,
        };
        if text.len() > MAX_EVENT {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
        match v.get("seq").and_then(Value::as_u64) {
            Some(seq) => {
                if seq <= last_seq {
                    continue; // already processed (a replay after resume)
                }
                last_seq = seq;
                *resume = Some((stream_id.clone(), last_seq));
                handler.event(&v);
                let _ = sink.send(Message::Text(json!({"type": "ack", "seq": seq}).to_string().into())).await;
                if let Ok(mut g) = info.lock() {
                    g.last_seq = last_seq;
                }
                if kind == "session.revoked" {
                    client.drop_session(v.pointer("/data/reason").and_then(Value::as_str).unwrap_or("session_revoked"));
                }
            }
            None => match kind {
                "ping" => {
                    let pong = json!({"type": "pong", "t": v.get("t").cloned().unwrap_or(Value::Null)});
                    if sink.send(Message::Text(pong.to_string().into())).await.is_err() {
                        return End::Lost("send failed".into());
                    }
                }
                "bye" => {
                    let reason = v.get("reason").and_then(Value::as_str).unwrap_or("unknown").to_string();
                    return match reason.as_str() {
                        "session_revoked" | "banned" => {
                            if reason == "session_revoked" {
                                client.drop_session("session_revoked");
                            }
                            End::Stop(reason)
                        }
                        // Another connection of this account took this one's place (the server's limit): reconnecting
                        // would push that one out in turn.
                        "replaced" => End::Stop(reason),
                        _ => End::Lost(format!("bye: {reason}")),
                    };
                }
                _ => {}
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;

    /// A server that closes the stream says why in the details line; a 1008 (the reference server's frame limit)
    /// keeps the reconnection's backoff growing; the server's text is cleaned and bounded.
    #[test]
    fn a_close_frame_gives_its_code_and_a_1008_keeps_backing_off() {
        let frame = |code: u16, reason: &str| CloseFrame { code: code.into(), reason: reason.to_string().into() };
        assert!(matches!(closed(Some(&frame(1008, "too many frames"))), End::Throttled(w) if w == "the server closed the connection (1008: too many frames)"));
        assert!(matches!(closed(Some(&frame(1001, ""))), End::Lost(w) if w == "the server closed the connection (1001)"));
        assert!(matches!(closed(None), End::Lost(w) if w == "the server closed the connection"));
        let long = format!("a\u{7}b\n{}", "x".repeat(200));
        let End::Lost(w) = closed(Some(&frame(1011, &long))) else { panic!("1011 reconnects as usual") };
        assert!(!w.contains('\u{7}') && !w.contains('\n') && w.len() < 120, "{w}");
    }
}
