// The Discord presence's two steps (presence.ts) on made-up states: what each situation becomes at each level, and,
// for every situation and every combination of the player's choices, that nothing private reaches the activity: not
// another player's name, not a room code, password, user, room or match id, not a relay or service address.
import test from "node:test";
import assert from "node:assert/strict";
import { DEFAULT_PREFS, ICONS, RESULT_S, buildActivity, sampleSituations, situationOf } from "./presence.ts";
import { GLYPHS } from "./art.ts";
import { app as fr } from "../locales/fr/app.ts";
import { app as en } from "../locales/en/app.ts";

const NOW = 1_794_346_807;
const iso = (s) => new Date(s * 1000).toISOString();
const fill = (dict) => (k, v) => {
  let s = dict[k];
  assert.ok(typeof s === "string", `missing text ${k}`);
  for (const [n, x] of Object.entries(v ?? {})) s = s.split(`{${n}}`).join(String(x));
  return s;
};
const EN = fill(en);
// Every variable value shows in the output: a value passed to a text can never hide from the leak test.
const ECHO = (k, v) => [k, ...Object.values(v ?? {})].join("|");

// ---- the made-up live state: everything a server sends, with private values everywhere --------------------------------
const PRIVATE = [
  "Lou", "Zed", "Mika", "Camille", "u_me", "u_b", "u_c", "u_d", "7KQ2MX", "r_42", "m_9x2k", "p_77", "t_55", "hunter2",
  "relay.example.org", "203.0.113.7", "47313", "K7Q2MXA9PL4DZB3W8R6TNY5HCF", "play.example.org", "Vss_Docks00", "turf",
  "duel-ranked",
];
const user = (id, name) => ({ id, display_name: name });
const room = (extra = {}) => ({
  id: "r_42", code: "7KQ2MX", owner: "u_b", has_password: true, password: "hunter2", state: "open",
  options: { teams: [2, 2], mode: "turf", stage: "Vss_Docks00", password: "hunter2" },
  members: [
    { user: user("u_b", "Lou"), team: "alpha", ready: true, joined_at: iso(NOW - 600) },
    { user: user("u_me", "Camille"), team: "bravo", ready: false, joined_at: iso(NOW - 130) },
    { user: user("u_c", "Zed"), team: "alpha", ready: true, joined_at: iso(NOW - 500) },
    { user: user("u_d", "Mika"), team: "spectator", ready: false, joined_at: iso(NOW - 90) },
  ],
  match_id: null, created_at: iso(NOW - 700), ...extra,
});
const party = { id: "p_77", leader: "u_b", members: [{ user: user("u_b", "Lou") }, { user: user("u_me", "Camille") }], invites: [{ user: user("u_c", "Zed") }], max_size: 4 };
const ticket = (state = "searching") => ({ id: "t_55", queue_id: "duel-ranked", party_id: "p_77", members: [user("u_b", "Lou"), user("u_me", "Camille")], created_at: iso(NOW - 95), state, match_id: state === "match_found" ? "m_9x2k" : null });
const match = (state, you = { user_id: "u_me", member_name: "Camille", team: "bravo", role: "join" }, extra = {}) => ({
  id: "m_9x2k", state, origin: { kind: "queue", queue_id: "duel-ranked" },
  rules: { teams: [4, 4], mode: "turf", stage: "Vss_Docks00", rated: true, ladder: "duel" },
  participants: [
    { user: user("u_b", "Lou"), member_name: "Lou", team: "alpha", role: "host" },
    { user: user("u_me", "Camille"), member_name: "Camille", team: "bravo", role: "join" },
  ],
  spectators: [{ user: user("u_d", "Mika"), member_name: "Mika" }],
  you,
  game: { match_id: "K7Q2MXA9PL4DZB3W8R6TNY5HCF", relay: { host: "relay.example.org", port: 47313, room: "K7Q2MXA9PL4DZB3W8R6TNY5HCF" }, rules: "players=2,teams=Camille:A,Lou:B,mode=turf", host_name: "Lou" },
  created_at: iso(NOW - 300), ended_at: null, outcome: null, ...extra,
});
const queues = { "duel-ranked": { rated: true, teams: [4, 4] } };

const NAMES = {
  modeName: (id) => (id === "turf" ? "Area Rush" : null),
  stageName: (id) => (id === "Vss_Docks00" ? "The Docks" : null),
};
const idle = { state: "idle", mode: null, started_at: null, restarted: false };
const running = (mode, at = NOW - 70) => ({ state: "running", mode, started_at: at, restarted: false });
const facts = (over = {}) => ({
  page: "play", game: idle, launcherSince: NOW - 3600, onlineSince: null, server: null, live: null, now: NOW, ...NAMES, ...over,
});
const online = (live, over = {}) => facts({ server: "Open Ink", onlineSince: NOW - 1000, live: { me: "u_me", room: null, ticket: null, party: null, match: null, ended: null, queues, ...live }, ...over });

