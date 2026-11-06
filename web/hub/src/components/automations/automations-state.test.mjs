import assert from "node:assert/strict";
import { test } from "node:test";

// Everything here is in the viewer's local time, so pin the zone: Kolkata is
// UTC+5:30, which shifts both the clock and, for a late UTC time, the day.
process.env.TZ = "Asia/Kolkata";

const { automationKind, fileProblem, lastResults, localClock, nextRunWords, parseIso, problemOf, scheduleWords, sortAutomations, weekStrip } = await import("./automations-state.ts");

// Friday 6 Nov 2026, 10:00 in Kolkata (04:30 UTC).
const NOW = new Date("2026-11-06T04:30:00Z");

const sched = (o = {}) => ({
  id: "SCHED-1",
  title: "Nightly audit",
  kind: "task",
  status: "active",
  every_minutes: 1440,
  at_time: "02:00",
  next_at: "2026-11-07T02:00:00Z",
  state: "",
  fails: 0,
  last_rc: null,
  last_at: "",
  ...o,
});

test("automationKind maps the schedule kinds onto the three tiles", () => {
  assert.equal(automationKind({ kind: "task" }), "work");
  assert.equal(automationKind({ kind: "improve" }), "work");
  assert.equal(automationKind({ kind: "check" }), "run");
  assert.equal(automationKind({ kind: "watch" }), "run");
  assert.equal(automationKind({ kind: "clean" }), "automation");
  assert.equal(automationKind({ kind: "reflect" }), "automation");
  assert.equal(automationKind({ kind: "" }), "automation");
});

test("parseIso reads the hub's moments, zoneless ones as UTC", () => {
  assert.equal(parseIso("2026-11-06T04:30:00Z").getTime(), NOW.getTime());
  assert.equal(parseIso("2026-11-06T04:30:00").getTime(), NOW.getTime());
  assert.equal(parseIso(""), null);
  assert.equal(parseIso("soon"), null);
});

test("localClock turns a UTC time of day into the viewer's 24-hour clock", () => {
  assert.equal(localClock("02:00", NOW), "07:30");
  assert.equal(localClock("20:00", NOW), "01:30");
  assert.equal(localClock("9:05", NOW), "14:35");
  assert.equal(localClock("", NOW), "");
  assert.equal(localClock("noon", NOW), "");
});

test("scheduleWords says the cadence in local words", () => {
  assert.equal(scheduleWords(sched(), NOW), "Every day at 07:30");
  assert.equal(scheduleWords(sched({ every_minutes: 1440, at_time: "" }), NOW), "Every day");
  assert.equal(scheduleWords(sched({ every_minutes: 2880, at_time: "02:30" }), NOW), "Every 2 days at 08:00");
  assert.equal(scheduleWords(sched({ every_minutes: 360, at_time: "" }), NOW), "Every 6 hours");
  assert.equal(scheduleWords(sched({ every_minutes: 60, at_time: "" }), NOW), "Every hour");
  assert.equal(scheduleWords(sched({ every_minutes: 30, at_time: "" }), NOW), "Every 30 minutes");
  // Monday 9 Nov 03:30 UTC is Monday 09:00 in Kolkata.
  assert.equal(scheduleWords(sched({ every_minutes: 10080, at_time: "03:30", next_at: "2026-11-09T03:30:00Z" }), NOW), "Every Monday at 09:00");
  // Saturday 21:00 UTC is already Sunday in Kolkata: the local weekday wins.
  assert.equal(scheduleWords(sched({ every_minutes: 10080, at_time: "21:00", next_at: "2026-11-07T21:00:00Z" }), NOW), "Every Sunday at 02:30");
  assert.equal(scheduleWords(sched({ every_minutes: 10080, at_time: "", next_at: "" }), NOW), "Every week");
  assert.equal(scheduleWords(sched({ every_minutes: 20160, at_time: "09:00", next_at: "2026-11-09T09:00:00Z" }), NOW), "Every 2 weeks on Monday at 14:30");
  assert.equal(scheduleWords(sched({ every_minutes: 0, at_time: "" }), NOW), "");
});

test("nextRunWords is short and only for an active automation", () => {
  assert.equal(nextRunWords(sched({ next_at: "2026-11-06T04:50:00Z" }), NOW), "in 20 min");
  assert.equal(nextRunWords(sched({ next_at: "2026-11-06T07:40:00Z" }), NOW), "in 3 h");
  assert.equal(nextRunWords(sched({ next_at: "2026-11-09T04:30:00Z" }), NOW), "in 3 d");
  assert.equal(nextRunWords(sched({ next_at: "2026-11-06T04:00:00Z" }), NOW), "due now");
  assert.equal(nextRunWords(sched({ status: "paused" }), NOW), "");
  assert.equal(nextRunWords(sched({ next_at: "" }), NOW), "");
  assert.equal(nextRunWords(sched({ next_at: "2026-11-06T04:50:00Z" }), NOW.getTime()), "in 20 min");
});

