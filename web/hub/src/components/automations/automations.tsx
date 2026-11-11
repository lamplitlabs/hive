"use client";

import { useEffect, useMemo, useState } from "react";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import {
  CalendarClockIcon,
  CheckIcon,
  CopyIcon,
  FileCode2Icon,
  HistoryIcon,
  Loader2Icon,
  MoreHorizontalIcon,
  PauseIcon,
  PencilIcon,
  PlayIcon,
  PlusIcon,
  SquareIcon,
  TriangleAlertIcon,
  WrenchIcon,
} from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { ConfirmDialog, showApiError } from "@/components/hive/confirm";
import { KindTile } from "@/components/hive/kind-tile";
import { PageHeader } from "@/components/hive/page-header";
import { SectionHeading } from "@/components/hive/section-heading";
import { StatusBadge, StatusIcon } from "@/components/hive/status-badge";
import {
  Disclosure,
  EmptyState,
  ErrorNotice,
  Notice,
  SkeletonRows,
} from "@/components/hive/panel";
import { Mono, ToneBadge, When } from "@/components/hive/who";
import {
  ApiError,
  api,
  automationHistoryPath,
  query,
  writes,
  type AutomationEntry,
  type AutomationHistory,
  type AutomationHistoryEntry,
  type AutomationPlace,
  type AutomationProject,
  type AutomationRepoFile,
  type AutomationSchedule,
  type AutomationsPayload,
} from "@/lib/api";
import {
  EMPTY_ENTRY,
  EVERY_PRESETS,
  KIND_WORDS,
  cadenceWords,
  cleanEntry,
  daemonProblems,
  entryFromFile,
  entryFromSchedule,
  entryHints,
  everyMinutes,
  fileEntryOf,
  fileProblems,
  fileState,
  historySummary,
  historyWords,
  inFolder,
  isScriptKind,
  isSlug,
  keptWords,
  scriptHome,
  scriptInHive,
  scriptOutside,
  slugify,
  stateWords,
  plainProblem,
  localWhen,
  utcClockLocal,
} from "@/lib/automations";
import { num, plural } from "@/lib/format";
import { hrefs } from "@/lib/nav";
import { invalidate, useApi, useNow } from "@/lib/use-api";
import { cn } from "@/lib/utils";
import {
  automationKind,
  fileProblem,
  lastResults,
  nextRunWords,
  problemOf,
  scheduleWords,
  sortAutomations,
  weekStrip,
  type Problem,
  type WeekDay,
} from "./automations-state";

// Automations (spec "Automations"): the standing jobs of each project - a
// task, an improvement, a check or a watch that runs on a cadence. A week
// strip at the top shows the next seven days with a dot per scheduled run;
// below it one card per automation with its kind tile, its schedule in the
// viewer's own time, the next run and the last five results. Anything held
// or failing sorts first with its problem in one sentence and a Fix button.
//
// The place to keep an automation is the project's repository, one file per
// automation under `.hive/automations/`; this page writes those files (one
// commit on the default branch, never a push) through the same checks the
// daemon applies them with. A schedule can also live in the store alone, the
// way `hive schedule add` makes one. The dialogs and the history panel at
// the bottom of this file are unchanged by the redesign.

export const refreshAutomations = () =>
  invalidate(
    (path) =>
      path.startsWith("automations") ||
      path === "runner" ||
      path.startsWith("events"),
  );

const ANY = "__any__";
const CUSTOM = "__custom__";

type EditTarget = {
  project: AutomationProject;
  entry: AutomationEntry;
  // repo: the repository automation `replace` names (its own file or a list
  // item); store: the schedule's id names it; move: a hand-set schedule
  // going into the repository.
  mode: "repo" | "store" | "move";
  replace?: string;
  // The live row, when there is one (a held file has none).
  schedule: AutomationSchedule | null;
  title: string;
};

export function AutomationsPageClient() {
  const sp = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const project = sp.get("project") || "";
  const q = useApi<AutomationsPayload>("automations", { refresh: 30000 });
  const [adding, setAdding] = useState<string | null>(null);
  const [editing, setEditing] = useState<EditTarget | null>(null);
  const go = (p: string) =>
    router.replace(pathname + query({ project: p || undefined }), {
      scroll: false,
    });
  const all = useMemo(() => q.data?.projects ?? [], [q.data]);
  const shown = useMemo(() => (project ? all.filter((p) => p.slug === project) : all), [all, project]);
  const schedules = useMemo(() => shown.flatMap((p) => p.schedules), [shown]);

  return (
    <div className="min-w-0">
      <PageHeader
        title="Automations"
        subtitle="Work that runs on a schedule"
        action={
          <Button onClick={() => setAdding(project || "")}>
            <PlusIcon /> New automation
          </Button>
        }
      />
      <div className="mb-4 flex min-w-0 flex-wrap items-center justify-between gap-3">
        <Select
          value={project || "all"}
          onValueChange={(v) => go(v === "all" ? "" : v)}
        >
          <SelectTrigger
            id="automations-project"
            aria-label="Project"
            className="w-full sm:w-48"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All projects</SelectItem>
            {all.map((p) => (
              <SelectItem key={p.slug} value={p.slug}>
                {p.slug}
              </SelectItem>
            ))}
            {project && !all.some((p) => p.slug === project) ? (
              <SelectItem value={project}>{project}</SelectItem>
            ) : null}
          </SelectContent>
        </Select>
      </div>

      {q.data ? <WeekStrip schedules={schedules} /> : null}

      <div className="grid min-w-0 gap-6">
        {q.error && !q.data ? (
          <ErrorNotice error={q.error} onRetry={() => void q.reload()}>
            {"The automations didn't load (" +
              q.error +
              "). Check that Hive is running, then try again."}
          </ErrorNotice>
        ) : null}
        {!q.data && !q.error ? (
          <SkeletonRows n={4} />
        ) : q.data && shown.length === 0 ? (
          <EmptyState
            title={
              project
                ? "Nothing scheduled for " + project
                : "Nothing scheduled yet"
            }
            icon={<CalendarClockIcon className="size-[18px]" />}
            action={
              <Button size="sm" variant="outline" onClick={() => setAdding(project || "")}>
                <PlusIcon /> New automation
              </Button>
            }
          >
            {project
              ? "This project has no standing jobs and no checkout on this machine to keep them in. Register a checkout, or add one that lives in the store."
              : "A standing job runs on a cadence without anyone asking: a nightly audit, a weekly dependency check, a watch on a page that must stay up. Add one and it is written into the project's repository as .hive/automations/<name>.yaml."}
          </EmptyState>
        ) : q.data ? (
          shown.map((p) => (
            <ProjectGroup
              key={p.slug}
              p={p}
              single={shown.length === 1}
              onAdd={() => setAdding(p.slug)}
              onEdit={setEditing}
            />
          ))
        ) : null}
      </div>

      {q.data ? (
        <>
          <AutomationDialog
            open={adding !== null}
            onOpenChange={(o) => (o ? null : setAdding(null))}
            data={q.data}
            project={adding ?? ""}
          />
          <AutomationDialog
            open={!!editing}
            onOpenChange={(o) => (o ? null : setEditing(null))}
            data={q.data}
            project={editing?.project.slug ?? ""}
            target={editing}
          />
        </>
      ) : null}
    </div>
  );
}

// ----- the week strip -----------------------------------------------------

const STRIP_DOTS = 5;

