import assert from "node:assert/strict";
import { register } from "node:module";
import { test } from "node:test";

// The self-improvement gate's one human step per project (the inbox's gate
// card, the project hero), pure: no package needed, so it runs in the
// package-less measure clone too.
register("../project/tsx-test-loader.mjs", import.meta.url);

const { RECORD_WORD, foldSteps, gateStep, noneStep, splitAlternative } = await import("./evolve-gate.ts");

const ref = (id) => ({ id, rev: 1, seeded: false, seeded_from: "" });
const gate = (evolve_next, extra = {}) => ({ fixtures_enabled: 6, baseline: true, evolve_level: "L1", evolve_next, ...extra });

test("noneStep reads 'none' with or without the daemon's reason, and nothing else", () => {
  assert.deepEqual(noneStep("none"), { none: true, why: "" });
  assert.deepEqual(noneStep("  none - the daemon measures the baseline "), { none: true, why: "the daemon measures the baseline" });
  assert.deepEqual(noneStep("none: waiting"), { none: true, why: "waiting" });
  assert.deepEqual(noneStep("nonetheless"), { none: false, why: "" });
  assert.deepEqual(noneStep("hive evolve setup"), { none: false, why: "" });
});

test("splitAlternative separates the template copy from the write-your-own command", () => {
  assert.deepEqual(splitAlternative("hive evolve seed demo  (template copy), or hive record new evolution --title Constitution"), { command: "hive evolve seed demo", alt: "hive record new evolution --title Constitution" });
  assert.deepEqual(splitAlternative(" hive evolve seed demo "), { command: "hive evolve seed demo", alt: "" });
});

test("no gate, no step, a step the daemon owns or a baseline in flight ask nothing of the owner", () => {
  assert.equal(gateStep("demo", null), null);
  assert.equal(gateStep("demo", gate("")), null);
  assert.equal(gateStep("demo", gate("none - measuring")), null);
  assert.equal(gateStep("demo", gate("hive job add --kind experiment")), null);
  assert.equal(gateStep("demo", gate("hive evolve setup", { baseline: false, baseline_job: { id: "job-1", status: "running" } })), null);
  assert.equal(gateStep("demo", gate("hive evolve setup", { baseline: false, baseline_failed_at: "2026-11-09T08:00:00Z" })), null);
  assert.equal(gateStep("demo", gate("hive something else")), null);
});

test("the seed step names the one record a project lacks, or both", () => {
  const both = gateStep("demo", gate("hive evolve seed demo  (template copy), or hive record new evolution", { constitution: null, evaluators: null }));
  assert.equal(both.kind, "records");
  assert.equal(both.label, "add the self-improvement rules");
  assert.equal(both.title, "Add the self-improvement rules for demo");
  assert.match(both.body, /has neither yet/);
  assert.equal(both.command, "hive evolve seed demo");
  assert.equal(both.alt, "hive record new evolution");
  const con = gateStep("demo", gate("hive evolve seed demo", { constitution: null, evaluators: ref("EVO-evaluators-demo") }));
  assert.equal(con.label, "add a constitution");
  assert.equal(con.title, "Add a constitution for demo");
  assert.match(con.body, new RegExp("has no " + RECORD_WORD.constitution.toLowerCase() + " yet"));
  assert.equal("alt" in con, false);
  // A row that does not say (an older hub) falls back to the title the command names.
  const evaFromCmd = gateStep("demo", gate("hive evolve seed demo --title Evaluators"));
  assert.equal(evaFromCmd.label, "add scoring rules");
  assert.match(evaFromCmd.body, /has no scoring rules yet/);
  assert.equal(gateStep("demo", gate("hive evolve seed demo")).label, "add a constitution");
});

test("the constitution, starter fixtures and private tests each get their own card", () => {
  const c = gateStep("demo", gate("hive record new evolution --title Constitution"));
  assert.deepEqual([c.kind, c.label, c.title, c.command, c.level, c.slug], ["constitution", "write a constitution", "Write a constitution for demo", "hive record new evolution --title Constitution", "L1", "demo"]);
  const s = gateStep("demo", gate("hive evolve fixture seed demo"));
  assert.deepEqual([s.kind, s.label, s.title], ["seed", "add test cases", "Add starter test cases for demo"]);
  const h = gateStep("demo", gate("hive evolve setup --heldout"));
  assert.deepEqual([h.kind, h.label, h.title], ["heldout", "set up private tests", "Set up two private tests"]);
  assert.match(h.body, /Self-improvement for demo starts/);
});

test("foldSteps folds the per-machine private tests into one card where the first stood", () => {
  const h = (slug) => gateStep(slug, gate("hive evolve setup"));
  const seed = gateStep("tool", gate("hive evolve fixture seed tool"));
  const one = [seed, h("demo")];
  assert.equal(foldSteps(one), one, "a single private-tests step stays as it is");
  const folded = foldSteps([seed, h("demo"), gateStep("web", gate("hive record new evolution")), h("web"), h("api")]);
  assert.deepEqual(folded.map((s) => s.kind), ["seed", "heldout", "constitution"]);
  assert.deepEqual(folded[1].slugs, ["demo", "web", "api"]);
  assert.match(folded[1].body, /serves demo, web and api;/);
});
