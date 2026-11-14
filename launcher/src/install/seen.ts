// What the player opened in this session (a settings section, the note on the native mode): the "while it compiles"
// cards count a section looked at as checked. Kept in memory only: an installation lasts one session.
import { useSyncExternalStore } from "react";

const seen = new Set<string>();
const subs = new Set<() => void>();
let version = 0;

export function markSeen(id: string): void {
  if (seen.has(id)) return;
  seen.add(id);
  version++;
  subs.forEach((f) => f());
}

/** The set (the same object, re-rendered on change). */
export function useSeen(): ReadonlySet<string> {
  useSyncExternalStore((f) => { subs.add(f); return () => subs.delete(f); }, () => version);
  return seen;
}

/** The Settings page saved the game's settings: the cards read them again. */
export const SETTINGS_SAVED = "kalamar:settings-saved";
