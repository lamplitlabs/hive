import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  createAutomationsMock,
  everyMinutes,
  everyWords,
  yamlItem,
} from "./automations.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const fixture = JSON.parse(
  await readFile(join(here, "api", "automations.json"), "utf8"),
);

// The mock answers the shapes crates/pulse-hub/src/automations.rs serializes
// and the owner console (web/hub/src/lib/api.ts) reads.
const LIST_KEYS = [
  "kinds",
  "limits",
  "projects",
  "script_job_kinds",
  "script_kinds",
];
const CHECK_KEYS = ["branch", "path", "problems", "warning", "yaml"];
const WRITE_KEYS = [
  "branch",
  "commit",
  "path",
  "schedule",
  "synced",
  "warning",
  "where",
  "yaml",
];

const NOW = "2026-09-08T09:00:00Z";
const setup = () => {
  const data = structuredClone(fixture);
  const events = [];
  const mock = createAutomationsMock({
    data,
    now: () => NOW,
    emit: (actor, action, subject) => events.push(action + " " + subject),
  });
  return { data, events, mock };
};
const keys = (o) => Object.keys(o).sort();
const entry = (over = {}) => ({
  name: "deps-audit",
  title: "Dependency audit",
  every: "1d",
  at: "03:00",
  kind: "task",
  brief: "Audit the dependencies.",
  skill: "",
  script: "",
  job_kind: "",
  tier: 1,
  priority: 0,
  run_as: "",
  times: 0,
  paused: false,
  ...over,
});

test("cadence words and minutes match the hub's", () => {
  assert.equal(everyMinutes("2w"), 20160);
  assert.equal(everyMinutes("nope"), null);
  assert.equal(everyWords(1440, "02:00"), "every day at 02:00 UTC");
  assert.equal(everyWords(30, "02:00"), "every 30 minutes");
});

test("yamlItem writes the hub's field order and leaves defaults out", () => {
  assert.equal(
    yamlItem(entry()),
    '  - name: deps-audit\n    title: Dependency audit\n    every: 1d\n    at: "03:00"\n    brief: Audit the dependencies.\n',
  );
  const check = yamlItem(
    entry({
      kind: "check",
      brief: "",
      script: "scripts/x.sh",
      job_kind: "improve",
      tier: 0,
      paused: true,
    }),
  );
  assert.match(check, /kind: check\n/);
  assert.match(check, /job_kind: improve\n/);
  assert.match(check, /tier: 0\n/);
  assert.match(check, /paused: true\n$/);
  assert.match(
    yamlItem(entry({ brief: "one\ntwo" })),
    /brief: \|-\n      one\n      two\n/,
  );
});

test("list carries every project and the limits", () => {
  const { mock } = setup();
  const r = mock.list(new URLSearchParams());
  assert.equal(r.status, 200);
  assert.deepEqual(keys(r.body), LIST_KEYS);
  assert.equal(r.body.projects.length, 2);
  assert.equal(
    mock.list(new URLSearchParams("project=demo-lib")).body.projects.length,
    1,
  );
});

test("check names the problems and composes the YAML without writing", () => {
  const { data, mock } = setup();
  const before = JSON.stringify(data);
  const ok = mock.check({ project: "demo-app", entry: entry(), where: "repo" });
  assert.equal(ok.status, 200);
  assert.deepEqual(keys(ok.body), CHECK_KEYS);
  assert.deepEqual(ok.body.problems, []);
  assert.equal(ok.body.path, ".pulse/pulse.yaml");
  assert.equal(ok.body.branch, "main");
  const bad = mock.check({
    project: "demo-app",
    entry: entry({ name: "nightly-audit", every: "5m", skill: "nope" }),
    where: "repo",
  });
  assert.ok(
    bad.body.problems.some((p) =>
      p.includes("already has an entry named nightly-audit"),
    ),
  );
  assert.ok(bad.body.problems.some((p) => p.includes("under the floor")));
  assert.ok(bad.body.problems.some((p) => p.includes("skill: nope")));
  assert.equal(
    mock
      .check({ project: "demo-lib", entry: entry(), where: "repo" })
      .body.warning.startsWith("There is no .pulse/pulse.yaml"),
    true,
  );
  assert.equal(mock.check({ project: "nope", entry: entry() }).status, 404);
  assert.equal(JSON.stringify(data), before, "a check writes nothing");
});

