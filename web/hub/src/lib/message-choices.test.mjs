import { test } from "node:test";
import assert from "node:assert/strict";
import { isDefaultChoice, orderChoices } from "./message-choices.ts";

test("the recommended choice comes first and is the default, wherever the agent wrote it", () => {
  const choices = [{ label: "Merge it" }, { label: "Squash it", note: "one commit on main", recommended: true }, { label: "Leave it" }];
  const ordered = orderChoices(choices);
  assert.deepEqual(ordered.map((c) => c.label), ["Squash it", "Merge it", "Leave it"]);
  assert.ok(isDefaultChoice(ordered, ordered[0]));
  assert.ok(!isDefaultChoice(ordered, ordered[1]), "the first written is not the default when another is recommended");
  assert.deepEqual(choices.map((c) => c.label), ["Merge it", "Squash it", "Leave it"], "the message's own list is left as it is");
});

test("a message from an older hub marks none: the first stays the default", () => {
  const old = [{ label: "Yes" }, { label: "No" }];
  assert.deepEqual(orderChoices(old), old);
  assert.ok(isDefaultChoice(old, old[0]));
  assert.ok(!isDefaultChoice(old, old[1]));
  assert.deepEqual(orderChoices(undefined), []);
});
