import test from "node:test";
import assert from "node:assert/strict";
import { cleanText } from "./cleanText.ts";

test("bidirectional overrides, isolates and marks are removed from every server string", () => {
  // U+202E would make "Kalamar ‮laiciffo" read as an official name.
  const pushed = { type: "room.invite", data: { from: { display_name: "Kalamar ‮laiciffo" }, notes: ["a⁦b⁩c", "‏x‎", "﻿y​z"], n: 3, ok: true, none: null } };
  const clean = cleanText(pushed);
  assert.equal(clean.data.from.display_name, "Kalamar laiciffo");
  assert.deepEqual(clean.data.notes, ["abc", "x", "yz"]);
  assert.equal(clean.data.n, 3);
  assert.equal(clean.data.ok, true);
  assert.equal(clean.data.none, null);
  assert.equal(cleanText({ ["k‮ey"]: 1 })["key"], 1);
});

test("joiners that emoji and scripts need are kept, and right-to-left letters too", () => {
  assert.equal(cleanText("\u{1F469}‍\u{1F469}"), "\u{1F469}‍\u{1F469}");
  assert.equal(cleanText("می‌خواهم"), "می‌خواهم");
  assert.equal(cleanText("שלום"), "שלום");
});
