import assert from "node:assert/strict";
import { test } from "node:test";
import { workReportsPath } from "./api.ts";
import {
  cleanSummary,
  commitURL,
  evidenceCount,
  evidenceKind,
  landed,
  landingWords,
  matchesReport,
  orderEvidence,
  reportsEmpty,
  reportsSummary,
  taskBadge,
  taskLine,
  taskReleases,
  taskTone,
  verifyState,
} from "./reports.ts";

const job = (over = {}) => ({
  id: "job-1",
  kind: "task",
  title: "Build it",
  status: "done",
  agent: "iris",
  finished_at: "2026-11-08T10:00:00Z",
  summary: "SUMMARY: done - built the page",
  verify: "verify exit 0",
  commits: 2,
  landing: null,
  release: "",
  artifacts: [],
  artifacts_total: 0,
  report: null,
  ...over,
});

const task = (jobs, over = {}) => ({
  id: "TASK-001",
  project: "demo-app",
  title: "Reports page",
  brief: "List every done task",
  priority: 0,
  created_by: "human",
  created_at: "2026-11-07T10:00:00Z",
  done_at: "2026-11-08T11:00:00Z",
  jobs,
  ...over,
});

const merged = { action: "land.merged", at: "2026-11-08T10:30:00Z", sha: "a".repeat(40), number: 12, url: "https://github.com/o/r/pull/12", verify: "" };
const local = { action: "land.landed", at: "2026-11-08T10:30:00Z", sha: "b".repeat(40), number: 0, url: "", verify: "verify exit 0" };
const pr = { action: "land.pr", at: "2026-11-08T10:30:00Z", sha: "", number: 13, url: "https://github.com/o/r/pull/13", verify: "" };

test("the reports path carries only the filters given", () => {
  assert.equal(workReportsPath(), "reports");
  assert.equal(workReportsPath({ project: "demo-app", limit: 50 }), "reports?project=demo-app&limit=50");
  assert.equal(workReportsPath({ project: "" }), "reports");
});

test("a SUMMARY line reads without its key word and outcome code", () => {
  assert.equal(cleanSummary("SUMMARY: done - built the page"), "built the page");
  assert.equal(cleanSummary("SUMMARY: nothing-to-do - covered by abc"), "covered by abc");
  assert.equal(cleanSummary("blocked: no key"), "no key");
  assert.equal(cleanSummary("plain words"), "plain words");
  assert.equal(cleanSummary(""), "");
});

test("verify notes read as passed, failed, did not run or unknown", () => {
  assert.equal(verifyState(""), null);
  assert.deepEqual(verifyState("verify exit 0"), { text: "Checks passed", tone: "ok" });
  assert.deepEqual(verifyState("verify exit 1"), { text: "Checks failed (exit 1)", tone: "bad" });
  assert.equal(verifyState("verify exit 71 (disk full, verification did not run)").tone, "warn");
  assert.equal(verifyState("verified earlier by iris at 2026-11-08 (tree abc…), same verify command").tone, "ok");
  assert.equal(verifyState("verify timed out after 20 min").tone, "bad");
  assert.equal(verifyState("no verify command").tone, "warn");
  assert.deepEqual(verifyState("something else"), { text: "something else", tone: "neutral" });
});

test("a landing says where the commits went", () => {
  assert.deepEqual(landingWords(null), { label: "Not landed", tone: "neutral" });
  assert.deepEqual(landingWords(merged), { label: "Pull request #12 merged", tone: "ok" });
  assert.deepEqual(landingWords(local), { label: "Merged into the default branch", tone: "ok" });
  assert.deepEqual(landingWords(pr), { label: "Pull request #13 open for review", tone: "warn" });
  assert.equal(landed(job({ landing: merged })), true);
  assert.equal(landed(job({ landing: pr })), false);
  assert.equal(landed(job()), false);
});

