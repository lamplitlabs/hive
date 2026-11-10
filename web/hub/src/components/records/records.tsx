"use client";

import Link from "next/link";
import { useEffect, useMemo, useRef, useState } from "react";
import { BookOpenIcon, BrainIcon, FileTextIcon, HistoryIcon, LightbulbIcon, ListChecksIcon, ListIcon, MapIcon, MessageSquareTextIcon, PackageIcon, ScaleIcon, TrendingUpIcon, WrenchIcon } from "lucide-react";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { Column } from "@/components/hive/data-table";
import { CardGrid, ItemCard, ItemCardKind, ItemCardMeta, ItemCardTitle } from "@/components/hive/item-card";
import { ProjectAvatar } from "@/components/hive/avatar";
import { KindTile } from "@/components/hive/kind-tile";
import { kindFor } from "@/components/hive/kind-tile-state";
import { ListRow, ListRows } from "@/components/hive/list-row";
import { Markdown } from "@/components/hive/markdown";
import { Disclosure, ErrorNotice, Kv, Notice, SkeletonRows } from "@/components/hive/panel";
import { SectionHeading } from "@/components/hive/section-heading";
import { Mono, ProjectLink, StatusBadge, When, Who } from "@/components/hive/who";
import { awaitsDecision, RecordDecisionButtons } from "@/components/records/decide-record";
import { RecordDraftPanel } from "@/components/records/draft-panel";
import { groupRecords, headings, slugify, wantsContents, type Heading } from "@/components/records/records-state";
import { hrefs } from "@/lib/nav";
import { evidenceLabel, recordEvidence } from "@/lib/record-evidence";
import { enc, type RecordDetail, type RecordRow } from "@/lib/api";
import { useApi, useNow } from "@/lib/use-api";
import { fmtDate, parseDate, rel, sentence } from "@/lib/format";
import { recordKindWord, statusWord, tierWord } from "@/lib/words";
import { cn } from "@/lib/utils";

// The names of the shelves (filter chips): what a reader calls a group of
// these records. RECORD_KIND in lib/words holds the singular badge words.
const SHELF: Record<string, string> = {
  adr: "Decisions",
  plan: "Plans",
  wu: "Work items",
  feature: "Features",
  memory: "Memory",
  evolution: "Self-improvement",
  workflow: "Workflows",
  skill: "Skills",
  guide: "Guides",
  context: "Context",
  architecture: "Architecture",
  instruction: "Instructions",
  template: "Templates",
  usage: "Usage notes",
  prompt: "Prompts",
};

// Singular words for the kinds lib/words does not name yet.
const EXTRA_KIND: Record<string, string> = { context: "Context", architecture: "Architecture", instruction: "Instruction", usage: "Usage note" };

const KIND_ICON: Record<string, React.ComponentType<{ className?: string }>> = {
  adr: ScaleIcon,
  plan: MapIcon,
  wu: ListChecksIcon,
  feature: PackageIcon,
  memory: BrainIcon,
  evolution: TrendingUpIcon,
  workflow: WrenchIcon,
  skill: LightbulbIcon,
  guide: BookOpenIcon,
  prompt: MessageSquareTextIcon,
};

// A shelf's name ("Decisions", "Work items"); unknown kinds read in sentence case.
export const kindLabel = (k: string) => SHELF[k] || recordKindWord(k);
// One record's kind ("Decision", "Work item").
export const kindWord = (k: string) => EXTRA_KIND[k] || recordKindWord(k);

