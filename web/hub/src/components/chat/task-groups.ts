// The words and the order of the Chat page's one list. Every entry is a
// conversation: a plain one, one with sub-conversations under it, or a task
// - the owner's conversation with the lead agent, with one agent session per
// step the lead planned under it (`tasks` in GET /api/v1/chat). Kept apart
// from the page so the rules can be tested without a browser.

import type { ChatTask, ChatTaskSession } from "@/lib/api";
import type { PipMood } from "@/components/hive/mascots";

export type Tone = "run" | "wait" | "ok" | "neutral" | "bad";

export interface Mood {
  word: string;
  tone: Tone;
  pip: PipMood;
  dot: string;
}

// The colour of each tone's word, the same in the list, a header and a panel.
export const TONE_TEXT: Record<Tone, string> = {
  run: "text-primary",
  wait: "text-warn",
  ok: "text-ok",
  neutral: "text-muted-foreground",
  bad: "text-bad",
};

// "now", "4m", "2h", "3d": the time a list row gives a moment.
export function relShort(d: string, now: number): string {
  const t = Date.parse(d);
  if (!Number.isFinite(t)) return "";
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 60) return "now";
  if (s < 3600) return Math.round(s / 60) + "m";
  if (s < 86400) return Math.round(s / 3600) + "h";
  return Math.round(s / 86400) + "d";
}

// Where a pull request's review stands (PrState.state from the hub): the
// reviewer agent reads every pull request Hive opens, merges the simple ones
// and hands the rest to the owner. Two states are the owner's turn; five are
// the reviewer's or the daemon's (merge_waiting: approved, the merge waits
// for its checks or for GitHub); merged and closed leave the entry to its
// own status.
//
// A pull request that could not be opened is held: `held` the runner tries
// again on its own after a pause (a fault on the way to GitHub), and
// `held_for_owner` only the owner moves (a conflict: "this one is for
// you"), so it is the owner's turn too.
export const PR_NEEDS_OWNER = new Set(["needs_owner", "review_failed", "held_for_owner"]);

// The one rule for "waits on you", the hub's (hive_tasks::owner_turn, which
// the lifecycle view and GET /chat's owner_turn read): the lead asked the
// owner (status review), or the task's pull request is in a state only the
// owner moves. web/hub/mock/lifecycle.test.mjs holds it to the shared cases.
export function ownerTurn(status: string, pr: string | null | undefined): boolean {
  return status === "review" || (!!pr && PR_NEEDS_OWNER.has(pr));
}
export const PR_IN_REVIEW = new Set(["pr_open", "in_review", "merge_asked", "merge_waiting", "changes_requested"]);

export const NEEDS_REVIEW: Mood = { word: "Needs your review", tone: "wait", pip: "waiting", dot: "bg-warn" };

// Work done but without a pull request yet: a finished step whose branch is
// still on it, a conversation whose branch holds commits main lacks. The
// daemon opens the pull request on its next pass, so nobody is asked.
export const WAITING_FOR_PR: Mood = { word: "Opening a pull request", tone: "neutral", pip: "waiting", dot: "bg-muted-foreground/40" };

// The pull request could not be opened and the runner will not try again
// until the work changes: the owner decides (resolve the conflict, or say
// what to do on the task's thread).
// Said as what it is (2026-11-28, plain words): the work does not merge.
export const PR_HELD_FOR_OWNER: Mood = { word: "Merge conflict", tone: "wait", pip: "worried", dot: "bg-warn" };

// The pull request's word, the same for a step, a task and a conversation,
// so a section reads as one thing; null once it is merged or closed, and
// for a state this build does not know.
export function prMood(state: string | undefined | null): Mood | null {
  switch (state) {
    case "needs_owner":
      return NEEDS_REVIEW;
    case "review_failed":
      return { word: "Review failed", tone: "wait", pip: "worried", dot: "bg-warn" };
    case "pr_open":
      return { word: "Waiting for review", tone: "neutral", pip: "waiting", dot: "bg-primary/50" };
    case "in_review":
      return { word: "Being reviewed", tone: "run", pip: "busy", dot: "bg-primary" };
    case "merge_asked":
      return { word: "Merging", tone: "run", pip: "busy", dot: "bg-primary" };
    case "merge_waiting":
      return { word: "Approved, merging soon", tone: "neutral", pip: "waiting", dot: "bg-primary/50" };
    case "changes_requested":
      return { word: "Changes requested", tone: "neutral", pip: "curious", dot: "bg-primary/50" };
    case "held_for_owner":
      return PR_HELD_FOR_OWNER;
    default:
      return null;
  }
}

