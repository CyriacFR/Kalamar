//! The community servers the player added, saved in the launcher data directory (`servers.rs` describes them).
//!
//! Layout: `online\servers.json` (list and selection) and one folder per server, `online\<id>\`, for that server's
//! own files (the community service's session and match files, `proto/`). Nothing here touches the network except
//! [`resolve_text`], which looks a relay's host name up when a launch or the player asks for it.

use crate::error::{AppError, AppResult, FieldError};
use crate::servercard;
use crate::util;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAX_SERVERS: usize = 64;

/// A community server, described by names (`servers.rs`): its relay (host name or IPv4, UDP port) and the https
/// address of its community service (the protocol client, `proto/`). Keys of earlier files that are not read any more
/// are ignored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Server {
    pub id: String,
    pub name: String,
    /// Relay: host name or IPv4 literal.
    #[serde(default)]
    pub relay: Option<String>,
    /// `None`: the relay's default port.
    #[serde(default)]
    pub relay_port: Option<u16>,
    /// Base URL of the community service (`proto/`): `https://`, or plain `http://` to this computer for a test server.
    #[serde(default)]
    pub api: Option<String>,
    /// Last address the relay's host name resolved to, for display only (each launch resolves again).
    #[serde(default)]
    pub last_ip: Option<String>,
    /// Community service: keep the event stream connected from the launcher's start (off by default).
    #[serde(default)]
    pub background: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Store {
    pub schema: u32,
    pub selected: Option<String>,
    pub servers: Vec<Server>,
}

impl Default for Store {
    fn default() -> Self {
        Store { schema: 1, selected: None, servers: Vec::new() }
    }
}

/// What a caller sends to add or edit a server. `relay` is empty for none.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ServerInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub relay: String,
    #[serde(default)]
    pub relay_port: Option<u16>,
    #[serde(default)]
    pub api: String,
}

pub fn online_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("online")
}

fn store_path(data_dir: &Path) -> PathBuf {
    online_dir(data_dir).join("servers.json")
}

/// Ids are generated here; anything else read from the file is dropped, so an id is always a safe folder name.
pub fn valid_id(id: &str) -> bool {
    (8..=16).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn load(data_dir: &Path) -> Store {
    let path = store_path(data_dir);
    let Ok(Some(text)) = util::read_text_limited(&path, 1 << 20) else { return Store::default() };
    match serde_json::from_str::<Store>(&text) {
        Ok(mut s) => {
            s.servers.retain(|x| valid_id(&x.id));
            s.servers.truncate(MAX_SERVERS);
            if s.selected.as_ref().is_some_and(|id| !s.servers.iter().any(|x| &x.id == id)) {
                s.selected = None;
            }
            s
        }
        Err(_) => {
            let _ = std::fs::rename(&path, path.with_extension("json.bad"));
            Store::default()
        }
    }
}

pub fn save(data_dir: &Path, store: &Store) -> AppResult<()> {
    let text = serde_json::to_string_pretty(store).map_err(|e| AppError::Internal(e.to_string()))?;
    util::atomic_write(&store_path(data_dir), text.as_bytes())
}

fn field(key: &str, message: impl Into<String>) -> FieldError {
    FieldError { key: key.into(), message: message.into() }
}

/// Checks the fields of a server; the error is one message per field.
pub fn validate(input: &ServerInput) -> Result<Server, Vec<FieldError>> {
    let mut errors = Vec::new();
    let name = servercard::valid_name(&input.name).unwrap_or_else(|m| {
        errors.push(field("name", m));
        String::new()
    });
    // A host name or an IPv4 literal; IPv6 is refused (the game is IPv4-only).
    let relay = match input.relay.trim() {
        "" => None,
        r => match crate::servers::parse_host(r) {
            Ok(h) => Some(h.text()),
            Err(e) => {
                errors.push(field("relay", e.message()));
                None
            }
        },
    };
    if input.relay_port == Some(0) {
        errors.push(field("relay_port", "relay port: 1 to 65535"));
    }
    // A server of the community protocol may name relays per match only (its `relay` is then null).
    if input.relay.trim().is_empty() && input.api.trim().is_empty() {
        errors.push(field("relay", "a relay (host name or IPv4 address) is needed"));
    }
    let api = match input.api.trim() {
        "" => None,
        a => match crate::servers::parse_api(a) {
            Ok(u) => Some(u),
            Err(e) => {
                errors.push(field("api", e.message()));
                None
            }
        },
    };
    if !errors.is_empty() {
        return Err(errors);
    }
    let relay_port = if relay.is_some() { input.relay_port } else { None };
    Ok(Server { id: String::new(), name, relay, relay_port, api, last_ip: None, background: false })
}

/// Adds a server (`input.id` empty) or updates one.
pub fn upsert(data_dir: &Path, input: &ServerInput) -> AppResult<Store> {
    let mut server = validate(input).map_err(AppError::Validation)?;
    let mut store = load(data_dir);
    match input.id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => {
            if !store.servers.iter().any(|s| s.id == id) {
                return Err(AppError::InvalidInput("unknown server".into()));
            }
            server.id = id.to_string();
        }
        None => {
            if store.servers.len() >= MAX_SERVERS {
                return Err(AppError::InvalidInput(format!("at most {MAX_SERVERS} servers")));
            }
            server.id = util::random_hex(4);
        }
    }
    match store.servers.iter_mut().find(|s| s.id == server.id) {
        Some(slot) => {
            server.last_ip = slot.last_ip.clone().filter(|_| slot.relay == server.relay);
            server.background = slot.background;
            *slot = server.clone()
        }
        None => {
            store.servers.push(server.clone());
            if store.selected.is_none() {
                store.selected = Some(server.id.clone());
            }
        }
    }
    save(data_dir, &store)?;
    Ok(store)
}

