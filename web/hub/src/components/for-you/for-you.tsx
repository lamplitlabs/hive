"use client";

import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { ArrowRightIcon, CheckIcon, ChevronUpIcon, ClockIcon, ReplyIcon, XIcon } from "lucide-react";
import { toast } from "sonner";
import { Button, buttonVariants } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/pulse/confirm";
import { AnswerBox } from "@/components/pulse/decide-buttons";
import { EmptyState, ErrorNotice, SkeletonRows } from "@/components/pulse/panel";
import { KindTile } from "@/components/pulse/kind-tile";
import { ListRow, ListRows } from "@/components/pulse/list-row";
import { PageHeader } from "@/components/pulse/page-header";
import { SectionHeading } from "@/components/pulse/section-heading";
import { SidePanel, useOpenItem } from "@/components/pulse/side-panel";
import { Markdown } from "@/components/pulse/markdown";
import { When } from "@/components/pulse/who";
import { useInbox, useInboxSources } from "@/components/pulse/inbox";
import { DecideRecordDialog } from "@/components/records/decide-record";
import { DraftActions } from "@/components/records/draft-panel";
import { AcceptSuggestionDialog, DeclineSuggestionDialog } from "@/components/tasks/suggested-work";
import { useCrumbs } from "@/components/shell/crumbs";
import { enc, projectLabel, writes, type Org, type RecordDecision, type RecordDetail, type RecordRow } from "@/lib/api";
import { plural, sentence } from "@/lib/format";
import { hrefs } from "@/lib/nav";
import { decisionLabels, isOpenQuestion } from "@/lib/owner-questions";
import { useApi } from "@/lib/use-api";
import { agentName } from "@/lib/words";
import { cn } from "@/lib/utils";
import { filterRows, forYouProjects, forYouRows, groupRows, SECTION_NOTE, SECTION_TITLE, waitingWords, type ForYouRow } from "./for-you-state";
import { rowHref } from "./row-href";

// The For you page (spec "Navigation changes"): everything that waits on the
// owner's answer across projects, one list per kind, answered in place. The
// rows come from the same inbox the sidebar count and the bell read, so the
// three never disagree. A decision row opens the record in a side panel with
// the three answers in its header; Y and N on a focused row with a yes/no
// pair answer it (the confirm dialog still asks for the words).

type YesNo = { yes: () => void; no: () => void; yesLabel: string; noLabel: string };

