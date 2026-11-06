import test from "node:test";
import assert from "node:assert/strict";
import { columnOf, groupByColumn, matchesStatus, priorityIcon, reviewLine, ringDash, stepsLine, wantsNewTask } from "./tasks-state.ts";

test("every task status lands in one of the four board columns", () => {
  assert.equal(columnOf({ status: "open" }), "planned");
  assert.equal(columnOf({ status: "planned" }), "planned");
  assert.equal(columnOf({ status: "in_progress" }), "in_progress");
  assert.equal(columnOf({ status: "review" }), "review");
  assert.equal(columnOf({ status: "done" }), "done");
  assert.equal(columnOf({ status: "cancelled" }), "done");
  // Something the hub has not met yet still has a home.
  assert.equal(columnOf({ status: "weird" }), "planned");
});

test("the board keeps the list's order inside each column and shows every column", () => {
  const g = groupByColumn([
    { id: 1, status: "done" },
    { id: 2, status: "open" },
    { id: 3, status: "review" },
    { id: 4, status: "planned" },
    { id: 5, status: "cancelled" },
  ]);
  assert.deepEqual(Object.keys(g), ["planned", "in_progress", "review", "done"]);
  assert.deepEqual(g.planned.map((t) => t.id), [2, 4]);
  assert.deepEqual(g.in_progress, []);
  assert.deepEqual(g.review.map((t) => t.id), [3]);
  assert.deepEqual(g.done.map((t) => t.id), [1, 5]);
});

test("priority arrows: up from 30, right from 10, down below", () => {
  assert.equal(priorityIcon(50), "up");
  assert.equal(priorityIcon(30), "up");
  assert.equal(priorityIcon(29), "right");
  assert.equal(priorityIcon(10), "right");
  assert.equal(priorityIcon(9), "down");
  assert.equal(priorityIcon(0), "down");
});

test("the progress ring's dash is the done arc then the gap, clamped", () => {
  const half = ringDash(1, 2, 10);
  assert.equal(half.circumference, 62.83);
  assert.equal(half.fraction, 0.5);
  assert.equal(half.dash, "31.42 31.42");
  const none = ringDash(0, 3, 10);
  assert.equal(none.dash, "0 62.83");
  const full = ringDash(3, 3, 10);
  assert.equal(full.dash, "62.83 0");
  // More done than planned, or nothing planned: never past the ring.
  assert.equal(ringDash(5, 3, 10).dash, "62.83 0");
  assert.equal(ringDash(0, 0, 10).dash, "0 62.83");
  assert.equal(ringDash(2, 0, 10).fraction, 0);
});

test("the ring's words", () => {
  assert.equal(stepsLine(1, 2), "1 of 2 steps");
  assert.equal(stepsLine(1, 1), "1 of 1 step");
  assert.equal(stepsLine(0, 0), "No steps yet");
});

test("the status filter keeps today's params: active, a status, all", () => {
  assert.equal(matchesStatus({ status: "open" }, "active"), true);
  assert.equal(matchesStatus({ status: "review" }, ""), true);
  assert.equal(matchesStatus({ status: "done" }, "active"), false);
  assert.equal(matchesStatus({ status: "cancelled" }, "active"), false);
  assert.equal(matchesStatus({ status: "cancelled" }, "all"), true);
  assert.equal(matchesStatus({ status: "review" }, "review"), true);
  assert.equal(matchesStatus({ status: "open" }, "review"), false);
});

test("the review line names the priority, the project and the lead", () => {
  assert.equal(reviewLine({ priorityWord: "High", project: "demo-app", lead: "Pip" }), "A high-priority task for demo-app, led by Pip.");
  assert.equal(reviewLine({ priorityWord: "Urgent", project: "demo-app", lead: "Pip" }), "An urgent-priority task for demo-app, led by Pip.");
  assert.equal(reviewLine({ priorityWord: "Normal", project: "demo-app" }), "A task for demo-app, led by its lead agent.");
  assert.equal(reviewLine({ priorityWord: "Low", project: "" }), "A low-priority task, led by its lead agent.");
});

test("?new=1 opens the dialog; 0, false and nothing do not", () => {
  assert.equal(wantsNewTask("1"), true);
  assert.equal(wantsNewTask("true"), true);
  assert.equal(wantsNewTask("0"), false);
  assert.equal(wantsNewTask("false"), false);
  assert.equal(wantsNewTask(""), false);
  assert.equal(wantsNewTask(null), false);
  assert.equal(wantsNewTask(undefined), false);
});
