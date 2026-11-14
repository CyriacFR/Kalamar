//! What one presence says, as it goes into `SET_ACTIVITY`. The page builds the words (in the interface language, from
//! the player's chosen level: `src/discord/presence.ts`); this side keeps only the members Discord's activity has that
//! the launcher uses, checks each of them, and writes the JSON. Nothing else can reach the payload: no party secret, no
//! button, no URL, no image key outside the list of the launcher's own pictures, and a text that looks like an address,
//! a match code or an e-mail is dropped (a second guard behind the page's, which never puts them there).

use serde::{Deserialize, Serialize};

/// The pictures registered in the maintainers' Discord application (`docs/discord-presence.md`): the logo, and the
/// launcher's own small icons for what the player does. No picture of the game is ever named.
pub const LARGE_IMAGE: &str = "kalamar_logo";
pub const SMALL_IMAGES: &[&str] = &["act_solo", "act_online", "act_room", "act_search", "act_match", "act_watch", "act_done"];

/// Discord refuses texts shorter than 2 or longer than 128 characters; the launcher stays well inside.
const TEXT_MIN: usize = 2;
const TEXT_MAX: usize = 100;
/// The largest group size the launcher ever shows (rooms and parties are far smaller).
const PARTY_MAX: u32 = 64;
/// 2020-01-01: a start time before it is not a real one.
const EPOCH_FLOOR: u64 = 1_577_836_800;

/// What the page sends: words and numbers only (`discord_publish`).
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ActivityInput {
    pub details: Option<String>,
    pub state: Option<String>,
    /// Unix time in seconds that Discord counts the elapsed time from.
    pub start: Option<u64>,
    pub large_text: Option<String>,
    pub small_image: Option<String>,
    pub small_text: Option<String>,
    /// `[in the group, of at most]`.
    pub party: Option<[u32; 2]>,
}

/// A checked activity. Equal activities are not sent twice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Activity {
    pub details: Option<String>,
    pub state: Option<String>,
    pub start: Option<u64>,
    pub large_text: Option<String>,
    pub small_image: Option<&'static str>,
    pub small_text: Option<String>,
    pub party: Option<[u32; 2]>,
}

