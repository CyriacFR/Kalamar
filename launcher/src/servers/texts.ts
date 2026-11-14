// Texts of the community-server features (dictionaries in src/locales/<code>/servers.ts).
import { useI18n } from "../i18n";
import { lookup } from "../locales";

export function useS() {
  const { lang } = useI18n();
  const s = (key: string, vars?: Record<string, string | number>) => lookup("servers", lang, key, vars);
  return { s, lang };
}
