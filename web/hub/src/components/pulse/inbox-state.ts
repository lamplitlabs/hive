import { daemonControlState, type DaemonControlState } from "./daemon-control-state";
import { foldSteps, gateStep, type GateStep } from "./evolve-gate";
import { awaitsDecision } from "../records/records-state";
import { draftState, type DraftState } from "../records/draft-state";
import { evolveGate, registeredProjects, type Compute, type ComputeRequest, type EvolveGate, type HarnessStatus, type Org, type OwnerQuestion, type ProjectCardData, type ProjectsPayload, type Proposal, type RecordRow, type RecordsPayload, type Runner, type ServicePayload, type SettingsPayload, type WorkSuggestion, type WorkSuggestionsPayload } from "../../lib/api";
import { andList, plural } from "../../lib/format";
import { sortOwnerQuestions } from "../../lib/owner-questions";
import type { Loaded } from "../../lib/use-api";

// The For-you count's pure logic: no React, no DOM, relative imports only, so
// node --test runs it as it is (inbox-state.test.mjs). inbox.ts is the hook
// layer that fetches the sources and re-exports every name here.

// The one "for you" count. Everything that says how many things wait for the
// owner - the home hero, the sidebar tile, a project's hero, its overview tile
// and its For-you tab - reads it from here, so no two places can disagree.
// An item is "for you" when nothing happens until the owner says yes or no: a
// proposed decision record, a proposal awaiting the owner, a question
// addressed to the owner (the loop's stop questions included), a pending
// compute request, a drafted constitution or evaluators, a suggested work
// item the agents could not settle, the one gate step a project waits at,
// and - for the whole machine - a paused or unset daemon, or a model
// provider that isn't connected (no agent can run without one).

export interface InboxItems {
  decisions: RecordRow[];
  proposals: Proposal[];
  questions: OwnerQuestion[];
  // Open questions the agents settle themselves (the lead while the owner is
  // away, or a setting): not counted in `total`, only named beside For you.
  agentsDeciding: number;
  compute: ComputeRequest[];
  work: WorkSuggestion[];
  drafts: { slug: string; r: DraftState }[];
  steps: GateStep[];
  machine: boolean;
  // The model provider in use when it isn't connected (never signed in, or
  // the sign-in expired); null when it is, or while the hub hasn't said.
  provider: { name: string; label: string; expired: boolean } | null;
  total: number;
}

export interface Inbox extends InboxItems {
  // True once the org payload arrived - before that the count means nothing.
  ready: boolean;
  // Inputs still loading / failed, as the short phrases the inbox shows.
  checking: string[];
  down: string[];
  downWhy: string[];
  daemon: DaemonControlState;
  gateOf: (p: ProjectCardData) => EvolveGate | null;
  forProject: (slug: string) => InboxItems;
}

export interface InboxSources {
  org: Loaded<Org>;
  proposed: Loaded<RecordsPayload>;
  reg: Loaded<ProjectsPayload>;
  compute: Loaded<Compute>;
  // Optional: an older hub has no tasks/suggested route.
  suggested?: Loaded<WorkSuggestionsPayload>;
  runner: Loaded<Runner>;
  service: Loaded<ServicePayload>;
  settings: Loaded<SettingsPayload>;
  // Optional: the provider's sign-in, for the "Connect a model provider" step.
  harness?: Loaded<HarnessStatus>;
}

// Where each source comes from: the hub API path useInboxSources (inbox.ts)
// reads it at and how often it re-reads it (ms; none means useApi's default).
// Kept here, beside the count, so node --test sees which route feeds which
// kind; the hook layer only calls useApi with these. A path shared with
// another screen shares useApi's cache, so it is written exactly as there.
export interface InboxSourcePath {
  path: string;
  refresh?: number;
}

export const INBOX_SOURCE_PATHS: Readonly<Record<keyof InboxSources, InboxSourcePath>> = {
  org: { path: "org" },
  proposed: { path: "records?kind=adr&status=Proposed", refresh: 60000 },
  reg: { path: "projects", refresh: 60000 },
  compute: { path: "compute" },
  suggested: { path: "tasks/suggested?status=owner", refresh: 60000 },
  runner: { path: "runner" },
  service: { path: "service", refresh: 60000 },
  settings: { path: "settings" },
  harness: { path: "harness", refresh: 60000 },
};

const sum = (items: Omit<InboxItems, "total">) => items.decisions.length + items.proposals.length + items.questions.length + items.compute.length + items.work.length + items.drafts.length + items.steps.length + (items.machine ? 1 : 0) + (items.provider ? 1 : 0);

