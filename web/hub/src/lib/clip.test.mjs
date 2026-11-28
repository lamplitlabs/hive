import assert from "node:assert/strict";
import { test } from "node:test";
import { clippedTitle, isClipped } from "./clip.ts";

test("isClipped ignores a pixel of rounding", () => {
  assert.equal(isClipped(208, 208), false);
  assert.equal(isClipped(209, 208), false);
  assert.equal(isClipped(336, 208), true);
});

test("clippedTitle gives the whole text back only when it is cut", () => {
  assert.equal(clippedTitle("Clone: this machine copies projects to work on them", 336, 208), "Clone: this machine copies projects to work on them");
  assert.equal(clippedTitle("Off: updates are mine", 150, 208), undefined);
  assert.equal(clippedTitle("  I'm away:\n  Hive adds tools  ", 293, 208), "I'm away: Hive adds tools");
  assert.equal(clippedTitle("", 300, 10), undefined);
  assert.equal(clippedTitle(null, 300, 10), undefined);
});
