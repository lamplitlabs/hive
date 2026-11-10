"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { ActivityIcon, ArrowRightIcon, BarChart3Icon, BellIcon, CheckCircle2Icon, FileDiffIcon, FolderGit2Icon, FolderIcon, FunnelIcon, LayoutGridIcon, ListIcon, MessageCircleQuestionIcon, MessageSquareIcon, MessagesSquareIcon, PencilIcon, RadioIcon, ReplyIcon, ScaleIcon, SearchIcon, ServerIcon, SettingsIcon, TargetIcon, VoteIcon } from "lucide-react";
import { Button, buttonVariants } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Input } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ActivityChart } from "@/components/pulse/charts";
import { FeedList, ProposalCard, QuestionCard, workingRows, workingSummary } from "@/components/pulse/cards";
import { AmendmentCard, SuggestionCard } from "@/components/board/vote-cards";
import { HygienePanel } from "@/components/pulse/hygiene";
import { Clamp } from "@/components/pulse/clamp";
import { DataTable } from "@/components/pulse/data-table";
import { DecideButtons } from "@/components/pulse/decide-buttons";
import { DocRev, GateCard, SelfImprovementPanel } from "@/components/pulse/evolve";
import { useInbox } from "@/components/pulse/inbox";
import { KindTile } from "@/components/pulse/kind-tile";
import { ListRow, ListRows } from "@/components/pulse/list-row";
import { ComputeDecide, ComputeInboxCard } from "@/components/compute/compute";
import { ProposedRecordCard } from "@/components/records/decide-record";
import { DraftCard, RecordDraftPanel } from "@/components/records/draft-panel";
import { LiveFeed } from "@/components/pulse/live-feed";
import { Markdown } from "@/components/pulse/markdown";
import { GOAL_ANCHOR, GoalEditor } from "@/components/project/goal";
import { LeadsPanel } from "@/components/project/leads";
import { MachinesPanel } from "@/components/project/machines";
import { healthLines, topThreads } from "@/components/project/project-state";
import { alertTone, funnelLine, funnelStages, heldGroups, ruleLine, settledLine } from "@/components/project/funnel-state";
import { AGENTS_DECIDED_HINT, AGENTS_DECIDED_SUMMARY, settledLabel, splitQuestions, waitingNote, whoSettled } from "@/components/project/tabs-state";
import { ToolkitPanel } from "@/components/project/toolkit";
import { Count, Disclosure, EmptyState, ErrorNotice, Kpi, KpiGrid, Kv, Notice, Panel, SkeletonRows, Toolbar } from "@/components/pulse/panel";
import { useViewMode, ViewToggle } from "@/components/pulse/view-toggle";
import { Avatar, Mono, PostLink, ProjectLink, StatusBadge, ThreadLink, ToneBadge, When, Who } from "@/components/pulse/who";
import { ThreadCards, ThreadParticipants, threadTitle } from "@/components/board/sheets";
import { ChatPane } from "@/components/chat/chat-pane";
import { CardGrid, ItemCard, ItemCardActions, ItemCardBody, ItemCardHeader, ItemCardKind, ItemCardTitle, SectionHeading } from "@/components/pulse/item-card";
import { TaskCards } from "@/components/tasks/task-card";
import { NewTaskButton } from "@/components/tasks/task-dialogs";
import { SuggestedWorkPanel, WorkSuggestionCard } from "@/components/tasks/suggested-work";
import { TASK_COLUMNS } from "@/components/tasks/task-columns";
import { kindLabel, RECORD_COLUMNS, RecordCards, RecordSheet } from "@/components/records/records";
import { hrefs } from "@/lib/nav";
import { askTitle, decisionLabels, isOpenQuestion } from "@/lib/owner-questions";
import { cn } from "@/lib/utils";
import { num, plural, rel, sentence } from "@/lib/format";
import { agentName, statusWord, threadName } from "@/lib/words";
import { codeDeliveryKpiState } from "@/lib/code-deliveries";
import { codeDeliveriesPath, type CodeDeliveriesPayload, isBaselineRow, projectPath, RECORD_KINDS, TASK_STATUSES, type Amendment, type Compute, type Constitution, type Evolution, type EvolutionArchiveRow, type EvolutionFunnel, type MessagesPayload, type Notices, type Overview, type OwnerQuestion, type ProjectGoalPayload, type ProjectInfo, type ProjectNeeds, type Proposal, type RecordRow, type RecordsPayload, type Suggestion, type Task, type TasksPayload, type ThreadRow, type Timeline } from "@/lib/api";
import { useApi, type Loaded } from "@/lib/use-api";

function Wait<T>({ q, rows = 5, children }: { q: Loaded<T>; rows?: number; children: (d: T) => React.ReactNode }) {
  if (q.error && !q.data) return <ErrorNotice error={q.error} onRetry={() => void q.reload()} />;
  if (q.data === undefined) return <SkeletonRows n={rows} />;
  return <>{children(q.data)}</>;
}

// A one-of filter as the segmented control /tasks uses: every choice is
// visible and one tap away, with its count inside. On a phone the row
// scrolls sideways. `you` marks a count that waits for the owner.
function Segments({ label, value, onChange, options, className }: { label: string; value: string; onChange: (v: string) => void; options: { id: string; label: string; n?: number; you?: boolean }[]; className?: string }) {
  return (
    <Tabs value={value} onValueChange={onChange} className={cn("min-w-0 max-w-full", className)}>
      <div className="scroll-strip min-w-0 max-w-full">
        <TabsList aria-label={label}>
          {options.map((o) => (
            <TabsTrigger key={o.id} value={o.id} className="flex-none px-2.5">
              {o.label}
              {o.n !== undefined ? <span className={cn("tnum min-w-4 rounded-full px-1.5 text-xs font-normal leading-5", o.you && o.n ? "bg-you-soft text-you-ink" : "bg-background/70 text-muted-foreground")}>{num(o.n)}</span> : null}
            </TabsTrigger>
          ))}
        </TabsList>
      </div>
    </Tabs>
  );
}

// A search box with its magnifier, like the one on /tasks.
function Search({ label, value, onChange, className }: { label: string; value: string; onChange: (v: string) => void; className?: string }) {
  return (
    <span className={cn("relative min-w-0", className)}>
      <SearchIcon className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden />
      <Input type="search" aria-label={label} placeholder={label} value={value} onChange={(e) => onChange(e.target.value)} className="h-9 pl-8" />
    </span>
  );
}

// A quiet "See all" link for a panel's title row.
function SeeAll({ href, children = "See all" }: { href: string; children?: React.ReactNode }) {
  return (
    <Link href={href} className="inline-flex items-center gap-1 font-medium text-primary underline-offset-2 hover:underline">
      {children}
      <ArrowRightIcon className="size-3.5" aria-hidden />
    </Link>
  );
}

// "Prefer the terminal?" - the CLI way, folded under the plain instruction.
function Terminal({ children }: { children: React.ReactNode }) {
  return (
    <Disclosure summary="Prefer the terminal?" className="mt-2 text-left" contentClassName="mt-1.5 text-[13px] leading-relaxed text-muted-foreground">
      {children}
    </Disclosure>
  );
}

const byNewest = <T,>(d: (x: T) => string) => (a: T, b: T) => (d(a) < d(b) ? 1 : d(a) > d(b) ? -1 : 0);
const isActiveTask = (x: Task) => x.status !== "done" && x.status !== "cancelled";
const NONE: never[] = [];

// ----- overview -----------------------------------------------------------

