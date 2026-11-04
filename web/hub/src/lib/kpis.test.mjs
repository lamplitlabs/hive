import assert from "node:assert/strict";
import { test } from "node:test";
import { changeText, changeTone, costText, dayLabel, kpiChartRows, kpiFigure, kpiPath, kpiPreviousWindow, kpiSummaryLine, kpiTokensFigure, kpiWindow, kpisEmpty, perDayText, tokensText } from "./kpis.ts";

const day = (d, over = {}) => ({ day: d, sessions: 0, prompt_tokens: 0, completion_tokens: 0, cost: 0, jobs_done: 0, jobs_failed: 0, landings: 0, tasks_done: 0, ...over });
// Fourteen days, Sept 1 to Sept 14: landings climb 1..14, one session a day.
const series = Array.from({ length: 14 }, (_, i) => day("2026-09-" + String(i + 1).padStart(2, "0"), { landings: i + 1, sessions: 1, prompt_tokens: 1000, completion_tokens: 100 }));
const payload = { series, totals: { sessions: 14, prompt_tokens: 14000, completion_tokens: 1400, cost: 0, jobs_done: 0, jobs_failed: 0, landings: 105, tasks_done: 0 } };

test("the path carries the day count and the optional project", () => {
  assert.equal(kpiPath(30), "kpis?days=30");
  assert.equal(kpiPath(7, "demo-app"), "kpis?days=7&project=demo-app");
  assert.equal(kpiPath(7, ""), "kpis?days=7");
});

test("the window is the last N days, the previous window the N before it", () => {
  assert.deepEqual(kpiWindow(payload, 7).map((r) => r.day), ["2026-09-08", "2026-09-09", "2026-09-10", "2026-09-11", "2026-09-12", "2026-09-13", "2026-09-14"]);
  assert.deepEqual(kpiPreviousWindow(payload, 7).map((r) => r.landings), [1, 2, 3, 4, 5, 6, 7]);
  // A window longer than the series takes what there is and compares with nothing.
  assert.equal(kpiWindow(payload, 30).length, 14);
  assert.deepEqual(kpiPreviousWindow(payload, 30), []);
  // Missing payloads are empty, not errors.
  assert.deepEqual(kpiWindow(null, 7), []);
  assert.deepEqual(kpiPreviousWindow(undefined, 7), []);
});

test("a figure sums the window, sparks per day and compares with the window before", () => {
  const f = kpiFigure(payload, "landings", 7);
  assert.equal(f.total, 8 + 9 + 10 + 11 + 12 + 13 + 14);
  assert.deepEqual(f.spark, [8, 9, 10, 11, 12, 13, 14]);
  assert.equal(f.perDay, 77 / 7);
  // 77 against 28 before: +175%.
  assert.equal(f.change, 175);
  // No full previous window: no comparison.
  assert.equal(kpiFigure(payload, "landings", 14).change, null);
  assert.equal(kpiFigure(payload, "landings", 30).change, null);
  // A previous window of zeros is not a baseline either.
  assert.equal(kpiFigure(payload, "jobs_done", 7).change, null);
  // Empty payload: zeros all round.
  assert.deepEqual(kpiFigure(null, "sessions", 7), { total: 0, spark: [], perDay: 0, change: null });
});

test("the tokens figure adds prompt and completion tokens per day", () => {
  const f = kpiTokensFigure(payload, 7);
  assert.equal(f.total, 7 * 1100);
  assert.deepEqual(f.spark, [1100, 1100, 1100, 1100, 1100, 1100, 1100]);
  assert.equal(f.change, 0);
  const grew = { series: [...series.slice(0, 7), ...series.slice(7).map((r) => ({ ...r, completion_tokens: 1100 }))] };
  assert.equal(kpiTokensFigure(grew, 7).change, 91);
  assert.deepEqual(kpiTokensFigure(null, 7), { total: 0, spark: [], perDay: 0, change: null });
});

test("tokens and money read the way a person says them", () => {
  assert.equal(tokensText(0), "0");
  assert.equal(tokensText(950), "950");
  assert.equal(tokensText(1_200), "1.2K");
  assert.equal(tokensText(84_000), "84K");
  assert.equal(tokensText(1_234_567), "1.2M");
  assert.equal(tokensText(58_000_000), "58M");
  assert.equal(tokensText(1_800_000_000), "1.8B");
  assert.equal(tokensText(12_000_000_000), "12B");
  assert.equal(costText(0), "$0.00");
  assert.equal(costText(0.35), "$0.35");
  assert.equal(costText(12.4), "$12.40");
  assert.equal(costText(250.7), "$251");
  assert.equal(costText(1240), "$1,240");
});

test("a per-day rate rounds to what reads well", () => {
  assert.equal(perDayText(0), "0/day");
  assert.equal(perDayText(0.04), "<0.1/day");
  assert.equal(perDayText(0.25), "0.3/day");
  assert.equal(perDayText(1.5), "1.5/day");
  assert.equal(perDayText(3), "3/day");
  assert.equal(perDayText(12.4), "12/day");
});

test("the change line and its tone depend on direction and on what is counted", () => {
  assert.equal(changeText({ change: 12, total: 50 }, "7d"), "+12% vs the 7 days before");
  assert.equal(changeText({ change: -30, total: 50 }, "30d"), "-30% vs the 30 days before");
  assert.equal(changeText({ change: 0, total: 50 }, "14d"), "Same as the 14 days before");
  assert.equal(changeText({ change: null, total: 50 }, "7d"), "New in the last 7 days");
  assert.equal(changeText({ change: null, total: 0 }, "7d"), "Nothing yet");
  // More landings is good; more failures or spend is not.
  assert.equal(changeTone({ change: 12 }, "up-good"), "ok");
  assert.equal(changeTone({ change: -12 }, "up-good"), "bad");
  assert.equal(changeTone({ change: 12 }, "up-bad"), "bad");
  assert.equal(changeTone({ change: -12 }, "up-bad"), "ok");
  assert.equal(changeTone({ change: 12 }, "neutral"), "neutral");
  assert.equal(changeTone({ change: 0 }, "up-good"), "neutral");
  assert.equal(changeTone({ change: null }, "up-bad"), "neutral");
});

test("chart rows carry a short UTC day label and a tokens total", () => {
  assert.equal(dayLabel("2026-09-07"), "Sep 7");
  assert.equal(dayLabel("2026-12-25"), "Dec 25");
  assert.equal(dayLabel("not a day"), "not a day");
  const rows = kpiChartRows([day("2026-09-07", { prompt_tokens: 500, completion_tokens: 50, landings: 3, jobs_failed: 1 })]);
  assert.deepEqual(rows, [{ day: "2026-09-07", label: "Sep 7", landings: 3, jobs_done: 0, jobs_failed: 1, tasks_done: 0, sessions: 0, tokens: 550, cost: 0 }]);
});

test("the summary line names the window's landings and tokens", () => {
  assert.equal(kpiSummaryLine(payload, "7d"), "77 landings, 7.7K tokens in 7 days");
  assert.equal(kpiSummaryLine({ series: [day("2026-09-07", { landings: 1 })] }, "7d"), "1 landing, 0 tokens in 7 days");
  assert.equal(kpiSummaryLine(null, "30d"), "0 landings, 0 tokens in 30 days");
});

test("an all-zero payload is empty, a missing one too", () => {
  assert.equal(kpisEmpty(null), true);
  assert.equal(kpisEmpty({ totals: day("x") }), true);
  assert.equal(kpisEmpty(payload), false);
  assert.equal(kpisEmpty({ totals: { ...day("x"), tasks_done: 1 } }), false);
});
