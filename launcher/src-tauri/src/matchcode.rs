//! The match code: everything a guest needs to join a private match, in one copy-pasteable string.
//!
//! There is no central service yet, so the host's launcher builds the code and the players share it by hand. It holds
//! no secret of the account kind: the relay room is the only "password" of a relay match, and it is a per-match random
//! value that the guests must know anyway.
//!
//! Text form: `KLM1-XXXXX-XXXXX-...` (prefix = product code + version digit, then the payload in base32 with the
//! alphabet of the room codes, grouped by five; case, spaces and dashes are free when reading).
//!
//! Payload (version 1), big-endian, then a CRC-16/CCITT-FALSE of everything before it:
//!
//! | offset | size | field |
//! |---|---|---|
//! | 0 | 1 | version (1) |
//! | 1 | 4 | relay IPv4 |
//! | 5 | 2 | relay port |
//! | 7 | 1 | players (2-8) in the high nibble, mode index in the low nibble (`MODES`) |
//! | 8 | 1 | round (0 for the first match of the group, +1 per "play again") |
//! | 9 | 15 | room base: 24 symbols of 5 bits (the room-code alphabet) |
//! | 24 | 1 | host nickname length n (1-32) |
//! | 25 | n | host nickname, UTF-8 |
//! | 25+n | 2 | CRC-16 |
//!
//! Version 2 is the same payload with the relay's host name after the version byte: | 1 | 1 | host name length h (1-100) |,
//! | 2 | h | host name, lower-case ASCII |, then version 1's fields from the IPv4 on (the IPv4 is the address the name had
//! when the code was made: a fallback for a guest whose resolver fails). A code made for a relay given as an IPv4 is
//! still version 1, unchanged. Each guest's launcher resolves the name itself when the match starts.
//!
//! The match id the runtime gets (`SPLAT3_MATCH_ID`, and the relay room, which must be the same value) is the room
//! base for round 0 and `<room base>-<round>` after, at most 28 characters. "Play again" is therefore the same code
//! with the round incremented, which every member of the group can compute alone.

use crate::lan::ROOM_ALPHABET;
use serde::Serialize;
use std::net::Ipv4Addr;

/// Product code in front of the version digit. Change together with the product name in `src/brand.ts`.
pub const PREFIX: &str = "KLM";
pub const VERSION: u8 = 1;
/// Codes whose relay is a host name.
pub const VERSION_NAMED: u8 = 2;
pub const MAX_RELAY_HOST: usize = 100;
/// Rule names as the runtime spells them (`SPLAT3_MATCH_RULES`, `mode=`); the index is stored in the code.
pub const MODES: [&str; 5] = ["turf", "zones", "tower", "rainmaker", "clams"];
pub const ROOM_LEN: usize = 24;
pub const MAX_HOST_BYTES: usize = 32;
pub const MIN_PLAYERS: u8 = 2;
pub const MAX_PLAYERS: u8 = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatchSpec {
    /// The relay's host name when the server was given by name (version 2 codes); the game gets `relay_ip`, resolved
    /// from it at launch.
    pub relay_host: Option<String>,
    pub relay_ip: Ipv4Addr,
    pub relay_port: u16,
    /// 24 symbols of the room-code alphabet.
    pub room: String,
    pub round: u8,
    pub host: String,
    pub players: u8,
    pub mode: String,
}

/// Why a code was refused. `id()` is the key the UI translates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeError {
    Empty,
    NotACode,
    UnsupportedVersion(u8),
    BadCharacter(char),
    Truncated,
    Checksum,
    Field(&'static str),
}

impl CodeError {
    pub fn id(&self) -> &'static str {
        match self {
            CodeError::Empty => "empty",
            CodeError::NotACode => "not_a_code",
            CodeError::UnsupportedVersion(_) => "version",
            CodeError::BadCharacter(_) => "character",
            CodeError::Truncated => "truncated",
            CodeError::Checksum => "checksum",
            CodeError::Field(_) => "field",
        }
    }
}

impl MatchSpec {
    /// The value of `SPLAT3_MATCH_ID` and `SPLAT3_LAN_ROOM`.
    pub fn match_id(&self) -> String {
        if self.round == 0 { self.room.clone() } else { format!("{}-{}", self.room, self.round) }
    }

    pub fn next_round(&self) -> Option<MatchSpec> {
        Some(MatchSpec { round: self.round.checked_add(1)?, ..self.clone() })
    }

