"use client";

import { useEffect, useMemo, useState } from "react";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import {
  CalendarClockIcon,
  CheckIcon,
  CopyIcon,
  FileCode2Icon,
  FolderGit2Icon,
  Loader2Icon,
  PauseIcon,
  PencilIcon,
  PlayIcon,
  PlusIcon,
  ShieldCheckIcon,
  SquareIcon,
  TimerIcon,
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
import { ConfirmDialog, showApiError } from "@/components/pulse/confirm";
import {
  CardGrid,
  ItemCard,
  ItemCardActions,
  ItemCardBody,
  ItemCardMeta,
  ItemCardTitle,
  SectionHeading,
} from "@/components/pulse/item-card";
import {
  Disclosure,
  EmptyState,
  ErrorNotice,
  Notice,
  PageTitle,
  SkeletonRows,
} from "@/components/pulse/panel";
import { Mono, StatusBadge, ToneBadge, When } from "@/components/pulse/who";
import {
  ApiError,
  query,
  writes,
  type AutomationEntry,
  type AutomationPlace,
  type AutomationProject,
  type AutomationSchedule,
  type AutomationsPayload,
} from "@/lib/api";
import {
  EMPTY_ENTRY,
  EVERY_PRESETS,
  KIND_WORDS,
  cadenceWords,
  cleanEntry,
  entryFromSchedule,
  entryHints,
  everyMinutes,
  fileEntryOf,
  isScriptKind,
  isSlug,
  keptWords,
  slugify,
  sortRows,
  stateWords,
} from "@/lib/automations";
import { num, parseDate, plural } from "@/lib/format";
import { hrefs } from "@/lib/nav";
import { invalidate, useApi, useNow } from "@/lib/use-api";
import { cn } from "@/lib/utils";

// Automations: the standing jobs of each project - a task, an improvement,
// a check or a watch that runs on a cadence. The place to keep one is the
// project's `.pulse/pulse.yaml`, so it travels with the code and a review
// sees it; this page writes that file (one commit on the default branch,
// never a push) through the same checks the daemon applies it with. A
// schedule can also live in the store alone, the way `pulse schedule add`
// makes one; the page shows both and can move the second kind into the file.

export const refreshAutomations = () =>
  invalidate(
    (p) =>
      p === "automations" ||
      p.startsWith("automations?") ||
      p === "runner" ||
      p.startsWith("tasks") ||
      p.startsWith("events"),
  );

const ANY = "__any__";
const CUSTOM = "__custom__";

type EditTarget = {
  project: AutomationProject;
  entry: AutomationEntry;
  // repo: the file entry `replace` names; store: the schedule `id` names;
  // move: a hand-set schedule going into the file.
  mode: "repo" | "store" | "move";
  schedule: AutomationSchedule;
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
  const all = q.data?.projects ?? [];
  const shown = project ? all.filter((p) => p.slug === project) : all;
  const total = shown.reduce(
    (n, p) => n + p.schedules.filter((s) => s.status !== "ended").length,
    0,
  );
  const inFile = shown.reduce(
    (n, p) =>
      n +
      p.schedules.filter((s) => s.status !== "ended" && s.origin_path).length,
    0,
  );

  return (
    <div className="min-w-0">
      <PageTitle
        icon={CalendarClockIcon}
        sub="Standing jobs: a task, a check or a watch that runs on a schedule. Each is kept in the project's PULSE file, so it travels with the code, or set by hand here."
        actions={
          <Button onClick={() => setAdding(project || "")}>
            <PlusIcon /> New automation
          </Button>
        }
      >
        Automations
      </PageTitle>
      <div className="mb-4 flex min-w-0">
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

      <div className="grid min-w-0 gap-6">
        {q.error && !q.data ? (
          <ErrorNotice error={q.error} onRetry={() => void q.reload()}>
            {"The automations didn't load (" +
              q.error +
              "). Check that PULSE is running, then try again."}
          </ErrorNotice>
        ) : null}
        {!q.data && !q.error ? (
          <SkeletonRows n={4} />
        ) : q.data && shown.length === 0 ? (
          <>
            <EmptyState
              title={
                project
                  ? "Nothing scheduled for " + project
                  : "Nothing scheduled yet"
              }
              icon={<CalendarClockIcon className="size-[18px]" />}
              action={
                <Button size="sm" onClick={() => setAdding(project || "")}>
                  <PlusIcon /> New automation
                </Button>
              }
            >
              {project
                ? "This project has no standing jobs and no checkout on this machine to keep them in. Register a checkout, or add one that lives in the store."
                : "A standing job runs on a cadence without anyone asking: a nightly audit, a weekly dependency check, a watch on a page that must stay up. Add one and it is written into the project's PULSE file."}
            </EmptyState>
            <Terminal />
          </>
        ) : q.data ? (
          <>
            <p className="max-w-3xl text-sm leading-relaxed text-muted-foreground">
              {total === 0
                ? "Nothing is live right now."
                : plural(total, "live automation") +
                  (inFile === total
                    ? total === 1
                      ? ", kept in the project's PULSE file."
                      : ", all kept in the projects' PULSE files."
                    : inFile === 0
                      ? ", all set by hand."
                      : ", " +
                        num(inFile) +
                        " kept in a PULSE file and " +
                        num(total - inFile) +
                        " set by hand.")}{" "}
              The daemon starts each job when its time comes and shows it under
              Tasks.
            </p>
            {shown.map((p) => (
              <ProjectGroup
                key={p.slug}
                p={p}
                single={shown.length === 1}
                onAdd={() => setAdding(p.slug)}
                onEdit={setEditing}
              />
            ))}
            <Terminal />
          </>
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

// The terminal side of it, and where the file sits.
function Terminal() {
  return (
    <Disclosure
      summary="Prefer the terminal, or the file itself?"
      contentClassName="grid max-w-3xl gap-2 text-sm leading-relaxed text-muted-foreground anywhere"
    >
      <p>
        Every automation is an entry under <Mono>schedules:</Mono> in{" "}
        <Mono>.pulse/pulse.yaml</Mono> on the project&apos;s default branch.
        Edit the file and commit; the daemon applies it on its next pass, and{" "}
        <Mono>pulse manifest check</Mono> says what it would refuse.
      </p>
      <p>
        From a shell on this machine, a schedule that lives in the store alone:{" "}
        <Mono className="block w-fit max-w-full">
          pulse schedule add --project P --title &apos;Nightly audit&apos;
          --every 1d --at 02:00 --brief &apos;Audit the dependencies.&apos;
        </Mono>
      </p>
    </Disclosure>
  );
}

// ----- one project --------------------------------------------------------

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
  const rows = sortRows(p.schedules);
  const live = rows.filter((s) => s.status !== "ended");
  const ended = rows.filter((s) => s.status === "ended");
  const [showEnded, setShowEnded] = useState(false);
  const fileProblems = p.file?.problems ?? [];
  const appliedProblems = p.applied?.problems ?? [];
  return (
    <section className="grid min-w-0 gap-3" aria-label={p.name || p.slug}>
      <SectionHeading
        as={single ? "h2" : "h3"}
        count={live.length}
        icon={FolderGit2Icon}
        note={<FileLine p={p} />}
        action={
          <Button size="sm" variant="outline" onClick={onAdd}>
            <PlusIcon /> Add
          </Button>
        }
      >
        <Link href={hrefs.project(p.slug)} className="hover:underline">
          {p.name || p.slug}
        </Link>
      </SectionHeading>
      {fileProblems.length ? (
        <Notice tone="warn">
          <p className="font-medium">
            {p.file?.path ?? ".pulse/pulse.yaml"} has problems the daemon will
            not apply:
          </p>
          <ul className="mt-1 list-disc pl-4">
            {fileProblems.map((x) => (
              <li key={x} className="anywhere">
                {x}
              </li>
            ))}
          </ul>
        </Notice>
      ) : appliedProblems.length && !p.file ? (
        <Notice tone="warn">
          <p className="font-medium">
            The daemon&apos;s last pass over the repository found problems:
          </p>
          <ul className="mt-1 list-disc pl-4">
            {appliedProblems.map((x) => (
              <li key={x} className="anywhere">
                {x}
              </li>
            ))}
          </ul>
        </Notice>
      ) : null}
      {live.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {p.repo
            ? "Nothing live. Add one and it goes into " +
              (p.file?.path ?? ".pulse/pulse.yaml") +
              " on " +
              p.repo.branch +
              "."
            : "Nothing live, and no checkout on this machine to keep a file in."}
        </p>
      ) : (
        <CardGrid cols={2}>
          {live.map((s) => (
            <ScheduleCard key={s.id} p={p} s={s} onEdit={onEdit} />
          ))}
        </CardGrid>
      )}
      {ended.length ? (
        <Disclosure
          summary={plural(ended.length, "ended automation")}
          open={showEnded}
          onToggle={setShowEnded}
          contentClassName="grid gap-3"
        >
          <CardGrid cols={2}>
            {ended.map((s) => (
              <ScheduleCard key={s.id} p={p} s={s} onEdit={onEdit} />
            ))}
          </CardGrid>
        </Disclosure>
      ) : null}
    </section>
  );
}

// Where the file stands, in a line: the path, the branch, how many entries,
// when the daemon last applied it.
function FileLine({ p }: { p: AutomationProject }) {
  if (!p.repo) return <span>No checkout on this machine</span>;
  if (!p.file)
    return (
      <span>
        No <Mono>.pulse/pulse.yaml</Mono> on {p.repo.branch} yet
      </span>
    );
  const n = p.file.entries?.length ?? 0;
  return (
    <span className="inline-flex flex-wrap items-center gap-x-1.5">
      <FileCode2Icon className="size-3.5" aria-hidden />
      <Mono>{p.file.path}</Mono>
      <span>on {p.repo.branch}</span>
      <span aria-hidden>·</span>
      <span>
        {p.file.entries
          ? plural(n, "entry").replace("entrys", "entries")
          : "unreadable"}
      </span>
      {p.applied?.at ? (
        <>
          <span aria-hidden>·</span>
          <span>
            applied <When d={p.applied.at} />
          </span>
        </>
      ) : null}
    </span>
  );
}

// ----- one schedule -------------------------------------------------------

// The next start: a time still ahead, or "due now" once the daemon is behind it.
function NextRun({ at }: { at: string }) {
  const now = useNow();
  const t = parseDate(at);
  if (t && t.getTime() <= now) return <span title={at}>due now</span>;
  return (
    <span>
      next <When d={at} />
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
  const live = s.status !== "ended";
  const script = isScriptKind(s.kind);
  const fromFile = !!s.origin_path;
  const fileEntry = fromFile ? fileEntryOf(p, s) : null;
  const state = stateWords(s);
  const kind = KIND_WORDS[s.kind]?.label ?? s.kind;
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
                ? "The file keeps its entry; the daemon honours this until the file says otherwise."
                : r.schedule?.next_at
                  ? "Next run " +
                    r.schedule.next_at.replace("T", " ").replace("Z", " UTC") +
                    "."
                  : undefined,
        },
      );
      refreshAutomations();
    } catch (e) {
      showApiError(e, "That didn't work");
    } finally {
      setBusy(null);
    }
  };
  const edit = () => {
    if (fromFile && fileEntry)
      onEdit({ project: p, entry: fileEntry, mode: "repo", schedule: s });
    else
      onEdit({
        project: p,
        entry: entryFromSchedule(s),
        mode: "store",
        schedule: s,
      });
  };
  const move = () =>
    onEdit({
      project: p,
      entry: entryFromSchedule(s),
      mode: "move",
      schedule: s,
    });
  return (
    <ItemCard className={cn(!live && "opacity-70")} ariaLabel={s.title}>
      <div className="flex min-w-0 items-start justify-between gap-3">
        <div className="flex min-w-0 items-center gap-3">
          <span
            className={cn(
              "flex size-9 shrink-0 items-center justify-center rounded-lg",
              live
                ? "bg-primary/10 text-primary"
                : "bg-muted text-muted-foreground",
            )}
            aria-hidden
          >
            {script ? (
              <ShieldCheckIcon className="size-[18px]" />
            ) : s.kind === "improve" || s.kind === "clean" ? (
              <WrenchIcon className="size-[18px]" />
            ) : (
              <TimerIcon className="size-[18px]" />
            )}
          </span>
          <div className="min-w-0">
            <ItemCardTitle className={cn(!live && "text-muted-foreground")}>
              {s.title}
            </ItemCardTitle>
            <p className="text-xs text-muted-foreground">
              {s.every_words || cadenceWords(s.every, s.at_time)}
              {" · " + kind}
              {s.tier === 0 ? " · tier 0" : ""}
            </p>
          </div>
        </div>
        <div className="flex shrink-0 flex-col items-end gap-1">
          <StatusBadge status={s.status} className="mt-0.5" />
          {state ? <ToneBadge tone={state.tone}>{state.text}</ToneBadge> : null}
        </div>
      </div>
      <ItemCardBody clamp={2} className="text-muted-foreground">
        {script ? (
          <>
            Runs <Mono>{s.script}</Mono>
            {s.job_kind && s.job_kind !== "task"
              ? "; a " +
                (s.kind === "check" ? "red run" : "firing") +
                " queues an " +
                s.job_kind +
                " job."
              : "."}
          </>
        ) : s.skill ? (
          <>
            Follows the skill <Mono>{s.skill}</Mono>
            {s.brief ? ": " + s.brief : "."}
          </>
        ) : (
          s.brief || "No brief."
        )}
      </ItemCardBody>
      <ItemCardMeta>
        <span
          className="inline-flex items-center gap-1"
          title={
            fromFile
              ? "Kept by the file; edit here or in the repository"
              : "Lives in the store on this machine"
          }
        >
          {fromFile ? (
            <FileCode2Icon className="size-3.5" aria-hidden />
          ) : (
            <PencilIcon className="size-3.5" aria-hidden />
          )}
          {keptWords(s)}
          {fromFile && s.origin_name ? (
            <Mono className="text-[11px]">#{s.origin_name}</Mono>
          ) : null}
        </span>
        {live && s.status === "active" && s.next_at ? (
          <NextRun at={s.next_at} />
        ) : null}
        {s.status === "paused" && s.note ? (
          <span className="anywhere">paused: {s.note}</span>
        ) : null}
        {s.status === "ended" && s.note ? (
          <span className="anywhere">ended: {s.note}</span>
        ) : null}
        <span className="tnum">
          {s.runs
            ? plural(s.runs, "run") +
              (s.max_runs ? " of " + num(s.max_runs) : "")
            : "not run yet"}
        </span>
        {s.last_job ? (
          <Link href={hrefs.task(s.last_job)} className="hover:underline">
            last job <Mono className="text-[11px]">{s.last_job}</Mono>
          </Link>
        ) : null}
        {s.agent ? (
          <span>
            as <Mono className="text-[11px]">{s.agent}</Mono>
          </span>
        ) : null}
      </ItemCardMeta>
      {live ? (
        <ItemCardActions>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void act("run")}
            disabled={!!busy}
            title="Queue one job now, without waiting for the cadence"
          >
            {busy === "run" ? (
              <Loader2Icon className="animate-spin" />
            ) : (
              <PlayIcon />
            )}{" "}
            Run now
          </Button>
          {s.status === "paused" ? (
            <Button
              size="sm"
              variant="outline"
              onClick={() => void act("resume")}
              disabled={!!busy}
            >
              {busy === "resume" ? (
                <Loader2Icon className="animate-spin" />
              ) : (
                <PlayIcon />
              )}{" "}
              Resume
            </Button>
          ) : (
            <Button
              size="sm"
              variant="outline"
              onClick={() => void act("pause")}
              disabled={!!busy}
            >
              {busy === "pause" ? (
                <Loader2Icon className="animate-spin" />
              ) : (
                <PauseIcon />
              )}{" "}
              Pause
            </Button>
          )}
          {!fromFile || fileEntry ? (
            <Button size="sm" variant="ghost" onClick={edit} disabled={!!busy}>
              <PencilIcon /> Edit
            </Button>
          ) : null}
          {!fromFile && p.repo ? (
            <Button
              size="sm"
              variant="ghost"
              onClick={move}
              disabled={!!busy}
              title="Write it into the project's PULSE file, so it travels with the code"
            >
              <FileCode2Icon /> Into the file
            </Button>
          ) : null}
          <Button
            size="sm"
            variant="ghost"
            className="text-destructive hover:text-destructive"
            onClick={() => setEnding(true)}
            disabled={!!busy}
          >
            <SquareIcon /> End
          </Button>
        </ItemCardActions>
      ) : null}
      <ConfirmDialog
        open={ending}
        onOpenChange={setEnding}
        title={"End " + s.title + "?"}
        description={
          fromFile && fileEntry
            ? "Its entry is removed from " +
              s.origin_path +
              " on " +
              (p.repo?.branch ?? "the default branch") +
              " in one commit (nothing is pushed), and the schedule ends. Jobs it already started finish."
            : "The schedule ends and runs no more. Jobs it already started finish."
        }
        action="End"
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
    </ItemCard>
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
  const hints = entryHints(clean, data.limits.min_minutes);
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
      replace:
        mode === "repo" ? target?.schedule.origin_name || undefined : undefined,
      id: mode === "store" ? target?.schedule.id : undefined,
      move: mode === "move" ? target?.schedule.id : undefined,
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
                r.schedule.next_at.replace("T", " ").replace("Z", " UTC") +
                "."
              : "it starts on its cadence.");
      toast.success(
        (clean.title || clean.name) +
          (mode === "new"
            ? " added."
            : mode === "move"
              ? " is in the file."
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
        description: "Paste it under schedules: in .pulse/pulse.yaml.",
      });
    } catch {
      toast.error("Couldn't copy. Select the text and copy it by hand.");
    }
  };

  const title =
    mode === "new"
      ? "New automation"
      : mode === "move"
        ? "Move " + target?.schedule.title + " into the file"
        : "Edit " + target?.schedule.title;
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
            ? "A job that runs on a cadence. Written into the project's PULSE file with one commit on its default branch (nothing is pushed), or kept in the store on this machine."
            : mode === "move"
              ? "The schedule becomes an entry of the project's PULSE file, committed on its default branch (nothing is pushed); the hand-set copy ends."
              : where === "repo"
                ? "Saved as a new commit on " +
                  (p?.repo?.branch ?? "the default branch") +
                  " (nothing is pushed); the daemon applies it on its next pass."
                : "Saved in the store on this machine, the way pulse schedule would."}
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
                  ? "Its file: " +
                    (p.file?.path ?? ".pulse/pulse.yaml") +
                    " on " +
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
                placeholder="Nightly dependency audit"
                maxLength={120}
                autoFocus={mode === "new"}
              />
              <p className="text-xs text-muted-foreground">
                What the task is called under Tasks.
              </p>
            </div>
            <div className="grid content-start gap-1.5">
              <Label htmlFor="auto-name">Name in the file</Label>
              <Input
                id="auto-name"
                value={e.name}
                onChange={(ev) => {
                  setNameTouched(true);
                  set("name", ev.target.value.toLowerCase());
                }}
                placeholder="nightly-audit"
                maxLength={64}
                spellCheck={false}
                className={cn(
                  "font-mono text-[13px]",
                  e.name && !isSlug(e.name) && "border-destructive",
                )}
              />
              <p className="text-xs text-muted-foreground">
                Lowercase, digits and dashes. Unique in the file.
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
              <Label htmlFor="auto-at">Time of day (UTC)</Label>
              <Input
                id="auto-at"
                value={e.at}
                onChange={(ev) => set("at", ev.target.value)}
                placeholder="02:00"
                maxLength={5}
                disabled={!daily}
                className="font-mono text-[13px]"
                spellCheck={false}
              />
              <p className="text-xs text-muted-foreground">
                {daily
                  ? "Optional. When a cadence of whole days fires."
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
                  placeholder="scripts/check-site.sh"
                  className="font-mono text-[13px]"
                  spellCheck={false}
                />
                <p className="text-xs text-muted-foreground">
                  {e.kind === "check"
                    ? "Relative to the repository root. Exit 0 is green; anything else is red and queues a job with its output."
                    : "Relative to the repository root. Output means it fired; a job is queued with what it said."}
                </p>
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
          <div className="flex items-center justify-between gap-2">
            <p className="text-sm font-medium">
              {where === "store"
                ? "As the file would say it"
                : "What goes into the file"}
            </p>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void copy()}
              disabled={!check?.yaml}
            >
              <CopyIcon /> Copy
            </Button>
          </div>
          <pre
            className="max-h-72 min-w-0 max-w-full overflow-auto rounded-xl border bg-muted/40 p-3 font-mono text-[12px] leading-relaxed whitespace-pre"
            aria-live="polite"
            aria-label="YAML preview"
          >
            {check?.yaml ||
              "# " +
                (checking
                  ? "checking…"
                  : project
                    ? "fill in a name and a cadence"
                    : "pick a project")}
          </pre>
          {check?.path ? (
            <p className="text-xs text-muted-foreground">
              Under <Mono>schedules:</Mono> in <Mono>{check.path}</Mono> on{" "}
              {check.branch || "the default branch"}.
            </p>
          ) : null}
          {problems.length &&
          (tried ||
            serverProblems ||
            (check && check.problems.length && !blank)) ? (
            <Notice tone="warn">
              <ul className="list-disc pl-4">
                {problems.map((x) => (
                  <li key={x} className="anywhere">
                    {x}
                  </li>
                ))}
              </ul>
            </Notice>
          ) : problems.length ? (
            <ul className="list-disc pl-4 text-xs text-muted-foreground">
              {problems.map((x) => (
                <li key={x} className="anywhere">
                  {x}
                </li>
              ))}
            </ul>
          ) : check ? (
            <p className="inline-flex items-center gap-1.5 text-xs text-ok">
              <CheckIcon className="size-3.5" aria-hidden /> Passes every check.
            </p>
          ) : null}
          {check?.warning ? <Notice tone="info">{check.warning}</Notice> : null}
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
                title="Lives in the store on this machine, like pulse schedule add"
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
                Add to the file
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
                ? "Move into the file"
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
