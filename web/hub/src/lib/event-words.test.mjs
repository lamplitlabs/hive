import assert from "node:assert/strict";
import { test } from "node:test";
import { VERB, durationWords, eventDetailText, eventVerb } from "./event-words.ts";

// The landing, review and runner events the Rust crates write (delivery.rs,
// land.rs, landing_outcome.rs, review.rs, stage_budget.rs, awake.rs,
// hive-board jobs.rs, yield_lane.rs). Each must read as a sentence, not as
// its raw words.
const ADDED = [
  "land.pr",
  "land.pr-checks",
  "land.pr-closed",
  "land.pr-conflict",
  "land.pr-held",
  "land.pr-refreshed",
  "land.pr-rebuilt",
  "land.rebase",
  "land.merged",
  "land.evidence",
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
  assert.equal(at({ opened: false, retargeted_from: "hive/task/TASK-1" }), "moved pull request #5 onto main");
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
  assert.equal(eventVerb({ action: "hand.revoke", subject: "studio-mac", detail: {} }), "revoked the machine studio-mac");
  assert.equal(eventVerb({ action: "hand.connect", subject: "studio-mac", detail: {} }), "connected the machine studio-mac");
  assert.equal(eventVerb({ action: "hand.join_code", subject: "", detail: {} }), "issued a join code for a machine");
  assert.equal(eventVerb({ action: "agent.retire", subject: "cinder", detail: {} }), "retired the worker cinder after its run");
  assert.equal(eventVerb({ action: "land.landed", subject: "", detail: {} }), "merged finished work");
});

test("durations read in words", () => {
  assert.equal(durationWords(400), "under a second");
  assert.equal(durationWords(1000), "1 second");
  assert.equal(durationWords(90 * 60 * 1000), "1 hour 30 minutes");
  assert.equal(durationWords(2 * 3600 * 1000), "2 hours");
});

// The self-improvement loop, schedule, lead and suggested-work events
// (hive-evolve archive.rs, feedback.rs, fixtures.rs, derive.rs, starter.rs,
// heal.rs, fleet.rs; hive-records seed.rs; hive-tasks schedules.rs,
// leads.rs, tasks.rs, suggestions.rs; hive-runner lead.rs, recover.rs).
const LOOP = [
  "evolution.archived",
  "evolution.feedback",
  "evolution.fixture.added",
  "evolution.fixture.derived",
  "evolution.fixture.derive_failed",
  "evolution.fixture.enabled",
  "evolution.fixture.rearmed",
  "evolution.fixture.retired",
  "evolution.fixture.seeded",
  "evolution.fixture.seed_failed",
  "evolution.flat.resumed",
  "evolution.flat.self_reviewed",
  "evolution.healed",
  "evolution.records.seeded",
  "evolution.records.seed_failed",
  "evolution.fleet.prepared",
  "evolution.fleet.cleaned",
  "schedule.added",
  "schedule.updated",
  "schedule.ended",
  "schedule.fired",
  "schedule.ran",
  "schedule.run",
  "schedule.red",
  "lead.run",
  "lead.seated",
  "task.followup",
  "task.lead",
  "task.nudged",
  "suggestion.new",
  "suggestion.review",
  "suggestion.reviewing",
  "suggestion.unreviewed",
  "suggestion.withdrawn",
];

test("every loop, schedule, lead and suggestion event has a plain-words sentence", () => {
  for (const action of LOOP) {
    assert.ok(VERB[action], action + " has a VERB entry");
    for (const detail of [null, {}, { project: "demo-app", state: "red", status: "accepted", outcome: "improved", enabled: true }]) {
      const said = eventVerb({ action, subject: "demo-app", detail });
      assert.ok(said.length > 0, action + " says something");
      assert.notEqual(said, raw(action), action + " is not its raw words");
      assert.doesNotMatch(said, /\b(fixture|evolution|evolve|archive|seed|derive|rearm|flat|lease|tier|executor|daemon|pr)\b/i, action + " uses no jargon: " + said);
    }
  }
});

test("loop events read their outcome", () => {
  assert.equal(eventVerb({ action: "evolution.fixture.added", subject: "x", detail: {} }), "added a measurement to the improvement loop");
  assert.equal(
    eventVerb({ action: "evolution.archived", subject: "a", detail: { outcome: "discarded" } }),
    "discarded an improvement experiment that made things worse",
  );
  assert.equal(eventVerb({ action: "evolution.archived", subject: "a", detail: { outcome: "new" } }), VERB["evolution.archived"]);
  assert.equal(eventVerb({ action: "evolution.fixture.enabled", subject: "x", detail: { enabled: false } }), "switched off a measurement of the improvement loop");
  assert.equal(eventVerb({ action: "evolution.fixture.enabled", subject: "x", detail: { enabled: true } }), "switched on a measurement of the improvement loop");
});

