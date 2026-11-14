//! The relay measurement's limits, on this computer only: a stand-in service on 127.0.0.1 answers `GET /v1/meta` and
//! `PUT /v1/me/game`, UDP sockets on loopback count the HELLOs they receive (and answer like a relay when asked), names
//! go through a stand-in resolver. No datagram leaves the machine: every relay address here is a loopback address of
//! this run only (see [`lo`]), and the addresses that must be refused are only checked by [`allowed`] (nothing is sent
//! to them).

use super::*;
use crate::proto::events::Handler;
use crate::proto::runner::{GameRun, Runner};
use crate::proto::{ConnHandler, RunnerTo};
use std::io::{Read, Write};
use std::net::{Ipv6Addr, TcpListener, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};

/// The loopback address of every stand-in relay of this run (a 127.x.y.z drawn once per process). The system hands out
/// ephemeral UDP ports to every process on the computer, and a port freed by one test run is soon given to another one's
/// socket: with every run on 127.0.0.1, a HELLO meant for a closed port reached a stand-in of a test running at the same
/// time (another `cargo test` on the same computer) and counted there, or got an answer. On an address of its own, a datagram
/// of another run cannot arrive here.
fn lo() -> Ipv4Addr {
    static LO: std::sync::OnceLock<Ipv4Addr> = std::sync::OnceLock::new();
    *LO.get_or_init(|| {
        let mut n = [0u8; 3];
        getrandom::fill(&mut n).unwrap();
        // Never .0 or .255 (a subnet's network and broadcast addresses are refused), nor 127.0.0.1 itself.
        Ipv4Addr::new(127, 1 + n[0] % 253, 1 + n[1] % 253, 1 + n[2] % 253)
    })
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build().unwrap()
}

/// A UDP socket on loopback that notes when relay HELLOs arrive; `answer`: replies with a CHALLENGE like a relay.
struct Udp {
    port: u16,
    hellos: Arc<Mutex<Vec<Instant>>>,
}

impl Udp {
    fn count(&self) -> usize {
        self.hellos.lock().unwrap().len()
    }
    fn first(&self) -> Option<Instant> {
        self.hellos.lock().unwrap().first().copied()
    }
}

fn udp(answer: bool) -> Udp {
    // The system may be short of socket buffers for a moment when the computer is busy (WSAENOBUFS): try again.
    let mut tries = 0;
    let s = loop {
        let made = UdpSocket::bind((lo(), 0)).and_then(|s| s.set_read_timeout(Some(Duration::from_millis(200))).map(|()| s));
        match made {
            Ok(s) => break s,
            Err(_) if tries < 50 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => panic!("a loopback socket: {e}"),
        }
    };
    let port = s.local_addr().unwrap().port();
    let hellos = Arc::new(Mutex::new(Vec::new()));
    let h = hellos.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 2048];
        // Until the test lets go of its handle.
        while Arc::strong_count(&h) > 1 {
            // Errors: the read timeout, or a reset after an answer to a socket already closed.
            let Ok((n, from)) = s.recv_from(&mut buf) else { continue };
            if n >= 8 && &buf[..4] == b"S3LR" {
                h.lock().unwrap().push(Instant::now());
            }
            if answer && n == 64 && buf[5] == 1 {
                let mut out = [0u8; 32];
                out[..6].copy_from_slice(b"S3LR\x01\x02");
                out[8..16].copy_from_slice(&buf[8..16]);
                let _ = s.send_to(&out, from);
            }
        }
    });
    Udp { port, hellos }
}

/// A loopback port nothing listens on (the system answers a datagram with "port unreachable"). Taken below the range the
/// system draws its ephemeral ports from, so that no socket of another test gets it before the HELLO arrives.
fn closed_port() -> u16 {
    let mut n = [0u8; 2];
    for _ in 0..1000 {
        getrandom::fill(&mut n).unwrap();
        let port = 20_000 + u16::from_be_bytes(n) % 10_000;
        if let Ok(s) = UdpSocket::bind((lo(), port)) {
            drop(s);
            return port;
        }
    }
    panic!("no free port below the ephemeral range");
}

type Make = dyn Fn(usize) -> Value + Send + Sync;

/// A stand-in service on loopback: `GET /v1/meta` answers `make(n)` (n: metas served before), `PUT /v1/me/game` is
/// noted with its arrival time, anything else is 404.
struct Api {
    port: u16,
    gets: Arc<AtomicUsize>,
    puts: Arc<Mutex<Vec<(Instant, Value)>>>,
}

