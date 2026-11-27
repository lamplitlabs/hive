import assert from "node:assert/strict";
import { test } from "node:test";
import { declineHappens, groupRecords, headings, RECORD_GROUPS, slugify, waitsOnYou, wantsContents } from "./records-state.ts";

test("headings: h2 and h3 in order, h1 and deeper levels skipped", () => {
  const md = "# Title\n\nIntro.\n\n## Context\ntext\n### Why now\n#### Not listed\n## Decision\n";
  assert.deepEqual(headings(md), [
    { level: 2, text: "Context", id: "context" },
    { level: 3, text: "Why now", id: "why-now" },
    { level: 2, text: "Decision", id: "decision" },
  ]);
});

test("headings: fenced code is not a heading; closing hashes and backticks drop", () => {
  const md = "## Steps ##\n```sh\n# not a heading\n## nor this\n```\n~~~\n## nor this either\n~~~\n### Run `hive land`\n";
  assert.deepEqual(headings(md), [
    { level: 2, text: "Steps", id: "steps" },
    { level: 3, text: "Run hive land", id: "run-hive-land" },
  ]);
});

test("headings: setext underlines count; a list dash does not", () => {
  const md = "Title\n=====\n\nOverview\n--------\n\n- item\n---\n\n## Overview\n";
  assert.deepEqual(headings(md), [
    { level: 2, text: "Overview", id: "overview" },
    { level: 2, text: "Overview", id: "overview-1" },
  ]);
});

test("headings: empty and heading-free bodies give nothing", () => {
  assert.deepEqual(headings(""), []);
  assert.deepEqual(headings("just a paragraph\n\nand another"), []);
  assert.deepEqual(headings("##   \n"), []);
});

test("slugify keeps letters and digits of any script", () => {
  assert.equal(slugify("What's next? (v2)"), "what-s-next-v2");
  assert.equal(slugify("Décisions clés"), "décisions-clés");
  assert.equal(slugify("  --  "), "");
});

test("wantsContents: three headings, or a long body with any", () => {
  assert.equal(wantsContents("## a\n## b\n## c\n"), true);
  assert.equal(wantsContents("## a\n## b\n"), false);
  assert.equal(wantsContents("## a\n" + "x".repeat(1600)), true);
  assert.equal(wantsContents("x".repeat(5000)), false);
});

test("groupRecords: waiting decisions first and not repeated, then the groups in order, newest first", () => {
  const rows = [
    { id: "ADR-1", kind: "adr", status: "Accepted", updated_at: "2026-01-01" },
    { id: "ADR-2", kind: "adr", status: "Proposed", updated_at: "2026-01-05" },
    { id: "PLAN-1", kind: "plan", status: "Active", updated_at: "2026-01-02" },
    { id: "WU-1", kind: "wu", status: "Open", updated_at: "2026-01-03" },
    { id: "WU-2", kind: "wu", status: "Open", updated_at: "2026-01-04" },
    { id: "GUIDE-1", kind: "guide", status: "", updated_at: "2026-01-01" },
    { id: "SKILL-1", kind: "skill", status: "", updated_at: "2026-01-02" },
    { id: "EVO-1", kind: "evolution", status: "", updated_at: "2026-01-02" },
  ];
  const g = groupRecords(rows);
  assert.deepEqual(
    g.map((x) => [x.group.key, x.rows.map((r) => r.id)]),
    [
      ["waiting", ["ADR-2"]],
      ["adr", ["ADR-1"]],
      ["plan", ["PLAN-1"]],
      ["wu", ["WU-2", "WU-1"]],
      ["evolution", ["EVO-1"]],
      ["framework", ["SKILL-1", "GUIDE-1"]],
    ],
  );
  assert.equal(waitsOnYou({ kind: "adr", status: "Proposed" }), true);
  assert.equal(waitsOnYou({ kind: "plan", status: "Proposed" }), false);
});

test("the groups cover the spec's six names", () => {
  assert.deepEqual(
    RECORD_GROUPS.map((g) => g.label),
    ["Decisions", "Plans", "Features", "Work items", "Memory", "Self-improvement", "Built-in guides"],
  );
  assert.equal(RECORD_GROUPS.filter((g) => !g.kinds.length).length, 1, "exactly one catch-all group");
});

test("declineHappens: says one job takes merged work out, only when some is merged", () => {
  const plain = declineHappens([]);
  assert.ok(!plain.includes("job"));
  assert.equal(declineHappens(undefined), plain);
  assert.equal(declineHappens(null), plain);
  const one = declineHappens([{ task: "TASK-141", number: 280, url: "u", head: "hive/task-TASK-141", at: "t" }]);
  assert.ok(one.startsWith(plain));
  assert.match(one, /pull request #280 is on the default branch\), so one job takes that work out/);
  const two = declineHappens([
    { task: "TASK-141", number: 280, url: "u", head: "h", at: "t" },
    { task: "TASK-141", number: 290, url: "u", head: "h", at: "t" },
  ]);
  assert.match(two, /pull requests #280, #290 are on the default branch/);
});
