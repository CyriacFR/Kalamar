//! Keeping the data of an earlier launcher.
//!
//! The folder of the earlier identifier (`dev.splat3recomp.launcher`) is moved whole to this one's at the first start
//! (`datadir`), so its files are already here: [`run_startup`] only upgrades older-format files in place, after a
//! backup copy. For a data folder elsewhere (a portable one, `KALAMAR_DATA`), Settings > Launcher imports from a path
//! the player gives: this module COPIES (never moves, never deletes) that folder's files into the current one.
//!
//! What lives in a data directory, and what happens to it:
//!
//! | file | handling |
//! |---|---|
//! | `config.json` (language, game root, LAN settings, default match server, launch options, UI choices) | validated, rewritten in the current schema |
//! | `profile.json` | validated (typed), copied |
//! | `start-modes.json`, `match-vars.json`, `lan-env.json` (overrides) | JSON-validated, copied |
//! | `online\servers.json` and each server's folder `online\<id>\` | JSON files validated and copied; a server's other files (its session, encrypted for the Windows user) copied byte for byte without being opened, and one that already exists is never replaced |
//! | `last_run.json`, `last_launch*.json`, `last_match.json` | JSON-validated, copied |
//! | `game-cache\` (catalogue, icons) | not copied: regenerable and large |
//!
//! A file that fails validation is kept under `migration-rejected\` and reported. `migration.json` records the source,
//! the date and the files, so a migration never runs twice.

