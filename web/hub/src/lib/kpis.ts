// The Home page's "Over time" panel: what the fleet got done, what it did
// on its own against how often it needed the owner, and what it spent, per
// day, from GET /api/v1/kpis. Pure functions, no DOM, so node --test can
// check the windows, the trends and the words.

import type { KpiDay, KpisPayload } from "./api";

export type KpiRange = "7d" | "14d" | "30d";
export const KPI_RANGES: KpiRange[] = ["7d", "14d", "30d"];
export const KPI_RANGE_LABEL: Record<KpiRange, string> = { "7d": "7 days", "14d": "14 days", "30d": "30 days" };
export const KPI_RANGE_DAYS: Record<KpiRange, number> = { "7d": 7, "14d": 14, "30d": 30 };

// The numbers a KPI cell shows: the window's total, its daily shape, and
// how it compares with the window just before it.
export interface KpiFigure {
  total: number;
  spark: number[];
  // Per-day average over the window, so a 7-day and a 30-day view compare.
  perDay: number;
  // Change against the previous window of the same length, in percent;
  // null when the previous window was empty (nothing to compare with).
  change: number | null;
}

export type KpiKey = Exclude<keyof KpiDay, "day">;

export function kpiPath(days: number, project?: string | null): string {
  const q = new URLSearchParams();
  q.set("days", String(days));
  if (project) q.set("project", project);
  return "kpis?" + q.toString();
}

// The last `days` entries of the series: the API zero-fills, so the slice
// is the window. Oldest first.
export function kpiWindow(p: Pick<KpisPayload, "series"> | null | undefined, days: number): KpiDay[] {
  const s = p?.series || [];
  return s.slice(Math.max(0, s.length - days));
}

// The window before the current one, for the comparison. Empty when the
// payload holds fewer days than two windows.
export function kpiPreviousWindow(p: Pick<KpisPayload, "series"> | null | undefined, days: number): KpiDay[] {
  const s = p?.series || [];
  const end = s.length - days;
  if (end <= 0) return [];
  return s.slice(Math.max(0, end - days), end);
}

const sum = (rows: KpiDay[], key: KpiKey): number => rows.reduce((a, r) => a + Number(r[key] || 0), 0);

function figureOf(cur: KpiDay[], prev: KpiDay[], days: number, value: (r: KpiDay) => number): KpiFigure {
  const total = cur.reduce((a, r) => a + value(r), 0);
  const before = prev.reduce((a, r) => a + value(r), 0);
  const change = prev.length === days && before > 0 ? Math.round(((total - before) / before) * 100) : null;
  return {
    total,
    spark: cur.map(value),
    perDay: cur.length ? total / cur.length : 0,
    change,
  };
}

export function kpiFigure(p: Pick<KpisPayload, "series"> | null | undefined, key: KpiKey, days: number): KpiFigure {
  return figureOf(kpiWindow(p, days), kpiPreviousWindow(p, days), days, (r) => Number(r[key] || 0));
}

// Prompt and completion tokens together: what the fleet spent in words.
export function kpiTokensFigure(p: Pick<KpisPayload, "series"> | null | undefined, days: number): KpiFigure {
  return figureOf(kpiWindow(p, days), kpiPreviousWindow(p, days), days, (r) => Number(r.prompt_tokens || 0) + Number(r.completion_tokens || 0));
}

// Tokens as a person reads them: "1.2M", "84K", "950".
export function tokensText(n: number): string {
  const v = Number(n || 0);
  if (v >= 1e9) return (v / 1e9).toFixed(v >= 1e10 ? 0 : 1) + "B";
  if (v >= 1e6) return (v / 1e6).toFixed(v >= 1e7 ? 0 : 1) + "M";
  if (v >= 1e3) return (v / 1e3).toFixed(v >= 1e4 ? 0 : 1) + "K";
  return String(Math.round(v));
}

// Money, when the provider prices its calls: "$12.40", "$0.35", "$1,240".
export function costText(n: number): string {
  const v = Number(n || 0);
  if (v >= 1000) return "$" + Math.round(v).toLocaleString("en-US");
  if (v >= 100) return "$" + v.toFixed(0);
  return "$" + v.toFixed(2);
}

// A per-day rate, rounded to what reads well: "12/day", "1.5/day", "0.2/day".
export function perDayText(n: number): string {
  if (n >= 10) return Math.round(n) + "/day";
  if (n >= 1) return (Math.round(n * 10) / 10).toString().replace(/\.0$/, "") + "/day";
  if (n > 0) return n.toFixed(1).replace(/^0\.0$/, "<0.1") + "/day";
  return "0/day";
}

// The change line under a KPI: "+12% vs previous 7 days", "no change",
// "new this window" when the window before was empty.
export function changeText(f: Pick<KpiFigure, "change" | "total">, range: KpiRange): string {
  if (f.change === null) return f.total > 0 ? "New in the last " + KPI_RANGE_LABEL[range] : "Nothing yet";
  if (f.change === 0) return "Same as the " + KPI_RANGE_LABEL[range] + " before";
  return (f.change > 0 ? "+" : "") + f.change + "% vs the " + KPI_RANGE_LABEL[range] + " before";
}

