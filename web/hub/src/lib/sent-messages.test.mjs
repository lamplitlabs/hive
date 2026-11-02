import assert from "node:assert/strict";
import { test } from "node:test";
import {
  SENT_TTL_MS,
  queuedBesides,
  reconcileSent,
  sentMessage,
  sentShowsNow,
  sentWords,
} from "./sent-messages.ts";

const idle = { status: "idle", queued: [] };
const running = { status: "running", queued: [] };

test("the words of a sent message are the owner's, or the hub's line for files alone", () => {
  assert.equal(sentWords("  fix the footer ", 0), "fix the footer");
  assert.equal(sentWords("", 1), "Look at the attached file.");
  assert.equal(sentWords("  ", 3), "Look at the attached files.");
  assert.equal(sentWords("", 0), "");
  const s = sentMessage("hello", [], 1000);
  assert.equal(s.text, "hello");
  assert.equal(s.at, 1000);
  assert.notEqual(sentMessage("hello", [], 1000).key, s.key, "keys are unique");
  assert.equal(s.steered, undefined, "a plain send is not marked steered");
  assert.equal(sentMessage("hello", [], 1000, true).steered, true, "a steer is");
});

test("a message sent to an idle agent shows at once; behind a turn the hub lists it as queued", () => {
  assert.equal(sentShowsNow("idle"), true);
  assert.equal(sentShowsNow("closed"), true);
  assert.equal(sentShowsNow("ended"), true);
  assert.equal(sentShowsNow(""), true);
  assert.equal(sentShowsNow("running"), false);
  assert.equal(sentShowsNow("waiting"), false);
});

test("a sent message leaves once its row arrives", () => {
  const s = sentMessage("fix the footer", [], 1000);
  const kept = reconcileSent([s], [{ role: "user", content: "fix the footer" }], running, 1500);
  assert.deepEqual(kept, []);
  // An agent's words, even the same, are not its row.
  assert.deepEqual(reconcileSent([s], [{ role: "assistant", content: "fix the footer" }], running, 1500), [s]);
  // A steered message's row (written mid-turn) settles it the same way.
  const st = sentMessage("go left", [], 1000, true);
  assert.deepEqual(reconcileSent([st], [{ role: "user", content: "go left" }], running, 1500), []);
  assert.deepEqual(reconcileSent([st], [], running, 1500), [st]);
});

test("a poll sent before the message cannot settle it", () => {
  const s = sentMessage("fix the footer", [], 1000);
  assert.deepEqual(reconcileSent([s], [], idle, 900), [s]);
});

test("a sent message stays while the turn runs and goes when the turn is over without it", () => {
  const s = sentMessage("fix the footer", [], 1000);
  assert.deepEqual(reconcileSent([s], [], running, 1500), [s]);
  assert.deepEqual(reconcileSent([s], [], { status: "waiting", queued: [] }, 1500), [s]);
  assert.deepEqual(reconcileSent([s], [], { status: "idle", queued: ["fix the footer"] }, 1500), [s]);
  assert.deepEqual(reconcileSent([s], [], idle, 1500), [], "idle with nothing queued: the hub dropped it");
});

test("a sent message is given up after the TTL even with the turn still running", () => {
  const s = sentMessage("fix the footer", [], 1000);
  assert.deepEqual(reconcileSent([s], [], running, 1000 + SENT_TTL_MS), [s]);
  assert.deepEqual(reconcileSent([s], [], running, 1001 + SENT_TTL_MS), []);
});

test("reconcile keeps the same array when nothing changes", () => {
  const list = [sentMessage("a", [], 1000), sentMessage("b", [], 1100)];
  assert.equal(reconcileSent(list, [], running, 1200), list);
  assert.equal(reconcileSent([], [], idle, 1200).length, 0);
});

test("the queued list drops what the page shows as sent", () => {
  const s = sentMessage("fix the footer", [], 1000);
  assert.deepEqual(queuedBesides(["fix the footer", "and the header"], [s]), ["and the header"]);
  const same = ["and the header"];
  assert.equal(queuedBesides(same, [s]), same);
  assert.equal(queuedBesides(same, []), same);
  // The steered list is deduped the same way against a steer just shown.
  const st = sentMessage("go left", [], 1000, true);
  assert.deepEqual(queuedBesides(["go left", "then right"], [st]), ["then right"]);
});
