//! Where the game stands in its flow, for the "Your match" block: the engine's status file (`auto-match.md`, "Status
//! file") reduced to what the page shows, and the time limit of the step the game is in.
//!
//! **Limits.** The engine stops itself when a step takes too long (`SPLAT3_MATCH_TIMEOUTS`). The launcher knows them
//! exactly when it can tell which step a state is in: the defaults of the engine's documentation, replaced by the
//! match's `game.timeouts` (which the launcher passes as that variable). The states `booting` and `lobby` cover the
//! boundary between two steps (`boot` ends when the local lobby's network is up, `list` starts after it), so no limit
//! is claimed for them: the page shows the elapsed time only.

use serde::Serialize;
use serde_json::Value;

/// `(step, default seconds)` as in the engine's documentation of `SPLAT3_MATCH_TIMEOUTS`.
pub const DEFAULT_LIMITS: &[(&str, u64)] = &[
    ("boot", 300),
    ("list", 90),
    ("create", 60),
    ("players", 300),
    ("join", 300),
    ("start", 360),
    ("stage", 90),
    ("match", 900),
    ("results", 120),
];

/// The step whose limit applies to a status state, when the state is exactly one step.
pub fn step_of(state: &str) -> Option<&'static str> {
    Some(match state {
        "creating-room" => "create",
        "waiting-for-players" => "players",
        "joining" => "join",
        "in-room" => "start",
        "loading-stage" => "stage",
        "in-match" => "match",
        "results" => "results",
        _ => return None,
    })
}

/// The limit in seconds of the step a state is in: the match's `timeouts` string (`boot=300,list=90`) over the defaults.
pub fn limit_of(state: &str, timeouts: Option<&str>) -> Option<u64> {
    let step = step_of(state)?;
    let given = timeouts
        .into_iter()
        .flat_map(|t| t.split(','))
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| k.trim() == step)
        .and_then(|(_, v)| v.trim().parse::<u64>().ok())
        .filter(|s| *s > 0);
    given.or_else(|| DEFAULT_LIMITS.iter().find(|(k, _)| *k == step).map(|(_, s)| *s))
}

/// The game's last status as the page shows it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GameProgress {
    pub state: String,
    pub detail: Option<String>,
    /// When the state began (UTC milliseconds, from the engine's `since_unix`).
    pub since_ms: Option<u64>,
    /// `waiting-for-players`: the players in the room (the host included) and the number wanted.
    pub present: Option<u32>,
    pub expected: Option<u32>,
    /// The game window is shown.
    pub visible: Option<bool>,
    /// The step's time limit in seconds; `None` when the launcher cannot say.
    pub limit_s: Option<u64>,
}

/// From the status file's text. `None` when it is not a JSON object with a state.
pub fn from_status(text: &str, timeouts: Option<&str>) -> Option<GameProgress> {
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let state = v.get("state")?.as_str()?.to_string();
    let num = |k: &str| v.get(k).and_then(Value::as_u64).filter(|n| *n <= 64).map(|n| n as u32);
    let since = v.get("since_unix").and_then(Value::as_f64).filter(|f| f.is_finite() && *f > 0.0 && *f < 1e11);
    Some(GameProgress {
        detail: v.get("detail").and_then(Value::as_str).filter(|d| !d.is_empty()).map(|d| d.chars().take(200).collect()),
        since_ms: since.map(|f| (f * 1000.0).round() as u64),
        present: num("present"),
        expected: num("expected"),
        visible: v.get("visible").and_then(Value::as_bool),
        limit_s: limit_of(&state, timeouts),
        state,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_follow_the_documented_defaults_and_the_matchs_overrides() {
        assert_eq!(limit_of("waiting-for-players", None), Some(300));
        assert_eq!(limit_of("creating-room", None), Some(60));
        assert_eq!(limit_of("in-room", None), Some(360));
        assert_eq!(limit_of("loading-stage", None), Some(90));
        assert_eq!(limit_of("in-match", None), Some(900));
        assert_eq!(limit_of("results", None), Some(120));
        assert_eq!(limit_of("joining", None), Some(300));
        assert_eq!(limit_of("waiting-for-players", Some("boot=100, players=40,list=9")), Some(40));
        assert_eq!(limit_of("waiting-for-players", Some("players=abc")), Some(300));
        assert_eq!(limit_of("waiting-for-players", Some("players=0")), Some(300));
        // The two states that straddle a step boundary, and states with no step: no limit is claimed.
        for s in ["booting", "lobby", "finished", "error", "waiting-command", "something-new"] {
            assert_eq!(limit_of(s, Some("boot=100")), None, "{s}");
        }
    }

    #[test]
    fn reads_the_status_file() {
        let t = r#"{"state":"waiting-for-players","detail":"waiting for the players","present":1,"expected":2,"visible":false,"since_unix":1794346577.12,"pid":4}"#;
        let p = from_status(t, None).unwrap();
        assert_eq!((p.state.as_str(), p.present, p.expected, p.visible, p.limit_s), ("waiting-for-players", Some(1), Some(2), Some(false), Some(300)));
        assert_eq!(p.since_ms, Some(1_794_346_577_120));
        assert_eq!(from_status("\u{feff}{\"state\":\"lobby\"}", None).unwrap().since_ms, None);
        assert!(from_status("{\"nostate\":1}", None).is_none());
        assert!(from_status("{\"state\":\"x\"", None).is_none());
        // Nonsense numbers are dropped.
        let q = from_status(r#"{"state":"waiting-for-players","present":-1,"expected":99999,"since_unix":-5}"#, None).unwrap();
        assert_eq!((q.present, q.expected, q.since_ms), (None, None, None));
    }
}