// What a record is, as its first words, with the kind's icon.
export function RecordKindBadge({ kind, className }: { kind: string; className?: string }) {
  return (
    <ItemCardKind icon={KIND_ICON[kind] || FileTextIcon} className={className}>
      {kindWord(kind)}
    </ItemCardKind>
  );
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

// "2026-09-02" -> "2 Sep 2026"; anything unreadable stays as written.
function dayWords(d?: string | null): string {
  const t = parseDate(d && /^\d{4}-\d{2}-\d{2}$/.test(d) ? d + "T00:00:00Z" : d);
  if (!t) return d || "";
  return t.getUTCDate() + " " + MONTHS[t.getUTCMonth()] + " " + t.getUTCFullYear();
}

function Updated({ d, className }: { d?: string | null; className?: string }) {
  const now = useNow();
  if (!d) return null;
  return (
    <time dateTime={d} title={fmtDate(d)} className={cn("whitespace-nowrap", className)}>
      {"Updated " + rel(d, now)}
    </time>
  );
}

// Delivery in words: "Delivered on 2 Sep 2026", "Not started yet".
export function deliveryWords(r: Pick<RecordRow, "delivery" | "delivered">): string {
  const d = (r.delivery || "").trim().toLowerCase();
  if (!d && !r.delivered) return "";
  if (d === "delivered" || (!d && r.delivered)) return r.delivered ? "Delivered on " + dayWords(r.delivered) : "Delivered";
  if (d === "not started" || d === "not_started") return "Not started yet";
  return statusWord(d) + (r.delivered ? " (" + dayWords(r.delivered) + ")" : "");
}

function Delivery({ r }: { r: RecordRow }) {
  const words = deliveryWords(r);
  if (!words) return null;
  const done = !!r.delivered;
  return (
    <span className="inline-flex items-center gap-1.5" title={"Delivery: " + r.delivery + (r.delivered ? ", " + r.delivered : "")}>
      <span className={cn("size-1.5 shrink-0 rounded-full", done ? "bg-ok" : "bg-muted-foreground/40")} aria-hidden />
      {words}
    </span>
  );
}

// The size of change only means something for the records that plan work
// (decisions, plans, work items, features); memory and guides have none.
export const TIER_KINDS: string[] = ["adr", "plan", "wu", "feature"];
const tierOf = (r: Pick<RecordRow, "kind" | "tier">) => (r.tier != null && TIER_KINDS.includes(r.kind) ? tierWord(r.tier) : null);

function Tier({ r }: { r: Pick<RecordRow, "kind" | "tier"> }) {
  const t = tierOf(r);
  return t?.short ? <span title={t.long}>{t.short}</span> : null;
}

// Up to four tags as quiet words, so a record found by a tag shows why.
function Tags({ tags }: { tags?: string[] }) {
  const all = tags || [];
  if (!all.length) return null;
  const shown = all.slice(0, 4);
  return <span className="anywhere" title={"Tags: " + all.join(", ")}>{"Tagged " + shown.join(", ") + (all.length > shown.length ? " and " + (all.length - shown.length) + " more" : "")}</span>;
}

// `showTier` = false drops the "Change" column (a shelf of memory or guides has no size of change).
export const RECORD_COLUMNS = (showProject: boolean, showTier = true): Column<RecordRow>[] => [
  // The title keeps whole words and a readable width; on a phone the kind, project and date make room for it.
  { key: "title", label: "Title", className: "min-w-44 break-words", sort: (r) => r.title, render: (r) => <Link href={hrefs.record(r.id, r.project)} title={r.id} className="font-medium hover:underline">{sentence(r.title)}</Link> },
  { key: "kind", label: "Kind", nowrap: true, hide: "sm", sort: (r) => r.kind, render: (r) => <RecordKindBadge kind={r.kind} className="font-normal" /> },
  { key: "status", label: "Status", nowrap: true, sort: (r) => r.status, render: (r) => <StatusBadge status={r.status} /> },
  ...(showProject ? ([{ key: "project", label: "Project", hide: "sm", sort: (r) => r.project, render: (r) => <ProjectLink slug={r.project} className="font-normal" /> }] as Column<RecordRow>[]) : []),
  ...(showTier ? ([{ key: "tier", label: "Change", hide: "md", sort: (r) => (tierOf(r) ? (r.tier ?? -1) : -1), render: (r) => <span className="text-[13px] text-muted-foreground"><Tier r={r} /></span> }] as Column<RecordRow>[]) : []),
  { key: "areas", label: "About", hide: "lg", render: (r) => <span className="text-[13px] text-muted-foreground">{(r.areas || []).join(", ")}</span> },
  { key: "delivery", label: "Delivery", hide: "lg", sort: (r) => r.delivered || r.delivery || "", render: (r) => <span className="text-[13px] text-muted-foreground"><Delivery r={r} /></span> },
  { key: "rev", label: "Revision", nowrap: true, hide: "lg", sort: (r) => r.current_rev, render: (r) => <span className="tnum text-[13px] text-muted-foreground">{r.current_rev}</span> },
  { key: "updated", label: "Updated", nowrap: true, hide: "sm", sort: (r) => r.updated_at || r.date, render: (r) => <When d={r.updated_at || r.date} className="text-muted-foreground" /> },
  { key: "id", label: "Id", nowrap: true, hide: "md", sort: (r) => r.kind + (r.number ?? 0).toString().padStart(5, "0") + r.slug, render: (r) => <span className="text-xs text-muted-foreground">{r.id}</span> },
];

// One record as a library card: its title first, then what it is (kind and
// status), whose it is and when it changed. The small facts (area, tier,
// delivery) follow; the id stays in the tooltip of the date for whoever needs it.
// A decision that waits for the owner carries the marigold rule.
export function RecordCard({ r, showProject, onOpen, active }: { r: RecordRow; showProject?: boolean; onOpen?: (r: RecordRow) => void; active?: boolean }) {
  const open = onOpen ? () => onOpen(r) : undefined;
  const areas = r.areas || [];
  const waits = awaitsDecision(r);
  return (
    <ItemCard onOpen={open} href={open ? undefined : hrefs.record(r.id, r.project)} active={active} highlight={waits} ariaLabel={kindWord(r.kind) + " " + r.id + ": " + r.title}>
      <ItemCardTitle chevron={!!open}>{sentence(r.title)}</ItemCardTitle>
      <div className="flex min-w-0 flex-wrap items-center gap-x-2.5 gap-y-1.5 text-[13px]">
        <RecordKindBadge kind={r.kind} />
        <StatusBadge status={r.status} />
        {showProject ? (
          // A link card cannot nest another link, so the project is plain text there.
          open ? <ProjectLink slug={r.project} className="font-normal text-muted-foreground" /> : <span className="text-muted-foreground">{r.project}</span>
        ) : null}
      </div>
      {areas.length || tierOf(r)?.short || deliveryWords(r) || r.tags?.length ? (
        <ItemCardMeta className="gap-x-3.5">
          {areas.length ? <span>{"About " + areas.join(", ")}</span> : null}
          <Tier r={r} />
          <Delivery r={r} />
          <Tags tags={r.tags} />
        </ItemCardMeta>
      ) : null}
      <div className="mt-auto flex min-w-0 flex-wrap items-center justify-between gap-x-3 gap-y-1 pt-1 text-[13px] text-muted-foreground" title={"Record " + r.id + (r.current_rev > 1 ? ", revision " + r.current_rev : "")}>
        <Updated d={r.updated_at || r.date} />
      </div>
    </ItemCard>
  );
}

export function RecordCards({ rows, showProject, onOpen, activeId, cols = 3 }: { rows: RecordRow[]; showProject?: boolean; onOpen?: (r: RecordRow) => void; activeId?: string | null; cols?: 1 | 2 | 3 | 4 }) {
  if (!rows.length) return null;
  return (
    <CardGrid cols={cols}>
      {rows.map((r) => (
        <RecordCard key={r.project + "/" + r.id} r={r} showProject={showProject} onOpen={onOpen} active={activeId === r.id} />
      ))}
    </CardGrid>
  );
}

// One record as a 44 px list row: the kind tile, the title, then status,
// project and area as meta; updated on the right. A decision that waits
// for the owner carries a marigold tile and its Decline / Accept in reach.
export function RecordListRow({ r, showProject, projectName, selected, onOpen, decide }: { r: RecordRow; showProject?: boolean; projectName?: string; selected?: boolean; onOpen: (r: RecordRow) => void; decide?: boolean }) {
  const waits = awaitsDecision(r);
  const areas = r.areas || [];
  return (
    <ListRow
      lead={<KindTile kind={kindFor(r.kind)} size={28} waiting={waits} title={kindWord(r.kind)} />}
      title={sentence(r.title)}
      ariaLabel={kindWord(r.kind) + " " + r.id + ": " + r.title}
      selected={selected}
      onOpen={() => onOpen(r)}
      meta={
        <>
          <StatusBadge status={r.status} size="sm" />
          {showProject ? (
            <span className="inline-flex min-w-0 items-center gap-1.5">
              <ProjectAvatar slug={r.project} name={projectName} size={20} />
              <span className="truncate">{projectName || r.project}</span>
            </span>
          ) : null}
          {areas.length ? <AreaChip areas={areas} /> : null}
        </>
      }
      time={<When d={r.updated_at || r.date} />}
      actions={waits && decide ? <RecordDecisionButtons record={r} size="xs" /> : undefined}
      actionsVisible={waits && decide}
    />
  );
}

// The first area as a quiet chip; the rest ride in the tooltip.
function AreaChip({ areas }: { areas: string[] }) {
  const [first, ...rest] = areas;
  return (
    <span className="inline-flex h-5 max-w-40 shrink items-center truncate rounded-md border border-border px-1.5 text-xs font-medium text-muted-foreground" title={"About " + areas.join(", ")}>
      <span className="truncate">{sentence(first!) + (rest.length ? " +" + rest.length : "")}</span>
    </span>
  );
}

// The default view: rows grouped by kind, each group under its kind's
// heading and count. The decisions that wait for the owner come first in
// their own marigold group, with Decline / Accept inline - that group
// stands in for the old "N decisions wait" banner.
export function RecordGroups({ rows, showProject, projectNames, activeId, onOpen }: { rows: RecordRow[]; showProject?: boolean; projectNames?: Map<string, string>; activeId?: string | null; onOpen: (r: RecordRow) => void }) {
  const groups = useMemo(() => groupRecords(rows), [rows]);
  if (!groups.length) return null;
  return (
    <div className="grid gap-6">
      {groups.map(({ group, rows: mine }) => {
        const waiting = group.key === "waiting";
        return (
          <section key={group.key} aria-labelledby={"records-" + group.key}>
            <SectionHeading id={"records-" + group.key} icon={GROUP_ICON[group.key] || FileTextIcon} count={mine.length} countTone={waiting ? "you" : "neutral"} note={waiting ? "Decline or accept each one here, or open it to read the reasoning first." : undefined}>
              {group.label}
            </SectionHeading>
            <ListRows ariaLabel={group.label}>
              {mine.map((r) => (
                <RecordListRow key={r.project + "/" + r.id} r={r} showProject={showProject} projectName={projectNames?.get(r.project)} selected={activeId === r.id} onOpen={onOpen} decide={waiting} />
              ))}
            </ListRows>
          </section>
        );
      })}
    </div>
  );
}

const GROUP_ICON: Record<string, React.ComponentType<{ className?: string; strokeWidth?: number; "aria-hidden"?: boolean }>> = {
  waiting: ScaleIcon,
  adr: ScaleIcon,
  plan: MapIcon,
  feature: PackageIcon,
  wu: ListChecksIcon,
  memory: BrainIcon,
  evolution: TrendingUpIcon,
  framework: BookOpenIcon,
};

// The record as an overlay sheet, for the pages that open one from a card
// (Home, a project's tabs). The Records page docks the same body in a
// SidePanel instead. Kept at reading width: about 72 characters a line.
export function RecordSheet({ id, project, onClose }: { id: string | null; project?: string; onClose: () => void }) {
  return (
    <Sheet open={!!id} onOpenChange={(o) => !o && onClose()}>
      {/* The Sheet sizes itself with data-[side] variants, so the override must use the same ones. */}
      <SheetContent side="right" className="scroll-thin gap-0 overflow-y-auto rounded-xl p-0 data-[side=right]:w-full data-[side=right]:sm:max-w-[640px]">
        {id ? <RecordSheetBody id={id} project={project} /> : null}
      </SheetContent>
    </Sheet>
  );
}

function RecordSheetBody({ id, project }: { id: string; project?: string }) {
  const { data } = useApi<RecordDetail>("records/" + enc(id) + (project ? "?project=" + enc(project) : ""));
  const r = data?.record.id === id ? data.record : undefined;
  return (
    <>
      <SheetHeader className="flex-row items-start gap-3 border-b px-4 py-3">
        <span className="mt-0.5 flex shrink-0 items-center">
          <KindTile kind={r ? kindFor(r.kind) : "record"} size={28} waiting={r ? awaitsDecision(r) : false} title={r ? kindWord(r.kind) : undefined} />
        </span>
        <div className="min-w-0 flex-1">
          <SheetTitle className="min-w-0 truncate pr-8 text-base font-semibold leading-6 tracking-tight" title={r?.title}>{r ? sentence(r.title) : "Record " + id}</SheetTitle>
          <SheetDescription className="mt-0.5 min-w-0 truncate text-sm leading-5">{r ? kindWord(r.kind) + " in " + r.project : "Opening the record…"}</SheetDescription>
        </div>
      </SheetHeader>
      <div className="px-4 py-4">
        <RecordBody id={id} project={project} />
      </div>
    </>
  );
}

const FOLD = "rounded-lg border bg-card px-3 py-1";

// The record itself, inside the panel: who and when, the decision's answers
// when it waits, a contents list for a long one, the text at reading width,
// then where it came from, its history as a timeline and the file's fields.
export function RecordBody({ id, project }: { id: string; project?: string }) {
  const [rev, setRev] = useState<string>("");
  const params = [project ? "project=" + enc(project) : "", rev ? "rev=" + enc(rev) : ""].filter(Boolean).join("&");
  const { data: fresh, error, reload } = useApi<RecordDetail>("records/" + enc(id) + (params ? "?" + params : ""));
  // Keep the last revision on screen while another one loads, so the panel
  // (and the History fold) stays put instead of dropping to a skeleton.
  const [kept, setKept] = useState<RecordDetail | undefined>(fresh);
  if (fresh && fresh !== kept) setKept(fresh);
  const data = fresh ?? (kept && kept.record.id === id ? kept : undefined);
  const switching = !fresh && !!data;
  const revisions = useMemo(() => [...(data?.revisions || [])].sort((a, b) => b.rev - a.rev), [data]);
  // The panel's title already names the record; drop a first heading that repeats it.
  const body = useMemo(() => {
    const r = data?.record;
    if (!r) return "";
    return (data.body || "").replace(/^\s*#\s+(.+)\n+/, (line, h: string) => (h.trim().toLowerCase() === r.title.trim().toLowerCase() ? "" : line));
  }, [data]);
  const toc = useMemo(() => headings(body), [body]);
  if (error && !data)
    return (
      <div className="grid gap-3">
        <p className="text-sm text-muted-foreground">{"Record " + id + " didn't open."}</p>
        <ErrorNotice error={error} onRetry={() => void reload()} />
      </div>
    );
  if (!data)
    return (
      <div className="grid gap-3">
        <p className="text-sm text-muted-foreground">Opening the record…</p>
        <SkeletonRows n={8} />
      </div>
    );
  const r = data.record;
  const fm = data.frontmatter || {};
  const shownRev = rev ? Number(rev) : r.current_rev;
  const pickRev = (n: number) => setRev(n === r.current_rev ? "" : String(n));
  const areas = r.areas || [];
  const author = r.created_by || (typeof fm.author === "string" ? fm.author : "");
  const evidence = recordEvidence(r.project, data.body || "", fm);
  const tags = r.tags || [];
  const draft = data.draft && r.kind === "evolution" ? { id: r.id, rev: r.current_rev, seeded: false, seeded_from: "", draft: data.draft, draft_job: null } : null;
  return (
    <div className="grid gap-4">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 text-[13px] text-muted-foreground">
        <StatusBadge status={r.status} />
        <ProjectLink slug={r.project} className="font-normal" />
        {r.updated_at || r.date ? <Updated d={r.updated_at || r.date} /> : null}
        {areas.length ? <span>{"About " + areas.join(", ")}</span> : null}
        <Tier r={r} />
        <Delivery r={r} />
        {author ? (
          <span className="inline-flex items-center gap-1.5">
            Written by <Who row={{ author }} compact />
          </span>
        ) : null}
        {revisions.length > 1 ? (
          <Select value={String(shownRev)} onValueChange={(v) => pickRev(Number(v))}>
            <SelectTrigger aria-label="Revision" size="sm" className="h-7 w-fit text-[13px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {revisions.map((x) => (
                <SelectItem key={x.rev} value={String(x.rev)}>
                  {"Revision " + x.rev + " of " + r.current_rev + (x.rev === r.current_rev ? " (latest)" : "")}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        ) : null}
      </div>
      {awaitsDecision(r) ? (
        <div className="surface-you sticky top-0 z-10 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-lg border px-3 py-2">
          <ScaleIcon className="size-4 shrink-0 text-you-ink" aria-hidden />
          <span className="min-w-0 flex-1 basis-48 text-sm font-medium anywhere">This decision waits for your answer.</span>
          <RecordDecisionButtons record={r} size="sm" />
        </div>
      ) : null}
      {draft ? <RecordDraftPanel r={draft} body={data.draft?.body} /> : null}
      {shownRev !== r.current_rev ? (
        <Notice tone="info">
          <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <span>{"You're reading revision " + shownRev + " of " + r.current_rev + ", an older version."}</span>
            <span className="flex flex-wrap gap-2">
              {shownRev > 1 ? (
                <Button size="xs" variant="outline" onClick={() => pickRev(shownRev - 1)}>
                  Older
                </Button>
              ) : null}
              <Button size="xs" variant="outline" onClick={() => pickRev(shownRev + 1)}>
                Newer
              </Button>
              <Button size="xs" variant="outline" onClick={() => setRev("")}>
                Back to the latest
              </Button>
            </span>
          </span>
        </Notice>
      ) : null}
      {wantsContents(body, toc) ? <Contents toc={toc} /> : null}
      <ReadingArticle body={body} toc={toc} switching={switching} />
      {evidence.length ? (
        <section aria-labelledby="record-evidence" className="rounded-lg border bg-card px-4 py-3">
          <h3 id="record-evidence" className="text-sm font-semibold">Where this came from</h3>
          <p className="mt-0.5 text-[13px] text-muted-foreground">The runs and board posts this record mentions. Open one to read what the agents found.</p>
          <ul className="mt-3 grid gap-1.5">
            {evidence.map((e) => {
              const l = evidenceLabel(e);
              return (
                <li key={e.kind + e.id}>
                  <Link href={e.href} title={e.id} className="inline-flex max-w-full items-center gap-2 rounded-md text-sm text-foreground underline-offset-2 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
                    {e.kind === "job" ? <HistoryIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden /> : <MessageSquareTextIcon className="size-4 shrink-0 text-muted-foreground" aria-hidden />}
                    <span className="min-w-0 anywhere">{l.label}</span>
                    {l.when ? <span className="shrink-0 text-[13px] text-muted-foreground">{l.when}</span> : null}
                  </Link>
                </li>
              );
            })}
          </ul>
        </section>
      ) : null}
      <section aria-labelledby="record-history" className="rounded-lg border bg-card px-4 py-3">
        <h3 id="record-history" className="flex items-center gap-2 text-sm font-semibold">
          <HistoryIcon className="size-4 shrink-0 text-muted-foreground" strokeWidth={1.75} aria-hidden />
          History
          <span className="tnum inline-flex h-5 items-center rounded-md bg-muted px-1.5 text-xs font-medium text-muted-foreground">{revisions.length || r.current_rev}</span>
        </h3>
        <HistoryTimeline r={r} revisions={revisions} shownRev={shownRev} pickRev={pickRev} />
      </section>
      <Disclosure variant="row" className={FOLD} contentClassName="mt-0 border-t pt-3 pb-2" summary="Technical details">
        <div className="grid gap-3">
          <Kv
            rows={[
              ["Record", <Mono key="id">{r.id}</Mono>],
              ...(r.path ? ([["File", <Mono key="path">{r.path}</Mono>]] as [React.ReactNode, React.ReactNode][]) : []),
              ["Dated", <When key="date" d={r.date} exact />],
              ...(tags.length ? ([["Tags", tags.join(", ")]] as [React.ReactNode, React.ReactNode][]) : []),
            ]}
          />
          {Object.keys(fm).length ? (
            <>
              <p className="text-[13px] text-muted-foreground">The fields at the top of the record&apos;s file:</p>
              <Kv rows={Object.entries(fm).map(([k, v]) => [k, <Mono key={k} className="anywhere">{Array.isArray(v) ? v.join(", ") : typeof v === "object" && v !== null ? JSON.stringify(v) : String(v)}</Mono>])} />
            </>
          ) : null}
        </div>
      </Disclosure>
    </div>
  );
}

// The text at reading width (~72 characters a line). The h2 and h3
// headings get the ids `headings()` computed, so the contents list can
// scroll the panel to them without moving the page behind it.
function ReadingArticle({ body, toc, switching }: { body: string; toc: Heading[]; switching: boolean }) {
  const ref = useRef<HTMLElement>(null);
  useEffect(() => {
    const root = ref.current;
    if (!root) return;
    const seen = new Map<string, number>();
    const hs = Array.from(root.querySelectorAll<HTMLElement>("h2, h3"));
    for (const h of hs) {
      const base = slugify(h.textContent || "") || "section";
      const n = seen.get(base) || 0;
      seen.set(base, n + 1);
      h.id = n ? base + "-" + n : base;
    }
  }, [body, toc]);
  return (
    <article ref={ref} className={cn("min-w-0 max-w-[68ch] transition-opacity motion-reduce:transition-none", switching && "opacity-60")} aria-busy={switching || undefined}>
      {body.trim() ? <Markdown source={body} /> : <p className="text-sm text-muted-foreground">This record has no text yet.</p>}
    </article>
  );
}

// The contents list of a long record: its h2 and h3 headings; a click
// scrolls the panel's own box, so the page behind it stays put.
function Contents({ toc }: { toc: Heading[] }) {
  const go = (e: React.MouseEvent<HTMLAnchorElement>, h: Heading) => {
    const el = document.getElementById(h.id);
    if (!el) return;
    e.preventDefault();
    const box = el.closest<HTMLElement>("[data-side-panel] .overflow-y-auto, [data-slot=sheet-content] .overflow-y-auto");
    if (box) {
      const b = box.getBoundingClientRect();
      const c = el.getBoundingClientRect();
      box.scrollTo({ top: c.top - b.top + box.scrollTop - 8, behavior: "smooth" });
    } else el.scrollIntoView({ block: "start", behavior: "smooth" });
    el.setAttribute("tabindex", "-1");
    el.focus({ preventScroll: true });
  };
  return (
    <nav aria-label="Contents" className="rounded-lg border bg-muted/40 px-4 py-3">
      <p className="flex items-center gap-2 text-xs font-medium text-muted-foreground">
        <ListIcon className="size-3.5" strokeWidth={1.75} aria-hidden />
        Contents
      </p>
      <ol className="mt-1.5 grid gap-0.5 text-[13px]">
        {toc.map((h) => (
          <li key={h.id} className={cn(h.level === 3 && "pl-3.5")}>
            <a href={"#" + h.id} onClick={(e) => go(e, h)} className="inline-block max-w-full truncate rounded-sm leading-6 text-foreground underline-offset-2 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
              {h.text}
            </a>
          </li>
        ))}
      </ol>
    </nav>
  );
}

// The record's history as a timeline: one dot per revision, newest first,
// with who wrote it, when and their note. The revision on screen is the
// filled dot; another one is a click away.
function HistoryTimeline({ r, revisions, shownRev, pickRev }: { r: RecordRow; revisions: RecordDetail["revisions"]; shownRev: number; pickRev: (n: number) => void }) {
  if (!revisions.length) {
    return (
      <ol className="mt-3 ml-1.5 grid gap-0 border-l text-[13px]">
        {r.updated_at && r.updated_at !== r.created_at ? (
          <li className="relative grid gap-1 pb-3 pl-4">
            <span className="absolute -left-[4.5px] top-1.5 size-2 rounded-full bg-primary ring-2 ring-card" aria-hidden />
            <span className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
              <span className="font-medium text-foreground">Updated</span>
              <When d={r.updated_at} className="text-muted-foreground" />
            </span>
          </li>
        ) : null}
        <li className="relative grid gap-1 pl-4">
          <span className={cn("absolute -left-[4.5px] top-1.5 size-2 rounded-full ring-2 ring-card", r.updated_at && r.updated_at !== r.created_at ? "bg-muted-foreground/40" : "bg-primary")} aria-hidden />
          <span className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
            <span className="font-medium text-foreground">Created</span>
            {r.created_by ? <Who row={{ author: r.created_by }} compact /> : null}
            <When d={r.created_at || r.date} className="text-muted-foreground" />
          </span>
        </li>
      </ol>
    );
  }
  return (
    <ol className="mt-3 ml-1.5 grid gap-0 border-l text-[13px]">
      {revisions.map((x) => {
        const shown = x.rev === shownRev;
        return (
          <li key={x.rev} className="relative grid gap-1 pb-3 pl-4 last:pb-0">
            <span className={cn("absolute -left-[4.5px] top-1.5 size-2 rounded-full ring-2 ring-card", shown ? "bg-primary" : "bg-muted-foreground/40")} aria-hidden />
            <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
              <button
                type="button"
                aria-current={shown ? "true" : undefined}
                aria-label={"Read revision " + x.rev}
                className={cn("rounded-sm underline-offset-2 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring", shown ? "font-semibold text-foreground" : "text-link")}
                onClick={() => pickRev(x.rev)}
              >
                {"Revision " + x.rev + (x.rev === r.current_rev ? " (latest)" : "")}
              </button>
              {x.author ? <Who row={{ author: x.author }} compact /> : null}
              <When d={x.at} className="text-muted-foreground" />
            </div>
            {x.note ? <span className="text-muted-foreground anywhere">{x.note}</span> : null}
          </li>
        );
      })}
    </ol>
  );
}
