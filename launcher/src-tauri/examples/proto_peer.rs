//! A headless second launcher for trying the community page against a test server: it logs in as a guest, joins a
//! private room by code (or a queue), gets ready (or accepts the found match), and plays the match with a simulated
//! game (no game is started: it writes the status and result files the way the runtime does, with the same record as
//! the launcher's stand-in executable, so both reports agree). Developer tool; not part of the launcher.
//!
//! ```text
//! cargo run --example proto_peer -- --host 127.0.0.1:8080 --name Bravo --room 7KQ2MX
//! cargo run --example proto_peer -- --host 127.0.0.1:8080 --name Bravo --queue duel
//! cargo run --example proto_peer -- --host 127.0.0.1:8080 --name Bravo --activity bo3 --action find --standin <exe>
//! ```
//!
//! `--standin <exe>`: the game is that executable, started as a process with the match's environment (the launcher's
//! stand-in emulates the engine: capability list, contract 2, rounds); its list is read and declared like the
//! launcher does. Without it, a simulated game in this process (contract 1).
//!
//! `--game-root <root> --userdata <dir>`: the real game (`<root>\build\splat3.exe`) with its own user data folder (never
//! the player's own `userdata`), started the way the launcher starts it (caches, translator and profile folder under
//! `<dir>`, `--profile <dir>\profile`), windowed. `--play <s>`: the engine's test aid `SPLAT3_MATCH_PLAY` (the player
//! inks for that many seconds). `--dir <folder>`: this peer's data folder (kept at the end).
//!
//! `--loadout-rules <json | @file>`: loadout constraints handled as if the match carried them (`rules.loadout`), for a server
//! that imposes none; `--pick weapon=<id>,gear.head=<id>`: this player's current items for a choice.
//!
//! Stage drafts (1.1.6): `--stages <scene,scene,...>` makes the simulated game list these stages (so the server can draft
//! them; the record then names the match's stage); during a draft the peer acts when its turn comes, after
//! `--draft-delay <s>` (default 2), choosing the first stages of `--draft-prefer <scene,...>` still available, else the
//! first available ones; `--draft-idle` never acts (the server's time-outs play for it).
//!
//! Spectators (1.1.7, `spectators.md`): `--spectators` makes the simulated game an engine with places for spectators
//! (contract 2, result files of version 2: a spectator's says `spectated`), so the peer declares the seat;
//! `--create-room <options json>` creates a room (its code goes to `--code-file <path>`) and starts it once
//! `--start-when <n>` members are in and ready; `--seat spectator` joins a room's stands, `--move-to spectator` joins a
//! team then moves into the stands (and `--move-to alpha|bravo` back); `--watch` takes the first place
//! `GET /v1/spectate` offers; `--fake-win <0|1>` makes the simulated game record that winner (a disagreement, a
//! witness's report); `--live <file>` gives the real game a live input file (`SHOT <png>` lines, for screenshots).
//!
//! `--relay-via <ipv4:port>`: the real game reaches the relay through that address (a test's UDP forwarder, to cut one
//! player's network without stopping its game). `--game-exe <path>`: another build than `<root>\build\splat3.exe`.
//! `--dev-login`: the server's developer login under `--name` (the same account when the peer starts again).
//! `--probe-queue <s>` (with `--queue`): from `s` seconds on, every 3 s, tries to search the queue again and prints each
//! change of the answer (in a match, a cooldown, free), then waits for it before exiting.

use kalamar_lib::proto::glue::{self, AddInput, Discovered};
use kalamar_lib::proto::http::Body;
use kalamar_lib::proto::runner::{GameHost, GameRun};
use kalamar_lib::proto::{Hub, Sink};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

struct Printer {
    matches: Arc<Mutex<Vec<Value>>>,
    tx: tokio::sync::mpsc::UnboundedSender<Value>,
}

