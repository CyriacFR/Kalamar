// The live state of the community server in use, shared by the community page and the overlay (match found, invites,
// the running match): one module-level store fed by GET /v1/state and the event stream's pushes. Nothing here runs
// before the community page is opened for the first time.
import { useSyncExternalStore } from "react";
import { onProto, proto, toFail, type Fail, type J, type Overview, type Push, type RelayView, type RunnerStatus, type StreamInfo } from "./api";

export interface Live {
  server: string | null;
  overview: Overview | null;
  loading: boolean;
  error: Fail | null;
  me: J | null;
  room: J | null;
  roomInvites: J[];
  ticket: J | null;
  party: J | null;
  partyInvites: J[];
  /** The player's current match (pending_accept to running). */
  match: J | null;
  /** The last match that ended (completed, void or cancelled), for its outcome. */
  ended: J | null;
  stream: StreamInfo | null;
  runner: RunnerStatus | null;
  presence: Record<string, string>;
  /** Bumped when friends, ratings or queues changed on the server: the tabs refetch. */
  rev: { friends: number; ratings: number; queues: number; stats: number; activities: number; badges: number };
  /** Latest revision of each activity's view announced by `activity.updated`. */
  activityRev: Record<string, number>;
  /** Titles of the activities seen in the list (for a match's origin). */
  activityTitles: Record<string, J>;
  /** Series as last pushed (`series.updated`) or fetched. */
  series: Record<string, J>;
  /** Messages of the server for this player (`activity.notice`), shown once each by the overlay. */
  notices: { n: number; notice: J; activity: string }[];
  /** Local time when the state was last read from the server. */
  syncedAt: number;
  /** Local names of game identifiers (stage scene and mode key to the game's own label). */
  names: { stages: Record<string, string>; modes: Record<string, string> };
  /** What the queue list said of each queue (ranked or not, team sizes), for the Discord presence. */
  queueFacts: Record<string, { rated: boolean; teams: [number, number] | null }>;
}

const EMPTY: Live = {
  server: null, overview: null, loading: false, error: null, me: null, room: null, roomInvites: [], ticket: null, party: null,
  partyInvites: [], match: null, ended: null, stream: null, runner: null, presence: {}, rev: { friends: 0, ratings: 0, queues: 0, stats: 0, activities: 0, badges: 0 },
  activityRev: {}, activityTitles: {}, series: {}, notices: [], syncedAt: 0, names: { stages: {}, modes: {} }, queueFacts: {},
};

/** Keeps whether each listed queue is ranked and its team sizes (the Queues tab calls it with the list it read). */
export function noteQueues(items: J[]): void {
  const facts: Live["queueFacts"] = {};
  for (const q of items) {
    if (typeof q?.id !== "string") continue;
    const t = Array.isArray(q.teams) && q.teams.length === 2 && q.teams.every((n: unknown) => typeof n === "number" && Number.isInteger(n) && n >= 1 && n <= 8) ? [q.teams[0], q.teams[1]] as [number, number] : null;
    facts[q.id] = { rated: q.rated === true, teams: t };
  }
  if (JSON.stringify(facts) !== JSON.stringify(live.queueFacts)) set({ queueFacts: facts });
}

let live: Live = EMPTY;
const subs = new Set<() => void>();
let listening = false;

function set(patch: Partial<Live> | ((l: Live) => Partial<Live>)) {
  const p = typeof patch === "function" ? patch(live) : patch;
  live = { ...live, ...p };
  subs.forEach((f) => f());
}

export function useLive(): Live {
  return useSyncExternalStore((f) => { subs.add(f); return () => subs.delete(f); }, () => live);
}

export function getLive(): Live {
  return live;
}

/** Applies an answer of the server right away (the matching push may follow; it carries the same object). */
export function setLive(patch: Partial<Live> | ((l: Live) => Partial<Live>)): void {
  set(patch);
}

/** The server's clock now (ms), from the offset measured on its meta. */
export function serverNow(): number {
  return Date.now() + (live.overview?.offset_s ?? 0) * 1000;
}

export function features(): string[] {
  return live.overview?.check?.features ?? [];
}

export function has(area: string): boolean {
  return features().includes(area);
}

