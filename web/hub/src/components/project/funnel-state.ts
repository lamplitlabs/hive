// The self-improvement funnel's pure logic: the stages a suggestion passes on
// its way to a landed change, and the one-line words the panel uses for the
// counts the hub sends (GET projects/<slug>/evolution `funnel`). No React and
// no "@/" imports, so node --test covers it (funnel-state.test.mjs) in a clone
// without node_modules too. tabs.tsx renders it.

import type { EvolutionFunnel, EvolutionFunnelHeld } from "../../lib/api";
import type { Tone } from "../../lib/format";

// One tile of the funnel strip.
export interface FunnelStage {
  key: string;
  label: string;
  n: number;
  sub?: string;
}

// The counts the stages read, so a partial payload (a test, an older hub)
// still renders.
type Counts = Pick<EvolutionFunnel, "posted" | "checked" | "needs_votes" | "eligible" | "plannable" | "in_flight" | "blocked" | "built" | "tried"> & { held?: readonly EvolutionFunnelHeld[] | null };

// The strip's stages in funnel order, suggestion to change. "Gave up" trails
// only when something was tried and abandoned; the strip otherwise ends on
// what was built. A stage's `sub` says what stops part of it from moving on
// (votes still open, a gate, a blocked job) and is absent when nothing does.
export function funnelStages(f: Counts): FunnelStage[] {
  const heldN = (f.held ?? []).reduce((a, h) => a + h.count, 0);
  const sub = (n: number, word: string) => (n > 0 ? n + " " + word : undefined);
  const stages: FunnelStage[] = [
    { key: "posted", label: "Suggested", n: f.posted, sub: sub(f.needs_votes, f.needs_votes === 1 ? "needs votes" : "need votes") },
    { key: "checked", label: "With a check", n: f.checked },
    { key: "eligible", label: "Agreed", n: f.eligible, sub: sub(heldN, "held") },
    { key: "plannable", label: "Ready to plan", n: f.plannable },
    { key: "in_flight", label: "In progress", n: f.in_flight, sub: sub(f.blocked, "blocked") },
    { key: "built", label: "Built", n: f.built },
  ];
  if (f.tried > 0) stages.push({ key: "tried", label: "Gave up", n: f.tried });
  return stages;
}

// "a, b, c" of the parts that are not zero, in the order given.
function parts(items: [number, string][]): string {
  return items
    .filter(([n]) => n > 0)
    .map(([n, word]) => n + " " + word)
    .join(", ");
}

// The panel header's one sentence: "7 suggested, 3 agreed, 1 ready, 1 in
// progress, 2 built". Zero parts are skipped; nothing posted says so.
export function funnelLine(f: Pick<EvolutionFunnel, "posted" | "eligible" | "plannable" | "in_flight" | "built">): string {
  if (!f.posted) return "No suggestions yet";
  return parts([
    [f.posted, "suggested"],
    [f.eligible, "agreed"],
    [f.plannable, "ready"],
    [f.in_flight, "in progress"],
    [f.built, "built"],
  ]);
}

// "2 names no check, 1 net votes" - what keeps the agreed suggestions from
// the planner, biggest group first as the hub sends them. Empty when nothing
// is held.
export function heldGroups(held: readonly EvolutionFunnelHeld[] | null | undefined): string {
  return (held ?? [])
    .filter((h) => h.count > 0)
    .map((h) => h.count + " " + h.gate)
    .join(", ");
}

// The same with its lead-in, "held: 2 names no check, 1 net votes", for a
// sentence; empty when nothing is held.
export function heldLine(held: readonly EvolutionFunnelHeld[] | null | undefined): string {
  const groups = heldGroups(held);
  return groups ? "held: " + groups : "";
}

// The tone of a stall alert: something stuck is a warning, an empty or a
// settled funnel is only information. An unknown code is treated as stuck.
export function alertTone(code: string): Tone {
  switch (code) {
    case "empty":
    case "settled":
      return "info";
    case "held":
    case "votes":
    case "blocked":
    default:
      return "warn";
  }
}

// The closed tail: "2 built, 1 tried, 1 turned down, 3 expired". Zero parts
// are skipped; empty when nothing has settled yet.
export function settledLine(f: Pick<EvolutionFunnel, "built" | "tried" | "rejected" | "expired">): string {
  return parts([
    [f.built, "built"],
    [f.tried, "tried"],
    [f.rejected, "turned down"],
    [f.expired, "expired"],
  ]);
}

// The rule line: what a suggestion needs before it becomes work, as the two
// evolve.* settings say.
export function ruleLine(f: Pick<EvolutionFunnel, "require_check" | "check_skips_quorum">): string {
  const base = f.require_check ? "A suggestion must name a check that fails on main and passes with its change" : "Suggestions without a check become work too";
  return base + (f.check_skips_quorum ? "; a check skips the vote count" : "");
}
