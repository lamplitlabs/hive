import assert from "node:assert/strict";
import { register } from "node:module";
import { test } from "node:test";

// The For you tab's split between what waits for the owner and what the
// agents settle themselves, without react: tabs-state.ts and inbox-state.ts
// import only pure .ts modules, so this suite runs in a clone without
// node_modules too (the way pulse evolve measure scores the hub's coverage
// fixtures). The rendered audit is agents-decided-audit.test.mjs.
register("./tsx-test-loader.mjs", import.meta.url);

const { AGENTS_DECIDED_HINT, AGENTS_DECIDED_SUMMARY, deciderOf, settledLabel, splitQuestions, waitingNote, whoSettled } = await import("./tabs-state.ts");
const { buildInbox } = await import("../pulse/inbox-state.ts");

const base = { project: "demo-app", slug: "demo-app", area: "governance", thread: "vote-reasons", kind: "question", stop: false, stale: false, agent: "agent-rose", body: "" };
const OWNER_Q = { ...base, id: "q-owner", date: "2026-10-28T10:00:00Z", summary: "Ship the beta to everyone?", decider: "owner" };
const OLD_Q = { ...base, id: "q-old", date: "2026-10-28T10:30:00Z", summary: "An older row without a decider" };
const LEAD_Q = { ...base, id: "q-lead", date: "2026-10-28T11:00:00Z", summary: "Retry the flaky landing?", decider: "lead" };
const SETTING_Q = { ...base, id: "q-setting", date: "2026-10-28T12:00:00Z", summary: "Raise the daily operating-layer cap?", decider: "setting" };

test("a question is the owner's unless it names another decider", () => {
  assert.equal(deciderOf(OWNER_Q), "owner");
  assert.equal(deciderOf(OLD_Q), "owner");
  assert.equal(deciderOf(LEAD_Q), "lead");
  assert.equal(deciderOf(SETTING_Q), "setting");
});

test("the split keeps the owner's questions waiting and the agents' in the audit, each in order", () => {
  const { questions, agentsDecided } = splitQuestions([LEAD_Q, OWNER_Q, SETTING_Q, OLD_Q]);
  assert.deepEqual(questions.map((x) => x.id), ["q-owner", "q-old"]);
  assert.deepEqual(agentsDecided.map((x) => x.id), ["q-lead", "q-setting"]);
  assert.deepEqual(splitQuestions([]), { questions: [], agentsDecided: [] });
});

test("the audit names who settled each question", () => {
  assert.equal(AGENTS_DECIDED_SUMMARY, "Answered by the agents");
  assert.match(AGENTS_DECIDED_HINT, /never counted as waiting for you/);
  assert.equal(whoSettled("lead"), "the lead");
  assert.equal(whoSettled("setting"), "a setting");
  assert.equal(settledLabel(LEAD_Q), "Settled by the lead: Retry the flaky landing?");
  assert.equal(settledLabel(SETTING_Q), "Settled by a setting: Raise the daily operating-layer cap?");
});

test("the waiting note says where to answer, or how many the agents settle", () => {
  assert.equal(waitingNote(false, 2), "To answer, open PULSE on the machine that runs it.");
  assert.equal(waitingNote(true, 0), undefined);
  assert.equal(waitingNote(true, 1), "1 question the agents are deciding themselves.");
  assert.equal(waitingNote(true, 2), "2 questions the agents are deciding themselves.");
});

test("For you count stays accurate: agents-settled questions never enter the waiting total", () => {
  const loaded = (data) => ({ loading: false, error: "", data });
  const empty = { loading: false, error: "", data: undefined };
  const inbox = buildInbox({
    org: loaded({ name: "org", projects: [], totals: {}, proposals: [], questions: [OWNER_Q, LEAD_Q, SETTING_Q] }),
    proposed: empty,
    reg: empty,
    compute: empty,
    runner: empty,
    service: empty,
    settings: empty,
  });
  const mine = inbox.forProject("demo-app");
  assert.equal(mine.questions.length, 1, "only the owner-decidable question waits");
  assert.equal(mine.questions[0].id, OWNER_Q.id);
  assert.equal(mine.agentsDeciding, 2, "both agent-settled classes are counted separately");
  assert.equal(mine.total, 1, "the For you total excludes what the agents settled");
  // The tab's own split agrees with the shared count.
  assert.equal(splitQuestions([OWNER_Q, LEAD_Q, SETTING_Q]).agentsDecided.length, mine.agentsDeciding);
});
