"use client";

import { useId, useState } from "react";
import { CheckIcon, SendIcon, XIcon } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { ConfirmDialog, showApiError } from "./confirm";
import { writes } from "@/lib/api";
import { invalidate } from "@/lib/use-api";

export function DecideButtons({
  id,
  summary,
  stop,
  canDecide = true,
  size = "default",
  yesLabel = "Yes",
  noLabel = "No",
}: {
  id: string;
  summary: string;
  stop?: boolean;
  canDecide?: boolean;
  size?: "xs" | "sm" | "default";
  yesLabel?: string;
  noLabel?: string;
}) {
  const [open, setOpen] = useState<"yes" | "no" | null>(null);
  if (!canDecide) return <span className="ml-auto text-[13px] text-muted-foreground">To answer, open PULSE on the machine that runs it.</span>;
  const yes = open === "yes";
  return (
    <>
      <span className="flex w-full flex-wrap justify-start gap-2 sm:ml-auto sm:w-auto sm:justify-end">
        <Button size={size} variant="outline" onClick={() => setOpen("no")} aria-label={noLabel + " to: " + summary}>
          <XIcon /> {noLabel}
        </Button>
        <Button size={size} onClick={() => setOpen("yes")} aria-label={yesLabel + " to: " + summary}>
          <CheckIcon /> {yesLabel}
        </Button>
      </span>
      <ConfirmDialog
        open={open !== null}
        onOpenChange={(o) => !o && setOpen(null)}
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

// A question the owner answers in words ("Which payment sandbox should the
// fixtures hit?"): a box for the answer and one button that sends it, so
// the owner never has to squeeze an answer into the note under a Yes. The
// board records it as the owner's yes with these words, which the agents
// read as the answer. Declining (not now, don't pursue it) stays a quiet
// second choice, before the answer as everywhere else.
export function AnswerBox({ id, summary, canDecide = true }: { id: string; summary: string; canDecide?: boolean }) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [declining, setDeclining] = useState(false);
  const field = useId();
  if (!canDecide) return <span className="ml-auto text-[13px] text-muted-foreground">To answer, open PULSE on the machine that runs it.</span>;
  const send = async () => {
    const words = text.trim();
    if (!words) return;
    setBusy(true);
    const t = toast.loading("Sending your answer…");
    try {
      await writes.decide(id, "yes", words);
      toast.success("Answer sent. The agents will act on it.", { id: t });
      setText("");
      invalidate();
    } catch (e) {
      toast.dismiss(t);
      showApiError(e);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="grid w-full gap-2">
      <label htmlFor={field} className="text-[13px] font-medium text-foreground">
        Your answer
      </label>
      <Textarea
        id={field}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void send();
        }}
        rows={2}
        placeholder="Write the answer the way you'd tell a colleague."
        aria-label={"Your answer to: " + summary}
        className="min-h-16 bg-card"
        disabled={busy}
      />
      <span className="flex flex-wrap items-center justify-end gap-2">
        <Button size="sm" variant="ghost" onClick={() => setDeclining(true)} disabled={busy} aria-label={"Decline to answer: " + summary}>
          <XIcon /> Not now
        </Button>
        <Button size="sm" onClick={() => void send()} disabled={busy || !text.trim()} title={text.trim() ? undefined : "Write an answer first"}>
          <SendIcon /> Send answer
        </Button>
      </span>
      <ConfirmDialog
        open={declining}
        onOpenChange={setDeclining}
        title="Leave this unanswered?"
        description={
          <>
            <span className="block">{summary}</span>
            <span className="mt-2 block">The agents learn you declined and won&apos;t go ahead with what they asked about.</span>
          </>
        }
        action="Decline"
        words={{ label: "A note for the agents (optional)", placeholder: "Not now, because…" }}
        success="Declined. The agents won't go ahead."
        onConfirm={(words) => writes.decide(id, "no", words)}
      />
    </div>
  );
}