impl Sink for Printer {
    fn emit(&self, channel: &str, p: Value) {
        match channel {
            "proto-event" => {
                let kind = p["type"].as_str().unwrap_or("");
                let d = &p["data"];
                let what = match kind {
                    k if k.starts_with("match.") => format!("{} {}", d["id"].as_str().unwrap_or(""), d["state"].as_str().unwrap_or("")),
                    "room.updated" => format!("{} members, state {}", d["members"].as_array().map(|m| m.len()).unwrap_or(0), d["state"].as_str().unwrap_or("")),
                    _ => String::new(),
                };
                println!("[event] {} {kind} {what}", now_hms());
                if kind.starts_with("match.") {
                    self.matches.lock().unwrap().push(d.clone());
                }
                let _ = self.tx.send(p);
            }
            "proto-runner" => println!("[runner] {} {} {} {}", now_hms(), p["phase"].as_str().unwrap_or(""), p["game_state"].as_str().unwrap_or(""), p["code"].as_str().unwrap_or("")),
            "proto-stream" => println!("[stream] {}", p["state"].as_str().unwrap_or("")),
            _ => println!("[{channel}] {p}"),
        }
    }
}

/// The simulated game: the same states and the same record as the launcher's stand-in executable.
struct Game {
    run: Arc<Mutex<GameRun>>,
    matches: Arc<Mutex<Vec<Value>>>,
}

fn var(env: &[(String, String)], k: &str) -> String {
    env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default()
}

fn write(path: &str, text: &str) {
    let tmp = format!("{path}.tmp");
    let _ = std::fs::write(&tmp, text);
    let _ = std::fs::rename(&tmp, path);
}

