import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { KPI_KEYS, createKpisMock } from "./kpis.mjs";

const params = (q = "") => new URLSearchParams(q);
const fixture = JSON.parse(readFileSync(join(dirname(fileURLToPath(import.meta.url)), "api/kpis.json"), "utf8"));

test("the fixture holds sixty consecutive days ending on the mock's today, every count filled", () => {
  assert.equal(fixture.series.length, 60);
  assert.equal(fixture.series[59].day, "2026-09-07");
  for (let i = 1; i < fixture.series.length; i++) {
    const a = Date.parse(fixture.series[i - 1].day + "T00:00:00Z");
    const b = Date.parse(fixture.series[i].day + "T00:00:00Z");
    assert.equal(b - a, 86400000, fixture.series[i].day);
  }
  assert.deepEqual(KPI_KEYS, ["sessions", "prompt_tokens", "completion_tokens", "cost", "jobs_done", "jobs_failed", "landings", "tasks_done", "reviews", "reviews_merged", "findings", "vulns_found", "vuln_fixes", "owner_asks"]);
  for (const r of fixture.series) {
    for (const k of KPI_KEYS) assert.equal(typeof r[k], "number", r.day + " " + k);
    // The autonomy counts stay in the shape the hub would record: merged
    // reviews are a subset of reviews, and the fleet posts and asks daily.
    assert.ok(r.reviews_merged <= r.reviews, r.day + " merged > reviews");
    assert.ok(r.findings > 0 && r.owner_asks > 0, r.day + " quiet board");
  }
  const sum = (k) => fixture.series.reduce((a, r) => a + r[k], 0);
  assert.ok(sum("reviews") > 0 && sum("vulns_found") > 0 && sum("vuln_fixes") > 0, "the fixture shows work done alone");
});

test("the mock answers the window asked for, defaulting to 30 days, and sums it", () => {
  const mock = createKpisMock(fixture, ["demo-app", "demo-lib"]);
  const { status, body } = mock.read(params());
  assert.equal(status, 200);
  assert.equal(body.days, 30);
  assert.equal(body.series.length, 30);
  assert.equal(body.to, "2026-09-07");
  assert.equal(body.from, "2026-08-09");
  assert.equal(body.project, "");
  assert.equal(body.priced, false);
  assert.equal(body.totals.landings, body.series.reduce((a, r) => a + r.landings, 0));
  for (const k of KPI_KEYS) assert.equal(body.totals[k], body.series.reduce((a, r) => a + r[k], 0), k);
  assert.deepEqual(Object.keys(body.totals), KPI_KEYS);

  const week = mock.read(params("days=7")).body;
  assert.equal(week.series.length, 7);
  assert.equal(week.from, "2026-09-01");
  assert.equal(mock.read(params("days=90")).body.series.length, 60);
});

test("the mock rejects a bad day count and an unknown project like the hub", () => {
  const mock = createKpisMock(fixture, ["demo-app"]);
  for (const q of ["days=0", "days=91", "days=abc", "days=-3"]) assert.equal(mock.read(params(q)).status, 400, q);
  assert.equal(mock.read(params("project=nope")).status, 404);
  const one = mock.read(params("project=demo-app&days=7")).body;
  assert.equal(one.project, "demo-app");
  assert.equal(one.series.length, 7);
  const all = mock.read(params("days=7")).body;
  assert.ok(one.totals.landings < all.totals.landings);
  // A project's share scales every count, and merged reviews never outnumber reviews.
  assert.ok(one.totals.findings < all.totals.findings);
  assert.ok(one.totals.owner_asks < all.totals.owner_asks);
  for (const r of one.series) assert.ok(r.reviews_merged <= r.reviews, r.day);
});
