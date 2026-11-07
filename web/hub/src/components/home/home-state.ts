// Pure logic for the Home page (app/page.tsx): the greeting and the date,
// the attention strip (one row per thing that is wrong, with its fix), the
// five waiting items the "Needs you" column shows, the one line under a
// project's name, and the three numbers of "This week". Reads only what the
// page already holds; nothing here fetches. Kept free of `@/` imports so
// `node --test` can load it (home-state.test.mjs).
import type { ComputeRequest, KpiDay, OwnerQuestion, ProjectCardData, Proposal, RecordRow, WorkSuggestion } from "../../lib/api";
import type { GateStep } from "../pulse/evolve";
import type { DraftState } from "../records/draft-panel";

// ----- header ---------------------------------------------------------------

// "Good morning" by the hour, the same cut-offs the old hero used: the small
// hours still count as evening (someone up at 2 am is ending a day, not
// starting one).
export function greetingFor(hour: number): string {
  if (hour < 5) return "Good evening";
  if (hour < 12) return "Good morning";
  if (hour < 18) return "Good afternoon";
  return "Good evening";
}

// "Good morning, Manish" with a name, "Good morning" alone without one.
export function greetingLine(hour: number, name?: string | null): string {
  const n = (name || "").trim();
  return greetingFor(hour) + (n ? ", " + n : "");
}

// "Thursday 6 November": weekday, day and month, no year (the owner knows the year).
export function dateLine(d: Date, locale = "en-GB"): string {
  return d.toLocaleDateString(locale, {
    weekday: "long",
    day: "numeric",
    month: "long",
  });
}

// ----- attention strip -----------------------------------------------------

export type AttentionTone = "failed" | "warning";

// What the strip knows about the runner: the pause and the plain label the
// daemon-control state carries, and whether it has been read at all.
export interface AttentionDaemon {
  label: string;
  pause: "on" | "off" | "unknown";
  canResume: boolean;
  setup: "install" | "start" | "finish" | null;
}

export interface AttentionInbox {
  machine: boolean;
  provider: { name: string; label: string; expired: boolean } | null;
  down: string[];
  downWhy: string[];
}

export type AttentionFix =
  // Resume the paused runner (writes stop = 0); `enabled` false when this console may not.
  | { kind: "resume"; enabled: boolean }
  // Start / install / finish the background service, through the existing service dialog.
  | { kind: "service"; action: "install" | "start" | "finish" }
  // Open a page.
  | { kind: "link"; href: string }
  // Reload the inputs that failed.
  | { kind: "retry" };

export interface AttentionRow {
  id: string;
  tone: AttentionTone;
  // One sentence, no trailing period ("Runner is off").
  text: string;
  // The fix button's words ("Start runner").
  action: string;
  fix: AttentionFix;
  // The technical reason, for a tooltip; "" when there is none.
  why: string;
}

