import assert from "node:assert/strict";
import { test } from "node:test";
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
