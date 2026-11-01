import assert from "node:assert/strict";
import { test } from "node:test";
import { claimAgent, claimTitle, NOW_EMPTY, nowCountsLine, nowView } from "./now-state.ts";

const claim = (over) => ({ id: "c1", thread_key: "hub/now", summary: "Now panel", expires: "", ...over });
const card = (over) => ({ slug: "demo", missing: false, ...over });

test("an org with no claims shows the empty state with zero counts", () => {
  const v = nowView({ active_claims: [], projects: [] });
  assert.equal(v.empty, true);
  assert.equal(v.running, 0);
  assert.equal(v.headline, NOW_EMPTY);
  assert.equal(v.headline, "Nothing running");
  assert.deepEqual(v.agents, []);
  assert.equal(nowCountsLine(v), "0 queued · 0 failed");
});

test("missing arrays are treated as empty", () => {
  const v = nowView({});
  assert.equal(v.empty, true);
  assert.equal(v.queued, 0);
  assert.equal(v.failed, 0);
});

test("claims group per agent, busiest agent first, titles kept in order", () => {
  const v = nowView({
    active_claims: [
      claim({ id: "a", agent: "umber", summary: "Now panel", slug: "hub" }),
      claim({ id: "b", agent: "cinder", summary: "Fix flake" }),
      claim({ id: "c", agent: "umber", summary: "", thread_key: "hub/labels" }),
    ],
    projects: [],
  });
  assert.equal(v.empty, false);
  assert.equal(v.running, 3);
  assert.equal(v.headline, "3 claims running · 2 agents");
  assert.deepEqual(
    v.agents.map((a) => [a.agent, a.items.map((i) => i.title)]),
    [
      ["umber", ["Now panel", "hub/labels"]],
      ["cinder", ["Fix flake"]],
    ],
  );
  assert.equal(v.agents[0].items[0].slug, "hub");
});

test("one claim reads in the singular", () => {
  const v = nowView({ active_claims: [claim({ agent: "iris" })], projects: [] });
  assert.equal(v.headline, "1 claim running · 1 agent");
});

test("agent and title fall back when fields are blank", () => {
  assert.equal(claimAgent(claim({ agent: "", name: "Iris" })), "Iris");
  assert.equal(claimAgent(claim({ runner: "copilot" })), "copilot");
  assert.equal(claimAgent(claim({})), "unknown agent");
  assert.equal(claimTitle(claim({ summary: "  " })), "hub/now");
  assert.equal(claimTitle(claim({ summary: "", thread_key: "" })), "(untitled)");
});

test("queued comes from the runner when loaded, else from project cards", () => {
  const projects = [
    card({ slug: "a", baseline_job: { id: "j1", status: "queued" } }),
    card({ slug: "b", baseline_job: { id: "j2", status: "leased" } }),
    card({ slug: "c", baseline_job: null }),
  ];
  assert.equal(nowView({ active_claims: [], projects }).queued, 1);
  assert.equal(nowView({ active_claims: [], projects }, null).queued, 1);
  assert.equal(nowView({ active_claims: [], projects }, { queued: 4 }).queued, 4);
  assert.equal(nowView({ active_claims: [], projects }, { queued: 0 }).queued, 0);
});

test("failed counts projects whose baseline failed, skipping missing ones", () => {
  const v = nowView({
    active_claims: [],
    projects: [
      card({ slug: "a", baseline_failed_at: "2026-11-01T05:00:00Z" }),
      card({ slug: "b" }),
      card({ slug: "gone", missing: true, baseline_failed_at: "2026-11-01T05:00:00Z" }),
    ],
  });
  assert.equal(v.failed, 1);
  assert.deepEqual(v.failedProjects, ["a"]);
  assert.equal(v.empty, true);
  assert.equal(nowCountsLine(v), "0 queued · 1 failed");
});
