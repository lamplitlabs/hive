"use client";

import Link from "next/link";
import { useState } from "react";
import { GitPullRequestIcon } from "lucide-react";
import { ConfirmDialog } from "@/components/pulse/confirm";
import { Avatar } from "@/components/pulse/who";
import { deliveryView } from "@/components/tasks/delivery-state";
import { Button } from "@/components/ui/button";
import { taskProgress, writes, type Task, type TaskDelivery, type TaskWorker } from "@/lib/api";
import { fmtDate, rel, sentence, truncate } from "@/lib/format";
import { hrefs } from "@/lib/nav";
import { useNow } from "@/lib/use-api";
import { cn } from "@/lib/utils";

export function isLeadPlanning(t: Pick<Task, "lead">, w: Pick<TaskWorker, "agent" | "kind">): boolean {
  return (!!t.lead && w.agent === t.lead) || w.kind === "lead";
}

// A relative time inside a sentence, exact time in the tooltip.
export function Ago({ d, className }: { d?: string | null; className?: string }) {
  const now = useNow();
  if (!d) return null;
  return (
    <time dateTime={d} title={fmtDate(d)} className={className}>
      {rel(d, now)}
    </time>
  );
}

// Why an active task has nobody on it right now, in the runner's own terms
// (tendTasks): a queued job waits for a free worker, a failed job goes back
// to the lead, done jobs wait for the task's pull request.
export function idleLine(t: Task): string | null {
  const p = taskProgress(t);
  if (p.working.length || p.running || (t.status !== "in_progress" && t.status !== "planned")) return null;
  if (p.queued) return "The next step is waiting for a free agent.";
  if (p.blocked) return "A step is blocked. The lead agent will sort it out or ask you.";
  if (!p.total) return t.lead ? "The lead agent is planning the steps." : "The lead agent is starting on it.";
  if (p.failed) return "A step failed. The lead agent is planning another way.";
  return "Every step is done. Waiting for the pull request.";
}

// One readable sentence: "cinder is working on Await the price lookup,
// started 9 minutes ago." It flows as text (never a flex row), so a narrow
// card wraps it like a sentence instead of squeezing each word into a column.
// The job id, its kind and the machine stay in the tooltip.
// `bare` drops the step's title where the row already shows it.
export function TaskWorkerInline({ t, w, compact, bare }: { t: Pick<Task, "lead">; w: TaskWorker; compact?: boolean; bare?: boolean }) {
  const planning = isLeadPlanning(t, w);
  const what = sentence(w.title || "") || "a step";
  const tip = ["Job " + w.job, w.kind ? "kind: " + w.kind : "", w.executor ? "on machine " + w.executor : ""].filter(Boolean).join(", ");
  return (
    <span className="min-w-0 anywhere" title={tip}>
      <Avatar seed={w.agent} size={18} className="mr-1.5 -mt-0.5 align-middle" />
      <span className="font-medium text-foreground">{w.agent || "An agent"}</span>
      {planning ? " is planning the steps" : bare ? " is working on it" : " is working on "}
      {planning || bare ? null : <span className="text-foreground">{truncate(what, compact ? 56 : 96)}</span>}
      {w.since ? (
        <>
          {", started "}
          <Ago d={w.since} />
        </>
      ) : null}
      {"."}
      {w.harness_session && !compact ? (
        <>
          {" "}
          <Link href={hrefs.harness({ id: w.harness_session })} title={"Run " + w.harness_session} className="font-medium text-primary underline-offset-2 hover:underline">
            Watch the run
          </Link>
        </>
      ) : null}
    </span>
  );
}

// Who is on the task right now, one sentence per agent.
export function TaskWorkingLine({ t, compact, className }: { t: Task; compact?: boolean; className?: string }) {
  const { working } = taskProgress(t);
  if (!working.length) return null;
  const shown = compact ? working.slice(0, 1) : working;
  return (
    <div className={cn("grid min-w-0 gap-1 text-[13px] leading-relaxed text-muted-foreground", className)}>
      {shown.map((w, i) => (
        <p key={w.job + i} className={cn("min-w-0", compact && "line-clamp-2")}>
          <TaskWorkerInline t={t} w={w} compact={compact} />
        </p>
      ))}
      {compact && working.length > 1 ? <p className="pl-6">{working.length === 2 ? "and 1 more agent" : "and " + (working.length - 1) + " more agents"}</p> : null}
    </div>
  );
}

// Where the task's code stands: each task gets one pull request on GitHub
// (branch pulse/task-<id>), opened when its work is done; the reviewer agent
// merges the simple ones and hands the rest to the owner, whose Merge is a
// `review.ask` the daemon performs on its next pass - so the line only says
// "Merging…" after the click, never "Merged" by itself. Older work may still
// show landing states. `taskId` enables the button; a card without it reads only.
export function TaskDeliveryLine({ delivery, taskId, className }: { delivery: TaskDelivery | null | undefined; taskId?: string; className?: string }) {
  const [merge, setMerge] = useState(false);
  const v = deliveryView(delivery);
  if (!v) return null;
  const tone = v.tone;
  const number = delivery?.number ? "PR #" + delivery.number : "the pull request";
  return (
    <div className={cn("mt-1 flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-xl border px-3 py-2 text-[13px]", tone === "ok" ? "border-ok/30 bg-ok/10" : tone === "warn" ? "border-warn/35 bg-warn/10" : "bg-card", className)} data-testid="task-delivery">
      <GitPullRequestIcon className={cn("size-4 shrink-0", tone === "ok" ? "text-ok" : tone === "warn" ? "text-warn" : "text-muted-foreground")} aria-hidden />
      <span className="min-w-0 flex-1 leading-relaxed anywhere">
        {v.before}
        {v.link ? (
          <a href={v.link.href} target="_blank" rel="noreferrer" className="font-medium underline underline-offset-2">
            {v.link.text}
          </a>
        ) : null}
        {v.after}
      </span>
      {v.canMerge && taskId ? (
        <>
          <Button type="button" size="xs" onClick={() => setMerge(true)} data-testid="task-merge">
            Merge
          </Button>
          <ConfirmDialog
            open={merge}
            onOpenChange={setMerge}
            title={"Merge " + number + " into " + (delivery?.base || "the default branch") + "?"}
            description="The daemon merges it on GitHub as you."
            action="Merge"
            onConfirm={() => writes.mergeTaskPr(taskId)}
            success={"Merging " + number + "\u2026"}
            invalidates={(p) => p.startsWith("tasks") || p.startsWith("chat") || p.startsWith("events")}
          />
        </>
      ) : null}
    </div>
  );
}
