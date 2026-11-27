"use client";

import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { ArrowRightIcon, CheckIcon, ClockIcon, ShieldAlertIcon, XIcon } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/hive/confirm";
import { AnswerPicker } from "@/components/hive/decide-buttons";
import { ErrorNotice, SkeletonRows } from "@/components/hive/panel";
import { KindTile } from "@/components/hive/kind-tile";
import { ListItem, ListRow, ListRows } from "@/components/hive/list-row";
import { SectionHeading } from "@/components/hive/section-heading";
import { SidePanel, useOpenItem } from "@/components/hive/side-panel";
import { Markdown } from "@/components/hive/markdown";
import { When } from "@/components/hive/who";
import { useInbox, useInboxSources } from "@/components/hive/inbox";
import { DecideRecordDialog } from "@/components/records/decide-record";
import { AcceptSuggestionDialog, DeclineSuggestionDialog } from "@/components/tasks/suggested-work";
import { enc, projectLabel, writes, type Org, type RecordDecision, type RecordDetail, type RecordRow } from "@/lib/api";
import { plural, sentence } from "@/lib/format";
import { hrefs } from "@/lib/nav";
import { questionAnswers, type QuestionAnswers } from "@/lib/owner-questions";
import { useApi } from "@/lib/use-api";
import { agentName } from "@/lib/words";
import { cn } from "@/lib/utils";
import { forYouRows, HARD_STOP_LABEL, hardStopRows, hardStopWords, type ForYouRow } from "./asks-state";
import { rowHref } from "./row-href";
import { GoalProposalCard, GoalProposalRowActions } from "@/components/project/goal-proposal";
import { isGoalProposal } from "@/components/project/goal-proposal-state";

// The hard-stop strip on Sessions (TASK-129): For you is gone, and only the
// asks the hard stops keep for the owner - money, what can't be undone,
// credentials, publication, the project goal - still wait on a person.
// Everything else is the project lead's to decide. One compact list over the session landing; a
// question shows one button per answer, the Recommended one first (nothing
// is typed); absent when nothing waits. A
// decision row opens the record in a side panel with its three answers; Y
// and N on a focused row with a yes/no pair answer it (the confirm dialog
// still asks for the words).

type YesNo = { yes: () => void; no: () => void; yesLabel: string; noLabel: string };

// The asks at a hard stop, from the same inbox the bell reads: the strip
// lists them and the sidebar's Sessions item counts them.
export function useHardStops() {
  const inbox = useInbox();
  const { org } = useInboxSources();
  const brain = org.data?.totals.brain || "";
  const rows = useMemo(() => hardStopRows(forYouRows({ ...inbox, brain, machineWhy: inbox.downWhy[0] })), [inbox, brain]);
  return { inbox, rows, brain, ready: inbox.ready };
}

