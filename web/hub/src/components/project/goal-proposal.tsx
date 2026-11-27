"use client";

import Link from "next/link";
import { useId, useState } from "react";
import { CheckIcon, Loader2Icon, PencilIcon, SparklesIcon } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { ConfirmDialog, showApiError } from "@/components/hive/confirm";
import { AnswerButton } from "@/components/hive/decide-buttons";
import { SectionHeading } from "@/components/hive/section-heading";
import { When } from "@/components/hive/who";
import { projectPath, writes, type GoalProposal, type ProjectGoalPayload } from "@/lib/api";
import { num } from "@/lib/format";
import { hrefs } from "@/lib/nav";
import { invalidate, useApi } from "@/lib/use-api";
import { cn } from "@/lib/utils";
import { editThenUse, GOAL_PROPOSAL_ANCHOR, GOAL_PROPOSAL_LABELS as L, GOAL_PROPOSAL_TITLE, goalAnswers, goalProposalView, MAX_GOAL_CHARS, type GoalAnswers } from "./goal-proposal-state";

// A proposed project goal waiting for the owner (TASK-117), wherever it
// shows: the project's Overview (its own panel) and the Sessions strip of
// hard stops (under its row; a goal change is the owner's alone). The words,
// the two answers and the recommended one live in goal-proposal-state.ts.

// Every view that shows a goal or the inbox: the org rows carry the proposal.
const goalViews = (path: string) => path === "org" || path === "projects" || path.startsWith("projects/") || path.startsWith("events");

export function GoalProposalCard({ slug, g, canDecide, className, framed = true, withActions = true }: { slug: string; g: GoalProposal; canDecide: boolean; className?: string; framed?: boolean; withActions?: boolean }) {
  const goal = useApi<ProjectGoalPayload>(projectPath(slug, "goal"), { refresh: 60000 });
  const v = goalProposalView(g, slug, goal.data?.goal ?? null);
  const answers = goalAnswers(g, goal.data?.goal ?? null);
  const recommended = answers.options.find((o) => o.recommended);
  return (
    <div
      className={cn(framed && "attn rounded-lg border border-info/40 bg-info/5 p-4", "grid min-w-0 gap-3 text-sm", className)}
      aria-label={v.title}
      role="group"
    >
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[13px] text-muted-foreground">
        <SparklesIcon className="size-3.5 shrink-0 text-info" aria-hidden />
        <span>{v.by}</span>
        {g.date ? (
          <>
            <span aria-hidden>·</span>
            <When d={g.date} />
          </>
        ) : null}
        <Link href={hrefs.post(slug, g.post)} className="ml-auto underline underline-offset-2 hover:text-foreground">Open the post</Link>
      </div>
      <blockquote className="max-w-3xl whitespace-pre-wrap border-l-2 border-info/50 pl-3 leading-relaxed text-foreground anywhere">{v.text}</blockquote>
      <p className="max-w-3xl leading-relaxed text-muted-foreground anywhere">
        <span className="font-medium text-foreground">Why: </span>
        {v.why}
      </p>
      <p className="text-[13px] text-muted-foreground">{v.replaces} {v.dismissNote}</p>
      {recommended ? (
        <p className="max-w-3xl text-[13px] text-muted-foreground anywhere" data-goal-recommended={recommended.answer}>
          Recommended: <span className="font-medium text-foreground">{recommended.label}</span> - {answers.why}
        </p>
      ) : null}
      {withActions ? <GoalProposalActions slug={slug} g={g} answers={answers} canDecide={canDecide} canEdit={goal.data?.can_edit !== false} /> : null}
    </div>
  );
}

// The owner's answers: Accept the drafted goal / Keep the current goal, the
// recommended one first, filled and marked (the decide route's yes and no; a
// yes sets the goal), and Edit then use, which says yes and then saves the
// owner's words as the goal.
export function GoalProposalActions({ slug, g, answers, canDecide, canEdit = true, size = "sm", className }: { slug: string; g: GoalProposal; answers: GoalAnswers; canDecide: boolean; canEdit?: boolean; size?: "xs" | "sm"; className?: string }) {
  const [open, setOpen] = useState<"yes" | "no" | "edit" | null>(null);
  if (!canDecide) return <span className="text-[13px] text-muted-foreground">To answer, open Hive on the machine that runs it.</span>;
  const yes = open === "yes";
  const ordered = [...answers.options].sort((a, b) => Number(b.recommended) - Number(a.recommended));
  return (
    <>
      <span className={cn("flex flex-wrap items-center gap-2", className)} data-answers="pick">
        {ordered.map((o) => (
          <AnswerButton key={o.label} label={o.label} recommended={o.recommended} effect={o.effect} size={size} about="the proposed goal" onClick={() => setOpen(o.answer === "yes" ? "yes" : "no")} />
        ))}
        {canEdit ? (
          <Button size={size} variant="outline" onClick={() => setOpen("edit")} aria-label={L.edit + ": the proposed goal"}>
            <PencilIcon /> {L.edit}
          </Button>
        ) : null}
      </span>
      <ConfirmDialog
        open={open === "yes" || open === "no"}
        onOpenChange={(o) => !o && setOpen(null)}
        title={yes ? L.use + "?" : L.dismiss + "?"}
        description={
          <>
            <span className="block max-h-40 overflow-y-auto whitespace-pre-wrap font-medium text-foreground anywhere">{g.text}</span>
            <span className="mt-2 block">{yes ? "Every agent on this project works toward it from its next run." : "The current goal stays; the same draft is not proposed again."}</span>
            {answers.options.find((o) => o.answer === (yes ? "yes" : "no"))?.recommended ? <span className="mt-2 block">{"The recommended answer: " + answers.why + "."}</span> : null}
          </>
        }
        action={yes ? L.use : L.dismiss}
        words={{ label: "A note for the agents (optional)", placeholder: yes ? "Yes, because…" : "Not this one, because…" }}
        success={yes ? "Goal set. Agents read it from their next run." : "Kept the current goal. The draft is dismissed."}
        invalidates={goalViews}
        onConfirm={(words) => writes.decide(g.post, yes ? "yes" : "no", words)}
      />
      {open === "edit" ? <EditThenUse slug={slug} g={g} onClose={() => setOpen(null)} /> : null}
    </>
  );
}

