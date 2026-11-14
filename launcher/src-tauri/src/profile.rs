//! Player profile (name + appearance + gear + weapon + splashtag), its validation against the game catalogue and its
//! conversion to the runtime's `save_profile` schema (`engine/docs/save-profile.md`).
//!
//! The chosen profile lives in `profile.json` in the launcher data directory (schema below). At launch the launcher
//! converts it ([`to_runtime`]), writes the result atomically to `<userdata>\launcher-save-profile.json` and passes
//! `TOTK_SAVE_PROFILE=<that file>`: the runtime applies it to the encrypted save while starting. What it did is read
//! back from the run log ([`parse_log`]).
//!
//! ```json
//! { "schema": 1, "name": "Kay",
//!   "style": 0, "skin": 2, "eyes": 5, "hair": 12, "eyebrows": 1, "legwear": 3,
//!   "head": 1000, "clothes": 8031, "shoes": 4024, "weapon": 10,
//!   "banner": 11001, "title": { "adjective": 142, "subject": 0 }, "badges": [1000000, null, null] }
//! ```
//!
//! Every number is the id the game stores (see `game::catalogue`); `null` means "leave unchanged / none".

use crate::error::{AppError, AppResult, FieldError};
use crate::game::catalogue::Catalogue;
use crate::util;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const PROFILE_SCHEMA: u32 = 1;
/// Conservative name limit: the console game's nickname limit is 10 characters; no documentation says otherwise.
pub const NAME_MAX_CHARS: usize = 10;
pub const BADGE_SLOTS: usize = 3;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Title {
    pub adjective: Option<i64>,
    pub subject: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Profile {
    pub schema: u32,
    pub name: String,
    pub style: Option<i64>,
    pub skin: Option<i64>,
    pub eyes: Option<i64>,
    pub hair: Option<i64>,
    pub eyebrows: Option<i64>,
    pub legwear: Option<i64>,
    /// Color variation of the legwear (0-based; most legwear has one).
    pub legwear_variation: Option<i64>,
    pub head: Option<i64>,
    pub clothes: Option<i64>,
    pub shoes: Option<i64>,
    pub weapon: Option<i64>,
    pub banner: Option<i64>,
    pub title: Title,
    pub badges: [Option<i64>; BADGE_SLOTS],
}

impl Default for Profile {
    fn default() -> Self {
        Profile {
            schema: PROFILE_SCHEMA,
            name: String::new(),
            style: None,
            skin: None,
            eyes: None,
            hair: None,
            eyebrows: None,
            legwear: None,
            legwear_variation: None,
            head: None,
            clothes: None,
            shoes: None,
            weapon: None,
            banner: None,
            title: Title::default(),
            badges: [None; BADGE_SLOTS],
        }
    }
}

impl Profile {
    /// (catalogue category, chosen id) for every single-id field.
    fn choices(&self) -> Vec<(&'static str, &'static str, Option<i64>)> {
        vec![
            ("style", "style", self.style),
            ("skin", "skin", self.skin),
            ("eyes", "eyes", self.eyes),
            ("hair", "hair", self.hair),
            ("eyebrows", "eyebrows", self.eyebrows),
            ("legwear", "legwear", self.legwear),
            ("head", "head", self.head),
            ("clothes", "clothes", self.clothes),
            ("shoes", "shoes", self.shoes),
            ("weapon", "weapon", self.weapon),
            ("banner", "banner", self.banner),
            ("title.adjective", "adjective", self.title.adjective),
            ("title.subject", "subject", self.title.subject),
        ]
    }
}

pub fn validate_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("name is required".into());
    }
    if n.chars().count() > NAME_MAX_CHARS {
        return Err(format!("name is limited to {NAME_MAX_CHARS} characters"));
    }
    if n.chars().any(|c| c.is_control() || c == '<' || c == '>') {
        return Err("name contains characters that are not allowed".into());
    }
    Ok(n.to_string())
}

fn entry<'a>(cat: &'a Catalogue, category: &str, id: i64) -> Option<&'a crate::game::catalogue::Entry> {
    cat.categories.get(category)?.iter().find(|e| e.id == id)
}

fn has(cat: &Catalogue, category: &str, id: i64) -> bool {
    cat.categories.get(category).is_some_and(|v| v.iter().any(|e| e.id == id && e.selectable))
}

