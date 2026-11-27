// The answers a lead's message offers, in the order the console shows them:
// the recommended one first (the hub marks exactly one, TASK-129), the rest
// as the agent wrote them. A message from an older hub marks none: then the
// first is the default, as before. Pure, so node --test can check it.

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

// Whether a choice is the one to show as the default: the marked one, or
// the first when none is marked.
export function isDefaultChoice<T extends ChoiceLike>(choices: readonly T[], c: T): boolean {
  return choices.some((x) => x.recommended) ? !!c.recommended : choices[0] === c;
}