    /// Checks the fields an encoded code must satisfy.
    pub fn validate(&self) -> Result<(), CodeError> {
        if self.room.len() != ROOM_LEN || !self.room.bytes().all(|b| ROOM_ALPHABET.contains(&b)) {
            return Err(CodeError::Field("room"));
        }
        if !(MIN_PLAYERS..=MAX_PLAYERS).contains(&self.players) {
            return Err(CodeError::Field("players"));
        }
        if !MODES.contains(&self.mode.as_str()) {
            return Err(CodeError::Field("mode"));
        }
        let h = self.host.trim();
        if h.is_empty() || h.len() > MAX_HOST_BYTES || h != self.host || h.chars().any(char::is_control) {
            return Err(CodeError::Field("host"));
        }
        if self.relay_port == 0 {
            return Err(CodeError::Field("port"));
        }
        if let Some(h) = &self.relay_host
            && (h.is_empty() || h.len() > MAX_RELAY_HOST || !matches!(crate::servers::parse_host(h), Ok(crate::servers::Host::Name(ref n)) if n == h))
        {
            return Err(CodeError::Field("relay"));
        }
        Ok(())
    }
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

fn b32_encode(bytes: &[u8]) -> String {
    let (mut acc, mut bits, mut out) = (0u32, 0u32, String::new());
    for &b in bytes {
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ROOM_ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
        acc &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(ROOM_ALPHABET[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn b32_decode(s: &str) -> Result<Vec<u8>, CodeError> {
    let (mut acc, mut bits, mut out) = (0u32, 0u32, Vec::new());
    for c in s.chars() {
        let v = u8::try_from(c).ok().and_then(|b| ROOM_ALPHABET.iter().position(|&a| a == b)).ok_or(CodeError::BadCharacter(c))?;
        acc = (acc << 5) | v as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    // Leftover bits are padding: fewer than five, and zero.
    if acc != 0 {
        return Err(CodeError::Checksum);
    }
    Ok(out)
}

fn pack_room(room: &str) -> Vec<u8> {
    let (mut acc, mut bits, mut out) = (0u32, 0u32, Vec::new());
    for b in room.bytes() {
        let v = ROOM_ALPHABET.iter().position(|&a| a == b).unwrap_or(0) as u32;
        acc = (acc << 5) | v;
        bits += 5;
        while bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
        acc &= (1 << bits) - 1;
    }
    out
}

fn unpack_room(bytes: &[u8]) -> String {
    let (mut acc, mut bits, mut out) = (0u32, 0u32, String::new());
    for &b in bytes {
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ROOM_ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
        acc &= (1 << bits) - 1;
    }
    out
}

/// The code for a spec; the spec must validate.
pub fn encode(spec: &MatchSpec) -> Result<String, CodeError> {
    spec.validate()?;
    let mode = MODES.iter().position(|m| *m == spec.mode).unwrap_or(0) as u8;
    let version = if spec.relay_host.is_some() { VERSION_NAMED } else { VERSION };
    let mut p = vec![version];
    if let Some(h) = &spec.relay_host {
        p.push(h.len() as u8);
        p.extend(h.as_bytes());
    }
    p.extend(spec.relay_ip.octets());
    p.extend(spec.relay_port.to_be_bytes());
    p.push((spec.players << 4) | mode);
    p.push(spec.round);
    p.extend(pack_room(&spec.room));
    p.push(spec.host.len() as u8);
    p.extend(spec.host.as_bytes());
    let crc = crc16(&p);
    p.extend(crc.to_be_bytes());
    let body = b32_encode(&p);
    let groups: Vec<&str> = body.as_bytes().chunks(5).map(|c| std::str::from_utf8(c).unwrap_or("")).collect();
    Ok(format!("{PREFIX}{version}-{}", groups.join("-")))
}

/// Reads a code typed or pasted by a person: case, spaces, dashes and surrounding text such as quotes are tolerated.
pub fn decode(text: &str) -> Result<MatchSpec, CodeError> {
    let cleaned: String = text.chars().filter(|c| !c.is_whitespace() && *c != '-' && *c != '"' && *c != '\'').map(|c| c.to_ascii_uppercase()).collect();
    if cleaned.is_empty() {
        return Err(CodeError::Empty);
    }
    let rest = cleaned.strip_prefix(PREFIX).ok_or(CodeError::NotACode)?;
    let mut chars = rest.chars();
    let version = chars.next().and_then(|c| c.to_digit(10)).ok_or(CodeError::NotACode)? as u8;
    if version != VERSION && version != VERSION_NAMED {
        return Err(CodeError::UnsupportedVersion(version));
    }
    let bytes = b32_decode(chars.as_str())?;
    if bytes.len() < 25 + 1 + 2 {
        return Err(CodeError::Truncated);
    }
    let (all, crc) = bytes.split_at(bytes.len() - 2);
    if crc16(all) != u16::from_be_bytes([crc[0], crc[1]]) {
        return Err(CodeError::Checksum);
    }
    if all[0] != version {
        return Err(CodeError::UnsupportedVersion(all[0]));
    }
    // Version 2: the relay's host name, then version 1's layout from the IPv4 on.
    let (relay_host, owned): (Option<String>, Vec<u8>) = if version == VERSION_NAMED {
        let h = *all.get(1).ok_or(CodeError::Truncated)? as usize;
        let name = all.get(2..2 + h).ok_or(CodeError::Truncated)?;
        let name = std::str::from_utf8(name).map_err(|_| CodeError::Field("relay"))?.to_string();
        let mut v1 = vec![VERSION];
        v1.extend(all.get(2 + h..).ok_or(CodeError::Truncated)?);
        (Some(name), v1)
    } else {
        (None, all.to_vec())
    };
    let body = &owned[..];
    if body.len() < 25 {
        return Err(CodeError::Truncated);
    }
    let n = body[24] as usize;
    if body.len() != 25 + n {
        return Err(CodeError::Truncated);
    }
    let host = std::str::from_utf8(&body[25..]).map_err(|_| CodeError::Field("host"))?.to_string();
    let mode = *MODES.get((body[7] & 15) as usize).ok_or(CodeError::Field("mode"))?;
    let spec = MatchSpec {
        relay_host,
        relay_ip: Ipv4Addr::new(body[1], body[2], body[3], body[4]),
        relay_port: u16::from_be_bytes([body[5], body[6]]),
        players: body[7] >> 4,
        mode: mode.to_string(),
        round: body[8],
        room: unpack_room(&body[9..24]),
        host,
    };
    spec.validate()?;
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> MatchSpec {
        MatchSpec {
            relay_host: None,
            relay_ip: Ipv4Addr::new(203, 0, 113, 7),
            relay_port: 47313,
            room: "ABCDEFGHJKLMNPQRSTUVWXYZ".into(),
            round: 0,
            host: "Kay".into(),
            players: 2,
            mode: "turf".into(),
        }
    }

    #[test]
    fn a_relay_given_by_name_travels_in_a_version_2_code() {
        let s = MatchSpec { relay_host: Some("relay.example.org".into()), ..spec() };
        let code = encode(&s).unwrap();
        assert!(code.starts_with("KLM2-"), "{code}");
        let back = decode(&code).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.relay_ip, Ipv4Addr::new(203, 0, 113, 7), "the fallback address travels too");
        // without a name the code stays version 1, byte for byte as before
        assert!(encode(&spec()).unwrap().starts_with("KLM1-"));
        // names are lower-case host names only, and not too long
        for bad in ["Relay.Example.org", "203.0.113.7", "a b", ""] {
            assert_eq!(encode(&MatchSpec { relay_host: Some(bad.into()), ..spec() }).unwrap_err(), CodeError::Field("relay"), "{bad}");
        }
        let long = format!("{}.org", "a.".repeat(60));
        assert_eq!(encode(&MatchSpec { relay_host: Some(long), ..spec() }).unwrap_err(), CodeError::Field("relay"));
        // a changed symbol is caught by the checksum
        let mut chars: Vec<char> = code.chars().collect();
        let at = 8;
        chars[at] = if chars[at] == 'A' { 'B' } else { 'A' };
        assert!(decode(&chars.into_iter().collect::<String>()).is_err());
    }
    #[test]
    fn a_version_1_code_made_before_servers_by_name_is_still_accepted() {
        // Produced by the launcher before KLM2 existed (master 6679cf1).
        let old = "KLM1-AHFSA-6JHZD-JUAAA-AJS3B-KT4CL-U5DMV-6ENW7-FPX2D-KPSZV-YW7";
        assert_eq!(decode(old).unwrap(), spec());
        assert_eq!(encode(&spec()).unwrap(), old, "version 1 codes are unchanged byte for byte");
    }

    #[test]
    fn round_trip_keeps_every_field() {
        for (players, mode, host, round) in [(2, "turf", "Kay", 0), (8, "clams", "Émilie 🦑", 7), (4, "zones", "x", 255), (3, "rainmaker", &"é".repeat(16)[..], 1)] {
            let s = MatchSpec { players, mode: mode.into(), host: host.into(), round, ..spec() };
            let code = encode(&s).unwrap();
            assert!(code.starts_with("KLM1-"), "{code}");
            assert_eq!(decode(&code).unwrap(), s, "{code}");
        }
        let code = encode(&spec()).unwrap();
        assert!(code.len() < 70, "{} {code}", code.len());
        // Groups of five, nothing but the alphabet.
        assert!(code[5..].split('-').all(|g| g.len() <= 5 && g.bytes().all(|b| ROOM_ALPHABET.contains(&b))), "{code}");
    }

    #[test]
    fn reading_is_forgiving_about_how_it_was_copied() {
        let s = spec();
        let code = encode(&s).unwrap();
        assert_eq!(decode(&code.to_lowercase()).unwrap(), s);
        assert_eq!(decode(&format!("  \"{}\"\n", code.replace('-', " "))).unwrap(), s);
        assert_eq!(decode(&code.replace('-', "")).unwrap(), s);
    }

    #[test]
    fn corruption_is_detected() {
        let code = encode(&spec()).unwrap();
        // Change any single symbol of the body: never accepted as the same match.
        let body_start = 5;
        for i in body_start..code.len() {
            let c = code.as_bytes()[i];
            if c == b'-' {
                continue;
            }
            let other = ROOM_ALPHABET[(ROOM_ALPHABET.iter().position(|&a| a == c).unwrap() + 1) % 32] as char;
            let mut bad = code.clone();
            bad.replace_range(i..i + 1, &other.to_string());
            match decode(&bad) {
                Err(CodeError::Checksum) | Err(CodeError::Truncated) | Err(CodeError::Field(_)) => {}
                other => panic!("symbol {i} of {code}: {other:?}"),
            }
        }
        // A dropped symbol and a cut code.
        assert!(decode(&code[..code.len() - 1]).is_err());
        assert!(decode(&code[..20]).is_err());
        assert_eq!(decode("KLM1-AAAA").unwrap_err(), CodeError::Truncated);
        // A symbol outside the alphabet (0, 1, O, I are not used).
        let mut bad = code.clone();
        bad.replace_range(6..7, "0");
        assert_eq!(decode(&bad).unwrap_err(), CodeError::BadCharacter('0'));
    }

    #[test]
    fn version_and_prefix_are_checked_first() {
        let code = encode(&spec()).unwrap();
        assert_eq!(decode(&code.replacen("KLM1", "KLM3", 1)).unwrap_err(), CodeError::UnsupportedVersion(3));
        // a version 1 payload under a version 2 prefix: refused (the payload says 1)
        assert_eq!(decode(&code.replacen("KLM1", "KLM2", 1)).unwrap_err(), CodeError::UnsupportedVersion(1));
        assert_eq!(decode("ABC1-AAAAA").unwrap_err(), CodeError::NotACode);
        assert_eq!(decode("   ").unwrap_err(), CodeError::Empty);
        assert_eq!(decode("hello world").unwrap_err(), CodeError::NotACode);
        // Payload version byte different from the prefix digit, with a valid checksum: refused too.
        let mut p = vec![2u8];
        p.extend([203, 0, 113, 7, 0xB7, 0x91, 0x20, 0]);
        p.extend(pack_room(&spec().room));
        p.push(3);
        p.extend(b"Kay");
        p.extend(crc16(&p).to_be_bytes());
        assert_eq!(decode(&format!("KLM1{}", b32_encode(&p))).unwrap_err(), CodeError::UnsupportedVersion(2));
    }

    #[test]
    fn invalid_specs_are_not_encoded() {
        assert_eq!(encode(&MatchSpec { players: 1, ..spec() }).unwrap_err(), CodeError::Field("players"));
        assert_eq!(encode(&MatchSpec { players: 9, ..spec() }).unwrap_err(), CodeError::Field("players"));
        assert_eq!(encode(&MatchSpec { mode: "ranked".into(), ..spec() }).unwrap_err(), CodeError::Field("mode"));
        assert_eq!(encode(&MatchSpec { host: String::new(), ..spec() }).unwrap_err(), CodeError::Field("host"));
        assert_eq!(encode(&MatchSpec { host: "x".repeat(33), ..spec() }).unwrap_err(), CodeError::Field("host"));
        assert_eq!(encode(&MatchSpec { host: " pad".into(), ..spec() }).unwrap_err(), CodeError::Field("host"));
        assert_eq!(encode(&MatchSpec { room: "short".into(), ..spec() }).unwrap_err(), CodeError::Field("room"));
        assert_eq!(encode(&MatchSpec { room: "0".repeat(24), ..spec() }).unwrap_err(), CodeError::Field("room"));
        assert_eq!(encode(&MatchSpec { relay_port: 0, ..spec() }).unwrap_err(), CodeError::Field("port"));
    }

    #[test]
    fn match_id_and_rounds() {
        let s = spec();
        assert_eq!(s.match_id(), s.room);
        let n = s.next_round().unwrap();
        assert_eq!(n.round, 1);
        assert_eq!(n.match_id(), format!("{}-1", s.room));
        assert!(n.match_id().len() <= 28 && crate::lan::normalize_room(&n.match_id()).is_ok());
        assert_eq!(decode(&encode(&n).unwrap()).unwrap(), n);
        assert!(MatchSpec { round: 255, ..s }.next_round().is_none());
    }

    #[test]
    fn a_fresh_room_makes_a_valid_code() {
        let room = crate::lan::generate_room_code();
        let s = MatchSpec { room, ..spec() };
        assert_eq!(decode(&encode(&s).unwrap()).unwrap(), s);
    }
}
