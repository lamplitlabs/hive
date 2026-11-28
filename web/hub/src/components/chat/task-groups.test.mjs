import assert from "node:assert/strict";
import { test } from "node:test";
import { GROUP_ORDER, entryGroup, entryId, foldedEntries, jobWaitingForPr, listEntries, ownerTurn, prGroupOf, prMood, prWaitLine, sessionWords, stepWords, taskFinished, taskGroup, taskGroupOf, taskMood, taskMoodOf, taskOwnerTurn, taskPrState, taskTitleOf } from "./task-groups.ts";

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

test("work done without a pull request yet waits for one: a step by its branch, a task by its steps, a conversation by the hub's answer", () => {
  // A done job keeps its branch until the pull request is merged or closed.
  assert.equal(jobWaitingForPr("done", "hive/iris/job-1"), true);
  assert.equal(jobWaitingForPr("done", ""), false);
  assert.equal(jobWaitingForPr("done", undefined), false);
  assert.equal(jobWaitingForPr("failed", "hive/iris/job-1"), false);
  assert.equal(jobWaitingForPr("leased", "hive/iris/job-1"), false);
  const waiting = session({ job_status: "done", status: "ended", branch: "hive/iris/job-1" });
  assert.deepEqual(sessionWords(waiting), { word: "Waiting for its pull request", tone: "neutral" });
  assert.deepEqual(stepWords("done", false, "hive/iris/job-1"), { word: "Waiting for its pull request", tone: "neutral" });
  assert.deepEqual(stepWords("done", true, "hive/iris/job-1"), { word: "Working", tone: "run" }, "a session at it outranks the branch");
  assert.deepEqual(stepWords("done", false, ""), { word: "Done", tone: "ok" });
  // The task: the section sits between working and open; a step waiting for
  // the pull request outranks Done (the task is not on main yet) but not an
  // agent at another step, nor the owner's turn.
  assert.deepEqual(GROUP_ORDER, ["needs_review", "working", "review", "open", "finished", "archived"]);
  assert.equal(taskMood(task({ sessions: [waiting] })).word, "Waiting for its pull request");
  assert.equal(taskGroup(task({ sessions: [waiting] })), "review");
  assert.equal(taskMood(task({ status: "done", sessions: [waiting] })).word, "Waiting for its pull request");
  assert.equal(taskGroup(task({ status: "done", sessions: [waiting] })), "review");
  assert.equal(taskGroup(task({ status: "cancelled", sessions: [waiting] })), "review");
  assert.equal(taskMood(task({ sessions: [waiting, session({ job_status: "leased", status: "running" })] })).word, "Working");
  assert.equal(taskGroup(task({ sessions: [waiting, session({ job_status: "leased" })] })), "working");
  assert.equal(taskMood(task({ status: "review", needs_you: true, sessions: [waiting] })).word, "Asked you");
  assert.equal(taskGroup(task({ status: "review", needs_you: true, sessions: [waiting] })), "working");
  assert.equal(taskGroup(task({ status: "done", sessions: [session({ job_status: "done", branch: "" })] })), "finished");
  // The task's own page reads the same from its jobs.
  const state = (over = {}) => ({ status: "in_progress", lead: "one-lead", needs_you: false, working: false, waiting_for_pr: true, ...over });
  assert.equal(taskMoodOf(state()).word, "Waiting for its pull request");
  assert.equal(taskGroupOf(state()), "review");
  assert.equal(taskMoodOf(state({ status: "done" })).word, "Waiting for its pull request");
  assert.equal(taskMoodOf(state({ working: true })).word, "Working");
  assert.equal(taskMoodOf(state({ status: "done", working: true })).word, "Waiting for its pull request", "a done task's running session does not hide the step without a pull request");
  assert.equal(taskGroupOf(state({ status: "done", working: true })), "review");
  assert.deepEqual(taskMoodOf(state()), taskMood(task({ sessions: [waiting] })));
  // A conversation: the hub's answer puts a finished or idle one in the
  // section; one being worked on, asked about or put away keeps its own.
  assert.equal(groupOf({ id: "c", status: "closed", waiting_to_land: true }), "review");
  assert.equal(groupOf({ id: "c", status: "idle", waiting_to_land: true }), "review");
  assert.equal(groupOf({ id: "c", status: "ended", waiting_to_land: true }), "review");
  assert.equal(groupOf({ id: "c", status: "closed", waiting_to_land: false }), "finished");
  assert.equal(groupOf({ id: "c", status: "closed" }), "finished");
  assert.equal(groupOf({ id: "c", status: "idle", waiting_to_land: true, live: { status: "running" } }), "working");
  assert.equal(groupOf({ id: "c", status: "waiting", waiting_to_land: true }), "working");
  assert.equal(groupOf({ id: "c", status: "archived", waiting_to_land: true }), "archived");
  // A family with a finished parent and a sub-conversation waiting for its
  // pull request is listed under In review.
  const rows = [
    { id: "p", status: "closed", created_at: "2026-10-31T10:00:00Z" },
    { id: "k", status: "closed", parent: "p", created_at: "2026-10-31T10:01:00Z", waiting_to_land: true },
  ];
  const entries = listEntries(rows, []);
  assert.equal(entries.length, 1);
  assert.equal(entryGroup(entries[0]), "review");
});

