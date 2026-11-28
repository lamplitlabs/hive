import assert from "node:assert/strict";
import { test } from "node:test";
import { revealIn, revealOffset } from "./scroll.ts";

// A box 100 tall, scrolled to 500, so it shows 500..600.
const view = { scroll: 500, size: 100 };

test("nearest leaves a child already in view alone", () => {
  assert.equal(revealOffset("nearest", { start: 520, size: 30 }, view), null);
});

test("nearest scrolls the least that brings the child in", () => {
  // Above the view: its top lands at the top.
  assert.equal(revealOffset("nearest", { start: 450, size: 30 }, view), 450);
  // Below the view: its bottom lands at the bottom.
  assert.equal(revealOffset("nearest", { start: 650, size: 30 }, view), 580);
});

test("start, center and end place the child where asked", () => {
  assert.equal(revealOffset("start", { start: 650, size: 30 }, view), 650);
  assert.equal(revealOffset("center", { start: 650, size: 30 }, view), 615);
  assert.equal(revealOffset("end", { start: 650, size: 30 }, view), 580);
});

test("a child near the top never asks for a negative scroll", () => {
  assert.equal(revealOffset("center", { start: 10, size: 30 }, view), 0);
  assert.equal(revealOffset("end", { start: 10, size: 30 }, view), 0);
});

// A fake scroll box: a viewport `clientHeight` x `clientWidth` placed on the
// page at (boxTop, boxLeft), scrolled to (scrollTop, scrollLeft), recording
// every scrollTo call. A child is placed by its offset inside the content.
function fakeBox({ scrollTop = 0, scrollLeft = 0, clientHeight = 100, clientWidth = 200, boxTop = 40, boxLeft = 15 } = {}) {
  const calls = [];
  return {
    scrollTop,
    scrollLeft,
    clientHeight,
    clientWidth,
    scrollHeight: 2000,
    scrollWidth: 4000,
    calls,
    getBoundingClientRect: () => ({ top: boxTop, left: boxLeft, height: clientHeight, width: clientWidth }),
    scrollTo(opts) {
      calls.push(opts);
    },
  };
}

// A child whose content offset is (top, left): on the page it sits at the
// box's position plus that offset less the box's scroll.
function fakeChild(box, { top = 0, left = 0, height = 30, width = 50 } = {}) {
  const b = box.getBoundingClientRect();
  return {
    getBoundingClientRect: () => ({
      top: b.top + top - box.scrollTop,
      left: b.left + left - box.scrollLeft,
      height,
      width,
    }),
  };
}

test("revealIn scrolls the box vertically for each block", () => {
  const cases = [
    ["start", 650],
    ["center", 615],
    ["end", 580],
    ["nearest", 580],
  ];
  for (const [block, top] of cases) {
    const box = fakeBox({ scrollTop: 500 });
    revealIn(box, fakeChild(box, { top: 650 }), block, "smooth");
    assert.deepEqual(box.calls, [{ top, behavior: "smooth" }], block);
  }
});

test("revealIn defaults to nearest, auto and the vertical axis", () => {
  const box = fakeBox({ scrollTop: 500 });
  revealIn(box, fakeChild(box, { top: 450 }));
  assert.deepEqual(box.calls, [{ top: 450, behavior: "auto" }]);
});

test("revealIn leaves the box alone when nearest finds the child in view", () => {
  const box = fakeBox({ scrollTop: 500 });
  revealIn(box, fakeChild(box, { top: 520 }), "nearest");
  assert.deepEqual(box.calls, []);
  const wide = fakeBox({ scrollLeft: 300 });
  revealIn(wide, fakeChild(wide, { left: 320 }), "nearest", "auto", "x");
  assert.deepEqual(wide.calls, []);
});

test("revealIn scrolls the box horizontally for each block", () => {
  // A box 200 wide scrolled to 300 shows 300..500; the child is 50 wide at 600.
  const cases = [
    ["start", 600],
    ["center", 525],
    ["end", 450],
    ["nearest", 450],
  ];
  for (const [block, left] of cases) {
    const box = fakeBox({ scrollLeft: 300 });
    revealIn(box, fakeChild(box, { left: 600 }), block, "auto", "x");
    assert.deepEqual(box.calls, [{ left, behavior: "auto" }], block);
  }
  // Left of the view, nearest brings its left edge to the box's left edge.
  const box = fakeBox({ scrollLeft: 300 });
  revealIn(box, fakeChild(box, { left: 100 }), "nearest", "auto", "x");
  assert.deepEqual(box.calls, [{ left: 100, behavior: "auto" }]);
});

test("revealIn never asks the box for a negative scroll on either axis", () => {
  const box = fakeBox({ scrollTop: 0, scrollLeft: 0 });
  revealIn(box, fakeChild(box, { top: 10, left: 10 }), "end");
  revealIn(box, fakeChild(box, { top: 10, left: 10 }), "center", "auto", "x");
  assert.deepEqual(box.calls, [
    { top: 0, behavior: "auto" },
    { left: 0, behavior: "auto" },
  ]);
});

test("following the end: near it follows, a move up lets go, a move down keeps", async () => {
  const { endGap, followAfterScroll, FOLLOW_SLACK } = await import("./scroll.ts");
  const at = (scrollTop, scrollHeight = 2000, clientHeight = 500) => ({ scrollTop, scrollHeight, clientHeight });
  assert.equal(endGap(at(1500)), 0);
  assert.equal(endGap(at(1271)), 229);
  assert.equal(endGap(at(1900)), 0, "never negative");
  // At or near the end it always follows.
  assert.equal(followAfterScroll(false, 0, at(1500)), true);
  assert.equal(followAfterScroll(false, 0, at(1500 - FOLLOW_SLACK + 1)), true);
  // The reader scrolling up to read lets go.
  assert.equal(followAfterScroll(true, 1500, at(1200)), false);
  // A smooth scroll on its way down, or content grown under a pinned
  // reader (TASK-171: 229 px short on open), keeps following.
  assert.equal(followAfterScroll(true, 300, at(600)), true);
  assert.equal(followAfterScroll(true, 1271, at(1271)), true);
  // Scrolling down short of the end does not catch on again by itself.
  assert.equal(followAfterScroll(false, 300, at(600)), false);
});
