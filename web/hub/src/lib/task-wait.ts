// The wait line of an owner's task whose next step has sat in the queue past
// ten minutes. The daemon writes one job.owner-wait event per such wait,
// naming every hold it saw (crates/pulse-runner/src/owner_wait.rs: the
// machine's load over daemon.max_load, every slot busy with the fleet's work,
// a restart drain, the disk, the daily cap); the hub hands the oldest open one
// over as Task.waiting. So the page says "The next step has waited since
// 04:40 UTC: lanes full, load 27 > 24." instead of "waiting for a free agent"
// all day. Pure: no DOM, no fetch.
import type { TaskWait } from "./api";

// "04:40 UTC" when `since` falls on `now`'s UTC day, else "Nov 3, 04:40 UTC".
export function waitSince(since: string, now: number = Date.now()): string {
  const t = new Date(since);
  if (Number.isNaN(t.getTime())) return since;
  const hh = String(t.getUTCHours()).padStart(2, "0");
  const mm = String(t.getUTCMinutes()).padStart(2, "0");
  const clock = `${hh}:${mm} UTC`;
  const today = new Date(now);
  if (
    t.getUTCFullYear() === today.getUTCFullYear() &&
    t.getUTCMonth() === today.getUTCMonth() &&
    t.getUTCDate() === today.getUTCDate()
  ) {
    return clock;
  }
  const month = t.toLocaleString("en-US", { month: "short", timeZone: "UTC" });
  return `${month} ${t.getUTCDate()}, ${clock}`;
}

// The sentence for a named wait, or null when there is none to name.
export function waitLine(w: TaskWait | null | undefined, now: number = Date.now()): string | null {
  if (!w || !w.since) return null;
  const reasons = (w.reasons ?? []).filter((r) => typeof r === "string" && r.trim() !== "");
  const since = waitSince(w.since, now);
  if (!reasons.length) return `The next step has waited for a free agent since ${since}.`;
  return `The next step has waited since ${since}: ${reasons.join(", ")}.`;
}
