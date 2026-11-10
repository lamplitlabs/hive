import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  createAutomationsMock,
  everyMinutes,
  everyWords,
  yamlFile,
  yamlItem,
} from "./automations.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const fixture = JSON.parse(
  await readFile(join(here, "api", "automations.json"), "utf8"),
);

// The mock answers the shapes crates/hive-hub/src/automations.rs serializes
// and the owner console (web/hub/src/lib/api.ts) reads.
const LIST_KEYS = [
  "kinds",
  "limits",
  "projects",
  "script_job_kinds",
  "script_kinds",
];
const CHECK_KEYS = ["branch", "form", "path", "problems", "warning", "yaml"];
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

test("yamlFile puts the fields at the top level, without the name", () => {
  assert.equal(
    yamlFile(entry()),
    'title: Dependency audit\nevery: 1d\nat: "03:00"\nbrief: Audit the dependencies.\n',
  );
  assert.match(
    yamlFile(entry({ brief: "one\ntwo", paused: true })),
    /^title: Dependency audit\nevery: 1d\nat: "03:00"\nbrief: \|-\n  one\n  two\npaused: true\n$/,
  );
});

test("list carries every project, the folder of each and the limits", () => {
  const { mock } = setup();
  const r = mock.list(new URLSearchParams());
  assert.equal(r.status, 200);
  assert.deepEqual(keys(r.body), LIST_KEYS);
  assert.equal(r.body.projects.length, 2);
  const app = r.body.projects[0];
  assert.equal(app.automations.dir, ".hive/automations");
  assert.deepEqual(
    app.automations.files.map((f) => f.name),
    ["docs-sweep", "nightly-audit", "site-check"],
  );
  assert.deepEqual(
    app.applied.files,
    app.automations.files.map((f) => f.path),
  );
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
  assert.equal(ok.body.path, ".hive/automations/deps-audit.yaml");
  assert.equal(ok.body.form, "file");
  assert.equal(ok.body.branch, "main");
  assert.equal(ok.body.yaml, yamlFile(entry()));
  const bad = mock.check({
    project: "demo-app",
    entry: entry({ name: "nightly-audit", every: "5m", skill: "nope" }),
    where: "repo",
  });
  assert.ok(
    bad.body.problems.some(
      (p) =>
        p ===
        ".hive/automations/nightly-audit.yaml already holds an automation named `nightly-audit`; edit that one or pick another name",
    ),
    bad.body.problems.join("; "),
  );
  assert.ok(bad.body.problems.some((p) => p.includes("under the floor")));
  assert.ok(bad.body.problems.some((p) => p.includes("skill: nope")));
  // A list item is edited where it is, so the YAML is an item of the list.
  const item = mock.check({
    project: "demo-app",
    entry: entry({ name: "weekly-tidy", title: "Weekly tidy", every: "1w" }),
    where: "repo",
    replace: "weekly-tidy",
  });
  assert.equal(item.body.form, "list");
  assert.equal(item.body.path, ".hive/hive.yaml");
  assert.match(item.body.yaml, /^  - name: weekly-tidy\n/);
  // Renaming it onto a name a file holds is refused.
  const clash = mock.check({
    project: "demo-app",
    entry: entry({ name: "site-check" }),
    where: "repo",
    replace: "weekly-tidy",
  });
  assert.ok(
    clash.body.problems.includes(
      "`site-check` is already an automation (.hive/automations/site-check.yaml); pick another name",
    ),
    clash.body.problems.join("; "),
  );
  assert.ok(
    mock
      .check({
        project: "demo-app",
        entry: entry(),
        where: "repo",
        replace: "gone",
      })
      .body.problems.includes(
        "demo-app on main has no automation named `gone` to replace",
      ),
  );
  assert.equal(
    mock.check({ project: "demo-lib", entry: entry(), where: "repo" }).body
      .warning,
    "",
  );
  assert.equal(mock.check({ project: "nope", entry: entry() }).status, 404);
  assert.equal(JSON.stringify(data), before, "a check writes nothing");
});

