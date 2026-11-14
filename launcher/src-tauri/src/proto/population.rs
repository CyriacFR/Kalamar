//! How many players a server has right now (`GET /v1/population`, public, optional area `population`): shown on the
//! Play page and at the top of the Community page while one of them is on screen. The page asks every 30 s at most,
//! never while the window is hidden or the game runs; any failure is silent (nothing is shown).

use crate::proto::{http, url as purl};
use serde::Serialize;
use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;

/// The declared area that offers it.
pub const AREA: &str = "population";
/// A population document is a few numbers: anything longer is refused.
const MAX_ANSWER: usize = 4 * 1024;
/// Short: the number is a decoration, a slow server must not hold a request open for long.
const TIMEOUT: Duration = Duration::from_secs(6);
/// More players than any server has: a larger number is a wrong document.
const MAX_COUNT: u64 = 10_000_000;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Population {
    pub online: u32,
    pub in_match: Option<u32>,
    pub searching: Option<u32>,
    pub updated_at: Option<String>,
}

/// `{"online": 12, "in_match": 4, "searching": 2, "updated_at": "..."}`. `online` is required; `in_match` and
/// `searching` may be absent or null. A count that is negative, not a whole number or absurdly large refuses the whole
/// document (a server that gets one number wrong may get the others wrong too).
pub fn parse(v: &Value) -> Option<Population> {
    let o = v.as_object()?;
    let count = |x: &Value| x.as_u64().filter(|n| *n <= MAX_COUNT).map(|n| n as u32);
    let optional = |k: &str| -> Result<Option<u32>, ()> {
        match o.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(x) => count(x).map(Some).ok_or(()),
        }
    };
    let online = count(o.get("online")?)?;
    let in_match = optional("in_match").ok()?;
    let searching = optional("searching").ok()?;
    let updated_at = o
        .get("updated_at")
        .and_then(Value::as_str)
        .filter(|s| s.len() <= 64 && crate::proto::tokens::parse_time(s).is_some())
        .map(str::to_string);
    Some(Population { online, in_match, searching, updated_at })
}

fn client() -> Option<&'static reqwest::Client> {
    static C: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    C.get_or_init(|| http::build_client(TIMEOUT).ok()).as_ref()
}

/// One request to the service at `api` (already checked when the server was saved: https, or http to this computer),
/// without the player's login. `None` on any failure.
pub async fn fetch(api: &str, lang: &str) -> Option<Population> {
    let r = http::send(client()?, "GET", &purl::join(api, "/v1/population"), None, &http::Body::None, lang, MAX_ANSWER).await.ok()?;
    parse(&r.body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_full_document() {
        let p = parse(&json!({"online": 12, "in_match": 4, "searching": 2, "updated_at": "2026-11-11T18:00:00Z"})).unwrap();
        assert_eq!(p, Population { online: 12, in_match: Some(4), searching: Some(2), updated_at: Some("2026-11-11T18:00:00Z".into()) });
    }

    #[test]
    fn optional_counts_may_be_absent_or_null() {
        let p = parse(&json!({"online": 3})).unwrap();
        assert_eq!((p.online, p.in_match, p.searching, p.updated_at), (3, None, None, None));
        let p = parse(&json!({"online": 0, "in_match": null, "updated_at": "yesterday"})).unwrap();
        assert_eq!((p.online, p.in_match, p.updated_at), (0, None, None), "an unreadable date is left out, not the counts");
    }

    #[test]
    fn wrong_counts_refuse_the_document() {
        for bad in [
            json!({}),
            json!({"online": null}),
            json!({"online": -1}),
            json!({"online": 2.5}),
            json!({"online": "12"}),
            json!({"online": 12, "in_match": -4}),
            json!({"online": 12, "searching": 0.5}),
            json!({"online": 12, "in_match": "4"}),
            json!({"online": 20_000_000}),
            json!([12]),
        ] {
            assert_eq!(parse(&bad), None, "{bad}");
        }
    }
}