test("weekStrip lays the next seven local days out with today first", () => {
  const days = weekStrip([], NOW);
  assert.equal(days.length, 7);
  assert.deepEqual(
    days.map((d) => d.weekday + " " + d.day),
    ["Fri 6", "Sat 7", "Sun 8", "Mon 9", "Tue 10", "Wed 11", "Thu 12"],
  );
  assert.deepEqual(days.map((d) => d.today), [true, false, false, false, false, false, false]);
  assert.equal(days[0].key, "2026-11-06");
  assert.equal(days[0].date.getHours(), 0);
  assert.deepEqual(days.map((d) => d.runs.length), [0, 0, 0, 0, 0, 0, 0]);
});

test("weekStrip puts one dot per run, expanding the cadence over the week", () => {
  const daily = sched({ id: "A", title: "Daily", every_minutes: 1440, next_at: "2026-11-07T02:00:00Z" });
  const days = weekStrip([daily], NOW);
  // 02:00 UTC is 07:30 local the same day; the first is Saturday, then one a day.
  assert.deepEqual(days.map((d) => d.runs.length), [0, 1, 1, 1, 1, 1, 1]);
  assert.equal(days[1].runs[0].id, "A");
  assert.equal(days[1].runs[0].at.toISOString(), "2026-11-07T02:00:00.000Z");
  assert.equal(days[6].runs[0].at.toISOString(), "2026-11-12T02:00:00.000Z");
});

test("weekStrip counts a late-UTC run on the local day it lands", () => {
  // 20:00 UTC on the 6th is 01:30 on the 7th in Kolkata.
  const days = weekStrip([sched({ every_minutes: 1440, next_at: "2026-11-06T20:00:00Z" })], NOW);
  assert.deepEqual(days.map((d) => d.runs.length), [0, 1, 1, 1, 1, 1, 1]);
});

test("weekStrip keeps an overdue run on today and leaves paused and ended ones out", () => {
  const overdue = sched({ id: "O", every_minutes: 1440, next_at: "2026-11-05T02:00:00Z" });
  const paused = sched({ id: "P", status: "paused", next_at: "2026-11-07T02:00:00Z" });
  const ended = sched({ id: "E", status: "ended", next_at: "2026-11-07T02:00:00Z" });
  const days = weekStrip([overdue, paused, ended], NOW);
  // Overdue lands now (today), then every day from there.
  assert.deepEqual(days.map((d) => d.runs.map((r) => r.id)), [["O"], ["O"], ["O"], ["O"], ["O"], ["O"], ["O"]]);
  assert.equal(days[0].runs[0].at.getTime(), NOW.getTime());
});

test("weekStrip handles a sub-day cadence and a one-off", () => {
  const hourly = sched({ id: "H", every_minutes: 360, next_at: "2026-11-06T06:30:00Z" });
  const once = sched({ id: "1", every_minutes: 0, next_at: "2026-11-08T06:30:00Z" });
  const days = weekStrip([hourly, once], NOW);
  // Today 12:00, 18:00, 00:00(next day) local -> 2 today, then 4 a day.
  assert.equal(days[0].runs.filter((r) => r.id === "H").length, 2);
  assert.equal(days[1].runs.filter((r) => r.id === "H").length, 4);
  assert.deepEqual(days.map((d) => d.runs.filter((r) => r.id === "1").length), [0, 0, 1, 0, 0, 0, 0]);
  // Runs on a day are in time order, across automations.
  const ts = days[2].runs.map((r) => r.at.getTime());
  assert.deepEqual(ts, [...ts].sort((a, b) => a - b));
});

test("weekStrip caps a broken cadence instead of looping", () => {
  const days = weekStrip([sched({ every_minutes: 1, next_at: "2026-11-06T04:30:00Z" })], NOW);
  const total = days.reduce((n, d) => n + d.runs.length, 0);
  assert.ok(total <= 400 && total > 0, String(total));
});

test("lastResults reads the last five runs from the history, oldest left", () => {
  const h = (what, o = {}) => ({ seq: 0, at: "2026-11-01T00:00:00Z", actor: "daemon", what, run: 0, rc: 0, timeout: false, seconds: 1, state: "", previous: "", job: "", job_status: "", job_agent: "", job_finished_at: "", queued: false, why: "", note: "", changed: [], origin: "", ...o });
  const entries = [
    h("ran", { rc: 1, state: "red", at: "2026-11-06T00:00:00Z" }),
    h("red", { at: "2026-11-06T00:00:00Z" }),
    h("ran", { rc: 0, state: "green", at: "2026-11-05T00:00:00Z" }),
    h("paused", { at: "2026-11-04T12:00:00Z" }),
    h("ran", { rc: null, at: "2026-11-04T00:00:00Z" }),
    h("ran", { rc: 0, state: "green", timeout: true, at: "2026-11-03T00:00:00Z" }),
    h("ran", { rc: 0, state: "green", at: "2026-11-02T00:00:00Z" }),
    h("ran", { rc: 0, state: "green", at: "2026-11-01T00:00:00Z" }),
    h("added"),
  ];
  const r = lastResults(entries, { kind: "check", state: "red", last_rc: 1, last_at: "2026-11-06T00:00:00Z" });
  assert.equal(r.from, "history");
  assert.deepEqual(
    r.results.map((x) => x.status),
    ["passed", "failed", "failed", "passed", "failed"],
  );
  assert.deepEqual(
    r.results.map((x) => x.detail),
    ["green", "timed out", "could not start", "green", "red"],
  );
  assert.equal(r.results[4].at, "2026-11-06T00:00:00Z");
});

