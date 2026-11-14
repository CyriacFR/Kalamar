//! The presence thread. It alone owns the connection to Discord, so the window and the game never wait for it: the page
//! hands it the latest activity through a channel and goes on. What it does:
//!
//! - nothing at all while the presence is off (it is only started the first time the player switches it on);
//! - takes the machine-wide lock (`ipc::OwnerLock`): a second launcher, or the game started alone, then stays quiet;
//! - looks for Discord by listing the pipe names first (nothing is opened when none exists), then tries
//!   `discord-ipc-0..9` in order: the first that answers the handshake with READY is used; one that stays silent, or
//!   closes, is skipped. No Discord: tried again every 30 s, without any message on screen;
//! - writes an activity only when it changed, after it stayed the same for a moment (`settle`), and never more than
//!   Discord's documented limit (5 updates per 20 s); the latest one wins, the ones in between are never sent;
//! - reads what Discord writes, without ever waiting: a Ping is answered, a Close or a broken pipe means Discord went
//!   (looked for again later), an error answer to an activity is kept for the page and that activity is not resent;
//! - when switched off, and when the launcher quits, clears the presence and closes the pipe.

use super::activity::{self, Activity};
use super::ipc::{self, Conn, Decoder, OwnerLock, OP_CLOSE, OP_FRAME, OP_HANDSHAKE, OP_PING, OP_PONG};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Options {
    pub client_id: String,
    /// `discord-ipc-` (tests: a pipe name of their own, so a Discord running on the machine is never reached).
    pub pipe_prefix: String,
    /// The machine-wide lock; `None` in tests that do not share it.
    pub lock_name: Option<String>,
    /// A Discord that is not running is looked for again this often.
    pub retry: Duration,
    /// After Discord refused the application id, this often.
    pub refused_retry: Duration,
    /// Discord's limit for activity updates (Game SDK, `UpdateActivity`): 5 per 20 seconds.
    pub per_window: usize,
    pub window: Duration,
    /// A change waits this long for the next one before going out.
    pub settle: Duration,
    /// How long Discord may take to answer the handshake.
    pub answer_wait: Duration,
    /// While connected, how often the pipe is looked at.
    pub poll: Duration,
    pub pid: u32,
}