function EditThenUse({ slug, g, onClose }: { slug: string; g: GoalProposal; onClose: () => void }) {
  const id = useId();
  const [text, setText] = useState(g.text);
  const [busy, setBusy] = useState(false);
  const count = Array.from(text.trim()).length;
  const tooLong = count > MAX_GOAL_CHARS;

  const save = async () => {
    const plan = editThenUse(g.text, text);
    if (!plan.ok) {
      toast.error(plan.error);
      return;
    }
    setBusy(true);
    const notice = toast.loading("Setting the goal…");
    try {
      await writes.decide(g.post, "yes", plan.words);
    } catch (error) {
      toast.dismiss(notice);
      showApiError(error, "The goal wasn't set");
      setBusy(false);
      return;
    }
    try {
      if (plan.edited) await writes.setProjectGoal(slug, plan.text);
      toast.success("Goal set. Agents read it from their next run.", { id: notice });
      onClose();
    } catch (error) {
      toast.dismiss(notice);
      showApiError(error, "The proposed goal was used, but your edits weren't saved; edit the goal again");
    } finally {
      invalidate(goalViews);
      setBusy(false);
    }
  };

  return (
    <Dialog open onOpenChange={(o) => !o && !busy && onClose()}>
      <DialogContent className="sm:max-w-xl">
        <form onSubmit={(e) => { e.preventDefault(); if (!busy) void save(); }} className="grid gap-4">
          <DialogHeader>
            <DialogTitle>Edit the proposed goal, then use it</DialogTitle>
            <DialogDescription>Change what you want; saving says yes to the proposal and sets your words as the goal of {slug}.</DialogDescription>
          </DialogHeader>
          <div className="grid gap-2">
            <Label htmlFor={id}>Goal</Label>
            <Textarea id={id} value={text} onChange={(e) => setText(e.target.value)} className="max-h-72 resize-y" rows={8} autoFocus disabled={busy} aria-invalid={tooLong} aria-describedby={id + "-count"} />
            <p id={id + "-count"} className={"text-xs " + (tooLong ? "text-bad" : "text-muted-foreground")}>
              {num(count)} of {num(MAX_GOAL_CHARS)} characters.
            </p>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" disabled={busy} onClick={onClose}>Cancel</Button>
            <Button type="submit" disabled={busy || !count || tooLong}>
              {busy ? <Loader2Icon className="animate-spin" /> : <CheckIcon />} Use this goal
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

// The answers on a row of the Sessions strip, where the card under the row
// carries none: the same goal in force decides which one is recommended.
export function GoalProposalRowActions({ slug, g, canDecide }: { slug: string; g: GoalProposal; canDecide: boolean }) {
  const goal = useApi<ProjectGoalPayload>(projectPath(slug, "goal"), { refresh: 60000 });
  return <GoalProposalActions slug={slug} g={g} answers={goalAnswers(g, goal.data?.goal ?? null)} canDecide={canDecide} canEdit={goal.data?.can_edit !== false} size="xs" className="w-full justify-start sm:ml-auto sm:w-auto sm:justify-end" />;
}

// The Overview's panel: the card under its heading, linkable as #goal-proposal.
export function GoalProposalPanel({ slug, g, canDecide }: { slug: string; g: GoalProposal; canDecide: boolean }) {
  return (
    <section aria-label="Proposed goal" className="min-w-0 scroll-mt-20" id={GOAL_PROPOSAL_ANCHOR}>
      <SectionHeading icon={SparklesIcon} note="Waits for your yes" as="h2">
        {GOAL_PROPOSAL_TITLE}
      </SectionHeading>
      <GoalProposalCard slug={slug} g={g} canDecide={canDecide} />
    </section>
  );
}