test("schedule and suggestion events say how they went", () => {
  assert.equal(eventVerb({ action: "schedule.ran", subject: "S", detail: { state: "red" } }), "ran a scheduled check, which failed");
  assert.equal(eventVerb({ action: "schedule.ran", subject: "S", detail: { state: "ok" } }), "ran a scheduled check, which passed");
  assert.equal(eventVerb({ action: "schedule.ran", subject: "S", detail: {} }), "ran a scheduled check");
  assert.equal(eventVerb({ action: "schedule.red", subject: "S", detail: { kind: "watch" } }), "saw a watched script stop working");
  assert.equal(eventVerb({ action: "schedule.red", subject: "S", detail: { kind: "check" } }), "saw a scheduled check start failing");
  assert.equal(eventVerb({ action: "suggestion.review", subject: "SUG-1", detail: { status: "declined" } }), "declined a suggested task");
  assert.equal(eventVerb({ action: "suggestion.review", subject: "SUG-1", detail: { status: "owner" } }), "asked you to decide on a suggested task");
  assert.equal(eventVerb({ action: "suggestion.review", subject: "SUG-1", detail: {} }), "reviewed a suggested task");
});

// The tool connection, skill, record, project and agent events the Rust
// crates write (hive-store mcp_servers.rs, skills.rs, records.rs, forget.rs;
// hive-hub mcp.rs, lifecycle.rs; hive-board projects.rs, agents.rs; the
// runner's auto_adopt.rs; hive-records drafts).
const SETUP = [
  "mcp.added",
  "mcp.updated",
  "mcp.enabled",
  "mcp.disabled",
  "mcp.checked",
  "mcp.signed_in",
  "mcp.signed_out",
  "skill.added",
  "skill.updated",
  "skill.enabled",
  "skill.disabled",
  "record.status",
  "record.draft",
  "record.auto_adopted",
  "record.imported",
  "project.moved",
  "project.renamed",
  "project.forgotten",
  "project.hook_failed",
  "agent.join",
  "agent.resume",
  "agent.prune",
];

test("every tool connection, skill, record, project and agent event has a plain-words sentence", () => {
  for (const action of SETUP) {
    assert.ok(VERB[action], action + " has a VERB entry");
    for (const detail of [null, {}, "Accepted", { project: "demo-app", name: "github", ok: false, from: "a", to: "b", changed: ["body"] }]) {
      const said = eventVerb({ action, subject: "MCP-001", detail });
      assert.ok(said.length > 0, action + " says something");
      assert.notEqual(said, raw(action), action + " is not its raw words");
      assert.doesNotMatch(said, /\b(mcp|hook|prune|auto_adopted|signed_in|signed_out)\b/i, action + " uses no jargon: " + said);
      }
    }
  });

// Credential, conversation, goal, rules, hook, setting and Hive-file events
// (hive-cred-setup setup.rs, hive-harness conversation.rs, hive-goals
// review.rs, hive-tally amend.rs, hive-hooks hooks.rs, hive-store store.rs,
// hive-tasks and hive-runner manifest.rs).
const OWNER = [
  "cred.set",
  "cred.clear",
  "secret.key",
  "secret.set",
  "secret.removed",
  "secret.granted",
  "secret.refused",
  "secret.read",
  "chat.wake",
  "chat.finish",
  "goal.amend",
  "constitution.amended",
  "hook.added",
  "setting.retired",
  "manifest.synced",
  "manifest.problems",
  "manifest.problems_cleared",
];

test("every credential, goal, rules and Hive-file event has a plain-words sentence", () => {
  for (const action of OWNER) {
    assert.ok(VERB[action], action + " has a VERB entry");
    for (const detail of [null, {}, { project: "demo-app", applied: true, problems: [] }]) {
      const said = eventVerb({ action, subject: "demo-app", detail });
      assert.ok(said.length > 0, action + " says something");
      assert.notEqual(said, raw(action), action + " is not its raw words");
      assert.doesNotMatch(said, /\b(cred|manifest|constitution|amend(ed)?|retired|synced|problems_cleared)\b/i, action + " uses no jargon: " + said);
    }
  }
});

test("tool connection and skill events name what they changed", () => {
  assert.equal(eventVerb({ action: "mcp.added", subject: "MCP-003", detail: { name: "github" } }), "added the tool connection github");
  assert.equal(eventVerb({ action: "mcp.added", subject: "MCP-003", detail: {} }), "added a tool connection");
  assert.equal(eventVerb({ action: "mcp.signed_out", subject: "MCP-003", detail: { name: "clerk" } }), "signed out of the tool connection clerk");
  assert.equal(eventVerb({ action: "mcp.checked", subject: "MCP-003", detail: { name: "github", ok: true } }), "checked the tool connection github, which answered");
  assert.equal(eventVerb({ action: "mcp.checked", subject: "MCP-003", detail: { name: "github", ok: false } }), "checked the tool connection github, which did not answer");
  assert.equal(eventVerb({ action: "skill.disabled", subject: "SKILL-2", detail: { name: "flake-triager" } }), "switched off the skill flake-triager");
  assert.equal(eventDetailText({ action: "skill.updated", detail: { name: "x", changed: ["description", "body"] } }), "Changed: description, body");
  assert.equal(eventDetailText({ action: "mcp.updated", detail: { name: "x", changed: [] } }), "");
});