test("a repo write adds a file of its own, commits and gives the live row; an edit rewrites it", () => {
  const { data, events, mock } = setup();
  const r = mock.write({ project: "demo-app", entry: entry(), where: "repo" });
  assert.equal(r.status, 201);
  assert.deepEqual(keys(r.body), WRITE_KEYS);
  assert.equal(r.body.where, "repo");
  assert.equal(r.body.path, ".hive/automations/deps-audit.yaml");
  assert.ok(r.body.commit);
  assert.equal(r.body.schedule.origin_name, "deps-audit");
  assert.equal(
    r.body.schedule.origin,
    "repo:.hive/automations/deps-audit.yaml#deps-audit",
  );
  assert.equal(r.body.schedule.every_words, "every day at 03:00 UTC");
  const p = data.projects[0];
  assert.deepEqual(
    p.automations.files.map((f) => f.name),
    ["deps-audit", "docs-sweep", "nightly-audit", "site-check"],
  );
  assert.deepEqual(
    p.file.entries.map((e) => e.name),
    ["weekly-tidy"],
    "the list file is left alone",
  );
  const f = p.automations.files.find((x) => x.name === "deps-audit");
  assert.equal(f.text, yamlFile(entry()));
  assert.equal(p.applied.commit, r.body.commit);
  assert.ok(p.applied.files.includes(f.path));
  assert.ok(
    p.applied.problems.some((x) => x.startsWith("schedule `docs-sweep`:")),
    "the held file stays reported",
  );
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
  // A rename moves the file in one commit: the old row ends, a new one
  // starts, as in the hub.
  const renamed = mock.write({
    project: "demo-app",
    entry: entry({ name: "deps-weekly", every: "1w" }),
    where: "repo",
    replace: "deps-audit",
  });
  assert.equal(renamed.status, 200);
  assert.equal(renamed.body.path, ".hive/automations/deps-weekly.yaml");
  assert.ok(!p.automations.files.some((x) => x.name === "deps-audit"));
  assert.notEqual(renamed.body.schedule.id, r.body.schedule.id);
  assert.equal(
    p.schedules.find((s) => s.id === r.body.schedule.id).status,
    "ended",
  );
  assert.equal(
    renamed.body.schedule.origin,
    "repo:.hive/automations/deps-weekly.yaml#deps-weekly",
  );
  // A list item is edited in place and stays in the list.
  const tidy = mock.write({
    project: "demo-app",
    entry: entry({
      name: "weekly-tidy",
      title: "Weekly tidy",
      every: "2w",
      kind: "clean",
      brief: "Tidy.",
    }),
    where: "repo",
    replace: "weekly-tidy",
  });
  assert.equal(tidy.status, 200);
  assert.equal(tidy.body.path, ".hive/hive.yaml");
  assert.equal(p.file.entries[0].every, "2w");
  assert.match(
    p.file.text,
    /- name: weekly-tidy\n    title: Weekly tidy\n    every: 2w\n/,
  );
  assert.ok(!p.automations.files.some((x) => x.name === "weekly-tidy"));
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
  assert.equal(lib.file, null, "no list file is created");
  assert.equal(lib.automations.files.length, 1);
  assert.equal(
    lib.automations.files[0].path,
    ".hive/automations/deps-audit.yaml",
  );
  const old = lib.schedules.find((s) => s.id === r.body.schedule.id);
  assert.equal(old.status, "ended");
  assert.match(
    old.note,
    /^moved to \.hive\/automations\/deps-audit\.yaml as deps-audit/,
  );
  assert.equal(
    m.body.schedule.origin_path,
    ".hive/automations/deps-audit.yaml",
  );
  assert.ok(lib.applied, "the folder was applied");
  assert.deepEqual(lib.applied.files, [".hive/automations/deps-audit.yaml"]);
});

