import assert from "node:assert/strict";
import { test } from "node:test";

const { chosenFile, emptyWords, fileChoices, fileNote, fileParam, shownEntries, sourceOf, sourceWords, undoNote } = await import("./memory-state.ts");

const entry = (day, text) => ({ day, text, line: day ? "[" + day + "] " + text : text });
const file = (key, kind, project, entries = [], extra = {}) => ({
  key,
  kind,
  project,
  path: "~/.hive/memory/" + key + ".md",
  exists: entries.length > 0,
  size: entries.length * 40,
  updated_at: entries.length ? "2026-11-05T10:00:00Z" : "",
  entries,
  ...extra,
});

const FILES = [
  file("hive", "project", "hive", []),
  file("demo-app", "project", "demo-app", [entry("2026-10-01", "Uses pnpm"), entry("", "Hand-written line")]),
  file("old-shop", "project", "", [entry("2026-01-01", "Gone project")]),
  file("conversations", "conversations", "", [entry("2026-10-20", "From a session")]),
  file("user", "user", "", [entry("2026-09-01", "Short replies")]),
  file("alpha", "project", "alpha", [entry("2026-10-02", "Alpha fact")]),
];

test("fileParam names the user's file 'user', the sessions file 'none' and a project by its key", () => {
  assert.equal(fileParam({ key: "user", kind: "user" }), "user");
  assert.equal(fileParam({ key: "conversations", kind: "conversations" }), "none");
  assert.equal(fileParam({ key: "demo-app", kind: "project" }), "demo-app");
});

test("chosenFile resolves ?file=, falls back to the older ?key=, then to the user's file", () => {
  assert.equal(chosenFile(FILES, "none").key, "conversations");
  assert.equal(chosenFile(FILES, "demo-app").key, "demo-app");
  assert.equal(chosenFile(FILES, "user").key, "user");
  assert.equal(chosenFile(FILES, null, "conversations").key, "conversations");
  assert.equal(chosenFile(FILES, null, null).key, "user");
  assert.equal(chosenFile(FILES, "no-such-file").key, "user");
  assert.equal(chosenFile([], "user"), null);
  const noUser = FILES.filter((f) => f.kind !== "user");
  assert.equal(chosenFile(noUser, "nothing").key, "hive");
});

test("fileChoices lists You, then the sessions file, then projects (with entries first, alphabetical), then orphans", () => {
  const c = fileChoices(FILES);
  assert.deepEqual(
    c.map((x) => [x.title, x.param, x.icon, x.count]),
    [
      ["You", "user", "user", 1],
      ["Sessions without a project", "none", "sessions", 1],
      ["alpha", "alpha", "project", 1],
      ["demo-app", "demo-app", "project", 2],
      ["hive", "hive", "project", 0],
      ["old-shop", "old-shop", "orphan", 1],
    ],
  );
});

test("sourceOf: a line kept from this page or written by hand is yours, the sessions file's lines are a session's, the rest an agent's", () => {
  const project = { kind: "project" };
  assert.equal(sourceOf(entry("2026-10-01", "x"), project), "agent");
  assert.equal(sourceOf(entry("", "x"), project), "you");
  assert.equal(sourceOf(entry("2026-10-01", "x"), { kind: "conversations" }), "session");
  assert.equal(sourceOf(entry("2026-10-01", "x"), { kind: "user" }), "agent");
  const mine = new Set(["[2026-10-01] x"]);
  assert.equal(sourceOf(entry("2026-10-01", "x"), project, mine), "you");
  assert.equal(sourceOf(entry("2026-10-01", "x"), { kind: "conversations" }, mine), "you");
  assert.equal(sourceWords("you"), "Kept by you");
  assert.equal(sourceWords("agent"), "Kept by an agent");
  assert.equal(sourceWords("session"), "Kept by a session");
});

test("shownEntries puts the newest first and narrows by every word of the search", () => {
  const f = file("demo-app", "project", "demo-app", [entry("2026-10-01", "Uses pnpm v11"), entry("2026-10-02", "Prefers short replies"), entry("2026-10-03", "pnpm overrides live in the workspace file")]);
  assert.deepEqual(
    shownEntries(f, "").map((e) => e.day),
    ["2026-10-03", "2026-10-02", "2026-10-01"],
  );
  assert.deepEqual(
    shownEntries(f, "pnpm").map((e) => e.day),
    ["2026-10-03", "2026-10-01"],
  );
  assert.deepEqual(
    shownEntries(f, "PNPM workspace").map((e) => e.day),
    ["2026-10-03"],
  );
  assert.deepEqual(shownEntries(f, "2026-10-02").map((e) => e.day), ["2026-10-02"]);
});

test("fileNote counts the entries and names the file, or says there is no file yet", () => {
  assert.equal(fileNote({ exists: true, entries: [1, 2], path: "~/.hive/memory/demo-app.md" }), "2 memories in ~/.hive/memory/demo-app.md");
  assert.equal(fileNote({ exists: true, entries: [1], path: "p" }), "1 memory in p");
  assert.equal(fileNote({ exists: false, entries: [], path: "p" }), "Nothing yet; no file yet");
});

test("emptyWords and undoNote speak to the file and the day", () => {
  assert.match(emptyWords({ kind: "user" }), /every project/);
  assert.match(emptyWords({ kind: "conversations" }), /Sessions/);
  assert.match(emptyWords({ kind: "project" }), /Agents keep a fact/);
  assert.equal(undoNote({ day: "2026-10-01" }, "2026-11-06"), "Back in the file, dated today instead of 2026-10-01.");
  assert.equal(undoNote({ day: "2026-11-06" }, "2026-11-06"), "Back in the file.");
  assert.equal(undoNote({ day: "" }, "2026-11-06"), "Back in the file.");
});
