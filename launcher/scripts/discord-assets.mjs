// Writes the pictures of the Discord presence the maintainers upload to the Discord application (docs/discord-presence.md):
// docs/discord-assets/<key>.svg and <key>.png, 1024x1024, transparent outside the drawing. `kalamar_logo` is the
// launcher's icon (app-icon.svg); the act_* pictures are the drawings of src/discord/art.ts, the same the Settings
// preview shows. The PNGs are rendered by the installed Microsoft Edge (or Chrome) in headless mode, from the local
// SVG files: nothing is downloaded.
//
//   node scripts/discord-assets.mjs
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { GLYPHS, glyphSvg } from "../src/discord/art.ts";

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const out = path.join(root, "docs", "discord-assets");
const SIZE = 1024;
fs.mkdirSync(out, { recursive: true });

const files = {};
const logo = fs.readFileSync(path.join(root, "app-icon.svg"), "utf8").replace(/width="512" height="512"/, `width="${SIZE}" height="${SIZE}"`);
files.kalamar_logo = logo;
for (const key of Object.keys(GLYPHS)) files[key] = glyphSvg(key, SIZE);

const browsers = [
  "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
  "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
  "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
];
const exe = browsers.find((p) => fs.existsSync(p));
if (!exe) throw new Error("no Edge or Chrome found");
const profile = fs.mkdtempSync(path.join(os.tmpdir(), "kalamar-assets-profile-"));

for (const [key, svg] of Object.entries(files)) {
  const svgPath = path.join(out, `${key}.svg`);
  fs.writeFileSync(svgPath, svg + "\n");
  // A page that holds the SVG at its exact size, on a transparent background.
  const page = path.join(profile, `${key}.html`);
  fs.writeFileSync(page, `<!doctype html><html><body style="margin:0;background:transparent"><img src="${pathToFileURL(svgPath)}" width="${SIZE}" height="${SIZE}" style="display:block"></body></html>`);
  const png = path.join(out, `${key}.png`);
  const r = spawnSync(exe, [
    "--headless=new", `--user-data-dir=${profile}`, "--no-first-run", "--no-default-browser-check", "--disable-gpu", "--hide-scrollbars",
    "--force-device-scale-factor=1", "--default-background-color=00000000", `--window-size=${SIZE},${SIZE}`, `--screenshot=${png}`, pathToFileURL(page).href,
  ], { stdio: "ignore", timeout: 60_000 });
  if (r.status !== 0 || !fs.existsSync(png)) throw new Error(`${key}: the browser did not write the picture`);
  console.log(`${key}.png`);
}
fs.rmSync(profile, { recursive: true, force: true });