use crate::config::{self, LauncherConfig};
use crate::error::{AppError, AppResult};
use crate::profile::{self, Profile};
use crate::util;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MARKER: &str = "migration.json";
pub const REJECTED_DIR: &str = "migration-rejected";
const TOP_FILES: [&str; 9] = [
    "config.json", "profile.json", "start-modes.json", "match-vars.json", "lan-env.json", "last_run.json", "last_launch.json", "last_launch_p2.json", "last_match.json",
];
const REGENERABLE: [&str; 2] = ["game-cache", "relay.log"];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Migration {
    /// `import` (copied from another directory) or `in-place` (same directory, older formats upgraded).
    pub kind: String,
    pub source: String,
    pub date: u64,
    pub files: usize,
    /// Relative paths copied (names only, never content).
    pub imported: Vec<String>,
    /// Relative paths that failed validation; their copies are in `migration-rejected\`.
    pub rejected: Vec<String>,
    /// Left alone on purpose (caches) or already present (kept).
    pub skipped: Vec<String>,
    /// The notice has been shown (or none is due).
    pub shown: bool,
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

pub fn marker(dir: &Path) -> Option<Migration> {
    util::read_text_limited(&dir.join(MARKER), 1 << 20).ok().flatten().and_then(|t| serde_json::from_str(&t).ok())
}

fn write_marker(dir: &Path, m: &Migration) -> AppResult<()> {
    let text = serde_json::to_string_pretty(m).map_err(|e| AppError::Internal(e.to_string()))?;
    util::atomic_write(&dir.join(MARKER), text.as_bytes())
}

/// Marks the notice as shown.
pub fn acknowledge(dir: &Path) -> AppResult<()> {
    if let Some(mut m) = marker(dir) {
        m.shown = true;
        write_marker(dir, &m)?;
    }
    Ok(())
}

/// At first start: import from an earlier directory when this one has no configuration yet; otherwise upgrade older
/// formats in place. Runs once (the marker). Returns what it did, if anything.
pub fn run_startup(current: &Path, legacy: &[PathBuf]) -> Option<Migration> {
    if marker(current).is_some() {
        return None;
    }
    let has_config = current.join("config.json").is_file();
    if !has_config
        && let Some(src) = legacy.iter().find(|d| !same_dir(d, current) && (d.join("config.json").is_file() || d.join("profile.json").is_file()))
    {
        return import_from(current, src, false).ok();
    }
    if has_config { upgrade_in_place(current).ok() } else { None }
}

fn rel_string(rel: &Path) -> String {
    rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

fn collect(source: &Path) -> (Vec<PathBuf>, Vec<String>) {
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    for name in TOP_FILES {
        if source.join(name).is_file() {
            files.push(PathBuf::from(name));
        }
    }
    fn walk(base: &Path, rel: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(base.join(rel)) else { return };
        for e in rd.flatten() {
            let r = rel.join(e.file_name());
            match e.file_type() {
                Ok(t) if t.is_dir() => walk(base, &r, out),
                Ok(t) if t.is_file() => out.push(r),
                _ => {}
            }
        }
    }
    walk(source, Path::new("online"), &mut files);
    for name in REGENERABLE {
        if source.join(name).exists() {
            skipped.push(format!("{name} (rebuilt by the launcher)"));
        }
    }
    (files, skipped)
}

/// A file of a server's own folder (`online\<id>\...`) that is not JSON: its session and the like. Copied without
/// being opened, and never replaced.
fn opaque(rel: &Path) -> bool {
    rel.starts_with("online") && rel.components().count() >= 3 && rel.extension().is_none_or(|e| e != "json")
}

/// `Ok` when the file is sound for its kind. A server's opaque files are never opened.
fn validate(path: &Path, rel: &Path) -> Result<(), String> {
    let name = rel.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if opaque(rel) {
        return if path.metadata().map(|m| m.len() > 0).unwrap_or(false) { Ok(()) } else { Err("empty file".into()) };
    }
    let text = util::read_text_limited(path, 1 << 20).map_err(|e| e.to_string())?.ok_or("missing")?;
    if name.ends_with(".json") {
        let v: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("not JSON: {e}"))?;
        return match name.as_str() {
            "config.json" => serde_json::from_value::<LauncherConfig>(v).map(|_| ()).map_err(|e| format!("not a launcher configuration: {e}")),
            "profile.json" => serde_json::from_value::<Profile>(v).map(|_| ()).map_err(|e| format!("not a profile: {e}")),
            _ if v.is_object() || v.is_array() => Ok(()),
            _ => Err("unexpected JSON".into()),
        };
    }
    Ok(())
}

/// Copies the data of an earlier launcher into `current`. With `overwrite`, an existing file is first saved under
/// `migration-backup\<date>\`; a server's opaque file that already exists is never replaced.
pub fn import_from(current: &Path, source: &Path, overwrite: bool) -> AppResult<Migration> {
    if !source.is_dir() {
        return Err(AppError::InvalidInput(format!("{}: not a folder", source.display())));
    }
    if same_dir(source, current) {
        return Err(AppError::InvalidInput("this is the current data folder".into()));
    }
    let (files, mut skipped) = collect(source);
    if files.is_empty() {
        return Err(AppError::InvalidInput("no launcher data found in this folder".into()));
    }
    let date = util::now_secs();
    let mut m = Migration { kind: "import".into(), source: source.display().to_string(), date, ..Default::default() };
    for rel in files {
        let (from, to) = (source.join(&rel), current.join(&rel));
        let shown = rel_string(&rel);
        if validate(&from, &rel).is_err() {
            let kept = current.join(REJECTED_DIR).join(&rel);
            if let Some(d) = kept.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::copy(&from, &kept);
            m.rejected.push(shown);
            continue;
        }
        if to.exists() && (!overwrite || opaque(&rel)) {
            skipped.push(format!("{shown} (already present, kept)"));
            continue;
        }
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d).map_err(|e| AppError::io(d, &e))?;
        }
        if to.exists() {
            let bak = current.join("migration-backup").join(date.to_string()).join(&rel);
            if let Some(d) = bak.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::copy(&to, &bak);
        }
        std::fs::copy(&from, &to).map_err(|e| AppError::io(&to, &e))?;
        m.imported.push(shown);
    }
    // Rewrite the configuration in the current schema (new fields get their defaults).
    if m.imported.iter().any(|f| f == "config.json") {
        let cfg = config::load(current);
        config::save(current, &cfg)?;
    }
    m.files = m.imported.len();
    m.skipped = skipped;
    m.shown = m.imported.is_empty() && m.rejected.is_empty();
    write_marker(current, &m)?;
    Ok(m)
}

