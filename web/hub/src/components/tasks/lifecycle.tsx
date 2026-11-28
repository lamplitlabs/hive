"use client";

// Where a task stands in the lifecycle of docs/sdlc.md (TASK-141): the
// Lifecycle tab beside a task (its phases in order, what each left behind
// as links, the gate to the next) and the Phases view of the Tasks page (a
// project's tasks by phase). The hub derives the phases; lib/lifecycle.ts
// words them.

import Link from "next/link";
import { CircleCheckIcon, CircleDashedIcon, CircleDotIcon, CircleSlashIcon, ExternalLinkIcon, RotateCcwIcon } from "lucide-react";
import { ErrorNotice, SkeletonRows } from "@/components/hive/panel";
import { When } from "@/components/hive/who";
import type { ProjectLifecycle, TaskLifecycle } from "@/lib/api";
import {
  type MarkView,
  type PhaseColumn,
  type StepView,
  phaseColumns,
  phaseLine,
  phasesSummary,
  progressLine,
  stepViews,
} from "@/lib/lifecycle";
import { hrefs } from "@/lib/nav";
import { useApi } from "@/lib/use-api";
import { cn } from "@/lib/utils";

const enc = encodeURIComponent;

// Both poll no faster than the Sessions page reads its tasks (use-api's
// default, 30 s): the live events already refetch them within seconds of a
// job, run, task or land event (refresh.ts), and the poll is only the net
// under those.
export function useTaskLifecycle(id: string) {
  return useApi<TaskLifecycle>(id ? "tasks/" + enc(id) + "/lifecycle" : null);
}

export function useProjectLifecycle(project: string | null) {
  return useApi<ProjectLifecycle>(project ? "tasks/lifecycle?project=" + enc(project) : null);
}

// The task header's phase chip: "Build · an agent is at work".
export function PhaseChip({ l, className }: { l: TaskLifecycle | undefined; className?: string }) {
  if (!l) return null;
  const owner = l.waits_on === "owner";
  return (
    <span
      className={cn("inline-flex items-center gap-1", owner && "font-medium text-you-ink", className)}
      title={l.note || "Lifecycle phase (docs/sdlc.md)"}
    >
      {phaseLine(l, l.phases)}
      {l.rework ? <RotateCcwIcon className="size-3" aria-label="sent back once" /> : null}
    </span>
  );
}

function markHref(m: MarkView): { href: string; external: boolean } | null {
  const l = m.link;
  if (!l) return null;
  switch (l.to) {
    case "task":
      return { href: hrefs.task(l.id), external: false };
    case "run":
      return { href: hrefs.harness({ id: l.id }), external: false };
    case "chat":
      return { href: hrefs.conversations(l.id), external: false };
    case "url":
      return /^https?:\/\//i.test(l.href) ? { href: l.href, external: true } : null;
  }
}

function Mark({ m }: { m: MarkView }) {
  const to = markHref(m);
  const chip = cn(
    "inline-flex max-w-full items-center gap-1 rounded-md border bg-background px-1.5 py-0.5 text-xs",
    m.mono && "font-mono text-[11.5px]",
  );
  const body = (
    <>
      <span className="truncate">{m.text}</span>
      {to?.external ? <ExternalLinkIcon className="size-3 shrink-0 opacity-60" aria-hidden /> : null}
    </>
  );
  return (
    <span className="inline-flex min-w-0 max-w-full items-center gap-1">
      {to ? (
        to.external ? (
          <a href={to.href} target="_blank" rel="noreferrer" className={cn(chip, "hover:bg-muted")} title={m.title}>
            {body}
          </a>
        ) : (
          <Link href={to.href} className={cn(chip, "hover:bg-muted")} title={m.title}>
            {body}
          </Link>
        )
      ) : (
        <span className={chip} title={m.title}>
          {body}
        </span>
      )}
      {m.note ? <span className="truncate text-xs text-muted-foreground">{m.note}</span> : null}
    </span>
  );
}

const STATE_ICON = {
  passed: CircleCheckIcon,
  current: CircleDotIcon,
  ahead: CircleDashedIcon,
  skipped: CircleSlashIcon,
} as const;

function Step({ s, last }: { s: StepView; last: boolean }) {
  const Icon = STATE_ICON[s.state] ?? CircleDashedIcon;
  const current = s.state === "current";
  return (
    <li className="relative grid grid-cols-[1.25rem_1fr] gap-x-2" aria-current={current ? "step" : undefined}>
      <span className="relative flex justify-center">
        <Icon
          className={cn(
            "relative z-10 mt-0.5 size-4 bg-background",
            s.state === "passed" && "text-ok",
            current && "text-primary",
            (s.state === "ahead" || s.state === "skipped") && "text-muted-foreground/60",
          )}
          aria-hidden
        />
        {!last ? <span className="absolute top-5 bottom-0 w-px bg-border" aria-hidden /> : null}
      </span>
      {/* A phase ahead says what comes next, so its words stay at full
          strength (TASK-173: 70% opacity put them at 3.05 to 4.15:1); its
          hollow grey mark and grey name say it has not started. */}
      <div className="grid min-w-0 gap-1 pb-4">
        <div className="flex min-w-0 flex-wrap items-baseline gap-x-2">
          <span className={cn("text-[13px] font-semibold", current ? "text-foreground" : s.state === "ahead" ? "text-muted-foreground" : "text-foreground/85")}>{s.label}</span>
          <span className={cn("text-xs", current ? "font-medium text-primary" : "text-muted-foreground")}>{s.stateWord}</span>
          {s.at ? <When d={s.at} className="text-xs text-muted-foreground" /> : null}
        </div>
        {s.marks.length ? (
          <div className="flex min-w-0 flex-wrap gap-1.5">
            {s.marks.map((m) => (
              <Mark key={m.key} m={m} />
            ))}
          </div>
        ) : (
          <p className="text-xs text-muted-foreground">{s.empty}</p>
        )}
        {s.gate && s.state !== "skipped" ? (
          <p className="text-xs leading-relaxed text-muted-foreground">
            <span className="font-medium text-foreground/80">{s.state === "passed" ? "Passed when " : "Passes when "}</span>
            {s.gate}
          </p>
        ) : null}
      </div>
    </li>
  );
}

