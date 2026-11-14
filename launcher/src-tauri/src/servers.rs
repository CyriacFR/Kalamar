//! Community servers described by names: parsing what the player types or a card carries, resolving a relay's host
//! name, and which features a server unlocks.
//!
//! A server is `name`, `relay` (host name or IPv4 + UDP port; optional for a server with a service) and optional `api`
//! (the base URL of its community service, spoken to by `proto/`). The game itself never resolves names
//! (`SPLAT3_LAN=relay:<ipv4>:<port>`), so the launcher resolves the relay's host name with the system resolver at each
//! match launch and passes the address; the last good address is kept for display only. A server that lists several
//! relays also has their names looked up and one UDP `HELLO` sent to each, to measure the latency
//! (`proto/relayping.rs`, with its limits). All of it happens only for a server the player added: with no server the
//! launcher stays offline (every network access is listed in the README, "Network accesses"). IPv6 literals are refused
//! for now: the game side is IPv4-only.

use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, Ipv6Addr};

/// The relay's default UDP port (`lan::DEFAULT_RELAY_PORT`).
pub const DEFAULT_RELAY_PORT: u16 = crate::lan::DEFAULT_RELAY_PORT;
pub const MAX_HOST_LEN: usize = 253;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Host {
    /// A DNS name, lower-case.
    Name(String),
    Ip(Ipv4Addr),
}

impl Host {
    pub fn text(&self) -> String {
        match self {
            Host::Name(n) => n.clone(),
            Host::Ip(ip) => ip.to_string(),
        }
    }
}

/// Why a host, a line or a URL was refused; `id()` is translated by the interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddrError {
    Empty,
    /// An IPv6 literal: the game side is IPv4-only for now.
    Ipv6,
    /// Looks like an IPv4 address but is not a valid one (e.g. 300.1.2.3).
    BadIp,
    /// 0.0.0.0, broadcast or multicast.
    NotAServer,
    BadName,
    BadPort,
    BadUrl,
}

impl AddrError {
    pub fn id(&self) -> &'static str {
        match self {
            AddrError::Empty => "empty",
            AddrError::Ipv6 => "ipv6",
            AddrError::BadIp => "bad_ip",
            AddrError::NotAServer => "not_a_server",
            AddrError::BadName => "bad_name",
            AddrError::BadPort => "bad_port",
            AddrError::BadUrl => "bad_url",
        }
    }
    pub fn message(&self) -> &'static str {
        match self {
            AddrError::Empty => "enter a host name or an IPv4 address",
            AddrError::Ipv6 => "IPv6 addresses are not supported yet (the game is IPv4-only): use a host name or an IPv4 address",
            AddrError::BadIp => "not a valid IPv4 address",
            AddrError::NotAServer => "this address cannot be a server",
            AddrError::BadName => "not a valid host name (letters, digits, dots and dashes)",
            AddrError::BadPort => "port: 1 to 65535",
            AddrError::BadUrl => "the service address must be an https:// URL",
        }
    }
}

fn is_label(l: &str) -> bool {
    !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-') && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A host name or an IPv4 literal (no port).
pub fn parse_host(s: &str) -> Result<Host, AddrError> {
    let s = s.trim().trim_end_matches('.');
    if s.is_empty() {
        return Err(AddrError::Empty);
    }
    let bare = s.trim_start_matches('[').trim_end_matches(']');
    if s.starts_with('[') || bare.parse::<Ipv6Addr>().is_ok() || s.matches(':').count() >= 2 {
        return Err(AddrError::Ipv6);
    }
    if let Ok(ip) = s.parse::<Ipv4Addr>() {
        if ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast() {
            return Err(AddrError::NotAServer);
        }
        return Ok(Host::Ip(ip));
    }
    // All digits and dots but not an address: a typo of an IPv4 address, not a name.
    if s.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Err(AddrError::BadIp);
    }
    let lower = s.to_ascii_lowercase();
    if lower.len() > MAX_HOST_LEN || !lower.split('.').all(is_label) {
        return Err(AddrError::BadName);
    }
    // The last label of a name is never all digits.
    if lower.rsplit('.').next().is_some_and(|l| l.bytes().all(|b| b.is_ascii_digit())) {
        return Err(AddrError::BadName);
    }
    Ok(Host::Name(lower))
}

