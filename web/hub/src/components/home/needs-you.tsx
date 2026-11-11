"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { CheckIcon, GitMergeIcon, MessageSquarePlusIcon, PlusIcon, ReplyIcon, ScaleIcon, XIcon } from "lucide-react";
import { Button, buttonVariants } from "@/components/ui/button";
import { ProjectAvatar } from "@/components/hive/avatar";
import { ConfirmDialog } from "@/components/hive/confirm";
import { DecideButtons } from "@/components/hive/decide-buttons";
import { CopyButton } from "@/components/hive/evolve";
import type { Inbox } from "@/components/hive/inbox";
import { KindTile, type Kind } from "@/components/hive/kind-tile";
import { ListRow, ListRows } from "@/components/hive/list-row";
import { EmptyState } from "@/components/hive/panel";
import { SectionHeading } from "@/components/hive/section-heading";
import { When } from "@/components/hive/who";
import { ComputeDecide } from "@/components/compute/compute";
import { awaitsDecision, DecideRecordDialog } from "@/components/records/decide-record";
import { DraftActions } from "@/components/records/draft-panel";
import { AcceptSuggestionDialog, DeclineSuggestionDialog } from "@/components/tasks/suggested-work";
import { NewTaskDialog } from "@/components/tasks/task-dialogs";
import { writes, type Org, type Proposal, type RecordDecision, type RecordRow, type WorkSuggestion } from "@/lib/api";
import { sentence } from "@/lib/format";
import { hrefs } from "@/lib/nav";
import { decisionLabels, isOpenQuestion } from "@/lib/owner-questions";
import { agentName } from "@/lib/words";
import { itemProject, needsYouWord, seeAllLabel, topFive, type NeedsYouItem } from "./home-state";

// "Needs you" (spec "Home" 3): the top five waiting items as rows, each with
// its kind tile, title, project avatar and age, and the item's own yes / no
// on the right, always in sight. The buttons are the ones the cards use
// elsewhere (the same confirms, the same writes); the row itself opens the
// item's page.
// "See all 10" leads to For you. With nothing waiting: "Nothing needs you"
// and three ways to start something.
export function NeedsYou({ inbox, org }: { inbox: Inbox; org: Org }) {
  const items = topFive(inbox);
  const brain = org.totals.brain || "";
  const names = new Map(org.projects.map((p) => [p.slug, p.name || p.slug]));
  return (
    <section aria-labelledby="home-needs-you" className="min-w-0">
      <SectionHeading id="home-needs-you" icon={ScaleIcon} count={inbox.ready ? inbox.total : undefined} countTone={inbox.total ? "you" : "neutral"} action={<Link href={hrefs.forYou()}>{seeAllLabel(inbox.total)}</Link>}>
        Needs you
      </SectionHeading>
      {items.length ? (
        <ListRows ariaLabel="Waiting for you">
          {items.map((item) => {
            const slug = itemProject(item, brain);
            return <NeedsYouRow key={item.key} item={item} slug={slug} name={names.get(slug) || slug} canDecide={org.can_decide} />;
          })}
        </ListRows>
      ) : inbox.ready ? (
        <CaughtUp />
      ) : (
        <EmptyState compact title="Checking what waits for you…" />
      )}
    </section>
  );
}

// All caught up (spec "Home" - Other states): nothing waits, so the column
// offers the three things the owner starts from here - a session with an
// agent, a task for the fleet, and a look at what landed since.
function CaughtUp() {
  const [newTask, setNewTask] = useState(false);
  return (
    <EmptyState
      compact
      title="Nothing needs you"
      action={
        <Button size="sm" variant="outline" asChild>
          <Link href={hrefs.conversations()}>
            <MessageSquarePlusIcon /> Start a session
          </Link>
        </Button>
      }
      secondary={
        <>
          <Button size="sm" variant="outline" onClick={() => setNewTask(true)}>
            <PlusIcon /> Give a task
          </Button>
          <Button size="sm" variant="outline" asChild>
            <Link href={hrefs.codeDeliveries()}>
              <GitMergeIcon /> Review what landed
            </Link>
          </Button>
          <NewTaskDialog open={newTask} onOpenChange={setNewTask} />
        </>
      }
    >
      The agents carry on; a question or a proposal for you shows up here.
    </EmptyState>
  );
}

const KIND: Record<NeedsYouItem["kind"], Kind> = {
  question: "question",
  decision: "decision",
  proposal: "proposal",
  compute: "machine",
  work: "suggestion",
  draft: "record",
  step: "rule",
};

