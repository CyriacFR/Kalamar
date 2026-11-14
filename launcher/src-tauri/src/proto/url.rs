//! Addresses of the protocol (`server-card.md`, `overview.md` 3.3): the one-line form a player types
//! (`host[:port]`, the HTTPS port of the discovery document), the API base URL, and the one exception to HTTPS: plain
//! `http` to the loopback (`127.0.0.1`, `localhost`, `[::1]`), for a test server on the player's own machine.

use crate::servers::{self, AddrError, Host};
use reqwest::Url;

pub const MAX_API_LEN: usize = 200;

/// Why an address was refused; `id()` is translated by the interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlError {
    Addr(AddrError),
    /// `http://` to anything but the loopback.
    NotHttps,
    /// A path, a query, a user name... after the host of a one-line form.
    Path,
    /// Not a URL the API may have (user info, query, fragment, too long, another scheme).
    BadApi,
}

impl UrlError {
    pub fn id(&self) -> &'static str {
        match self {
            UrlError::Addr(a) => a.id(),
            UrlError::NotHttps => "not_https",
            UrlError::Path => "path",
            UrlError::BadApi => "bad_api",
        }
    }
    pub fn message(&self) -> &'static str {
        match self {
            UrlError::Addr(a) => a.message(),
            UrlError::NotHttps => "the server must be reached over https (plain http is only accepted for 127.0.0.1, localhost and [::1])",
            UrlError::Path => "type only the server's name (and a port), without a path",
            UrlError::BadApi => "not a valid service address (https://host[:port][/path], at most 200 characters, no user name, query or fragment)",
        }
    }
}

impl From<AddrError> for UrlError {
    fn from(a: AddrError) -> Self {
        UrlError::Addr(a)
    }
}

/// `localhost`, any `127.x.y.z`, `::1` (with or without brackets).
pub fn is_loopback(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    if h == "localhost" || h == "::1" {
        return true;
    }
    h.parse::<std::net::Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Https,
    Http,
}

/// The one-line form: a host (DNS name or IPv4), an optional port, and the scheme when the player typed one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostLine {
    pub host: Host,
    pub port: Option<u16>,
    pub scheme: Option<Scheme>,
}

impl HostLine {
    /// `host[:port]` as typed (lower-case host).
    pub fn text(&self) -> String {
        match self.port {
            Some(p) => format!("{}:{p}", self.host.text()),
            None => self.host.text(),
        }
    }

    pub fn loopback(&self) -> bool {
        is_loopback(&self.host.text())
    }

    /// The discovery URLs to try, in order: https first; plain http only for the loopback (typed `http://`: http only).
    pub fn discovery_urls(&self) -> Vec<String> {
        let at = |scheme: &str| {
            let port = self.port.map(|p| format!(":{p}")).unwrap_or_default();
            format!("{scheme}://{}{port}/.well-known/kalamar-server.json", self.host.text())
        };
        match (self.scheme, self.loopback()) {
            (Some(Scheme::Http), true) => vec![at("http")],
            (_, true) => vec![at("https"), at("http")],
            _ => vec![at("https")],
        }
    }
}

/// `host[:port]`; a pasted `https://` prefix and one trailing `/` are tolerated, `http://` only for the loopback, and
/// nothing else after the host (`server-card.md` 1). IPv6 literals are not part of protocol 1.0.
pub fn parse_host_line(s: &str) -> Result<HostLine, UrlError> {
    let s = s.trim();
    let (scheme, rest) = if let Some(r) = strip_prefix_ci(s, "https://") {
        (Some(Scheme::Https), r)
    } else if let Some(r) = strip_prefix_ci(s, "http://") {
        (Some(Scheme::Http), r)
    } else {
        (None, s)
    };
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    if rest.is_empty() {
        return Err(AddrError::Empty.into());
    }
    if rest.contains(['/', '?', '#', '@', ' ']) {
        return Err(UrlError::Path);
    }
    let (host, port) = servers::parse_line(rest)?;
    let line = HostLine { host, port, scheme };
    if scheme == Some(Scheme::Http) && !line.loopback() {
        return Err(UrlError::NotHttps);
    }
    Ok(line)
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    (s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix)).then(|| &s[prefix.len()..])
}