test("a repo write appends to the file, commits and gives the live row; an edit replaces", () => {
  const { data, events, mock } = setup();
  const r = mock.write({ project: "demo-app", entry: entry(), where: "repo" });
  assert.equal(r.status, 201);
  assert.deepEqual(keys(r.body), WRITE_KEYS);
  assert.equal(r.body.where, "repo");
  assert.ok(r.body.commit);
  assert.equal(r.body.schedule.origin_name, "deps-audit");
  assert.equal(r.body.schedule.every_words, "every day at 03:00 UTC");
  const p = data.projects[0];
  assert.deepEqual(
    p.file.entries.map((e) => e.name),
    ["nightly-audit", "site-check", "weekly-tidy", "deps-audit"],
  );
  assert.match(p.file.text, /- name: deps-audit\n/);
  assert.equal(p.applied.commit, r.body.commit);
  const e = mock.write({
    project: "demo-app",
    entry: entry({ title: "Dependency audit, weekly", every: "1w" }),
    where: "repo",
    replace: "deps-audit",
  });
  assert.equal(e.status, 200);
  assert.equal(e.body.schedule.id, r.body.schedule.id, "the same row, updated");
  assert.equal(e.body.schedule.every, "1w");
  const same = mock.write({
    project: "demo-app",
    entry: entry({ title: "Dependency audit, weekly", every: "1w" }),
    where: "repo",
    replace: "deps-audit",
  });
  assert.equal(
    same.body.commit,
    null,
    "nothing to commit when the file already says it",
  );
  assert.ok(events.includes("schedule.add " + r.body.schedule.id));
});

test("a write with problems is refused with them", () => {
  const { mock } = setup();
  const r = mock.write({
    project: "demo-app",
    entry: entry({ brief: "" }),
    where: "repo",
  });
  assert.equal(r.status, 400);
  assert.ok(Array.isArray(r.body.problems) && r.body.problems.length === 1);
  assert.equal(r.body.error, "The automation has problems");
});

test("a store write adds a hand-set row; move puts it into the file and ends the copy", () => {
  const { data, mock } = setup();
  const r = mock.write({ project: "demo-lib", entry: entry(), where: "store" });
  assert.equal(r.status, 201);
  assert.equal(r.body.where, "store");
  assert.equal(r.body.schedule.origin, "");
  assert.equal(r.body.schedule.created_by, "human");
  const u = mock.write({
    project: "demo-lib",
    entry: entry({ paused: true }),
    where: "store",
    id: r.body.schedule.id,
  });
  assert.equal(u.status, 200);
  assert.equal(u.body.schedule.status, "paused");
  const m = mock.write({
    project: "demo-lib",
    entry: entry(),
    where: "repo",
    move: r.body.schedule.id,
  });
  assert.equal(m.status, 201);
  const lib = data.projects[1];
  assert.ok(lib.file, "the file is created");
  assert.equal(lib.file.entries.length, 1);
  const old = lib.schedules.find((s) => s.id === r.body.schedule.id);
  assert.equal(old.status, "ended");
  assert.match(old.note, /^moved to \.pulse\/pulse\.yaml as deps-audit/);
  assert.equal(m.body.schedule.origin_path, ".pulse/pulse.yaml");
});

test("ending a file-kept schedule removes its entry; remove does the same by name", () => {
  const { data, mock } = setup();
  const r = mock.action("SCHED-002", "end", { note: "the site moved" });
  assert.equal(r.status, 200);
  assert.equal(r.body.removed, "site-check");
  const p = data.projects[0];
  assert.ok(!p.file.entries.some((e) => e.name === "site-check"));
  assert.equal(p.schedules.find((s) => s.id === "SCHED-002").status, "ended");
  assert.doesNotMatch(p.file.text, /site-check/);
  const x = mock.remove({ project: "demo-app", name: "weekly-tidy" });
  assert.equal(x.status, 200);
  assert.equal(p.schedules.find((s) => s.id === "SCHED-003").status, "ended");
  assert.equal(
    mock.remove({ project: "demo-app", name: "weekly-tidy" }).status,
    404,
  );
  assert.equal(mock.action("SCHED-999", "pause", {}).status, 404);
});

test("pause, resume and run act on the row", () => {
  const { mock } = setup();
  assert.equal(
    mock.action("SCHED-001", "pause", { note: "freeze" }).body.schedule.status,
    "paused",
  );
  assert.equal(
    mock.action("SCHED-001", "resume", {}).body.schedule.status,
    "active",
  );
  assert.equal(mock.action("SCHED-001", "run", {}).body.schedule.next_at, NOW);
  assert.equal(
    mock.action("SCHED-004", "end", {}).body.schedule.status,
    "ended",
  );
});
