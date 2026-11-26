"use client";

import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { ColumnsIcon, ListIcon, PlusIcon, RouteIcon, SearchIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ProjectAvatar } from "@/components/hive/avatar";
import { DataTable } from "@/components/hive/data-table";
import { PageHeader } from "@/components/hive/page-header";
import { EmptyState, ErrorNotice, SkeletonPage, SkeletonRows } from "@/components/hive/panel";
import { SectionHeading } from "@/components/hive/section-heading";
import { useViewMode } from "@/components/hive/view-toggle";
import { PhasesView } from "@/components/tasks/lifecycle";
import { TaskCard } from "@/components/tasks/task-card";
import { NewTaskDialog } from "@/components/tasks/task-dialogs";
import { TASK_COLUMNS } from "@/components/tasks/task-columns";
import { COLUMN_LABEL, groupByColumn, matchesStatus, TASK_COLUMNS_ORDER, wantsNewTask, type TaskColumn } from "@/components/tasks/tasks-state";
import { projectLabel, registeredProjects, taskProgress, TASK_STATUSES, type Org, type Task, type TasksPayload } from "@/lib/api";
import { andList, plural } from "@/lib/format";
import { hrefs } from "@/lib/nav";
import { statusWord } from "@/lib/words";
import { useApi } from "@/lib/use-api";
import { cn } from "@/lib/utils";

export default function TasksPage() {
  return (
    <Suspense fallback={<SkeletonPage rows={3} />}>
      <TasksScreen />
    </Suspense>
  );
}

const isActive = (t: Task) => t.status !== "done" && t.status !== "cancelled";

// The status filter keeps today's `status` search param (hrefs.tasks), so
// links from the palette and other pages work unchanged: no param means
// "active" (anything not closed); a status name; or "all". The board shows
// every column whatever the filter; the list narrows to it.
const STATUS_FILTERS: [string, string][] = [
  ["active", "Open"],
  ["review", statusWord("review")],
  ...TASK_STATUSES.filter((s) => s !== "review").map((s): [string, string] => [s, statusWord(s)]),
  ["all", "All"],
];

type View = "board" | "list" | "phases";

// Board or List, remembered per browser under the shared view-mode key
// ("cards" is the board, "table" the list); the board is the default.
// Phases (a project's tasks by lifecycle phase, TASK-141) rides in the URL
// as ?view=phases, so a link can open it.
function useView(phases: boolean, setPhases: (on: boolean) => void): [View, (v: View) => void] {
  const [mode, setMode] = useViewMode("tasks", "cards");
  const set = useCallback(
    (v: View) => {
      if (v === "phases") return setPhases(true);
      setMode(v === "list" ? "table" : "cards");
      if (phases) setPhases(false);
    },
    [setMode, phases, setPhases],
  );
  return [phases ? "phases" : mode === "table" ? "list" : "board", set];
}

// One sentence on the list in view: how many are open and what needs you.
function summary(scope: Task[]): string {
  const active = scope.filter(isActive);
  if (!scope.length) return "";
  if (!active.length) return "No open task right now. Everything you gave the agents is done or cancelled.";
  const review = active.filter((t) => t.status === "review").length;
  const working = active.filter((t) => taskProgress(t).working.length > 0).length;
  const bits = [review ? (review === 1 ? "1 waits for your decision" : review + " wait for your decision") : "", working ? (working === 1 ? "agents are working on 1 right now" : "agents are working on " + working + " right now") : ""].filter(Boolean);
  return plural(active.length, "open task") + (bits.length ? ": " + andList(bits) + "." : ".");
}

// A board column: its heading with the count, then the cards. "Asked you"
// cards carry their marigold edge; the column itself stays quiet.
function BoardColumn({ column, tasks, showProject, onOpen }: { column: TaskColumn; tasks: Task[]; showProject: boolean; onOpen: (t: Task) => void }) {
  return (
    <section aria-labelledby={"col-" + column} className="flex min-w-0 flex-col gap-3">
      <SectionHeading as="h2" id={"col-" + column} count={tasks.length} className="mb-0">
        {COLUMN_LABEL[column]}
      </SectionHeading>
      {tasks.length ? (
        <div className="grid min-w-0 gap-3">
          {tasks.map((t) => (
            <TaskCard key={t.id} t={t} showProject={showProject} onOpen={onOpen} compact={column === "done"} />
          ))}
        </div>
      ) : (
        <p className="rounded-lg border border-dashed px-3 py-4 text-center text-[13px] text-muted-foreground">{column === "review" ? "Nothing waits for you." : "Nothing here."}</p>
      )}
    </section>
  );
}

