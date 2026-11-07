// The live feed's words for the store's events: what an event's action reads
// as after the actor's name ("daemon found pull request #41 conflicts with
// main"), and the second line some events carry. Pure: no React and no
// runtime imports, so node --test can load it (the -state.ts pattern);
// components/pulse/live-feed.tsx renders it. The few sentences that need the
// setting labels or the status words stay there.
import type { StoreEvent } from "./api";

// What a post of each kind reads as in a sentence: "aurora picked up a thread".
export const POSTED: Record<string, string> = {
  claim: "picked up a thread",
  handoff: "handed work over",
  finding: "posted a finding",
  question: "asked a question",
  answer: "answered a question",
  suggestion: "made a suggestion",
  opinion: "gave an opinion",
  vote: "voted",
  amendment: "proposed a rule change",
  result: "posted a result",
  notice: "posted a notice",
  proposal: "made a proposal",
};

// A task's new status as what the agent did: "started on a task".
export const MOVED: Record<string, string> = {
  needs_owner: "asked you about a task",
  review: "asked you about a task",
  in_progress: "started on a task",
  planned: "planned a task",
  done: "finished a task",
  cancelled: "cancelled a task",
  open: "reopened a task",
};

// The rest of the store's actions, as verbs. Unknown actions read as their
// own words ("thing.happened" reads "thing happened"). For the landing and
// review events "a pull request" becomes "pull request #41" and "main" the
// branch it targets when the event names them (see eventVerb).
export const VERB: Record<string, string> = {
  "compute.requested": "asked for an extra machine",
  "compute.approved": "approved an extra machine",
  "compute.denied": "turned down an extra machine",
  "compute.released": "released an extra machine",
  "job.add": "queued a job",
  "job.done": "finished a job",
  "job.failed": "stopped a job that failed",
  "job.cancelled": "cancelled a job",
  // pulse-board jobs.rs and the runner's recover.rs: a job that ended badly
  // was tried again; the outcome picks the sentence (RECOVERED).
  "job.recover": "picked up a job that ended badly",
  // yield_lane.rs: a fleet run held so your own queued job gets its place.
  "job.yield": "paused a job to make room for yours",
  "message.sent": "sent a message",
  "record.created": "added a record",
  "record.updated": "updated a record",
  "record.revised": "updated a record",
  "executor.add": "added a machine",
  "executor.remove": "removed a machine",
  "executor.removed": "removed a machine",
  "task.created": "created a task",
  "task.amended": "changed a task",
  "task.cancelled": "cancelled a task",
  "land.landed": "merged finished work",
  "land.batch": "merged several finished branches, checked as one",
  "land.batch-failed": "could not merge several finished branches as one",
  "land.pushed": "pushed merged work",
  "land.held": "held back a merge",
  "land.reverted": "saw merged work reverted",
  // delivery.rs: the finished work's pull request was opened, or updated
  // (LAND_PR picks the sentence from the event's detail).
  "land.pr": "opened a pull request",
  "land.pr-checks": "saw the checks fail on a pull request",
  "land.pr-closed": "saw a pull request closed without being merged",
  "land.pr-conflict": "found a pull request conflicts with main and cannot merge",
  "land.pr-held": "held back a pull request until the work changes",
  "land.pr-refreshed": "brought a pull request up to date with main",
  "land.pr-rebuilt": "rebuilt a pull request without the commits of the one it built on, which was closed",
  "land.rebase": "asked a worker to redo finished work that no longer fits on main",
  "land.merged": "saw a pull request merged",
  "land.missing": "found a finished job's branch missing, so it never arrived",
  "land.measured": "measured how a merged change affected the work after it",
  // review.rs: the pull request review loop.
  "review.queued": "asked an agent to review a pull request",
  "review.changes": "asked for changes on a pull request",
  "review.merged": "merged a pull request after its review",
  "review.failed": "could not finish a pull request's review",
  "review.owner": "asked you to decide on a pull request",
  "hook.failed": "had a check fail",
  "project.add": "added the project",
  "project.removed": "removed the project",
  "project.restored": "restored the project",
  "settings.set": "changed a setting",
  "memory.remembered": "kept a memory for later runs",
  "memory.forgotten": "forgot a memory",
  "agent.cleanup": "cleaned up after a run",
  "agent.retire": "retired a worker after its run",
  "daemon.sweep": "ran the runner's cleanup",
  // stage_budget.rs: one step of the runner's pass ran past its time.
  "daemon.slow-stage": "took longer than planned on one step of its pass",
  // awake.rs: the computer slept; runs in flight lost that time.
  "daemon.slept": "noticed the computer was asleep",
};

// job.recover's outcomes (recover.rs, pulse-board jobs.rs).
const RECOVERED: Record<string, string> = {
  retried: "tried a job again",
  replanned: "planned a job again from the start",
  requeued: "put a job back in the queue",
  held: "held a job back for you to look at",
};

// land.pr: one event for a new pull request and for each update of it.
function landPr(d: Record<string, unknown>): string {
  if (d.opened !== false) return VERB["land.pr"];
  if (d.refreshed === true) return "brought a pull request up to date with main";
  if (str(d.retargeted_from)) return "moved a pull request onto main";
  return "updated a pull request with new work";
}

export const rec = (d: unknown): Record<string, unknown> | null => (d && typeof d === "object" && !Array.isArray(d) ? (d as Record<string, unknown>) : null);
export const str = (v: unknown) => (typeof v === "string" ? v : "");
export const scalar = (v: unknown) => (typeof v === "string" || typeof v === "number" || typeof v === "boolean" ? String(v) : "");

