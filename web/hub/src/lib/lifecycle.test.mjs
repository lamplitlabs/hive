import assert from "node:assert/strict";
import { test } from "node:test";
import { markView, phaseColumns, phaseLabel, phaseLine, phasesSummary, progressLine, shortJob, stepViews, waitsOnWord } from "./lifecycle.ts";

// A task in Build, as GET tasks/{id}/lifecycle returns it.
const building = {
  task: "TASK-141",
  phase: "build",
  waits_on: "agent",
  note: "job-20261126T134608Z-6d882d is at work",
  rework: false,
  steps: [
    { phase: "intake", state: "passed", at: "2026-11-26T13:41:22Z", left: [{ kind: "task", id: "TASK-141", at: "2026-11-26T13:41:22Z" }, { kind: "origin", id: "chat:c-42" }], gate: "a lead takes the task up" },
    { phase: "shape", state: "passed", at: "2026-11-26T13:45:00Z", left: [{ kind: "lead", id: "job-20261126T134500Z-aaaaaa", note: "done", session: "run-lead" }], gate: "a job exists for the task" },
    { phase: "build", state: "current", at: "2026-11-26T13:46:08Z", left: [{ kind: "job", id: "job-20261126T134608Z-6d882d", note: "leased", session: "run-20261126T154346Z-237c5a" }], gate: "every step's run has ended" },
    { phase: "verify", state: "ahead", left: [], gate: "each step's own commits passed the check" },
    { phase: "review", state: "ahead", left: [], gate: "the reviewer says merge" },
    { phase: "deliver", state: "ahead", left: [], gate: "the pull request is merged" },
  ],
  phases: [
    { phase: "intake", label: "Intake", gate: "" },
    { phase: "shape", label: "Shape and plan", gate: "" },
  ],
};

test("phaseLine names the phase and who the task waits on", () => {
  assert.equal(phaseLine(building), "Build · an agent is at work");
  assert.equal(phaseLine({ phase: "review", waits_on: "owner" }), "Review · waits on you");
  assert.equal(phaseLine({ phase: "done", waits_on: "nobody" }), "Done");
  assert.equal(phaseLabel("shape"), "Shape and plan");
  assert.equal(phaseLabel("shape", [{ phase: "shape", label: "Plan", gate: "" }]), "Plan");
  assert.equal(phaseLabel("someday"), "someday");
  assert.equal(waitsOnWord("martians"), "");
});

test("stepViews keeps the order, the states and every mark", () => {
  const v = stepViews(building);
  assert.deepEqual(v.map((s) => s.phase), ["intake", "shape", "build", "verify", "review", "deliver"]);
  assert.deepEqual(v.map((s) => s.stateWord), ["Passed", "Passed", "Now", "Ahead", "Ahead", "Ahead"]);
  assert.equal(v[1].label, "Shape and plan");
  assert.equal(v[2].marks[0].text, "job 6d882d");
  assert.equal(v[2].marks[0].note, "at work");
  assert.deepEqual(v[2].marks[0].link, { to: "run", id: "run-20261126T154346Z-237c5a" });
  assert.equal(v[3].empty, "Nothing yet.");
  assert.equal(v[2].empty, "");
  assert.equal(progressLine(building), "Passed 2 of 6 phases");
  assert.equal(progressLine({ ...building, phase: "done" }), "Done: every phase passed");
  assert.equal(progressLine({ ...building, phase: "cancelled" }), "Cancelled after 2 of 6 phases");
  const skipped = stepViews({ ...building, steps: [{ phase: "review", state: "skipped", left: [], gate: "" }] });
  assert.equal(skipped[0].empty, "Passed over.");
});

