//! The discovery document (`/.well-known/kalamar-server.json`), the server card (`*.kalamar.json`) and the checks a
//! launcher makes before saving a server (`server-card.md`). Unknown members are ignored; `x-` members are the
//! operator's. Every string is untrusted text: it is shown as plain text, never interpreted.

use crate::proto::url as purl;
use crate::servers;
use serde::Serialize;
use serde_json::Value;

pub const MAX_DOC_BYTES: usize = 64 * 1024;
pub const MAX_NAME_CHARS: usize = 64;

/// A server's relay as stored: host (name or IPv4) and UDP port (`None` = default).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelayAddr {
    pub host: String,
    pub port: Option<u16>,
}

impl RelayAddr {
    pub fn text(&self) -> String {
        match self.port {
            Some(p) => format!("{}:{p}", self.host),
            None => self.host.clone(),
        }
    }
}

/// `host[:port]` of a relay (UDP, default port 47313), or `null`.
pub fn parse_relay(v: &Value) -> Result<Option<RelayAddr>, String> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => {
            let (h, p) = servers::parse_line(s).map_err(|e| format!("relay: {}", e.message()))?;
            Ok(Some(RelayAddr { host: h.text(), port: p }))
        }
        _ => Err("relay: a string host[:port] or null".into()),
    }
}