export async function req<T = J>(method: string, path: string, body?: unknown): Promise<T> {
  if (!live.server) throw toFail({ kind: "invalid_input", message: "no server", code: "no_server" });
  return proto.request<T>(live.server, method, path, body);
}

function bump(k: keyof Live["rev"]) {
  set((l) => ({ rev: { ...l.rev, [k]: l.rev[k] + 1 } }));
}

function applyState(s: J) {
  set({
    me: s?.me ?? live.me,
    room: s?.room ?? null,
    roomInvites: Array.isArray(s?.room_invites) ? s.room_invites : [],
    ticket: s?.ticket ?? null,
    party: s?.party ?? null,
    partyInvites: Array.isArray(s?.party_invites) ? s.party_invites : [],
    match: s?.match ?? null,
    syncedAt: Date.now(),
  });
}

export async function refreshState(): Promise<void> {
  if (!live.server || !live.overview?.session.logged_in) return;
  try {
    applyState(await req("GET", "/v1/state"));
  } catch (e) {
    set({ error: toFail(e) });
  }
  void loadNames();
}

let namesAsked = false;

/** The names of the game's stages and modes from the player's own game data (the engine's list, already read for the
 *  login's declaration): the community page names game identifiers with them first (`match-contract.md` 2). */
async function loadNames(): Promise<void> {
  if (namesAsked) return;
  namesAsked = true;
  try {
    const e = await proto.engine();
    const n = e?.names;
    if (n && typeof n === "object") set({ names: { stages: n.stages ?? {}, modes: n.modes ?? {} } });
  } catch {
    namesAsked = false;
  }
}

const ENDED = ["completed", "void", "cancelled"];

/** A match object from an answer or a push: the current match, or the last one when it has ended. */
export function applyMatch(m: J): void {
  if (!m || typeof m !== "object" || !m.id) return;
  // 1.1.7: a running match without `you` is no longer the player's (a spectator who left its place).
  if (!ENDED.includes(m.state) && !m.you) {
    set((l) => ({ match: l.match && l.match.id === m.id ? null : l.match }));
    return;
  }
  // 1.1.10: an undecided match this player is free of (its report sent) never replaces the player's next match.
  if (!ENDED.includes(m.state) && m.you?.free === true && live.match && live.match.id !== m.id) return;
  if (ENDED.includes(m.state)) set((l) => ({ match: l.match && l.match.id !== m.id ? l.match : null, ended: m }));
  else set({ match: m });
}

function onEvent(p: Push) {
  if (p.server !== live.server) return;
  const type: string = p.type ?? "";
  const d = p.data ?? {};
  switch (type) {
    case "resync":
      void refreshState();
      return;
    case "meta.changed":
      // The server's offer changed while it runs (1.1.9: a reloaded configuration): everything drawn from it is read again.
      void reload(true);
      (["queues", "activities", "ratings", "badges"] as const).forEach(bump);
      return;
    case "me.updated":
      set({ me: d });
      return;
    case "session.revoked":
      return;
    case "room.updated":
      set({ room: d });
      return;
    case "room.closed":
      if (!live.room || live.room.id === d.room_id) set({ room: null });
      return;
    case "room.invite":
      set((l) => ({ roomInvites: [...l.roomInvites.filter((i: J) => i.room_id !== d.room_id), d] }));
      return;
    case "party.updated":
      set({ party: d });
      return;
    case "party.closed":
      if (!live.party || live.party.id === d.party_id) set({ party: null });
      return;
    case "party.invite":
      set((l) => ({ partyInvites: [...l.partyInvites.filter((i: J) => i.party_id !== d.party_id), d] }));
      return;
    case "queue.ticket":
      set({ ticket: d.ticket ? { ...d.ticket, reason: d.reason ?? null } : null });
      return;
    case "friend.request":
    case "friend.added":
    case "friend.removed":
      bump("friends");
      return;
    case "friend.presence":
      set((l) => ({ presence: { ...l.presence, [d.user_id]: d.presence } }));
      return;
    case "rating.changed":
      bump("ratings");
      bump("badges");
      return;
    case "activity.updated":
      if (typeof d.activity_id === "string") set((l) => ({ activityRev: { ...l.activityRev, [d.activity_id]: Number(d.revision) || (l.activityRev[d.activity_id] ?? 0) + 1 } }));
      bump("activities");
      return;
    case "activity.notice":
      if (d.notice) set((l) => ({ notices: [...l.notices.slice(-9), { n: (l.notices[l.notices.length - 1]?.n ?? 0) + 1, notice: d.notice, activity: String(d.activity_id ?? "") }] }));
      return;
    case "series.updated":
      if (typeof d.id === "string") set((l) => ({ series: { ...l.series, [d.id]: d } }));
      return;
  }
  if (type.startsWith("match.")) {
    applyMatch(d);
    if (ENDED.includes(d.state)) {
      bump("stats");
      bump("ratings");
    }
  }
}