/// Validates against the catalogue and returns the cleaned-up profile (current schema, trimmed name).
pub fn validate_profile(p: &Profile, cat: &Catalogue) -> AppResult<Profile> {
    let mut errors = Vec::new();
    let name = match validate_name(&p.name) {
        Ok(n) => n,
        Err(message) => {
            errors.push(FieldError { key: "name".into(), message });
            String::new()
        }
    };
    for (field, category, value) in p.choices() {
        if let Some(id) = value
            && !has(cat, category, id)
        {
            errors.push(FieldError { key: field.into(), message: format!("{id} is not a selectable {category} in the game data") });
        }
    }
    // Hair and eyebrows must belong to the style's species (the runtime refuses the whole profile otherwise).
    if let Some(style) = p.style
        && let Some(species) = entry(cat, "style", style).and_then(|e| e.species.as_deref())
    {
        for (field, category, value) in [("hair", "hair", p.hair), ("eyebrows", "eyebrows", p.eyebrows)] {
            if let Some(id) = value
                && let Some(e) = entry(cat, category, id)
                && e.species.as_deref().is_some_and(|s| s != species)
            {
                errors.push(FieldError { key: field.into(), message: format!("{id} is a {} look but the style is {species}", e.species.as_deref().unwrap_or("?")) });
            }
        }
    }
    if let (Some(v), Some(leg)) = (p.legwear_variation, p.legwear) {
        let max = entry(cat, "legwear", leg).and_then(|e| e.variations).unwrap_or(1).max(1);
        if v < 0 || v >= max {
            errors.push(FieldError { key: "legwear_variation".into(), message: format!("variation {v} does not exist (0..{})", max - 1) });
        }
    }
    if !errors.is_empty() {
        return Err(AppError::Validation(errors));
    }
    // Badges cannot be applied (see `game::catalogue::apply_save_constraints`): an old selection is dropped.
    Ok(Profile { schema: PROFILE_SCHEMA, name, badges: [None; BADGE_SLOTS], ..p.clone() })
}

pub fn profile_path(data_dir: &Path) -> PathBuf {
    data_dir.join("profile.json")
}

/// Loads `profile.json`. A file from an older launcher (string ids) or a damaged one only keeps its name.
pub fn load_profile(data_dir: &Path) -> Profile {
    let Some(text) = util::read_text_limited(&profile_path(data_dir), 1 << 20).ok().flatten() else {
        return Profile::default();
    };
    if let Ok(p) = serde_json::from_str::<Profile>(&text) {
        return p;
    }
    let name = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v.get("name")?.as_str().map(str::to_string)).unwrap_or_default();
    Profile { name, ..Default::default() }
}

pub fn save_profile(data_dir: &Path, p: &Profile) -> AppResult<()> {
    let text = serde_json::to_string_pretty(p).map_err(|e| AppError::Internal(e.to_string()))?;
    util::atomic_write(&profile_path(data_dir), text.as_bytes())
}

// ---- conversion to the runtime schema -------------------------------------------------------------------------

/// The four looks, by launcher style id. The launcher's ids are the order of the creation screen's four "kind" buttons
/// (`L_BtnKind_00..03`: 0 and 1 under the Inkling heading, 2 and 3 under the Octoling one, by pane position), and the
/// save's `Coordinates/ModelType` is main's `spl::PlayerModelType` enum "SquidF, SquidM, OctopusF, OctopusM" = 0..3, so
/// launcher style id == save value == index into this table.
pub const STYLE_NAMES: [&str; 4] = ["inkling_f", "inkling_m", "octoling_f", "octoling_m"];

pub fn style_name(id: i64) -> Option<&'static str> {
    usize::try_from(id).ok().and_then(|i| STYLE_NAMES.get(i)).copied()
}

#[cfg(test)]
pub fn style_id(name: &str) -> Option<i64> {
    STYLE_NAMES.iter().position(|n| *n == name).map(|i| i as i64)
}

/// What the user chose to do at launch (stored in `config.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyOptions {
    /// `skip_first_launch`: the game boots straight to the hub instead of the intro, character creation and tutorial.
    pub skip_first_launch: bool,
    /// `unlock_all`: the save owns every weapon, gear, banner, title word and emote.
    pub unlock_all: bool,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        ApplyOptions { skip_first_launch: true, unlock_all: true }
    }
}

/// The runtime applies appearance as one block (style, hair, eyebrows, legwear, skin, eyes): a profile without one of
/// them is refused whole, so the launcher does not pass it and says which fields are missing.
pub fn missing_appearance(p: &Profile) -> Vec<&'static str> {
    [("style", p.style), ("hair", p.hair), ("eyebrows", p.eyebrows), ("legwear", p.legwear), ("skin", p.skin), ("eyes", p.eyes)]
        .into_iter()
        .filter(|(_, v)| v.is_none())
        .map(|(k, _)| k)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvertError {
    /// Fields the runtime needs and the profile leaves empty.
    Incomplete(Vec<&'static str>),
    /// A value outside what the runtime accepts (`key`, reason).
    Invalid(&'static str, String),
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConvertError::Incomplete(k) => write!(f, "missing: {}", k.join(", ")),
            ConvertError::Invalid(k, why) => write!(f, "{k}: {why}"),
        }
    }
}

