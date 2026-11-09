import assert from "node:assert/strict";
import { test } from "node:test";
import {
  andList,
  expiresAt,
  firstSentence,
  fmtDate,
  hashHue,
  initials,
  KIND_TONE,
  KINDS,
  num,
  parseDate,
  plural,
  prioLabel,
  PRIORITIES,
  rel,
  sentence,
  shortId,
  sizeText,
  span,
  statusTone,
  titleCase,
  truncate,
  untilText,
} from "./format.ts";

test("sentence trims whitespace and capitalizes the first letter", () => {
  assert.equal(sentence("  which option should agents use?  "), "Which option should agents use?");
  assert.equal(sentence("already capitalized."), "Already capitalized.");
  assert.equal(sentence(""), "");
});

test("firstSentence returns the first complete sentence when present", () => {
  assert.equal(firstSentence("first reason. second detail."), "First reason.");
  assert.equal(firstSentence("why did it stop? because a gate failed."), "Why did it stop?");
  assert.equal(firstSentence("no punctuation yet"), "No punctuation yet");
  assert.equal(firstSentence("   "), "");
});

test("num groups thousands and reads missing values as zero", () => {
  assert.equal(num(0), "0");
  assert.equal(num(1234567), "1,234,567");
  assert.equal(num(undefined), "0");
  assert.equal(num(null), "0");
  assert.equal(num("42"), "42");
});

test("plural says one thing, and every other count in the plural", () => {
  assert.equal(plural(1, "job"), "1 job");
  assert.equal(plural(0, "job"), "0 jobs");
  assert.equal(plural(2, "commit"), "2 commits");
  assert.equal(plural(1500, "line"), "1,500 lines");
});

test("andList joins one, two and three items the way a sentence does", () => {
  assert.equal(andList([]), "");
  assert.equal(andList(["a"]), "a");
  assert.equal(andList(["a", "b"]), "a and b");
  assert.equal(andList(["a", "b", "c"]), "a, b and c");
  assert.equal(andList(["a", "b", "c", "d"]), "a, b, c and d");
});

test("sizeText reads bytes in B, KB, MB and GB across each boundary", () => {
  assert.equal(sizeText(undefined), "0 B");
  assert.equal(sizeText(null), "0 B");
  assert.equal(sizeText(0), "0 B");
  assert.equal(sizeText(1023), "1023 B");
  assert.equal(sizeText(1024), "1 KB");
  assert.equal(sizeText(184 * 1024 + 100), "184 KB");
  assert.equal(sizeText(1 << 20), "1.0 MB");
  assert.equal(sizeText(1.5 * (1 << 20)), "1.5 MB");
  assert.equal(sizeText((1 << 30) - (1 << 20)), "1023.0 MB");
  assert.equal(sizeText(1 << 30), "1.0 GB");
  assert.equal(sizeText(1.2 * (1 << 30)), "1.2 GB");
});

test("fmtDate shows an ISO stamp with a space and UTC", () => {
  assert.equal(fmtDate("2026-11-08T09:10:27Z"), "2026-11-08 09:10:27 UTC");
  assert.equal(fmtDate(undefined), "");
  assert.equal(fmtDate(null), "");
});

test("parseDate accepts dates, hour stamps and offsets, and rejects the rest", () => {
  assert.equal(parseDate("2026-11-08")?.toISOString(), "2026-11-08T00:00:00.000Z");
  assert.equal(parseDate("2026-11-08T09")?.toISOString(), "2026-11-08T09:00:00.000Z");
  assert.equal(parseDate("2026-11-08T09:10:27")?.toISOString(), "2026-11-08T09:10:27.000Z");
  assert.equal(parseDate("2026-11-08T09:10:27Z")?.toISOString(), "2026-11-08T09:10:27.000Z");
  assert.equal(parseDate("2026-11-08T11:10:27+02:00")?.toISOString(), "2026-11-08T09:10:27.000Z");
  assert.equal(parseDate(""), null);
  assert.equal(parseDate(null), null);
  assert.equal(parseDate("not a date"), null);
});