function onStream(p: Push) {
  if (p.server === live.server) set({ stream: p as unknown as StreamInfo });
}

function onSession(p: Push) {
  if (p.server !== live.server || !live.overview) return;
  set((l) => ({ ...EMPTY, server: l.server, overview: l.overview && { ...l.overview, session: { logged_in: false, provider: null, user_id: null, display_name: null } }, error: toFail({ kind: "protocol", code: String(p.reason ?? "session_revoked"), message: "" }) }));
}

function onRunner(p: Push) {
  if (p.server === live.server) set({ runner: p as unknown as RunnerStatus });
}

/** 1.1.9: the relays' measurement moved on (pending, then the values). */
function onRelays(p: Push) {
  if (p.server !== live.server || !Array.isArray(p.relays)) return;
  set((l) => ({ overview: l.overview && { ...l.overview, relays: p.relays as RelayView[] } }));
}

async function listenOnce() {
  if (listening) return;
  listening = true;
  await Promise.all([onProto("proto-event", onEvent), onProto("proto-stream", onStream), onProto("proto-session", onSession), onProto("proto-runner", onRunner), onProto("proto-relays", onRelays)]);
}

/** Opens a server: listens to the pushes, fetches its meta, starts the stream when logged in, reads the live state. */
export async function activate(server: string): Promise<void> {
  await listenOnce();
  if (live.server !== server) live = { ...EMPTY, server, names: live.names };
  set({ loading: true, error: null });
  try {
    const o = await proto.connect(server);
    set({ overview: o, stream: freshest(o.stream), runner: o.runner ?? live.runner, loading: false });
    if (o.session.logged_in) await refreshState();
  } catch (e) {
    set({ loading: false, error: toFail(e) });
  }
}

/** Fetches the overview again (`meta` from the server when `refreshMeta`). */
export async function reload(refreshMeta = false): Promise<void> {
  if (!live.server) return;
  try {
    const o = await proto.overview(live.server, refreshMeta);
    set({ overview: o, stream: freshest(o.stream) ?? live.stream, runner: o.runner ?? live.runner });
  } catch (e) {
    set({ error: toFail(e) });
  }
}

/** The stream state of an answer may be older than a push already received (the push wins unless it said "stopped"). */
function freshest(s: StreamInfo | null): StreamInfo | null {
  return live.stream && live.stream.state !== "stopped" ? live.stream : s;
}

/** After a login: the overview (stream started by the backend) and the live state. */
export async function loggedIn(o: Overview): Promise<void> {
  set({ overview: o, stream: freshest(o.stream), error: null });
  await refreshState();
}

export async function logout(all = false): Promise<void> {
  if (!live.server) return;
  const o = await proto.logout(live.server, all);
  set({ ...EMPTY, server: live.server, overview: o, names: live.names });
}

/** Saves the player's data from the server in a file they choose; the answer is its path, or null when they gave up. */
export async function exportData(): Promise<string | null> {
  if (!live.server) return null;
  return proto.exportData(live.server);
}

/** Erases the account on the server, then shows the server as after a logout. */
export async function eraseAccount(typedName: string): Promise<void> {
  if (!live.server) return;
  const o = await proto.erase(live.server, typedName);
  set({ ...EMPTY, server: live.server, overview: o, names: live.names });
}

/** The player's own user id. */
export function myId(): string | null {
  return live.me?.id ?? live.overview?.session.user_id ?? null;
}
