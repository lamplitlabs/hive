import assert from "node:assert/strict";
import { test } from "node:test";
import { CLOSED_DIALOG, DENY_COMPUTE_NOTE, answerKey, closeAnswer, computeAnswer, focusAnswer, optionAnswer, pickAnswer, proposalAnswer } from "./answer-dialog.ts";

// A dialog as the console mounts it: one ConfirmDialog keyed by answerKey,
// whose note starts at the spec's `initial` each time it mounts and whose
// button sends the spec's write with the note (components/hive/
// confirm-dialog.tsx keeps the note in its own state, so a new key is a new
// note). It drives the pure state the way use-answer-dialog.ts does.
function harness(specOf) {
  let state = CLOSED_DIALOG;
  let mounted = null; // { key, note }
  const sync = () => {
    const key = answerKey(state);
    if (state.answer === null) mounted = null;
    else if (!mounted || mounted.key !== key) mounted = { key, note: specOf(state.answer).words.initial ?? "" };
  };
  const writes = [];
  const fake = {
    approveCompute: async (id, words) => void writes.push(["approveCompute", id, words]),
    denyCompute: async (id, words) => void writes.push(["denyCompute", id, words]),
    decide: async (id, answer, words) => void writes.push(["decide", id, answer, words]),
    decideOption: async (id, option, words) => void writes.push(["decideOption", id, option, words]),
  };
  return {
    writes,
    pick(a) {
      state = pickAnswer(state, a);
      sync();
    },
    type(text) {
      assert.ok(state.open, "typing needs an open dialog");
      mounted.note = text;
    },
    cancel() {
      state = closeAnswer(state);
      sync();
    },
    get note() {
      return mounted?.note;
    },
    get open() {
      return state.open;
    },
    get key() {
      return answerKey(state);
    },
    async confirm() {
      assert.ok(state.open, "the button is only there while the dialog is open");
      await specOf(state.answer).send(fake, mounted.note);
      state = closeAnswer(state);
      sync();
    },
  };
}

test("a note typed under Deny is never sent with an Approve", async () => {
  const d = harness((a) => computeAnswer("C-7", a));
  d.pick("deny");
  assert.equal(d.note, DENY_COMPUTE_NOTE, "Deny opens with its reason written: one tap");
  d.type("Too dear this month");
  d.cancel();
  d.pick("approve");
  assert.equal(d.note, "", "Approve opens empty");
  await d.confirm();
  assert.deepEqual(d.writes, [["approveCompute", "C-7", ""]]);
});

test("a note typed and cancelled is gone at the next pick of the same answer", async () => {
  const d = harness((a) => proposalAnswer("P-1", a));
  d.pick("no");
  const first = d.key;
  d.type("Not now");
  d.cancel();
  assert.equal(d.key, first, "closing keeps the key, so the dialog closes with its own words");
  assert.equal(d.open, false);
  d.pick("no");
  assert.notEqual(d.key, first, "each opening mounts a fresh dialog");
  assert.equal(d.note, "");
  await d.confirm();
  assert.deepEqual(d.writes, [["decide", "P-1", "no", ""]]);
});

test("each answer's button sends its own write with the words typed under it", async () => {
  const compute = harness((a) => computeAnswer("C-1", a));
  compute.pick("deny");
  await compute.confirm();
  compute.pick("approve");
  compute.type("For the load test");
  await compute.confirm();
  assert.deepEqual(compute.writes, [
    ["denyCompute", "C-1", DENY_COMPUTE_NOTE],
    ["approveCompute", "C-1", "For the load test"],
  ]);

  const proposal = harness((a) => proposalAnswer("P-2", a));
  proposal.pick("yes");
  proposal.type("Go");
  await proposal.confirm();
  assert.deepEqual(proposal.writes, [["decide", "P-2", "yes", "Go"]]);

  const options = [{ label: "Ship it", answer: "yes" }, { label: "Wait a week", answer: "no" }];
  const byLabel = harness((label) => optionAnswer("Q-1", options.find((o) => o.label === label), true));
  byLabel.pick("Wait a week");
  byLabel.type("After the release");
  byLabel.cancel();
  byLabel.pick("Ship it");
  await byLabel.confirm();
  assert.deepEqual(byLabel.writes, [["decideOption", "Q-1", "Ship it", ""]]);

  const yesNo = harness((label) => optionAnswer("Q-2", options.find((o) => o.label === label), false));
  yesNo.pick("Wait a week");
  await yesNo.confirm();
  assert.deepEqual(yesNo.writes, [["decide", "Q-2", "no", ""]]);
});

test("Deny is destructive and needs words; Approve is neither", () => {
  assert.equal(computeAnswer("C", "deny").destructive, true);
  assert.equal(computeAnswer("C", "deny").words.required, true);
  assert.equal(computeAnswer("C", "approve").destructive, false);
  assert.ok(!computeAnswer("C", "approve").words.required);
  assert.equal(proposalAnswer("P", "no").destructive, true);
  assert.equal(proposalAnswer("P", "yes").action, "Yes, accept");
});

test("the focus goes to the one recommended answer, and to none when none or several are", () => {
  assert.equal(focusAnswer([{ label: "Later" }, { label: "Now", recommended: true }]), "Now");
  assert.equal(focusAnswer([{ label: "Yes" }, { label: "No" }]), null);
  assert.equal(focusAnswer([{ label: "A", recommended: true }, { label: "B", recommended: true }]), null);
});

test("the closed dialog has no key of an answer, and closing twice changes nothing", () => {
  assert.equal(answerKey(CLOSED_DIALOG), "none");
  const s = closeAnswer(pickAnswer(CLOSED_DIALOG, "yes"));
  assert.equal(closeAnswer(s), s);
});