export function ForYou() {
  const sp = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const inbox = useInbox();
  const { org, compute } = useInboxSources();
  const o = org.data;
  const brain = o?.totals.brain || "";
  const canDecide = o?.can_decide !== false;
  const project = sp.get("project") || "";
  // ?answer=<question id> (Home's Answer button) opens that question's answer box.
  const answerKey = sp.get("answer") ? "question:" + sp.get("answer") : "";
  const [openId, setOpenId] = useOpenItem();

  const all = useMemo(() => forYouRows({ ...inbox, brain, machineWhy: inbox.downWhy[0] }), [inbox, brain]);
  const chips = useMemo(() => forYouProjects(all), [all]);
  const rows = useMemo(() => filterRows(all, project || null), [all, project]);
  const groups = useMemo(() => groupRows(rows), [rows]);

  useCrumbs(project ? [{ label: "For you", href: hrefs.forYou() }, { label: nameOf(o, project) }] : [{ label: "For you" }]);

  const setProject = (slug: string) => {
    const q = new URLSearchParams(sp.toString());
    if (slug) q.set("project", slug);
    else q.delete("project");
    const s = q.toString();
    router.replace(pathname + (s ? "?" + s : ""), { scroll: false });
  };

  const open = openId ? all.find((r) => r.key === openId) || null : null;
  const openDecision = open && open.section === "decisions" ? inbox.decisions.find((r) => "decision:" + r.id === open.key) || null : null;
  const close = useCallback(() => setOpenId(null), [setOpenId]);

  const loading = !inbox.ready && all.length === 0;
  const subtitle = inbox.ready ? waitingWords(inbox.total) + (inbox.agentsDeciding ? " · " + plural(inbox.agentsDeciding, "question") + " the agents settle themselves" : "") : "Everything that waits on your answer, across projects";

  return (
    <div className="flex items-start gap-6">
      <main className="min-w-0 flex-1">
        <PageHeader title="For you" subtitle={subtitle} />
        {chips.length > 1 ? (
          <div className="mb-5 flex flex-wrap items-center gap-1.5" role="group" aria-label="Filter by project">
            <Chip active={!project} onClick={() => setProject("")}>
              All <span className="tnum text-muted-foreground">{all.length}</span>
            </Chip>
            {chips.map((c) => (
              <Chip key={c.slug} active={project === c.slug} onClick={() => setProject(project === c.slug ? "" : c.slug)}>
                {nameOf(o, c.slug)} <span className="tnum text-muted-foreground">{c.n}</span>
              </Chip>
            ))}
          </div>
        ) : null}
        {org.error && !o ? <ErrorNotice error={org.error} onRetry={() => void org.reload()} /> : null}
        {loading ? (
          <SkeletonRows n={6} />
        ) : groups.length === 0 ? (
          <EmptyState title={project ? "Nothing waits on you in " + nameOf(o, project) : "Nothing waits on you"} hint={inbox.downWhy.length ? "Nothing that PULSE can see, but " + inbox.downWhy.join(" and ") + "." : inbox.checking.length ? "Checking " + inbox.checking.join(" and ") + "…" : "New decisions, proposals and questions show up here the moment an agent needs your answer."} action={project ? <Button variant="outline" size="sm" onClick={() => setProject("")}>Show every project</Button> : undefined} />
        ) : (
          <div className="grid min-w-0 gap-8">
            {groups.map((g) => (
              // min-w-0: a grid track is auto-sized to its content, so a
              // row's unbreakable actions would widen the page on a phone.
              <section key={g.section} aria-labelledby={"for-you-" + g.section} className="min-w-0">
                <SectionHeading id={"for-you-" + g.section} count={g.rows.length} note={SECTION_NOTE[g.section]} className="mb-3">
                  {SECTION_TITLE[g.section]}
                </SectionHeading>
                <ListRows ariaLabel={SECTION_TITLE[g.section]}>
                  {g.rows.map((r) => (
                    <Row key={r.key} r={r} inbox={inbox} brain={brain} canDecide={canDecide} caps={compute.data?.caps} startOpen={r.key === answerKey} selected={openId === r.key} onOpen={r.section === "decisions" ? () => setOpenId(openId === r.key ? null : r.key) : undefined} projectName={nameOf(o, r.project)} router={router} />
                  ))}
                </ListRows>
              </section>
            ))}
          </div>
        )}
      </main>
      <SidePanel open={!!openDecision} onClose={close} width="reading" title={openDecision ? sentence(openDecision.title) : "Decision"} subtitle={openDecision ? openDecision.id + " · " + nameOf(o, openDecision.project) : undefined} lead={<KindTile kind="decision" size={28} waiting />} actions={openDecision ? <Link href={hrefs.record(openDecision.id, openDecision.project)} className="text-xs font-medium text-link hover:underline">Open page</Link> : undefined} footer={openDecision ? <RecordDecide record={openDecision} onDone={close} /> : undefined}>
        {openDecision ? <RecordReading key={openDecision.id} record={openDecision} /> : null}
      </SidePanel>
    </div>
  );
}

function nameOf(o: Org | undefined, slug: string): string {
  if (!slug) return "";
  const p = [...(o?.projects ?? []), ...(o?.commons ? [o.commons] : [])].find((x) => x.slug === slug);
  return p ? projectLabel(p) : slug;
}

function Chip({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button type="button" aria-pressed={active} onClick={onClick} className={cn("inline-flex h-7 items-center gap-1.5 rounded-md border px-2.5 text-xs font-medium transition-colors", active ? "border-foreground/20 bg-accent text-foreground" : "border-border bg-card text-muted-foreground hover:bg-muted hover:text-foreground")}>
      {children}
    </button>
  );
}

// ----- rows -----------------------------------------------------------------

