// Where work stands in the lifecycle of docs/sdlc.md (TASK-141), as the
// console says it: the task page's Lifecycle tab (its phases in order, what
// each left behind, the gate to the next) and the Tasks page's Phases view
// (a project's tasks by phase). The hub derives the phase
// (GET tasks/{id}/lifecycle, GET tasks/lifecycle?project=P); this module
// only turns it into words and links.
// Pure: type imports only, so node --test runs it without a bundler.
import type {
  LifecycleMark,
  LifecyclePhase,
  LifecyclePhaseInfo,
  LifecycleStep,
  LifecycleStepState,
  LifecycleWaitsOn,
  ProjectLifecycle,
  TaskLifecycle,
} from "./api";

// The phases a task can be in, in the order a change passes them, then
// where tasks end. Direction stands above them all; release, operate and
// learn are a project's, not one task's (docs/sdlc.md).
export const TASK_PHASES: LifecyclePhase[] = [
  "intake",
  "shape",
  "build",
  "verify",
  "review",
  "deliver",
];
export const END_PHASES: LifecyclePhase[] = ["done", "cancelled"];

// docs/sdlc.md's headings; the hub sends the same words as `label`, these
// stand in for a hub that sends none.
export const PHASE_LABEL: Record<LifecyclePhase, string> = {
  intake: "Intake",
  shape: "Shape and plan",
  build: "Build",
  verify: "Verify",
  review: "Review",
  deliver: "Deliver",
  done: "Done",
  cancelled: "Cancelled",
};

export function phaseLabel(p: string, info?: LifecyclePhaseInfo[]): string {
  const sent = info?.find((i) => i.phase === p)?.label;
  return sent || PHASE_LABEL[p as LifecyclePhase] || p;
}

// Who the task waits on now, as a short clause; "" when nobody.
export const WAITS_ON_WORDS: Record<LifecycleWaitsOn, string> = {
  nobody: "",
  owner: "waits on you",
  lead: "waits on the lead",
  agent: "an agent is at work",
  reviewer: "waits on the reviewer",
  runner: "waits on the runner",
  github: "waits on GitHub",
};

export function waitsOnWord(w: string): string {
  return WAITS_ON_WORDS[w as LifecycleWaitsOn] ?? "";
}

export const STEP_STATE_WORD: Record<LifecycleStepState, string> = {
  passed: "Passed",
  current: "Now",
  ahead: "Ahead",
  skipped: "Skipped",
};

// The one line under a task's title: "Build · an agent is at work".
export function phaseLine(l: Pick<TaskLifecycle, "phase" | "waits_on">, info?: LifecyclePhaseInfo[]): string {
  const who = waitsOnWord(l.waits_on);
  return phaseLabel(l.phase, info) + (who ? " · " + who : "");
}

// Where a mark's link goes: a task's page, a run, a conversation, or the
// pull request on its host. The component turns it into an href (lib/nav).
export type MarkLink =
  | { to: "task"; id: string }
  | { to: "run"; id: string }
  | { to: "chat"; id: string }
  | { to: "url"; href: string };

export interface MarkView {
  key: string;
  kind: string;
  text: string; // what the chip reads
  note: string; // a word after it: the job's status, the verdict's note
  title: string; // the whole id, for the tooltip
  link: MarkLink | null;
  mono: boolean; // an id, set in monospace
}

// A job id's last part: job-20261126T134608Z-6d882d reads "6d882d".
export function shortJob(id: string): string {
  const m = /^job-\d{8}T\d{6}Z-([0-9a-f]+)$/.exec(id);
  return m ? m[1] : id;
}

const VERDICT_WORD: Record<string, string> = {
  pr_open: "open",
  in_review: "in review",
  needs_owner: "handed to you",
  merge_asked: "merge asked",
  merge_waiting: "merge waits on its checks",
  changes_requested: "changes asked",
  review_failed: "review failed",
  merged: "merged",
  held: "held",
  closed: "closed unmerged",
};

const JOB_STATUS_WORD: Record<string, string> = {
  queued: "queued",
  leased: "at work",
  running: "at work",
  blocked: "blocked",
  done: "done",
  failed: "failed",
  cancelled: "cancelled",
};

export function markView(m: LifecycleMark, i = 0): MarkView {
  const base = { key: m.kind + ":" + m.id + ":" + i, kind: m.kind, note: "", title: m.id, link: null as MarkLink | null, mono: true };
  switch (m.kind) {
    case "task":
      return { ...base, text: m.id, link: { to: "task", id: m.id } };
    case "origin": {
      const [from, ...rest] = m.id.split(":");
      const id = rest.join(":");
      const word = from === "chat" ? "a conversation" : from === "job" ? "job " + shortJob(id) : from === "suggestion" ? id || "a suggestion" : m.id;
      return { ...base, text: "from " + word, mono: false, link: from === "chat" && id ? { to: "chat", id } : null };
    }
    case "lead":
    case "job": {
      const words = m.note ? (JOB_STATUS_WORD[m.note] ?? m.note) : "";
      return {
        ...base,
        text: (m.kind === "lead" ? "lead " : "job ") + shortJob(m.id),
        note: words,
        link: m.session ? { to: "run", id: m.session } : null,
      };
    }
    case "branch":
      return { ...base, text: m.id, note: m.note ? "from job " + shortJob(m.note) : "" };
    case "pr":
      return { ...base, text: "PR #" + m.id, title: "Pull request #" + m.id, link: m.url ? { to: "url", href: m.url } : null };
    case "verdict":
      return { ...base, text: VERDICT_WORD[m.id] ?? m.id.replaceAll("_", " "), note: m.note ?? "", mono: false };
    case "merge":
      return { ...base, text: "merged #" + m.id, title: "Pull request #" + m.id + " merged", link: m.url ? { to: "url", href: m.url } : null };
    default:
      return { ...base, text: m.id, note: m.note ?? "" };
  }
}