test("ending a repo-kept schedule removes its file or list entry; remove does the same by name", () => {
  const { data, mock } = setup();
  const r = mock.action("SCHED-002", "end", { note: "the site moved" });
  assert.equal(r.status, 200);
  assert.equal(r.body.removed, "site-check");
  const p = data.projects[0];
  assert.ok(!p.automations.files.some((f) => f.name === "site-check"));
  assert.equal(p.schedules.find((s) => s.id === "SCHED-002").status, "ended");
  assert.ok(!p.applied.files.includes(".hive/automations/site-check.yaml"));
  const x = mock.remove({ project: "demo-app", name: "weekly-tidy" });
  assert.equal(x.status, 200);
  assert.equal(p.schedules.find((s) => s.id === "SCHED-003").status, "ended");
  assert.equal(p.file.entries.length, 0);
  assert.doesNotMatch(p.file.text, /weekly-tidy/);
  assert.equal(
    mock.remove({ project: "demo-app", name: "weekly-tidy" }).status,
    400,
  );
  assert.equal(
    mock.remove({ project: "nope", name: "weekly-tidy" }).status,
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

test("history lists what happened newest first, pages by seq, and grows with the actions", () => {
  const { mock } = setup();
  const q = (o = {}) => new URLSearchParams(o);
  const r = mock.history("SCHED-002", q({ limit: "5" }));
  assert.equal(r.status, 200);
  assert.deepEqual(keys(r.body), [
    "entries",
    "jobs",
    "last_output",
    "more",
    "schedule",
  ]);
  assert.equal(r.body.entries.length, 5);
  assert.equal(r.body.more, true);
  const seqs = r.body.entries.map((e) => e.seq);
  assert.deepEqual(seqs, [...seqs].sort((a, b) => b - a), "newest first");
  assert.equal(r.body.entries[0].what, "skipped");
  assert.equal(r.body.entries[0].job, "JOB-0402");
  assert.equal(r.body.schedule.script_outside, true);
  assert.equal(r.body.schedule.script_home, ".hive/automations/site-check.sh");
  assert.deepEqual(
    r.body.jobs.map((j) => j.id),
    ["JOB-0402", "JOB-0377"],
  );
  assert.equal(r.body.last_output, "HTTP 502 from https://demo.example/health");
  // The next page starts below the last seq of this one.
  const older = mock.history(
    "SCHED-002",
    q({ before: String(seqs[seqs.length - 1]), limit: "100" }),
  );
  assert.ok(older.body.entries.every((e) => e.seq < seqs[seqs.length - 1]));
  assert.equal(older.body.more, false);
  assert.equal(older.body.entries[older.body.entries.length - 1].what, "added");
  // An action is the newest entry afterwards.
  mock.action("SCHED-002", "pause", { note: "known outage" });
  const after = mock.history("SCHED-002", q({ limit: "1" })).body.entries[0];
  assert.equal(after.what, "paused");
  assert.equal(after.actor, "human");
  assert.equal(after.note, "known outage");
  assert.ok(after.seq > seqs[0]);
  assert.equal(mock.history("SCHED-999", q()).status, 404);
});

test("a new check's script must sit under .hive/; an edit may keep the old one", () => {
  const { mock } = setup();
  const outside = mock.check({
    project: "demo-app",
    where: "store",
    entry: entry({ name: "site-2", kind: "check", script: "scripts/site.sh" }),
  });
  assert.ok(
    outside.body.problems.some((p) => p.includes("is outside .hive/")),
    outside.body.problems.join("|"),
  );
  const inside = mock.check({
    project: "demo-app",
    where: "store",
    entry: entry({
      name: "site-2",
      kind: "check",
      script: ".hive/automations/site-2.sh",
    }),
  });
  assert.deepEqual(inside.body.problems, []);
  // SCHED-002 runs scripts/check-site.sh from before the rule: an edit that
  // keeps it passes, another outside path does not.
  const kept = mock.check({
    project: "demo-app",
    where: "repo",
    replace: "site-check",
    entry: entry({
      name: "site-check",
      kind: "check",
      every: "1h",
      script: "scripts/check-site.sh",
    }),
  });
  assert.deepEqual(kept.body.problems, []);
  const moved = mock.check({
    project: "demo-app",
    where: "repo",
    replace: "site-check",
    entry: entry({
      name: "site-check",
      kind: "check",
      every: "1h",
      script: "scripts/other.sh",
    }),
  });
  assert.ok(moved.body.problems.some((p) => p.includes("is outside .hive/")));
  // The list flags the old row.
  const row = mock
    .list(new URLSearchParams({ project: "demo-app" }))
    .body.projects[0].schedules.find((s) => s.id === "SCHED-002");
  assert.equal(row.script_outside, true);
  assert.equal(row.script_home, ".hive/automations/site-check.sh");
});
