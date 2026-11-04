import assert from "node:assert/strict";
import { test } from "node:test";
import { GROUP_ORDER, entryGroup, entryId, foldedEntries, jobWaitingToLand, listEntries, sessionWords, stepWords, taskFinished, taskGroup, taskGroupOf, taskMood, taskMoodOf, taskTitleOf } from "./task-groups.ts";

const session = (over = {}) => ({ job: "job-1", kind: "feature", title: "Build it", job_status: "queued", agent: "", session: "", status: "", started_at: "", updated_at: "", ended_at: "", cost: 0, steps: 0, ...over });
const task = (over = {}) => ({ id: "TASK-001", project: "one", title: "Ship it", status: "in_progress", priority: 10, lead: "one-lead", created_by: "owner", created_at: "", updated_at: "", last_at: "", messages: 0, last_message_at: "", last_author: "", needs_you: false, sessions: [], ...over });

test("a task's word puts the owner's turn first, then what the agents do", () => {
  assert.equal(taskMood(task({ status: "review", needs_you: true })).word, "Asked you");
  assert.equal(taskMood(task({ status: "review", needs_you: true })).tone, "wait");
  assert.equal(taskMood(task({ sessions: [session({ job_status: "leased", status: "running" })] })).word, "Working");
  assert.equal(taskMood(task({ sessions: [session({ job_status: "leased" })] })).word, "Working");
  assert.equal(taskMood(task({ sessions: [session({ job_status: "done", status: "ended" })] })).word, "In progress");
  assert.equal(taskMood(task({ status: "open", lead: "" })).word, "New");
  assert.equal(taskMood(task({ status: "open" })).word, "Lead agent starting");
  assert.equal(taskMood(task({ status: "planned" })).word, "Planned");
  assert.equal(taskMood(task({ status: "done", sessions: [session({ status: "running" })] })).word, "Done");
  assert.equal(taskMood(task({ status: "cancelled" })).word, "Cancelled");
  assert.equal(taskFinished(task({ status: "done" })), true);
  assert.equal(taskFinished(task({ status: "cancelled" })), true);
  assert.equal(taskFinished(task({ status: "review" })), false);
});

test("the task's own page reads the same mood from its status, lead, review and workers", () => {
  // The list (GET /chat) and the task view (GET /tasks/{id}) hold different
  // rows; both go through taskMoodOf, so one task never has two words.
  const state = (over = {}) => ({ status: "in_progress", lead: "one-lead", needs_you: false, working: false, ...over });
  assert.equal(taskMoodOf(state({ status: "review", needs_you: true, working: true })).word, "Asked you");
  assert.equal(taskMoodOf(state({ working: true })).word, "Working");
  assert.equal(taskMoodOf(state({ working: true })).pip, "busy");
  assert.equal(taskMoodOf(state({ status: "done", working: true })).word, "Done");
  assert.equal(taskMoodOf(state({ status: "open", lead: "" })).word, "New");
  assert.deepEqual(taskMoodOf(state()), taskMood(task()));
  assert.deepEqual(taskMoodOf(state({ working: true })), taskMood(task({ sessions: [session({ job_status: "leased" })] })));
});

test("a step's word comes from the job unless an agent is at it right now", () => {
  assert.deepEqual(stepWords("queued", false), { word: "Waiting", tone: "neutral" });
  assert.deepEqual(stepWords("leased", false), { word: "Starting", tone: "run" });
  assert.deepEqual(stepWords("leased", true), { word: "Working", tone: "run" });
  assert.deepEqual(stepWords("done", false), { word: "Done", tone: "ok" });
  assert.equal(stepWords("needs-review", false).word, "Needs review");
});

