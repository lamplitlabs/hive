// Whether a daily cap (the jobs_per_day setting) stops the fleet right now.
// The runner counts the distinct jobs that ran today across every machine
// (runs_today in GET /api/v1/runner) and once that count reaches the cap no
// new run starts until midnight UTC - silently, from the owner's side, which
// is how 174 runs met a cap of 10 one afternoon. The settings page asks this
// module while the owner types a value and again once it is saved, so the
// row itself says that the day is already used up. Pure: no DOM, no fetch.
import type { Runner } from "./api";

export const CAP_WARNING_TAIL = "no new run starts until midnight UTC. Set 0 for no cap.";

// Mirrors daily_cap in crates/pulse-runner/src/run.rs: empty, 0, none, off
// and unlimited mean no cap; a non-negative whole number is the cap. Anything
// else (which the store refuses to save anyway) is null, so nothing warns on
// a half-typed value.
export function dailyCap(value: string): number | null {
  const raw = value.trim().toLowerCase();
  if (raw === "" || raw === "0" || raw === "none" || raw === "off" || raw === "unlimited") return 0;
  if (!/^\d+$/.test(raw)) return null;
  return Number(raw);
}

// Today's count from the runner payload; undefined while the payload has not
// arrived or the runner is not available on this console.
export function runsTodayOf(runner: Pick<Runner, "available" | "jobs"> | undefined | null): number | undefined {
  if (!runner || !runner.available) return undefined;
  const n = runner.jobs?.runs_today;
  return typeof n === "number" && Number.isFinite(n) ? n : undefined;
}

// The warning for a jobs_per_day value, or "" when the value does not stop
// the fleet today: the cap is off, is still above today's count, is not a
// number yet, or today's count is unknown.
export function capWarning(value: string, runsToday: number | undefined): string {
  const cap = dailyCap(value);
  if (cap === null || cap <= 0 || runsToday === undefined || runsToday < cap) return "";
  const n = runsToday.toLocaleString("en-US");
  return "Today's " + n + (runsToday === 1 ? " run already reaches" : " runs already reach") + " this cap: " + CAP_WARNING_TAIL;
}
