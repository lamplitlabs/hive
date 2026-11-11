import { test } from "node:test";
import assert from "node:assert/strict";
import { MASK, canSaveSecret, normalizeSecretName, secretNameProblem, setByWords, vaultNote } from "./secrets-state.ts";

test("a secret name is an upper-case env name PULSE does not set itself", () => {
  assert.equal(secretNameProblem(""), "");
  assert.equal(secretNameProblem("GITHUB_TOKEN"), "");
  assert.equal(secretNameProblem("_X1"), "");
  assert.match(secretNameProblem("github-token"), /upper-case/);
  assert.match(secretNameProblem("1TOKEN"), /upper-case/);
  assert.match(secretNameProblem("PULSE_JOB"), /PULSE sets/);
  assert.match(secretNameProblem("PATH"), /PULSE sets/);
  assert.match(secretNameProblem("A".repeat(65)), /64/);
});

test("a typed name is normalized the way the owner means it", () => {
  assert.equal(normalizeSecretName(" npm token "), "NPM_TOKEN");
  assert.equal(normalizeSecretName("sentry-dsn"), "SENTRY_DSN");
});

test("the form saves a valid name with a value within the cap only", () => {
  assert.equal(canSaveSecret("NPM_TOKEN", "abc", 65536), true);
  assert.equal(canSaveSecret("NPM_TOKEN", "", 65536), false);
  assert.equal(canSaveSecret("npm", "abc", 65536), false);
  assert.equal(canSaveSecret("NPM_TOKEN", "x".repeat(10), 4), false);
});

test("rows say who set a secret and never show its value", () => {
  assert.equal(setByWords("human"), "set by you");
  assert.equal(setByWords(""), "set by you");
  assert.equal(setByWords("delta-47"), "set by delta-47");
  assert.equal(MASK.length, 8);
  assert.match(vaultNote(0), /^No secrets yet\./);
  assert.match(vaultNote(1), /^1 secret\./);
  assert.match(vaultNote(3), /secrets:/);
});
