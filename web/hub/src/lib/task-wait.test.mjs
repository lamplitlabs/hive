import assert from "node:assert/strict";
import { test } from "node:test";
import { waitLine, waitSince } from "./task-wait.ts";

const now = Date.parse("2026-11-03T15:00:00Z");

test("waitSince names the clock today and the day before that", () => {
  assert.equal(waitSince("2026-11-03T04:40:00Z", now), "04:40 UTC");
  assert.equal(waitSince("2026-11-02T23:05:00Z", now), "Nov 2, 23:05 UTC");
  assert.equal(waitSince("not a date", now), "not a date");
});

test("waitLine names the holds the daemon saw", () => {
  const w = { job: "job-1", project: "p", title: "T", since: "2026-11-03T04:40:00Z", at: "2026-11-03T04:51:00Z", reasons: ["lanes full", "load 27 > 24"] };
  assert.equal(waitLine(w, now), "The next step has waited since 04:40 UTC: lanes full, load 27 > 24.");
  assert.equal(waitLine({ ...w, reasons: [] }, now), "The next step has waited for a free agent since 04:40 UTC.");
  assert.equal(waitLine({ ...w, reasons: undefined }, now), "The next step has waited for a free agent since 04:40 UTC.");
  assert.equal(waitLine({ ...w, reasons: ["", "restart drain"] }, now), "The next step has waited since 04:40 UTC: restart drain.");
  assert.equal(waitLine(undefined, now), null);
  assert.equal(waitLine(null, now), null);
  assert.equal(waitLine({ ...w, since: "" }, now), null);
});
