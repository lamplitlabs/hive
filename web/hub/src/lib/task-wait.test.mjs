import assert from "node:assert/strict";
import { test } from "node:test";
import { holdWords, waitLine, waitSince } from "./task-wait.ts";

const now = Date.parse("2026-11-03T15:00:00Z");

test("waitSince names the clock today and the day before that", () => {
  assert.equal(waitSince("2026-11-03T04:40:00Z", now), "04:40 UTC");
  assert.equal(waitSince("2026-11-02T23:05:00Z", now), "Nov 2, 23:05 UTC");
  assert.equal(waitSince("not a date", now), "not a date");
});

test("waitLine names the holds the daemon saw", () => {
  const w = { job: "job-1", project: "p", title: "T", since: "2026-11-03T04:40:00Z", at: "2026-11-03T04:51:00Z", reasons: ["lanes full", "load 27 > 24"] };
  assert.equal(waitLine(w, now), "The next step has waited since 04:40 UTC because all agent slots were taken and the machine's load was 27, over its limit of 24.");
  assert.equal(waitLine({ ...w, reasons: [] }, now), "The next step has waited for a free agent since 04:40 UTC.");
  assert.equal(waitLine({ ...w, reasons: undefined }, now), "The next step has waited for a free agent since 04:40 UTC.");
  assert.equal(waitLine({ ...w, reasons: ["", "restart drain"] }, now), "The next step has waited since 04:40 UTC because Hive was restarting.");
  assert.equal(waitLine({ ...w, reasons: ["lanes full", "disk floor", "daily cap reached"] }, now), "The next step has waited since 04:40 UTC because all agent slots were taken, the disk was nearly full and today's limit was reached.");
  assert.equal(waitLine(undefined, now), null);
  assert.equal(waitLine(null, now), null);
  assert.equal(waitLine({ ...w, since: "" }, now), null);
});

test("holdWords says each of the daemon's holds in plain words", () => {
  assert.equal(holdWords("lanes full"), "all agent slots were taken");
  assert.equal(holdWords("load 27 > 24"), "the machine's load was 27, over its limit of 24");
  assert.equal(holdWords("load 3.5 > 2"), "the machine's load was 3.5, over its limit of 2");
  assert.equal(holdWords("memory 512 MB < 1024 MB"), "only 512 MB of memory was free, under the 1024 MB it keeps");
  assert.equal(holdWords("machine load"), "the machine was too busy");
  assert.equal(holdWords("memory pressure"), "memory ran low");
  assert.equal(holdWords("restart drain"), "Hive was restarting");
  assert.equal(holdWords("bulk disk away"), "the bulk disk wasn't connected");
  // A hold the console doesn't know yet reads as the daemon wrote it.
  assert.equal(holdWords(" a brand-new hold "), "a brand-new hold");
});