test("span words a duration in its largest whole unit", () => {
  assert.equal(span(0), "1 second");
  assert.equal(span(1), "1 second");
  assert.equal(span(40), "40 seconds");
  assert.equal(span(60), "1 minute");
  assert.equal(span(9 * 60), "9 minutes");
  assert.equal(span(3600), "1 hour");
  assert.equal(span(3 * 3600), "3 hours");
  assert.equal(span(86400), "1 day");
  assert.equal(span(26 * 86400), "26 days");
  assert.equal(span(30 * 86400), "1 month");
  assert.equal(span(200 * 86400), "7 months");
  assert.equal(span(365 * 86400), "1 year");
  assert.equal(span(2 * 365 * 86400), "2 years");
});

// A fixed clock: 2026-11-08T12:00:00Z.
const NOW = Date.parse("2026-11-08T12:00:00Z");

test("rel says how long ago, or how soon, against the clock it is given", () => {
  assert.equal(rel("2026-11-08T12:00:00Z", NOW), "just now");
  assert.equal(rel("2026-11-08T11:59:30Z", NOW), "just now");
  assert.equal(rel("2026-11-08T12:00:30Z", NOW), "just now");
  assert.equal(rel("2026-11-08T11:59:10Z", NOW), "50 seconds ago");
  assert.equal(rel("2026-11-08T11:51:00Z", NOW), "9 minutes ago");
  assert.equal(rel("2026-11-08T11:00:00Z", NOW), "1 hour ago");
  assert.equal(rel("2026-11-05T12:00:00Z", NOW), "3 days ago");
  assert.equal(rel("2026-11-08T15:00:00Z", NOW), "in 3 hours");
  assert.equal(rel("2026-11-08T12:05:00Z", NOW), "in 5 minutes");
});

test("rel hands back what it cannot read", () => {
  assert.equal(rel("someday", NOW), "someday");
  assert.equal(rel("", NOW), "");
  assert.equal(rel(null, NOW), "");
  assert.equal(rel(undefined, NOW), "");
});

test("untilText counts down to a moment or says how long it has expired", () => {
  assert.equal(untilText(null, NOW), "");
  assert.equal(untilText(new Date("2026-11-10T12:00:00Z"), NOW), "in 2 days");
  assert.equal(untilText(new Date("2026-11-08T12:00:01Z"), NOW), "in 1 second");
  assert.equal(untilText(new Date("2026-11-08T12:00:00Z"), NOW), "in 1 second");
  assert.equal(untilText(new Date("2026-11-08T11:00:00Z"), NOW), "expired 1 hour ago");
  assert.equal(untilText(new Date("2026-10-08T12:00:00Z"), NOW), "expired 1 month ago");
});

test("expiresAt adds the expiry days to the date, and is null without either", () => {
  assert.equal(expiresAt({ date: "2026-11-08", expiry_days: 3 })?.toISOString(), "2026-11-11T00:00:00.000Z");
  assert.equal(expiresAt({ date: "2026-11-08" }), null);
  assert.equal(expiresAt({ date: "2026-11-08", expiry_days: 0 }), null);
  assert.equal(expiresAt({ date: "", expiry_days: 3 }), null);
});

test("expiresAt and untilText together word a record's expiry", () => {
  assert.equal(untilText(expiresAt({ date: "2026-11-01", expiry_days: 14 }), NOW), "in 7 days");
  assert.equal(untilText(expiresAt({ date: "2026-10-01", expiry_days: 7 }), NOW), "expired 1 month ago");
});

test("hashHue is a stable hue in 0..359", () => {
  assert.equal(hashHue(""), 0);
  assert.equal(hashHue("a"), 97);
  assert.equal(hashHue("quartz-34"), hashHue("quartz-34"));
  for (const seed of ["quartz-34", "amistio-template", "a much longer seed with spaces"]) {
    const h = hashHue(seed);
    assert.ok(Number.isInteger(h) && h >= 0 && h < 360, `${seed} -> ${h}`);
  }
});

