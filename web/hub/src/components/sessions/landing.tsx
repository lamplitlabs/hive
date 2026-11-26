"use client";

import { AttentionStrip, retryInputs } from "@/components/sessions/attention-strip";
import { HardStops } from "@/components/sessions/hard-stops";
import { SetupCard } from "@/components/sessions/setup-card";
import { attentionRows, isFirstRun, type AttentionDaemon, type SetupSnapshot } from "@/components/sessions/sessions-state";
import { WorkingNow } from "@/components/sessions/working-now";
import { useInbox } from "@/components/hive/inbox";
import { registeredProjects, type HarnessStatus, type Org, type ServicePayload, type TasksPayload } from "@/lib/api";
import { hrefs } from "@/lib/nav";
import { useApi } from "@/lib/use-api";

// What Sessions shows besides the conversations, now that the console opens
// on it (TASK-129: Home and For you went). Above the page, only when there
// is something: the broken-strip (the runner is not running, no model
// provider, an input that did not load) and the strip of asks only the owner
// may answer (money, what can't be undone, credentials, publication), with
// the card that sets Hive up between them on a new machine. Each is absent
// when nothing is wrong, nothing is left to set up or nothing waits.
export function SessionsAlerts() {
  const { attention, service, setup, firstRun, harness, daemon } = useSessionsState();
  return (
    <>
      <AttentionStrip rows={attention} service={service} onRetry={retryInputs} />
      {firstRun ? <SetupCard snapshot={setup} harness={harness.data} service={service} daemon={daemon} /> : null}
      <HardStops />
    </>
  );
}

// Under the new-session box on the landing: the agents at work (only while
// one runs).
export function SessionsLanding() {
  const { org, tasks } = useSessionsState();
  const o = org.data;
  if (!o) return null;
  const names = new Map(registeredProjects(o).map((p) => [p.slug, p.name || p.slug]));
  return <WorkingNow claims={o.active_claims} tasks={tasks.data?.tasks ?? []} names={names} />;
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
