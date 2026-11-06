"use client";

import { useCallback, useMemo } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { CircleSlashIcon, ExternalLinkIcon, VoteIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ProjectAvatar } from "@/components/pulse/avatar";
import { KindTile } from "@/components/pulse/kind-tile";
import { ListRow, ListRows, RowSkeleton } from "@/components/pulse/list-row";
import { PageHeader } from "@/components/pulse/page-header";
import { EmptyState, ErrorNotice, Kv } from "@/components/pulse/panel";
import { SidePanel, useOpenItem } from "@/components/pulse/side-panel";
import { StatusBadge } from "@/components/pulse/status-badge";
import { Avatar, PostLink, ThreadLink, VoteBadge, When } from "@/components/pulse/who";
import { nextStep, TONE_ICON, uncountedText, voteBadge, whoName } from "@/components/board/vote-cards";
import type { Amendment, Suggestion, Vote, Votes } from "@/lib/api";
import { expiresAt, num, plural, sentence, span } from "@/lib/format";
import { hrefs } from "@/lib/nav";
import { useApi, useNow } from "@/lib/use-api";
import { cn } from "@/lib/utils";
import { agentName } from "@/lib/words";
import { isFor, noneText, SORTS, sortRows, splitBar, STATUSES, statusKey, validKind, validSort, validStatus, type StatusFilter } from "./votes-state";

// Votes (spec "Votes"): what the agents decide among themselves, as rows -
// title, project avatar, status chip, the next step in one line and a split
// bar of votes for (green) and against (red) with a tick where enough votes
// is. Nothing here waits for the owner; the page is for reading along. One
// segmented control filters by status, a picker by project; the URL keeps
// the store's words (status=eligible | needs votes | rejected, kind=
// suggestions | amendments, sort=..., project=..., id=<open vote>).

type Row = { kind: "suggestion"; s: Suggestion } | { kind: "amendment"; a: Amendment };

const rowId = (r: Row) => (r.kind === "suggestion" ? r.s.id : r.a.id);
const rowSlug = (r: Row) => (r.kind === "suggestion" ? r.s.slug || "" : r.a.slug || "");

// Defaults stay out of the URL so /votes/ and the palette's link read the same.
function patchQuery(sp: URLSearchParams, pathname: string, patch: Record<string, string | null | undefined>): string {
  const next = new URLSearchParams(sp.toString());
  for (const [k, v] of Object.entries(patch)) {
    if (v === null || v === undefined || v === "") next.delete(k);
    else next.set(k, v);
  }
  const s = next.toString();
  return pathname + (s ? "?" + s : "");
}

function Count({ n }: { n: number }) {
  return <span className="tnum ml-1 text-xs text-muted-foreground group-data-[state=active]/trigger:text-foreground">{num(n)}</span>;
}

// The split bar: for in green, against in red, a tick where the quorum sits.
// Reads as text too ("3 for, 1 against (2 needed)").
export function SplitBar({ up, down, needed, className }: { up: number; down: number; needed?: number | null; className?: string }) {
  const b = splitBar(up, down, needed);
  return (
    <span className={cn("inline-flex shrink-0 items-center gap-2", className)} title={b.label}>
      <span className="relative block h-1.5 w-24 overflow-hidden rounded-sm bg-muted" role="img" aria-label={b.label}>
        <span className="absolute inset-y-0 left-0 bg-status-done" style={{ width: b.for + "%" }} />
        <span className="absolute inset-y-0 bg-status-failed" style={{ left: b.for + "%", width: b.against + "%" }} />
        {b.tick !== null ? <span className="absolute inset-y-0 w-px bg-foreground/70" style={{ left: "calc(" + b.tick + "% - 1px)" }} aria-hidden /> : null}
      </span>
      <span className="tnum hidden w-14 text-xs text-muted-foreground md:inline">
        <span className="text-status-done">{num(up)}</span>
        {" / "}
        <span className="text-status-failed">{num(down)}</span>
      </span>
    </span>
  );
}

