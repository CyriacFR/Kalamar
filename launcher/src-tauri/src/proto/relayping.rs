//! Several relays (protocol 1.1.9, `relay.md` 4.1): the round trip from this computer to each relay a server lists
//! in `meta.relays`, declared as `launcher.relay_rtt_ms` in `PUT /v1/me/game` so the server can give a match the relay
//! that suits its players. The server still names the relay of each match; nothing here chooses one.
//!
//! **Network.** One relay protocol `HELLO` (64 bytes, UDP, from a fresh socket) per relay address and the `CHALLENGE`
//! that answers it: it joins nothing and leaves no state on the relay. Relay host names are looked up first (the
//! resolver of the [`super::Hub`]), and the server's own name when its scope must be known ([`allowed`]). Only for a
//! server the player is logged in to and that lists at least two relays, never while the game runs, in a background
//! task ([`RelayJob`]): a login, the event stream and the match never wait for it; logging out, choosing another server
//! or removing it cancels it. A server's list is not trusted:
//! - an address (resolved IPv4 + port) gets at most one `HELLO` per [`KEEP`] from this launcher, whatever server, id or
//!   list names it, and a name is looked up at most once per [`KEEP`] ([`Prober`], one for the whole launcher); the
//!   entries of one list that name the same `host:port` are measured once ([`retained`]);
//! - one server gets at most [`BUDGET`] `HELLO`s and [`BUDGET`] relay name lookups per [`KEEP`] (a sliding window),
//!   however often its list changes: the protocol's own limits (at most 16 relays, each measured at most once per ten
//!   minutes) are exactly what a well-behaved server uses;
//! - one measurement at a time per server: events that arrive meanwhile are folded into one more pass, a
//!   `meta.changed` of a revision already seen is ignored, and the meta is read again after a random delay of up to
//!   [`SPREAD`] (not every launcher at once);
//! - addresses that are not a relay's are refused ([`allowed`]);
//! - any failure (no answer, a closed port, a name that does not resolve, a refused address) is `null`, and the
//!   declaration leaves [`TIMEOUT`] after the `HELLO`s whatever answered: the server cannot tell a closed port from a
//!   silent one by when it arrives.

use super::Sink;
use super::client::Client;
use super::runner::GameHost;
use crate::servers::{self, Host, Resolver};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How long a relay may take to answer before it counts as unreachable (`relay.md` 4.1); also how long after its
/// `HELLO`s a measurement is declared, whatever answered.
pub const TIMEOUT: Duration = Duration::from_secs(2);
/// An address is measured, and a name looked up, at most once per this long (`relay.md` 4.1: "at most once per relay
/// per 10 minutes"); also the window of [`BUDGET`].
pub const KEEP: Duration = Duration::from_secs(600);
/// Entries of `meta.relays` read (the protocol's limit).
pub const MAX_RELAYS: usize = 16;
/// `HELLO`s, and relay name lookups, one server may cause per [`KEEP`].
pub const BUDGET: usize = 16;
/// Ports below this are never measured (system services, not a relay).
pub const MIN_PORT: u16 = 1024;
/// On `meta.changed`, the meta is read again after a random delay below this.
pub const SPREAD: Duration = Duration::from_secs(5);
/// Name lookups that run side by side (each holds a blocking thread while it runs).
pub const LOOKUPS_AT_ONCE: usize = 4;
/// What the launcher remembers of an address or a name is forgotten after this long.
const FORGET: Duration = Duration::from_secs(24 * 3600);

/// Round trips by relay id, in milliseconds (`None`: no answer, or not measured).
pub type Rtt = BTreeMap<String, Option<u32>>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relay {
    pub id: String,
    pub host: Host,
    pub port: u16,
}

type Key = (String, u16);

fn key(r: &Relay) -> Key {
    (r.host.text(), r.port)
}

