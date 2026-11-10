// The self-improvement gate's pure logic: which one human step a project
// waits at (the inbox's gate card, the project hero), folded once per machine
// for the shared private tests. No React, no DOM, relative imports only, so
// node --test reaches it through the inbox's tests (inbox-state.test.mjs);
// evolve.tsx re-exports every name for its callers.
import type { EvolveGate, EvolveLevel } from "../../lib/api";
import { andList } from "../../lib/format";

// The two records self-improvement reads, by what they are for.
export const RECORD_WORD = { constitution: "Constitution", evaluators: "Scoring rules" } as const;

// A next step that asks nothing of the owner: "none", or "none - <what the
// daemon does instead>". The part after the dash, when there is one.
export const noneStep = (next: string): { none: boolean; why: string } => {
  const m = /^none\b\s*(?:[-–:]\s*)?([^]*)$/.exec(next.trim());
  return m ? { none: true, why: (m[1] ?? "").trim() } : { none: false, why: "" };
};

// The steps only the owner can take. The baseline is not one of them since
// ADR-031: the daemon measures it, so no gate card ever asks for it.
export type GateKind = "seed" | "constitution" | "records" | "heldout";

export interface GateStep {
  kind: GateKind;
  label: string; // the short "what is missing" phrase ("Next step: Set up private tests")
  slug: string;
  slugs?: string[]; // every project one shared step serves, when the inbox folds them (the held-out set is one per machine)
  level: EvolveLevel;
  title: string;
  body: string;
  command: string; // one command, ready to paste
  alt?: string; // the other way to the same step, when the hub names one ("or hive record new ...")
}

// The hub writes the seed step as `seed-cmd  (template copy), or write-cmd`:
// two commands joined by prose. Split it so each copies on its own.
export function splitAlternative(next: string): { command: string; alt: string } {
  const [command, alt = ""] = next.trim().split(/\s+\(template copy\),?\s+or\s+/, 2);
  return { command: command.trim(), alt: alt.trim() };
}

const GATE_LABEL: Record<GateKind, string> = { seed: "add test cases", constitution: "write a constitution", records: "add the self-improvement rules", heldout: "set up private tests" };

// Which of the loop's two records a project lacks: what the row says first
// (null is "none"), the title the seed command names when the row is silent.
function missingRecords(g: EvolveGate, next: string): ("constitution" | "evaluators")[] {
  const fromRow = ([g.constitution === null ? "constitution" : "", g.evaluators === null ? "evaluators" : ""] as const).filter((x): x is "constitution" | "evaluators" => !!x);
  return fromRow.length ? fromRow : [/--title Evaluators\b/.test(next) ? "evaluators" : "constitution"];
}

const HELDOUT_WHY = "Private tests are two test cases agents never see, so self-improvement can't tune itself to pass them.";

// Which human step, if any, a project waits for. One card per project: the
// hub's `evolve_next` names the step. A project the record seed did not reach
// (ADR-030) gets the seeding step; an older hub still names `hive record new
// evolution`. The baseline gate has no card (ADR-031): with a baseline job
// open, a failure the daemon retries, or a step that starts with `none`, the
// row asks nothing - and an older hub's `hive job add ... experiment` is the
// daemon's job now, so it asks nothing either. The held-out fixtures
// (ADR-012) are the one step left that is the owner's, and the card says so.
export function gateStep(slug: string, g: EvolveGate | null): GateStep | null {
  if (!g) return null;
  const next = g.evolve_next.trim();
  if (!next || noneStep(next).none || (!g.baseline && (g.baseline_job || g.baseline_failed_at)) || next.startsWith("hive job add")) return null;
  const base = { slug, level: g.evolve_level };
  if (next.startsWith("hive evolve seed")) {
    const missing = missingRecords(g, next);
    const one = missing.length === 1 ? missing[0] : undefined;
    const { command, alt } = splitAlternative(next);
    return {
      ...base,
      kind: "records",
      label: one ? "add " + (one === "constitution" ? "a constitution" : "scoring rules") : GATE_LABEL.records,
      title: one ? "Add " + (one === "constitution" ? "a constitution" : "scoring rules") + " for " + slug : "Add the self-improvement rules for " + slug,
      body: "Self-improvement follows two short records: a constitution (what agents may change) and scoring rules (how a change is judged). This project has " + (one ? "no " + RECORD_WORD[one].toLowerCase() : "neither") + " yet. Copy the command and run it once in a terminal on this machine: it starts from the template, and you can make the text your own afterwards.",
      command,
      ...(alt ? { alt } : {}),
    };
  }
  if (next.startsWith("hive record new evolution"))
    return {
      ...base,
      kind: "constitution",
      label: GATE_LABEL.constitution,
      title: "Write a constitution for " + slug,
      body: "The constitution holds the rules self-improvement follows. Without one, agents only measure and never try a change. Write it in your own words, then add it with the command.",
      command: next,
    };
  if (next.startsWith("hive evolve fixture seed"))
    return {
      ...base,
      kind: "seed",
      label: GATE_LABEL.seed,
      title: "Add starter test cases for " + slug,
      body: "There are no test cases yet, so nothing can be measured. The starter set adds six small ones to begin with; you can add your own later.",
      command: next,
    };
  if (next.startsWith("hive evolve setup"))
    return {
      ...base,
      kind: "heldout",
      label: GATE_LABEL.heldout,
      title: "Set up two private tests",
      body: HELDOUT_WHY + " You write them once, Hive keeps them outside every project, and the runner does everything else. Self-improvement for " + slug + " starts once they're in.",
      command: next,
    };
  return null;
}

// The held-out set is one per machine (evolve.heldout_dir), so when several
// projects wait for it the inbox shows one card that names them all, not one
// card per project saying the same thing. Every other step stays per project.
export function foldSteps(steps: GateStep[]): GateStep[] {
  const shared = steps.filter((s) => s.kind === "heldout");
  if (shared.length < 2) return steps;
  const slugs = shared.map((s) => s.slug);
  const one: GateStep = {
    ...shared[0],
    slugs,
    body: HELDOUT_WHY + " You write them once and the runner does everything else. One set on this machine serves " + andList(slugs) + "; self-improvement starts once they're in.",
  };
  const rest = steps.filter((s) => s.kind !== "heldout");
  const at = steps.indexOf(shared[0]);
  return [...rest.slice(0, at), one, ...rest.slice(at)];
}