test("a task takes the same section a conversation would", () => {
  assert.equal(taskGroup(task({ status: "review", needs_you: true })), "working");
  assert.equal(taskGroup(task({ sessions: [session({ job_status: "leased" })] })), "working");
  assert.equal(taskGroup(task({ status: "planned" })), "open");
  assert.equal(taskGroup(task({ status: "open", lead: "" })), "open");
  assert.equal(taskGroup(task({ status: "in_progress", sessions: [session({ job_status: "done", status: "ended" })] })), "open");
  assert.equal(taskGroup(task({ status: "done" })), "finished");
  assert.equal(taskGroup(task({ status: "cancelled" })), "finished");
});

test("work done but not on main yet waits to land: a step by its branch, a task by its steps, a conversation by the hub's answer", () => {
  // A done job keeps its branch until the landing clears it.
  assert.equal(jobWaitingToLand("done", "pulse/iris/job-1"), true);
  assert.equal(jobWaitingToLand("done", ""), false);
  assert.equal(jobWaitingToLand("done", undefined), false);
  assert.equal(jobWaitingToLand("failed", "pulse/iris/job-1"), false);
  assert.equal(jobWaitingToLand("leased", "pulse/iris/job-1"), false);
  const landing = session({ job_status: "done", status: "ended", branch: "pulse/iris/job-1" });
  assert.deepEqual(sessionWords(landing), { word: "Waiting to land", tone: "wait" });
  assert.deepEqual(stepWords("done", false, "pulse/iris/job-1"), { word: "Waiting to land", tone: "wait" });
  assert.deepEqual(stepWords("done", true, "pulse/iris/job-1"), { word: "Working", tone: "run" }, "a session at it outranks the branch");
  assert.deepEqual(stepWords("done", false, ""), { word: "Done", tone: "ok" });
  // The task: the section sits between working and open; a step waiting to
  // land outranks Done (the task is not on main yet) but not an agent at
  // another step, nor the owner's turn.
  assert.deepEqual(GROUP_ORDER, ["working", "landing", "open", "finished", "archived"]);
  assert.equal(taskMood(task({ sessions: [landing] })).word, "Waiting to land");
  assert.equal(taskGroup(task({ sessions: [landing] })), "landing");
  assert.equal(taskMood(task({ status: "done", sessions: [landing] })).word, "Waiting to land");
  assert.equal(taskGroup(task({ status: "done", sessions: [landing] })), "landing");
  assert.equal(taskGroup(task({ status: "cancelled", sessions: [landing] })), "landing");
  assert.equal(taskMood(task({ sessions: [landing, session({ job_status: "leased", status: "running" })] })).word, "Working");
  assert.equal(taskGroup(task({ sessions: [landing, session({ job_status: "leased" })] })), "working");
  assert.equal(taskMood(task({ status: "review", needs_you: true, sessions: [landing] })).word, "Asked you");
  assert.equal(taskGroup(task({ status: "review", needs_you: true, sessions: [landing] })), "working");
  assert.equal(taskGroup(task({ status: "done", sessions: [session({ job_status: "done", branch: "" })] })), "finished");
  // The task's own page reads the same from its jobs.
  const state = (over = {}) => ({ status: "in_progress", lead: "one-lead", needs_you: false, working: false, landing: true, ...over });
  assert.equal(taskMoodOf(state()).word, "Waiting to land");
  assert.equal(taskGroupOf(state()), "landing");
  assert.equal(taskMoodOf(state({ status: "done" })).word, "Waiting to land");
  assert.equal(taskMoodOf(state({ working: true })).word, "Working");
  assert.equal(taskMoodOf(state({ status: "done", working: true })).word, "Waiting to land", "a done task's running session does not hide the unlanded step");
  assert.equal(taskGroupOf(state({ status: "done", working: true })), "landing");
  assert.deepEqual(taskMoodOf(state()), taskMood(task({ sessions: [landing] })));
  // A conversation: the hub's answer puts a finished or idle one in the
  // section; one being worked on, asked about or put away keeps its own.
  assert.equal(groupOf({ id: "c", status: "closed", waiting_to_land: true }), "landing");
  assert.equal(groupOf({ id: "c", status: "idle", waiting_to_land: true }), "landing");
  assert.equal(groupOf({ id: "c", status: "ended", waiting_to_land: true }), "landing");
  assert.equal(groupOf({ id: "c", status: "closed", waiting_to_land: false }), "finished");
  assert.equal(groupOf({ id: "c", status: "closed" }), "finished");
  assert.equal(groupOf({ id: "c", status: "idle", waiting_to_land: true, live: { status: "running" } }), "working");
  assert.equal(groupOf({ id: "c", status: "waiting", waiting_to_land: true }), "working");
  assert.equal(groupOf({ id: "c", status: "archived", waiting_to_land: true }), "archived");
  // A family with a finished parent and a sub-conversation waiting to land
  // is listed under Waiting to land.
  const rows = [
    { id: "p", status: "closed", created_at: "2026-10-31T10:00:00Z" },
    { id: "k", status: "closed", parent: "p", created_at: "2026-10-31T10:01:00Z", waiting_to_land: true },
  ];
  const entries = listEntries(rows, []);
  assert.equal(entries.length, 1);
  assert.equal(entryGroup(entries[0]), "landing");
});