impl GameHost for Game {
    fn launch(&self, env: Vec<(String, String)>) -> Result<(), String> {
        println!("[game] simulated launch:");
        for (k, v) in &env {
            println!("        {k}={v}");
        }
        *self.run.lock().unwrap() = GameRun { running: true, ..Default::default() };
        let (run, matches) = (self.run.clone(), self.matches.clone());
        std::thread::spawn(move || {
            let (id, role, me, host) = (var(&env, "SPLAT3_MATCH_ID"), var(&env, "SPLAT3_MATCH"), var(&env, "SPLAT3_NICKNAME"), var(&env, "SPLAT3_MATCH_HOST"));
            // Contract 2 paths carry `{id}` (the engine puts the round's match id there).
        let (status, result) = (var(&env, "SPLAT3_MATCH_STATUS").replace("{id}", &id), var(&env, "SPLAT3_MATCH_RESULT").replace("{id}", &id));
            let set = |state: &str| {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                write(&status, &json!({"version": 1, "match_id": id, "role": role, "state": state, "since_unix": now, "visible": state == "in-match", "pid": std::process::id()}).to_string());
                std::thread::sleep(Duration::from_millis(1300));
            };
            let steps: &[&str] = if role == "host" { &["booting", "lobby", "creating-room", "waiting-for-players", "in-room", "loading-stage", "in-match"] } else { &["booting", "lobby", "joining", "in-room", "loading-stage", "in-match"] };
            for s in steps {
                set(s);
            }
            std::thread::sleep(Duration::from_secs(2));
            // The players and their teams: from the rules when the game got them (contract 2: every participant; the
            // letter S seats a spectator), else in arrival order, the host first (the stand-in's order).
            let rules = var(&env, "SPLAT3_MATCH_RULES");
            let teams: Vec<(String, String)> = rules
                .split(',')
                .filter_map(|e| e.strip_prefix("teams=").unwrap_or(e).rsplit_once(':'))
                .filter(|(n, l)| !n.contains('=') && matches!(*l, "A" | "B" | "S"))
                .map(|(n, l)| (n.to_string(), l.to_string()))
                .collect();
            let spectator = teams.iter().any(|(n, l)| *n == me && l == "S");
            let list: Vec<(String, &str)> = if teams.is_empty() {
                let others: Vec<String> = matches
                    .lock()
                    .unwrap()
                    .iter()
                    .rev()
                    .find(|m| m["game"]["match_id"] == id.as_str())
                    .and_then(|m| m["participants"].as_array().cloned())
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|p| p["member_name"].as_str().map(str::to_string))
                    .collect();
                let host_name = if role == "host" { me.clone() } else { host.clone() };
                let mut list = vec![host_name.clone()];
                list.extend(others.into_iter().filter(|n| *n != host_name));
                list.into_iter().enumerate().map(|(i, n)| (n, if i % 2 == 0 { "alpha" } else { "bravo" })).collect()
            } else {
                teams.iter().filter(|(_, l)| l != "S").map(|(n, l)| (n.clone(), if l == "A" { "alpha" } else { "bravo" })).collect()
            };
            let (mut pa, mut pb) = (0u32, 0u32);
            let players: Vec<Value> = list
                .iter()
                .enumerate()
                .map(|(i, (name, team))| {
                    let paint = 210 - (i as u32) * 23 + (name.len() as u32 % 5) * 7;
                    if *team == "alpha" { pa += paint } else { pb += paint }
                    json!({"name": name, "self": *name == me, "team": team, "paint": paint, "kills": (i * 3 + 2) % 7, "assists": i % 3, "deaths": (i * 2 + 1) % 5, "specials": i % 2, "disconnected": false})
                })
                .collect();
            let total = (pa + pb).max(1) as f64;
            let (ta, tb) = ((pa as f64 / total * 520.0) as u32, (pb as f64 / total * 520.0) as u32);
            let win: u32 = arg("--fake-win").and_then(|w| w.parse().ok()).filter(|w| *w <= 1).unwrap_or(if pa >= pb { 0 } else { 1 });
            set("results");
            // The match's stage when it has one (a drafted stage), else the stand-in's.
            let stage_scene = matches.lock().unwrap().iter().rev().find(|m| m["game"]["match_id"] == id.as_str()).and_then(|m| m["rules"]["stage"].as_str().map(str::to_string)).unwrap_or_else(|| "Stage_A".into());
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let mut file = json!({
                "version": 1, "match_id": id, "role": role, "rule": 0, "mode": "turf", "stage_id": 11, "stage_scene": stage_scene,
                "end_utc": now, "battle_frames": 10800, "duration_s": 180.0, "win_team": win, "winner": if win == 0 { "alpha" } else { "bravo" }, "deemed_result": -1,
                "teams": [{"team": "alpha", "paint_points": pa, "turf_permille": ta}, {"team": "bravo", "paint_points": pb, "turf_permille": tb}, {"team": "charlie", "paint_points": 0, "turf_permille": 0}],
                "players": players, "t0_unix": now as f64, "timeline_s": [["lobby loading", 7.9]]
            });
            // An engine with spectators writes version 2 files: a spectator's says `spectated` (a witness's).
            if std::env::args().any(|a| a == "--spectators") {
                file["version"] = json!(2);
                file["round"] = json!(1);
                file["outcome"] = json!(if spectator { "spectated" } else { "completed" });
                file["seat"] = json!(if spectator { "spectator" } else { "player" });
                file["seat_asked"] = file["seat"].clone();
                if role == "host" {
                    file["settings_ok"] = json!(true);
                }
            }
            write(&result, &serde_json::to_string_pretty(&file).unwrap());
            println!("[game] result written");
            set("finished");
            *run.lock().unwrap() = GameRun { running: false, exited: true, exit_code: Some(0), stopped_by_user: false };
        });
        Ok(())
    }
    fn status(&self) -> GameRun {
        self.run.lock().unwrap().clone()
    }
    fn stop(&self) {
        *self.run.lock().unwrap() = GameRun { running: false, exited: true, exit_code: Some(1), stopped_by_user: true };
    }
    fn build_id(&self) -> String {
        "proto-peer".into()
    }
    /// `--stages`: a made-up list with these scenes, every one loadable (for drafts).
    fn engine(&self) -> Option<kalamar_lib::proto::engine::EngineInfo> {
        let watch = std::env::args().any(|a| a == "--spectators");
        let stages: Vec<Value> = match arg("--stages") {
            Some(s) => s.split(',').filter(|s| !s.is_empty()).enumerate().map(|(i, s)| json!({"stage": i + 1, "scene": s, "label": s, "in_stage_lists": true})).collect(),
            None if watch => Vec::new(),
            None => return None,
        };
        let mut options = json!({"format": "splat3-match-options", "version": 1, "modes": [{"key": "turf", "label": "Mode A"}], "stages": stages, "stage_list": "all", "teams": {"places_per_team": 4, "players_max": 8, "uneven": true}, "result": {"version": 2, "outcomes": ["completed", "aborted"]}});
        if watch {
            // The engine's `spectators` member as dc39dfe lists it (two places, the wait as a rule key).
            options["teams"]["spectators"] = json!(true);
            options["spectators"] = json!({"max": 2, "team": "S", "in_players": true, "session_max": 10, "modes": ["turf"], "verified_max": 2,
                "verified_modes": ["turf"], "host_verified": true, "seat": ["status", "result"], "result_outcome": "spectated",
                "wait": {"key": "spectators_wait", "min": 0, "max": 600, "default": 60, "needs": "teams_by_name"}});
            options["result"]["outcomes"] = json!(["completed", "aborted", "spectated"]);
        }
        Some(kalamar_lib::proto::engine::EngineInfo {
            fingerprint: "proto-peer".into(),
            options: Some(options),
            room_stages: None,
            note: None,
            lang: None,
        })
    }
}

