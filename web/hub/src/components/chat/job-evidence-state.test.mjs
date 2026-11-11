import test from "node:test";
import assert from "node:assert/strict";

import {
  defaultEvidenceFile,
  evidenceEmptyText,
  hasJobEvidence,
  hasReportPage,
  isImageName,
  jobEvidenceCount,
  jobEvidenceLabel,
  jobEvidenceTab,
  jobEvidenceTip,
  jobEvidenceWord,
  keepSelection,
  REPORT_PAGE,
  reportJob,
  REPORT_WORD,
  reportTip,
  shortTitle,
} from "./job-evidence-state.ts";

test("a job's evidence count comes from the task detail and tolerates its absence", () => {
  assert.equal(jobEvidenceCount(undefined), 0);
  assert.equal(jobEvidenceCount({}), 0);
  assert.equal(jobEvidenceCount({ artifacts: 0 }), 0);
  assert.equal(jobEvidenceCount({ artifacts: -2 }), 0);
  assert.equal(jobEvidenceCount({ artifacts: Number.NaN }), 0);
  assert.equal(jobEvidenceCount({ artifacts: 2 }), 2);
  assert.equal(jobEvidenceCount({ artifacts: 2.7 }), 2);
  assert.equal(hasJobEvidence({ artifacts: 2 }), true);
  assert.equal(hasJobEvidence({ artifacts: 0 }), false);
  assert.equal(hasJobEvidence(undefined), false);
});

test("the control says how many files, in the owner's words", () => {
  assert.equal(jobEvidenceWord(1), "1 file");
  assert.equal(jobEvidenceWord(2), "2 files");
  assert.equal(jobEvidenceWord(0), "0 files");
  assert.equal(jobEvidenceWord(1200), "1,200 files");
  assert.match(jobEvidenceTip({ id: "job-1", artifacts: 2 }), /^2 files .*job-1/);
});

test("one canvas tab per job, keyed by the job id and labelled by its title", () => {
  const t = jobEvidenceTab({ id: "job-20261103T020633Z-9f1c2a", title: "Hub: show job artifacts" });
  assert.equal(t.kind, "evidence");
  assert.equal(t.key, "job-20261103T020633Z-9f1c2a");
  assert.equal(t.id, "evidence:job-20261103T020633Z-9f1c2a");
  assert.equal(t.label, "Evidence: Hub: show job artifacts");
  assert.equal(jobEvidenceTab({ id: "j", title: "" }).label, "Evidence");
  assert.equal(
    jobEvidenceTab({ id: "a", title: "x" }).id !== jobEvidenceTab({ id: "b", title: "x" }).id,
    true,
  );
});

test("long step titles are cut short for the tab strip", () => {
  assert.equal(shortTitle("  short  "), "short");
  const long = "Await the price lookup before reading the total";
  const cut = shortTitle(long);
  assert.ok(cut.length <= 28, cut);
  assert.ok(cut.endsWith("\u2026"), cut);
  assert.equal(jobEvidenceLabel(long), "Evidence: " + cut);
});

test("the tab opens on the newest image, else the newest file", () => {
  const files = [
    { name: "report.md", size: 10, modified: "2026-11-03T02:40:00Z" },
    { name: "task-view.png", size: 1000, modified: "2026-11-03T02:39:00Z" },
    { name: "older.PNG", size: 900, modified: "2026-11-03T02:38:00Z" },
  ];
  assert.equal(defaultEvidenceFile(files), "task-view.png");
  assert.equal(defaultEvidenceFile(files.slice(0, 1)), "report.md");
  assert.equal(defaultEvidenceFile([]), "");
  // The report comes first: it holds the pictures and what they mean.
  assert.equal(defaultEvidenceFile([...files, { name: "report.html", size: 5, modified: "" }]), "report.html");
  assert.equal(isImageName("a.webp"), true);
  assert.equal(isImageName("a.SVG"), true);
  assert.equal(isImageName(".png"), false);
  assert.equal(isImageName("notes.html"), false);
});

test("the selection survives a refresh while its file is still listed", () => {
  const files = [
    { name: "report.md", size: 10, modified: "" },
    { name: "shot.png", size: 1, modified: "" },
  ];
  assert.equal(keepSelection("report.md", files), "report.md");
  assert.equal(keepSelection("gone.png", files), "shot.png");
  assert.equal(keepSelection("", files), "shot.png");
  assert.equal(keepSelection("report.md", []), "");
});

test("an empty folder and a failed listing read differently", () => {
  assert.match(evidenceEmptyText(""), /left no files/);
  assert.match(evidenceEmptyText("HTTP 404"), /did not load \(HTTP 404\)/);
});

test("the report link shows only for a folder that holds the before/after page", () => {
  assert.equal(REPORT_PAGE, "report.html");
  assert.equal(REPORT_WORD, "See the report");
  assert.equal(hasReportPage(undefined), false);
  assert.equal(hasReportPage([]), false);
  // The manifest and the pictures alone are not the page.
  assert.equal(hasReportPage([{ name: "report.json" }, { name: "before-home.png" }]), false);
  assert.equal(hasReportPage([{ name: "after.png" }, { name: "report.html" }]), true);
  assert.match(reportTip("session"), /^The report Hive built for this session: /);
  assert.match(reportTip("run"), /Opens in a new tab\.$/);
});

test("a finished task shows the report of the step that did the work", () => {
  const step = (id, status, commits) => ({ id, status, commits });
  // The work made the commits; the rebase after it made none of its own.
  assert.equal(reportJob([step("work", "done", 3), step("rebase", "done", 0)]).id, "work");
  assert.equal(reportJob([step("a", "failed", 0), step("b", "done", 1), step("c", "done", 2)]).id, "c");
  // Equals: the earliest. No commits at all: the first done step.
  assert.equal(reportJob([step("a", "done", 1), step("b", "done", 1)]).id, "a");
  assert.equal(reportJob([step("a", "done", null), step("b", "done", undefined)]).id, "a");
  // Nothing done: the latest that ended; still running: none.
  assert.equal(reportJob([step("a", "failed", 0), step("b", "cancelled", 0)]).id, "b");
  assert.equal(reportJob([step("a", "leased", 0)]), undefined);
  assert.equal(reportJob([]), undefined);
});
