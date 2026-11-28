"use client";

import { AttentionStrip, retryInputs } from "@/components/sessions/attention-strip";
import { SetupCard } from "@/components/sessions/setup-card";
import { attentionRows, isFirstRun, type AttentionDaemon, type SetupSnapshot } from "@/components/sessions/sessions-state";
import { HiveMascot } from "@/components/hive/mascots";
import { useInbox } from "@/components/hive/inbox";
import { registeredProjects, type HarnessStatus, type Org, type ServicePayload, type TasksPayload } from "@/lib/api";
import { hrefs } from "@/lib/nav";
import { useApi } from "@/lib/use-api";

// What Sessions shows besides the conversations, now that the console opens
// on it (TASK-129: Home and For you went). Over the new-session box, only
// when there is something: the broken-strip (the runner is not running, no
// model provider, an input that did not load) and, on a new machine, the
// card that sets Hive up. Each is absent when nothing is wrong or nothing is
// left to set up.
export function SessionsAlerts() {
  const { attention, service, setup, firstRun, harness, daemon } = useSessionsState();
  return (
    <>
      <AttentionStrip rows={attention} service={service} onRetry={retryInputs} />
      {firstRun ? <SetupCard snapshot={setup} harness={harness.data} service={service} daemon={daemon} /> : null}
    </>
  );
}

// The landing's title: Pip and one friendly line. A new machine is welcomed
// to Hive and pointed at the setup steps; an owner coming back is greeted by
// name. The page's one h1 (the top bar's crumb still says Sessions).
export function SessionsGreeting() {
  const { org, firstRun } = useSessionsState();
  const name = (org.data?.owner_name || "").trim();
  const title = !org.data ? "Sessions" : firstRun ? "Welcome to Hive" : name ? "Welcome back, " + name : "Welcome back";
  const sub = firstRun
    ? "Finish the steps below, then tell an agent what you need."
    : "Tell an agent what you need. It works on its own branch and reports back here.";
  return (
    <header className="flex min-w-0 items-center gap-4 pt-6 pb-1 sm:pt-8">
      <HiveMascot mood="happy" size={56} decorative className="float shrink-0" />
      <div className="min-w-0">
        <h1 className="truncate text-2xl font-semibold leading-8 tracking-tight sm:text-[28px] sm:leading-9">{title}</h1>
        <p className="mt-0.5 text-sm leading-5 text-muted-foreground sm:text-[15px]">{sub}</p>
      </div>
    </header>
  );
}

// Why the new-session box can't start anything yet, or "" when it can: with
// no model provider signed in, an agent has nothing to think with, so the
// box says so and points at the step that fixes it instead of letting a new
// owner press Start into an error. Unknown (the harness still loading, or a
// hub that can't say) never blocks.
export function useStartBlock(): string {
  const { setup } = useSessionsState();
  const p = setup.provider;
  if (!p) return "";
  if (p.expired) return p.label + " sign-in expired. Sign in again above, then start.";
  if (!p.loggedIn) return "Connect a model provider first (step 1 above): agents need one to think with.";
  return "";
}

// The page's model, read through useApi's shared cache: two callers on one
// screen cost no second request.
function useSessionsState() {
  const org = useApi<Org>("org");
  const tasks = useApi<TasksPayload>("tasks");
  const service = useApi<ServicePayload>("service", { refresh: 60000 });
  // The setup card needs the whole harness status to open the provider wizard.
  const harness = useApi<HarnessStatus>("harness", { refresh: 60000 });
  // The runner's state and the inputs that failed: the same model the bell reads.
  const inbox = useInbox();
  const o = org.data;
  const projects = registeredProjects(o);
  const daemon: AttentionDaemon = {
    label: inbox.daemon.label,
    pause: inbox.daemon.pause,
    canResume: inbox.daemon.canResume && !!o?.can_decide,
    setup: inbox.daemon.setup,
  };
  // The provider's sign-in as the setup card reads it: undefined while the
  // harness loads (so the card doesn't flash), null when the hub can't say.
  const hp = harness.data?.provider;
  const setup: SetupSnapshot = {
    provider: harness.data
      ? hp
        ? { label: hp.label, loggedIn: !!hp.login?.logged_in, expired: !!inbox.provider?.expired }
        : null
      : harness.error
        ? null
        : undefined,
    projects: projects.length,
    daemon,
  };
  // Before the org arrives nothing counts as a first run.
  const firstRun = !!o && isFirstRun(setup);
  // In a first run the provider and the runner are steps of the setup card,
  // not problems in the strip; the strip keeps only inputs that failed.
  const attention = o
    ? attentionRows(
        { machine: inbox.machine, provider: inbox.provider, down: inbox.down, downWhy: inbox.downWhy },
        daemon,
        {
          settings: hrefs.settings,
          connect: (name) => "/settings/?connect=" + encodeURIComponent(name) + "#harness",
        },
      ).filter((row) => !firstRun || row.fix.kind === "retry")
    : [];
  return { org, tasks, service: service.data, harness, inbox, daemon, setup, firstRun, attention };
}