test("project and record events say what happened to them", () => {
  assert.equal(eventVerb({ action: "project.renamed", subject: "demo-app", detail: { from: "Demo", to: "Demo App" } }), "renamed the project from \u201cDemo\u201d to \u201cDemo App\u201d");
  assert.equal(eventVerb({ action: "project.renamed", subject: "demo-app", detail: {} }), "renamed the project");
  assert.equal(eventDetailText({ action: "project.moved", detail: { from: "/old", to: "/new" } }), "Now in /new");
  assert.equal(eventDetailText({ action: "project.hook_failed", detail: { error: "no starter records" } }), "no starter records");
  assert.equal(eventDetailText({ action: "project.forgotten", detail: { total: 1 } }), "1 stored row removed");
  assert.equal(eventDetailText({ action: "project.forgotten", detail: { total: 42 } }), "42 stored rows removed");
  assert.equal(eventVerb({ action: "record.auto_adopted", subject: "ADR-001", detail: {} }), "adopted a drafted record change while you were away");
  assert.equal(eventVerb({ action: "agent.join", subject: "cinder", detail: {} }), "joined as a new agent");
});

test("goal and Hive-file events say how they went", () => {
  assert.equal(eventVerb({ action: "goal.amend", subject: "demo-app", detail: { applied: true } }), "set the project\u2019s goal from an accepted proposal");
  assert.equal(eventVerb({ action: "goal.amend", subject: "demo-app", detail: { applied: false, why: "too long" } }), "could not set the project\u2019s goal from an accepted proposal");
  assert.equal(eventDetailText({ action: "goal.amend", detail: { applied: false, why: "too long" } }), "too long");
  assert.equal(eventDetailText({ action: "goal.amend", detail: { applied: true } }), "");
  assert.equal(eventVerb({ action: "manifest.synced", subject: "demo-app", detail: { problems: [] } }), "applied the project\u2019s Hive files from its default branch");
  assert.equal(
    eventVerb({ action: "manifest.synced", subject: "demo-app", detail: { problems: ["nightly: unknown kind"] } }),
    "applied the project\u2019s Hive files from its default branch, some with problems",
  );
  assert.equal(eventDetailText({ action: "manifest.problems", detail: { problems: ["a: bad", "b: bad"] } }), "a: bad, b: bad");
  assert.equal(eventDetailText({ action: "manifest.problems_cleared", detail: { problems: [] } }), "");
});

// Store upgrade, record seeding, archive, machine, message and board-cleanup
// events (hive-store store.rs, records.rs, chat.rs, archive.rs,
// hive-records bundle.rs and import.rs, hive-evolve starter.rs,
// hive-board remove.rs, hive-runner pair.rs, hive-tasks messages.rs).
const UPKEEP = [
  "schema.records",
  "schema.superseded",
  "schema.widened",
  "store.archived",
  "records.framework.seeded",
  "records.framework.seed_failed",
  "records.migrated",
  "executor.update",
  "message.read",
  "post.remove.duplicates",
];

test("every store upgrade, record seeding and upkeep event has a plain-words sentence", () => {
  for (const action of UPKEEP) {
    assert.ok(VERB[action], action + " has a VERB entry");
    for (const detail of [null, {}, { project: "demo-app", rows: 3, written: 2, count: 1 }]) {
      const said = eventVerb({ action, subject: "demo-app", detail });
      assert.ok(said.length > 0, action + " says something");
      assert.notEqual(said, raw(action), action + " is not its raw words");
      assert.doesNotMatch(
        said,
        /\b(schema|framework|seeded|seed_failed|migrated|superseded|widened|archived|executor|duplicates)\b/i,
        action + " uses no jargon: " + said,
      );
    }
  }
});

test("a machine update names the machine", () => {
  assert.equal(eventVerb({ action: "executor.update", subject: "studio", detail: {} }), "changed the settings of the machine studio");
  assert.equal(eventVerb({ action: "store.archived", subject: "archive", detail: {} }), "moved old history to the archive file to keep the store small");
});

test("clearing repeated posts is not read as a new post", () => {
  assert.equal(eventVerb({ action: "post.remove.duplicates", subject: "x", detail: {} }), "removed repeated posts from the board");
  assert.equal(eventVerb({ action: "post.remove.duplicates", subject: "x", detail: { removed: 1 } }), "removed 1 repeated post from the board");
  assert.equal(eventVerb({ action: "post.remove.duplicates", subject: "x", detail: { removed: 3 } }), "removed 3 repeated posts from the board");
});