/** One made-up moment per kind of situation. */
const CASES = {
  launcher: facts(),
  locker: facts({ page: "profile" }),
  offline: facts({ game: running("original") }),
  training: facts({ game: running("range") }),
  lan: facts({ game: running("lan") }),
  private: facts({ game: running("match") }),
  online: online({ party }),
  room: online({ room: room(), party }),
  queue: online({ ticket: ticket(), party }),
  found: online({ ticket: ticket("match_found"), match: match("pending_accept"), party }),
  draft: online({ match: match("drafting") }),
  match: online({ match: match("running"), room: room({ state: "in_match", match_id: "m_9x2k" }) }, { game: running("match") }),
  watch: online({ match: match("running", { user_id: "u_me", member_name: "Camille", team: "spectator", seat: "spectator" }) }, { game: running("match") }),
  result: online({ ended: match("completed", undefined, { ended_at: iso(NOW - 30), outcome: { counted: true, winner: "bravo", record: { players: [{ name: "Lou" }] } } }) }),
};

const LEVELS = [];
for (const detail of ["minimal", "activity"]) for (const mode_stage of [false, true]) for (const server of [false, true]) LEVELS.push({ enabled: true, detail, mode_stage, server });

test("each made-up moment is the situation it stands for", () => {
  for (const [kind, f] of Object.entries(CASES)) assert.equal(situationOf(f).kind, kind, kind);
});

test("nothing private reaches the activity, at any level, in any situation", () => {
  for (const [kind, f] of Object.entries(CASES)) {
    const s = situationOf(f);
    for (const p of LEVELS) {
      for (const tr of [EN, ECHO]) {
        const out = JSON.stringify(buildActivity(s, p, tr, "Kalamar"));
        for (const bad of PRIVATE) assert.ok(!out.includes(bad), `${kind} ${JSON.stringify(p)}: "${bad}" in ${out}`);
        if (!p.server || p.detail === "minimal") assert.ok(!out.includes("Open Ink"), `${kind}: the server's name without the option: ${out}`);
        if (!p.mode_stage || p.detail === "minimal") assert.ok(!out.includes("Area Rush") && !out.includes("The Docks"), `${kind}: mode or stage without the option: ${out}`);
      }
    }
  }
});

test("the situation itself holds no name, code, id or address", () => {
  for (const [kind, f] of Object.entries(CASES)) {
    const s = JSON.stringify(situationOf(f));
    for (const bad of PRIVATE) assert.ok(!s.includes(bad), `${kind}: "${bad}" in ${s}`);
  }
});

const act = (f, p = {}) => buildActivity(situationOf(f), { ...DEFAULT_PREFS, enabled: true, ...p }, EN, "Kalamar");

test("minimal: in game or in the launcher, and since when", () => {
  assert.deepEqual(act(CASES.launcher, { detail: "minimal" }), { details: "In the launcher", large_text: "Kalamar", start: NOW - 3600 });
  assert.deepEqual(act(CASES.room, { detail: "minimal", mode_stage: true, server: true }), { details: "In the launcher", large_text: "Kalamar", start: NOW - 3600 });
  assert.deepEqual(act(CASES.match, { detail: "minimal", mode_stage: true, server: true }), { details: "In game", large_text: "Kalamar", start: NOW - 70 });
  assert.deepEqual(act(CASES.training, { detail: "minimal" }), { details: "In game", large_text: "Kalamar", start: NOW - 70 });
});

test("activity: the exact words of each situation", () => {
  const two = (f, p) => { const a = act(f, p); return [a.details, a.state ?? null, a.party ?? null, a.start ?? null, a.small_image ?? null]; };
  assert.deepEqual(two(CASES.launcher), ["In the launcher", null, null, NOW - 3600, null]);
  assert.deepEqual(two(CASES.locker), ["In the launcher", "In the Locker", null, NOW - 3600, null]);
  assert.deepEqual(two(CASES.offline), ["In game", "Offline", null, NOW - 70, "act_solo"]);
  assert.deepEqual(two(CASES.training), ["In game", "Training", null, NOW - 70, "act_solo"]);
  assert.deepEqual(two(CASES.lan), ["In game", "Local network", null, NOW - 70, "act_match"]);
  assert.deepEqual(two(CASES.private), ["In game", "Private match", null, NOW - 70, "act_match"]);
  assert.deepEqual(two(CASES.online), ["Online", "In a party", [2, 4], NOW - 1000, "act_online"]);
  assert.deepEqual(two(CASES.room), ["Private room", "2 v 2", [3, 4], NOW - 130, "act_room"]);
  assert.deepEqual(two(CASES.queue), ["Looking for a match", "Ranked · 4 v 4", [2, 4], NOW - 95, "act_search"]);
  assert.deepEqual(two(CASES.found), ["Match found", "Ranked · 4 v 4", null, NOW - 95, "act_search"]);
  assert.deepEqual(two(CASES.draft), ["Picking the stage", "Ranked · 4 v 4", null, NOW - 300, "act_match"]);
  assert.deepEqual(two(CASES.match), ["In a match", "Ranked · 4 v 4", null, NOW - 70, "act_match"]);
  assert.deepEqual(two(CASES.watch), ["Spectating", "Ranked · 4 v 4", null, NOW - 70, "act_watch"]);
  assert.deepEqual(two(CASES.result), ["Match over", "Victory", null, null, "act_done"]);
});