export function buildInbox(q: InboxSources): Inbox {
  const o = q.org.data;
  const projects = registeredProjects(o);
  const regRows = q.reg.data?.projects ?? [];
  const brain = o?.totals.brain ?? "";
  const gateOf = (p: ProjectCardData) => evolveGate(p, regRows.find((r) => r.slug === p.slug));
  const decisions = (q.proposed.data?.records ?? []).filter(awaitsDecision);
  const proposals = (o?.proposals ?? []).filter((p) => p.status === "awaiting");
  const allQuestions = sortOwnerQuestions(o?.questions ?? []);
  const questions = allQuestions.filter((x) => (x.decider ?? "owner") === "owner");
  const agentsDeciding = allQuestions.length - questions.length;
  const compute = (q.compute.data?.requests ?? []).filter((r) => r.status === "pending");
  const work = (q.suggested?.data?.suggestions ?? []).filter((s) => s.status === "owner");
  const drafts = projects.flatMap((p) => {
    const g = gateOf(p);
    return [draftState(g?.constitution), draftState(g?.evaluators)].filter((r): r is DraftState => !!r?.draft).map((r) => ({ slug: p.slug, r }));
  });
  // One held-out step can serve every project; the machine-wide count folds it, a project's own count keeps its step.
  const ownSteps = projects.map((p) => gateStep(p.slug, gateOf(p))).filter((s): s is GateStep => s !== null);
  const steps = foldSteps(ownSteps);
  const daemon = daemonControlState({ runner: q.runner, service: q.service, settings: q.settings });
  const machine = daemon.pause === "on" || !!daemon.setup;
  const hp = q.harness?.data?.provider;
  const expired = !!hp?.login?.logged_in && !!hp.login.expires_at && new Date(hp.login.expires_at).getTime() < Date.now();
  const provider = hp && (!hp.login?.logged_in || expired) ? { name: hp.name, label: hp.label, expired } : null;
  const all = { decisions, proposals, questions, agentsDeciding, compute, work, drafts, steps, machine, provider };
  const decisionsDown = !!q.proposed.error && !q.proposed.data;
  const computeDown = !!q.compute.error && !q.compute.data;
  return {
    ...all,
    total: sum(all),
    ready: !!o,
    checking: [!q.proposed.data && !decisionsDown ? "decisions" : "", !q.compute.data && !computeDown ? "compute requests" : ""].filter(Boolean),
    down: [decisionsDown ? "decisions could not be loaded" : "", computeDown ? "compute requests could not be loaded" : ""].filter(Boolean),
    downWhy: [decisionsDown ? "decisions could not be loaded (" + q.proposed.error + ")" : "", computeDown ? "compute requests could not be loaded (" + q.compute.error + ")" : ""].filter(Boolean),
    daemon,
    gateOf,
    forProject: (slug) => {
      const mine = {
        decisions: decisions.filter((r) => r.project === slug),
        proposals: proposals.filter((p) => (p.slug || brain) === slug),
        questions: questions.filter((x) => (x.slug || brain) === slug),
        agentsDeciding: allQuestions.filter((x) => ((x.decider ?? "owner") !== "owner") && (x.slug || brain) === slug).length,
        compute: compute.filter((r) => r.project === slug),
        work: work.filter((s) => s.project === slug),
        drafts: drafts.filter((d) => d.slug === slug),
        steps: ownSteps.filter((s) => s.slug === slug),
        machine: false,
        provider: null,
      };
      return { ...mine, total: sum(mine) };
    },
  };
}

// The breakdown as one short sentence: "1 proposal and 2 questions", or the fallback when nothing waits.
export function inboxBreakdown(items: InboxItems, empty = "nothing waits"): string {
  const parts = [
    items.decisions.length ? plural(items.decisions.length, "decision") : "",
    items.proposals.length ? plural(items.proposals.length, "proposal") : "",
    items.questions.length ? plural(items.questions.length, "question") : "",
    items.compute.length ? plural(items.compute.length, "request") : "",
    items.work.length ? plural(items.work.length, "suggestion") : "",
    items.drafts.length ? plural(items.drafts.length, "draft") : "",
    items.steps.length + (items.machine ? 1 : 0) + (items.provider ? 1 : 0) ? plural(items.steps.length + (items.machine ? 1 : 0) + (items.provider ? 1 : 0), "step") : "",
  ].filter(Boolean);
  return parts.length ? andList(parts) : empty;
}
