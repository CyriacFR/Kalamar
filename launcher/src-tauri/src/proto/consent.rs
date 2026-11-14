//! Which matches this launcher may start without asking its player (`docs/security-model.md`, "A server cannot start
//! your game on its own").
//!
//! A match object only says what the server claims. What this launcher did itself during this run is kept here, per
//! server, in memory only (a launcher that starts again remembers nothing): the requests it sent and that the server
//! answered with success. A match is **engaged** when one of them covers it:
//!
//! | the player, from this launcher | covers |
//! |---|---|
//! | opened a queue ticket (`POST /v1/queues/{id}/tickets`) | the next match of that queue (the ticket is used up by its launch, dropped by `DELETE /v1/tickets/current`) |
//! | accepted a found match, acted in its draft, took a place in its stands | that match |
//! | created, joined, readied in a room, changed its team, started it | the room's matches until the player leaves it |
//! | acted in an activity (`POST /v1/activities/{id}/actions/...`) | the activity's matches |
//! | took a place in a series' stands, or played one of its games | the series' next games |
//! | confirmed the launch of a match (the launcher's own question) | that match |
//!
//! A match the player declined (or whose stands it left) is never covered. The replacement of a launched match that was
//! voided (`outcome.rematch_id`) is covered like the launched one. Everything else is started only after the player
//! says yes ([`crate::proto::runner`]), and so is any launch beyond [`SHORT_CAP`] in [`SHORT_WINDOW`] or [`LONG_CAP`]
//! in [`LONG_WINDOW`] from one server.

use serde_json::Value;
use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Launches one server may cause without a question in [`SHORT_WINDOW`].
pub const SHORT_CAP: usize = 6;
pub const SHORT_WINDOW: Duration = Duration::from_secs(10 * 60);
/// Launches one server may cause without a question in [`LONG_WINDOW`].
pub const LONG_CAP: usize = 20;
pub const LONG_WINDOW: Duration = Duration::from_secs(3600);
/// Ids kept per kind (a long session cannot grow them without bound; the oldest go first).
const MAX_IDS: usize = 256;

/// Why a launch needs the player's yes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// Nothing this launcher did in this run covers the match (it was never joined here, or the launcher started
    /// again since).
    NotJoined,
    /// The server has started the game too often lately.
    TooMany,
}

impl Ask {
    pub fn code(self) -> &'static str {
        match self {
            Ask::NotJoined => "not-joined",
            Ask::TooMany => "too-many",
        }
    }
}

/// A bounded set that forgets its oldest ids.
#[derive(Debug, Default)]
struct Ids {
    order: VecDeque<String>,
    set: HashSet<String>,
}

impl Ids {
    fn insert(&mut self, id: &str) {
        if id.is_empty() || id.len() > 128 || self.set.contains(id) {
            return;
        }
        if self.order.len() >= MAX_IDS
            && let Some(old) = self.order.pop_front()
        {
            self.set.remove(&old);
        }
        self.order.push_back(id.to_string());
        self.set.insert(id.to_string());
    }
    fn remove(&mut self, id: &str) {
        if self.set.remove(id) {
            self.order.retain(|x| x != id);
        }
    }
    fn contains(&self, id: &str) -> bool {
        self.set.contains(id)
    }
    fn clear(&mut self) {
        self.order.clear();
        self.set.clear();
    }
}

#[derive(Debug, Default)]
struct Inner {
    matches: Ids,
    refused: Ids,
    launched: Ids,
    rooms: Ids,
    queues: Ids,
    activities: Ids,
    series: Ids,
    launches: VecDeque<Instant>,
}

/// One server's engagements during this run of the launcher.
#[derive(Debug, Default)]
pub struct Consent {
    inner: Mutex<Inner>,
}

fn segments(path: &str) -> Vec<&str> {
    path.split('?').next().unwrap_or("").trim_matches('/').split('/').collect()
}

fn text<'a>(v: &'a Value, pointer: &str) -> Option<&'a str> {
    v.pointer(pointer).and_then(Value::as_str).filter(|s| !s.is_empty())
}

