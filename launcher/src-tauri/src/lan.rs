//! LAN play (experimental): the environment the runtime reads (`engine/docs/lan-play.md`, section 2).
//!
//! | variable | set when | value |
//! |---|---|---|
//! | `SPLAT3_LAN` | always | `lan`, `lan:<ip>/<prefix>` or `relay:<ipv4>[:<port>]` (default port 47313) |
//! | `SPLAT3_LAN_ROOM` | relay | 1 to 32 of `A-Z a-z 0-9 - _` |
//! | `SPLAT3_LAN_NAME` | relay | the player name, for the relay's log (32 bytes at most) |
//! | `SPLAT3_LAN_BIND` | lan, when asked | `any` |
//! | `SPLAT3_NET_LOG` | when asked | `1` |
//!
//! There is no host or join variable: the role is chosen in the game's lobby. The runtime never resolves names, so a
//! relay is an IPv4 address here (manual play on a network; servers given by name are in `servers.rs`).

use crate::error::{AppError, AppResult, FieldError};
use crate::util;
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

pub const ENV_LAN: &str = "SPLAT3_LAN";
pub const ENV_ROOM: &str = "SPLAT3_LAN_ROOM";
pub const ENV_NAME: &str = "SPLAT3_LAN_NAME";
pub const ENV_BIND: &str = "SPLAT3_LAN_BIND";
pub const ENV_NET_LOG: &str = "SPLAT3_NET_LOG";
pub const DEFAULT_RELAY_PORT: u16 = 47313;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LanMode {
    /// Same local network: the game's own broadcast discovery.
    #[default]
    Lan,
    /// Over the internet through a relay.
    Relay,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LanRequest {
    pub mode: LanMode,
    /// Relay IPv4 address (an IPv4 literal; the runtime resolves nothing).
    #[serde(default)]
    pub relay_host: String,
    #[serde(default = "default_port")]
    pub relay_port: u16,
    #[serde(default)]
    pub room: String,
    /// `SPLAT3_NET_LOG=1`.
    #[serde(default)]
    pub net_log: bool,
    /// lan mode only: `SPLAT3_LAN_BIND=any`.
    #[serde(default)]
    pub bind_any: bool,
    /// lan mode only, optional: `<ip>/<prefix>` to pick an adapter address instead of the automatic one.
    #[serde(default)]
    pub lan_address: String,
}

fn default_port() -> u16 {
    DEFAULT_RELAY_PORT
}

/// Room codes: the generator's alphabet is upper-case letters and digits without look-alikes (the runtime upper-cases codes).
pub const ROOM_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
pub const ROOM_CODE_LEN: usize = 24;

/// A long random room code (24 characters from a 32-character alphabet, 120 bits) from the operating system's
/// random generator; the code is the only access control of a public relay.
pub fn generate_room_code() -> String {
    let mut bytes = [0u8; ROOM_CODE_LEN];
    if getrandom::fill(&mut bytes).is_err() {
        // Never hand out a predictable code: fall back to the hash-based source only if the OS call fails.
        let hex = util::random_hex(ROOM_CODE_LEN);
        for (b, pair) in bytes.iter_mut().zip(hex.as_bytes().chunks(2)) {
            *b = std::str::from_utf8(pair).ok().and_then(|s| u8::from_str_radix(s, 16).ok()).unwrap_or(0);
        }
    }
    // 32 symbols: the low five bits are uniform.
    bytes.iter().map(|b| ROOM_ALPHABET[(*b & 31) as usize] as char).collect()
}

/// 1 to 32 of `A-Z a-z 0-9 - _`. (The runtime sends it in upper case.)
pub fn normalize_room(room: &str) -> Result<String, String> {
    let r = room.trim();
    if r.is_empty() || r.len() > 32 || !r.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("room code: 1 to 32 letters, digits, - or _".into());
    }
    Ok(r.to_string())
}

/// An IPv4 literal (no host names: a relay given by name is a community server, `servers.rs`).
pub fn parse_ipv4(s: &str) -> Result<Ipv4Addr, String> {
    s.trim().parse::<Ipv4Addr>().map_err(|_| "relay: an IPv4 address such as 203.0.113.7 is required (the game never resolves names)".to_string())
}

/// `<a.b.c.d>/<prefix>` (prefix 1 to 32).
pub fn normalize_cidr(s: &str) -> Result<String, String> {
    let (ip, prefix) = s.trim().split_once('/').ok_or("address: expected a.b.c.d/prefix")?;
    let ip = ip.parse::<Ipv4Addr>().map_err(|_| "address: not an IPv4 address".to_string())?;
    let prefix: u8 = prefix.parse().map_err(|_| "address: prefix must be 1 to 32".to_string())?;
    if !(1..=32).contains(&prefix) {
        return Err("address: prefix must be 1 to 32".into());
    }
    Ok(format!("{ip}/{prefix}"))
}

