"use client";

import { useEffect, useRef } from "react";
import { CheckIcon, StarIcon, XIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "./confirm";
import { writes, type AskOption } from "@/lib/api";
import type { QuestionAnswers } from "@/lib/owner-questions";
import { cn } from "@/lib/utils";
import { useAnswerDialog } from "./use-answer-dialog";
import { focusAnswer, optionAnswer, type YesNoAnswer } from "@/lib/answer-dialog";

export function DecideButtons({
  id,
  summary,
  stop,
  canDecide = true,
  size = "default",
  yesLabel = "Yes",
  noLabel = "No",
  className,
}: {
  id: string;
  summary: string;
  stop?: boolean;
  canDecide?: boolean;
  size?: "xs" | "sm" | "default";
  yesLabel?: string;
  noLabel?: string;
  // In a list row the pair keeps to the row's right edge on a phone too ("w-auto justify-end").
  className?: string;
}) {
  // Keyed by each opening (lib/answer-dialog.ts): a note typed under No is
  // never sent with a Yes.
  const { answer: open, key, pick, dialog } = useAnswerDialog<YesNoAnswer>();
  if (!canDecide) return <span className="ml-auto text-[13px] text-muted-foreground">To answer, open Hive on the machine that runs it.</span>;
  const yes = open === "yes";
  return (
    <>
      <span className={cn("flex w-full flex-wrap justify-start gap-2 sm:ml-auto sm:w-auto sm:justify-end", className)}>
        <Button size={size} variant="outline" onClick={() => pick("no")} aria-label={noLabel + " to: " + summary}>
          <XIcon /> {noLabel}
        </Button>
        <Button size={size} onClick={() => pick("yes")} aria-label={yesLabel + " to: " + summary}>
          <CheckIcon /> {yesLabel}
        </Button>
      </span>
      <ConfirmDialog
        key={key}
        {...dialog}
        title={yes ? yesLabel + "?" : noLabel + "?"}
        description={
          <>
            <span className="block">{summary}</span>
            {stop ? <span className="mt-2 block">{yes ? "Self-improvement carries on past this stop. The change that stopped it is not tried again as written." : "Self-improvement stays paused until you say yes."}</span> : <span className="mt-2 block">Your answer goes on the board, so the agents know what to do next.</span>}
          </>
        }
        action={yes ? yesLabel : noLabel}
        words={{ label: "A note for the agents (optional)", placeholder: stop ? (yes ? "Reviewed, carry on." : "Keep it paused because…") : yes ? "Yes, because…" : "No, because…" }}
        success={stop ? (yes ? "Answered. Self-improvement carries on." : "Answered. Self-improvement stays paused.") : yes ? "You said yes. The agents will act on it." : "You said no. The agents won't go ahead."}
        onConfirm={(words) => writes.decide(id, yes ? "yes" : "no", words)}
      />
    </>
  );
}

// One answer to pick: the recommended one filled, starred and marked, the
// others outlined; `filled` fills a default the agent did not mark (the
// first answer of a message from an older hub, TASK-149 item 14) without
// calling it recommended; `effect` (what picking it does) is its tooltip. Shared by
// the board's questions (AnswerPicker) and a lead's message in a task or
// chat, so the recommended answer reads the same wherever an agent asks.
export function AnswerButton({
  ref,
  label,
  recommended,
  filled,
  effect,
  about,
  size = "default",
  disabled,
  className,
  onClick,
}: {
  ref?: React.Ref<HTMLButtonElement>;
  label: string;
  recommended?: boolean;
  filled?: boolean;
  effect?: string;
  about?: string;
  size?: "xs" | "sm" | "default";
  disabled?: boolean;
  className?: string;
  onClick: () => void;
}) {
  return (
    <Button
      ref={ref}
      type="button"
      size={size}
      variant={recommended || filled ? "default" : "outline"}
      className={className}
      disabled={disabled}
      onClick={onClick}
      aria-label={label + (recommended ? " (recommended)" : "") + (about ? " to: " + about : "")}
      title={effect || undefined}
    >
      {recommended ? <StarIcon aria-hidden /> : null}
      {label}
      {recommended ? <span className="rounded-sm bg-primary-foreground/20 px-1 text-[11px] font-medium leading-4">Recommended</span> : null}
    </Button>
  );
}

// The answers a question for the owner offers, one button each, the
// recommended one first, filled and marked (TASK-129: "always give option no
// text.. and add recommended one as well.. always"). Picking one asks to
// confirm with an optional note and records it; there is no box to type an
// answer into. A question that names no answers, or none recommended, is
// not the owner's yet: it says so and offers nothing to press - the hub
// hands it back to its lead, which names the answers or decides it.
export function AnswerPicker({
  id,
  summary,
  answers,
  stop,
  canDecide = true,
  size = "default",
  className,
  focusFirst = false,
}: {
  id: string;
  summary: string;
  answers: QuestionAnswers;
  stop?: boolean;
  canDecide?: boolean;
  size?: "xs" | "sm" | "default";
  className?: string;
  // Give the recommended answer the focus (the one ask in front of the owner).
  focusFirst?: boolean;
}) {
  // Keyed by each opening (lib/answer-dialog.ts), by the answer's label: a
  // note typed under one answer is never sent with another.
  const { answer: pickedLabel, key, pick, dialog } = useAnswerDialog<string>();
  const first = useRef<HTMLButtonElement>(null);
  const focusOn = answers.kind === "pick" ? focusAnswer(answers.options) : null;
  // The recommended answer - first, filled and starred - takes the focus when
  // the answers are put in front of the owner, so Enter or Space picks it
  // (TASK-149 item 22). Only with `focusFirst`: a list of asks would fight
  // over the focus otherwise.
  useEffect(() => {
    if (focusFirst && focusOn) first.current?.focus({ preventScroll: true });
  }, [focusFirst, focusOn]);
  if (!canDecide) return <span className="ml-auto text-[13px] text-muted-foreground">To answer, open Hive on the machine that runs it.</span>;
  if (answers.kind === "with-lead") {
    return (
      <span className={cn("ml-auto text-[13px] text-muted-foreground anywhere", className)} data-answers="with-lead">
        With the lead: {answers.missing}. You are asked once it names the answers to pick.
      </span>
    );
  }
  const picked: AskOption | null = answers.options.find((o) => o.label === pickedLabel) ?? null;
  const spec = picked ? optionAnswer(id, picked, answers.byLabel) : null;
  return (
    <>
      <span className={cn("flex w-full flex-wrap justify-start gap-2 sm:ml-auto sm:w-auto sm:justify-end", className)} data-answers="pick">
        {answers.options.map((o) => (
          <AnswerButton key={o.label} ref={o.label === focusOn ? first : undefined} label={o.label} recommended={o.recommended} effect={o.effect} size={size} about={summary} onClick={() => pick(o.label)} />
        ))}
      </span>
      <ConfirmDialog
        key={key}
        {...dialog}
        title={(picked?.label ?? "") + "?"}
        description={
          <>
            <span className="block">{summary}</span>
            {picked?.effect ? <span className="mt-2 block">{sentence(picked.effect)}</span> : null}
            {picked?.recommended ? <span className="mt-2 block">The recommended answer{answers.why ? ": " + answers.why : ""}.</span> : null}
            {stop ? <span className="mt-2 block">{picked?.answer === "yes" ? "Self-improvement carries on past this stop. The change that stopped it is not tried again as written." : "Self-improvement stays paused until you say it may carry on."}</span> : <span className="mt-2 block">Your answer goes on the board, so the agents know what to do next.</span>}
          </>
        }
        action={spec?.action ?? ""}
        words={spec?.words ?? { label: "A note for the agents (optional)" }}
        success={spec?.success ?? ""}
        onConfirm={(words) => (spec ? spec.send(writes, words) : Promise.resolve({}))}
      />
    </>
  );
}

function sentence(t: string): string {
  const s = t.trim();
  if (!s) return s;
  return s.charAt(0).toUpperCase() + s.slice(1) + (/[.!?]$/.test(s) ? "" : ".");
}

// Kept for the pages that still name it: a question now always answers with
// a pick. Without the answers it can only say it is with the lead.
export function AnswerBox({ id, summary, canDecide = true, answers }: { id: string; summary: string; canDecide?: boolean; autoFocus?: boolean; answers?: QuestionAnswers }) {
  return <AnswerPicker id={id} summary={summary} canDecide={canDecide} answers={answers ?? { kind: "with-lead", missing: "it names no answers to pick" }} />;
}
