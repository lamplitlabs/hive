// Scroll one box so a child shows, and only that box. The browser's
// scrollIntoView walks every overflowing ancestor as well, so a transcript
// following its last line or a canvas centring a file's line would also
// drag the column it sits in - and on the Chat page that column is shared
// with the conversations list, which must stay where the owner left it.

export type Block = "start" | "center" | "end" | "nearest";
export type Axis = "y" | "x";

// Where to put `child` (its offset and size inside the box's scrollable
// content) given the box's viewport (its scroll offset and inner size);
// null when `nearest` finds it already in view. Pure, so it has a test.
export function revealOffset(
  block: Block,
  child: { start: number; size: number },
  view: { scroll: number; size: number },
): number | null {
  const end = child.start + child.size;
  let target: number | null = null;
  switch (block) {
    case "start":
      target = child.start;
      break;
    case "center":
      target = child.start - (view.size - child.size) / 2;
      break;
    case "end":
      target = end - view.size;
      break;
    case "nearest":
      if (child.start < view.scroll) target = child.start;
      else if (end > view.scroll + view.size) target = end - view.size;
      break;
  }
  return target === null ? null : Math.max(0, target);
}

export function revealIn(
  box: HTMLElement,
  child: HTMLElement,
  block: Block = "nearest",
  behavior: ScrollBehavior = "auto",
  axis: Axis = "y",
): void {
  const b = box.getBoundingClientRect();
  const c = child.getBoundingClientRect();
  if (axis === "y") {
    const top = revealOffset(
      block,
      { start: c.top - b.top + box.scrollTop, size: c.height },
      { scroll: box.scrollTop, size: box.clientHeight },
    );
    if (top !== null) box.scrollTo({ top, behavior });
  } else {
    const left = revealOffset(
      block,
      { start: c.left - b.left + box.scrollLeft, size: c.width },
      { scroll: box.scrollLeft, size: box.clientWidth },
    );
    if (left !== null) box.scrollTo({ left, behavior });
  }
}
