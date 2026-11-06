// The Automations page's pure parts (spec "Automations"): the week strip,
// the kind tile each automation gets, its schedule in the viewer's own
// words and time, the next run, the last five results and the order of the
// cards. No React, only type imports, so node --test covers it.

import type { AutomationHistoryEntry, AutomationSchedule } from "../../lib/api";

// ----- kinds ----------------------------------------------------------------

// The tile a card wears. A task or an improvement queues work; a check or a
// watch is a run of a script; a clean-up (and anything else) is the generic
// automation.
export type AutomationTileKind = "work" | "run" | "automation";

export function automationKind(a: Pick<AutomationSchedule, "kind">): AutomationTileKind {
  switch (a.kind) {
    case "task":
    case "improve":
      return "work";
    case "check":
    case "watch":
      return "run";
    default:
      return "automation";
  }
}

const SCRIPT_KINDS = ["check", "watch"];
const isScript = (kind: string) => SCRIPT_KINDS.includes(kind);

// ----- time -----------------------------------------------------------------

const MINUTE = 60000;
const DAY_MINUTES = 1440;
const WEEK_MINUTES = 7 * DAY_MINUTES;
const WEEKDAY_SHORT = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const WEEKDAY_LONG = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

// An ISO moment as the hub sends it; one without a zone is UTC.
export function parseIso(iso: string | null | undefined): Date | null {
  if (!iso) return null;
  const s = /Z$|[+-]\d{2}:\d{2}$/.test(iso) ? iso : iso + "Z";
  const d = new Date(s);
  return isNaN(d.getTime()) ? null : d;
}

const two = (n: number) => String(n).padStart(2, "0");

// A UTC time of day ("02:00") on the viewer's clock ("07:30" in Kolkata),
// as the spec shows it: 24-hour, zero-padded. "" when it is not a time.
export function localClock(hhmm: string, now: Date = new Date()): string {
  const m = /^(\d{1,2}):(\d{2})$/.exec((hhmm || "").trim());
  if (!m) return "";
  const d = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate(), Number(m[1]), Number(m[2])));
  if (isNaN(d.getTime())) return "";
  return two(d.getHours()) + ":" + two(d.getMinutes());
}

// The cadence in the owner's words and time: "Every day at 08:00", "Every
// Monday at 09:00", "Every 6 hours", "Every 30 minutes". A weekly schedule
// names the local weekday of its next run when it has one.
export function scheduleWords(
  a: Pick<AutomationSchedule, "every_minutes" | "at_time"> & Partial<Pick<AutomationSchedule, "next_at">>,
  now: Date = new Date(),
): string {
  const m = Number(a.every_minutes) || 0;
  const at = localClock(a.at_time, now);
  const atWords = at ? " at " + at : "";
  if (m <= 0) return at ? "At " + at : "";
  if (m % WEEK_MINUTES === 0) {
    const n = m / WEEK_MINUTES;
    const next = parseIso(a.next_at);
    const dow = next ? WEEKDAY_LONG[next.getDay()] : "";
    if (n === 1) return (dow ? "Every " + dow : "Every week") + atWords;
    return "Every " + n + " weeks" + (dow ? " on " + dow : "") + atWords;
  }
  if (m % DAY_MINUTES === 0) {
    const n = m / DAY_MINUTES;
    return (n === 1 ? "Every day" : "Every " + n + " days") + atWords;
  }
  if (m % 60 === 0) {
    const n = m / 60;
    return n === 1 ? "Every hour" : "Every " + n + " hours";
  }
  return "Every " + m + " minutes";
}

// The next run, short: "in 40 min", "in 3 h", "in 2 d", "due now" once the
// daemon is behind it. "" for anything not active or without a time.
export function nextRunWords(a: Pick<AutomationSchedule, "status" | "next_at">, now: Date | number = new Date()): string {
  if (a.status !== "active") return "";
  const t = parseIso(a.next_at);
  if (!t) return "";
  const ms = t.getTime() - (typeof now === "number" ? now : now.getTime());
  if (ms <= 0) return "due now";
  const min = Math.round(ms / MINUTE);
  if (min < 60) return "in " + Math.max(1, min) + " min";
  const h = Math.round(min / 60);
  if (h < 48) return "in " + h + " h";
  return "in " + Math.round(h / 24) + " d";
}

