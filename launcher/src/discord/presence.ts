// The Discord presence as the player's contacts see it. Two pure steps, run by the launcher on every change and by the
// Settings preview (so the preview is exactly what goes out):
//
//   situationOf(facts)  what the launcher knows of the moment (the page, the game, the community server's live state)
//                       reduced to what a presence may ever say: a kind, times, team sizes, a group size, and names
//                       of a mode, a stage and the server only as the player's own game data and server list give them;
//   buildActivity(...)  that situation written in the interface language at the player's level.
//
// Never in a situation, so never in a presence: another player's name, a room code or password, a match or user id,
// an address (relay, service), a path. The back end checks the result once more (`src-tauri/src/discord/activity.rs`)
// and owns the connection to the Discord application. `presence.test.mjs` runs both steps on made-up states.
import type { J } from "../community/api";

export type Detail = "minimal" | "activity";
export interface Prefs { enabled: boolean; detail: Detail; mode_stage: boolean; server: boolean }
export const DEFAULT_PREFS: Prefs = { enabled: false, detail: "activity", mode_stage: false, server: false };

export type Kind =
  | "launcher" | "locker"                                   // the launcher, the game not running
  | "offline" | "training" | "lan" | "private"              // the game, started from the Play page
  | "online" | "room" | "queue" | "found" | "draft" | "match" | "watch" | "result"; // a community server

export interface Situation {
  kind: Kind;
  /** The game is running. */
  game: boolean;
  /** Unix seconds the activity's elapsed time counts from (null: no time shown). */
  since: number | null;
  /** The minimal level's time: the game's start while it runs, else the launcher's. */
  base: number | null;
  /** How the match counts: ranked, unranked, a room's. */
  ranking: "rated" | "unrated" | "room" | null;
  /** [alpha, bravo] team sizes. */
  teams: [number, number] | null;
  /** [in the group, at most]: a room's players, a party's members. */
  party: [number, number] | null;
  /** Names from the player's own game data (or the launcher's own word for a mode): never the server's, never an id. */
  mode: string | null;
  stage: string | null;
  /** The community server's name as saved on this PC; never its address. */
  server: string | null;
  outcome: "win" | "loss" | "void" | null;
}

/** The live state of the community server in use, as `community/store.ts` keeps it. */
export interface LiveFacts {
  me: string | null;
  room: J; ticket: J; party: J; match: J; ended: J;
  /** What the Queues tab learnt of each queue (rated, team sizes). */
  queues: Record<string, { rated: boolean; teams: [number, number] | null }>;
}

export interface Facts {
  /** The page on screen: play, profile (the Locker), community, settings, setup. */
  page: string;
  game: { state: string; mode: string | null; started_at: number | null; restarted: boolean };
  launcherSince: number;
  /** When the player was first seen logged in on the server this session. */
  onlineSince: number | null;
  /** The saved name of the server whose live state this is; null without one. */
  server: string | null;
  /** The live state, only while logged in on that server. */
  live: LiveFacts | null;
  modeName: (id: string) => string | null;
  stageName: (id: string) => string | null;
  /** Unix seconds. */
  now: number;
}

/** A match's result stays in the presence this long after it ended. */
export const RESULT_S = 120;
const ENDED = ["completed", "void", "cancelled"];

const secs = (iso: unknown): number | null => {
  if (typeof iso !== "string") return null;
  const t = Date.parse(iso);
  return Number.isFinite(t) ? Math.floor(t / 1000) : null;
};
const id = (v: unknown): string | null => (typeof v === "string" && v.length > 0 && v.length <= 64 ? v : null);
const count = (v: unknown, max = 64): number | null => (typeof v === "number" && Number.isInteger(v) && v >= 0 && v <= max ? v : null);

function teamsOf(v: unknown): [number, number] | null {
  if (!Array.isArray(v) || v.length !== 2) return null;
  const [a, b] = [count(v[0], 8), count(v[1], 8)];
  return a && b ? [a, b] : null;
}

function group(n: number | null, max: number | null): [number, number] | null {
  return n !== null && max !== null && n >= 1 && n <= max ? [n, max] : null;
}

const isSpectator = (m: J): boolean => m?.you?.team === "spectator" || m?.you?.seat === "spectator";

function rankingOf(m: J): Situation["ranking"] {
  if (m?.origin?.kind === "room") return "room";
  if (m?.rules?.rated === true) return "rated";
  if (m?.rules?.rated === false) return "unrated";
  return null;
}