// The author a post id ends with: "20260901T203300Z-cinder" -> "cinder".
const postAuthor = (id: string) => id.replace(/^\d{8}T\d{6}Z-?/, "");
const isMachine = (a: string) => a.startsWith("executor.");

// A duration in words, from milliseconds: "40 seconds", "3 minutes",
// "1 hour 5 minutes".
export function durationWords(ms: number): string {
  const unit = (n: number, word: string) => n + " " + word + (n === 1 ? "" : "s");
  const sec = Math.round(ms / 1000);
  if (sec < 1) return "under a second";
  if (sec < 60) return unit(sec, "second");
  const min = Math.round(sec / 60);
  if (min < 60) return unit(min, "minute");
  const h = Math.floor(min / 60);
  return unit(h, "hour") + (min % 60 ? " " + unit(min % 60, "minute") : "");
}

// Names a pull request and its target branch when the event carries them.
function namePr(sentence: string, d: Record<string, unknown> | null): string {
  let s = sentence;
  const n = d?.number;
  if (typeof n === "number" && n > 0) s = s.replace("a pull request", "pull request #" + n);
  const base = str(d?.base).trim();
  if (base && base !== "main") s = s.replace(/\bmain\b/, base);
  return s;
}

// What the event's action reads as after the actor's name, or "" when the
// sentence needs words this module does not hold (a setting's label, a
// task's status) - live-feed.tsx says those.
export function eventVerb(e: Pick<StoreEvent, "action" | "subject" | "detail">): string {
  const d = rec(e.detail);
  const a = e.action;
  if (a === "mention") return "was mentioned" + (str(d?.by) ? " by " + str(d?.by) : "");
  if (a === "post" || a.startsWith("post.")) {
    const kind = a.startsWith("post.") ? a.slice(5) : str(d?.kind);
    return POSTED[kind] ?? "posted on the board";
  }
  if (a === "decide") {
    const answer = str(d?.answer);
    const who = postAuthor(e.subject || "");
    const what = who ? " to " + who + "\u2019s proposal" : "";
    return answer === "yes" ? "said yes" + what : answer === "no" ? "said no" + what : "answered" + (who ? " " + who + "\u2019s proposal" : "");
  }
  if (a === "message.sent") return "sent a message" + (str(d?.task_id) ? " about " + str(d?.task_id) : "");
  if (isMachine(a) && VERB[a] && e.subject) return VERB[a].replace(/a machine$/, "the machine " + e.subject);
  // The subject is the worker's name: "retired the worker cinder after its run".
  if (a === "agent.retire" && e.subject) return VERB[a].replace(/a worker/, "the worker " + e.subject);
  if (a === "job.recover") return RECOVERED[str(d?.outcome)] ?? VERB[a];
  if (a === "daemon.slept" && typeof d?.seconds === "number" && d.seconds > 0) return "noticed the computer was asleep for about " + durationWords(d.seconds * 1000);
  if (a === "land.pr") return namePr(d ? landPr(d) : VERB[a], d);
  if (a === "land.measured" && d?.regressed === true) return "measured a merged change and found the work after it got slower";
  if (VERB[a]) return a.startsWith("land.") || a.startsWith("review.") ? namePr(VERB[a], d) : VERB[a];
  return a
    .replace(/[._-]+/g, " ")
    .replace(/\bdaemon\b/g, "runner")
    .replace(/\bexecutor\b/g, "machine")
    .trim();
}

const names = (v: unknown): string[] => (Array.isArray(v) ? v.filter((f): f is string => typeof f === "string" && f.trim() !== "") : []);
const firstFive = (items: string[]) => items.slice(0, 5).join(", ") + (items.length > 5 ? " +" + (items.length - 5) + " more" : "");

// The second line an event's own fields make, or "" to let the feed fall
// back to the detail's summary, title, note or text.
export function eventDetailText(e: Pick<StoreEvent, "action" | "detail">): string {
  const o = rec(e.detail);
  if (!o) return "";
  const a = e.action;
  // A red base names the checks main itself fails (the runner's `failures`),
  // not its 1500-char verify tail.
  if (a === "land.base-red") {
    const failures = names(o.failures);
    if (failures.length) return (str(o.note) ? str(o.note) + ": " : "") + firstFive(failures);
  }
  // stage_budget.rs: {"stage", "ms", "budget_ms", "ran", "skipped"}.
  if (a === "daemon.slow-stage" && (str(o.stage) || typeof o.ms === "number")) {
    const stage = str(o.stage) || "A step";
    let line = stage.charAt(0).toUpperCase() + stage.slice(1);
    if (typeof o.ms === "number") line += " took " + durationWords(o.ms);
    if (typeof o.budget_ms === "number" && o.budget_ms > 0) line += (typeof o.ms === "number" ? ", over its budget of " : " ran over its budget of ") + durationWords(o.budget_ms);
    const skipped = names(o.skipped);
    if (skipped.length) line += "; " + firstFive(skipped) + " wait" + (skipped.length === 1 ? "s" : "") + " for the next pass";
    return line;
  }
  if (a === "land.pr-checks") {
    const checks = names(o.checks);
    if (checks.length) return "Failing: " + firstFive(checks);
  }
  if (a === "land.pr-conflict" || a === "land.rebase") {
    const files = names(o.files);
    if (files.length) return "Both changed " + firstFive(files);
  }
  if (a === "review.failed" && str(o.error)) return str(o.error);
  return "";
}
