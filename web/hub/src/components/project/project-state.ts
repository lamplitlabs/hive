// The project page's pure logic: what the header says about the repository
// and the project's health, which agents stand in the header, and how the
// Overview's rows are picked. No React and no "@/" imports, so node --test
// covers it (project-state.test.mjs).

export type RepoHost = "github" | "gitlab" | "bitbucket" | "git" | "local";

export interface RepoChip {
  host: RepoHost;
  // "owner/name" for a hosted remote, the folder's last segment otherwise.
  name: string;
  branch: string;
  // Where the chip points: the remote's web page, or "" for a local folder.
  href: string;
}

// Read a git remote ("git@github.com:owner/app.git", "https://github.com/owner/app")
// into the host and the owner/name pair. Anything else is a plain git remote.
export function parseRemote(remote: string | null | undefined): { host: RepoHost; path: string; web: string } {
  const r = (remote || "").trim();
  if (!r) return { host: "local", path: "", web: "" };
  const m = /^(?:(?:ssh|https?|git):\/\/)?(?:[^@/]+@)?([^/:]+)[/:](.+?)(?:\.git)?\/?$/.exec(r);
  if (!m) return { host: "git", path: r, web: "" };
  const hostName = m[1].toLowerCase();
  const path = m[2];
  const host: RepoHost = hostName.endsWith("github.com") ? "github" : hostName.endsWith("gitlab.com") ? "gitlab" : hostName.endsWith("bitbucket.org") ? "bitbucket" : "git";
  const web = host === "git" ? (/^https?:\/\//.test(r) ? r.replace(/\.git$/, "") : "") : "https://" + hostName + "/" + path;
  return { host, path, web };
}

// The repository chip: a hosted remote shows owner/name and its logo, a
// local checkout shows the folder's name.
export function repoChip(info: { remote?: string | null; repo?: string | null; repo_name?: string | null; branch?: string | null } | null | undefined, card?: { path?: string | null; branch?: string | null } | null): RepoChip | null {
  if (!info && !card) return null;
  const remote = parseRemote(info?.remote);
  const folder = (info?.repo || card?.path || "").replace(/\/+$/, "");
  const folderName = info?.repo_name || folder.split("/").filter(Boolean).pop() || "";
  const branch = (info?.branch || card?.branch || "").trim();
  if (remote.host !== "local") return { host: remote.host, name: remote.path || folderName, branch, href: remote.web };
  if (!folderName && !branch) return null;
  return { host: "local", name: folderName, branch, href: "" };
}

export type HealthKind = "missing" | "stopped" | "failing" | "waits" | "working" | "step" | "quiet" | "no-board";

export interface Health {
  kind: HealthKind;
  // The status word the StatusBadge folds into its icon and tint.
  status: string;
  // The badge's text.
  label: string;
}

// How many failed jobs the header fetches to colour its badge, and the
// window that counts: a job that failed in the last day. The store keeps
// every job still marked failed, of every age - hundreds on a project that
// has run for months, most of them needing nothing - so the total alone
// would keep the badge red for good. Older failures stay on the Runs page.
export const RECENT_FAILURES_PAGE = 20;
const RECENT_FAILURE_MS = 24 * 60 * 60 * 1000;

export function recentFailures(jobs: { finished_at?: string | null }[] | null | undefined, now: number): number {
  return (jobs || []).filter((j) => {
    const t = Date.parse(j.finished_at || "");
    return Number.isFinite(t) && now - t < RECENT_FAILURE_MS;
  }).length;
}

// The one badge in the header: the project's state in a word, the same
// order the project card on Home uses, plus "no board" and jobs that failed
// in the last day (`failed`, from recentFailures).
export function projectHealth(p: {
  present?: boolean;
  missing?: boolean;
  error?: string;
  stop_present?: boolean;
  board_present?: boolean;
  claims_active?: number;
}, opts: { forYou?: number; step?: string | null; failed?: number } = {}): Health {
  // "lost" is the status word the badge reads as failed (red); "missing" has no mapping.
  if (p.error || p.missing || p.present === false) return { kind: "missing", status: "lost", label: "Missing on disk" };
  if (p.board_present === false) return { kind: "no-board", status: "warning", label: "No board" };
  if (p.stop_present) return { kind: "stopped", status: "stopped", label: "Self-improvement stopped" };
  if (opts.failed) return { kind: "failing", status: "failed", label: (opts.failed === 1 ? "1 job" : (opts.failed >= RECENT_FAILURES_PAGE ? RECENT_FAILURES_PAGE + "+" : opts.failed) + " jobs") + " failed today" };
  if (opts.forYou) return { kind: "waits", status: "waiting", label: opts.forYou === 1 ? "1 thing waits for you" : opts.forYou + " things wait for you" };
  if ((p.claims_active ?? 0) > 0) return { kind: "working", status: "running", label: "Agents are working" };
  if (opts.step) return { kind: "step", status: "pending", label: "Next step: " + opts.step };
  return { kind: "quiet", status: "ok", label: "All quiet" };
}

// The agents who post here, most recent first, de-duplicated by name; the
// header shows `cap` faces and counts the rest.
export function posterNames(agents: { name?: string | null; agent?: string; session?: string; last?: string; posts?: number }[] | null | undefined, cap = 5): { shown: string[]; more: number } {
  const seen = new Set<string>();
  const sorted = [...(agents || [])].sort((a, b) => (b.last || "").localeCompare(a.last || "") || (b.posts || 0) - (a.posts || 0));
  const names: string[] = [];
  for (const a of sorted) {
    const n = (a.agent || a.name || a.session || "").trim();
    if (!n || n === "human" || n === "owner" || seen.has(n)) continue;
    seen.add(n);
    names.push(n);
  }
  return { shown: names.slice(0, cap), more: Math.max(0, names.length - cap) };
}

// Pick the open threads the Overview shows: the ones with activity first,
// capped at `limit`.
export function topThreads<T extends { last?: string; posts?: number; open_questions?: number; open_handoffs?: number }>(rows: T[], limit = 5): T[] {
  return [...rows]
    .sort((a, b) => (b.open_questions || 0) + (b.open_handoffs || 0) - ((a.open_questions || 0) + (a.open_handoffs || 0)) || (b.last || "").localeCompare(a.last || ""))
    .slice(0, limit);
}

// Three lines for the Health panel, each a plain phrase.
export function healthLines(input: {
  branch?: string;
  lastLanded?: string | null; // ISO date of the last code delivery, or null
  machines?: number | null; // machines that fit the project, null while unknown
  needsWords?: boolean; // whether the project names what it needs
  delivered?: number | string | null; // code deliveries in the window
}): { repository: string; machines: string; delivered: string } {
  const branch = input.branch ? "On " + input.branch : "Branch unknown";
  const repository = input.lastLanded ? branch + ", last landed " + input.lastLanded : branch + ", nothing landed yet";
  const machines =
    input.machines === null || input.machines === undefined
      ? "Machines: checking"
      : input.machines === 0
        ? input.needsWords
          ? "No machine fits its needs"
          : "No machine runs it"
        : input.machines === 1
          ? "Runs on 1 machine"
          : "Runs on " + input.machines + " machines";
  const n = input.delivered;
  const delivered = n === null || n === undefined || n === "…" ? "Code delivered: counting" : n === "—" ? "Code delivered: not counted yet" : n === 0 ? "No code delivered yet" : n + (n === 1 ? " code delivery" : " code deliveries");
  return { repository, machines, delivered };
}

// The project tabs' icons live in page.tsx; this says which tabs carry a
// count and where the count comes from, so the strip and its labels agree.
export type CountedTab = "for-you" | "work" | "board";

export function tabCountLabel(id: CountedTab, n: number): string {
  if (id === "for-you") return n === 1 ? "1 thing waits for you" : n + " things wait for you";
  if (id === "work") return n === 1 ? "1 open task" : n + " open tasks";
  return n === 1 ? "1 open thread" : n + " open threads";
}
