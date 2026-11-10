import test from "node:test";
import assert from "node:assert/strict";
import { attentionRows, dateLine, greetingFor, greetingLine, isFirstRun, itemProject, needsYou, needsYouWord, projectLine, runnerKeptRunning, seeAllLabel, setupSteps, topFive, weekNumbers } from "./home-state.ts";

const hrefs = { settings: (s) => "/settings/#" + (s || ""), connect: (p) => "/settings/#harness-" + p };
const quietInbox = { machine: false, provider: null, down: [], downWhy: [] };
const aliveDaemon = { label: "alive", pause: "off", canResume: true, setup: null };

test("greeting follows the hour and carries the name when there is one", () => {
  assert.equal(greetingFor(2), "Good evening");
  assert.equal(greetingFor(8), "Good morning");
  assert.equal(greetingFor(13), "Good afternoon");
  assert.equal(greetingFor(20), "Good evening");
  assert.equal(greetingLine(8, "Manish"), "Good morning, Manish");
  assert.equal(greetingLine(8, "  "), "Good morning");
  assert.equal(greetingLine(8, null), "Good morning");
});

test("the date line is weekday, day and month", () => {
  assert.equal(dateLine(new Date(2026, 10, 6)), "Friday 6 November");
  assert.equal(dateLine(new Date(2026, 10, 5)), "Thursday 5 November");
});

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

const items = (over = {}) => ({
  decisions: [],
  proposals: [],
  questions: [],
  compute: [],
  work: [],
  drafts: [],
  steps: [],
  ...over,
});

test("needs-you orders questions and decisions first, then proposals, compute, work, drafts, steps", () => {
  const out = needsYou(
    items({
      steps: [{ kind: "constitution", slug: "demo", title: "Approve the constitution", label: "approve" }],
      drafts: [{ slug: "demo", r: { id: "EVO-constitution", rev: 1, draft: { author: "iris", at: "2026-11-05T10:00:00Z" } } }],
      work: [{ id: "SUG-001", project: "demo", created_at: "2026-11-05T09:00:00Z", updated_at: "" }],
      compute: [{ id: "CR-1", project: "demo", at: "2026-11-05T08:00:00Z" }],
      proposals: [
        { id: "p-old", date: "2026-11-01T00:00:00Z" },
        { id: "p-new", date: "2026-11-05T00:00:00Z" },
      ],
      decisions: [{ id: "ADR-001", project: "demo", date: "2026-11-04T00:00:00Z", updated_at: "" }],
      questions: [
        { id: "q-stop", date: "2026-11-02T00:00:00Z" },
        { id: "q-later", date: "2026-11-05T00:00:00Z" },
      ],
    }),
  );
  assert.deepEqual(
    out.map((i) => i.key),
    ["q/q-stop", "q/q-later", "d/ADR-001", "p/p-new", "p/p-old", "c/CR-1", "w/SUG-001", "f/demo/EVO-constitution", "s/demo/constitution"],
  );
  assert.equal(topFive(items({ proposals: out.filter((i) => i.kind === "proposal").map((i) => i.p), questions: [1, 2, 3, 4].map((n) => ({ id: "q" + n, date: "" })) })).length, 5);
  assert.deepEqual(topFive(items()), []);
});

test("each waiting item says what it is under its title", () => {
  assert.equal(needsYouWord({ kind: "question", key: "", at: "", q: { id: "q", stop: true } }), "Self-improvement paused");
  assert.equal(needsYouWord({ kind: "question", key: "", at: "", q: { id: "q", stop: false } }), "Question");
  assert.equal(needsYouWord({ kind: "decision", key: "", at: "", r: { id: "ADR-1" } }), "Decision");
  assert.equal(needsYouWord({ kind: "compute", key: "", at: "", c: { id: "CR-1" } }), "Machine request");
  assert.equal(needsYouWord({ kind: "step", key: "", at: "", step: { kind: "k", slug: "demo", title: "", label: "" } }), "Setup step");
});

test("see-all names the count only when more wait than the column shows", () => {
  assert.equal(seeAllLabel(0), "See all");
  assert.equal(seeAllLabel(5), "See all");
  assert.equal(seeAllLabel(12), "See all 12");
});

test("an item's project falls back to the brain for board-wide proposals and questions", () => {
  assert.equal(itemProject({ kind: "proposal", key: "", at: "", p: { id: "p" } }, "hive"), "hive");
  assert.equal(itemProject({ kind: "proposal", key: "", at: "", p: { id: "p", slug: "demo" } }, "hive"), "demo");
  assert.equal(itemProject({ kind: "question", key: "", at: "", q: { id: "q", slug: "" } }, "hive"), "hive");
  assert.equal(itemProject({ kind: "decision", key: "", at: "", r: { id: "ADR-1", project: "demo" } }, "hive"), "demo");
  assert.equal(itemProject({ kind: "step", key: "", at: "", step: { kind: "k", slug: "demo", title: "", label: "" } }, "hive"), "demo");
});

test("a project's line says what waits, else what is wrong, else when it last moved", () => {
  const rel = () => "3 hours ago";
  const ok = { present: true, missing: false, error: "", stop_present: false, board_present: true, last_post: { date: "2026-11-06T00:00:00Z" }, goal_source: "owner" };
  assert.deepEqual(projectLine(ok, 3, rel), { text: "3 things wait for you", tone: "you" });
  assert.deepEqual(projectLine(ok, 1, rel), { text: "1 thing waits for you", tone: "you" });
  assert.deepEqual(projectLine(ok, 0, rel), { text: "Active 3 hours ago", tone: "neutral" });
  assert.deepEqual(projectLine({ ...ok, missing: true }, 0, rel), { text: "Missing from disk", tone: "bad" });
  assert.deepEqual(projectLine({ ...ok, error: "clone failed" }, 0, rel), { text: "clone failed", tone: "bad" });
  assert.deepEqual(projectLine({ ...ok, stop_present: true }, 0, rel), { text: "Paused by a STOP file", tone: "bad" });
  assert.deepEqual(projectLine({ ...ok, board_present: false }, 0, rel), { text: "No board yet", tone: "bad" });
  assert.deepEqual(projectLine({ ...ok, goal_source: "generated" }, 0, rel), { text: "Runs on a generated goal", tone: "neutral", setGoal: true });
  assert.deepEqual(projectLine({ ...ok, last_post: null }, 0, rel), { text: "Quiet so far", tone: "neutral" });
});

test("this week sums the last seven days of the series", () => {
  const day = (landings, tasks_done, owner_asks) => ({ landings, tasks_done, owner_asks });
  const series = [day(9, 9, 9), ...Array.from({ length: 7 }, (_, i) => day(1, i, 2))];
  assert.deepEqual(weekNumbers(series), { landings: 7, tasksDone: 21, decisions: 14 });
  assert.deepEqual(weekNumbers([day(1, 2, 3)]), { landings: 1, tasksDone: 2, decisions: 3 });
  assert.equal(weekNumbers([]), null);
  assert.equal(weekNumbers(null), null);
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
