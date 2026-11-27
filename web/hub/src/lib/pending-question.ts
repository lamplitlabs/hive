// How the console offers a session's pending question (TASK-149 item 13):
// the `question` tool now requires two to six choices and a recommended one,
// so the user answers with one pick, the recommended one first and marked,
// and there is no box to type into. A question asked before choices were
// required (a conversation still waiting from an older build) names none:
// only then is the answer typed. Pure, so node --test can check it.

export interface PendingChoice {
  // What is sent back as the answer: the choice as the agent wrote it.
  answer: string;
  // The label on the button (the text before ` - `) and what picking it does.
  label: string;
  effect: string;
  recommended: boolean;
}

export interface PendingQuestionLike {
  choices?: readonly string[] | null;
  recommended?: string | null;
}

export function pendingChoices(q: PendingQuestionLike): { choices: PendingChoice[]; typed: boolean } {
  const all = [...(q.choices ?? [])].filter((c) => c.trim());
  const want = (q.recommended ?? "").trim().toLowerCase();
  const choices = all.map((answer) => {
    const cut = answer.indexOf(" - ");
    const label = (cut >= 0 ? answer.slice(0, cut) : answer).trim();
    const effect = cut >= 0 ? answer.slice(cut + 3).trim() : "";
    return { answer, label, effect, recommended: false };
  });
  const i = want ? choices.findIndex((c) => c.answer.trim().toLowerCase() === want || c.label.toLowerCase() === want) : -1;
  if (i >= 0) {
    const [pick] = choices.splice(i, 1);
    pick.recommended = true;
    choices.unshift(pick);
  }
  return { choices, typed: choices.length === 0 };
}
