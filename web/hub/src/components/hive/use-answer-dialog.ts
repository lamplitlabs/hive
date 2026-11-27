import { useCallback, useState } from "react";
import { answerKey, closeAnswer, CLOSED_DIALOG, pickAnswer, type AnswerDialogState } from "@/lib/answer-dialog";

// The keyed answer dialog in React state (lib/answer-dialog.ts says why):
// give the ConfirmDialog `key={key}` and `{...dialog}` (open, onOpenChange),
// and call `pick` from each answer's button. `answer` stays set while the
// dialog closes, so its words do not flip to the other answer's mid-exit.
export function useAnswerDialog<A extends string>() {
  const [s, set] = useState<AnswerDialogState<A>>(CLOSED_DIALOG);
  const pick = useCallback((a: A) => set((x) => pickAnswer(x, a)), []);
  const onOpenChange = useCallback((o: boolean) => {
    if (!o) set((x) => closeAnswer(x));
  }, []);
  return { answer: s.answer, key: answerKey(s), pick, dialog: { open: s.open, onOpenChange } };
}
