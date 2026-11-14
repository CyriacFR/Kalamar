// Every interface sound is plain parameters: check them (length, levels, pitches) without any audio device.
import test from "node:test";
import assert from "node:assert/strict";
import { RECIPES, SOUND_NAMES, gainFor, lengthOf, MIN_GAP_MS } from "./recipes.ts";

test("the five sounds exist", () => {
  assert.deepEqual([...SOUND_NAMES].sort(), ["confirm", "error", "found", "soft", "success"]);
});

for (const name of ["confirm", "found", "soft", "success", "error"]) {
  test(`${name}: short, quiet and well-formed`, () => {
    const len = lengthOf(name);
    assert.ok(len >= 0.04 && len < 0.3, `${name} lasts ${len}s`);
    for (const v of RECIPES[name].voices) {
      assert.ok(["sine", "triangle", "noise"].includes(v.kind));
      assert.ok(v.dur > v.attack && v.attack > 0 && v.at >= 0);
      assert.ok(v.from >= 20 && v.to >= 20 && v.from <= 8000 && v.to <= 8000, "audible, safe pitches (exponential ramp needs > 0)");
      assert.ok(v.peak > 0 && v.peak <= 0.8);
    }
  });
}

test("only the 'found' sound may play while the game runs; it is the loudest", () => {
  assert.deepEqual(SOUND_NAMES.filter((n) => RECIPES[n].duringGame), ["found"]);
  const peak = (n) => Math.max(...RECIPES[n].voices.map((v) => v.peak));
  assert.ok(peak("found") >= peak("confirm") && peak("confirm") > peak("soft"));
});

test("volume curve: silent at 0, low by default, bounded at 100, junk is safe", () => {
  assert.equal(gainFor(0), 0);
  assert.ok(gainFor(35) < 0.25 && gainFor(35) > 0.1);
  assert.ok(gainFor(100) <= 0.9 + 1e-9);
  assert.equal(gainFor(NaN), 0);
  assert.equal(gainFor(500), gainFor(100));
  assert.ok(MIN_GAP_MS >= 100 && MIN_GAP_MS <= 200);
});
