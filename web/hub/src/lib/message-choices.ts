// The answers a lead's message offers, in the order the console shows them:
// the recommended one first (the hub marks exactly one, TASK-129), the rest
// as the agent wrote them. A message from an older hub marks none: then the
// first is still the default (filled), but nothing calls it recommended -
// the agent never said so (TASK-149 item 14). Pure, so node --test can check it.

export interface ChoiceLike {
  label: string;
  recommended?: boolean;
}

export function orderChoices<T extends ChoiceLike>(choices: readonly T[] | null | undefined): T[] {
  const all = [...(choices ?? [])];
  const i = all.findIndex((c) => c.recommended);
  if (i <= 0) return all;
  return [all[i], ...all.slice(0, i), ...all.slice(i + 1)];
}

// Whether a choice is the one to show as the default (filled): the marked
// one, or the first when none is marked.
export function isDefaultChoice<T extends ChoiceLike>(choices: readonly T[], c: T): boolean {
  return choices.some((x) => x.recommended) ? !!c.recommended : choices[0] === c;
}

// How a choice's button looks: `recommended` (filled, starred and labelled -
// the agent marked it), `default` (filled only - the first of a message that
// marks none) or `plain` (outlined).
export type ChoiceLook = "recommended" | "default" | "plain";

export function choiceLook<T extends ChoiceLike>(choices: readonly T[], c: T): ChoiceLook {
  const marked = choices.filter((x) => x.recommended);
  if (marked.length === 1) return marked[0] === c ? "recommended" : "plain";
  return choices[0] === c ? "default" : "plain";
}
