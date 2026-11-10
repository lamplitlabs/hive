// The For you tab's pure logic: which questions wait for the owner and which
// the agents settle themselves, and the words the agents-decided audit uses.
// No React and no "@/" imports, so node --test covers it
// (tabs-state.test.mjs) in a clone without node_modules too, the way hive
// evolve measure scores the hub's coverage fixtures. tabs.tsx renders it.

import type { OwnerQuestion } from "../../lib/api";
import { plural } from "../../lib/format";

// The audit's toggle line (followed by " · N").
export const AGENTS_DECIDED_SUMMARY = "Answered by the agents";

// The hint under the closed toggle.
export const AGENTS_DECIDED_HINT = "Settled by the lead or a setting; never counted as waiting for you.";

// Who answers a question: the owner unless the row says otherwise (an older
// hub row carries no decider and is the owner's).
export function deciderOf(q: Pick<OwnerQuestion, "decider">): "owner" | "lead" | "setting" {
  return q.decider ?? "owner";
}

// Split a project's questions into what still waits for the owner and what
// the agents settle - the lead while the owner is away, or a setting. Only the
// first half counts toward "Waiting for you"; the second is the audit. Both
// keep the order they came in.
export function splitQuestions<Q extends Pick<OwnerQuestion, "decider">>(all: readonly Q[]): { questions: Q[]; agentsDecided: Q[] } {
  const questions: Q[] = [];
  const agentsDecided: Q[] = [];
  for (const q of all) (deciderOf(q) === "owner" ? questions : agentsDecided).push(q);
  return { questions, agentsDecided };
}

// "the lead" or "a setting", for "Settled by ...". Anything that is not a
// setting was settled by the lead: the owner's own questions never reach the
// audit.
export function whoSettled(decider: OwnerQuestion["decider"]): string {
  return decider === "setting" ? "a setting" : "the lead";
}

// The audit row's accessible name.
export function settledLabel(q: Pick<OwnerQuestion, "decider" | "summary">): string {
  return "Settled by " + whoSettled(q.decider) + ": " + q.summary;
}

// The note beside "Waiting for you": a hub that cannot decide says where to
// answer; otherwise how many questions the agents are settling, or nothing.
export function waitingNote(canDecide: boolean, agentsDecided: number): string | undefined {
  if (!canDecide) return "To answer, open Hive on the machine that runs it.";
  return agentsDecided ? plural(agentsDecided, "question") + " the agents are deciding themselves." : undefined;
}
