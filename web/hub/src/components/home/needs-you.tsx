"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { ArrowRightIcon, CheckIcon, GitMergeIcon, MessageSquarePlusIcon, PlusIcon, ScaleIcon, XIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ProjectAvatar } from "@/components/pulse/avatar";
import { ConfirmDialog } from "@/components/pulse/confirm";
import { DecideButtons } from "@/components/pulse/decide-buttons";
import { CopyButton } from "@/components/pulse/evolve";
import type { Inbox } from "@/components/pulse/inbox";
import { KindTile, type Kind } from "@/components/pulse/kind-tile";
import { ListRow, ListRows } from "@/components/pulse/list-row";
import { EmptyState } from "@/components/pulse/panel";
import { SectionHeading } from "@/components/pulse/section-heading";
import { StatusBadge } from "@/components/pulse/status-badge";
import { When } from "@/components/pulse/who";
import { ComputeDecide } from "@/components/compute/compute";
import { RecordDecisionButtons } from "@/components/records/decide-record";
import { DraftActions } from "@/components/records/draft-panel";
import { AcceptSuggestionDialog, DeclineSuggestionDialog } from "@/components/tasks/suggested-work";
import { NewTaskDialog } from "@/components/tasks/task-dialogs";
import { writes, type Org, type Proposal, type WorkSuggestion } from "@/lib/api";
import { sentence } from "@/lib/format";
import { hrefs } from "@/lib/nav";
import { decisionLabels, isOpenQuestion } from "@/lib/owner-questions";
import { itemProject, seeAllLabel, topFive, type NeedsYouItem } from "./home-state";

// "Needs you" (spec "Home" 3): the top five waiting items as rows, each with
// its kind tile, title, project avatar and age, and the item's own yes / no
// on the right. The buttons are the ones the cards use elsewhere (the same
// confirms, the same writes); the row itself opens the item's page.
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

function NeedsYouRow({ item, slug, name, canDecide }: { item: NeedsYouItem; slug: string; name: string; canDecide: boolean }) {
  const meta = (
    <>
      <ProjectAvatar slug={slug} name={name} size={20} />
      <span className="truncate">{name}</span>
    </>
  );
  const lead = <KindTile kind={KIND[item.kind]} waiting />;
  const time = item.at ? <When d={item.at} /> : undefined;
  switch (item.kind) {
    case "question": {
      const q = item.q;
      const stop = q.stop === true;
      const open = !stop && isOpenQuestion(q.summary, q.body);
      const labels = decisionLabels(q.body, stop ? { yes: "Yes, carry on", no: "No, keep it paused" } : { yes: "Yes", no: "No" });
      const title = stop ? "Self-improvement on " + slug + " stopped. May it carry on?" : sentence(q.summary);
      return (
        <ListRow
          lead={lead}
          title={title}
          meta={meta}
          time={time}
          status={stop ? <StatusBadge status="paused" label="Paused" size="sm" /> : undefined}
          // A "which…?" question needs words: its row leads to For you, where the
          // answer box is. The row is the link; the cue is a span (an <a> inside
          // the row's <a> would be invalid HTML).
          href={open ? hrefs.forYou({ project: slug }) : hrefs.post(slug, q.id)}
          actionsVisible={open}
          actions={
            open ? (
              <span className="inline-flex items-center gap-1 text-xs font-medium text-link">
                Answer <ArrowRightIcon className="size-3.5" aria-hidden />
              </span>
            ) : (
              <DecideButtons id={q.id} summary={title} stop={stop} canDecide={canDecide && q.decider !== "setting"} size="xs" yesLabel={labels.yes} noLabel={labels.no} />
            )
          }
          ariaLabel={(stop ? "Self-improvement paused: " : "Question for you: ") + q.summary}
        />
      );
    }
    case "decision": {
      const r = item.r;
      return <ListRow lead={lead} title={r.title} meta={meta} time={time} href={hrefs.record(r.id, r.project)} actions={canDecide ? <RecordDecisionButtons record={r} size="xs" /> : undefined} ariaLabel={"Decision waiting: " + r.title} />;
    }
    case "proposal": {
      const p = item.p;
      return <ListRow lead={lead} title={sentence(p.summary)} meta={meta} time={time} href={hrefs.post(slug, p.id)} actions={canDecide && p.can_decide !== false ? <ProposalDecide p={p} /> : undefined} ariaLabel={"Proposal: " + p.summary} />;
    }
    case "compute": {
      const c = item.c;
      const title = c.size + " for " + c.hours + (c.hours === 1 ? " hour" : " hours") + (c.reason ? ": " + sentence(c.reason) : "");
      return <ListRow lead={lead} title={title} meta={meta} time={time} href={hrefs.compute()} actions={canDecide ? <ComputeDecide r={c} size="xs" /> : undefined} ariaLabel={"Machine request: " + title} />;
    }
    case "work": {
      const s = item.s;
      return <ListRow lead={lead} title={s.title} meta={meta} time={time} href={hrefs.project(s.project, "suggestions")} actions={canDecide ? <WorkDecide s={s} /> : undefined} ariaLabel={"Suggested work: " + s.title} />;
    }
    case "draft": {
      const d = item.d;
      const what = d.r.id.includes("evaluators") ? "New evaluators drafted" : "A new constitution drafted";
      return <ListRow lead={lead} title={what} meta={meta} time={time} href={hrefs.project(d.slug, "evolution")} actions={canDecide ? <DraftActions r={d.r} size="xs" /> : undefined} ariaLabel={what + " for " + name} />;
    }
    case "step": {
      const step = item.step;
      return (
        <ListRow lead={lead} title={step.title} meta={meta} href={hrefs.project(step.slug, "evolution")} actions={step.command ? <CopyButton text={step.command} label="Copy command" size="xs" variant="outline" /> : undefined} ariaLabel={"Next step for " + name + ": " + step.title}>
          {step.label}
        </ListRow>
      );
    }
  }
}

// Yes / No for a proposal: the card's confirm (components/pulse/cards.tsx
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