// The next seven days as cells: weekday and day number, today with the teal
// ring, and one small dot per run due that day (five at most, then "+N").
// The cell's label names every run, so a reader without the dots has it.
function WeekStrip({ schedules }: { schedules: AutomationSchedule[] }) {
  const now = useNow(60000);
  const days = useMemo(() => weekStrip(schedules, new Date(now)), [schedules, now]);
  return (
    <ol className="mb-6 grid grid-cols-7 gap-1.5 sm:gap-2" aria-label="The next seven days">
      {days.map((d) => (
        <WeekCell key={d.key} d={d} />
      ))}
    </ol>
  );
}

function WeekCell({ d }: { d: WeekDay }) {
  const n = d.runs.length;
  const label =
    d.weekday +
    " " +
    d.day +
    (d.today ? ", today" : "") +
    ": " +
    (n ? plural(n, "run") + " - " + d.runs.map((r) => r.title + " at " + r.at.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })).join(", ") : "nothing scheduled");
  return (
    <li
      className={cn(
        "surface-raised flex min-w-0 flex-col items-center gap-1.5 rounded-lg border px-1 py-2 text-center",
        d.today && "ring-2 ring-ring ring-offset-2 ring-offset-background",
      )}
      aria-label={label}
      title={label}
    >
      <span className={cn("text-xs leading-4", d.today ? "font-medium text-foreground" : "text-muted-foreground")}>{d.weekday}</span>
      <span className="tnum text-sm font-semibold leading-5 text-foreground">{d.day}</span>
      <span className="flex h-2 items-center justify-center gap-1" aria-hidden>
        {d.runs.slice(0, STRIP_DOTS).map((r, i) => (
          <i key={r.id + ":" + i} className="block size-1.5 rounded-full bg-primary" />
        ))}
        {n > STRIP_DOTS ? <span className="tnum text-xs leading-none text-muted-foreground">+{n - STRIP_DOTS}</span> : null}
      </span>
    </li>
  );
}

// ----- one project --------------------------------------------------------

// One card's worth of an automation: a live schedule, or a file of
// `.hive/automations/` that no schedule runs from (held or not applied yet).
type Card =
  | { key: string; status: string; failing: boolean; kind: "schedule"; s: AutomationSchedule }
  | { key: string; status: string; failing: boolean; kind: "file"; f: AutomationRepoFile };

// A project's automations, one card each, held and failing ones first and
// the ended ones folded away at the end.
function ProjectGroup({
  p,
  single,
  onAdd,
  onEdit,
}: {
  p: AutomationProject;
  single: boolean;
  onAdd: () => void;
  onEdit: (t: EditTarget) => void;
}) {
  const [showEnded, setShowEnded] = useState(false);
  const listProblems = p.file?.problems ?? [];
  const otherProblems = daemonProblems(p);
  const dir = p.automations?.dir ?? ".hive/automations";
  const cards = useMemo(() => {
    const out: Card[] = p.schedules.map((s) => ({ key: s.id, status: s.status, failing: !!problemOf(s), kind: "schedule", s }));
    // Files without a live schedule: held (the daemon refused them) or not
    // applied yet. A file a schedule runs from is the schedule's card.
    for (const f of p.automations?.files ?? []) {
      const st = fileState(p, f);
      if (st.row === null) out.push({ key: "file:" + f.path, status: st.state, failing: st.state === "held", kind: "file", f });
    }
    return sortAutomations(out);
  }, [p]);
  const live = cards.filter((c) => c.status !== "ended");
  const ended = cards.filter((c) => c.status === "ended");
  return (
    <section className="grid min-w-0 gap-3" aria-label={p.name || p.slug}>
      <SectionHeading
        as={single ? "h2" : "h3"}
        count={live.length}
        note={<FileLine p={p} />}
        action={
          <Button size="sm" variant="ghost" onClick={onAdd}>
            <PlusIcon /> Add
          </Button>
        }
      >
        <Link href={hrefs.project(p.slug)} className="hover:underline">
          {p.name || p.slug}
        </Link>
      </SectionHeading>
      {listProblems.length ? (
        <Notice tone="warn">
          <p className="font-medium">
            {p.file?.path ?? ".hive/hive.yaml"} has problems the daemon will
            not apply:
          </p>
          <ul className="mt-1 list-disc pl-4">
            {listProblems.map((x) => (
              <li key={x} className="anywhere">
                {plainProblem(x)}
              </li>
            ))}
          </ul>
        </Notice>
      ) : null}
      {otherProblems.length ? (
        <Notice tone="warn">
          <p className="font-medium">
            The daemon&apos;s last pass over the repository found problems:
          </p>
          <ul className="mt-1 list-disc pl-4">
            {otherProblems.map((x) => (
              <li key={x} className="anywhere">
                {plainProblem(x)}
              </li>
            ))}
          </ul>
        </Notice>
      ) : null}
      {live.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {p.repo
            ? "Nothing live. Add one and it goes into " +
              dir +
              "/ on " +
              p.repo.branch +
              "."
            : "Nothing live, and no checkout on this machine to keep a file in."}
        </p>
      ) : (
        <CardGrid>
          {live.map((c) =>
            c.kind === "schedule" ? (
              <ScheduleCard key={c.key} p={p} s={c.s} onEdit={onEdit} />
            ) : (
              <FileCard key={c.key} p={p} f={c.f} onEdit={onEdit} />
            ),
          )}
        </CardGrid>
      )}
      {ended.length ? (
        <Disclosure
          summary={plural(ended.length, "ended automation")}
          open={showEnded}
          onToggle={setShowEnded}
          contentClassName="grid gap-3"
        >
          <CardGrid>
            {ended.map((c) =>
              c.kind === "schedule" ? (
                <ScheduleCard key={c.key} p={p} s={c.s} onEdit={onEdit} />
              ) : null,
            )}
          </CardGrid>
        </Disclosure>
      ) : null}
    </section>
  );
}

function CardGrid({ children }: { children: React.ReactNode }) {
  return <div className="grid min-w-0 gap-3 md:grid-cols-2">{children}</div>;
}

// Where the project keeps its automations, in a line: the folder (how many
// files), the older list file if there is one, when the daemon last read
// them. Nothing when there is no checkout here.
function FileLine({ p }: { p: AutomationProject }) {
  if (!p.repo) return <span>No checkout on this machine</span>;
  const files = p.automations?.files ?? [];
  const dir = p.automations?.dir ?? ".hive/automations";
  if (!files.length && !p.file)
    return (
      <span>
        No <Mono>{dir}/</Mono> on {p.repo.branch} yet
      </span>
    );
  const n = p.file?.entries?.length ?? 0;
  return (
    <span className="inline-flex flex-wrap items-center gap-x-1.5">
      <FileCode2Icon className="size-3.5" aria-hidden />
      {files.length ? (
        <>
          <Mono>{dir}/</Mono>
          <span>{plural(files.length, "file")}</span>
        </>
      ) : null}
      {p.file ? (
        <>
          {files.length ? <span aria-hidden>·</span> : null}
          <Mono>{p.file.path}</Mono>
          <span>
            {p.file.entries
              ? plural(n, "entry").replace("entrys", "entries")
              : "unreadable"}
          </span>
        </>
      ) : null}
      {p.applied?.at ? (
        <>
          <span aria-hidden>·</span>
          <span>
            read <When d={p.applied.at} />
          </span>
        </>
      ) : null}
    </span>
  );
}

// ----- the card ----------------------------------------------------------

