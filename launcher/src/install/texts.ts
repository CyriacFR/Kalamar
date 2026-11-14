// Texts of the install wizard (dictionaries in src/locales/<code>/install.ts). Kept apart from the main dictionary: the
// wizard's error codes, step names and advice come from src-tauri/src/installer/ and are only used here.
import { useI18n } from "../i18n";
import { lookup } from "../locales";
import { install as en } from "../locales/en/install";

export type WKey = keyof typeof en;
export function useW() {
  const { lang } = useI18n();
  const w = (key: WKey | string, vars?: Record<string, string | number>): string => lookup("install", lang, key, vars);
  const has = (key: string) => key in en;
  return { w, has, lang };
}

/** "12 min 30 s", "45 s", "1 h 05". */
export function fmtDuration(secs: number | null | undefined): string {
  if (secs == null || !Number.isFinite(secs)) return "—";
  const s = Math.max(0, Math.round(secs));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min ${String(s % 60).padStart(2, "0")} s`;
  return `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")}`;
}

export const gb = (bytes: number | null | undefined, digits = 0) => (bytes == null ? "?" : (bytes / 1e9).toFixed(digits));

/** A rough duration for an estimate: "40 s" under a minute and a half, else whole minutes ("3 min", "38 min"). */
export function approx(secs: number | null | undefined): string {
  if (secs == null || !Number.isFinite(secs)) return "—";
  if (secs < 90) return `${Math.max(10, Math.round(secs / 10) * 10)} s`;
  return `${Math.round(secs / 60)} min`;
}