/// Pure mapping from the launcher profile to the runtime's `profile.json`. `null` splashtag, gear, weapon and (with an
/// existing save) appearance fields are left out: "unchanged" in the save. Badges are never sent (the save's badge
/// record is unknown). A **new** save (`save_exists` false) needs the whole appearance, else
/// [`ConvertError::Incomplete`] names what is missing; with a save every appearance field is optional.
pub fn to_runtime(p: &Profile, opts: ApplyOptions, save_exists: bool) -> Result<serde_json::Value, ConvertError> {
    use crate::game::catalogue::{DROPPED_GEAR, EYES_MAX, SKIN_MAX};
    use serde_json::{Map, Value, json};
    let missing = missing_appearance(p);
    if !save_exists && !missing.is_empty() {
        return Err(ConvertError::Incomplete(missing));
    }
    let complete = missing.is_empty();
    let name = validate_name(&p.name).map_err(|m| ConvertError::Invalid("name", m))?;
    if let Some(skin) = p.skin
        && !(0..=SKIN_MAX).contains(&skin)
    {
        return Err(ConvertError::Invalid("skin", format!("{skin} is outside 0..{SKIN_MAX}")));
    }
    if let Some(eyes) = p.eyes
        && !(0..=EYES_MAX).contains(&eyes)
    {
        return Err(ConvertError::Invalid("eyes", format!("{eyes} is outside 0..{EYES_MAX}")));
    }
    if p.legwear_variation.is_some_and(|v| v < 0) {
        return Err(ConvertError::Invalid("legwear_variation", "negative".into()));
    }
    let mut o = Map::new();
    o.insert("version".into(), json!(1));
    o.insert("name".into(), json!(name));
    if let Some(id) = p.style {
        let style = style_name(id).ok_or_else(|| ConvertError::Invalid("style", format!("{id} is not one of the four styles")))?;
        o.insert("style".into(), json!(style));
    }
    for (key, v) in [("hair", p.hair), ("eyebrows", p.eyebrows), ("legwear", p.legwear), ("skin", p.skin), ("eyes", p.eyes)] {
        if let Some(id) = v {
            o.insert(key.into(), json!(id));
        }
    }
    // A complete character always gets a variation (0 by default); a partial one leaves the save's own.
    match (p.legwear_variation, complete) {
        (Some(v), _) => {
            o.insert("legwear_variation".into(), json!(v));
        }
        (None, true) => {
            o.insert("legwear_variation".into(), json!(0));
        }
        (None, false) => {}
    }
    // The runtime counts an absent `skip_first_launch` as true only with the whole appearance, so a partial profile
    // never ends the first launch of a save that is still in it; asking for "no" is always sent.
    if complete || !opts.skip_first_launch {
        o.insert("skip_first_launch".into(), json!(opts.skip_first_launch));
    }
    o.insert("unlock_all".into(), json!(opts.unlock_all));

    let mut tag = Map::new();
    for (key, v) in [("banner", p.banner), ("adjective", p.title.adjective), ("subject", p.title.subject)] {
        if let Some(id) = v {
            tag.insert(key.into(), json!(id));
        }
    }
    if !tag.is_empty() {
        o.insert("splashtag".into(), Value::Object(tag));
    }
    let mut gear = Map::new();
    for (key, v) in [("head", p.head), ("clothes", p.clothes), ("shoes", p.shoes)] {
        if let Some(id) = v {
            if DROPPED_GEAR.contains(&(key, id)) {
                return Err(ConvertError::Invalid(key, format!("{id} is dropped by the game after loading the save")));
            }
            gear.insert(key.into(), json!(id));
        }
    }
    if !gear.is_empty() {
        o.insert("gear".into(), Value::Object(gear));
    }
    if let Some(w) = p.weapon {
        o.insert("weapon".into(), json!(w));
    }
    Ok(Value::Object(o))
}

/// Whether the user data directory already holds a save: `<profile dir>\save\<uid>\save.dat` (the runtime keeps one
/// folder per account). The runtime's own rule is the same file; a commit still waiting in `save-tx` is not counted.
pub fn save_exists(profile_dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(profile_dir.join("save")) else { return false };
    rd.flatten().any(|e| e.path().join("save.dat").is_file())
}

/// File the runtime profile is written to, next to the user data of the game root.
pub fn runtime_profile_path(userdata: &Path) -> PathBuf {
    userdata.join("launcher-save-profile.json")
}

/// Environment of a launch that applies a profile: the file to apply and the player name (`player_name` /
/// `SPLAT3_NICKNAME`, the nickname the game shows).
pub fn profile_env(file: &Path, name: &str) -> Vec<(String, String)> {
    let mut env = vec![("TOTK_SAVE_PROFILE".to_string(), file.display().to_string())];
    let n = name.trim();
    if !n.is_empty() {
        env.push(("TOTK_PLAYER_NAME".into(), n.to_string()));
        env.push(("SPLAT3_NICKNAME".into(), n.to_string()));
    }
    env
}

/// Converts, writes the runtime profile atomically and returns the environment to add to the launch.
pub fn prepare_launch(p: &Profile, opts: ApplyOptions, userdata: &Path, save_exists: bool) -> Result<Vec<(String, String)>, ConvertError> {
    let value = to_runtime(p, opts, save_exists)?;
    let file = runtime_profile_path(userdata);
    let text = serde_json::to_string_pretty(&value).map_err(|e| ConvertError::Invalid("profile", e.to_string()))?;
    util::atomic_write(&file, text.as_bytes()).map_err(|e| ConvertError::Invalid("profile", e.to_string()))?;
    Ok(profile_env(&file, &p.name))
}

// ---- reading back what the runtime did -----------------------------------------------------------------------