pub fn select(data_dir: &Path, id: Option<&str>) -> AppResult<Store> {
    let mut store = load(data_dir);
    if let Some(id) = id
        && !store.servers.iter().any(|s| s.id == id)
    {
        return Err(AppError::InvalidInput("unknown server".into()));
    }
    store.selected = id.map(str::to_string);
    save(data_dir, &store)?;
    Ok(store)
}

/// Removes a server with its folder (the page has asked for a confirmation).
pub fn remove(data_dir: &Path, id: &str) -> AppResult<Store> {
    let mut store = load(data_dir);
    if !store.servers.iter().any(|s| s.id == id) {
        return Err(AppError::InvalidInput("unknown server".into()));
    }
    store.servers.retain(|s| s.id != id);
    if store.selected.as_deref() == Some(id) {
        store.selected = None;
    }
    save(data_dir, &store)?;
    let dir = online_dir(data_dir).join(id);
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir).map_err(|e| AppError::io(&dir, &e))?;
    }
    Ok(store)
}

/// A stored host (name or IPv4 literal) as the IPv4 literal to give the game; a name is looked up now.
pub fn resolve_text(r: &dyn crate::servers::Resolver, host: &str) -> AppResult<String> {
    let h = crate::servers::parse_host(host).map_err(|e| AppError::InvalidInput(format!("{host}: {}", e.message())))?;
    crate::servers::resolve(r, &h).map(|ip| ip.to_string()).map_err(|e| match e {
        crate::servers::ResolveError::Failed(m) => AppError::NotAvailable(format!("the name {host} could not be resolved ({m}): check the name and the Internet connection")),
        crate::servers::ResolveError::NoIpv4 => AppError::NotAvailable(format!("the name {host} has no IPv4 address (the game is IPv4-only)")),
    })
}

/// Keeps the last good address of a server's relay, for display only.
pub fn remember_ip(data_dir: &Path, id: &str, ip: &str) {
    let mut store = load(data_dir);
    if let Some(s) = store.servers.iter_mut().find(|s| s.id == id)
        && s.last_ip.as_deref() != Some(ip)
    {
        s.last_ip = Some(ip.to_string());
        let _ = save(data_dir, &store);
    }
}

fn same_relay(s: &Server, host: &str, port: Option<u16>) -> bool {
    let p = |x: Option<u16>| x.unwrap_or(crate::servers::DEFAULT_RELAY_PORT);
    s.relay.as_deref().is_some_and(|r| r.eq_ignore_ascii_case(host)) && p(s.relay_port) == p(port)
}