// The rows of the strip, worst first: the runner (nothing works without it),
// then the model provider (no agent runs without one), then inputs the page
// could not load. A runner that is simply being checked is not a problem.
export function attentionRows(
  inbox: AttentionInbox,
  daemon: AttentionDaemon,
  hrefs: {
    settings: (section?: string) => string;
    connect: (provider: string) => string;
  },
): AttentionRow[] {
  const rows: AttentionRow[] = [];
  if (daemon.pause === "on") {
    rows.push({
      id: "runner",
      tone: "failed",
      text: "Runner is paused: nothing new starts",
      action: "Resume",
      fix: { kind: "resume", enabled: daemon.canResume },
      why: daemon.canResume ? "" : "To resume, open PULSE on the machine that runs it.",
    });
  } else if (daemon.label === "silent") {
    rows.push({
      id: "runner",
      tone: "failed",
      text: "Runner is not responding",
      action: daemon.setup ? "Start runner" : "Runner controls",
      fix: daemon.setup ? { kind: "service", action: daemon.setup } : { kind: "link", href: hrefs.settings("daemon") },
      why: "Its last heartbeat is stale.",
    });
  } else if (daemon.label === "stopped" || daemon.label === "no heartbeat") {
    rows.push({
      id: "runner",
      tone: "failed",
      text: "Runner is off",
      action: daemon.setup ? "Start runner" : "Runner controls",
      fix: daemon.setup ? { kind: "service", action: daemon.setup } : { kind: "link", href: hrefs.settings("daemon") },
      why: "",
    });
  } else if (daemon.label === "unknown" || daemon.label === "unavailable") {
    rows.push({
      id: "runner",
      tone: "warning",
      text: "Runner status is unknown",
      action: "Runner controls",
      fix: { kind: "link", href: hrefs.settings("daemon") },
      why: "PULSE can't read the runner's state.",
    });
  } else if (daemon.setup === "finish" || daemon.setup === "install") {
    // The runner works, but stops when the terminal closes: the machine's own step.
    rows.push({
      id: "service",
      tone: "warning",
      text: daemon.setup === "finish" ? "The console isn't a background service yet" : "PULSE stops when you close its terminal",
      action: daemon.setup === "finish" ? "Finish setup" : "Keep PULSE running",
      fix: { kind: "service", action: daemon.setup },
      why: "",
    });
  } else if (inbox.machine) {
    rows.push({
      id: "machine",
      tone: "warning",
      text: "This machine needs a step",
      action: "Runner controls",
      fix: { kind: "link", href: hrefs.settings("daemon") },
      why: "",
    });
  }
  if (inbox.provider) {
    const p = inbox.provider;
    rows.push({
      id: "provider",
      tone: "warning",
      text: p.expired ? p.label + " sign-in expired" : p.label + " is not connected",
      action: p.expired ? "Sign in" : "Connect",
      fix: { kind: "link", href: hrefs.connect(p.name) },
      why: "",
    });
  }
  inbox.down.forEach((d, i) => {
    rows.push({
      id: "down/" + i,
      tone: "warning",
      text: d.charAt(0).toUpperCase() + d.slice(1),
      action: "Retry",
      fix: { kind: "retry" },
      why: inbox.downWhy[i] || "",
    });
  });
  return rows;
}

// ----- first run -----------------------------------------------------------

// What the setup card reads: whether a model provider is signed in (null while
// the harness hasn't answered), how many projects there are, and the runner.
export interface SetupSnapshot {
  // The provider in use and its sign-in; undefined while the harness loads.
  provider: { label: string; loggedIn: boolean; expired: boolean } | null | undefined;
  projects: number;
  daemon: AttentionDaemon;
}

export type SetupStepId = "provider" | "project" | "running";

export interface SetupStep {
  id: SetupStepId;
  title: string;
  // One line under the title: what is in place, or what the step does.
  note: string;
  done: boolean;
  // The button's words when the step is not done ("Connect", "Add project", "Keep running").
  action: string;
}

// Home is a first run until a model provider is connected AND a project is
// registered: with either missing the agents cannot do anything, so the
// three-step setup card replaces "Needs you". An expired sign-in is not a
// first run (the attention strip says so, and what waits on the owner still
// matters); nor is the runner step by itself (a runner in a terminal works;
// the service only keeps it running). Unknown inputs (the harness still
// loading) count as present, so the card does not flash while the page loads.
export function isFirstRun(s: Pick<SetupSnapshot, "provider" | "projects">): boolean {
  if (s.projects === 0) return true;
  return s.provider === null || (!!s.provider && !s.provider.loggedIn);
}

// The runner counts as "kept running" when nothing in the daemon state asks
// for a service step and it is not known to be off or paused.
export function runnerKeptRunning(d: AttentionDaemon): boolean {
  if (d.setup) return false;
  if (d.pause === "on") return false;
  return d.label === "running" || d.label === "alive" || d.label === "checking";
}

// The fix behind the "Keep PULSE running" step: resume a paused runner,
// run the service step the machine needs, else the runner's settings.
export function runningFix(d: AttentionDaemon, settingsHref: (section?: string) => string): AttentionFix {
  if (d.pause === "on") return { kind: "resume", enabled: d.canResume };
  if (d.setup) return { kind: "service", action: d.setup };
  return { kind: "link", href: settingsHref("daemon") };
}