// The one line a pull request row on the Sessions page shows after its word
// when the reviewer approved it and the merge waits: what it waits for (the
// checks still running, or GitHub and when the merge is tried again), from
// the review.merge-wait event's note. Empty in every other state.
export function prWaitLine(state: string | undefined | null, note: string | undefined | null): string {
  if (state !== "merge_waiting") return "";
  const n = (note || "").trim().replace(/^approved;\s*/i, "");
  return n ? n.charAt(0).toUpperCase() + n.slice(1) : "";
}

// The section a pull request puts its entry in, or null when the entry's own
// status decides (no pull request, or one merged or closed).
export function prGroupOf(state: string | undefined | null): ListGroup | null {
  if (!state) return null;
  if (PR_NEEDS_OWNER.has(state)) return "needs_review";
  if (PR_IN_REVIEW.has(state)) return "review";
  return null;
}

// A step waits for the task's pull request once its job is done and its
// branch is still on the job: the daemon clears the branch when the pull
// request is merged or closed.
export function jobWaitingForPr(jobStatus: string, branch: string | undefined): boolean {
  return jobStatus === "done" && !!branch;
}

// What a task's mood is read from, the same for the list's row (GET /chat)
// and the task's own page (GET /tasks/{id}): its status, whether it has a
// lead, whether the lead waits on the owner, whether an agent is at a step,
// whether a finished step still waits for the pull request, and where the
// pull request's review stands once there is one.
export interface TaskState {
  status: string;
  lead: string;
  needs_you: boolean;
  working: boolean;
  waiting_for_pr?: boolean;
  pr?: string | null;
}

function taskClosed(status: string): boolean {
  return status === "done" || status === "cancelled";
}

// A task's state in the list's words, on the same scale as a conversation's:
// the owner's turn first, then what the agents are doing. The pull request
// outranks "Done" - the task is not on main until it is merged - but not an
// agent still at another step; the owner's Merge outranks everything but the
// lead's question.
export function taskMoodOf(t: TaskState): Mood {
  if (t.needs_you) return { word: "Asked you", tone: "wait", pip: "waiting", dot: "bg-warn" };
  const pr = prMood(t.pr);
  if (pr && PR_NEEDS_OWNER.has(t.pr!)) return pr;
  if (taskClosed(t.status)) {
    if (pr) return pr;
    if (t.waiting_for_pr) return WAITING_FOR_PR;
    if (t.status === "done") return { word: "Done", tone: "neutral", pip: "happy", dot: "bg-ok/50" };
    return { word: "Cancelled", tone: "neutral", pip: "asleep", dot: "bg-muted-foreground/40" };
  }
  if (t.working) return { word: "Working", tone: "run", pip: "busy", dot: "bg-primary" };
  if (pr) return pr;
  if (t.waiting_for_pr) return WAITING_FOR_PR;
  if (t.status === "open") return { word: t.lead ? "Starting" : "New", tone: "ok", pip: "curious", dot: "bg-ok" };
  if (t.status === "planned") return { word: "Planned", tone: "ok", pip: "curious", dot: "bg-ok" };
  return { word: "In progress", tone: "ok", pip: "happy", dot: "bg-ok" };
}

export function taskStateOf(t: ChatTask): TaskState {
  return {
    status: t.status,
    lead: t.lead,
    needs_you: t.needs_you,
    working: t.sessions.some((s) => s.status === "running" || s.job_status === "leased"),
    waiting_for_pr: t.sessions.some((s) => jobWaitingForPr(s.job_status, s.branch)),
    pr: taskPrState(t),
  };
}

// Where a task's pull request stands for its mood and its steps: the hold
// while one stands (the newest word on it), else the open one's review.
export function taskPrState(t: Pick<ChatTask, "pr" | "hold">): string | undefined {
  return t.hold?.state || t.pr?.state;
}

// Whether the task waits on the owner: the hub's word when it sends one,
// else the same rule read here.
export function taskOwnerTurn(t: ChatTask): boolean {
  return t.owner_turn ?? ownerTurn(t.status, taskPrState(t));
}