test("the one list orders tasks and conversations by what moved last, a sub-conversation counting for its parent", () => {
  const rows = [
    { id: "c-old", status: "idle", parent: "", created_at: "2026-10-18T08:00:00Z", last_at: "2026-10-18T08:30:00Z" },
    { id: "c-kid", status: "running", parent: "c-old", created_at: "2026-10-18T09:00:00Z", last_at: "2026-10-18T12:30:00Z" },
    { id: "c-new", status: "closed", parent: "", created_at: "2026-10-18T11:00:00Z", last_at: "2026-10-18T11:00:00Z" },
  ];
  const tasks = [task({ id: "TASK-001", status: "planned", created_at: "2026-10-18T10:00:00Z", last_at: "2026-10-18T12:00:00Z" })];
  const entries = listEntries(rows, tasks);
  assert.deepEqual(entries.map(entryId), ["c-old", "TASK-001", "c-new"]);
  assert.deepEqual(entries.map(entryGroup), ["working", "open", "finished"]);
  assert.equal(entries[0].kind, "conversation");
  assert.deepEqual(entries[0].family.children.map((c) => c.id), ["c-kid"]);
});

test("a step's word follows the job until its session runs", () => {
  assert.deepEqual(sessionWords(session()), { word: "Waiting", tone: "neutral" });
  assert.deepEqual(sessionWords(session({ job_status: "leased" })), { word: "Starting", tone: "run" });
  assert.deepEqual(sessionWords(session({ job_status: "leased", status: "running" })), { word: "Working", tone: "run" });
  assert.deepEqual(sessionWords(session({ job_status: "blocked" })), { word: "Blocked", tone: "wait" });
  assert.deepEqual(sessionWords(session({ job_status: "done", status: "ended" })), { word: "Done", tone: "ok" });
  assert.deepEqual(sessionWords(session({ job_status: "failed", status: "provider_error" })), { word: "Failed", tone: "bad" });
  assert.deepEqual(sessionWords(session({ job_status: "cancelled" })), { word: "Cancelled", tone: "neutral" });
  assert.equal(sessionWords(session({ job_status: "odd_state" })).word, "Odd state");
});

test("the task's title is the brief's first real line, cut at a word when long", () => {
  assert.equal(taskTitleOf("Add a dark mode toggle\n\nAnd tests."), "Add a dark mode toggle");
  assert.equal(taskTitleOf("\n\n## Settings page\n- toggle"), "Settings page");
  assert.equal(taskTitleOf("1. Ship the login\n2. Then 2FA"), "Ship the login");
  assert.equal(taskTitleOf("2FA for login"), "2FA for login");
  assert.equal(taskTitleOf("   "), "");
  const long = "word ".repeat(40).trim();
  const title = taskTitleOf(long);
  assert.ok(title.endsWith("\u2026"), title);
  assert.ok(title.length <= 121, String(title.length));
  assert.ok(!title.includes("word\u2026") || title.endsWith("word\u2026"));
  assert.equal(taskTitleOf("a".repeat(200)), "a".repeat(120) + "\u2026");
});

