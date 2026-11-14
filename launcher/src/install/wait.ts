// "While it compiles": what the player can set up during an installation, and how an update's length is told before it
// starts. Pure logic (no React, no IPC), tested by wait.test.mjs; the cards are in WaitTasks.tsx.
import type { UpdatePreview } from "../api";

export type TaskId = "locker" | "controls" | "display" | "server" | "native";
export const TASKS: TaskId[] = ["locker", "controls", "display", "server", "native"];

/** What the launcher knows of the player's setup, from its own files (nothing is guessed). */
export interface TaskFacts {
  /** The Locker can read the game's names, icons and colors: the merge is complete, or a game is installed. */
  gameData: boolean;
  /** A name is chosen; the whole appearance too (style, hair, eyebrows, legwear, skin, eyes). */
  hasName: boolean;
  profileComplete: boolean;
  /** The values of the game's settings file (null: not read yet). */
  settings: Record<string, string> | null;
  /** What the player opened in this session: `set.mouse`, `set.controls`, `set.display`, `native`. */
  seen: ReadonlySet<string>;
  /** Community servers added; logged in on the one in use (null: nothing to log into, or not known). */
  servers: number;
  loggedIn: boolean | null;
}

export interface TaskState {
  id: TaskId;
  done: boolean;
  /** Half done (a name without the appearance, the mouse without the keys, a server without the login). */
  partial: boolean;
  /** Not possible yet (the Locker before the merge is complete). */
  locked: boolean;
  /** Optional: online play is not everybody's. */
  optional: boolean;
}

const DISPLAY_KEYS = ["fullscreen", "window_size", "output_res", "fps_limit", "render_scale", "vk_present", "perf_overlay"];

export function taskStates(f: TaskFacts): TaskState[] {
  const keys = Object.keys(f.settings ?? {});
  const mouse = keys.some((k) => k.startsWith("mouse_gyro") || k === "mouse_sens") || f.seen.has("set.mouse");
  const binds = keys.some((k) => k.startsWith("bind_")) || f.seen.has("set.controls");
  const display = keys.some((k) => DISPLAY_KEYS.includes(k)) || f.seen.has("set.display");
  const one = (id: TaskId, done: boolean, partial = false, locked = false, optional = false): TaskState => ({ id, done, partial: !done && partial, locked: locked && !done, optional });
  return [
    one("locker", f.gameData && f.profileComplete, f.hasName, !f.gameData),
    one("controls", mouse && binds, mouse || binds),
    one("display", display),
    one("server", f.servers > 0 && f.loggedIn !== false, f.servers > 0, false, true),
    one("native", f.seen.has("native")),
  ];
}

/** Done tasks, of the ones that count (the optional ones count only once done). */
export function progressOf(states: TaskState[]): { done: number; total: number } {
  const counted = states.filter((s) => !s.optional || s.done);
  return { done: counted.filter((s) => s.done).length, total: counted.length };
}

/** An update longer than this asks when to run it (now, when the launcher closes, later); a shorter one is one click. */
export const LONG_UPDATE_SECS = 120;

/** `short`: a quick update; `long`: a part of the game recompiled; `full`: everything recompiled. */
export type UpdateClass = "short" | "long" | "full";

export function updateClass(u: UpdatePreview | null | undefined, eta: number): UpdateClass {
  if (u?.kind === "full") return "full";
  return eta > LONG_UPDATE_SECS ? "long" : "short";
}

/** The one reason, as the suffix of an `upd.why.*` text: what changed, in plain words. */
export function updateWhy(u: UpdatePreview | null | undefined): string {
  if (!u) return "runtime";
  if (u.kind === "full") {
    for (const r of ["force", "toolchain", "flags", "headers", "objects_missing", "lift_missing"] as const) if (u.reasons.includes(r)) return r;
    return "headers";
  }
  return u.kind === "none" ? "none" : u.kind;
}

/** Minutes for a button ("~12 min"): at least one. */
export const minutes = (secs: number) => Math.max(1, Math.round(secs / 60));
