import assert from "node:assert/strict";
import { test } from "node:test";
import { holdsWords, inTextField, nameWords, nextIndex, openerName, opensRow, rowKey } from "./list-row-state.ts";

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

test("nameWords are the letters and digits, in lowercase, without what is in brackets", () => {
  assert.deepEqual(nameWords("Self-improvement stopped. May it carry on?"), ["self", "improvement", "stopped", "may", "it", "carry", "on"]);
  assert.deepEqual(nameWords("  Version   the client API\nwith a v2 package "), ["version", "the", "client", "api", "with", "a", "v2", "package"]);
  assert.deepEqual(nameWords("Deploy \u{1F680} now!"), ["deploy", "now"]);
  assert.deepEqual(nameWords("cinder \u00b7 Await the lookup"), ["cinder", "await", "the", "lookup"]);
  assert.deepEqual(nameWords("Extra machine for 8 hours (Standard_D4s_v5)"), ["extra", "machine", "for", "8", "hours"]);
  assert.deepEqual(nameWords("Search (Command K)"), ["search"]);
  assert.deepEqual(nameWords(""), []);
});

test("a name holds a control's words when they stand side by side in it, in order", () => {
  assert.equal(holdsWords("Decision waiting: Version the client API", "Version the client API"), true);
  assert.equal(holdsWords("Runner off: Agents can't start work", "Runner off Agents can't start work"), true, "punctuation is only a gap");
  assert.equal(holdsWords("For you, 10 waiting", "For you 10"), true);
  assert.equal(holdsWords("For you, 10 waiting", "10 For you"), false, "the order counts");
  assert.equal(holdsWords("Suggested application", "app"), false, "a word, not a run of letters");
  assert.equal(holdsWords("cinder is working on the feed", "cinder the feed"), false, "side by side, not scattered");
  assert.equal(holdsWords("Anything", ""), true);
  assert.equal(holdsWords("", "Shown"), false);
});

test("a label that holds the title's words names the row's opener", () => {
  assert.equal(openerName("Decision waiting: Version the client API", "Version the client API"), "Decision waiting: Version the client API");
  assert.equal(openerName("Proposal: agreed: second reviewer for checkout", "Agreed: second reviewer for checkout"), "Proposal: agreed: second reviewer for checkout", "case and punctuation do not count");
  assert.equal(openerName("demo-app: 4 things wait for you", "demo-app"), "demo-app: 4 things wait for you");
  assert.equal(openerName("  Thread release-notes  ", "release-notes"), "Thread release-notes");
});

test("a label without the title's words gets the title first, so the name holds what is shown", () => {
  assert.equal(openerName("Self-improvement paused on demo-app: Evolution stopped; owner review needed", "Self-improvement stopped. May it carry on?"), "Self-improvement stopped. May it carry on? Self-improvement paused on demo-app: Evolution stopped; owner review needed");
  assert.equal(openerName("Machine request: fuzzing the parser overnight", "Extra machine for 8 hours"), "Extra machine for 8 hours. Machine request: fuzzing the parser overnight");
  const name = openerName("Suggested work: Cache the feed. Show the details", "Suggested work, waiting for you");
  assert.ok(holdsWords(name, "Suggested work, waiting for you"), name);
});

test("no label leaves the title to name the opener; a title of several nodes keeps the label", () => {
  assert.equal(openerName(undefined, "Cache the feed"), undefined);
  assert.equal(openerName("", "Cache the feed"), undefined);
  assert.equal(openerName("   ", "Cache the feed"), undefined);
  assert.equal(openerName("cinder is working on the feed", undefined), "cinder is working on the feed");
  assert.equal(openerName("Open the row", ""), "Open the row");
});
