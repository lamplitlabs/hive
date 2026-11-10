// The For you page's model (spec "Navigation changes": one page lists
// everything that waits on you across projects). Pure: it turns the inbox's
// typed lists into rows grouped by kind, filtered by project and ordered by
// urgency then recency, so node --test can check it without React.

import type { Kind } from "../hive/kind-tile-state";

export type ForYouSection =
  | "decisions"
  | "proposals"
  | "questions"
  | "compute"
  | "work"
  | "drafts"
  | "steps"
  | "problems";

export const SECTION_ORDER: ForYouSection[] = ["problems", "questions", "decisions", "proposals", "compute", "work", "drafts", "steps"];

export const SECTION_TITLE: Record<ForYouSection, string> = {
  decisions: "Decisions",
  proposals: "Proposals",
  questions: "Questions",
  compute: "Compute requests",
  work: "Suggested work",
  drafts: "Drafts",
  steps: "Gate steps",
  problems: "Machine and provider",
};

export const SECTION_NOTE: Record<ForYouSection, string> = {
  decisions: "Records an agent proposed; accept, defer or decline each.",
  proposals: "Changes an agent wants to make and needs your yes or no on.",
  questions: "Agents asked you something and stopped until you answer.",
  compute: "Agents asked for more machines; each costs money while it runs.",
  work: "Ideas the agents could not settle themselves.",
  drafts: "Text an agent drafted from the repository, to adopt or discard.",
  steps: "One step a project's self-improvement loop waits on.",
  problems: "Agents can't work until these are fixed.",
};

export const SECTION_KIND: Record<ForYouSection, Kind> = {
  decisions: "decision",
  proposals: "proposal",
  questions: "question",
  compute: "machine",
  work: "suggestion",
  drafts: "record",
  steps: "rule",
  problems: "machine",
};

// One row on the page, whichever kind it came from. `urgent` lifts it to
// the top of its section (a stop question, an expired sign-in); `at` is
// the item's own date (ISO or "" when unknown) for the order within.
export interface ForYouRow {
  key: string;
  section: ForYouSection;
  kind: Kind;
  project: string;
  title: string;
  meta?: string;
  href?: string;
  at: string;
  urgent?: boolean;
  /** When the row answers to yes / no keys. */
  yesNo?: boolean;
  /** The button word on a row whose fix is a page away (the row links there). */
  fix?: string;
}

export interface ForYouInput {
  decisions: { id: string; project: string; title: string; kind: string; updated_at?: string; date?: string }[];
  proposals: { id: string; slug?: string; summary: string; date: string; thread_key?: string }[];
  questions: { id: string; slug?: string; summary: string; date: string; stop?: boolean; thread_key?: string; on_post?: string }[];
  compute: { id: string; project: string; size: string; hours: number; requested_by: string; at: string }[];
  work: { id: string; project: string; title: string; updated_at?: string; created_at?: string; reviewed_at?: string }[];
  drafts: { slug: string; r: { id: string; draft?: { author?: string; at?: string } | null } }[];
  steps: { kind: string; slug: string; slugs?: string[]; label: string; title: string }[];
  machine: boolean;
  machineWhy?: string;
  provider: { name: string; label: string; expired: boolean } | null;
  /** The brain project, where proposals and questions without a slug belong. */
  brain?: string;
}

const first = (...v: (string | undefined | null)[]) => v.find((x) => !!x) || "";

// A self-improvement stop reads the same wherever it waits (Home, the
// project, here): the question it asks, not the gate's log line.
export const STOP_TITLE = "Self-improvement stopped. May it carry on?";
const isStop = (q: { stop?: boolean; summary: string }) => q.stop === true || /Evolution stopped; owner review needed/i.test(q.summary || "");

