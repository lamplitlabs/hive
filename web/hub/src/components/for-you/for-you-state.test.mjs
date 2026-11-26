import assert from "node:assert/strict";
import { test } from "node:test";
import { SECTION_ORDER, SECTION_TITLE, filterRows, forYouProjects, forYouRows, groupRows, latestRows, orderRows, waitingWords } from "./for-you-state.ts";
import { hrefs } from "../../lib/nav.ts";

const empty = { decisions: [], proposals: [], questions: [], compute: [], work: [], drafts: [], steps: [], machine: false, provider: null, brain: "amistio" };

const input = {
  ...empty,
  decisions: [{ id: "ADR-0042", project: "demo-app", title: "Use SQLite for the cache", kind: "adr", updated_at: "2026-11-05T10:00:00Z" }],
  proposals: [
    { id: "P-1", slug: "demo-app", summary: "Raise the retry budget", date: "2026-11-06T08:00:00Z" },
    { id: "P-2", summary: "Rename the brain crate", date: "2026-11-04T08:00:00Z" },
  ],
  questions: [
    { id: "Q-1", slug: "demo-app", summary: "Keep the old API?", date: "2026-11-01T08:00:00Z", stop: false },
    { id: "Q-2", slug: "web", summary: "Evolution stopped; owner review needed", date: "2026-10-30T08:00:00Z", stop: true },
  ],
  compute: [{ id: "C-7", project: "web", size: "Standard_D4s_v5", hours: 2, requested_by: "quartz", at: "2026-11-06T09:00:00Z" }],
  work: [{ id: "SUG-003", project: "demo-app", title: "Add a smoke test", updated_at: "2026-11-02T08:00:00Z" }],
  drafts: [{ slug: "web", r: { id: "EVO-constitution", draft: { author: "iris", at: "2026-11-03T08:00:00Z" } } }],
  steps: [{ kind: "heldout", slug: "demo-app", slugs: ["demo-app", "web"], label: "Set up private tests", title: "Set up private tests" }],
  machine: true,
  machineWhy: "Paused in Settings",
  provider: { name: "anthropic", label: "Anthropic", expired: true },
};

test("every inbox item becomes one row with its project, and the brain takes the unslugged ones", () => {
  const rows = forYouRows(input);
  assert.equal(rows.length, 11);
  const byKey = Object.fromEntries(rows.map((r) => [r.key, r]));
  assert.equal(byKey["proposal:P-2"].project, "amistio");
  assert.equal(byKey["proposal:P-1"].project, "demo-app");
  assert.equal(byKey["question:Q-2"].urgent, true);
  assert.equal(byKey["question:Q-2"].title, "Self-improvement stopped. May it carry on?", "a stop reads as the question it asks");
  assert.equal(byKey["question:Q-1"].title, "Keep the old API?");
  assert.ok(!byKey["question:Q-1"].urgent);
  assert.equal(byKey["compute:C-7"].title, "Standard_D4s_v5 for 2 hours");
  assert.equal(byKey["step:heldout:demo-app,web"].project, "", "a shared step belongs to no one project");
  assert.equal(byKey["machine"].section, "problems");
  assert.equal(byKey["machine"].meta, "Paused in Settings");
  assert.match(byKey["provider:anthropic"].title, /expired/);
  assert.match(byKey["provider:anthropic"].href, /connect=anthropic/);
  assert.equal(byKey["decision:ADR-0042"].yesNo, true);
  assert.equal(byKey["draft:web:EVO-constitution"].yesNo, undefined);
  // A row whose fix is a page away names it on its button.
  assert.equal(byKey["provider:anthropic"].fix, "Sign in");
  assert.equal(forYouRows({ ...empty, provider: { name: "openai", label: "OpenAI", expired: false } })[0].fix, "Connect");
  assert.equal(byKey["machine"].fix, "Fix it");
  assert.equal(byKey["machine"].href, hrefs.runnerControls(), "Fix it opens the runner's controls, not a page that forwards");
  assert.equal(byKey["step:heldout:demo-app,web"].fix, "Open");
  assert.equal(byKey["question:Q-1"].fix, undefined);
});

test("sections keep the fixed order and drop when empty", () => {
  const groups = groupRows(forYouRows(input));
  assert.deepEqual(groups.map((g) => g.section), SECTION_ORDER);
  const few = groupRows(forYouRows({ ...empty, questions: input.questions }));
  assert.deepEqual(few.map((g) => g.section), ["questions"]);
  assert.deepEqual(groupRows([]), []);
  for (const s of SECTION_ORDER) assert.ok(SECTION_TITLE[s], s + " has a title");
});

