"use client";

import Link from "next/link";
import { Suspense, useCallback, useEffect, useRef, useState } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { FolderInputIcon, GitBranchIcon, HistoryIcon, LayoutDashboardIcon, ListChecksIcon, MessagesSquareIcon, MoreHorizontalIcon, PencilLineIcon, PlusIcon, ScaleIcon, Trash2Icon, TrendingUpIcon, UsersIcon, type LucideIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { PageTitle, EmptyState, SkeletonPage, SkeletonRows } from "@/components/hive/panel";
import { CardGrid, ItemCard, ItemCardActions, ItemCardBody, ItemCardHeader, ItemCardKind, ItemCardMeta, ItemCardTitle } from "@/components/hive/item-card";
import { SectionHeading } from "@/components/hive/section-heading";
import { ProjectAvatar } from "@/components/hive/avatar";
import { BrandLogo } from "@/components/hive/logo";
import { StatusBadge } from "@/components/hive/status-badge";
import { Avatar, Mono, When } from "@/components/hive/who";
import { CommonsLine, ProjectCard } from "@/components/hive/cards";
import { PostSheet, ThreadSheet } from "@/components/board/sheets";
import { NewTaskDialog } from "@/components/tasks/task-dialogs";
import { AddProjectButton, MoveDialog, RemoveDialog, RenameDialog, RestoreButton } from "@/components/project/project-dialogs";
import { AmendmentsTab, ChatTab, CleanupTab, EvolutionTab, ForYouTab, HistoryActivity, NoticesTab, OverviewTab, SuggestionsTab, TasksTab, TeamTab, ThreadsTab, TimelineTab } from "@/components/project/tabs";
import { posterNames, projectHealth, RECENT_FAILURES_PAGE, recentFailures, repoChip, tabCountLabel, type CountedTab } from "@/components/project/project-state";
import { hrefs, PROJECT_TABS, resolveProjectTab, type ProjectCanonicalTab } from "@/lib/nav";
import { allProjects, projectPath, registeredProjects, type Org, type Overview, type ProjectInfo, type ProjectsPayload, type RegisteredProject, type TasksPayload } from "@/lib/api";
import { failedJobsPath, type FailedJobsPage } from "@/lib/job-repair";
import { useApi, useNow } from "@/lib/use-api";
import { useInbox } from "@/components/hive/inbox";
import { gateStep } from "@/components/hive/evolve";
import { num } from "@/lib/format";
import { cn } from "@/lib/utils";
import { ArchiveIcon, FolderOpenIcon } from "lucide-react";

type BoardSub = "threads" | "suggestions" | "amendments" | "notices";
type HistorySub = "timeline" | "cleanup";
type TeamSub = "team" | "env";

const BOARD_SUBS: { id: BoardSub; label: string; note: string }[] = [
  { id: "threads", label: "Threads", note: "Where agents pick up work, ask each other questions and report what they found." },
  { id: "suggestions", label: "Suggestions", note: "Ideas the agents vote on. A suggestion with enough votes becomes a job." },
  { id: "amendments", label: "Rule changes", note: "Proposed changes to the rules that self-improvement follows (the constitution)." },
  { id: "notices", label: "Notices", note: "What waits for votes across the boards, and what waits for you." },
];

const HISTORY_SUBS: { id: HistorySub; label: string; note: string }[] = [
  { id: "timeline", label: "Timeline", note: "Everything that happened here, newest first." },
  { id: "cleanup", label: "Cleanup", note: "" },
];

const TEAM_SUBS: { id: TeamSub; label: string; note: string }[] = [
  { id: "team", label: "Team and tools", note: "The lead agents who run this project and the tools and skills they work with." },
  { id: "env", label: "Setup and records", note: "How agents build and run the project, its rules, and the records they keep." },
];

const boardSub = (v: string | null | undefined): BoardSub => (v === "suggestions" || v === "amendments" || v === "notices" ? v : "threads");
const historySub = (v: string | null | undefined): HistorySub => (v === "cleanup" ? "cleanup" : "timeline");
const teamSub = (v: string | null | undefined): TeamSub => (v === "env" ? "env" : "team");

// The tab strip's icons, one per section (spec "Project" - tabs with icons
// and counts). The ids come from lib/nav.ts so links and aliases agree.
const TAB_ICON: Record<ProjectCanonicalTab, LucideIcon> = {
  overview: LayoutDashboardIcon,
  "for-you": ScaleIcon,
  work: ListChecksIcon,
  board: MessagesSquareIcon,
  evolution: TrendingUpIcon,
  team: UsersIcon,
  history: HistoryIcon,
};

const isCounted = (id: ProjectCanonicalTab): id is CountedTab => id === "for-you" || id === "work" || id === "board";

export default function ProjectPage() {
  return (
    <Suspense fallback={<SkeletonPage rows={4} />}>
      <ProjectScreen />
    </Suspense>
  );
}

function ProjectScreen() {
  const sp = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const slug = sp.get("slug") || "";
  const tabParam = sp.get("tab");
  const resolved = resolveProjectTab(tabParam);
  const tab: ProjectCanonicalTab = resolved.tab;
  const subParam = sp.get("sub") || resolved.sub;
  const board = boardSub(subParam);
  const history = historySub(subParam);
  const team = teamSub(subParam);
  const thread = sp.get("thread");
  const post = sp.get("post");
  const task = sp.get("task");
  const org = useApi<Org>("org");
  const taskSummary = useApi<TasksPayload>(slug ? "tasks?project=" + encodeURIComponent(slug) : null, { refresh: 30000 });
  // The header's repository chip and the faces of the agents who post here.
  const info = useApi<ProjectInfo>(slug && slug !== "commons" ? projectPath(slug, "project") : null, { refresh: 60000 });
  const ov = useApi<Overview>(slug ? projectPath(slug, "overview") : null);
  const failed = useApi<FailedJobsPage>(slug && slug !== "commons" ? failedJobsPath(slug, "", RECENT_FAILURES_PAGE) : null, { refresh: 30000 });
  const now = useNow(60000);
  // The registry (inactive rows included) only matters on the picker.
  const reg = useApi<ProjectsPayload>(!slug ? "projects" : null, { refresh: false });
  // The one "for you" count, shared with the home page and the sidebar.
  const inbox = useInbox();
  const [removing, setRemoving] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [moving, setMoving] = useState(false);
  const [newTask, setNewTask] = useState(false);

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

  // Older links opened a task over this page (`?task=` off the Work tab);
  // a task is its conversation on the Chat page now. On the Work tab the
  // same param still picks the task in the chat beside the list.
  useEffect(() => {
    if (slug && task && tab !== "work") router.replace(hrefs.task(task));
  }, [slug, task, tab, router]);

  useEffect(() => {
    if (!slug) return;
    const wanted = tab === "overview" ? null : tab;
    const wantedSub = tab === "board" ? board : tab === "history" ? history : tab === "team" && team !== "team" ? team : null;
    if ((tabParam || null) === wanted && (sp.get("sub") || null) === wantedSub) return;
    const q = new URLSearchParams(sp.toString());
    if (wanted) q.set("tab", wanted);
    else q.delete("tab");
    if (wantedSub) q.set("sub", wantedSub);
    else q.delete("sub");
    router.replace(pathname + (q.toString() ? "?" + q.toString() : ""), { scroll: false });
  }, [slug, tab, board, history, team, tabParam, sp, router, pathname]);

  if (!slug) {
    const projects = registeredProjects(org.data);
    const commons = org.data?.commons;
    const inactive = (reg.data?.projects ?? []).filter((p) => p.status !== "active");
    return (
      <>
        <PageTitle icon={FolderOpenIcon} sub="Every project Hive works on. Open one to give its lead agent a task, answer what waits for you and follow its board." actions={<AddProjectButton onRegistered={(r) => setParams({ slug: r.project.slug })} />}>
          Projects
        </PageTitle>
        {!org.data && org.loading ? (
          <SkeletonRows n={4} className="[&>*]:h-24 [&>*]:rounded-lg" />
        ) : (
          <div className="grid gap-10">
            {projects.length ? (
              <CardGrid cols={3}>
                {projects.map((p) => (
                  <ProjectCard key={p.slug} p={p} gate={inbox.gateOf(p)} forYou={inbox.forProject(p.slug).total} />
                ))}
              </CardGrid>
            ) : (
              <EmptyState title="Add your first project" action={<AddProjectButton size="sm" onRegistered={(r) => setParams({ slug: r.project.slug })} />}>
                Pick the folder of a git checkout on this machine. Its lead agent starts on the tasks you give it.
              </EmptyState>
            )}
            {commons ? <CommonsLine c={commons} /> : null}
            {inactive.length ? (
              <section aria-label="Removed projects">
                <SectionHeading icon={ArchiveIcon} count={inactive.length} note="Hive no longer works on these. Their posts and history are kept, and you can bring one back.">
                  Removed projects
                </SectionHeading>
                <CardGrid cols={3}>
                  {inactive.map((p) => (
                    <InactivePick key={p.slug} p={p} onRestored={(slug) => setParams({ slug })} />
                  ))}
                </CardGrid>
              </section>
            ) : null}
          </div>
        )}
      </>
    );
  }

  const card = allProjects(org.data).find((p) => p.slug === slug);
  const isCommons = card?.kind === "commons" || slug === "commons";
  const canDecide = org.data?.can_decide ?? true;
  const mine = card ? inbox.forProject(card.slug) : null;
  const forYou = mine?.total ?? 0;
  const openList = (taskSummary.data?.tasks ?? []).filter((t) => t.status !== "done" && t.status !== "cancelled");
  const openTasks = openList.length;
  // The project's own name leads; the slug (its id in every link) stands
  // beside it once the two differ, so a renamed project is still findable.
  const name = (card?.name || "").trim();
  const title = isCommons ? "Shared board" : name && name !== slug ? name : slug;
  const counts: Partial<Record<ProjectCanonicalTab, number>> = card ? { "for-you": forYou, work: openTasks, board: card.threads_open } : {};
  const removed = () => setParams({ slug: null, tab: null, sub: null, thread: null, post: null, task: null });

  // A slug Hive doesn't know (an old link, a typo, a removed project): say
  // so and offer the way back, instead of six tabs that each fail to load.
  if (!card && !isCommons && org.data) {
    const gone = (reg.data?.projects ?? []).some((p) => p.slug === slug);
    return (
      <>
        <PageTitle icon={FolderOpenIcon}>Project not found</PageTitle>
        <EmptyState
          title={"Hive doesn't know a project called " + slug}
          action={
            <Button asChild variant="outline" size="sm">
              <Link href="/project/">All projects</Link>
            </Button>
          }
        >
          {gone ? "It was removed. You can bring it back from the projects list." : org.error ? "The project list didn't load: " + org.error : "Check the link, or pick the project from the list."}
        </EmptyState>
      </>
    );
  }

  const chip = isCommons ? null : repoChip(info.data, card);
  const step = card && !isCommons ? gateStep(slug, inbox.gateOf(card)) : null;
  const health = card
    ? isCommons
      ? forYou
        ? { status: "waiting", label: forYou === 1 ? "1 thing waits for you" : forYou + " things wait for you" }
        : { status: "ok", label: "All quiet" }
      : projectHealth(card, { forYou, step: step?.label ?? null, failed: failed.error ? 0 : recentFailures(failed.data?.jobs, now) })
    : null;
  const posters = posterNames(ov.data?.agents ?? card?.active_claims ?? []);

  return (
    <Tabs value={tab} onValueChange={(v) => setParams({ tab: v === "overview" ? null : v, sub: null, thread: null, post: null })} className="gap-0">
      <header className="mb-6 min-w-0 pt-5 sm:pt-6" aria-label={title}>
        <div className="flex min-w-0 items-start justify-between gap-x-4 gap-y-3 max-sm:flex-wrap">
          <div className="flex min-w-0 flex-1 items-start gap-3.5">
            {isCommons ? (
              <span className="flex size-12 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground" aria-hidden>
                <UsersIcon className="size-6" />
              </span>
            ) : (
              <ProjectAvatar slug={slug} name={name || slug} size={48} className="shrink-0" />
            )}
            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 flex-wrap items-center gap-x-2.5 gap-y-1">
                <h1 className="min-w-0 truncate text-2xl font-semibold leading-8 tracking-tight anywhere" title={title}>
                  {title}
                </h1>
                {!isCommons && title !== slug ? <Mono className="text-[13px] text-muted-foreground">{slug}</Mono> : null}
                {health ? <StatusBadge status={health.status} label={health.label} size="sm" /> : null}
              </div>
              <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1.5 text-sm text-muted-foreground">
                {isCommons ? (
                  <span>The shared board is for work that touches more than one project. Agents from every project post, ask and vote here.</span>
                ) : card && !card.present ? (
                  <span className="anywhere">
                    Its folder <Mono>{card.path}</Mono> is not on this disk, so agents cannot work on it. If it moved or was cloned again, point Hive to the new folder.
                  </span>
                ) : chip ? (
                  <RepoChip chip={chip} head={info.data?.head} />
                ) : card ? (
                  <span className="anywhere">Software project at <Mono>{card.path}</Mono></span>
                ) : (
                  <span>Loading the project…</span>
                )}
                {posters.shown.length ? <Posters names={posters.shown} more={posters.more} /> : null}
              </div>
            </div>
          </div>
          {isCommons || !card ? null : (
            <div className="flex shrink-0 items-center gap-2 max-sm:w-full max-sm:justify-end sm:pt-1">
              {card.present ? null : (
                <Button variant="outline" onClick={() => setMoving(true)}>
                  <FolderInputIcon /> Point to its folder
                </Button>
              )}
              <DropdownMenu>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <DropdownMenuTrigger asChild>
                      <Button variant="outline" size="icon" aria-label={"More actions for " + slug}>
                        <MoreHorizontalIcon />
                      </Button>
                    </DropdownMenuTrigger>
                  </TooltipTrigger>
                  <TooltipContent>More actions</TooltipContent>
                </Tooltip>
                <DropdownMenuContent align="end" className="w-56">
                  <DropdownMenuItem onSelect={() => setRenaming(true)}>
                    <PencilLineIcon /> Rename project…
                  </DropdownMenuItem>
                  <DropdownMenuItem onSelect={() => setMoving(true)}>
                    <FolderInputIcon /> Change folder…
                  </DropdownMenuItem>
                  <DropdownMenuSeparator />
                  <DropdownMenuItem variant="destructive" onSelect={() => setRemoving(true)}>
                    <Trash2Icon /> Remove project…
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
              <Button onClick={() => setNewTask(true)} disabled={!card.present} title={card.present ? undefined : "Point Hive to the project's folder first."}>
                <PlusIcon /> New task
              </Button>
            </div>
          )}
        </div>
        <div className="mt-4 min-w-0">
          <SectionTabs tab={tab} counts={counts} />
        </div>
      </header>

      {/* A thread or a post opens in a side panel docked to the right of the tab's content (a sheet below a desktop). */}
      <div className="flex min-w-0 items-start gap-0 lg:gap-6">
        <div role="tabpanel" aria-label={PROJECT_TABS.find((t) => t.id === tab)?.label ?? tab} className="min-w-0 flex-1">
          {tab === "overview" ? <OverviewTab slug={slug} canDecide={canDecide} /> : null}
          {tab === "for-you" ? <ForYouTab slug={slug} canDecide={canDecide} /> : null}
          {tab === "work" ? <WorkTab slug={slug} task={task} setParams={setParams} /> : null}
          {tab === "board" ? <BoardTab slug={slug} value={board} onValueChange={(next) => setParams({ tab: "board", sub: next, thread: null, post: null })} thread={thread} setParams={setParams} /> : null}
          {tab === "evolution" ? <EvolutionTab slug={slug} canDecide={canDecide} /> : null}
          {tab === "team" ? <TeamSection slug={slug} value={team} canDecide={canDecide} onValueChange={(next) => setParams({ tab: "team", sub: next === "team" ? null : next })} /> : null}
          {tab === "history" ? <HistoryTab slug={slug} value={history} onValueChange={(next) => setParams({ tab: "history", sub: next })} /> : null}
        </div>
        <ThreadSheet slug={slug} threadKey={post ? null : thread} onClose={() => setParams({ thread: null })} />
        <PostSheet slug={slug} postId={post} onClose={() => setParams({ post: null })} />
      </div>
      {card && !isCommons ? <NewTaskDialog open={newTask} onOpenChange={setNewTask} project={slug} onCreated={(t) => setParams({ tab: "work", task: t.id })} /> : null}
      {card && !isCommons ? <RemoveDialog open={removing} onOpenChange={setRemoving} kind="project" name={slug} onApplied={removed} /> : null}
      {card && !isCommons ? <RenameDialog open={renaming} onOpenChange={setRenaming} slug={slug} name={card.name || slug} /> : null}
      {card && !isCommons ? <MoveDialog open={moving} onOpenChange={setMoving} slug={slug} name={card.name || slug} path={card.path} present={card.present} /> : null}
    </Tabs>
  );
}

// The repository chip under the title: the host's logo (GitHub when the
// remote is one, a branch icon otherwise), owner/name and the branch.
function RepoChip({ chip, head }: { chip: ReturnType<typeof repoChip>; head?: string }) {
  if (!chip) return null;
  const body = (
    <>
      {chip.host === "github" ? <BrandLogo id="github" size={16} /> : <GitBranchIcon className="size-4" aria-hidden />}
      <span className="truncate font-medium text-foreground">{chip.name}</span>
      {chip.branch ? (
        <>
          <span aria-hidden className="text-muted-foreground/60">·</span>
          <span className="inline-flex min-w-0 items-center gap-1 truncate">
            {chip.host === "github" ? <GitBranchIcon className="size-3.5 shrink-0" aria-hidden /> : null}
            <Mono className="truncate text-[13px]">{chip.branch}</Mono>
          </span>
        </>
      ) : null}
    </>
  );
  const cls = "inline-flex min-w-0 max-w-full items-center gap-1.5 rounded-md bg-muted px-2 py-0.5 text-[13px]";
  const label = (chip.host === "github" ? "GitHub repository " : "Repository ") + chip.name + (chip.branch ? ", branch " + chip.branch : "") + (head ? ", at commit " + head.slice(0, 8) : "");
  return chip.href ? (
    <a href={chip.href} target="_blank" rel="noreferrer" className={cn(cls, "hover:bg-muted/70 hover:text-foreground")} title={label} aria-label={label + " (opens in a new tab)"}>
      {body}
    </a>
  ) : (
    <span className={cls} title={label} aria-label={label}>
      {body}
    </span>
  );
}

// The agents who post here, five faces and a "+N" for the rest.
function Posters({ names, more }: { names: string[]; more: number }) {
  return (
    <span className="inline-flex items-center" aria-label={"Agents here: " + names.join(", ") + (more ? " and " + more + " more" : "")} title={names.join(", ") + (more ? " and " + more + " more" : "")}>
      <span className="flex items-center -space-x-1.5">
        {names.map((n) => (
          <Avatar key={n} seed={n} size={22} className="ring-2 ring-background" />
        ))}
      </span>
      {more ? <span className="tnum ml-1.5 text-xs font-medium text-muted-foreground">+{num(more)}</span> : null}
    </span>
  );
}

// The project's sections as underlined tabs with an icon and a count each.
// On a phone the seven don't fit: the strip scrolls sideways with a fading
// edge, and it scrolls itself (not the page) so the open section is always
// in view, also when a link opened it.
function SectionTabs({ tab, counts }: { tab: ProjectCanonicalTab; counts: Partial<Record<ProjectCanonicalTab, number>> }) {
  const strip = useRef<HTMLDivElement>(null);
  const countKey = PROJECT_TABS.map((t) => counts[t.id as ProjectCanonicalTab] || 0).join(",");
  useEffect(() => {
    const c = strip.current;
    const el = c?.querySelector<HTMLElement>('[role="tab"][data-state="active"]');
    if (!c || !el) return;
    const pad = 16;
    const left = el.offsetLeft - c.offsetLeft;
    const right = left + el.offsetWidth;
    if (left < c.scrollLeft + pad) c.scrollTo({ left: Math.max(0, left - pad), behavior: "smooth" });
    else if (right > c.scrollLeft + c.clientWidth - pad) c.scrollTo({ left: right - c.clientWidth + pad, behavior: "smooth" });
  }, [tab, countKey]);
  return (
    <div ref={strip} className="scroll-strip min-w-0 max-w-full">
      <TabsList aria-label="Project sections">
        {PROJECT_TABS.map((t) => {
          const id = t.id as ProjectCanonicalTab;
          const Icon = TAB_ICON[id];
          const n = counts[id];
          const you = id === "for-you";
          return (
            <TabsTrigger key={id} value={id} className="gap-1.5 px-3">
              <Icon className="size-4 shrink-0" aria-hidden />
              <span>{t.label}</span>
              {n ? (
                <>
                  <span aria-hidden className={cn("tnum inline-flex h-5 min-w-5 items-center justify-center rounded-full px-1.5 text-xs font-semibold", you ? "bg-kind-marigold-soft text-kind-marigold" : "bg-muted text-muted-foreground")}>
                    {num(n)}
                  </span>
                  {isCounted(id) ? <span className="sr-only">{", " + tabCountLabel(id, n)}</span> : null}
                </>
              ) : null}
            </TabsTrigger>
          );
        })}
      </TabsList>
    </div>
  );
}

function WorkTab({ slug, task, setParams }: { slug: string; task: string | null; setParams: (patch: Record<string, string | null | undefined>) => void }) {
  return (
    <div className="grid gap-8 xl:grid-cols-[minmax(0,1fr)_minmax(380px,1fr)]">
      <section aria-label="Tasks" className="min-w-0">
        <SectionHeading icon={ListChecksIcon} note="What you asked for, and how far each task has come.">Tasks</SectionHeading>
        <TasksTab slug={slug} onOpen={(id) => setParams({ task: id })} activeId={task} />
      </section>
      <section aria-label="Sessions" className="min-h-[520px] min-w-0">
        <SectionHeading icon={MessagesSquareIcon} note="Talk with the lead agent about the project or one task.">Sessions</SectionHeading>
        <ChatTab slug={slug} taskId={task} onPickTask={(id) => setParams({ task: id })} />
      </section>
    </div>
  );
}

function BoardTab({ slug, value, onValueChange, thread, setParams }: { slug: string; value: BoardSub; onValueChange: (next: BoardSub) => void; thread: string | null; setParams: (patch: Record<string, string | null | undefined>) => void }) {
  const note = BOARD_SUBS.find((s) => s.id === value)?.note;
  return (
    <Tabs value={value} onValueChange={(next) => onValueChange(boardSub(next))} className="gap-0">
      <div className="mb-5 grid gap-2">
        <div className="scroll-strip min-w-0 max-w-full">
          <TabsList aria-label="Board sections">
            {BOARD_SUBS.map((s) => (
              <TabsTrigger key={s.id} value={s.id} className="px-3">
                {s.label}
              </TabsTrigger>
            ))}
          </TabsList>
        </div>
        {note ? <p className="text-sm text-muted-foreground">{note}</p> : null}
      </div>
      {value === "threads" ? <ThreadsTab slug={slug} onOpen={(k) => setParams({ thread: k })} activeKey={thread} /> : null}
      {value === "suggestions" ? <SuggestionsTab slug={slug} /> : null}
      {value === "amendments" ? <AmendmentsTab slug={slug} /> : null}
      {value === "notices" ? <NoticesTab slug={slug} /> : null}
    </Tabs>
  );
}

// Team & tools: the leads and the toolkit, with the project's setup and
// records (the old History > "Setup and records") one segment away.
function TeamSection({ slug, value, onValueChange, canDecide }: { slug: string; value: TeamSub; onValueChange: (next: TeamSub) => void; canDecide: boolean }) {
  const note = TEAM_SUBS.find((s) => s.id === value)?.note;
  return (
    <Tabs value={value} onValueChange={(next) => onValueChange(teamSub(next))} className="gap-0">
      <div className="mb-5 grid gap-2">
        <div className="scroll-strip min-w-0 max-w-full">
          <TabsList aria-label="Team and tools sections">
            {TEAM_SUBS.map((s) => (
              <TabsTrigger key={s.id} value={s.id} className="px-3">
                {s.label}
              </TabsTrigger>
            ))}
          </TabsList>
        </div>
        {note ? <p className="text-sm text-muted-foreground">{note}</p> : null}
      </div>
      <TeamTab slug={slug} sub={value} canDecide={canDecide} />
    </Tabs>
  );
}

// History: the timeline first, then notices and the posting activity that
// used to crowd the Overview; cleanup keeps its own segment.
function HistoryTab({ slug, value, onValueChange }: { slug: string; value: HistorySub; onValueChange: (next: HistorySub) => void }) {
  const note = HISTORY_SUBS.find((s) => s.id === value)?.note;
  return (
    <Tabs value={value} onValueChange={(next) => onValueChange(historySub(next))} className="gap-0">
      <div className="mb-5 grid gap-2">
        <div className="scroll-strip min-w-0 max-w-full">
          <TabsList aria-label="History sections">
            {HISTORY_SUBS.map((s) => (
              <TabsTrigger key={s.id} value={s.id} className="px-3">
                {s.label}
              </TabsTrigger>
            ))}
          </TabsList>
        </div>
        {note ? <p className="text-sm text-muted-foreground">{note}</p> : null}
      </div>
      {value === "timeline" ? (
        <div className="grid gap-10">
          <section aria-label="Timeline" className="min-w-0">
            <SectionHeading icon={HistoryIcon}>Timeline</SectionHeading>
            <TimelineTab slug={slug} />
          </section>
          <HistoryActivity slug={slug} />
        </div>
      ) : null}
      {value === "cleanup" ? <CleanupTab slug={slug} /> : null}
    </Tabs>
  );
}

// One removed project: Hive no longer works on it, its history is kept,
// and Restore brings it back.
function InactivePick({ p, onRestored }: { p: RegisteredProject; onRestored: (slug: string) => void }) {
  return (
    <ItemCard ariaLabel={p.slug + " (removed)"}>
      <ItemCardHeader right={p.last_seen ? <span>Last seen <When d={p.last_seen} /></span> : null}>
        <ItemCardKind>Removed</ItemCardKind>
      </ItemCardHeader>
      <ItemCardTitle>{p.slug}</ItemCardTitle>
      {p.name && p.name !== p.slug ? <ItemCardBody clamp={1}>{p.name}</ItemCardBody> : null}
      <ItemCardMeta>
        <Mono className="text-muted-foreground">{p.path}</Mono>
      </ItemCardMeta>
      <ItemCardActions>
        <span className="text-[13px] text-muted-foreground">{p.present ? "Restoring adds it back from the same folder." : "Put the folder back first; Hive can't restore it without it."}</span>
        <span className="sm:ml-auto">
          <RestoreButton slug={p.slug} onRestored={(r) => onRestored(r.project.slug)} />
        </span>
      </ItemCardActions>
    </ItemCard>
  );
}