export function situationOf(f: Facts): Situation {
  const running = f.game.state === "running" || f.game.restarted;
  const gameSince = running ? f.game.started_at : null;
  const out = (kind: Kind, extra: Partial<Situation> = {}): Situation => ({
    kind, game: running, since: null, base: gameSince ?? f.launcherSince, ranking: null, teams: null, party: null,
    mode: null, stage: null, server: null, outcome: null, ...extra,
  });
  const L = f.live;
  const named = (mode: unknown, stage: unknown) => ({ mode: id(mode) ? f.modeName(id(mode)!) : null, stage: id(stage) ? f.stageName(id(stage)!) : null });

  // A community match the player is in (as a player or in the stands), from "found" to its end.
  const m = L?.match;
  if (L && m && typeof m === "object" && !ENDED.includes(m.state) && m.you) {
    const rules = m.rules ?? {};
    const common = { ranking: rankingOf(m), teams: teamsOf(rules.teams), server: f.server, ...named(rules.mode, rules.stage) };
    if (m.state === "pending_accept") return out("found", { ...common, stage: null, since: secs(L.ticket?.created_at) ?? secs(m.created_at) });
    if (m.state === "drafting") return out(isSpectator(m) ? "watch" : "draft", { ...common, stage: null, since: secs(m.created_at) });
    const since = running && f.game.mode === "match" && gameSince ? gameSince : secs(m.created_at);
    return out(isSpectator(m) ? "watch" : "match", { ...common, since });
  }
  // The game started from the Play page.
  if (running && f.game.mode !== "match") {
    const kind: Kind = f.game.mode === "range" ? "training" : f.game.mode === "lan" ? "lan" : "offline";
    return out(kind, { since: gameSince });
  }
  if (L) {
    // The end of a community match, for a little while.
    const e = L.ended;
    const endedAt = secs(e?.ended_at);
    if (e && typeof e === "object" && e.you && endedAt !== null && f.now - endedAt >= 0 && f.now - endedAt < RESULT_S) {
      const o = e.outcome;
      const mine = e.you?.team;
      const outcome: Situation["outcome"] = isSpectator(e)
        ? null
        : e.state !== "completed" || !o?.counted ? "void" : (o.winner === "alpha" || o.winner === "bravo") && (mine === "alpha" || mine === "bravo") ? (o.winner === mine ? "win" : "loss") : null;
      return out("result", { ranking: rankingOf(e), teams: teamsOf(e.rules?.teams), server: f.server, outcome, ...named(e.rules?.mode, e.rules?.stage) });
    }
  }
  // A private match by code (no community match behind it).
  if (running) return out("private", { since: gameSince });
  if (L) {
    const partyOf = (): [number, number] | null => {
      const p = L.party;
      const n = Array.isArray(p?.members) ? p.members.length : null;
      return n !== null && n >= 2 ? group(n, count(p?.max_size)) : null;
    };
    const t = L.ticket;
    if (t && typeof t === "object" && (t.state === "searching" || t.state === "match_found")) {
      const q = typeof t.queue_id === "string" ? L.queues[t.queue_id] : undefined;
      return out(t.state === "match_found" ? "found" : "queue", {
        since: secs(t.created_at), ranking: q ? (q.rated ? "rated" : "unrated") : null, teams: q?.teams ?? null, party: partyOf(), server: f.server,
      });
    }
    const r = L.room;
    if (r && typeof r === "object" && r.state !== "closed") {
      const members: J[] = Array.isArray(r.members) ? r.members : [];
      const teams = teamsOf(r.options?.teams);
      const players = members.filter((x: J) => x?.team !== "spectator").length;
      const mine = members.find((x: J) => x?.user?.id === L.me);
      return out("room", {
        since: secs(mine?.joined_at) ?? secs(r.created_at) ?? f.onlineSince, teams, party: teams ? group(players, teams[0] + teams[1]) : null,
        server: f.server, ...named(r.options?.mode, r.options?.stage),
      });
    }
    return out("online", { since: f.onlineSince, party: partyOf(), server: f.server });
  }
  return out(f.page === "profile" ? "locker" : "launcher", { since: f.launcherSince });
}

/** What goes to the back end (`discord_publish`): words and numbers only. */
export interface ActivityInput {
  details?: string;
  state?: string;
  start?: number;
  large_text?: string;
  small_image?: string;
  small_text?: string;
  party?: [number, number];
}