function TasksScreen() {
  const sp = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const project = sp.get("project") || "";
  const status = sp.get("status") || "active";
  // A task opens as its conversation on the Chat page; this page only lists.
  const openTask = useCallback((id: string) => router.push(hrefs.task(id)), [router]);
  // Older links opened the task here (`?task=`): send them on.
  const stale = sp.get("task");
  useEffect(() => {
    if (stale) router.replace(hrefs.task(stale));
  }, [stale, router]);
  const [text, setText] = useState("");
  const setParams = useCallback(
    (patch: Record<string, string | null | undefined>) => {
      const q = new URLSearchParams(sp.toString());
      for (const [k, v] of Object.entries(patch)) {
        if (v === null || v === undefined || v === "") q.delete(k);
        else q.set(k, v);
      }
      const s = q.toString();
      router.push(pathname + (s ? "?" + s : ""), { scroll: false });
    },
    [sp, router, pathname],
  );
  const setPhases = useCallback((on: boolean) => setParams({ view: on ? "phases" : null }), [setParams]);
  const [view, setView] = useView(sp.get("view") === "phases", setPhases);
  // /tasks/?new=1 opens the New task dialog on load (other pages link here
  // for it); closing it drops the param so a reload does not reopen it.
  const [creating, setCreating] = useState(false);
  const newParam = sp.get("new");
  // The dismissal holds until the URL has dropped the param (derived state,
  // reset during render), so a later ?new=1 on this same page opens it again.
  const [dismissedNew, setDismissedNew] = useState(false);
  if (dismissedNew && !wantsNewTask(newParam)) setDismissedNew(false);
  const wantsNew = wantsNewTask(newParam) && !dismissedNew;
  const onCreatingChange = (o: boolean) => {
    setCreating(o);
    if (!o && wantsNew) {
      setDismissedNew(true);
      setParams({ new: null });
    }
  };
  const q = useApi<TasksPayload>("tasks", { refresh: 10000 });
  const org = useApi<Org>("org", { refresh: 60000 });
  const projects = useMemo(() => (org.data ? registeredProjects(org.data).map((p) => ({ slug: p.slug, name: p.name })) : Array.from(new Set((q.data?.tasks ?? []).map((t) => t.project))).map((slug) => ({ slug, name: "" }))), [org.data, q.data]);
  const all = useMemo(() => q.data?.tasks ?? [], [q.data]);
  // Counts in the filter and the summary follow the project filter, so they
  // describe the list the owner is looking at rather than the whole org.
  const scope = useMemo(() => (project ? all.filter((x) => x.project === project) : all), [all, project]);
  const searched = useMemo(() => {
    const t = text.trim().toLowerCase();
    return scope.filter((x) => !t || x.title.toLowerCase().includes(t) || x.id.toLowerCase().includes(t) || (x.brief || "").toLowerCase().includes(t));
  }, [scope, text]);
  const rows = useMemo(() => searched.filter((x) => matchesStatus(x, status)), [searched, status]);
  // The board: "active" (the default) shows the four columns with every
  // open task and the recent Done ones; a narrower status shows the same
  // columns with only what matches, so the filter still means something.
  const board = useMemo(() => groupByColumn(status === "active" ? searched.filter((x) => isActive(x) || x.status === "done").slice() : rows), [searched, rows, status]);
  const by = (s: string) => (s === "all" ? scope.length : s === "active" ? scope.filter(isActive).length : scope.filter((x) => x.status === s).length);
  const filtered = status !== "active" || Boolean(text.trim());
  const empty = all.length ? (
    filtered ? (
      <>No task matches. Clear the search or pick another status.</>
    ) : (
      <>Every task you gave is done or cancelled. Give the agents a new one.</>
    )
  ) : (
    <>Nothing yet. Give the agents a task: pick the project and say what you want done. The project&apos;s lead agent splits it into steps and reports back in its session.</>
  );
  const emptyTitle = all.length ? (filtered ? "No task matches" : "No open task") : "No tasks yet";
  const emptyAction =
    all.length && filtered ? undefined : (
      <Button size="sm" onClick={() => setCreating(true)}>
        <PlusIcon /> New task
      </Button>
    );
  const line = q.data ? summary(scope) : "The work you gave your agents";
  const projectName = project ? projects.find((p) => p.slug === project) : undefined;
  return (
    <>
      <PageHeader
        title="Tasks"
        subtitle={line}
        action={
          <Button onClick={() => setCreating(true)}>
            <PlusIcon /> New task
          </Button>
        }
      />
      <NewTaskDialog open={creating || wantsNew} onOpenChange={onCreatingChange} project={project || undefined} onCreated={(t) => openTask(t.id)} />
      <div className="mb-5 flex min-w-0 flex-wrap items-center gap-2">
        <Tabs value={view} onValueChange={(v) => setView(v as View)} className="shrink-0">
          <TabsList variant="segmented" aria-label="View">
            <TabsTrigger value="board">
              <ColumnsIcon data-icon="inline-start" aria-hidden /> Board
            </TabsTrigger>
            <TabsTrigger value="list">
              <ListIcon data-icon="inline-start" aria-hidden /> List
            </TabsTrigger>
            <TabsTrigger value="phases" title="A project's tasks by lifecycle phase">
              <RouteIcon data-icon="inline-start" aria-hidden /> Phases
            </TabsTrigger>
          </TabsList>
        </Tabs>
        <Select value={project || "__all"} onValueChange={(v) => setParams({ project: v === "__all" ? null : v })}>
          <SelectTrigger aria-label="Project" className="w-auto max-w-56">
            <SelectValue placeholder="Every project">
              {project ? (
                <>
                  <ProjectAvatar slug={project} name={projectName?.name} size={20} />
                  <span className="truncate">{projectName ? projectLabel(projectName) : project}</span>
                </>
              ) : (
                "Every project"
              )}
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__all">Every project</SelectItem>
            {projects.map((p) => (
              <SelectItem key={p.slug} value={p.slug}>
                <ProjectAvatar slug={p.slug} name={p.name} size={20} />
                <span className="truncate">{projectLabel(p)}</span>
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select value={status} onValueChange={(v) => setParams({ status: v === "active" ? null : v })}>
          <SelectTrigger aria-label="Status" className="w-auto">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {STATUS_FILTERS.filter(([s]) => s === "active" || s === "all" || s === status || by(s) > 0).map(([s, word]) => (
              <SelectItem key={s} value={s}>
                <span>{word}</span>
                {q.data ? <span className="tnum text-xs text-muted-foreground">{by(s)}</span> : null}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <span className="relative min-w-0 flex-1 basis-56 sm:max-w-xs">
          <SearchIcon className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden />
          <Input type="search" aria-label="Search tasks" placeholder="Search tasks" value={text} onChange={(e) => setText(e.target.value)} className="pl-8" />
        </span>
        {q.data && filtered ? (
          <span className="ml-auto shrink-0 text-[13px] text-muted-foreground tnum" aria-live="polite">
            {plural(rows.length, "task")}
          </span>
        ) : null}
      </div>
      {q.error && !q.data ? (
        <ErrorNotice error={q.error} onRetry={() => void q.reload()}>
          {"The task list didn't load (" + q.error + "). Check that Hive is running, then try again."}
        </ErrorNotice>
      ) : !q.data ? (
        <SkeletonRows n={8} />
      ) : view === "phases" ? (
        <PhasesView project={project} tasks={status === "active" ? searched : rows} />
      ) : !rows.length && !(view === "board" && status === "active" && searched.length) ? (
        <EmptyState title={emptyTitle} action={emptyAction}>
          {empty}
        </EmptyState>
      ) : view === "list" ? (
        <DataTable columns={TASK_COLUMNS(!project)} rows={rows} rowKey={(t) => t.id} onRow={(t) => openTask(t.id)} initialSort="updated" caption="Tasks" className={cn("rounded-lg", "[&_td]:py-2 [&_tr]:h-11")} />
      ) : (
        <div className="grid min-w-0 grid-cols-1 gap-5 sm:grid-cols-2 xl:grid-cols-4" aria-label="Task board">
          {TASK_COLUMNS_ORDER.map((c) => (
            <BoardColumn key={c} column={c} tasks={board[c]} showProject={!project} onOpen={(t) => openTask(t.id)} />
          ))}
        </div>
      )}
    </>
  );
}
