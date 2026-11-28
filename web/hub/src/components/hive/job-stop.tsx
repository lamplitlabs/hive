"use client";

import { useState } from "react";
import { SquareIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { ConfirmDialog } from "./confirm";
import {
  STOP_DISMISS,
  machineName,
  stopActionLabel,
  stopButtonLabel,
  stopButtonTitle,
  stopDialogDetail,
  stopDialogTitle,
  stopRequeueLabel,
  stopShown,
  stopSuccessMessage,
  type JobStopResult,
} from "./job-stop-state";
import { writes } from "@/lib/api";

// Stop a job's run wherever it runs - this machine, a paired or a rented
// one - as `hive job stop JOB [--requeue]` does from a terminal. The dialog
// names the machine, says what the stop does to the run's commits, and
// offers "Stop and requeue" to put the job back in the queue instead.
export function JobStopButton({
  job,
  status,
  executor,
  title,
  onDone,
  iconOnly,
  className,
}: {
  job: string;
  // The job's status ("leased" while its run goes); the control shows
  // only for a job being worked on.
  status: string;
  executor?: string | null;
  title?: string;
  onDone?: () => unknown;
  iconOnly?: boolean;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const [requeue, setRequeue] = useState(false);
  const [asked, setAsked] = useState(false);
  if (!stopShown(status)) return null;
  const label = stopButtonLabel(asked);
  const tip = stopButtonTitle(executor);
  const button = iconOnly ? (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button type="button" size="icon-sm" variant="ghost" className={"text-status-failed hover:text-status-failed " + (className ?? "")} onClick={() => setOpen(true)} disabled={asked} aria-label={tip + (title ? ": " + title : "")} data-testid="job-stop">
          <SquareIcon />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{asked ? label : tip}</TooltipContent>
    </Tooltip>
  ) : (
    <Button type="button" size="sm" variant="outline" className={"text-status-failed hover:text-status-failed " + (className ?? "")} onClick={() => setOpen(true)} disabled={asked} aria-label={tip + (title ? ": " + title : "")} title={tip} data-testid="job-stop">
      <SquareIcon /> {label}
    </Button>
  );
  return (
    <>
      {button}
      <ConfirmDialog
        open={open}
        onOpenChange={(o) => {
          setOpen(o);
          if (!o) setRequeue(false);
        }}
        title={stopDialogTitle(executor, requeue)}
        description={
          <>
            {title ? <span className="block font-medium text-foreground anywhere">{title}</span> : null}
            <span className="mt-2 block">{"Runs on " + machineName(executor) + ". " + stopDialogDetail(requeue)}</span>
          </>
        }
        action={stopActionLabel(requeue)}
        destructive
        dismiss={STOP_DISMISS}
        success={(r) => stopSuccessMessage(r as JobStopResult)}
        invalidates={(p) => ["tasks", "jobs", "harness", "running", "runner", "events"].some((path) => p === path || p.startsWith(path + "?") || p.startsWith(path + "/"))}
        onConfirm={async () => {
          const r = await writes.stopJob(job, requeue);
          setAsked(true);
          await onDone?.();
          return r;
        }}
      >
        <label className="flex cursor-pointer items-center gap-2 text-[13px] text-muted-foreground">
          <input type="checkbox" checked={requeue} onChange={(e) => setRequeue(e.target.checked)} data-testid="job-stop-requeue" />
          {stopRequeueLabel()}
        </label>
      </ConfirmDialog>
    </>
  );
}
