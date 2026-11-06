import assert from "node:assert/strict";
import { test } from "node:test";
import { CHORD_MS, GO, SHORTCUTS, SHORTCUT_GROUPS, groupedShortcuts, isCommandKey, keyChips, matchChord } from "./shortcuts.ts";

test("the shortcut list names every documented shortcut once", () => {
  const keys = SHORTCUTS.map((s) => s.keys);
  for (const k of ["G H", "G F", "G S", "G T", "⌘K", "⌘J", "?", "J / K", "Enter", "Y / N", "Esc"]) assert.ok(keys.includes(k), k);
  assert.equal(new Set(keys).size, keys.length, "no duplicates");
  for (const s of SHORTCUTS) {
    assert.ok(s.what, s.keys + " says what");
    assert.ok(s.where, s.keys + " says where");
  }
});

test("groupedShortcuts groups by where, in the fixed order, dropping empty groups", () => {
  const g = groupedShortcuts();
  assert.deepEqual(g.map((x) => x.where), [...SHORTCUT_GROUPS]);
  assert.equal(g[0].items.length, 4, "four G-chords");
  const custom = groupedShortcuts([{ keys: "X", what: "x", where: "Elsewhere" }, { keys: "Enter", what: "Open", where: "In a list" }]);
  assert.deepEqual(custom.map((x) => x.where), ["In a list", "Elsewhere"]);
});

test("keyChips splits chords, alternatives and the command key", () => {
  assert.deepEqual(keyChips("G H"), [{ kind: "key", text: "G" }, { kind: "word", text: "then" }, { kind: "key", text: "H" }]);
  assert.deepEqual(keyChips("J / K"), [{ kind: "key", text: "J" }, { kind: "word", text: "or" }, { kind: "key", text: "K" }]);
  assert.deepEqual(keyChips("⌘K"), [{ kind: "key", text: "⌘" }, { kind: "key", text: "K" }]);
  assert.deepEqual(keyChips("Esc"), [{ kind: "key", text: "Esc" }]);
  assert.deepEqual(keyChips("?"), [{ kind: "key", text: "?" }]);
});

test("G then H/F/S/T goes to Home, For you, Sessions, Tasks", () => {
  assert.deepEqual(matchChord(null, "g", false, false), { pending: "g" });
  assert.deepEqual(matchChord(null, "G", false, false), { pending: "g" }, "shift makes no difference");
  assert.deepEqual(matchChord("g", "h", false, false), { go: "/" });
  assert.deepEqual(matchChord("g", "f", false, false), { go: "/for-you/" });
  assert.deepEqual(matchChord("g", "s", false, false), { go: "/chat/" });
  assert.deepEqual(matchChord("g", "t", false, false), { go: "/tasks/" });
  assert.deepEqual(matchChord("g", "T", false, false), { go: "/tasks/" });
  assert.equal(matchChord("g", "x", false, false), null, "an unknown second key drops the chord");
  assert.equal(matchChord("g", "g", false, false), null, "G G is nothing");
  assert.equal(GO.h, "/");
});

test("⌘K searches and ⌘J asks, even inside a field", () => {
  assert.deepEqual(matchChord(null, "k", true, false), { open: "search" });
  assert.deepEqual(matchChord(null, "j", true, false), { open: "ask" });
  assert.deepEqual(matchChord(null, "K", true, true), { open: "search" });
  assert.deepEqual(matchChord(null, "j", true, true), { open: "ask" });
  assert.deepEqual(matchChord("g", "k", true, false), { open: "search" }, "the command key wins over a pending chord");
  assert.equal(matchChord(null, "h", true, false), null, "⌘H is the browser's");
});

test("? opens the help, but not when typing", () => {
  assert.deepEqual(matchChord(null, "?", false, false), { open: "help" });
  assert.equal(matchChord(null, "?", false, true), null);
});

test("letters typed into a field never navigate", () => {
  assert.equal(matchChord(null, "g", false, true), null);
  assert.equal(matchChord("g", "h", false, true), null);
  assert.equal(matchChord(null, "h", false, false), null, "a bare H does nothing");
  assert.equal(matchChord(null, "Enter", false, false), null);
  assert.equal(matchChord(null, "Escape", false, false), null);
});

test("the chord window is one second", () => {
  assert.equal(CHORD_MS, 1000);
});

test("isCommandKey is ⌘ on a Mac and Ctrl elsewhere", () => {
  assert.equal(isCommandKey({ metaKey: true, ctrlKey: false }, "MacIntel"), true);
  assert.equal(isCommandKey({ metaKey: false, ctrlKey: true }, "MacIntel"), false);
  assert.equal(isCommandKey({ metaKey: false, ctrlKey: true }, "Win32"), true);
  assert.equal(isCommandKey({ metaKey: true, ctrlKey: false }, "Linux x86_64"), false);
  assert.equal(isCommandKey({ metaKey: true, ctrlKey: false }, "iPhone"), true);
});
