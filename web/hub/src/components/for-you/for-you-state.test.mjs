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