/// A URL the launcher may call: https anywhere, http on the loopback only. Never a URL with user info.
pub fn allowed_url(u: &Url) -> bool {
    let host = u.host_str().unwrap_or("");
    u.username().is_empty()
        && u.password().is_none()
        && !host.is_empty()
        && match u.scheme() {
            "https" => true,
            "http" => is_loopback(host),
            _ => false,
        }
}

/// The API base URL (`api` of a card or discovery document): `https://host[:port][/path]` (or `http://` on the
/// loopback), at most 200 characters, no user info, query or fragment. Returned without a trailing slash.
pub fn parse_api(s: &str) -> Result<String, UrlError> {
    let s = s.trim();
    if s.is_empty() || s.len() > MAX_API_LEN || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(UrlError::BadApi);
    }
    let lower = s.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err(UrlError::BadApi);
    }
    if s.contains('?') || s.contains('#') {
        return Err(UrlError::BadApi);
    }
    let u = Url::parse(s).map_err(|_| UrlError::BadApi)?;
    if !u.username().is_empty() || u.password().is_some() || u.query().is_some() || u.fragment().is_some() {
        return Err(UrlError::BadApi);
    }
    let host = u.host_str().ok_or(UrlError::BadApi)?;
    if host.starts_with('[') {
        // IPv6 literals: only the loopback, for a local test server.
        if !is_loopback(host) {
            return Err(AddrError::Ipv6.into());
        }
    } else {
        servers::parse_host(host).map_err(|e| match e {
            AddrError::Ipv6 => UrlError::Addr(AddrError::Ipv6),
            _ => UrlError::BadApi,
        })?;
    }
    if u.scheme() == "http" && !is_loopback(host) {
        return Err(UrlError::NotHttps);
    }
    Ok(u.as_str().trim_end_matches('/').to_string())
}

/// `<api><path>` (the API's paths are relative to its base, which may have a path of its own).
pub fn join(api: &str, path: &str) -> String {
    format!("{}{}", api.trim_end_matches('/'), path)
}

/// The host of an API URL, to show the player when it differs from the typed one.
pub fn api_host(api: &str) -> String {
    Url::parse(api).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default()
}

/// `ws(s)://` form of an API URL.
pub fn websocket_url(api: &str, path_and_query: &str) -> String {
    let u = join(api, path_and_query);
    if let Some(r) = u.strip_prefix("https://") {
        format!("wss://{r}")
    } else if let Some(r) = u.strip_prefix("http://") {
        format!("ws://{r}")
    } else {
        u
    }
}

