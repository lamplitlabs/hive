import type { AskOption, GoalProposal, GoalSource, ProjectCardData, Proposal } from "../../lib/api";
import { agentName } from "../../lib/words";

// A proposed project goal waiting for the owner (TASK-117): a lead's
// reading or Hive's own draft posts it on the board as a `goal/amend`
// opinion; the hub puts the newest undecided one on the project's org row
// (`goal_proposal`). A goal change is a hard stop: only the owner takes it
// (the lead's decide refuses a `goal:` marker), so it waits on the Sessions
// strip with its two answers, the recommended one marked and why, and on
// the project's Overview as "Hive proposes this goal". The owner accepts it
// (the decide route sets the goal), edits it first, or keeps the current
// goal. Pure: no
// React, no DOM, relative imports only, so goal-proposal-state.test.mjs
// runs it under node --test.

export const GOAL_PROPOSAL_TITLE = "Hive proposes this goal";
// The Overview panel's id, so the strip and a notification can link straight to it.
export const GOAL_PROPOSAL_ANCHOR = "goal-proposal";
// The board thread the proposals live on (hive-goals review.rs AREA/AMEND_THREAD).
export const GOAL_PROPOSAL_THREAD = "goal/amend";
export const MAX_GOAL_CHARS = 4000;
// Why the owner alone decides it, as the hub words a hard stop (`hard_stop`):
// the Sessions strip lists it under the goal hard stop.
export const GOAL_HARD_STOP = "changes the project goal";

export const GOAL_PROPOSAL_LABELS = {
  use: "Accept the drafted goal",
  edit: "Edit then use",
  dismiss: "Keep the current goal",
} as const;

// A proposal row of the inbox that carries a proposed goal.
export type GoalInboxProposal = Proposal & { goal: GoalProposal };

export function isGoalProposal(p: Proposal | null | undefined): p is GoalInboxProposal {
  return !!p && !!p.goal && typeof p.goal.text === "string" && p.goal.text.trim() !== "";
}

// Who put it forward, in the owner's words: the project's lead by its role
// name, Hive itself for the drafting side-call (no agent, "hive", the
// runner, or a system name like "hive:goal-draft"), anyone else by name.
export function goalProposer(agent: string | null | undefined, slug: string): string {
  const a = (agent || "").trim();
  if (!a || a === "hive" || a === "daemon" || a.startsWith("hive:")) return "Hive";
  return agentName(a, slug);
}

// "Drafted by Hive" / "Proposed by Project lead".
export function goalProposerLine(agent: string | null | undefined, slug: string): string {
  const who = goalProposer(agent, slug);
  return who === "Hive" ? "Drafted by Hive from the project's evidence" : "Proposed by " + who;
}

// The goal proposals on the org rows as inbox proposals, so every count and
// list that reads the inbox (the Sessions strip, the bell, the project) shows
// them with no second source. A post the org's own proposal list already
// carries keeps that row and gains the goal; projects missing on disk or
// that sent no proposal add nothing.
export function goalInboxProposals(
  projects: readonly Pick<ProjectCardData, "slug" | "base" | "goal_proposal">[],
  existing: readonly Proposal[] = [],
): GoalInboxProposal[] {
  const out: GoalInboxProposal[] = [];
  const seen = new Set(existing.map((p) => p.id));
  for (const row of projects) {
    const g = row.goal_proposal;
    if (!g || !g.post || !(g.text || "").trim() || seen.has(g.post)) continue;
    seen.add(g.post);
    out.push({
      id: g.post,
      thread_key: GOAL_PROPOSAL_THREAD,
      date: g.date,
      status: "awaiting",
      decided_id: "",
      decided_date: "",
      summary: GOAL_PROPOSAL_TITLE,
      decided_summary: "",
      proposal_text: g.text,
      can_decide: true,
      hard_stop: GOAL_HARD_STOP,
      slug: row.slug,
      base: row.base,
      kind: "goal",
      agent: g.agent,
      goal: g,
    });
  }
  return out;
}

// The proposals list with the goal proposals folded in: an org proposal for
// the same post gains the goal (so it renders as the goal card), the rest are
// added after.
export function withGoalProposals(
  proposals: readonly Proposal[],
  projects: readonly Pick<ProjectCardData, "slug" | "base" | "goal_proposal">[],
): Proposal[] {
  const byPost = new Map<string, GoalProposal>();
  for (const row of projects) if (row.goal_proposal?.post) byPost.set(row.goal_proposal.post, row.goal_proposal);
  const merged = proposals.map((p) => {
    const g = byPost.get(p.id);
    return g && (g.text || "").trim() ? { ...p, summary: GOAL_PROPOSAL_TITLE, proposal_text: g.text, kind: "goal", hard_stop: GOAL_HARD_STOP, goal: g } : p;
  });
  return [...merged, ...goalInboxProposals(projects, proposals)];
}

// What the card says. `current` is the goal in force (null while it loads),
// so the card can say what taking the proposal replaces.
export interface GoalProposalView {
  title: string;
  by: string;
  text: string;
  why: string;
  // The goal in force, when it is the same text: nothing would change.
  same: boolean;
  replaces: string;
  dismissNote: string;
}

export function goalProposalView(g: GoalProposal, slug: string, current?: { text: string; source: GoalSource } | null): GoalProposalView {
  const text = (g.text || "").trim();
  const why = (g.why || "").trim();
  const same = !!current && squash(current.text) === squash(text);
  const replaces = same
    ? "It matches the goal in force; using it records your yes."
    : current?.source === "owner"
      ? "Using it replaces the goal you set."
      : "Using it replaces the goal Hive wrote for you.";
  return {
    title: GOAL_PROPOSAL_TITLE,
    by: goalProposerLine(g.agent, slug),
    text,
    why: why || "No reason was given. Open the post to see what it rests on.",
    same,
    replaces,
    dismissNote: "Keeping the current goal dismisses this draft; the same draft is not proposed again.",
  };
}