export type Tr = (key: string, vars?: Record<string, string | number>) => string;

const DETAILS: Record<Kind, string> = {
  launcher: "dc.a.launcher", locker: "dc.a.launcher", offline: "dc.a.game", training: "dc.a.game", lan: "dc.a.game", private: "dc.a.game",
  online: "dc.a.online", room: "dc.a.room", queue: "dc.a.queue", found: "dc.a.found", draft: "dc.a.draft", match: "dc.a.match",
  watch: "dc.a.watch", result: "dc.a.result",
};

/** The launcher's own small pictures (registered in the maintainers' Discord application, `docs/discord-presence.md`). */
export const ICONS: Partial<Record<Kind, string>> = {
  offline: "act_solo", training: "act_solo", lan: "act_match", private: "act_match", online: "act_online", room: "act_room",
  queue: "act_search", found: "act_search", draft: "act_match", match: "act_match", watch: "act_watch", result: "act_done",
};

const ONLINE: Kind[] = ["online", "room", "queue", "found", "draft", "match", "watch", "result"];
const WITH_MODE: Kind[] = ["room", "found", "draft", "match", "watch", "result"];
const SEP = " · ";

/** The activity at the player's level. `brand`: the product's name, the hover text of the logo. */
export function buildActivity(s: Situation, p: Prefs, tr: Tr, brand: string): ActivityInput {
  if (p.detail === "minimal") {
    const a: ActivityInput = { details: tr(s.game ? "dc.a.game" : "dc.a.launcher"), large_text: brand };
    if (s.base) a.start = s.base;
    return a;
  }
  const word = tr(DETAILS[s.kind]);
  const vs = s.teams ? tr("dc.vs", { a: s.teams[0], b: s.teams[1] }) : null;
  const ranking = s.ranking ? tr(`dc.s.${s.ranking}`) : null;
  let state: (string | null)[] = [];
  switch (s.kind) {
    case "locker": state = [tr("dc.s.locker")]; break;
    case "offline": state = [tr("dc.s.offline")]; break;
    case "training": state = [tr("dc.s.training")]; break;
    case "lan": state = [tr("dc.s.lan")]; break;
    case "private": state = [tr("dc.s.private")]; break;
    case "online": state = [s.party ? tr("dc.s.party") : null]; break;
    case "room": state = [vs]; break;
    case "queue": case "found": case "draft": case "match": case "watch": state = [ranking, vs]; break;
    case "result": state = [s.outcome ? tr(`dc.s.${s.outcome}`) : null]; break;
    default: break;
  }
  if (p.mode_stage && WITH_MODE.includes(s.kind)) state.push(s.mode, s.stage);
  const parts = state.filter((x): x is string => !!x);
  const a: ActivityInput = {
    details: p.server && s.server && ONLINE.includes(s.kind) ? word + SEP + s.server : word,
    large_text: brand,
  };
  if (parts.length) a.state = parts.join(SEP);
  if (s.since) a.start = s.since;
  const icon = ICONS[s.kind];
  if (icon) {
    a.small_image = icon;
    a.small_text = word;
  }
  if (s.party) a.party = s.party;
  return a;
}

/** The situations the Settings preview offers besides the present one (names given by the caller: the player's own). */
export function sampleSituations(now: number, names: { mode: string | null; stage: string | null; server: string | null }): Record<"room" | "queue" | "match" | "watch" | "game", Situation> {
  const base = { game: false, since: null, base: now - 600, ranking: null, teams: null, party: null, mode: null, stage: null, server: names.server, outcome: null } as Situation;
  return {
    room: { ...base, kind: "room", since: now - 130, teams: [2, 2], party: [3, 4], mode: names.mode, stage: names.stage },
    queue: { ...base, kind: "queue", since: now - 95, ranking: "rated", teams: [4, 4] },
    match: { ...base, kind: "match", game: true, since: now - 42, base: now - 70, ranking: "rated", teams: [4, 4], mode: names.mode, stage: names.stage },
    watch: { ...base, kind: "watch", game: true, since: now - 64, base: now - 80, ranking: "room", teams: [1, 1], mode: names.mode, stage: names.stage },
    game: { ...base, kind: "offline", game: true, since: now - 1500, base: now - 1500, server: null },
  };
}