export function taskMood(t: ChatTask): Mood {
  return taskMoodOf(taskStateOf(t));
}

export function taskFinished(t: ChatTask): boolean {
  return taskClosed(t.status);
}

// A task's section, on the same scale as a conversation's: needs your
// review while its pull request waits on the owner, working while the lead
// waits on you or an agent is at a step, in review while its pull request
// is open or a finished step still waits for one, open while it is new or
// planned, finished once done or cancelled (a task is never archived).
export function taskGroupOf(t: TaskState): ListGroup {
  if (t.pr && PR_NEEDS_OWNER.has(t.pr)) return "needs_review";
  if (t.needs_you || (!taskClosed(t.status) && t.working)) return "working";
  const pr = prGroupOf(t.pr);
  if (pr) return pr;
  if (t.waiting_for_pr) return "review";
  if (taskClosed(t.status)) return "finished";
  return "open";
}

export function taskGroup(t: ChatTask): ListGroup {
  return taskGroupOf(taskStateOf(t));
}

// A step's state: the job's word, except while an agent's session runs it;
// a done step with its branch still on it reads where the task's pull
// request stands (`pr`), or that it waits for one.
export function stepWords(jobStatus: string, running: boolean, branch?: string, pr?: string | null): { word: string; tone: Tone } {
  if (running) return { word: "Working", tone: "run" };
  if (jobWaitingForPr(jobStatus, branch)) {
    const m = prMood(pr) ?? WAITING_FOR_PR;
    return { word: m.word, tone: m.tone };
  }
  switch (jobStatus) {
    case "queued":
      return { word: "Waiting", tone: "neutral" };
    case "blocked":
      return { word: "Blocked", tone: "wait" };
    case "leased":
      return { word: "Starting", tone: "run" };
    case "done":
      return { word: "Done", tone: "ok" };
    case "failed":
      return { word: "Failed", tone: "bad" };
    case "cancelled":
      return { word: "Cancelled", tone: "neutral" };
  }
  // A state this build does not know yet: shown as words, not an identifier.
  const spaced = jobStatus.trim().replace(/[_-]+/g, " ");
  return { word: spaced.charAt(0).toUpperCase() + spaced.slice(1), tone: "neutral" };
}

export function sessionWords(s: ChatTaskSession, pr?: string | null): { word: string; tone: Tone } {
  return stepWords(s.job_status, s.status === "running", s.branch, pr);
}

// The task's name from what the owner typed: its first line without list or
// heading marks, cut at a word when it runs long. The whole text is the
// brief, so nothing is lost.
export const TASK_TITLE_MAX = 120;

