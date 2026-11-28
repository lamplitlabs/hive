// The running jobs a machine card lists on the Machines page, each with the
// console's Stop (job-stop.tsx, POST jobs/{id}/stop). A pure module so node
// tests can load it straight: no React and no "@/" imports.
//
// The page asks the hub once for the runs in flight (GET harness/sessions
// ?status=running) and every card picks its own: a run's harness session
// names the machine it runs on (HIVE_EXECUTOR, "local" for this machine).

// One GET for every card: the runs in flight, any project, any machine.
export const MACHINE_JOBS_PATH = "harness/sessions?status=running&limit=200";

// A card lists this many; the rest are counted behind "See all runs".
export const MACHINE_JOBS_SHOWN = 5;

// The fields of a harness session (lib/api.ts HarnessSession) a row needs.
export interface MachineRun {
  id: string;
  job_id: string;
  job_title: string;
  agent: string;
  project: string;
  executor: string;
  status: string;
  started_at: string;
  cancel_requested_at?: string;
}

export interface MachineJob {
  job: string;
  title: string;
  agent: string;
  project: string;
  executor: string;
  since: string;
  session: string;
  stopping: boolean;
}

// The runner's name for this machine (hive_board::LOCAL_EXECUTOR).
const LOCAL = "local";

// The machine a run is on as a card's name: a run whose session did not
// record a machine ran here (sessions from before the executor column).
export function runMachine(executor: string | null | undefined): string {
  const e = (executor ?? "").trim();
  return e || LOCAL;
}

// Whether a card is this machine's: the runner's local row, whatever it is called.
export function isLocalMachine(m: { name: string; kind?: string }): boolean {
  return m.kind === "local" || m.name === LOCAL;
}

// The jobs running on machine `m`: one row per job (a job whose run was held
// and resumed keeps its newest session), the longest-running first. A
// session without a job (a chat's own run) has nothing the job Stop can reach.
export function machineJobs(runs: readonly MachineRun[] | null | undefined, m: { name: string; kind?: string }): MachineJob[] {
  const local = isLocalMachine(m);
  const byJob = new Map<string, MachineRun>();
  for (const r of runs ?? []) {
    if (!r || r.status !== "running" || !r.job_id) continue;
    const on = runMachine(r.executor);
    if (local ? on !== LOCAL && on !== m.name : on !== m.name) continue;
    const had = byJob.get(r.job_id);
    if (!had || (r.started_at || "") > (had.started_at || "")) byJob.set(r.job_id, r);
  }
  return [...byJob.values()]
    .sort((a, b) => (a.started_at || "").localeCompare(b.started_at || "") || a.job_id.localeCompare(b.job_id))
    .map((r) => ({
      job: r.job_id,
      title: (r.job_title || "").trim() || r.job_id,
      agent: r.agent || "",
      project: r.project || "",
      executor: runMachine(r.executor),
      since: r.started_at || "",
      session: r.id,
      stopping: !!r.cancel_requested_at,
    }));
}

// The rows a card shows and how many wait behind "See all runs".
export function shownJobs(jobs: readonly MachineJob[], limit = MACHINE_JOBS_SHOWN): { shown: MachineJob[]; more: number } {
  const n = Math.max(0, Math.floor(limit));
  return { shown: jobs.slice(0, n), more: Math.max(0, jobs.length - n) };
}

// The list's heading: what runs on the machine now, counted.
export function machineJobsHeading(count: number): string {
  if (count <= 0) return "Nothing running";
  return count === 1 ? "1 job running" : count + " jobs running";
}

// What the list holds for a screen reader, naming the machine.
export function machineJobsLabel(name: string): string {
  return "Jobs running on " + name;
}

// A row's line under the title: the agent and the project, as far as known.
export function machineJobMeta(j: Pick<MachineJob, "agent" | "project">, agentWord: (id: string) => string = (id) => id): string {
  return [j.agent ? agentWord(j.agent) : "", j.project].filter(Boolean).join(" · ");
}

// The "+N more" link under a full list.
export function moreJobsLabel(more: number): string {
  return more > 0 ? "+" + more + " more running" : "";
}
