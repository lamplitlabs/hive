"use client";

import { Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { BookOpenIcon, LayoutGridIcon, ListIcon, SearchIcon, SlidersHorizontalIcon, XIcon } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { DataTable } from "@/components/pulse/data-table";
import { FilterChip } from "@/components/pulse/filter-chip";
import { RowSkeleton } from "@/components/pulse/list-row";
import { PageHeader } from "@/components/pulse/page-header";
import { EmptyState, ErrorNotice, SkeletonPage } from "@/components/pulse/panel";
import { SidePanel, useOpenItem } from "@/components/pulse/side-panel";
import { useViewMode } from "@/components/pulse/view-toggle";
import { RECORD_COLUMNS, RecordBody, RecordCards, RecordGroups, TIER_KINDS, kindLabel, kindWord } from "@/components/records/records";
import { OWNER_KINDS, waitsOnYou } from "@/components/records/records-state";
import { KindTile } from "@/components/pulse/kind-tile";
import { kindFor } from "@/components/pulse/kind-tile-state";
import { awaitsDecision } from "@/components/records/decide-record";
import { projectLabel, registeredProjects, RECORD_KINDS, type Org, type RecordsPayload } from "@/lib/api";
import { plural, sentence } from "@/lib/format";
import { statusWord } from "@/lib/words";
import { useApi } from "@/lib/use-api";

export default function RecordsPage() {
  return (
    <Suspense fallback={<SkeletonPage rows={3} />}>
      <RecordsScreen />
    </Suspense>
  );
}

// The records library (spec "Records"): rows grouped by kind by default,
// cards one click away; the decisions that wait for the owner pinned at
// the top with Decline / Accept inline; reading opens a side panel set for
// reading. The filters live in the URL (kind, status, project, fw) so
// every existing link keeps working; search and area stay local.
function RecordsScreen() {
  const sp = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const kind = sp.get("kind") || "";
  const status = sp.get("status") || "";
  const statusKey = status.toLowerCase();
  const project = sp.get("project") || "";
  const [id, setId] = useOpenItem("id");
  // The framework's own docs (workflows, skills, templates…) are indexed too;
  // they stay behind one toggle so the shelf opens on the records that are yours.
  const showFramework = sp.get("fw") === "1" || (!!kind && !OWNER_KINDS.includes(kind));
  const [text, setText] = useState("");
  const [area, setArea] = useState("");
  const [more, setMore] = useState(false);
  // "cards" | "table" is what the toggle remembers; the table of old is the grouped list now.
  const [view, setView] = useViewMode("records", "table");
  const chipsRef = useRef<HTMLDivElement>(null);

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

  const q = useApi<RecordsPayload>("records" + (project ? "?project=" + encodeURIComponent(project) : ""), { refresh: 60000 });
  const org = useApi<Org>("org", { refresh: 60000 });
  // The decisions that wait for the owner (ADR-028); the shell asks for the same path, so this is the cached answer.
  const proposed = useApi<RecordsPayload>("records?kind=adr&status=Proposed", { refresh: 60000 });
  const waitingAll = useMemo(() => (proposed.data?.records ?? []).filter(awaitsDecision), [proposed.data]);
  const showingWaiting = kind === "adr" && status === "Proposed" && !project;
  const projectRows = useMemo(() => (org.data ? registeredProjects(org.data) : []), [org.data]);
  const projects = useMemo(() => (projectRows.length ? projectRows.map((p) => p.slug) : project ? [project] : []), [projectRows, project]);
  const projectNames = useMemo(() => new Map(projectRows.map((p) => [p.slug, projectLabel(p)])), [projectRows]);
  const all = useMemo(() => q.data?.records ?? [], [q.data]);
  const scope = useMemo(() => (showFramework ? all : all.filter((r) => OWNER_KINDS.includes(r.kind))), [all, showFramework]);
  // One entry per status word: the store may hold "Active" and "active".
  const statuses = useMemo(() => {
    const m = new Map<string, string>();
    for (const r of scope) if (r.status && !m.has(r.status.toLowerCase())) m.set(r.status.toLowerCase(), r.status);
    return Array.from(m.values()).sort((a, b) => statusWord(a).localeCompare(statusWord(b)));
  }, [scope]);
  const areas = useMemo(() => Array.from(new Set(scope.flatMap((r) => r.areas || []))).sort(), [scope]);
  // Every kind the store holds, in the canonical order, plus the one in the
  // URL so a deep link to an empty shelf still shows which chip is selected.
  const kinds = useMemo(() => {
    const present = new Set<string>(all.map((r) => r.kind));
    if (kind) present.add(kind);
    const known: string[] = RECORD_KINDS;
    return known.filter((k) => present.has(k)).concat(Array.from(present).filter((k) => !known.includes(k)));
  }, [all, kind]);
  const ownerKinds = useMemo(() => kinds.filter((k) => OWNER_KINDS.includes(k)), [kinds]);
  const frameworkKinds = useMemo(() => kinds.filter((k) => !OWNER_KINDS.includes(k)), [kinds]);
  // Everything but the kind filter, so each chip can count what it would show.
  const baseAll = useMemo(() => {
    const t = text.trim().toLowerCase();
    return all.filter((r) => (!statusKey || (r.status || "").toLowerCase() === statusKey) && (!area || (r.areas || []).includes(area)) && (!t || r.id.toLowerCase().includes(t) || r.title.toLowerCase().includes(t) || (r.tags || []).some((x) => x.toLowerCase().includes(t))));
  }, [all, statusKey, area, text]);
  const base = useMemo(() => (showFramework ? baseAll : baseAll.filter((r) => OWNER_KINDS.includes(r.kind))), [baseAll, showFramework]);
  const rows = useMemo(() => (kind ? base.filter((r) => r.kind === kind) : base), [base, kind]);
  const perKind = useMemo(() => {
    const m = new Map<string, number>();
    for (const r of baseAll) m.set(r.kind, (m.get(r.kind) || 0) + 1);
    return m;
  }, [baseAll]);
  const frameworkCount = useMemo(() => baseAll.filter((r) => !OWNER_KINDS.includes(r.kind)).length, [baseAll]);
  const waitingShown = useMemo(() => rows.filter(waitsOnYou).length, [rows]);
  const filtered = !!(kind || status || project || area || text || showFramework);
  const moreActive = !!(status || area);
  const open = useMemo(() => (id ? all.find((r) => r.id === id) : undefined), [all, id]);

  // Keep the selected chip in view when the row scrolls (phones, deep links).
  useEffect(() => {
    chipsRef.current?.querySelector<HTMLElement>('[aria-pressed="true"]')?.scrollIntoView({ inline: "nearest", block: "nearest" });
  }, [kind, kinds]);

  const clearAll = () => {
    setText("");
    setArea("");
    setParams({ kind: null, status: null, project: null, fw: null });
  };

  const filters = (
    <div className="mb-4 grid gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <div className="relative w-full shrink-0 sm:w-60">
          <SearchIcon className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden />
          <Input type="search" aria-label="Search records" placeholder="Search records" value={text} onChange={(e) => setText(e.target.value)} className="h-8 pl-8" />
        </div>
        <div ref={chipsRef} role="group" aria-label="Kind of record" className="scroll-strip -mx-1 flex min-w-0 flex-1 flex-nowrap items-center gap-1.5 px-1 py-0.5">
          <FilterChip active={!kind && !showFramework} count={q.data ? base.length : undefined} onClick={() => setParams({ kind: null, fw: null })}>
            All
          </FilterChip>
          {ownerKinds.map((k) => (
            <FilterChip key={k} active={kind === k} count={q.data ? perKind.get(k) || 0 : undefined} onClick={() => setParams({ kind: kind === k ? null : k })}>
              {kindLabel(k)}
            </FilterChip>
          ))}
          {frameworkKinds.length ? (
            <>
              <span aria-hidden className="mx-0.5 h-4 w-px shrink-0 bg-border" />
              <FilterChip
                active={showFramework && !kind}
                count={q.data ? frameworkCount : undefined}
                title={showFramework ? "Hide the guides PULSE comes with" : "Show the guides PULSE comes with: workflows, skills, prompts, templates and more"}
                onClick={() => setParams(showFramework ? { fw: null, kind: kind && !OWNER_KINDS.includes(kind) ? null : kind } : { fw: "1" })}
              >
                Built-in guides
              </FilterChip>
              {showFramework
                ? frameworkKinds.map((k) => (
                    <FilterChip key={k} active={kind === k} count={q.data ? perKind.get(k) || 0 : undefined} onClick={() => setParams({ kind: kind === k ? null : k })}>
                      {kindLabel(k)}
                    </FilterChip>
                  ))
                : null}
            </>
          ) : null}
        </div>
        <Button size="sm" variant={more || moreActive ? "secondary" : "ghost"} className="shrink-0" aria-expanded={more} aria-controls="records-more-filters" onClick={() => setMore((v) => !v)}>
          <SlidersHorizontalIcon /> More filters
          {moreActive ? <span className="tnum inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-full bg-foreground/10 px-1.5 text-xs leading-none">{[status, area].filter(Boolean).length}</span> : null}
        </Button>
        <Tabs value={view} onValueChange={(v) => setView(v === "cards" ? "cards" : "table")} className="ml-auto gap-0">
          <TabsList variant="segmented" aria-label="View" className="h-8">
            <Tooltip>
              <TooltipTrigger asChild>
                <TabsTrigger value="table" aria-label="Show as a list" className="px-2">
                  <ListIcon />
                </TabsTrigger>
              </TooltipTrigger>
              <TooltipContent>Show as a list</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <TabsTrigger value="cards" aria-label="Show as cards" className="px-2">
                  <LayoutGridIcon />
                </TabsTrigger>
              </TooltipTrigger>
              <TooltipContent>Show as cards</TooltipContent>
            </Tooltip>
          </TabsList>
        </Tabs>
      </div>
      {more || moreActive ? (
        <div id="records-more-filters" className="flex flex-wrap items-center gap-2">
          <Select value={project || "__all"} onValueChange={(v) => setParams({ project: v === "__all" ? null : v })}>
            <SelectTrigger aria-label="Project" size="sm" className={SELECT}>
              <SelectValue placeholder="Project" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all">All projects</SelectItem>
              {projects.map((p) => (
                <SelectItem key={p} value={p}>
                  {projectNames.get(p) || p}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select value={statusKey || "__all"} onValueChange={(v) => setParams({ status: v === "__all" ? null : statuses.find((s) => s.toLowerCase() === v) || v })}>
            <SelectTrigger aria-label="Status" size="sm" className={SELECT}>
              <SelectValue placeholder="Status" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__all">All statuses</SelectItem>
              {statuses.map((s) => (
                <SelectItem key={s} value={s.toLowerCase()}>
                  {statusWord(s)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {areas.length ? (
            <Select value={area || "__all"} onValueChange={(v) => setArea(v === "__all" ? "" : v)}>
              <SelectTrigger aria-label="Area" size="sm" className={SELECT}>
                <SelectValue placeholder="Area" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__all">All areas</SelectItem>
                {areas.map((a) => (
                  <SelectItem key={a} value={a}>
                    {sentence(a)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          ) : null}
          {filtered ? (
            <Button size="sm" variant="ghost" className="shrink-0 text-muted-foreground" onClick={clearAll}>
              <XIcon /> Clear filters
            </Button>
          ) : null}
        </div>
      ) : project && !more ? (
        // A project link lands here with the project set: show it even while "More filters" is folded.
        <div className="flex flex-wrap items-center gap-2 text-[13px] text-muted-foreground">
          <span>{"In " + (projectNames.get(project) || project)}</span>
          <Button size="xs" variant="ghost" className="text-muted-foreground" onClick={() => setParams({ project: null })} aria-label="Show every project">
            <XIcon /> All projects
          </Button>
        </div>
      ) : null}
    </div>
  );

  return (
    <div className="flex min-w-0 items-start gap-0">
      <main className="min-w-0 flex-1">
        <PageHeader title="Records" subtitle="Decisions, plans, guides and notes" />
        {filters}
        {q.error && !q.data ? (
          <ErrorNotice error={q.error} onRetry={() => void q.reload()} />
        ) : !q.data ? (
          <RowSkeleton rows={8} />
        ) : !rows.length ? (
          <EmptyState
            icon={<BookOpenIcon className="size-[18px]" />}
            title={all.length ? "No records match" : "No records yet"}
            action={
              filtered ? (
                <Button size="sm" variant="outline" onClick={clearAll}>
                  Clear filters
                </Button>
              ) : undefined
            }
          >
            {all.length ? "Try another kind or status, or clear the filters to see every record." : "Agents write decisions, plans and notes here once a project is set up and work starts."}
          </EmptyState>
        ) : (
          <>
            <p className="sr-only" aria-live="polite">
              {showingWaiting ? plural(rows.length, "decision") + " waiting for your answer" : "Showing " + plural(rows.length, "record") + (waitingShown ? ", " + plural(waitingShown, "decision") + " waiting for your answer" : "")}
            </p>
            {waitingAll.length && !waitingShown && !showingWaiting ? (
              // The waiting decisions sit outside the current filters: one quiet line, not a banner.
              <p className="mb-3 text-[13px] text-muted-foreground">
                {plural(waitingAll.length, "decision") + (waitingAll.length === 1 ? " waits" : " wait") + " for your answer elsewhere. "}
                <button type="button" className="text-link underline-offset-2 hover:underline" onClick={() => setParams({ kind: "adr", status: "Proposed", project: null, fw: null })}>
                  Show them
                </button>
              </p>
            ) : null}
            {view === "cards" ? (
              <RecordCards rows={rows} showProject={!project} onOpen={(r) => setId(r.id)} activeId={id} cols={3} />
            ) : kind ? (
              <div className="overflow-hidden rounded-lg border bg-card">
                <DataTable columns={RECORD_COLUMNS(!project, TIER_KINDS.includes(kind))} rows={rows} rowKey={(r) => r.project + "/" + r.id} onRow={(r) => setId(r.id)} rowAriaSelected={(r) => r.id === id} initialSort="updated" caption={kindLabel(kind)} />
              </div>
            ) : (
              <RecordGroups rows={rows} showProject={!project} projectNames={projectNames} activeId={id} onOpen={(r) => setId(r.id)} />
            )}
          </>
        )}
      </main>
      <SidePanel
        open={!!id}
        onClose={() => setId(null)}
        width="reading"
        title={open ? sentence(open.title) : id ? "Record " + id : "Record"}
        subtitle={open ? kindWord(open.kind) + " in " + (projectNames.get(open.project) || open.project) : undefined}
        lead={<KindTile kind={open ? kindFor(open.kind) : "record"} size={28} waiting={open ? awaitsDecision(open) : false} title={open ? kindWord(open.kind) : undefined} />}
      >
        {id ? <RecordBody id={id} project={project || open?.project || undefined} /> : null}
      </SidePanel>
    </div>
  );
}

const SELECT = "h-8 w-[calc(50%-0.25rem)] shrink-0 sm:w-40";
