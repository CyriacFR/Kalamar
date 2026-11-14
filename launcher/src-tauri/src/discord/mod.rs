//! Discord presence ("Rich Presence"): what the player's Discord contacts see while Kalamar is open. Off by default.
//!
//! **Only through the Discord application installed on this PC**, by its local channel (`ipc.rs`: the named pipe
//! `\\.\pipe\discord-ipc-<n>` and its documented protocol). The launcher makes no network access for it, embeds no
//! Discord library, and opens nothing until the player switches the presence on (Settings › Discord). Discord itself
//! then shares the status with the player's contacts, as for any game.
//!
//! **One source**: the launcher. It knows the whole situation (the Locker, a room, a search, a match, the stands); the
//! game it starts is told to stay quiet (`SPLAT3_DISCORD_PRESENCE=launcher`, `launch::build_launch_spec`), and the game
//! started alone shows only "In game" under the same settings, which the launcher copies into its settings file
//! (`discord_rpc`, `discord_app_id`; `runtime/discord_rpc.c`). A machine-wide lock keeps a second launcher, or the game
//! started alone, quiet while one shows it.
//!
//! **What is said** is built by the page in the interface language (`src/discord/presence.ts`): the level the player
//! chose (minimal, or the activity), and on request the mode and stage (named by the player's own game data) and the
//! community server's name. Never another player's name, a room code or password, an address. This side checks what
//! the page sends (`activity.rs`), keeps Discord's rate limit and owns the connection (`worker.rs`).
//!
//! **The application id** (public: it names the "Kalamar" application and its pictures in Discord) is fixed at build
//! time: `KALAMAR_DISCORD_CLIENT_ID` (`build.rs`). Empty, the default and every development build: the feature does not
//! exist in that build (nothing in the interface, no pipe ever opened). How the maintainers create the application and its
//! pictures: `docs/discord-presence.md`.

pub mod activity;
#[cfg(all(test, windows))]
mod fake;
pub mod ipc;
pub mod worker;

use crate::commands::AppState;
use crate::error::AppResult;
use activity::{Activity, ActivityInput};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use worker::{Cmd, Options, Report, Status};

/// The application id compiled in, or empty.
pub const CLIENT_ID: &str = env!("KALAMAR_DISCORD_CLIENT_ID");
/// The machine-wide lock of "one presence at a time" (also taken by `runtime/discord_rpc.c`).
pub const LOCK_NAME: &str = r"Local\Kalamar.DiscordPresence";
/// Passed to every game the launcher starts: the launcher shows the presence, the game stays quiet.
pub const ENV_PRESENCE: &str = "SPLAT3_DISCORD_PRESENCE";
pub const ENV_PRESENCE_LAUNCHER: &str = "launcher";

/// This build has the feature (an application id was compiled in).
pub fn available() -> bool {
    !CLIENT_ID.is_empty()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Detail {
    /// "In game" or "In the launcher", and for how long.
    Minimal,
    /// What the player does (offline, training, room, search, match, stands) and the size of their group.
    #[default]
    Activity,
}

/// The player's choices (`config.json`, `discord`). Off by default; once on, the activity without mode, stage or server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct DiscordPrefs {
    pub enabled: bool,
    pub detail: Detail,
    /// Add the mode and the stage (names from the player's own game data).
    pub mode_stage: bool,
    /// Add the community server's name (never its address).
    pub server: bool,
}

/// The presence of this launcher: the thread is started the first time the player switches it on, never before.
pub struct Presence {
    opts: Option<Options>,
    report: Report,
    status: Arc<Mutex<Status>>,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    tx: Option<std::sync::mpsc::Sender<Cmd>>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// The page's latest activity, kept for the moment the presence is switched on.
    last: Option<Activity>,
}

