import assert from "node:assert/strict";
import { test } from "node:test";
import {
  addTargets,
  countWords,
  dayWords,
  fileHint,
  fileTitle,
  matchEntries,
  orderFiles,
  summaryLine,
  textRoom,
} from "./memory.ts";

const entry = (day, text) => ({ day, text, line: day ? "[" + day + "] " + text : text });
const file = (key, kind, project, entries = [], over = {}) => ({
  key,
  kind,
  project,
  scope: kind === "user" ? "user" : "project",
  path: "~/.hive/memory/" + key + ".md",
  exists: entries.length > 0,
  size: entries.length * 40,
  updated_at: entries.length ? "2026-11-01T10:00:00Z" : "",
  entries,
  ...over,
});

const user = file("user", "user", "", [entry("2026-10-01", "prefers short replies")]);
const shared = file("conversations", "conversations", "");
const demo = file("demo-app", "project", "demo-app", [
  entry("2026-10-20", "tests need cargo test -p app"),
  entry("", "hand written, no date"),
  entry("2026-10-28", "the API lives in crates/api"),
]);
const quiet = file("quiet-app", "project", "quiet-app");
const orphan = file("old-project", "project", "", [entry("2026-03-04", "gone but remembered")]);

test("files are named for a person and explained in one line", () => {
  assert.equal(fileTitle(user), "You, in every project");
  assert.equal(fileTitle(shared), "Sessions without a project");
  assert.equal(fileTitle(demo), "demo-app");
  assert.equal(fileTitle(orphan), "old-project");
  assert.match(fileHint(user), /every run on every project/);
  assert.match(fileHint(shared), /not attached to a project/);
  assert.match(fileHint(demo), /this project/);
  assert.match(fileHint(orphan), /No project goes by this key/);
});

test("the order is you, conversations, projects with entries, quiet projects, then orphans", () => {
  const all = orderFiles([orphan, quiet, demo, shared, user], false).map((f) => f.key);
  assert.deepEqual(all, ["user", "conversations", "demo-app", "quiet-app", "old-project"]);
  const some = orderFiles([orphan, quiet, demo, shared, user], true).map((f) => f.key);
  assert.deepEqual(some, ["user", "demo-app", "old-project"], "empty files drop out but the user's stays");
});

test("search needs every word, ignores case, and newest first flips the file order", () => {
  assert.equal(matchEntries(demo, "", false)[0].text, "tests need cargo test -p app");
  assert.equal(matchEntries(demo, "", true)[0].text, "the API lives in crates/api");
  assert.deepEqual(
    matchEntries(demo, "CARGO app", false).map((e) => e.text),
    ["tests need cargo test -p app"],
  );
  assert.deepEqual(matchEntries(demo, "2026-10-28", false).map((e) => e.text), ["the API lives in crates/api"], "the day is searchable");
  assert.deepEqual(matchEntries(demo, "nothing here", false), []);
});

test("counts and the summary read as sentences", () => {
  assert.equal(countWords(0), "nothing yet");
  assert.equal(countWords(1), "1 memory");
  assert.equal(countWords(1234), "1,234 memories");
  const files = [user, shared, demo, quiet, orphan];
  assert.equal(
    summaryLine({ on: true, total: 5, files, dir: "~/.hive/memory" }),
    "5 memories across 3 files, read into every run's prompt in ~/.hive/memory.",
  );
  assert.match(summaryLine({ on: true, total: 0, files: [shared, quiet], dir: "~/.hive/memory" }), /^Nothing remembered yet/);
  assert.match(summaryLine({ on: false, total: 5, files, dir: "~/.hive/memory" }), /^Memory is off.*5 memories still on disk/);
  assert.match(summaryLine({ on: false, total: 0, files: [], dir: "~/.hive/memory" }), /^Memory is off.*Nothing is stored/);
});

test("the Add box offers the user's file first and skips orphans that are gone from disk", () => {
  const gone = file("vanished", "project", "", [], { exists: false });
  const targets = addTargets([orphan, gone, quiet, demo, shared, user]);
  assert.deepEqual(
    targets.map((t) => t.value),
    ["user", "conversations", "demo-app", "quiet-app", "old-project"],
  );
  assert.equal(targets[0].label, "You, in every project");
});

test("the text limit counts the one-line form the hub keeps", () => {
  assert.deepEqual(textRoom("  two   words ", 600), { left: 591, over: false });
  assert.deepEqual(textRoom("x".repeat(601), 600), { left: -1, over: true });
  assert.equal(textRoom("", 600).left, 600);
});

test("a day reads as a date and a missing one as nothing", () => {
  assert.equal(dayWords("2026-01-02"), "Jan 2, 2026");
  assert.equal(dayWords(""), "");
  assert.equal(dayWords("not-a-day"), "");
});
