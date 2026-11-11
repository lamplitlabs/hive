import assert from "node:assert/strict";
import { test } from "node:test";
import { STATUS_CLASS, STATUS_KINDS, statusKind } from "./status-badge-state.ts";

test("every status kind has a tint class", () => {
  for (const k of STATUS_KINDS) assert.match(STATUS_CLASS[k], /^bg-status-[a-z]+-soft text-status-[a-z]+$/, k);
  assert.equal(STATUS_CLASS.queued, STATUS_CLASS.neutral, "queued and paused share the grey");
  assert.equal(STATUS_CLASS.paused, STATUS_CLASS.neutral);
});

test("running states: only work under way right now spins", () => {
  for (const s of ["running", "in_progress", "working", "leased", "starting", "provisioning", "cancelling", "stopping", "deploying"]) assert.equal(statusKind(s), "running", s);
});

test("done states, including things switched on but idle", () => {
  for (const s of ["done", "completed", "landed", "merged", "accepted", "closed", "succeeded", "finished", "passed", "eligible", "online", "connected", "enabled", "delivered", "answered"]) assert.equal(statusKind(s), "done", s);
  // An active lead, an automation that is on, a record in force: no spinner.
  for (const s of ["active", "Active", "on", "resumed", "live"]) assert.equal(statusKind(s), "done", s);
});

test("a kind's own name stands for itself", () => {
  for (const k of STATUS_KINDS) assert.equal(statusKind(k), k, k);
  // "warning" ends in -ing but is no work under way.
  assert.equal(statusKind("Warning"), "warning");
  assert.equal(statusKind("failing"), "failed");
  for (const s of ["missing", "expiring", "lagging"]) assert.equal(statusKind(s), "warning", s);
});

test("states that wait on the owner", () => {
  for (const s of ["waiting", "asked", "for_owner", "held", "hold", "awaiting", "review", "owner", "proposed", "suggested", "needs votes", "needs_votes"]) assert.equal(statusKind(s), "waits", s);
});

test("queued and paused", () => {
  for (const s of ["queued", "planned", "open", "pending", "scheduled", "idle", "draft"]) assert.equal(statusKind(s), "queued", s);
  for (const s of ["paused", "drained", "disabled", "deferred"]) assert.equal(statusKind(s), "paused", s);
});

test("failed states", () => {
  for (const s of ["failed", "error", "rejected", "cancelled", "canceled", "denied", "declined", "blocked", "stopped", "dead", "unreachable", "provider_error", "lost", "withdrawn"]) assert.equal(statusKind(s), "failed", s);
  assert.equal(statusKind("checkout_error"), "failed", "an *_error suffix fails");
});

test("states that need fixing", () => {
  for (const s of ["stale", "expired", "silent", "partially", "step_limit", "time_limit", "format_errors", "not_signed_in", "unchecked"]) assert.equal(statusKind(s), "warning", s);
  assert.equal(statusKind("token_limit"), "warning", "a *_limit suffix needs fixing");
});

test("info and neutral", () => {
  for (const s of ["superseded", "released", "retired"]) assert.equal(statusKind(s), "info", s);
  for (const s of ["", "   ", "unknown", "something_else"]) assert.equal(statusKind(s), "neutral", JSON.stringify(s));
  assert.equal(statusKind(null), "neutral");
  assert.equal(statusKind(undefined), "neutral");
});

test("statusKind is case- and separator-insensitive", () => {
  assert.equal(statusKind("In Progress"), "running");
  assert.equal(statusKind("in-progress"), "running");
  assert.equal(statusKind(" DONE "), "done");
  assert.equal(statusKind("Waiting for you"), "waits");
});