pub fn parse_port(s: &str) -> Result<u16, AddrError> {
    s.trim().parse::<u16>().ok().filter(|p| *p != 0).ok_or(AddrError::BadPort)
}

/// One typed line: `host`, `host:port`, `1.2.3.4` or `1.2.3.4:port`. `[v6]:port` and bare IPv6 are refused.
pub fn parse_line(s: &str) -> Result<(Host, Option<u16>), AddrError> {
    let s = s.trim();
    // Tolerate a pasted scheme ("relay://", "udp://") but nothing after a slash.
    let s = s.split_once("://").map(|(_, r)| r).unwrap_or(s);
    let s = s.trim_end_matches('/');
    if s.contains('/') || s.contains(' ') {
        return Err(AddrError::BadName);
    }
    if s.starts_with('[') || s.matches(':').count() >= 2 {
        return Err(AddrError::Ipv6);
    }
    match s.rsplit_once(':') {
        Some((h, p)) => Ok((parse_host(h)?, Some(parse_port(p)?))),
        None => Ok((parse_host(s)?, None)),
    }
}

/// `https://host[:port][/path]` (plain `http://` only to this computer, for a test server): the protocol's rules,
/// `proto::url::parse_api`. Returned normalized, without a trailing slash.
pub fn parse_api(s: &str) -> Result<String, AddrError> {
    crate::proto::url::parse_api(s).map_err(|e| match e {
        crate::proto::url::UrlError::Addr(AddrError::Ipv6) => AddrError::Ipv6,
        _ => AddrError::BadUrl,
    })
}

/// Name lookups, behind a trait so tests never touch the network.
pub trait Resolver {
    /// IPv4 addresses of `host` (the system resolver; IPv6 answers are dropped).
    fn lookup_v4(&self, host: &str) -> Result<Vec<Ipv4Addr>, String>;
}

/// The operating system's resolver (`getaddrinfo` through the standard library).
pub struct SystemResolver;

/// How long a lookup may take before it is reported as a timeout.
pub const LOOKUP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(6);

impl Resolver for SystemResolver {
    fn lookup_v4(&self, host: &str) -> Result<Vec<Ipv4Addr>, String> {
        use std::net::{SocketAddr, ToSocketAddrs};
        // getaddrinfo has no timeout of its own: it runs on a helper thread that is left behind if it hangs.
        let (tx, rx) = std::sync::mpsc::channel();
        let h = host.to_string();
        std::thread::spawn(move || {
            let r = (h.as_str(), 0u16).to_socket_addrs().map_err(|e| e.to_string()).map(|addrs| {
                addrs.filter_map(|a| if let SocketAddr::V4(v4) = a { Some(*v4.ip()) } else { None }).collect::<Vec<_>>()
            });
            let _ = tx.send(r);
        });
        rx.recv_timeout(LOOKUP_TIMEOUT).unwrap_or_else(|_| Err(format!("timeout: no answer within {} s", LOOKUP_TIMEOUT.as_secs())))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// The name does not exist or the resolver failed (no network...): the message says which.
    Failed(String),
    /// The name exists but has no IPv4 address.
    NoIpv4,
}


/// The IPv4 address to give the game for a host. An IPv4 literal is returned as is (no lookup).
pub fn resolve(r: &dyn Resolver, host: &Host) -> Result<Ipv4Addr, ResolveError> {
    match host {
        Host::Ip(ip) => Ok(*ip),
        Host::Name(n) => {
            let list = r.lookup_v4(n).map_err(ResolveError::Failed)?;
            list.into_iter().find(|ip| !ip.is_unspecified() && !ip.is_broadcast() && !ip.is_multicast()).ok_or(ResolveError::NoIpv4)
        }
    }
}

/// What the configured servers make visible. With no server the launcher is a neutral tool: offline play only.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Features {
    pub servers: usize,
    /// The private match tile (pre-filled with this server's relay).
    pub private_match: bool,
    /// Ranked play, friends and rankings: offered once a server exists (the server's own page says what it has).
    pub ranked: bool,
    pub friends: bool,
    pub rankings: bool,
    /// Name of the server that provides them (the selected one, else the first).
    pub provider: Option<String>,
    pub provider_id: Option<String>,
}