function Row({ r, inbox, brain, canDecide, caps, startOpen, selected, onOpen, projectName, router }: { r: ForYouRow; inbox: ReturnType<typeof useInbox>; brain: string; canDecide: boolean; caps?: { vms_now: number; max_vms: number; hours_today: number; max_hours_per_day: number }; startOpen?: boolean; selected: boolean; onOpen?: () => void; projectName: string; router: ReturnType<typeof useRouter> }) {
  const [yn, setYn] = useState<YesNo | null>(null);
  // A question or a proposal opens in place, under its row: its whole text
  // and, for a question that wants words, the answer box. Closed, the row is
  // one line like the rest.
  const [expanded, setExpanded] = useState(!!startOpen);
  const toggle = useCallback(() => setExpanded((v) => !v), []);
  // A link that opens this row (Home's Answer) brings it into view once.
  const item = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (startOpen) item.current?.scrollIntoView({ block: "center" });
  }, [startOpen]);
  const [kind, ...rest] = r.key.split(":");
  const id = rest.join(":");
  let actions: React.ReactNode = null;
  let details: React.ReactNode = null;
  if (kind === "question") {
    const q = inbox.questions.find((x) => x.id === id);
    if (q) {
      const shape = questionShape(q);
      actions = <QuestionActions q={q} shape={shape} brain={brain} canDecide={canDecide} register={setYn} answering={expanded} onAnswer={toggle} />;
      if (expanded) details = <QuestionDetails q={q} slug={q.slug || brain} asksWords={shape.asksWords} canDecide={canDecide} />;
    }
  } else if (kind === "proposal") {
    const p = inbox.proposals.find((x) => x.id === id);
    if (p) {
      actions = <ProposalActions p={p} canDecide={canDecide} register={setYn} />;
      if (expanded) details = <ItemText text={p.proposal_text} post={hrefs.post(p.slug || brain, p.id)} empty="The proposal says no more than its title." />;
    }
  } else if (kind === "decision") {
    const rec = inbox.decisions.find((x) => x.id === id);
    if (rec) actions = <RecordDecide record={rec} size="xs" register={setYn} />;
  } else if (kind === "compute") {
    const c = inbox.compute.find((x) => x.id === id);
    if (c) actions = <ComputeActions r={c} caps={caps} register={setYn} />;
  } else if (kind === "work") {
    const s = inbox.work.find((x) => x.id === id);
    if (s) actions = <WorkActions s={s} register={setYn} onOpenTask={(tid) => router.push(hrefs.task(tid))} />;
  } else if (kind === "draft") {
    const d = inbox.drafts.find((x) => x.slug + ":" + x.r.id === id);
    if (d) actions = <DraftActions r={d.r} size="xs" />;
  } else if (kind === "step" || kind === "machine" || kind === "provider") {
    // The row itself is the link to the fix; this is only its visible cue,
    // drawn as a button (an <a> or a <button> inside the row's <a> would be
    // invalid HTML).
    actions = (
      <span className={buttonVariants({ variant: kind === "step" ? "outline" : "default", size: "xs" })}>
        {r.fix || "Open"} <ArrowRightIcon aria-hidden />
      </span>
    );
  }
  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (!yn || e.metaKey || e.ctrlKey || e.altKey) return;
    const t = e.target as HTMLElement;
    if (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable) return;
    if (e.key === "y" || e.key === "Y") {
      e.preventDefault();
      yn.yes();
    } else if (e.key === "n" || e.key === "N") {
      e.preventDefault();
      yn.no();
    }
  };
  const opensInPlace = kind === "question" || kind === "proposal";
  const href = onOpen || opensInPlace || kind === "compute" || kind === "work" ? undefined : rowHref(r);
  return (
    // One list item: the row and, while it is open, its details under it.
    <div ref={item} onKeyDown={onKeyDown}>
      <ListRow
        lead={<KindTile kind={r.kind} size={28} waiting={r.urgent} />}
        title={r.title}
        meta={
          <>
            {/* Plain text, not a link: a row with an href is itself an <a>. */}
            {projectName ? <span className="truncate">{projectName}</span> : null}
            {r.meta ? <span className="truncate">{r.meta}</span> : null}
          </>
        }
        time={r.at ? <When d={r.at} /> : undefined}
        // Answering is this page's job: every row's buttons stay in sight.
        actions={actions}
        actionsVisible
        selected={selected || expanded}
        expanded={opensInPlace ? expanded : undefined}
        href={href}
        onOpen={onOpen ?? (opensInPlace ? toggle : undefined)}
        ariaLabel={r.title + (projectName ? " · " + projectName : "")}
      />
      {details ? (
        <div className="border-t bg-muted/30 px-3 py-3 sm:pl-[3.25rem]" data-row-details>
          {details}
        </div>
      ) : null}
    </div>
  );
}