/// During a draft (1.1.6), the choice of this peer for the current turn of `m`, when it may act.
fn draft_choice(m: &Value) -> Option<(u64, Vec<String>)> {
    let d = &m["draft"];
    if m["state"] != "drafting" || d["you"]["can_act"] != true || std::env::args().any(|a| a == "--draft-idle") {
        return None;
    }
    let turn = d["current"].as_u64()?;
    let t = &d["turns"][turn as usize];
    let n = t["count"].as_u64().unwrap_or(1) as usize;
    let avail: Vec<String> = d["options"].as_array()?.iter().filter(|o| o["kind"] == t["kind"] && o["state"] == "available").filter_map(|o| o["id"].as_str().map(str::to_string)).collect();
    let prefer: Vec<String> = arg("--draft-prefer").map(|s| s.split(',').map(str::to_string).collect()).unwrap_or_default();
    let mut ids: Vec<String> = prefer.into_iter().filter(|p| avail.contains(p)).take(n).collect();
    for a in &avail {
        if ids.len() >= n {
            break;
        }
        if !ids.contains(a) {
            ids.push(a.clone());
        }
    }
    Some((turn, ids))
}

/// The game as a process (the stand-in executable, or the real game with `real`).
struct Proc {
    exe: std::path::PathBuf,
    dir: std::path::PathBuf,
    child: Mutex<Option<std::process::Child>>,
    real: Option<RealGame>,
}

/// The real game: its user data folder and the launcher's usual variables (`launch::build_launch_spec`).
struct RealGame {
    root: std::path::PathBuf,
    userdata: std::path::PathBuf,
    play: Option<String>,
}

fn now_hms() -> String {
    chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}

