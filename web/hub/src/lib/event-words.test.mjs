import assert from "node:assert/strict";
import { test } from "node:test";
import { VERB, durationWords, eventDetailText, eventVerb } from "./event-words.ts";

// The landing, review and runner events the Rust crates write (delivery.rs,
// land.rs, landing_outcome.rs, review.rs, stage_budget.rs, awake.rs,
// pulse-board jobs.rs, yield_lane.rs). Each must read as a sentence, not as
// its raw words.
const ADDED = [
  "land.pr",
  "land.pr-checks",
  "land.pr-closed",
  "land.pr-conflict",
  "land.pr-held",
  "land.pr-refreshed",
  "land.pr-unstacked",
  "land.rebase",
  "land.merged",
  "land.missing",
  "land.measured",
  "review.queued",
  "review.changes",
  "review.merged",
  "review.failed",
  "review.owner",
  "daemon.slow-stage",
  "daemon.slept",
  "job.recover",
  "job.yield",
];

const raw = (a) => a.replace(/[._-]+/g, " ").replace(/\bdaemon\b/g, "runner").trim();

test("every landing, review and runner event has a plain-words sentence", () => {
  for (const action of ADDED) {
    assert.ok(VERB[action], action + " has a VERB entry");
    for (const detail of [null, {}, { project: "demo-app", number: 41, base: "main" }]) {
      const said = eventVerb({ action, subject: "demo-app", detail });
      assert.ok(said.length > 0, action + " says something");
      assert.notEqual(said, raw(action), action + " is not its raw words");
      assert.doesNotMatch(said, /\b(lease|tier|executor|daemon|pr)\b/i, action + " uses no jargon: " + said);
    }
  }
});

test("a pull request event names the pull request and its branch", () => {
  assert.equal(eventVerb({ action: "land.pr-conflict", subject: "demo-app", detail: {} }), "found a pull request conflicts with main and cannot merge");
  assert.equal(
    eventVerb({ action: "land.pr-conflict", subject: "demo-app", detail: { number: 41, base: "main" } }),
    "found pull request #41 conflicts with main and cannot merge",
  );
  assert.equal(
    eventVerb({ action: "land.pr-refreshed", subject: "demo-app", detail: { number: 7, base: "develop" } }),
    "brought pull request #7 up to date with develop",
  );
  assert.equal(eventVerb({ action: "review.owner", subject: "demo-app", detail: { number: 9 } }), "asked you to decide on pull request #9");
  assert.equal(eventVerb({ action: "review.changes", subject: "demo-app", detail: { number: 9 } }), "asked for changes on pull request #9");
});

test("land.pr says whether it opened, refreshed or moved the pull request", () => {
  const at = (d) => eventVerb({ action: "land.pr", subject: "demo-app", detail: { number: 5, base: "main", ...d } });
  assert.equal(at({ opened: true }), "opened pull request #5");
  assert.equal(at({ opened: false, refreshed: true }), "brought pull request #5 up to date with main");
  assert.equal(at({ opened: false, retargeted_from: "pulse/task/TASK-1" }), "moved pull request #5 onto main");
  assert.equal(at({ opened: false }), "updated pull request #5 with new work");
});

test("job.recover reads by its outcome; the runner's sleep says how long", () => {
  const recover = (outcome) => eventVerb({ action: "job.recover", subject: "job-1", detail: { outcome } });
  assert.equal(recover("retried"), "tried a job again");
  assert.equal(recover("replanned"), "planned a job again from the start");
  assert.equal(recover("requeued"), "put a job back in the queue");
  assert.equal(recover("held"), "held a job back for you to look at");
  assert.equal(recover("something-new"), VERB["job.recover"]);
  assert.equal(eventVerb({ action: "daemon.slept", subject: "", detail: { seconds: 2700 } }), "noticed the computer was asleep for about 45 minutes");
});

test("a slow stage shows its stage and how long it took on the second line", () => {
  const line = eventDetailText({ action: "daemon.slow-stage", detail: { stage: "delivery", ms: 184000, budget_ms: 120000, ran: 2, skipped: ["demo-lib"] } });
  assert.equal(line, "Delivery took 3 minutes, over its budget of 2 minutes; demo-lib waits for the next pass");
  assert.equal(eventDetailText({ action: "daemon.slow-stage", detail: { stage: "sweep", ms: 45000 } }), "Sweep took 45 seconds");
  assert.equal(eventDetailText({ action: "daemon.slow-stage", detail: {} }), "");
});

test("other events keep their own second lines", () => {
  assert.equal(eventDetailText({ action: "land.base-red", detail: { note: "main is red", failures: ["a", "b"] } }), "main is red: a, b");
  assert.equal(eventDetailText({ action: "land.pr-checks", detail: { checks: ["test (ubuntu)", "lint"] } }), "Failing: test (ubuntu), lint");
  assert.equal(eventDetailText({ action: "land.pr-conflict", detail: { files: ["src/a.ts"] } }), "Both changed src/a.ts");
  assert.equal(eventDetailText({ action: "review.failed", detail: { error: "gh: not found" } }), "gh: not found");
  assert.equal(eventDetailText({ action: "job.done", detail: { summary: "x" } }), "");
});

test("an unknown action still reads as its own words", () => {
  assert.equal(eventVerb({ action: "thing.happened", subject: "", detail: null }), "thing happened");
  assert.equal(eventVerb({ action: "daemon.new-stage", subject: "", detail: {} }), "runner new stage");
  assert.equal(eventVerb({ action: "executor.rebooted", subject: "", detail: {} }), "machine rebooted");
});

test("the earlier sentences still read the same", () => {
  assert.equal(eventVerb({ action: "post.claim", subject: "", detail: {} }), "picked up a thread");
  assert.equal(eventVerb({ action: "decide", subject: "20260901T203300Z-cinder", detail: { answer: "yes" } }), "said yes to cinder\u2019s proposal");
  assert.equal(eventVerb({ action: "executor.add", subject: "mac-mini", detail: {} }), "added the machine mac-mini");
  assert.equal(eventVerb({ action: "agent.retire", subject: "cinder", detail: {} }), "retired the worker cinder after its run");
  assert.equal(eventVerb({ action: "land.landed", subject: "", detail: {} }), "merged finished work");
});

test("durations read in words", () => {
  assert.equal(durationWords(400), "under a second");
  assert.equal(durationWords(1000), "1 second");
  assert.equal(durationWords(90 * 60 * 1000), "1 hour 30 minutes");
  assert.equal(durationWords(2 * 3600 * 1000), "2 hours");
});
