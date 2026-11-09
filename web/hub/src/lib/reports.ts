// Work reports (the /reports page): every task the fleet finished, what
// landed for it and where (commit, pull request, release), and the evidence
// it left (the agent's report, screenshots, the checks that ran). The words
// and the rules live here, apart from the components, so node --test can
// check them without a browser (only type imports, no runtime ones).

import type { ChatArtifact, WorkReport, WorkReportJob, WorkReportLanding, WorkReportsPayload } from "./api";

export type ReportTone = "ok" | "warn" | "bad" | "neutral";

const many = (n: number, word: string) => n.toLocaleString("en-US") + " " + word + (n === 1 ? "" : "s");

// A worker's SUMMARY line without its key word and outcome code:
// "SUMMARY: done - built it" reads "built it".
export function cleanSummary(text: string): string {
  let t = (text || "").trim();
  t = t.replace(/^SUMMARY:\s*/i, "");
  t = t.replace(/^(done|nothing-to-do)\s*[-:–—]\s*/i, "");
  t = t.replace(/^(blocked|failed)\s*:\s*/i, "");
  return t.trim();
}

// A verify note as the owner reads it, with its tone: an exit 0 or a
// replayed verify passed, any other exit failed, nothing said is unknown.
export function verifyState(note: string): { text: string; tone: ReportTone } | null {
  const t = (note || "").trim();
  if (!t) return null;
  const exit = /verify exit (-?\d+)/i.exec(t);
  if (exit) {
    if (exit[1] === "0") return { text: "Checks passed", tone: "ok" };
    if (/did not run/i.test(t)) return { text: "Checks did not run (exit " + exit[1] + ")", tone: "warn" };
    return { text: "Checks failed (exit " + exit[1] + ")", tone: "bad" };
  }
  if (/^verified earlier/i.test(t)) return { text: "Checks passed earlier on the same tree", tone: "ok" };
  if (/timed out/i.test(t)) return { text: "Checks timed out", tone: "bad" };
  if (/no verify command/i.test(t)) return { text: "No check is set for this project", tone: "warn" };
  return { text: t, tone: "neutral" };
}

// Where a job's commits went, in words.
export function landingWords(l: WorkReportLanding | null | undefined): { label: string; tone: ReportTone } {
  if (!l) return { label: "Not landed", tone: "neutral" };
  const pr = l.number > 0 ? "Pull request #" + l.number : "Pull request";
  switch (l.action) {
    case "land.merged":
      return { label: pr + " merged", tone: "ok" };
    case "land.landed":
      return { label: "Merged into the default branch", tone: "ok" };
    case "land.pr":
      return { label: pr + " open for review", tone: "warn" };
    default:
      return { label: "Landing " + l.action.replace(/^land\./, ""), tone: "neutral" };
  }
}

export function landed(j: WorkReportJob): boolean {
  return !!j.landing && (j.landing.action === "land.merged" || j.landing.action === "land.landed");
}

// The task's release: the tags that carry its landed commits, oldest name
// first ("v4", "v5"); none while no release carries them.
export function taskReleases(t: WorkReport): string[] {
  const tags = new Set<string>();
  for (const j of t.jobs) if (j.release) tags.add(j.release);
  return [...tags].sort((a, b) => a.localeCompare(b, "en", { numeric: true }));
}

export function evidenceCount(t: WorkReport): number {
  return t.jobs.reduce((n, j) => n + (j.artifacts_total || 0), 0);
}

// One line for a task in the list: what landed, the release, the evidence.
export function taskLine(t: WorkReport): string {
  const bits: string[] = [];
  const done = t.jobs.filter(landed).length;
  const open = t.jobs.filter((j) => j.landing?.action === "land.pr").length;
  if (!t.jobs.length) bits.push("No jobs recorded");
  else if (done) bits.push(done === t.jobs.length ? (done === 1 ? "Landed" : "All " + done + " jobs landed") : done + " of " + many(t.jobs.length, "job") + " landed");
  else if (open) bits.push(open === 1 ? "Pull request open" : many(open, "pull request") + " open");
  else bits.push("Nothing landed");
  const rel = taskReleases(t);
  bits.push(rel.length ? "Released in " + rel.join(", ") : done ? "Not released yet" : "");
  const ev = evidenceCount(t);
  bits.push(ev ? many(ev, "evidence file") : "No evidence files");
  return bits.filter(Boolean).join(" · ");
}

