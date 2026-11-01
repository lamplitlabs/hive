import assert from "node:assert/strict";
import { test } from "node:test";
import {
  EMPTY_ENTRY,
  cadenceWords,
  cleanEntry,
  entryFromFile,
  entryFromSchedule,
  entryHints,
  everyMinutes,
  fileEntryOf,
  keptWords,
  slugify,
  sortRows,
  stateWords,
} from "./automations.ts";

const sched = (over = {}) => ({
  id: "SCHED-001",
  project: "demo-app",
  title: "Nightly audit",
  kind: "task",
  brief: "Audit the dependencies.",
  tier: 1,
  priority: 0,
  every_minutes: 1440,
  at_time: "02:00",
  status: "active",
  next_at: "",
  last_at: "",
  last_job: "",
  runs: 3,
  max_runs: 0,
  note: "",
  created_by: "human",
  created_at: "",
  updated_at: "",
  script: "",
  job_kind: "",
  state: "",
  state_since: "",
  last_rc: null,
  last_output: "",
  fails: 0,
  agent: "",
  skill: "",
  origin: "",
  every: "1d",
  every_words: "every day at 02:00 UTC",
  origin_name: "",
  origin_path: "",
  ...over,
});

test("slugify turns a title into the file's name", () => {
  assert.equal(slugify("Audit the dependencies"), "audit-the-dependencies");
  assert.equal(slugify("  Café — nightly!  "), "cafe-nightly");
  assert.equal(slugify("---"), "");
  assert.equal(slugify("a".repeat(80)).length, 64);
});

test("everyMinutes reads the cadences the file takes", () => {
  assert.equal(everyMinutes("30m"), 30);
  assert.equal(everyMinutes("6h"), 360);
  assert.equal(everyMinutes("1d"), 1440);
  assert.equal(everyMinutes("2w"), 20160);
  assert.equal(everyMinutes("hourly"), 60);
  assert.equal(everyMinutes("Daily"), 1440);
  assert.equal(everyMinutes("90"), 90);
  assert.equal(everyMinutes("0h"), null);
  assert.equal(everyMinutes("soon"), null);
});

test("cadenceWords says the cadence the way the hub does", () => {
  assert.equal(cadenceWords("1d", "02:00"), "every day at 02:00 UTC");
  assert.equal(cadenceWords("2w"), "every 2 weeks");
  assert.equal(cadenceWords("6h"), "every 6 hours");
  assert.equal(cadenceWords("90m"), "every 90 minutes");
  assert.equal(cadenceWords("weekly"), "every week");
  assert.equal(cadenceWords("soon"), "every soon");
  assert.equal(cadenceWords(""), "");
});

test("entryHints names what is missing before the hub is asked", () => {
  assert.deepEqual(
    entryHints({ ...EMPTY_ENTRY, name: "nightly", brief: "Audit." }),
    [],
  );
  const empty = entryHints(EMPTY_ENTRY);
  assert.ok(empty.some((h) => h.startsWith("Give it a name")));
  assert.ok(empty.some((h) => h.startsWith("Say what each job does")));
  assert.ok(
    entryHints({ ...EMPTY_ENTRY, name: "Bad Name", brief: "x" }).some((h) =>
      h.includes("lowercase"),
    ),
  );
  assert.ok(
    entryHints({ ...EMPTY_ENTRY, name: "n", brief: "x", every: "10m" }).some(
      (h) => h.includes("shortest cadence is 30"),
    ),
  );
  assert.ok(
    entryHints({
      ...EMPTY_ENTRY,
      name: "n",
      brief: "x",
      every: "6h",
      at: "02:00",
    }).some((h) => h.includes("ignored")),
  );
  assert.ok(
    entryHints({ ...EMPTY_ENTRY, name: "n", brief: "x", at: "25:00" }).some(
      (h) => h.includes("HH:MM"),
    ),
  );
  const check = entryHints({ ...EMPTY_ENTRY, name: "n", kind: "check" });
  assert.ok(check.some((h) => h.includes("needs the script")));
  assert.ok(
    !check.some((h) => h.startsWith("Say what each job does")),
    "a check needs a script, not a brief",
  );
  assert.ok(
    entryHints({
      ...EMPTY_ENTRY,
      name: "n",
      kind: "check",
      script: "../x.sh",
    }).some((h) => h.includes("relative")),
  );
  assert.ok(
    entryHints({ ...EMPTY_ENTRY, name: "n", skill: "release-check" }).length ===
      0,
    "a skill stands in for the brief",
  );
});

