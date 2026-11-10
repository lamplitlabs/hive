import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { funnelOf, gateLabel } from "./funnel.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const suggestions = (slug) => JSON.parse(readFileSync(join(here, "api/projects", slug, "suggestions.json"), "utf8"));

test("demo-app's funnel moves: one suggestion queued as a job, one agreed but held on votes, no stall alert", () => {
  const f = funnelOf(suggestions("demo-app"));
  assert.equal(f.posted, 2);
  assert.equal(f.eligible, 2);
  assert.equal(f.plannable, 0);
  assert.equal(f.in_flight, 1, "the queued job counts as in flight, not as held");
  assert.equal(f.blocked, 0);
  assert.deepEqual(f.held, [{ gate: "net votes", count: 1 }]);
  assert.deepEqual(f.alerts, [], "something is in progress, so nothing stalls");
  assert.equal(f.require_check, true);
  assert.equal(f.check_skips_quorum, true);
});

test("demo-lib's funnel waits for votes: one suggestion, none agreed, a votes alert", () => {
  const f = funnelOf(suggestions("demo-lib"));
  assert.equal(f.posted, 1);
  assert.equal(f.needs_votes, 1);
  assert.equal(f.eligible, 0);
  assert.deepEqual(f.held, []);
  assert.equal(f.alerts.length, 1);
  assert.equal(f.alerts[0].code, "votes");
  assert.equal(f.alerts[0].count, 1);
  assert.match(f.alerts[0].text, /1 suggestion\(s\) wait for votes/);
});

test("an empty board says so and a settled one lists its tail", () => {
  const empty = funnelOf([]);
  assert.equal(empty.posted, 0);
  assert.deepEqual(empty.alerts.map((a) => a.code), ["empty"]);
  const settled = funnelOf([
    { status: "built", gate: "", job: "job-1", job_status: "done" },
    { status: "rejected", gate: "", job: "", job_status: "" },
    { status: "tried", gate: "", job: "job-2", job_status: "failed" },
  ]);
  assert.equal(settled.built, 1);
  assert.equal(settled.rejected, 1);
  assert.equal(settled.tried, 1);
  assert.equal(settled.in_flight, 0, "finished jobs are not in flight");
  assert.deepEqual(settled.alerts.map((a) => a.code), ["settled"]);
  assert.match(settled.alerts[0].text, /1 built, 1 tried, 1 turned down/);
});

test("held groups come biggest first and a blocked job raises its own alert beside the stall", () => {
  const f = funnelOf([
    { status: "eligible", gate: "names no check (Signal: check:<command>)", job: "", job_status: "" },
    { status: "eligible", gate: "names no check", job: "", job_status: "" },
    { status: "eligible", gate: "net votes <= 0", job: "", job_status: "" },
    { status: "eligible", gate: "blocked as job job-9", job: "job-9", job_status: "blocked" },
    { status: "eligible", gate: "", plannable: true, checked: true, job: "", job_status: "" },
  ]);
  assert.deepEqual(f.held, [
    { gate: "names no check", count: 2 },
    { gate: "net votes", count: 1 },
  ]);
  assert.equal(f.checked, 1);
  assert.equal(f.plannable, 1);
  assert.equal(f.in_flight, 1);
  assert.equal(f.blocked, 1);
  assert.deepEqual(f.alerts.map((a) => a.code), ["blocked"], "the funnel moves (one is ready), so only the blocked job is reported");
  // Every agreed one held and nothing in flight: the held alert names the groups.
  const stuck = funnelOf([
    { status: "eligible", gate: "addressed to the owner", job: "", job_status: "" },
    { status: "needs votes", gate: "", job: "", job_status: "" },
  ]);
  assert.deepEqual(stuck.alerts.map((a) => a.code), ["held"]);
  assert.match(stuck.alerts[0].text, /held \(1 for the owner\)/);
});

test("gateLabel maps the hub's gate reasons to their short labels and clips the rest", () => {
  assert.equal(gateLabel("net votes <= 0: the planner takes score > 0 only"), "net votes");
  assert.equal(gateLabel("without evidence of the problem"), "no evidence");
  assert.equal(gateLabel("points at owner-only paths: .github/"), "owner-only paths");
  assert.equal(gateLabel("Signal path is absent on main"), "waits for landing");
  assert.equal(gateLabel("duplicate of 20260901T141500Z-cinder"), "duplicate");
  assert.equal(gateLabel("operating-layer cap reached"), "operating-layer cap");
  assert.equal(gateLabel("loop cap: 2 open"), "loop cap");
  assert.equal(gateLabel("check selects no test"), "check selects no test");
  assert.equal(gateLabel("File: names a path that is gone"), "file gone");
  assert.equal(gateLabel("File holds none of the Signal's terms"), "file stale");
  assert.equal(gateLabel("some brand-new reason the hub grew later on"), "some brand-new reason th");
  assert.equal(gateLabel(""), "");
});