// The tone of a task's headline badge: landed (and released) is ok, only an
// open pull request waits, nothing landed is neutral.
export function taskTone(t: WorkReport): ReportTone {
  if (t.jobs.some(landed)) return "ok";
  if (t.jobs.some((j) => j.landing?.action === "land.pr")) return "warn";
  return "neutral";
}

export function taskBadge(t: WorkReport): string {
  if (taskReleases(t).length) return "Released";
  if (t.jobs.some(landed)) return "Landed";
  if (t.jobs.some((j) => j.landing?.action === "land.pr")) return "In review";
  return "Done";
}

export type EvidenceKind = "page" | "picture" | "data" | "text" | "file";

// What a file in a job's folder is, so the page can show pictures as
// thumbnails and lead with the report.
export function evidenceKind(name: string): EvidenceKind {
  const n = name.toLowerCase();
  if (/\.html?$/.test(n)) return "page";
  if (/\.(png|jpe?g|gif|webp|avif|svg)$/.test(n)) return "picture";
  if (/\.(json|csv|tsv)$/.test(n)) return "data";
  if (/\.(md|txt|log)$/.test(n)) return "text";
  return "file";
}

// The evidence in reading order: report.html first, then the other pages,
// pictures (before before after, by name), data, text and the rest.
export function orderEvidence(files: ChatArtifact[]): ChatArtifact[] {
  const rank: Record<EvidenceKind, number> = { page: 1, picture: 2, data: 3, text: 4, file: 5 };
  const key = (f: ChatArtifact) => (f.name === "report.html" ? 0 : rank[evidenceKind(f.name)]);
  return [...files].sort((a, b) => key(a) - key(b) || a.name.localeCompare(b.name, "en", { numeric: true }));
}

// A link to the landed commit on the project's forge (GitHub, GitLab,
// Bitbucket), or null for a remote it cannot read.
export function commitURL(remote: string | null | undefined, sha: string): string | null {
  if (!remote || !/^[a-f0-9]{7,64}$/i.test(sha)) return null;
  let source = remote.trim();
  if (/[\s\\]/.test(source)) return null;
  const scp = /^(?:[^@/:]+@)?(github\.com|gitlab\.com|bitbucket\.org):(.+)$/i.exec(source);
  if (scp) source = "https://" + scp[1] + "/" + scp[2];
  let url: URL;
  try {
    url = new URL(source);
  } catch {
    return null;
  }
  const host = url.hostname.toLowerCase();
  if (!["github.com", "gitlab.com", "bitbucket.org"].includes(host)) return null;
  const path = url.pathname.replace(/\/+$/, "").replace(/\.git$/i, "");
  if (!/^\/[^/]+\/[^/]+/.test(path)) return null;
  const sep = host === "gitlab.com" ? "/-/commit/" : host === "bitbucket.org" ? "/commits/" : "/commit/";
  return "https://" + host + path + sep + sha;
}

// Filter by the words in a task's id, title, brief, jobs and release.
export function matchesReport(t: WorkReport, words: string): boolean {
  const q = (words || "").trim().toLowerCase();
  if (!q) return true;
  const hay = [t.id, t.title, t.brief, t.project, ...t.jobs.flatMap((j) => [j.id, j.title, j.agent, j.release, j.landing?.sha || "", j.summary])].join(" ").toLowerCase();
  return q.split(/\s+/).every((w) => hay.includes(w));
}

// The page's headline counters.
export function reportsSummary(d: WorkReportsPayload | undefined): { tasks: number; landed: number; released: number; evidence: number } {
  const rows = d?.reports ?? [];
  return {
    tasks: d?.scope.total ?? 0,
    landed: rows.filter((t) => t.jobs.some(landed)).length,
    released: rows.filter((t) => taskReleases(t).length > 0).length,
    evidence: rows.filter((t) => evidenceCount(t) > 0).length,
  };
}

// What the page says when there is nothing to list: `shown` is how many
// reports are left after the search box filtered them.
export function reportsEmpty(d: WorkReportsPayload, shown: number): { title: string; body: string } | null {
  if (shown > 0) return null;
  if (d.reports.length) return { title: "No reports match", body: "No done task here has those words in its title, brief, jobs or release." };
  const where = d.scope.project ? " in " + d.scope.project : "";
  return {
    title: "No finished work yet",
    body: "No task" + where + " is done yet. When agents finish a task, its report shows up here: what landed, in which release, and the evidence they left.",
  };
}
