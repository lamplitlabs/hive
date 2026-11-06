// The kinds of object the console shows, each with one tint everywhere
// (spec "Icons and logos"). Pure: no React, no icons - the icon lookup lives
// in kind-tile.tsx so this module can be tested under node --test.

export type Kind =
  | "session"
  | "task"
  | "decision"
  | "proposal"
  | "question"
  | "suggestion"
  | "rule"
  | "run"
  | "machine"
  | "tool"
  | "skill"
  | "automation"
  | "memory"
  | "plan"
  | "guide"
  | "feature"
  | "work"
  | "project"
  | "record"
  | "vote";

export type Tint = "teal" | "blue" | "marigold" | "violet" | "slate" | "green" | "pink" | "grey";

export const KINDS: Kind[] = [
  "session",
  "task",
  "decision",
  "proposal",
  "question",
  "suggestion",
  "rule",
  "run",
  "machine",
  "tool",
  "skill",
  "automation",
  "memory",
  "plan",
  "guide",
  "feature",
  "work",
  "project",
  "record",
  "vote",
];

// One tint per kind. A decision is marigold while it waits on the owner and
// grey otherwise; `tintFor` applies that rule.
export const KIND_TINT: Record<Kind, Tint> = {
  session: "teal",
  task: "blue",
  decision: "grey",
  proposal: "violet",
  question: "marigold",
  suggestion: "violet",
  rule: "violet",
  run: "teal",
  machine: "slate",
  tool: "slate",
  skill: "green",
  automation: "blue",
  memory: "pink",
  plan: "grey",
  guide: "grey",
  feature: "grey",
  work: "grey",
  project: "slate",
  record: "grey",
  vote: "violet",
};

export const KIND_LABEL: Record<Kind, string> = {
  session: "Session",
  task: "Task",
  decision: "Decision",
  proposal: "Proposal",
  question: "Question",
  suggestion: "Suggestion",
  rule: "Rule change",
  run: "Run",
  machine: "Machine",
  tool: "Tool",
  skill: "Skill",
  automation: "Automation",
  memory: "Memory",
  plan: "Plan",
  guide: "Guide",
  feature: "Feature",
  work: "Work item",
  project: "Project",
  record: "Record",
  vote: "Vote",
};

export function tintFor(kind: Kind, waiting = false): Tint {
  if (kind === "decision") return waiting ? "marigold" : "grey";
  return KIND_TINT[kind];
}

// Tailwind utilities for a tint: the 10% wash and the tint's ink. Spelled out
// in full so the Tailwind scanner sees every class.
export const TINT_CLASS: Record<Tint, string> = {
  teal: "bg-kind-teal-soft text-kind-teal",
  blue: "bg-kind-blue-soft text-kind-blue",
  marigold: "bg-kind-marigold-soft text-kind-marigold",
  violet: "bg-kind-violet-soft text-kind-violet",
  slate: "bg-kind-slate-soft text-kind-slate",
  green: "bg-kind-green-soft text-kind-green",
  pink: "bg-kind-pink-soft text-kind-pink",
  grey: "bg-kind-grey-soft text-kind-grey",
};

// The store's own names (pulse record kinds, board post kinds, hub
// objects) mapped onto the console's kinds. Unknown names fall back to a
// grey record.
const RECORD_TO_KIND: Record<string, Kind> = {
  // pulse record kinds (api.ts RecordKind)
  adr: "decision",
  plan: "plan",
  wu: "work",
  feature: "feature",
  memory: "memory",
  workflow: "automation",
  skill: "skill",
  guide: "guide",
  context: "record",
  architecture: "record",
  evolution: "record",
  instruction: "guide",
  template: "record",
  usage: "guide",
  prompt: "record",
  env: "record",
  constitution: "rule",
  // board post kinds and owner items
  proposal: "proposal",
  question: "question",
  suggestion: "suggestion",
  rule: "rule",
  amendment: "rule",
  vote: "vote",
  decision: "decision",
  // hub objects
  session: "session",
  conversation: "session",
  chat: "session",
  thread: "session",
  task: "task",
  job: "run",
  run: "run",
  machine: "machine",
  runner: "machine",
  tool: "tool",
  connection: "tool",
  mcp: "tool",
  automation: "automation",
  schedule: "automation",
  project: "project",
  record: "record",
  work: "work",
};

export function kindFor(record_kind: string | null | undefined): Kind {
  const key = (record_kind || "").trim().toLowerCase().replace(/[\s-]+/g, "_");
  return RECORD_TO_KIND[key] ?? "record";
}

export function isKind(s: string): s is Kind {
  return (KINDS as string[]).includes(s);
}