impl Presence {
    /// `opts` `None`: a build without the feature; every call does nothing. `on_status` hears each change of state.
    pub fn new(opts: Option<Options>, on_status: impl Fn(&Status) + Send + Sync + 'static) -> Presence {
        let status = Arc::new(Mutex::new(Status::OFF));
        let s2 = status.clone();
        let report: Report = Arc::new(move |s: &Status| {
            *s2.lock().unwrap_or_else(|p| p.into_inner()) = s.clone();
            on_status(s);
        });
        Presence { opts, report, status, inner: Mutex::new(Inner::default()) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn set_enabled(&self, on: bool) {
        let Some(opts) = &self.opts else { return };
        let mut g = self.lock();
        if !on {
            // The page sends the present activity again when the presence comes back on.
            g.last = None;
        }
        if on && g.tx.is_none() {
            match worker::spawn(opts.clone(), self.report.clone()) {
                Ok((tx, thread)) => {
                    let _ = tx.send(Cmd::Activity(g.last.clone()));
                    g.tx = Some(tx);
                    g.thread = Some(thread);
                }
                Err(_) => return,
            }
        }
        if let Some(tx) = &g.tx {
            let _ = tx.send(Cmd::Enabled(on));
        }
    }

    /// The page's current activity (`None`: nothing to show). Kept while off, sent only while on.
    pub fn publish(&self, a: Option<Activity>) {
        let mut g = self.lock();
        g.last = a.clone();
        if let Some(tx) = &g.tx {
            let _ = tx.send(Cmd::Activity(a));
        }
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// The launcher quits: the presence is cleared and the pipe closed, waiting at most `wait` for it.
    pub fn shutdown(&self, wait: Duration) {
        let (tx, thread) = {
            let mut g = self.lock();
            (g.tx.take(), g.thread.take())
        };
        let Some(tx) = tx else { return };
        let (ack, done) = std::sync::mpsc::channel();
        if tx.send(Cmd::Stop(ack)).is_ok() && done.recv_timeout(wait).is_ok()
            && let Some(t) = thread
        {
            let _ = t.join();
        }
    }
}

/// What Settings › Discord shows.
#[derive(Debug, Serialize)]
pub struct DiscordView {
    /// This build has the feature; without it the section does not exist.
    pub available: bool,
    pub prefs: DiscordPrefs,
    pub status: Status,
    /// A Discord pipe exists on this PC (a listing of the pipe names: nothing is opened or sent).
    pub detected: bool,
}

fn view(state: &AppState, presence: &Presence) -> DiscordView {
    let available = available();
    DiscordView {
        available,
        prefs: state.cfg().discord,
        status: presence.status(),
        detected: available && !ipc::listed(ipc::PIPE_PREFIX).is_empty(),
    }
}

/// The game's settings file gets the same choice (`discord_rpc`) and the same application (`discord_app_id`), for the
/// game started without the launcher (`runtime/discord_rpc.c`). Only a built game's file is written, and only when it
/// differs; a build without the feature never touches it.
pub fn mirror_to_game(state: &AppState) {
    if !available() {
        return;
    }
    let Ok(root) = state.settings_root() else { return };
    if !root.join("build").is_dir() {
        return;
    }
    let path = crate::install::ini_path(&root);
    let on = state.cfg().discord.enabled;
    let _ = write_game_keys(&path, on, CLIENT_ID);
}

fn write_game_keys(path: &std::path::Path, on: bool, client_id: &str) -> AppResult<bool> {
    let mut doc = match crate::util::read_text_limited(path, 4 << 20)? {
        Some(t) => crate::ini::IniDoc::parse(&t),
        None => crate::ini::IniDoc::empty(),
    };
    let want = [("discord_rpc", if on { "1" } else { "0" }), ("discord_app_id", client_id)];
    if want.iter().all(|(k, v)| doc.get(k) == Some(*v)) {
        return Ok(false);
    }
    for (k, v) in want {
        doc.set(k, v, "discord");
    }
    crate::util::atomic_write(path, doc.serialize().as_bytes())?;
    Ok(true)
}

#[tauri::command(async)]
pub fn discord_view(state: tauri::State<AppState>, presence: tauri::State<Presence>) -> DiscordView {
    view(&state, &presence)
}

#[tauri::command(async)]
pub fn discord_set_prefs(state: tauri::State<AppState>, presence: tauri::State<Presence>, prefs: DiscordPrefs) -> AppResult<DiscordView> {
    if !available() {
        return Ok(view(&state, &presence));
    }
    state.update_config(|c| c.discord = prefs.clone())?;
    presence.set_enabled(prefs.enabled);
    mirror_to_game(&state);
    Ok(view(&state, &presence))
}

/// The page's activity for this moment (`None`: nothing). Checked here; sent only while the presence is on.
#[tauri::command]
pub fn discord_publish(presence: tauri::State<Presence>, activity: Option<ActivityInput>) {
    if !available() {
        return;
    }
    presence.publish(activity.and_then(|a| a.check(crate::util::now_secs())));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_without_an_application_id_has_no_presence_at_all() {
        if CLIENT_ID.is_empty() {
            assert!(!available());
            // nothing starts, whatever the page asks
            let p = Presence::new(None, |_| {});
            p.set_enabled(true);
            p.publish(ActivityInput { details: Some("In game".into()), ..Default::default() }.check(activity::tests::NOW));
            assert!(p.lock().tx.is_none());
            assert_eq!(p.status(), Status::OFF);
            p.shutdown(Duration::from_millis(10));
        } else {
            assert!(CLIENT_ID.len() >= 17 && CLIENT_ID.bytes().all(|b| b.is_ascii_digit()));
        }
    }

    #[test]
    fn the_presence_is_off_by_default_and_activity_level_once_on() {
        let p = DiscordPrefs::default();
        assert!(!p.enabled && p.detail == Detail::Activity && !p.mode_stage && !p.server);
        // a config written before the section existed, and a partial one
        let c: DiscordPrefs = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert_eq!(c, DiscordPrefs { enabled: true, ..Default::default() });
        let c: DiscordPrefs = serde_json::from_str(r#"{"detail":"minimal","server":true}"#).unwrap();
        assert!(!c.enabled && c.detail == Detail::Minimal && c.server);
    }

    #[test]
    fn the_games_settings_file_gets_the_same_choice_and_application() {
        let d = std::env::temp_dir().join(format!("s3l-dc-{}", crate::util::random_hex(4)));
        std::fs::create_dir_all(&d).unwrap();
        let ini = d.join("kalamar.ini");
        std::fs::write(&ini, "# mine\r\n[display]\r\nfullscreen = 1\r\n").unwrap();
        assert!(write_game_keys(&ini, true, "100000000000000001").unwrap());
        let text = std::fs::read_to_string(&ini).unwrap();
        assert!(text.starts_with("# mine\r\n[display]\r\nfullscreen = 1\r\n"), "the player's lines stay: {text}");
        let doc = crate::ini::IniDoc::parse(&text);
        assert_eq!((doc.get("discord_rpc"), doc.get("discord_app_id")), (Some("1"), Some("100000000000000001")));
        assert!(!write_game_keys(&ini, true, "100000000000000001").unwrap(), "unchanged: not written again");
        assert!(write_game_keys(&ini, false, "100000000000000001").unwrap());
        assert_eq!(crate::ini::IniDoc::parse(&std::fs::read_to_string(&ini).unwrap()).get("discord_rpc"), Some("0"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[cfg(windows)]
    #[test]
    fn switched_on_late_the_latest_activity_goes_out_and_quitting_clears_it() {
        use fake::{Behaviour, Fake};
        let prefix = format!("kalamar-test-{}-{}-", std::process::id(), crate::util::random_hex(4));
        let fake = Fake::start(&format!("{prefix}0"), Behaviour::Ready);
        let mut o = Options::discord("100000000000000001");
        o.pipe_prefix = prefix;
        o.lock_name = None;
        o.settle = Duration::ZERO;
        o.poll = Duration::from_millis(10);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let p = Presence::new(Some(o), move |s| s2.lock().unwrap().push(s.link));
        let a = |d: &str| ActivityInput { details: Some(d.into()), ..Default::default() }.check(activity::tests::NOW);
        p.publish(a("In the launcher"));
        p.publish(a("At the locker"));
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(fake.connections(), 0, "off: no thread, no pipe");
        assert!(p.lock().tx.is_none());
        p.set_enabled(true);
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while fake.activities().is_empty() && std::time::Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(fake.activities().len(), 1);
        assert_eq!(fake.activities()[0]["details"], "At the locker");
        assert_eq!(p.status().link, worker::Link::Connected);
        p.shutdown(Duration::from_secs(3));
        // the clear was answered before the thread ended: the stand-in has it
        assert_eq!(fake.commands().len(), 2);
        assert!(fake.commands()[1]["args"]["activity"].is_null(), "cleared when the launcher quits");
        assert_eq!(p.status().link, worker::Link::Off);
    }
}
