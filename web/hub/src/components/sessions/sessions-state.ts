// Pure logic for what Sessions (app/chat) shows besides the conversations,
// since Home went (TASK-129): the strip that says something is broken (one
// row per thing that is wrong, with its fix), the card that sets Hive up on a
// new machine, and how many agents at work the panel lists. Reads only what
// the page already holds; nothing here fetches. Kept free of `@/` imports so
// `node --test` can load it (sessions-state.test.mjs).

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
      why: daemon.canResume ? "" : "To resume, open Hive on the machine that runs it.",
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
      why: "Hive can't read the runner's state.",
    });
  } else if (daemon.setup === "finish" || daemon.setup === "install") {
    // The runner works, but stops when the terminal closes: the machine's own step.
    rows.push({
      id: "service",
      tone: "warning",
      text: daemon.setup === "finish" ? "The console isn't a background service yet" : "Hive stops when you close its terminal",
      action: daemon.setup === "finish" ? "Finish setup" : "Keep Hive running",
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

// The fix behind the "Keep Hive running" step: resume a paused runner,
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
      note: projectDone ? (s.projects === 1 ? "1 project" : s.projects.toLocaleString("en-US") + " projects") : "Point Hive at a repository on this Mac",
      done: projectDone,
      action: "Add project",
    },
    {
      id: "running",
      title: "Keep Hive running",
      note: runningDone ? "The runner is on" : s.daemon.pause === "on" ? "You paused the runner" : s.daemon.setup === "finish" ? "The console isn't a background service yet" : "Run it in the background so it starts with the machine",
      done: runningDone,
      action: s.daemon.pause === "on" ? "Resume" : s.daemon.setup === "finish" ? "Finish setup" : s.daemon.setup === "start" ? "Start runner" : "Keep running",
    },
  ];
}

// ----- working now ---------------------------------------------------------

export const WORKING_SHOWN = 6;
