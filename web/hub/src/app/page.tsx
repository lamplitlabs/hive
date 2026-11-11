"use client";

import { useRouter } from "next/navigation";
import { useEffect } from "react";
import { AttentionStrip, retryInputs } from "@/components/home/attention-strip";
import { SetupCard } from "@/components/home/first-run";
import { HomeHeader } from "@/components/home/header";
import { attentionRows, isFirstRun, type AttentionDaemon, type SetupSnapshot } from "@/components/home/home-state";
import { NeedsYou } from "@/components/home/needs-you";
import { ProjectsList } from "@/components/home/projects-list";
import { ThisWeek } from "@/components/home/this-week";
import { WorkingNow } from "@/components/home/working-now";
import { useInbox } from "@/components/hive/inbox";
import { ErrorNotice, SkeletonPage } from "@/components/hive/panel";
import { Mono } from "@/components/hive/who";
import { registeredProjects, type HarnessStatus, type Org, type ServicePayload, type TasksPayload } from "@/lib/api";
import { hrefs } from "@/lib/nav";
import { useApi } from "@/lib/use-api";

// Home (spec "Home"): one screen - what needs you, what is running, and
// whether anything is broken. Top to bottom: the greeting with New task and
// New session; the attention strip, only when something is wrong; then two
// columns - Needs you and Working now on the left (8 of 12), Projects and
// This week on the right (4). The metrics, the sparklines and the machine's
// details live on Insights and Machines now; the waiting count is in the
// sidebar and on For you.
//
// Four states: busy (everything above), all caught up (Needs you says
// "Nothing needs you" and offers three starts, the rest stays), first run
// (no provider or no project: the three-step setup card replaces Needs
// you), and something broken (the strip at the top).
export default function HubPage() {
  const router = useRouter();
  const org = useApi<Org>("org");
  const tasks = useApi<TasksPayload>("tasks");
  const service = useApi<ServicePayload>("service", { refresh: 60000 });
  // The inbox reads the harness too (same cache, no second request); the
  // setup card needs the whole status to open the provider wizard.
  const harness = useApi<HarnessStatus>("harness", { refresh: 60000 });
  // Everything that waits for the owner, the runner's state and the inputs
  // that failed: the same model the sidebar count and For you read.
  const inbox = useInbox();

  // "/#daemon-controls" used to open a fold on Home; the runner's controls
  // are on Machines now, so the old deep link lands there.
  useEffect(() => {
    const go = () => {
      if (window.location.hash === "#daemon-controls") router.replace(hrefs.compute());
    };
    go();
    window.addEventListener("hashchange", go);
    return () => window.removeEventListener("hashchange", go);
  }, [router]);

  if (org.error && !org.data)
    return (
      <div className="mx-auto w-full max-w-[1280px] pt-5 sm:pt-6">
        <h1 className="mb-4 text-[32px] font-semibold leading-10 tracking-tight">Home</h1>
        <ErrorNotice error={org.error} onRetry={() => void org.reload()}>
          Hive didn&apos;t answer ({org.error}). Start it with <Mono>hive hub</Mono> in a terminal, then try again.
        </ErrorNotice>
      </div>
    );
  if (!org.data) return <SkeletonPage rows={3} />;

  const o = org.data;
  const projects = registeredProjects(o);
  const names = new Map(projects.map((p) => [p.slug, p.name || p.slug]));
  const daemon: AttentionDaemon = {
    label: inbox.daemon.label,
    pause: inbox.daemon.pause,
    canResume: inbox.daemon.canResume && !!o.can_decide,
    setup: inbox.daemon.setup,
  };

  // The provider's sign-in as the setup card reads it: undefined while the
  // harness loads (so the card doesn't flash), null when the hub can't say.
  const hp = harness.data?.provider;
  const setup: SetupSnapshot = {
    provider: harness.data
      ? hp
        ? {
            label: hp.label,
            loggedIn: !!hp.login?.logged_in,
            expired: !!inbox.provider?.expired,
          }
        : null
      : harness.error
        ? null
        : undefined,
    projects: projects.length,
    daemon,
  };
  const firstRun = isFirstRun(setup);

  // In a first run the provider and the runner are steps of the setup card,
  // not problems in the strip; the strip keeps only inputs that failed.
  const attention = attentionRows(
    {
      machine: inbox.machine,
      provider: inbox.provider,
      down: inbox.down,
      downWhy: inbox.downWhy,
    },
    daemon,
    {
      settings: hrefs.settings,
      connect: (name) => "/settings/?connect=" + encodeURIComponent(name) + "#harness",
    },
  ).filter((row) => !firstRun || row.fix.kind === "retry");

  return (
    <div className="mx-auto w-full max-w-[1280px] space-y-8 pb-8">
      <HomeHeader name={o.owner_name} />

      <AttentionStrip rows={attention} service={service.data} onRetry={retryInputs} />

      <div className="grid min-w-0 grid-cols-1 gap-8 lg:grid-cols-12">
        <div className="min-w-0 space-y-8 lg:col-span-8">
          {firstRun ? <SetupCard snapshot={setup} harness={harness.data} service={service.data} daemon={daemon} /> : <NeedsYou inbox={inbox} org={o} />}
          <WorkingNow claims={o.active_claims} tasks={tasks.data?.tasks ?? []} names={names} />
        </div>
        <div className="min-w-0 space-y-8 lg:col-span-4">
          <ProjectsList projects={projects} waiting={(slug) => inbox.forProject(slug).total} onRegistered={(slug) => router.push(hrefs.project(slug))} />
          <ThisWeek />
        </div>
      </div>
    </div>
  );
}
