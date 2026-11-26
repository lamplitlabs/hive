import assert from "node:assert/strict";
import { test } from "node:test";
import { askOptions, askTitle, decisionLabels, isOpenQuestion, questionAnswers, sortOwnerQuestions } from "./owner-questions.ts";

test("stop questions lead, stale threads trail, newest first within a group", () => {
  const rows = [
    { id: "a", stop: false, stale: false, date: "2026-09-03T00:00:00Z" },
    { id: "b", stop: true, stale: true, date: "2026-09-04T00:00:00Z" },
    { id: "c", stop: true, stale: false, date: "2026-09-01T00:00:00Z" },
    { id: "d", stop: false, stale: false, date: "2026-09-05T00:00:00Z" },
    { id: "e", stop: true, stale: false, date: "2026-09-02T00:00:00Z" },
  ];
  assert.deepEqual(sortOwnerQuestions(rows).map((r) => r.id), ["e", "c", "b", "d", "a"]);
  assert.deepEqual(rows.map((r) => r.id), ["a", "b", "c", "d", "e"], "the input is left alone");
});

test("ties on date fall back to the id so the order is stable across reloads", () => {
  const rows = [
    { id: "y", stop: false, stale: false, date: "2026-09-03T00:00:00Z" },
    { id: "x", stop: false, stale: false, date: "2026-09-03T00:00:00Z" },
  ];
  assert.deepEqual(sortOwnerQuestions(rows).map((r) => r.id), ["x", "y"]);
});

test("a post labels the owner's buttons with its Yes: and No: lines", () => {
  const body = "Dependabot #20 in web/hub: a major upgrade.\n\nYes: Upgrade it\nNo: Leave it to me.\n\nYes queues a job that moves next to 17.x.";
  assert.deepEqual(decisionLabels(body), { yes: "Upgrade it", no: "Leave it to me" });
  assert.deepEqual(decisionLabels("Pick one."), { yes: "Yes", no: "No" });
  assert.deepEqual(decisionLabels(undefined, { yes: "Yes, carry on", no: "No, keep it paused" }), { yes: "Yes, carry on", no: "No, keep it paused" });
  // A sentence after the word is the explanation, not a label; bold marks are allowed.
  assert.deepEqual(decisionLabels("**Yes**: Do it\nNo: " + "x".repeat(41)), { yes: "Do it", no: "No" });
  assert.deepEqual(decisionLabels("Yes queues a job.\nNo leaves it."), { yes: "Yes", no: "No" });
});

test("the card names a post by its kind", () => {
  assert.equal(askTitle("question", "alpha"), "Question from alpha");
  assert.equal(askTitle(undefined, "alpha"), "Question from alpha");
  assert.equal(askTitle("finding", "daemon"), "Your call, from daemon");
  assert.equal(askTitle("handoff", "alpha"), "Handed to you by alpha");
  assert.equal(askTitle("suggestion", "alpha"), "Proposed to you by alpha");
});

test("a which/what/how question is answered in words, a should-we question with yes or no", () => {
  assert.equal(isOpenQuestion("Which payment sandbox should the checkout fixtures hit?", "Two exist in the vault."), true);
  assert.equal(isOpenQuestion("  what should the retry limit be", ""), true);
  assert.equal(isOpenQuestion("Should we upgrade next to 17?", ""), false);
  assert.equal(isOpenQuestion("May it carry on?", undefined), false);
  // A post that names its own buttons keeps them, whatever it asks.
  assert.equal(isOpenQuestion("Which way: upgrade now?", "Yes: Upgrade it\nNo: Leave it to me"), false);
  assert.equal(isOpenQuestion("Whichever works", ""), false);
});

// TASK-129: every question to the owner carries its answers and a
// recommended one; nothing is typed.
test("a post's Option: lines and its Recommended: line are the answers, recommended first", () => {
  const body = "The fixtures need a sandbox.\n\nOption: Stripe test mode - the fixtures call Stripe's sandbox\nOption: A local fake - no network\nRecommended: A local fake - the checks stay offline\n";
  const { options, why } = askOptions(body);
  assert.deepEqual(options.map((o) => [o.label, o.effect, o.answer, o.recommended]), [
    ["A local fake", "no network", "", true],
    ["Stripe test mode", "the fixtures call Stripe's sandbox", "", false],
  ]);
  assert.equal(why, "the checks stay offline");
  const yn = askOptions("Yes: Upgrade it - queues the upgrade\nNo: Leave it to me\nRecommended: No - the major breaks the API");
  assert.deepEqual(yn.options.map((o) => [o.label, o.answer, o.recommended]), [["Leave it to me", "no", true], ["Upgrade it", "yes", false]]);
  // A sentence after Yes: is the explanation, not an answer.
  assert.equal(askOptions("Yes: " + "x".repeat(61) + "\nNo: Keep it").options.length, 1);
});

test("a question with answers and a recommended one is picked; without, it is the lead's", () => {
  const ready = questionAnswers({ body: "Yes: Rename it\nNo: Keep it\nRecommended: Yes - nothing else uses the old name" }, false);
  assert.equal(ready.kind, "pick");
  assert.deepEqual(ready.options.map((o) => o.label), ["Rename it", "Keep it"]);
  assert.equal(ready.options[0].recommended, true);
  assert.equal(ready.byLabel, true);
  assert.equal(ready.why, "nothing else uses the old name");
  // No answers at all, or answers with none recommended: never a text box.
  assert.deepEqual(questionAnswers({ body: "Which payment sandbox should the fixtures hit?" }, false), { kind: "with-lead", missing: "it names no answers to pick" });
  assert.deepEqual(questionAnswers({ body: "Yes: Rename it\nNo: Keep it" }, false), { kind: "with-lead", missing: "it names no recommended answer" });
  // The hub's reading wins when it sends one.
  const hub = questionAnswers({ body: "", options: [{ label: "A", effect: "", answer: "", recommended: false }, { label: "B", effect: "", answer: "", recommended: true }], recommended_why: "safer" }, false);
  assert.equal(hub.kind, "pick");
  assert.equal(hub.why, "safer");
  assert.deepEqual(questionAnswers({ body: "", needs_options: "no recommended answer" }, false), { kind: "with-lead", missing: "no recommended answer" });
});

test("a stop of the loop always offers keep paused (recommended) and carry on", () => {
  const a = questionAnswers({ body: "evolution constraint violation" }, true, { yes: "Yes, carry on", no: "No, keep it paused" });
  assert.equal(a.kind, "pick");
  assert.deepEqual(a.options.map((o) => [o.label, o.answer, o.recommended]), [["No, keep it paused", "no", true], ["Yes, carry on", "yes", false]]);
  assert.equal(a.byLabel, false);
  // A stop post that names its own answers keeps them.
  const own = questionAnswers({ body: "Yes: Carry on - the loop resumes\nNo: Keep it paused\nRecommended: No - review the gate first" }, true);
  assert.equal(own.byLabel, true);
  assert.equal(own.options[0].label, "Keep it paused");
});

test("a which-question that names its answers is not answered in words", () => {
  assert.equal(isOpenQuestion("Which sandbox?", "Option: EU\nOption: US\nRecommended: US - billing uses it"), false);
});
