import assert from "node:assert/strict";
import { test } from "node:test";
import { healthLines, parseRemote, posterNames, projectHealth, repoChip, tabCountLabel, topThreads } from "./project-state.ts";

test("parseRemote reads ssh and https remotes into a host and a path", () => {
  assert.deepEqual(parseRemote("git@github.com:owner/demo-app.git"), { host: "github", path: "owner/demo-app", web: "https://github.com/owner/demo-app" });
  assert.deepEqual(parseRemote("https://github.com/owner/demo-app"), { host: "github", path: "owner/demo-app", web: "https://github.com/owner/demo-app" });
  assert.deepEqual(parseRemote("https://gitlab.com/group/sub/app.git"), { host: "gitlab", path: "group/sub/app", web: "https://gitlab.com/group/sub/app" });
  assert.equal(parseRemote("ssh://git@git.example.com/team/app.git").host, "git");
  assert.equal(parseRemote("https://git.example.com/team/app.git").web, "https://git.example.com/team/app");
  assert.deepEqual(parseRemote(""), { host: "local", path: "", web: "" });
  assert.deepEqual(parseRemote(undefined), { host: "local", path: "", web: "" });
});

test("repoChip prefers the remote and falls back to the folder", () => {
  assert.deepEqual(repoChip({ remote: "git@github.com:owner/demo-app.git", repo: "/home/owner/src/demo-app", branch: "main" }), { host: "github", name: "owner/demo-app", branch: "main", href: "https://github.com/owner/demo-app" });
  assert.deepEqual(repoChip({ remote: "", repo: "/home/owner/src/demo-app/", repo_name: "", branch: "main" }), { host: "local", name: "demo-app", branch: "main", href: "" });
  // Before the project info arrives the org card still names the branch.
  assert.deepEqual(repoChip(undefined, { path: "/home/owner/src/demo-app", branch: "feature/x" }), { host: "local", name: "demo-app", branch: "feature/x", href: "" });
  assert.equal(repoChip(null, null), null);
  assert.equal(repoChip({ remote: "", repo: "", branch: "" }), null);
});

test("projectHealth folds the project's state into one badge, worst first", () => {
  const ok = { present: true, missing: false, error: "", stop_present: false, board_present: true, claims_active: 0 };
  assert.equal(projectHealth({ ...ok, present: false }).kind, "missing");
  assert.equal(projectHealth({ ...ok, error: "boom" }).status, "lost");
  assert.equal(projectHealth({ ...ok, board_present: false }).kind, "no-board");
  assert.equal(projectHealth({ ...ok, stop_present: true }).kind, "stopped");
  assert.deepEqual(projectHealth(ok, { failed: 2, forYou: 3 }), { kind: "failing", status: "failed", label: "2 jobs failed" });
  assert.deepEqual(projectHealth(ok, { forYou: 1 }), { kind: "waits", status: "waiting", label: "1 thing waits for you" });
  assert.equal(projectHealth(ok, { forYou: 3 }).label, "3 things wait for you");
  assert.deepEqual(projectHealth({ ...ok, claims_active: 2 }), { kind: "working", status: "running", label: "Agents are working" });
  assert.equal(projectHealth(ok, { step: "add test cases" }).label, "Next step: add test cases");
  assert.deepEqual(projectHealth(ok), { kind: "quiet", status: "ok", label: "All quiet" });
});

test("posterNames lists the agents who post here, newest first, without the owner", () => {
  const agents = [
    { name: "cinder", posts: 4, last: "2026-09-07T08:00:00Z" },
    { name: "human", posts: 9, last: "2026-09-07T09:00:00Z" },
    { name: "iris", posts: 2, last: "2026-09-07T08:30:00Z" },
    { name: "cinder", posts: 1, last: "2026-09-06T08:00:00Z" },
    { name: "quartz", posts: 7, last: "2026-09-05T08:00:00Z" },
    { name: "ash", posts: 1, last: "2026-09-04T08:00:00Z" },
    { name: "bo", posts: 1, last: "2026-09-03T08:00:00Z" },
    { name: "cy", posts: 1, last: "2026-09-02T08:00:00Z" },
    { name: "dee", posts: 1, last: "2026-09-01T08:00:00Z" },
  ];
  assert.deepEqual(posterNames(agents), { shown: ["iris", "cinder", "quartz", "ash", "bo"], more: 2 });
  assert.deepEqual(posterNames(agents, 2), { shown: ["iris", "cinder"], more: 5 });
  assert.deepEqual(posterNames([]), { shown: [], more: 0 });
  assert.deepEqual(posterNames(null), { shown: [], more: 0 });
});

test("topThreads puts threads with open asks first, then the latest, capped", () => {
  const rows = [
    { key: "a", last: "2026-09-07T01:00:00Z", open_questions: 0 },
    { key: "b", last: "2026-09-07T03:00:00Z", open_questions: 0 },
    { key: "c", last: "2026-09-06T01:00:00Z", open_questions: 1 },
    { key: "d", last: "2026-09-07T02:00:00Z", open_handoffs: 1 },
  ];
  assert.deepEqual(topThreads(rows).map((r) => r.key), ["d", "c", "b", "a"]);
  assert.deepEqual(topThreads(rows, 2).map((r) => r.key), ["d", "c"]);
});

test("healthLines say the three things in plain words", () => {
  assert.deepEqual(healthLines({ branch: "main", lastLanded: "2 days ago", machines: 2, delivered: 14 }), { repository: "On main, last landed 2 days ago", machines: "Runs on 2 machines", delivered: "14 code deliveries" });
  assert.deepEqual(healthLines({ branch: "", lastLanded: null, machines: 0, needsWords: true, delivered: 0 }), { repository: "Branch unknown, nothing landed yet", machines: "No machine fits its needs", delivered: "No code delivered yet" });
  assert.deepEqual(healthLines({ branch: "main", machines: null, delivered: "…" }), { repository: "On main, nothing landed yet", machines: "Machines: checking", delivered: "Code delivered: counting" });
  assert.equal(healthLines({ machines: 1, delivered: 1 }).machines, "Runs on 1 machine");
  assert.equal(healthLines({ machines: 0, delivered: 1 }).machines, "No machine runs it");
  assert.equal(healthLines({ delivered: 1 }).delivered, "1 code delivery");
  assert.equal(healthLines({ delivered: "—" }).delivered, "Code delivered: not counted yet");
});

test("tabCountLabel reads a count for a screen reader", () => {
  assert.equal(tabCountLabel("for-you", 1), "1 thing waits for you");
  assert.equal(tabCountLabel("for-you", 3), "3 things wait for you");
  assert.equal(tabCountLabel("work", 1), "1 open task");
  assert.equal(tabCountLabel("board", 2), "2 open threads");
});