export function setupSteps(s: SetupSnapshot): SetupStep[] {
  const p = s.provider;
  const providerDone = !!p && p.loggedIn && !p.expired;
  const projectDone = s.projects > 0;
  const runningDone = runnerKeptRunning(s.daemon);
  return [
    {
      id: "provider",
      title: "Connect a model provider",
      note: providerDone ? p.label + " is connected" : p && p.expired ? p.label + " sign-in expired" : p ? p.label + " is not signed in yet" : "The agents think with a model you sign in to",
      done: providerDone,
      action: p && p.loggedIn ? "Sign in" : "Connect",
    },
    {
      id: "project",
      title: "Add a project",
      note: projectDone ? (s.projects === 1 ? "1 project" : s.projects.toLocaleString("en-US") + " projects") : "Point PULSE at a repository on this Mac",
      done: projectDone,
      action: "Add project",
    },
    {
      id: "running",
      title: "Keep PULSE running",
      note: runningDone ? "The runner is on" : s.daemon.pause === "on" ? "You paused the runner" : s.daemon.setup === "finish" ? "The console isn't a background service yet" : "Run it in the background so it starts with the machine",
      done: runningDone,
      action: s.daemon.pause === "on" ? "Resume" : s.daemon.setup === "finish" ? "Finish setup" : s.daemon.setup === "start" ? "Start runner" : "Keep running",
    },
  ];
}

// ----- needs you -----------------------------------------------------------

// A draft the agents filed for one project's loop record: constitution or evaluators.
export interface HomeDraft {
  slug: string;
  r: DraftState;
}

// A gate step a project waits at (components/pulse/evolve.tsx GateStep).
export type HomeStep = GateStep;

export interface HomeInboxItems {
  decisions: RecordRow[];
  proposals: Proposal[];
  questions: OwnerQuestion[];
  compute: ComputeRequest[];
  work: WorkSuggestion[];
  drafts: HomeDraft[];
  steps: HomeStep[];
}

export type NeedsYouItem =
  | { kind: "question"; key: string; at: string; q: OwnerQuestion }
  | { kind: "decision"; key: string; at: string; r: RecordRow }
  | { kind: "proposal"; key: string; at: string; p: Proposal }
  | { kind: "compute"; key: string; at: string; c: ComputeRequest }
  | { kind: "work"; key: string; at: string; s: WorkSuggestion }
  | { kind: "draft"; key: string; at: string; d: HomeDraft }
  | { kind: "step"; key: string; at: string; step: HomeStep };

export const NEEDS_YOU_SHOWN = 5;

const ms = (s?: string | null) => (s ? Date.parse(s) || 0 : 0);
const newestFirst = (a: { at: string }, b: { at: string }) => ms(b.at) - ms(a.at);

// The top five across kinds, in the order they should be read: questions and
// decisions first (the agents stand still until they are answered), then
// proposals, machine requests, suggested work, drafts, and gate steps. Within
// a group the newest comes first, except questions, which keep the inbox's
// own order (stop questions ahead of the rest).
export function needsYou(items: HomeInboxItems): NeedsYouItem[] {
  const questions = items.questions.map((q): NeedsYouItem => ({
    kind: "question",
    key: "q/" + q.id,
    at: q.date,
    q,
  }));
  const decisions = items.decisions
    .map((r): NeedsYouItem => ({
      kind: "decision",
      key: "d/" + r.id,
      at: r.updated_at || r.date,
      r,
    }))
    .sort(newestFirst);
  const proposals = items.proposals
    .map((p): NeedsYouItem => ({
      kind: "proposal",
      key: "p/" + p.id,
      at: p.date,
      p,
    }))
    .sort(newestFirst);
  const compute = items.compute
    .map((c): NeedsYouItem => ({
      kind: "compute",
      key: "c/" + c.id,
      at: c.at,
      c,
    }))
    .sort(newestFirst);
  const work = items.work
    .map((s): NeedsYouItem => ({
      kind: "work",
      key: "w/" + s.id,
      at: s.updated_at || s.created_at,
      s,
    }))
    .sort(newestFirst);
  const drafts = items.drafts
    .map((d): NeedsYouItem => ({
      kind: "draft",
      key: "f/" + d.slug + "/" + d.r.id,
      at: d.r.draft?.at || "",
      d,
    }))
    .sort(newestFirst);
  const steps = items.steps.map((step): NeedsYouItem => ({
    kind: "step",
    key: "s/" + step.slug + "/" + step.kind,
    at: "",
    step,
  }));
  return [...questions, ...decisions, ...proposals, ...compute, ...work, ...drafts, ...steps];
}