test("initials take two words' first letters, or one word's first two", () => {
  assert.equal(initials("quartz-34"), "Q3");
  assert.equal(initials("amistio template"), "AT");
  assert.equal(initials("web/hub"), "WH");
  assert.equal(initials("pulse_runner"), "PR");
  assert.equal(initials("cinder"), "CI");
  assert.equal(initials("x"), "X");
  assert.equal(initials(""), "?");
  assert.equal(initials("--"), "?");
});

test("truncate cuts past the limit with an ellipsis and leaves short text alone", () => {
  assert.equal(truncate("hello", 5), "hello");
  assert.equal(truncate("hello", 10), "hello");
  assert.equal(truncate("hello world", 5), "hello\u2026");
  assert.equal(truncate("", 3), "");
});

test("statusTone: waiting on the owner is marigold", () => {
  for (const s of ["awaiting", "pending", "review", "needs_owner", "needs owner", "needs you", "proposed", "waiting for you", "owner", "PENDING"]) {
    assert.equal(statusTone(s), "you", s);
  }
});

test("statusTone: good, broken, needs-a-look and moving states", () => {
  for (const s of ["active", "done", "landed", "accepted", "online", "connected", "Done"]) {
    assert.equal(statusTone(s), "ok", s);
  }
  for (const s of ["blocked", "failed", "stopped", "provider_error", "error", "verify_error", "unreachable"]) {
    assert.equal(statusTone(s), "bad", s);
  }
  for (const s of ["stale", "expired", "no expiry", "step_limit", "time_limit", "not_signed_in", "lost"]) {
    assert.equal(statusTone(s), "warn", s);
  }
  for (const s of ["in_progress", "running", "queued", "reviewing", "held", "cancelling"]) {
    assert.equal(statusTone(s), "info", s);
  }
});

test("statusTone: reviewing is moving work, not the owner's review", () => {
  assert.equal(statusTone("review"), "you");
  assert.equal(statusTone("reviewing"), "info");
});

test("statusTone: an unknown or missing status is neutral", () => {
  assert.equal(statusTone("cancelled"), "neutral");
  assert.equal(statusTone("something new"), "neutral");
  assert.equal(statusTone(""), "neutral");
  assert.equal(statusTone(null), "neutral");
  assert.equal(statusTone(undefined), "neutral");
});

test("every board kind has a tone", () => {
  for (const kind of KINDS) assert.ok(KIND_TONE[kind], kind);
  assert.equal(KIND_TONE.question, "warn");
  assert.equal(KIND_TONE.notice, "bad");
});

test("shortId shows a post id as its time and name", () => {
  assert.equal(shortId("20260906T182448Z-fable-go-planner"), "18:24 fable-go-planner");
  assert.equal(shortId("20261108T091047Z-quartz-34"), "09:10 quartz-34");
  assert.equal(shortId("TASK-085"), "TASK-085");
  assert.equal(shortId(""), "");
});

test("titleCase turns snake and kebab names into words", () => {
  assert.equal(titleCase("in_progress"), "In Progress");
  assert.equal(titleCase("needs-owner"), "Needs Owner");
  assert.equal(titleCase("step__limit"), "Step Limit");
  assert.equal(titleCase("done"), "Done");
  assert.equal(titleCase(""), "");
});

test("prioLabel names each priority band at and below its threshold", () => {
  assert.equal(prioLabel(-5), "low");
  assert.equal(prioLabel(0), "low");
  assert.equal(prioLabel(9), "low");
  assert.equal(prioLabel(10), "normal");
  assert.equal(prioLabel(29), "normal");
  assert.equal(prioLabel(30), "high");
  assert.equal(prioLabel(49), "high");
  assert.equal(prioLabel(50), "urgent");
  assert.equal(prioLabel(100), "urgent");
});

test("each priority choice is labelled with its own band", () => {
  for (const [p, label] of PRIORITIES) assert.ok(label.startsWith(prioLabel(p)), `${p}: ${label}`);
});