test("a task's line, badge and tone follow its best landing and release", () => {
  const released = task([job({ landing: local, release: "v5", artifacts_total: 3 }), job({ id: "job-2", landing: merged, release: "v4" })]);
  assert.deepEqual(taskReleases(released), ["v4", "v5"]);
  assert.equal(evidenceCount(released), 3);
  assert.equal(taskLine(released), "All 2 jobs landed · Released in v4, v5 · 3 evidence files");
  assert.equal(taskBadge(released), "Released");
  assert.equal(taskTone(released), "ok");

  const half = task([job({ landing: merged }), job({ id: "job-2" })]);
  assert.equal(taskLine(half), "1 of 2 jobs landed · Not released yet · No evidence files");
  assert.equal(taskBadge(half), "Landed");

  const review = task([job({ landing: pr, artifacts_total: 1 })]);
  assert.equal(taskLine(review), "Pull request open · 1 evidence file");
  assert.equal(taskBadge(review), "In review");
  assert.equal(taskTone(review), "warn");

  const none = task([]);
  assert.equal(taskLine(none), "No jobs recorded · No evidence files");
  assert.equal(taskBadge(none), "Done");
  assert.equal(taskTone(none), "neutral");
});

test("evidence reads report first, then pictures, data and text", () => {
  assert.equal(evidenceKind("report.html"), "page");
  assert.equal(evidenceKind("after-reports.PNG"), "picture");
  assert.equal(evidenceKind("report.json"), "data");
  assert.equal(evidenceKind("test-run.md"), "text");
  assert.equal(evidenceKind("trace.zip"), "file");
  const files = ["test-run.md", "report.json", "before-x.png", "after-x.png", "report.html", "trace.zip"].map((name) => ({ name, size: 1, modified: "" }));
  assert.deepEqual(
    orderEvidence(files).map((f) => f.name),
    ["report.html", "after-x.png", "before-x.png", "report.json", "test-run.md", "trace.zip"],
  );
});

test("a landed commit links to its forge, and nowhere for an unknown remote", () => {
  const sha = "c".repeat(40);
  assert.equal(commitURL("git@github.com:lamplitlabs/pulse.git", sha), "https://github.com/lamplitlabs/pulse/commit/" + sha);
  assert.equal(commitURL("https://gitlab.com/g/p.git", sha), "https://gitlab.com/g/p/-/commit/" + sha);
  assert.equal(commitURL("https://bitbucket.org/t/r", sha), "https://bitbucket.org/t/r/commits/" + sha);
  assert.equal(commitURL("https://example.com/x/y.git", sha), null);
  assert.equal(commitURL("", sha), null);
  assert.equal(commitURL("git@github.com:o/r.git", "not-a-sha"), null);
});

test("search matches task words, job ids, agents and releases", () => {
  const t = task([job({ release: "v5", agent: "cinder" })]);
  assert.equal(matchesReport(t, ""), true);
  assert.equal(matchesReport(t, "reports"), true);
  assert.equal(matchesReport(t, "v5 cinder"), true);
  assert.equal(matchesReport(t, "TASK-001"), true);
  assert.equal(matchesReport(t, "nothing-like-it"), false);
});

test("the summary counts landed, released and evidenced tasks", () => {
  const d = {
    scope: { project: "", limit: 50, total: 7, has_more: true },
    reports: [task([job({ landing: local, release: "v5", artifacts_total: 2 })]), task([job({ landing: pr })]), task([])],
  };
  assert.deepEqual(reportsSummary(d), { tasks: 7, landed: 1, released: 1, evidence: 1 });
  assert.deepEqual(reportsSummary(undefined), { tasks: 0, landed: 0, released: 0, evidence: 0 });
});

test("the empty state tells no work apart from no match", () => {
  const empty = { scope: { project: "demo-app", limit: 50, total: 0, has_more: false }, reports: [] };
  assert.equal(reportsEmpty(empty, 0).title, "No finished work yet");
  assert.match(reportsEmpty(empty, 0).body, /in demo-app/);
  const some = { ...empty, reports: [task([])] };
  assert.equal(reportsEmpty(some, 1), null);
  assert.equal(reportsEmpty(some, 0).title, "No reports match");
});