/// A refusal of the runtime, mapped to the profile field it is about so the UI can say it next to that field.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct Refusal {
    /// Profile field key (`hair`, `eyebrows`, `style`, `legwear`, `legwear_variation`, `skin`, `eyes`, `weapon`, `gear`
    /// with the id, `splashtag`, `badges`, `appearance`) or empty when it is about the save itself.
    pub field: String,
    /// Stable code the UI translates (`species`, `unselectable`, `gear_dropped`, ...); `other` keeps only `raw`.
    pub code: String,
    /// The id the runtime names, when it names one.
    pub id: Option<i64>,
    /// `species`: the species of the item (`inkling` or `octoling`).
    pub species: Option<String>,
    /// The style the runtime checked against (`inkling_f`...).
    pub style: Option<String>,
    /// The offending value is the save's own (a partial profile changed the style but not the hair): give a matching one.
    pub from_save: bool,
    /// The runtime's sentence, untranslated and unmodified.
    pub raw: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct Feedback {
    /// `applied` (the save was changed, backup made), `already` (0 changes: nothing written, no backup), `new_save`
    /// (no save yet: the profile goes into the one the game creates, then it restarts), `restarting`,
    /// `needs_appearance` (new save without the whole appearance: not applied), `failed` (refused or not written),
    /// `none` (no `save profile:` line in the log).
    pub state: String,
    pub changes: Option<u32>,
    pub backup: Option<String>,
    /// The runtime's reason when `failed`.
    pub reason: Option<String>,
    pub refusal: Option<Refusal>,
    /// `needs_appearance`: the fields the profile lacks.
    pub missing: Vec<String>,
    /// Backups the runtime deleted after this launch (`deleted N older backups`), and how many it keeps.
    pub deleted_backups: Option<u32>,
    pub keep_backups: Option<u32>,
    /// The `save profile:` lines themselves, shortened.
    pub lines: Vec<String>,
}

fn first_int(s: &str) -> Option<i64> {
    let digits: String = s.trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
    digits.parse().ok()
}

/// The id in parentheses of a runtime sentence: `fail(why, id)` prints `why (id); the save is left as it is`.
fn trailing_id(msg: &str) -> Option<i64> {
    let end = msg.rfind("); the save is left")?;
    let start = msg[..end].rfind(" (")?;
    msg[start + 2..end].parse().ok()
}

/// Maps one refusal sentence of `runtime/save_profile.c` (`fail()` and `check_species()`) to a [`Refusal`].
pub fn classify_refusal(msg: &str) -> Refusal {
    let mk = |field: &str, code: &str| Refusal { field: field.into(), code: code.into(), id: trailing_id(msg), raw: msg.to_string(), ..Default::default() };
    for what in ["hair", "eyebrows"] {
        if let Some(rest) = msg.strip_prefix(what).and_then(|r| r.strip_prefix(' ')) {
            let id = first_int(rest);
            let style = rest.split("style ").nth(1).map(|s| s.split([' ', ';', '(']).next().unwrap_or("").to_string()).filter(|s| !s.is_empty());
            let (code, species) = if rest.contains("belongs to Octolings") {
                ("species", Some("octoling".to_string()))
            } else if rest.contains("belongs to Inklings") {
                ("species", Some("inkling".to_string()))
            } else {
                ("unselectable", None)
            };
            return Refusal { field: what.into(), code: code.into(), id, species, style, from_save: msg.contains("it is the save's value"), raw: msg.to_string() };
        }
    }
    let table: [(&str, &str, &str); 14] = [
        ("style must be", "style", "style"),
        ("hair, eyebrows, legwear", "appearance", "integers"),
        ("legwear is not a selectable", "legwear", "legwear"),
        ("legwear_variation out of range", "legwear_variation", "legwear_variation"),
        ("skin must be", "skin", "skin_range"),
        ("eyes must be", "eyes", "eyes_range"),
        ("weapon is not a Versus", "weapon", "weapon"),
        ("weapon must be", "weapon", "weapon"),
        ("the game removes this gear", "gear", "gear_dropped"),
        ("gear id is not a player gear", "gear", "gear_unknown"),
        ("gear ids must be", "gear", "gear_unknown"),
        ("splashtag id is not in", "splashtag", "splashtag"),
        ("splashtag ids must be", "splashtag", "splashtag"),
        ("badge not owned", "badges", "badge"),
    ];
    for (prefix, field, code) in table {
        if msg.starts_with(prefix) {
            return mk(field, code);
        }
    }
    if msg.starts_with("the save has no") || msg.contains("ownership maps are missing") || msg.contains("does not hold the 11.2.0 layout") {
        return mk("", "save_layout");
    }
    if msg.contains("does not pass its check") {
        return mk("", "save_check");
    }
    mk("", "other")
}

