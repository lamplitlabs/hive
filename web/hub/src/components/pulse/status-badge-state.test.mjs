import assert from "node:assert/strict";
import { test } from "node:test";
import { STATUS_CLASS, STATUS_KINDS, statusKind } from "./status-badge-state.ts";

test("every status kind has a tint class", () => {
  for (const k of STATUS_KINDS) assert.match(STATUS_CLASS[k], /^bg-status-[a-z]+-soft text-status-[a-z]+$/, k);
  assert.equal(STATUS_CLASS.queued, STATUS_CLASS.neutral, "queued and paused share the grey");
  assert.equal(STATUS_CLASS.paused, STATUS_CLASS.neutral);
});

test("running states", () => {
  for (const s of ["running", "active", "in_progress", "working", "leased", "starting", "provisioning"]) assert.equal(statusKind(s), "running", s);
});

test("done states", () => {
  for (const s of ["done", "completed", "landed", "merged", "accepted", "closed", "succeeded", "finished", "passed", "eligible", "online", "connected", "enabled", "delivered", "answered"]) assert.equal(statusKind(s), "done", s);
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