impl GameHost for Proc {
    fn launch(&self, env: Vec<(String, String)>) -> Result<(), String> {
        println!("[game] {} process launch:", now_hms());
        for (k, v) in &env {
            println!("        {k}={v}");
        }
        let c = match &self.real {
            None => {
                let log = std::fs::File::create(self.dir.join("standin.log")).map_err(|e| e.to_string())?;
                std::process::Command::new(&self.exe).envs(env).env("STANDIN_STEP_MS", "400").current_dir(&self.dir).stdout(std::process::Stdio::null()).stderr(log).spawn().map_err(|e| e.to_string())?
            }
            Some(g) => {
                let u = &g.userdata;
                std::fs::create_dir_all(u.join("logs")).map_err(|e| e.to_string())?;
                std::fs::create_dir_all(u.join("profile")).map_err(|e| e.to_string())?;
                let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
                let log = u.join("logs").join(format!("run-{stamp}.log"));
                let err = std::fs::File::create(&log).map_err(|e| e.to_string())?;
                let out = std::fs::File::create(log.with_extension("log.out")).map_err(|e| e.to_string())?;
                let xlate = g.root.parent().unwrap_or(&g.root).join("xlate").join("totk_xlate.exe");
                let mut cmd = std::process::Command::new(&self.exe);
                cmd.arg("--profile").arg(u.join("profile")).current_dir(u).stdin(std::process::Stdio::null()).stdout(out).stderr(err);
                cmd.env("TOTK_SHADER_CACHE", u.join("shader-cache"))
                    .env("TOTK_PIPELINE_CACHE", u.join("pipeline-cache.bin"))
                    .env("TOTK_ROMFS_PROOF_DIR", u)
                    .env("TOTK_XLATE", xlate)
                    .env("TOTK_PROFILE_DIR", u.join("profile"))
                    .env("TOTK_FULLSCREEN", "0")
                    .env("TOTK_WINDOW_SIZE", "1280x720")
                    .env("TOTK_OUTPUT_RES", "1280x720")
                    .env("TOTK_VOLUME", "5");
                for k in ["TOTK_SAVE_PROFILE", "TOTK_INPUT_SCRIPT", "TOTK_INPUT_LIVE", "SPLAT3_MATCH_PLAY"] {
                    cmd.env_remove(k);
                }
                if let Some(p) = &g.play {
                    cmd.env("SPLAT3_MATCH_PLAY", p);
                }
                // `--live <file>`: the runtime's live input file (`SHOT <png>` lines take screenshots of the window).
                if let Some(live) = arg("--live") {
                    let _ = std::fs::write(&live, b"");
                    cmd.env("TOTK_INPUT_LIVE", live);
                }
                cmd.envs(env);
                // `--relay-via <ipv4:port>`: this game reaches the match's relay through that address instead (a UDP
                // forwarder of the test, which can stop forwarding to cut this player's network without a crash).
                if let Some(via) = arg("--relay-via") {
                    cmd.env("SPLAT3_LAN", format!("relay:{via}"));
                    println!("[game] relay through {via}");
                }
                let c = cmd.spawn().map_err(|e| e.to_string())?;
                println!("[game] pid {} log {}", c.id(), log.display());
                c
            }
        };
        *self.child.lock().unwrap() = Some(c);
        Ok(())
    }
    fn status(&self) -> GameRun {
        match self.child.lock().unwrap().as_mut().map(|c| c.try_wait()) {
            Some(Ok(None)) => GameRun { running: true, ..Default::default() },
            Some(Ok(Some(s))) => GameRun { running: false, exited: true, exit_code: s.code(), stopped_by_user: false },
            _ => GameRun { running: false, exited: true, ..Default::default() },
        }
    }
    fn stop(&self) {
        if let Some(c) = self.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
    }
    fn build_id(&self) -> String {
        kalamar_lib::proto::engine::fingerprint(&self.exe).unwrap_or_default()
    }
    fn engine(&self) -> Option<kalamar_lib::proto::engine::EngineInfo> {
        let cwd = self.real.as_ref().map(|g| g.userdata.clone()).unwrap_or_else(|| self.dir.clone());
        Some(kalamar_lib::proto::engine::engine_info(&self.dir, &self.exe, &cwd))
    }
    fn data_dir(&self) -> Option<std::path::PathBuf> {
        Some(self.dir.clone())
    }
    fn declare_rounds(&self) -> bool {
        std::env::args().any(|a| a == "--rounds")
    }
    fn local_loadout_rules(&self) -> Option<Value> {
        arg("--loadout-rules").map(|s| s.strip_prefix('@').and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or(s)).and_then(|s| serde_json::from_str(&s).ok())
    }
    fn current_loadout(&self) -> std::collections::HashMap<String, String> {
        // `--pick slot=id,slot=id`: the items this player "has", so a choice left to the player resolves at once.
        arg("--pick").map(|s| s.split(',').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect()).unwrap_or_default()
    }
}

fn arg(name: &str) -> Option<String> {
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        if a == name {
            return it.next();
        }
    }
    None
}