// Two lines and the answer (spec "Home" 3, 56 px rows): the title alone on
// the first, so it keeps its words; where it is, what it is and how long it
// has waited on the second; the item's own buttons in sight on the right.
const ROW = "min-h-[calc(var(--row-h)+0.75rem)]";

function NeedsYouRow({ item, slug, name, canDecide }: { item: NeedsYouItem; slug: string; name: string; canDecide: boolean }) {
  const lead = <KindTile kind={KIND[item.kind]} waiting />;
  const line = (what: string) => (
    <span className="inline-flex min-w-0 max-w-full items-center gap-1.5 leading-5">
      <ProjectAvatar slug={slug} name={name} size={20} />
      {/* On a narrow screen the kind word gives way first; the name and the age keep theirs. */}
      <span className="max-w-40 shrink-0 truncate">{name}</span>
      <span aria-hidden>·</span>
      <span className="min-w-0 truncate">{what}</span>
      {item.at ? (
        <>
          <span aria-hidden>·</span>
          <When d={item.at} className="shrink-0" />
        </>
      ) : null}
    </span>
  );
  const word = needsYouWord(item);
  switch (item.kind) {
    case "question": {
      const q = item.q;
      const stop = q.stop === true;
      const open = !stop && isOpenQuestion(q.summary, q.body);
      const labels = decisionLabels(q.body, stop ? { yes: "Carry on", no: "Keep paused" } : { yes: "Yes", no: "No" });
      const title = stop ? "Self-improvement stopped. May it carry on?" : sentence(q.summary);
      const who = stop ? "" : agentName(q.agent || q.name || "", slug);
      return (
        <ListRow
          className={ROW}
          lead={lead}
          title={title}
          // A "which…?" question needs words: its row leads to For you with
          // the answer box open. The row is the link; the button is its cue
          // (an <a> or a <button> inside the row's <a> would be invalid HTML).
          href={open ? answerHref(slug, q.id) : hrefs.post(slug, q.id)}
          actionsVisible
          actions={
            open ? (
              canDecide ? (
                <span className={buttonVariants({ size: "xs" })}>
                  <ReplyIcon aria-hidden /> Answer
                </span>
              ) : undefined
            ) : (
              <DecideButtons id={q.id} summary={title} stop={stop} canDecide={canDecide && q.decider !== "setting"} size="xs" yesLabel={labels.yes} noLabel={labels.no} className="w-auto justify-end" />
            )
          }
          ariaLabel={(stop ? "Self-improvement paused on " + name + ": " : "Question for you: ") + q.summary}
        >
          {line(who ? word + " from " + inSentence(who) : word)}
        </ListRow>
      );
    }
    case "decision": {
      const r = item.r;
      return (
        <ListRow className={ROW} lead={lead} title={sentence(r.title)} href={hrefs.record(r.id, r.project)} actionsVisible actions={canDecide && awaitsDecision(r) ? <RecordPair record={r} /> : undefined} ariaLabel={"Decision waiting: " + r.title}>
          {line(word)}
        </ListRow>
      );
    }
    case "proposal": {
      const p = item.p;
      const who = agentName(p.agent || p.name || "", slug);
      return (
        <ListRow className={ROW} lead={lead} title={sentence(p.summary)} href={hrefs.post(slug, p.id)} actionsVisible actions={canDecide && p.can_decide !== false ? <ProposalDecide p={p} /> : undefined} ariaLabel={"Proposal: " + p.summary}>
          {line(who ? word + " from " + inSentence(who) : word)}
        </ListRow>
      );
    }
    case "compute": {
      const c = item.c;
      const title = "Extra machine for " + c.hours + (c.hours === 1 ? " hour" : " hours") + (c.reason ? ": " + sentence(c.reason) : "");
      return (
        <ListRow className={ROW} lead={lead} title={title} href={hrefs.compute()} actionsVisible actions={canDecide ? <ComputeDecide r={c} size="xs" /> : undefined} ariaLabel={"Machine request: " + title}>
          {line(word + (c.size ? " · " + c.size : ""))}
        </ListRow>
      );
    }
    case "work": {
      const s = item.s;
      return (
        <ListRow className={ROW} lead={lead} title={s.title} href={hrefs.project(s.project, "suggestions")} actionsVisible actions={canDecide ? <WorkDecide s={s} /> : undefined} ariaLabel={"Suggested work: " + s.title}>
          {line(word)}
        </ListRow>
      );
    }
    case "draft": {
      const d = item.d;
      const what = d.r.id.includes("evaluators") ? "New evaluators drafted" : "A new constitution drafted";
      return (
        <ListRow className={ROW} lead={lead} title={what} href={hrefs.project(d.slug, "evolution")} actionsVisible actions={canDecide ? <DraftActions r={d.r} size="xs" /> : undefined} ariaLabel={what + " for " + name}>
          {line(word)}
        </ListRow>
      );
    }
    case "step": {
      const step = item.step;
      return (
        <ListRow className={ROW} lead={lead} title={step.title} href={hrefs.project(step.slug, "evolution")} actionsVisible actions={step.command ? <CopyButton text={step.command} label="Copy command" size="xs" variant="outline" /> : undefined} ariaLabel={"Next step for " + name + ": " + step.title}>
          {line(step.label || word)}
        </ListRow>
      );
    }
  }
}