/// The relays of `meta.relays` (its first [`MAX_RELAYS`] entries): a valid id seen once, a `host` that is a DNS name
/// or an IPv4 address (nothing else: no scheme, port, path or IPv6 literal), a port. Others are skipped.
pub fn relays_of(meta: &Value) -> Vec<Relay> {
    let Some(list) = meta.get("relays").and_then(Value::as_array) else { return Vec::new() };
    let mut out: Vec<Relay> = Vec::new();
    for r in list.iter().take(MAX_RELAYS) {
        let id = r.get("id").and_then(Value::as_str).unwrap_or("");
        let host = r.get("host").and_then(Value::as_str).unwrap_or("");
        let port = r.get("port").and_then(Value::as_u64).filter(|p| (1..=65535).contains(p));
        if id.is_empty() || id.len() > 260 || id.chars().any(char::is_control) || out.iter().any(|x| x.id == id) {
            continue;
        }
        if let (Ok(h), Some(p)) = (servers::parse_host(host), port) {
            out.push(Relay { id: id.to_string(), host: h, port: p as u16 });
        }
    }
    out
}

/// The relays measured and shown: the first entry of each `host:port` (another id naming the same address is declared
/// with its value).
pub fn retained(relays: &[Relay]) -> Vec<&Relay> {
    let mut out: Vec<&Relay> = Vec::new();
    for r in relays {
        if !out.iter().any(|x| x.host == r.host && x.port == r.port) {
            out.push(r);
        }
    }
    out
}

// ---- which addresses may be measured --------------------------------------------------------------------------------

/// Where the community server itself is, which widens the addresses its relays may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Scope {
    Public,
    /// In a private range ([`is_private`]): a server on the player's own private network (a LAN, a mesh VPN).
    Private,
    /// On this computer (a test server).
    Loopback,
}

/// 10/8, 172.16/12, 192.168/16 (RFC 1918) and 100.64/10 (shared address space: carrier-grade NAT, mesh VPNs and other
/// overlay networks).
pub fn is_private(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 10 || (o[0] == 172 && (16..32).contains(&o[1])) || (o[0] == 192 && o[1] == 168) || (o[0] == 100 && (64..128).contains(&o[1]))
}

pub fn scope_of(ip: IpAddr) -> Scope {
    match ip {
        IpAddr::V4(v4) if v4.is_loopback() => Scope::Loopback,
        IpAddr::V4(v4) if is_private(v4) => Scope::Private,
        IpAddr::V4(_) => Scope::Public,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => scope_of(IpAddr::V4(v4)),
            None if v6.is_loopback() => Scope::Loopback,
            None => Scope::Public,
        },
    }
}

/// Whether a relay address may get a `HELLO` when the server is in `scope`. Never: a port below [`MIN_PORT`]; 0/8;
/// link-local (169.254/16); multicast, reserved and the limited broadcast (224/3); the network or broadcast address of
/// one of this computer's subnets (`subnets`, address and prefix length; when it could not be read, any address ending
/// in .0 or .255, those of every /24 or wider). Loopback only for a server on loopback, a private range only for a
/// server in a private range. The socket never has `SO_BROADCAST` either, so the system refuses a broadcast this missed.
pub fn allowed(ip: Ipv4Addr, port: u16, scope: Scope, subnets: &[(Ipv4Addr, u8)]) -> bool {
    let o = ip.octets();
    if port < MIN_PORT || o[0] == 0 || o[0] >= 224 || ip.is_link_local() || subnet_edge(ip, subnets) {
        return false;
    }
    if ip.is_loopback() {
        return scope == Scope::Loopback;
    }
    if is_private(ip) {
        return scope == Scope::Private;
    }
    true
}

/// The network or broadcast address of one of `subnets` (a /31 or /32 has neither).
fn subnet_edge(ip: Ipv4Addr, subnets: &[(Ipv4Addr, u8)]) -> bool {
    if subnets.is_empty() {
        let last = ip.octets()[3];
        return last == 0 || last == 255;
    }
    let x = u32::from(ip);
    subnets.iter().any(|&(addr, len)| {
        if !(1..=30).contains(&len) {
            return false;
        }
        let mask = u32::MAX << (32 - u32::from(len));
        let net = u32::from(addr) & mask;
        x == net || x == net | !mask
    })
}

