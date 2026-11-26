import test from "node:test";
import assert from "node:assert/strict";
import { attentionRows, isFirstRun, runnerKeptRunning, setupSteps } from "./sessions-state.ts";

const hrefs = { settings: (s) => "/settings/#" + (s || ""), connect: (p) => "/settings/#harness-" + p };
const quietInbox = { machine: false, provider: null, down: [], downWhy: [] };
const aliveDaemon = { label: "alive", pause: "off", canResume: true, setup: null };

test("a healthy runner with a provider shows no attention rows", () => {
  assert.deepEqual(attentionRows(quietInbox, aliveDaemon, hrefs), []);
  assert.deepEqual(attentionRows(quietInbox, { ...aliveDaemon, label: "checking" }, hrefs), []);
});

test("a stopped runner asks to start it through the service when it can", () => {
  const rows = attentionRows(quietInbox, { label: "stopped", pause: "off", canResume: true, setup: "start" }, hrefs);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].tone, "failed");
  assert.equal(rows[0].text, "Runner is off");
  assert.equal(rows[0].action, "Start runner");
  assert.deepEqual(rows[0].fix, { kind: "service", action: "start" });
  const noService = attentionRows(quietInbox, { label: "no heartbeat", pause: "off", canResume: true, setup: null }, hrefs);
  assert.deepEqual(noService[0].fix, { kind: "link", href: "/settings/#daemon" });
});

test("a paused runner offers resume, disabled when this console may not", () => {
  const [row] = attentionRows(quietInbox, { label: "paused", pause: "on", canResume: false, setup: null }, hrefs);
  assert.equal(row.action, "Resume");
  assert.deepEqual(row.fix, { kind: "resume", enabled: false });
  assert.match(row.why, /machine that runs it/);
});

test("a working runner that only needs the service step is a warning, not a failure", () => {
  const [row] = attentionRows(quietInbox, { ...aliveDaemon, setup: "finish" }, hrefs);
  assert.equal(row.tone, "warning");
  assert.equal(row.id, "service");
  assert.deepEqual(row.fix, { kind: "service", action: "finish" });
});

test("provider and down inputs follow the runner, each with its own fix", () => {
  const rows = attentionRows({ machine: false, provider: { name: "copilot", label: "GitHub Copilot", expired: true }, down: ["tasks did not load"], downWhy: ["HTTP 500"] }, { label: "stopped", pause: "off", canResume: true, setup: "start" }, hrefs);
  assert.deepEqual(
    rows.map((r) => r.id),
    ["runner", "provider", "down/0"],
  );
  assert.equal(rows[1].text, "GitHub Copilot sign-in expired");
  assert.equal(rows[1].action, "Sign in");
  assert.deepEqual(rows[1].fix, { kind: "link", href: "/settings/#harness-copilot" });
  assert.equal(rows[2].text, "Tasks did not load");
  assert.equal(rows[2].why, "HTTP 500");
  assert.deepEqual(rows[2].fix, { kind: "retry" });
  const unconnected = attentionRows({ ...quietInbox, provider: { name: "openai", label: "OpenAI", expired: false } }, aliveDaemon, hrefs);
  assert.equal(unconnected[0].text, "OpenAI is not connected");
  assert.equal(unconnected[0].action, "Connect");
});

test("first run is no project or no connected provider; a loading harness is not a first run", () => {
  const connected = { label: "GitHub Copilot", loggedIn: true, expired: false };
  assert.equal(isFirstRun({ provider: connected, projects: 2 }), false);
  assert.equal(isFirstRun({ provider: connected, projects: 0 }), true);
  assert.equal(isFirstRun({ provider: null, projects: 2 }), true);
  assert.equal(isFirstRun({ provider: { ...connected, loggedIn: false }, projects: 2 }), true);
  // An expired sign-in is the attention strip's business, not a first run.
  assert.equal(isFirstRun({ provider: { ...connected, expired: true }, projects: 2 }), false);
  assert.equal(isFirstRun({ provider: undefined, projects: 2 }), false);
});

test("the setup card ticks each step off and names the fix for the rest", () => {
  const fresh = setupSteps({ provider: { label: "GitHub Copilot", loggedIn: false, expired: false }, projects: 0, daemon: { label: "no heartbeat", pause: "off", canResume: false, setup: "install" } });
  assert.deepEqual(
    fresh.map((s) => [s.id, s.done, s.action]),
    [
      ["provider", false, "Connect"],
      ["project", false, "Add project"],
      ["running", false, "Keep running"],
    ],
  );
  assert.equal(fresh[0].note, "GitHub Copilot is not signed in yet");
  const expired = setupSteps({ provider: { label: "GitHub Copilot", loggedIn: true, expired: true }, projects: 1, daemon: aliveDaemon });
  assert.deepEqual(expired.map((s) => [s.done, s.action]), [[false, "Sign in"], [true, "Add project"], [true, "Keep running"]]);
  assert.equal(expired[0].note, "GitHub Copilot sign-in expired");
  assert.equal(expired[1].note, "1 project");
  assert.equal(expired[2].note, "The runner is on");
  const paused = setupSteps({ provider: null, projects: 3, daemon: { label: "paused", pause: "on", canResume: true, setup: null } });
  assert.equal(paused[0].note, "The agents think with a model you sign in to");
  assert.deepEqual([paused[2].done, paused[2].action, paused[2].note], [false, "Resume", "You paused the runner"]);
  const finish = setupSteps({ provider: null, projects: 3, daemon: { ...aliveDaemon, setup: "finish" } });
  assert.deepEqual([finish[2].done, finish[2].action], [false, "Finish setup"]);
});

test("the runner counts as kept running only when on, not paused and with no service step", () => {
  assert.equal(runnerKeptRunning(aliveDaemon), true);
  assert.equal(runnerKeptRunning({ ...aliveDaemon, label: "checking" }), true);
  assert.equal(runnerKeptRunning({ ...aliveDaemon, setup: "finish" }), false);
  assert.equal(runnerKeptRunning({ ...aliveDaemon, pause: "on", label: "paused" }), false);
  assert.equal(runnerKeptRunning({ ...aliveDaemon, label: "stopped" }), false);
});