test("the mode and the stage, then the server's name, only when asked", () => {
  assert.equal(act(CASES.match, { mode_stage: true }).state, "Ranked · 4 v 4 · Area Rush · The Docks");
  assert.equal(act(CASES.room, { mode_stage: true }).state, "2 v 2 · Area Rush · The Docks");
  assert.equal(act(CASES.draft, { mode_stage: true }).state, "Ranked · 4 v 4 · Area Rush", "the draft decides the stage");
  assert.equal(act(CASES.match, { server: true }).details, "In a match · Open Ink");
  assert.equal(act(CASES.online, { server: true }).details, "Online · Open Ink");
  assert.equal(act(CASES.offline, { server: true, mode_stage: true }).details, "In game", "offline: no server");
  // a name the player's game does not give is left out (never the id, never the server's label)
  const f = online({ match: match("running", undefined, { rules: { teams: [1, 1], mode: "zones", stage: "Vss_Unknown", rated: false } }) }, { game: running("match") });
  assert.equal(act(f, { mode_stage: true }).state, "Unranked · 1 v 1");
});

test("a room match, a void match, a spectator's result, and a result that has expired", () => {
  const fromRoom = online({ match: match("running", undefined, { origin: { kind: "room", room_id: "r_42" } }) }, { game: running("match") });
  assert.equal(act(fromRoom).state, "Private room · 4 v 4");
  const lost = online({ ended: match("completed", undefined, { ended_at: iso(NOW - 10), outcome: { counted: true, winner: "alpha" } }) });
  assert.equal(act(lost).state, "Defeat");
  const voided = online({ ended: match("void", undefined, { ended_at: iso(NOW - 10), outcome: { counted: false } }) });
  assert.equal(act(voided).state, "No result");
  const watched = online({ ended: match("completed", { user_id: "u_me", team: "spectator", seat: "spectator" }, { ended_at: iso(NOW - 10), outcome: { counted: true, winner: "alpha" } }) });
  assert.equal(act(watched).state, undefined, "a spectator neither won nor lost");
  const old = online({ ended: match("completed", undefined, { ended_at: iso(NOW - RESULT_S - 1), outcome: { counted: true, winner: "bravo" } }) });
  assert.equal(situationOf(old).kind, "online");
});

test("the game started from Play wins over the server's menus; a community match wins over the game", () => {
  assert.equal(situationOf(online({ room: room() }, { game: running("range") })).kind, "training");
  assert.equal(situationOf(online({ match: match("running") }, { game: running("original") })).kind, "match");
  // a private match by code while logged in on a server
  assert.equal(situationOf(online({}, { game: running("match") })).kind, "private");
});

test("an unknown queue, a party of one and a full room", () => {
  const unknown = online({ ticket: { ...ticket(), queue_id: "elsewhere" } });
  assert.deepEqual([act(unknown).state, act(unknown).party], [undefined, undefined]);
  const solo = online({ ticket: { ...ticket(), party_id: null, members: [user("u_me", "Camille")] }, party: { ...party, members: [{ user: user("u_me", "Camille") }] } });
  assert.equal(act(solo).party, undefined, "a group of one is not a group");
  const full = online({ room: room({ options: { teams: [1, 1], mode: "turf" }, members: [room().members[0], room().members[1]] }) });
  assert.deepEqual(act(full).party, [2, 2]);
});

test("French words, and the preview's samples", () => {
  const s = situationOf(CASES.match);
  const a = buildActivity(s, { ...DEFAULT_PREFS, enabled: true, mode_stage: true }, fill(fr), "Kalamar");
  assert.deepEqual([a.details, a.state], ["En match", "Classé · 4 contre 4 · Area Rush · The Docks"]);
  const samples = sampleSituations(NOW, { mode: "Area Rush", stage: "The Docks", server: "Open Ink" });
  for (const [k, x] of Object.entries(samples)) {
    const out = buildActivity(x, { ...DEFAULT_PREFS, enabled: true, mode_stage: true, server: true }, EN, "Kalamar");
    assert.ok(out.details, k);
  }
  assert.equal(buildActivity(samples.room, { ...DEFAULT_PREFS, enabled: true, server: true }, EN, "Kalamar").details, "Private room · Open Ink");
});

test("every small picture the presence names is one of the launcher's drawings", () => {
  for (const k of Object.values(ICONS)) assert.ok(GLYPHS[k], k);
});

test("off by default; once on, the activity without mode, stage or server", () => {
  assert.deepEqual(DEFAULT_PREFS, { enabled: false, detail: "activity", mode_stage: false, server: false });
});
