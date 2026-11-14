// Texts of the community page (dictionaries in src/locales/<code>/community.ts). Everything the server declares (queue
// names, room options, tiers, leaderboard columns, rules, policies) comes from the server as localised text, not from here.
import { useI18n, pick } from "../i18n";
import { lookup } from "../locales";
import { community as en } from "../locales/en/community";
import type { Fail, LText } from "./api";

export type CKey = keyof typeof en;
export type CFn = (key: CKey, vars?: Record<string, string | number>) => string;

export function useC() {
  const { lang, t } = useI18n();
  const c: CFn = (key, vars) => lookup("community", lang, key, vars);
  /** Server text in the player's language (else English, else any). */
  const L = (x: LText | null | undefined): string => (x && typeof x === "object" ? pick(x, lang) : "");
  return { c, L, lang, t };
}

/** Protocol 1.1.11: a match won or lost because a player left it while it was being played (`outcome.forfeit` or a
 *  history entry's `forfeit`, reason `left`), from this player's side: whether it won, and why in a few words (the
 *  opponent left, the player left, a team-mate left). `null` for any other match (another forfeit reason shows the
 *  server's `summary`). `myTeam`: the player's team (`null` for a spectator: the team that left is named). */
export function leftForfeit(c: CFn, forfeit: unknown, myTeam: unknown, me: string | null, duel: boolean): { won: boolean | null; why: string } | null {
  const f = forfeit as { reason?: unknown; team?: unknown; players?: { user_id?: unknown }[] } | null | undefined;
  if (!f || f.reason !== "left" || (f.team !== "alpha" && f.team !== "bravo")) return null;
  if (myTeam !== "alpha" && myTeam !== "bravo") return { won: null, why: c("m.forfeit.team", { team: f.team === "alpha" ? "Alpha" : "Bravo" }) };
  if (f.team !== myTeam) return { won: true, why: duel ? c("m.forfeit.them") : c("m.forfeit.them.team") };
  const mine = Array.isArray(f.players) && f.players.some((p) => !!me && p?.user_id === me);
  return { won: false, why: mine ? c("m.forfeit.you") : c("m.forfeit.mate") };
}

export function duration(c: CFn, secs: number): string {
  const s = Math.max(0, Math.round(secs));
  return s >= 60 ? c("fmt.min", { m: Math.floor(s / 60), s: s % 60 }) : c("fmt.sec", { s });
}

/** One `missing` item of `incompatible_game` (`match-contract-2.md` 3); unknown items as they are. */
export function missingText(c: CFn, item: string): string {
  const [kind, rest = ""] = item.split(/:(.*)/s);
  switch (kind) {
    case "contract": return c("inc.contract", { v: rest });
    case "rule_key": return c("inc.rule_key", { k: rest });
    case "rounds": return c("inc.rounds");
    case "players": return c("inc.players", { n: rest });
    case "per_team": return c("inc.per_team", { n: rest });
    case "uneven_teams": return c("inc.uneven_teams");
    case "spectators": return rest ? c("inc.spectators.n", { n: rest }) : c("inc.spectators");
    case "spectator_mode": return c("inc.spectator_mode", { k: rest });
    case "spectator_host": return c("inc.spectator_host");
    case "loadout": return c("inc.loadout", { k: rest });
    case "mode": return c("inc.mode", { k: rest });
    case "stage": return c("inc.stage", { k: rest });
    case "launcher": return rest === "draft" ? c("inc.launcher.draft") : rest === "spectate" ? c("inc.launcher.spectate") : item.slice(0, 80);
    default: return item.slice(0, 80);
  }
}

/** `incompatible_game` with its `details.players[].missing`, as sentences ("your engine cannot do X: update it"). */
export function incompatibleText(f: Fail, c: CFn, me: string | null, names: Record<string, string> = {}): string | null {
  const players = Array.isArray(f.data?.players) ? f.data.players : null;
  if (f.code !== "incompatible_game" || !players) return null;
  const parts = players.slice(0, 8).map((p: { user_id?: string; missing?: unknown[] }) => {
    const list = (Array.isArray(p?.missing) ? p.missing : []).slice(0, 12).map((m) => missingText(c, String(m))).join(", ");
    return p?.user_id && p.user_id === me ? c("inc.me", { list }) : c("inc.other", { name: names[p?.user_id ?? ""] ?? p?.user_id ?? "?", list });
  });
  return parts.join(" ");
}

/** A command's failure as a sentence: the protocol's known codes are translated, the rest shows the server's message. */
export function failText(f: Fail, c: CFn, extra: Record<string, string | number> = {}, me: string | null = null): string {
  const inc = incompatibleText(f, c, me);
  if (inc) return inc;
  const key = `err.${f.code}` as CKey;
  const left = f.retry_after_s != null ? duration(c, f.retry_after_s) : "";
  if (key in en) return c(key, { left, message: f.message, field: f.fields[0]?.key ?? "", ...extra });
  return f.message || f.code;
}