// For you, filtered to the question's project, with its answer box open.
function answerHref(slug: string, id: string): string {
  const base = hrefs.forYou({ project: slug || undefined });
  return base + (base.includes("?") ? "&" : "?") + "answer=" + encodeURIComponent(id);
}

// agentName() starts a sentence ("The runner", "You"); after "from" it reads
// in lower case.
function inSentence(who: string): string {
  return who === "The runner" || who === "You" ? who.toLowerCase() : who;
}

// Decline / Accept for a decision record: the record's own confirm
// (components/records/decide-record.tsx); Defer waits on the record's page.
function RecordPair({ record }: { record: RecordRow }) {
  const [action, setAction] = useState<RecordDecision>("accept");
  const [open, setOpen] = useState(false);
  const start = (a: RecordDecision) => {
    setAction(a);
    setOpen(true);
  };
  return (
    <>
      <Button size="xs" variant="outline" onClick={() => start("Rejected")} aria-label={"Decline " + record.id + ": " + record.title}>
        <XIcon /> Decline
      </Button>
      <Button size="xs" onClick={() => start("accept")} aria-label={"Accept " + record.id + ": " + record.title}>
        <CheckIcon /> Accept
      </Button>
      <DecideRecordDialog record={record} action={action} open={open} onOpenChange={setOpen} />
    </>
  );
}

// Yes / No for a proposal: the card's confirm (components/hive/cards.tsx
// ProposalCard) in row size.
function ProposalDecide({ p }: { p: Proposal }) {
  const [open, setOpen] = useState<"yes" | "no" | null>(null);
  const yes = open === "yes";
  return (
    <>
      <Button size="xs" variant="outline" onClick={() => setOpen("no")} aria-label={"No, decline: " + p.summary}>
        <XIcon /> No
      </Button>
      <Button size="xs" onClick={() => setOpen("yes")} aria-label={"Yes, accept: " + p.summary}>
        <CheckIcon /> Yes
      </Button>
      <ConfirmDialog
        open={open !== null}
        onOpenChange={(o) => !o && setOpen(null)}
        title={yes ? "Accept this proposal?" : "Decline this proposal?"}
        description={
          <>
            <span className="block font-medium text-foreground">{sentence(p.summary)}</span>
            <span className="mt-2 block">{yes ? "The agents go ahead as proposed. If it asks for work, a job is queued." : "The agents won't pursue it as written."}</span>
          </>
        }
        action={yes ? "Yes, accept" : "No, decline"}
        words={{
          label: "A note for the agents (optional)",
          placeholder: yes ? "Accepted: keep the scope as written." : "Not now, because…",
        }}
        success={yes ? "Accepted. The agents will go ahead." : "Declined. The agents won't pursue it."}
        onConfirm={(words) => writes.decide(p.id, yes ? "yes" : "no", words)}
      />
    </>
  );
}

// Accept / Decline for suggested work: the suggestion's own dialogs
// (components/tasks/suggested-work.tsx); accepting opens the new task.
function WorkDecide({ s }: { s: WorkSuggestion }) {
  const router = useRouter();
  const [accept, setAccept] = useState(false);
  const [decline, setDecline] = useState(false);
  return (
    <>
      <Button size="xs" variant="outline" onClick={() => setDecline(true)} aria-label={"Decline: " + s.title}>
        <XIcon /> Decline
      </Button>
      <Button size="xs" onClick={() => setAccept(true)} aria-label={"Accept: " + s.title}>
        <CheckIcon /> Accept
      </Button>
      <AcceptSuggestionDialog s={s} open={accept} onOpenChange={setAccept} onAccepted={(t) => router.push(hrefs.task(t.id))} />
      <DeclineSuggestionDialog s={s} open={decline} onOpenChange={setDecline} />
    </>
  );
}
