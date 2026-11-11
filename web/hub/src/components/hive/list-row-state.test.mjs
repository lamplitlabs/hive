import assert from "node:assert/strict";
import { test } from "node:test";
import { inTextField, nextIndex, opensRow, rowKey } from "./list-row-state.ts";

test("rowKey reads J/K, arrows, Home/End, Enter and Escape", () => {
  assert.equal(rowKey("j"), "next");
  assert.equal(rowKey("J"), "next");
  assert.equal(rowKey("ArrowDown"), "next");
  assert.equal(rowKey("k"), "prev");
  assert.equal(rowKey("ArrowUp"), "prev");
  assert.equal(rowKey("Home"), "first");
  assert.equal(rowKey("End"), "last");
  assert.equal(rowKey("Enter"), "open");
  assert.equal(rowKey("Escape"), "blur");
  assert.equal(rowKey("a"), null);
  assert.equal(rowKey("Tab"), null);
});

test("rowKey leaves modified keys to the browser", () => {
  assert.equal(rowKey("j", { meta: true }), null);
  assert.equal(rowKey("k", { ctrl: true }), null);
  assert.equal(rowKey("ArrowDown", { alt: true }), null);
  assert.equal(rowKey("j", { meta: false }), "next");
});

test("nextIndex moves down and up and stops at the ends", () => {
  assert.equal(nextIndex("j", 0, 5), 1);
  assert.equal(nextIndex("j", 3, 5), 4);
  assert.equal(nextIndex("j", 4, 5), 4, "no wrap past the last");
  assert.equal(nextIndex("k", 4, 5), 3);
  assert.equal(nextIndex("k", 0, 5), 0, "no wrap before the first");
  assert.equal(nextIndex("ArrowDown", 1, 5), 2);
  assert.equal(nextIndex("ArrowUp", 1, 5), 0);
});

test("nextIndex enters the list from nowhere: J at the top, K at the bottom", () => {
  assert.equal(nextIndex("j", -1, 5), 0);
  assert.equal(nextIndex("k", -1, 5), 4);
  assert.equal(nextIndex("Home", -1, 5), 0);
  assert.equal(nextIndex("End", -1, 5), 4);
});

test("nextIndex jumps with Home/End and ignores other keys", () => {
  assert.equal(nextIndex("Home", 3, 5), 0);
  assert.equal(nextIndex("End", 1, 5), 4);
  assert.equal(nextIndex("Enter", 2, 5), 2);
  assert.equal(nextIndex("Escape", 2, 5), 2);
  assert.equal(nextIndex("x", 2, 5), 2);
});

test("nextIndex on an empty list is -1", () => {
  assert.equal(nextIndex("j", -1, 0), -1);
  assert.equal(nextIndex("k", 0, 0), -1);
});

test("opensRow accepts Enter and Space", () => {
  assert.equal(opensRow("Enter"), true);
  assert.equal(opensRow(" "), true);
  assert.equal(opensRow("Spacebar"), true);
  assert.equal(opensRow("j"), false);
});

test("inTextField spots inputs, textareas, selects and contenteditable", () => {
  assert.equal(inTextField("INPUT", false), true);
  assert.equal(inTextField("textarea", null), true);
  assert.equal(inTextField("select", undefined), true);
  assert.equal(inTextField("div", true), true);
  assert.equal(inTextField("div", "true"), true);
  assert.equal(inTextField("div", "plaintext-only"), true);
  assert.equal(inTextField("div", false), false);
  assert.equal(inTextField("div", "false"), false);
  assert.equal(inTextField("button", false), false);
  assert.equal(inTextField(null, null), false);
});
