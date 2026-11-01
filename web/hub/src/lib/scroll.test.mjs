import assert from "node:assert/strict";
import { test } from "node:test";
import { revealOffset } from "./scroll.ts";

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
