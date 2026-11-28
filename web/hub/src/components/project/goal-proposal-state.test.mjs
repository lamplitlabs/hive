import assert from "node:assert/strict";
import { register } from "node:module";
import { test } from "node:test";

// The goal proposal card's logic (TASK-117), tested without React: how a
// proposed goal on the org rows joins the inbox, what the card says, and
// what "Edit then use" writes.
register("./tsx-test-loader.mjs", import.meta.url);

const {
  applyEdit,
  GOAL_HARD_STOP,
  GOAL_PROPOSAL_LABELS,
  GOAL_PROPOSAL_TITLE,
  editThenUse,
  goalAnswers,
  goalInboxProposals,
  goalProposalView,
  goalProposer,
  isGoalProposal,
  projectGoalProposal,
  withGoalProposals,
  withoutGoalProposals,
  withInboxGoals,
} = await import("./goal-proposal-state.ts");
const { buildInbox } = await import("../hive/inbox-state.ts");

const goal = (extra = {}) => ({ post: "P-7", agent: "hive-lead", date: "2026-11-22T05:00:00Z", text: "Make Hive one system where humans, agents and machines work together.", why: "The owner said so in chat; ADR-070 is accepted.", ...extra });
const row = (slug, g, extra = {}) => ({ slug, base: "/b/" + slug, goal_proposal: g, ...extra });

test("a proposed goal on an org row becomes one awaiting inbox proposal", () => {
  const rows = [row("hive", goal()), row("demo", null), row("old", undefined)];
  const got = goalInboxProposals(rows);
  assert.equal(got.length, 1);
  const p = got[0];
  assert.equal(p.id, "P-7");
  assert.equal(p.slug, "hive");
  assert.equal(p.status, "awaiting");
  assert.equal(p.summary, GOAL_PROPOSAL_TITLE);
  assert.equal(p.proposal_text, goal().text);
  assert.equal(p.kind, "goal");
  assert.equal(p.can_decide, true);
  assert.equal(p.hard_stop, GOAL_HARD_STOP, "a goal change is a hard stop: the owner's alone");
  assert.ok(isGoalProposal(p));
});

test("an empty or postless proposal adds nothing, and one post is listed once", () => {
  assert.deepEqual(goalInboxProposals([row("a", goal({ text: "  " })), row("b", goal({ post: "" }))]), []);
  const twice = goalInboxProposals([row("a", goal()), row("a", goal())]);
  assert.equal(twice.length, 1);
});

test("a board proposal for the same post gains the goal instead of a second row", () => {
  const board = { id: "P-7", thread_key: "goal/amend", date: "2026-11-22T05:00:00Z", status: "awaiting", decided_id: "", decided_date: "", summary: "proposed goal: one system", decided_summary: "", proposal_text: "goal: amend", can_decide: true, slug: "hive" };
  const other = { ...board, id: "P-1", summary: "Raise the retry budget", proposal_text: "Raise it." };
  const merged = withGoalProposals([board, other], [row("hive", goal())]);
  assert.equal(merged.length, 2);
  assert.equal(merged[0].summary, GOAL_PROPOSAL_TITLE);
  assert.equal(merged[0].proposal_text, goal().text);
  assert.ok(isGoalProposal(merged[0]));
  assert.ok(!isGoalProposal(merged[1]));
});

test("the inbox count includes the proposed goal and the project's share of it", () => {
  const loaded = (data) => ({ data, error: "", loading: false, updatedAt: 1, reload: async () => {} });
  const org = { totals: { brain: "hive" }, can_decide: true, projects: [row("hive", goal()), row("demo", null)], proposals: [], questions: [], commons: null };
  const inbox = buildInbox({ org: loaded(org), proposed: loaded({ records: [] }), reg: loaded({ projects: [] }), compute: loaded({ requests: [] }), suggested: loaded({ suggestions: [] }), runner: loaded(undefined), service: loaded(undefined), settings: loaded(undefined), harness: loaded(undefined) });
  assert.equal(inbox.proposals.length, 1);
  assert.equal(inbox.forProject("hive").proposals.length, 1);
  assert.equal(inbox.forProject("demo").proposals.length, 0);
  // The Overview shows the proposal as its own panel, not again under Needs you.
  const mine = inbox.forProject("hive");
  const rest = withoutGoalProposals(mine);
  assert.equal(rest.proposals.length, 0);
  assert.equal(rest.total, mine.total - 1);
});

