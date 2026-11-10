import assert from "node:assert/strict";
import { test } from "node:test";
import { alertTone, funnelLine, funnelStages, heldGroups, heldLine, ruleLine, settledLine } from "./funnel-state.ts";

// A funnel that moves: seven suggestions, three agreed, one ready, one in
// progress, two built, one abandoned.
const MOVING = {
  posted: 7,
  checked: 4,
  needs_votes: 1,
  eligible: 3,
  plannable: 1,
  held: [
    { gate: "names no check", count: 2 },
    { gate: "net votes", count: 1 },
  ],
  in_flight: 1,
  blocked: 0,
  carded: 1,
  built: 2,
  tried: 1,
  rejected: 1,
  expired: 0,
  closed: 4,
  require_check: true,
  check_skips_quorum: true,
  alerts: [],
};

const EMPTY = { ...MOVING, posted: 0, checked: 0, needs_votes: 0, eligible: 0, plannable: 0, held: [], in_flight: 0, carded: 0, built: 0, tried: 0, rejected: 0, closed: 0 };

test("funnelStages keeps the funnel order and adds Gave up only when something was tried", () => {
  assert.deepEqual(
    funnelStages(MOVING).map((s) => [s.key, s.label, s.n]),
    [
      ["posted", "Suggested", 7],
      ["checked", "With a check", 4],
      ["eligible", "Agreed", 3],
      ["plannable", "Ready to plan", 1],
      ["in_flight", "In progress", 1],
      ["built", "Built", 2],
      ["tried", "Gave up", 1],
    ],
  );
  const quiet = funnelStages({ ...MOVING, tried: 0 });
  assert.equal(quiet.length, 6, "no Gave up tile without an abandoned attempt");
  assert.equal(quiet[quiet.length - 1].key, "built");
  // An empty funnel still has every stage, all at zero, so the strip keeps its shape.
  assert.deepEqual(
    funnelStages(EMPTY).map((s) => s.n),
    [0, 0, 0, 0, 0, 0],
  );
});

test("funnelStages says under a stage what keeps part of it from moving on", () => {
  const by = Object.fromEntries(funnelStages(MOVING).map((s) => [s.key, s.sub]));
  assert.equal(by.posted, "1 needs votes");
  assert.equal(by.eligible, "3 held", "the held groups are summed");
  assert.equal(by.in_flight, undefined, "nothing blocked, nothing said");
  const stuck = Object.fromEntries(funnelStages({ ...MOVING, needs_votes: 2, blocked: 1 }).map((s) => [s.key, s.sub]));
  assert.equal(stuck.posted, "2 need votes");
  assert.equal(stuck.in_flight, "1 blocked");
  // Nothing waiting, nothing said under any stage.
  assert.ok(funnelStages(EMPTY).every((s) => s.sub === undefined));
  assert.ok(funnelStages({ ...EMPTY, held: undefined }).every((s) => s.sub === undefined), "an older hub without held");
});

test("funnelLine says the non-zero parts in order and names the empty board", () => {
  assert.equal(funnelLine(MOVING), "7 suggested, 3 agreed, 1 ready, 1 in progress, 2 built");
  assert.equal(funnelLine({ ...MOVING, plannable: 0, in_flight: 0 }), "7 suggested, 3 agreed, 2 built");
  assert.equal(funnelLine({ posted: 2, eligible: 0, plannable: 0, in_flight: 0, built: 0 }), "2 suggested");
  assert.equal(funnelLine(EMPTY), "No suggestions yet");
});

test("heldLine groups by gate and is empty when nothing is held", () => {
  assert.equal(heldLine(MOVING.held), "held: 2 names no check, 1 net votes");
  assert.equal(heldGroups(MOVING.held), "2 names no check, 1 net votes");
  assert.equal(heldLine([{ gate: "for the owner", count: 1 }]), "held: 1 for the owner");
  // A zero group (an older hub might send one) is not read out.
  assert.equal(heldLine([{ gate: "duplicate", count: 0 }, { gate: "loop cap", count: 1 }]), "held: 1 loop cap");
  assert.equal(heldLine([]), "");
  assert.equal(heldGroups([]), "");
  assert.equal(heldLine(undefined), "");
  assert.equal(heldLine(null), "");
});

test("alertTone warns about what is stuck and informs about an empty or settled funnel", () => {
  assert.equal(alertTone("held"), "warn");
  assert.equal(alertTone("votes"), "warn");
  assert.equal(alertTone("blocked"), "warn");
  assert.equal(alertTone("empty"), "info");
  assert.equal(alertTone("settled"), "info");
  assert.equal(alertTone("something-new"), "warn", "an unknown code is treated as stuck");
});

test("settledLine lists the closed tail without zeros and is empty when nothing settled", () => {
  assert.equal(settledLine(MOVING), "2 built, 1 tried, 1 turned down");
  assert.equal(settledLine({ built: 0, tried: 0, rejected: 2, expired: 3 }), "2 turned down, 3 expired");
  assert.equal(settledLine({ built: 1, tried: 0, rejected: 0, expired: 0 }), "1 built");
  assert.equal(settledLine(EMPTY), "");
});

test("ruleLine follows the two evolve settings", () => {
  assert.equal(ruleLine({ require_check: true, check_skips_quorum: true }), "A suggestion must name a check that fails on main and passes with its change; a check skips the vote count");
  assert.equal(ruleLine({ require_check: true, check_skips_quorum: false }), "A suggestion must name a check that fails on main and passes with its change");
  assert.equal(ruleLine({ require_check: false, check_skips_quorum: false }), "Suggestions without a check become work too");
  assert.equal(ruleLine({ require_check: false, check_skips_quorum: true }), "Suggestions without a check become work too; a check skips the vote count");
});