export function HardStops() {
  const sp = useSearchParams();
  const router = useRouter();
  const { inbox, rows, brain } = useHardStops();
  const { org, compute } = useInboxSources();
  const o = org.data;
  const canDecide = o?.can_decide !== false;
  // ?answer=<question id> (a notification's link) opens that question under its row.
  const answerKey = sp.get("answer") ? "question:" + sp.get("answer") : "";
  const [openId, setOpenId] = useOpenItem();

  const open = openId ? rows.find((r) => r.key === openId) || null : null;
  const openDecision = open && open.section === "decisions" ? inbox.decisions.find((r) => "decision:" + r.id === open.key) || null : null;
  const close = useCallback(() => setOpenId(null), [setOpenId]);

  if (!inbox.ready || rows.length === 0) return null;
  return (
    <section id="hard-stops" aria-labelledby="hard-stops-title" className="min-w-0 rounded-lg border border-you/40 bg-you/5 p-3 sm:p-4">
      <SectionHeading id="hard-stops-title" icon={ShieldAlertIcon} count={rows.length} countTone="you" note="Money, what can't be undone, credentials, publication and the project goal wait for you; the leads decide the rest." className="mb-3">
        {hardStopWords(rows.length)}
      </SectionHeading>
      <ListRows ariaLabel="Decisions only you can make">
        {rows.map((r) => (
          <Row key={r.key} r={r} stop={HARD_STOP_LABEL[r.stopKind]} inbox={inbox} brain={brain} canDecide={canDecide} caps={compute.data?.caps} startOpen={r.key === answerKey} selected={openId === r.key} onOpen={r.section === "decisions" ? () => setOpenId(openId === r.key ? null : r.key) : undefined} projectName={r.projectWord || nameOf(o, r.project)} router={router} />
        ))}
      </ListRows>
      <SidePanel open={!!openDecision} onClose={close} width="reading" title={openDecision ? sentence(openDecision.title) : "Decision"} subtitle={openDecision ? openDecision.id + " · " + nameOf(o, openDecision.project) : undefined} lead={<KindTile kind="decision" size={28} waiting />} actions={openDecision ? <Link href={hrefs.record(openDecision.id, openDecision.project)} className="text-xs font-medium text-link hover:underline">Open page</Link> : undefined} footer={openDecision ? <RecordDecide record={openDecision} onDone={close} /> : undefined}>
        {openDecision ? <RecordReading key={openDecision.id} record={openDecision} /> : null}
      </SidePanel>
    </section>
  );
}

function nameOf(o: Org | undefined, slug: string): string {
  if (!slug) return "";
  const p = [...(o?.projects ?? []), ...(o?.commons ? [o.commons] : [])].find((x) => x.slug === slug);
  return p ? projectLabel(p) : slug;
}

// ----- rows -----------------------------------------------------------------

function Row({ r, stop, inbox, brain, canDecide, caps, startOpen, selected, onOpen, projectName, router }: { r: ForYouRow; stop: string; inbox: ReturnType<typeof useInbox>; brain: string; canDecide: boolean; caps?: { vms_now: number; max_vms: number; hours_today: number; max_hours_per_day: number }; startOpen?: boolean; selected: boolean; onOpen?: () => void; projectName: string; router: ReturnType<typeof useRouter> }) {
  const [yn, setYn] = useState<YesNo | null>(null);
  // A question or a proposal opens in place, under its row: its whole text
  // and, for a question, why its Recommended answer is the one; a proposed
  // goal opens from the start. Closed, the row is one line like the rest.
  const [expanded, setExpanded] = useState(!!startOpen || !!r.card);
  const toggle = useCallback(() => setExpanded((v) => !v), []);
  // A link that opens this row (a notification's ?answer=) brings it into view once.
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
      const answers = questionShape(q);
      actions = <AnswerPicker id={q.id} summary={q.summary} answers={answers} stop={isStopQuestion(q)} canDecide={canDecide} size="xs" />;
      if (expanded) details = <QuestionDetails q={q} slug={q.slug || brain} answers={answers} />;
    }
  } else if (kind === "proposal") {
    const p = inbox.proposals.find((x) => x.id === id);
    if (p && isGoalProposal(p)) {
      // A proposed goal (a goal change is the owner's alone): its answers on
      // the row, the recommended one first; its whole text, the Why and why
      // that answer is recommended in the card under it.
      actions = <GoalProposalRowActions slug={p.slug || brain} g={p.goal} canDecide={canDecide && p.can_decide !== false} />;
      if (expanded) details = <GoalProposalCard slug={p.slug || brain} g={p.goal} canDecide={canDecide && p.can_decide !== false} framed={false} withActions={false} />;
    } else if (p) {
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
    <ListItem ref={item} onKeyDown={onKeyDown}>
      <ListRow
        lead={<KindTile kind={r.kind} size={28} waiting={r.urgent} />}
        title={r.title}
        meta={
          <>
            {/* Plain text, not a link: the row's title already leads there. */}
            <span className="shrink-0 font-medium text-you-ink">{stop}</span>
            {projectName ? <span className="truncate">{projectName}</span> : null}
            {r.meta ? <span className="truncate">{r.meta}</span> : null}
          </>
        }
        time={r.at ? <When d={r.at} /> : undefined}
        // Answering is the strip's job: every row's buttons stay in sight.
        actions={actions}
        actionsVisible
        selected={selected || expanded}
        expanded={opensInPlace ? expanded : undefined}
        href={href}
        onOpen={onOpen ?? (opensInPlace ? toggle : undefined)}
        ariaLabel={stop + ": " + r.title + (projectName ? " · " + projectName : "")}
      />
      {details ? (
        <div className="border-t bg-muted/30 px-3 py-3 sm:pl-[3.25rem]" data-row-details>
          {details}
        </div>
      ) : null}
    </ListItem>
  );
}