// Whether a change is good news depends on what is counted: more landings
// is good, more failures or spend is not.
export type KpiDirection = "up-good" | "up-bad" | "neutral";
export function changeTone(f: Pick<KpiFigure, "change">, dir: KpiDirection): "ok" | "bad" | "neutral" {
  if (f.change === null || f.change === 0 || dir === "neutral") return "neutral";
  const up = f.change > 0;
  if (dir === "up-good") return up ? "ok" : "bad";
  return up ? "bad" : "ok";
}

// One row per day for the chart, with a short label.
export interface KpiChartRow {
  day: string;
  label: string;
  landings: number;
  jobs_done: number;
  jobs_failed: number;
  tasks_done: number;
  sessions: number;
  tokens: number;
  cost: number;
  reviews: number;
  findings: number;
  vuln_fixes: number;
  owner_asks: number;
}

export function dayLabel(day: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (!m) return day;
  const d = new Date(Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])));
  return d.toLocaleDateString("en-US", { month: "short", day: "numeric", timeZone: "UTC" });
}

export function kpiChartRows(rows: KpiDay[]): KpiChartRow[] {
  return rows.map((r) => ({
    day: r.day,
    label: dayLabel(r.day),
    landings: r.landings,
    jobs_done: r.jobs_done,
    jobs_failed: r.jobs_failed,
    tasks_done: r.tasks_done,
    sessions: r.sessions,
    tokens: r.prompt_tokens + r.completion_tokens,
    cost: r.cost,
    reviews: Number(r.reviews || 0),
    findings: Number(r.findings || 0),
    vuln_fixes: Number(r.vuln_fixes || 0),
    owner_asks: Number(r.owner_asks || 0),
  }));
}

// The panel's summary line: "126 landings, 1.8B tokens in 7 days".
export function kpiSummaryLine(p: Pick<KpisPayload, "series"> | null | undefined, range: KpiRange): string {
  const days = KPI_RANGE_DAYS[range];
  const w = kpiWindow(p, days);
  const landings = sum(w, "landings");
  const tokens = sum(w, "prompt_tokens") + sum(w, "completion_tokens");
  return landings.toLocaleString("en-US") + (landings === 1 ? " landing" : " landings") + ", " + tokensText(tokens) + " tokens in " + KPI_RANGE_LABEL[range];
}

const count = (n: number, one: string, many: string): string => n.toLocaleString("en-US") + " " + (n === 1 ? one : many);

// "(28 merged on their own)", "(1 merged on its own)", "(all merged on
// their own)"; nothing when none was.
function mergedNote(reviews: number, merged: number): string {
  if (merged <= 0) return "";
  if (merged >= reviews) return reviews === 1 ? " (merged on its own)" : " (all merged on their own)";
  return " (" + merged.toLocaleString("en-US") + " merged on " + (merged === 1 ? "its" : "their") + " own)";
}

// What the fleet did without a human, and how often it needed one:
// "31 reviews (28 merged on their own), 1,454 findings, 56 vulnerability
// fixes; the owner was asked 290 times in 30 days". A part at zero is left
// out; a window with work but no ask says so ("never asked the owner");
// a window with nothing at all says "Nothing on its own yet".
export function kpiAutonomyLine(p: Pick<KpisPayload, "series"> | null | undefined, range: KpiRange): string {
  const days = KPI_RANGE_DAYS[range];
  const w = kpiWindow(p, days);
  const reviews = sum(w, "reviews");
  const merged = sum(w, "reviews_merged");
  const findings = sum(w, "findings");
  const fixes = sum(w, "vuln_fixes");
  const asks = sum(w, "owner_asks");
  const parts: string[] = [];
  if (reviews > 0) parts.push(count(reviews, "review", "reviews") + mergedNote(reviews, merged));
  if (findings > 0) parts.push(count(findings, "finding", "findings"));
  if (fixes > 0) parts.push(count(fixes, "vulnerability fix", "vulnerability fixes"));
  const when = " in " + KPI_RANGE_LABEL[range];
  if (!parts.length) return asks > 0 ? "The owner was asked " + count(asks, "time", "times") + when + ", nothing done on its own yet" : "Nothing on its own yet" + when;
  const done = parts.join(", ");
  if (asks === 0) return done + "; never asked the owner" + when;
  return done + "; the owner was asked " + count(asks, "time", "times") + when;
}

// The share of reviews the agents merged without the owner, in whole
// percent; null when there was no review to speak of.
export function reviewsAloneShare(p: Pick<KpisPayload, "series"> | null | undefined, days: number): number | null {
  const w = kpiWindow(p, days);
  const reviews = sum(w, "reviews");
  if (reviews <= 0) return null;
  return Math.round((Math.min(reviews, sum(w, "reviews_merged")) / reviews) * 100);
}

// True when the whole payload is zero: a fresh install, or one whose store
// has not seen a session yet. The panel then shows a hint instead of a
// flat chart.
export function kpisEmpty(p: Pick<KpisPayload, "totals"> | null | undefined): boolean {
  const t = p?.totals;
  if (!t) return true;
  return !(t.sessions || t.jobs_done || t.jobs_failed || t.landings || t.tasks_done || t.prompt_tokens || t.completion_tokens || t.reviews || t.reviews_merged || t.findings || t.vulns_found || t.vuln_fixes || t.owner_asks);
}
