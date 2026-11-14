// Screenshots of the launcher's screens in the browser preview (vite --mode mock, src/dev/mock/), taken with the
// installed Microsoft Edge (or Chrome) in headless mode through the DevTools protocol. Nothing is downloaded.
//
//   npx vite --mode mock --port 1430           (in another terminal)
//   node scripts/ui-shots.mjs --base http://localhost:1430 --out <folder> [--only <regexp>] [--lang fr,es] [--mode simple,avance]
//
// The list of screens is in scripts/ui-shots.list.mjs: each one is a URL of the mock (the situation) plus optional
// clicks. Files are written as <out>/<mode>/<lang>/<name>-<width>.png.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { shots } from "./ui-shots.list.mjs";

const arg = (k, d) => {
  const i = process.argv.indexOf(`--${k}`);
  return i > 0 ? process.argv[i + 1] : d;
};
const base = arg("base", "http://localhost:1430");
const out = arg("out", path.join(os.tmpdir(), "kalamar-shots"));
const only = arg("only", null) ? new RegExp(arg("only")) : null;
const langs = arg("lang", "en").split(",");
const modes = arg("mode", "simple,avance").split(",");
const widths = arg("widths", "1280x800,1000x720").split(",").map((s) => s.split("x").map(Number));
const port = Number(arg("port", 9300 + Math.floor(Math.random() * 400)));

const browsers = [
  "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
  "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
  "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
];
const exe = browsers.find((p) => fs.existsSync(p));
if (!exe) throw new Error("no Edge or Chrome found");
const profile = fs.mkdtempSync(path.join(os.tmpdir(), "kalamar-shots-profile-"));
const browser = spawn(exe, [
  "--headless=new", `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, "--no-first-run", "--no-default-browser-check",
  "--hide-scrollbars", "--force-device-scale-factor=1", "--window-size=1280,800", "about:blank",
], { stdio: "ignore" });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function json(url, method = "GET") {
  for (let i = 0; i < 100; i++) {
    try {
      const r = await fetch(url, { method });
      if (r.ok) return await r.json();
    } catch { /* not up yet */ }
    await sleep(100);
  }
  throw new Error(`no answer from ${url}`);
}

let ws, seq = 0;
const pending = new Map();
const waiters = [];
function send(method, params = {}) {
  const id = ++seq;
  ws.send(JSON.stringify({ id, method, params }));
  return new Promise((resolve, reject) => pending.set(id, { resolve, reject, method }));
}
function waitEvent(name, ms = 8000) {
  return new Promise((resolve) => {
    const t = setTimeout(() => resolve(null), ms);
    waiters.push({ name, done: (p) => { clearTimeout(t); resolve(p); } });
  });
}
async function evaluate(expression) {
  const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(`${expression}\n  -> ${r.exceptionDetails.exception?.description ?? r.exceptionDetails.text}`);
  return r.result?.value;
}

async function main() {
  await json(`http://127.0.0.1:${port}/json/version`);
  const list = await json(`http://127.0.0.1:${port}/json/list`);
  const page = list.find((t) => t.type === "page");
  ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((r) => ws.addEventListener("open", r, { once: true }));
  ws.addEventListener("message", (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id && pending.has(msg.id)) {
      const p = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) p.reject(new Error(`${p.method}: ${msg.error.message}`));
      else p.resolve(msg.result);
    } else if (msg.method) {
      for (let i = waiters.length - 1; i >= 0; i--) if (waiters[i].name === msg.method) { waiters[i].done(msg.params); waiters.splice(i, 1); }
      if (msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error") console.log("  console:", msg.params.args.map((a) => a.value ?? a.description).join(" "));
      if (msg.method === "Runtime.exceptionThrown") console.log("  page error:", msg.params.exceptionDetails.exception?.description);
    }
  });
  await send("Page.enable");
  await send("Runtime.enable");
  await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: "reduce" }] });

  let n = 0;
  for (const mode of modes) {
    for (const lang of langs) {
      for (const s of shots) {
        if (only && !only.test(s.name)) continue;
        if (s.modes && !s.modes.includes(mode)) continue;
        if (s.langs && !s.langs.includes(lang)) continue;
        for (const [w, h] of s.sizes ?? widths) {
          const dir = path.join(out, mode, lang);
          fs.mkdirSync(dir, { recursive: true });
          const file = path.join(dir, `${s.name}-${w}.png`);
          await send("Emulation.setDeviceMetricsOverride", { width: w, height: h, deviceScaleFactor: 1, mobile: false });
          const url = `${base}/?lang=${lang}${mode === "avance" ? "&adv=1" : ""}${s.q ? `&${s.q}` : ""}`;
          const loaded = waitEvent("Page.loadEventFired");
          await send("Page.navigate", { url });
          await loaded;
          await evaluate(`new Promise((ok) => { const t0 = Date.now(); const f = () => (document.querySelector('#root')?.children.length || Date.now() - t0 > 5000) ? ok(true) : setTimeout(f, 50); f(); })`);
          await evaluate("document.fonts.ready.then(() => true)");
          await sleep(s.wait ?? 700);
          try {
            for (const step of s.steps ?? []) {
              await evaluate(step);
              await sleep(s.stepWait ?? 450);
            }
          } catch (e) {
            console.log(`  ! ${s.name}: ${e.message}`);
          }
          if (s.full) {
            await evaluate(`(() => { const st = document.createElement('style'); st.textContent = 'html,body,#root,.shell{height:auto!important;overflow:visible!important}.main{overflow:visible!important}.rail{position:sticky;top:0;height:100vh}'; document.head.appendChild(st); return true; })()`);
            await sleep(200);
            const full = await evaluate("Math.max(document.documentElement.scrollHeight, document.body.scrollHeight)");
            await send("Emulation.setDeviceMetricsOverride", { width: w, height: Math.min(Math.max(h, full), 6000), deviceScaleFactor: 1, mobile: false });
            await sleep(250);
          }
          const shot = await send("Page.captureScreenshot", { format: "png" });
          fs.writeFileSync(file, Buffer.from(shot.data, "base64"));
          n++;
          console.log(`${mode}/${lang}/${s.name}-${w}.png`);
        }
      }
    }
  }
  console.log(`${n} screenshots in ${out}`);
}

main()
  .catch((e) => { console.error(e); process.exitCode = 1; })
  .finally(() => {
    try { ws?.close(); } catch { /* closed */ }
    browser.kill();
    setTimeout(() => { try { fs.rmSync(profile, { recursive: true, force: true }); } catch { /* still locked */ } }, 500);
  });