export function topFive(items: HomeInboxItems): NeedsYouItem[] {
  return needsYou(items).slice(0, NEEDS_YOU_SHOWN);
}

// What a waiting item is, in the words under its title on Home ("Decision",
// "Machine request"). A stop question is self-improvement waiting to carry
// on; the row adds who asked a question when the post names an agent.
export function needsYouWord(item: NeedsYouItem): string {
  switch (item.kind) {
    case "question":
      return item.q.stop ? "Self-improvement paused" : "Question";
    case "decision":
      return "Decision";
    case "proposal":
      return "Proposal";
    case "compute":
      return "Machine request";
    case "work":
      return "Suggested work";
    case "draft":
      return "Draft to review";
    case "step":
      return "Setup step";
  }
}

// "See all 10" when more wait than the column shows; "See all" otherwise.
export function seeAllLabel(total: number): string {
  return total > NEEDS_YOU_SHOWN ? "See all " + total.toLocaleString("en-US") : "See all";
}

// The project an item belongs to; `brain` stands in for proposals and
// questions that name none (they come from the source checkout's board).
export function itemProject(item: NeedsYouItem, brain: string): string {
  switch (item.kind) {
    case "question":
      return item.q.slug || brain;
    case "decision":
      return item.r.project;
    case "proposal":
      return item.p.slug || brain;
    case "compute":
      return item.c.project;
    case "work":
      return item.s.project;
    case "draft":
      return item.d.slug;
    case "step":
      return item.step.slug;
  }
}

// ----- working now ---------------------------------------------------------

export const WORKING_SHOWN = 6;

// ----- projects ------------------------------------------------------------

export interface ProjectLine {
  // The words under the name.
  text: string;
  // Marigold when something waits on the owner; "bad" when the project is broken; else quiet.
  tone: "you" | "bad" | "neutral";
  // The project still runs on the goal PULSE wrote for it: the row shows a
  // "Set goal" link to the project's goal (what the old banner did).
  setGoal?: boolean;
}

// One line for a project row: what waits on the owner when anything does,
// else what is wrong, else when its board last moved. `rel` turns a date
// into "3 hours ago"; the page passes lib/format's.
export function projectLine(p: Pick<ProjectCardData, "present" | "missing" | "error" | "stop_present" | "board_present" | "last_post" | "goal_source">, waiting: number, rel: (d: string) => string): ProjectLine {
  if (waiting > 0)
    return {
      text: waiting === 1 ? "1 thing waits for you" : waiting.toLocaleString("en-US") + " things wait for you",
      tone: "you",
    };
  if (p.missing || !p.present) return { text: "Missing from disk", tone: "bad" };
  if (p.error) return { text: p.error, tone: "bad" };
  if (p.stop_present) return { text: "Paused by a STOP file", tone: "bad" };
  if (!p.board_present) return { text: "No board yet", tone: "bad" };
  if (p.goal_source === "generated") return { text: "Runs on a generated goal", tone: "neutral", setGoal: true };
  if (p.last_post?.date) return { text: "Active " + rel(p.last_post.date), tone: "neutral" };
  return { text: "Quiet so far", tone: "neutral" };
}

// ----- this week -----------------------------------------------------------

export interface WeekNumbers {
  landings: number;
  tasksDone: number;
  decisions: number;
}

// The last seven days of the KPI series summed: changes landed, tasks done,
// and the times the fleet asked the owner (the decisions made). The API
// zero-fills, so the slice is the window.
export function weekNumbers(series: readonly Pick<KpiDay, "landings" | "tasks_done" | "owner_asks">[] | null | undefined, days = 7): WeekNumbers | null {
  if (!series || !series.length) return null;
  const w = series.slice(Math.max(0, series.length - days));
  const sum = (f: (d: (typeof w)[number]) => number) => w.reduce((n, d) => n + (Number(f(d)) || 0), 0);
  return {
    landings: sum((d) => d.landings),
    tasksDone: sum((d) => d.tasks_done),
    decisions: sum((d) => d.owner_asks),
  };
}
