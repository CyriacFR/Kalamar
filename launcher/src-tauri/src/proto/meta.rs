//! `GET /v1/meta` (`api.md`, "Meta"): the authoritative description of what a server offers. The launcher keeps the
//! document as given (the interface renders the server's declarations as data) and checks only what it must act on:
//! a common protocol version, a match contract it implements, the declared areas, the login providers, the limits.

use crate::proto::contract;
use crate::proto::discovery::{common_version, localised, parse_relay};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// Areas this launcher knows (`overview.md` 2). Unknown ones a server declares are ignored.
pub const AREAS: &[&str] = &["rooms", "queues", "parties", "friends", "presence", "ratings", "leaderboards", "stats", "moderation", "activities", "series", "badges", "profiles", "population", "draft", "spectators"];

/// The longest display name this launcher lets a player choose: the game shows at most 10 characters.
pub const DISPLAY_NAME_MAX: usize = 10;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Compat {
    pub ok: bool,
    /// `protocol` (no common 1.x version: another launcher is needed), `contract` (no match contract this launcher
    /// implements), `auth` (no login provider this launcher can use), `invalid` (the document is not a meta document).
    pub reason: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetaCheck {
    pub compat: Compat,
    /// Declared areas this launcher knows, in its order.
    pub features: Vec<String>,
    pub relay: Option<String>,
    pub name: String,
    /// The server's clock, RFC 3339, for countdowns.
    pub time: Option<String>,
    pub display_name_max: usize,
    pub display_name_min: usize,
}

fn providers_usable(meta: &Value) -> bool {
    meta.pointer("/auth/providers").and_then(Value::as_array).is_some_and(|l| {
        l.iter().any(|p| matches!(p.get("kind").and_then(Value::as_str), Some("direct") | Some("oauth_loopback")) && p.get("id").and_then(Value::as_str).is_some_and(valid_id))
    })
}

/// Ids of the protocol: 1 to 64 of `A-Z a-z 0-9 _ -`.
pub fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub fn check(meta: &Value) -> MetaCheck {
    let features: Vec<String> = meta
        .get("features")
        .and_then(Value::as_array)
        .map(|a| AREAS.iter().filter(|k| a.iter().any(|x| x.as_str() == Some(k))).map(|k| k.to_string()).collect())
        .unwrap_or_default();
    let versions: Vec<String> = meta
        .pointer("/protocol/versions")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let version = common_version(&versions);
    let contract_ok = meta.get("game_contract").is_some_and(|g| {
        g.get("id").and_then(Value::as_str) == Some(contract::CONTRACT_ID)
            && g.get("versions").and_then(Value::as_array).is_some_and(|v| v.iter().any(|x| x.as_u64().is_some_and(|n| contract::CONTRACT_VERSIONS.contains(&(n as u32)))))
    });
    let reason = if !meta.is_object() || meta.get("features").is_none() {
        Some("invalid")
    } else if version.is_none() {
        Some("protocol")
    } else if !contract_ok {
        Some("contract")
    } else if !providers_usable(meta) {
        Some("auth")
    } else {
        None
    };
    let lim = |k: &str| meta.pointer(&format!("/limits/display_name/{k}")).and_then(Value::as_u64).map(|n| n as usize);
    MetaCheck {
        compat: Compat { ok: reason.is_none(), reason: reason.map(str::to_string), version },
        features,
        relay: meta.get("relay").and_then(|r| parse_relay(r).ok().flatten()).map(|r| r.text()),
        // No control or hidden format character (a bidirectional override reorders what the player reads).
        name: meta.get("name").and_then(Value::as_str).unwrap_or_default().chars().filter(|c| !crate::util::unsafe_char(*c)).take(64).collect(),
        time: meta.get("time").and_then(Value::as_str).map(str::to_string),
        display_name_max: lim("max_len").unwrap_or(DISPLAY_NAME_MAX).clamp(1, DISPLAY_NAME_MAX),
        display_name_min: lim("min_len").unwrap_or(1).clamp(1, DISPLAY_NAME_MAX),
    }
}

/// What the Play page shows of a server before its community page is opened: the description and the declared areas of
/// its last meta, kept in `online\<id>\meta-summary.json` (public data the server gave, a few hundred bytes). Written
/// when the launcher reads the meta anyway (adding the server, opening its page), read without any request.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Summary {
    /// Localised plain text: language tag -> text (the page sets it as text, never as markup).
    pub description: Value,
    /// Declared areas this launcher knows, in its order.
    pub features: Vec<String>,
}

const SUMMARY_FILE: &str = "meta-summary.json";
const SUMMARY_TEXT_MAX: usize = 400;
const SUMMARY_LANGS_MAX: usize = 16;

