import assert from "node:assert/strict";
import { test } from "node:test";
import { glanceLine, glanceTiles } from "./glance-state.ts";

const base = () => ({
  inbox: 0,
  inboxChecking: false,
  agentsDeciding: 0,
  workingAgents: 0,
  things: 0,
  jobs: { running: 0, queued: 0 },
  runnerChecking: false,
  runnerDown: false,
  runnerLabel: "Runner on",
  runnerTone: "ok",
  runnerNote: "",
  projects: [{ slug: "a" }, { slug: "b" }],
  agentsActive: 3,
});

test("a quiet machine reads all caught up, nobody working, runner on", () => {
  const t = glanceTiles(base());
  assert.equal(glanceLine(t), "Waiting for you: 0 · Working now: Nobody · This machine: Runner on · Projects: 2");
  assert.equal(t[0].tone, "neutral");
  assert.equal(t[0].sub, "All caught up");
  assert.equal(t[1].sub, "Give a task when you're ready");
  assert.equal(t[2].tone, "ok");
  assert.equal(t[2].sub, "Agents can work");
  assert.equal(t[3].sub, "3 agents active across them");
});

test("things waiting for the owner take the marigold tone and name the agents' own questions", () => {
  const t = glanceTiles({ ...base(), inbox: 4, agentsDeciding: 2 });
  assert.equal(t[0].value, "4");
  assert.equal(t[0].tone, "you");
  assert.equal(t[0].sub, "Plus 2 questions the agents settle");
  const one = glanceTiles({ ...base(), inbox: 1 });
  assert.equal(one[0].sub, "Each one waits until you decide");
});

test("while the inbox loads the tile says so instead of a false zero", () => {
  const t = glanceTiles({ ...base(), inboxChecking: true });
  assert.equal(t[0].value, "…");
  assert.equal(t[0].sub, "Checking what's waiting");
});

test("agents at work are counted with what they hold, and the queue joins the line", () => {
  const t = glanceTiles({ ...base(), workingAgents: 2, things: 3, jobs: { running: 2, queued: 5 } });
  assert.equal(t[1].value, "2 agents");
  assert.equal(t[1].sub, "3 things picked up · 5 jobs queued");
  assert.equal(t[1].tone, "info");
});

test("a running job with no claim still counts as work; a queue alone is named while idle", () => {
  const t = glanceTiles({ ...base(), jobs: { running: 1, queued: 0 } });
  assert.equal(t[1].value, "1 agent");
  assert.equal(t[1].sub, "1 job running");
  const idle = glanceTiles({ ...base(), jobs: { running: 0, queued: 8 } });
  assert.equal(idle[1].value, "Nobody");
  assert.equal(idle[1].sub, "8 jobs queued");
});

test("a runner that does not answer is named as unreachable, in the bad tone", () => {
  const t = glanceTiles({ ...base(), runnerDown: true, jobs: null, runnerLabel: "Runner off", runnerTone: "neutral" });
  assert.equal(t[2].value, "Runner unreachable");
  assert.equal(t[2].tone, "bad");
});

test("a pending update is the runner's line; a paused runner asks to open the controls", () => {
  const up = glanceTiles({ ...base(), runnerNote: "A new build waits for 2 runs to finish" });
  assert.equal(up[2].sub, "A new build waits for 2 runs to finish");
  const paused = glanceTiles({ ...base(), runnerLabel: "Runner paused", runnerTone: "warn" });
  assert.equal(paused[2].value, "Runner paused");
  assert.equal(paused[2].sub, "Open the controls to fix it");
  assert.equal(paused[2].tone, "warn");
});

test("projects needing attention turn the tile warn; none at all asks for the first", () => {
  const t = glanceTiles({ ...base(), projects: [{ slug: "a", missing: true }, { slug: "b" }, { slug: "c", stop_present: true }] });
  assert.equal(t[3].value, "3");
  assert.equal(t[3].sub, "2 projects need attention");
  assert.equal(t[3].tone, "warn");
  const none = glanceTiles({ ...base(), projects: [], agentsActive: 0 });
  assert.equal(none[3].sub, "Add your first one");
  const quiet = glanceTiles({ ...base(), agentsActive: 0 });
  assert.equal(quiet[3].sub, "All in order");
});

test("missing arrays are treated as empty", () => {
  const t = glanceTiles({ ...base(), workingAgents: undefined, things: undefined, projects: undefined, jobs: null });
  assert.equal(t[1].value, "Nobody");
  assert.equal(t[3].value, "0");
});