/// This computer's IPv4 addresses and their on-link prefix lengths (every adapter); empty when they cannot be read.
#[cfg(windows)]
pub fn local_subnets() -> Vec<(Ipv4Addr, u8)> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_FRIENDLY_NAME, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH};
    use windows_sys::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};
    const ERROR_BUFFER_OVERFLOW: u32 = 111;
    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER | GAA_FLAG_SKIP_FRIENDLY_NAME;
    let mut size: u32 = 16 * 1024;
    for _ in 0..4 {
        // u64 cells: the list is made of 8-byte aligned structures.
        let mut buf: Vec<u64> = vec![0; (size as usize).div_ceil(8)];
        let first = buf.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // SAFETY: `buf` holds `size` writable bytes; the call writes at most that and says so otherwise.
        let r = unsafe { GetAdaptersAddresses(u32::from(AF_INET), flags, std::ptr::null(), first, &mut size) };
        if r == ERROR_BUFFER_OVERFLOW {
            continue;
        }
        if r != 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut a = first.cast_const();
        // SAFETY: a linked list the call built inside `buf`, which outlives the walk; every pointer is null or valid.
        unsafe {
            while !a.is_null() {
                let mut u = (*a).FirstUnicastAddress.cast_const();
                while !u.is_null() {
                    let sa = (*u).Address.lpSockaddr;
                    if !sa.is_null() && (*sa).sa_family == AF_INET {
                        let sin = &*sa.cast::<SOCKADDR_IN>();
                        out.push((Ipv4Addr::from(u32::from_be(sin.sin_addr.S_un.S_addr)), (*u).OnLinkPrefixLength));
                    }
                    u = (*u).Next.cast_const();
                }
                a = (*a).Next.cast_const();
            }
        }
        return out;
    }
    Vec::new()
}

#[cfg(not(windows))]
pub fn local_subnets() -> Vec<(Ipv4Addr, u8)> {
    Vec::new()
}

// ---- one HELLO ------------------------------------------------------------------------------------------------------

/// The relay protocol's HELLO (version 1): `S3LR`, the version, type 1, two zero bytes, the nonce, zero padding.
pub fn hello(nonce: u64) -> [u8; 64] {
    let mut b = [0u8; 64];
    b[..4].copy_from_slice(b"S3LR");
    b[4] = 1;
    b[5] = 0x01;
    b[8..16].copy_from_slice(&nonce.to_be_bytes());
    b
}

/// Whether `b` is the CHALLENGE that answers the HELLO of `nonce`.
pub fn is_challenge(b: &[u8], nonce: u64) -> bool {
    b.len() >= 32 && &b[..4] == b"S3LR" && b[5] == 0x02 && b[8..16] == nonce.to_be_bytes()
}

fn random_u64() -> u64 {
    let mut n = [0u8; 8];
    let _ = getrandom::fill(&mut n);
    u64::from_be_bytes(n)
}

/// The round trip to one relay address, in milliseconds; `None` when it did not answer before `deadline` (or the
/// system reported the port closed). On the async runtime: no thread of its own.
pub async fn ping(target: SocketAddrV4, deadline: tokio::time::Instant) -> Option<u32> {
    let sock = tokio::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.ok()?;
    sock.set_broadcast(false).ok()?;
    sock.connect(target).await.ok()?;
    let nonce = random_u64();
    let start = Instant::now();
    sock.send(&hello(nonce)).await.ok()?;
    let mut buf = [0u8; 128];
    loop {
        let got = tokio::time::timeout_at(deadline, sock.recv(&mut buf)).await.ok()?.ok()?;
        if is_challenge(&buf[..got], nonce) {
            return Some((start.elapsed().as_millis() as u32).max(1));
        }
    }
}

// ---- what this launcher remembers, whatever server ----------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Seen {
    at: Instant,
    /// The round trip; while a new HELLO is out, the previous one (if any).
    ms: Option<u32>,
    /// The answer (or its absence) is known; else a HELLO is out.
    done: bool,
}

