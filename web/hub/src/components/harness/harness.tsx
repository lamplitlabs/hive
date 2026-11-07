"use client";

import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import {
  ActivityIcon,
  BanIcon,
  BookOpenIcon,
  BotIcon,
  BrainIcon,
  ChevronRightIcon,
  CircleAlertIcon,
  CircleCheckIcon,
  CircleHelpIcon,
  CircleIcon,
  CircleXIcon,
  ClipboardIcon,
  ClipboardListIcon,
  CopyIcon,
  CpuIcon,
  DownloadIcon,
  FileCodeIcon,
  FilePenIcon,
  FilePlusIcon,
  FileTextIcon,
  FilesIcon,
  GitForkIcon,
  GlobeIcon,
  InfoIcon,
  ListChecksIcon,
  LoaderCircleIcon,
  MessageSquareIcon,
  PlayIcon,
  PlugIcon,
  ScrollTextIcon,
  SearchIcon,
  SlidersHorizontalIcon,
  TelescopeIcon,
  TerminalIcon,
  TimerOffIcon,
  TriangleAlertIcon,
  WebhookIcon,
  XIcon,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { ProjectAvatar } from "@/components/pulse/avatar";
import { ConfirmDialog } from "@/components/pulse/confirm";
import { DataTable, type Column } from "@/components/pulse/data-table";
import { FilterChip } from "@/components/pulse/filter-chip";
import { ListRow, ListRows } from "@/components/pulse/list-row";
import { BrandLogo, logoFor } from "@/components/pulse/logo";
import { Markdown } from "@/components/pulse/markdown";
import { PageHeader } from "@/components/pulse/page-header";
import { Disclosure, EmptyState, ErrorNotice, Kv, Notice, SkeletonRows } from "@/components/pulse/panel";
import { SectionHeading } from "@/components/pulse/section-heading";
import { SidePanel, useOpenItem } from "@/components/pulse/side-panel";
import { StatusBadge, StatusIcon } from "@/components/pulse/status-badge";
import { Avatar, When } from "@/components/pulse/who";
import type { HarnessModel, HarnessModelsResponse, HarnessSession, HarnessSessionResponse, HarnessSessionStatus, HarnessSessionsResponse, HarnessStep, ProjectsPayload } from "@/lib/api";
import { api, apiHref, enc, errorText, projectLabel, query, writes } from "@/lib/api";
import { num, parseDate, plural, sentence, span } from "@/lib/format";
import { agentName, statusWord } from "@/lib/words";
import { hrefs } from "@/lib/nav";
import { useApi, useLiveEvents, useNow } from "@/lib/use-api";
import { cn } from "@/lib/utils";
import { apiStatus, badgeStatus, chipActive, elapsedWords, matchesStatus, msText, openByDefault, pairSteps, RUN_GROUPS, runSeconds, statusChips, stepSummary, stepTool, type StepEntry, type StepTool } from "./harness-state";

// Runs: every agent run as the harness saw it. The table says what each
// run was asked and how it ended; a run in flight sits above it as a live
// row with a ticking clock. Picking a run opens the transcript in the side
// panel (a bottom sheet on a phone): the steps as folding rows, what the
// model said, the commands it ran, what came back, and how it ended.

// Run states statusWord does not know yet (see the report's requests).
const RUN_WORD: Partial<Record<HarnessSessionStatus, string>> = { format_error: "Kept answering in the wrong format", not_logged_in: "Not signed in" };

const STATUSES: { value: string; label: string }[] = [{ value: "all", label: "All statuses" }, ...(["running", "submitted", "step_limit", "time_limit", "format_error", "provider_error", "not_logged_in", "cancelled", "lost", "held", "resumed"] as HarnessSessionStatus[]).map((s) => ({ value: s, label: runWord(s) }))];

function runWord(status: string): string {
  return RUN_WORD[status as HarnessSessionStatus] ?? statusWord(status);
}

function compact(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(n >= 10_000_000 ? 0 : 1).replace(/\.0$/, "") + "m";
  if (n >= 1_000) return (n / 1_000).toFixed(n >= 10_000 ? 0 : 1).replace(/\.0$/, "") + "k";
  return num(n);
}

function exitKey(session: Pick<HarnessSession, "status" | "exit_status">): string {
  return session.status === "running" ? "running" : session.exit_status || session.status;
}

type RunTone = "run" | "ok" | "warn" | "bad" | "neutral";

const RUN_ICON_TONE: Record<RunTone, string> = { run: "text-primary", ok: "text-ok", warn: "text-warn", bad: "text-bad", neutral: "text-muted-foreground" };

function runState(session: Pick<HarnessSession, "status" | "exit_status" | "cancel_requested_at">): { word: string; tone: RunTone; Icon: React.ComponentType<{ className?: string }>; spin?: boolean; raw: string } {
  const key = exitKey(session);
  if (key === "running" && session.cancel_requested_at) return { word: "Cancelling", tone: "warn", Icon: LoaderCircleIcon, spin: true, raw: "cancelling" };
  const word = runWord(key);
  switch (key) {
    case "running":
      return { word, tone: "run", Icon: LoaderCircleIcon, spin: true, raw: key };
    case "submitted":
      return { word, tone: "ok", Icon: CircleCheckIcon, raw: key };
    case "step_limit":
    case "format_error":
      return { word, tone: "warn", Icon: CircleAlertIcon, raw: key };
    case "time_limit":
      return { word, tone: "warn", Icon: TimerOffIcon, raw: key };
    case "provider_error":
    case "not_logged_in":
      return { word, tone: "bad", Icon: CircleXIcon, raw: key };
    case "cancelled":
      return { word, tone: "neutral", Icon: BanIcon, raw: key };
    case "lost":
      return { word, tone: "neutral", Icon: CircleHelpIcon, raw: key };
    default:
      return { word, tone: "neutral", Icon: InfoIcon, raw: key };
  }
}

// The sentence the last step of a transcript ends on.
function endSentence(session: HarnessSession, fallback: string): string {
  switch (exitKey(session)) {
    case "submitted":
      return "Finished and handed in its work.";
    case "step_limit":
      return "Stopped early: it used every step it was allowed.";
    case "time_limit":
      return "Stopped early: it ran out of time.";
    case "format_error":
      return "Stopped early: the model kept answering in the wrong format.";
    case "provider_error":
      return "Stopped early: the model provider returned an error.";
    case "not_logged_in":
      return "Stopped early: PULSE is not signed in to the model provider.";
    case "cancelled":
      return "Cancelled before it finished.";
    case "lost":
      return "PULSE lost track of this run before it ended.";
    default:
      return fallback ? fallback.charAt(0).toUpperCase() + fallback.slice(1) : "Ended.";
  }
}

function executorName(executor: string | undefined): string {
  return (executor || "").trim();
}

// The fact names remote machines only; every run has some executor.
function remoteExecutor(executor: string | undefined): string {
  const e = executorName(executor);
  return e && e !== "local" ? e : "";
}

const machineWord = (e: string) => (e === "local" ? "This machine" : e);

function updateSearch(pathname: string, router: ReturnType<typeof useRouter>, next: Record<string, string>) {
  const clean = Object.fromEntries(Object.entries(next).filter(([, v]) => v && v !== "all"));
  router.replace(pathname + query(clean), { scroll: false });
}

// The model by its display name; the facts a curious owner may want sit in the tooltip.
function modelInfo(model: string, models: HarnessModel[]): { name: string; tip: string } {
  const m = models.find((x) => x.id === model);
  if (!m) return { name: model || "Unknown model", tip: model ? "Model " + model : "" };
  const context = m.context_window >= 1000 ? Math.round(m.context_window / 1000) + "k" : String(m.context_window);
  const tip = [
    m.id + (m.vendor ? ", by " + m.vendor : ""),
    m.premium ? "Each request counts as " + (m.premium === 1 ? "1 premium request" : num(m.premium) + " premium requests") : "No premium requests",
    m.context_window ? "Reads up to " + context + " tokens at once" : "",
    m.tool_calls ? "" : "Cannot call tools",
    m.preview ? "Preview model" : "",
  ].filter(Boolean);
  return { name: m.name || m.id, tip: tip.join(". ") + "." };
}

function jobHref(s: Pick<HarnessSession, "project" | "job_id">): string {
  if (/^TASK-/.test(s.job_id)) return hrefs.task(s.job_id);
  return hrefs.tasks(s.project ? { project: s.project } : {});
}

const REPORT_WORD: Record<string, string> = { SUMMARY: "Summary", FINDING: "Finding", SUGGESTION: "Suggestion", QUESTION: "Question", VOTE: "Vote" };

function ReportLines({ text }: { text: string }) {
  const keys = /^(SUMMARY|FINDING|SUGGESTION|QUESTION|VOTE):/;
  return (
    <div className="grid gap-2">
      {text.split("\n").map((line, i) => {
        const hit = keys.exec(line);
        if (!hit && !line.trim()) return null;
        return (
          <div key={i} className="min-w-0 text-sm leading-relaxed">
            {hit ? <span className="mr-1.5 font-semibold text-foreground">{REPORT_WORD[hit[1]] + ":"}</span> : null}
            <span className="whitespace-pre-wrap text-foreground/90 anywhere">{hit ? line.slice(hit[0].length).trimStart() : line}</span>
          </div>
        );
      })}
    </div>
  );
}

export function exitWords(code: number | null): { text: string; bad: boolean } | null {
  if (code === null || code === undefined) return null;
  return code === 0 ? { text: "Succeeded", bad: false } : { text: "Failed (exit code " + code + ")", bad: true };
}

function Observation({ s }: { s: HarnessStep }) {
  const lines = (s.content || "").split("\n");
  const long = lines.length > 30;
  const [open, setOpen] = useState(lines.length <= 12);
  const [all, setAll] = useState(!long);
  const shown = all ? s.content : lines.slice(0, 30).join("\n");
  const exit = exitWords(s.exit_code);
  return (
    <Disclosure
      variant="row"
      open={open}
      onToggle={setOpen}
      className="rounded-lg border bg-muted/30 px-2"
      contentClassName="-mx-2 mt-0 px-0 pb-0"
      summary={
        <span className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[13px] font-normal text-muted-foreground" title={s.command || undefined}>
          <span className="font-medium text-foreground">{open ? "Output" : "Show the output"}</span>
          <span>{plural(lines.length, "line")}</span>
          {exit ? <span className={cn(exit.bad ? "text-bad" : "text-ok")}>{exit.text}</span> : null}
          {s.duration_ms ? <span>{"Took " + (s.duration_ms < 1000 ? s.duration_ms + " ms" : span(s.duration_ms / 1000))}</span> : null}
        </span>
      }
    >
      <pre className="max-h-[52vh] overflow-auto border-t px-3 py-2.5 font-mono text-xs leading-relaxed whitespace-pre-wrap text-foreground/85 anywhere">{shown || "(no output)"}</pre>
      {long ? (
        <button type="button" className="w-full border-t px-3 py-2 text-left text-[13px] font-medium text-primary hover:underline" onClick={() => setAll((v) => !v)}>
          {all ? "Show only the first 30 lines" : "Show all " + num(lines.length) + " lines"}
        </button>
      ) : null}
    </Disclosure>
  );
}

const STEP_DOT: Record<string, string> = {
  assistant: "bg-primary/10 text-primary",
  command: "bg-muted text-foreground",
  read: "bg-muted text-foreground",
  grep: "bg-muted text-foreground",
  glob: "bg-muted text-foreground",
  edit: "bg-muted text-foreground",
  write: "bg-muted text-foreground",
  plan: "bg-primary/10 text-primary",
  mcp: "bg-muted text-foreground",
  skill: "bg-muted text-foreground",
  fetch: "bg-muted text-foreground",
  subagent: "bg-primary/10 text-primary",
  task: "bg-primary/10 text-primary",
  hook: "bg-muted text-muted-foreground",
  observation: "bg-muted text-muted-foreground",
  note: "bg-muted text-muted-foreground",
  error: "bg-warn/15 text-warn",
  exit: "bg-card text-foreground ring-1 ring-border",
};

export const STEP_LABEL: Record<string, string> = {
  assistant: "The agent",
  command: "Ran a command",
  read: "Read a file",
  grep: "Searched the files",
  glob: "Listed files by name",
  edit: "Edited a file",
  write: "Wrote a file",
  plan: "Updated its plan",
  mcp: "Called an MCP tool",
  skill: "Loaded a skill",
  fetch: "Read a web page",
  subagent: "Handed a sub-task to a helper",
  task: "Worked the task board",
  hook: "Ran one of your hooks",
  observation: "What came back",
  note: "Note from PULSE",
  error: "Something went wrong",
  exit: "How it ended",
};

// The typed tools of the harness (read, grep, glob, edit, write, mcp, skill,
// fetch, search, process, memory, task, hook) show their request the way a command
// shows its line; the reply follows as an observation like any command's output.
const TOOL_STEP_KINDS: string[] = ["read", "grep", "glob", "edit", "write", "mcp", "skill", "fetch", "search", "process", "memory", "task", "hook"];

export const STEP_ICON: Record<string, typeof InfoIcon> = {
  assistant: BotIcon,
  command: TerminalIcon,
  observation: TerminalIcon,
  read: FileCodeIcon,
  grep: SearchIcon,
  glob: FilesIcon,
  edit: FilePenIcon,
  write: FilePlusIcon,
  plan: ListChecksIcon,
  mcp: PlugIcon,
  skill: BookOpenIcon,
  fetch: GlobeIcon,
  process: ActivityIcon,
  memory: BrainIcon,
  search: TelescopeIcon,
  subagent: GitForkIcon,
  task: ClipboardListIcon,
  hook: WebhookIcon,
  error: TriangleAlertIcon,
};

function StepIcon({ s, session }: { s: HarnessStep; session: HarnessSession }) {
  // A helper's exit is its own, not the run's.
  if (s.kind === "exit" && !(s.depth ?? 0)) {
    const st = runState(session);
    return <st.Icon className={cn("size-3.5", RUN_ICON_TONE[st.tone])} />;
  }
  if (s.kind === "exit") return s.content === "submitted" ? <CircleCheckIcon className="size-3.5 text-ok" /> : <CircleAlertIcon className="size-3.5 text-warn" />;
  const Icon = STEP_ICON[s.kind] ?? InfoIcon;
  return <Icon className="size-3.5" />;
}// A plan step holds the checklist the agent keeps: "plan: 1/3 done" then
// one "- [ ]", "- [~]" or "- [x]" line per item. It reads as a list.
export function planItems(content: string): { text: string; status: "todo" | "doing" | "done" }[] {
  const items: { text: string; status: "todo" | "doing" | "done" }[] = [];
  for (const line of (content || "").split("\n")) {
    const m = /^-\s*\[([ x~])\]\s*(.*)$/.exec(line.trim());
    if (!m) continue;
    items.push({ text: m[2], status: m[1] === "x" ? "done" : m[1] === "~" ? "doing" : "todo" });
  }
  return items;
}

function PlanCard({ s }: { s: HarnessStep }) {
  const items = planItems(s.content);
  const head = (s.content || "").split("\n")[0] || "";
  if (!items.length) return <p className="text-sm leading-relaxed whitespace-pre-wrap text-foreground anywhere">{s.content}</p>;
  return (
    <div className="grid min-w-0 gap-1.5 rounded-lg border bg-muted/30 px-4 py-3">
      <p className="text-[13px] font-medium text-muted-foreground">{head}</p>
      <ul className="grid gap-1 text-sm leading-relaxed">
        {items.map((it, i) => (
          <li key={i} className="flex min-w-0 items-start gap-2">
            {it.status === "done" ? <CircleCheckIcon className="mt-1 size-3.5 shrink-0 text-ok" aria-label="done" /> : it.status === "doing" ? <LoaderCircleIcon className="mt-1 size-3.5 shrink-0 animate-spin text-primary" aria-label="in progress" /> : <CircleIcon className="mt-1 size-3.5 shrink-0 text-muted-foreground" aria-label="to do" />}
            <span className={cn("min-w-0 anywhere", it.status === "done" && "text-muted-foreground line-through")}>{it.text}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

// Steps of one run are minutes apart: the clock time says more than "1mo ago".
function StepTime({ d }: { d: string }) {
  const t = parseDate(d);
  if (!t) return <When d={d} />;
  return (
    <time dateTime={d} title={d.replace("T", " ").replace("Z", " UTC")} className="tnum whitespace-nowrap">
      {t.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}
    </time>
  );
}

const END_TONE: Record<RunTone, string> = {
  run: "border-primary/30 bg-primary/[0.05]",
  ok: "border-ok/35 bg-ok/[0.06]",
  warn: "border-warn/40 bg-warn/[0.07]",
  bad: "border-bad/30 bg-bad/[0.05]",
  neutral: "border-border bg-muted/40",
};

function EndCard({ s, session }: { s: HarnessStep; session: HarnessSession }) {
  const st = runState(session);
  return (
    <div className={cn("grid min-w-0 gap-2.5 rounded-lg border px-4 py-3", END_TONE[st.tone])}>
      <p className="text-sm font-semibold leading-snug text-foreground" title={s.content || session.exit_status || undefined}>
        {endSentence(session, s.content || session.exit_status)}
      </p>
      {session.status === "submitted" && session.submission ? <ReportLines text={session.submission} /> : null}
    </div>
  );
}

function transcriptText(d: HarnessSessionResponse): string {
  const lines = [`${d.session.id} ${d.session.status}`, `${d.session.project} ${d.session.job_id} ${d.session.job_title}`, `${d.session.agent} ${d.session.provider}/${d.session.model}`, ""];
  for (const s of d.steps) {
    lines.push(`## ${s.n}${s.depth ? " >" + s.depth : ""} ${s.kind} ${s.at || ""}`.trim());
    if (s.command) lines.push("$ " + s.command);
    if (s.content) lines.push(s.content);
    if (s.exit_code !== null) lines.push("exit " + s.exit_code);
    lines.push("");
  }
  if (d.session.submission) lines.push("## submission", d.session.submission);
  return lines.join("\n");
}

// Cancel a run in flight. The shared confirm dialog carries the optimistic
// toast and shows a failure as what / why / fix; the hub stops the agent
// after the step it is on, so the button reads "Cancelling…" until then.
function CancelRunButton({ session, onDone, iconOnly }: { session: HarnessSession; onDone: () => Promise<void>; iconOnly?: boolean }) {
  const [confirm, setConfirm] = useState(false);
  if (session.status !== "running") return null;
  const name = session.job_title || session.job_id;
  const pending = !!session.cancel_requested_at;
  const label = pending ? "Cancelling…" : "Cancel run";
  const button = iconOnly ? (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button type="button" size="icon-sm" variant="ghost" className="text-status-failed hover:text-status-failed" onClick={() => setConfirm(true)} disabled={pending} aria-label={label + ": " + name}>
          <BanIcon />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  ) : (
    <Button type="button" size="sm" variant="outline" className="text-status-failed hover:text-status-failed" onClick={() => setConfirm(true)} disabled={pending} aria-label={label + ": " + name}>
      <BanIcon /> {label}
    </Button>
  );
  return (
    <>
      {button}
      <ConfirmDialog
        open={confirm}
        onOpenChange={setConfirm}
        title={"Cancel this run?"}
        description={
          <>
            <span className="block font-medium text-foreground anywhere">{name}</span>
            <span className="mt-2 block">The agent stops after the step it is on. What it did so far stays in the transcript; the job goes back to the board.</span>
          </>
        }
        action="Cancel the run"
        destructive
        dismiss="Keep running"
        success="Cancel requested. The agent stops after its current step."
        invalidates="harness/sessions"
        onConfirm={async () => {
          await writes.harnessCancel(session.id);
          await onDone();
        }}
      />
    </>
  );
}

export function mergeSteps(current: HarnessStep[], next: HarnessStep[]): HarnessStep[] {
  const seen = new Map<number, HarnessStep>();
  for (const s of current) seen.set(s.n, s);
  for (const s of next) seen.set(s.n, s);
  return [...seen.values()].sort((a, b) => a.n - b.n);
}

function dedupeSessions(rows: HarnessSession[]): HarnessSession[] {
  const seen = new Set<string>();
  const out: HarnessSession[] = [];
  for (const row of rows) {
    if (seen.has(row.id)) continue;
    seen.add(row.id);
    out.push(row);
  }
  return out;
}

function LiveState({ state }: { state: "connecting" | "live" | "polling" }) {
  const text = state === "live" ? "Live" : state === "polling" ? "Checking every few seconds" : "Connecting";
  return (
    <span aria-live="polite" className="inline-flex items-center gap-1.5 text-[13px] text-muted-foreground" title={state === "live" ? "New steps appear as they happen." : state === "polling" ? "The live stream is down; the page checks for new steps every 5 seconds." : undefined}>
      <span className={cn("size-2 rounded-full", state === "live" ? "bg-ok" : state === "polling" ? "bg-warn" : "bg-muted-foreground/40")} aria-hidden />
      {text}
    </span>
  );
}
// ----- filters ---------------------------------------------------------------

// The exact-status picker and the agent / machine fields fold under "More
// filters"; the four chips above carry the everyday choice.
// Which project's runs: every project, or one, with its avatar.
function ProjectSelect({ project, projects, onChange }: { project: string; projects: ProjectsPayload | undefined; onChange: (slug: string) => void }) {
  const list = projects?.projects ?? [];
  const current = list.find((p) => p.slug === project);
  return (
    <Select value={project || "__all"} onValueChange={(v) => onChange(v === "__all" ? "" : v)}>
      <SelectTrigger size="sm" aria-label="Project" className="h-8 w-auto max-w-56 shrink-0">
        <SelectValue placeholder="Every project">
          {project ? (
            <>
              <ProjectAvatar slug={project} name={current?.name} size={20} />
              <span className="truncate">{current ? projectLabel(current) : project}</span>
            </>
          ) : (
            "Every project"
          )}
        </SelectValue>
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="__all">Every project</SelectItem>
        {list.map((p) => (
          <SelectItem key={p.slug} value={p.slug}>
            <ProjectAvatar slug={p.slug} name={p.name} size={20} />
            <span className="truncate">{projectLabel(p)}</span>
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

// The filters that are not chips: the exact harness status (the chips fold
// several into one), the agent, and the machine the run happened on.
function MoreFilters({
  status,
  agent,
  executor,
  executors,
  onChange,
  onExecutorChange,
  onClear,
  hasFilters,
}: {
  status: string;
  agent: string;
  executor: string;
  executors: string[];
  onChange: (next: { project?: string; status?: string; agent?: string }) => void;
  onExecutorChange: (next: string) => void;
  onClear: () => void;
  hasFilters: boolean;
}) {
  const [agentState, setAgentState] = useState({ value: agent, draft: agent });
  const agentDraft = agentState.value === agent ? agentState.draft : agent;
  const setAgentDraft = (draft: string) => setAgentState({ value: agent, draft });
  const commitAgent = () => onChange({ agent: agentDraft.trim() });
  const exact = STATUSES.some((s) => s.value === status) ? status : "all";
  return (
    <div id="runs-more-filters" className="flex flex-wrap items-center gap-2">
      <Select value={exact} onValueChange={(v) => onChange({ status: v === "all" ? "" : v })}>
        <SelectTrigger size="sm" className="h-8 w-[calc(50%-0.25rem)] shrink-0 sm:w-52" aria-label="Exact status">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {STATUSES.map((s) => (
            <SelectItem key={s.value} value={s.value}>
              {s.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Input
        value={agentDraft}
        onChange={(e) => setAgentDraft(e.target.value)}
        onBlur={commitAgent}
        onKeyDown={(e) => {
          if (e.key === "Enter") commitAgent();
        }}
        placeholder="Agent"
        aria-label="Agent"
        className="h-8 w-[calc(50%-0.25rem)] shrink-0 sm:w-40"
      />
      {executors.length > 1 || executor ? (
        <Select value={executor || "all"} onValueChange={(v) => onExecutorChange(v === "all" ? "" : v)}>
          <SelectTrigger size="sm" className="h-8 w-[calc(50%-0.25rem)] shrink-0 sm:w-44" aria-label="Machine">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All machines</SelectItem>
            {executors.map((e) => (
              <SelectItem key={e} value={e}>
                {machineWord(e)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      ) : null}
      {hasFilters ? (
        <Button type="button" variant="ghost" size="sm" onClick={onClear}>
          <XIcon /> Clear filters
        </Button>
      ) : null}
    </div>
  );
}

// ----- the list --------------------------------------------------------------

// A run's time so far, ticking once a second; its own component so only
// the clock re-renders.
function Elapsed({ startedAt, className }: { startedAt: string; className?: string }) {
  const now = useNow(1000);
  return (
    <span className={cn("tnum whitespace-nowrap", className)} title={startedAt ? "Started " + startedAt.replace("T", " ").replace("Z", " UTC") : undefined}>
      {elapsedWords(startedAt, now)}
    </span>
  );
}

// The model with its provider's mark: one colour, 16 px, the display name beside it.
function ModelCell({ session, models, className }: { session: Pick<HarnessSession, "provider" | "model">; models: HarnessModel[]; className?: string }) {
  const model = modelInfo(session.model, models);
  const logo = logoFor(session.provider) ?? logoFor(session.model);
  return (
    <span className={cn("inline-flex min-w-0 items-center gap-1.5", className)} title={model.tip}>
      {logo ? <BrandLogo id={logo} size={16} color="mono" className="text-muted-foreground" /> : <CpuIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />}
      <span className="min-w-0 truncate">{model.name}</span>
    </span>
  );
}

function ProjectCell({ slug, projects, className }: { slug: string; projects: ProjectsPayload | undefined; className?: string }) {
  if (!slug) return <span className="text-muted-foreground">—</span>;
  const p = (projects?.projects ?? []).find((x) => x.slug === slug);
  return (
    <span className={cn("inline-flex min-w-0 items-center gap-1.5", className)}>
      <ProjectAvatar slug={slug} name={p?.name} size={20} />
      <span className="min-w-0 truncate">{p ? projectLabel(p) : slug}</span>
    </span>
  );
}

function AgentCell({ agent, className }: { agent: string; className?: string }) {
  if (!agent) return <span className="text-muted-foreground">—</span>;
  return (
    <span className={cn("inline-flex min-w-0 items-center gap-1.5", className)} title={agent}>
      <Avatar seed={agent} size={20} />
      <span className="min-w-0 truncate">{agentName(agent)}</span>
    </span>
  );
}

// Runs in flight, above the table: the status, the clock, and a way to stop them.
function LiveRuns({ rows, models, selected, onOpen, onDone }: { rows: HarnessSession[]; models: HarnessModel[]; selected: string | null; onOpen: (id: string) => void; onDone: () => Promise<void> }) {
  return (
    <section aria-labelledby="runs-live" className="mb-5">
      <SectionHeading id="runs-live" icon={ActivityIcon} count={rows.length}>
        Running now
      </SectionHeading>
      <ListRows ariaLabel="Runs in flight">
        {rows.map((r) => (
          <ListRow
            key={r.id}
            lead={r.agent ? <Avatar seed={r.agent} size={28} /> : <ProjectAvatar slug={r.project || "pulse"} size={20} />}
            title={r.job_title || r.job_id}
            // On a phone the meta line keeps what fits one thumb's width -
            // project, steps and the time so far (the row's time column is
            // desktop-only); the agent and the model wait for a wider screen.
            meta={
              <>
                {r.agent ? <span className="max-sm:hidden" title={r.agent}>{agentName(r.agent)}</span> : null}
                {r.project ? <span className="truncate">{r.project}</span> : null}
                <ModelCell session={r} models={models} className="max-sm:hidden" />
                <span className="shrink-0 whitespace-nowrap">{plural(r.steps, "step")}</span>
                <Elapsed startedAt={r.started_at} className="shrink-0 sm:hidden" />
              </>
            }
            // Cancel sits in the status slot, which never wraps: as a row
            // action it would drop onto a line of its own on a phone.
            status={
              <>
                <StatusBadge status={r.cancel_requested_at ? "cancelling" : "running"} label={r.cancel_requested_at ? "Cancelling" : "Running"} size="sm" className="max-sm:hidden" />
                <StatusIcon status={r.cancel_requested_at ? "cancelling" : "running"} label={r.cancel_requested_at ? "Cancelling" : "Running"} className="sm:hidden" />
                <Elapsed startedAt={r.started_at} className="max-sm:hidden" />
                <span
                  className="inline-flex"
                  onClick={(e) => e.stopPropagation()}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") e.stopPropagation();
                  }}
                >
                  <CancelRunButton session={r} onDone={onDone} iconOnly />
                </span>
              </>
            }
            selected={selected === r.id}
            onOpen={() => onOpen(r.id)}
          />
        ))}
      </ListRows>
    </section>
  );
}

function runColumns(projects: ProjectsPayload | undefined, models: HarnessModel[], now: number, showProject: boolean): Column<HarnessSession>[] {
  const cols: Column<HarnessSession>[] = [
    {
      key: "status",
      label: <span className="sr-only">Status</span>,
      nowrap: true,
      className: "w-8 pr-0",
      sort: (r) => runState(r).word,
      render: (r) => {
        const st = runState(r);
        return <StatusIcon status={badgeStatus(st.raw)} label={st.word} />;
      },
    },
    {
      key: "title",
      label: "Run",
      sort: (r) => (r.job_title || r.job_id).toLowerCase(),
      render: (r) => (
        <span className="grid min-w-0 gap-0.5">
          <span className="min-w-0 truncate font-medium text-foreground" title={r.job_title || r.job_id}>
            {r.job_title || r.job_id}
          </span>
          {/* The folded meta on a narrow screen: the state word gives way (with an ellipsis), the names stay whole. */}
          <span className="flex min-w-0 items-center gap-x-2 overflow-hidden text-xs text-muted-foreground md:hidden">
            {r.project ? <span className="shrink-0">{r.project}</span> : null}
            {r.agent ? <span className="shrink-0">{agentName(r.agent)}</span> : null}
            <span className="min-w-0 truncate">{runState(r).word}</span>
          </span>
        </span>
      ),
    },
  ];
  if (showProject) cols.push({ key: "project", label: "Project", hide: "md", nowrap: true, sort: (r) => r.project, render: (r) => <ProjectCell slug={r.project} projects={projects} className="max-w-40" /> });
  cols.push(
    { key: "agent", label: "Agent", hide: "lg", nowrap: true, sort: (r) => r.agent, render: (r) => <AgentCell agent={r.agent} className="max-w-36" /> },
    { key: "model", label: "Model", hide: "lg", nowrap: true, sort: (r) => r.model, render: (r) => <ModelCell session={r} models={models} className="max-w-44" /> },
    { key: "steps", label: "Steps", hide: "xl", align: "right", nowrap: true, sort: (r) => r.steps, render: (r) => <span title={plural(r.steps, "step") + ", " + plural(r.calls, "model call")}>{num(r.steps)}</span> },
    { key: "duration", label: "Took", hide: "md", align: "right", nowrap: true, sort: (r) => runSeconds(r.started_at, r.ended_at || r.updated_at, now), render: (r) => (r.started_at ? span(runSeconds(r.started_at, r.ended_at || r.updated_at, now)) : "—") },
    { key: "cost", label: "Credits", hide: "xl", align: "right", nowrap: true, sort: (r) => r.cost, render: (r) => (r.cost ? <span title="What the model provider charged. For GitHub Copilot, premium requests.">{num(r.cost)}</span> : <span className="text-muted-foreground">—</span>) },
    { key: "when", label: "When", align: "right", nowrap: true, sort: (r) => r.started_at, render: (r) => <When d={r.started_at} className="text-muted-foreground" /> },
  );
  return cols;
}

// ----- the transcript --------------------------------------------------------

const TOOL_ICON: Record<StepTool, typeof InfoIcon> = {
  shell: TerminalIcon,
  edit: FilePenIcon,
  read: FileTextIcon,
  web: GlobeIcon,
  search: SearchIcon,
  model: MessageSquareIcon,
};

// The icon a step's row carries: the tool family first (a terminal for
// shell work, a pen for edits...), else the step kind's own icon.
function ToolIcon({ s, session, className }: { s: HarnessStep; session: HarnessSession; className?: string }) {
  const tool = stepTool(s.kind);
  if (!tool || s.kind === "exit") return <StepIcon s={s} session={session} />;
  const Icon = TOOL_ICON[tool];
  return <Icon className={cn("size-3.5", className)} />;
}

// The nearest ancestor that scrolls: the panel's body on desktop, the
// sheet's on a phone.
function scrollBoxOf(el: HTMLElement | null): HTMLElement | null {
  let cur = el?.parentElement ?? null;
  while (cur) {
    const o = getComputedStyle(cur).overflowY;
    if ((o === "auto" || o === "scroll") && cur.scrollHeight > cur.clientHeight) return cur;
    cur = cur.parentElement;
  }
  return null;
}

// What a step's body shows once unfolded: the model's words as markdown,
// a command as it was typed, a tool's request in mono, the plan as a
// checklist, a note or an error as text.
function StepBody({ s, session }: { s: HarnessStep; session: HarnessSession }) {
  const depth = Math.min(s.depth ?? 0, 3);
  if (s.kind === "assistant")
    return (
      <div className="min-w-0 text-sm leading-relaxed">
        <Markdown source={s.content} className="[&_pre]:whitespace-pre-wrap" />
      </div>
    );
  if (s.kind === "command")
    return (
      <pre className="min-w-0 rounded-lg bg-zinc-950 px-3 py-2.5 font-mono text-[13px] leading-relaxed whitespace-pre-wrap text-zinc-100 anywhere dark:bg-black">
        <span className="text-emerald-300 select-none">$ </span>
        {s.command}
      </pre>
    );
  if (TOOL_STEP_KINDS.includes(s.kind)) return <pre className="min-w-0 rounded-lg border bg-muted/30 px-3 py-2.5 font-mono text-[13px] leading-relaxed whitespace-pre-wrap text-foreground/90 anywhere">{s.content || s.command}</pre>;
  if (s.kind === "plan") return <PlanCard s={s} />;
  if (s.kind === "subagent")
    return (
      <div className="grid min-w-0 gap-1 rounded-lg border border-primary/30 bg-primary/[0.05] px-4 py-3">
        <p className="text-[13px] font-medium text-muted-foreground">{s.command || "subagent"}</p>
        <p className="text-sm leading-relaxed whitespace-pre-wrap text-foreground anywhere">{s.content}</p>
      </div>
    );
  if (s.kind === "observation") return <Observation s={s} />;
  if (s.kind === "note") return <p className="text-sm leading-relaxed whitespace-pre-wrap text-muted-foreground anywhere">{sentence(s.content)}</p>;
  if (s.kind === "error") return <p className="text-sm leading-relaxed whitespace-pre-wrap text-foreground anywhere">{s.content}</p>;
  if (depth) return <p className="text-sm leading-relaxed text-muted-foreground">{s.content === "submitted" ? "The helper reported back." : "The helper stopped: " + (s.content || "unknown") + "."}</p>;
  return <EndCard s={s} session={session} />;
}

// One step as a 32 px folding row: the tool's icon, what it did (the
// command in mono), how long it took and when; unfolded, the request and
// the output that came back.
function StepRow({ entry, session }: { entry: StepEntry; session: HarnessSession }) {
  const s = entry.step;
  const depth = Math.min(s.depth ?? 0, 3);
  const tokens = (s.prompt_tokens || 0) + (s.completion_tokens || 0);
  const tip = ["Step " + s.n, depth ? "helper, depth " + depth : "", s.model ? "model " + s.model : "", tokens ? compact(tokens) + " tokens" : ""].filter(Boolean).join(", ");
  const mono = s.kind === "command" || TOOL_STEP_KINDS.includes(s.kind);
  const label = s.kind === "exit" && depth ? "How the helper ended" : s.kind === "command" && s.title?.trim() ? s.title.trim() : (STEP_LABEL[s.kind] ?? "Step " + s.n);
  const summary = stepSummary(s);
  const took = msText(s.duration_ms || entry.output?.duration_ms || 0);
  const exit = entry.output ? exitWords(entry.output.exit_code) : null;
  return (
    <li className="min-w-0" style={depth ? { marginLeft: depth * 20 } : undefined} data-depth={depth || undefined}>
      <details className="group/step min-w-0" open={openByDefault(s) || undefined}>
        <summary
          className="flex h-8 min-w-0 cursor-pointer list-none items-center gap-2 rounded-md px-1.5 text-[13px] select-none hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-details-marker]:hidden"
          title={tip}
        >
          <span className={cn("flex size-5 shrink-0 items-center justify-center rounded-md", STEP_DOT[s.kind] ?? STEP_DOT.note)} aria-hidden>
            <ToolIcon s={s} session={session} />
          </span>
          <span className="flex min-w-0 flex-1 items-baseline gap-2">
            <span className="shrink-0 font-medium text-foreground">{label}</span>
            {summary && !(s.kind === "command" && !s.title?.trim() && summary === label) ? (
              <span className={cn("min-w-0 truncate text-muted-foreground", mono && "font-mono text-[13px] text-foreground/80")}>{summary}</span>
            ) : null}
          </span>
          <span className="ml-auto flex shrink-0 items-center gap-2 text-xs text-muted-foreground tabular-nums">
            {exit?.bad ? <span className="text-status-failed">{exit.text}</span> : null}
            {took ? <span>{took}</span> : null}
            {s.at ? <StepTime d={s.at} /> : null}
            <ChevronRightIcon className="size-3.5 shrink-0 transition-transform duration-150 group-open/step:rotate-90 motion-reduce:transition-none" aria-hidden />
          </span>
        </summary>
        <div className="mt-1 mb-3 ml-7 grid min-w-0 gap-2">
          <StepBody s={s} session={session} />
          {entry.output ? <Observation s={entry.output} /> : null}
        </div>
      </details>
    </li>
  );
}

// The run at a glance: four cells - how it ended (or that it is running,
// with the time so far ticking), how long it took, what it cost, how many
// tokens it read and wrote.
function RunSummary({ session: s, state: st, running, now }: { session: HarnessSession; state: ReturnType<typeof runState>; running: boolean; now: number }) {
  const totalTokens = (s.prompt_tokens || 0) + (s.completion_tokens || 0);
  const cell = "grid min-w-0 gap-0.5 px-3 py-2.5";
  const label = "text-xs text-muted-foreground";
  const value = "tnum min-w-0 truncate text-sm font-medium text-foreground";
  return (
    <dl className="grid min-w-0 grid-cols-2 overflow-hidden rounded-lg border bg-card sm:grid-cols-4 [&>div:nth-child(n+2)]:border-l [&>div:nth-child(3)]:max-sm:border-l-0 [&>div:nth-child(n+3)]:max-sm:border-t">
      <div className={cell}>
        <dt className={label}>Status</dt>
        <dd className="min-w-0">
          <StatusBadge status={badgeStatus(st.raw)} label={st.word} size="sm" />
        </dd>
      </div>
      <div className={cell}>
        <dt className={label}>{running ? "Running for" : "Duration"}</dt>
        <dd className={value} title={s.started_at ? "Started " + s.started_at.replace("T", " ").replace("Z", " UTC") : undefined}>
          {running ? <Elapsed startedAt={s.started_at} /> : s.started_at ? span(runSeconds(s.started_at, s.ended_at || s.updated_at, now)) : "—"}
        </dd>
      </div>
      <div className={cell}>
        <dt className={label}>Credits</dt>
        <dd className={value} title="What the model provider charged (provider units). For GitHub Copilot, premium requests.">
          {s.cost ? num(s.cost) : "—"}
        </dd>
      </div>
      <div className={cell}>
        <dt className={label}>Tokens</dt>
        <dd className={value} title={totalTokens ? compact(s.prompt_tokens) + " prompt, " + compact(s.completion_tokens) + " completion, over " + plural(s.calls, "model call") : undefined}>
          {totalTokens ? compact(totalTokens) : "—"}
        </dd>
      </div>
    </dl>
  );
}

function RunPanel({ id, models, projects, onClose }: { id: string | null; models: HarnessModel[]; projects: ProjectsPayload | undefined; onClose: () => void }) {
  const live = useLiveEvents();
  const q = useApi<HarnessSessionResponse>(id ? "harness/sessions/" + enc(id) : null, { refresh: live.state === "polling" ? 5000 : 30000 });
  const [extra, setExtra] = useState<{ id: string; steps: HarnessStep[] }>({ id: "", steps: [] });
  const [followState, setFollowState] = useState({ id, follow: true, atBottom: true });
  const lastEvent = useRef({ id, eventId: 0 });
  const bottom = useRef<HTMLDivElement | null>(null);
  const follow = followState.id === id ? followState.follow : true;
  const atBottom = followState.id === id ? followState.atBottom : true;
  const d = useMemo(() => {
    if (!q.data || !id) return undefined;
    return { ...q.data, steps: mergeSteps(q.data.steps, extra.id === id ? extra.steps : []) };
  }, [extra, id, q.data]);
  const running = d?.session.status === "running";
  const now = useNow();

  useEffect(() => {
    if (!id) return;
    const timer = setTimeout(() => {
      setExtra((cur) => {
        if (cur.id !== id) return { id, steps: [] };
        const full = q.data?.steps;
        if (!full?.length || !cur.steps.length) return cur;
        const fullNs = new Set(full.map((s) => s.n));
        const steps = cur.steps.filter((s) => !fullNs.has(s.n));
        return steps.length === cur.steps.length ? cur : { id, steps };
      });
    }, 0);
    return () => clearTimeout(timer);
  }, [id, q.data?.steps, q.updatedAt]);

  // Follow new steps while the run is going: the panel's body scrolls to
  // its end, never the page behind it.
  useEffect(() => {
    if (!(follow && atBottom && running)) return;
    const box = scrollBoxOf(bottom.current);
    if (box) box.scrollTop = box.scrollHeight;
  }, [d?.steps.length, follow, atBottom, running]);

  // Reading back up pauses following until the owner returns to the end.
  useEffect(() => {
    if (!d) return;
    const box = scrollBoxOf(bottom.current);
    if (!box) return;
    const onScroll = () => setFollowState((cur) => ({ id, follow: cur.id === id ? cur.follow : true, atBottom: box.scrollHeight - box.scrollTop - box.clientHeight < 96 }));
    box.addEventListener("scroll", onScroll, { passive: true });
    return () => box.removeEventListener("scroll", onScroll);
  }, [d, id]);

  useEffect(() => {
    if (!id || !d) return;
    const lastSeen = lastEvent.current.id === id ? lastEvent.current.eventId : 0;
    const hit = live.events.filter((e) => e.id > lastSeen && e.subject === id && (e.action === "harness.step" || e.action === "harness.finish" || e.action === "harness.cancel")).sort((a, b) => a.id - b.id);
    if (!hit.length) return;
    lastEvent.current = { id, eventId: hit[hit.length - 1].id };
    const lastN = d.steps.reduce((max, s) => Math.max(max, s.n), -1);
    const path = "harness/sessions/" + enc(id);
    const finished = hit.some((e) => e.action === "harness.finish" || e.action === "harness.cancel");
    api<HarnessSessionResponse>(path + "?after=" + lastN)
      .then((r) => {
        setExtra((cur) => ({ id, steps: mergeSteps(cur.id === id ? cur.steps : [], r.steps) }));
        if (finished) void q.reload();
      })
      .catch(() => {
        void q.reload();
      });
  }, [d, id, live.events, q]);

  const s = d?.session;
  const st = s ? runState(s) : null;
  const trajectoryHref = s?.trajectory ? apiHref("harness/sessions/" + enc(s.id) + "/trajectory") : "";
  const entries = useMemo(() => (d ? pairSteps(d.steps) : []), [d]);
  // The run's own ending reads as the final result under the steps.
  const endIndex = entries.findIndex((e) => e.step.kind === "exit" && !(e.step.depth ?? 0));
  const timeline = endIndex >= 0 ? entries.filter((_, i) => i !== endIndex) : entries;
  const ending = endIndex >= 0 ? entries[endIndex].step : null;
  const project = s?.project ? (projects?.projects ?? []).find((p) => p.slug === s.project) : undefined;

  let body: React.ReactNode;
  if (!id) body = null;
  else if (q.error && !d)
    body = (
      <ErrorNotice error={q.error} onRetry={() => void q.reload()}>
        {"This run's transcript didn't load (" + q.error + ")."}
      </ErrorNotice>
    );
  else if (!d || !s || !st)
    body = (
      <div aria-busy="true">
        <SkeletonRows n={5} />
      </div>
    );
  else {
    const errorTone = st.tone === "bad" ? "bad" : "warn";
    body = (
      <div className="grid min-w-0 gap-4" aria-live={running ? "polite" : undefined}>
        <RunSummary session={s} state={st} running={running} now={now} />
        {remoteExecutor(s.executor) || running ? (
          <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-[13px] text-muted-foreground">
            {remoteExecutor(s.executor) ? <span title="The machine (executor) the run happened on">{"On " + remoteExecutor(s.executor)}</span> : null}
            {running ? <LiveState state={live.state} /> : null}
          </div>
        ) : null}
        {s.cancel_requested_at && s.status === "running" ? <Notice tone="warn">You asked to cancel this run. The agent stops after the step it is on.</Notice> : null}
        {s.error ? <Notice tone={errorTone}>{s.error}</Notice> : null}
        {running ? (
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <Button size="sm" variant={follow ? "secondary" : "outline"} aria-pressed={follow} onClick={() => setFollowState((cur) => ({ id, follow: cur.id === id ? !cur.follow : false, atBottom: cur.id === id ? cur.atBottom : true }))} title="Keep the newest step in view as it arrives">
              <PlayIcon /> {follow ? "Following new steps" : "Follow new steps"}
            </Button>
            <CancelRunButton session={s} onDone={q.reload} />
          </div>
        ) : null}
        <section aria-labelledby="run-steps" className="min-w-0">
          <SectionHeading id="run-steps" as="h3" count={timeline.length} className="mb-1">
            Steps
          </SectionHeading>
          {timeline.length ? (
            <ol aria-label="Steps" className="min-w-0 -mx-1.5">
              {timeline.map((e) => (
                <StepRow key={e.step.n} entry={e} session={s} />
              ))}
            </ol>
          ) : (
            <p className="text-sm text-muted-foreground">{running ? "The agent is getting started. Its first step shows up here." : "This run has no recorded steps."}</p>
          )}
        </section>
        {ending ? (
          <section aria-labelledby="run-result" className="min-w-0">
            <SectionHeading id="run-result" as="h3" className="mb-1">
              Result
            </SectionHeading>
            <EndCard s={ending} session={s} />
          </section>
        ) : null}
        <Disclosure summary="Technical details">
          <div className="rounded-lg border bg-muted/30 px-3 py-3">
            <Kv
              className="text-[13px] [&_dd]:font-mono [&_dd]:text-xs"
              rows={[
                ["Run", s.id],
                ["Job", s.job_id],
                ["Model", s.provider + "/" + s.model],
                ...(s.executor ? [["Machine", s.executor] as [React.ReactNode, React.ReactNode]] : []),
                ["Folder", s.cwd || "—"],
                [
                  "Record file",
                  <span key="t" className="inline-flex min-w-0 flex-wrap items-center gap-1.5">
                    <span className="anywhere">{s.trajectory || "not recorded"}</span>
                    {s.trajectory ? (
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <Button size="icon-xs" variant="ghost" aria-label="Copy the record file path" onClick={() => navigator.clipboard.writeText(s.trajectory).then(() => toast.success("Path copied."))}>
                            <ClipboardIcon />
                          </Button>
                        </TooltipTrigger>
                        <TooltipContent>Copy the path</TooltipContent>
                      </Tooltip>
                    ) : null}
                  </span>,
                ],
                ["Tokens", compact(s.prompt_tokens) + " prompt, " + compact(s.completion_tokens) + " completion"],
              ]}
            />
          </div>
        </Disclosure>
        <div ref={bottom} />
      </div>
    );
  }

  return (
    <SidePanel
      open={!!id}
      onClose={onClose}
      width="run"
      title={
        s ? (
          <Link href={jobHref(s)} className="underline-offset-2 hover:underline" title={"Open the job " + s.job_id}>
            {s.job_title || s.job_id}
          </Link>
        ) : (
          "Run"
        )
      }
      subtitle={
        s
          ? [project ? projectLabel(project) : s.project, s.agent ? agentName(s.agent) : "", modelInfo(s.model, models).name, s.started_at ? "started " + (parseDate(s.started_at)?.toLocaleString([], { dateStyle: "medium", timeStyle: "short" }) ?? s.started_at) : ""].filter(Boolean).join(" · ")
          : id
            ? "Run " + id
            : undefined
      }
      lead={s ? <ProjectAvatar slug={s.project || "pulse"} name={project?.name} size={20} /> : undefined}
      actions={
        d ? (
          <>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  size="icon-sm"
                  variant="ghost"
                  aria-label="Copy the transcript as text"
                  onClick={() =>
                    navigator.clipboard
                      .writeText(transcriptText(d))
                      .then(() => toast.success("Transcript copied."))
                      .catch(() => toast.error("Could not copy the transcript."))
                  }
                >
                  <CopyIcon />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Copy as text</TooltipContent>
            </Tooltip>
            {trajectoryHref ? (
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button size="icon-sm" variant="ghost" asChild>
                    <a href={trajectoryHref} download aria-label="Download the full record of the run">
                      <DownloadIcon />
                    </a>
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Download the record (trajectory)</TooltipContent>
              </Tooltip>
            ) : null}
          </>
        ) : null
      }
    >
      {body}
    </SidePanel>
  );
}

// ----- the page --------------------------------------------------------------

// Today in one line under the chips: what ran, what finished, what it cost.
function Summary({ counts }: { counts: HarnessSessionsResponse["counts"] | undefined }) {
  if (!counts) return null;
  const today = counts.today ? plural(counts.today, "run") + " today: " + num(counts.submitted_today) + " finished, " + num(counts.failed_today) + " stopped early." : "No runs today yet.";
  return (
    <p className="text-[13px] text-muted-foreground">
      {today}{" "}
      {counts.cost_today ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <span tabIndex={0} className="underline decoration-muted-foreground/50 decoration-dotted underline-offset-4 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
              {plural(counts.cost_today, "model credit") + " used today."}
            </span>
          </TooltipTrigger>
          <TooltipContent>What the model provider charged (provider units). For GitHub Copilot, premium requests.</TooltipContent>
        </Tooltip>
      ) : null}
    </p>
  );
}

export function HarnessPageClient() {
  const sp = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const project = sp.get("project") || "";
  const status = sp.get("status") || "";
  const agent = sp.get("agent") || "";
  const [selected, setSelected] = useOpenItem("id");
  const limit = 50;
  const [executor, setExecutor] = useState("");
  const [moreOpen, setMoreOpen] = useState(false);
  const [olderState, setOlderState] = useState<{ key: string; pages: HarnessSessionsResponse[]; loading: boolean; error: string }>({ key: "", pages: [], loading: false, error: "" });
  // The chips fold several statuses into one; the API filters on one exact
  // status, so a many-status chip asks for everything and the page sifts.
  const wanted = apiStatus(status);
  const filterKey = [project, wanted, agent].join("\n");
  const older = useMemo(() => (olderState.key === filterKey ? olderState.pages : []), [filterKey, olderState.key, olderState.pages]);
  const olderLoading = olderState.key === filterKey && olderState.loading;
  const olderError = olderState.key === filterKey ? olderState.error : "";
  const path = "harness/sessions" + query({ project, status: wanted, agent, limit });
  const sessions = useApi<HarnessSessionsResponse>(path, { refresh: 5000 });
  const models = useApi<HarnessModelsResponse>("harness/models", { refresh: 60000 });
  const projects = useApi<ProjectsPayload>("projects", { refresh: 60000 });
  const loadedPages = useMemo(() => (sessions.data ? [sessions.data, ...older] : older), [older, sessions.data]);
  const loadedRows = useMemo(() => dedupeSessions(loadedPages.flatMap((p) => p.sessions)), [loadedPages]);
  const counts = sessions.data?.counts;
  const live = useLiveEvents();
  const now = useNow();
  const lastRunEvent = useMemo(() => live.events.reduce((at, e) => (e.action.startsWith("harness.") && e.at > at ? e.at : at), ""), [live.events]);
  const lastRunEventAt = parseDate(lastRunEvent)?.getTime() ?? 0;
  const liveNow = live.state === "live" && lastRunEventAt > 0 && now - lastRunEventAt < 2 * 60_000;
  const executors = useMemo(() => Array.from(new Set(loadedRows.map((r) => executorName(r.executor)).filter(Boolean))).sort(), [loadedRows]);
  const machineRows = useMemo(() => (executor ? loadedRows.filter((r) => executorName(r.executor) === executor) : loadedRows), [executor, loadedRows]);
  const chips = useMemo(() => statusChips(machineRows), [machineRows]);
  const rows = useMemo(() => machineRows.filter((r) => matchesStatus(r.status, status)), [machineRows, status]);
  const liveRows = useMemo(() => (status && !matchesStatus("running", status) ? [] : rows.filter((r) => r.status === "running")), [rows, status]);
  const tableRows = useMemo(() => rows.filter((r) => r.status !== "running"), [rows]);
  const change = useCallback((next: { project?: string; status?: string; agent?: string }) => updateSearch(pathname, router, { project, status, agent, id: selected || "", ...next }), [agent, pathname, project, router, selected, status]);
  const lastPage = loadedPages[loadedPages.length - 1];
  const canLoadMore = !!lastPage?.next_before;
  const modelRows = useMemo(() => models.data?.models ?? [], [models.data]);
  const hasFilters = !!(project || status || agent || executor);
  const showProject = !project;
  const clear = () => {
    setExecutor("");
    updateSearch(pathname, router, { id: selected || "" });
  };
  const loadOlder = () => {
    const before = lastPage?.next_before;
    if (!before || olderLoading) return;
    setOlderState((cur) => ({ key: filterKey, pages: cur.key === filterKey ? cur.pages : [], loading: true, error: "" }));
    api<HarnessSessionsResponse>("harness/sessions" + query({ project, status: wanted, agent, limit, before }))
      .then((page) => setOlderState((cur) => ({ key: filterKey, pages: [...(cur.key === filterKey ? cur.pages : []), page], loading: false, error: "" })))
      .catch((e: unknown) => {
        const t = errorText(e);
        setOlderState((cur) => ({ key: filterKey, pages: cur.key === filterKey ? cur.pages : [], loading: false, error: [t.error, t.why, t.fix].filter(Boolean).join(" ") }));
      });
  };
  const columns = useMemo(() => runColumns(projects.data, modelRows, now, showProject), [modelRows, now, projects.data, showProject]);
  const exactStatus = status && !RUN_GROUPS.some((g) => g.key === status) ? runWord(status) : "";
  const moreActive = !!(agent || executor);
  return (
    <>
      <PageHeader title="Runs" subtitle="Every agent run, step by step" />
      <div className="mb-5 grid min-w-0 gap-3">
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <ProjectSelect project={project} projects={projects.data} onChange={(slug) => change({ project: slug })} />
          <span aria-hidden className="mx-0.5 hidden h-4 w-px shrink-0 bg-border sm:inline-block" />
          {/* On a phone the chips take a row of their own under the project and the filters button, so neither gets squeezed. */}
          <div role="group" aria-label="Filter runs by how they went" className="scroll-strip -mx-1 flex min-w-0 flex-1 flex-nowrap items-center gap-1.5 px-1 py-0.5 max-sm:order-last max-sm:basis-full">
          <FilterChip active={!status} count={machineRows.length} onClick={() => change({ status: "" })}>
            All
          </FilterChip>
          {chips.map((c) => (
            <FilterChip key={c.key} active={chipActive(status, c.key)} count={c.count} onClick={() => change({ status: chipActive(status, c.key) ? "" : c.key })}>
              {c.label}
            </FilterChip>
          ))}
          {exactStatus ? (
            <FilterChip active count={rows.length} onClick={() => change({ status: "" })} className="max-w-full">
              {exactStatus}
            </FilterChip>
          ) : null}
          </div>
          <Button type="button" variant={moreOpen || moreActive ? "secondary" : "ghost"} size="sm" className="shrink-0 max-sm:ml-auto" aria-expanded={moreOpen} aria-controls="runs-more-filters" onClick={() => setMoreOpen((v) => !v)}>
            <SlidersHorizontalIcon /> <span className="max-sm:sr-only">More filters</span>
            {moreActive ? <span className="tnum inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-full bg-foreground/10 px-1.5 text-xs leading-none">{[agent, executor].filter(Boolean).length}</span> : null}
          </Button>
        </div>
        {moreOpen || moreActive ? <MoreFilters status={status} agent={agent} executor={executor} executors={executors} onChange={change} onExecutorChange={setExecutor} onClear={clear} hasFilters={hasFilters} /> : null}
        <Summary counts={counts} />
      </div>
      {sessions.error && !sessions.data ? (
        <ErrorNotice error={sessions.error} onRetry={() => void sessions.reload()}>
          {"The list of runs didn't load (" + sessions.error + "). Check that PULSE is running, then try again."}
        </ErrorNotice>
      ) : null}
      {!sessions.data ? (
        sessions.error ? null : (
          <SkeletonRows n={6} className="[&>*]:h-11 [&>*]:rounded-md" />
        )
      ) : rows.length === 0 ? (
        hasFilters ? (
          <EmptyState
            title="No runs match these filters"
            icon={<SearchIcon className="size-[18px]" />}
            action={
              <Button size="sm" variant="outline" onClick={clear}>
                <XIcon /> Clear filters
              </Button>
            }
          >
            Try another project or status, or clear the filters to see every run.
          </EmptyState>
        ) : (
          <EmptyState
            title="No runs yet"
            icon={<BotIcon className="size-[18px]" />}
            action={
              <Button asChild size="sm" variant="outline">
                <Link href={hrefs.settings("harness")}>Open sign-in &amp; models</Link>
              </Button>
            }
          >
            Each time an agent starts a job, its run shows up here with a step-by-step transcript. Sign in to a model provider and pick models so agents can start.
          </EmptyState>
        )
      ) : (
        <>
          {liveRows.length ? <LiveRuns rows={liveRows} models={modelRows} selected={selected} onOpen={setSelected} onDone={sessions.reload} /> : null}
          {tableRows.length ? (
            <DataTable
              columns={columns}
              rows={tableRows}
              rowKey={(r) => r.id}
              onRow={(r) => setSelected(r.id)}
              rowAriaSelected={(r) => r.id === selected}
              rowClassName={(r) => (r.id === selected ? "bg-accent/60" : undefined)}
              initialSort="when"
              initialDir="desc"
              caption="Runs"
              className="min-w-0 [&_td]:py-2 [&_tr]:h-11"
            />
          ) : liveRows.length ? null : (
            <EmptyState title="No finished runs here" icon={<ScrollTextIcon className="size-[18px]" />}>
              The runs in flight sit above; finished ones land here.
            </EmptyState>
          )}
          <div className="mt-3 flex min-w-0 flex-wrap items-center justify-between gap-2 px-1 text-[13px] text-muted-foreground">
            <span className="inline-flex flex-wrap items-center gap-x-3 gap-y-1">
              <span>{rows.length === loadedRows.length ? "Showing " + plural(rows.length, "run") : "Showing " + num(rows.length) + " of " + plural(loadedRows.length, "loaded run")}</span>
              {liveNow ? (
                <span className="inline-flex items-center gap-1.5" title="A run changed in the last two minutes. The list keeps itself up to date.">
                  <span className="size-2 rounded-full bg-status-done" aria-hidden /> Live
                </span>
              ) : null}
            </span>
            {olderError ? <span className="text-status-failed">{olderError}</span> : null}
            {canLoadMore ? (
              <Button type="button" size="sm" variant="outline" onClick={loadOlder} disabled={olderLoading}>
                {olderLoading ? "Loading…" : "Load older runs"}
              </Button>
            ) : null}
          </div>
        </>
      )}
      <RunPanel id={selected} models={modelRows} projects={projects.data} onClose={() => setSelected(null)} />
    </>
  );
}
