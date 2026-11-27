import { test } from "node:test";
import assert from "node:assert/strict";
import { pendingChoices } from "./pending-question.ts";

test("a pending question is answered with one pick, the recommended first and marked, nothing typed", () => {
  const { choices, typed } = pendingChoices({
    choices: ["tabs - indent with tabs", "spaces - indent with four spaces"],
    recommended: "spaces - indent with four spaces",
  });
  assert.equal(typed, false, "no box to type into when the agent named choices");
  assert.deepEqual(
    choices.map((c) => [c.label, c.effect, c.recommended]),
    [
      ["spaces", "indent with four spaces", true],
      ["tabs", "indent with tabs", false],
    ],
  );
  assert.equal(choices[0].answer, "spaces - indent with four spaces", "the answer sent is the choice as written");
  // The recommended one may be named by its label, in any case.
  assert.equal(pendingChoices({ choices: ["red", "Blue - the sea"], recommended: "blue" }).choices[0].label, "Blue");
});

test("an older question without choices is still answered by typing; one without a recommended marks none", () => {
  assert.deepEqual(pendingChoices({ choices: [] }), { choices: [], typed: true });
  assert.deepEqual(pendingChoices({}), { choices: [], typed: true });
  const old = pendingChoices({ choices: ["red", "blue"] });
  assert.equal(old.typed, false);
  assert.deepEqual(old.choices.map((c) => [c.label, c.recommended]), [["red", false], ["blue", false]]);
});
