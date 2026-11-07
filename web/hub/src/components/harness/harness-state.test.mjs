import assert from "node:assert/strict";
import { test } from "node:test";
import { apiStatus, badgeStatus, chipActive, chipOf, elapsedWords, groupCounts, groupOf, matchesStatus, msText, openByDefault, pairSteps, RUN_GROUPS, runSeconds, statusChips, stepSummary, stepTool } from "./harness-state.ts";

const step = (n, kind, extra = {}) => ({ n, at: "", kind, content: "", command: "", exit_code: null, duration_ms: 0, prompt_tokens: 0, completion_tokens: 0, model: "", ...extra });

test("every harness status belongs to exactly one chip", () => {
  const all = ["running", "submitted", "step_limit", "time_limit", "format_error", "provider_error", "not_logged_in", "cancelled", "lost", "held", "resumed"];
  const seen = new Map();
  for (const g of RUN_GROUPS) for (const s of g.statuses) seen.set(s, (seen.get(s) || 0) + 1);
  for (const s of all) assert.equal(seen.get(s), 1, s);
  assert.equal(groupOf("running"), "running");
  assert.equal(groupOf("submitted"), "finished");
  assert.equal(groupOf("step_limit"), "stopped");
  assert.equal(groupOf("provider_error"), "failed");
  assert.equal(groupOf("something_new"), "stopped");
});

test("matchesStatus and chipActive agree on groups and exact statuses", () => {
  assert.equal(matchesStatus("submitted", ""), true);
  assert.equal(matchesStatus("submitted", "finished"), true);
  assert.equal(matchesStatus("cancelled", "finished"), false);
  assert.equal(matchesStatus("cancelled", "stopped"), true);
  assert.equal(matchesStatus("cancelled", "cancelled"), true);
  assert.equal(matchesStatus("lost", "cancelled"), false);
  assert.equal(chipActive("", "running"), false);
  assert.equal(chipActive("failed", "failed"), true);
  assert.equal(chipActive("failed", "stopped"), false);
  // An old link with ?status=cancelled lights the chip that holds it.
  assert.equal(chipActive("cancelled", "stopped"), true);
  assert.equal(chipActive("cancelled", "failed"), false);
});

test("groupCounts tallies rows per chip", () => {
  assert.deepEqual(groupCounts(["running", "submitted", "submitted", "cancelled", "provider_error"]), { running: 1, finished: 2, stopped: 1, failed: 1 });
  assert.deepEqual(groupCounts([]), { running: 0, finished: 0, stopped: 0, failed: 0 });
});

test("stepTool maps step kinds onto tool icons", () => {
  assert.equal(stepTool("command"), "shell");
  assert.equal(stepTool("process"), "shell");
  assert.equal(stepTool("edit"), "edit");
  assert.equal(stepTool("write"), "edit");
  assert.equal(stepTool("read"), "read");
  assert.equal(stepTool("fetch"), "web");
  assert.equal(stepTool("search"), "web");
  assert.equal(stepTool("grep"), "search");
  assert.equal(stepTool("glob"), "search");
  assert.equal(stepTool("assistant"), "model");
  assert.equal(stepTool("plan"), null);
  assert.equal(stepTool("exit"), null);
});

test("pairSteps folds an observation under the call before it, at the same depth", () => {
  const rows = pairSteps([step(1, "assistant", { content: "Let me look." }), step(2, "command", { command: "ls" }), step(3, "observation", { content: "a\nb" }), step(4, "read", { command: "x.ts" }), step(5, "observation", { content: "code" }), step(6, "observation", { content: "stray" }), step(7, "subagent", { command: "helper" }), step(8, "command", { command: "pwd", depth: 1 }), step(9, "observation", { content: "/", depth: 1 }), step(10, "observation", { content: "report", depth: 0 }), step(11, "exit", { content: "submitted" })]);
  assert.deepEqual(
    rows.map((r) => [r.step.n, r.output?.n ?? null]),
    [
      [1, null],
      [2, 3],
      [4, 5],
      [6, null],
      [7, null],
      [8, 9],
      [10, null],
      [11, null],
    ],
  );
});

test("stepSummary shows the command, the request, or the first line", () => {
  assert.equal(stepSummary(step(1, "command", { command: "cargo test\n--workspace" })), "cargo test");
  assert.equal(stepSummary(step(1, "read", { content: "src/lib.rs" })), "src/lib.rs");
  assert.equal(stepSummary(step(1, "assistant", { content: "\n\nFirst words.\nMore." })), "First words.");
  assert.equal(stepSummary(step(1, "plan", { content: "plan: 1/3 done\n- [x] a" })), "plan: 1/3 done");
  assert.equal(stepSummary(step(1, "note", { content: "x".repeat(200) })).length, 160);
  assert.equal(stepSummary(step(1, "exit", { content: "" })), "");
});

