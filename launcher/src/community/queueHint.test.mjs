import test from "node:test";
import assert from "node:assert/strict";
import { queueHint } from "./queueHint.ts";

const q = (id, searching, extra = {}) => ({ id, teams: [1, 1], searching, availability: { open: true }, ...extra });

test("two players in two queues of the same format: alone here, the other queue is named", () => {
  const h = queueHint("duel-ranked", [q("duel-ranked", 1), q("duel", 1), q("squad", 3, { teams: [4, 4] })]);
  assert.deepEqual(h, { here: 1, others: [{ id: "duel", n: 1 }], reason: "others" });
});

test("alone with nobody elsewhere, waiting with company, unknown without a count", () => {
  assert.equal(queueHint("duel", [q("duel", 1), q("duel-ranked", 0)]).reason, "alone");
  assert.equal(queueHint("duel", [q("duel", 2)]).reason, "waiting");
  assert.equal(queueHint("duel", [q("duel", null)]).reason, "unknown");
  assert.equal(queueHint("duel", null).reason, "unknown");
});

test("a closed queue and a match that fell through are said first", () => {
  assert.equal(queueHint("duel", [q("duel", 2, { availability: { open: false } })]).reason, "closed");
  assert.equal(queueHint("duel", [q("duel", 2)], "requeued").reason, "requeued");
});
