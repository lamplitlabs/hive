import assert from "node:assert/strict";
import { test } from "node:test";
import { KINDS, KIND_LABEL, KIND_TINT, TINT_CLASS, isKind, kindFor, tintFor } from "./kind-tile-state.ts";

test("every kind has a tint, a label and a tint class", () => {
  for (const k of KINDS) {
    assert.ok(KIND_TINT[k], k + " has a tint");
    assert.ok(KIND_LABEL[k], k + " has a label");
    assert.match(TINT_CLASS[KIND_TINT[k]], /^bg-kind-[a-z]+-soft text-kind-[a-z]+$/);
  }
});

test("tints follow the spec table", () => {
  assert.equal(tintFor("session"), "teal");
  assert.equal(tintFor("task"), "blue");
  assert.equal(tintFor("proposal"), "violet");
  assert.equal(tintFor("question"), "marigold");
  assert.equal(tintFor("suggestion"), "violet");
  assert.equal(tintFor("rule"), "violet");
  assert.equal(tintFor("run"), "teal");
  assert.equal(tintFor("machine"), "slate");
  assert.equal(tintFor("tool"), "slate");
  assert.equal(tintFor("skill"), "green");
  assert.equal(tintFor("automation"), "blue");
  assert.equal(tintFor("memory"), "pink");
  for (const k of ["plan", "guide", "feature", "work", "record"]) assert.equal(tintFor(k), "grey", k);
  assert.equal(tintFor("project"), "slate");
  assert.equal(tintFor("vote"), "violet");
});

test("a decision is marigold only while it waits on the owner", () => {
  assert.equal(tintFor("decision"), "grey");
  assert.equal(tintFor("decision", false), "grey");
  assert.equal(tintFor("decision", true), "marigold");
  assert.equal(tintFor("task", true), "blue", "waiting changes nothing else");
});

test("kindFor maps the store's record kinds", () => {
  assert.equal(kindFor("adr"), "decision");
  assert.equal(kindFor("plan"), "plan");
  assert.equal(kindFor("wu"), "work");
  assert.equal(kindFor("feature"), "feature");
  assert.equal(kindFor("memory"), "memory");
  assert.equal(kindFor("skill"), "skill");
  assert.equal(kindFor("guide"), "guide");
  assert.equal(kindFor("workflow"), "automation");
  for (const k of ["context", "architecture", "evolution", "template", "prompt"]) assert.equal(kindFor(k), "record", k);
});

test("kindFor maps board post kinds and hub objects", () => {
  assert.equal(kindFor("proposal"), "proposal");
  assert.equal(kindFor("question"), "question");
  assert.equal(kindFor("suggestion"), "suggestion");
  assert.equal(kindFor("amendment"), "rule");
  assert.equal(kindFor("rule"), "rule");
  assert.equal(kindFor("vote"), "vote");
  assert.equal(kindFor("conversation"), "session");
  assert.equal(kindFor("job"), "run");
  assert.equal(kindFor("runner"), "machine");
  assert.equal(kindFor("connection"), "tool");
});

test("kindFor is case- and separator-insensitive and falls back to a grey record", () => {
  assert.equal(kindFor("Proposal"), "proposal");
  assert.equal(kindFor("  ADR "), "decision");
  assert.equal(kindFor("rule-change"), "record", "unknown compound names fall back");
  assert.equal(kindFor("nonsense"), "record");
  assert.equal(kindFor(""), "record");
  assert.equal(kindFor(null), "record");
  assert.equal(kindFor(undefined), "record");
  assert.equal(tintFor(kindFor("nonsense")), "grey");
});

test("isKind guards free text", () => {
  assert.equal(isKind("task"), true);
  assert.equal(isKind("adr"), false);
});