// A question's whole text and why its Recommended answer is the one. The
// post on the board is one link away.
function QuestionDetails({ q, slug, answers }: { q: ReturnType<typeof useInbox>["questions"][number]; slug: string; answers: QuestionAnswers }) {
  return (
    <div className="grid max-w-[68ch] gap-3">
      <ItemText text={q.body} post={hrefs.post(slug, q.id)} empty="The question says no more than its title." />
      {answers.kind === "pick" && answers.why ? (
        <p className="text-sm text-muted-foreground">
          Recommended: <span className="font-medium text-foreground">{answers.options.find((o) => o.recommended)?.label}</span> - {answers.why}
        </p>
      ) : null}
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
          {/* .prose-hive reads long text at 16 px; under a row it keeps the list's 14 (hence the !). */}
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

// How a question is answered (TASK-129): one button per answer, the
// Recommended one first, nothing typed. A stop of the self-improvement loop
// carries on or stays paused; an ask that names no answers stays with its
// lead until the lead writes them out.
function questionShape(q: ReturnType<typeof useInbox>["questions"][number]): QuestionAnswers {
  return questionAnswers(q, isStopQuestion(q));
}

function isStopQuestion(q: ReturnType<typeof useInbox>["questions"][number]): boolean {
  return q.stop === true || /Evolution stopped; owner review needed|evolution constraint violation/i.test((q.summary || "") + "\n" + (q.body || ""));
}

function ProposalActions({ p, canDecide, register }: { p: ReturnType<typeof useInbox>["proposals"][number]; canDecide: boolean; register: (p: YesNo | null) => void }) {
  const [open, setOpen] = useState<"yes" | "no" | null>(null);
  const yes = useCallback(() => setOpen("yes"), []);
  const no = useCallback(() => setOpen("no"), []);
  const can = canDecide && p.can_decide !== false;
  useRegister(register, can ? { yes, no, yesLabel: "Accept", noLabel: "Decline" } : null);
  if (!can) return <span className="text-xs text-muted-foreground">Answer on the machine that runs Hive</span>;
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
      {/* One dialog for both answers: the key mounts it anew for each, so the words Deny opens with are never sent with an Approve. */}
      <ConfirmDialog
        key={which ?? "closed"}
        open={which !== null}
        onOpenChange={(v) => !v && setWhich(null)}
        title={approve ? "Approve the extra machine?" : "Deny the extra machine?"}
        description={
          <span>
            {agentName(r.requested_by)} asks for one extra machine ({r.size}) for {plural(r.hours, "hour")}
            {r.reason ? <>: &ldquo;{r.reason.trim()}&rdquo;</> : null}. {caps ? caps.vms_now + " of " + caps.max_vms + " extra machines are in use, " + caps.hours_today + " of " + plural(caps.max_hours_per_day, "hour") + " used today. " : ""}
            {approve ? "Hive starts it within the limits you set, and removes it when the time is up." : "The agent sees your answer the next time it checks."}
          </span>
        }
        action={approve ? "Approve" : "Deny"}
        destructive={!approve}
        words={{ label: approve ? "A note for the agent (optional)" : "Tell the agent why", placeholder: approve ? "Approved: needed for the load test." : "Not now. Use the machines you have.", required: !approve, initial: approve ? "" : "Not now. Use the machines you have." }}
        success={approve ? "Approved. Hive will start the machine." : "Denied. The agent will see your note."}
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
