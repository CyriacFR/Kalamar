// Every language must carry exactly the keys of the English reference, with the same {variables} and the same
// "|" separators (the install wizard splits some texts on them). The type checker enforces the keys too; this test
// also covers the variables and runs without a build.
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const ts = require("typescript");
const dir = path.dirname(fileURLToPath(import.meta.url));
const SECTIONS = ["app", "community", "servers", "install"];
const codes = fs.readdirSync(dir, { withFileTypes: true }).filter((d) => d.isDirectory()).map((d) => d.name);

function load(code, section) {
  const src = fs.readFileSync(path.join(dir, code, `${section}.ts`), "utf8");
  const js = ts.transpileModule(src, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 } }).outputText;
  const m = { exports: {} };
  new Function("module", "exports", js)(m, m.exports);
  return m.exports[section];
}
const vars = (s) => [...s.matchAll(/\{(\w+)\}/g)].map((x) => x[1]).sort().join(",");
const bars = (s) => s.split("|").length;

test("the language folders are the ones registered in index.ts", () => {
  const index = fs.readFileSync(path.join(dir, "index.ts"), "utf8");
  for (const c of codes) assert.ok(index.includes(`code: "${c}"`), `${c} is not registered in src/locales/index.ts`);
  assert.ok(codes.includes("fr") && codes.includes("en"));
});

for (const section of SECTIONS) {
  const ref = load("en", section);
  for (const code of codes.filter((c) => c !== "en")) {
    test(`${code}/${section} matches the English keys and variables`, () => {
      const d = load(code, section);
      const missing = Object.keys(ref).filter((k) => !(k in d));
      const extra = Object.keys(d).filter((k) => !(k in ref));
      assert.deepEqual(missing, [], `keys missing in ${code}/${section}`);
      assert.deepEqual(extra, [], `unknown keys in ${code}/${section}`);
      const bad = [];
      for (const k of Object.keys(ref)) {
        if (typeof d[k] !== "string" || d[k].trim() === "") bad.push(`${k}: empty`);
        else if (vars(d[k]) !== vars(ref[k])) bad.push(`${k}: variables {${vars(d[k])}} instead of {${vars(ref[k])}}`);
        else if (bars(d[k]) !== bars(ref[k])) bad.push(`${k}: "|" separators differ`);
      }
      assert.deepEqual(bad, []);
    });
  }
}
