import assert from "node:assert/strict";
import { test } from "node:test";
import { connectionState, dedupeSuggestions, servesProjects, skillUseText, toolCount, toolCountText, unconnectedSuggestions } from "./connections-state.ts";

test("connectionState: Connected = done, Needs sign-in = waits, Off = paused, Failed, unchecked", () => {
  assert.deepEqual(connectionState({ status: "enabled", auth: "none", last_check_at: "2026-11-01T00:00:00Z", last_check_ok: true, last_check_note: "github 1.2: 3 tools: a, b, c" }), { kind: "connected", status: "connected", label: "Connected", detail: "github 1.2: 3 tools: a, b, c" });
  assert.equal(connectionState({ status: "disabled", auth: "none", last_check_ok: true }).label, "Off");
  assert.equal(connectionState({ status: "disabled", auth: "none", last_check_ok: true }).status, "neutral");
  assert.equal(connectionState({ status: "enabled", auth: "oauth", sign_in: { signed_in: false, note: "run hive mcp login" } }).label, "Needs sign-in");
  assert.equal(connectionState({ status: "enabled", auth: "oauth", sign_in: { signed_in: false } }).status, "waiting");
  assert.equal(connectionState({ status: "enabled", auth: "oauth", last_check_at: "x", last_check_ok: false, last_check_note: "401 unauthorized: run `hive mcp login`" }).kind, "sign_in");
  assert.equal(connectionState({ status: "enabled", auth: "bearer", last_check_at: "x", last_check_ok: false, last_check_note: "connection refused\nmore" }).label, "Failed");
  assert.equal(connectionState({ status: "enabled", auth: "bearer", last_check_at: "x", last_check_ok: false, last_check_note: "connection refused\nmore" }).detail, "connection refused");
  assert.equal(connectionState({ status: "enabled", auth: "none" }).label, "Not checked yet");
  assert.equal(connectionState({ status: "enabled", auth: "none" }).status, "pending");
});

test("toolCount reads the check note", () => {
  assert.equal(toolCount("github 1.2: 12 tools: a, b"), 12);
  assert.equal(toolCount("x: 1 tool: a"), 1);
  assert.equal(toolCount("tools: a, b, c"), 3);
  assert.equal(toolCount(""), null);
  assert.equal(toolCount("connection refused"), null);
  assert.equal(toolCountText("x: 1 tool: a"), "1 tool");
  assert.equal(toolCountText("github 1.2: 12 tools: a, b"), "12 tools");
  assert.equal(toolCountText(null), "");
});

test("servesProjects: empty or * is every project", () => {
  assert.deepEqual(servesProjects(""), []);
  assert.deepEqual(servesProjects("*"), []);
  assert.deepEqual(servesProjects("demo-app"), ["demo-app"]);
  assert.deepEqual(servesProjects("a, b ,,c"), ["a", "b", "c"]);
});

test("skillUseText counts uses", () => {
  assert.equal(skillUseText({ uses: 0 }), "Not used yet");
  assert.equal(skillUseText({}), "Not used yet");
  assert.equal(skillUseText({ uses: 1 }), "Used once");
  assert.equal(skillUseText({ uses: 4 }), "Used 4 times");
});

test("dedupeSuggestions keeps one card per tool and counts the other projects", () => {
  const rows = dedupeSuggestions([
    { name: "github", project: "a", why: "issues" },
    { name: "GitHub", project: "b", why: "prs" },
    { name: "linear", project: "a", why: "tickets" },
    { name: "github", project: "a", why: "dup" },
  ]);
  assert.equal(rows.length, 2);
  assert.equal(rows[0].name, "github");
  assert.equal(rows[0].why, "issues");
  assert.deepEqual(rows[0].also, ["b"]);
  assert.deepEqual(rows[1].also, []);
});

test("unconnectedSuggestions drops tools already connected for the project or shared", () => {
  const rows = [
    { name: "github", project: "a" },
    { name: "linear", project: "a" },
    { name: "notion", project: "b" },
  ];
  const left = unconnectedSuggestions(rows, [
    { name: "GitHub", project: "" },
    { name: "linear", project: "b" },
    { name: "notion", project: "b" },
  ]);
  assert.deepEqual(
    left.map((r) => r.name),
    ["linear"],
  );
});