// A question's whole text and, when it wants an answer in words, the box to
// write it in (focused as it opens). The post on the board is one link away.
function QuestionDetails({ q, slug, asksWords, canDecide }: { q: ReturnType<typeof useInbox>["questions"][number]; slug: string; asksWords: boolean; canDecide: boolean }) {
  return (
    <div className="grid max-w-[68ch] gap-3">
      <ItemText text={q.body} post={hrefs.post(slug, q.id)} empty={asksWords ? "" : "The question says no more than its title."} />
      {asksWords ? <AnswerBox id={q.id} summary={q.summary} canDecide={canDecide} autoFocus /> : null}
    </div>
  );
}

// An item's own words as Markdown, and the link to its post.
function ItemText({ text, post, empty }: { text?: string | null; post: string; empty: string }) {
  const body = (text || "").trim();
  return (
    <div className="grid min-w-0 gap-1.5">
      {body ? (
        <article className="min-w-0">
          {/* .prose-pulse reads long text at 16 px; under a row it keeps the list's 14 (hence the !). */}
          <Markdown source={body} className="text-sm! leading-relaxed!" />
        </article>
      ) : empty ? (
        <p className="text-sm text-muted-foreground">{empty}</p>
      ) : null}
      <Link href={post} className="inline-flex w-fit items-center gap-1 text-xs font-medium text-link hover:underline">
        Read it on the board <ArrowRightIcon className="size-3.5" aria-hidden />
      </Link>
    </div>
  );
}

// Each actions piece reports its yes/no handlers to the row (register) so the
// Y / N keys open the same confirm dialogs the buttons do.

function Pair({ yes, no, yesLabel, noLabel, destructiveNo = true }: YesNo & { destructiveNo?: boolean }) {
  return (
    <span className="inline-flex items-center gap-1.5">
      <Button size="xs" variant="outline" className={cn(destructiveNo && "text-bad hover:text-bad")} onClick={no} aria-label={noLabel} title={noLabel + " (N)"}>
        <XIcon /> {noLabel}
      </Button>
      <Button size="xs" onClick={yes} aria-label={yesLabel} title={yesLabel + " (Y)"}>
        <CheckIcon /> {yesLabel}
      </Button>
    </span>
  );
}