impl Consent {
    pub fn new() -> Consent {
        Consent::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A request this launcher sent and the server answered with success (`answer`: the answer's body). Only the
    /// player's own actions count; the runner's reports (`launch`, `status`, `result`) are not engagements.
    pub fn note_request(&self, method: &str, path: &str, answer: &Value) {
        let s = segments(path);
        let mut g = self.lock();
        match (method, s.as_slice()) {
            ("POST", ["v1", "queues", q, "tickets"]) => g.queues.insert(q),
            ("DELETE", ["v1", "tickets", "current"]) => g.queues.clear(),
            ("POST", ["v1", "matches", id, "accept" | "draft"]) => {
                if !g.refused.contains(id) {
                    g.matches.insert(id);
                }
            }
            ("POST", ["v1", "matches", id, "decline"]) => {
                g.matches.remove(id);
                g.refused.insert(id);
            }
            ("POST", ["v1", "matches", id, "spectate"]) => {
                g.refused.remove(id);
                g.matches.insert(id);
                if let Some(sid) = text(answer, "/series/id") {
                    g.series.insert(sid);
                }
            }
            ("DELETE", ["v1", "matches", id, "spectate"]) => {
                g.matches.remove(id);
                g.refused.insert(id);
            }
            ("POST", ["v1", "series", id, "spectate"]) => g.series.insert(id),
            ("DELETE", ["v1", "series", id, "spectate"]) => g.series.remove(id),
            ("POST", ["v1", "rooms"] | ["v1", "rooms", "join"]) => {
                if let Some(id) = text(answer, "/id") {
                    g.rooms.insert(id);
                }
            }
            ("POST", ["v1", "rooms", id, "ready" | "start"]) | ("PUT", ["v1", "rooms", id, "members", _]) => g.rooms.insert(id),
            ("POST", ["v1", "rooms", id, "leave"]) => g.rooms.remove(id),
            ("POST", ["v1", "activities", id, "actions", _]) => g.activities.insert(id),
            _ => {}
        }
    }

    /// The player said yes to this match (the launcher's question).
    pub fn confirm(&self, match_id: &str) {
        let mut g = self.lock();
        g.refused.remove(match_id);
        g.matches.insert(match_id);
    }

    /// Something this launcher did in this run covers the match `m` (see the module's table).
    pub fn covers(&self, m: &Value) -> bool {
        let g = self.lock();
        let Some(id) = text(m, "/id") else { return false };
        if g.refused.contains(id) {
            return false;
        }
        if g.matches.contains(id) {
            return true;
        }
        if text(m, "/series/id").is_some_and(|s| g.series.contains(s)) {
            return true;
        }
        match text(m, "/origin/kind") {
            Some("room") => text(m, "/origin/room_id").is_some_and(|r| g.rooms.contains(r)),
            Some("queue") => text(m, "/origin/queue_id").is_some_and(|q| g.queues.contains(q)),
            Some("activity") => text(m, "/origin/activity_id").is_some_and(|a| g.activities.contains(a)),
            _ => false,
        }
    }

    /// Whether one more launch now stays within the caps.
    pub fn within_caps(&self, now: Instant) -> bool {
        let mut g = self.lock();
        while g.launches.front().is_some_and(|t| now.duration_since(*t) > LONG_WINDOW) {
            g.launches.pop_front();
        }
        let short = g.launches.iter().filter(|t| now.duration_since(**t) <= SHORT_WINDOW).count();
        short < SHORT_CAP && g.launches.len() < LONG_CAP
    }

    /// Why the launch of `m` needs the player's yes, if it does (`continuing`: a round sent to the open game, no new
    /// process: it does not count against the caps).
    pub fn ask(&self, m: &Value, continuing: bool, now: Instant) -> Option<Ask> {
        if !self.covers(m) {
            return Some(Ask::NotJoined);
        }
        if !continuing && !self.within_caps(now) {
            return Some(Ask::TooMany);
        }
        None
    }

    /// The game was started (or a round sent to it) for `m`: the match and its series stay covered, the queue ticket
    /// is used up, and a fresh launch counts against the caps.
    pub fn launched(&self, m: &Value, fresh: bool, now: Instant) {
        let mut g = self.lock();
        if let Some(id) = text(m, "/id") {
            g.matches.insert(id);
            g.launched.insert(id);
        }
        if let Some(s) = text(m, "/series/id") {
            g.series.insert(s);
        }
        if text(m, "/origin/kind") == Some("queue")
            && let Some(q) = text(m, "/origin/queue_id")
        {
            g.queues.remove(q);
        }
        if fresh {
            g.launches.push_back(now);
            while g.launches.len() > LONG_CAP + SHORT_CAP {
                g.launches.pop_front();
            }
        }
    }

    /// A match ended void and names its replacement: covered when the voided one was launched here.
    pub fn rematch(&self, voided: &str, replacement: &str) {
        let mut g = self.lock();
        if g.launched.contains(voided) && !g.refused.contains(replacement) {
            g.matches.insert(replacement);
        }
    }

    /// The player refused this match (the launcher's question): it is never started.
    pub fn refuse(&self, match_id: &str) {
        let mut g = self.lock();
        g.matches.remove(match_id);
        g.refused.insert(match_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn m(id: &str, origin: Value) -> Value {
        json!({"id": id, "state": "ready", "origin": origin})
    }

    #[test]
    fn a_match_pushed_by_the_server_alone_is_not_covered() {
        let c = Consent::new();
        assert!(!c.covers(&m("m_1", json!({"kind": "queue", "queue_id": "duel"}))));
        assert!(!c.covers(&m("m_1", json!({"kind": "room", "room_id": "r_1"}))));
        assert!(!c.covers(&json!({"id": "m_1", "series": {"id": "s_1"}})));
        assert_eq!(c.ask(&m("m_1", Value::Null), false, Instant::now()), Some(Ask::NotJoined));
        // The runner's own reports engage nothing.
        for p in ["/v1/matches/m_1/launch", "/v1/matches/m_1/status", "/v1/matches/m_1/result"] {
            c.note_request("POST", p, &json!({}));
        }
        c.note_request("GET", "/v1/matches/current", &json!({"match": {"id": "m_1"}}));
        c.note_request("POST", "/v1/rooms/join", &json!({}));
        assert!(!c.covers(&m("m_1", json!({"kind": "room", "room_id": "r_1"}))));
    }

    #[test]
    fn a_ticket_covers_the_next_match_of_its_queue_once() {
        let c = Consent::new();
        c.note_request("POST", "/v1/queues/duel/tickets", &json!({"id": "t_1"}));
        let a = m("m_1", json!({"kind": "queue", "queue_id": "duel"}));
        assert!(c.covers(&a));
        assert!(!c.covers(&m("m_1", json!({"kind": "queue", "queue_id": "other"}))));
        c.launched(&a, true, Instant::now());
        assert!(c.covers(&a), "the launched match stays covered (a resync)");
        assert!(!c.covers(&m("m_2", json!({"kind": "queue", "queue_id": "duel"}))), "the ticket was used up");
        c.note_request("POST", "/v1/queues/duel/tickets", &json!({}));
        c.note_request("DELETE", "/v1/tickets/current", &Value::Null);
        assert!(!c.covers(&m("m_3", json!({"kind": "queue", "queue_id": "duel"}))), "a cancelled ticket covers nothing");
    }

    #[test]
    fn accepting_covers_that_match_and_declining_refuses_it() {
        let c = Consent::new();
        c.note_request("POST", "/v1/matches/m_1/accept", &json!({}));
        assert!(c.covers(&m("m_1", Value::Null)));
        assert!(!c.covers(&m("m_2", Value::Null)));
        c.note_request("POST", "/v1/rooms", &json!({"id": "r_1"}));
        c.note_request("POST", "/v1/matches/m_3/decline", &json!({}));
        assert!(!c.covers(&m("m_3", json!({"kind": "room", "room_id": "r_1"}))), "declined wins over the room");
        c.refuse("m_1");
        assert!(!c.covers(&m("m_1", Value::Null)));
    }

    #[test]
    fn a_room_covers_its_matches_until_the_player_leaves() {
        let c = Consent::new();
        c.note_request("POST", "/v1/rooms/join", &json!({"id": "r_9", "code": "ABC"}));
        assert!(c.covers(&m("m_1", json!({"kind": "room", "room_id": "r_9"}))));
        assert!(c.covers(&m("m_2", json!({"kind": "room", "room_id": "r_9"}))));
        assert!(!c.covers(&m("m_1", json!({"kind": "room", "room_id": "r_8"}))));
        c.note_request("POST", "/v1/rooms/r_9/leave", &Value::Null);
        assert!(!c.covers(&m("m_3", json!({"kind": "room", "room_id": "r_9"}))));
        c.note_request("POST", "/v1/rooms/r_7/ready", &json!({}));
        assert!(c.covers(&m("m_4", json!({"kind": "room", "room_id": "r_7"}))));
    }

    #[test]
    fn activities_series_spectators_and_rematches() {
        let c = Consent::new();
        c.note_request("POST", "/v1/activities/cup/actions/register", &json!({}));
        let first = json!({"id": "m_1", "origin": {"kind": "activity", "activity_id": "cup"}, "series": {"id": "s_1"}});
        assert!(c.covers(&first));
        c.launched(&first, true, Instant::now());
        // The series' next game, whatever its origin says.
        assert!(c.covers(&json!({"id": "m_2", "series": {"id": "s_1"}})));
        // A place in the stands of a series' game seats for the series.
        c.note_request("POST", "/v1/matches/m_5/spectate", &json!({"id": "m_5", "series": {"id": "s_5"}}));
        assert!(c.covers(&m("m_5", Value::Null)) && c.covers(&json!({"id": "m_6", "series": {"id": "s_5"}})));
        c.note_request("DELETE", "/v1/matches/m_5/spectate", &Value::Null);
        assert!(!c.covers(&m("m_5", Value::Null)));
        c.note_request("POST", "/v1/series/s_9/spectate", &json!({}));
        assert!(c.covers(&json!({"id": "m_9", "series": {"id": "s_9"}})));
        c.note_request("DELETE", "/v1/series/s_9/spectate", &json!({}));
        assert!(!c.covers(&json!({"id": "m_10", "series": {"id": "s_9"}})));
        // A replacement is covered only for a match launched here.
        c.rematch("m_1", "m_r1");
        c.rematch("m_unknown", "m_r2");
        assert!(c.covers(&m("m_r1", Value::Null)) && !c.covers(&m("m_r2", Value::Null)));
    }

    #[test]
    fn launches_beyond_the_caps_need_a_question() {
        let c = Consent::new();
        let t0 = Instant::now();
        c.confirm("m_x");
        for i in 0..SHORT_CAP {
            assert_eq!(c.ask(&m("m_x", Value::Null), false, t0), None, "launch {i}");
            c.launched(&m("m_x", Value::Null), true, t0);
        }
        assert_eq!(c.ask(&m("m_x", Value::Null), false, t0), Some(Ask::TooMany));
        assert_eq!(c.ask(&m("m_x", Value::Null), true, t0), None, "a round in the open game is no launch");
        // The short window passes; the hour still counts.
        let later = t0 + SHORT_WINDOW + Duration::from_secs(1);
        assert_eq!(c.ask(&m("m_x", Value::Null), false, later), None);
        let mut t = later;
        for _ in SHORT_CAP..LONG_CAP {
            t += Duration::from_secs(150);
            assert_eq!(c.ask(&m("m_x", Value::Null), false, t), None);
            c.launched(&m("m_x", Value::Null), true, t);
        }
        assert_eq!(c.ask(&m("m_x", Value::Null), false, t + Duration::from_secs(150)), Some(Ask::TooMany));
    }

    #[test]
    fn ids_are_bounded() {
        let c = Consent::new();
        for i in 0..(MAX_IDS + 10) {
            c.note_request("POST", &format!("/v1/matches/m_{i}/accept"), &json!({}));
        }
        assert!(!c.covers(&m("m_0", Value::Null)), "the oldest are forgotten");
        assert!(c.covers(&m(&format!("m_{}", MAX_IDS + 9), Value::Null)));
    }
}