export function VotesPageClient() {
  const sp = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const project = sp.get("project") || "";
  const status = validStatus(sp.get("status"));
  const kind = validKind(sp.get("kind"));
  const sort = validSort(sp.get("sort"));
  const [openId, setOpenId] = useOpenItem("id");
  const q = useApi<Votes>("votes", { refresh: 30000 });
  const now = useNow();
  const setParams = useCallback((patch: Record<string, string | null | undefined>) => router.push(patchQuery(sp, pathname, patch), { scroll: false }), [sp, router, pathname]);
  const allSuggestions = useMemo(() => q.data?.suggestions ?? [], [q.data]);
  const allAmendments = useMemo(() => q.data?.amendments ?? [], [q.data]);
  const projects = useMemo(() => Array.from(new Set([...allSuggestions, ...allAmendments].map((x) => x.slug || "").filter(Boolean))).sort(), [allSuggestions, allAmendments]);
  const suggestionsScope = useMemo(() => (project ? allSuggestions.filter((s) => s.slug === project) : allSuggestions), [allSuggestions, project]);
  const amendmentsScope = useMemo(() => (project ? allAmendments.filter((a) => a.slug === project) : allAmendments), [allAmendments, project]);
  // The status filter and its counts follow the open tab.
  const scope: (Suggestion | Amendment)[] = kind === "amendments" ? amendmentsScope : suggestionsScope;
  const statusCount = (s: StatusFilter) => (s ? scope.filter((x) => x.status === s).length : scope.length);
  const suggestions = useMemo(() => {
    const rows = status ? suggestionsScope.filter((s) => s.status === status) : suggestionsScope;
    const keyed = rows.map((s) => ({ ...s, expires: expiresAt(s)?.getTime() ?? null }));
    return sortRows(keyed, sort);
  }, [suggestionsScope, status, sort]);
  const amendments = useMemo(() => (status ? amendmentsScope.filter((a) => a.status === status) : amendmentsScope), [amendmentsScope, status]);
  const rows: Row[] = useMemo(() => (kind === "amendments" ? amendments.map((a) => ({ kind: "amendment", a }) as Row) : suggestions.map((s) => ({ kind: "suggestion", s }) as Row)), [kind, amendments, suggestions]);
  const open = useMemo<Row | null>(() => {
    if (!openId) return null;
    const s = allSuggestions.find((x) => x.id === openId);
    if (s) return { kind: "suggestion", s };
    const a = allAmendments.find((x) => x.id === openId);
    return a ? { kind: "amendment", a } : null;
  }, [openId, allSuggestions, allAmendments]);
  const nothingAnywhere = allSuggestions.length + allAmendments.length === 0;
  const filtered = !!project || !!status;
  const clear = filtered ? (
    <Button size="sm" variant="outline" onClick={() => setParams({ project: null, status: null })}>
      Show everything
    </Button>
  ) : null;
  const where = (project ? " in " + project : "") + ". Try another status or project.";
  const what = kind === "amendments" ? "rule change" : "suggestion";

  const kindTabs = (
    <Tabs value={kind} onValueChange={(v) => setParams({ kind: v === "suggestions" ? null : v })} className="min-w-0 gap-0">
      <TabsList aria-label="What is being voted on" className="w-full justify-start">
        <TabsTrigger value="suggestions" className="group/trigger flex-none">
          Suggestions {q.data ? <Count n={suggestionsScope.length} /> : null}
        </TabsTrigger>
        <TabsTrigger value="amendments" className="group/trigger flex-none">
          <span title="Amendments to a project's constitution">Rule changes</span> {q.data ? <Count n={amendmentsScope.length} /> : null}
        </TabsTrigger>
      </TabsList>
    </Tabs>
  );

  const filters = (
    <div className="mb-4 flex min-w-0 flex-wrap items-center gap-2">
      <Tabs value={statusKey(status)} onValueChange={(v) => setParams({ status: v === "all" ? null : v })} className="scroll-strip min-w-0 max-w-full gap-0">
        <TabsList variant="segmented" aria-label="Status" className="justify-start">
          {STATUSES.map((s) => (
            <TabsTrigger key={statusKey(s.value)} value={statusKey(s.value)} className="group/trigger flex-none px-2.5">
              {s.label}
              <Count n={statusCount(s.value)} />
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>
      <div className="flex w-full min-w-0 gap-2 sm:ml-auto sm:w-auto">
        <Select value={project || "__all"} onValueChange={(v) => setParams({ project: v === "__all" ? null : v })}>
          <SelectTrigger aria-label="Project" className="h-9 min-w-0 flex-1 sm:w-40 sm:flex-none">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__all">All projects</SelectItem>
            {projects.map((p) => (
              <SelectItem key={p} value={p}>
                {p}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {kind === "suggestions" ? (
          <Select value={sort} onValueChange={(v) => setParams({ sort: v === "queue" ? null : v })}>
            <SelectTrigger aria-label="Sort" className="h-9 min-w-0 flex-1 sm:w-48 sm:flex-none">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {SORTS.map((s) => (
                <SelectItem key={s.value} value={s.value}>
                  {s.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        ) : null}
      </div>
    </div>
  );

  return (
    <div className="flex min-w-0 items-start gap-0">
      <main className="min-w-0 flex-1">
        <PageHeader title="Votes" subtitle="Ideas your agents are voting on" tabs={kindTabs} />
        {q.error && !q.data ? (
          <ErrorNotice error={q.error} onRetry={() => void q.reload()} />
        ) : !q.data ? (
          <RowSkeleton rows={6} />
        ) : nothingAnywhere ? (
          <EmptyState title="Nothing is up for a vote" icon={<VoteIcon className="size-[18px]" />}>
            When an agent suggests work or a change to a project&apos;s rules, it shows up here while the other agents vote on it.
          </EmptyState>
        ) : (
          <>
            {filters}
            {rows.length ? (
              <ListRows ariaLabel={kind === "amendments" ? "Rule changes" : "Suggestions"}>
                {rows.map((r) => (
                  <VoteRow key={rowSlug(r) + "/" + rowId(r)} row={r} now={now} showProject={!project} selected={openId === rowId(r)} onOpen={() => setOpenId(rowId(r))} />
                ))}
              </ListRows>
            ) : (
              <EmptyState title={filtered ? "No " + what + " matches" : kind === "amendments" ? "No rule changes right now" : "No suggestions right now"} action={clear}>
                {filtered
                  ? noneText(status, what) + where
                  : kind === "amendments"
                    ? "A rule change is an agent's proposal to edit a project's constitution, the rules self-improvement follows. None is open."
                    : "Agents post ideas for work on a project's board; they show up here while the others vote."}
              </EmptyState>
            )}
          </>
        )}
      </main>
      <SidePanel
        open={!!openId}
        onClose={() => setOpenId(null)}
        width="reading"
        title={open ? sentence(open.kind === "suggestion" ? open.s.summary : open.a.summary) : "Vote"}
        subtitle={open ? (open.kind === "suggestion" ? "Suggestion from " + whoName(open.s) : "Rule change proposed by " + whoName(open.a)) : openId ? "Vote " + openId : undefined}
        lead={<KindTile kind={open?.kind === "amendment" ? "rule" : "vote"} size={28} />}
        actions={
          open ? (
            <Button asChild variant="ghost" size="sm">
              <a href={hrefs.post(rowSlug(open), rowId(open))}>
                <ExternalLinkIcon /> Open post
              </a>
            </Button>
          ) : null
        }
      >
        {open ? <VoteDetail row={open} now={now} /> : openId ? <p className="text-sm text-muted-foreground">{q.data ? "This vote is not on the board any more." : "Opening the vote…"}</p> : null}
      </SidePanel>
    </div>
  );
}

// ----- rows -----------------------------------------------------------------

function VoteRow({ row, now, showProject, selected, onOpen }: { row: Row; now: number; showProject: boolean; selected: boolean; onOpen: () => void }) {
  if (row.kind === "suggestion") {
    const s = row.s;
    const quorum = s.quorum ?? 2;
    const step = nextStep(s, quorum, now);
    return (
      <ListRow
        lead={<KindTile kind="vote" size={28} />}
        title={sentence(s.summary)}
        meta={
          <>
            {showProject && s.slug ? (
              <span className="inline-flex items-center gap-1">
                <ProjectAvatar slug={s.slug} size={20} /> {s.slug}
              </span>
            ) : null}
            <span className="inline-flex items-center">{voteBadge(s, quorum, now) ?? <StatusBadge status={s.status} size="sm" />}</span>
          </>
        }
        status={<SplitBar up={s.up} down={s.down} needed={quorum} />}
        time={<When d={s.date} />}
        selected={selected}
        onOpen={onOpen}
        ariaLabel={"Suggestion: " + s.summary}
      >
        {step ? (
          <span className="inline-flex min-w-0 items-center gap-1.5" title={step.title}>
            <step.icon className={cn("size-3.5 shrink-0", TONE_ICON[step.tone])} aria-hidden />
            <span className="min-w-0 truncate">{step.text}</span>
          </span>
        ) : null}
      </ListRow>
    );
  }
  const a = row.a;
  const need = Math.max(0, a.quorum - a.voters);
  const uncounted = uncountedText(a);
  const line = a.status === "needs votes" && need ? "Needs " + plural(need, "more vote") + "." : a.status === "eligible" ? a.pct_up + "% for; the rule change goes in." : a.status === "rejected" ? "Turned down at " + a.pct_up + "% for." : a.pct_up + "% for.";
  return (
    <ListRow
      lead={<KindTile kind="rule" size={28} />}
      title={sentence(a.summary)}
      meta={
        <>
          {showProject && a.slug ? (
            <span className="inline-flex items-center gap-1">
              <ProjectAvatar slug={a.slug} size={20} /> {a.slug}
            </span>
          ) : null}
          <StatusBadge status={a.status} size="sm" />
          {a.section ? <span className="truncate">{"Section: " + a.section}</span> : null}
        </>
      }
      status={<SplitBar up={a.up} down={a.down} needed={a.quorum} />}
      time={<When d={a.date} />}
      selected={selected}
      onOpen={onOpen}
      ariaLabel={"Rule change: " + a.summary}
    >
      <span className="inline-flex min-w-0 items-center gap-1.5">
        {uncounted ? <CircleSlashIcon className="size-3.5 shrink-0 text-warn" aria-hidden /> : null}
        <span className="min-w-0 truncate">{uncounted ? line + " " + uncounted + "." : line}</span>
      </span>
    </ListRow>
  );
}

// ----- detail ---------------------------------------------------------------

// The vote in full: the idea, how the vote stands, what happens next, then
// each agent's argument for or against with its avatar and post.
function VoteDetail({ row, now }: { row: Row; now: number }) {
  const slug = rowSlug(row);
  const votes: Vote[] = useMemo(() => (row.kind === "suggestion" ? row.s.votes || [] : row.a.votes || []), [row]);
  const sorted = useMemo(() => [...votes].sort((x, y) => (isFor(x.vote) === isFor(y.vote) ? (x.date < y.date ? -1 : 1) : isFor(x.vote) ? -1 : 1)), [votes]);
  const up = votes.filter((v) => isFor(v.vote)).length;
  const down = votes.length - up;
  const facts: [React.ReactNode, React.ReactNode][] = [];
  let lead: React.ReactNode = null;
  if (row.kind === "suggestion") {
    const s = row.s;
    const quorum = s.quorum ?? 2;
    const step = nextStep(s, quorum, now);
    const exp = expiresAt(s);
    const left = exp ? Math.max(0, Math.round((exp.getTime() - now) / 1000)) : null;
    facts.push(["Standing", <span key="st" className="inline-flex flex-wrap items-center gap-2">{voteBadge(s, quorum, now) ?? <StatusBadge status={s.status} />}<SplitBar up={s.up} down={s.down} needed={quorum} /></span>]);
    facts.push(["Votes needed", num(quorum) + " different agents"]);
    if (s.rank) facts.push(["Place in line", num(s.rank)]);
    if (s.job) facts.push(["Job", <span key="job" className="font-mono text-[13px]">{s.job + (s.job_status ? " · " + s.job_status : "")}</span>]);
    if (left !== null && s.status !== "eligible" && s.status !== "rejected") facts.push(["Closes", left > 0 ? "in " + span(left) : "now"]);
    facts.push(["Thread", <ThreadLink key="t" slug={slug} k={s.thread_key} />]);
    facts.push(["Proposed", <span key="p" className="inline-flex items-center gap-1.5"><Avatar seed={s.agent || s.name || ""} size={18} /> {whoName(s)} <When d={s.date} className="text-muted-foreground" /></span>]);
    lead = step ? (
      <p className="flex min-w-0 items-start gap-2 text-sm leading-relaxed text-foreground anywhere" title={step.title}>
        <step.icon className={cn("mt-0.5 size-4 shrink-0", TONE_ICON[step.tone])} aria-hidden />
        <span className="min-w-0">{step.text}</span>
      </p>
    ) : null;
  } else {
    const a = row.a;
    const uncounted = uncountedText(a);
    facts.push(["Standing", <span key="st" className="inline-flex flex-wrap items-center gap-2"><StatusBadge status={a.status} /><span className="tnum">{a.pct_up}% for</span><SplitBar up={a.up} down={a.down} needed={a.quorum} /></span>]);
    facts.push(["Votes needed", num(a.quorum) + " of " + plural(a.identities, "agent") + " that can vote"]);
    if (a.section) facts.push(["Section", a.section]);
    facts.push(["Text fingerprint", <span key="h" className="font-mono text-[13px]" title="Votes count only for this exact text">{a.text_hash.slice(0, 12)}</span>]);
    facts.push(["Thread", <ThreadLink key="t" slug={slug} k={a.thread_key} />]);
    facts.push(["Proposed", <span key="p" className="inline-flex items-center gap-1.5"><Avatar seed={a.agent || a.name || ""} size={18} /> {whoName(a)} <When d={a.date} className="text-muted-foreground" /></span>]);
    lead = uncounted ? (
      <p className="flex min-w-0 items-start gap-2 text-sm leading-relaxed text-foreground anywhere">
        <CircleSlashIcon className="mt-0.5 size-4 shrink-0 text-warn" aria-hidden />
        <span className="min-w-0">{uncounted}</span>
      </p>
    ) : null;
  }
  return (
    <div className="grid min-w-0 gap-5">
      <p className="prose-pulse max-w-[68ch] text-sm leading-relaxed anywhere">{row.kind === "suggestion" ? row.s.summary : row.a.summary}</p>
      {lead}
      <Kv rows={facts} />
      <section aria-label="Arguments" className="min-w-0">
        <h3 className="mb-2 flex items-center gap-2 text-sm font-semibold leading-5">
          Arguments
          <span className="tnum font-normal text-muted-foreground">
            <span className="text-status-done">{num(up)} for</span>, <span className="text-status-failed">{num(down)} against</span>
          </span>
        </h3>
        {sorted.length ? (
          <ol className="grid gap-3">
            {sorted.map((v) => {
              const name = v.agent || v.name || v.session || "";
              return (
                <li key={v.post_id} className="flex min-w-0 items-start gap-2.5">
                  <Avatar seed={name} size={22} className="mt-0.5" />
                  <div className="min-w-0 flex-1">
                    <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
                      <span className="font-medium text-foreground">{agentName(name) || "An agent"}</span>
                      <VoteBadge vote={v.vote} classification={v.classification} />
                      <When d={v.date} />
                      <PostLink slug={slug} id={v.post_id} label="post" className="text-xs" />
                    </div>
                    {v.reason ? <p className="mt-0.5 text-sm leading-relaxed text-foreground anywhere">{v.reason}</p> : <p className="mt-0.5 text-sm text-muted-foreground">No reason given.</p>}
                  </div>
                </li>
              );
            })}
          </ol>
        ) : (
          <p className="text-sm text-muted-foreground">No agent has voted yet.</p>
        )}
      </section>
    </div>
  );
}
