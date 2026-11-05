// Pure logic for the home page's "At a glance" strip (glance.tsx): the four
// tiles under the greeting - what waits for the owner, who is working, how
// the runner is and how many projects there are - each a short label, a
// figure and one line under it, with the tone that colours its dot. Reads
// only what the page already holds. Kept free of `@/` imports so
// `node --test` can load it (glance-state.test.mjs).
import type { Org, Runner } from "../../lib/api";

export type GlanceTone = "neutral" | "ok" | "warn" | "bad" | "info" | "accent" | "you";

export interface GlanceTile {
  id: "waiting" | "working" | "runner" | "projects";
  label: string;
  // The figure: a count, or a short phrase when a count says nothing.
  value: string;
  sub: string;
  tone: GlanceTone;
}

export interface GlanceInput {
  // The For you total, and whether its sources are still loading.
  inbox: number;
  inboxChecking: boolean;
  // Open questions the agents settle themselves (named, not counted).
  agentsDeciding: number;
  // Agents at work right now (the Working now panel's rows: an agent on a task
  // step or holding a board claim), and how many things they hold in all.
  workingAgents: number;
  things: number;
  // The runner's counts when it answered; null while it loads or when it is down.
  jobs: Pick<Runner["jobs"], "running" | "queued"> | null;
  runnerChecking: boolean;
  runnerDown: boolean;
  // The runner's headline ("Runner on", "Runner paused", ...) and tone, from daemon-control-state.
  runnerLabel: string;
  runnerTone: GlanceTone;
  // A pending self-update, as one short line, or "".
  runnerNote: string;
  projects: Pick<Org["projects"][number], "slug" | "missing" | "error" | "stop_present" | "board_present">[];
  agentsActive: number;
}

function plural(n: number, one: string, many = one + "s"): string {
  return n.toLocaleString("en-US") + " " + (n === 1 ? one : many);
}

export function glanceTiles(i: GlanceInput): GlanceTile[] {
  const agentsN = i.workingAgents ?? 0;
  const things = i.things ?? 0;
  const running = typeof i.jobs?.running === "number" ? i.jobs.running : 0;
  const queued = typeof i.jobs?.queued === "number" ? i.jobs.queued : 0;
  const busy = agentsN > 0 || running > 0;
  const troubled = (i.projects ?? []).filter((p) => p.missing || p.error || p.stop_present || p.board_present === false).length;
  const projectsN = (i.projects ?? []).length;

  const waiting: GlanceTile = {
    id: "waiting",
    label: "Waiting for you",
    value: i.inbox > 0 ? String(i.inbox) : i.inboxChecking ? "…" : "0",
    sub: i.inbox > 0 ? (i.agentsDeciding > 0 ? "Plus " + plural(i.agentsDeciding, "question") + " the agents settle" : "Each one waits until you decide") : i.inboxChecking ? "Checking what's waiting" : "All caught up",
    tone: i.inbox > 0 ? "you" : "neutral",
  };

  const working: GlanceTile = {
    id: "working",
    label: "Working now",
    value: i.runnerChecking && !agentsN ? "…" : busy ? plural(Math.max(agentsN, running ? 1 : 0), "agent") : "Nobody",
    sub: busy
      ? [agentsN ? plural(things, "thing") + " picked up" : running ? plural(running, "job") + " running" : "", queued ? plural(queued, "job") + " queued" : ""].filter(Boolean).join(" · ") || "Agents are busy"
      : i.runnerChecking
        ? "Checking agents"
        : queued
          ? plural(queued, "job") + " queued"
          : "Give a task when you're ready",
    tone: busy ? "info" : "neutral",
  };

  const runner: GlanceTile = {
    id: "runner",
    label: "This machine",
    value: i.runnerDown ? "Runner unreachable" : i.runnerLabel,
    sub: i.runnerDown ? "The page updates once it answers again" : i.runnerNote || (i.runnerChecking ? "Checking the runner" : i.runnerTone === "ok" ? "Agents can work" : i.runnerTone === "neutral" ? "Open the controls" : "Open the controls to fix it"),
    tone: i.runnerDown ? "bad" : i.runnerTone,
  };

  const projects: GlanceTile = {
    id: "projects",
    label: "Projects",
    value: String(projectsN),
    sub: projectsN === 0 ? "Add your first one" : troubled ? plural(troubled, "project") + " need" + (troubled === 1 ? "s" : "") + " attention" : i.agentsActive ? plural(i.agentsActive, "agent") + " active across them" : "All in order",
    tone: troubled ? "warn" : "neutral",
  };

  return [waiting, working, runner, projects];
}

// The one-line summary a screen reader or a test can check: the tiles as "label: value".
export function glanceLine(tiles: GlanceTile[]): string {
  return tiles.map((t) => t.label + ": " + t.value).join(" · ");
}
