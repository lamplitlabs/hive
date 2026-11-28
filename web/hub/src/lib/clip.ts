// Text cut to its box: whether it is, and the tooltip that gives it back.
// Pure so node --test covers it (clip.test.mjs); the select trigger and
// other one-line boxes set the title from it when the pointer or focus
// arrives, so a value ended with an ellipsis can still be read whole.

// A box is clipped when its content is wider than what it shows; a pixel of
// rounding is not a cut.
export function isClipped(scrollWidth: number, clientWidth: number): boolean {
  return scrollWidth - clientWidth > 1;
}

// The tooltip for a one-line box: its whole text when the box cuts it,
// nothing when it fits (a tooltip that repeats what is in sight is noise).
export function clippedTitle(text: string | null | undefined, scrollWidth: number, clientWidth: number): string | undefined {
  const t = (text ?? "").replace(/\s+/g, " ").trim();
  if (!t) return undefined;
  return isClipped(scrollWidth, clientWidth) ? t : undefined;
}
