import test from "node:test";
import assert from "node:assert/strict";
import { acted, actingTeams, draftWanted, headline, log, markOf, needed, selectable, toggle, turnClock, usable } from "./draftState.ts";

const base = (over = {}) => ({
  version: 1, state: "running", decider: "captain", turn_s: 20, current: 1,
  options: [
    { id: "s1", kind: "stage", state: "banned", by: "alpha", turn: 0 },
    { id: "s2", kind: "stage", state: "available" },
    { id: "s3", kind: "stage", state: "unavailable", reason: "game" },
    { id: "s4", kind: "stage", state: "teleported" },
  ],
  turns: [
    { index: 0, action: "ban", kind: "stage", teams: ["alpha"], count: 1, state: "done", choices: [{ team: "alpha", ids: ["s1"], by: "players" }], acted: [] },
    { index: 1, action: "ban", kind: "stage", teams: ["bravo"], count: 1, state: "current", started_at: "2026-11-11T20:00:00Z", deadline: "2026-11-11T20:00:20Z", choices: [], acted: [] },
    { index: 2, action: "random", kind: "stage", teams: [], count: 1, state: "pending", choices: [], acted: [] },
  ],
  teams: { alpha: { players: ["a"], captain: "a" }, bravo: { players: ["b"], captain: "b" } },
  you: { team: "bravo", role: "captain", can_act: true, choice: null },
  ...over,
});

test("only a version 1 draft is drawn; the screen stays while the game is not on its way", () => {
  assert.equal(usable(base()), true);
  assert.equal(usable({ ...base(), version: 2 }), false);
  assert.equal(draftWanted({ state: "drafting", draft: base() }), true);
  assert.equal(draftWanted({ state: "ready", draft: { ...base(), state: "done" } }), true);
  assert.equal(draftWanted({ state: "ready", draft: null }), false);
  assert.equal(draftWanted({ state: "running", draft: { ...base(), state: "done" } }), false);
});

test("whose turn it is, from the player's side", () => {
  assert.deepEqual(headline(base()), { kind: "you", action: "ban", what: "stage", n: 1, vote: false });
  assert.equal(headline(base({ you: { team: "alpha", role: "captain", can_act: false } })).kind, "them");
  assert.equal(headline(base({ you: { team: "bravo", role: "none", can_act: false } })).kind, "team");
  assert.equal(headline(base({ current: 2 })).kind, "server");
  assert.equal(headline(base({ state: "done", current: null })).kind, "done");
  const both = base({ turns: [{ action: "ban", kind: "stage", teams: ["alpha", "bravo"], count: 2, state: "current", acted: ["bravo"] }], current: 0, you: { team: "bravo", can_act: false } });
  assert.deepEqual(headline(both), { kind: "hidden", action: "ban", what: "stage", n: 2, acted: true });
  assert.deepEqual(acted(both), ["bravo"]);
  assert.deepEqual(actingTeams(both), ["alpha", "bravo"]);
  assert.equal(needed(both), 2);
});

test("only available options of the turn's kind can be chosen, and only on one's turn", () => {
  const d = base();
  assert.equal(selectable(d, d.options[1]), true);
  assert.equal(selectable(d, d.options[0]), false, "banned");
  assert.equal(selectable(d, d.options[2]), false, "unavailable");
  assert.equal(selectable(d, { id: "m1", kind: "mode", state: "available" }), false, "another kind");
  assert.equal(selectable(base({ you: { team: "bravo", can_act: false } }), d.options[1]), false);
  assert.equal(markOf(d.options[3]), "unavailable", "an unknown state");
});

test("a click toggles, replaces with one option, stops at the count with several", () => {
  assert.deepEqual(toggle([], "s1", 1), ["s1"]);
  assert.deepEqual(toggle(["s1"], "s2", 1), ["s2"]);
  assert.deepEqual(toggle(["s1"], "s1", 1), []);
  assert.deepEqual(toggle(["s1"], "s2", 2), ["s1", "s2"]);
  assert.deepEqual(toggle(["s1", "s2"], "s3", 2), ["s1", "s2"]);
});

test("the ring counts down from the turn's start to its deadline", () => {
  const c = turnClock(base(), Date.parse("2026-11-11T20:00:15Z"));
  assert.deepEqual(c, { left: 5, fraction: 0.25 });
  assert.deepEqual(turnClock(base(), Date.parse("2026-11-11T20:00:30Z")), { left: 0, fraction: 0 });
  assert.equal(turnClock(base({ current: null }), 0), null);
});

test("the log has one line per team's choice, skipped turns included", () => {
  const d = base({
    turns: [
      ...base().turns.slice(0, 1),
      { index: 1, action: "ban", kind: "stage", teams: ["bravo"], count: 1, state: "done", choices: [{ team: "bravo", ids: ["s2"], by: "timeout" }] },
      { index: 2, action: "ban", kind: "stage", teams: ["alpha"], count: 0, state: "skipped", choices: [] },
      { index: 3, action: "random", kind: "stage", teams: [], count: 1, state: "done", choices: [{ team: null, ids: ["s4"], by: "server" }] },
    ],
  });
  const l = log(d);
  assert.deepEqual(l.map((x) => [x.index, x.team, x.by, x.ids.join(), x.skipped]), [
    [0, "alpha", "players", "s1", false],
    [1, "bravo", "timeout", "s2", false],
    [2, "alpha", "server", "", true],
    [3, null, "server", "s4", false],
  ]);
});
