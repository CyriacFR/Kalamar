// The list of interface languages. To add a language:
//   1. copy src/locales/en/ to src/locales/<code>/ (four files: app, community, servers, install) and translate the values;
//      the compiler refuses a language that misses a key or has an extra one (and `npm run test:ui` re-checks `{vars}`);
//   2. add one entry below (code, name in its own language, BCP 47 tag for dates, title word order);
//   3. add the code to `LANGUAGES` in src-tauri/src/config.rs, and the label maps ({ en, fr, ... }) of
//      src-tauri/resources/settings-schema.json.
// A missing key in a language falls back to English (the reference), then the key itself.
import { app as fr_app } from "./fr/app";
import { community as fr_community } from "./fr/community";
import { servers as fr_servers } from "./fr/servers";
import { install as fr_install } from "./fr/install";
import { app as en_app } from "./en/app";
import { community as en_community } from "./en/community";
import { servers as en_servers } from "./en/servers";
import { install as en_install } from "./en/install";
import { app as es_app } from "./es/app";
import { community as es_community } from "./es/community";
import { servers as es_servers } from "./es/servers";
import { install as es_install } from "./es/install";
import { app as ja_app } from "./ja/app";
import { community as ja_community } from "./ja/community";
import { servers as ja_servers } from "./ja/servers";
import { install as ja_install } from "./ja/install";

export interface Locale {
  code: string;
  /** The language's own name, shown in the language menu. */
  name: string;
  /** BCP 47 tag for dates and numbers. */
  tag: string;
  /** "subject adjective" (French, Spanish) or "adjective subject" for composed titles. */
  titleOrder: "adjective-first" | "subject-first";
  app: Record<keyof typeof en_app, string>;
  community: Record<keyof typeof en_community, string>;
  servers: Record<keyof typeof en_servers, string>;
  install: Record<keyof typeof en_install, string>;
}

export const LOCALES = [
  { code: "en", name: "English", tag: "en-GB", titleOrder: "adjective-first", app: en_app, community: en_community, servers: en_servers, install: en_install },
  { code: "fr", name: "Français", tag: "fr-FR", titleOrder: "subject-first", app: fr_app, community: fr_community, servers: fr_servers, install: fr_install },
  { code: "es", name: "Español", tag: "es-419", titleOrder: "subject-first", app: es_app, community: es_community, servers: es_servers, install: es_install },
  { code: "ja", name: "日本語", tag: "ja-JP", titleOrder: "adjective-first", app: ja_app, community: ja_community, servers: ja_servers, install: ja_install },
] as const satisfies readonly Locale[];

export type Lang = (typeof LOCALES)[number]["code"];
export const LANGS: { code: Lang; name: string }[] = LOCALES.map((l) => ({ code: l.code, name: l.name }));
export const DEFAULT_LANG: Lang = "en";
export const FALLBACK_LANG: Lang = "en";

export function isLang(code: unknown): code is Lang {
  return typeof code === "string" && LOCALES.some((l) => l.code === code);
}

export function localeOf(code: string | undefined): (typeof LOCALES)[number] {
  return LOCALES.find((l) => l.code === code) ?? LOCALES.find((l) => l.code === FALLBACK_LANG)!;
}

/** The first of the system's preferred languages that the launcher supports ("ja-JP" -> "ja"), else English. */
export function detectLang(preferred: readonly string[] = typeof navigator === "undefined" ? [] : (navigator.languages?.length ? navigator.languages : [navigator.language])): Lang {
  for (const p of preferred) {
    const base = String(p).toLowerCase().split(/[-_]/)[0];
    if (isLang(base)) return base;
  }
  return FALLBACK_LANG;
}

/** Date and time for the interface language. */
export function formatDateTime(date: Date, lang: string): string {
  return date.toLocaleString(localeOf(lang).tag, { dateStyle: "medium", timeStyle: "short" });
}

/** A composed title is "subject adjective" in some languages, "adjective subject" in others. */
export function subjectFirst(lang: string): boolean {
  return localeOf(lang).titleOrder === "subject-first";
}

/** Text of a key in a language: that language, else English, else the key. Variables `{name}` are filled in. */
export function lookup(section: "app" | "community" | "servers" | "install", lang: string, key: string, vars?: Record<string, string | number>): string {
  const get = (code: string) => (LOCALES.find((l) => l.code === code)?.[section] as Record<string, string> | undefined)?.[key];
  let s = get(lang) ?? get(FALLBACK_LANG) ?? key;
  if (vars) for (const [k, v] of Object.entries(vars)) s = s.split(`{${k}}`).join(String(v));
  return s;
}