// ----- the week strip -------------------------------------------------------

export interface WeekRun {
  id: string;
  title: string;
  at: Date;
}

export interface WeekDay {
  date: Date; // local midnight
  key: string; // "2026-11-06" in local time
  weekday: string; // "Fri"
  day: number; // 6
  today: boolean;
  runs: WeekRun[]; // in time order
}

// How many runs of one automation the strip expands at most: a 30-minute
// cadence over a week is 336, so this only guards a broken cadence.
const MAX_RUNS_PER_AUTOMATION = 400;

const localMidnight = (d: Date, plusDays = 0) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + plusDays);
const localKey = (d: Date) => d.getFullYear() + "-" + two(d.getMonth() + 1) + "-" + two(d.getDate());

// The next seven days from today, each with the runs due on it: every
// active automation's next run, then one more every `every_minutes` until
// the week ends. A run already due lands on today. Local time throughout.
export function weekStrip(
  list: Array<Pick<AutomationSchedule, "id" | "title" | "status" | "next_at" | "every_minutes">>,
  now: Date = new Date(),
): WeekDay[] {
  const starts = Array.from({ length: 8 }, (_, i) => localMidnight(now, i));
  const days: WeekDay[] = starts.slice(0, 7).map((date, i) => ({
    date,
    key: localKey(date),
    weekday: WEEKDAY_SHORT[date.getDay()],
    day: date.getDate(),
    today: i === 0,
    runs: [],
  }));
  const end = starts[7].getTime();
  for (const a of list) {
    if (a.status !== "active") continue;
    const first = parseIso(a.next_at);
    if (!first) continue;
    let t = Math.max(first.getTime(), now.getTime());
    const step = (Number(a.every_minutes) || 0) * MINUTE;
    let n = 0;
    while (t < end && n < MAX_RUNS_PER_AUTOMATION) {
      const i = starts.findIndex((s, k) => k < 7 && s.getTime() <= t && t < starts[k + 1].getTime());
      if (i >= 0) days[i].runs.push({ id: a.id, title: a.title, at: new Date(t) });
      n += 1;
      if (step <= 0) break;
      t += step;
    }
  }
  for (const d of days) d.runs.sort((x, y) => x.at.getTime() - y.at.getTime());
  return days;
}

// ----- the last five results -------------------------------------------------

// One result as a status word the StatusIcon knows ("passed", "failed",
// "done", "running", "queued", "skipped") and what happened, for its title.
export interface RunResult {
  status: "passed" | "failed" | "done" | "running" | "queued" | "skipped";
  detail: string;
  at: string;
}

export interface LastResults {
  results: RunResult[]; // oldest first, newest at the right; at most five
  from: "history" | "schedule" | "none";
}

const DONE_JOB = ["done", "landed", "completed", "complete", "finished", "succeeded", "merged", "delivered"];
const FAILED_JOB = ["failed", "error", "cancelled", "canceled", "rejected", "blocked", "stopped", "dead", "lost"];
const RUNNING_JOB = ["running", "in_progress", "working", "leased", "starting", "reviewing", "resumed"];

function runResult(e: AutomationHistoryEntry, kind: string): RunResult | null {
  if (e.what === "ran") {
    if (e.rc === null || e.rc === undefined) return { status: "failed", detail: "could not start", at: e.at };
    if (e.timeout) return { status: "failed", detail: "timed out", at: e.at };
    if (e.state === "red") return { status: "failed", detail: "red", at: e.at };
    if (e.state === "broken") return { status: "failed", detail: "found it broken", at: e.at };
    if (e.state === "green") return { status: "passed", detail: "green", at: e.at };
    if (e.state === "ok") return { status: "passed", detail: "quiet", at: e.at };
    return e.rc === 0 ? { status: "passed", detail: "exit 0", at: e.at } : { status: "failed", detail: "exit " + e.rc, at: e.at };
  }
  if (e.what === "fired") {
    const js = (e.job_status || "").toLowerCase();
    const job = e.job ? " " + e.job : "";
    if (DONE_JOB.includes(js)) return { status: "done", detail: "job" + job + " done", at: e.at };
    if (FAILED_JOB.includes(js)) return { status: "failed", detail: "job" + job + " " + js, at: e.at };
    if (RUNNING_JOB.includes(js)) return { status: "running", detail: "job" + job + " running", at: e.at };
    return { status: "queued", detail: "job" + job + " queued", at: e.at };
  }
  if (e.what === "skipped") return { status: "skipped", detail: e.job ? "skipped, " + e.job + " was still open" : "skipped", at: e.at };
  if (e.what === "red" && !isScript(kind)) return { status: "failed", detail: "red", at: e.at };
  return null;
}