test("msText and runSeconds read durations the way the owner does", () => {
  assert.equal(msText(0), "");
  assert.equal(msText(420), "420 ms");
  assert.equal(msText(3200), "3 s");
  assert.equal(msText(125000), "2 min 5 s");
  assert.equal(msText(120000), "2 min");
  assert.equal(msText(3_720_000), "1 h 2 min");
  assert.equal(msText(47 * 3_600_000), "47 h 0 min");
  assert.equal(msText(53 * 3_600_000), "2 days 5 h");
  assert.equal(msText(30 * 24 * 3_600_000), "30 days");
  const now = Date.parse("2026-11-06T10:00:00Z");
  assert.equal(runSeconds("2026-11-06T09:59:00Z", "", now), 60);
  assert.equal(runSeconds("2026-11-06T09:59:00Z", "2026-11-06T09:59:30Z", now), 30);
  assert.equal(runSeconds("", "", now), 0);
  assert.equal(runSeconds("2026-11-06T10:00:00Z", "2026-11-06T10:00:00Z", now), 1);
});

test("openByDefault unfolds the model's words and the ending, folds tool calls", () => {
  assert.equal(openByDefault({ kind: "assistant" }), true);
  assert.equal(openByDefault({ kind: "exit" }), true);
  assert.equal(openByDefault({ kind: "command" }), false);
  assert.equal(openByDefault({ kind: "read" }), false);
});

test("badgeStatus hands StatusBadge a word it knows for every harness status", () => {
  assert.equal(badgeStatus("held"), "paused");
  assert.equal(badgeStatus("resumed"), "superseded");
  assert.equal(badgeStatus("not_logged_in"), "error");
  assert.equal(badgeStatus("format_error"), "format_errors");
  assert.equal(badgeStatus("submitted"), "submitted");
  assert.equal(badgeStatus("brand_new"), "brand_new");
});

test("chipOf and statusChips count the runs at hand per chip, in page order", () => {
  assert.equal(chipOf({ status: "running" }), "running");
  assert.equal(chipOf({ status: "submitted" }), "finished");
  assert.equal(chipOf({ status: "cancelled" }), "stopped");
  assert.equal(chipOf({ status: "not_logged_in" }), "failed");
  const runs = [{ status: "running" }, { status: "submitted" }, { status: "submitted" }, { status: "step_limit" }, { status: "provider_error" }];
  assert.deepEqual(statusChips(runs), [
    { key: "running", label: "Running", count: 1 },
    { key: "finished", label: "Finished", count: 2 },
    { key: "stopped", label: "Stopped early", count: 1 },
    { key: "failed", label: "Failed", count: 1 },
  ]);
  assert.deepEqual(
    statusChips([]).map((c) => c.count),
    [0, 0, 0, 0],
  );
});

test("apiStatus asks the hub for one exact status only when the chip holds one", () => {
  assert.equal(apiStatus(""), "");
  assert.equal(apiStatus("running"), "running");
  assert.equal(apiStatus("finished"), "submitted");
  assert.equal(apiStatus("stopped"), "");
  assert.equal(apiStatus("failed"), "");
  assert.equal(apiStatus("cancelled"), "cancelled");
});

test("elapsedWords ticks a live row's time so far", () => {
  const now = Date.parse("2026-11-06T10:00:00Z");
  assert.equal(elapsedWords(undefined, now), "");
  assert.equal(elapsedWords("not a date", now), "");
  assert.equal(elapsedWords("2026-11-06T10:00:00Z", now), "0s");
  assert.equal(elapsedWords("2026-11-06T09:59:18Z", now), "42s");
  assert.equal(elapsedWords("2026-11-06T09:56:53Z", now), "3m 07s");
  assert.equal(elapsedWords("2026-11-06T08:57:55Z", now), "1h 02m 05s");
  assert.equal(elapsedWords("2026-11-04T10:00:01Z", now), "47h 59m 59s");
  // Past two days the clock reads in days, not "1363h 24m 40s".
  assert.equal(elapsedWords("2026-11-04T10:00:00Z", now), "2 days");
  assert.equal(elapsedWords("2026-11-04T05:00:00Z", now), "2 days 5 h");
  assert.equal(elapsedWords("2026-09-09T18:35:20Z", now), "57 days");
  // A browser clock behind the server never reads negative.
  assert.equal(elapsedWords("2026-11-06T10:00:30Z", now), "0s");
});