export interface StepView {
  phase: LifecyclePhase;
  label: string;
  state: LifecycleStepState;
  stateWord: string;
  at: string;
  gate: string;
  marks: MarkView[];
  // What the phase says when it left nothing: "Nothing yet." ahead of the
  // task, "Passed over." when skipped, "" otherwise.
  empty: string;
}

export function stepViews(l: TaskLifecycle): StepView[] {
  return l.steps.map((s: LifecycleStep) => ({
    phase: s.phase,
    label: phaseLabel(s.phase, l.phases),
    state: s.state,
    stateWord: STEP_STATE_WORD[s.state] ?? s.state,
    at: s.at ?? "",
    gate: s.gate,
    marks: s.left.map(markView),
    empty: s.left.length ? "" : s.state === "skipped" ? "Passed over." : s.state === "ahead" ? "Nothing yet." : "Nothing recorded.",
  }));
}

// "Passed 3 of 6 phases" for the tab's count line.
export function progressLine(l: TaskLifecycle): string {
  const total = l.steps.length;
  const passed = l.steps.filter((s) => s.state === "passed").length;
  if (l.phase === "done") return "Done: every phase passed";
  if (l.phase === "cancelled") return "Cancelled after " + passed + " of " + total + " phases";
  return "Passed " + passed + " of " + total + " phases";
}

// ----- a project's tasks by phase -------------------------------------------

export interface PhaseTask<T> {
  task: T;
  waits: string; // waitsOnWord
  note: string;
  rework: boolean;
}

export interface PhaseColumn<T> {
  phase: LifecyclePhase;
  label: string;
  gate: string;
  tasks: PhaseTask<T>[];
}

// The columns of the Phases view: the task's path in order, then Done, and
// Cancelled only when it holds a task, so the board reads left to right the
// way work moves. `tasks` is the page's list after its search and filters:
// a task the hub placed but the page filtered out is left out, and one the
// page shows but the hub has not placed yet (filed after the read) waits
// in Intake.
export function phaseColumns<T extends { id: string }>(p: ProjectLifecycle | null | undefined, tasks: T[]): PhaseColumn<T>[] {
  const byId = new Map(tasks.map((t) => [t.id, t]));
  const rows = new Map((p?.tasks ?? []).map((r) => [r.task, r]));
  const info = p?.phases;
  const placed = new Set<string>();
  const columns: PhaseColumn<T>[] = [...TASK_PHASES, ...END_PHASES].map((phase) => {
    const ids = info?.find((x) => x.phase === phase)?.tasks ?? (p?.tasks ?? []).filter((r) => r.phase === phase).map((r) => r.task);
    const list: PhaseTask<T>[] = [];
    for (const id of ids) {
      const task = byId.get(id);
      if (!task || placed.has(id)) continue;
      placed.add(id);
      const r = rows.get(id);
      list.push({ task, waits: waitsOnWord(r?.waits_on ?? ""), note: r?.note ?? "", rework: !!r?.rework });
    }
    return { phase, label: phaseLabel(phase, info), gate: info?.find((x) => x.phase === phase)?.gate ?? "", tasks: list };
  });
  if (p) {
    const intake = columns[0];
    for (const t of tasks) if (!placed.has(t.id)) intake.tasks.push({ task: t, waits: "", note: "not placed yet", rework: false });
  }
  return columns.filter((c) => c.phase !== "cancelled" || c.tasks.length > 0);
}

// One sentence over the Phases view: where the open work sits.
export function phasesSummary(columns: PhaseColumn<unknown>[]): string {
  const open = columns.filter((c) => TASK_PHASES.includes(c.phase));
  const n = open.reduce((a, c) => a + c.tasks.length, 0);
  if (!n) return "No open task. Everything here is done or cancelled.";
  const owner = open.reduce((a, c) => a + c.tasks.filter((t) => t.waits === WAITS_ON_WORDS.owner).length, 0);
  const busiest = open.slice().sort((a, b) => b.tasks.length - a.tasks.length)[0];
  const parts = [n + (n === 1 ? " open task" : " open tasks") + ", most in " + busiest.label];
  if (owner) parts.push(owner + (owner === 1 ? " waits on you" : " wait on you"));
  return parts.join("; ") + ".";
}