#[tokio::main]
async fn main() {
    let host = arg("--host").unwrap_or_else(|| "127.0.0.1:8080".into());
    let name = arg("--name").unwrap_or_else(|| "Bravo".into());
    let room = arg("--room");
    let queue = arg("--queue");
    let rounds: usize = arg("--matches").and_then(|n| n.parse().ok()).unwrap_or(1);
    let keep_dir = arg("--dir");
    let dir = keep_dir.clone().map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join(format!("kalamar-peer-{}", std::process::id())));
    std::fs::create_dir_all(&dir).unwrap();
    let preview = match glue::discover(&host).await {
        Ok(Discovered::Service { preview }) => preview,
        other => panic!("no community service at {host}: {other:?}"),
    };
    println!("server: {} ({}), features {:?}", preview.name, preview.api, preview.features);
    let id = glue::add(&dir, AddInput { name: preview.name.clone(), api: preview.api.clone(), relay: preview.relay.clone() }).await.unwrap_or_else(|e| panic!("{e:?}"));
    let matches = Arc::new(Mutex::new(Vec::new()));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let game: Arc<dyn GameHost> = match (arg("--standin"), arg("--game-root"), arg("--userdata")) {
        (_, Some(root), Some(ud)) => {
            let root = std::path::PathBuf::from(root);
            let userdata = std::path::PathBuf::from(ud);
            assert!(!userdata.ends_with("userdata"), "userdata\\ is the player's own folder: give the peer its own");
            // `--game-exe <path>`: another build of the game than `<root>\build\splat3.exe`.
            let exe = arg("--game-exe").map(std::path::PathBuf::from).unwrap_or_else(|| root.join("build").join("splat3.exe"));
            Arc::new(Proc { exe, dir: dir.clone(), child: Mutex::new(None), real: Some(RealGame { root, userdata, play: arg("--play") }) })
        }
        (Some(exe), _, _) => Arc::new(Proc { exe: exe.into(), dir: dir.clone(), child: Mutex::new(None), real: None }),
        _ => Arc::new(Game { run: Arc::new(Mutex::new(GameRun::default())), matches: matches.clone() }),
    };
    let hub = Hub::with_system_resolver(dir.clone(), Arc::new(Printer { matches: matches.clone(), tx }), game.clone());
    let conn = hub.conn(&id, "en").unwrap();
    conn.client.fetch_meta().await.unwrap();
    // `--dev-login`: the server's developer login with the name as user name, so a peer started again is the same
    // account (a guest login makes a new one each time).
    let me = if std::env::args().any(|a| a == "--dev-login") {
        conn.client.login_direct("dev", json!({"username": name})).await.unwrap()
    } else {
        conn.client.login_direct("guest", json!({"display_name": name})).await.unwrap()
    };
    println!("logged in as {} ({})", me["display_name"], me["id"]);
    conn.relays.declare().await.unwrap();
    hub.ensure_stream(&id, &conn);
    let call = |m: &'static str, p: String, b: Value| {
        let c = conn.client.clone();
        async move { c.call(m, &p, if b.is_null() { Body::None } else { Body::Json(b) }, true).await }
    };
    let mut room_id = None;
    let team_of = |body: &Value| body["members"].as_array().and_then(|m| m.iter().find(|x| x["user"]["id"] == me["id"])).map(|x| x["team"].clone()).unwrap_or(Value::Null);
    // 1.1.7: `--create-room <options>`: this peer owns a room (its code for the others in `--code-file`).
    let owner = arg("--create-room");
    if let Some(opts) = &owner {
        // `@file`: the options in a file (a command line may strip the quotes of a JSON value).
        let text = opts.strip_prefix('@').map(|p| std::fs::read_to_string(p).unwrap_or_else(|e| panic!("--create-room {p}: {e}"))).unwrap_or_else(|| opts.clone());
        let options: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("--create-room: {e}"));
        let r = call("POST", "/v1/rooms".into(), json!({"options": options})).await.unwrap_or_else(|e| panic!("create: {}", e.message()));
        room_id = r.body["id"].as_str().map(str::to_string);
        let code = r.body["code"].as_str().unwrap_or_default().to_string();
        println!("created room {code}: options {} spectators {}", r.body["options"], r.body["spectators"]);
        if let Some(f) = arg("--code-file") {
            std::fs::write(f, &code).unwrap();
        }
    }
    if let Some(code) = &room {
        let seat = arg("--seat");
        let mut body = json!({"code": code});
        if let Some(s) = &seat {
            body["seat"] = json!(s);
        }
        let r = call("POST", "/v1/rooms/join".into(), body).await.unwrap_or_else(|e| panic!("join: {} {}", e.code(), e.message()));
        room_id = r.body["id"].as_str().map(str::to_string);
        println!("joined room {} as {}", code, team_of(&r.body));
        // `--move-to <team>`: a team change after joining (into the stands and back).
        if let Some(t) = arg("--move-to") {
            let path = format!("/v1/rooms/{}/members/{}", room_id.as_deref().unwrap_or(""), me["id"].as_str().unwrap_or(""));
            match call("PUT", path, json!({"team": t})).await {
                Ok(r) => println!("moved to {} (room spectators {})", team_of(&r.body), r.body["spectators"]),
                Err(e) => println!("move to {t} refused: {} {} {}", e.status(), e.code(), e.message()),
            }
        }
        call("POST", format!("/v1/rooms/{}/ready", room_id.as_deref().unwrap_or("")), json!({"ready": true})).await.unwrap();
        println!("ready");
    }
    // The owner starts once `--start-when` members are in and ready (players and spectators).
    if let (Some(_), Some(rid)) = (&owner, room_id.clone()) {
        let want: usize = arg("--start-when").and_then(|n| n.parse().ok()).unwrap_or(2);
        loop {
            tokio::time::sleep(Duration::from_millis(700)).await;
            let Ok(r) = call("GET", format!("/v1/rooms/{rid}"), Value::Null).await else { continue };
            let members = r.body["members"].as_array().cloned().unwrap_or_default();
            if members.len() >= want && members.iter().all(|m| m["ready"] == true || m["user"]["id"] == me["id"]) {
                let teams: Vec<String> = members.iter().map(|m| format!("{}:{}", m["user"]["display_name"].as_str().unwrap_or("?"), m["team"].as_str().unwrap_or("?"))).collect();
                println!("starting the room: {}", teams.join(", "));
                match call("POST", format!("/v1/rooms/{rid}/start"), json!({})).await {
                    Ok(m) => println!("started {}: game rules {}, players {}, spectators {}", m.body["id"], m.body["game"]["rules"], m.body["game"]["players"], m.body["spectators"]),
                    Err(e) => println!("start refused: {} {} {}", e.status(), e.code(), e.message()),
                }
                break;
            }
        }
    }
    // `--watch`: the first place the list of watchable matches offers.
    if std::env::args().any(|a| a == "--watch") {
        for _ in 0..120 {
            if let Ok(l) = call("GET", "/v1/spectate".into(), Value::Null).await
                && let Some(item) = l.body["items"].as_array().and_then(|a| a.iter().find(|i| i["can_take"] == true)).cloned()
            {
                let path = match item["kind"].as_str() {
                    Some("series") => format!("/v1/series/{}/spectate", item["series_id"].as_str().unwrap_or("")),
                    _ => format!("/v1/matches/{}/spectate", item["match_id"].as_str().unwrap_or("")),
                };
                match call("POST", path.clone(), json!({})).await {
                    Ok(_) => println!("took a place: {path}"),
                    Err(e) => println!("place refused: {} {} {}", e.status(), e.code(), e.message()),
                }
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    if let Some(act) = arg("--activity") {
        let action = arg("--action").unwrap_or_else(|| "find".into());
        let view = call("GET", format!("/v1/activities/{act}"), Value::Null).await;
        let rev = view.as_ref().map(|r| r.body["revision"].clone()).unwrap_or(Value::Null);
        match call("POST", format!("/v1/activities/{act}/actions/{action}"), json!({"values": {}, "revision": rev})).await {
            Ok(r) => println!("activity {act}: {action} -> {}", r.body["notice"]),
            Err(e) => println!("activity {act}: {action} failed: {}", e.message()),
        }
    }
    if let Some(q) = &queue {
        call("POST", format!("/v1/queues/{q}/tickets"), json!({})).await.unwrap_or_else(|e| panic!("queue: {}", e.message()));
        println!("searching in {q}");
    }
    // `--probe-queue <s>`: from that many seconds after the start, every 3 s, try to search the queue again (a ticket,
    // cancelled at once when it is taken): when a player can search again after a match, and what refuses it before
    // (in_match, queue_cooldown). Each change of the answer is printed.
    let mut probe = None;
    if let (Some(q), Some(after)) = (queue.clone(), arg("--probe-queue").and_then(|s| s.parse::<u64>().ok())) {
        let c = conn.client.clone();
        probe = Some(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(after)).await;
            let mut last = String::new();
            loop {
                let r = c.call("POST", &format!("/v1/queues/{q}/tickets"), Body::Json(json!({})), true).await;
                let what = match &r {
                    Ok(_) => "201 searching".to_string(),
                    Err(e) => format!("{} {} {}", e.status(), e.code(), e.message()),
                };
                if what != last {
                    println!("[probe] {} {what}", now_hms());
                    last = what;
                }
                if r.is_ok() {
                    let _ = c.call("DELETE", "/v1/tickets/current", Body::None, true).await;
                    println!("[probe] {} free to search again (ticket cancelled)", now_hms());
                    return;
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }));
    }
    let mut decided = 0;
    while let Some(ev) = rx.recv().await {
        let kind = ev["type"].as_str().unwrap_or("");
        let d = &ev["data"];
        if kind == "match.found" {
            let mid = d["id"].as_str().unwrap_or_default().to_string();
            match call("POST", format!("/v1/matches/{mid}/accept"), json!({})).await {
                Ok(_) => println!("accepted {mid}"),
                Err(e) => println!("accept failed: {}", e.message()),
            }
        }
        if kind.starts_with("match.") && d["draft"].is_object() {
            let dr = &d["draft"];
            if let Some(t) = dr["current"].as_u64().map(|i| &dr["turns"][i as usize]) {
                println!("[draft] turn {}: {} {} x{} by {} (you: {})", dr["current"], t["action"], t["kind"], t["count"], t["teams"], dr["you"]["role"]);
            }
            if kind == "match.ready" {
                println!("[draft] done: stage {}, game rules {}", d["rules"]["stage"], d["game"]["rules"]);
            }
        }
        if let Some((turn, ids)) = draft_choice(d) {
            let mid = d["id"].as_str().unwrap_or_default().to_string();
            let delay: u64 = arg("--draft-delay").and_then(|s| s.parse().ok()).unwrap_or(2);
            tokio::time::sleep(Duration::from_secs(delay)).await;
            match call("POST", format!("/v1/matches/{mid}/draft"), json!({"turn": turn, "choices": ids})).await {
                Ok(_) => println!("[draft] chose {ids:?} on turn {turn}"),
                Err(e) => println!("[draft] {} {}: {}", e.status(), e.code(), e.message()),
            }
        }
        if kind == "series.updated" && d["state"].as_str().is_some_and(|s| s != "running") {
            println!("series {} over: {}", d["id"], serde_json::to_string(d).unwrap_or_default());
            break;
        }
        if kind == "match.completed" || kind == "match.void" {
            println!("decision: {}", serde_json::to_string(&d["outcome"]).unwrap_or_default());
            if d["series"].is_object() {
                continue; // a series: the series event ends the run
            }
            decided += 1;
            if decided >= rounds {
                break;
            }
            if let Some(r) = &room_id {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let _ = call("POST", format!("/v1/rooms/{r}/ready"), json!({"ready": true})).await;
                println!("ready again");
            }
        }
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    // The queue probe goes on until the player may search again (a cooldown: at most an hour).
    if let Some(p) = probe {
        let _ = tokio::time::timeout(Duration::from_secs(3600), p).await;
    }
    if let Some(r) = &room_id {
        let _ = call("POST", format!("/v1/rooms/{r}/leave"), json!({})).await;
    }
    conn.stop_stream();
    if keep_dir.is_none() {
        conn.client.logout(false).await;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