impl Summary {
    /// Keeps strings only (at most 400 characters, 16 languages) and the areas this launcher knows.
    pub fn new(description: &Value, features: &[String]) -> Summary {
        let description = match description {
            Value::Object(m) => Value::Object(
                m.iter()
                    .filter(|(k, _)| !k.is_empty() && k.len() <= 16)
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), Value::String(s.chars().take(SUMMARY_TEXT_MAX).collect()))))
                    .take(SUMMARY_LANGS_MAX)
                    .collect(),
            ),
            _ => Value::Object(Default::default()),
        };
        Summary { description, features: AREAS.iter().filter(|a| features.iter().any(|f| f == *a)).map(|a| a.to_string()).collect() }
    }

    pub fn of_meta(meta: &Value, check: &MetaCheck) -> Summary {
        Summary::new(&localised(meta.get("description")), &check.features)
    }
}

pub fn load_summary(server_dir: &Path) -> Option<Summary> {
    let bytes = std::fs::read(server_dir.join(SUMMARY_FILE)).ok().filter(|b| b.len() <= 64 * 1024)?;
    let s: Summary = serde_json::from_slice(&bytes).ok()?;
    Some(Summary::new(&s.description, &s.features))
}

/// Writes the summary when it differs from the one kept (opening the page again writes nothing).
pub fn remember_summary(server_dir: &Path, s: &Summary) {
    if load_summary(server_dir).as_ref() == Some(s) {
        return;
    }
    if let Ok(b) = serde_json::to_vec(s) {
        let _ = crate::util::atomic_write(&server_dir.join(SUMMARY_FILE), &b);
    }
}

/// A login provider of `auth.providers`.
pub fn provider<'a>(meta: &'a Value, id: &str) -> Option<&'a Value> {
    meta.pointer("/auth/providers")?.as_array()?.iter().find(|p| p.get("id").and_then(Value::as_str) == Some(id))
}

