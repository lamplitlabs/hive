// The words and the order of the Chat page's one list. Every entry is a
// conversation: a plain one, one with sub-conversations under it, or a task
// - the owner's conversation with the lead agent, with one agent session per
// step the lead planned under it (`tasks` in GET /api/v1/chat). Kept apart
// from the page so the rules can be tested without a browser.

import type { ChatTask, ChatTaskSession } from "@/lib/api";
import type { PipMood } from "@/components/pulse/mascots";

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

// The words of work that is done but not on the default branch yet: a
// finished step whose branch the daemon has still to land, a conversation
// whose branch holds commits main lacks. The same mood for a step, a task
// and a conversation, so the section reads as one thing.
export const WAITING_TO_LAND: Mood = { word: "Waiting to land", tone: "wait", pip: "waiting", dot: "bg-warn" };

// The step after waiting: a landing has taken the branch and is merging and
// running the project's checks on it (its pulse-land.lock names the branch).
// "Working" is an agent at it, "Waiting to land" is the branch asking to
// land, "Reviewing" is the landing checking it.
export const REVIEWING: Mood = { word: "Reviewing", tone: "run", pip: "busy", dot: "bg-primary" };

// A step waits to land once its job is done and its branch is still on the
// job: the landing clears the branch when the commits reach the default
// branch (the runner's pending landings are exactly these jobs).
export function jobWaitingToLand(jobStatus: string, branch: string | undefined): boolean {
  return jobStatus === "done" && !!branch;
}

// What a task's mood is read from, the same for the list's row (GET /chat)
// and the task's own page (GET /tasks/{id}): its status, whether it has a
// lead, whether the lead waits on the owner, whether an agent is at a step,
// whether a finished step still waits to land.
export interface TaskState {
  status: string;
  lead: string;
  needs_you: boolean;
  working: boolean;
  landing?: boolean;
  reviewing?: boolean;
}

function taskClosed(status: string): boolean {
  return status === "done" || status === "cancelled";
}

// A task's state in the list's words, on the same scale as a conversation's:
// the owner's turn first, then what the agents are doing. A step waiting to
// land outranks "Done" - the task is not on main yet - but not an agent
// still at another step.
export function taskMoodOf(t: TaskState): Mood {
  if (t.needs_you) return { word: "Asked you", tone: "wait", pip: "waiting", dot: "bg-warn" };
  if (t.reviewing) return REVIEWING;
  if (taskClosed(t.status) && t.landing) return WAITING_TO_LAND;
  if (t.status === "done") return { word: "Done", tone: "neutral", pip: "happy", dot: "bg-ok/50" };
  if (t.status === "cancelled") return { word: "Cancelled", tone: "neutral", pip: "asleep", dot: "bg-muted-foreground/40" };
  if (t.working) return { word: "Working", tone: "run", pip: "busy", dot: "bg-primary" };
  if (t.landing) return WAITING_TO_LAND;
  if (t.status === "open") return { word: t.lead ? "Lead agent starting" : "New", tone: "ok", pip: "curious", dot: "bg-ok" };
  if (t.status === "planned") return { word: "Planned", tone: "ok", pip: "curious", dot: "bg-ok" };
  return { word: "In progress", tone: "ok", pip: "happy", dot: "bg-ok" };
}

export function taskStateOf(t: ChatTask): TaskState {
  return {
    status: t.status,
    lead: t.lead,
    needs_you: t.needs_you,
    working: t.sessions.some((s) => s.status === "running" || s.job_status === "leased"),
    landing: t.sessions.some((s) => jobWaitingToLand(s.job_status, s.branch)),
    reviewing: t.sessions.some((s) => !!s.reviewing),
  };
}

export function taskMood(t: ChatTask): Mood {
  return taskMoodOf(taskStateOf(t));
}

export function taskFinished(t: ChatTask): boolean {
  return taskClosed(t.status);
}

// A task's section, on the same scale as a conversation's: working while
// the lead waits on you or an agent is at a step, waiting to land while a
// finished step's branch is not on main yet, open while it is new or
// planned, finished once done or cancelled (a task is never archived).
export function taskGroupOf(t: TaskState): ListGroup {
  if (t.needs_you || (!taskClosed(t.status) && t.working)) return "working";
  if (t.landing || t.reviewing) return "landing";
  if (taskClosed(t.status)) return "finished";
  return "open";
}

export function taskGroup(t: ChatTask): ListGroup {
  return taskGroupOf(taskStateOf(t));
}

// A step's state: the job's word, except while an agent's session runs it;
// a done step with its branch still on it waits to land.
export function stepWords(jobStatus: string, running: boolean, branch?: string, reviewing?: boolean): { word: string; tone: Tone } {
  if (running) return { word: "Working", tone: "run" };
  if (reviewing) return { word: REVIEWING.word, tone: REVIEWING.tone };
  if (jobWaitingToLand(jobStatus, branch)) return { word: WAITING_TO_LAND.word, tone: WAITING_TO_LAND.tone };
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

export function sessionWords(s: ChatTaskSession): { word: string; tone: Tone } {
  return stepWords(s.job_status, s.status === "running", s.branch, s.reviewing);
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

// The list's sections: a conversation is working while its agent runs or
// waits on you, waiting to land while its branch holds commits the default
// branch lacks and nobody is at it, open while it is idle between turns,
// finished once the agent or the owner closed it (a message reopens it),
// and archived when it was put away - put away is the owner's word, so an
// archived conversation stays archived whatever its branch holds.
export type ListGroup = "working" | "landing" | "open" | "finished" | "archived";

export const GROUP_ORDER: ListGroup[] = ["working", "landing", "open", "finished", "archived"];

export interface Grouped {
  id: string;
  status: string;
  parent?: string;
  created_at?: string;
  live?: { status: string } | null;
  // The hub's answer (GET /chat): commits on the branch that main lacks.
  waiting_to_land?: boolean;
  // A landing is checking the branch now (GET /chat).
  reviewing?: boolean;
}

export function groupOf(c: Grouped): ListGroup {
  const status = c.live?.status || c.status;
  if (status === "running" || status === "waiting") return "working";
  if (status === "archived") return "archived";
  if (c.waiting_to_land || c.reviewing) return "landing";
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