// The Lifecycle tab beside a task.
export function LifecyclePanel({ id }: { id: string }) {
  const q = useTaskLifecycle(id);
  const l = q.data;
  if (!l) {
    return q.error ? <ErrorNotice error={q.error} onRetry={() => void q.reload()} /> : <SkeletonRows n={4} />;
  }
  const steps = stepViews(l);
  return (
    <section className="grid min-w-0 gap-3" aria-label="Lifecycle">
      <div className="grid gap-0.5 px-1">
        <h3 className="flex items-center gap-1.5 text-xs font-semibold tracking-wide text-muted-foreground uppercase">
          Lifecycle
          <span className="font-normal normal-case tracking-normal">{progressLine(l)}</span>
        </h3>
        <p className={cn("text-[13px]", l.waits_on === "owner" ? "font-medium text-you-ink" : "text-foreground")}>
          {phaseLine(l, l.phases)}
          {l.rework ? <span className="text-muted-foreground"> · sent back once</span> : null}
        </p>
        {l.note ? <p className="text-xs leading-relaxed text-muted-foreground">{l.note}</p> : null}
      </div>
      <ol className="grid min-w-0 px-1">
        {steps.map((s, i) => (
          <Step key={s.phase} s={s} last={i === steps.length - 1} />
        ))}
      </ol>
    </section>
  );
}

// ----- the Tasks page's Phases view -----------------------------------------

function PhaseCard<T extends { id: string; title: string }>({ c }: { c: PhaseColumn<T> }) {
  return (
    <section
      className="flex max-h-[34rem] min-w-0 flex-col rounded-xl border bg-card"
      aria-label={c.label + ", " + c.tasks.length + " tasks"}
    >
      <header className="grid gap-0.5 border-b px-3 py-2">
        <div className="flex items-baseline gap-2">
          <h3 className="text-[13px] font-semibold">{c.label}</h3>
          <span className="tnum text-xs text-muted-foreground">{c.tasks.length}</span>
        </div>
        {c.gate ? <p className="line-clamp-2 text-[11.5px] leading-snug text-muted-foreground" title={c.gate}>Next when {c.gate}</p> : null}
      </header>
      <ul className="scroll-thin grid min-h-0 gap-1 overflow-y-auto p-1.5">
        {c.tasks.length ? (
          c.tasks.map(({ task, waits, note, rework }) => (
            <li key={task.id}>
              <Link
                href={hrefs.task(task.id)}
                className="grid min-w-0 gap-0.5 rounded-lg px-2 py-1.5 hover:bg-muted/60"
                title={note || task.title}
              >
                <span className="line-clamp-2 text-[13px] leading-snug text-foreground">{task.title}</span>
                <span className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                  <span className="font-mono text-[11px]">{task.id}</span>
                  {waits ? (
                    <span className={cn("truncate", waits === "waits on you" && "font-medium text-you-ink")}>· {waits}</span>
                  ) : null}
                  {rework ? <RotateCcwIcon className="size-3 shrink-0" aria-label="sent back once" /> : null}
                </span>
              </Link>
            </li>
          ))
        ) : (
          <li className="px-2 py-1.5 text-xs text-muted-foreground">None here.</li>
        )}
      </ul>
    </section>
  );
}

// A project's tasks by phase, after the page's own search and filters.
export function PhasesView<T extends { id: string; title: string }>({
  project,
  tasks,
}: {
  project: string;
  tasks: T[];
}) {
  const q = useProjectLifecycle(project || null);
  if (!project) {
    return <p className="px-1 text-sm text-muted-foreground">Pick a project to see its tasks by lifecycle phase.</p>;
  }
  if (!q.data) {
    return q.error ? <ErrorNotice error={q.error} onRetry={() => void q.reload()} /> : <SkeletonRows n={4} />;
  }
  const columns = phaseColumns(q.data, tasks);
  return (
    <div className="grid min-w-0 gap-3">
      <p className="px-1 text-[13px] text-muted-foreground">{phasesSummary(columns)}</p>
      <div className="grid min-w-0 gap-3 sm:grid-cols-2 xl:grid-cols-4">
        {columns.map((c) => (
          <PhaseCard key={c.phase} c={c} />
        ))}
      </div>
    </div>
  );
}
