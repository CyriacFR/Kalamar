import test from "node:test";
import assert from "node:assert/strict";
import { redact } from "./redact.ts";

test("a copied diagnostic carries neither the account name nor a room code", () => {
  const log = [
    "C:\\Users\\Someone\\AppData\\Roaming\\x\\matches\\K7Q2MXA9PL4DZB3W8R6TNY5HCF\\status.json",
    "SPLAT3_LAN_ROOM=K7Q2MXA9PL4DZB3W8R6TNY5HCF",
    "KLM_MATCH_ID=K7Q2MXA9PL4DZB3W8R6TNY5HCF-2",
    "net: joining relay room K7Q2MX as Ana",
    "code: KLM1-ABCDE-FGHJK-LMNPQ-R",
    "relay 203.0.113.7:47313 unreachable (timed out after 20 s)",
  ].join("\n");
  const out = redact(log);
  assert.ok(!out.includes("Someone"), out);
  assert.ok(!out.includes("K7Q2MX"), out);
  assert.ok(!out.includes("ABCDE"), out);
  assert.ok(out.includes("relay 203.0.113.7:47313 unreachable"), "the rest stays readable");
  assert.ok(out.includes("<user>") && out.includes("<room>") && out.includes("KLM1-<code>"));
});

test("error codes that name a room are kept", () => {
  assert.equal(redact("error: room-not-found"), "error: room-not-found");
});