/// The area a request path belongs to (`api.md`, endpoint index), `"core"` for the core, `None` for paths the generic
/// request command never sends (logins, tokens, the event stream, the match reports the runner owns, test hooks,
/// operator extensions). `"queues|draft"`: either area allows it (a decline answers a found match, or leaves a draft).
pub fn area_of(method: &str, path: &str) -> Option<&'static str> {
    let p = path.split('?').next().unwrap_or("");
    let seg: Vec<&str> = p.trim_start_matches('/').split('/').collect();
    if seg.first() != Some(&"v1") || seg.iter().any(|s| s.is_empty() || *s == ".." || *s == ".") {
        return None;
    }
    let s = |i: usize| seg.get(i).copied().unwrap_or("");
    match s(1) {
        "auth" | "events" | "_test" => None,
        x if x.starts_with("x-") => None,
        "me" if s(2) == "game" => None,
        "me" if s(2) == "friend-code" => Some("friends"),
        "me" | "state" => Some("core"),
        "matches" => match s(3) {
            "" => Some("core"),
            "accept" if method == "POST" => Some("queues"),
            "decline" if method == "POST" => Some("queues|draft"),
            "draft" if method == "POST" => Some("draft"),
            // 1.1.7: a place in the stands, taken or left.
            "spectate" if method == "POST" || method == "DELETE" => Some("spectators"),
            _ => None, // launch, status, result: the match runner only
        },
        "users" => match s(3) {
            "" => Some("core"),
            "ratings" => Some("ratings"),
            "stats" | "matches" => Some("stats"),
            "badges" => Some("badges"),
            "profile" => Some("profiles"),
            _ => None,
        },
        "activities" => Some("activities"),
        "series" if s(3) == "spectate" => Some("spectators"),
        "series" => Some("series"),
        "spectate" => Some("spectators"),
        "titles" => Some("badges"),
        "rooms" => Some("rooms"),
        "queues" | "tickets" => Some("queues"),
        "party" | "parties" => Some("parties"),
        "friends" => Some("friends"),
        "ladders" if s(3) == "leaderboard" => Some("leaderboards"),
        "ladders" | "seasons" => Some("ratings"),
        "reports" | "blocks" => Some("moderation"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta() -> Value {
        json!({
            "name": "Local test server", "protocol": {"versions": ["1.0"]}, "time": "2026-11-11T03:44:21Z",
            "features": ["rooms", "queues", "teleport", "presence"], "relay": "127.0.0.1:47313",
            "game_contract": {"id": "lan-auto-match", "versions": [1]},
            "limits": {"display_name": {"min_len": 1, "max_len": 16}},
            "auth": {"providers": [{"id": "guest", "kind": "direct", "fields": []}, {"id": "magic", "kind": "telepathy"}]},
        })
    }

    #[test]
    fn a_compatible_server() {
        let c = check(&meta());
        assert!(c.compat.ok, "{:?}", c.compat);
        assert_eq!(c.compat.version.as_deref(), Some("1.0"));
        assert_eq!(c.features, ["rooms", "queues", "presence"]); // the unknown area is ignored
        assert_eq!(c.relay.as_deref(), Some("127.0.0.1:47313"));
        assert_eq!(c.display_name_max, 10, "never more than the game shows");
        assert!(provider(&meta(), "guest").is_some() && provider(&meta(), "nope").is_none());
    }

    #[test]
    fn the_summary_keeps_plain_text_and_known_areas_and_is_written_once() {
        let mut m = meta();
        m["description"] = json!({"fr": "Serveur de test", "en": "x".repeat(900), "de": 3, "": "empty tag"});
        let s = Summary::of_meta(&m, &check(&m));
        assert_eq!(s.features, ["rooms", "queues", "presence"]);
        assert_eq!(s.description["fr"], "Serveur de test");
        assert_eq!(s.description["en"].as_str().unwrap().chars().count(), SUMMARY_TEXT_MAX);
        assert!(s.description.get("de").is_none() && s.description.get("").is_none(), "{:?}", s.description);

        let dir = std::env::temp_dir().join(format!("kalamar-summary-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load_summary(&dir), None);
        remember_summary(&dir, &s);
        assert_eq!(load_summary(&dir).as_ref(), Some(&s));
        let written = std::fs::metadata(dir.join(SUMMARY_FILE)).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        remember_summary(&dir, &s);
        assert_eq!(std::fs::metadata(dir.join(SUMMARY_FILE)).unwrap().modified().unwrap(), written, "unchanged: not rewritten");
        // A file edited by hand is cleaned the same way when read.
        std::fs::write(dir.join(SUMMARY_FILE), br#"{"description":{"fr":"ok","es":["no"]},"features":["teleport","rooms"]}"#).unwrap();
        let back = load_summary(&dir).unwrap();
        assert_eq!(back.features, ["rooms"]);
        assert_eq!(back.description, json!({"fr": "ok"}));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn incompatible_servers_say_why() {
        let mut m = meta();
        m["protocol"]["versions"] = json!(["2.0"]);
        assert_eq!(check(&m).compat.reason.as_deref(), Some("protocol"));
        let mut m = meta();
        m["game_contract"] = json!({"id": "lan-auto-match", "versions": [3]});
        assert_eq!(check(&m).compat.reason.as_deref(), Some("contract"));
        let mut m = meta();
        m["auth"]["providers"] = json!([{"id": "magic", "kind": "telepathy"}]);
        assert_eq!(check(&m).compat.reason.as_deref(), Some("auth"));
        assert_eq!(check(&json!([1])).compat.reason.as_deref(), Some("invalid"));
    }

    #[test]
    fn paths_map_to_areas() {
        for (m, p, a) in [
            ("GET", "/v1/me", Some("core")),
            ("PATCH", "/v1/me", Some("core")),
            ("GET", "/v1/state", Some("core")),
            ("GET", "/v1/users/u_1", Some("core")),
            ("GET", "/v1/users/u_1/ratings?season=s3", Some("ratings")),
            ("GET", "/v1/users/u_1/matches?limit=5", Some("stats")),
            ("GET", "/v1/matches/current", Some("core")),
            ("GET", "/v1/matches/m_1", Some("core")),
            ("POST", "/v1/matches/m_1/accept", Some("queues")),
            ("POST", "/v1/matches/m_1/decline", Some("queues|draft")),
            ("POST", "/v1/matches/m_1/draft", Some("draft")),
            ("GET", "/v1/spectate", Some("spectators")),
            ("POST", "/v1/matches/m_1/spectate", Some("spectators")),
            ("DELETE", "/v1/matches/m_1/spectate", Some("spectators")),
            ("POST", "/v1/series/sr_1/spectate", Some("spectators")),
            ("GET", "/v1/series/sr_1", Some("series")),
            ("GET", "/v1/matches/m_1/draft", None),
            ("POST", "/v1/matches/m_1/launch", None),
            ("POST", "/v1/matches/m_1/result", None),
            ("POST", "/v1/rooms/join", Some("rooms")),
            ("POST", "/v1/queues/duel/tickets", Some("queues")),
            ("DELETE", "/v1/tickets/current", Some("queues")),
            ("POST", "/v1/party/invites", Some("parties")),
            ("POST", "/v1/parties/p_1/join", Some("parties")),
            ("POST", "/v1/me/friend-code", Some("friends")),
            ("GET", "/v1/friends/requests", Some("friends")),
            ("GET", "/v1/ladders/duel/leaderboard?cursor=x", Some("leaderboards")),
            ("GET", "/v1/ladders", Some("ratings")),
            ("PUT", "/v1/blocks/u_2", Some("moderation")),
            ("POST", "/v1/auth/guest/login", None),
            ("POST", "/v1/events/ticket", None),
            ("PUT", "/v1/me/game", None),
            ("POST", "/v1/_test/clock", None),
            ("GET", "/v1/x-shop/items", None),
            ("GET", "/v1/activities", Some("activities")),
            ("POST", "/v1/activities/cup/actions/register", Some("activities")),
            ("GET", "/v1/series/sr_1", Some("series")),
            ("GET", "/v1/users/u_1/badges", Some("badges")),
            ("GET", "/v1/titles", Some("badges")),
            ("GET", "/v1/users/u_1/profile", Some("profiles")),
            ("GET", "/v1/operator/matches", None),
            ("GET", "/v2/meta", None),
            ("GET", "/v1/rooms/../auth", None),
            ("GET", "https://evil.example/v1/me", None),
        ] {
            assert_eq!(area_of(m, p), a, "{m} {p}");
        }
    }
}