test("entryFromSchedule keeps a hand-set schedule's fields and names it", () => {
  const e = entryFromSchedule(sched());
  assert.equal(e.name, "nightly-audit");
  assert.equal(e.every, "1d");
  assert.equal(e.at, "02:00");
  assert.equal(e.brief, "Audit the dependencies.");
  assert.equal(e.paused, false);
  assert.equal(e.job_kind, "task");
  const w = entryFromSchedule(
    sched({
      kind: "watch",
      script: "scripts/watch.sh",
      job_kind: "improve",
      status: "paused",
      origin_name: "ci-watch",
    }),
  );
  assert.equal(w.name, "ci-watch");
  assert.equal(w.job_kind, "improve");
  assert.equal(w.paused, true);
});

test("entryFromFile fills the file's defaults", () => {
  const e = entryFromFile({ name: "x", title: "X", every: "6h" });
  assert.equal(e.kind, "task");
  assert.equal(e.tier, 1);
  assert.equal(e.priority, 0);
  assert.equal(e.times, 0);
  assert.equal(e.paused, false);
  assert.equal(
    entryFromFile({ name: "x", title: "X", every: "6h", tier: 0 }).tier,
    0,
  );
});

test("cleanEntry trims and drops the fields the kind does not take", () => {
  const task = cleanEntry({
    ...EMPTY_ENTRY,
    name: " n ",
    title: " T ",
    brief: "a\r\nb\n\n",
    script: "scripts/x.sh",
    job_kind: "improve",
  });
  assert.equal(task.name, "n");
  assert.equal(task.brief, "a\nb");
  assert.equal(task.script, "");
  assert.equal(task.job_kind, "");
  const check = cleanEntry({
    ...EMPTY_ENTRY,
    name: "c",
    kind: "check",
    brief: "ignored",
    skill: "s",
    script: " scripts/check.sh ",
  });
  assert.equal(check.brief, "");
  assert.equal(check.skill, "");
  assert.equal(check.script, "scripts/check.sh");
  assert.equal(check.job_kind, "task");
});

test("fileEntryOf finds the file entry a row came from", () => {
  const project = {
    slug: "demo-app",
    name: "Demo",
    repo: null,
    applied: null,
    schedules: [],
    options: { skills: [], agents: [] },
    file: {
      path: ".pulse/pulse.yaml",
      text: "",
      entries: [{ name: "nightly", title: "Nightly", every: "1d" }],
      needs: [],
      problems: [],
    },
  };
  assert.equal(fileEntryOf(project, sched()), null);
  const e = fileEntryOf(
    project,
    sched({ origin_name: "nightly", origin_path: ".pulse/pulse.yaml" }),
  );
  assert.equal(e?.title, "Nightly");
  assert.equal(e?.kind, "task");
  assert.equal(
    fileEntryOf({ ...project, file: null }, sched({ origin_name: "nightly" })),
    null,
  );
});

test("keptWords and stateWords speak for the row", () => {
  assert.equal(keptWords(sched()), "set by you");
  assert.equal(
    keptWords(sched({ created_by: "demo-app-lead-rsi" })),
    "set by demo-app-lead-rsi",
  );
  assert.equal(
    keptWords(sched({ origin_path: ".pulse/pulse.yaml" })),
    "in .pulse/pulse.yaml",
  );
  assert.equal(stateWords(sched()), null);
  assert.deepEqual(stateWords(sched({ kind: "check", state: "green" })), {
    text: "green",
    tone: "ok",
  });
  assert.deepEqual(
    stateWords(sched({ kind: "check", state: "red", fails: 3 })),
    { text: "red ×3", tone: "bad" },
  );
  assert.deepEqual(stateWords(sched({ kind: "watch", state: "ok" })), {
    text: "quiet",
    tone: "ok",
  });
});

test("sortRows puts live rows first, ended last, order kept within", () => {
  const rows = [
    sched({ id: "a", status: "ended" }),
    sched({ id: "b", status: "paused" }),
    sched({ id: "c" }),
    sched({ id: "d" }),
  ];
  assert.deepEqual(
    sortRows(rows).map((s) => s.id),
    ["c", "d", "b", "a"],
  );
});