/// The checked discovery document. Localised texts are kept as given (objects of strings) for the interface.
#[derive(Debug, Clone, Serialize)]
pub struct Discovery {
    pub name: String,
    pub description: Value,
    pub api: String,
    pub relay: Option<RelayAddr>,
    pub versions: Vec<String>,
    pub features: Vec<String>,
    pub operator_name: String,
    pub operator_contact: String,
    pub rules_url: Option<String>,
    pub privacy_url: String,
    pub region: Option<String>,
    pub languages: Vec<String>,
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// A localised text: an object whose string entries are kept (anything else gives `{}`).
pub fn localised(v: Option<&Value>) -> Value {
    match v {
        Some(Value::Object(m)) => Value::Object(m.iter().filter(|(_, s)| s.is_string()).map(|(k, s)| (k.clone(), s.clone())).collect()),
        _ => Value::Object(Default::default()),
    }
}

fn https_link(v: &Value, key: &str, required: bool, problems: &mut Vec<String>) -> Option<String> {
    match v.get(key) {
        None | Some(Value::Null) if !required => None,
        Some(Value::String(s)) => match reqwest::Url::parse(s) {
            Ok(u) if purl::allowed_url(&u) && s.len() <= 512 => Some(s.clone()),
            _ => {
                problems.push(format!("{key}: an https URL"));
                None
            }
        },
        _ => {
            problems.push(format!("{key}: required (an https URL)"));
            None
        }
    }
}

/// Checks the discovery document. `Err` lists every problem (in English, for the log and the "details" line).
pub fn parse_discovery(bytes: &[u8]) -> Result<Discovery, Vec<String>> {
    if bytes.len() > MAX_DOC_BYTES {
        return Err(vec!["the document is larger than 64 KiB".into()]);
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|e| vec![format!("not JSON: {e}")])?;
    if !v.is_object() {
        return Err(vec!["not a JSON object".into()]);
    }
    let mut p = Vec::new();
    match v.get("kalamar_server").and_then(Value::as_i64) {
        Some(1) => {}
        Some(n) => p.push(format!("kalamar_server: format {n} is not supported (1)")),
        None => p.push("kalamar_server: missing (this is not a Kalamar server document)".into()),
    }
    let name = check_name(&v, &mut p);
    let api = match str_of(&v, "api") {
        Some(a) => purl::parse_api(a).map_err(|e| p.push(format!("api: {}", e.message()))).unwrap_or_default(),
        None => {
            p.push("api: required".into());
            String::new()
        }
    };
    let relay = match v.get("relay") {
        Some(r) => parse_relay(r).map_err(|e| p.push(e)).unwrap_or(None),
        None => {
            p.push("relay: required (a string or null)".into());
            None
        }
    };
    let versions = strings(v.get("protocol").and_then(|x| x.get("versions")));
    if versions.is_empty() {
        p.push("protocol.versions: required".into());
    }
    if !v.get("features").is_some_and(Value::is_array) {
        p.push("features: required (an array)".into());
    }
    let features = strings(v.get("features"));
    let op = v.get("operator").cloned().unwrap_or(Value::Null);
    let operator_name = str_of(&op, "name").map(str::trim).filter(|s| !s.is_empty() && s.chars().count() <= 64).map(str::to_string);
    let operator_contact = str_of(&op, "contact").filter(|c| c.len() <= 200 && (c.starts_with("mailto:") || c.starts_with("https://"))).map(str::to_string);
    if operator_name.is_none() {
        p.push("operator.name: required".into());
    }
    if operator_contact.is_none() {
        p.push("operator.contact: required (mailto: or https:)".into());
    }
    let rules_url = https_link(&v, "rules_url", false, &mut p);
    let privacy_url = https_link(&v, "privacy_url", true, &mut p).unwrap_or_default();
    if !p.is_empty() {
        return Err(p);
    }
    Ok(Discovery {
        name,
        description: localised(v.get("description")),
        api,
        relay,
        versions,
        features,
        operator_name: operator_name.unwrap_or_default(),
        operator_contact: operator_contact.unwrap_or_default(),
        rules_url,
        privacy_url,
        region: str_of(&v, "region").map(|s| s.chars().take(32).collect()),
        languages: strings(v.get("languages")).into_iter().take(16).collect(),
    })
}

fn check_name(v: &Value, p: &mut Vec<String>) -> String {
    match str_of(v, "name").map(str::trim) {
        Some(n) if !n.is_empty() && n.chars().count() <= MAX_NAME_CHARS && !n.chars().any(crate::util::unsafe_char) => n.to_string(),
        _ => {
            p.push("name: 1 to 64 characters".into());
            String::new()
        }
    }
}

/// A server card of this protocol (`server-card.md` 3).
#[derive(Debug, Clone, Serialize)]
pub struct Card {
    pub name: String,
    pub api: Option<String>,
    pub relay: Option<RelayAddr>,
    /// `host[:port]` of the discovery document, when the card says where to refresh it from.
    pub discovery: Option<String>,
}

/// `Ok(None)`: the file is not a card of this protocol (no `kalamar_server` member): an older card format.
pub fn parse_card(bytes: &[u8]) -> Result<Option<Card>, Vec<String>> {
    if bytes.len() > MAX_DOC_BYTES {
        return Err(vec!["the card is larger than 64 KiB".into()]);
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|e| vec![format!("not JSON: {e}")])?;
    if v.get("kalamar_server").is_none() {
        return Ok(None);
    }
    let mut p = Vec::new();
    if v.get("kalamar_server").and_then(Value::as_i64) != Some(1) {
        p.push("kalamar_server: only format 1 is supported".into());
    }
    let name = check_name(&v, &mut p);
    let api = match v.get("api") {
        Some(Value::Null) => None,
        Some(Value::String(a)) => purl::parse_api(a).map_err(|e| p.push(format!("api: {}", e.message()))).ok(),
        _ => {
            p.push("api: required (a URL or null)".into());
            None
        }
    };
    let relay = match v.get("relay") {
        Some(r) => parse_relay(r).map_err(|e| p.push(e)).unwrap_or(None),
        None => {
            p.push("relay: required (a string or null)".into());
            None
        }
    };
    if api.is_none() && relay.is_none() && p.is_empty() {
        p.push("a card needs an api, a relay, or both".into());
    }
    let discovery = match str_of(&v, "discovery") {
        Some(d) => match purl::parse_host_line(d) {
            Ok(l) => Some(l.text()),
            Err(e) => {
                p.push(format!("discovery: {}", e.message()));
                None
            }
        },
        None => None,
    };
    if !p.is_empty() {
        return Err(p);
    }
    Ok(Some(Card { name, api, relay, discovery }))
}

/// The highest `1.x` version a server lists (this launcher speaks 1.0; any 1.y server works with it).
pub fn common_version(versions: &[String]) -> Option<String> {
    versions
        .iter()
        .filter_map(|s| {
            let (maj, min) = s.split_once('.')?;
            (maj == "1").then(|| min.parse::<u32>().ok().map(|m| (m, s.clone()))).flatten()
        })
        .max_by_key(|(m, _)| *m)
        .map(|(_, s)| s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC_EXAMPLE: &str = r#"{
  "kalamar_server": 1,
  "name": "Thursday Night Squids",
  "description": {"en": "A small community for Thursday nights.", "fr": "Une petite communauté du jeudi soir."},
  "api": "https://play.example.org/api",
  "relay": "relay.example.org:47313",
  "protocol": {"versions": ["1.0"]},
  "features": ["rooms", "queues", "parties", "friends", "presence", "ratings", "leaderboards", "stats", "moderation"],
  "operator": {"name": "Camille", "contact": "mailto:admin@example.org"},
  "rules_url": "https://play.example.org/rules",
  "privacy_url": "https://play.example.org/privacy",
  "region": "eu-west",
  "languages": ["fr", "en"],
  "x-mine": {"anything": true},
  "future_member": 42
}"#;

    #[test]
    fn the_specification_example_is_accepted() {
        let d = parse_discovery(SPEC_EXAMPLE.as_bytes()).unwrap();
        assert_eq!(d.name, "Thursday Night Squids");
        assert_eq!(d.api, "https://play.example.org/api");
        assert_eq!(d.relay, Some(RelayAddr { host: "relay.example.org".into(), port: Some(47313) }));
        assert_eq!(d.features.len(), 9);
        assert_eq!(d.description["fr"], "Une petite communauté du jeudi soir.");
        assert_eq!((d.region.as_deref(), d.languages.len()), (Some("eu-west"), 2));
        assert_eq!(common_version(&d.versions).as_deref(), Some("1.0"));
    }

    #[test]
    /// A bidirectional override in a server's name would make another text appear.
    fn a_name_with_a_bidi_override_is_refused() {
        // The example's name is replaced: a replacement that finds nothing would leave the document valid.
        assert!(SPEC_EXAMPLE.contains("\"Thursday Night Squids\""));
        let doc = SPEC_EXAMPLE.replace("Thursday Night Squids", "Kalamar \u{202E}laiciffo");
        assert!(parse_discovery(doc.as_bytes()).unwrap_err().join("|").contains("name"));
        let doc = SPEC_EXAMPLE.replace("Thursday Night Squids", "Thursday \u{2066}Night\u{2069} Squids");
        assert!(parse_discovery(doc.as_bytes()).is_err());
        let doc = SPEC_EXAMPLE.replace("Thursday Night Squids", "Thursday Squids \u{1F991}\u{200D}");
        assert!(parse_discovery(doc.as_bytes()).is_ok(), "emoji joiners are fine");
    }

    #[test]
    fn the_reference_servers_document_is_accepted() {
        // As served by kalamar-service with deploy/kalamar.local.toml (no description, rules_url null, loopback http).
        let doc = r#"{"api":"http://127.0.0.1:8080","features":["rooms","queues"],"kalamar_server":1,"name":"Local test server","operator":{"contact":"mailto:nobody@example.invalid","name":"Local tester"},"privacy_url":"https://example.invalid/privacy","protocol":{"versions":["1.0"]},"relay":"127.0.0.1:47313","rules_url":null}"#;
        let d = parse_discovery(doc.as_bytes()).unwrap();
        assert_eq!((d.api.as_str(), d.rules_url.as_deref()), ("http://127.0.0.1:8080", None));
        assert_eq!(d.relay.unwrap().text(), "127.0.0.1:47313");
    }

    #[test]
    fn bad_documents_list_their_problems() {
        let e = parse_discovery(br#"{"kalamar_server": 2, "name": "", "api": "http://evil.example", "relay": "[::1]:1", "protocol": {}, "operator": {}, "privacy_url": "http://x.example"}"#).unwrap_err();
        let all = e.join("|");
        for k in ["kalamar_server", "name", "api", "relay", "protocol.versions", "features", "operator.name", "operator.contact", "privacy_url"] {
            assert!(all.contains(k), "{k}: {all}");
        }
        assert!(parse_discovery(b"[1]").is_err());
        assert!(parse_discovery(b"not json").is_err());
        assert!(parse_discovery(&vec![b' '; MAX_DOC_BYTES + 1]).is_err());
        // relay null is allowed
        let doc = SPEC_EXAMPLE.replace("\"relay.example.org:47313\"", "null");
        assert_eq!(parse_discovery(doc.as_bytes()).unwrap().relay, None);
    }

    #[test]
    fn cards() {
        let c = parse_card(br#"{"kalamar_server": 1, "name": "Thursday Night Squids", "api": "https://play.example.org/api", "relay": "relay.example.org:47313"}"#).unwrap().unwrap();
        assert_eq!((c.api.as_deref(), c.relay.as_ref().map(RelayAddr::text).as_deref()), (Some("https://play.example.org/api"), Some("relay.example.org:47313")));
        // relay-only card
        let c = parse_card(br#"{"kalamar_server": 1, "name": "R", "api": null, "relay": "203.0.113.7"}"#).unwrap().unwrap();
        assert!(c.api.is_none() && c.relay.is_some());
        // an older card (no kalamar_server): not this format
        assert!(parse_card(br#"{"name": "Friends server", "address": "203.0.113.7", "port": 47314}"#).unwrap().is_none());
        assert!(parse_card(br#"{"kalamar_server": 1, "name": "N", "api": null, "relay": null}"#).is_err());
        assert!(parse_card(br#"{"kalamar_server": 1, "name": "N", "api": "http://x.example", "relay": null}"#).is_err());
        let c = parse_card(br#"{"kalamar_server": 1, "name": "N", "api": null, "relay": "r.example", "discovery": "play.example.org"}"#).unwrap().unwrap();
        assert_eq!(c.discovery.as_deref(), Some("play.example.org"));
    }

    #[test]
    fn version_negotiation() {
        let v = |l: &[&str]| common_version(&l.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(v(&["1.0"]).as_deref(), Some("1.0"));
        assert_eq!(v(&["1.0", "1.3", "2.0"]).as_deref(), Some("1.3"));
        assert_eq!(v(&["2.0"]), None);
        assert_eq!(v(&[]), None);
        assert_eq!(v(&["1"]), None);
    }
}