// The frame every automation card shares: the kind tile, the title and the
// schedule, a status on the right, then the body and one row of actions.
function AutomationCard({
  kind,
  title,
  schedule,
  status,
  problem,
  children,
  actions,
  ariaLabel,
}: {
  kind: AutomationSchedule["kind"];
  title: string;
  schedule: string;
  status?: React.ReactNode;
  problem?: Problem | null;
  children?: React.ReactNode;
  actions: React.ReactNode;
  ariaLabel: string;
}) {
  return (
    <article
      className={cn(
        "surface-raised relative flex min-w-0 flex-col gap-3 rounded-lg border p-4 text-left",
        problem?.tone === "failed" && "border-[color-mix(in_oklch,var(--color-status-failed)_40%,var(--border))]",
      )}
      aria-label={ariaLabel}
    >
      <div className="flex min-w-0 items-start gap-3">
        <KindTile kind={automationKind({ kind })} size={28} title={KIND_WORDS[kind]?.label ?? kind} className="mt-0.5" />
        <div className="min-w-0 flex-1">
          <h3 className="min-w-0 truncate text-sm font-semibold leading-5 text-foreground" title={title}>
            {title}
          </h3>
          <p className="min-w-0 truncate text-[13px] leading-5 text-muted-foreground">{schedule || "\u2014"}</p>
        </div>
        {status ? <div className="flex shrink-0 items-center gap-1.5">{status}</div> : null}
      </div>
      {problem ? (
        <p className="flex min-w-0 items-start gap-2 text-[13px] leading-5 text-foreground">
          <StatusBadge status={problem.tone} size="sm" className="mt-px" />
          <span className="min-w-0 anywhere">{problem.text}</span>
        </p>
      ) : null}
      {children}
      <div
        className="mt-auto flex min-w-0 flex-wrap items-center gap-2 pt-1"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") e.stopPropagation();
        }}
      >
        {actions}
      </div>
    </article>
  );
}

// The last five results as small icons, oldest on the left. From the
// automation's history when it has loaded; until then (or when the history
// is empty) the one result the list payload carries for a check or a watch.
function LastResults({ s }: { s: AutomationSchedule }) {
  const q = useApi<AutomationHistory>(automationHistoryPath(s.id, { limit: 12 }), { refresh: 60000 });
  const entries = q.data?.entries;
  const r = useMemo(() => lastResults(entries, s), [entries, s]);
  if (!r.results.length) {
    return <span className="text-[13px] text-muted-foreground">{q.data || q.error ? "No results yet" : "\u2026"}</span>;
  }
  const label = r.from === "schedule" || r.results.length === 1 ? "Last result" : "Last " + r.results.length + " results";
  return (
    <span className="inline-flex min-w-0 items-center gap-1.5 text-[13px] text-muted-foreground" aria-label={label}>
      <span className="inline-flex items-center gap-1">
        {r.results.map((x, i) => (
          <Tooltip key={x.at + ":" + i}>
            <TooltipTrigger asChild>
              <span className="inline-flex">
                <StatusIcon status={x.status} className="size-3.5" />
              </span>
            </TooltipTrigger>
            <TooltipContent>{x.detail + ", " + localWhen(x.at)}</TooltipContent>
          </Tooltip>
        ))}
      </span>
      <span className="truncate">{label.toLowerCase()}</span>
    </span>
  );
}

// A file of `.hive/automations/` that no schedule runs from: the daemon
// holds it (something is wrong) or has not applied it yet. Its problems
// show here, and Fix opens the form on it.
function FileCard({
  p,
  f,
  onEdit,
}: {
  p: AutomationProject;
  f: AutomationRepoFile;
  onEdit: (t: EditTarget) => void;
}) {
  const st = fileState(p, f);
  const problems = fileProblems(p, f).map(plainProblem);
  const e = f.entry;
  const held = st.state === "held";
  const edit = e
    ? () =>
        onEdit({
          project: p,
          entry: entryFromFile(e),
          mode: "repo",
          replace: f.name,
          schedule: null,
          title: e.title || f.name,
        })
    : null;
  return (
    <AutomationCard
      kind={e?.kind || "task"}
      title={e?.title || f.name}
      schedule={e ? scheduleWords({ every_minutes: everyMinutes(e.every) ?? 0, at_time: e.at }) || cadenceWords(e.every, e.at) : "Unreadable"}
      status={<StatusBadge status={held ? "warning" : "pending"} label={held ? "Held" : "Not applied yet"} size="sm" title={"The daemon's view of " + f.path} />}
      problem={held ? fileProblem(problems, !!e) : null}
      ariaLabel={e?.title || f.name}
      actions={
        <>
          {edit ? (
            <Button size="sm" variant={held ? "default" : "outline"} onClick={edit} title={"Edit " + f.path}>
              {held ? <WrenchIcon /> : <PencilIcon />} {held ? "Fix" : "Edit"}
            </Button>
          ) : null}
          <span className="ml-auto inline-flex min-w-0 items-center gap-1 text-[13px] text-muted-foreground" title={f.path}>
            <FileCode2Icon className="size-3.5" aria-hidden />
            <span className="truncate">{f.path}</span>
          </span>
        </>
      }
    >
      {problems.length > 1 ? (
        <ul className="list-disc pl-4 text-[13px] text-muted-foreground">
          {problems.slice(1).map((x) => (
            <li key={x} className="anywhere">
              {x}
            </li>
          ))}
        </ul>
      ) : null}
      {st.state === "pending" ? <p className="text-[13px] text-muted-foreground">The daemon applies it on its next pass.</p> : null}
    </AutomationCard>
  );
}

// ----- one schedule -------------------------------------------------------

// The next start: "in 3 h", or "due now" once the daemon is behind it.
function NextRun({ s }: { s: AutomationSchedule }) {
  const now = useNow();
  const words = nextRunWords(s, now);
  if (!words) return null;
  return (
    <span className="inline-flex items-center gap-1 whitespace-nowrap" title={localWhen(s.next_at, new Date(now))}>
      <CalendarClockIcon className="size-3.5" aria-hidden />
      {words === "due now" ? "Due now" : "Next " + words}
    </span>
  );
}

