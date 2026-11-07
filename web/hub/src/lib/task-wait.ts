// The wait line of an owner's task whose next step has sat in the queue past
// ten minutes. The daemon writes one job.owner-wait event per such wait,
// naming every hold it saw (crates/pulse-runner/src/owner_wait.rs: the
// machine's load over daemon.max_load, every slot busy with the fleet's work,
// a restart drain, the disk, the daily cap); the hub hands the oldest open one
// over as Task.waiting. So the page says "The next step has waited since
// 04:40 UTC because all agent slots were taken and the machine's load was
// 27, over its limit of 24." instead of "waiting for a free agent" all day.
// Pure: no DOM, no fetch.
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

// One of the daemon's short holds (owner_wait.rs short_reason and
// tick_reasons: "lanes full", "load 27 > 24", "restart drain") in the
// owner's words. A hold this list doesn't know reads as the daemon wrote it.
export function holdWords(reason: string): string {
  const r = reason.trim();
  let m = /^load (\d+(?:\.\d+)?) > (\d+(?:\.\d+)?)$/.exec(r);
  if (m) return `the machine's load was ${m[1]}, over its limit of ${m[2]}`;
  m = /^memory (\d+) MB < (\d+) MB$/.exec(r);
  if (m) return `only ${m[1]} MB of memory was free, under the ${m[2]} MB it keeps`;
  switch (r) {
    case "lanes full":
      return "all agent slots were taken";
    case "machine load":
      return "the machine was too busy";
    case "memory pressure":
      return "memory ran low";
    case "restart drain":
      return "PULSE was restarting";
    case "disk floor":
      return "the disk was nearly full";
    case "bulk disk away":
      return "the bulk disk wasn't connected";
    case "daily cap reached":
      return "today's limit was reached";
    default:
      return r;
  }
}

// "a", "a and b", "a, b and c".
function andList(items: string[]): string {
  if (items.length < 2) return items.join("");
  return items.slice(0, -1).join(", ") + " and " + items[items.length - 1];
}

// The sentence for a named wait, or null when there is none to name.
export function waitLine(w: TaskWait | null | undefined, now: number = Date.now()): string | null {
  if (!w || !w.since) return null;
  const reasons = (w.reasons ?? []).filter((r) => typeof r === "string" && r.trim() !== "");
  const since = waitSince(w.since, now);
  if (!reasons.length) return `The next step has waited for a free agent since ${since}.`;
  return `The next step has waited since ${since} because ${andList(reasons.map(holdWords))}.`;
}