// The owner's two answers, one recommended and why (TASK-129: a hard stop is
// asked with answers to pick and one recommended). Accepting is recommended:
// the daemon posts a draft only when it differs materially from the goal in
// force and from every draft dismissed before, so the draft's own Why is the
// reason. A draft that says what the goal in force already says recommends
// keeping it: nothing would change.
export interface GoalAnswers {
  options: AskOption[];
  why: string;
}

export function goalAnswers(g: GoalProposal, current?: { text: string; source: GoalSource } | null): GoalAnswers {
  const same = !!current && squash(current.text) === squash(g.text);
  const ownWords = current?.source === "owner";
  const accept: AskOption = {
    label: GOAL_PROPOSAL_LABELS.use,
    effect: "the drafted goal replaces " + (ownWords ? "the goal you set" : "the goal Hive wrote for you") + "; every agent on this project works toward it from its next run",
    answer: "yes",
    recommended: !same,
  };
  const keep: AskOption = {
    label: GOAL_PROPOSAL_LABELS.dismiss,
    effect: "the goal in force stays; this draft is dismissed and not proposed again",
    answer: "no",
    recommended: same,
  };
  const why = same ? "the draft says what the goal in force already says, so nothing would change" : firstSentence(g.why) || "it is drafted from what the project's work shows, and it differs from the goal in force";
  return { options: [accept, keep], why };
}

function firstSentence(t: string | null | undefined): string {
  const s = squash(t || "");
  if (!s) return "";
  const m = /^(.+?[.!?])(\s|$)/.exec(s);
  const one = (m ? m[1] : s).replace(/[.!?]+$/, "");
  return one.charAt(0).toLowerCase() + one.slice(1);
}

const squash = (s: string) => (s || "").replace(/\s+/g, " ").trim();

// "Edit then use": when the owner changed the words, the edited text is saved
// as the owner's goal first, then the yes on the proposal records the
// decision (the hub sees the goal is newer than the draft and keeps the
// owner's words). An edit the server refuses writes nothing else. Unchanged
// words need only the yes. Too long or empty text is refused before any write.
export type EditPlan =
  | { ok: true; text: string; edited: boolean; words: string }
  | { ok: false; error: string };

export function editThenUse(proposed: string, edited: string, max = MAX_GOAL_CHARS): EditPlan {
  const text = (edited || "").replace(/\r\n/g, "\n").trim();
  const n = Array.from(text).length;
  if (!n) return { ok: false, error: "Write a goal with 1 to " + max.toLocaleString("en-US") + " characters." };
  if (n > max) return { ok: false, error: "The goal is " + n.toLocaleString("en-US") + " characters; keep it to " + max.toLocaleString("en-US") + "." };
  const changed = squash(text) !== squash(proposed);
  return { ok: true, text, edited: changed, words: changed ? "Used with my edits." : "Used as proposed." };
}

// The writes "Edit then use" makes, in order (the console passes `writes`).
export type EditWrites = {
  setProjectGoal: (slug: string, text: string) => Promise<unknown>;
  decide: (id: string, answer: "yes" | "no", words: string) => Promise<unknown>;
};

// What came of "Edit then use": `goalSet` says whether the goal in force
// changed, `error` (with its title) what went wrong.
export type EditOutcome = { ok: true } | { ok: false; goalSet: boolean; title: string; error: unknown };

export async function applyEdit(plan: Extract<EditPlan, { ok: true }>, slug: string, post: string, w: EditWrites): Promise<EditOutcome> {
  if (plan.edited) {
    try {
      await w.setProjectGoal(slug, plan.text);
    } catch (error) {
      return { ok: false, goalSet: false, title: "Your goal wasn't saved; the proposal still waits", error };
    }
  }
  try {
    await w.decide(post, "yes", plan.words);
  } catch (error) {
    return plan.edited
      ? { ok: false, goalSet: true, title: "Your goal is set, but the proposal wasn't marked as used", error }
      : { ok: false, goalSet: false, title: "The goal wasn't set", error };
  }
  return { ok: true };
}

// The Overview shows the goal proposal as its own panel; its "Needs you"
// list leaves it out so the page does not ask twice.
export function withoutGoalProposals<T extends { proposals: Proposal[]; total: number }>(items: T): T {
  const kept = items.proposals.filter((p) => !isGoalProposal(p));
  return { ...items, proposals: kept, total: items.total - (items.proposals.length - kept.length) };
}

// The project's waiting goal proposal from the org rows, or null.
export function projectGoalProposal(projects: readonly Pick<ProjectCardData, "slug" | "goal_proposal">[] | undefined, slug: string): GoalProposal | null {
  const g = projects?.find((p) => p.slug === slug)?.goal_proposal;
  return g && g.post && (g.text || "").trim() ? g : null;
}

// A project's own proposals list with the inbox's goal proposals folded in
// (the project route does not know the goal draft): a row for the same post
// gains the goal, the rest are added after.
export function withInboxGoals(proposals: readonly Proposal[], inbox: readonly Proposal[]): Proposal[] {
  const goals = inbox.filter(isGoalProposal);
  const byId = new Map(goals.map((g) => [g.id, g]));
  const merged = proposals.map((p) => byId.get(p.id) ?? p);
  const have = new Set(proposals.map((p) => p.id));
  return [...merged, ...goals.filter((g) => !have.has(g.id))];
}