function ScheduleCard({
  p,
  s,
  onEdit,
}: {
  p: AutomationProject;
  s: AutomationSchedule;
  onEdit: (t: EditTarget) => void;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [ending, setEnding] = useState(false);
  const [history, setHistory] = useState(false);
  const live = s.status !== "ended";
  const outside = scriptOutside(s);
  const fromFile = !!s.origin_path;
  const fileEntry = fromFile ? fileEntryOf(p, s) : null;
  const problem = live ? problemOf(s) : null;
  const state = stateWords(s);
  const act = async (action: "pause" | "resume" | "run") => {
    setBusy(action);
    try {
      const r = await writes.automationAction(s.id, action);
      toast.success(
        action === "run"
          ? "Running " + s.title + " now."
          : action === "pause"
            ? s.title + " is paused."
            : s.title + " is back on.",
        {
          description:
            action === "run"
              ? "The job shows under Tasks in a moment."
              : fromFile
                ? "The file keeps its entry; the daemon honours the pause until you resume."
                : r.warning || undefined,
        },
      );
      refreshAutomations();
    } catch (e) {
      showApiError(e, "That didn't work");
    } finally {
      setBusy(null);
    }
  };
  const edit = () =>
    onEdit({
      project: p,
      entry: fileEntry ?? entryFromSchedule(s),
      mode: fromFile ? "repo" : "store",
      replace: fromFile ? s.origin_name : s.id,
      schedule: s,
      title: s.title,
    });
  const move = () =>
    onEdit({
      project: p,
      entry: entryFromSchedule(s),
      mode: "move",
      replace: s.id,
      schedule: s,
      title: s.title,
    });
  // The one visible action: Fix when something is wrong (resume a paused
  // one, else open the entry to edit it), Run now otherwise; the rest sits
  // in the menu.
  const canEdit = live && (!fromFile || !!fileEntry);
  const fix = s.status === "paused" ? () => void act("resume") : canEdit ? edit : () => setHistory(true);
  const stateTip = state ? (s.kind === "check" ? "The check is " : "The watch is ") + state.text + "." : undefined;
  return (
    <>
      <AutomationCard
        kind={s.kind}
        title={s.title}
        schedule={scheduleWords(s) || s.every_words || cadenceWords(s.every, s.at_time)}
        status={
          <>
            {outside ? (
              <Tooltip>
                <TooltipTrigger asChild>
                  <span className="inline-flex" role="img" aria-label="Script needs a move">
                    <TriangleAlertIcon className="size-4 text-status-warning" strokeWidth={1.75} aria-hidden />
                  </span>
                </TooltipTrigger>
                <TooltipContent>
                  {"Its script " + s.script + " sits outside .hive/. The place for it is " + (s.script_home || scriptHome(s.origin_name || s.id)) + "."}
                </TooltipContent>
              </Tooltip>
            ) : null}
            <StatusBadge status={s.status} size="sm" title={stateTip} />
          </>
        }
        problem={problem}
        ariaLabel={s.title}
        actions={
          <>
            {live && problem ? (
              <Button size="sm" onClick={fix} disabled={!!busy} title={s.status === "paused" ? "Turn it back on" : "Open it to put it right"}>
                {busy === "resume" ? <Loader2Icon className="animate-spin" /> : <WrenchIcon />} {s.status === "paused" ? "Resume" : "Fix"}
              </Button>
            ) : live && s.status === "paused" ? (
              <Button size="sm" variant="outline" onClick={() => void act("resume")} disabled={!!busy} title="Turn it back on">
                {busy === "resume" ? <Loader2Icon className="animate-spin" /> : <PlayIcon />} Resume
              </Button>
            ) : live ? (
              <Button size="sm" variant="outline" onClick={() => void act("run")} disabled={!!busy} title="Queue one job now, without waiting for the cadence">
                {busy === "run" ? <Loader2Icon className="animate-spin" /> : <PlayIcon />} Run now
              </Button>
            ) : null}
            <DropdownMenu>
              <Tooltip>
                <TooltipTrigger asChild>
                  <DropdownMenuTrigger asChild>
                    <Button size="icon-sm" variant="ghost" className="ml-auto" aria-label={"More for " + s.title} disabled={!!busy}>
                      {busy && busy !== "run" && busy !== "resume" ? <Loader2Icon className="animate-spin" /> : <MoreHorizontalIcon />}
                    </Button>
                  </DropdownMenuTrigger>
                </TooltipTrigger>
                <TooltipContent>More</TooltipContent>
              </Tooltip>
              <DropdownMenuContent align="end" className="w-60">
                <DropdownMenuItem onSelect={() => setHistory(true)} aria-label={"History of " + s.title}>
                  <HistoryIcon /> History
                </DropdownMenuItem>
                {live && (problem || s.status === "paused") ? (
                  <DropdownMenuItem onSelect={() => void act("run")} aria-label={"Run " + s.title + " now"}>
                    <PlayIcon /> Run now
                  </DropdownMenuItem>
                ) : null}
                {live && s.status !== "paused" ? (
                  <DropdownMenuItem onSelect={() => void act("pause")} aria-label={"Pause " + s.title}>
                    <PauseIcon /> Pause
                  </DropdownMenuItem>
                ) : null}
                {canEdit ? (
                  <DropdownMenuItem onSelect={edit} aria-label={"Edit " + s.title}>
                    <PencilIcon /> Edit
                  </DropdownMenuItem>
                ) : null}
                {live && !fromFile && p.repo ? (
                  <DropdownMenuItem onSelect={move} aria-label={"Move " + s.title + " into the repository"}>
                    <FileCode2Icon /> Move into the repository
                  </DropdownMenuItem>
                ) : null}
                {live ? (
                  <>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem variant="destructive" onSelect={() => setEnding(true)} aria-label={"End " + s.title}>
                      <SquareIcon /> End it
                    </DropdownMenuItem>
                  </>
                ) : null}
              </DropdownMenuContent>
            </DropdownMenu>
          </>
        }
      >
        <div className="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-1.5 text-[13px] leading-5 text-muted-foreground">
          {live && s.status === "active" ? <NextRun s={s} /> : null}
          <LastResults s={s} />
          <span
            className="inline-flex min-w-0 items-center gap-1"
            title={fromFile ? "Kept by the file; edit here or in the repository" : "Lives in the store on this machine"}
          >
            {fromFile ? <FileCode2Icon className="size-3.5 shrink-0" aria-hidden /> : <PencilIcon className="size-3.5 shrink-0" aria-hidden />}
            <span className="truncate">{keptWords(s)}</span>
          </span>
          {s.status === "paused" && s.note ? <span className="anywhere">paused: {s.note}</span> : null}
          {s.status === "ended" && s.note ? <span className="anywhere">ended: {s.note}</span> : null}
          {s.max_runs ? (
            <span className="tnum">
              {num(s.runs)} of {num(s.max_runs)} runs
            </span>
          ) : null}
          {s.agent ? (
            <span>
              as <Mono className="text-xs">{s.agent}</Mono>
            </span>
          ) : null}
        </div>
      </AutomationCard>
      <HistoryDialog
        open={history}
        onOpenChange={setHistory}
        s={s}
      />
      <ConfirmDialog
        open={ending}
        onOpenChange={setEnding}
        title={"End " + s.title + "?"}
        description={
          fromFile && fileEntry
            ? (inFolder(s.origin_path)
                ? "Its file " + s.origin_path + " is removed"
                : "Its entry in " + s.origin_path + " is removed") +
              " in a commit on " +
              (p.repo?.branch ?? "the default branch") +
              " and the schedule stops. You can add it back any time."
            : "The schedule stops and stays in the list as ended, with a note. Nothing that it queued is touched."
        }
        action="End it"
        destructive
        words={{
          label: "Why (optional)",
          placeholder: fromFile
            ? ""
            : "What replaces it, or why it is no longer needed",
        }}
        onConfirm={(note) => writes.automationAction(s.id, "end", note)}
        success={s.title + " ended."}
        invalidates={(path) =>
          path.startsWith("automations") ||
          path === "runner" ||
          path.startsWith("events")
        }
      />
    </>
  );
}

// ----- history ------------------------------------------------------------

const HISTORY_PAGE = 50;

// What happened to one automation, newest first: each run of its script
// (exit code, time, state), the job it queued or found open and where that
// job stands now, its red/green turns, and every change by hand or by the
// file. The last output sits at the top, as it is the thing most often
// wanted. Older pages load on demand.
function HistoryDialog({
  open,
  onOpenChange,
  s,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  s: AutomationSchedule;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[92dvh] overflow-y-auto sm:max-w-3xl">
        {open ? <HistoryPanel s={s} /> : null}
      </DialogContent>
    </Dialog>
  );
}

function HistoryPanel({ s }: { s: AutomationSchedule }) {
  const q = useApi<AutomationHistory>(
    automationHistoryPath(s.id, { limit: HISTORY_PAGE }),
    { refresh: 15000 },
  );
  // Older pages, fetched once each and kept below the live first page.
  const [older, setOlder] = useState<AutomationHistoryEntry[]>([]);
  const [olderMore, setOlderMore] = useState<boolean | null>(null);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [olderError, setOlderError] = useState("");
  const first = q.data?.entries ?? [];
  const seen = new Set(first.map((e) => e.seq));
  const entries = [...first, ...older.filter((e) => !seen.has(e.seq))];
  const more = olderMore ?? q.data?.more ?? false;
  const kind = q.data?.schedule.kind ?? s.kind;
  const loadOlder = async () => {
    const last = entries[entries.length - 1];
    if (!last) return;
    setLoadingOlder(true);
    setOlderError("");
    try {
      const r = await api<AutomationHistory>(
        automationHistoryPath(s.id, { before: last.seq, limit: HISTORY_PAGE }),
      );
      setOlder((cur) => [...cur, ...r.entries]);
      setOlderMore(r.more);
    } catch (e) {
      setOlderError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoadingOlder(false);
    }
  };
  const row = q.data?.schedule ?? s;
  const script = isScriptKind(row.kind);
  const state = stateWords(row);
  return (
    <>
      <DialogHeader>
        <DialogTitle className="flex flex-wrap items-center gap-2">
          <HistoryIcon className="size-4" aria-hidden />
          {row.title}
          <StatusBadge status={row.status} />
          {state ? <ToneBadge tone={state.tone}>{state.text}</ToneBadge> : null}
        </DialogTitle>
        <DialogDescription className="anywhere">
          <Mono className="text-xs">{row.id}</Mono>{" "}
          {row.every_words || cadenceWords(row.every, row.at_time)}
          {" · " + (KIND_WORDS[row.kind]?.label ?? row.kind)}
          {script ? (
            <>
              {" · runs "}
              <Mono className="text-xs">{row.script}</Mono>
            </>
          ) : null}
          {" · " + keptWords(row)}
          {row.runs
            ? " · " +
              plural(row.runs, "run") +
              (row.max_runs ? " of " + num(row.max_runs) : "") +
              " in all"
            : " · not run yet"}
        </DialogDescription>
      </DialogHeader>
      {q.error && !q.data ? (
        <ErrorNotice error={q.error} onRetry={() => void q.reload()}>
          {"The history didn't load (" + q.error + ")."}
        </ErrorNotice>
      ) : !q.data ? (
        <SkeletonRows n={5} />
      ) : (
        <div className="grid gap-4 text-sm">
          {script && q.data.last_output ? (
            <section className="grid gap-1.5" aria-label="Last output">
              <p className="text-xs font-medium text-muted-foreground">
                Last output
                {row.last_rc !== null && row.last_rc !== undefined
                  ? " · exit " + row.last_rc
                  : ""}
                {row.last_at ? (
                  <>
                    {" · "}
                    <When d={row.last_at} />
                  </>
                ) : null}
              </p>
              <pre className="max-h-48 overflow-auto rounded-lg border bg-muted/30 p-3 font-mono text-xs leading-relaxed whitespace-pre-wrap anywhere">
                {q.data.last_output}
              </pre>
            </section>
          ) : null}
          <section className="grid gap-1.5" aria-label="What happened">
            <p className="text-xs font-medium text-muted-foreground">
              What happened · {historySummary(entries, kind)}
            </p>
            {entries.length === 0 ? (
              <p className="text-muted-foreground">
                Nothing yet. The first run shows here when its time comes.
              </p>
            ) : (
              <ol className="divide-y rounded-lg border">
                {entries.map((e) => (
                  <HistoryLine key={e.seq} e={e} kind={kind} />
                ))}
              </ol>
            )}
            {more ? (
              <div className="flex items-center gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => void loadOlder()}
                  disabled={loadingOlder}
                >
                  {loadingOlder ? (
                    <Loader2Icon className="animate-spin" />
                  ) : null}{" "}
                  Older
                </Button>
                {olderError ? (
                  <span className="text-xs text-bad anywhere">{olderError}</span>
                ) : null}
              </div>
            ) : null}
          </section>
          {q.data.jobs.length ? (
            <section className="grid gap-1.5" aria-label="Jobs">
              <p className="text-xs font-medium text-muted-foreground">
                Jobs it queued · {plural(q.data.jobs.length, "job")}
              </p>
              <ul className="divide-y rounded-lg border">
                {q.data.jobs.slice(0, 20).map((j) => (
                  <li
                    key={j.id}
                    className="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-1.5"
                  >
                    <Link
                      href={hrefs.task(j.id)}
                      className="hover:underline"
                      title={j.title}
                    >
                      <Mono className="text-xs">{j.id}</Mono>
                    </Link>
                    <StatusBadge status={j.status} />
                    <span className="min-w-0 flex-1 truncate text-muted-foreground">
                      {j.title}
                    </span>
                    {j.agent ? (
                      <Mono className="text-xs">{j.agent}</Mono>
                    ) : null}
                    {j.finished_at || j.created_at ? (
                      <When d={j.finished_at || j.created_at} className="text-xs text-muted-foreground" />
                    ) : null}
                  </li>
                ))}
              </ul>
              {q.data.jobs.length > 20 ? (
                <p className="text-xs text-muted-foreground">
                  The newest 20 of {num(q.data.jobs.length)}; the rest are
                  under Tasks.
                </p>
              ) : null}
            </section>
          ) : null}
        </div>
      )}
    </>
  );
}

const HISTORY_MARK: Record<string, string> = {
  ok: "bg-ok",
  bad: "bg-bad",
  warn: "bg-warn",
  info: "bg-info",
  neutral: "bg-muted-foreground/40",
};

function HistoryLine({
  e,
  kind,
}: {
  e: AutomationHistoryEntry;
  kind: string;
}) {
  const w = historyWords(e, kind);
  return (
    <li className="grid gap-0.5 px-3 py-2 sm:grid-cols-[auto_minmax(0,1fr)_auto] sm:items-start sm:gap-x-3">
      <span
        className={cn(
          "mt-1.5 inline-block size-2 shrink-0 rounded-full",
          HISTORY_MARK[w.tone],
        )}
        aria-hidden
      />
      <div className="min-w-0">
        <p className="font-medium">{w.title}</p>
        {w.detail ? (
          <p className="text-muted-foreground anywhere">{w.detail}</p>
        ) : null}
        {e.job && (e.what === "fired" || e.what === "skipped" || e.what === "ran") ? (
          <p className="flex flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
            <Link href={hrefs.task(e.job)} className="hover:underline">
              <Mono className="text-xs">{e.job}</Mono>
            </Link>
            {e.job_status ? <StatusBadge status={e.job_status} /> : null}
            {e.job_agent ? (
              <span>
                by <Mono className="text-xs">{e.job_agent}</Mono>
              </span>
            ) : null}
            {e.job_finished_at ? (
              <span>
                finished <When d={e.job_finished_at} />
              </span>
            ) : null}
          </p>
        ) : null}
      </div>
      <When d={e.at} className="text-xs text-muted-foreground" />
    </li>
  );
}

// ----- the form -----------------------------------------------------------

// New, edit (repo entry or store row) and move share one form. The hub
// checks the draft as the owner types (debounced) and shows the YAML it
// would write; the local hints only save a round trip for the obvious.
function AutomationDialog({
  open,
  onOpenChange,
  data,
  project,
  target,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  data: AutomationsPayload;
  project: string;
  target?: EditTarget | null;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[92dvh] overflow-y-auto sm:max-w-4xl">
        {open ? (
          <AutomationForm
            data={data}
            project={project}
            target={target ?? null}
            close={() => onOpenChange(false)}
          />
        ) : null}
      </DialogContent>
    </Dialog>
  );
}

type CheckState = {
  yaml: string;
  problems: string[];
  warning: string;
  path: string;
  branch: string;
  form: "file" | "list";
} | null;

function AutomationForm({
  data,
  project: initialProject,
  target,
  close,
}: {
  data: AutomationsPayload;
  project: string;
  target: EditTarget | null;
  close: () => void;
}) {
  const projects = data.projects;
  const [project, setProject] = useState(
    initialProject ||
      projects.find((p) => p.repo)?.slug ||
      projects[0]?.slug ||
      "",
  );
  const [e, setE] = useState<AutomationEntry>(() =>
    target ? target.entry : { ...EMPTY_ENTRY },
  );
  const [nameTouched, setNameTouched] = useState(!!target);
  const [customEvery, setCustomEvery] = useState(
    () =>
      !!target && !EVERY_PRESETS.some((x) => x.value === target.entry.every),
  );
  const [more, setMore] = useState(
    () =>
      !!target &&
      (target.entry.tier !== 1 ||
        target.entry.priority !== 0 ||
        !!target.entry.run_as ||
        target.entry.times !== 0 ||
        target.entry.paused),
  );
  const [check, setCheck] = useState<CheckState>(null);
  const [checking, setChecking] = useState(false);
  const [busy, setBusy] = useState<AutomationPlace | null>(null);
  const [tried, setTried] = useState(false);
  const [serverProblems, setServerProblems] = useState<string[] | null>(null);
  const set = <K extends keyof AutomationEntry>(
    k: K,
    v: AutomationEntry[K],
  ) => {
    setServerProblems(null);
    setE((cur) => ({ ...cur, [k]: v }));
  };
  const p = projects.find((x) => x.slug === project) ?? null;
  const hasRepo = !!p?.repo;
  const mode = target?.mode ?? "new";
  const where: AutomationPlace | null =
    mode === "repo" || mode === "move"
      ? "repo"
      : mode === "store"
        ? "store"
        : null;
  const clean = useMemo(() => cleanEntry(e), [e]);
  // The script the automation already runs: an edit may keep it where it
  // is, even outside .hive/.
  const keptScript = target?.schedule?.script ?? target?.entry.script ?? "";
  const hints = entryHints(clean, data.limits.min_minutes, keptScript);
  const minutes = everyMinutes(clean.every);
  const daily = minutes !== null && minutes % 1440 === 0;
  const script = isScriptKind(clean.kind);
  const kinds = [
    ...data.kinds,
    ...data.script_kinds.filter((k) => !data.kinds.includes(k)),
  ];
  const payload = useMemo(
    () => ({
      project,
      entry: clean,
      where: where ?? ("repo" as const),
      replace: mode === "repo" ? target?.replace || undefined : undefined,
      id: mode === "store" ? target?.schedule?.id : undefined,
      move: mode === "move" ? target?.schedule?.id : undefined,
    }),
    [project, clean, where, mode, target],
  );
  const payloadKey = JSON.stringify(payload);

  // The hub's view of the draft, half a second after the last keystroke.
  useEffect(() => {
    if (!project) return;
    let gone = false;
    const id = setTimeout(() => {
      setChecking(true);
      writes
        .checkAutomation(JSON.parse(payloadKey))
        .then((r) => {
          if (!gone) setCheck(r);
        })
        .catch(() => {
          if (!gone) setCheck(null);
        })
        .finally(() => {
          if (!gone) setChecking(false);
        });
    }, 450);
    return () => {
      gone = true;
      clearTimeout(id);
    };
  }, [payloadKey, project]);

  const problems =
    serverProblems ?? (check && check.problems.length ? check.problems : hints);
  // An untouched form lists what it needs quietly; red is for real problems.
  const blank = !clean.name && !clean.brief && !clean.script && !clean.skill;
  const ready =
    !!project && hints.length === 0 && !!clean.name && !!clean.every;

  const submit = async (place: AutomationPlace) => {
    setTried(true);
    if (!ready) return;
    setBusy(place);
    const verb =
      mode === "new" ? "Adding" : mode === "move" ? "Moving" : "Saving";
    const tid = toast.loading(verb + " " + (clean.title || clean.name) + "…");
    try {
      const r = await writes.writeAutomation({ ...payload, where: place });
      const description =
        r.where === "repo"
          ? r.commit
            ? "Committed to " +
              r.branch +
              " as " +
              String(r.commit).slice(0, 10) +
              " (not pushed). The daemon applied it" +
              (r.synced?.problems?.length
                ? ", with problems: " + r.synced.problems.join("; ")
                : ".")
            : "The file already said this; nothing to commit."
          : "It lives in the store on this machine; " +
            (r.schedule?.next_at
              ? "next run " +
                localWhen(r.schedule.next_at) +
                "."
              : "it starts on its cadence.");
      toast.success(
        (clean.title || clean.name) +
          (mode === "new"
            ? " added."
            : mode === "move"
              ? " is in the repository."
              : " saved."),
        { id: tid, description, duration: 8000 },
      );
      if (r.warning) toast.message(r.warning, { duration: 8000 });
      refreshAutomations();
      close();
    } catch (err) {
      toast.dismiss(tid);
      if (
        err instanceof ApiError &&
        Array.isArray(err.body.problems) &&
        err.body.problems.length
      )
        setServerProblems(err.body.problems.map(String));
      showApiError(
        err,
        mode === "new"
          ? "The automation wasn't added"
          : "The automation wasn't saved",
      );
    } finally {
      setBusy(null);
    }
  };

  const copy = async () => {
    const text = check?.yaml ?? "";
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      toast.success("YAML copied.", {
        description: check?.path
          ? check.form === "list"
            ? "Paste it under schedules: in " + check.path + "."
            : "Save it as " + check.path + "."
          : "Save it as .hive/automations/<name>.yaml.",
      });
    } catch {
      toast.error("Couldn't copy. Select the text and copy it by hand.");
    }
  };

  const title =
    mode === "new"
      ? "New automation"
      : mode === "move"
        ? "Move " + target?.title + " into the repository"
        : "Edit " + target?.title;
  const skills = p?.options.skills ?? [];
  const agents = p?.options.agents ?? [];
  const everyValue = customEvery
    ? CUSTOM
    : EVERY_PRESETS.some((x) => x.value === e.every)
      ? e.every
      : CUSTOM;

  return (
    <>
      <DialogHeader>
        <DialogTitle>{title}</DialogTitle>
        <DialogDescription>
          {mode === "new"
            ? "A job that runs on a cadence. Written into the project's repository as .hive/automations/<name>.yaml with one commit on its default branch (nothing is pushed), or kept in the store on this machine."
            : mode === "move"
              ? "The schedule becomes a file of the project's .hive/automations folder, committed on its default branch (nothing is pushed); the hand-set copy ends."
              : where === "repo"
                ? "Saved as a new commit on " +
                  (p?.repo?.branch ?? "the default branch") +
                  " (nothing is pushed); the daemon applies it at once."
                : "Saved in the store on this machine, the way hive schedule would."}
        </DialogDescription>
      </DialogHeader>
      <div className="grid gap-5 md:grid-cols-[minmax(0,1fr)_minmax(0,22rem)]">
        <div className="grid content-start gap-4">
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="grid content-start gap-1.5">
              <Label htmlFor="auto-project">Project</Label>
              <Select
                value={project}
                onValueChange={setProject}
                disabled={mode !== "new"}
              >
                <SelectTrigger id="auto-project" className="w-full">
                  <SelectValue placeholder="Pick a project" />
                </SelectTrigger>
                <SelectContent>
                  {projects.map((x) => (
                    <SelectItem key={x.slug} value={x.slug}>
                      {x.slug}
                      {x.repo ? "" : " (no checkout here)"}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                {p?.repo
                  ? "Its automations: " +
                    (p.automations?.dir ?? ".hive/automations") +
                    "/ on " +
                    p.repo.branch +
                    "."
                  : p
                    ? "No checkout on this machine, so it can only live in the store."
                    : ""}
              </p>
            </div>
            <div className="grid content-start gap-1.5">
              <Label htmlFor="auto-kind">What it does</Label>
              <Select
                value={e.kind}
                onValueChange={(v) => {
                  set("kind", v);
                  if (isScriptKind(v) && !e.job_kind) set("job_kind", "task");
                }}
              >
                <SelectTrigger id="auto-kind" className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {kinds.map((k) => (
                    <SelectItem key={k} value={k}>
                      {KIND_WORDS[k]?.label ?? k}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="text-xs text-muted-foreground">
                {KIND_WORDS[e.kind]?.hint ?? ""}
              </p>
            </div>
          </div>
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="grid content-start gap-1.5">
              <Label htmlFor="auto-title">Title</Label>
              <Input
                id="auto-title"
                value={e.title}
                onChange={(ev) => {
                  set("title", ev.target.value);
                  if (!nameTouched) set("name", slugify(ev.target.value));
                }}
                placeholder="e.g. Nightly dependency audit"
                maxLength={120}
                autoFocus={mode === "new"}
              />
              <p className="text-xs text-muted-foreground">
                What the task is called under Tasks.
              </p>
            </div>
            <div className="grid content-start gap-1.5">
              <Label htmlFor="auto-name">Name</Label>
              <Input
                id="auto-name"
                value={e.name}
                onChange={(ev) => {
                  setNameTouched(true);
                  set("name", ev.target.value.toLowerCase());
                }}
                placeholder="e.g. nightly-audit"
                maxLength={64}
                spellCheck={false}
                className={cn(
                  "font-mono text-[13px]",
                  e.name && !isSlug(e.name) && "border-destructive",
                )}
              />
              <p className="text-xs text-muted-foreground">
                Lowercase, digits and dashes; the file is named after it. Unique
                in the project.
              </p>
            </div>
          </div>
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="grid content-start gap-1.5">
              <Label htmlFor="auto-every">How often</Label>
              <div className="flex gap-2">
                <Select
                  value={everyValue}
                  onValueChange={(v) => {
                    if (v === CUSTOM) {
                      setCustomEvery(true);
                      return;
                    }
                    setCustomEvery(false);
                    set("every", v);
                  }}
                >
                  <SelectTrigger
                    id="auto-every"
                    className={cn("w-full", customEvery && "w-32 shrink-0")}
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {EVERY_PRESETS.map((x) => (
                      <SelectItem key={x.value} value={x.value}>
                        {x.label}
                      </SelectItem>
                    ))}
                    <SelectItem value={CUSTOM}>Custom…</SelectItem>
                  </SelectContent>
                </Select>
                {customEvery ? (
                  <Input
                    aria-label="Cadence"
                    value={e.every}
                    onChange={(ev) => set("every", ev.target.value)}
                    placeholder="90m, 3d, 2w"
                    className="font-mono text-[13px]"
                    spellCheck={false}
                  />
                ) : null}
              </div>
              <p className="text-xs text-muted-foreground">
                {clean.every
                  ? cadenceWords(clean.every, daily ? clean.at : "") +
                    (data.limits.min_minutes
                      ? ". Shortest: " + data.limits.min_minutes + " min."
                      : ".")
                  : "30m, 6h, 1d, 2w, hourly, daily, weekly."}
              </p>
            </div>
            <div className="grid content-start gap-1.5">
              <Label htmlFor="auto-at">Time of day, in UTC</Label>
              <Input
                id="auto-at"
                value={e.at}
                onChange={(ev) => set("at", ev.target.value)}
                placeholder="e.g. 02:00"
                maxLength={5}
                disabled={!daily}
                className="font-mono text-[13px]"
                spellCheck={false}
              />
              <p className="text-xs text-muted-foreground">
                {daily
                  ? utcClockLocal(e.at)
                    ? "That's " + utcClockLocal(e.at) + " your time."
                    : "Optional. When a cadence of whole days fires."
                  : "Only for cadences of whole days."}
              </p>
            </div>
          </div>
          {script ? (
            <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_11rem]">
              <div className="grid content-start gap-1.5">
                <Label htmlFor="auto-script">Script</Label>
                <Input
                  id="auto-script"
                  value={e.script}
                  onChange={(ev) => set("script", ev.target.value)}
                  placeholder={scriptHome(clean.name || "check-site")}
                  className="font-mono text-[13px]"
                  spellCheck={false}
                />
                <p className="text-xs text-muted-foreground">
                  {"Under .hive/ in the repository, beside the automation's file (" +
                    scriptHome(clean.name) +
                    "), committed on " +
                    (p?.repo?.branch || "the default branch") +
                    ". " +
                    (e.kind === "check"
                      ? "Exit 0 is green; anything else is red and queues a job with its output."
                      : "Output means it fired; a job is queued with what it said.")}
                </p>
                {keptScript && !scriptInHive(keptScript) ? (
                  <p className="text-xs text-warn">
                    It runs <Mono className="text-xs">{keptScript}</Mono>{" "}
                    from outside .hive/ today; that keeps working. To move it,
                    commit the script at{" "}
                    <Mono className="text-xs">{scriptHome(clean.name)}</Mono>{" "}
                    first, then name that path here.
                  </p>
                ) : null}
              </div>
              <div className="grid content-start gap-1.5">
                <Label htmlFor="auto-job-kind">Then queue</Label>
                <Select
                  value={e.job_kind || "task"}
                  onValueChange={(v) => set("job_kind", v)}
                >
                  <SelectTrigger id="auto-job-kind" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {data.script_job_kinds.map((k) => (
                      <SelectItem key={k} value={k}>
                        {k === "task" ? "a task" : "an " + k}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  The job that fixes it.
                </p>
              </div>
            </div>
          ) : (
            <>
              <div className="grid content-start gap-1.5">
                <Label htmlFor="auto-brief">Brief</Label>
                <Textarea
                  id="auto-brief"
                  value={e.brief}
                  onChange={(ev) => set("brief", ev.target.value)}
                  rows={4}
                  className="min-h-24 text-[13px]"
                  placeholder={
                    "What each job does and what counts as done. Agents read this as the task; say where to look and what to leave behind."
                  }
                />
                <p className="text-xs text-muted-foreground">
                  Markdown is fine. Never a secret: the file is in the
                  repository.
                </p>
              </div>
              <div className="grid content-start gap-1.5">
                <Label htmlFor="auto-skill">Skill to follow</Label>
                <Select
                  value={e.skill || ANY}
                  onValueChange={(v) => set("skill", v === ANY ? "" : v)}
                >
                  <SelectTrigger id="auto-skill" className="w-full sm:w-72">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={ANY}>
                      None - the brief says it all
                    </SelectItem>
                    {e.skill && !skills.includes(e.skill) ? (
                      <SelectItem value={e.skill}>
                        {e.skill} (not found here)
                      </SelectItem>
                    ) : null}
                    {skills.map((k) => (
                      <SelectItem key={k} value={k}>
                        {k}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  A skill that is on for this project; the job reads it first.
                  With a skill, the brief may be short or empty.
                </p>
              </div>
            </>
          )}
          <Disclosure
            summary="More: tier, priority, who runs it, how many times"
            open={more}
            onToggle={setMore}
            contentClassName="grid gap-3"
          >
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="grid content-start gap-1.5">
                <Label htmlFor="auto-tier">Tier</Label>
                <Select
                  value={String(e.tier)}
                  onValueChange={(v) => set("tier", Number(v))}
                >
                  <SelectTrigger id="auto-tier" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="1">
                      1 - may change behaviour, lands through a pull request
                    </SelectItem>
                    <SelectItem value="0">
                      0 - small and safe, lands on its own
                    </SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="grid content-start gap-1.5">
                <Label htmlFor="auto-priority">Priority</Label>
                <Input
                  id="auto-priority"
                  type="number"
                  value={e.priority}
                  onChange={(ev) =>
                    set("priority", Number(ev.target.value) || 0)
                  }
                  className="tnum"
                />
                <p className="text-xs text-muted-foreground">
                  Higher runs first when the queue is full. 0 is normal.
                </p>
              </div>
              <div className="grid content-start gap-1.5">
                <Label htmlFor="auto-run-as">Who runs it</Label>
                <Select
                  value={e.run_as || ANY}
                  onValueChange={(v) => set("run_as", v === ANY ? "" : v)}
                >
                  <SelectTrigger id="auto-run-as" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={ANY}>Any agent</SelectItem>
                    {e.run_as && !agents.includes(e.run_as) ? (
                      <SelectItem value={e.run_as}>
                        {e.run_as} (not found here)
                      </SelectItem>
                    ) : null}
                    {agents.map((a) => (
                      <SelectItem key={a} value={a}>
                        {a}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  One of the project&apos;s custom agents, or whoever is free.
                </p>
              </div>
              <div className="grid content-start gap-1.5">
                <Label htmlFor="auto-times">How many times</Label>
                <Input
                  id="auto-times"
                  type="number"
                  min={0}
                  value={e.times}
                  onChange={(ev) =>
                    set("times", Math.max(0, Number(ev.target.value) || 0))
                  }
                  className="tnum"
                />
                <p className="text-xs text-muted-foreground">
                  0 keeps going; a number ends it after that many jobs.
                </p>
              </div>
            </div>
            <div className="flex items-center gap-3">
              <Switch
                id="auto-paused"
                checked={e.paused}
                onCheckedChange={(v) => set("paused", v)}
              />
              <Label htmlFor="auto-paused" className="font-normal">
                {mode === "new" ? "Start paused" : "Paused"}
              </Label>
            </div>
          </Disclosure>
        </div>

        <aside className="grid min-w-0 content-start gap-3 md:border-l md:pl-5">
          <p className="text-sm font-medium">Before it&apos;s saved</p>
          {!check && !checking ? <p className="text-xs text-muted-foreground">{project ? "Fill in a name and how often it runs; Hive checks it as you type." : "Pick a project first."}</p> : checking && !check ? <p className="inline-flex items-center gap-1.5 text-xs text-muted-foreground"><Loader2Icon className="size-3.5 animate-spin" aria-hidden /> Checking…</p> : null}
          {problems.length &&
          (tried ||
            serverProblems ||
            (check && check.problems.length && !blank)) ? (
            <Notice tone="warn">
              <ul className="list-disc pl-4">
                {problems.map((x) => (
                  <li key={x} className="anywhere">
                    {plainProblem(x)}
                  </li>
                ))}
              </ul>
            </Notice>
          ) : problems.length ? (
            <ul className="list-disc pl-4 text-xs text-muted-foreground">
              {problems.map((x) => (
                <li key={x} className="anywhere">
                  {plainProblem(x)}
                </li>
              ))}
            </ul>
          ) : check ? (
            <p className="inline-flex items-center gap-1.5 text-xs text-ok">
              <CheckIcon className="size-3.5" aria-hidden /> Passes every check.
            </p>
          ) : null}
          {check?.warning ? <Notice tone="info">{check.warning}</Notice> : null}
          {/* The YAML is what a reviewer reads in the repository; most owners never need it, so it waits behind a fold. */}
          <Disclosure summary={where === "store" ? "Show it as a Hive file would" : "Show the Hive file entry"} contentClassName="grid gap-2">
            <div className="flex items-center justify-end">
              <Button size="sm" variant="ghost" onClick={() => void copy()} disabled={!check?.yaml}>
                <CopyIcon /> Copy
              </Button>
            </div>
            <pre className="max-h-72 min-w-0 max-w-full overflow-auto rounded-xl border bg-muted/40 p-3 font-mono text-xs leading-relaxed whitespace-pre" aria-live="polite" aria-label="The Hive file entry">
              {check?.yaml || "# " + (checking ? "checking…" : project ? "fill in a name and how often it runs" : "pick a project")}
            </pre>
            {check?.path ? (
              <p className="text-xs text-muted-foreground">
                {check.form === "list" ? (
                  <>
                    Under <Mono>schedules:</Mono> in <Mono>{check.path}</Mono> on {check.branch || "the default branch"}.
                  </>
                ) : (
                  <>
                    As <Mono>{check.path}</Mono> on {check.branch || "the default branch"}.
                  </>
                )}
              </p>
            ) : null}
          </Disclosure>
        </aside>
      </div>
      <DialogFooter className="items-center gap-2 sm:justify-between">
        <Button variant="ghost" onClick={close} disabled={!!busy}>
          Cancel
        </Button>
        <div className="flex flex-wrap items-center justify-end gap-2">
          {mode === "new" ? (
            <>
              <Button
                variant="outline"
                onClick={() => void submit("store")}
                disabled={!!busy || !project || (tried && !ready)}
                title="Lives in the store on this machine, like hive schedule add"
              >
                {busy === "store" ? (
                  <Loader2Icon className="animate-spin" />
                ) : null}{" "}
                Keep in the store
              </Button>
              <Button
                onClick={() => void submit("repo")}
                disabled={!!busy || !hasRepo || (tried && !ready)}
                title={
                  hasRepo
                    ? "One commit on the default branch; nothing is pushed"
                    : "This project has no checkout on this machine"
                }
              >
                {busy === "repo" ? (
                  <Loader2Icon className="animate-spin" />
                ) : (
                  <FileCode2Icon />
                )}{" "}
                Add to the repository
              </Button>
            </>
          ) : (
            <Button
              onClick={() => void submit(where ?? "repo")}
              disabled={
                !!busy || (where === "repo" && !hasRepo) || (tried && !ready)
              }
            >
              {busy ? (
                <Loader2Icon className="animate-spin" />
              ) : mode === "move" ? (
                <FileCode2Icon />
              ) : (
                <CheckIcon />
              )}
              {mode === "move"
                ? "Move into the repository"
                : where === "repo"
                  ? "Commit the change"
                  : "Save"}
            </Button>
          )}
        </div>
      </DialogFooter>
    </>
  );
}
