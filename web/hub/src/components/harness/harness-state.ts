import type { HarnessSessionStatus, HarnessStep, HarnessStepKind } from "@/lib/api";

// Pure helpers for the Runs page: how the harness statuses fold into the
// four filter chips, which tool icon a step gets, how a command pairs with
// the output that came back, and the one-line summary a folded step shows.
// No React, no runtime imports - the node test beside this file loads it.

export type RunGroup = "running" | "finished" | "stopped" | "failed";

// Every harness status belongs to exactly one chip. "Stopped early" is a
// run that ended without handing in its work for a reason that is nobody's
// fault (a limit, a cancel, a restart); "Failed" is a run the model side
// broke (provider error, sign-in, wrong format).
export const RUN_GROUPS: { key: RunGroup; label: string; statuses: HarnessSessionStatus[] }[] = [
  { key: "running", label: "Running", statuses: ["running"] },
  { key: "finished", label: "Finished", statuses: ["submitted"] },
  { key: "stopped", label: "Stopped early", statuses: ["step_limit", "time_limit", "cancelled", "lost", "held", "resumed"] },
  { key: "failed", label: "Failed", statuses: ["format_error", "provider_error", "not_logged_in"] },
];

const GROUP_KEYS = new Set<string>(RUN_GROUPS.map((g) => g.key));

export function groupOf(status: string): RunGroup {
  for (const g of RUN_GROUPS) if ((g.statuses as string[]).includes(status)) return g.key;
  return "stopped";
}

export function isGroup(value: string): value is RunGroup {
  return GROUP_KEYS.has(value);
}

// Does a run with this status belong under the `status` param?
export function matchesStatus(status: string, param: string): boolean {
  const v = (param || "").trim();
  if (!v) return true;
  if (isGroup(v)) return groupOf(status) === v;
  return status === v;
}

// A chip lights up for its own key and for any exact status it holds.
export function chipActive(param: string, group: RunGroup): boolean {
  const v = (param || "").trim();
  if (!v) return false;
  if (v === group) return true;
  if (isGroup(v)) return false;
  return groupOf(v) === group;
}

// Per-chip counts over the rows at hand.
export function groupCounts(statuses: readonly string[]): Record<RunGroup, number> {
  const out: Record<RunGroup, number> = { running: 0, finished: 0, stopped: 0, failed: 0 };
  for (const s of statuses) out[groupOf(s)] += 1;
  return out;
}

// The chip a run sits under: its status, or while it runs, "running"
// whatever the exit field says.
export function chipOf(run: { status: string }): RunGroup {
  return groupOf((run.status || "").trim());
}

// The four chips with their counts over the runs at hand, in the order the
// page shows them. An empty chip stays (the component dims it) so the owner
// sees what the filter would show.
export function statusChips(runs: readonly { status: string }[]): { key: RunGroup; label: string; count: number }[] {
  const counts = groupCounts(runs.map((r) => r.status));
  return RUN_GROUPS.map((g) => ({ key: g.key, label: g.label, count: counts[g.key] }));
}

// What to ask the API for under this `status` param: the hub filters on one
// exact status, so a chip that holds a single status (Running, Finished)
// asks for it, an exact status asks for itself, and a chip holding several
// (Stopped early, Failed) asks for everything and the page filters the rows
// with matchesStatus.
export function apiStatus(param: string): string {
  const v = (param || "").trim();
  if (!v) return "";
  if (!isGroup(v)) return v;
  const g = RUN_GROUPS.find((x) => x.key === v);
  return g && g.statuses.length === 1 ? g.statuses[0] : "";
}

// The tool family a step belongs to, for its icon: a terminal for shell
// work, a pen for edits, a page for reads, a globe for the web, a magnifier
// for searches, a speech bubble for the model's own words. Anything else
// keeps the step kind's own icon (null here).
export type StepTool = "shell" | "edit" | "read" | "web" | "search" | "model";

const TOOL_OF: Partial<Record<HarnessStepKind, StepTool>> = {
  command: "shell",
  process: "shell",
  observation: "shell",
  edit: "edit",
  write: "edit",
  read: "read",
  fetch: "web",
  search: "web",
  grep: "search",
  glob: "search",
  assistant: "model",
};

export function stepTool(kind: string): StepTool | null {
  return TOOL_OF[kind as HarnessStepKind] ?? null;
}

// One row of the timeline: a step and, when a command or tool call was
// followed by what came back, that observation folded under it.
export interface StepEntry {
  step: HarnessStep;
  output?: HarnessStep;
}

const CALL_KINDS: readonly string[] = ["command", "read", "grep", "glob", "edit", "write", "mcp", "skill", "fetch", "search", "process", "memory", "task", "hook", "subagent"];

