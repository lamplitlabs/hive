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

// Following the end of a log, like a terminal: the box stays pinned to its
// newest line while the reader sits there, and lets go when they scroll up
// to read. Only the reader's own move up lets go: a box that grows under a
// pinned reader (a picture loading, the canvas opening beside it and
// rewrapping the lines, a smooth scroll still on its way down) keeps
// following, so a page opens at its newest message whatever the window.
export const FOLLOW_SLACK = 80;

export type ScrollBox = {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
};

// How far the box's bottom edge is above the end of its content.
export function endGap(box: ScrollBox): number {
  return Math.max(0, box.scrollHeight - box.scrollTop - box.clientHeight);
}

// Whether to keep following after a scroll event: near the end always
// follows; a move up away from it lets go; anything else (a programmatic
// scroll down, a box clamping its offset) keeps what it was.
export function followAfterScroll(
  following: boolean,
  prevTop: number,
  box: ScrollBox,
): boolean {
  if (endGap(box) < FOLLOW_SLACK) return true;
  if (box.scrollTop < prevTop) return false;
  return following;
}
