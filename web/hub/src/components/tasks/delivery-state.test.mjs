import test from "node:test";
import assert from "node:assert/strict";
import { deliveryView } from "./delivery-state.ts";

const pr = { url: "https://g/pr/7", number: 7 };
const line = (v) => v.before + (v.link ? v.link.text : "") + v.after;

test("a task's pull request: open links PR #N, merged links it, held names the note", () => {
  const open = deliveryView({ state: "pr_open", ...pr });
  assert.deepEqual(open.link, { href: "https://g/pr/7", text: "PR #7" });
  assert.equal(line(open), "PR #7 open, waiting for review.");
  assert.equal(open.tone, "neutral");
  assert.equal(open.canMerge, false);
  const merged = deliveryView({ state: "merged", ...pr });
  assert.equal(merged.before, "Merged: ");
  assert.equal(merged.link.text, "PR #7");
  const held = deliveryView({ state: "held", note: "gh is not logged in" });
  assert.equal(held.before, "Held: gh is not logged in");
  assert.equal(held.tone, "warn");
  // A conflict is the owner's: the runner waits for the work to change.
  const mine = deliveryView({ state: "held_for_owner", note: "does not merge cleanly; this one is for you" });
  assert.equal(line(mine), "Held for you: does not merge cleanly; this one is for you. The runner tries again once the work changes.");
  assert.equal(mine.tone, "warn");
  assert.equal(line(deliveryView({ state: "held_for_owner" })), "Held for you. The runner tries again once the work changes.");
  assert.equal(deliveryView({ state: "none" }), null);
  assert.equal(deliveryView(null), null);
});

test("the review states read as one sentence each and only three offer Merge", () => {
  const cases = [
    [{ state: "in_review" }, "PR #7 is being reviewed.", "neutral", false],
    [
      { state: "needs_owner", reasons: ["42 files", "touches migrations"], note: "Left to you", can_merge: true },
      "Needs your review: PR #7 \u2014 42 files; touches migrations. Left to you.",
      "warn",
      true,
    ],
    [{ state: "needs_owner", can_merge: true }, "Needs your review: PR #7.", "warn", true],
    [{ state: "merge_asked" }, "Merging PR #7\u2026", "neutral", false],
    [
      { state: "merge_waiting", note: "approved; the merge waits for the checks still running (ci/build)" },
      "Approved: PR #7 \u2014 the merge waits for the checks still running (ci/build).",
      "neutral",
      true,
    ],
    [{ state: "merge_waiting" }, "Approved: PR #7 merges soon.", "neutral", true],
    [
      { state: "changes_requested", note: "tests missing", follow_up: "JOB-9" },
      "Changes requested on PR #7: tests missing. Follow-up JOB-9 is queued.",
      "neutral",
      false,
    ],
    [{ state: "changes_requested" }, "Changes requested on PR #7.", "neutral", false],
    [
      { state: "review_failed", note: "gh: rate limited.", can_merge: true },
      "The review of PR #7 failed: gh: rate limited.",
      "warn",
      true,
    ],
  ];
  for (const [input, text, tone, canMerge] of cases) {
    const v = deliveryView({ ...pr, ...input });
    assert.ok(v, input.state);
    assert.equal(line(v), text, input.state);
    assert.equal(v.link.href, "https://g/pr/7", input.state);
    assert.equal(v.tone, tone, input.state);
    assert.equal(v.canMerge, canMerge, input.state);
    assert.doesNotMatch(text, /\bland/i, input.state);
  }
});

test("without a url the pull request is named in plain text, and Merge follows the state when the hub is older than can_merge", () => {
  const v = deliveryView({ state: "needs_owner", number: 3, reasons: ["big"] });
  assert.equal(v.link, null);
  assert.equal(v.before, "Needs your review: PR #3 \u2014 big.");
  assert.equal(v.canMerge, true);
  const open = deliveryView({ state: "pr_open" });
  assert.equal(open.before, "The pull request open, waiting for review.");
  // The hub's word wins over the state: a merge already asked is not offered again.
  assert.equal(deliveryView({ state: "review_failed", can_merge: false }).canMerge, false);
});

test("older landing states still read, without landing words", () => {
  for (const state of ["waiting", "landed", "asked", "pushed", "pr"]) {
    const v = deliveryView({ state, waiting: 2, number: 3, url: "u" });
    assert.ok(v, state);
    assert.doesNotMatch(v.before + v.after, /\bland/i, state);
  }
});
