// Helpers over the game catalogue (names, icons, title composition). The catalogue itself is built at run time by the
// Rust backend from the player's own game files; nothing here contains game data.
import { convertFileSrc } from "@tauri-apps/api/core";
import { subjectFirst } from "./locales";
import type { GameCatalogue, GameEntry, Named } from "./api";

/** URL of an icon key (`rom/...` or `lay/...`), served by the backend's `gameicon` protocol from the app data cache. */
export function iconUrl(key: string): string {
  return convertFileSrc(key, "gameicon");
}

type NameMap = Record<string, string>;

export function pickName(name: NameMap | undefined, lang: string): string {
  if (!name) return "";
  return name[lang] || name.en || Object.values(name).find((v) => !!v) || "";
}

/** Display name: the language text from the game's messages, else the internal row id. */
export function entryName(e: GameEntry | undefined, lang: string): string {
  if (!e) return "";
  return pickName(e.name, lang) || e.row;
}

export function namedName(n: Named | undefined, key: string, lang: string): string {
  return pickName(n?.name, lang) || key;
}

export function byId(list: GameEntry[] | undefined): Map<number, GameEntry> {
  return new Map((list ?? []).map((e) => [e.id, e]));
}

/** A title is "adjective subject" or "subject adjective" depending on the language (see src/locales/index.ts). */
export function titleText(lang: string, adjective: string, subject: string): string {
  const parts = subjectFirst(lang) ? [subject, adjective] : [adjective, subject];
  return parts.filter(Boolean).join(" ");
}

export function normalise(s: string): string {
  return s.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
}

export function selectable(cat: GameCatalogue, id: keyof GameCatalogue["categories"]): GameEntry[] {
  return (cat.categories[id] ?? []).filter((e) => e.selectable);
}

export function rarityStars(n: number | null): string {
  return n == null || n < 0 ? "" : "★".repeat(Math.min(n, 5));
}