impl Api {
    fn gets(&self) -> usize {
        self.gets.load(SeqCst)
    }
    fn puts(&self) -> Vec<(Instant, Value)> {
        self.puts.lock().unwrap().clone()
    }
    /// `launcher.relay_rtt_ms` of each PUT (`Null` when absent).
    fn rtts(&self) -> Vec<Value> {
        self.puts().into_iter().map(|(_, v)| v["launcher"]["relay_rtt_ms"].clone()).collect()
    }
}

fn api(make: impl Fn(usize) -> Value + Send + Sync + 'static) -> Api {
    let make: Arc<Make> = Arc::new(make);
    let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = l.local_addr().unwrap().port();
    let (gets, puts) = (Arc::new(AtomicUsize::new(0)), Arc::new(Mutex::new(Vec::new())));
    let (g, p) = (gets.clone(), puts.clone());
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
            let Some(start) = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4) else { continue };
            let len: usize = head.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0))).unwrap_or(0);
            while buf.len() < start + len {
                match s.read(&mut b) {
                    Ok(0) | Err(_) => break,
                    Ok(k) => buf.extend_from_slice(&b[..k]),
                }
            }
            let (status, body) = if head.starts_with("PUT /v1/me/game") {
                p.lock().unwrap().push((Instant::now(), serde_json::from_slice(&buf[start..]).unwrap_or(Value::Null)));
                ("204 No Content", String::new())
            } else if head.starts_with("GET /v1/meta") {
                ("200 OK", make(g.fetch_add(1, SeqCst)).to_string())
            } else {
                ("404 Not Found", json!({"error": {"code": "not_found", "message": "x"}}).to_string())
            };
            let resp = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = s.write_all(resp.as_bytes());
        }
    });
    Api { port, gets, puts }
}

fn meta(relays: Vec<Value>) -> Value {
    json!({"name": "t", "time": "2026-11-12T00:00:00Z", "features": [], "relays": relays})
}

fn relay(id: &str, port: u16) -> Value {
    json!({"id": id, "host": lo().to_string(), "port": port})
}

fn changed(revision: u64) -> Value {
    json!({"seq": revision, "type": "meta.changed", "at": "2026-11-12T00:00:00Z", "data": {"revision": revision}})
}

#[derive(Default)]
struct Game {
    running: AtomicBool,
}

impl GameHost for Game {
    fn launch(&self, _: Vec<(String, String)>) -> Result<(), String> {
        Ok(())
    }
    fn status(&self) -> GameRun {
        GameRun { running: self.running.load(SeqCst), ..Default::default() }
    }
    fn stop(&self) {}
    fn build_id(&self) -> String {
        "test".into()
    }
}

#[derive(Default)]
struct Col {
    seen: Mutex<Vec<(String, Value)>>,
}

impl Sink for Col {
    fn emit(&self, channel: &str, payload: Value) {
        self.seen.lock().unwrap().push((channel.to_string(), payload));
    }
}

impl Col {
    /// The `relays` of every `proto-relays` push.
    fn relays(&self) -> Vec<Value> {
        self.seen.lock().unwrap().iter().filter(|(c, _)| c == "proto-relays").map(|(_, v)| v["relays"].clone()).collect()
    }
}

/// Names that resolve, the others do not; counts the lookups.
#[derive(Default)]
struct Names {
    map: HashMap<String, Ipv4Addr>,
    lookups: AtomicUsize,
}

impl Resolver for Names {
    fn lookup_v4(&self, host: &str) -> Result<Vec<Ipv4Addr>, String> {
        self.lookups.fetch_add(1, SeqCst);
        self.map.get(host).map(|ip| vec![*ip]).ok_or_else(|| "no such host".into())
    }
}

fn prober() -> Arc<Prober> {
    Arc::new(Prober::new(Arc::new(Names::default())))
}

/// One server in use: its client (logged in or not), relay job, stream handler, pushes and game.
struct One {
    client: Arc<Client>,
    job: Arc<RelayJob>,
    handler: ConnHandler,
    col: Arc<Col>,
    game: Arc<Game>,
}

impl Drop for One {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.client.dir);
    }
}

