// The one place that names the product: change it here (and `productName`/`title` in
// src-tauri/tauri.conf.json, `PREFIX` in src-tauri/src/matchcode.rs for the match-code prefix).
export const BRAND = {
  name: "Kalamar",
  /** Three letters shown in the match code and the compact wordmark. */
  short: "KLM",
  tagline: { fr: "Le menu de vos parties privées", en: "The menu for your private matches", es: "El menú de tus partidas privadas", ja: "プライベートマッチのメニュー" },
} as const;
