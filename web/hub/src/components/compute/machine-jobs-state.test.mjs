import assert from "node:assert/strict";
import { test } from "node:test";
import { MACHINE_JOBS_PATH, MACHINE_JOBS_SHOWN, isLocalMachine, machineJobMeta, machineJobs, machineJobsHeading, machineJobsLabel, moreJobsLabel, runMachine, shownJobs } from "./machine-jobs-state.ts";

const run = (over) => ({
  id: "hs-" + (over.job_id ?? "x"),
  job_id: "job-a",
  job_title: "Fix the retry",
  agent: "cinder",
  project: "demo-app",
  executor: "local",
  status: "running",
  started_at: "2026-11-28T10:00:00Z",
  ...over,
});

test("the page asks once for every run in flight", () => {
  assert.match(MACHINE_JOBS_PATH, /^harness\/sessions\?/);
  assert.match(MACHINE_JOBS_PATH, /status=running/);
});

test("runMachine: a session that named no machine ran here", () => {
  assert.equal(runMachine("build-mini"), "build-mini");
  assert.equal(runMachine(" vm-7 "), "vm-7");
  assert.equal(runMachine(""), "local");
  assert.equal(runMachine(null), "local");
  assert.equal(runMachine(undefined), "local");
});

test("isLocalMachine: the runner's local row by kind or name", () => {
  assert.equal(isLocalMachine({ name: "local", kind: "local" }), true);
  assert.equal(isLocalMachine({ name: "local" }), true);
  assert.equal(isLocalMachine({ name: "studio", kind: "local" }), true);
  assert.equal(isLocalMachine({ name: "build-mini", kind: "dial" }), false);
});

test("machineJobs keeps the running jobs of that machine only", () => {
  const runs = [
    run({ job_id: "job-here", executor: "local" }),
    run({ job_id: "job-blank", executor: "" }),
    run({ job_id: "job-mini", executor: "build-mini" }),
    run({ job_id: "job-done", executor: "build-mini", status: "submitted" }),
    run({ job_id: "", executor: "build-mini" }),
  ];
  assert.deepEqual(machineJobs(runs, { name: "local", kind: "local" }).map((j) => j.job), ["job-blank", "job-here"]);
  assert.deepEqual(machineJobs(runs, { name: "build-mini", kind: "dial" }).map((j) => j.job), ["job-mini"], "an ended run and a run with no job are not listed");
  assert.deepEqual(machineJobs(runs, { name: "hive-vm1", kind: "ssh" }), []);
  assert.deepEqual(machineJobs(null, { name: "local" }), []);
  assert.deepEqual(machineJobs(undefined, { name: "local" }), []);
});

test("machineJobs: one row per job, its newest session, longest-running first", () => {
  const runs = [
    run({ id: "hs-old", job_id: "job-b", started_at: "2026-11-28T09:00:00Z" }),
    run({ id: "hs-new", job_id: "job-b", started_at: "2026-11-28T11:00:00Z" }),
    run({ id: "hs-a", job_id: "job-a", started_at: "2026-11-28T10:00:00Z" }),
  ];
  const jobs = machineJobs(runs, { name: "local" });
  assert.deepEqual(jobs.map((j) => [j.job, j.session]), [["job-a", "hs-a"], ["job-b", "hs-new"]]);
});

test("machineJobs: a row carries what the Stop needs", () => {
  const [j] = machineJobs([run({ job_id: "job-z", job_title: "  ", executor: "build-mini", cancel_requested_at: "2026-11-28T10:05:00Z" })], { name: "build-mini" });
  assert.equal(j.job, "job-z");
  assert.equal(j.title, "job-z", "an empty title falls back to the job id");
  assert.equal(j.executor, "build-mini", "the Stop names the machine the run is on");
  assert.equal(j.stopping, true);
  assert.equal(j.since, "2026-11-28T10:00:00Z");
  const [k] = machineJobs([run({ job_id: "job-y", executor: "" })], { name: "local" });
  assert.equal(k.executor, "local");
  assert.equal(k.stopping, false);
});

test("shownJobs caps the list and counts the rest", () => {
  const jobs = machineJobs(
    Array.from({ length: 7 }, (_, i) => run({ job_id: "job-" + i, started_at: "2026-11-28T10:0" + i + ":00Z" })),
    { name: "local" },
  );
  const { shown, more } = shownJobs(jobs);
  assert.equal(shown.length, MACHINE_JOBS_SHOWN);
  assert.equal(more, 7 - MACHINE_JOBS_SHOWN);
  assert.deepEqual(shownJobs(jobs.slice(0, 2)), { shown: jobs.slice(0, 2), more: 0 });
  assert.deepEqual(shownJobs([], 3), { shown: [], more: 0 });
});

test("words: heading, label, meta and more", () => {
  assert.equal(machineJobsHeading(0), "Nothing running");
  assert.equal(machineJobsHeading(1), "1 job running");
  assert.equal(machineJobsHeading(3), "3 jobs running");
  assert.equal(machineJobsLabel("build-mini"), "Jobs running on build-mini");
  assert.equal(machineJobMeta({ agent: "cinder", project: "demo-app" }), "cinder · demo-app");
  assert.equal(machineJobMeta({ agent: "cinder", project: "demo-app" }, (a) => a.toUpperCase()), "CINDER · demo-app");
  assert.equal(machineJobMeta({ agent: "", project: "demo-app" }), "demo-app");
  assert.equal(machineJobMeta({ agent: "", project: "" }), "");
  assert.equal(moreJobsLabel(0), "");
  assert.equal(moreJobsLabel(2), "+2 more running");
});
