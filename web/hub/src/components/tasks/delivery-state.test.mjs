import test from "node:test";
import assert from "node:assert/strict";
import { deliveryView } from "./delivery-state.ts";

test("a task's pull request: open links PR #N, merged links it, held names the note", () => {
  const open = deliveryView({ state: "pr_open", url: "https://g/pr/7", number: 7 });
  assert.deepEqual(open.link, { href: "https://g/pr/7", text: "PR #7" });
  assert.match(open.after, /open/);
  assert.equal(open.tone, "ok");
  const merged = deliveryView({ state: "merged", url: "https://g/pr/7", number: 7 });
  assert.equal(merged.before, "Merged: ");
  assert.equal(merged.link.text, "PR #7");
  const held = deliveryView({ state: "held", note: "gh is not logged in" });
  assert.equal(held.before, "Held: gh is not logged in");
  assert.equal(held.tone, "warn");
  assert.equal(deliveryView({ state: "none" }), null);
  assert.equal(deliveryView(null), null);
});

test("older landing states still read, without landing words", () => {
  for (const state of ["waiting", "landed", "asked", "pushed", "pr"]) {
    const v = deliveryView({ state, waiting: 2, number: 3, url: "u" });
    assert.ok(v, state);
    assert.doesNotMatch(v.before + v.after, /\bland/i, state);
  }
});
