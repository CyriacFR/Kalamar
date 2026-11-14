// The stage draft of a match (protocol 1.1.6, `draft.md`) read as data: whose turn it is, what the player may select,
// the turn log. Pure functions, no React: the screen (Draft.tsx) draws what they say, the tests run them in Node.
// Every member is read defensively: unknown actions, states and reasons fall back to something neutral.

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type J = any;

export type Team = "alpha" | "bravo";
export const TEAMS: Team[] = ["alpha", "bravo"];

export function isTeam(t: unknown): t is Team {
  return t === "alpha" || t === "bravo";
}

/** A draft the screen can draw: version 1, with options and turns. */
export function usable(d: J): boolean {
  return !!d && typeof d === "object" && d.version === 1 && Array.isArray(d.options) && Array.isArray(d.turns);
}

/** The draft screen is wanted: the match is drafting, or its draft just ended and the game is not on its way yet. */
export function draftWanted(m: J): boolean {
  if (!m || !usable(m.draft)) return false;
  return m.state === "drafting" || (m.state === "ready" && m.draft.state === "done");
}

export function currentTurn(d: J): J | null {
  const i = d?.current;
  return typeof i === "number" && Array.isArray(d?.turns) ? (d.turns[i] ?? null) : null;
}

/** How many options the current turn takes from each acting team (at least 1). */
export function needed(d: J): number {
  const n = Number(currentTurn(d)?.count);
  return Number.isFinite(n) && n >= 1 ? Math.min(16, Math.floor(n)) : 1;
}

export type Mark = "available" | "banned" | "picked" | "unavailable";

/** An option's state as the screen draws it (unknown states: unavailable). */
export function markOf(o: J): Mark {
  return ["available", "banned", "picked", "unavailable"].includes(o?.state) ? o.state : "unavailable";
}

/** The player may select this option now: their turn, the turn's kind, an available option. */
export function selectable(d: J, o: J): boolean {
  const t = currentTurn(d);
  return !!t && d?.you?.can_act === true && d?.state === "running" && o?.kind === t.kind && markOf(o) === "available";
}

/** The selection after a click on `id`, for a turn that takes `n` options: a second click removes it; with one option
 *  a click replaces it; with several, a click past `n` is ignored. */
export function toggle(selected: string[], id: string, n: number): string[] {
  if (selected.includes(id)) return selected.filter((x) => x !== id);
  if (n <= 1) return [id];
  return selected.length >= n ? selected : [...selected, id];
}

/** What the screen's headline says, from the player's point of view. */
export type Headline =
  | { kind: "you"; action: string; what: string; n: number; vote: boolean }
  | { kind: "team"; action: string; what: string; n: number; captain: string | null }
  | { kind: "them"; action: string; what: string; n: number; team: Team | "both" }
  | { kind: "hidden"; action: string; what: string; n: number; acted: boolean }
  | { kind: "server"; action: string; what: string; n: number }
  | { kind: "done" }
  | { kind: "waiting" };

export function headline(d: J): Headline {
  if (!usable(d)) return { kind: "waiting" };
  if (d.state === "done") return { kind: "done" };
  const t = currentTurn(d);
  if (d.state !== "running" || !t) return { kind: "waiting" };
  const action = typeof t.action === "string" ? t.action : "";
  const what = t.kind === "mode" ? "mode" : "stage";
  const n = needed(d);
  const teams: Team[] = (Array.isArray(t.teams) ? t.teams : []).filter(isTeam);
  const mine: Team | null = isTeam(d.you?.team) ? d.you.team : null;
  if (teams.length === 0) return { kind: "server", action, what, n };
  if (d.you?.can_act === true) return { kind: "you", action, what, n, vote: d.decider === "vote" };
  if (mine && teams.includes(mine)) {
    // Both teams at once: ours chose, the other team's choice is hidden until it chose too.
    if (teams.length > 1) return { kind: "hidden", action, what, n, acted: (t.acted ?? []).includes(mine) };
    return { kind: "team", action, what, n, captain: typeof d.teams?.[mine]?.captain === "string" ? d.teams[mine].captain : null };
  }
  return { kind: "them", action, what, n, team: teams.length > 1 ? "both" : teams[0] };
}

/** Seconds of the turn left, and the ring's fraction, against the server's clock (`now` in ms). */
export function turnClock(d: J, now: number): { left: number; fraction: number } | null {
  const t = currentTurn(d);
  const end = Date.parse(t?.deadline ?? "");
  if (!t || !Number.isFinite(end)) return null;
  const left = Math.max(0, Math.ceil((end - now) / 1000));
  const start = Date.parse(t.started_at ?? "");
  const total = Number.isFinite(start) && end > start ? (end - start) / 1000 : Number(d.turn_s) > 0 ? Number(d.turn_s) : left;
  return { left, fraction: total > 0 ? Math.max(0, Math.min(1, left / total)) : 0 };
}

/** One line of the turn log. */
export interface LogLine {
  index: number;
  action: string;
  kind: string;
  team: Team | null;
  by: string;
  ids: string[];
  skipped: boolean;
}

/** The turns played, in order (a skipped turn too, a turn with several teams gives one line per team). */
export function log(d: J): LogLine[] {
  const out: LogLine[] = [];
  for (const [i, t] of (Array.isArray(d?.turns) ? d.turns : []).entries()) {
    const index = typeof t?.index === "number" ? t.index : i;
    const action = typeof t?.action === "string" ? t.action : "";
    const kind = t?.kind === "mode" ? "mode" : "stage";
    if (t?.state === "skipped") {
      out.push({ index, action, kind, team: isTeam(t.teams?.[0]) ? t.teams[0] : null, by: "server", ids: [], skipped: true });
      continue;
    }
    if (t?.state !== "done") continue;
    for (const c of Array.isArray(t.choices) ? t.choices : []) {
      out.push({ index, action, kind, team: isTeam(c?.team) ? c.team : null, by: typeof c?.by === "string" ? c.by : "server", ids: (Array.isArray(c?.ids) ? c.ids : []).filter((x: unknown) => typeof x === "string"), skipped: false });
    }
  }
  return out;
}

/** The current turn's teams that already chose (their choice hidden): for "chose" marks next to the teams. */
export function acted(d: J): Team[] {
  const t = currentTurn(d);
  return (Array.isArray(t?.acted) ? t.acted : []).filter(isTeam);
}

/** The team whose turn it is now, or both, or none. */
export function actingTeams(d: J): Team[] {
  const t = currentTurn(d);
  return d?.state === "running" ? (Array.isArray(t?.teams) ? t.teams : []).filter(isTeam) : [];
}

/** The ids of the options of a kind, in the server's order. */
export function idsOf(d: J, kind: "stage" | "mode"): J[] {
  return (Array.isArray(d?.options) ? d.options : []).filter((o: J) => o?.kind === kind && typeof o?.id === "string").slice(0, 128);
}