impl Options {
    pub fn discord(client_id: &str) -> Options {
        Options {
            client_id: client_id.to_string(),
            pipe_prefix: ipc::PIPE_PREFIX.to_string(),
            lock_name: Some(super::LOCK_NAME.to_string()),
            retry: Duration::from_secs(30),
            refused_retry: Duration::from_secs(600),
            per_window: 5,
            window: Duration::from_secs(20),
            settle: Duration::from_millis(1500),
            answer_wait: Duration::from_secs(3),
            poll: Duration::from_millis(500),
            pid: std::process::id(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Link {
    /// Switched off: no connection, nothing sent.
    Off,
    /// On, and no Discord answered: looked for again every `retry`.
    Waiting,
    Connected,
    /// Another Kalamar of this PC shows the presence (a second launcher, or the game started alone).
    Busy,
    /// Discord refused the application id (a build with a wrong id).
    Refused,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Status {
    pub link: Link,
    /// Discord's code when it refused the application or the last activity.
    pub code: Option<i64>,
    /// Activities written since the presence was switched on (the page's advanced mode; tests).
    pub sent: u32,
}

impl Status {
    pub const OFF: Status = Status { link: Link::Off, code: None, sent: 0 };
}

pub enum Cmd {
    Enabled(bool),
    Activity(Option<Activity>),
    Stop(Sender<()>),
}

pub type Report = Arc<dyn Fn(&Status) + Send + Sync>;

pub fn spawn(opts: Options, report: Report) -> std::io::Result<(Sender<Cmd>, std::thread::JoinHandle<()>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::Builder::new().name("discord-presence".into()).spawn(move || run(opts, report, rx))?;
    Ok((tx, handle))
}

/// One connection to Discord.
struct Live {
    conn: Box<dyn Conn>,
    dec: Decoder,
    /// What was last written on this connection: `None` nothing yet, `Some(None)` a clear.
    sent: Option<Option<Activity>>,
}

struct W {
    o: Options,
    report: Report,
    enabled: bool,
    desired: Option<Activity>,
    changed: Instant,
    live: Option<Live>,
    lock: Option<OwnerLock>,
    next_try: Instant,
    history: VecDeque<Instant>,
    status: Status,
    nonce: u64,
    party_id: String,
}

enum Hand {
    Ready(Decoder),
    /// Discord closed the handshake with this code (4000: invalid client id).
    Refused(i64),
    Failed,
}

fn run(o: Options, report: Report, rx: Receiver<Cmd>) {
    let now = Instant::now();
    let mut w = W {
        party_id: format!("kalamar-{}", crate::util::random_hex(8)),
        o,
        report,
        enabled: false,
        desired: None,
        changed: now,
        live: None,
        lock: None,
        next_try: now,
        history: VecDeque::new(),
        status: Status::OFF,
        nonce: 0,
    };
    loop {
        let wait = w.wait_time(Instant::now());
        let mut cmds = Vec::new();
        match rx.recv_timeout(wait) {
            Ok(c) => cmds.push(c),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                w.off(Link::Off);
                return;
            }
        }
        while let Ok(c) = rx.try_recv() {
            cmds.push(c);
        }
        for c in cmds {
            match c {
                Cmd::Enabled(on) => {
                    if on != w.enabled {
                        w.enabled = on;
                        if on {
                            w.next_try = Instant::now();
                            w.set_status(Status { link: Link::Waiting, code: None, sent: 0 });
                        } else {
                            w.off(Link::Off);
                            // Switched on again, the page hands over the present activity: an old one is never shown.
                            w.desired = None;
                        }
                    }
                }
                Cmd::Activity(a) => {
                    if a != w.desired {
                        w.desired = a;
                        w.changed = Instant::now();
                    }
                }
                Cmd::Stop(ack) => {
                    w.off(Link::Off);
                    let _ = ack.send(());
                    return;
                }
            }
        }
        if w.enabled {
            w.step(Instant::now());
        }
    }
}

impl W {
    fn set_status(&mut self, s: Status) {
        if s != self.status {
            self.status = s;
            (self.report)(&self.status);
        }
    }

    fn set_link(&mut self, link: Link, code: Option<i64>) {
        let sent = self.status.sent;
        self.set_status(Status { link, code, sent });
    }

    /// Nothing pending: sleep until a command (off), the next attempt (looking for Discord) or the next look at the
    /// pipe; a change waiting for its moment wakes the thread exactly then.
    fn wait_time(&self, now: Instant) -> Duration {
        if !self.enabled {
            return Duration::from_secs(3600);
        }
        let Some(live) = &self.live else {
            return self.next_try.saturating_duration_since(now).max(Duration::from_millis(5));
        };
        let mut wait = self.o.poll;
        if !same(&live.sent, &self.desired) {
            let mut at = self.changed + self.o.settle;
            if self.history.len() >= self.o.per_window
                && let Some(first) = self.history.front()
            {
                at = at.max(*first + self.o.window);
            }
            wait = wait.min(at.saturating_duration_since(now));
        }
        wait.max(Duration::from_millis(5))
    }

    fn step(&mut self, now: Instant) {
        if self.live.is_none() && now >= self.next_try {
            self.connect(now);
        }
        if self.live.is_some() {
            self.poll(now);
        }
        if self.live.is_some() {
            self.maybe_send(now);
        }
    }

    fn connect(&mut self, now: Instant) {
        if self.lock.is_none()
            && let Some(name) = self.o.lock_name.clone()
        {
            match OwnerLock::try_take(&name) {
                Some(l) => self.lock = Some(l),
                None => {
                    self.next_try = now + self.o.retry;
                    self.set_link(Link::Busy, None);
                    return;
                }
            }
        }
        for n in ipc::listed(&self.o.pipe_prefix) {
            let Some(mut conn) = ipc::connect(&format!("{}{n}", self.o.pipe_prefix)) else { continue };
            match handshake(conn.as_mut(), &self.o.client_id, self.o.answer_wait) {
                Hand::Ready(dec) => {
                    self.live = Some(Live { conn, dec, sent: None });
                    self.set_link(Link::Connected, None);
                    return;
                }
                Hand::Refused(4000) => {
                    // The application id itself is refused: no other Discord will take it either.
                    self.next_try = now + self.o.refused_retry;
                    self.lock = None;
                    self.set_link(Link::Refused, Some(4000));
                    return;
                }
                Hand::Refused(_) | Hand::Failed => {}
            }
        }
        self.next_try = Instant::now() + self.o.retry;
        self.set_link(Link::Waiting, None);
    }

    /// Discord went: the connection is dropped and Discord looked for again at the next attempt.
    fn lost(&mut self, now: Instant) {
        self.live = None;
        self.next_try = now + self.o.retry;
        self.set_link(Link::Waiting, None);
    }

    fn poll(&mut self, now: Instant) {
        let Some(live) = self.live.as_mut() else { return };
        let mut buf = [0u8; 4096];
        loop {
            match live.conn.read_now(&mut buf) {
                Ok(0) => break,
                Ok(n) => live.dec.push(&buf[..n]),
                Err(_) => return self.lost(now),
            }
        }
        loop {
            let frame = match live.dec.next_frame() {
                Ok(Some(f)) => f,
                Ok(None) => break,
                Err(_) => return self.lost(now),
            };
            match frame.op {
                OP_PING => {
                    let pong = pong(&frame.raw);
                    if live.conn.write_all(&pong).is_err() {
                        return self.lost(now);
                    }
                }
                OP_CLOSE => return self.lost(now),
                OP_FRAME if frame.body["evt"] == "ERROR" => {
                    // An activity Discord did not take: said on the page, and not sent again until it changes.
                    let code = frame.body["data"]["code"].as_i64();
                    let sent = self.status.sent;
                    self.set_status(Status { link: Link::Connected, code, sent });
                    return;
                }
                _ => {}
            }
        }
    }

    fn maybe_send(&mut self, now: Instant) {
        let Some(live) = self.live.as_mut() else { return };
        if same(&live.sent, &self.desired) {
            return;
        }
        // The first activity of a connection goes at once; a change waits to see whether another follows.
        if live.sent.is_some() && now.saturating_duration_since(self.changed) < self.o.settle {
            return;
        }
        while self.history.front().is_some_and(|t| now.saturating_duration_since(*t) >= self.o.window) {
            self.history.pop_front();
        }
        if self.history.len() >= self.o.per_window {
            return;
        }
        self.nonce += 1;
        let json = activity::set_activity(self.desired.as_ref(), self.o.pid, self.nonce, &self.party_id);
        if live.conn.write_all(&ipc::encode(OP_FRAME, &json)).is_err() {
            return self.lost(now);
        }
        live.sent = Some(self.desired.clone());
        self.history.push_back(now);
        let s = Status { link: Link::Connected, code: None, sent: self.status.sent + 1 };
        self.set_status(s);
    }

    /// Switched off or quitting: the presence is cleared (when one was shown), the pipe closed, the lock given back.
    fn off(&mut self, link: Link) {
        if let Some(mut live) = self.live.take()
            && matches!(live.sent, Some(Some(_)))
        {
            self.nonce += 1;
            let json = activity::set_activity(None, self.o.pid, self.nonce, &self.party_id);
            if live.conn.write_all(&ipc::encode(OP_FRAME, &json)).is_ok() {
                // Discord's answer to that clear (by its nonce), briefly, so it is taken before the pipe closes.
                let nonce = self.nonce.to_string();
                let deadline = Instant::now() + Duration::from_millis(400);
                let mut buf = [0u8; 4096];
                'wait: while Instant::now() < deadline {
                    match live.conn.read_now(&mut buf) {
                        Ok(0) => std::thread::sleep(Duration::from_millis(5)),
                        Ok(n) => live.dec.push(&buf[..n]),
                        Err(_) => break,
                    }
                    while let Ok(Some(f)) = live.dec.next_frame() {
                        if f.op == OP_FRAME && f.body["nonce"] == nonce.as_str() {
                            break 'wait;
                        }
                    }
                }
            }
        }
        self.lock = None;
        self.history.clear();
        self.set_status(Status { link, code: None, sent: 0 });
    }
}

/// The connection already shows `desired` (nothing written yet counts as showing nothing).
fn same(sent: &Option<Option<Activity>>, desired: &Option<Activity>) -> bool {
    match sent {
        None => desired.is_none(),
        Some(s) => s == desired,
    }
}

fn pong(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + raw.len());
    out.extend_from_slice(&OP_PONG.to_le_bytes());
    out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    out.extend_from_slice(raw);
    out
}

/// Writes the handshake and waits (polling) for Discord's READY event.
fn handshake(conn: &mut dyn Conn, client_id: &str, wait: Duration) -> Hand {
    if conn.write_all(&ipc::encode(OP_HANDSHAKE, &activity::handshake(client_id))).is_err() {
        return Hand::Failed;
    }
    let deadline = Instant::now() + wait;
    let mut dec = Decoder::default();
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline {
        match conn.read_now(&mut buf) {
            Ok(0) => std::thread::sleep(Duration::from_millis(5)),
            Ok(n) => dec.push(&buf[..n]),
            Err(_) => return Hand::Failed,
        }
        loop {
            let f = match dec.next_frame() {
                Ok(Some(f)) => f,
                Ok(None) => break,
                Err(_) => return Hand::Failed,
            };
            match f.op {
                OP_PING => {
                    if conn.write_all(&pong(&f.raw)).is_err() {
                        return Hand::Failed;
                    }
                }
                OP_CLOSE => return Hand::Refused(f.body["code"].as_i64().unwrap_or(0)),
                OP_FRAME if f.body["evt"] == "READY" => return Hand::Ready(dec),
                OP_FRAME if f.body["evt"] == "ERROR" => return Hand::Refused(f.body["data"]["code"].as_i64().unwrap_or(0)),
                _ => {}
            }
        }
    }
    Hand::Failed
}

#[cfg(all(test, windows))]
mod tests {
    use super::super::fake::{Behaviour, Fake};
    use super::*;
    use crate::discord::activity::{ActivityInput, tests::NOW};
    use std::sync::Mutex;

    /// A test id: never a real application's, and it only ever reaches the test pipe.
    const TEST_ID: &str = "100000000000000001";

    fn opts(prefix: &str) -> Options {
        Options {
            client_id: TEST_ID.into(),
            pipe_prefix: prefix.into(),
            lock_name: None,
            retry: Duration::from_millis(150),
            refused_retry: Duration::from_secs(60),
            per_window: 5,
            window: Duration::from_secs(20),
            settle: Duration::from_millis(0),
            answer_wait: Duration::from_millis(400),
            poll: Duration::from_millis(10),
            pid: 4242,
        }
    }

    fn prefix() -> String {
        format!("kalamar-test-{}-{}-", std::process::id(), crate::util::random_hex(4))
    }

    struct Probe {
        tx: Sender<Cmd>,
        seen: Arc<Mutex<Vec<Status>>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Probe {
        fn start(o: Options) -> Probe {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let s2 = seen.clone();
            let (tx, thread) = spawn(o, Arc::new(move |s: &Status| s2.lock().unwrap().push(s.clone()))).unwrap();
            Probe { tx, seen, thread: Some(thread) }
        }
        fn send(&self, c: Cmd) {
            self.tx.send(c).unwrap();
        }
        fn show(&self, details: &str) {
            let a = ActivityInput { details: Some(details.into()), state: Some("Ranked · 4 v 4".into()), start: Some(NOW - 30), ..Default::default() };
            self.send(Cmd::Activity(a.check(NOW + 10)));
        }
        fn link(&self) -> Link {
            self.seen.lock().unwrap().last().map(|s| s.link).unwrap_or(Link::Off)
        }
        fn wait_link(&self, want: Link) -> bool {
            wait_until(|| self.link() == want)
        }
        fn stop(mut self) {
            let (ack, done) = std::sync::mpsc::channel();
            self.send(Cmd::Stop(ack));
            done.recv_timeout(Duration::from_secs(3)).expect("the thread stops and says so");
            self.thread.take().unwrap().join().unwrap();
        }
    }

    fn wait_until(mut f: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        f()
    }

    fn details(fake: &Fake) -> Vec<Option<String>> {
        fake.activities().iter().map(|a| a["details"].as_str().map(str::to_string)).collect()
    }

    #[test]
    fn handshake_activity_and_a_clear_when_the_launcher_quits() {
        let p = prefix();
        let fake = Fake::start(&format!("{p}0"), Behaviour::Ready);
        let w = Probe::start(opts(&p));
        w.send(Cmd::Enabled(true));
        w.show("In a match");
        assert!(wait_until(|| fake.activities().len() == 1), "the activity arrives");
        let hs = fake.handshakes();
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0], serde_json::json!({ "v": 1, "client_id": TEST_ID }));
        let set = fake.commands();
        assert_eq!(set[0]["args"]["pid"], 4242);
        assert_eq!(set[0]["args"]["activity"]["details"], "In a match");
        assert_eq!(set[0]["args"]["activity"]["assets"]["large_image"], "kalamar_logo");
        assert!(w.wait_link(Link::Connected));
        w.stop();
        assert!(wait_until(|| fake.commands().len() == 2), "a clear is written before the pipe closes");
        assert!(fake.commands()[1]["args"]["activity"].is_null());
        assert!(wait_until(|| !fake.connected()), "the pipe is closed");
    }

    #[test]
    fn nothing_reaches_the_pipe_while_the_presence_is_off_and_switching_it_off_clears_it() {
        let p = prefix();
        let fake = Fake::start(&format!("{p}0"), Behaviour::Ready);
        let w = Probe::start(opts(&p));
        w.show("Private room");
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(fake.connections(), 0, "not even a handshake while off");
        w.send(Cmd::Enabled(true));
        assert!(wait_until(|| details(&fake) == [Some("Private room".into())]), "switched on: the activity the page last gave goes out");
        w.send(Cmd::Enabled(false));
        assert!(wait_until(|| fake.commands().len() == 2 && !fake.connected()));
        assert!(fake.commands()[1]["args"]["activity"].is_null());
        assert!(w.wait_link(Link::Off));
        w.show("In a match");
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(fake.connections(), 1, "off again: no new connection");
        w.stop();
    }

    #[test]
    fn updates_respect_discords_limit_and_the_latest_one_wins() {
        let p = prefix();
        let fake = Fake::start(&format!("{p}0"), Behaviour::Ready);
        let mut o = opts(&p);
        o.per_window = 3;
        o.window = Duration::from_millis(1500);
        let w = Probe::start(o);
        w.send(Cmd::Enabled(true));
        assert!(w.wait_link(Link::Connected));
        let t0 = Instant::now();
        for i in 0..20 {
            w.show(&format!("Update {i}"));
            std::thread::sleep(Duration::from_millis(15));
        }
        // inside the first window: at most 3 activities, whatever the page sent
        while t0.elapsed() < Duration::from_millis(1200) {
            assert!(fake.activities().len() <= 3, "{:?}", details(&fake));
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(wait_until(|| details(&fake).last() == Some(&Some("Update 19".into()))), "the latest arrives once the window allows");
        let n = fake.activities().len();
        assert!(n <= 6, "the ones in between were never sent: {n}");
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(fake.activities().len(), n, "an unchanged activity is not sent again");
        w.stop();
    }

    #[test]
    fn a_change_settles_before_it_goes_out() {
        let p = prefix();
        let fake = Fake::start(&format!("{p}0"), Behaviour::Ready);
        let mut o = opts(&p);
        o.settle = Duration::from_millis(300);
        let w = Probe::start(o);
        w.send(Cmd::Enabled(true));
        w.show("Private room");
        assert!(wait_until(|| fake.activities().len() == 1), "the first activity of a connection goes at once");
        for i in 0..5 {
            w.show(&format!("Private room {i}"));
            std::thread::sleep(Duration::from_millis(40));
        }
        assert!(wait_until(|| fake.activities().len() == 2));
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(details(&fake), [Some("Private room".into()), Some("Private room 4".into())]);
        w.stop();
    }

    #[test]
    fn no_discord_is_quiet_and_a_discord_started_later_is_found() {
        let p = prefix();
        let w = Probe::start(opts(&p));
        w.send(Cmd::Enabled(true));
        w.show("In the launcher");
        assert!(w.wait_link(Link::Waiting));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(w.link(), Link::Waiting, "no error, just waiting");
        let fake = Fake::start(&format!("{p}0"), Behaviour::Ready);
        assert!(wait_until(|| details(&fake) == [Some("In the launcher".into())]), "found at the next attempt");
        w.stop();
    }

    #[test]
    fn discord_closed_then_opened_again() {
        let p = prefix();
        let fake = Fake::start(&format!("{p}0"), Behaviour::Ready);
        let w = Probe::start(opts(&p));
        w.send(Cmd::Enabled(true));
        w.show("In a match");
        assert!(wait_until(|| fake.activities().len() == 1));
        fake.close();
        assert!(w.wait_link(Link::Waiting), "Discord went: the thread notices without writing");
        w.show("Match over");
        fake.open();
        assert!(wait_until(|| details(&fake).last() == Some(&Some("Match over".into()))), "a new connection, the current activity again");
        assert_eq!(fake.handshakes().len(), 2);
        assert!(w.wait_link(Link::Connected));
        w.stop();
    }

    #[test]
    fn a_silent_pipe_is_skipped_for_the_next_discord() {
        let p = prefix();
        let silent = Fake::start(&format!("{p}0"), Behaviour::Silent);
        let ptb = Fake::start(&format!("{p}4"), Behaviour::Ready);
        let w = Probe::start(opts(&p));
        w.send(Cmd::Enabled(true));
        w.show("In game");
        assert!(wait_until(|| ptb.activities().len() == 1), "the fifth pipe answers");
        assert_eq!(silent.activities().len(), 0);
        assert_eq!(silent.handshakes().len(), 1, "the first pipe was tried, and given up after its wait");
        w.stop();
    }

    #[test]
    fn a_refused_application_id_is_said_and_nothing_is_sent() {
        let p = prefix();
        let fake = Fake::start(&format!("{p}0"), Behaviour::RefuseClientId);
        let w = Probe::start(opts(&p));
        w.send(Cmd::Enabled(true));
        w.show("In game");
        assert!(w.wait_link(Link::Refused));
        assert_eq!(w.seen.lock().unwrap().last().unwrap().code, Some(4000));
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(fake.handshakes().len(), 1, "not hammered: tried again only after refused_retry");
        assert_eq!(fake.activities().len(), 0);
        w.stop();
    }

    #[test]
    fn pings_are_answered_and_an_error_answer_is_kept() {
        let p = prefix();
        let fake = Fake::start(&format!("{p}0"), Behaviour::Ready);
        let w = Probe::start(opts(&p));
        w.send(Cmd::Enabled(true));
        w.show("In game");
        assert!(wait_until(|| fake.activities().len() == 1));
        fake.ping(br#"{"n":7}"#);
        assert!(wait_until(|| fake.pongs() == vec![br#"{"n":7}"#.to_vec()]), "the pong echoes the ping");
        fake.reject_next();
        w.show("Refused activity");
        assert!(wait_until(|| w.seen.lock().unwrap().last().unwrap().code == Some(4002)));
        assert_eq!(w.link(), Link::Connected, "an error answer does not end the connection");
        w.stop();
    }

    #[test]
    fn two_launchers_one_presence() {
        let p = prefix();
        let fake = Fake::start(&format!("{p}0"), Behaviour::Ready);
        let lock = format!(r"Local\Kalamar.Test.{}", crate::util::random_hex(6));
        let mut o = opts(&p);
        o.lock_name = Some(lock.clone());
        let first = Probe::start(o.clone());
        first.send(Cmd::Enabled(true));
        first.show("Premier");
        assert!(wait_until(|| fake.activities().len() == 1));
        let second = Probe::start(o);
        second.send(Cmd::Enabled(true));
        second.show("Second");
        assert!(second.wait_link(Link::Busy), "the second one stays quiet");
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(details(&fake), [Some("Premier".into())]);
        first.stop();
        assert!(wait_until(|| details(&fake).last() == Some(&Some("Second".into()))), "the second takes over once the first quits");
        second.stop();
    }

    #[test]
    fn nothing_private_ever_reaches_the_pipe() {
        // What a page with a mistake could hand over: every text member carrying something that must never be shown.
        // Pseudonyms cannot be recognised here: the page's own test (`presence.test.mjs`) checks those.
        let p = prefix();
        let fake = Fake::start(&format!("{p}0"), Behaviour::Ready);
        let w = Probe::start(opts(&p));
        w.send(Cmd::Enabled(true));
        let private = ["relay.example.org:47313 (203.0.113.7)", "https://play.example.org", "KLM2-7KQ2M-XA9PL-4DZB3", "camille@example.org"];
        for (i, bad) in private.iter().enumerate() {
            let a = ActivityInput {
                details: Some(format!("In a match {i}")),
                state: Some((*bad).into()),
                large_text: Some((*bad).into()),
                small_image: Some("act_match".into()),
                small_text: Some((*bad).into()),
                ..Default::default()
            };
            w.send(Cmd::Activity(a.check(NOW)));
            std::thread::sleep(Duration::from_millis(60));
        }
        assert!(wait_until(|| details(&fake).last() == Some(&Some("In a match 3".into()))));
        let wire = String::from_utf8_lossy(&fake.raw()).to_string();
        for bad in ["relay.example.org", "203.0.113.7", "play.example.org", "7KQ2M", "camille", "KLM2"] {
            assert!(!wire.contains(bad), "{bad} reached the pipe");
        }
        w.stop();
    }
}
