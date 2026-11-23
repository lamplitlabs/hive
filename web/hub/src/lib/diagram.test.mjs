import assert from "node:assert/strict";
import { test } from "node:test";
import { diagramLabel, diagramWords, isDiagram } from "./diagram.ts";

const FLOW = [
  "┌────────┐  task   ┌────────┐",
  "│ owner  │ ──────→ │ lead   │",
  "└────────┘         └───┬────┘",
  "                       │ jobs",
  "                       ↓",
  "                    agents",
].join("\n");

test("a plain block with a line character is a diagram; code and sample output are not", () => {
  assert.equal(isDiagram(FLOW), true);
  assert.equal(isDiagram(FLOW, "text"), true);
  assert.equal(isDiagram("hive status\n  → 3 running\n", "text"), false);
  assert.equal(isDiagram(FLOW, "bash"), false);
});

test("a screen reader gets the words of a diagram in reading order, each once", () => {
  assert.deepEqual(diagramWords(FLOW), ["task", "owner", "lead", "jobs", "agents"]);
  assert.equal(diagramLabel(FLOW), "Diagram: task, owner, lead, jobs, agents");
  assert.equal(diagramLabel("┌──┐\n└──┘"), "Diagram");
});