test("a pull request's review state groups the entry first: the owner's two states on top, the reviewer's five under In review, merged and closed by the entry's own status", () => {
  for (const state of ["needs_owner", "review_failed"]) assert.equal(prGroupOf(state), "needs_review", state);
  for (const state of ["pr_open", "in_review", "merge_asked", "merge_waiting", "changes_requested"]) assert.equal(prGroupOf(state), "review", state);
  for (const state of ["merged", "closed", "", undefined, null, "odd"]) assert.equal(prGroupOf(state), null, String(state));
  assert.equal(prMood("needs_owner").word, "Needs your review");
  assert.equal(prMood("needs_owner").tone, "wait");
  assert.equal(prMood("review_failed").word, "Review failed");
  assert.equal(prMood("in_review").word, "In review");
  assert.equal(prMood("merge_asked").word, "Merging");
  assert.equal(prMood("changes_requested").word, "Changes requested");
  assert.equal(prMood("merged"), null);
  // A task: its pull request outranks Done and the steps still carrying
  // their branch, but an agent at a step or the lead's question comes
  // first; the owner's Merge comes before everything.
  const pr = (state) => ({ number: 7, url: "https://g/pr/7", state });
  const waiting = session({ job_status: "done", status: "ended", branch: "hive/iris/job-1" });
  assert.equal(taskMood(task({ sessions: [waiting], pr: pr("pr_open") })).word, "Pull request open");
  assert.equal(taskGroup(task({ sessions: [waiting], pr: pr("pr_open") })), "review");
  assert.equal(taskMood(task({ status: "done", sessions: [waiting], pr: pr("in_review") })).word, "In review");
  assert.equal(taskGroup(task({ status: "done", pr: pr("in_review") })), "review");
  assert.equal(taskMood(task({ status: "done", pr: pr("merged") })).word, "Done");
  assert.equal(taskGroup(task({ status: "done", pr: pr("merged") })), "finished");
  assert.equal(taskGroup(task({ status: "in_progress", pr: pr("closed") })), "open");
  assert.equal(taskMood(task({ sessions: [waiting, session({ job_status: "leased", status: "running" })], pr: pr("pr_open") })).word, "Working");
  assert.equal(taskGroup(task({ sessions: [session({ job_status: "leased" })], pr: pr("pr_open") })), "working");
  assert.equal(taskMood(task({ status: "review", needs_you: true, pr: pr("needs_owner") })).word, "Asked you");
  assert.equal(taskMood(task({ sessions: [session({ job_status: "leased", status: "running" })], pr: pr("needs_owner") })).word, "Needs your review");
  assert.equal(taskGroup(task({ sessions: [session({ job_status: "leased" })], pr: pr("needs_owner") })), "needs_review");
  assert.equal(taskGroup(task({ status: "done", pr: pr("review_failed") })), "needs_review");
  assert.equal(taskMood(task({ status: "done", pr: pr("review_failed") })).word, "Review failed");
  // Its steps read the pull request's word instead of waiting for one.
  assert.deepEqual(sessionWords(waiting, "in_review"), { word: "In review", tone: "run" });
  assert.deepEqual(stepWords("done", false, "hive/x", "needs_owner"), { word: "Needs your review", tone: "wait" });
  assert.deepEqual(stepWords("done", false, "hive/x", "merged"), { word: "Waiting for its pull request", tone: "neutral" }, "a branch the daemon has not cleared yet");
  assert.deepEqual(stepWords("done", true, "hive/x", "needs_owner"), { word: "Working", tone: "run" });
  assert.deepEqual(stepWords("done", false, "", "needs_owner"), { word: "Done", tone: "ok" });
  // The task's own page reads the same from the delivery state.
  assert.equal(taskMoodOf({ status: "in_progress", lead: "", needs_you: false, working: false, pr: "merge_asked" }).word, "Merging");
  assert.equal(taskGroupOf({ status: "in_progress", lead: "", needs_you: false, working: false, pr: "changes_requested" }), "review");
  // A conversation: the pull request's state wins over the branch's commits,
  // the owner's turn over a running agent; a merged one is finished whatever
  // its branch still holds (a squash leaves commits main lacks).
  assert.equal(groupOf({ id: "c", status: "closed", waiting_to_land: true, pr: pr("pr_open") }), "review");
  assert.equal(groupOf({ id: "c", status: "closed", waiting_to_land: true, pr: pr("merged") }), "finished");
  assert.equal(groupOf({ id: "c", status: "idle", waiting_to_land: false, pr: pr("changes_requested") }), "review");
  assert.equal(groupOf({ id: "c", status: "idle", pr: pr("needs_owner"), live: { status: "running" } }), "needs_review");
  assert.equal(groupOf({ id: "c", status: "idle", pr: pr("pr_open"), live: { status: "running" } }), "working");
  assert.equal(groupOf({ id: "c", status: "archived", pr: pr("needs_owner") }), "needs_review", "the owner's Merge is asked for even on a put-away conversation");
  assert.equal(groupOf({ id: "c", status: "archived", pr: pr("pr_open") }), "archived");
  assert.equal(groupOf({ id: "c", status: "closed", pr: null }), "finished");
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

test("an approved pull request whose merge waits says what it waits for, in one line", () => {
  assert.equal(prMood("merge_waiting").word, "Approved, merge waits");
  assert.equal(prMood("merge_waiting").tone, "neutral");
  assert.equal(
    prWaitLine("merge_waiting", "approved; the merge waits for the checks still running (ci/build)"),
    "The merge waits for the checks still running (ci/build)",
  );
  assert.equal(
    prWaitLine("merge_waiting", "approved; GitHub refused the merge (HTTP 502); retried at 2026-09-06T10:05:00Z (attempt 2)"),
    "GitHub refused the merge (HTTP 502); retried at 2026-09-06T10:05:00Z (attempt 2)",
  );
  assert.equal(prWaitLine("merge_waiting", ""), "");
  assert.equal(prWaitLine("review_failed", "approved; anything"), "", "only a waiting merge has the line");
});

test("a pull request held for the owner is the owner's turn; one the runner retries is not", () => {
  const done = session({ job_status: "done", branch: "hive/w/job-1" });
  const conflict = task({ sessions: [done], hold: { state: "held_for_owner", note: "does not merge cleanly; this one is for you", at: "" } });
  assert.equal(taskMood(conflict).word, "Pull request held for you");
  assert.equal(taskMood(conflict).tone, "wait");
  assert.equal(taskGroup(conflict), "needs_review");
  assert.equal(sessionWords(done, taskPrState(conflict)).word, "Pull request held for you");
  assert.equal(taskOwnerTurn(conflict), true);
  // A hub that says so wins.
  assert.equal(taskOwnerTurn({ ...conflict, owner_turn: false }), false);

  const fault = task({ sessions: [done], hold: { state: "held", note: "gh: could not resolve host", at: "" } });
  assert.equal(taskMood(fault).word, "Waiting for its pull request");
  assert.equal(taskGroup(fault), "review");
  assert.equal(taskOwnerTurn(fault), false);

  // The owner's turn outranks a step at work, as on the lifecycle view.
  const busy = task({ sessions: [done, session({ job: "job-2", job_status: "leased" })], hold: conflict.hold });
  assert.equal(taskMood(busy).word, "Pull request held for you");
  assert.equal(ownerTurn("in_progress", "held_for_owner"), true);
  assert.equal(ownerTurn("review", ""), true);
  assert.equal(ownerTurn("in_progress", "held"), false);
});
