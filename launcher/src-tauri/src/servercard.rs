//! The "server card" of the first format, a small JSON file an operator hands to players (cards of the community
//! protocol, `{"kalamar_server": 1, ...}`, are read by `proto/discovery.rs`).
//!
//! A card is at most 64 KiB, parsed strictly: these keys, these types, nothing else.
//!
//! ```json
//! { "name": "Friends server", "relay": "relay.example.org:47313", "api": "https://play.example.org" }
//! ```
//!
//! `relay` is a host name or an IPv4 literal with an optional port (older cards name it `match_server`); `api` is
//! optional. Nothing is fetched from the network: the card is a local file. Cards of earlier launchers may also carry
//! `address`, `port` and `ca_pem`, which belonged to a mode this launcher no longer has: they are accepted and ignored.

use crate::error::{AppError, AppResult};
use serde::Deserialize;
use serde::de::IgnoredAny;

pub const MAX_CARD_BYTES: u64 = 64 * 1024;

fn bad(msg: impl Into<String>) -> AppError {
    AppError::InvalidInput(msg.into())
}

pub fn valid_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 40 || n.chars().any(crate::util::unsafe_char) {
        return Err("name: 1 to 40 characters, no control characters".into());
    }
    Ok(n.to_string())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCard {
    name: String,
    #[serde(default)]
    relay: Option<String>,
    /// `https://` address of the community service (called by the protocol client, `proto/`).
    #[serde(default)]
    api: Option<String>,
    /// Older cards: the relay of private matches, under this key.
    #[serde(default)]
    match_server: Option<String>,
    // Keys earlier launchers wrote for a mode that is gone: accepted, never read.
    #[serde(default, rename = "address")]
    _address: Option<IgnoredAny>,
    #[serde(default, rename = "port")]
    _port: Option<IgnoredAny>,
    #[serde(default, rename = "ca_pem")]
    _ca_pem: Option<IgnoredAny>,
}

/// A parsed card. Hosts are host names (preferred) or IPv4 literals; IPv6 is refused (the game is IPv4-only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    pub name: String,
    pub relay: Option<(String, Option<u16>)>,
    pub api: Option<String>,
}

/// Parses a server card. Every defect is an error naming it; nothing is repaired or guessed.
pub fn parse_card(bytes: &[u8]) -> AppResult<Card> {
    if bytes.len() as u64 > MAX_CARD_BYTES {
        return Err(bad(format!("server card: larger than {MAX_CARD_BYTES} bytes")));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| bad("server card: not UTF-8 text"))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let raw: RawCard = serde_json::from_str(text).map_err(|e| bad(format!("server card: {e}")))?;
    let name = valid_name(&raw.name).map_err(|m| bad(format!("server card: {m}")))?;
    let relay_text = raw.relay.as_deref().or(raw.match_server.as_deref()).map(str::trim).filter(|r| !r.is_empty());
    let relay = match relay_text {
        None => None,
        Some(r) => {
            let (h, p) = crate::servers::parse_line(r).map_err(|e| bad(format!("server card, relay: {}", e.message())))?;
            Some((h.text(), p))
        }
    };
    let api = match raw.api.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
        None => None,
        Some(a) => Some(crate::servers::parse_api(a).map_err(|e| bad(format!("server card, api: {}", e.message())))?),
    };
    if relay.is_none() {
        return Err(bad("server card: a relay is needed"));
    }
    Ok(Card { name, relay, api })
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn a_good_card_parses() {
        let c = parse_card(br#"{"name":"Test","relay":"203.0.113.8"}"#).unwrap();
        assert_eq!((c.name.as_str(), c.relay.clone(), c.api.clone()), ("Test", Some(("203.0.113.8".into(), None)), None));
        let c = parse_card(br#"{"name":"Test","relay":"203.0.113.8:5000"}"#).unwrap();
        assert_eq!(c.relay, Some(("203.0.113.8".into(), Some(5000))));
        let c = parse_card(br#"{"name":"Test","relay":"Relay.Example.org:47313","api":"https://play.example.org"}"#).unwrap();
        assert_eq!((c.relay.clone(), c.api.as_deref()), (Some(("relay.example.org".into(), Some(47313))), Some("https://play.example.org")));
        let c = parse_card(br#"{"name":"Old","match_server":"198.51.100.4:47313"}"#).unwrap();
        assert_eq!(c.relay, Some(("198.51.100.4".into(), Some(47313))));
        let mut with_bom = vec![0xEF, 0xBB, 0xBF];
        with_bom.extend(br#"{"name":"Test","relay":"203.0.113.8"}"#);
        assert!(parse_card(&with_bom).is_ok());
    }

    #[test]
    fn keys_of_earlier_cards_are_ignored() {
        // a card of an earlier launcher: its relay is kept, the rest is not read
        let c = parse_card(br#"{"name":"Friends","address":"203.0.113.7","port":47314,"relay":"203.0.113.8:47313","ca_pem":"-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n"}"#).unwrap();
        assert_eq!(c, Card { name: "Friends".into(), relay: Some(("203.0.113.8".into(), Some(47313))), api: None });
        // without a relay such a card describes nothing this launcher can use
        assert!(parse_card(br#"{"name":"Friends","address":"203.0.113.7","port":47314,"ca_pem":"x"}"#).is_err());
    }

    #[test]
    fn a_bad_card_is_refused() {
        let cases: Vec<(&str, &str)> = vec![
            ("", "empty"),
            ("[]", "array"),
            ("{", "truncated"),
            (r#"{"name":"T","relay":"203.0.113.8","extra":1}"#, "unknown key"),
            (r#"{"name":5,"relay":"203.0.113.8"}"#, "name as a number"),
            (r#"{"name":"","relay":"203.0.113.8"}"#, "empty name"),
            (r#"{"name":"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx","relay":"203.0.113.8"}"#, "name too long"),
            (r#"{"name":"a\u0007b","relay":"203.0.113.8"}"#, "control character in the name"),
            (r#"{"name":"T"}"#, "no relay"),
            (r#"{"name":"T","relay":null,"api":"https://play.example.org"}"#, "a null relay"),
            (r#"{"name":"T","relay":"exa mple.org"}"#, "bad host name"),
            (r#"{"name":"T","relay":"[2001:db8::1]:47313"}"#, "IPv6 relay"),
            (r#"{"name":"T","relay":"0.0.0.0"}"#, "unspecified"),
            (r#"{"name":"T","relay":"224.0.0.1"}"#, "multicast"),
            (r#"{"name":"T","relay":"300.1.1.1"}"#, "octet over 255"),
            (r#"{"name":"T","relay":5}"#, "relay as a number"),
            (r#"{"name":"T","relay":"1.2.3.4:0"}"#, "relay port 0"),
            (r#"{"name":"T","relay":"1.2.3.4:99999"}"#, "relay port too large"),
            (r#"{"name":"T","relay":"r.example.org","api":"http://x.example.org"}"#, "api not https"),
            (r#"{"relay":"203.0.113.8"}"#, "name missing"),
        ];
        for (json, why) in cases {
            assert!(parse_card(json.as_bytes()).is_err(), "{why}: {json}");
        }
        assert!(parse_card(&[0xFF, 0xFE, 0x00]).is_err(), "not UTF-8");
    }

    #[test]
    fn an_oversized_card_is_refused_before_parsing() {
        let big = format!(r#"{{"name":"{}","relay":"203.0.113.8"}}"#, "x".repeat(MAX_CARD_BYTES as usize));
        let e = parse_card(big.as_bytes()).unwrap_err().to_string();
        assert!(e.contains("larger than"), "{e}");
    }
}