import { familyGroup, families, groupOf, reportVerb } from "./task-groups.ts";

const conv = (id, over = {}) => ({ id, status: "idle", parent: "", created_at: "2026-10-18T10:00:00Z", ...over });

test("sub-conversations stand under their parent, orphans on their own", () => {
  const rows = [
    conv("c-3", { parent: "c-1", created_at: "2026-10-18T12:00:00Z" }),
    conv("c-1"),
    conv("c-2", { parent: "c-1", created_at: "2026-10-18T11:00:00Z" }),
    conv("c-4", { parent: "c-gone" }),
    conv("c-5", { parent: "c-5" }),
  ];
  const fams = families(rows);
  assert.deepEqual(
    fams.map((f) => [f.c.id, f.children.map((c) => c.id)]),
    [
      ["c-1", ["c-2", "c-3"]],
      ["c-4", []],
      ["c-5", []],
    ],
  );
});

test("a family sits in its most pressing member's section", () => {
  assert.equal(groupOf(conv("a", { status: "closed" })), "finished");
  assert.equal(groupOf(conv("a", { status: "closed", live: { status: "running" } })), "working");
  assert.equal(groupOf(conv("a", { status: "archived" })), "archived");
  assert.equal(groupOf(conv("a", { status: "ended" })), "open");
  const [fam] = families([conv("p", { status: "closed" }), conv("k", { parent: "p", status: "waiting" })]);
  assert.equal(familyGroup(fam), "working");
  const [quiet] = families([conv("p", { status: "closed" }), conv("k", { parent: "p", status: "archived" })]);
  assert.equal(familyGroup(quiet), "finished");
  const [alone] = families([conv("p", { status: "archived" })]);
  assert.equal(familyGroup(alone), "archived");
});

test("a folded finished section keeps only the open entry, other sections stay whole", () => {
  const rows = [
    conv("c-1", { status: "closed" }),
    conv("c-2", { status: "closed" }),
    conv("c-3", { parent: "c-2", status: "closed" }),
  ];
  const entries = listEntries(rows, [task({ id: "t-1", status: "done", created_at: "2026-10-18T09:00:00Z" })]);
  assert.deepEqual(entries.map(entryId), ["c-1", "c-2", "t-1"]);
  assert.deepEqual(foldedEntries(entries, "finished", true, "").map(entryId), []);
  assert.deepEqual(foldedEntries(entries, "finished", true, "c-1").map(entryId), ["c-1"]);
  assert.deepEqual(foldedEntries(entries, "finished", true, "c-3").map(entryId), ["c-2"]);
  assert.deepEqual(foldedEntries(entries, "finished", true, "t-1").map(entryId), ["t-1"]);
  assert.equal(foldedEntries(entries, "finished", false, ""), entries);
  assert.equal(foldedEntries(entries, "open", true, ""), entries);
});

test("a report note says what the sub-conversation did", () => {
  assert.equal(reportVerb("finished"), "finished");
  assert.equal(reportVerb("asks"), "asks you");
  assert.equal(reportVerb("odd"), "reports");
});

test("a branch a landing is checking reads Reviewing, after waiting to land", () => {
  assert.equal(groupOf({ id: "c", status: "closed", waiting_to_land: true, reviewing: true }), "landing");
  assert.equal(groupOf({ id: "c", status: "idle", reviewing: true, live: { status: "running" } }), "working");
  assert.equal(stepWords("done", false, "pulse/x", true).word, "Reviewing");
  assert.equal(stepWords("done", true, "pulse/x", true).word, "Working");
  assert.equal(stepWords("done", false, "pulse/x", false).word, "Waiting to land");
  assert.equal(taskMoodOf({ status: "done", lead: "", needs_you: false, working: false, landing: true, reviewing: true }).word, "Reviewing");
  assert.equal(taskGroupOf({ status: "done", lead: "", needs_you: false, working: false, reviewing: true }), "landing");
});
