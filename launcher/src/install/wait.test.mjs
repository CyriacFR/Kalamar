// What is done during an installation, and how an update's length is told (src/install/wait.ts).
import test from "node:test";
import assert from "node:assert/strict";
import { LONG_UPDATE_SECS, progressOf, taskStates, updateClass, updateWhy } from "./wait.ts";

const facts = (over = {}) => ({ gameData: false, hasName: false, profileComplete: false, settings: {}, seen: new Set(), servers: 0, loggedIn: null, ...over });
const by = (states) => Object.fromEntries(states.map((s) => [s.id, s]));

test("the Locker is locked until the merged game data is there, then done with a whole character", () => {
  assert.equal(by(taskStates(facts())).locker.locked, true);
  // a name typed before is kept, but the card stays locked: the looks come from the game's files
  const named = by(taskStates(facts({ hasName: true })));
  assert.equal(named.locker.locked, true);
  assert.equal(named.locker.partial, true);
  const open = by(taskStates(facts({ gameData: true, hasName: true })));
  assert.deepEqual([open.locker.locked, open.locker.done, open.locker.partial], [false, false, true]);
  assert.equal(by(taskStates(facts({ gameData: true, hasName: true, profileComplete: true }))).locker.done, true);
});

test("mouse and keys: from the settings file or from having looked at them", () => {
  assert.equal(by(taskStates(facts({ settings: { mouse_gyro_cm360: "34.5" } }))).controls.partial, true);
  assert.equal(by(taskStates(facts({ settings: { mouse_gyro_cm360: "34.5", bind_zr: "key:mouse_l" } }))).controls.done, true);
  assert.equal(by(taskStates(facts({ settings: { mouse_gyro_sens: "0.05" }, seen: new Set(["set.controls"]) }))).controls.done, true);
  assert.equal(by(taskStates(facts({ settings: null }))).controls.done, false);
});

test("display, server (optional) and the note on the native mode", () => {
  assert.equal(by(taskStates(facts({ settings: { fullscreen: "1" } }))).display.done, true);
  assert.equal(by(taskStates(facts({ seen: new Set(["set.display", "native"]) }))).native.done, true);
  const s = by(taskStates(facts({ servers: 1, loggedIn: false })));
  assert.deepEqual([s.server.done, s.server.partial, s.server.optional], [false, true, true]);
  assert.equal(by(taskStates(facts({ servers: 1, loggedIn: true }))).server.done, true);
  // a relay-only server: nothing to log into
  assert.equal(by(taskStates(facts({ servers: 1, loggedIn: null }))).server.done, true);
});

test("the count leaves the optional server out until it is done", () => {
  assert.deepEqual(progressOf(taskStates(facts())), { done: 0, total: 4 });
  assert.deepEqual(progressOf(taskStates(facts({ servers: 1, loggedIn: true, seen: new Set(["native"]) }))), { done: 2, total: 5 });
});

test("an update says whether it is short, long or a full recompilation, and why", () => {
  const u = (kind, reasons = [], extra = {}) => ({ kind, reasons, changed_files: 3, changed_runtime: 3, changed_generator: 0, lift: false, compile: true, objects: 2, generated_objects: 0, objects_total: 737, lift_secs: 0, compile_secs: 30, compile_secs_max: 30, ...extra });
  assert.equal(updateClass(u("runtime"), 40), "short");
  assert.equal(updateClass(u("runtime"), LONG_UPDATE_SECS + 1), "long");
  assert.equal(updateClass(u("generator"), 300), "long");
  // this morning's case: a header shared by the whole generated code, everything recompiled
  assert.equal(updateClass(u("full", ["headers"]), 700), "full");
  assert.equal(updateWhy(u("full", ["headers"])), "headers");
  assert.equal(updateWhy(u("full", ["lift_missing", "objects_missing"])), "objects_missing");
  assert.equal(updateWhy(u("full", ["toolchain", "headers"])), "toolchain");
  assert.equal(updateWhy(u("generator")), "generator");
  assert.equal(updateWhy(u("runtime")), "runtime");
  assert.equal(updateWhy(null), "runtime");
});
