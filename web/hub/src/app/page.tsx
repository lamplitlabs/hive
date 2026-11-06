"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useState, type ReactNode } from "react";
import { ArrowRightIcon, ChevronDownIcon, CircleCheckIcon, CirclePowerIcon, PlugZapIcon, ServerIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CodeDeliveredKpi } from "@/components/code-delivery/report";
import { ActivityChart, lastDays } from "@/components/pulse/charts";
import { GoalNudge, useGoalNudges } from "@/components/home/goal-nudge";
import { KpisPanel } from "@/components/home/kpis-panel";
import { HomeHero } from "@/components/home/hero";
import { awakeView } from "@/components/shell/awake";
import { CommonsLine, commonsLive, ProjectCard, ProposalCard, QuestionCard, WorkingNow, workingRows, workingSummary } from "@/components/pulse/cards";
import { MachinePanel } from "@/components/compute/machine";
import { ComputeInboxCard } from "@/components/compute/compute";
import { DaemonControls } from "@/components/pulse/daemon-controls";
import { daemonHeadline, daemonUpdateText } from "@/components/pulse/daemon-control-state";
import { GateCard } from "@/components/pulse/evolve";
import { useInbox } from "@/components/pulse/inbox";
import { CardGrid, ItemCard, ItemCardBody, ItemCardHeader, ItemCardKind, ItemCardMeta, ItemCardTitle, SectionHeading } from "@/components/pulse/item-card";
import { LiveFeed } from "@/components/pulse/live-feed";
import { FailedJobsSection } from "@/components/pulse/job-repair-section";
import { EmptyState, ErrorNotice, Kpi, KpiGrid, Kv, Notice, Panel, SkeletonPage, SkeletonRows } from "@/components/pulse/panel";
import { Mono, ProjectLink } from "@/components/pulse/who";
import { AddProjectButton } from "@/components/project/project-dialogs";
import { ProposedRecordCard } from "@/components/records/decide-record";
import { DraftCard } from "@/components/records/draft-panel";
import { RecordSheet } from "@/components/records/records";
import { RecentTasks } from "@/components/home/recent-tasks";
import { NewTaskButton } from "@/components/tasks/task-dialogs";
import { WorkSuggestionCard } from "@/components/tasks/suggested-work";
import { hrefs } from "@/lib/nav";
import { providerInfo } from "@/lib/providers";
import { num, plural } from "@/lib/format";
import { registeredProjects, type Compute, type Org, type RecordRow, type Runner, type RunningPayload, type ServicePayload, type SettingsPayload, type TasksPayload, type Timeline } from "@/lib/api";
import { useApi } from "@/lib/use-api";

