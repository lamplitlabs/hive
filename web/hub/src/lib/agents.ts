// Pure wording for the Agents page (src/app/agents/page.tsx): how a removed
// agent's row reads and how the runner's worker counts read in the summary
// line. Kept free of `@/` imports so node --test can load it (agents.test.mjs).
import type { Runner } from "./api";

// pulse_board::RETIRE_REASON (crates/pulse-board/src/agents.rs): the reason
// a worker's row carries once its run has ended and nothing waited for it.
// Workers live for one run; there is no idle pool and nothing is kept ready.
export const RETIRE_REASON = "retired after its run; workers are not kept idle";

export const isRetireReason = (reason?: string | null) => (reason || "").trim().toLowerCase().startsWith("retired after its run");

// The status badge of a pruned row: "Retired" for a worker whose run ended,
// "Removed" for one pruned after days without useful output.
export const prunedWord = (reason?: string | null) => (isRetireReason(reason) ? "Retired" : "Removed");

// The second line of a pruned row. "no useful output since … (limit 14
// days)" reads "Removed after 14 days without useful output"; the retire
// reason reads "Retired after its run".
export function prunedText(reason: string): string {
  if (isRetireReason(reason)) return "Retired after its run";
  const days = /limit\s+(\d+)\s+days?/i.exec(reason);
  if (days && /useful/i.test(reason)) return "Removed after " + days[1] + " days without useful output";
  return "Removed: " + reason;
}

const num = (n: unknown): string => Number(n || 0).toLocaleString("en-US");
const plural = (n: number, word: string): string => num(n) + " " + word + (n === 1 ? "" : "s");

// The summary's worker sentence. A worker exists only while it has a job:
// `busy` ones hold a lease, `idle` ones are between dispatch and their run's
// start (or holding held work). None are kept in a pool, so the sentence
// counts only what is there: "2 workers active, 1 between runs."
export function workersLine(r: Pick<Runner, "available" | "workers"> | undefined | null): string {
  if (!r?.available || !r.workers?.pool) return "";
  const w = r.workers;
  return plural(w.pool, "worker") + " active" + (w.idle ? ", " + num(w.idle) + " between runs." : ".");
}