function useRegister(register: ((p: YesNo | null) => void) | undefined, pair: YesNo | null) {
  const { yes, no, yesLabel, noLabel } = pair || ({} as Partial<YesNo>);
  useEffect(() => {
    if (!register) return;
    register(yes && no ? { yes, no, yesLabel: yesLabel || "Yes", noLabel: noLabel || "No" } : null);
    return () => register(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- handlers are stable per item id
  }, [register, !!yes, yesLabel, noLabel]);
}

type QuestionShape = { stop: boolean; asksWords: boolean; labels: { yes: string; no: string } };

// How a question is answered: a stop question carries on or stays paused; a
// "which…?" question wants words; the rest are a yes or a no.
function questionShape(q: ReturnType<typeof useInbox>["questions"][number]): QuestionShape {
  const body = (q.body || "").trim();
  const stop = q.stop === true || /Evolution stopped; owner review needed|evolution constraint violation/i.test((q.summary || "") + "\n" + body);
  const labels = decisionLabels(body, stop ? { yes: "Carry on", no: "Keep paused" } : { yes: "Yes", no: "No" });
  return { stop, labels, asksWords: !stop && isOpenQuestion(q.summary, body) };
}

function QuestionActions({ q, shape, brain, canDecide, register, answering, onAnswer }: { q: ReturnType<typeof useInbox>["questions"][number]; shape: QuestionShape; brain: string; canDecide: boolean; register: (p: YesNo | null) => void; answering: boolean; onAnswer: () => void }) {
  const [open, setOpen] = useState<"yes" | "no" | null>(null);
  const { stop, labels, asksWords } = shape;
  const yes = useCallback(() => setOpen("yes"), []);
  const no = useCallback(() => setOpen("no"), []);
  useRegister(register, canDecide && !asksWords ? { yes, no, yesLabel: labels.yes, noLabel: labels.no } : null);
  const slug = q.slug || brain;
  if (!canDecide) return <span className="text-xs text-muted-foreground">Answer on the machine that runs PULSE</span>;
  // The answer box opens under the row (Row's details), not in it.
  if (asksWords)
    return (
      <Button size="xs" variant={answering ? "outline" : "default"} onClick={onAnswer} aria-expanded={answering} aria-label={(answering ? "Close the answer to: " : "Answer: ") + q.summary}>
        {answering ? <ChevronUpIcon /> : <ReplyIcon />} Answer
      </Button>
    );
  const isYes = open === "yes";
  return (
    <>
      <Pair yes={yes} no={no} yesLabel={labels.yes} noLabel={labels.no} destructiveNo={!stop} />
      <ConfirmDialog
        open={open !== null}
        onOpenChange={(v) => !v && setOpen(null)}
        title={isYes ? labels.yes + "?" : labels.no + "?"}
        description={
          <>
            <span className="block font-medium text-foreground">{sentence(q.summary)}</span>
            <span className="mt-2 block">{stop ? (isYes ? "Self-improvement carries on past this stop. The change that stopped it is not tried again as written." : "Self-improvement stays paused until you say yes.") : "Your answer goes on the board for " + slug + ", so the agents know what to do next."}</span>
          </>
        }
        action={isYes ? labels.yes : labels.no}
        words={{ label: "A note for the agents (optional)", placeholder: stop ? (isYes ? "Reviewed, carry on." : "Keep it paused because…") : isYes ? "Yes, because…" : "No, because…" }}
        success={stop ? (isYes ? "Answered. Self-improvement carries on." : "Answered. Self-improvement stays paused.") : isYes ? "You said yes. The agents will act on it." : "You said no. The agents won't go ahead."}
        onConfirm={(words) => writes.decide(q.id, isYes ? "yes" : "no", words)}
      />
    </>
  );
}

function ProposalActions({ p, canDecide, register }: { p: ReturnType<typeof useInbox>["proposals"][number]; canDecide: boolean; register: (p: YesNo | null) => void }) {
  const [open, setOpen] = useState<"yes" | "no" | null>(null);
  const yes = useCallback(() => setOpen("yes"), []);
  const no = useCallback(() => setOpen("no"), []);
  const can = canDecide && p.can_decide !== false;
  useRegister(register, can ? { yes, no, yesLabel: "Accept", noLabel: "Decline" } : null);
  if (!can) return <span className="text-xs text-muted-foreground">Answer on the machine that runs PULSE</span>;
  const isYes = open === "yes";
  return (
    <>
      <Pair yes={yes} no={no} yesLabel="Accept" noLabel="Decline" />
      <ConfirmDialog
        open={open !== null}
        onOpenChange={(v) => !v && setOpen(null)}
        title={isYes ? "Accept this proposal?" : "Decline this proposal?"}
        description={
          <>
            <span className="block font-medium text-foreground">{sentence(p.summary)}</span>
            <span className="mt-2 block">{isYes ? "The agents go ahead as proposed. If it asks for work, a job is queued." : "The agents won't pursue it as written."}</span>
          </>
        }
        action={isYes ? "Yes, accept" : "No, decline"}
        destructive={!isYes}
        words={{ label: "A note for the agents (optional)", placeholder: isYes ? "Accepted: keep the scope as written." : "Not now, because…" }}
        success={isYes ? "Accepted. The agents will go ahead." : "Declined. The agents won't pursue it."}
        onConfirm={(words) => writes.decide(p.id, isYes ? "yes" : "no", words)}
      />
    </>
  );
}

const COMPUTE_CHANGES = (p: string) => p.startsWith("compute") || p === "org";

function ComputeActions({ r, caps, register }: { r: ReturnType<typeof useInbox>["compute"][number]; caps?: { vms_now: number; max_vms: number; hours_today: number; max_hours_per_day: number }; register: (p: YesNo | null) => void }) {
  const [which, setWhich] = useState<"approve" | "deny" | null>(null);
  const yes = useCallback(() => setWhich("approve"), []);
  const no = useCallback(() => setWhich("deny"), []);
  useRegister(register, { yes, no, yesLabel: "Approve", noLabel: "Deny" });
  const approve = which === "approve";
  return (
    <>
      <Pair yes={yes} no={no} yesLabel="Approve" noLabel="Deny" />
      <ConfirmDialog
        open={which !== null}
        onOpenChange={(v) => !v && setWhich(null)}
        title={approve ? "Approve the extra machine?" : "Deny the extra machine?"}
        description={
          <span>
            {agentName(r.requested_by)} asks for one extra machine ({r.size}) for {plural(r.hours, "hour")}
            {r.reason ? <>: &ldquo;{r.reason.trim()}&rdquo;</> : null}. {caps ? caps.vms_now + " of " + caps.max_vms + " extra machines are in use, " + caps.hours_today + " of " + plural(caps.max_hours_per_day, "hour") + " used today. " : ""}
            {approve ? "PULSE starts it within the limits you set, and removes it when the time is up." : "The agent sees your answer the next time it checks."}
          </span>
        }
        action={approve ? "Approve" : "Deny"}
        destructive={!approve}
        words={{ label: approve ? "A note for the agent (optional)" : "Tell the agent why", placeholder: approve ? "Approved: needed for the load test." : "Not now. Use the machines you have.", required: !approve }}
        success={approve ? "Approved. PULSE will start the machine." : "Denied. The agent will see your note."}
        onConfirm={(w) => (approve ? writes.approveCompute(r.id, w) : writes.denyCompute(r.id, w))}
        invalidates={COMPUTE_CHANGES}
      />
    </>
  );
}

function WorkActions({ s, register, onOpenTask }: { s: ReturnType<typeof useInbox>["work"][number]; register: (p: YesNo | null) => void; onOpenTask: (id: string) => void }) {
  const [accept, setAccept] = useState(false);
  const [decline, setDecline] = useState(false);
  const yes = useCallback(() => setAccept(true), []);
  const no = useCallback(() => setDecline(true), []);
  useRegister(register, { yes, no, yesLabel: "Accept", noLabel: "Decline" });
  return (
    <>
      <Pair yes={yes} no={no} yesLabel="Accept" noLabel="Decline" />
      <AcceptSuggestionDialog s={s} open={accept} onOpenChange={setAccept} onAccepted={(t) => onOpenTask(t.id)} />
      <DeclineSuggestionDialog s={s} open={decline} onOpenChange={setDecline} />
    </>
  );
}

// Accept / Defer / Decline for a Proposed decision record, in a row (xs) or
// the panel's footer. Y accepts, N declines; Defer stays a click.
function RecordDecide({ record, size = "sm", onDone, register }: { record: RecordRow; size?: "xs" | "sm"; onDone?: () => void; register?: (p: YesNo | null) => void }) {
  const [action, setAction] = useState<RecordDecision>("accept");
  const [open, setOpen] = useState(false);
  const start = useCallback((a: RecordDecision) => {
    setAction(a);
    setOpen(true);
  }, []);
  const yes = useCallback(() => start("accept"), [start]);
  const no = useCallback(() => start("Rejected"), [start]);
  useRegister(register, { yes, no, yesLabel: "Accept", noLabel: "Decline" });
  return (
    <span className={cn("inline-flex flex-wrap items-center gap-1.5", size === "sm" && "w-full justify-end gap-2")}>
      <Button size={size} variant="outline" className="text-bad hover:text-bad" onClick={no} aria-label={"Decline " + record.id} title="Decline (N)">
        <XIcon /> Decline
      </Button>
      <Button size={size} variant="outline" onClick={() => start("Deferred")} aria-label={"Defer " + record.id}>
        <ClockIcon /> Defer
      </Button>
      <Button size={size} onClick={yes} aria-label={"Accept " + record.id} title="Accept (Y)">
        <CheckIcon /> Accept
      </Button>
      <DecideRecordDialog record={record} action={action} open={open} onOpenChange={setOpen} onDone={() => { onDone?.(); toast.dismiss(); }} />
    </span>
  );
}

// The record's text inside the reading panel: the body as Markdown, without
// the first heading when it only repeats the title the panel already shows.
function RecordReading({ record }: { record: RecordRow }) {
  const { data, error, reload } = useApi<RecordDetail>("records/" + enc(record.id) + "?project=" + enc(record.project));
  if (error && !data) return <ErrorNotice error={error} onRetry={() => void reload()} />;
  if (!data) return <SkeletonRows n={8} />;
  const r = data.record;
  const body = (data.body || "").replace(/^\s*#\s+(.+)\n+/, (line, h: string) => (h.trim().toLowerCase() === r.title.trim().toLowerCase() ? "" : line));
  const areas = r.areas || [];
  return (
    <div className="grid gap-4">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[13px] text-muted-foreground">
        <span>{"Proposed " + (r.created_by ? "by " + agentName(r.created_by, r.project) + " " : "")}</span>
        <When d={r.updated_at || r.date} />
        {areas.length ? <span>{"About " + areas.join(", ")}</span> : null}
      </div>
      <article className="prose-sm min-w-0">{body.trim() ? <Markdown source={body} /> : <p className="text-sm text-muted-foreground">This record has no text yet.</p>}</article>
    </div>
  );
}