/// Reads the `save profile:` lines of a run log (`rt_log` output). Formats: `runtime/save_profile.c` and
/// `docs/save-profile.md`.
pub fn parse_log(text: &str) -> Feedback {
    let mut f = Feedback { state: "none".into(), ..Default::default() };
    let (mut applied, mut already, mut new_save, mut restarting, mut needs) = (false, false, false, false, false);
    let mut changes = 0u32;
    for line in text.lines() {
        let Some(at) = line.find("save profile: ") else { continue };
        let msg = &line[at + "save profile: ".len()..];
        f.lines.push(line[at..].chars().take(400).collect());
        if let Some(pos) = msg.find(" applied to ") {
            let rest = &msg[pos..];
            if rest.contains(": 0 changes (") {
                already = true;
                restarting = false; // the restarted game found the save already done
            } else {
                applied = true;
                if let Some(open) = rest.find(" (")
                    && let Some(n) = rest[open + 2..].split_whitespace().next().and_then(|n| n.parse::<u32>().ok())
                {
                    changes += n;
                }
                if let Some(b) = rest.find("backup ") {
                    let b = &rest[b + "backup ".len()..];
                    let end = b.find("):").or_else(|| b.find(')')).unwrap_or(b.len());
                    f.backup = Some(b[..end].to_string());
                }
            }
        } else if msg.contains(" already matches ") {
            already = true;
            restarting = false;
        } else if let Some(rest) = msg.strip_prefix("deleted ") {
            f.deleted_backups = first_int(rest).and_then(|n| u32::try_from(n).ok());
            f.keep_backups = rest.split("newest ").nth(1).and_then(first_int).and_then(|n| u32::try_from(n).ok());
        } else if msg.starts_with("no save yet, and a new save needs the whole appearance") {
            needs = true;
            f.missing = msg
                .split("lacks ")
                .nth(1)
                .map(|r| r.split(';').next().unwrap_or("").split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
                .unwrap_or_default();
        } else if msg.starts_with("no save yet") {
            new_save = true;
        } else if msg.starts_with("restarting") {
            restarting = true;
        } else if (msg.contains("left as it is") || msg.contains("profile off") || msg.contains("cannot write") || msg.contains("cannot back")) && f.reason.is_none() {
            f.reason = Some(msg.to_string());
            f.refusal = Some(classify_refusal(msg));
        }
    }
    if applied {
        f.changes = Some(changes);
    }
    f.state = if f.reason.is_some() && !applied {
        "failed"
    } else if restarting {
        "restarting"
    } else if applied {
        "applied"
    } else if already {
        "already"
    } else if needs {
        "needs_appearance"
    } else if new_save {
        "new_save"
    } else {
        "none"
    }
    .into();
    if already && !applied {
        f.changes = Some(0);
    }
    f
}

/// The backups the runtime keeps of the save before it changes it: `<profile dir>\save-profile-backup\yyyymmdd-hhmmss`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct BackupInfo {
    pub dir: String,
    pub exists: bool,
    pub count: usize,
    /// Name of the newest folder (`yyyymmdd-hhmmss`).
    pub newest: Option<String>,
}

fn is_backup_name(n: &str) -> bool {
    n.len() == 15 && n.as_bytes()[8] == b'-' && n.bytes().enumerate().all(|(i, b)| i == 8 || b.is_ascii_digit())
}