impl Seen {
    fn fresh(&self) -> bool {
        self.at.elapsed() < KEEP
    }
    /// A HELLO still waiting for its answer (one whose measurement was cancelled ends with the previous value).
    fn pending(&self) -> bool {
        !self.done && self.at.elapsed() < TIMEOUT + Duration::from_millis(500)
    }
}

/// The launcher's memory of relay addresses and names, shared by every server (the [`super::Hub`] owns it): an address
/// gets one HELLO per [`KEEP`], a name one lookup per [`KEEP`], whoever asks.
pub struct Prober {
    resolver: Arc<dyn Resolver + Send + Sync>,
    targets: Mutex<HashMap<SocketAddrV4, Seen>>,
    names: Mutex<HashMap<String, (Instant, Option<Ipv4Addr>)>>,
}

impl Prober {
    pub fn new(resolver: Arc<dyn Resolver + Send + Sync>) -> Prober {
        Prober { resolver, targets: Mutex::new(HashMap::new()), names: Mutex::new(HashMap::new()) }
    }

    /// A name looked up within [`KEEP`]: `Some(None)` when it did not resolve; `None` when not looked up recently.
    fn name(&self, n: &str) -> Option<Option<Ipv4Addr>> {
        lock(&self.names).get(n).filter(|(at, _)| at.elapsed() < KEEP).map(|(_, ip)| *ip)
    }

    /// Looks the names up with the resolver, [`LOOKUPS_AT_ONCE`] at a time (blocking threads), and remembers them.
    async fn look_up(&self, names: &[String]) {
        for chunk in names.chunks(LOOKUPS_AT_ONCE) {
            let jobs = chunk.iter().map(|n| {
                let (r, n) = (self.resolver.clone(), n.clone());
                tokio::task::spawn_blocking(move || {
                    let ip = servers::resolve(&*r, &Host::Name(n.clone())).ok();
                    (n, ip)
                })
            });
            let got = futures_util::future::join_all(jobs).await;
            let mut g = lock(&self.names);
            g.retain(|_, (at, _)| at.elapsed() < FORGET);
            for (n, ip) in got.into_iter().flatten() {
                g.insert(n, (Instant::now(), ip));
            }
        }
    }

    fn seen(&self, t: SocketAddrV4) -> Option<Seen> {
        lock(&self.targets).get(&t).copied()
    }

    /// Takes an address for a HELLO, unless it had one within [`KEEP`]: true when the caller sends it.
    fn claim(&self, t: SocketAddrV4) -> bool {
        let mut g = lock(&self.targets);
        if g.get(&t).is_some_and(Seen::fresh) {
            return false;
        }
        g.retain(|_, s| s.at.elapsed() < FORGET);
        let previous = g.get(&t).and_then(|s| s.ms);
        g.insert(t, Seen { at: Instant::now(), ms: previous, done: false });
        true
    }

    fn record(&self, t: SocketAddrV4, ms: Option<u32>) {
        if let Some(s) = lock(&self.targets).get_mut(&t) {
            s.ms = ms;
            s.done = true;
        }
    }
}

// ---- one server's measurement ---------------------------------------------------------------------------------------

/// What became of one retained relay (by `host:port`) at the last pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Its name is being looked up.
    Pending,
    /// Measured (or being measured) at this address.
    At(SocketAddrV4),
    /// Its name does not resolve to an IPv4 address.
    NoAddress,
    /// An address that is not measured ([`allowed`]).
    Refused,
    /// Not measured: this server's budget was spent.
    Skipped,
}

/// How the Account tab shows a relay.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RelayState {
    Pending,
    Ok,
    /// No answer (or a name that does not resolve).
    Silent,
    /// An address that is not measured.
    Refused,
    /// Not measured yet.
    Unmeasured,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RelayView {
    pub id: String,
    pub state: RelayState,
    pub ms: Option<u32>,
}

#[derive(Default)]
struct Budget {
    hellos: VecDeque<Instant>,
    lookups: VecDeque<Instant>,
}