// Home is a briefing, not a dashboard. It answers, in this order: is
// anything waiting for me (the hero and For you), who is working and on
// what (Working now, with the tasks I gave last), how are my projects, and
// how the fleet moves over time (numbers; the chart unfolds on request).
// The runner's controls, failed jobs, the live feed and version info live
// under one fold at the end - the Runner tile and "#daemon-controls" open it.
// Each fact appears once; the sidebar carries the For you count.
export default function HubPage() {
  const router = useRouter();
  const org = useApi<Org>("org");
  const tl = useApi<Timeline>("timeline", { refresh: 60000 });
  const runner = useApi<Runner>("runner");
  const tasks = useApi<TasksPayload>("tasks");
  const compute = useApi<Compute>("compute");
  // Whether this machine keeps PULSE alive yet (ADR-038): the one step a shared copy asks for.
  const service = useApi<ServicePayload>("service", { refresh: 60000 });
  const settings = useApi<SettingsPayload>("settings");
  // Which PULSE runs and whether an update waits (for "Restart now").
  const running = useApi<RunningPayload>("running", { refresh: 60000 });
  // Everything that waits for the owner's yes or no - the same count the sidebar and the project pages show.
  const forYou = useInbox();
  const [openRecord, setOpenRecord] = useState<RecordRow | null>(null);
  const [allInbox, setAllInbox] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  // The Runner tile and the This machine card open the folded details and scroll to the controls.
  const [toRunner, setToRunner] = useState(false);
  // The shared board is not a project: it gets one row under the grid, and
  // only while something is live on it.
  const projects = registeredProjects(org.data);
  // Projects whose lead still works toward the generated goal (ADR-040), minus the notes the owner put away.
  const goalNotes = useGoalNudges(projects);

  useEffect(() => {
    const openFromHash = () => {
      if (window.location.hash === "#daemon-controls") {
        setDetailsOpen(true);
        window.localStorage.setItem("pulse.home.details", "open");
        window.requestAnimationFrame(() => document.getElementById("daemon-controls")?.scrollIntoView());
      }
    };
    if (window.localStorage.getItem("pulse.home.details") === "open") window.requestAnimationFrame(() => setDetailsOpen(true));
    openFromHash();
    window.addEventListener("hashchange", openFromHash);
    return () => window.removeEventListener("hashchange", openFromHash);
  }, []);

  useEffect(() => {
    if (!toRunner || !detailsOpen) return;
    const el = document.getElementById("daemon-controls");
    if (!el) return;
    el.scrollIntoView({ block: "start" });
    window.requestAnimationFrame(() => setToRunner(false));
  }, [toRunner, detailsOpen]);

  if (org.error && !org.data)
    return (
      <div className="pt-8 sm:pt-12">
        <h1 className="mb-4 text-[30px] font-semibold tracking-tight">Home</h1>
        <ErrorNotice error={org.error} onRetry={() => void org.reload()}>
          PULSE didn&apos;t answer ({org.error}). Start it with <Mono>pulse hub</Mono> in a terminal, then try again.
        </ErrorNotice>
      </div>
    );
  if (!org.data) return <SkeletonPage rows={4} />;

  const o = org.data;
  const commons = o.commons;
  const { decisions: adrs, proposals: awaiting, questions, agentsDeciding, compute: pendingCompute, work: suggestedWork, drafts, steps, machine: machineStep, provider: providerStep, total: inbox, daemon, checking, down, downWhy, gateOf } = forYou;
  // The inbox shows its first few cards and unfolds on request: Home says
  // what waits, the cards past the fold are one click away.
  const INBOX_SHOWN = 4;
  const openTasks = (tasks.data?.tasks ?? []).filter((t) => t.status !== "done" && t.status !== "cancelled");
  const working = workingRows(o.active_claims, openTasks);
  // The tasks the owner gave last: the ones that asked them first, then the newest.
  const recent = [...openTasks].sort((a, b) => (a.status === "review") === (b.status === "review") ? (a.updated_at < b.updated_at ? 1 : -1) : a.status === "review" ? -1 : 1).slice(0, 4);
  const attention = projects.filter((p) => p.error || p.missing || p.stop_present || !p.board_present);
  const r = runner.data;
  const sparkFor = (slug: string): number[] | undefined => {
    const s = tl.data?.series?.find((x) => x.slug === slug);
    return s ? lastDays(s, 14) : undefined;
  };
  const runnerChecking = !runner.data && !runner.error;
  const runnerDown = !!runner.error && !runner.data;
  const paused = daemon.pause === "on";
  const setDetails = (open: boolean) => {
    setDetailsOpen(open);
    window.localStorage.setItem("pulse.home.details", open ? "open" : "closed");
  };
  const openRunner = () => {
    setDetails(true);
    setToRunner(true);
  };
  // Every card that waits for the owner, in the order they should be read:
  // decisions first, then proposals and questions, then the machine's own.
  const inboxCards: ReactNode[] = [
    // No model, no agents: connecting one comes before every decision.
    ...(providerStep
      ? [
          <ItemCard key="provider" href={"/settings/?connect=" + encodeURIComponent(providerStep.name) + "#harness"} highlight ariaLabel="Connect a model provider">
            <ItemCardHeader>
              <ItemCardKind icon={PlugZapIcon} tone="you">Models</ItemCardKind>
            </ItemCardHeader>
            <ItemCardTitle chevron>{providerStep.expired ? "Sign in to " + providerInfo(providerStep.name, providerStep.label).title + " again" : "Connect a model provider"}</ItemCardTitle>
            <ItemCardBody>{providerStep.expired ? "The sign-in expired, so agents can't call their models. It takes a minute; nothing else changes." : "Agents need a model to work with. Sign in with an account you already have, paste an API key, or use models running on this computer."}</ItemCardBody>
            <ItemCardMeta>
              <span className="inline-flex items-center gap-1 font-medium text-primary">{providerStep.expired ? "Sign in again" : "Connect a provider"} <ArrowRightIcon className="size-3.5" aria-hidden /></span>
            </ItemCardMeta>
          </ItemCard>,
        ]
      : []),
    ...adrs.map((rec) => <ProposedRecordCard key={rec.project + "/" + rec.id} r={rec} onOpen={setOpenRecord} />),
    ...awaiting.map((p) => <ProposalCard key={p.id} r={p} slug={p.slug || o.totals.brain} canDecide={o.can_decide} />),
    ...questions.map((q) => <QuestionCard key={(q.slug || "") + "/" + q.id} q={q} slug={q.slug || o.totals.brain} canDecide={o.can_decide} />),
    ...pendingCompute.map((cr) => <ComputeInboxCard key={cr.id} r={cr} caps={compute.data?.caps} />),
    ...suggestedWork.map((s) => <WorkSuggestionCard key={s.id} s={s} onOpenTask={(id) => router.push(hrefs.task(id))} />),
    ...drafts.map(({ slug, r: d }) => <DraftCard key={slug + "/" + d.id} slug={slug} r={d} />),
    ...(machineStep
      ? [
          <ItemCard key="machine" onOpen={openRunner} highlight ariaLabel="Open the runner controls">
            <ItemCardHeader>
              <ItemCardKind icon={CirclePowerIcon} tone="you">This machine</ItemCardKind>
            </ItemCardHeader>
            <ItemCardTitle chevron>{daemon.pause === "on" ? "Resume work on this machine" : daemon.setup === "start" ? "Start the runner" : "Keep PULSE running in the background"}</ItemCardTitle>
            <ItemCardBody>{daemon.pause === "on" ? "Agents finish what they're doing and start nothing new until you resume." : daemon.setup === "start" ? "Nothing runs until the runner is on. Start it here; you'll see what it does before it does it." : "Set PULSE up as a background service so agents keep working after you close the terminal. You'll see exactly what changes before anything is installed."}</ItemCardBody>
            <ItemCardMeta>
              <span className="inline-flex items-center gap-1 font-medium text-primary">Open the runner controls <ArrowRightIcon className="size-3.5" aria-hidden /></span>
            </ItemCardMeta>
          </ItemCard>,
        ]
      : []),
    ...steps.map((s) => <GateCard key={s.slug + "/" + s.kind} step={s} />),
  ];

  return (
    <>
      <HomeHero
        inbox={inbox}
        paused={paused}
        daemonDown={runnerDown}
        inboxChecking={checking.length > 0}
        activeAgents={o.totals.agents_active}
        projects={projects.length}
        ownerAway={o.owner_presence === "away"}
        canEdit={!!settings.data?.can_edit}
        awake={awakeView(r)}
        glance={{
          inbox,
          inboxChecking: checking.length > 0,
          agentsDeciding,
          workingAgents: working.length,
          things: working.reduce((n, w) => n + w.claims.length + (w.task ? 1 : 0), 0),
          jobs: r ? r.jobs : null,
          runnerChecking,
          runnerDown,
          runnerLabel: daemonHeadline(daemon),
          runnerTone: paused ? "warn" : daemon.tone,
          runnerNote: !paused && daemon.update ? daemonUpdateText(daemon.update) : "",
          projects,
          agentsActive: o.totals.agents_active,
        }}
        onOpenRunner={openRunner}
        actions={<NewTaskButton size="default" variant={inbox ? "outline" : "default"} />}
      />

      {inbox ? (
        <section id="for-you" className="mt-12 scroll-mt-6" aria-label="For you">
          <SectionHeading count={inbox} note={down.length ? <span className="text-warn">{down.join("; ")}.</span> : "Nothing here moves until you answer. Read what the agents found, then decide."}>
            For you
          </SectionHeading>
          {agentsDeciding > 0 ? (
            <p className="-mt-3 mb-4 text-xs text-muted-foreground">
              and {agentsDeciding} more the agents are deciding (the lead while you are away, or a setting answers them).
            </p>
          ) : null}
          <CardGrid cols={2} className="max-xl:grid-cols-1">
            {(allInbox ? inboxCards : inboxCards.slice(0, INBOX_SHOWN))}
          </CardGrid>
          {inboxCards.length > INBOX_SHOWN ? (
            <button type="button" className="mt-3 flex min-h-11 w-full items-center justify-center gap-1.5 rounded-2xl border border-dashed px-4 text-sm font-medium text-muted-foreground transition-colors hover:border-foreground/30 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none" onClick={() => setAllInbox((v) => !v)} aria-expanded={allInbox} aria-controls="for-you">
              {allInbox ? <>Show fewer <ChevronDownIcon className="size-4 rotate-180" aria-hidden /></> : <>Show all {inboxCards.length} waiting <ChevronDownIcon className="size-4" aria-hidden /></>}
            </button>
          ) : null}
        </section>
      ) : (
        <p id="for-you" className="mt-10 flex max-w-3xl scroll-mt-6 items-center gap-3 rounded-2xl border bg-card px-4 py-3.5 text-sm leading-relaxed text-muted-foreground">
          <CircleCheckIcon className="size-5 shrink-0 text-ok" aria-hidden />
          <span className="min-w-0 anywhere">{downWhy.length ? "Nothing waits for you that PULSE can see, but " + downWhy.join(" and ") + "." : checking.length ? "Checking " + checking.join(" and ") + "…" : "Nothing waits for your answer right now. New proposals and questions will show up here."}</span>
        </p>
      )}

      <section id="working" aria-label="Working now" className="mt-12 scroll-mt-6">
        <SectionHeading note="Who is on what right now, and the tasks you gave last.">Working now</SectionHeading>
        <div className="grid min-w-0 items-start gap-4 xl:grid-cols-[3fr_2fr]">
          <Panel
            title="Agents"
            actions={
              <span className="flex min-w-0 items-center gap-3 text-[13px]">
                <span>{workingSummary(working, o.active_claims.length)}</span>
                {r?.jobs.queued ? <span className="tnum text-muted-foreground">· {plural(r.jobs.queued, "job")} queued</span> : null}
              </span>
            }
          >
            <WorkingNow claims={o.active_claims} tasks={openTasks} />
          </Panel>
          <Panel
            title="Your tasks"
            actions={
              <Button asChild variant="ghost" size="sm" className="-my-1">
                <Link href={hrefs.tasks()}>
                  All tasks <ArrowRightIcon />
                </Link>
              </Button>
            }
          >
            {tasks.data ? (
              <RecentTasks tasks={recent} empty="No open tasks. Give one with New task: the project's lead agent splits it into steps and reports back in its chat." />
            ) : tasks.error ? (
              <ErrorNotice error={tasks.error} onRetry={() => void tasks.reload()} />
            ) : (
              <SkeletonRows n={2} />
            )}
          </Panel>
        </div>
      </section>

      <section id="projects" aria-label="Your projects" className="mt-12 scroll-mt-6">
        <SectionHeading count={projects.length} note="Open a project to talk to its lead agent, see its tasks and follow its board." action={<AddProjectButton size="sm" variant="outline" onRegistered={(rr) => router.push(hrefs.project(rr.project.slug))} />}>
          Your projects
        </SectionHeading>
        {attention.length || goalNotes.rows.length ? (
          <div className="mb-3 grid gap-2" aria-label="Needs attention">
            {attention.map((p) => (
              <Notice key={p.slug} tone="warn">
                <ProjectLink slug={p.slug} />{" "}
                <span className="text-muted-foreground">{p.stop_present ? "is paused: a STOP file in its folder keeps self-improvement from running. Delete the file to let it carry on." : p.missing || !p.present ? "is missing from disk. Put the folder back at " + p.path + ", or remove the project." : !p.board_present ? "has no board yet, so its agents can't post. Run pulse init in its folder to create one." : p.error}</span>
              </Notice>
            ))}
            {goalNotes.rows.map((p) => (
              <GoalNudge key={"goal/" + p.slug} p={p} onDismiss={() => goalNotes.dismiss(p.slug)} />
            ))}
          </div>
        ) : null}
        {projects.length ? (
          <CardGrid cols={3}>
            {projects.map((p) => (
              <ProjectCard key={p.slug} p={p} spark={sparkFor(p.slug)} gate={gateOf(p)} forYou={forYou.forProject(p.slug).total} />
            ))}
          </CardGrid>
        ) : (
          <EmptyState title="Add your first project" action={<AddProjectButton size="sm" onRegistered={(rr) => router.push(hrefs.project(rr.project.slug))} />}>
            Point PULSE at a checkout on this machine. Its lead agent reads the code, sets up the board and is ready for your first task. You can also run <Mono>pulse init</Mono> inside the checkout.
          </EmptyState>
        )}
        {commons && commonsLive(commons) ? <CommonsLine c={commons} className="mt-3" /> : null}
      </section>

      <section aria-label="Over time" className="mt-12">
        <KpisPanel height={200} />
      </section>

      <section aria-label="Details for this machine" className="mt-12">
        <button
          type="button"
          className="group flex w-full min-w-0 items-center justify-between gap-3 rounded-2xl border bg-card px-4 py-3.5 text-left transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring sm:px-5"
          aria-expanded={detailsOpen}
          aria-controls="pulse-home-details"
          onClick={() => setDetails(!detailsOpen)}
        >
          <span className="flex min-w-0 items-center gap-3">
            <ServerIcon className="size-[18px] shrink-0 text-muted-foreground" aria-hidden />
            <span className="min-w-0">
              <span className="block text-[15px] font-semibold">Details for this machine</span>
              <span className="block text-[13px] text-muted-foreground">The runner and its controls, failed jobs, the live event feed, board activity and version info.</span>
            </span>
          </span>
          <ChevronDownIcon className={"size-4 shrink-0 text-muted-foreground transition-transform motion-reduce:transition-none " + (detailsOpen ? "rotate-180" : "")} aria-hidden />
        </button>
        {detailsOpen ? (
          <div id="pulse-home-details" className="mt-4 grid gap-6">
            <section id="daemon-controls" aria-label="Runner controls" className="scroll-mt-6">
              <DaemonControls runner={runner} service={service} settings={settings} running={running} />
            </section>

            <section aria-label="At a glance">
              <KpiGrid className="mb-0">
                <Kpi n={o.totals.agents_active} label="Agents active" sub={num(o.totals.agents) + " known in all"} />
                <Kpi n={o.totals.votes_open} label="Open votes" sub="Suggestions and rule changes" href={hrefs.votes()} />
                <CodeDeliveredKpi />
                <Kpi n={o.totals.posts_24h} label="Posts today" sub={o.totals.questions_open ? plural(o.totals.questions_open, "question") + " still open" : num(o.totals.posts) + " in all"} />
              </KpiGrid>
            </section>

            <FailedJobsSection />

            <div className="grid min-w-0 gap-4 xl:grid-cols-2">
              <MachinePanel r={r} error={runner.error && !r ? runner.error : undefined} />
              <LiveFeed limit={40} height={320} />
            </div>

            {tl.data ? <ActivityChart tl={tl.data} bys={["total", "project", "kind"]} height={180} /> : null}

            <Panel title="About this hub">
              <Kv
                rows={[
                  ["Projects", plural(o.totals.projects, "project")],
                  ["Self-update builds from", o.totals.brain ? <Mono key="brain">{o.totals.brain}</Mono> : "No source checkout registered"],
                  ["Version", o.binary_version ? <Mono key="pulse">{o.binary_version}</Mono> : "Unknown"],
                  ["Store", <Mono key="store">{o.store || o.registry}</Mono>],
                ]}
              />
            </Panel>
          </div>
        ) : null}
      </section>

      {/* Read on an inbox card opens the record here; its header carries the same three answers. */}
      <RecordSheet id={openRecord?.id ?? null} project={openRecord?.project} onClose={() => setOpenRecord(null)} />
    </>
  );
}