/// True for a text that must never be in a presence even if the page put it there: something with a scheme
/// (`https://`), an IPv4 address, a match code of the launcher (`KLM1-`, `KLM2-`) or an e-mail address. The page never
/// writes those (`presence.test.mjs`); this only catches a mistake.
pub fn looks_private(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    if lower.contains("://") || lower.contains("klm1-") || lower.contains("klm2-") || lower.contains('@') {
        return true;
    }
    // a dotted quad of numbers
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() && (i == 0 || !bytes[i - 1].is_ascii_digit() && bytes[i - 1] != b'.') {
            let mut j = i;
            let mut parts = 0;
            loop {
                let start = j;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                if j == start || j - start > 3 {
                    break;
                }
                parts += 1;
                if parts == 4 || j >= bytes.len() || bytes[j] != b'.' {
                    break;
                }
                j += 1;
            }
            if parts == 4 {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// One text member: trimmed, control characters out, at most `TEXT_MAX` characters (cut on a character, with an
/// ellipsis), dropped when shorter than Discord takes or when it looks private.
fn text(s: &Option<String>) -> Option<String> {
    let s = s.as_deref()?;
    let clean: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if looks_private(&clean) {
        return None;
    }
    let out = if clean.chars().count() > TEXT_MAX {
        let mut t: String = clean.chars().take(TEXT_MAX - 1).collect();
        t.push('…');
        t
    } else {
        clean
    };
    (out.chars().count() >= TEXT_MIN).then_some(out)
}

impl ActivityInput {
    /// The activity Discord will get, or `None` when nothing in it is usable (then the presence is cleared).
    pub fn check(&self, now: u64) -> Option<Activity> {
        let small_image = self.small_image.as_deref().and_then(|k| SMALL_IMAGES.iter().find(|x| **x == k).copied());
        let a = Activity {
            details: text(&self.details),
            state: text(&self.state),
            start: self.start.filter(|s| *s >= EPOCH_FLOOR && *s <= now + 300),
            large_text: text(&self.large_text),
            small_text: small_image.and_then(|_| text(&self.small_text)),
            small_image,
            party: self.party.filter(|[n, max]| *n >= 1 && n <= max && *max <= PARTY_MAX),
        };
        (a.details.is_some() || a.state.is_some()).then_some(a)
    }
}

/// The handshake: protocol version 1 and the application id.
pub fn handshake(client_id: &str) -> String {
    serde_json::json!({ "v": 1, "client_id": client_id }).to_string()
}

/// `SET_ACTIVITY` for this process; `None` clears the presence. `party_id` is a random word of this launcher's session
/// (Discord shows a group size only with an id); it says nothing about the room or the server.
pub fn set_activity(a: Option<&Activity>, pid: u32, nonce: u64, party_id: &str) -> String {
    use serde_json::{Map, Value, json};
    let activity = a.map(|a| {
        let mut o = Map::new();
        if let Some(d) = &a.details {
            o.insert("details".into(), json!(d));
        }
        if let Some(s) = &a.state {
            o.insert("state".into(), json!(s));
        }
        if let Some(t) = a.start {
            o.insert("timestamps".into(), json!({ "start": t }));
        }
        let mut assets = Map::new();
        assets.insert("large_image".into(), json!(LARGE_IMAGE));
        if let Some(t) = &a.large_text {
            assets.insert("large_text".into(), json!(t));
        }
        if let Some(k) = a.small_image {
            assets.insert("small_image".into(), json!(k));
            if let Some(t) = &a.small_text {
                assets.insert("small_text".into(), json!(t));
            }
        }
        o.insert("assets".into(), Value::Object(assets));
        if let Some(p) = a.party {
            o.insert("party".into(), json!({ "id": party_id, "size": p }));
        }
        o.insert("instance".into(), json!(false));
        Value::Object(o)
    });
    // A clear has no activity member at all (as `discord-rpc`'s ClearPresence writes it).
    let args = match activity {
        Some(a) => json!({ "pid": pid, "activity": a }),
        None => json!({ "pid": pid }),
    };
    json!({ "cmd": "SET_ACTIVITY", "nonce": nonce.to_string(), "args": args }).to_string()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const NOW: u64 = 1_794_346_807;

    fn input(details: &str, state: &str) -> ActivityInput {
        ActivityInput { details: Some(details.into()), state: Some(state.into()), start: Some(NOW - 60), ..Default::default() }
    }

    #[test]
    fn texts_are_trimmed_capped_and_kept_inside_what_discord_takes() {
        let a = ActivityInput { details: Some("  In a\tmatch \n".into()), state: Some("x".into()), ..Default::default() }.check(NOW).unwrap();
        assert_eq!(a.details.as_deref(), Some("In a match"));
        assert_eq!(a.state, None, "one character is refused by Discord: the member goes");
        let long = "é".repeat(300);
        let a = ActivityInput { details: Some(long), ..Default::default() }.check(NOW).unwrap();
        assert_eq!(a.details.as_ref().unwrap().chars().count(), TEXT_MAX);
        assert!(a.details.unwrap().ends_with('…'));
        // nothing left to say: no activity at all
        assert_eq!(ActivityInput { details: Some(" ".into()), ..Default::default() }.check(NOW), None);
    }

    #[test]
    fn only_the_launchers_own_pictures_and_sane_numbers_go_out() {
        let mut i = input("In a match", "Ranked · 4 v 4");
        i.small_image = Some("some_game_picture".into());
        i.small_text = Some("In a match".into());
        i.party = Some([5, 4]);
        i.start = Some(NOW + 3600);
        let a = i.check(NOW).unwrap();
        assert_eq!((a.small_image, a.small_text.as_deref(), a.party, a.start), (None, None, None, None));
        i.small_image = Some("act_match".into());
        i.party = Some([2, 4]);
        i.start = Some(NOW - 10);
        let a = i.check(NOW).unwrap();
        assert_eq!((a.small_image, a.small_text.as_deref(), a.party, a.start), (Some("act_match"), Some("In a match"), Some([2, 4]), Some(NOW - 10)));
        assert_eq!(input("a", "b").check(NOW), None);
        assert!(i.clone().check(NOW).is_some());
        i.party = Some([0, 4]);
        assert_eq!(i.check(NOW).unwrap().party, None);
    }

    #[test]
    fn the_page_cannot_add_members_of_its_own() {
        let bad = r#"{"details":"In a match","secrets":{"join":"7KQ2MX"}}"#;
        assert!(serde_json::from_str::<ActivityInput>(bad).is_err());
        let bad = r#"{"details":"In a match","buttons":[{"label":"Join","url":"https://x"}]}"#;
        assert!(serde_json::from_str::<ActivityInput>(bad).is_err());
    }

    #[test]
    fn addresses_codes_and_mails_never_reach_the_payload() {
        for s in ["relay.example.org:47313 via https://play.example.org", "203.0.113.7", "at 10.0.0.12 tonight", "KLM2-ABCDE-FGHJK", "klm1-xyz", "camille@example.org"] {
            assert!(looks_private(s), "{s}");
            let a = input(s, "Private room").check(NOW).unwrap();
            assert_eq!(a.details, None, "{s}");
        }
        for s in ["Private room · 2 v 2", "Server 1.2", "In a match · Area Rush", "4 v 4", "v1.2.3", "The 3 musketeers", "1.2.3.4.5.6"] {
            let expect = s == "1.2.3.4.5.6";
            assert_eq!(looks_private(s), expect, "{s}");
        }
    }

    /// The pictures named here are the ones drawn by the page (`src/discord/art.ts`) and shipped for the maintainers to upload
    /// (`docs/discord-assets/<key>.png`, 1024x1024).
    #[test]
    fn every_picture_key_has_its_drawing_and_its_file() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let art = std::fs::read_to_string(root.join("src/discord/art.ts")).unwrap();
        let mut drawn: Vec<&str> = art.lines().filter_map(|l| l.trim().split_once(": {").map(|(k, _)| k)).filter(|k| k.starts_with("act_")).collect();
        drawn.sort_unstable();
        let mut listed = SMALL_IMAGES.to_vec();
        listed.sort_unstable();
        assert_eq!(drawn, listed);
        for key in SMALL_IMAGES.iter().chain([LARGE_IMAGE].iter()) {
            let png = std::fs::read(root.join(format!("docs/discord-assets/{key}.png"))).unwrap_or_else(|_| panic!("{key}.png"));
            assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "{key}");
            let (w, h) = (u32::from_be_bytes(png[16..20].try_into().unwrap()), u32::from_be_bytes(png[20..24].try_into().unwrap()));
            assert_eq!((w, h), (1024, 1024), "{key}");
        }
    }

    #[test]
    fn the_payload_has_exactly_the_members_the_launcher_uses() {
        let mut i = input("Private room", "4 v 4");
        i.large_text = Some("Kalamar".into());
        i.small_image = Some("act_room".into());
        i.small_text = Some("Private room".into());
        i.party = Some([3, 8]);
        let a = i.check(NOW).unwrap();
        let v: serde_json::Value = serde_json::from_str(&set_activity(Some(&a), 4242, 7, "kalamar-0011223344556677")).unwrap();
        assert_eq!(v["cmd"], "SET_ACTIVITY");
        assert_eq!(v["nonce"], "7");
        assert_eq!(v["args"]["pid"], 4242);
        let act = v["args"]["activity"].as_object().unwrap();
        let mut keys: Vec<&str> = act.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["assets", "details", "instance", "party", "state", "timestamps"]);
        assert_eq!(act["assets"]["large_image"], LARGE_IMAGE);
        assert_eq!(act["assets"]["small_image"], "act_room");
        assert_eq!(act["party"]["size"], serde_json::json!([3, 8]));
        assert_eq!(act["timestamps"]["start"], NOW - 60);
        // cleared: no activity member
        let v: serde_json::Value = serde_json::from_str(&set_activity(None, 4242, 8, "p")).unwrap();
        assert_eq!(v["args"], serde_json::json!({ "pid": 4242 }));
        let h: serde_json::Value = serde_json::from_str(&handshake("100000000000000001")).unwrap();
        assert_eq!(h, serde_json::json!({ "v": 1, "client_id": "100000000000000001" }));
    }
}