/// Percent-encodes a query value (RFC 3986 unreserved characters kept).
pub fn query_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn one_line_forms() {
        let l = parse_host_line("Play.Example.org").unwrap();
        assert_eq!((l.host.clone(), l.port, l.scheme), (Host::Name("play.example.org".into()), None, None));
        assert_eq!(l.discovery_urls(), ["https://play.example.org/.well-known/kalamar-server.json"]);
        let l = parse_host_line("https://play.example.org:8443/").unwrap();
        assert_eq!(l.discovery_urls(), ["https://play.example.org:8443/.well-known/kalamar-server.json"]);
        assert_eq!(l.text(), "play.example.org:8443");
        let l = parse_host_line("203.0.113.7").unwrap();
        assert_eq!(l.host, Host::Ip(Ipv4Addr::new(203, 0, 113, 7)));
        // the loopback: https first, then plain http; typed http: http only
        assert_eq!(parse_host_line("127.0.0.1:8080").unwrap().discovery_urls(), ["https://127.0.0.1:8080/.well-known/kalamar-server.json", "http://127.0.0.1:8080/.well-known/kalamar-server.json"]);
        assert_eq!(parse_host_line("http://localhost:8080").unwrap().discovery_urls(), ["http://localhost:8080/.well-known/kalamar-server.json"]);
    }

    #[test]
    fn one_line_refusals() {
        assert_eq!(parse_host_line("http://play.example.org").unwrap_err(), UrlError::NotHttps);
        assert_eq!(parse_host_line("play.example.org/api").unwrap_err(), UrlError::Path);
        assert_eq!(parse_host_line("https://user@play.example.org").unwrap_err(), UrlError::Path);
        assert_eq!(parse_host_line("play.example.org?x=1").unwrap_err(), UrlError::Path);
        assert_eq!(parse_host_line("[2001:db8::1]:443").unwrap_err(), UrlError::Addr(AddrError::Ipv6));
        assert_eq!(parse_host_line("").unwrap_err(), UrlError::Addr(AddrError::Empty));
        assert_eq!(parse_host_line("https://").unwrap_err(), UrlError::Addr(AddrError::Empty));
        assert_eq!(parse_host_line("play.example.org:0").unwrap_err(), UrlError::Addr(AddrError::BadPort));
        assert_eq!(parse_host_line("ftp://x").unwrap_err().id(), "path");
    }

    #[test]
    fn api_urls_https_or_loopback_http() {
        assert_eq!(parse_api(" https://play.example.org/api/ ").unwrap(), "https://play.example.org/api");
        assert_eq!(parse_api("https://Play.Example.org:8443").unwrap(), "https://play.example.org:8443");
        assert_eq!(parse_api("http://127.0.0.1:8080").unwrap(), "http://127.0.0.1:8080");
        assert_eq!(parse_api("http://localhost:8080/x").unwrap(), "http://localhost:8080/x");
        assert_eq!(parse_api("http://[::1]:8080").unwrap(), "http://[::1]:8080");
        assert_eq!(parse_api("http://play.example.org").unwrap_err(), UrlError::NotHttps);
        assert_eq!(parse_api("http://10.0.0.1").unwrap_err(), UrlError::NotHttps);
        assert_eq!(parse_api("https://user@play.example.org").unwrap_err(), UrlError::BadApi);
        assert_eq!(parse_api("https://play.example.org/?a=1").unwrap_err(), UrlError::BadApi);
        assert_eq!(parse_api("https://play.example.org/#f").unwrap_err(), UrlError::BadApi);
        assert_eq!(parse_api("ftp://play.example.org").unwrap_err(), UrlError::BadApi);
        assert_eq!(parse_api("https://[2001:db8::1]/").unwrap_err(), UrlError::Addr(AddrError::Ipv6));
        assert_eq!(parse_api("https://exa mple.org").unwrap_err(), UrlError::BadApi);
        assert_eq!(parse_api(&format!("https://{}.org", "a".repeat(300))).unwrap_err(), UrlError::BadApi);
    }

    #[test]
    fn allowed_urls_never_downgrade() {
        assert!(allowed_url(&Url::parse("https://a.example/x").unwrap()));
        assert!(allowed_url(&Url::parse("http://127.0.0.1:8080/x").unwrap()));
        assert!(allowed_url(&Url::parse("http://localhost/x").unwrap()));
        assert!(!allowed_url(&Url::parse("http://a.example/x").unwrap()));
        assert!(!allowed_url(&Url::parse("https://u:p@a.example/x").unwrap()));
        assert!(!allowed_url(&Url::parse("file:///c:/x").unwrap()));
    }

    #[test]
    fn joins_and_websocket_urls() {
        assert_eq!(join("https://a.example/api", "/v1/meta"), "https://a.example/api/v1/meta");
        assert_eq!(websocket_url("https://a.example/api", "/v1/events?resume=x"), "wss://a.example/api/v1/events?resume=x");
        assert_eq!(websocket_url("http://127.0.0.1:8080", "/v1/events"), "ws://127.0.0.1:8080/v1/events");
        assert_eq!(query_escape("a b/c"), "a%20b%2Fc");
        assert_eq!(api_host("https://Play.example.org:8443/api"), "play.example.org");
    }
}
