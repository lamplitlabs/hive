import { test } from "node:test";
import assert from "node:assert/strict";
import { contextChip, groupIcon, modelFamily } from "./settings-state.ts";

test("groupIcon names a lucide icon per settings group", () => {
  assert.equal(groupIcon("general"), "Settings2");
  assert.equal(groupIcon("appearance"), "Palette");
  assert.equal(groupIcon("harness"), "Cpu");
  assert.equal(groupIcon("daemon"), "Play");
  assert.equal(groupIcon("notify"), "Bell");
  assert.equal(groupIcon("credential"), "KeyRound");
  assert.equal(groupIcon("compute"), "Cloud");
  assert.equal(groupIcon("stop"), "ShieldAlert");
  assert.equal(groupIcon("nonsense"), "SlidersHorizontal");
});

test("modelFamily reads the family word out of a model id", () => {
  assert.equal(modelFamily("claude-opus-5-5"), "Opus");
  assert.equal(modelFamily("claude-sonnet-5-5"), "Sonnet");
  assert.equal(modelFamily("claude-haiku-4"), "Haiku");
  assert.equal(modelFamily("gpt-5.1"), "GPT");
  assert.equal(modelFamily("o3-mini"), "GPT");
  assert.equal(modelFamily("gemini-3-pro"), "Gemini");
  assert.equal(modelFamily("llama4:16x17b"), "Llama");
  assert.equal(modelFamily("qwen3-coder:30b"), "Other");
  assert.equal(modelFamily(""), "Other");
  assert.equal(modelFamily(undefined), "Other");
});

test("contextChip shortens tokens to k or M", () => {
  assert.equal(contextChip(200000), "200k");
  assert.equal(contextChip(1000000), "1M");
  assert.equal(contextChip(262144), "262k");
  assert.equal(contextChip(128000), "128k");
  assert.equal(contextChip(1500000), "1.5M");
  assert.equal(contextChip(0), "");
  assert.equal(contextChip(undefined), "");
  assert.equal(contextChip(null), "");
});