// A project's home in five panels (spec "Project"): what needs you, who is
// working, the open threads, the goal and the project's health. Everything
// else moved: leads and the toolkit to Team & tools, posts to History,
// failed jobs to Runs behind one link.
export function OverviewTab({ slug, canDecide = true }: { slug: string; canDecide?: boolean }) {
  const ov = useApi<Overview>(projectPath(slug, "overview"));
  const tasks = useApi<TasksPayload>("tasks?project=" + encodeURIComponent(slug));
  const inbox = useInbox();
  const commons = slug === "commons";
  return (
    <Wait q={ov} rows={8}>
      {(o) => {
        const open = (tasks.data?.tasks ?? []).filter(isActiveTask);
        return (
          <div className="grid gap-6 lg:grid-cols-12">
            <div className="grid min-w-0 content-start gap-8 lg:col-span-8">
              <NeedsYou slug={slug} mine={inbox.forProject(slug)} canDecide={canDecide} />
              <section aria-label="Working now" className="min-w-0">
                <SectionHeading icon={RadioIcon} count={o.claims_active || undefined} note={workingSummary(workingRows(o.active_claims, open, slug), o.claims_active)} as="h2">
                  Working now
                </SectionHeading>
                <WorkingRowsList claims={o.active_claims} slug={slug} tasks={open} />
                {o.stale_claims.length ? <StaleClaims claims={o.stale_claims} slug={slug} /> : null}
              </section>
              <section aria-label="Open threads" className="min-w-0">
                <SectionHeading icon={MessagesSquareIcon} count={o.threads_open || undefined} action={<SeeAll href={hrefs.project(slug, "board")}>Open the board</SeeAll>} as="h2">
                  Open threads
                </SectionHeading>
                {o.open_threads.length ? (
                  <ThreadRows rows={topThreads(o.open_threads, 5)} slug={slug} />
                ) : (
                  <p className="rounded-lg border border-dashed px-4 py-3 text-sm leading-relaxed text-muted-foreground">No thread is open. Agents open one when they pick up work or ask each other something.</p>
                )}
              </section>
            </div>
            <aside className="grid min-w-0 content-start gap-8 lg:col-span-4" aria-label="About this project">
              {commons ? null : <GoalPanel slug={slug} />}
              <HealthPanel slug={slug} commons={commons} />
            </aside>
          </div>
        );
      }}
    </Wait>
  );
}

// The top three things that wait for the owner here, as rows the owner can
// act on in place; the For you tab holds them all. Every kind the inbox
// knows gets a row; the ones with a yes/no get their buttons.
function NeedsYou({ slug, mine, canDecide, limit = 3, heading = true }: { slug: string; mine: ReturnType<ReturnType<typeof useInbox>["forProject"]>; canDecide: boolean; limit?: number; heading?: boolean }) {
  const [openRecord, setOpenRecord] = useState<RecordRow | null>(null);
  const rows = inboxRows(slug, mine, canDecide, setOpenRecord);
  const shown = limit ? rows.slice(0, limit) : rows;
  const more = rows.length - shown.length;
  const nodes = shown.map((r) => <div key={r.key}>{r.node}</div>);
  return (
    <section aria-label="Needs you" className="min-w-0">
      {heading ? (
        <SectionHeading icon={ScaleIcon} count={mine.total || undefined} action={more > 0 || rows.length > 0 ? <SeeAll href={hrefs.project(slug, "for-you")}>{more > 0 ? "See all " + num(rows.length) : "See all"}</SeeAll> : undefined} note={mine.agentsDeciding ? plural(mine.agentsDeciding, "question") + " the agents answered themselves" : undefined} as="h2">
          Needs you
        </SectionHeading>
      ) : null}
      {shown.length ? (
        <ListRows ariaLabel="Needs you">{nodes}</ListRows>
      ) : (
        <p className="flex items-center gap-2 rounded-lg border border-dashed px-4 py-3 text-sm leading-relaxed text-muted-foreground">
          <CheckCircle2Icon className="size-4 shrink-0 text-status-done" aria-hidden /> Nothing waits for you here. Agents post a question or a proposal when they need your word.
        </p>
      )}
      <RecordSheet id={openRecord?.id ?? null} project={openRecord?.project} onClose={() => setOpenRecord(null)} />
    </section>
  );
}

// Each inbox item as a ListRow, in the two lines Home uses: the ask alone on
// the first, so it keeps its words; who asked, how long ago and the first
// line of the text on the second; the action that settles it in sight on the
// right. With `expand`, a row opens its full card underneath (the For you
// tab) instead of leaving the page.
type InboxRow = { key: string; node: React.ReactNode };
type Expand = { expanded: string | null; onExpand: (key: string | null) => void };

const TWO_LINES = "min-h-[calc(var(--row-h)+0.75rem)]";