test("markView makes every id a link where one exists", () => {
  assert.deepEqual(markView({ kind: "task", id: "TASK-1" }).link, { to: "task", id: "TASK-1" });
  const origin = markView({ kind: "origin", id: "chat:c-42" });
  assert.equal(origin.text, "from a conversation");
  assert.deepEqual(origin.link, { to: "chat", id: "c-42" });
  assert.equal(markView({ kind: "origin", id: "job:job-20261126T134608Z-abc123" }).text, "from job abc123");
  assert.equal(markView({ kind: "origin", id: "suggestion:SUG-007" }).text, "from SUG-007");
  const pr = markView({ kind: "pr", id: "276", url: "https://github.com/o/r/pull/276" });
  assert.equal(pr.text, "PR #276");
  assert.deepEqual(pr.link, { to: "url", href: "https://github.com/o/r/pull/276" });
  assert.equal(markView({ kind: "pr", id: "276" }).link, null);
  assert.equal(markView({ kind: "merge", id: "276", url: "u" }).text, "merged #276");
  assert.equal(markView({ kind: "verdict", id: "changes_requested", note: "add a test" }).text, "changes asked");
  assert.equal(markView({ kind: "verdict", id: "changes_requested", note: "add a test" }).note, "add a test");
  assert.equal(markView({ kind: "verdict", id: "something_new" }).text, "something new");
  const branch = markView({ kind: "branch", id: "hive/fjord-93/job-1", note: "job-20261126T134608Z-6d882d" });
  assert.equal(branch.note, "from job 6d882d");
  assert.equal(branch.link, null);
  // A job with no run yet has no link.
  assert.equal(markView({ kind: "job", id: "job-x", note: "queued" }).link, null);
  assert.equal(markView({ kind: "job", id: "job-x", note: "queued" }).note, "queued");
  assert.equal(shortJob("job-20261126T134608Z-6d882d"), "6d882d");
  assert.equal(shortJob("job-custom"), "job-custom");
});

const tasks = [{ id: "TASK-1" }, { id: "TASK-2" }, { id: "TASK-3" }, { id: "TASK-9" }];
const project = {
  project: "demo",
  phases: [
    { phase: "intake", label: "Intake", gate: "", tasks: ["TASK-1"] },
    { phase: "shape", label: "Shape and plan", gate: "", tasks: [] },
    { phase: "build", label: "Build", gate: "", tasks: ["TASK-2"] },
    { phase: "verify", label: "Verify", gate: "", tasks: [] },
    { phase: "review", label: "Review", gate: "", tasks: ["TASK-3", "TASK-7"] },
    { phase: "deliver", label: "Deliver", gate: "", tasks: [] },
    { phase: "done", label: "Done", gate: "", tasks: [] },
    { phase: "cancelled", label: "Cancelled", gate: "", tasks: [] },
  ],
  tasks: [
    { task: "TASK-1", phase: "intake", waits_on: "lead", note: "", rework: false },
    { task: "TASK-2", phase: "build", waits_on: "agent", note: "", rework: false },
    { task: "TASK-3", phase: "review", waits_on: "owner", note: "the reviewer handed it to you", rework: true },
    { task: "TASK-7", phase: "review", waits_on: "reviewer", note: "", rework: false },
  ],
};

test("phaseColumns groups the page's tasks by the hub's phases", () => {
  const cols = phaseColumns(project, tasks);
  // Cancelled shows only when it holds a task.
  assert.deepEqual(cols.map((c) => c.phase), ["intake", "shape", "build", "verify", "review", "deliver", "done"]);
  const ids = (p) => cols.find((c) => c.phase === p).tasks.map((t) => t.task.id);
  // TASK-9 was filed after the hub read: it waits in Intake.
  assert.deepEqual(ids("intake"), ["TASK-1", "TASK-9"]);
  assert.equal(cols[0].tasks[1].note, "not placed yet");
  assert.deepEqual(ids("build"), ["TASK-2"]);
  // TASK-7 is filtered out by the page, so it is left out here too.
  assert.deepEqual(ids("review"), ["TASK-3"]);
  const review = cols.find((c) => c.phase === "review").tasks[0];
  assert.equal(review.waits, "waits on you");
  assert.equal(review.rework, true);
  assert.equal(phasesSummary(cols), "4 open tasks, most in Intake; 1 waits on you.");
});

test("phaseColumns without a hub answer places nothing", () => {
  const cols = phaseColumns(null, tasks);
  assert.equal(cols.reduce((a, c) => a + c.tasks.length, 0), 0);
  assert.equal(phasesSummary(cols), "No open task. Everything here is done or cancelled.");
  const withCancelled = phaseColumns({ ...project, phases: undefined, tasks: [{ task: "TASK-1", phase: "cancelled", waits_on: "nobody", note: "cancelled", rework: false }] }, tasks);
  assert.ok(withCancelled.some((c) => c.phase === "cancelled" && c.tasks.length === 1));
});