async fn one(api: &Api, prober: &Arc<Prober>, logged_in: bool) -> One {
    let dir = std::env::temp_dir().join(format!("s3l-relays-{}", crate::util::random_hex(4)));
    let client = Arc::new(Client::new("srv", &format!("http://127.0.0.1:{}", api.port), dir, "fr").unwrap());
    client.fetch_meta().await.unwrap();
    if logged_in {
        client.accept_session(&json!({"access_token": "test-access", "refresh_token": "test-refresh", "me": {"id": "u1", "display_name": "T"}}), "guest").unwrap();
    }
    let game = Arc::new(Game::default());
    let host: Arc<dyn GameHost> = game.clone();
    let col = Arc::new(Col::default());
    let sink: Arc<dyn Sink> = col.clone();
    let job = RelayJob::new("srv", client.clone(), host.clone(), sink.clone(), prober.clone(), Duration::from_millis(100));
    let runner = Runner::new(client.clone(), host, Arc::new(RunnerTo { server: "srv".into(), sink: sink.clone() }), Arc::new(Names::default()));
    let handler = ConnHandler { server: "srv".into(), sink, client: client.clone(), runner, relays: job.clone() };
    One { client, job, handler, col, game }
}

/// Waits until the job has no pass running.
async fn idle(job: &RelayJob, secs: u64) {
    let end = Instant::now() + Duration::from_secs(secs);
    while lock(&job.flight).running {
        assert!(Instant::now() < end, "the measurement did not end within {secs} s");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn states(job: &RelayJob) -> Vec<(String, RelayState)> {
    job.view().into_iter().map(|r| (r.id, r.state)).collect()
}

// ---- what is parsed and what may be measured -----------------------------------------------------------------------

#[test]
fn hello_and_challenge_are_the_relay_protocols() {
    let h = hello(0x0102_0304_0506_0708);
    assert_eq!(&h[..8], b"S3LR\x01\x01\x00\x00");
    assert_eq!(&h[8..16], &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert!(h[16..].iter().all(|b| *b == 0));
    let mut c = [0u8; 32];
    c[..6].copy_from_slice(b"S3LR\x01\x02");
    c[8..16].copy_from_slice(&h[8..16]);
    assert!(is_challenge(&c, 0x0102_0304_0506_0708));
    assert!(!is_challenge(&c, 9) && !is_challenge(&c[..31], 0x0102_0304_0506_0708) && !is_challenge(&h, 0x0102_0304_0506_0708));
}

#[test]
fn only_a_host_and_a_port_make_a_relay() {
    let meta = json!({"relays": [
        {"id": "eu", "host": "relay.example.org", "port": 47313, "region": "eu-west"},
        {"id": "us", "host": "203.0.113.7", "port": 47314},
        {"id": "eu", "host": "twice.example.org", "port": 47313},
        {"id": "v6", "host": "[2001:db8::1]", "port": 47313},
        {"id": "v6b", "host": "::1", "port": 47313},
        {"id": "path", "host": "relay.example.org/x", "port": 47313},
        {"id": "port-in-host", "host": "relay.example.org:1", "port": 47313},
        {"id": "scheme", "host": "udp://192.168.1.1", "port": 47313},
        {"id": "scheme2", "host": "relay://relay.example.org", "port": 47313},
        {"id": "any", "host": "0.0.0.0", "port": 47313},
        {"id": "bcast", "host": "255.255.255.255", "port": 47313},
        {"id": "mdns", "host": "224.0.0.251", "port": 5353},
        {"id": "no-port", "host": "relay.example.org"},
        {"id": "port0", "host": "relay.example.org", "port": 0},
        {"id": "port-big", "host": "relay.example.org", "port": 70000},
        {"id": "", "host": "relay.example.org", "port": 47313},
        "nonsense",
    ]});
    let r = relays_of(&meta);
    assert_eq!(r.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["eu", "us"]);
    assert_eq!(r[1], Relay { id: "us".into(), host: Host::Ip(Ipv4Addr::new(203, 0, 113, 7)), port: 47314 });
    assert!(relays_of(&json!({"relay": "relay.example.org"})).is_empty());
    let many: Vec<Value> = (0..40).map(|i| json!({"id": format!("r{i}"), "host": "203.0.113.7", "port": 47313 + i})).collect();
    assert_eq!(relays_of(&json!({"relays": many})).len(), MAX_RELAYS, "the protocol's 16 at most");
    // The same host:port under several ids is measured (and shown) once.
    let same: Vec<Value> = (0..16).map(|i| json!({"id": format!("r{i}"), "host": "203.0.113.7", "port": 47313})).collect();
    let same = relays_of(&json!({"relays": same}));
    assert_eq!((same.len(), retained(&same).len()), (16, 1));
}

#[test]
fn which_addresses_may_get_a_hello() {
    use Scope::*;
    let ip = |s: &str| s.parse::<Ipv4Addr>().unwrap();
    // This computer's subnets, as `local_subnets` reads them: a LAN /24, an overlay-network /16, the loopback /8.
    let nets = [(ip("192.168.1.20"), 24), (ip("100.100.3.4"), 16), (ip("127.0.0.1"), 8)];
    let ok = |a: &str, port: u16, scope: Scope| allowed(ip(a), port, scope, &nets);
    // Public addresses: for every server, from port 1024.
    for s in [Public, Private, Loopback] {
        assert!(ok("203.0.113.7", 47313, s) && ok("198.51.100.1", 1024, s), "{s:?}");
        for port in [1, 53, 137, 443, 1023] {
            assert!(!ok("203.0.113.7", port, s), "port {port}");
        }
        // Never: this network, link-local (cloud metadata), multicast, reserved, limited broadcast.
        for a in ["0.0.0.0", "0.1.2.3", "169.254.169.254", "169.254.1.1", "224.0.0.251", "239.255.255.250", "240.0.0.1", "255.255.255.255"] {
            assert!(!ok(a, 47313, s), "{a} {s:?}");
        }
    }
    // Loopback: only for a server on loopback.
    assert!(ok("127.0.0.1", 47313, Loopback) && ok("127.0.0.53", 5353, Loopback));
    assert!(!ok("127.0.0.1", 47313, Public) && !ok("127.0.0.1", 47313, Private));
    // Private ranges: only for a server in a private range (a LAN, a mesh VPN).
    for a in ["10.0.0.1", "172.16.0.1", "172.31.255.1", "192.168.1.1", "100.64.0.1", "100.100.3.7", "100.127.1.1"] {
        assert!(ok(a, 47313, Private), "{a}");
        assert!(!ok(a, 47313, Public) && !ok(a, 47313, Loopback), "{a}");
    }
    // Next to the private ranges, public.
    for a in ["172.15.0.1", "172.32.0.1", "100.63.0.1", "100.128.0.1", "11.0.0.1"] {
        assert!(ok(a, 47313, Public), "{a}");
    }
    // A subnet's network and broadcast addresses are refused; its other addresses are hosts (an overlay-network peer may well
    // end in .0 or .255 inside its /16).
    for a in ["192.168.1.255", "192.168.1.0", "100.100.255.255", "100.100.0.0", "127.255.255.255"] {
        assert!(!ok(a, 47313, Private) && !ok(a, 47313, Loopback), "{a}");
    }
    assert!(ok("100.100.3.255", 47313, Private) && ok("100.100.7.0", 47313, Private) && ok("192.168.1.37", 47313, Private));
    // When the subnets cannot be read: every .0 and .255 (those of any /24 or wider).
    assert!(!allowed(ip("10.1.2.255"), 47313, Private, &[]) && !allowed(ip("203.0.113.0"), 47313, Public, &[]));
    assert!(allowed(ip("10.1.2.3"), 47313, Private, &[]));
    // A /31 or /32 has no broadcast.
    assert!(ok("203.0.113.9", 47313, Public) && allowed(ip("203.0.113.9"), 47313, Public, &[(ip("203.0.113.8"), 31)]));
}

#[test]
fn where_a_server_is() {
    let v4 = |s: &str| IpAddr::V4(s.parse().unwrap());
    assert_eq!(scope_of(v4("127.0.0.1")), Scope::Loopback);
    assert_eq!(scope_of(v4("100.100.3.4")), Scope::Private);
    assert_eq!(scope_of(v4("192.168.0.10")), Scope::Private);
    assert_eq!(scope_of(v4("203.0.113.7")), Scope::Public);
    assert_eq!(scope_of(IpAddr::V6(Ipv6Addr::LOCALHOST)), Scope::Loopback);
    assert_eq!(scope_of(IpAddr::V6(Ipv4Addr::new(10, 0, 0, 1).to_ipv6_mapped())), Scope::Private);
    assert_eq!(scope_of(IpAddr::V6("2001:db8::1".parse().unwrap())), Scope::Public);
    // The narrower of the two wins.
    assert_eq!(Scope::Public.min(Scope::Loopback), Scope::Public);
    assert_eq!(Scope::Private.min(Scope::Loopback), Scope::Private);
}

#[test]
fn this_computers_subnet_broadcasts_are_never_measured() {
    let nets = local_subnets();
    if cfg!(windows) {
        assert!(nets.iter().any(|(ip, len)| ip.is_loopback() && *len == 8), "the loopback adapter is listed: {nets:?}");
    }
    for &(addr, len) in nets.iter().filter(|(_, len)| (1..=30).contains(len)) {
        let mask = u32::MAX << (32 - u32::from(len));
        let (net, bcast) = (Ipv4Addr::from(u32::from(addr) & mask), Ipv4Addr::from((u32::from(addr) & mask) | !mask));
        for s in [Scope::Public, Scope::Private, Scope::Loopback] {
            assert!(!allowed(bcast, 47313, s, &nets) && !allowed(net, 47313, s, &nets), "{addr}/{len}");
        }
    }
}

// ---- one HELLO ------------------------------------------------------------------------------------------------------

#[test]
fn a_hello_is_timed_and_a_silent_or_closed_port_is_none() {
    let (up, silent, closed) = (udp(true), udp(false), closed_port());
    rt().block_on(async {
        let at = |p| SocketAddrV4::new(lo(), p);
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        let (a, b, c) = tokio::join!(ping(at(up.port), deadline), ping(at(silent.port), deadline), ping(at(closed), deadline));
        assert!(a.is_some_and(|ms| (1..1000).contains(&ms)), "{a:?}");
        assert_eq!((b, c), (None, None));
        assert!(tokio::time::Instant::now() >= deadline - Duration::from_millis(20), "the silent one is waited for");
    });
    assert_eq!((up.count(), silent.count()), (1, 1));
}

// ---- one server's measurement ---------------------------------------------------------------------------------------

/// The compatibility cases: a 1.1.8 meta (only `relay`) and a list with a single relay are never measured or declared.
#[test]
fn one_relay_or_none_is_never_measured() {
    let u = udp(true);
    let p = u.port;
    let a = api(move |n| {
        if n % 2 == 0 {
            json!({"name": "t", "time": "2026-11-12T00:00:00Z", "features": [], "relay": format!("{}:{p}", lo())})
        } else {
            let mut m = meta(vec![relay("only", p)]);
            m["relay"] = json!(format!("{}:{p}", lo()));
            m
        }
    });
    rt().block_on(async {
        let s = one(&a, &prober(), true).await;
        s.job.declare().await.unwrap();
        s.job.kick();
        idle(&s.job, 5).await;
        for i in 0..4 {
            s.handler.event(&changed(i + 1));
            idle(&s.job, 5).await;
        }
        assert_eq!(a.gets(), 5, "each new revision is read");
        assert_eq!(u.count(), 0, "no HELLO");
        assert_eq!(a.rtts(), vec![Value::Null], "declared once, without relays");
        assert!(s.job.view().is_empty() && s.job.known().is_none());
    });
}

/// A login (`declare` then `kick`, as `glue::after_login` does) is declared at once; the measurement follows in the
/// background and is declared when it ends, 2 s after its HELLOs whatever answered.
#[test]
fn a_login_is_declared_at_once_and_the_measurement_follows() {
    let (up, silent, closed) = (udp(true), udp(false), closed_port());
    let (pu, ps) = (up.port, silent.port);
    let a = api(move |_| meta(vec![relay("up", pu), relay("silent", ps), relay("closed", closed)]));
    rt().block_on(async {
        let s = one(&a, &prober(), true).await;
        assert_eq!(states(&s.job).iter().map(|(_, st)| *st).collect::<Vec<_>>(), vec![RelayState::Unmeasured; 3]);
        let t0 = Instant::now();
        s.job.declare().await.unwrap();
        assert!(t0.elapsed() < Duration::from_secs(1), "the login does not wait for a measurement");
        assert_eq!(a.rtts(), vec![Value::Null], "nothing measured yet: nothing declared");
        assert_eq!(up.count() + silent.count(), 0);
        s.job.kick();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(states(&s.job).iter().all(|(_, st)| *st == RelayState::Pending), "{:?}", states(&s.job));
        idle(&s.job, 8).await;
        assert_eq!((up.count(), silent.count()), (1, 1));
        let puts = a.puts();
        assert_eq!(puts.len(), 2);
        let rtt = &puts[1].1["launcher"]["relay_rtt_ms"];
        assert!(rtt["up"].as_u64().is_some_and(|ms| (1..1000).contains(&ms)), "{rtt}");
        assert_eq!((&rtt["silent"], &rtt["closed"]), (&Value::Null, &Value::Null), "{rtt}");
        let after = puts[1].0.duration_since(up.first().unwrap());
        assert!(after >= TIMEOUT - Duration::from_millis(50) && after < TIMEOUT + Duration::from_secs(1), "{after:?}");
        assert_eq!(states(&s.job), vec![("up".into(), RelayState::Ok), ("silent".into(), RelayState::Silent), ("closed".into(), RelayState::Silent)]);
        // The page was told: pending first, then the values.
        let pushed = s.col.relays();
        assert!(pushed.first().unwrap().as_array().unwrap().iter().all(|r| r["state"] == "pending"), "{pushed:?}");
        assert_eq!(pushed.last().unwrap(), &serde_json::to_value(s.job.view()).unwrap());
        assert_eq!(pushed.last().unwrap()[0]["ms"], rtt["up"]);
    });
}

/// A closed port (refused at once by the system) and a silent one: the declaration leaves at the same time.
#[test]
fn a_closed_port_and_a_silent_one_are_declared_at_the_same_time() {
    let (up, silent) = (udp(true), udp(false));
    let mut took = Vec::new();
    rt().block_on(async {
        for other in [closed_port(), silent.port] {
            let pu = up.port;
            let a = api(move |_| meta(vec![relay("up", pu), relay("other", other)]));
            let s = one(&a, &prober(), true).await;
            let t0 = Instant::now();
            s.job.kick();
            idle(&s.job, 8).await;
            let puts = a.puts();
            assert_eq!(puts.len(), 1);
            assert_eq!(puts[0].1["launcher"]["relay_rtt_ms"]["other"], Value::Null);
            took.push(puts[0].0.duration_since(t0));
        }
    });
    assert!(took.iter().all(|t| *t >= TIMEOUT), "{took:?}");
    let gap = took[0].abs_diff(took[1]);
    assert!(gap < Duration::from_millis(300), "closed {:?} against silent {:?}", took[0], took[1]);
}

/// Ten `meta.changed` back to back with the same list: the meta is read at most twice, each address gets one HELLO;
/// one more event later reads the meta again without measuring; a revision already seen is ignored.
#[test]
fn a_burst_of_meta_changed_reads_the_meta_at_most_twice_and_measures_once() {
    let (x, y) = (udp(true), udp(false));
    let (px, py) = (x.port, y.port);
    let a = api(move |_| meta(vec![relay("a", px), relay("b", py)]));
    rt().block_on(async {
        let s = one(&a, &prober(), true).await;
        for i in 0..10 {
            s.handler.event(&changed(i + 1));
        }
        idle(&s.job, 10).await;
        assert!(a.gets() <= 3, "1 at start, then at most 2: {}", a.gets());
        assert_eq!((x.count(), y.count()), (1, 1));
        let gets = a.gets();
        s.handler.event(&changed(11));
        idle(&s.job, 5).await;
        assert_eq!((a.gets() - gets, x.count(), y.count()), (1, 1, 1), "read again, not measured again");
        s.handler.event(&changed(11));
        s.handler.event(&changed(4));
        idle(&s.job, 5).await;
        assert_eq!(a.gets() - gets, 1, "revisions already seen are ignored");
        assert_eq!(a.puts().len(), 1, "declared once: the values did not change");
    });
}

/// A `meta.changed` whose revision is the meta's own (or older) reads nothing.
#[test]
fn a_revision_already_read_is_ignored() {
    let (x, y) = (udp(true), udp(true));
    let (px, py) = (x.port, y.port);
    let a = api(move |_| {
        let mut m = meta(vec![relay("a", px), relay("b", py)]);
        m["revision"] = json!(5);
        m
    });
    rt().block_on(async {
        let s = one(&a, &prober(), true).await;
        s.handler.event(&changed(5));
        s.handler.event(&changed(3));
        idle(&s.job, 5).await;
        assert_eq!((a.gets(), x.count()), (1, 0));
        s.handler.event(&changed(6));
        idle(&s.job, 8).await;
        assert_eq!((a.gets(), x.count(), y.count()), (2, 1, 1));
    });
}

/// The ids change at every meta, the addresses do not: no new HELLO, and the new ids are declared with the old values.
#[test]
fn renamed_ids_reuse_the_measurement() {
    let (x, y) = (udp(true), udp(false));
    let (px, py) = (x.port, y.port);
    let a = api(move |n| meta(vec![relay(&format!("a{n}"), px), relay(&format!("b{n}"), py)]));
    rt().block_on(async {
        let s = one(&a, &prober(), true).await;
        for i in 0..3 {
            s.handler.event(&changed(i + 1));
            idle(&s.job, 8).await;
        }
        assert_eq!((x.count(), y.count()), (1, 1));
        let rtts = a.rtts();
        assert_eq!(rtts.len(), 3, "declared again for each new list");
        let last = rtts.last().unwrap();
        assert!(last["a3"].as_u64().is_some() && last["b3"].is_null() && last.as_object().unwrap().len() == 2, "{last}");
        assert_eq!(last["a3"], rtts[0]["a1"]);
    });
}

/// Sixteen ids on one address: one HELLO, one line on the page, every id declared with its value; then fifty
/// `meta.changed` back to back with sixteen new ids each time: nothing more.
#[test]
fn sixteen_ids_on_one_address_get_one_hello_and_a_flood_changes_nothing() {
    let x = udp(true);
    let px = x.port;
    let a = api(move |n| meta((0..16).map(|i| relay(&format!("r{n}-{i}"), px)).collect()));
    rt().block_on(async {
        let s = one(&a, &prober(), true).await;
        s.handler.event(&changed(1));
        idle(&s.job, 8).await;
        assert_eq!(x.count(), 1);
        assert_eq!(states(&s.job), vec![("r1-0".into(), RelayState::Ok)]);
        let rtt = s.job.known().unwrap();
        assert_eq!(rtt.len(), 16);
        assert!(rtt.values().all(|v| v.is_some() && *v == rtt["r1-0"]), "{rtt:?}");
        for i in 0..50 {
            s.handler.event(&changed(i + 2));
        }
        idle(&s.job, 10).await;
        assert_eq!(x.count(), 1, "the address had its HELLO for ten minutes");
        assert!(a.gets() <= 4, "{}", a.gets());
    });
}

/// A list of sixteen new addresses at every meta (forty addresses in turn), fifty `meta.changed` back to back then
/// three more one after the other: never more than the budget, never two HELLOs to one address.
#[test]
fn a_list_that_changes_every_time_gets_no_more_than_the_budget() {
    let rx: Vec<Udp> = (0..40).map(|_| udp(true)).collect();
    let ports: Vec<u16> = rx.iter().map(|u| u.port).collect();
    let a = api(move |n| meta((0..16).map(|i| relay(&format!("r{n}-{i}"), ports[(n * 7 + i) % 40])).collect()));
    rt().block_on(async {
        let s = one(&a, &prober(), true).await;
        for i in 0..50 {
            s.handler.event(&changed(i + 1));
        }
        idle(&s.job, 15).await;
        for i in 0..3 {
            s.handler.event(&changed(i + 100));
            idle(&s.job, 8).await;
        }
        let counts: Vec<usize> = rx.iter().map(Udp::count).collect();
        assert_eq!(counts.iter().sum::<usize>(), BUDGET, "{counts:?}");
        assert!(counts.iter().all(|c| *c <= 1), "{counts:?}");
        assert!(a.gets() >= 4, "the meta was read again: {}", a.gets());
        // What was not measured says so.
        assert!(states(&s.job).iter().any(|(_, st)| *st == RelayState::Unmeasured), "{:?}", states(&s.job));
    });
}

/// Logins, stream resyncs and two servers listing the same addresses, all at once: one HELLO per address, and both
/// servers get the value.
#[test]
fn concurrent_triggers_make_one_measurement() {
    let (x, y) = (udp(true), udp(false));
    let (px, py) = (x.port, y.port);
    let a = api(move |_| meta(vec![relay("a", px), relay("b", py)]));
    let shared = prober();
    rt().block_on(async {
        let s1 = one(&a, &shared, true).await;
        let s2 = one(&a, &shared, true).await;
        for _ in 0..5 {
            s1.job.kick();
        }
        for _ in 0..3 {
            s1.handler.resync();
        }
        s2.job.kick();
        s2.handler.resync();
        tokio::time::sleep(Duration::from_millis(300)).await;
        idle(&s1.job, 8).await;
        idle(&s2.job, 8).await;
        assert_eq!((x.count(), y.count()), (1, 1));
        for s in [&s1, &s2] {
            let rtt = s.job.known().unwrap();
            assert!(rtt["a"].is_some() && rtt["b"].is_none(), "{rtt:?}");
        }
        assert!(a.rtts().iter().any(|r| r["a"].as_u64().is_some()));
    });
}

/// While the game runs nothing is measured, and a `meta.changed` keeps the values already declared.
#[test]
fn nothing_is_measured_while_the_game_runs_and_the_last_values_stay() {
    let (x, y, z) = (udp(true), udp(false), udp(true));
    let (px, py, pz) = (x.port, y.port, z.port);
    let current = Arc::new(Mutex::new(meta(vec![relay("a", px), relay("b", py)])));
    let m = current.clone();
    let a = api(move |_| m.lock().unwrap().clone());
    rt().block_on(async {
        let s = one(&a, &prober(), true).await;
        s.job.kick();
        idle(&s.job, 8).await;
        let before = a.rtts().last().unwrap()["a"].clone();
        assert!(before.as_u64().is_some());
        s.game.running.store(true, SeqCst);
        *current.lock().unwrap() = meta(vec![relay("a", px), relay("b2", py), relay("c", pz)]);
        for i in 0..5 {
            s.handler.event(&changed(i + 1));
        }
        idle(&s.job, 8).await;
        assert_eq!((x.count(), y.count(), z.count()), (1, 1, 0), "no HELLO while the game runs");
        let last = a.rtts().last().unwrap().clone();
        assert_eq!(last["a"], before, "the value already declared stays: {last} / {:?}", a.rtts());
        assert!(last["b2"].is_null() && last["c"].is_null(), "{last}");
        // Game over: the new address is measured at the next occasion.
        s.game.running.store(false, SeqCst);
        s.job.kick();
        idle(&s.job, 8).await;
        assert_eq!((x.count(), y.count(), z.count()), (1, 1, 1), "{:?} {:?} gets={} puts={}", states(&s.job), s.col.relays().last(), a.gets(), a.puts().len());
    });
    // With the game running from the start: nothing at all.
    let (x, y) = (udp(true), udp(true));
    let (px, py) = (x.port, y.port);
    let a = api(move |n| meta(vec![relay("a", px), relay(&format!("b{n}"), py)]));
    rt().block_on(async {
        let s = one(&a, &prober(), true).await;
        s.game.running.store(true, SeqCst);
        for i in 0..5 {
            s.handler.event(&changed(i + 1));
        }
        idle(&s.job, 8).await;
        assert_eq!((x.count(), y.count()), (0, 0));
        assert!(a.rtts().iter().all(Value::is_null));
    });
}

/// Logged out: nothing. Cancelled (logout, another server, removal) while the HELLOs are out: nothing is declared.
#[test]
fn a_cancelled_or_logged_out_measurement_declares_nothing() {
    let (x, y) = (udp(true), udp(false));
    let (px, py) = (x.port, y.port);
    let a = api(move |_| meta(vec![relay("a", px), relay("b", py)]));
    rt().block_on(async {
        let out = one(&a, &prober(), false).await;
        out.job.kick();
        out.handler.event(&changed(1));
        idle(&out.job, 5).await;
        assert_eq!((x.count(), y.count(), a.puts().len()), (0, 0, 0));

        let s = one(&a, &prober(), true).await;
        s.job.kick();
        let end = Instant::now() + Duration::from_secs(5);
        while y.count() == 0 {
            assert!(Instant::now() < end);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        s.job.cancel();
        assert!(!lock(&s.job.flight).running);
        tokio::time::sleep(TIMEOUT + Duration::from_secs(1)).await;
        assert!(a.puts().is_empty(), "nothing declared after the cancel");
        assert!(states(&s.job).iter().all(|(_, st)| *st != RelayState::Pending));
    });
}

/// Names go through the hub's resolver, once per ten minutes; a name that does not resolve and an address that is
/// refused are `null` (and shown as such).
#[test]
fn names_go_through_the_hubs_resolver_once() {
    let (x, y) = (udp(true), udp(true));
    let names = Arc::new(Names {
        map: [("eu.relay.test".to_string(), lo()), ("us.relay.test".to_string(), lo())].into_iter().collect(),
        lookups: AtomicUsize::new(0),
    });
    let shared = Arc::new(Prober::new(names.clone()));
    let (px, py) = (x.port, y.port);
    let a = api(move |_| {
        meta(vec![
            json!({"id": "eu", "host": "eu.relay.test", "port": px}),
            json!({"id": "us", "host": "us.relay.test", "port": py}),
            json!({"id": "gone", "host": "gone.relay.test", "port": px}),
            json!({"id": "low", "host": "127.0.0.1", "port": 80}),
        ])
    });
    rt().block_on(async {
        let s = one(&a, &shared, true).await;
        s.job.kick();
        idle(&s.job, 8).await;
        assert_eq!(names.lookups.load(SeqCst), 3, "three names, the server is an IPv4");
        assert_eq!((x.count(), y.count()), (1, 1));
        assert_eq!(
            states(&s.job),
            vec![("eu".into(), RelayState::Ok), ("us".into(), RelayState::Ok), ("gone".into(), RelayState::Silent), ("low".into(), RelayState::Refused)]
        );
        let rtt = s.job.known().unwrap();
        assert!(rtt["eu"].is_some() && rtt["us"].is_some() && rtt["gone"].is_none() && rtt["low"].is_none(), "{rtt:?}");
        s.job.kick();
        idle(&s.job, 5).await;
        assert_eq!((names.lookups.load(SeqCst), x.count(), y.count()), (3, 1, 1), "looked up and measured once per ten minutes");
    });
}