export function pairSteps(steps: readonly HarnessStep[]): StepEntry[] {
  const out: StepEntry[] = [];
  for (const s of steps) {
    const last = out[out.length - 1];
    if (s.kind === "observation" && last && !last.output && CALL_KINDS.includes(last.step.kind) && (last.step.depth ?? 0) === (s.depth ?? 0)) {
      last.output = s;
      continue;
    }
    out.push({ step: s });
  }
  return out;
}

function firstLine(text: string, max = 160): string {
  const line = (text || "").split("\n").find((l) => l.trim()) || "";
  const t = line.trim();
  return t.length > max ? t.slice(0, max - 1).trimEnd() + "…" : t;
}

// What a folded step shows beside its icon: the command for a command, the
// request for a tool, the first line for the model's text.
export function stepSummary(s: Pick<HarnessStep, "kind" | "command" | "content" | "title">): string {
  if (s.kind === "command") return firstLine(s.command || s.content);
  if (CALL_KINDS.includes(s.kind)) return firstLine(s.command || s.content);
  if (s.kind === "plan") return firstLine(s.content);
  return firstLine(s.content || s.command);
}

// The raw word StatusBadge maps onto its icon and colour for each harness
// status. The badge's own table knows most of them; the rest borrow a word
// of the right kind: a held run waits for the runner, not the owner, so it
// is paused rather than marigold; a resumed run has ended (carried on by a
// later run), so it is quiet information, not a spinner; a missing sign-in
// is a failure of the run.
const BADGE_WORD: Record<string, string> = {
  running: "running",
  cancelling: "cancelling",
  submitted: "submitted",
  step_limit: "step_limit",
  time_limit: "time_limit",
  format_error: "format_errors",
  provider_error: "provider_error",
  not_logged_in: "error",
  cancelled: "cancelled",
  lost: "lost",
  held: "paused",
  resumed: "superseded",
};

export function badgeStatus(key: string): string {
  return BADGE_WORD[key] ?? key;
}

// A live row's time so far, ticking every second: "42s", "3m 07s",
// "1h 02m 05s"; past two days the seconds stop mattering, so it reads in
// days ("2 days 5 h", then "56 days" from a week on) instead of "1363h 24m
// 40s". Empty without a start, never negative when the browser's clock
// lags the server's.
export function elapsedWords(startedAt: string | undefined, now: number): string {
  const start = startedAt ? Date.parse(startedAt) : NaN;
  if (!Number.isFinite(start) || !start) return "";
  const s = Math.max(0, Math.floor((now - start) / 1000));
  const two = (n: number) => (n < 10 ? "0" + n : String(n));
  if (s < 60) return s + "s";
  const m = Math.floor(s / 60);
  if (m < 60) return m + "m " + two(s % 60) + "s";
  const h = Math.floor(m / 60);
  if (h < 48) return h + "h " + two(m % 60) + "m " + two(s % 60) + "s";
  return daysWords(h);
}

// Whole hours past two days as days: "2 days 5 h" within the first week,
// "56 days" after it.
function daysWords(hours: number): string {
  const d = Math.floor(hours / 24);
  const rest = hours % 24;
  return d + " days" + (d < 7 && rest ? " " + rest + " h" : "");
}

// Milliseconds as the owner reads them: "420 ms", "3 s", "2 min 5 s",
// "1 h 2 min", and in days past two days.
export function msText(ms: number): string {
  if (!ms || ms < 0) return "";
  if (ms < 1000) return Math.round(ms) + " ms";
  const sec = Math.round(ms / 1000);
  if (sec < 60) return sec + " s";
  const min = Math.floor(sec / 60);
  const rest = sec % 60;
  if (min < 60) return rest ? min + " min " + rest + " s" : min + " min";
  const h = Math.floor(min / 60);
  if (h < 48) return h + " h " + (min % 60) + " min";
  return daysWords(h);
}

// Seconds between two timestamps; a run with no end ends now. Parsing is
// kept to Date.parse so this stays a plain module.
export function runSeconds(start: string | undefined, end: string | undefined, now: number): number {
  const s = start ? Date.parse(start) : NaN;
  if (!Number.isFinite(s) || !s) return 0;
  const e = end ? Date.parse(end) : NaN;
  return Math.max(1, ((Number.isFinite(e) && e ? e : now) - s) / 1000);
}

// A step whose body should start unfolded: the model's own words and how
// the run ended read in order; tool calls fold until asked.
export function openByDefault(s: Pick<HarnessStep, "kind">): boolean {
  return s.kind === "assistant" || s.kind === "error" || s.kind === "exit" || s.kind === "note";
}
