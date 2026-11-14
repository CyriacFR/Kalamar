// Interface texts. The dictionaries live in src/locales/<code>/ and the language list in src/locales/index.ts.
import { createContext, useContext } from "react";
import type { AppError, Labels } from "./api";
import { DEFAULT_LANG, lookup, type Lang } from "./locales";
import { app as en } from "./locales/en/app";

export { LANGS, LOCALES, detectLang, isLang, formatDateTime, subjectFirst, type Lang } from "./locales";

export type Key = keyof typeof en;
/** The reference (English) dictionary, for "does this key exist" checks. */
export const DICTS = { en } as const;

export type TFn = (key: Key, vars?: Record<string, string | number>) => string;

export function makeT(lang: Lang): TFn {
  return (key, vars) => lookup("app", lang, key, vars);
}

export const I18nContext = createContext<{ lang: Lang; t: TFn }>({ lang: DEFAULT_LANG, t: makeT(DEFAULT_LANG) });
export const useI18n = () => useContext(I18nContext);

/** Picks the text for the current language from a {en, fr, ...} label map: that language, else English, else the first one. */
export function pick(labels: Labels | undefined, lang: string): string {
  if (!labels) return "";
  return labels[lang] || labels.en || Object.values(labels).find((v) => !!v) || "";
}

/** Human text for a backend error, translated by its kind. */
export function errorText(e: AppError, t: TFn): string {
  const key = `err.${e.kind}` as Key;
  if (!(key in en)) return e.message;
  const details = e.details.map((d) => (`check.${d}` in en ? t(`check.${d}` as Key) : d)).join(", ");
  return t(key, { message: e.message, details });
}