/// Room left in a sliding window of [`KEEP`].
fn room(q: &mut VecDeque<Instant>) -> bool {
    while q.front().is_some_and(|t| t.elapsed() >= KEEP) {
        q.pop_front();
    }
    q.len() < BUDGET
}

#[derive(Default)]
struct Flight {
    running: bool,
    /// Asked again while running: one more pass.
    again: bool,
    /// Read the meta again first (`meta.changed`).
    reload: bool,
    generation: u64,
    handle: Option<tokio::task::JoinHandle<()>>,
}

/// One server's relay measurement and declaration (one per connection): at most one pass at a time, in the background.
pub struct RelayJob {
    server: String,
    client: Arc<Client>,
    host: Arc<dyn GameHost>,
    sink: Arc<dyn Sink>,
    prober: Arc<Prober>,
    spread: Duration,
    budget: Mutex<Budget>,
    flight: Mutex<Flight>,
    /// The highest `meta.changed` revision acted on.
    revision: Mutex<Option<u64>>,
    outcome: Mutex<HashMap<Key, Outcome>>,
    /// What the last `PUT /v1/me/game` declared of the relays; also makes the PUTs go one at a time.
    declared: tokio::sync::Mutex<Option<Option<Rtt>>>,
}

impl RelayJob {
    pub fn new(server: &str, client: Arc<Client>, host: Arc<dyn GameHost>, sink: Arc<dyn Sink>, prober: Arc<Prober>, spread: Duration) -> Arc<RelayJob> {
        Arc::new(RelayJob {
            server: server.to_string(),
            client,
            host,
            sink,
            prober,
            spread,
            budget: Mutex::default(),
            flight: Mutex::default(),
            revision: Mutex::new(None),
            outcome: Mutex::default(),
            declared: tokio::sync::Mutex::new(None),
        })
    }

    fn relays(&self) -> Vec<Relay> {
        self.client.cached_meta().map(|(m, _)| relays_of(&m)).unwrap_or_default()
    }

    /// Measures what needs it, in the background, then declares the values when they changed. Must be called on the
    /// tokio runtime. A pass already running gets one more pass instead.
    pub fn kick(self: &Arc<Self>) {
        self.start(false);
    }

    /// `meta.changed`: the meta is read again after a random delay (then measured and declared as by [`Self::kick`]),
    /// unless its `revision` was already seen.
    pub fn meta_changed(self: &Arc<Self>, data: &Value) {
        if let Some(r) = data.get("revision").and_then(Value::as_u64) {
            let have = self.client.cached_meta().and_then(|(m, _)| m.get("revision").and_then(Value::as_u64));
            let mut seen = lock(&self.revision);
            if seen.is_some_and(|s| r <= s) || have.is_some_and(|h| r <= h) {
                return;
            }
            *seen = Some(r);
        }
        self.start(true);
    }

    fn start(self: &Arc<Self>, reload: bool) {
        let mut f = lock(&self.flight);
        f.reload |= reload;
        if f.running {
            f.again = true;
            return;
        }
        f.running = true;
        f.again = false;
        let (me, generation) = (self.clone(), f.generation);
        f.handle = Some(tokio::spawn(async move { me.run(generation).await }));
    }

    async fn run(self: Arc<Self>, generation: u64) {
        loop {
            let reload = std::mem::take(&mut lock(&self.flight).reload);
            self.pass(reload).await;
            let mut f = lock(&self.flight);
            if f.generation != generation {
                return;
            }
            if !f.again && !f.reload {
                f.running = false;
                return;
            }
            f.again = false;
        }
    }

    /// Stops a pass under way (logout, another server chosen, server removed): nothing more is sent for it.
    pub fn cancel(&self) {
        {
            let mut f = lock(&self.flight);
            f.generation += 1;
            f.running = false;
            f.again = false;
            f.reload = false;
            if let Some(h) = f.handle.take() {
                h.abort();
            }
        }
        lock(&self.outcome).retain(|_, o| *o != Outcome::Pending);
    }