/// Same directory: files of an older format are rewritten in the current schema, after a backup copy in
/// `backup-pre-upgrade\` (once). Nothing is renamed or deleted.
pub fn upgrade_in_place(dir: &Path) -> AppResult<Migration> {
    let mut m = Migration { kind: "in-place".into(), source: dir.display().to_string(), date: util::now_secs(), shown: true, ..Default::default() };
    let backup = |name: &str| {
        let to = dir.join("backup-pre-upgrade").join(name);
        if !to.exists() {
            if let Some(d) = to.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::copy(dir.join(name), &to);
        }
    };
    let differs = |text: &str, normalised: &str| {
        serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')).ok() != serde_json::from_str::<serde_json::Value>(normalised).ok()
    };
    if let Ok(Some(text)) = util::read_text_limited(&dir.join("config.json"), 1 << 20)
        && let Ok(cfg) = serde_json::from_str::<LauncherConfig>(text.trim_start_matches('\u{feff}'))
        && let Ok(norm) = serde_json::to_string_pretty(&cfg)
        && differs(&text, &norm)
    {
        backup("config.json");
        config::save(dir, &cfg)?;
        m.imported.push("config.json (upgraded)".into());
    }
    if let Ok(Some(text)) = util::read_text_limited(&dir.join("profile.json"), 1 << 20)
        && let Ok(p) = serde_json::from_str::<Profile>(text.trim_start_matches('\u{feff}'))
        && let Ok(norm) = serde_json::to_string_pretty(&p)
        && differs(&text, &norm)
    {
        backup("profile.json");
        profile::save_profile(dir, &p)?;
        m.imported.push("profile.json (upgraded)".into());
    }
    m.files = m.imported.len();
    write_marker(dir, &m)?;
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("s3l-mig-{tag}-{}", util::random_hex(4)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A session file as the launcher writes it: binary, encrypted for the Windows user.
    const SESSION: &[u8] = b"\x01\x00\x00\x00\xd0\x8c\x9d\xdfSECRET-SESSION-CONTENT\xff\xfe";

    fn old_dir(tag: &str) -> PathBuf {
        let d = tmp(tag);
        std::fs::write(d.join("config.json"), r#"{"language":"en","lan_enabled":true,"lan":{"mode":"relay","relay_host":"10.0.0.5","room":"ROOM"}}"#).unwrap();
        std::fs::write(d.join("profile.json"), r#"{"schema":1,"name":"Kay","style":0}"#).unwrap();
        std::fs::write(d.join("start-modes.json"), r#"{"modes":[]}"#).unwrap();
        std::fs::write(d.join("last_run.json"), r#"{"state":"exited"}"#).unwrap();
        std::fs::create_dir_all(d.join("online").join("abc123")).unwrap();
        std::fs::write(d.join("online").join("servers.json"), r#"{"servers":[]}"#).unwrap();
        std::fs::write(d.join("online").join("abc123").join("session.dat"), SESSION).unwrap();
        std::fs::create_dir_all(d.join("game-cache").join("k")).unwrap();
        std::fs::write(d.join("game-cache").join("k").join("catalogue.json"), "{}").unwrap();
        d
    }

    fn snapshot(d: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        fn walk(base: &Path, d: &Path, out: &mut Vec<(String, Vec<u8>)>) {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                if e.file_type().unwrap().is_dir() {
                    walk(base, &e.path(), out);
                } else {
                    out.push((e.path().strip_prefix(base).unwrap().display().to_string(), std::fs::read(e.path()).unwrap()));
                }
            }
        }
        walk(d, d, &mut out);
        out.sort();
        out
    }

    #[test]
    fn a_fresh_install_has_nothing_to_do() {
        let (new, none) = (tmp("fresh"), tmp("nolegacy"));
        assert!(run_startup(&new, std::slice::from_ref(&none)).is_none());
        assert!(marker(&new).is_none());
        for d in [new, none] {
            std::fs::remove_dir_all(d).unwrap();
        }
    }

    #[test]
    fn old_data_is_copied_once_sessions_intact_and_the_old_folder_untouched() {
        let (old, new) = (old_dir("old"), tmp("new"));
        let before = snapshot(&old);
        let m = run_startup(&new, std::slice::from_ref(&old)).expect("import");
        assert_eq!(m.kind, "import");
        assert!(m.rejected.is_empty(), "{m:?}");
        assert_eq!(m.files, 6, "{:?}", m.imported); // config, profile, start-modes, last_run, servers.json, session.dat
        assert!(m.skipped.iter().any(|s| s.starts_with("game-cache")));
        // The session is the same bytes, and nothing the report says contains them.
        assert_eq!(std::fs::read(new.join("online").join("abc123").join("session.dat")).unwrap(), SESSION);
        let report = serde_json::to_string(&m).unwrap();
        assert!(!report.contains("SECRET-SESSION-CONTENT") && !format!("{m:?}").contains("SECRET"));
        assert!(m.imported.contains(&"online/abc123/session.dat".to_string()));
        // The configuration came through, in the current schema.
        let cfg = config::load(&new);
        assert_eq!((cfg.language.as_str(), cfg.lan.relay_host.as_str(), cfg.match_server.as_str()), ("en", "10.0.0.5", ""));
        assert_eq!(profile::load_profile(&new).name, "Kay");
        assert!(!new.join("game-cache").exists());
        // Never moved nor deleted.
        assert_eq!(snapshot(&old), before);
        // A second start is a no-op.
        let again = snapshot(&new);
        assert!(run_startup(&new, std::slice::from_ref(&old)).is_none());
        assert_eq!(snapshot(&new), again);
        for d in [old, new] {
            std::fs::remove_dir_all(d).unwrap();
        }
    }

    #[test]
    fn when_both_exist_nothing_is_overwritten() {
        let (old, new) = (old_dir("both-old"), tmp("both-new"));
        std::fs::write(new.join("config.json"), r#"{"language":"fr"}"#).unwrap();
        let m = run_startup(&new, std::slice::from_ref(&old)).expect("in place");
        assert_eq!(m.kind, "in-place");
        assert_eq!(config::load(&new).language, "fr");
        assert!(!new.join("online").exists() && !new.join("profile.json").exists());
        // The older-format config was upgraded after a backup copy.
        assert!(new.join("backup-pre-upgrade").join("config.json").is_file());
        assert_eq!(std::fs::read_to_string(new.join("backup-pre-upgrade").join("config.json")).unwrap(), r#"{"language":"fr"}"#);
        assert!(config::load(&new).match_server.is_empty());
        for d in [old, new] {
            std::fs::remove_dir_all(d).unwrap();
        }
    }

    #[test]
    fn a_corrupted_file_is_kept_aside_and_reported() {
        let (old, new) = (old_dir("bad-old"), tmp("bad-new"));
        std::fs::write(old.join("start-modes.json"), "{ nope").unwrap();
        std::fs::write(old.join("profile.json"), r#"{"name": 12}"#).unwrap();
        let m = import_from(&new, &old, false).unwrap();
        assert_eq!(m.rejected, vec!["profile.json".to_string(), "start-modes.json".to_string()]);
        assert!(!m.imported.contains(&"profile.json".to_string()));
        assert_eq!(std::fs::read_to_string(new.join(REJECTED_DIR).join("start-modes.json")).unwrap(), "{ nope");
        assert!(!new.join("start-modes.json").exists());
        assert!(!m.shown); // the user is told
        for d in [old, new] {
            std::fs::remove_dir_all(d).unwrap();
        }
    }

    #[test]
    fn a_custom_path_import_backs_up_what_it_replaces_and_never_replaces_a_session() {
        let (old, new) = (old_dir("custom-old"), tmp("custom-new"));
        std::fs::create_dir_all(new.join("online").join("abc123")).unwrap();
        std::fs::write(new.join("online").join("abc123").join("session.dat"), b"my-own-session").unwrap();
        std::fs::write(new.join("profile.json"), r#"{"schema":1,"name":"Current"}"#).unwrap();
        let m = import_from(&new, &old, true).unwrap();
        assert_eq!(profile::load_profile(&new).name, "Kay");
        assert_eq!(std::fs::read(new.join("online").join("abc123").join("session.dat")).unwrap(), b"my-own-session");
        assert!(m.skipped.iter().any(|s| s.starts_with("online/abc123/session.dat")));
        let bak = new.join("migration-backup").join(m.date.to_string()).join("profile.json");
        assert!(std::fs::read_to_string(bak).unwrap().contains("Current"));
        assert!(import_from(&new, &new, true).is_err());
        assert!(import_from(&new, &tmp("empty"), true).is_err());
        for d in [old, new] {
            std::fs::remove_dir_all(d).unwrap();
        }
    }
}