pub fn features(servers: &[(String, String)], selected: Option<&str>) -> Features {
    let provider = selected.and_then(|id| servers.iter().find(|(i, _)| i == id)).or_else(|| servers.first());
    let any = !servers.is_empty();
    Features {
        servers: servers.len(),
        private_match: any,
        ranked: any,
        friends: any,
        rankings: any,
        provider: provider.map(|(_, n)| n.clone()),
        provider_id: provider.map(|(i, _)| i.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_names_and_ipv4() {
        assert_eq!(parse_host(" Relay.Example.org. ").unwrap(), Host::Name("relay.example.org".into()));
        assert_eq!(parse_host("localhost").unwrap(), Host::Name("localhost".into()));
        assert_eq!(parse_host("203.0.113.7").unwrap(), Host::Ip(Ipv4Addr::new(203, 0, 113, 7)));
        assert_eq!(parse_host("").unwrap_err(), AddrError::Empty);
        assert_eq!(parse_host("300.1.2.3").unwrap_err(), AddrError::BadIp);
        assert_eq!(parse_host("1.2.3").unwrap_err(), AddrError::BadIp);
        assert_eq!(parse_host("0.0.0.0").unwrap_err(), AddrError::NotAServer);
        assert_eq!(parse_host("224.0.0.1").unwrap_err(), AddrError::NotAServer);
        assert_eq!(parse_host("-bad.example").unwrap_err(), AddrError::BadName);
        assert_eq!(parse_host("a..b").unwrap_err(), AddrError::BadName);
        assert_eq!(parse_host("under_score.example").unwrap_err(), AddrError::BadName);
        assert_eq!(parse_host(&format!("{}.com", "a".repeat(64))).unwrap_err(), AddrError::BadName);
        assert_eq!(parse_host("host.123").unwrap_err(), AddrError::BadName);
    }

    #[test]
    fn ipv6_literals_are_refused_with_a_clear_message() {
        for s in ["::1", "2001:db8::1", "[2001:db8::1]", "fe80::1%3"] {
            assert_eq!(parse_host(s).unwrap_err(), AddrError::Ipv6, "{s}");
        }
        for s in ["[2001:db8::1]:47313", "2001:db8::1", "::1"] {
            assert_eq!(parse_line(s).unwrap_err(), AddrError::Ipv6, "{s}");
        }
        assert!(AddrError::Ipv6.message().contains("IPv4"));
    }

    #[test]
    fn lines_with_ports() {
        assert_eq!(parse_line("relay.example.org").unwrap(), (Host::Name("relay.example.org".into()), None));
        assert_eq!(parse_line("relay.example.org:47313").unwrap(), (Host::Name("relay.example.org".into()), Some(47313)));
        assert_eq!(parse_line("203.0.113.7:5000").unwrap(), (Host::Ip(Ipv4Addr::new(203, 0, 113, 7)), Some(5000)));
        assert_eq!(parse_line("udp://relay.example.org:47313/").unwrap().1, Some(47313));
        assert_eq!(parse_line("relay.example.org:0").unwrap_err(), AddrError::BadPort);
        assert_eq!(parse_line("relay.example.org:70000").unwrap_err(), AddrError::BadPort);
        assert_eq!(parse_line("relay.example.org:abc").unwrap_err(), AddrError::BadPort);
        assert_eq!(parse_line("a b").unwrap_err(), AddrError::BadName);
        assert_eq!(parse_line("example.org/path").unwrap_err(), AddrError::BadName);
        assert_eq!(parse_line("   ").unwrap_err(), AddrError::Empty);
    }

    #[test]
    fn api_urls() {
        assert_eq!(parse_api(" https://play.example.org/api ").unwrap(), "https://play.example.org/api");
        assert!(parse_api("https://play.example.org:8443").is_ok());
        assert_eq!(parse_api("http://play.example.org").unwrap_err(), AddrError::BadUrl);
        assert_eq!(parse_api("https://user@play.example.org").unwrap_err(), AddrError::BadUrl);
        assert_eq!(parse_api("https://[2001:db8::1]/").unwrap_err(), AddrError::Ipv6);
        assert_eq!(parse_api("https://exa mple.org").unwrap_err(), AddrError::BadUrl);
        assert_eq!(parse_api(&format!("https://{}.org", "a".repeat(300))).unwrap_err(), AddrError::BadUrl);
        // a test server on this computer may use plain http; nothing else may
        assert_eq!(parse_api("http://127.0.0.1:8080").unwrap(), "http://127.0.0.1:8080");
        assert_eq!(parse_api("http://10.0.0.1:8080").unwrap_err(), AddrError::BadUrl);
    }

    struct Fake(Result<Vec<Ipv4Addr>, String>, std::cell::Cell<usize>);
    impl Resolver for Fake {
        fn lookup_v4(&self, _host: &str) -> Result<Vec<Ipv4Addr>, String> {
            self.1.set(self.1.get() + 1);
            self.0.clone()
        }
    }

    #[test]
    fn resolver_wrapper() {
        let ok = Fake(Ok(vec![Ipv4Addr::UNSPECIFIED, Ipv4Addr::new(198, 51, 100, 4)]), Default::default());
        assert_eq!(resolve(&ok, &Host::Name("relay.example.org".into())).unwrap(), Ipv4Addr::new(198, 51, 100, 4));
        assert_eq!(ok.1.get(), 1);
        // an IPv4 literal is never looked up
        assert_eq!(resolve(&ok, &Host::Ip(Ipv4Addr::new(10, 0, 0, 2))).unwrap(), Ipv4Addr::new(10, 0, 0, 2));
        assert_eq!(ok.1.get(), 1);
        let none = Fake(Ok(vec![]), Default::default());
        assert_eq!(resolve(&none, &Host::Name("v6only.example".into())).unwrap_err(), ResolveError::NoIpv4);
        let fail = Fake(Err("no such host".into()), Default::default());
        assert!(matches!(resolve(&fail, &Host::Name("nope.example".into())), Err(ResolveError::Failed(m)) if m == "no such host"));
    }

    #[test]
    fn visibility_rules() {
        let none = features(&[], None);
        assert_eq!(none.servers, 0);
        assert!(!none.private_match && !none.ranked && !none.friends && !none.rankings);
        assert!(none.provider.is_none());
        let list = vec![("a1".to_string(), "Alpha".to_string()), ("b2".to_string(), "Bravo".to_string())];
        let f = features(&list, Some("b2"));
        assert!(f.private_match && f.ranked && f.friends && f.rankings);
        assert_eq!(f.provider.as_deref(), Some("Bravo"));
        assert_eq!(features(&list, None).provider.as_deref(), Some("Alpha"));
        assert_eq!(features(&list, Some("gone")).provider.as_deref(), Some("Alpha"));
    }

    #[test]
    fn the_system_resolver_answers_with_ipv4_only() {
        // `localhost` comes from the hosts file: no network involved.
        let v = SystemResolver.lookup_v4("localhost").unwrap();
        assert!(v.contains(&Ipv4Addr::LOCALHOST), "{v:?}");
        assert_eq!(resolve(&SystemResolver, &Host::Name("localhost".into())).unwrap(), Ipv4Addr::LOCALHOST);
    }
}
