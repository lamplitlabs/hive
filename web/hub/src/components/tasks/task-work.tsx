"use client";

import Link from "next/link";
import { useState, type ReactNode } from "react";
import { GitPullRequestIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Avatar, Mono } from "@/components/pulse/who";
import { enc, post, taskProgress, type Task, type TaskDelivery, type TaskWorker } from "@/lib/api";
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
// to the lead, done jobs wait for their branch to land.
export function idleLine(t: Task): string | null {
  const p = taskProgress(t);
  if (p.working.length || p.running || (t.status !== "in_progress" && t.status !== "planned")) return null;
  if (p.queued) return "The next step is waiting for a free agent.";
  if (p.blocked) return "A step is blocked. The lead agent will sort it out or ask you.";
  if (!p.total) return t.lead ? "The lead agent is planning the steps." : "Waiting for the lead agent to plan the steps.";
  if (p.failed) return "A step failed. The lead agent is planning another way.";
  return "Every step is done. Waiting to merge the work.";
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

// Where the task's code is once it landed: still on this machine, or on
// GitHub (pushed, or on the pull request from pulse/landed). The button asks
// the daemon for a pull request; it answers on its next landing pass, so
// the line says "asked" until then (ADR-037, the pull request path).
export function TaskDeliveryLine({ id, delivery, onChange, className }: { id: string; delivery: TaskDelivery | null | undefined; onChange: () => void; className?: string }) {
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  if (!delivery || delivery.state === "none") return null;
  const d = delivery;
  const sha = d.sha ? <Mono className="text-xs">{d.sha.slice(0, 12)}</Mono> : null;
  let text: ReactNode;
  let tone: "neutral" | "ok" | "warn" = "neutral";
  switch (d.state) {
    case "waiting":
      text = d.waiting === 1 ? "One finished step waits to land on the main branch." : d.waiting + " finished steps wait to land on the main branch.";
      break;
    case "landed":
      text = (
        <>
          Landed on the main branch on this machine {sha}, not on GitHub yet.{d.note ? " " + d.note : ""}
        </>
      );
      tone = "warn";
      break;
    case "held":
      text = (
        <>
          Landed on this machine {sha}, but it did not reach GitHub{d.note ? ": " + d.note : "."}
        </>
      );
      tone = "warn";
      break;
    case "asked":
      text = "A pull request was asked for. PULSE opens it on its next landing pass.";
      break;
    case "pushed":
      text = (
        <>
          Pushed to {d.remote || "the remote"} {sha}.
        </>
      );
      tone = "ok";
      break;
    case "pr":
      text = (
        <>
          On pull request{" "}
          {d.url ? (
            <a href={d.url} target="_blank" rel="noreferrer" className="font-medium underline underline-offset-2">
              #{d.number}
            </a>
          ) : (
            "#" + d.number
          )}
          {d.remote ? " into " + d.remote : ""}, waiting for your merge.
        </>
      );
      tone = "ok";
      break;
    default:
      return null;
  }
  const ask = async () => {
    setBusy(true);
    setFailed(null);
    try {
      await post("tasks/" + enc(id) + "/pull-request", {});
      onChange();
    } catch (e) {
      setFailed(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className={cn("mt-1 flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-xl border px-3 py-2 text-[13px]", tone === "ok" ? "border-ok/30 bg-ok/10" : tone === "warn" ? "border-warn/35 bg-warn/10" : "bg-card", className)} data-testid="task-delivery">
      <GitPullRequestIcon className={cn("size-4 shrink-0", tone === "ok" ? "text-ok" : tone === "warn" ? "text-warn" : "text-muted-foreground")} aria-hidden />
      <span className="min-w-0 flex-1 leading-relaxed anywhere">{text}</span>
      {d.can_ask ? (
        <Button size="sm" variant="outline" onClick={ask} disabled={busy} title="PULSE pushes the main branch to pulse/landed on GitHub and opens a pull request for you to merge.">
          <GitPullRequestIcon /> {busy ? "Asking…" : "Open a pull request"}
        </Button>
      ) : null}
      {failed ? (
        <span role="alert" className="basis-full text-bad">
          {failed}
        </span>
      ) : null}
    </div>
  );
}