export function forYouRows(i: ForYouInput): ForYouRow[] {
  const brain = i.brain || "";
  const rows: ForYouRow[] = [];
  if (i.machine) rows.push({ key: "machine", section: "problems", kind: "machine", project: "", title: "The runner isn't working", meta: i.machineWhy || "Agents can't start work on this machine", href: "/#daemon-controls", at: "", urgent: true, fix: "Fix it" });
  if (i.provider) rows.push({ key: "provider:" + i.provider.name, section: "problems", kind: "tool", project: "", title: i.provider.expired ? (i.provider.label || i.provider.name) + " sign-in expired" : "Connect " + (i.provider.label || i.provider.name), meta: i.provider.expired ? "Sign in again so agents can use their model" : "Agents have no model provider yet", href: "/settings/?connect=" + encodeURIComponent(i.provider.name) + "#harness", at: "", urgent: true, fix: i.provider.expired ? "Sign in" : "Connect" });
  for (const q of i.questions) rows.push({ key: "question:" + q.id, section: "questions", kind: "question", project: q.slug || brain, title: isStop(q) ? STOP_TITLE : q.summary, at: q.date, urgent: !!q.stop, yesNo: true });
  for (const r of i.decisions) rows.push({ key: "decision:" + r.id, section: "decisions", kind: "decision", project: r.project, title: r.title, meta: r.id, at: first(r.updated_at, r.date), yesNo: true });
  for (const p of i.proposals) rows.push({ key: "proposal:" + p.id, section: "proposals", kind: "proposal", project: p.slug || brain, title: p.summary, at: p.date, yesNo: true });
  for (const c of i.compute) rows.push({ key: "compute:" + c.id, section: "compute", kind: "machine", project: c.project, title: c.size + " for " + c.hours + (c.hours === 1 ? " hour" : " hours"), meta: "Asked by " + c.requested_by, at: c.at, yesNo: true });
  for (const w of i.work) rows.push({ key: "work:" + w.id, section: "work", kind: "suggestion", project: w.project, title: w.title, meta: w.id, at: first(w.reviewed_at, w.updated_at, w.created_at), yesNo: true });
  for (const d of i.drafts) rows.push({ key: "draft:" + d.slug + ":" + d.r.id, section: "drafts", kind: "record", project: d.slug, title: d.r.id === "EVO-evaluators" ? "New evaluators drafted" : "A new constitution drafted", meta: d.r.draft?.author ? "By " + d.r.draft.author : undefined, at: d.r.draft?.at || "" });
  for (const s of i.steps) rows.push({ key: "step:" + s.kind + ":" + (s.slugs?.join(",") || s.slug), section: "steps", kind: "rule", project: s.slugs && s.slugs.length > 1 ? "" : s.slug, title: s.title, meta: s.label, at: "", fix: "Open" });
  return rows;
}

// Projects that have at least one row, most waiting first, for the filter chips.
export function forYouProjects(rows: ForYouRow[]): { slug: string; n: number }[] {
  const n = new Map<string, number>();
  for (const r of rows) if (r.project) n.set(r.project, (n.get(r.project) || 0) + 1);
  return [...n.entries()].map(([slug, count]) => ({ slug, n: count })).sort((a, b) => b.n - a.n || a.slug.localeCompare(b.slug));
}

// The rows one project filter keeps: machine-wide problems and shared steps
// (no project) stay whatever the filter.
export function filterRows(rows: ForYouRow[], project: string | null | undefined): ForYouRow[] {
  if (!project) return rows;
  return rows.filter((r) => !r.project || r.project === project);
}

// Within a section: urgent first, then newest first, then by key so the
// order is stable when dates tie or are missing.
export function orderRows(rows: ForYouRow[]): ForYouRow[] {
  return [...rows].sort((a, b) => Number(!!b.urgent) - Number(!!a.urgent) || (a.at < b.at ? 1 : a.at > b.at ? -1 : 0) || a.key.localeCompare(b.key));
}

export function groupRows(rows: ForYouRow[]): { section: ForYouSection; rows: ForYouRow[] }[] {
  return SECTION_ORDER.map((section) => ({ section, rows: orderRows(rows.filter((r) => r.section === section)) })).filter((g) => g.rows.length > 0);
}

// The bell's five: the most pressing across sections - problems, then stop
// questions, then everything else newest first.
export function latestRows(rows: ForYouRow[], n = 5): ForYouRow[] {
  const rank = (r: ForYouRow) => (r.section === "problems" ? 0 : r.urgent ? 1 : 2);
  return [...rows].sort((a, b) => rank(a) - rank(b) || (a.at < b.at ? 1 : a.at > b.at ? -1 : 0) || a.key.localeCompare(b.key)).slice(0, n);
}

// "Nothing waits on you" / "1 thing waits on you" / "12 things wait on you".
export function waitingWords(n: number): string {
  if (n === 0) return "Nothing waits on you";
  return n === 1 ? "1 thing waits on you" : n.toLocaleString("en-US") + " things wait on you";
}