    async fn pass(&self, reload: bool) {
        if reload {
            let ms = self.spread.as_millis() as u64;
            if ms > 0 {
                tokio::time::sleep(Duration::from_millis(random_u64() % ms)).await;
            }
            if self.client.fetch_meta().await.is_err() {
                return;
            }
        }
        if !self.client.session_info().logged_in {
            return;
        }
        let relays = self.relays();
        if relays.len() >= 2 && !self.host.status().running {
            self.measure(&relays).await;
        }
        if self.client.session_info().logged_in {
            self.declare_if_changed().await;
        }
        self.emit();
    }

    /// One measurement of the retained relays that need one (see the module's rules). Returns [`TIMEOUT`] after the
    /// HELLOs when any was sent.
    async fn measure(&self, relays: &[Relay]) {
        let keep = retained(relays);
        let mut names: Vec<String> = Vec::new();
        {
            let mut b = lock(&self.budget);
            for r in &keep {
                if let Host::Name(n) = &r.host
                    && r.port >= MIN_PORT
                    && self.prober.name(n).is_none()
                    && !names.contains(n)
                    && room(&mut b.lookups)
                {
                    b.lookups.push_back(Instant::now());
                    names.push(n.clone());
                }
            }
        }
        if !names.is_empty() {
            {
                let mut o = lock(&self.outcome);
                for r in &keep {
                    if matches!(&r.host, Host::Name(n) if names.contains(n)) {
                        o.insert(key(r), Outcome::Pending);
                    }
                }
            }
            self.emit();
            self.prober.look_up(&names).await;
        }
        if self.host.status().running || !self.client.session_info().logged_in {
            lock(&self.outcome).retain(|_, o| *o != Outcome::Pending);
            return;
        }
        let scope = self.scope().await;
        let subnets = local_subnets();
        let mut send: Vec<SocketAddrV4> = Vec::new();
        // Another server's HELLO to the same address may still be out: its answer is waited for.
        let mut others = Duration::ZERO;
        let mut next: HashMap<Key, Outcome> = HashMap::new();
        {
            let mut b = lock(&self.budget);
            for r in &keep {
                let ip = match &r.host {
                    Host::Ip(ip) => Some(Some(*ip)),
                    Host::Name(n) => self.prober.name(n),
                };
                let o = match ip {
                    _ if r.port < MIN_PORT => Outcome::Refused,
                    None => Outcome::Skipped,
                    Some(None) => Outcome::NoAddress,
                    Some(Some(ip)) if !allowed(ip, r.port, scope, &subnets) => Outcome::Refused,
                    Some(Some(ip)) => {
                        let t = SocketAddrV4::new(ip, r.port);
                        match self.prober.seen(t) {
                            _ if send.contains(&t) => Outcome::At(t),
                            Some(s) if s.fresh() => {
                                if s.pending() {
                                    let answered = s.at + TIMEOUT + Duration::from_millis(250);
                                    others = others.max(answered.saturating_duration_since(Instant::now()));
                                }
                                Outcome::At(t)
                            }
                            _ if room(&mut b.hellos) => {
                                if self.prober.claim(t) {
                                    b.hellos.push_back(Instant::now());
                                    send.push(t);
                                }
                                Outcome::At(t)
                            }
                            _ => Outcome::Skipped,
                        }
                    }
                };
                next.insert(key(r), o);
            }
        }
        *lock(&self.outcome) = next;
        if send.is_empty() && others.is_zero() {
            return;
        }
        self.emit();
        let start = tokio::time::Instant::now();
        let deadline = start + TIMEOUT;
        let got = futures_util::future::join_all(send.iter().map(|t| ping(*t, deadline))).await;
        for (t, ms) in send.iter().zip(got) {
            self.prober.record(*t, ms);
        }
        // Whatever answered or failed early (a closed port is reported at once), the declaration leaves at the deadline.
        let end = if send.is_empty() { start + others } else { deadline.max(start + others) };
        tokio::time::sleep_until(end).await;
    }

