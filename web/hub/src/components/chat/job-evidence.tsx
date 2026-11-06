"use client";

// The small "N files" control beside a step or under a worker's report:
// the screenshots and reports the job left in its artifacts folder, the
// owner's evidence for what is being landed. Where a canvas is near (the
// task view) it opens the job's evidence tab; elsewhere (the project's
// chat) it is a link to the newest file, in a new tab. The words and rules
// are in job-evidence-state.ts.

import { ImagesIcon } from "lucide-react";
import type { ReactNode } from "react";
import {
  hasJobEvidence,
  jobEvidenceCount,
  jobEvidenceTip,
  jobEvidenceWord,
} from "@/components/chat/job-evidence-state";
import {
  apiHref,
  jobArtifactHref,
  jobArtifactsPath,
  type JobArtifactsResponse,
  type TaskJob,
} from "@/lib/api";
import { useApi } from "@/lib/use-api";
import { cn } from "@/lib/utils";

const CHIP =
  "inline-flex shrink-0 items-center gap-1 rounded-md px-1.5 py-0.5 text-xs text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground";
const LINK =
  "inline-flex items-center gap-1 font-medium text-primary underline-offset-2 hover:underline";

export function JobEvidence({
  job,
  onOpen,
  variant = "chip",
  className,
}: {
  job: TaskJob | undefined;
  /** Opens the job's evidence tab on the canvas; without it the control links to the newest file. */
  onOpen?: (job: TaskJob) => void;
  variant?: "chip" | "link";
  className?: string;
}) {
  if (!job || !hasJobEvidence(job)) return null;
  const n = jobEvidenceCount(job);
  const cls = cn(variant === "link" ? LINK : CHIP, className);
  const inner = (
    <>
      <ImagesIcon
        className={variant === "link" ? "size-3.5" : "size-3"}
        aria-hidden
      />
      <span className="tnum">{jobEvidenceWord(n)}</span>
    </>
  );
  if (onOpen)
    return (
      <button
        type="button"
        onClick={() => onOpen(job)}
        className={cls}
        title={jobEvidenceTip(job)}
        aria-label={jobEvidenceWord(n) + " of evidence for " + job.id}
        data-evidence-for={job.id}
      >
        {inner}
      </button>
    );
  return (
    <EvidenceLink job={job} className={cls} tip={jobEvidenceTip(job)}>
      {inner}
    </EvidenceLink>
  );
}

// No canvas nearby: the control links to the job's newest file in a new
// tab. The listing is read once to learn the name; until it comes the
// control points at the listing itself, which is still the right folder.
function EvidenceLink({
  job,
  className,
  tip,
  children,
}: {
  job: TaskJob;
  className: string;
  tip: string;
  children: ReactNode;
}) {
  const { data } = useApi<JobArtifactsResponse>(jobArtifactsPath(job.id), {
    refresh: false,
  });
  const first = data?.artifacts?.[0]?.name;
  const href = first
    ? jobArtifactHref(job.id, first)
    : apiHref(jobArtifactsPath(job.id));
  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className={className}
      title={tip + (first ? " Opens " + first + "." : "")}
      data-evidence-for={job.id}
    >
      {children}
    </a>
  );
}