// The second line: who or what, the age, then as much of the text as fits
// (the text goes last, so it is what the ellipsis cuts), without its
// Markdown ticks and stars.
function SecondLine({ who, at, text }: { who: string; at?: string | null; text?: string | null }) {
  return (
    <>
      {who}
      {at ? (
        <>
          {" · "}
          <When d={at} />
        </>
      ) : null}
      {text ? " · " + text.replace(/`|\*\*/g, "") : null}
    </>
  );
}

// agentName() starts a sentence ("The runner", "You"); after "from" it reads
// in lower case.
const inSentence = (who: string) => (who === "The runner" || who === "You" ? who.toLowerCase() : who);

// The cue on a row that leads to an answer box: drawn as a button, but a
// span, since the row itself is the link or the opener.
function AnswerCue() {
  return (
    <span className={buttonVariants({ size: "xs" })}>
      <ReplyIcon aria-hidden /> Answer
    </span>
  );
}

function inboxRows(slug: string, mine: Omit<ReturnType<ReturnType<typeof useInbox>["forProject"]>, "total" | "agentsDeciding" | "machine" | "provider">, canDecide: boolean, openRecord: (r: RecordRow) => void, expand?: Expand): InboxRow[] {
  const rows: InboxRow[] = [];
  const opener = (key: string, href: string) => (expand ? { onOpen: () => expand.onExpand(expand.expanded === key ? null : key), selected: expand.expanded === key } : { href });
  for (const r of mine.decisions) {
    const key = "d" + r.id;
    rows.push({
      key,
      node: (
        <ListRow
          className={TWO_LINES}
          lead={<KindTile kind="record" waiting title="Record to adopt" />}
          title={r.title}
          status={<StatusBadge status={r.status} size="sm" />}
          onOpen={() => openRecord(r)}
          ariaLabel={"Record proposed: " + r.title}
        >
          <SecondLine who={kindLabel(r.kind) + " " + r.id} at={r.date} text="Open it to adopt or send it back." />
        </ListRow>
      ),
    });
  }
  for (const p of mine.proposals) {
    const key = "p" + p.id;
    const who = p.agent || p.name || p.session || "";
    rows.push({
      key,
      node: (
        <ListRow
          className={TWO_LINES}
          lead={<KindTile kind="proposal" waiting title="Proposal" />}
          title={p.summary}
          actionsVisible
          actions={canDecide && p.can_decide !== false ? <DecideButtons id={p.id} summary={p.summary} canDecide={canDecide} size="xs" className="w-auto justify-end" /> : undefined}
          {...opener(key, hrefs.post(p.slug || slug, p.id))}
          ariaLabel={"Proposal: " + p.summary}
        >
          <SecondLine who={who ? "Proposal from " + inSentence(agentName(who)) : "Proposal"} at={p.date} text={firstLine(p.proposal_text)} />
        </ListRow>
      ),
    });
  }
  for (const q of mine.questions) {
    const key = "q" + q.id;
    const stop = q.stop === true;
    const who = q.agent || q.name || q.session || "";
    const openQ = !stop && isOpenQuestion(q.summary, q.body);
    const labels = decisionLabels(q.body, stop ? { yes: "Carry on", no: "Keep paused" } : { yes: "Yes", no: "No" });
    const title = stop ? "Self-improvement stopped. May it carry on?" : q.summary;
    const what = stop ? "Self-improvement paused" : askTitle(q.kind, who ? inSentence(agentName(who)) : "an agent");
    rows.push({
      key,
      node: (
        <ListRow
          className={TWO_LINES}
          lead={<KindTile kind="question" waiting title={stop ? "Self-improvement paused" : "Question for you"} />}
          title={title}
          actionsVisible
          actions={canDecide ? openQ ? <AnswerCue /> : <DecideButtons id={q.id} summary={title} stop={stop} canDecide={canDecide} size="xs" yesLabel={labels.yes} noLabel={labels.no} className="w-auto justify-end" /> : undefined}
          {...(openQ && !expand ? { href: hrefs.project(q.slug || slug, "for-you") } : opener(key, hrefs.post(q.slug || slug, q.id)))}
          ariaLabel={(stop ? "Self-improvement paused: " : "Question for you: ") + q.summary}
        >
          <SecondLine who={q.stale ? what + " · older thread" : what} at={q.date} text={firstLine(q.body)} />
        </ListRow>
      ),
    });
  }
  for (const cr of mine.compute) {
    const key = "c" + cr.id;
    rows.push({
      key,
      node: (
        <ListRow
          className={TWO_LINES}
          lead={<KindTile kind="machine" waiting title="Machine request" />}
          title={"Extra machine for " + (cr.hours === 1 ? "1 hour" : cr.hours + " hours") + (cr.size ? " (" + cr.size + ")" : "")}
          actionsVisible
          actions={canDecide ? <ComputeDecide r={cr} size="xs" /> : undefined}
          {...opener(key, hrefs.compute())}
          ariaLabel={"Machine request: " + cr.reason}
        >
          <SecondLine who={"Asked by " + inSentence(agentName(cr.requested_by))} at={cr.at} text={cr.reason} />
        </ListRow>
      ),
    });
  }
  for (const w of mine.work) {
    const key = "w" + w.id;
    rows.push({
      key,
      node: (
        <ListRow
          className={TWO_LINES}
          lead={<KindTile kind="work" waiting title="Suggested work" />}
          title={w.title}
          status={<StatusBadge status={w.status} size="sm" />}
          {...opener(key, hrefs.tasks({ project: slug }))}
          ariaLabel={"Suggested work: " + w.title}
        >
          <SecondLine who={"Suggested by " + (w.suggested_by === "human" ? "you" : inSentence(agentName(w.suggested_by)))} at={w.updated_at || w.created_at} text={w.verdict || "Your call: make it a task or let it go."} />
        </ListRow>
      ),
    });
  }
  for (const d of mine.drafts) {
    const key = "r" + d.r.id;
    const what = d.r.id.toLowerCase().includes("evaluator") ? "scoring rules" : "constitution";
    rows.push({
      key,
      node: (
        <ListRow
          className={TWO_LINES}
          lead={<KindTile kind="rule" waiting title="Draft to review" />}
          title={"A new " + what + " to review"}
          {...opener(key, hrefs.project(slug, "evolution"))}
          ariaLabel={"Draft to review: " + d.r.id}
        >
          <SecondLine who={d.r.draft?.author ? "Drafted by " + inSentence(agentName(d.r.draft.author)) : "Draft " + d.r.id} at={d.r.draft?.at} text="An agent drafted the project's own text; adopt it or keep the template." />
        </ListRow>
      ),
    });
  }
  for (const st of mine.steps) {
    const key = "s" + st.kind + st.slug;
    rows.push({
      key,
      node: (
        <ListRow
          className={TWO_LINES}
          lead={<KindTile kind="plan" waiting title="Setup step" />}
          title={st.title}
          {...opener(key, hrefs.project(slug, "evolution"))}
          ariaLabel={"Setup step: " + st.title}
        >
          <SecondLine who={st.label} text={st.body} />
        </ListRow>
      ),
    });
  }
  return rows;
}

const firstLine = (t: string | undefined | null): string => sentence((t || "").trim().split(/\r?\n/).find((l) => l.trim()) || "");

// Who is working here, one row per agent: a face, the task or the claim,
// a running badge and how long.
function WorkingRowsList({ claims, slug, tasks }: { claims: Overview["active_claims"]; slug: string; tasks: Task[] }) {
  const rows = workingRows(claims, tasks, slug);
  if (!rows.length) return <p className="rounded-lg border border-dashed px-4 py-3 text-sm leading-relaxed text-muted-foreground">Nobody is working right now. When an agent starts a job or picks up a thread, it shows up here.</p>;
  return (
    <ListRows ariaLabel="Working now">
      {rows.map((r) => {
        const s = r.slug || slug;
        const c = r.claims[0];
        const what = r.task ? r.task.t.title : c ? sentence(c.summary) : "working";
        const since = r.task ? r.task.w.since : c?.date || c?.expires;
        const extra = r.task ? r.claims.length : Math.max(0, r.claims.length - 1);
        return (
          <ListRow
            key={r.key}
            lead={<Avatar seed={r.agent} size={28} />}
            title={what}
            meta={extra ? <span className="tnum">{"+" + plural(extra, "more thing")}</span> : undefined}
            status={<StatusBadge status="running" label="Working" size="sm" />}
            time={since ? <When d={since} /> : undefined}
            href={r.task ? hrefs.task(r.task.t.id) : c ? hrefs.post(s, c.id) : undefined}
            ariaLabel={agentName(r.agent) + " is working on " + what}
          >
            {agentName(r.agent) + (r.task ? " on task " + r.task.t.id : c ? " on " + threadName(c.thread_key) : "")}
          </ListRow>
        );
      })}
    </ListRows>
  );
}

// Claims that outlived their agent: a quiet fold under "Working now" so the
// owner can tell "busy" from "stuck". Each names its thread.
function StaleClaims({ claims, slug }: { claims: Overview["stale_claims"]; slug: string }) {
  return (
    <Disclosure className="mt-3 text-sm" summary={(claims.length === 1 ? "1 thing was" : plural(claims.length, "thing") + " were") + " picked up and then went quiet"}>
      <ul className="grid gap-2">
        {claims.map((c) => (
          <li key={c.id} className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[13px] text-muted-foreground">
            <Who row={c} compact />
            <span>on</span>
            <ThreadLink slug={slug} k={c.thread_key} />
            <span>
              , ran out <When d={c.expires} />
            </span>
          </li>
        ))}
      </ul>
    </Disclosure>
  );
}

// The goal in one paragraph (folded to two lines) and the Edit button; the
// full panel with its review lives behind "Edit".
function GoalPanel({ slug }: { slug: string }) {
  const q = useApi<ProjectGoalPayload>(projectPath(slug, "goal"), { refresh: 60000 });
  const [editing, setEditing] = useState(false);
  const goal = q.data?.goal;
  const canEdit = q.data?.can_edit === true;
  return (
    <section aria-label="Project goal" className="min-w-0" id={GOAL_ANCHOR}>
      <SectionHeading
        icon={TargetIcon}
        note={goal ? (goal.source === "owner" ? "Set by you" : "Written by PULSE") : undefined}
        action={
          goal && canEdit ? (
            <Button size="sm" variant="outline" onClick={() => setEditing(true)}>
              <PencilIcon /> Edit
            </Button>
          ) : undefined
        }
        as="h2"
      >
        Goal
      </SectionHeading>
      <div className="rounded-lg border bg-card p-4">
        {q.error && !q.data ? (
          <ErrorNotice error={q.error} onRetry={() => void q.reload()} />
        ) : goal ? (
          <Clamp lines={2} className="[&>p]:text-sm [&>p]:text-foreground">{goal.text}</Clamp>
        ) : (
          <SkeletonRows n={2} />
        )}
      </div>
      {editing && goal && canEdit ? <GoalEditor slug={slug} goal={goal} onClose={() => setEditing(false)} /> : null}
    </section>
  );
}

// Three lines: the repository and what landed last, the machines that run
// the project, and the code delivered in the window.
function HealthPanel({ slug, commons }: { slug: string; commons: boolean }) {
  const info = useApi<ProjectInfo>(commons ? null : projectPath(slug, "project"), { refresh: 60000 });
  const needs = useApi<ProjectNeeds>(commons ? null : "projects/" + encodeURIComponent(slug) + "/needs", { refresh: 60000 });
  const cd = useApi<CodeDeliveriesPayload>(commons ? null : codeDeliveriesPath({ project: slug, limit: 100 }), { refresh: 60000 });
  if (commons) return null;
  const kpi = codeDeliveryKpiState(cd.data, cd.loading, cd.error);
  const last = cd.data?.deliveries?.[0]?.at ?? null;
  const fits = needs.data ? needs.data.machines.filter((m) => m.fits && m.status === "active").length : null;
  const lines = healthLines({ branch: info.data?.branch, lastLanded: last ? rel(last) : null, machines: fits, needsWords: !!needs.data?.needs, delivered: kpi.n });
  return (
    <section aria-label="Health" className="min-w-0">
      <SectionHeading icon={ActivityIcon} as="h2">
        Health
      </SectionHeading>
      <ul className="grid gap-0 rounded-lg border bg-card">
        <HealthLine icon={FolderGit2Icon} href={hrefs.codeDeliveries({ project: slug })} title={info.data?.head ? "At commit " + info.data.head.slice(0, 8) : undefined}>
          {info.data || info.error ? lines.repository : "Repository: checking"}
        </HealthLine>
        <HealthLine icon={ServerIcon} href={hrefs.compute()}>
          {lines.machines}
        </HealthLine>
        <HealthLine icon={FileDiffIcon} href={hrefs.codeDeliveries({ project: slug })} title={kpi.sub}>
          {lines.delivered}
        </HealthLine>
      </ul>
      <p className="mt-2 text-[13px] text-muted-foreground">
        <Link href={hrefs.harness({ project: slug, status: "failed" })} className="font-medium text-link underline-offset-2 hover:underline">
          Jobs that failed
        </Link>
        {" "}are on the Runs page.
      </p>
    </section>
  );
}

function HealthLine({ icon: Icon, href, title, children }: { icon: typeof ActivityIcon; href: string; title?: string; children: React.ReactNode }) {
  return (
    <li className="border-b last:border-b-0">
      <Link href={href} title={title} className="flex min-h-11 items-center gap-3 px-4 py-2 text-sm text-foreground transition-colors hover:bg-muted/60 focus-visible:bg-muted/60 focus-visible:outline-none">
        <Icon className="size-4 shrink-0 text-muted-foreground" aria-hidden />
        <span className="min-w-0 flex-1 truncate">{children}</span>
        <ArrowRightIcon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
      </Link>
    </li>
  );
}

// Open threads as rows: the status icon, the title, the area chip, the
// participants, the post count and the last activity; the row opens the thread.
export function ThreadRows({ rows, slug, onOpen, activeKey }: { rows: ThreadRow[]; slug: string; onOpen?: (key: string) => void; activeKey?: string | null }) {
  return (
    <ListRows ariaLabel="Threads">
      {rows.map((t) => (
        <ListRow
          key={t.key}
          lead={<KindTile kind="session" title="Thread" />}
          title={threadTitle(t)}
          meta={
            <>
              <span className="rounded-md bg-muted px-1.5 py-px font-mono text-xs">{t.area}</span>
              <ThreadParticipants t={t} />
              <span className="tnum">{plural(t.posts, "post")}</span>
            </>
          }
          status={<StatusBadge status={t.status} size="sm" />}
          time={<When d={t.last} />}
          href={onOpen ? undefined : hrefs.thread(slug, t.key)}
          onOpen={onOpen ? () => onOpen(t.key) : undefined}
          selected={activeKey === t.key}
          ariaLabel={"Thread " + threadTitle(t)}
        >
          {t.summary ? sentence(t.summary) : t.last_by ? "Last by " + agentName(t.last_by) : null}
        </ListRow>
      ))}
    </ListRows>
  );
}

// ----- tasks ----------------------------------------------------------------

export function TasksTab({ slug, onOpen, activeId }: { slug: string; onOpen: (id: string) => void; activeId?: string | null }) {
  const q = useApi<TasksPayload>("tasks?project=" + encodeURIComponent(slug), { refresh: 10000 });
  const [status, setStatus] = useState("active");
  const [text, setText] = useState("");
  const [view, setView] = useViewMode("project-tasks");
  const all = q.data?.tasks ?? NONE;
  const rows = useMemo(() => {
    const t = text.trim().toLowerCase();
    return all
      .filter((x) => (status === "all" ? true : status === "active" ? isActiveTask(x) : x.status === status))
      .filter((x) => !t || x.title.toLowerCase().includes(t) || x.id.toLowerCase().includes(t) || (x.brief || "").toLowerCase().includes(t))
      .sort(byNewest((x) => x.updated_at));
  }, [all, status, text]);
  const chips = [
    { id: "active", label: "Open", n: all.filter(isActiveTask).length },
    { id: "all", label: "All", n: all.length },
    ...TASK_STATUSES.map((s) => ({ id: s, label: statusWord(s), n: all.filter((x) => x.status === s).length, you: s === "review" })).filter((c) => c.n || c.id === status),
  ];
  return (
    <div>
      <SuggestedWorkPanel slug={slug} onOpenTask={onOpen} className="mb-6" />
      <Toolbar className="mb-4">
        <Segments label="Status" value={status} onChange={setStatus} options={chips} />
        <span className="flex min-w-0 flex-1 basis-56 items-center gap-2">
          <Search label="Search tasks" value={text} onChange={setText} className="flex-1 lg:max-w-64" />
          <ViewToggle value={view} onChange={setView} className="ml-auto shrink-0" />
        </span>
      </Toolbar>
      <Wait q={q}>
        {() => {
          const empty =
            status === "active" && !text ? (
              <>
                {slug} has no open task. Press <b>New task</b>, say what you want, and the lead agent starts on it. Its questions and progress show up in the chat.
              </>
            ) : (
              "No task matches. Try another word or status."
            );
          if (view === "table")
            return rows.length ? (
              <div className="overflow-x-auto rounded-lg border bg-card">
                <DataTable columns={TASK_COLUMNS(false)} rows={rows} rowKey={(t) => t.id} onRow={(t) => onOpen(t.id)} initialSort="updated" />
              </div>
            ) : (
              <EmptyState title="No tasks" action={status === "active" && !text ? <NewTaskButton project={slug} onCreated={(t: Task) => onOpen(t.id)} /> : undefined}>{empty}</EmptyState>
            );
          return <TaskCards tasks={rows} onOpen={(t) => onOpen(t.id)} activeId={activeId} empty={empty} emptyTitle={status === "active" && !text ? "No open task" : undefined} emptyAction={status === "active" && !text ? <NewTaskButton project={slug} onCreated={(t: Task) => onOpen(t.id)} /> : undefined} />;
        }}
      </Wait>
    </div>
  );
}

// ----- chat -----------------------------------------------------------------

export function ChatTab({ slug, taskId, onPickTask }: { slug: string; taskId: string | null; onPickTask: (id: string | null) => void }) {
  const msgs = useApi<MessagesPayload>("messages?project=" + encodeURIComponent(slug) + (taskId ? "&task=" + encodeURIComponent(taskId) : ""), { refresh: 8000 });
  const tasks = useApi<TasksPayload>("tasks?project=" + encodeURIComponent(slug));
  const lead = (tasks.data?.tasks ?? []).find((t) => t.lead)?.lead || slug + "-lead";
  const scoped = taskId ? (tasks.data?.tasks ?? []).find((t) => t.id === taskId) : undefined;
  return (
    <div className="grid gap-3">
      <Toolbar className="mb-0">
        <span className="inline-flex min-w-0 items-center gap-2 text-sm">
          <Avatar seed={lead} size={24} />
          <span className="text-muted-foreground">Talking with</span>
          <span className="font-medium anywhere" title={lead}>
            {agentName(lead, slug)}
          </span>
        </span>
        <Select value={taskId || "__all"} onValueChange={(v) => onPickTask(v === "__all" ? null : v)}>
          <SelectTrigger aria-label="What the session is about" className="h-9 w-full sm:w-72">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__all">Whole project</SelectItem>
            {(tasks.data?.tasks ?? []).map((t) => (
              <SelectItem key={t.id} value={t.id}>
                {t.title + " (" + t.id + ")"}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {scoped ? (
          <Button asChild size="sm" variant="ghost">
            <Link href={hrefs.task(scoped.id)}>Open the task</Link>
          </Button>
        ) : null}
      </Toolbar>
      {msgs.error && !msgs.data ? <ErrorNotice error={msgs.error} onRetry={() => void msgs.reload()} /> : msgs.data === undefined ? <SkeletonRows n={6} /> : <ChatPane project={slug} taskId={taskId} messages={msgs.data.messages} tasks={tasks.data?.tasks} lead={lead} leadWork={msgs.data.lead_work} />}
    </div>
  );
}

// ----- threads --------------------------------------------------------------

// The board: every thread of this project as a list (the status icon, the
// title, the area, who talks in it, how many posts, the last activity) with
// the cards one toggle away. The choice is remembered per screen.
export function ThreadsTab({ slug, onOpen, activeKey }: { slug: string; onOpen: (key: string) => void; activeKey?: string | null }) {
  const q = useApi<ThreadRow[]>(projectPath(slug, "threads"));
  const [status, setStatus] = useState("open");
  const [area, setArea] = useState("all");
  const [text, setText] = useState("");
  const [view, setView] = useViewMode("project-board", "table");
  const all = q.data ?? NONE;
  const areas = useMemo(() => Array.from(new Set(all.map((t) => t.area))).sort(), [all]);
  const rows = useMemo(() => {
    const t = text.trim().toLowerCase();
    return all.filter((x) => (status === "all" || x.status === status) && (area === "all" || x.area === area) && (!t || x.key.toLowerCase().includes(t) || x.summary.toLowerCase().includes(t))).sort(byNewest((x) => x.last));
  }, [all, status, area, text]);
  const chips = [
    { id: "open", label: "Open", n: all.filter((x) => x.status === "open").length },
    { id: "closed", label: "Closed", n: all.filter((x) => x.status === "closed").length },
    { id: "all", label: "All", n: all.length },
  ];
  return (
    <div>
      <Toolbar className="mb-4">
        <Segments label="Status" value={status} onChange={setStatus} options={chips} />
        {areas.length > 1 ? (
          <Select value={area} onValueChange={setArea}>
            <SelectTrigger aria-label="Area" className="h-9 w-40">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All areas</SelectItem>
              {areas.map((a) => (
                <SelectItem key={a} value={a}>
                  {sentence(a)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        ) : null}
        <Search label="Search threads" value={text} onChange={setText} className="w-full sm:w-56" />
        <span className="ml-auto flex items-center gap-2">
          {text.trim() ? <Count>{plural(rows.length, "thread")}</Count> : null}
          <Tabs value={view === "cards" ? "cards" : "list"} onValueChange={(v) => setView(v === "cards" ? "cards" : "table")}>
            <TabsList variant="segmented" aria-label="Show threads as">
              <TabsTrigger value="list" aria-label="Show as a list">
                <ListIcon /> List
              </TabsTrigger>
              <TabsTrigger value="cards" aria-label="Show as cards">
                <LayoutGridIcon /> Cards
              </TabsTrigger>
            </TabsList>
          </Tabs>
        </span>
      </Toolbar>
      <Wait q={q} rows={8}>
        {() => {
          const empty = all.length ? "No thread matches. Try another word, status or area." : "No threads yet. A thread starts when an agent picks up work, asks another agent something or makes a suggestion.";
          if (view === "cards") return <ThreadCards rows={rows} slug={slug} onOpen={onOpen} activeKey={activeKey} empty={empty} />;
          return rows.length ? <ThreadRows rows={rows} slug={slug} onOpen={onOpen} activeKey={activeKey} /> : <EmptyState>{empty}</EmptyState>;
        }}
      </Wait>
    </div>
  );
}

// ----- proposals ------------------------------------------------------------

// The project's inbox: every card the "N for you" count counts - proposed
// decisions, proposals, questions, compute requests, drafts and the gate
// step - each with its yes or no on it. "All" adds the proposals already
// decided, for the record.
// For you: everything that waits for the owner on this project as rows
// (the same row style Home uses), each with its action in place; a row
// opens to the full card underneath for the asks that need more than a yes
// or no (an answer in words, a machine's caps, a draft to read). Decided
// proposals stay one toggle away.
export function ForYouTab({ slug, canDecide }: { slug: string; canDecide: boolean }) {
  const q = useApi<Proposal[]>(projectPath(slug, "proposals"));
  // Questions addressed to the owner and the evolution loop's stop questions wait for a yes or no like a proposal does; an older hub has no such route.
  const qs = useApi<OwnerQuestion[]>(projectPath(slug, "questions"), { refresh: 60000 });
  const compute = useApi<Compute>("compute");
  const inbox = useInbox();
  const [show, setShow] = useState<"awaiting" | "all">("awaiting");
  const [openRecord, setOpenRecord] = useState<RecordRow | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  return (
    <Wait q={q}>
      {(all) => {
        const mine = inbox.forProject(slug);
        // The project's own routes are the source for proposals and questions; the shared inbox adds the rest.
        // Count only what the owner alone can decide: questions the lead answers while the owner is away, or that a setting answers, are the agents' to settle.
        const allQuestions = qs.data ?? mine.questions;
        const { questions, agentsDecided } = splitQuestions(allQuestions);
        const awaiting = all.filter((p) => p.status === "awaiting");
        const decided = all.filter((p) => p.status !== "awaiting");
        const waiting = mine.decisions.length + awaiting.length + questions.length + mine.compute.length + mine.work.length + mine.drafts.length + mine.steps.length;
        const items = { ...mine, proposals: awaiting, questions };
        const rows = inboxRows(slug, items, canDecide, setOpenRecord, { expanded, onExpand: setExpanded });
        const cards: Record<string, React.ReactNode> = {};
        for (const r of mine.decisions) cards["d" + r.id] = <ProposedRecordCard r={r} onOpen={setOpenRecord} />;
        for (const p of awaiting) cards["p" + p.id] = <ProposalCard r={p} slug={slug} canDecide={canDecide} />;
        for (const x of questions) cards["q" + x.id] = <QuestionCard q={x} slug={slug} canDecide={canDecide} />;
        for (const cr of mine.compute) cards["c" + cr.id] = <ComputeInboxCard r={cr} caps={compute.data?.caps} />;
        for (const w of mine.work) cards["w" + w.id] = <WorkSuggestionCard s={w} slug={slug} />;
        for (const { r } of mine.drafts) cards["r" + r.id] = <DraftCard slug={slug} r={r} />;
        for (const st of mine.steps) cards["s" + st.kind + st.slug] = <GateCard step={st} />;
        return (
          <div className="grid gap-6">
            <section aria-label="Waiting for you" className="min-w-0">
              <SectionHeading icon={ScaleIcon} count={waiting || undefined} note={waitingNote(canDecide, agentsDecided.length)} as="h2">
                Waiting for you
              </SectionHeading>
              {waiting ? (
                <ListRows ariaLabel="Waiting for you">
                  {rows.map((row) => (
                    <div key={row.key}>
                      {row.node}
                      {expanded === row.key && cards[row.key] ? (
                        <div className="bg-muted/20 px-3 py-3 sm:pl-14" data-row-details>
                          {cards[row.key]}
                        </div>
                      ) : null}
                    </div>
                  ))}
                </ListRows>
              ) : (
                <EmptyState icon={<CheckCircle2Icon className="size-[18px]" />} title="Nothing waits for you">
                  {"You're all caught up on " + slug + ". When an agent needs your yes or no, it shows up here."}
                </EmptyState>
              )}
            </section>
            {decided.length || agentsDecided.length ? (
              <section aria-label="Answered" className="min-w-0">
                <SectionHeading
                  icon={CheckCircle2Icon}
                  count={decided.length + agentsDecided.length}
                  action={
                    <Button size="sm" variant="ghost" onClick={() => setShow(show === "all" ? "awaiting" : "all")} aria-expanded={show === "all"}>
                      {show === "all" ? "Hide" : "Show"}
                    </Button>
                  }
                  as="h2"
                >
                  Answered
                </SectionHeading>
                {show === "all" ? (
                  <div className="grid gap-4">
                    {agentsDecided.length ? <AgentsDecidedAudit questions={agentsDecided} slug={slug} /> : null}
                    {decided.length ? (
                      <CardGrid cols={2}>
                        {decided.map((p) => (
                          <ProposalCard key={p.id} r={p} slug={slug} canDecide={canDecide} />
                        ))}
                      </CardGrid>
                    ) : null}
                  </div>
                ) : null}
              </section>
            ) : null}
            <RecordSheet id={openRecord?.id ?? null} project={openRecord?.project} onClose={() => setOpenRecord(null)} />
          </div>
        );
      }}
    </Wait>
  );
}

// The questions the agents settle themselves - the lead answers them while
// the owner is away, or a setting answers them - kept out of every "N for
// you" count but auditable here, collapsed under a toggle so the audit never
// reads as more waiting work. The rows are read-only: the decision is already
// recorded, so there are no buttons - only a link to the thread with the
// answer. Which questions those are, and the words, live in tabs-state.ts.
export function AgentsDecidedAudit({ questions, slug }: { questions: OwnerQuestion[]; slug: string }) {
  const [open, setOpen] = useState(false);
  const hint = open ? null : <span>{AGENTS_DECIDED_HINT}</span>;
  return (
    <div className="mb-4" data-agents-decided="">
      <Disclosure summary={<span>{AGENTS_DECIDED_SUMMARY} · {num(questions.length)}</span>} hint={hint} open={open} onToggle={setOpen}>
        <CardGrid cols={2}>
          {questions.map((x) => (
            <ItemCard key={x.id} ariaLabel={settledLabel(x)}>
              <ItemCardHeader right={<When d={x.date} />}>
                <ItemCardKind icon={MessageCircleQuestionIcon}>Settled by {whoSettled(x.decider)}</ItemCardKind>
              </ItemCardHeader>
              <ItemCardTitle>{x.summary}</ItemCardTitle>
              <ItemCardBody clamp={3}>{sentence(x.body)}</ItemCardBody>
              <ItemCardActions>
                <Who row={x} />
                <span className="flex w-full flex-wrap justify-start gap-2 sm:ml-auto sm:w-auto sm:justify-end">
                  <Link href={hrefs.post(slug, x.id)} title={"Thread " + x.thread + ", post " + x.id} className="text-[13px] font-medium text-muted-foreground underline-offset-2 hover:text-foreground hover:underline">
                    See it on the board
                  </Link>
                </span>
              </ItemCardActions>
            </ItemCard>
          ))}
        </CardGrid>
      </Disclosure>
    </div>
  );
}

// ----- suggestions ---------------------------------------------------------

export function SuggestionsTab({ slug }: { slug: string }) {
  const q = useApi<Suggestion[]>(projectPath(slug, "suggestions"));
  const ov = useApi<Overview>(projectPath(slug, "overview"));
  const [status, setStatus] = useState("all");
  return (
    <Wait q={q}>
      {(all) => {
        const quorum = ov.data?.quorum ?? all[0]?.quorum ?? 2;
        const statuses = Array.from(new Set(all.map((s) => s.status))).sort();
        const rows = status === "all" ? all : all.filter((s) => s.status === status);
        return (
          <div>
            <Toolbar className="mb-4">
              <Segments label="Status" value={status} onChange={setStatus} options={[{ id: "all", label: "All", n: all.length }, ...statuses.map((s) => ({ id: s, label: statusWord(s), n: all.filter((x) => x.status === s).length }))]} />
            </Toolbar>
            {rows.length ? (
              <CardGrid cols={2}>
                {rows.map((s) => (
                  <SuggestionCard key={s.id} s={s} quorum={quorum} slug={slug} />
                ))}
              </CardGrid>
            ) : (
              <EmptyState icon={<VoteIcon className="size-[18px]" />} title="No suggestions">
                Agents post ideas here and vote on each other&apos;s. Once a suggestion has enough votes and passes the checks, it becomes a job.
              </EmptyState>
            )}
          </div>
        );
      }}
    </Wait>
  );
}

// ----- amendments ----------------------------------------------------------

export function AmendmentsTab({ slug }: { slug: string }) {
  const q = useApi<Amendment[]>(projectPath(slug, "amendments"));
  const c = useApi<Constitution>("constitution?project=" + encodeURIComponent(slug), { refresh: 60000 });
  return (
    <Wait q={q}>
      {(all) => (
        <div className="grid gap-6">
          {all.length ? (
            <CardGrid cols={2}>
              {all.map((a) => (
                <AmendmentCard key={a.id} a={a} slug={slug} />
              ))}
            </CardGrid>
          ) : (
            <EmptyState icon={<FileDiffIcon className="size-[18px]" />} title="No rule changes">
              Nobody has proposed a change to the rules. An agent proposes one on the board; it passes when enough agents vote for it.
            </EmptyState>
          )}
          <ConstitutionPanel c={c} slug={slug} />
        </div>
      )}
    </Wait>
  );
}

// The rules the project's self-improvement follows, with the rule changes made so far.
function ConstitutionPanel({ c, slug }: { c: Loaded<Constitution>; slug: string }) {
  return (
    <Panel title="The rules (constitution)" icon={FileDiffIcon} actions={c.data?.record ? <SeeAll href={hrefs.record(c.data.record.id, slug)}>Open the record</SeeAll> : null}>
      {c.error && !c.data && !noRecord(c.error) ? (
        <ErrorNotice error={c.error} onRetry={() => void c.reload()} />
      ) : c.data === undefined && !c.error ? (
        <SkeletonRows n={4} />
      ) : c.data?.body ? (
        <Markdown source={c.data.body} className="max-h-[60vh] overflow-y-auto" />
      ) : (
        <p className="text-sm leading-relaxed text-muted-foreground">
          {slug} has no constitution yet. It holds the limits every agent works inside; PULSE writes one from its template when the project is set up.
        </p>
      )}
      {c.data?.amendments?.length ? (
        <div className="mt-4 border-t pt-3">
          <div className="mb-2 text-[13px] font-semibold">Rule changes so far</div>
          <ul className="grid gap-2 text-[13px]">
            {c.data.amendments.map((a) => (
              <li key={a.id} className="flex flex-wrap items-center gap-2">
                <PostLink slug={slug} id={a.id} label={"Rule change by " + (a.id.replace(/^\d{8}T\d{6}Z-?/, "") || "an agent")} className="font-medium text-foreground" />
                <StatusBadge status={a.status} />
                <span className="tnum text-muted-foreground">{num(a.votes_up) + " for, " + num(a.votes_down) + " against, " + num(a.quorum) + " needed"}</span>
                {a.passed ? <ToneBadge tone="ok">Passed</ToneBadge> : null}
                {a.protected ? <ToneBadge tone="bad" title="This part of the rules only you can change">Protected section</ToneBadge> : null}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </Panel>
  );
}

// The hub answers 404 "no such record" when a project has no constitution
// indexed; that is an empty state, not a failure.
function noRecord(err: string): boolean {
  return /no such record|not found|404/i.test(err);
}

// ----- evolution ------------------------------------------------------------

const kept = (r: EvolutionArchiveRow) => r.kept === 1 || r.kept === true;
const score = (n: number) => (Number.isFinite(n) ? n.toFixed(3).replace(/\.?0+$/, "") : "—");

export function EvolutionTab({ slug, canDecide = true }: { slug: string; canDecide?: boolean }) {
  const q = useApi<Evolution>(projectPath(slug, "evolution"), { refresh: 60000 });
  return (
    <Wait q={q} rows={8}>
      {(e) => {
        const rows = e.archive?.rows ?? [];
        // The daemon's baseline (ADR-031) is an archive row like any other; it is a measurement, not a cycle, so the tallies leave it out.
        const cycles = rows.filter((r) => !isBaselineRow(r));
        const baselineRow = rows.find(isBaselineRow);
        const loop = e.loop ?? { loop: "", limits: "", tolerance: "", stop: "", heldout_present: false };
        const present = !!(e.constitution || e.evaluators || e.archive?.record || rows.length);
        const stopped = loop.stop === "1";
        const keptN = cycles.filter(kept).length;
        // The gates and fixtures (ADR-029) show whenever the hub reports them, set up or not: they are what the owner asked about.
        const fixtures = e.fixtures ?? [];
        const gated = e.gates !== undefined || !!e.gates_error || fixtures.length > 0;
        // The loop's records (ADR-030): onboarding seeds them from the template; agents draft the project's own text and the owner adopts it (ADR-046).
        const docs = { constitution: e.constitution ?? null, evaluators: e.evaluators ?? null };
        const docWord = (doc: Evolution["constitution"]) =>
          !doc ? "Missing" : !doc.seeded ? (doc.draft ? "Your own text, with a new draft to review" : doc.seeded_from ? "Your edited copy of the template" : "Your own text") : doc.draft ? "Template text, with a draft to review" : doc.draft_job ? "Template text; an agent is drafting your own" : "Template text";
        const empty = (
          <EmptyState icon={<ActivityIcon className="size-[18px]" />} title="Self-improvement isn't set up">
            Self-improvement lets agents try changes to {slug} on their own and keep only the ones that score better. It needs the project&apos;s rules and scoring rules. PULSE adds both when it sets a project up; for an older project, add them from a terminal. Then turn it on in{" "}
            <Link href={hrefs.settings("evolve")} className="font-medium text-foreground underline underline-offset-2">
              Settings
            </Link>
            .
            <Terminal>
              For a project added before this existed, run <Mono>{"pulse evolve seed --project " + slug}</Mono>.
            </Terminal>
          </EmptyState>
        );
        // The empty state is the whole tab only when there is nothing else: with the setup step or the funnel showing, that is the one call to action.
        if (!present && !gated && !e.funnel) return empty;
        return (
          <div className="grid gap-6">
            {e.funnel ? <FunnelPanel f={e.funnel} slug={slug} /> : null}
            {gated ? <SelfImprovementPanel gates={e.gates} error={e.gates_error} fixtures={fixtures} docs={docs} canDecide={canDecide} /> : null}
            {!present ? null : (
              <>
                {stopped ? <Notice tone="bad">Self-improvement is stopped. It stays stopped until you clear the stop in Settings.</Notice> : null}
                {!e.constitution || !e.evaluators ? (
                  <Notice tone="warn">
                    {"Self-improvement can't run yet: " + [!e.constitution ? "the rules (constitution)" : "", !e.evaluators ? "the scoring rules" : ""].filter(Boolean).join(" and ") + (!e.constitution && !e.evaluators ? " are" : " is") + " missing. " + (gated ? "Add them with the setup step above." : "PULSE adds them when it sets a project up.")}
                    <Terminal>
                      Run <Mono>{"pulse evolve seed --project " + slug}</Mono> to add what is missing, or write your own and add it with <Mono>pulse record new evolution</Mono>.
                    </Terminal>
                  </Notice>
                ) : null}
                <Panel title="Experiments so far" icon={ActivityIcon} actions={<span>{plural(cycles.length, "experiment") + (cycles.length ? ", " + keptN + " kept" : "") + (baselineRow ? ", starting score " + score(baselineRow.variant_score) : "")}</span>} dense>
                  {rows.length ? (
                    <DataTable
                      dense
                      rows={rows}
                      rowKey={(r) => r.id}
                      columns={[
                        { key: "at", label: "When", render: (r) => <When d={r.at} /> },
                        // The daemon's baseline reads as what it is (ADR-031), never as a kept experiment.
                        {
                          key: "hypothesis",
                          label: "What it tried",
                          render: (r) =>
                            isBaselineRow(r) ? (
                              <ToneBadge tone="info" title="The runner measured the project as it stood; every experiment is scored against this">
                                Starting score
                              </ToneBadge>
                            ) : (
                              <span className="anywhere" title={r.id}>{r.hypothesis || "An experiment"}</span>
                            ),
                        },
                        { key: "score", label: "Score", align: "right", render: (r) => <span className="tnum">{isBaselineRow(r) ? score(r.variant_score) : score(r.baseline_score) + " → " + score(r.variant_score)}</span> },
                        { key: "kept", label: "Result", render: (r) => (isBaselineRow(r) ? <ToneBadge tone="neutral">Measured</ToneBadge> : <ToneBadge tone={kept(r) ? "ok" : "neutral"}>{kept(r) ? "Kept" : "Dropped"}</ToneBadge>) },
                        { key: "branch", label: "Branch", render: (r) => (r.branch ? <Mono>{r.branch}</Mono> : "") },
                        { key: "sandbox", label: "Where it ran", render: (r) => r.sandbox || "" },
                        { key: "note", label: "Note", render: (r) => <span className="anywhere text-muted-foreground">{r.note}</span> },
                      ]}
                    />
                  ) : (
                    <EmptyState className="m-4">No experiments yet. The runner first measures a starting score; with self-improvement on, every experiment follows here with its score and whether it was kept.</EmptyState>
                  )}
                </Panel>
                <div className="grid gap-4 lg:grid-cols-2">
                  <Panel title="The rules (constitution)" icon={FileDiffIcon} actions={e.constitution ? <DocRev doc={e.constitution} slug={slug} /> : undefined} footer={<span>{docWord(e.constitution)}</span>}>
                    {e.constitution ? (
                      <>
                        <RecordDraftPanel r={{ id: e.constitution.record.id, rev: e.constitution.record.current_rev, seeded: e.constitution.seeded === true, seeded_from: e.constitution.seeded_from ?? "", draft: e.constitution.draft, draft_job: e.constitution.draft_job }} body={e.constitution.draft?.body} className="mb-3" />
                        <Markdown source={e.constitution.body} className="max-h-[50vh] overflow-y-auto" />
                      </>
                    ) : (
                      <p className="text-sm leading-relaxed text-muted-foreground">Not written yet. These are the limits experiments must stay inside: from the template, or in your own words.</p>
                    )}
                  </Panel>
                  <Panel title="Scoring rules" icon={CheckCircle2Icon} actions={e.evaluators ? <DocRev doc={e.evaluators} slug={slug} /> : undefined} footer={<span>{docWord(e.evaluators)}</span>}>
                    {e.evaluators ? (
                      <>
                        <RecordDraftPanel r={{ id: e.evaluators.record.id, rev: e.evaluators.record.current_rev, seeded: e.evaluators.seeded === true, seeded_from: e.evaluators.seeded_from ?? "", draft: e.evaluators.draft, draft_job: e.evaluators.draft_job }} body={e.evaluators.draft?.body} className="mb-3" />
                        <Markdown source={e.evaluators.body} className="max-h-[50vh] overflow-y-auto" />
                      </>
                    ) : (
                      <p className="text-sm leading-relaxed text-muted-foreground">Not written yet. It describes the checks agents may see; the private tests stay on your machine.</p>
                    )}
                  </Panel>
                </div>
                <Disclosure variant="row" summary="Limits and older history" className="rounded-lg border bg-card px-3 py-1.5 text-sm sm:px-4" contentClassName="pb-3">
                  <Kv
                    rows={[
                      ["Allowed drop in score", loop.tolerance || "—"],
                      ["Limits", loop.limits || "None"],
                    ]}
                  />
                  {e.archive?.record && e.archive.body ? (
                    <div className="mt-4 border-t pt-3">
                      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
                        <span className="text-[13px] font-semibold">Older history</span>
                        <SeeAll href={hrefs.record(e.archive.record.id, slug)}>Open the record</SeeAll>
                      </div>
                      <Markdown source={e.archive.body} className="max-h-[50vh] overflow-y-auto" />
                    </div>
                  ) : null}
                </Disclosure>
              </>
            )}
          </div>
        );
      }}
    </Wait>
  );
}

// How the project's suggestions move from the board to a landed change: the
// stall alerts first (the backend words them), the stages as a strip, then
// what holds the agreed ones, what settled and the rule a suggestion follows.
// The suggested tile opens the board's suggestions; the in-progress tile the
// project's work.
function FunnelPanel({ f, slug }: { f: EvolutionFunnel; slug: string }) {
  const stages = funnelStages(f);
  const held = heldGroups(f.held);
  const settled = settledLine(f);
  const tileHref: Record<string, string | undefined> = {
    posted: hrefs.project(slug, "suggestions"),
    in_flight: hrefs.project(slug, "work"),
  };
  const tileTone = (key: string): "neutral" | "warn" | "ok" => (key === "in_flight" && f.blocked > 0 ? "warn" : key === "built" && f.built > 0 ? "ok" : "neutral");
  return (
    <Panel title="From suggestion to change" icon={FunnelIcon} actions={<span>{funnelLine(f)}</span>}>
      <div className="grid gap-4">
        {f.alerts?.length ? (
          <div className="grid gap-2">
            {f.alerts.map((a, i) => (
              <Notice key={a.code + i} tone={alertTone(a.code)}>
                {a.text}
              </Notice>
            ))}
          </div>
        ) : null}
        <KpiGrid className={cn("mb-0 grid-cols-2 sm:grid-cols-3", stages.length > 6 ? "md:grid-cols-7" : "md:grid-cols-6")}>
          {stages.map((s) => (
            <Kpi key={s.key} n={s.n} label={s.label} sub={s.sub} href={tileHref[s.key]} tone={tileTone(s.key)} />
          ))}
        </KpiGrid>
        <Kv
          rows={[
            ["Held", held || "—"],
            ["Settled", settled || "—"],
            ["Rule", ruleLine(f)],
          ]}
        />
      </div>
    </Panel>
  );
}

// ----- timeline -------------------------------------------------------------

// Everything that happened on the project, newest first.
export function TimelineTab({ slug }: { slug: string }) {
  return <LiveFeed project={slug} limit={60} height={640} title="Everything that happened" />;
}

// The posting activity that used to crowd the Overview: the board's latest
// posts and the posts-per-day chart, each under its own heading.
export function HistoryActivity({ slug }: { slug: string }) {
  const tl = useApi<Timeline>(projectPath(slug, "timeline"), { refresh: 60000 });
  // There is no per-project activity route; the overview's recent posts are
  // the board's latest and refresh with it.
  const ov = useApi<Overview>(projectPath(slug, "overview"), { refresh: 30000 });
  return (
    <>
      <section aria-label="Recent posts" className="min-w-0">
        <SectionHeading icon={MessageSquareIcon} count={ov.data?.recent_posts.length || undefined} action={<SeeAll href={hrefs.project(slug, "board")}>Open the board</SeeAll>} as="h2">
          Recent posts
        </SectionHeading>
        <div className="rounded-lg border bg-card">
          <Wait q={ov} rows={8}>
            {(o) => <FeedList rows={o.recent_posts} slug={slug} limit={30} />}
          </Wait>
        </div>
      </section>
      <section aria-label="Posts per day" className="min-w-0">
        <SectionHeading icon={BarChart3Icon} as="h2">
          Posts per day
        </SectionHeading>
        {tl.error && !tl.data ? <ErrorNotice error={tl.error} onRetry={() => void tl.reload()} /> : tl.data ? <ActivityChart tl={tl.data} bys={["total", "kind"]} height={220} title="Posts per day" /> : <SkeletonRows n={4} />}
      </section>
    </>
  );
}

// ----- notices --------------------------------------------------------------

export function NoticesTab({ slug }: { slug: string }) {
  const q = useApi<Notices>(projectPath(slug, "notices"));
  return (
    <Wait q={q}>
      {(n) => (
        <div className="grid gap-4">
          <NoticeStats n={n} slug={slug} />
          <Panel title="What agents vote on next" icon={BellIcon} dense>
            {n.ballot.length ? (
              <DataTable
                dense
                rows={n.ballot}
                rowKey={(b) => b.slug}
                columns={[
                  { key: "slug", label: "Project", render: (b) => <ProjectLink slug={b.slug} /> },
                  { key: "suggestions", label: "Suggestions", align: "right", render: (b) => num(b.suggestions) },
                  { key: "amendments", label: "Rule changes", align: "right", render: (b) => num(b.amendments) },
                  { key: "proposals", label: "Proposals", align: "right", render: (b) => num(b.proposals) },
                ]}
              />
            ) : (
              <EmptyState className="m-4">Nothing to vote on: no suggestion or rule change needs votes right now.</EmptyState>
            )}
          </Panel>
        </div>
      )}
    </Wait>
  );
}

// The notice numbers that are not zero; when all are, one sentence says so.
function NoticeStats({ n, slug }: { n: Notices; slug: string }) {
  const tiles = [
    n.votes_open ? <Kpi key="v" icon={VoteIcon} n={n.votes_open} label="Open votes" sub="Agents are voting" /> : null,
    n.proposals_awaiting ? <Kpi key="p" icon={MessagesSquareIcon} n={n.proposals_awaiting} label="Proposals for you" sub="Waiting for your answer" href={hrefs.project(slug, "for-you")} tone="you" /> : null,
    n.commons_questions ? <Kpi key="q" icon={MessageSquareIcon} n={n.commons_questions} label="Questions on the shared board" sub="Across all projects" href={hrefs.project("commons")} /> : null,
  ].filter(Boolean);
  if (!tiles.length) return <p className="text-sm text-muted-foreground">Nothing waits right now: no open votes, no proposals for you and no questions on the shared board.</p>;
  return <KpiGrid className={cn("mb-0", tiles.length === 3 ? "md:grid-cols-3" : tiles.length === 2 ? "md:grid-cols-2" : "grid-cols-1 md:grid-cols-1")}>{tiles}</KpiGrid>;
}

// ----- cleanup ----------------------------------------------------------------

// Routine housekeeping (the runner's sweep), kept off the Overview.
export function CleanupTab({ slug }: { slug: string }) {
  return (
    <div className="grid gap-4">
      <p className="max-w-3xl text-sm leading-relaxed text-muted-foreground">PULSE tidies up after agents on its own: old branches, finished worktrees and threads that went quiet. This is what it found and did.</p>
      <HygienePanel project={slug} limit={30} />
    </div>
  );
}

// ----- team & tools -----------------------------------------------------------

// Team & tools: the lead agents who run this project (one card each, robot
// face and all), the toolkit they work with, and the machines that fit; the
// "env" segment keeps the setup notes, the constitution and the records.
export function TeamTab({ slug, sub, canDecide = true }: { slug: string; sub: "team" | "env"; canDecide?: boolean }) {
  if (sub === "env") return <EnvTab slug={slug} />;
  // Each panel carries its own title, count and action (Seat a lead, Change);
  // a heading above each would say the name twice.
  return (
    <div className="grid gap-6">
      <LeadsPanel slug={slug} canDecide={canDecide} />
      <ToolkitPanel slug={slug} canDecide={canDecide} />
      <MachinesPanel slug={slug} canDecide={canDecide} />
    </div>
  );
}

// ----- environment ----------------------------------------------------------

export function EnvTab({ slug }: { slug: string }) {
  const recs = useApi<RecordsPayload>("records?project=" + encodeURIComponent(slug) + "&kind=context", { refresh: 60000 });
  const all = useApi<RecordsPayload>("records?project=" + encodeURIComponent(slug), { refresh: 60000 });
  const c = useApi<Constitution>("constitution?project=" + encodeURIComponent(slug), { refresh: 60000 });
  const env = (recs.data?.records ?? []).find((r) => /environment/i.test(r.slug) || /environment/i.test(r.title));
  return (
    <div className="grid gap-8">
      <div className="grid gap-4 lg:grid-cols-2">
        <Panel title="How to build and run it" icon={SettingsIcon} actions={env ? <SeeAll href={hrefs.record(env.id, slug)}>Open the record</SeeAll> : null}>
          {recs.error && !recs.data ? (
            <ErrorNotice error={recs.error} onRetry={() => void recs.reload()} />
          ) : recs.data === undefined ? (
            <SkeletonRows n={4} />
          ) : env ? (
            <EnvBody id={env.id} slug={slug} />
          ) : (
            <p className="text-sm leading-relaxed text-muted-foreground">
              No notes yet. Agents write them from <Mono>docs/context/environment.md</Mono> the first time they work in this folder.
            </p>
          )}
        </Panel>
        <ConstitutionPanel c={c} slug={slug} />
      </div>
      <ProjectRecords slug={slug} q={all} />
    </div>
  );
}

// The project's shelf of the library: its decisions, plans, work units,
// features, memory and evolution records as cards, the dense table one click
// away, narrowed by kind or text, each opening in place. Every filter the
// library knows lives on the Records page; "browse" leads there pre-filtered.
function ProjectRecords({ slug, q }: { slug: string; q: Loaded<RecordsPayload> }) {
  const [kind, setKind] = useState("all");
  const [text, setText] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const [view, setView] = useViewMode("project-records");
  const all = q.data?.records ?? NONE;
  // The kinds this project holds, in the canonical order, unknown ones last.
  const kinds = useMemo(() => {
    const present = new Set<string>(all.map((r) => r.kind));
    const known: string[] = RECORD_KINDS;
    return known.filter((k) => present.has(k)).concat(Array.from(present).filter((k) => !known.includes(k)));
  }, [all]);
  const rows = useMemo(() => {
    const t = text.trim().toLowerCase();
    return all
      .filter((r) => (kind === "all" || r.kind === kind) && (!t || r.id.toLowerCase().includes(t) || r.title.toLowerCase().includes(t) || (r.tags || []).some((x) => x.toLowerCase().includes(t))))
      .sort(byNewest((r) => r.updated_at || r.date));
  }, [all, kind, text]);
  const chips = [{ id: "all", label: "All", n: all.length }, ...kinds.map((k) => ({ id: k, label: kindLabel(k), n: all.filter((r) => r.kind === k).length }))];
  const clear = () => {
    setKind("all");
    setText("");
  };
  return (
    <section aria-label="Records of this project">
      <SectionHeading icon={FolderIcon} count={q.data ? all.length : undefined} note="Decisions, plans and notes the agents keep about this project." action={<SeeAll href={hrefs.records({ project: slug })}>Open in Records</SeeAll>}>
        Records
      </SectionHeading>
      {all.length ? (
        <Toolbar className="mb-4">
          <Segments label="Kind" value={kind} onChange={setKind} options={chips} />
          <Search label="Search records" value={text} onChange={setText} className="w-full sm:w-56" />
          <span className="ml-auto flex items-center gap-2">
            {text.trim() ? <Count>{plural(rows.length, "record")}</Count> : null}
            <ViewToggle value={view} onChange={setView} />
          </span>
        </Toolbar>
      ) : null}
      <Wait q={q} rows={3}>
        {() => {
          if (!all.length)
            return (
              <EmptyState title="No records yet">
                Nothing is saved for {slug} yet. When PULSE sets a project up it reads the decisions, plans and notes in its docs folder; agents add more as they work.
              </EmptyState>
            );
          if (!rows.length)
            return (
              <EmptyState
                title="No matching records"
                action={
                  <Button size="sm" variant="outline" onClick={clear}>
                    Clear filters
                  </Button>
                }
              >
                No record matches. Try another word or kind.
              </EmptyState>
            );
          if (view === "table")
            return (
              <div className="overflow-x-auto rounded-lg border bg-card">
                <DataTable columns={RECORD_COLUMNS(false)} rows={rows} rowKey={(r) => r.id} onRow={(r) => setOpen(r.id)} initialSort="updated" caption={"Records of " + slug} />
              </div>
            );
          return <RecordCards rows={rows} onOpen={(r) => setOpen(r.id)} activeId={open} cols={3} />;
        }}
      </Wait>
      <RecordSheet id={open} project={slug} onClose={() => setOpen(null)} />
    </section>
  );
}

function EnvBody({ id, slug }: { id: string; slug: string }) {
  const q = useApi<{ body: string }>("records/" + encodeURIComponent(id) + "?project=" + encodeURIComponent(slug), { refresh: 60000 });
  return <Wait q={q}>{(d) => <Markdown source={d.body} className="max-h-[60vh] overflow-y-auto" />}</Wait>;
}
