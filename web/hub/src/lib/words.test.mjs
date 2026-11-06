import assert from "node:assert/strict";
import { test } from "node:test";
import { agentName } from "./words.ts";

test("role leads read as their role, project leads as the project's lead", () => {
  assert.equal(agentName("demo-app-lead-security"), "Security lead");
  assert.equal(agentName("demo-app-lead-rsi"), "Self-improvement lead");
  assert.equal(agentName("demo-app-lead-release-notes"), "Release notes lead");
  assert.equal(agentName("demo-app-lead-security", "demo-lib"), "Security lead of demo-app");
  assert.equal(agentName("demo-app-lead"), "demo-app lead");
  assert.equal(agentName("demo-app-lead", "demo-app"), "Project lead");
});

test("workers read as their model and number; codenames, you and the runner as people say them", () => {
  assert.equal(agentName("fable-worker-1"), "Fable worker 1");
  assert.equal(agentName("hub-worker"), "Hub worker");
  assert.equal(agentName("cinder"), "cinder");
  assert.equal(agentName("human"), "You");
  assert.equal(agentName("daemon"), "The runner");
  assert.equal(agentName(""), "");
  assert.equal(agentName(undefined), "");
});