export function taskTitleOf(text: string): string {
  const first =
    text
      .split("\n")
      .map((l) => l.replace(/^(?:\s*(?:[#>*-]+|\d+[.)]))*\s*/, "").trim())
      .find((l) => l.length > 0) || "";
  if (first.length <= TASK_TITLE_MAX) return first;
  const cut = first.slice(0, TASK_TITLE_MAX);
  const at = cut.lastIndexOf(" ");
  return (at > TASK_TITLE_MAX / 2 ? cut.slice(0, at) : cut).replace(/[\s,;:.!?-]+$/, "") + "\u2026";
}

// ----- sub-conversations ----------------------------------------------------

// The list's sections: a conversation needs your review while its pull
// request waits on the owner, is working while its agent runs or waits on
// you, in review while its pull request is open or its branch holds commits
// the default branch lacks and nobody is at it, open while it is idle
// between turns, finished once the agent or the owner closed it (a message
// reopens it), and archived when it was put away - put away is the owner's
// word, so an archived conversation stays archived whatever its branch holds.
export type ListGroup = "needs_review" | "working" | "review" | "open" | "finished" | "archived";

export const GROUP_ORDER: ListGroup[] = ["needs_review", "working", "review", "open", "finished", "archived"];

export interface Grouped {
  id: string;
  status: string;
  parent?: string;
  created_at?: string;
  live?: { status: string } | null;
  // The hub's answer (GET /chat): commits on the branch that main lacks.
  waiting_to_land?: boolean;
  // The pull request opened for the conversation or from its branch (GET /chat).
  pr?: { state: string } | null;
}

export function groupOf(c: Grouped): ListGroup {
  const status = c.live?.status || c.status;
  const pr = prGroupOf(c.pr?.state);
  if (pr === "needs_review") return pr;
  if (status === "running" || status === "waiting") return "working";
  if (status === "archived") return "archived";
  if (pr) return pr;
  if (c.waiting_to_land && !c.pr) return "review";
  if (status === "closed") return "finished";
  return "open";
}

// A conversation with its sub-conversations under it: the ones its agent
// (or the owner, from its page) started. A sub-conversation whose parent is
// not in the list (put away while it carries on, or outside the latest
// rows) stands on its own.
export interface Family<T extends Grouped> {
  c: T;
  children: T[];
}

export function families<T extends Grouped>(rows: T[]): Family<T>[] {
  const ids = new Set(rows.map((c) => c.id));
  const top = rows.filter((c) => !c.parent || !ids.has(c.parent) || c.parent === c.id);
  const byParent = new Map<string, T[]>();
  for (const c of rows) {
    if (!c.parent || c.parent === c.id || !ids.has(c.parent)) continue;
    const list = byParent.get(c.parent) ?? [];
    list.push(c);
    byParent.set(c.parent, list);
  }
  // Children in the order they were started, oldest first: the way the
  // parent's transcript names them.
  return top.map((c) => ({
    c,
    children: (byParent.get(c.id) ?? []).sort((a, b) => (a.created_at ?? "").localeCompare(b.created_at ?? "")),
  }));
}

// A family's section is its most pressing member's: a finished parent with
// a sub-conversation still working is listed under Working.
export function familyGroup<T extends Grouped>(f: Family<T>): ListGroup {
  let at = GROUP_ORDER.indexOf(groupOf(f.c));
  for (const child of f.children) at = Math.min(at, GROUP_ORDER.indexOf(groupOf(child)));
  return GROUP_ORDER[at];
}

// ----- the one list ---------------------------------------------------------

// An entry of the list: a conversation with its sub-conversations, or a
// task with its steps. `at` is its latest movement - a sub-conversation's
// reply counts for its parent - so the list orders by what moved last.
export type ListEntry<T extends Grouped> = { kind: "conversation"; family: Family<T>; at: string } | { kind: "task"; task: ChatTask; at: string };

interface Moved {
  created_at?: string;
  last_at?: string;
}

function movedAt(c: Moved): string {
  return c.last_at || c.created_at || "";
}

export function listEntries<T extends Grouped & Moved>(rows: T[], tasks: ChatTask[]): ListEntry<T>[] {
  const entries: ListEntry<T>[] = families(rows).map((family) => ({
    kind: "conversation",
    family,
    at: [family.c, ...family.children].map(movedAt).reduce((a, b) => (a > b ? a : b), ""),
  }));
  for (const task of tasks) entries.push({ kind: "task", task, at: movedAt(task) });
  return entries.sort((a, b) => b.at.localeCompare(a.at));
}

export function entryGroup<T extends Grouped>(e: ListEntry<T>): ListGroup {
  return e.kind === "task" ? taskGroup(e.task) : familyGroup(e.family);
}

export function entryId<T extends Grouped>(e: ListEntry<T>): string {
  return e.kind === "task" ? e.task.id : e.family.c.id;
}

// The finished section starts folded: finished conversations pile up fast
// and crowd out the ones still moving. Folded, the section shows only its
// header with the count - and the entry the owner has open, so the page
// never hides the conversation it is showing. `openId` is the open
// conversation or task; a sub-conversation keeps its family visible.
export function foldedEntries<T extends Grouped>(entries: ListEntry<T>[], group: ListGroup, folded: boolean, openId: string): ListEntry<T>[] {
  if (group !== "finished" || !folded) return entries;
  return entries.filter((e) => entryId(e) === openId || (e.kind === "conversation" && e.family.children.some((c) => c.id === openId)));
}

// What a report note from a sub-conversation says it did, as the verb that
// follows its title: "Docs check · finished".
export function reportVerb(kind: string): string {
  switch (kind) {
    case "finished":
      return "finished";
    case "replied":
      return "replied";
    case "asks":
      return "asks you";
    case "blocked":
      return "is blocked";
    case "stopped":
      return "stopped";
    case "ended":
      return "ended its session";
  }
  return "reports";
}
