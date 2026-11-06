import assert from "node:assert/strict";
import { test } from "node:test";
import { isFor, noneText, sortRows, splitBar, validSort, validStatus } from "./votes-state.ts";

test("splitBar: votes short of quorum leave a gap before the tick", () => {
  const b = splitBar(1, 0, 2);
  assert.equal(b.for, 50);
  assert.equal(b.against, 0);
  assert.equal(b.tick, 100);
  assert.equal(b.enough, false);
  assert.equal(b.label, "1 for, 0 against (2 needed)");
});

test("splitBar: votes past quorum fill the bar and the tick sits inside it", () => {
  const b = splitBar(3, 1, 2);
  assert.equal(b.for, 75);
  assert.equal(b.against, 25);
  assert.equal(b.tick, 50);
  assert.equal(b.enough, true);
  assert.equal(b.total, 4);
});

test("splitBar: no quorum means no tick; nothing cast is an empty bar", () => {
  assert.equal(splitBar(2, 2).tick, null);
  assert.equal(splitBar(2, 2).for, 50);
  const empty = splitBar(0, 0, 3);
  assert.equal(empty.for, 0);
  assert.equal(empty.against, 0);
  assert.equal(empty.tick, 100);
  assert.equal(empty.enough, false);
  assert.equal(splitBar(0, 0).total, 1);
});

test("splitBar never goes negative and rounds to a tenth", () => {
  const b = splitBar(-1, 2, 3);
  assert.equal(b.for, 0);
  assert.equal(b.against, 66.7);
  assert.equal(b.tick, 100);
});

test("the status and sort filters only accept the store's words", () => {
  assert.equal(validStatus("needs votes"), "needs votes");
  assert.equal(validStatus("bogus"), "");
  assert.equal(validStatus(null), "");
  assert.equal(validSort("expiry"), "expiry");
  assert.equal(validSort("x"), "queue");
  assert.equal(noneText("rejected", "suggestion"), "No suggestion was turned down");
  assert.equal(noneText("", "rule change"), "No rule change");
});

test("sortRows: queue keeps the order, the others sort by their field", () => {
  const rows = [
    { id: "a", date: "2026-01-02", score: 1, voters: 1, quorum: 3, expires: 30 },
    { id: "b", date: "2026-01-03", score: 2, voters: 2, quorum: 3, expires: 10 },
    { id: "c", date: "2026-01-01", score: 0, voters: 3, quorum: 3, expires: null },
  ];
  assert.deepEqual(sortRows(rows, "queue").map((r) => r.id), ["a", "b", "c"]);
  assert.deepEqual(sortRows(rows, "quorum").map((r) => r.id), ["c", "b", "a"]);
  assert.deepEqual(sortRows(rows, "expiry").map((r) => r.id), ["b", "a", "c"]);
  assert.deepEqual(sortRows(rows, "newest").map((r) => r.id), ["b", "a", "c"]);
  assert.deepEqual(rows.map((r) => r.id), ["a", "b", "c"], "the input is left alone");
});

test("isFor reads the board's vote words", () => {
  for (const v of ["up", "yes", "+1", "for", "UP"]) assert.equal(isFor(v), true, v);
  for (const v of ["down", "no", "-1", "", undefined]) assert.equal(isFor(v), false, String(v));
});