/// "Add a server" from one typed line: a host name (preferred) or an IPv4 address, with an optional port. The name
/// shown defaults to the host. Nothing is contacted.
pub fn add_line(data_dir: &Path, line: &str, name: Option<&str>, api: Option<&str>) -> AppResult<Store> {
    let (host, port) = crate::servers::parse_line(line).map_err(|e| AppError::Validation(vec![field("line", e.id())]))?;
    let store = load(data_dir);
    if store.servers.iter().any(|s| same_relay(s, &host.text(), port)) {
        return Err(AppError::Validation(vec![field("line", "duplicate")]));
    }
    let name = name.map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| host.text());
    let input = ServerInput {
        name: name.chars().take(40).collect(),
        relay: host.text(),
        relay_port: port,
        api: api.unwrap_or("").to_string(),
        ..Default::default()
    };
    upsert(data_dir, &input).map_err(|e| match e {
        AppError::Validation(f) => AppError::Validation(f.into_iter().map(|x| FieldError { key: "line".into(), message: if x.key == "name" { "name".into() } else if x.key == "api" { "bad_url".into() } else { x.message } }).collect()),
        other => other,
    })
}

/// Earlier launchers kept a "default match server" (`ip` or `ip:port`) in config.json. It becomes a server entry once
/// (nothing is lost: the setting itself is left as it was). Returns whether an entry was added.
pub fn migrate_match_server(data_dir: &Path, match_server: &str) -> bool {
    let s = match_server.trim();
    if s.is_empty() {
        return false;
    }
    let Ok((host, port)) = crate::servers::parse_line(s) else { return false };
    let store = load(data_dir);
    if store.servers.iter().any(|x| same_relay(x, &host.text(), port)) {
        return false;
    }
    let input = ServerInput { name: format!("Match server {}", host.text()).chars().take(40).collect(), relay: host.text(), relay_port: port, ..Default::default() };
    upsert(data_dir, &input).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("s3l-online-{tag}-{}", util::random_hex(4)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn input(name: &str, relay: &str) -> ServerInput {
        ServerInput { id: None, name: name.into(), relay: relay.into(), relay_port: None, api: String::new() }
    }

    #[test]
    fn servers_by_name_and_the_migration_of_the_old_match_server() {
        let d = tmp("names");
        // one typed line: a host name, a relay-only server, named after the host
        let s = add_line(&d, "Relay.Example.org:47313", None, Some("https://play.example.org")).unwrap();
        let x = &s.servers[0];
        assert_eq!((x.name.as_str(), x.relay.as_deref(), x.relay_port), ("relay.example.org", Some("relay.example.org"), Some(47313)));
        assert_eq!(x.api.as_deref(), Some("https://play.example.org"));
        assert_eq!(s.selected.as_deref(), Some(x.id.as_str()));
        // the same relay twice, an IPv6 literal, a bad port: refused with a code the page translates
        let code = |r: AppResult<Store>| match r {
            Err(AppError::Validation(f)) => f[0].message.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(code(add_line(&d, "relay.example.org", None, None)), "duplicate");
        assert_eq!(code(add_line(&d, "[2001:db8::1]:47313", None, None)), "ipv6");
        assert_eq!(code(add_line(&d, "203.0.113.7:0", None, None)), "bad_port");
        assert_eq!(code(add_line(&d, "203.0.113.7", None, Some("ftp://x"))), "bad_url");
        // an IPv4 line, with a display name
        let s = add_line(&d, "203.0.113.7", Some("Léa's place"), None).unwrap();
        assert_eq!(s.servers.len(), 2);
        assert_eq!(s.servers[1].name, "Léa's place");
        // the old default match server becomes an entry once, and only if no server has that relay
        assert!(!migrate_match_server(&d, "203.0.113.7"));
        assert!(!migrate_match_server(&d, ""));
        assert!(migrate_match_server(&d, "198.51.100.9:7000"));
        assert!(!migrate_match_server(&d, "198.51.100.9:7000"));
        let s = load(&d);
        let m = s.servers.iter().find(|x| x.relay.as_deref() == Some("198.51.100.9")).unwrap();
        assert_eq!(m.relay_port, Some(7000));
        // a schema-1 file (no api, no last_ip) still loads
        std::fs::write(store_path(&d), r#"{"schema":1,"selected":"0a1b2c3d","servers":[{"id":"0a1b2c3d","name":"Old","relay":"203.0.113.8"}]}"#).unwrap();
        let s = load(&d);
        assert_eq!((s.servers[0].api.clone(), s.servers[0].last_ip.clone(), s.selected.as_deref()), (None, None, Some("0a1b2c3d")));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn keys_an_earlier_launcher_wrote_are_ignored() {
        let d = tmp("old-keys");
        // `address` and `port` belonged to a mode this launcher no longer has: the entry loads without them, and
        // the next save leaves them out
        std::fs::create_dir_all(online_dir(&d)).unwrap();
        std::fs::write(
            store_path(&d),
            r#"{"schema":1,"selected":"0a1b2c3d","servers":[{"id":"0a1b2c3d","name":"Old","address":"203.0.113.7","port":47314,"relay":"203.0.113.8","relay_port":5000}]}"#,
        )
        .unwrap();
        let s = load(&d);
        assert_eq!(s.servers.len(), 1);
        let x = &s.servers[0];
        assert_eq!((x.name.as_str(), x.relay.as_deref(), x.relay_port, s.selected.as_deref()), ("Old", Some("203.0.113.8"), Some(5000), Some("0a1b2c3d")));
        select(&d, Some("0a1b2c3d")).unwrap();
        let text = std::fs::read_to_string(store_path(&d)).unwrap();
        assert!(!text.contains("address") && !text.contains("47314"), "{text}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_relay_stored_by_an_earlier_launcher_keeps_private_match_visible() {
        let d = tmp("legacy-relay");
        // nothing stored: no server, nothing visible
        let none: Vec<(String, String)> = load(&d).servers.iter().map(|s| (s.id.clone(), s.name.clone())).collect();
        assert!(!crate::servers::features(&none, None).private_match);
        // the setting of an earlier launcher, an IPv4 with the default port
        assert!(migrate_match_server(&d, "198.51.100.23:47313"));
        let st = load(&d);
        assert_eq!(st.servers.len(), 1);
        let x = &st.servers[0];
        assert_eq!((x.relay.as_deref(), x.relay_port), (Some("198.51.100.23"), Some(47313)));
        let list: Vec<(String, String)> = st.servers.iter().map(|s| (s.id.clone(), s.name.clone())).collect();
        let f = crate::servers::features(&list, st.selected.as_deref());
        assert!(f.private_match && f.ranked && f.friends && f.rankings, "visible with zero re-typing");
        // an IPv4 relay needs no lookup and is what the game gets
        struct Never;
        impl crate::servers::Resolver for Never {
            fn lookup_v4(&self, _h: &str) -> Result<Vec<std::net::Ipv4Addr>, String> {
                Err("must not be called".into())
            }
        }
        assert_eq!(resolve_text(&Never, x.relay.as_deref().unwrap()).unwrap(), "198.51.100.23");
        // starting again does not duplicate it
        assert!(!migrate_match_server(&d, "198.51.100.23"));
        assert_eq!(load(&d).servers.len(), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn names_are_resolved_for_the_game_and_failures_are_clear() {
        struct R(Result<Vec<std::net::Ipv4Addr>, String>);
        impl crate::servers::Resolver for R {
            fn lookup_v4(&self, _h: &str) -> Result<Vec<std::net::Ipv4Addr>, String> {
                self.0.clone()
            }
        }
        assert_eq!(resolve_text(&R(Ok(vec!["198.51.100.4".parse().unwrap()])), "relay.example.org").unwrap(), "198.51.100.4");
        assert_eq!(resolve_text(&R(Err("x".into())), "203.0.113.7").unwrap(), "203.0.113.7", "an IPv4 literal needs no lookup");
        let e = resolve_text(&R(Err("no such host".into())), "nope.example.org").unwrap_err().to_string();
        assert!(e.contains("nope.example.org") && e.contains("could not be resolved"), "{e}");
        let e = resolve_text(&R(Ok(vec![])), "v6only.example.org").unwrap_err().to_string();
        assert!(e.contains("IPv4"), "{e}");
    }

    #[test]
    fn fields_are_validated() {
        assert!(validate(&input("A", "203.0.113.7")).is_ok());
        assert!(validate(&ServerInput { api: "https://play.example.org".into(), ..input("A", "") }).is_ok(), "a service without a relay");
        for (i, key) in [
            (input("", "203.0.113.7"), "name"),
            (input("A", ""), "relay"),
            (input("A", "exa mple.org"), "relay"),
            (input("A", "2001:db8::1"), "relay"),
            (input("A", "0.0.0.0"), "relay"),
            (ServerInput { api: "http://x.example".into(), ..input("A", "203.0.113.7") }, "api"),
            (ServerInput { relay_port: Some(0), ..input("A", "1.2.3.4") }, "relay_port"),
        ] {
            let e = validate(&i).unwrap_err();
            assert!(e.iter().any(|f| f.key == key), "{key}: {e:?}");
        }
        let s = validate(&ServerInput { relay_port: Some(9), ..input(" A ", " 1.2.3.4 ") }).unwrap();
        assert_eq!((s.name.as_str(), s.relay.as_deref(), s.relay_port), ("A", Some("1.2.3.4"), Some(9)));
        // no relay: its port is dropped
        assert_eq!(validate(&ServerInput { relay_port: Some(9), api: "https://play.example.org".into(), ..input("A", "") }).unwrap().relay_port, None);
    }

    #[test]
    fn add_edit_select_and_remove() {
        let d = tmp("crud");
        assert!(upsert(&d, &input("", "203.0.113.7")).is_err());
        assert!(load(&d).servers.is_empty());
        let s = upsert(&d, &input("A", "203.0.113.7")).unwrap();
        let id = s.servers[0].id.clone();
        assert!(valid_id(&id));
        assert_eq!(s.selected.as_deref(), Some(id.as_str())); // the first one is selected
        let edit = ServerInput { id: Some(id.clone()), relay_port: Some(47315), ..input("A2", "203.0.113.9") };
        let s = upsert(&d, &edit).unwrap();
        assert_eq!((s.servers[0].name.as_str(), s.servers[0].relay_port), ("A2", Some(47315)));
        assert!(upsert(&d, &ServerInput { id: Some("ffffffff".into()), ..input("x", "203.0.113.7") }).is_err());
        // a second server, selection
        let s = upsert(&d, &input("B", "203.0.113.8")).unwrap();
        let id2 = s.servers[1].id.clone();
        assert_ne!(id, id2);
        assert_eq!(select(&d, Some(&id2)).unwrap().selected.as_deref(), Some(id2.as_str()));
        assert!(select(&d, Some("nope")).is_err());
        // removing deletes that server's folder only
        std::fs::create_dir_all(online_dir(&d).join(&id)).unwrap();
        std::fs::create_dir_all(online_dir(&d).join(&id2)).unwrap();
        std::fs::write(online_dir(&d).join(&id2).join("session.dat"), b"x").unwrap();
        let s = remove(&d, &id2).unwrap();
        assert_eq!(s.servers.len(), 1);
        assert_eq!(s.selected, None);
        assert!(!online_dir(&d).join(&id2).exists() && online_dir(&d).join(&id).exists());
        assert!(remove(&d, &id2).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_corrupt_or_hostile_store_is_set_aside_or_cleaned() {
        let d = tmp("bad");
        util::atomic_write(&store_path(&d), b"{ nope").unwrap();
        assert_eq!(load(&d), Store::default());
        assert!(online_dir(&d).join("servers.json.bad").exists());
        util::atomic_write(
            &store_path(&d),
            br#"{"schema":1,"selected":"..","servers":[{"id":"..\\..\\x","name":"E","relay":"1.2.3.4"},{"id":"0a1b2c3d","name":"ok","relay":"1.2.3.4"}]}"#,
        )
        .unwrap();
        let s = load(&d);
        assert_eq!(s.servers.len(), 1);
        assert_eq!(s.selected, None); // a selection of a dropped or unknown id is cleared
        std::fs::remove_dir_all(&d).unwrap();
    }
}
