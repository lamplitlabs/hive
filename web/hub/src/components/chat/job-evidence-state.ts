// The evidence a fleet job left: the screenshots and reports its agent
// saved in the job's artifacts folder, shown beside a task the way a
// conversation's Artifacts tab shows its own. The words and the rules live
// here, apart from the components, so they can be tested without a browser
// (node --test loads this file as is: only type imports, no runtime ones).
//
// The task view opens one canvas tab per job ("evidence:<jobId>"); the
// tab lists the files and shows the selected one (images inline, html in a
// sandboxed frame, text as text) through the same artifact components a
// conversation uses, with the job's folder as the source (lib/api's
// ArtifactSource { kind: "job", id } and jobArtifactHref).

import type { ChatArtifact, TaskJob } from "@/lib/api";
import type { CanvasTab } from "@/lib/canvas";

// How many files the job's folder holds, as the task detail reports it
// (TaskJob.artifacts): 0 for a job the hub did not count - an older hub, or
// a job without a folder.
export function jobEvidenceCount(
  j: Pick<TaskJob, "artifacts"> | undefined,
): number {
  const n = j?.artifacts;
  return typeof n === "number" && Number.isFinite(n) && n > 0
    ? Math.floor(n)
    : 0;
}

export function hasJobEvidence(
  j: Pick<TaskJob, "artifacts"> | undefined,
): boolean {
  return jobEvidenceCount(j) > 0;
}

// "3 files", "1 file": the control's words in a step row or under a report.
export function jobEvidenceWord(n: number): string {
  return n.toLocaleString("en-US") + (n === 1 ? " file" : " files");
}

// The canvas tab for a job's evidence: one per job, keyed by the job id so
// two steps' folders sit side by side (lib/canvas's tabId: "evidence:<id>");
// the label carries the step's title, cut short for the tab strip.
export function jobEvidenceTab(j: Pick<TaskJob, "id" | "title">): CanvasTab {
  return {
    id: "evidence:" + j.id,
    kind: "evidence",
    key: j.id,
    label: jobEvidenceLabel(j.title),
  };
}

export function jobEvidenceLabel(title: string): string {
  const t = shortTitle(title);
  return t ? "Evidence: " + t : "Evidence";
}

const TITLE_MAX = 28;

export function shortTitle(title: string): string {
  const t = (title || "").trim();
  if (t.length <= TITLE_MAX) return t;
  return t.slice(0, TITLE_MAX - 1).trimEnd() + "\u2026";
}

// The tooltip of the evidence control.
export function jobEvidenceTip(j: Pick<TaskJob, "id" | "artifacts">): string {
  return (
    jobEvidenceWord(jobEvidenceCount(j)) +
    " the agent left as evidence for job " +
    j.id +
    ": screenshots, reports. Open them."
  );
}

// The file the evidence tab shows first: the newest image if there is one
// (a screenshot is what the owner came for), else the newest file. The
// listing comes newest first from the hub.
export function defaultEvidenceFile(artifacts: ChatArtifact[]): string {
  if (!artifacts.length) return "";
  const image = artifacts.find((a) => isImageName(a.name));
  return (image ?? artifacts[0]).name;
}

const IMAGE_EXT = new Set(["png", "jpg", "jpeg", "gif", "webp", "svg", "avif"]);

export function isImageName(name: string): boolean {
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return false;
  return IMAGE_EXT.has(name.slice(dot + 1).toLowerCase());
}

// The selected file after the listing refreshed: kept while it is still
// there, else the default pick.
export function keepSelection(
  selected: string,
  artifacts: ChatArtifact[],
): string {
  if (selected && artifacts.some((a) => a.name === selected)) return selected;
  return defaultEvidenceFile(artifacts);
}

// What the tab says when the folder turned out empty or the listing failed.
export function evidenceEmptyText(error: string): string {
  if (error) return "The files did not load (" + error + ").";
  return "This step left no files. Screenshots and reports an agent saves in its artifacts folder show up here.";
}

// The before/after page an agent builds with `pulse report` from the
// report.json and screenshots it saved in the same folder: each visible
// change as it was and as it is now. It is the file the owner opens first,
// so the task, the session and the run link it on its own.
export const REPORT_PAGE = "report.html";

export const REPORT_WORD = "See the report";

export function hasReportPage(
  artifacts: Pick<ChatArtifact, "name">[] | undefined,
): boolean {
  return !!artifacts?.some((a) => a.name === REPORT_PAGE);
}

// The tooltip of a report link: whose report it is and where it opens.
export function reportTip(of: "job" | "session" | "run"): string {
  return (
    "The before/after report this " +
    of +
    "'s agent built: each change as it was and as it is now. Opens in a new tab."
  );
}
