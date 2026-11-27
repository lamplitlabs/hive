import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { jobWaitingForPr, ownerTurn } from "../src/components/chat/task-groups.ts";
import { projectLifecycle, taskLifecycle } from "./lifecycle.mjs";

const task = (over) => ({ id: "TASK-1", project: "demo", status: "open", lead: "", created_at: "2026-11-26T10:00:00Z", ...over });

test("the mock places a task on its phase with its marks", () => {
  const intake = taskLifecycle({ task: task() });
  assert.equal(intake.phase, "intake");
  assert.equal(intake.waits_on, "lead");
  assert.deepEqual(intake.steps.map((s) => s.state), ["current", "ahead", "ahead", "ahead", "ahead", "ahead"]);

  const build = taskLifecycle({
    task: task({ status: "in_progress", lead: "lead-a", origin: "chat:c-1" }),
    jobs: [{ id: "job-20261126T100500Z-abc123", kind: "task", status: "leased", harness_session: "run-1", created_at: "2026-11-26T10:05:00Z" }],
  });
  assert.equal(build.phase, "build");
  assert.equal(build.waits_on, "agent");
  assert.deepEqual(build.steps[0].left.map((m) => m.kind), ["task", "origin"]);
  assert.equal(build.steps[2].left[0].session, "run-1");
  assert.equal(build.phases.length, 6);

  const review = taskLifecycle({
    task: task({ status: "review", lead: "lead-a" }),
    jobs: [{ id: "job-x", kind: "task", status: "done", branch: "hive/a/job-x" }],
    delivery: { state: "needs_owner", number: 12, url: "https://github.com/o/r/pull/12", at: "2026-11-26T11:00:00Z", note: "" },
  });
  assert.equal(review.phase, "review");
  assert.equal(review.waits_on, "owner");
  assert.deepEqual(review.steps[4].left.map((m) => m.kind), ["pr", "verdict"]);

  const cancelled = taskLifecycle({ task: task({ status: "cancelled" }) });
  assert.equal(cancelled.phase, "cancelled");
  assert.equal(cancelled.steps[0].state, "passed");
  assert.equal(cancelled.steps[2].state, "skipped");
});

test("the mock groups a project's tasks by phase", () => {
  const p = projectLifecycle("demo", [{ task: task() }, { task: task({ id: "TASK-2", status: "done" }) }]);
  assert.deepEqual(p.phases.find((x) => x.phase === "intake").tasks, ["TASK-1"]);
  assert.deepEqual(p.phases.find((x) => x.phase === "done").tasks, ["TASK-2"]);
  assert.equal(p.phases.length, 8);
  assert.equal(p.tasks.length, 2);
});

// The cases crates/hive-hub/src/phases.rs writes into a store and reads
// back through the hub's routes: the mock answers every one with what the
// hub serializes, field for field (lifecycle-answers.json, written by the
// hub's test), and the Sessions list's owner rule waits on the owner where
// the hub does.
const here = dirname(fileURLToPath(import.meta.url));
const read = (name) => JSON.parse(readFileSync(join(here, name), "utf8"));
const { cases, defaults } = read("lifecycle-cases.json");
const { phases, answers } = read("lifecycle-answers.json");

// A case as the mock reads a task detail, with the fixture's defaults for
// what the case leaves out and the store's clock for the events' time.
const detailOf = (c) => ({
  task: task({ ...c.task, id: defaults.task_id, created_at: defaults.task_created_at }),
  jobs: c.jobs.map((j) => ({ created_at: defaults.job.created_at, finished_at: "", branch: "", ...j })),
  delivery: c.delivery ? { ...defaults.delivery, at: defaults.at, ...c.delivery } : null,
});

test("the mock answers every shared case with what the hub's route serializes", () => {
  assert.ok(cases.length >= 25);
  assert.deepEqual(Object.keys(answers).sort(), cases.map((c) => c.name).sort());
  for (const c of cases) {
    const { phases: mine, ...l } = taskLifecycle(detailOf(c));
    assert.deepEqual([l.phase, l.waits_on], [c.phase, c.waits_on], c.name);
    assert.deepEqual(l, answers[c.name], c.name);
    assert.deepEqual(mine, phases, c.name);
    const row = projectLifecycle("proj", [detailOf(c)]).tasks[0];
    assert.deepEqual(row, { task: l.task, phase: l.phase, waits_on: l.waits_on, note: l.note, rework: l.rework }, c.name);
  }
});

test("the Sessions list waits on the owner exactly where the hub does", () => {
  for (const c of cases) {
    const d = detailOf(c);
    const waiting = d.jobs.some((j) => j.kind !== "lead" && jobWaitingForPr(j.status, j.branch));
    const turn = ownerTurn(c.task.status, d.delivery && (waiting || !d.delivery.state.startsWith("held")) ? d.delivery.state : "");
    // A closed task is the board's word, whoever's turn its pull request was.
    const open = c.phase !== "done" && c.phase !== "cancelled";
    assert.equal(turn && open, c.waits_on === "owner", c.name);
  }
});