test("the card names who proposed it, says what it replaces, and always has a why", () => {
  assert.equal(goalProposer("hive-lead", "hive"), "Project lead");
  assert.equal(goalProposer("hive:goal-draft", "hive"), "Hive");
  assert.equal(goalProposer("", "hive"), "Hive");
  const v = goalProposalView(goal(), "hive", { text: "Make hive more useful.", source: "generated" });
  assert.equal(v.title, "Hive proposes this goal");
  assert.equal(v.by, "Proposed by Project lead");
  assert.equal(v.text, goal().text);
  assert.equal(v.why, goal().why);
  assert.equal(v.same, false);
  assert.match(v.replaces, /goal Hive wrote/);
  assert.match(goalProposalView(goal(), "hive", { text: "Mine.", source: "owner" }).replaces, /goal you set/);
  const drafted = goalProposalView(goal({ agent: "hive:goal-draft", why: "" }), "hive", null);
  assert.equal(drafted.by, "Drafted by Hive from the project's evidence");
  assert.match(drafted.why, /No reason was given/);
  assert.equal(goalProposalView(goal(), "hive", { text: "  " + goal().text.replace(/ /g, "  "), source: "generated" }).same, true);
  assert.deepEqual(GOAL_PROPOSAL_LABELS, { use: "Accept the drafted goal", edit: "Edit then use", dismiss: "Keep the current goal" });
});

test("the owner's two answers: accept is recommended with the draft's why; a draft that changes nothing recommends keeping", () => {
  const a = goalAnswers(goal(), { text: "Make hive more useful.", source: "generated" });
  assert.deepEqual(a.options.map((o) => [o.label, o.answer, o.recommended]), [
    ["Accept the drafted goal", "yes", true],
    ["Keep the current goal", "no", false],
  ]);
  assert.equal(a.options.filter((o) => o.recommended).length, 1, "exactly one recommended");
  assert.equal(a.why, "the owner said so in chat; ADR-070 is accepted");
  assert.match(a.options[0].effect, /goal Hive wrote/);
  assert.match(goalAnswers(goal(), { text: "Mine.", source: "owner" }).options[0].effect, /goal you set/);
  // No Why on the draft: the reason still says why accepting is recommended.
  assert.match(goalAnswers(goal({ why: "" }), null).why, /differs from the goal in force/);
  // The same words as the goal in force: keep it.
  const same = goalAnswers(goal(), { text: goal().text, source: "owner" });
  assert.deepEqual(same.options.map((o) => o.recommended), [false, true]);
  assert.match(same.why, /nothing would change/);
});

test("on the Sessions strip a proposed goal is a goal hard stop with its card open", async () => {
  const { forYouRows, hardStopRows, HARD_STOP_LABEL } = await import("../sessions/asks-state.ts");
  const loaded = (data) => ({ data, error: "", loading: false, updatedAt: 1, reload: async () => {} });
  const org = { totals: { brain: "hive" }, can_decide: true, projects: [row("hive", goal())], proposals: [{ id: "P-1", slug: "hive", status: "awaiting", summary: "Rename a helper", date: "2026-11-21T00:00:00Z", hard_stop: "" }], questions: [], commons: null };
  const inbox = buildInbox({ org: loaded(org), proposed: loaded({ records: [] }), reg: loaded({ projects: [] }), compute: loaded({ requests: [] }), suggested: loaded({ suggestions: [] }), runner: loaded(undefined), service: loaded(undefined), settings: loaded(undefined), harness: loaded(undefined) });
  const strip = hardStopRows(forYouRows({ decisions: [], proposals: inbox.proposals, questions: [], compute: [], work: [], drafts: [], steps: [], machine: false, brain: "hive" }));
  assert.deepEqual(strip.map((r) => [r.key, r.stopKind, r.meta, r.card]), [["proposal:P-7", "goal", "New project goal", true]], "the routine proposal stays the lead's");
  assert.equal(HARD_STOP_LABEL.goal, "Project goal");
});

