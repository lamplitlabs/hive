import assert from "node:assert/strict";
import { test } from "node:test";
import { RETIRE_REASON, isRetireReason, prunedText, prunedWord, workersLine } from "./agents.ts";

test("a retired worker's row reads Retired, a stale one Removed", () => {
  assert.equal(RETIRE_REASON, "retired after its run; workers are not kept idle");
  assert.equal(isRetireReason(RETIRE_REASON), true);
  assert.equal(isRetireReason("Retired after its run"), true);
  assert.equal(isRetireReason("no useful output since 2026-08-20 (limit 14 days)"), false);
  assert.equal(isRetireReason(""), false);
  assert.equal(isRetireReason(undefined), false);

  assert.equal(prunedWord(RETIRE_REASON), "Retired");
  assert.equal(prunedWord("no useful output since 2026-08-20 (limit 14 days)"), "Removed");
  assert.equal(prunedWord(null), "Removed");
});

test("prunedText names the retire and the stale-prune cases", () => {
  assert.equal(prunedText(RETIRE_REASON), "Retired after its run");
  assert.equal(prunedText("no useful output since 2026-08-20 (limit 14 days)"), "Removed after 14 days without useful output");
  assert.equal(prunedText("no useful output since 2026-08-20 (limit 1 day)"), "Removed after 1 days without useful output");
  assert.equal(prunedText("manual cleanup"), "Removed: manual cleanup");
});

test("workersLine counts the workers that exist and never claims a pool", () => {
  const runner = (workers) => ({ available: true, workers: { pool: 0, idle: 0, busy: 0, idle_names: [], prune_after_days: 14, ...workers } });
  assert.equal(workersLine(runner({ pool: 3, busy: 2, idle: 1 })), "3 workers active, 1 between runs.");
  assert.equal(workersLine(runner({ pool: 1, busy: 1, idle: 0 })), "1 worker active.");
  assert.equal(workersLine(runner({ pool: 2, busy: 0, idle: 2 })), "2 workers active, 2 between runs.");
  assert.equal(workersLine(runner({ pool: 0 })), "");
  assert.equal(workersLine({ available: false, workers: { pool: 2, idle: 0, busy: 2, idle_names: [], prune_after_days: 14 } }), "");
  assert.equal(workersLine(undefined), "");
  assert.doesNotMatch(workersLine(runner({ pool: 3, busy: 2, idle: 1 })), /pool|keeps|ready/);
});
