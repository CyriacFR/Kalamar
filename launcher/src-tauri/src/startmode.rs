//! The mode cards of the Play screen and the environment each one sets (`engine/docs/save-profile.md`, "Start
//! scene"). The runtime picks its first scene from `SPLAT3_START` (which wins over the `start_scene` setting):
//!
//! | card | `SPLAT3_START` | result |
//! |---|---|---|
//! | `hub` | `plaza` | the hub in free play, playable about 30 s after launch; title screen and news skipped |
//! | `range` | `range` | the shooting range, playable |
//! | `lan` | `lobby-lan` | the LAN lobby; needs `SPLAT3_LAN` (the LAN environment of `lan.rs`), else the game returns to the hub |
//! | `original` | `title` | the untouched boot (title screen, news, then the plaza); `title` also overrides a `start_scene=` of `totk.ini` |
//!
//! `story` is deliberately absent: the runtime hangs on its transition.
//!
//! A `start-modes.json` in the launcher data directory overrides the table without a rebuild (same shape as the
//! default; entries with the same `id` replace the defaults, unknown ids are ignored: a card needs UI):
//!
//! ```json
//! { "modes": [ { "id": "range", "var": "SPLAT3_START", "value": "range", "enabled": true } ] }
//! ```
//!
//! The provisional `skip_title` entry of earlier versions is gone: there is no separate variable, `plaza` skips the
//! title screen and the news by itself. A `skip_title` key left in an old file is ignored.

use crate::util;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const ENV_START: &str = "SPLAT3_START";

/// Ids of the cards, in display order.
#[cfg(test)]
pub const MODE_IDS: [&str; 4] = ["hub", "range", "lan", "original"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModeEntry {
    pub id: String,
    pub var: String,
    pub value: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartTable {
    pub modes: Vec<ModeEntry>,
}

fn entry(id: &str, value: &str, enabled: bool) -> ModeEntry {
    ModeEntry { id: id.into(), var: ENV_START.into(), value: value.into(), enabled }
}

impl Default for StartTable {
    fn default() -> Self {
        StartTable {
            modes: vec![
                entry("hub", "plaza", true),
                entry("range", "range", true),
                entry("lan", "lobby-lan", true),
                entry("original", "title", true),
            ],
        }
    }
}

fn valid_var(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && n.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') && !n.starts_with(|c: char| c.is_ascii_digit())
}

fn valid_value(v: &str) -> bool {
    !v.is_empty() && v.len() <= 128 && !v.chars().any(|c| c.is_control())
}

impl StartTable {
    /// Defaults overlaid with `start-modes.json` (when present and sane).
    pub fn load(data_dir: &Path) -> StartTable {
        let mut t = StartTable::default();
        let Some(over) = util::read_text_limited(&data_dir.join("start-modes.json"), 64 * 1024)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        else {
            return t;
        };
        if let Some(list) = over.get("modes").and_then(|m| serde_json::from_value::<Vec<ModeEntry>>(m.clone()).ok()) {
            for m in list.into_iter().filter(|m| valid_var(&m.var) && valid_value(&m.value)) {
                if let Some(x) = t.modes.iter_mut().find(|x| x.id == m.id) {
                    *x = m;
                }
            }
        }
        t
    }

    pub fn get(&self, id: &str) -> Option<&ModeEntry> {
        self.modes.iter().find(|m| m.id == id)
    }

    /// The variable and value of an enabled card; `None` for an unknown or disabled id.
    pub fn env(&self, id: &str) -> Option<(String, String)> {
        self.get(id).filter(|m| m.enabled).map(|m| (m.var.clone(), m.value.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_final_names_with_every_card_on() {
        let t = StartTable::default();
        assert_eq!(t.modes.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), MODE_IDS);
        assert_eq!(t.env("hub"), Some(("SPLAT3_START".into(), "plaza".into())));
        assert_eq!(t.env("range"), Some(("SPLAT3_START".into(), "range".into())));
        assert_eq!(t.env("lan"), Some(("SPLAT3_START".into(), "lobby-lan".into())));
        assert_eq!(t.env("original"), Some(("SPLAT3_START".into(), "title".into())));
        assert_eq!(t.env("story"), None); // never offered: it hangs on the transition
        assert!(t.get("skip_title").is_none());
    }

    #[test]
    fn a_json_file_overrides_known_cards_and_ignores_the_rest() {
        let d = std::env::temp_dir().join(format!("s3l-start-{}", util::random_hex(4)));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("start-modes.json"),
            r#"{"modes":[{"id":"range","var":"S3_SCENE","value":"shooting-range","enabled":true},{"id":"hub","var":"lower case","value":"plaza","enabled":true},
                         {"id":"new","var":"S3_X","value":"1","enabled":true},{"id":"online","var":"SPLAT3_START","value":"lobby","enabled":true}],
               "skip_title":{"var":"S3_SKIP","value":"yes","enabled":true}}"#,
        )
        .unwrap();
        let t = StartTable::load(&d);
        assert_eq!(t.env("range"), Some(("S3_SCENE".into(), "shooting-range".into())));
        assert_eq!(t.env("hub"), Some(("SPLAT3_START".into(), "plaza".into()))); // the bad variable name was refused
        assert!(t.get("new").is_none() && t.get("skip_title").is_none());
        assert!(t.get("online").is_none()); // the card of a removed mode, ignored like any unknown id
        std::fs::write(d.join("start-modes.json"), "{ nope").unwrap();
        assert_eq!(StartTable::load(&d), StartTable::default());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