test("Edit then use: unchanged words need only the yes; bad text writes nothing", () => {
  const t = goal().text;
  assert.deepEqual(editThenUse(t, "  " + t + "\n"), { ok: true, text: t, edited: false, words: "Used as proposed." });
  const e = editThenUse(t, t + " Measured by teams onboarded.");
  assert.equal(e.ok, true);
  assert.equal(e.edited, true);
  assert.equal(e.words, "Used with my edits.");
  assert.equal(editThenUse(t, "   ").ok, false);
  const long = editThenUse(t, "x".repeat(4001));
  assert.equal(long.ok, false);
  assert.match(long.error, /4,001 characters/);
  assert.deepEqual(editThenUse(t, "a\r\nb"), { ok: true, text: "a\nb", edited: true, words: "Used with my edits." });
});

test("Edit then use saves the owner's words before the yes; a refused edit says no yes", async () => {
  const t = goal().text;
  const recorder = (fail = {}) => {
    const calls = [];
    return {
      calls,
      setProjectGoal: async (slug, text) => {
        calls.push(["goal", slug, text]);
        if (fail.goal) throw new Error("refused");
      },
      decide: async (id, answer, words) => {
        calls.push(["decide", id, answer, words]);
        if (fail.decide) throw new Error("down");
      },
    };
  };
  const edited = editThenUse(t, t + " Measured by teams onboarded.");
  const w = recorder();
  assert.deepEqual(await applyEdit(edited, "hive", "P-7", w), { ok: true });
  assert.deepEqual(w.calls, [
    ["goal", "hive", t + " Measured by teams onboarded."],
    ["decide", "P-7", "yes", "Used with my edits."],
  ]);
  // The server refuses the words: no yes follows, the draft still waits.
  const refused = recorder({ goal: true });
  const r = await applyEdit(edited, "hive", "P-7", refused);
  assert.equal(r.ok, false);
  assert.equal(r.goalSet, false);
  assert.match(r.title, /still waits/);
  assert.deepEqual(refused.calls.map((c) => c[0]), ["goal"]);
  // Saved, but the yes failed: the goal is the owner's words all the same.
  const late = await applyEdit(edited, "hive", "P-7", recorder({ decide: true }));
  assert.equal(late.ok, false);
  assert.equal(late.goalSet, true);
  // Unchanged words: the yes alone.
  const plain = recorder();
  assert.deepEqual(await applyEdit(editThenUse(t, t), "hive", "P-7", plain), { ok: true });
  assert.deepEqual(plain.calls, [["decide", "P-7", "yes", "Used as proposed."]]);
});

test("the project's proposal is read from its own org row", () => {
  const rows = [row("hive", goal()), row("demo", null)];
  assert.equal(projectGoalProposal(rows, "hive")?.post, "P-7");
  assert.equal(projectGoalProposal(rows, "demo"), null);
  assert.equal(projectGoalProposal(undefined, "hive"), null);
  assert.equal(projectGoalProposal([row("x", goal({ text: "" }))], "x"), null);
});

test("a project's proposals gain the inbox's goal proposals once", () => {
  const g = { id: "P-7", status: "awaiting", summary: "Hive proposes this goal", proposal_text: "Ship it", kind: "goal", goal: { post: "P-7", text: "Ship it", why: "Because", date: "2026-11-22T05:00:00Z" } };
  const plain = { id: "P-1", status: "awaiting", summary: "Raise the budget", proposal_text: "" };
  const out = withInboxGoals([plain, { id: "P-7", status: "awaiting", summary: "goal/amend", proposal_text: "" }], [g, { id: "P-2", summary: "other" }]);
  assert.deepEqual(out.map((p) => p.id), ["P-1", "P-7"]);
  assert.equal(out[1].goal.text, "Ship it");
  assert.deepEqual(withInboxGoals([plain], [g]).map((p) => p.id), ["P-1", "P-7"]);
  assert.deepEqual(withInboxGoals([plain], []).map((p) => p.id), ["P-1"]);
});