/// The variables a LAN launch adds, in a stable order. Validation errors are per field.
pub fn build_lan_env(req: &LanRequest, player_name: &str) -> AppResult<Vec<(String, String)>> {
    let mut errors = Vec::new();
    let mut env = Vec::new();
    match req.mode {
        LanMode::Lan => {
            let value = if req.lan_address.trim().is_empty() {
                "lan".to_string()
            } else {
                match normalize_cidr(&req.lan_address) {
                    Ok(c) => format!("lan:{c}"),
                    Err(m) => {
                        errors.push(FieldError { key: "lan_address".into(), message: m });
                        String::new()
                    }
                }
            };
            env.push((ENV_LAN.to_string(), value));
            if req.bind_any {
                env.push((ENV_BIND.to_string(), "any".to_string()));
            }
        }
        LanMode::Relay => {
            let ip = parse_ipv4(&req.relay_host).map_err(|m| errors.push(FieldError { key: "relay_host".into(), message: m })).ok();
            if req.relay_port == 0 {
                errors.push(FieldError { key: "relay_port".into(), message: "port must be 1 to 65535".into() });
            }
            let room = normalize_room(&req.room).map_err(|m| errors.push(FieldError { key: "room".into(), message: m })).ok();
            let value = match ip {
                Some(ip) if req.relay_port == DEFAULT_RELAY_PORT => format!("relay:{ip}"),
                Some(ip) => format!("relay:{ip}:{}", req.relay_port),
                None => String::new(),
            };
            env.push((ENV_LAN.to_string(), value));
            env.push((ENV_ROOM.to_string(), room.unwrap_or_default()));
            let name = player_name.trim();
            if !name.is_empty() {
                env.push((ENV_NAME.to_string(), cut_utf8(name, 32)));
            }
        }
    }
    if req.net_log {
        env.push((ENV_NET_LOG.to_string(), "1".to_string()));
    }
    if !errors.is_empty() {
        return Err(AppError::Validation(errors));
    }
    Ok(env)
}

fn cut_utf8(s: &str, max: usize) -> String {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(mode: LanMode) -> LanRequest {
        LanRequest { mode, relay_host: String::new(), relay_port: DEFAULT_RELAY_PORT, room: String::new(), net_log: false, bind_any: false, lan_address: String::new() }
    }

    fn pairs(env: &[(String, String)]) -> Vec<(&str, &str)> {
        env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
    }

    #[test]
    fn lan_mode_sets_only_the_mode() {
        let env = build_lan_env(&req(LanMode::Lan), "Zoe").unwrap();
        assert_eq!(pairs(&env), vec![("SPLAT3_LAN", "lan")]);
        let mut r = req(LanMode::Lan);
        r.bind_any = true;
        r.net_log = true;
        r.lan_address = " 192.168.1.20/24 ".into();
        r.room = "ignored".into();
        let env = build_lan_env(&r, "").unwrap();
        assert_eq!(pairs(&env), vec![("SPLAT3_LAN", "lan:192.168.1.20/24"), ("SPLAT3_LAN_BIND", "any"), ("SPLAT3_NET_LOG", "1")]);
    }

    #[test]
    fn relay_mode_uses_the_default_port_silently() {
        let mut r = req(LanMode::Relay);
        r.relay_host = "203.0.113.7".into();
        r.room = "Abc-123_x".into();
        let env = build_lan_env(&r, "Kay").unwrap();
        assert_eq!(pairs(&env), vec![("SPLAT3_LAN", "relay:203.0.113.7"), ("SPLAT3_LAN_ROOM", "Abc-123_x"), ("SPLAT3_LAN_NAME", "Kay")]);
        r.relay_port = 7777;
        r.net_log = true;
        let env = build_lan_env(&r, "").unwrap();
        assert_eq!(pairs(&env), vec![("SPLAT3_LAN", "relay:203.0.113.7:7777"), ("SPLAT3_LAN_ROOM", "Abc-123_x"), ("SPLAT3_NET_LOG", "1")]);
    }

    #[test]
    fn no_invented_variables() {
        // The old contract (role, player identity, TOTK_LAN*) must be gone.
        let mut r = req(LanMode::Relay);
        r.relay_host = "10.0.0.1".into();
        r.room = "ROOM".into();
        let env = build_lan_env(&r, "Kay").unwrap();
        assert!(env.iter().all(|(k, _)| k.starts_with("SPLAT3_") && !k.contains("ROLE") && !k.contains("PLAYER") && !k.starts_with("TOTK_")));
    }

    #[test]
    fn errors_are_per_field() {
        let mut r = req(LanMode::Relay);
        r.relay_host = "relay.example.org".into();
        r.relay_port = 0;
        r.room = "bad room!".into();
        match build_lan_env(&r, "").unwrap_err() {
            AppError::Validation(v) => assert_eq!(v.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(), vec!["relay_host", "relay_port", "room"]),
            e => panic!("{e:?}"),
        }
        let mut l = req(LanMode::Lan);
        l.lan_address = "10.0.0.1/40".into();
        assert!(build_lan_env(&l, "").is_err());
    }

    #[test]
    fn room_rules_and_generated_codes() {
        assert!(normalize_room("A").is_ok());
        assert!(normalize_room(&"x".repeat(32)).is_ok());
        assert!(normalize_room(&"x".repeat(33)).is_err());
        assert!(normalize_room("").is_err());
        assert!(normalize_room("a b").is_err());
        assert!(normalize_room("é").is_err());
        let a = generate_room_code();
        let b = generate_room_code();
        assert_eq!(a.len(), ROOM_CODE_LEN);
        assert_ne!(a, b);
        assert!(normalize_room(&a).is_ok(), "{a}");
    }

    #[test]
    fn host_names_are_told_apart_from_addresses() {
        assert!(parse_ipv4("relay.example.org").is_err());
        assert!(parse_ipv4("999.1.1.1").is_err());

    }

    #[test]
    fn long_names_are_cut_on_a_character_boundary() {
        let mut r = req(LanMode::Relay);
        r.relay_host = "10.0.0.1".into();
        r.room = "R".into();
        let env = build_lan_env(&r, &"é".repeat(30)).unwrap();
        let name = &env.iter().find(|(k, _)| k == ENV_NAME).unwrap().1;
        assert!(name.len() <= 32 && name.chars().all(|c| c == 'é'));
    }
}
