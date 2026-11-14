import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { GAME_LANGS, canonicalGameLang, currentChoice, gameLangName, launcherToGame, needsFonts, optionDisabled } from "./gameLang.ts";

test("like the launcher: fr, en, es and ja map to the game language of the same name", () => {
  assert.deepEqual(["fr", "en", "es", "ja"].map(launcherToGame), ["fr", "en-US", "es", "ja"]);
  assert.equal(launcherToGame("de"), "en-US");
  for (const l of ["fr", "en", "es", "ja"]) assert.ok(GAME_LANGS.some((g) => g.code === launcherToGame(l)));
});

test("a language set in the settings file is shown as an explicit choice, whatever the preference", () => {
  assert.equal(currentChoice(undefined, undefined), "launcher");
  assert.equal(currentChoice("", "launcher"), "launcher");
  assert.equal(currentChoice("  ", "windows"), "windows");
  assert.equal(currentChoice(undefined, "windows"), "windows");
  assert.equal(currentChoice("de", "launcher"), "de");
  assert.equal(currentChoice("de", "windows"), "de");
  assert.equal(currentChoice("pt-br", "launcher"), "pt-BR");
  // the older spellings of the two Chinese languages are the same choices as the new ones
  assert.equal(currentChoice("zh-CN", "launcher"), "zh-Hans");
  assert.equal(currentChoice("zh-TW", "launcher"), "zh-Hant");
  assert.equal(canonicalGameLang("fr"), "fr");
});

test("Chinese and Korean are greyed out without the extracted system fonts, the others never", () => {
  const off = GAME_LANGS.filter((l) => optionDisabled(l.code, false)).map((l) => l.code);
  assert.deepEqual(off.sort(), ["ko", "zh-Hans", "zh-Hant"]);
  assert.equal(GAME_LANGS.filter((l) => optionDisabled(l.code, true)).length, 0);
  assert.ok(needsFonts("zh-CN") && needsFonts("zh-TW") && !needsFonts("ja"));
});

test("every language is named in its own language and is a value of the settings schema and of the Rust list", () => {
  assert.equal(gameLangName("fr"), "Français");
  assert.equal(gameLangName("ja"), "日本語");
  assert.equal(gameLangName("zh-Hant"), "繁體中文");
  assert.equal(gameLangName("xx"), "xx");
  const schema = JSON.parse(fs.readFileSync(new URL("../src-tauri/resources/settings-schema.json", import.meta.url), "utf8"));
  const lang = schema.fields.find((f) => f.key === "lang");
  assert.equal(lang.ini_section, "language");
  for (const l of GAME_LANGS) assert.ok(lang.options.includes(l.code), l.code);
  const rust = fs.readFileSync(new URL("../src-tauri/src/gamelang.rs", import.meta.url), "utf8");
  const list = rust.match(/GAME_LANGS: &\[&str\] = &\[([^\]]*)\]/)[1].match(/"[^"]+"/g).map((x) => x.slice(1, -1));
  assert.deepEqual(list, GAME_LANGS.map((l) => l.code));
});
