import assert from "node:assert/strict";
import { test } from "node:test";
import { LOCKDOWN_BADGE, LOCKDOWN_BAR, lockdownFlipMessage, lockdownHint, lockdownOf } from "./lockdown-state.ts";

test("the live state's word wins over the row, and the row is the fallback", () => {
  assert.equal(lockdownOf({ lockdown: false }, { lockdown: true }), true);
  assert.equal(lockdownOf({ lockdown: true }, { lockdown: false }), false);
  assert.equal(lockdownOf({ lockdown: true }, null), true);
  assert.equal(lockdownOf({ lockdown: true }), true);
  assert.equal(lockdownOf({ lockdown: false }), false);
  // An older hub's live state has no lockdown field: the row decides.
  assert.equal(lockdownOf({ lockdown: true }, {}), true);
  // A row an older hub wrote without the field reads as off.
  assert.equal(lockdownOf({}), false);
});

test("the words say what the sandbox binds, in plain words", () => {
  assert.match(lockdownHint(true), /no network/i);
  assert.match(lockdownHint(true), /worktree/);
  assert.match(lockdownHint(true), /credentials/);
  assert.match(lockdownHint(false), /as usual/);
  assert.match(LOCKDOWN_BAR, /sandbox/);
  assert.match(LOCKDOWN_BAR, /Landing is yours/);
  assert.equal(LOCKDOWN_BADGE, "in lockdown (sandbox)");
});

test("a flip says the session restarts and whether the running turn finishes first", () => {
  assert.match(lockdownFlipMessage(true, false), /^Lockdown on from the next turn:/);
  assert.match(lockdownFlipMessage(true, true), /this one finishes as it started/);
  assert.match(lockdownFlipMessage(false, false), /^Lockdown off/);
  assert.match(lockdownFlipMessage(false, false), /outside open again/);
});