test("lastResults reads a task's fired jobs by their status", () => {
  const h = (what, o = {}) => ({ seq: 0, at: "2026-11-01T00:00:00Z", actor: "daemon", what, run: 0, rc: null, timeout: false, seconds: 0, state: "", previous: "", job: "", job_status: "", job_agent: "", job_finished_at: "", queued: false, why: "", note: "", changed: [], origin: "", ...o });
  const entries = [
    h("fired", { job: "JOB-4", job_status: "running", queued: true }),
    h("skipped", { job: "JOB-3", queued: false }),
    h("fired", { job: "JOB-3", job_status: "failed", queued: true }),
    h("fired", { job: "JOB-2", job_status: "done", queued: true }),
    h("fired", { job: "JOB-1", job_status: "", queued: true }),
    h("run", { actor: "human" }),
  ];
  const r = lastResults(entries, { kind: "task", state: "", last_rc: null, last_at: "" });
  assert.deepEqual(
    r.results.map((x) => x.status),
    ["queued", "done", "failed", "skipped", "running"],
  );
  assert.equal(r.results[3].detail, "skipped, JOB-3 was still open");
});

test("lastResults falls back to the schedule's own last state when there is no history", () => {
  const red = lastResults(undefined, { kind: "check", state: "red", last_rc: 1, last_at: "2026-11-06T00:00:00Z" });
  assert.equal(red.from, "schedule");
  assert.deepEqual(red.results.map((x) => x.status), ["failed"]);
  const ok = lastResults(null, { kind: "watch", state: "ok", last_rc: 0, last_at: "2026-11-06T00:00:00Z" });
  assert.deepEqual(ok.results.map((x) => [x.status, x.detail]), [["passed", "quiet"]]);
  // A task knows nothing of its own; an empty history is still "history".
  assert.deepEqual(lastResults(undefined, { kind: "task", state: "", last_rc: null, last_at: "2026-11-06T00:00:00Z" }), { results: [], from: "none" });
  assert.deepEqual(lastResults([], { kind: "task", state: "", last_rc: null, last_at: "" }), { results: [], from: "history" });
  // Never run: nothing to show either way.
  assert.deepEqual(lastResults(undefined, { kind: "check", state: "", last_rc: null, last_at: "" }).results, []);
});

test("problemOf says what is wrong in one sentence", () => {
  assert.deepEqual(problemOf(sched({ kind: "check", state: "red", fails: 1 })), { text: "The check has failed.", tone: "failed" });
  assert.deepEqual(problemOf(sched({ kind: "check", state: "red", fails: 3 })), { text: "The check has failed 3 times in a row.", tone: "failed" });
  assert.deepEqual(problemOf(sched({ kind: "watch", state: "broken", fails: 2 })), { text: "The watch has found it broken 2 times in a row.", tone: "failed" });
  assert.deepEqual(problemOf(sched({ kind: "check", state: "", last_rc: null, last_at: "2026-11-05T00:00:00Z" })), { text: "Its last run could not start.", tone: "failed" });
  assert.equal(problemOf(sched({ kind: "check", state: "green" })), null);
  assert.equal(problemOf(sched({ kind: "check", state: "" })), null);
  assert.equal(problemOf(sched({ kind: "task", state: "red", fails: 5 })), null);
  assert.equal(problemOf(sched({ kind: "check", state: "red", status: "ended" })), null);
  // A paused check that is red still has its problem (resume is the fix).
  assert.equal(problemOf(sched({ kind: "check", state: "red", status: "paused" })).tone, "failed");
});

test("fileProblem takes the first problem and counts the rest", () => {
  assert.deepEqual(fileProblem(["Needs how often it runs.", "Needs a brief."], true), { text: "Needs how often it runs. (+1 more)", tone: "warning" });
  assert.deepEqual(fileProblem(["Needs a brief."], true), { text: "Needs a brief.", tone: "warning" });
  assert.deepEqual(fileProblem([], false), { text: "The file could not be read.", tone: "warning" });
  assert.deepEqual(fileProblem([], true), { text: "The daemon holds it.", tone: "warning" });
});

test("sortAutomations puts held and failing first, then live, paused, ended; stable within", () => {
  const list = [
    { id: "a", status: "active", failing: false },
    { id: "b", status: "paused", failing: false },
    { id: "c", status: "active", failing: true },
    { id: "d", status: "ended", failing: true },
    { id: "e", status: "held", failing: true },
    { id: "f", status: "active", failing: false },
    { id: "g", status: "pending", failing: false },
    { id: "h", status: "ended", failing: false },
  ];
  assert.deepEqual(
    sortAutomations(list).map((x) => x.id),
    ["c", "e", "a", "f", "b", "g", "d", "h"],
  );
});
