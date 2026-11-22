import assert from "node:assert/strict";
import { test } from "node:test";
import {
  EMPTY_ENTRY,
  cadenceWords,
  cleanEntry,
  daemonProblems,
  entryFromFile,
  entryFromSchedule,
  entryHints,
  everyMinutes,
  fileEntryOf,
  historySummary,
  historyWords,
  keptWords,
  scriptHome,
  scriptInHive,
  scriptOutside,
  slugify,
  sortRows,
  stateWords,
  plainProblem,
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

test("a check's script lives under .hive/, unless it is the one it already runs", () => {
  assert.equal(scriptHome("site-check"), ".hive/automations/site-check.sh");
  assert.equal(scriptHome(" "), ".hive/automations/<name>.sh");
  assert.ok(scriptInHive(".hive/automations/x.sh"));
  assert.ok(scriptInHive("./.hive/x.sh"));
  assert.ok(!scriptInHive(".hive/"));
  assert.ok(!scriptInHive("scripts/check.sh"));
  assert.ok(!scriptInHive(".hivey/check.sh"));
  // The folder's name from before the rename is the project's folder too.
  assert.ok(scriptInHive(".pulse/automations/x.sh"));
  assert.ok(!scriptInHive(".pulse/"));
  assert.ok(!scriptInHive(".pulsed/check.sh"));
  const outside = entryHints({
    ...EMPTY_ENTRY,
    name: "site",
    kind: "check",
    script: "scripts/check.sh",
  });
  assert.equal(outside.length, 1, outside.join("|"));
  assert.ok(outside[0].includes(".hive/automations/site.sh"), outside[0]);
  assert.deepEqual(
    entryHints({
      ...EMPTY_ENTRY,
      name: "site",
      kind: "check",
      script: ".hive/automations/site.sh",
    }),
    [],
  );
  // An edit keeps the script it already runs.
  assert.deepEqual(
    entryHints(
      { ...EMPTY_ENTRY, name: "site", kind: "check", script: "scripts/check.sh" },
      30,
      "scripts/check.sh",
    ),
    [],
  );
  // The empty-script hint names the place.
  assert.ok(
    entryHints({ ...EMPTY_ENTRY, name: "site", kind: "watch" }).some((h) =>
      h.includes(".hive/automations/site.sh is the place"),
    ),
  );
  assert.equal(scriptOutside(sched()), false);
  assert.equal(
    scriptOutside(sched({ kind: "check", script: "scripts/check.sh" })),
    true,
  );
  assert.equal(
    scriptOutside(
      sched({ kind: "check", script: ".hive/automations/check.sh" }),
    ),
    false,
  );
  // The hub's own word wins over the path when it is there.
  assert.equal(
    scriptOutside(
      sched({ kind: "watch", script: "scripts/w.sh", script_outside: false }),
    ),
    false,
  );
});

test("daemonProblems leaves the skill folders' problems to the Skills page", () => {
  const p = {
    slug: "demo-app",
    name: "Demo",
    repo: null,
    file: null,
    schedules: [],
    options: { skills: [], agents: [] },
    automations: { dir: ".hive/automations", files: [] },
    applied: {
      problems: [
        "skill .github/skills/design-taste-frontend/SKILL.md: the file is 61 kB; a skill is at most 32 kB",
        "schedule `nightly`: needs `every`",
        ".hive/hive.yaml: two items are named `tidy`",
      ],
    },
  };
  assert.deepEqual(daemonProblems(p), [
    "schedule `nightly`: needs `every`",
    ".hive/hive.yaml: two items are named `tidy`",
  ]);
});

test("historyWords and historySummary say what happened in the owner's words", () => {
  const entry = (over = {}) => ({
    seq: 1,
    at: "2026-09-01T09:00:03Z",
    actor: "daemon",
    what: "ran",
    run: 0,
    rc: null,
    timeout: false,
    seconds: 0,
    state: "",
    previous: "",
    job: "",
    job_status: "",
    job_agent: "",
    job_finished_at: "",
    queued: false,
    why: "",
    note: "",
    changed: [],
    origin: "",
    ...over,
  });
  const red = historyWords(
    entry({ run: 611, rc: 1, seconds: 4.2, state: "red", job: "JOB-0402", queued: true }),
    "check",
  );
  assert.equal(red.title, "Run 611: exit 1 in 4.2s");
  assert.equal(red.detail, "The check failed. Queued JOB-0402.");
  assert.equal(red.tone, "bad");
  const again = historyWords(
    entry({ run: 612, rc: 1, seconds: 40, state: "red", job: "JOB-0402" }),
    "check",
  );
  assert.equal(again.title, "Run 612: exit 1 in 40s");
  assert.equal(
    again.detail,
    "The check failed. JOB-0402 was still open, so nothing new was queued.",
  );
  const quiet = historyWords(
    entry({ run: 3, rc: 0, seconds: 1.0, state: "ok" }),
    "watch",
  );
  assert.equal(quiet.title, "Run 3: exit 0 in 1s");
  assert.equal(quiet.detail, "Nothing to report.");
  assert.equal(quiet.tone, "ok");
  const out = historyWords(
    entry({ run: 4, rc: 124, timeout: true, seconds: 1200, state: "red" }),
    "check",
  );
  assert.equal(out.title, "Run 4: exit 124 (timed out) in 1200s");
  assert.equal(out.tone, "warn");
  assert.equal(historyWords(entry({ run: 5 }), "check").title, "Run 5 could not start");
  assert.deepEqual(
    historyWords(
      entry({ what: "red", state: "broken", previous: "ok", note: "20260901T083004Z-daemon" }),
      "watch",
    ),
    {
      title: "Turned broken from quiet",
      detail: "Posted 20260901T083004Z-daemon on the board.",
      tone: "bad",
    },
  );
  assert.equal(
    historyWords(entry({ what: "green", state: "green", previous: "red" }), "check").title,
    "Turned green from red",
  );
  assert.equal(
    historyWords(entry({ what: "fired", job: "JOB-1", why: "due" }), "task").title,
    "Queued JOB-1",
  );
  assert.equal(
    historyWords(entry({ what: "skipped", job: "JOB-1" }), "task").detail,
    "JOB-1 was still open.",
  );
  assert.equal(historyWords(entry({ what: "added", actor: "human" }), "task").title, "Added by you");
  assert.equal(historyWords(entry({ what: "added", actor: "daemon" }), "task").title, "Added by the daemon");
  assert.equal(
    historyWords(
      entry({ what: "added", actor: "daemon", origin: "repo:.hive/automations/site.yaml#site" }),
      "check",
    ).title,
    "Added from .hive/automations/site.yaml",
  );
  assert.equal(
    historyWords(
      entry({
        what: "updated",
        actor: "daemon",
        changed: ["every"],
        origin: "repo:.hive/automations/site.yaml#site",
      }),
      "check",
    ).title,
    "Changed by .hive/automations/site.yaml",
  );
  assert.equal(
    historyWords(entry({ what: "updated", actor: "demo-lead", changed: ["every", "at"] }), "task")
      .detail,
    "every, at changed.",
  );
  assert.equal(
    historyWords(entry({ what: "paused", actor: "human", note: "release week" }), "task").detail,
    "release week",
  );
  assert.equal(historyWords(entry({ what: "run", actor: "human" }), "task").title, "Run now, asked by you");

  assert.equal(historySummary([], "check"), "Nothing has happened yet.");
  assert.equal(
    historySummary([entry({ what: "added" })], "check"),
    "Nothing has happened yet.",
  );
  assert.equal(
    historySummary([entry({ what: "paused" }), entry({ what: "added" })], "task"),
    "No runs yet; 1 change.",
  );
  assert.equal(
    historySummary(
      [
        entry({ rc: 1, state: "red", run: 3 }),
        entry({ what: "fired", job: "J" }),
        entry({ rc: 0, state: "green", run: 2 }),
        entry({ run: 1 }),
      ],
      "check",
    ),
    "3 runs, 1 red, 1 could not start, 1 job queued on this page.",
  );
  assert.equal(
    historySummary(
      [entry({ rc: 0, state: "broken", run: 2 }), entry({ rc: 0, state: "ok", run: 1 })],
      "watch",
    ),
    "2 runs, 1 firing on this page.",
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
      path: ".hive/hive.yaml",
      text: "",
      entries: [{ name: "nightly", title: "Nightly", every: "1d" }],
      needs: [],
      problems: [],
    },
  };
  assert.equal(fileEntryOf(project, sched()), null);
  const e = fileEntryOf(
    project,
    sched({ origin_name: "nightly", origin_path: ".hive/hive.yaml" }),
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
    keptWords(sched({ origin_path: ".hive/hive.yaml" })),
    "in .hive/hive.yaml",
  );
  assert.equal(stateWords(sched()), null);
  assert.deepEqual(stateWords(sched({ kind: "check", state: "green" })), {
    text: "passing",
    tone: "ok",
  });
  assert.deepEqual(
    stateWords(sched({ kind: "check", state: "red", fails: 3 })),
    { text: "failing, 3 times in a row", tone: "bad" },
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

test("plainProblem turns the hub's field words into a sentence", () => {
  assert.equal(
    plainProblem("needs `every`, the cadence (30m, 6h, 1d, 2w, hourly, daily, weekly)"),
    "Needs how often it runs (30m, 6h, 1d, 2w, hourly, daily, weekly)",
  );
  assert.equal(plainProblem("`script` must sit under .hive/"), "Script must sit under .hive/.");
  assert.equal(plainProblem("needs a `brief` (what each job does) or a `skill` to follow"), "Needs a brief (what each job does) or a skill to follow.");
});