pub fn backup_info(profile_dir: &Path) -> BackupInfo {
    let dir = profile_dir.join("save-profile-backup");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map(|rd| rd.flatten().filter(|e| e.path().is_dir()).filter_map(|e| e.file_name().into_string().ok()).filter(|n| is_backup_name(n)).collect())
        .unwrap_or_default();
    names.sort();
    BackupInfo { dir: dir.display().to_string(), exists: dir.is_dir(), count: names.len(), newest: names.last().cloned() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::catalogue::{Entry, neutral};

    fn cat_with_gear() -> Catalogue {
        let mut c = neutral();
        c.categories.insert("head".into(), vec![Entry { id: 1000, selectable: true, ..Default::default() }, Entry { id: 20001, selectable: false, ..Default::default() }]);
        c.categories.insert("badge".into(), vec![Entry { id: 7, selectable: true, ..Default::default() }, Entry { id: 8, selectable: true, ..Default::default() }]);
        c
    }

    #[test]
    fn name_rules() {
        assert_eq!(validate_name("  Zoé ").unwrap(), "Zoé");
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
        assert!(validate_name("12345678901").is_err());
        assert!(validate_name("1234567890").is_ok());
        assert!(validate_name("ééééééééé").is_ok()); // counted in characters, not bytes
        assert!(validate_name("a\u{7}b").is_err());
        assert!(validate_name("<b>").is_err());
    }

    #[test]
    fn profile_validation_reports_each_field() {
        let cat = cat_with_gear();
        let ok = Profile { name: "Kay".into(), style: Some(1), hair: Some(12), head: Some(1000), badges: [Some(7), Some(8), None], ..Default::default() };
        assert!(validate_profile(&ok, &cat).unwrap().badges.iter().all(Option::is_none)); // badges are dropped
        let bad = Profile { name: "".into(), hair: Some(99), head: Some(20001), shoes: Some(5), badges: [Some(7), Some(7), Some(404)], title: Title { adjective: Some(1), subject: None }, ..Default::default() };
        match validate_profile(&bad, &cat).unwrap_err() {
            AppError::Validation(v) => assert_eq!(
                v.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(),
                vec!["name", "hair", "head", "shoes", "title.adjective"]
            ),
            e => panic!("{e:?}"),
        }
    }

    #[test]
    fn json_shape_matches_the_documented_schema() {
        let p = Profile { name: "Kay".into(), style: Some(0), badges: [Some(1), None, None], title: Title { adjective: Some(142), subject: Some(0) }, ..Default::default() };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["schema"], 1);
        assert_eq!(v["title"]["adjective"], 142);
        assert_eq!(v["badges"], serde_json::json!([1, null, null]));
        assert!(v["weapon"].is_null());
        let back: Profile = serde_json::from_value(v).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn old_or_broken_files_only_keep_the_name() {
        let d = std::env::temp_dir().join(format!("s3l-prof-{}", util::random_hex(4)));
        assert_eq!(load_profile(&d), Profile::default());
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(profile_path(&d), r#"{"name":"Old","style":"style_1","hairstyle":"hair_2"}"#).unwrap();
        let p = load_profile(&d);
        assert_eq!(p.name, "Old");
        assert_eq!(p.style, None);
        std::fs::write(profile_path(&d), "{{").unwrap();
        assert_eq!(load_profile(&d).name, "");
        let p = Profile { name: "Ink".into(), legwear: Some(2), ..Default::default() };
        save_profile(&d, &p).unwrap();
        assert_eq!(load_profile(&d), p);
        std::fs::remove_dir_all(&d).unwrap();
    }

    fn full() -> Profile {
        Profile {
            name: "Kay".into(), style: Some(2), skin: Some(2), eyes: Some(18), hair: Some(33), eyebrows: Some(21), legwear: Some(1), legwear_variation: Some(2),
            head: Some(1018), clothes: Some(5001), shoes: Some(6003), weapon: Some(50), banner: Some(11020),
            title: Title { adjective: Some(200), subject: Some(300) }, badges: [Some(5), None, None], ..Default::default()
        }
    }

    #[test]
    fn style_ids_match_the_save_enum() {
        // ModelType: SquidF 0, SquidM 1, OctopusF 2, OctopusM 3 (docs/save-profile.md); launcher id == save value.
        assert_eq!(style_name(0), Some("inkling_f"));
        assert_eq!(style_name(1), Some("inkling_m"));
        assert_eq!(style_name(2), Some("octoling_f"));
        assert_eq!(style_name(3), Some("octoling_m"));
        assert_eq!(style_name(4), None);
        assert_eq!(style_name(-1), None);
        for (i, n) in STYLE_NAMES.iter().enumerate() {
            assert_eq!(style_id(n), Some(i as i64));
        }
        assert_eq!(style_id("nope"), None);
    }

    #[test]
    fn full_profile_maps_to_the_example_shape() {
        let v = to_runtime(&full(), ApplyOptions { skip_first_launch: true, unlock_all: false }, false).unwrap();
        // tools/profile/profile.example.json, field for field
        assert_eq!(
            v,
            serde_json::json!({
                "version": 1, "name": "Kay", "style": "octoling_f", "hair": 33, "eyebrows": 21, "legwear": 1, "legwear_variation": 2,
                "skin": 2, "eyes": 18, "skip_first_launch": true, "unlock_all": false,
                "splashtag": {"banner": 11020, "adjective": 200, "subject": 300},
                "gear": {"head": 1018, "clothes": 5001, "shoes": 6003}, "weapon": 50
            })
        );
        assert!(v["splashtag"].get("badges").is_none()); // badges are never sent
    }

    #[test]
    fn nulls_are_left_out_and_appearance_is_all_or_nothing() {
        let mut p = full();
        p.head = None;
        p.weapon = None;
        p.banner = None;
        p.title = Title::default();
        p.legwear_variation = None;
        let v = to_runtime(&p, ApplyOptions::default(), false).unwrap();
        assert!(v.get("splashtag").is_none() && v.get("weapon").is_none());
        assert_eq!(v["gear"], serde_json::json!({"clothes": 5001, "shoes": 6003}));
        assert_eq!(v["legwear_variation"], 0);
        assert_eq!(v["unlock_all"], true);
        p.clothes = None;
        p.shoes = None;
        assert!(to_runtime(&p, ApplyOptions::default(), false).unwrap().get("gear").is_none());
        let mut q = full();
        q.hair = None;
        q.skin = None;
        assert_eq!(to_runtime(&q, ApplyOptions::default(), false).unwrap_err(), ConvertError::Incomplete(vec!["hair", "skin"]));
        assert_eq!(missing_appearance(&Profile::default()).len(), 6);
    }

    #[test]
    fn values_the_runtime_would_refuse_are_refused_here() {
        let mut p = full();
        p.skin = Some(9);
        assert!(matches!(to_runtime(&p, ApplyOptions::default(), false), Err(ConvertError::Invalid("skin", _))));
        let mut p = full();
        p.eyes = Some(21);
        assert!(matches!(to_runtime(&p, ApplyOptions::default(), false), Err(ConvertError::Invalid("eyes", _))));
        let mut p = full();
        p.style = Some(7);
        assert!(matches!(to_runtime(&p, ApplyOptions::default(), false), Err(ConvertError::Invalid("style", _))));
        let mut p = full();
        p.shoes = Some(3009);
        assert!(matches!(to_runtime(&p, ApplyOptions::default(), false), Err(ConvertError::Invalid("shoes", _))));
        let mut p = full();
        p.name = "".into();
        assert!(matches!(to_runtime(&p, ApplyOptions::default(), false), Err(ConvertError::Invalid("name", _))));
    }

    #[test]
    fn species_and_variation_are_validated_against_the_catalogue() {
        let mut cat = cat_with_gear();
        let mk = |id, species: &str, variations| Entry { id, selectable: true, species: Some(species.into()), variations, ..Default::default() };
        cat.categories.insert("style".into(), vec![mk(0, "inkling", None), mk(2, "octoling", None)]);
        cat.categories.insert("hair".into(), vec![mk(3, "inkling", None), mk(33, "octoling", None)]);
        cat.categories.insert("legwear".into(), vec![mk(1, "", Some(3))]);
        let ok = Profile { name: "K".into(), style: Some(2), hair: Some(33), legwear: Some(1), legwear_variation: Some(2), ..Default::default() };
        assert!(validate_profile(&ok, &cat).is_ok());
        let bad = Profile { hair: Some(3), legwear_variation: Some(3), ..ok.clone() };
        match validate_profile(&bad, &cat).unwrap_err() {
            AppError::Validation(v) => assert_eq!(v.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(), vec!["hair", "legwear_variation"]),
            e => panic!("{e:?}"),
        }
    }

    #[test]
    fn env_and_file_for_a_launch() {
        let env = profile_env(Path::new("C:/g/userdata/launcher-save-profile.json"), " Kay ");
        assert_eq!(env[0], ("TOTK_SAVE_PROFILE".to_string(), "C:/g/userdata/launcher-save-profile.json".to_string()));
        assert_eq!(env[1], ("TOTK_PLAYER_NAME".to_string(), "Kay".to_string()));
        assert_eq!(env[2], ("SPLAT3_NICKNAME".to_string(), "Kay".to_string()));
        assert_eq!(profile_env(Path::new("p.json"), "  ").len(), 1);
        let d = std::env::temp_dir().join(format!("s3l-prep-{}", util::random_hex(4)));
        let env = prepare_launch(&full(), ApplyOptions::default(), &d, false).unwrap();
        let text = std::fs::read_to_string(runtime_profile_path(&d)).unwrap();
        assert!(text.contains("\"octoling_f\""));
        assert_eq!(env[0].1, runtime_profile_path(&d).display().to_string());
        assert!(prepare_launch(&Profile::default(), ApplyOptions::default(), &d, false).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn log_lines_of_every_outcome() {
        let applied = "[host] save profile: C:/u/p.json applied to C:/u/profile/save/x/save.dat (14 changes, backup C:/u/profile/save-profile-backup/20261110-040000):\n   ModelType=2";
        let f = parse_log(applied);
        assert_eq!((f.state.as_str(), f.changes), ("applied", Some(14)));
        assert_eq!(f.backup.as_deref(), Some("C:/u/profile/save-profile-backup/20261110-040000"));
        assert_eq!(parse_log("save profile: C:/p.json already matches C:/s/save.dat").state, "already");
        let zero = parse_log("[host] save profile: C:/u/p.json applied to C:/u/profile/save/x/save.dat: 0 changes (the save already matches; nothing written, no backup)");
        assert_eq!((zero.state.as_str(), zero.changes, zero.backup), ("already", Some(0), None));
        let first = "save profile: no save yet; the profile goes into the save the game creates, then the game restarts";
        assert_eq!(parse_log(first).state, "new_save");
        let both = format!("{first}\nsave profile: C:/p applied to C:/s (30 changes, backup C:/b):\nsave profile: restarting so the game loads the new save");
        let f = parse_log(&both);
        assert_eq!((f.state.as_str(), f.changes), ("restarting", Some(30)));
        let after = format!("{both}
save profile: C:/p already matches C:/s");
        assert_eq!(parse_log(&after).state, "applied"); // the restarted process confirmed it
        let bad = parse_log("save profile: hair is not a hairstyle of that style (3); the save is left as it is");
        assert_eq!(bad.state, "failed");
        assert!(bad.reason.unwrap().contains("hairstyle"));
        assert_eq!(parse_log("[host] nothing here").state, "none");
        assert!(parse_log("save profile: cannot read C:/x.json; the save is left as it is").reason.is_some());
    }

    #[test]
    fn partial_profiles_are_accepted_only_on_an_existing_save() {
        let weapon_only = Profile { name: "Kay".into(), weapon: Some(50), ..Default::default() };
        // new save: refused, naming the six fields
        assert_eq!(
            to_runtime(&weapon_only, ApplyOptions::default(), false).unwrap_err(),
            ConvertError::Incomplete(vec!["style", "hair", "eyebrows", "legwear", "skin", "eyes"])
        );
        // existing save: only what the profile sets; no skip_first_launch=true for a partial profile (the runtime
        // would count it as ending the first launch), no appearance, no variation
        let v = to_runtime(&weapon_only, ApplyOptions::default(), true).unwrap();
        assert_eq!(v, serde_json::json!({"version": 1, "name": "Kay", "unlock_all": true, "weapon": 50}));
        // asking for "no" is always sent
        let v = to_runtime(&weapon_only, ApplyOptions { skip_first_launch: false, unlock_all: false }, true).unwrap();
        assert_eq!(v["skip_first_launch"], false);
        // one appearance field alone is fine on an existing save, and out-of-range values are still refused
        let eyes = Profile { name: "Kay".into(), eyes: Some(4), ..Default::default() };
        assert_eq!(to_runtime(&eyes, ApplyOptions::default(), true).unwrap()["eyes"], 4);
        let bad = Profile { eyes: Some(40), ..eyes };
        assert!(matches!(to_runtime(&bad, ApplyOptions::default(), true), Err(ConvertError::Invalid("eyes", _))));
        // a complete profile behaves as before on an existing save
        let v = to_runtime(&full(), ApplyOptions::default(), true).unwrap();
        assert_eq!(v["skip_first_launch"], true);
        assert_eq!(v["legwear_variation"], 2);
    }

    #[test]
    fn save_detection_looks_for_save_dat_under_save_uid() {
        let d = std::env::temp_dir().join(format!("s3l-save-{}", util::random_hex(4)));
        assert!(!save_exists(&d));
        std::fs::create_dir_all(d.join("save").join("14661551906a0791439812cc30d2ab74")).unwrap();
        assert!(!save_exists(&d)); // an empty account folder is no save
        std::fs::write(d.join("save").join("14661551906a0791439812cc30d2ab74").join("save.dat"), b"x").unwrap();
        assert!(save_exists(&d));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn backups_are_counted_by_their_folder_names() {
        let d = std::env::temp_dir().join(format!("s3l-bak-{}", util::random_hex(4)));
        assert_eq!(backup_info(&d).count, 0);
        let b = d.join("save-profile-backup");
        for n in ["20261110-040000", "20261110-050101", "notes", "2026-11-10"] {
            std::fs::create_dir_all(b.join(n)).unwrap();
        }
        std::fs::write(b.join("20261111-000000"), b"a file, not a backup").unwrap();
        let i = backup_info(&d);
        assert_eq!((i.count, i.newest.as_deref(), i.exists), (2, Some("20261110-050101"), true));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn new_log_lines_of_the_final_runtime() {
        let zero = "save profile: p.json applied to s/save.dat: 0 changes (the save already matches; nothing written, no backup)";
        let f = parse_log(zero);
        assert_eq!((f.state.as_str(), f.changes, f.backup.as_deref()), ("already", Some(0), None));
        let one = "save profile: p.json applied to s/save.dat (1 changes, backup C:/u/save-profile-backup/20261110-050101):
  WeaponId 10 -> 50
save profile: deleted 2 older backups (the newest 10 are kept)";
        let f = parse_log(one);
        assert_eq!((f.state.as_str(), f.changes, f.deleted_backups, f.keep_backups), ("applied", Some(1), Some(2), Some(10)));
        let new = "save profile: no save yet, and a new save needs the whole appearance: the profile lacks hair, skin; the profile is not applied, the game creates its own character";
        let f = parse_log(new);
        assert_eq!(f.state, "needs_appearance");
        assert_eq!(f.missing, vec!["hair", "skin"]);
    }

    #[test]
    fn refusals_are_mapped_to_the_field_they_are_about() {
        let r = classify_refusal("hair 33 belongs to Octolings, not to the style inkling_f (it is the save's value: give a matching one in the profile too); the save is left as it is");
        assert_eq!((r.field.as_str(), r.code.as_str(), r.id), ("hair", "species", Some(33)));
        assert_eq!((r.species.as_deref(), r.style.as_deref(), r.from_save), (Some("octoling"), Some("inkling_f"), true));
        let r = classify_refusal("eyebrows 2 belongs to Inklings, not to the style octoling_m (2); the save is left as it is");
        assert_eq!((r.field.as_str(), r.species.as_deref(), r.style.as_deref(), r.from_save), ("eyebrows", Some("inkling"), Some("octoling_m"), false));
        let r = classify_refusal("hair 99 is not a selectable one of the style inkling_m (99); the save is left as it is");
        assert_eq!((r.code.as_str(), r.id), ("unselectable", Some(99)));
        let r = classify_refusal("the game removes this gear from the save when it loads it; do not equip it (3007); the save is left as it is");
        assert_eq!((r.field.as_str(), r.code.as_str(), r.id), ("gear", "gear_dropped", Some(3007)));
        let r = classify_refusal("weapon is not a Versus main weapon (5555); the save is left as it is");
        assert_eq!((r.field.as_str(), r.id), ("weapon", Some(5555)));
        assert_eq!(classify_refusal("skin must be 0..8 (9); the save is left as it is").field, "skin");
        assert_eq!(classify_refusal("legwear_variation out of range (7); the save is left as it is").field, "legwear_variation");
        assert_eq!(classify_refusal("the save has no UserName (0); the save is left as it is").code, "save_layout");
        let r = classify_refusal("something new; the save is left as it is");
        assert_eq!((r.code.as_str(), r.raw.as_str()), ("other", "something new; the save is left as it is"));
        // end to end through the log
        let f = parse_log("[x] save profile: hair 33 belongs to Octolings, not to the style inkling_f (33); the save is left as it is");
        assert_eq!(f.state, "failed");
        assert_eq!(f.refusal.unwrap().field, "hair");
    }
}
