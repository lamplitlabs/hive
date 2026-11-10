import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createReportsMock } from "./reports.mjs";

const fixture = JSON.parse(await readFile(new URL("./api/reports.json", import.meta.url), "utf8"));
const mock = createReportsMock(fixture, ["demo-app", "hive", "demo-lib"]);
const q = (s = "") => new URLSearchParams(s);

test("mock reports list done tasks newest first in the route's shape", () => {
  const r = mock.read(q());
  assert.equal(r.status, 200);
  assert.deepEqual(r.body.scope, { project: "", limit: 50, total: 3, has_more: false });
  assert.deepEqual(r.body.reports.map((t) => t.id), ["TASK-014", "TASK-013", "TASK-011"]);
  const job = r.body.reports[0].jobs[0];
  for (const k of ["id", "kind", "title", "status", "agent", "finished_at", "summary", "verify", "commits", "landing", "release", "artifacts", "artifacts_total", "report"]) assert.ok(k in job, k);
});

test("the fixture's evidence names files the job artifacts mock serves", async () => {
  const job = fixture.reports[0].jobs[0];
  for (const f of job.artifacts) await readFile(new URL("./api/jobs/" + job.id + "/artifacts/" + f.name, import.meta.url));
});

test("mock reports filter by project, page by limit and validate like the real route", () => {
  assert.deepEqual(mock.read(q("project=hive")).body.reports.map((t) => t.id), ["TASK-013"]);
  const one = mock.read(q("limit=1")).body;
  assert.equal(one.reports.length, 1);
  assert.equal(one.scope.has_more, true);
  assert.equal(mock.read(q("project=nope")).status, 404);
  for (const bad of ["0", "201", "x", "-1"]) assert.equal(mock.read(q("limit=" + bad)).status, 400, bad);
});