    /// Where the server is: the address its meta came from and the address its name resolves to, the wider scope
    /// only when both agree (a proxy or a name that answers differently on a second lookup gets the narrower one).
    async fn scope(&self) -> Scope {
        let peer = self.client.peer().map(|a| scope_of(a.ip()));
        if peer == Some(Scope::Public) {
            return Scope::Public;
        }
        let host = super::url::api_host(&self.client.api);
        let named = if super::url::is_loopback(&host) {
            Scope::Loopback
        } else if let Ok(ip) = host.parse::<Ipv4Addr>() {
            scope_of(IpAddr::V4(ip))
        } else {
            if self.prober.name(&host).is_none() {
                self.prober.look_up(std::slice::from_ref(&host)).await;
            }
            self.prober.name(&host).flatten().map_or(Scope::Public, |ip| scope_of(IpAddr::V4(ip)))
        };
        peer.map_or(named, |p| p.min(named))
    }

    /// What to declare now, from what was measured (no network): every relay of the list, `null` for one that did not
    /// answer, was refused or is not measured yet. `None` (nothing declared) with fewer than two relays or before
    /// anything was measured. While the game runs, the last values are declared again whatever their age.
    pub fn known(&self) -> Option<Rtt> {
        let relays = self.relays();
        if relays.len() < 2 {
            return None;
        }
        let outcome = lock(&self.outcome);
        let mut measured = false;
        let rtt = relays
            .iter()
            .map(|r| {
                let ms = match outcome.get(&key(r)) {
                    Some(Outcome::At(t)) => {
                        let s = self.prober.seen(*t);
                        measured |= s.is_some_and(|s| !s.pending() || s.ms.is_some());
                        s.and_then(|s| s.ms)
                    }
                    Some(Outcome::NoAddress) => {
                        measured = true;
                        None
                    }
                    _ => None,
                };
                (r.id.clone(), ms)
            })
            .collect();
        measured.then_some(rtt)
    }

    /// The retained relays as the Account tab shows them (empty with fewer than two relays).
    pub fn view(&self) -> Vec<RelayView> {
        let relays = self.relays();
        if relays.len() < 2 {
            return Vec::new();
        }
        let outcome = lock(&self.outcome);
        retained(&relays)
            .into_iter()
            .map(|r| {
                let (state, ms) = match outcome.get(&key(r)) {
                    Some(Outcome::Pending) => (RelayState::Pending, None),
                    Some(Outcome::At(t)) => match self.prober.seen(*t) {
                        Some(s) if s.pending() => (RelayState::Pending, None),
                        Some(Seen { ms: Some(ms), .. }) => (RelayState::Ok, Some(ms)),
                        Some(_) => (RelayState::Silent, None),
                        None => (RelayState::Unmeasured, None),
                    },
                    Some(Outcome::NoAddress) => (RelayState::Silent, None),
                    Some(Outcome::Refused) => (RelayState::Refused, None),
                    Some(Outcome::Skipped) | None => (RelayState::Unmeasured, None),
                };
                RelayView { id: r.id.clone(), state, ms }
            })
            .collect()
    }

    /// Tells the page (`proto-relays`).
    fn emit(&self) {
        self.sink.emit("proto-relays", json!({"server": self.server, "relays": self.view()}));
    }

    /// `PUT /v1/me/game` with what is known of the relays now ([`Self::known`]; nothing is measured here).
    pub async fn declare(&self) -> Result<(), super::http::CallError> {
        let mut last = self.declared.lock().await;
        let rtt = self.known();
        super::declare_game(&self.client, self.host.clone(), rtt.clone()).await?;
        *last = Some(rtt);
        Ok(())
    }

    /// The same, only when the relays' values differ from the last declaration.
    async fn declare_if_changed(&self) {
        let mut last = self.declared.lock().await;
        let rtt = self.known();
        if last.as_ref() == Some(&rtt) {
            return;
        }
        if super::declare_game(&self.client, self.host.clone(), rtt.clone()).await.is_ok() {
            *last = Some(rtt);
        }
    }
}

#[cfg(test)]
#[path = "relayping_tests.rs"]
mod tests;
