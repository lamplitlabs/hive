import assert from "node:assert/strict";
import { register } from "node:module";
import { test } from "node:test";

// Render both question classes the For you tab shows: what still waits for the
// owner (QuestionCard with decision buttons) and what the agents settled
// themselves (the collapsed "Answered by the agents" audit). The point of the
// experiment: the owner can audit the agents' answers without those rows ever
// inflating the waiting count. Which questions land in which class, the
// audit's words and the count are pure and tested without react in
// tabs-state.test.mjs; this suite only renders, so it needs react and the
// typescript the loader transpiles .tsx with. A clone without node_modules
// (the one hive evolve measure scores the hub's coverage in) skips it
// instead of failing.
register("./tsx-test-loader.mjs", import.meta.url);

const React = await import("react").then((m) => m.default, () => null);
const ts = await import("typescript").then(() => true, () => false);
const skip = React && ts ? false : "react and typescript are not installed (npm ci)";
const render = skip ? null : await import("react-dom/server");
const tabs = skip ? null : await import("./tabs.tsx");
const cards = skip ? null : await import("../hive/cards.tsx");
const { AGENTS_DECIDED_SUMMARY } = await import("./tabs-state.ts");

const base = {
  project: "demo-app",
  slug: "demo-app",
  thread_key: "demo-app:governance/vote-reasons",
  area: "governance",
  thread: "vote-reasons",
  kind: "question",
  on_post: "",
  stop: false,
  stale: false,
  thread_last: "",
  agent: "agent-rose",
};

const OWNER_Q = {
  ...base,
  id: "q-owner",
  date: "2026-10-28T10:00:00Z",
  summary: "Ship the beta to everyone?",
  body: "The fleet asks whether the beta opens to all projects now.",
  decider: "owner",
};

const LEAD_Q = {
  ...base,
  id: "q-lead",
  date: "2026-10-28T11:00:00Z",
  summary: "Retry the flaky landing?",
  body: "Settled by the lead while the owner was away.",
  decider: "lead",
};

const SETTING_Q = {
  ...base,
  id: "q-setting",
  date: "2026-10-28T12:00:00Z",
  summary: "Raise the daily operating-layer cap?",
  body: "A configured default settled this one.",
  decider: "setting",
};

const e = (...args) => React.createElement(...args);
const html = (...args) => render.renderToStaticMarkup(e(...args));

test("agents-decided audit lists the lead/setting questions collapsed under a toggle", { skip }, () => {
  const out = html(tabs.AgentsDecidedAudit, { questions: [LEAD_Q, SETTING_Q], slug: "demo-app" });
  // Collapsed under a toggle: a closed <details> carries the summary line.
  assert.match(out, /<details(?![^>]*\bopen\b)/);
  assert.ok(out.includes(AGENTS_DECIDED_SUMMARY), "toggle names the agents-decided class");
  assert.ok(out.includes("· 2"), "toggle carries the count of settled questions");
  // Both agent-settled classes are auditable; the owner's row never leaks in.
  assert.ok(out.includes(LEAD_Q.summary), "lead-settled question renders");
  assert.ok(out.includes(SETTING_Q.summary), "setting-settled question renders");
  assert.ok(out.includes("Settled by the lead"), "the lead class is named");
  assert.ok(out.includes("Settled by a setting"), "the setting class is named");
  assert.ok(!out.includes(OWNER_Q.summary), "owner question never leaks into the audit");
});

test("agents-decided rows are read-only (nothing to decide) while the owner's card keeps its buttons", { skip }, () => {
  const audit = html(tabs.AgentsDecidedAudit, { questions: [LEAD_Q], slug: "demo-app" });
  assert.ok(!audit.includes('aria-label="Yes to'), "settled questions offer no Yes button");
  assert.ok(!audit.includes('aria-label="No to'), "settled questions offer no No button");
  assert.ok(audit.includes("See it on the board"), "the audit links to the thread holding the answer");
  const waiting = html(cards.QuestionCard, { q: OWNER_Q, slug: "demo-app", canDecide: true });
  assert.ok(waiting.includes('aria-label="Yes to'), "owner-decidable question still renders its decision buttons");
});
