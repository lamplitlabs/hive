import assert from "node:assert/strict";
import { test } from "node:test";
import { CAP_WARNING_TAIL, capWarning, dailyCap, runsTodayOf } from "./cap-warning-state.ts";

const runner = (runs_today, extra = {}) => ({ available: true, jobs: { queued: 0, leased: 0, blocked: 0, running: 0, runs_today, cap: 0 }, ...extra });

test("dailyCap reads the value the way the runner does", () => {
  for (const off of ["", "0", " 0 ", "none", "None", "off", "unlimited"]) assert.equal(dailyCap(off), 0, JSON.stringify(off));
  assert.equal(dailyCap("10"), 10);
  assert.equal(dailyCap(" 174 "), 174);
  assert.equal(dailyCap("1e3"), null);
  assert.equal(dailyCap("-5"), null);
  assert.equal(dailyCap("ten"), null);
  assert.equal(dailyCap("1"), 1);
});

test("runsTodayOf reads the runner payload and nothing else", () => {
  assert.equal(runsTodayOf(undefined), undefined);
  assert.equal(runsTodayOf(null), undefined);
  assert.equal(runsTodayOf({ available: false }), undefined);
  assert.equal(runsTodayOf(runner(174)), 174);
  assert.equal(runsTodayOf(runner(0)), 0);
  assert.equal(runsTodayOf({ available: true, jobs: {} }), undefined);
  assert.equal(runsTodayOf({ available: true, jobs: { runs_today: "7" } }), undefined);
});

test("the incident: 174 runs today and a cap of 10 warns in the owner's words", () => {
  assert.equal(capWarning("10", 174), "Today's 174 runs already reach this cap: no new run starts until midnight UTC. Set 0 for no cap.");
  assert.ok(capWarning("10", 174).endsWith(CAP_WARNING_TAIL));
});

test("the cap exactly at today's count warns too, one above it does not", () => {
  assert.notEqual(capWarning("174", 174), "");
  assert.equal(capWarning("175", 174), "");
  assert.equal(capWarning("1", 1), "Today's 1 run already reaches this cap: no new run starts until midnight UTC. Set 0 for no cap.");
});

test("no cap, an unknown count or an unreadable value stay quiet", () => {
  assert.equal(capWarning("0", 174), "");
  assert.equal(capWarning("", 174), "");
  assert.equal(capWarning("none", 174), "");
  assert.equal(capWarning("10", undefined), "");
  assert.equal(capWarning("abc", 174), "");
  assert.equal(capWarning("-1", 174), "");
  assert.equal(capWarning("10", 0), "");
  assert.equal(capWarning("10", 9), "");
});

test("large counts are formatted with thousands separators", () => {
  assert.equal(capWarning("1000", 1234), "Today's 1,234 runs already reach this cap: no new run starts until midnight UTC. Set 0 for no cap.");
});
