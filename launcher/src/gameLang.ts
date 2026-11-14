// The language of the GAME, chosen next to the launcher's (Settings > Language). The settings file keeps only an
// explicit code in `lang`; "like the launcher" and "like Windows" are the absence of it plus a launcher preference
// (`game_lang_follow`), applied at launch by src-tauri/src/gamelang.rs, which holds the same rules.
import type { Lang } from "./locales";

export type GameChoice = "launcher" | "windows" | string;

/** The languages offered, each named in its own language, in the order of the list. */
export const GAME_LANGS: { code: string; name: string }[] = [
  { code: "ja", name: "日本語" },
  { code: "en-US", name: "English (US)" },
  { code: "en-GB", name: "English (UK)" },
  { code: "fr", name: "Français" },
  { code: "fr-CA", name: "Français (Canada)" },
  { code: "de", name: "Deutsch" },
  { code: "it", name: "Italiano" },
  { code: "es", name: "Español" },
  { code: "es-419", name: "Español (Latinoamérica)" },
  { code: "nl", name: "Nederlands" },
  { code: "pt", name: "Português" },
  { code: "pt-BR", name: "Português (Brasil)" },
  { code: "ru", name: "Русский" },
  { code: "ko", name: "한국어" },
  { code: "zh-Hans", name: "简体中文" },
  { code: "zh-Hant", name: "繁體中文" },
];

/** "Like the launcher": the game language that matches a launcher language (English for any other). */
export function launcherToGame(lang: Lang | string): string {
  return ({ fr: "fr", es: "es", ja: "ja" } as Record<string, string>)[lang] ?? "en-US";
}

/** The older spellings of the two Chinese languages. */
export function canonicalGameLang(code: string): string {
  return code === "zh-CN" ? "zh-Hans" : code === "zh-TW" ? "zh-Hant" : code;
}

/** Chinese and Korean read the console's system fonts, extracted apart. */
export function needsFonts(code: string): boolean {
  return ["zh-Hans", "zh-Hant", "ko"].includes(canonicalGameLang(code));
}

/** The choice to show: an explicit `lang` of the settings file wins (it is the player's, whoever wrote it), else the
 *  launcher's preference. */
export function currentChoice(iniLang: string | undefined, follow: string | undefined): GameChoice {
  const v = (iniLang ?? "").trim();
  if (v) {
    const c = canonicalGameLang(v);
    return GAME_LANGS.some((l) => l.code.toLowerCase() === c.toLowerCase()) ? GAME_LANGS.find((l) => l.code.toLowerCase() === c.toLowerCase())!.code : v;
  }
  return follow === "windows" ? "windows" : "launcher";
}

/** An option is grayed out when it needs fonts that are not there (the one in use stays selectable). */
export function optionDisabled(code: string, fontsOk: boolean): boolean {
  return needsFonts(code) && !fontsOk;
}

/** The name of a game language in its own language ("Français"), or the code itself for one we do not list. */
export function gameLangName(code: string): string {
  return GAME_LANGS.find((l) => l.code === canonicalGameLang(code))?.name ?? code;
}