// The last five runs of an automation from its history (newest first, as
// the hub sends it), oldest at the left. Without history the list payload
// knows only the last result of a check or watch (its state and exit code),
// so that one result stands in, marked `from: "schedule"`.
export function lastResults(
  entries: AutomationHistoryEntry[] | null | undefined,
  s: Pick<AutomationSchedule, "kind" | "state" | "last_rc" | "last_at">,
): LastResults {
  if (entries) {
    const out: RunResult[] = [];
    for (const e of entries) {
      const r = runResult(e, s.kind);
      if (r) out.push(r);
      if (out.length === 5) break;
    }
    if (out.length) return { results: out.reverse(), from: "history" };
  }
  if (isScript(s.kind) && s.last_at) {
    if (s.state === "red") return { results: [{ status: "failed", detail: "red", at: s.last_at }], from: "schedule" };
    if (s.state === "broken") return { results: [{ status: "failed", detail: "found it broken", at: s.last_at }], from: "schedule" };
    if (s.state === "green") return { results: [{ status: "passed", detail: "green", at: s.last_at }], from: "schedule" };
    if (s.state === "ok") return { results: [{ status: "passed", detail: "quiet", at: s.last_at }], from: "schedule" };
    if (s.last_rc === null) return { results: [{ status: "failed", detail: "could not start", at: s.last_at }], from: "schedule" };
  }
  return { results: [], from: entries ? "history" : "none" };
}

// ----- problems and order ------------------------------------------------------

export interface Problem {
  text: string; // one sentence
  tone: "failed" | "warning";
}

// What is wrong with a live automation, in one sentence, or null when
// nothing is. Only a check or a watch reports a result of its own; a task
// just queues a job, whose fate the Tasks page tells.
export function problemOf(s: Pick<AutomationSchedule, "kind" | "status" | "state" | "fails" | "last_rc" | "last_at">): Problem | null {
  if (s.status === "ended" || !isScript(s.kind)) return null;
  const n = Number(s.fails) || 0;
  const streak = n > 1 ? " " + n + " times in a row" : "";
  if (s.state === "red") return { text: "The check has failed" + streak + ".", tone: "failed" };
  if (s.state === "broken") return { text: "The watch has found it broken" + streak + ".", tone: "failed" };
  if (s.state && s.state !== "green" && s.state !== "ok") return { text: "Its last run reported " + s.state + ".", tone: "warning" };
  if (s.last_at && s.last_rc === null) return { text: "Its last run could not start.", tone: "failed" };
  return null;
}

// A held file's problem in one sentence: the first thing wrong with it
// (already in plain words), with a count of the rest.
export function fileProblem(problems: string[], readable: boolean): Problem {
  if (!readable && !problems.length) return { text: "The file could not be read.", tone: "warning" };
  if (!problems.length) return { text: "The daemon holds it.", tone: "warning" };
  const more = problems.length - 1;
  return { text: problems[0] + (more > 0 ? " (+" + more + " more)" : ""), tone: "warning" };
}

// Cards in the order the page shows them: anything held or failing first,
// then the live ones, then paused and not-yet-applied, ended last; within a
// group the hub's order (by what fires next) stays.
export function sortAutomations<T extends { status: string; failing: boolean }>(list: T[]): T[] {
  const rank = (x: T) => {
    if (x.failing && x.status !== "ended") return 0;
    if (x.status === "active") return 1;
    if (x.status === "ended") return 3;
    return 2;
  };
  return list.map((x, i) => [x, i] as const).sort((a, b) => rank(a[0]) - rank(b[0]) || a[1] - b[1]).map(([x]) => x);
}
