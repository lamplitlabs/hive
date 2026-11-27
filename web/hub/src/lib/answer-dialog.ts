// One confirm dialog for the answers of a two-answer (or n-answer) ask:
// Approve / Deny, Accept / Decline, Yes / No, or a question's picks. Pure, so
// node --test drives it (answer-dialog.test.mjs); components/hive/
// use-answer-dialog.ts holds it in React state.
//
// The dialog is keyed by the opening, not by "open or closed" (TASK-149 item
// 17): every pick mounts a fresh dialog, so a note typed under Deny is never
// sent with an Approve, nor one typed and cancelled with the next pick of the
// same answer; closing keeps the key and the answer, so the dialog plays its
// exit animation with its own title instead of being unmounted at once.

export interface AnswerDialogState<A extends string> {
  open: boolean;
  // The answer the dialog is (or was last) about; null before the first pick.
  answer: A | null;
  // How many times it was opened: part of the key.
  n: number;
}

export const CLOSED_DIALOG: AnswerDialogState<never> = { open: false, answer: null, n: 0 };

export function pickAnswer<A extends string>(s: AnswerDialogState<A>, answer: A): AnswerDialogState<A> {
  return { open: true, answer, n: s.n + 1 };
}

export function closeAnswer<A extends string>(s: AnswerDialogState<A>): AnswerDialogState<A> {
  return s.open ? { ...s, open: false } : s;
}

// The React key of the dialog: the same while one opening is shown and while
// it closes, new for the next pick.
export function answerKey<A extends string>(s: AnswerDialogState<A>): string {
  return s.answer === null ? "none" : s.n + ":" + s.answer;
}

// The answer to give the focus to when a row of answers is put in front of
// the owner (TASK-149 item 22): the recommended one, which is shown first,
// filled and starred; none when no single answer is recommended.
export function focusAnswer<T extends { label: string; recommended?: boolean }>(options: readonly T[]): string | null {
  const marked = options.filter((o) => o.recommended);
  return marked.length === 1 ? marked[0].label : null;
}

// ----- what each answer sends ------------------------------------------------
//
// The words and the write of each answer, kept out of the JSX so one test
// (answer-dialog.test.mjs) can open each dialog, type, cancel, pick the other
// answer and assert what its button sends (TASK-149 item 24: "Approve carried
// the words of Deny" was found by reading). Every dialog that asks for one of
// these answers takes its words and its write from here.

// The writes these dialogs make; lib/api.ts `writes` has this shape.
export interface AnswerWrites {
  approveCompute(id: string, words: string): Promise<unknown>;
  denyCompute(id: string, words: string): Promise<unknown>;
  decide(id: string, answer: "yes" | "no", words: string): Promise<unknown>;
  decideOption(id: string, option: string, words: string): Promise<unknown>;
}

export interface AnswerWords {
  label: string;
  placeholder?: string;
  required?: boolean;
  initial?: string;
}

export interface AnswerSpec {
  action: string;
  destructive: boolean;
  words: AnswerWords;
  success: string;
  send: (w: AnswerWrites, words: string) => Promise<unknown>;
}

// A compute request. Deny opens with its reason already written, so it is
// one tap wherever it is asked - on the strip and on the Compute page alike
// (TASK-149 item 17); the agent still hears why.
export type ComputeAnswer = "approve" | "deny";
export const DENY_COMPUTE_NOTE = "Not now. Use the machines you have.";

export function computeAnswer(id: string, a: ComputeAnswer): AnswerSpec {
  if (a === "approve") {
    return {
      action: "Approve",
      destructive: false,
      words: { label: "A note for the agent (optional)", placeholder: "Approved: needed for the load test.", initial: "" },
      success: "Approved. Hive will start the machine.",
      send: (w, words) => w.approveCompute(id, words),
    };
  }
  return {
    action: "Deny",
    destructive: true,
    words: { label: "Tell the agent why", placeholder: DENY_COMPUTE_NOTE, required: true, initial: DENY_COMPUTE_NOTE },
    success: "Denied. The agent will see your note.",
    send: (w, words) => w.denyCompute(id, words),
  };
}

// A yes or no on a post (a proposal, or a question with two answers).
export type YesNoAnswer = "yes" | "no";

export function decideAnswer(id: string, a: YesNoAnswer, o: { yesLabel: string; noLabel: string; yesPlaceholder: string; noPlaceholder: string; yesSuccess: string; noSuccess: string }): AnswerSpec {
  const yes = a === "yes";
  return {
    action: yes ? o.yesLabel : o.noLabel,
    destructive: !yes,
    words: { label: "A note for the agents (optional)", placeholder: yes ? o.yesPlaceholder : o.noPlaceholder, initial: "" },
    success: yes ? o.yesSuccess : o.noSuccess,
    send: (w, words) => w.decide(id, a, words),
  };
}

// A proposal's Accept / Decline, the same on its card and on the strip.
export function proposalAnswer(id: string, a: YesNoAnswer): AnswerSpec {
  return decideAnswer(id, a, {
    yesLabel: "Yes, accept",
    noLabel: "No, decline",
    yesPlaceholder: "Accepted: keep the scope as written.",
    noPlaceholder: "Not now, because…",
    yesSuccess: "Accepted. The agents will go ahead.",
    noSuccess: "Declined. The agents won't pursue it.",
  });
}

// One of a question's answers: by its label when the hub reads the answers
// from the post (`byLabel`), else its yes or no.
export function optionAnswer(id: string, o: { label: string; answer: "yes" | "no" | "" }, byLabel: boolean): AnswerSpec {
  return {
    action: o.label,
    destructive: false,
    words: { label: "A note for the agents (optional)", placeholder: "Because…", initial: "" },
    success: "Answered: " + o.label + ". The agents will act on it.",
    send: (w, words) => (byLabel ? w.decideOption(id, o.label, words) : w.decide(id, o.answer === "yes" ? "yes" : "no", words)),
  };
}