test("within a section: urgent first, then newest, then by key", () => {
  const qs = orderRows(forYouRows({ ...empty, questions: input.questions }));
  assert.deepEqual(qs.map((r) => r.key), ["question:Q-2", "question:Q-1"]);
  const ps = orderRows(forYouRows({ ...empty, proposals: input.proposals }));
  assert.deepEqual(ps.map((r) => r.key), ["proposal:P-1", "proposal:P-2"]);
  const tie = orderRows([
    { key: "b", section: "steps", kind: "rule", project: "", title: "", at: "" },
    { key: "a", section: "steps", kind: "rule", project: "", title: "", at: "" },
  ]);
  assert.deepEqual(tie.map((r) => r.key), ["a", "b"]);
});

test("the project filter keeps that project's rows and the machine-wide ones", () => {
  const rows = forYouRows(input);
  const web = filterRows(rows, "web");
  assert.deepEqual(
    web.map((r) => r.key).sort(),
    ["compute:C-7", "draft:web:EVO-constitution", "machine", "provider:anthropic", "question:Q-2", "step:heldout:demo-app,web"].sort(),
  );
  assert.equal(filterRows(rows, null).length, rows.length);
  assert.equal(filterRows(rows, "").length, rows.length);
  assert.deepEqual(forYouProjects(rows), [
    { slug: "demo-app", n: 4 },
    { slug: "web", n: 3 },
    { slug: "amistio", n: 1 },
  ]);
});

test("the bell's five lead with problems, then stop questions, then the newest", () => {
  const five = latestRows(forYouRows(input));
  assert.equal(five.length, 5);
  assert.deepEqual(five.slice(0, 3).map((r) => r.key), ["machine", "provider:anthropic", "question:Q-2"]);
  assert.deepEqual(five.slice(3).map((r) => r.key), ["compute:C-7", "proposal:P-1"]);
  assert.equal(latestRows([], 5).length, 0);
});

test("waiting words", () => {
  assert.equal(waitingWords(0), "Nothing waits on you");
  assert.equal(waitingWords(1), "1 thing waits on you");
  assert.equal(waitingWords(12), "12 things wait on you");
});

test("rows fall back to plain words when the inbox leaves a field out", () => {
  const rows = forYouRows({
    decisions: [{ id: "ADR-7", project: "demo-app", title: "Undated", kind: "adr" }],
    proposals: [{ id: "P-9", summary: "No slug, no brain", date: "2026-11-01T00:00:00Z" }],
    questions: [
      { id: "Q-9", summary: "No slug either", date: "2026-11-01T00:00:00Z" },
      { id: "Q-10", slug: "web", summary: "evolution stopped; OWNER review needed by the gate", date: "2026-11-02T00:00:00Z" },
      { id: "Q-11", slug: "web", date: "2026-11-03T00:00:00Z" },
    ],
    compute: [{ id: "C-1", project: "web", size: "Standard_D2s_v5", hours: 1, requested_by: "quartz", at: "2026-11-01T00:00:00Z" }],
    work: [
      { id: "SUG-9", project: "web", title: "Undated idea" },
      { id: "SUG-10", project: "web", title: "Reviewed idea", created_at: "2026-11-01T00:00:00Z", updated_at: "2026-11-02T00:00:00Z", reviewed_at: "2026-11-03T00:00:00Z" },
      { id: "SUG-11", project: "web", title: "Created only", created_at: "2026-11-01T00:00:00Z" },
    ],
    drafts: [
      { slug: "web", r: { id: "EVO-evaluators", draft: { at: "2026-11-04T00:00:00Z" } } },
      { slug: "demo-app", r: { id: "EVO-constitution", draft: null } },
    ],
    steps: [
      { kind: "heldout", slug: "web", label: "Set up private tests", title: "Set up private tests" },
      { kind: "card", slug: "demo-app", slugs: ["demo-app"], label: "Name a check", title: "Name a check" },
    ],
    machine: true,
    provider: { name: "gemini", label: "", expired: false },
  });
  const byKey = Object.fromEntries(rows.map((r) => [r.key, r]));
  // No brain set: unslugged proposals and questions belong to no project.
  assert.equal(byKey["proposal:P-9"].project, "");
  assert.equal(byKey["question:Q-9"].project, "");
  // A gate's stop line reads as the stop question even without the flag, but only the flag makes it urgent.
  assert.equal(byKey["question:Q-10"].title, "Self-improvement stopped. May it carry on?");
  assert.ok(!byKey["question:Q-10"].urgent);
  assert.equal(byKey["question:Q-11"].title, undefined, "a question without a summary is not taken for a stop");
  // Dates: the first one known, else none.
  assert.equal(byKey["decision:ADR-7"].at, "");
  assert.equal(byKey["work:SUG-9"].at, "");
  assert.equal(byKey["work:SUG-10"].at, "2026-11-03T00:00:00Z");
  assert.equal(byKey["work:SUG-11"].at, "2026-11-01T00:00:00Z");
  assert.equal(byKey["compute:C-1"].title, "Standard_D2s_v5 for 1 hour");
  // Drafts: the evaluators read as such; no author, no date.
  assert.equal(byKey["draft:web:EVO-evaluators"].title, "New evaluators drafted");
  assert.equal(byKey["draft:web:EVO-evaluators"].meta, undefined);
  assert.equal(byKey["draft:web:EVO-evaluators"].at, "2026-11-04T00:00:00Z");
  assert.equal(byKey["draft:demo-app:EVO-constitution"].title, "A new constitution drafted");
  assert.equal(byKey["draft:demo-app:EVO-constitution"].at, "");
  // Steps: one project's step stays that project's, keyed by its slug.
  assert.equal(byKey["step:heldout:web"].project, "web");
  assert.equal(byKey["step:card:demo-app"].project, "demo-app");
  // Machine and provider: the default reason, the provider's name when it has no label.
  assert.equal(byKey["machine"].meta, "Agents can't start work on this machine");
  assert.equal(byKey["provider:gemini"].title, "Connect gemini");
  assert.equal(byKey["provider:gemini"].meta, "Agents have no model provider yet");
  assert.equal(byKey["provider:gemini"].href, "/settings/?connect=gemini#harness");
  const expired = forYouRows({ ...empty, provider: { name: "copilot", label: "", expired: true } })[0];
  assert.equal(expired.title, "copilot sign-in expired");
  assert.equal(expired.meta, "Sign in again so agents can use their model");
});

test("filter chips with the same count read in name order", () => {
  const row = (key, project) => ({ key, section: "work", kind: "suggestion", project, title: "", at: "" });
  assert.deepEqual(forYouProjects([row("1", "web"), row("2", "api"), row("3", ""), row("4", "web"), row("5", "api"), row("6", "zed")]), [
    { slug: "api", n: 2 },
    { slug: "web", n: 2 },
    { slug: "zed", n: 1 },
  ]);
  assert.deepEqual(forYouProjects([]), []);
});

test("order puts newer first whichever way the rows arrive", () => {
  const row = (key, at, urgent) => ({ key, section: "work", kind: "suggestion", project: "", title: "", at, urgent });
  const older = row("a", "2026-11-01T00:00:00Z");
  const newer = row("b", "2026-11-02T00:00:00Z");
  const undated = row("c", "");
  assert.deepEqual(orderRows([older, newer, undated]).map((r) => r.key), ["b", "a", "c"]);
  assert.deepEqual(orderRows([undated, newer, older]).map((r) => r.key), ["b", "a", "c"]);
  assert.deepEqual(orderRows([newer, row("u", "", true)]).map((r) => r.key), ["u", "b"], "urgent beats newer");
  const bell = latestRows([older, newer, row("p", "", false)].concat([{ ...row("m", ""), section: "problems" }]), 2);
  assert.deepEqual(bell.map((r) => r.key), ["m", "b"]);
});

test("a proposed goal is a proposal row that names itself and opens as its card", () => {
  const rows = forYouRows({ ...empty, proposals: [{ id: "P-9", slug: "hive", summary: "Hive proposes this goal", date: "2026-11-22T05:00:00Z", kind: "goal" }, { id: "P-1", slug: "hive", summary: "Raise the budget", date: "2026-11-21T05:00:00Z" }] });
  const goal = rows.find((r) => r.key === "proposal:P-9");
  assert.equal(goal.section, "proposals");
  assert.equal(goal.meta, "New project goal");
  assert.equal(goal.card, true);
  const plain = rows.find((r) => r.key === "proposal:P-1");
  assert.equal(plain.card, undefined);
  assert.equal(plain.meta, undefined);
});
