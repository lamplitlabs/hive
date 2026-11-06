// Status vocabulary for the badge: the store's many states (task, job, run,
// vote, machine, connection, thread, delivery) fold into nine kinds, each an
// icon plus a word plus one tint. Pure: no React, no icons, no imports (node
// --test resolves nothing for us); the word itself comes from lib/words.ts
// statusWord in status-badge.tsx.

export type StatusKind = "running" | "done" | "waits" | "queued" | "paused" | "failed" | "warning" | "info" | "neutral";

export const STATUS_KINDS: StatusKind[] = ["running", "done", "waits", "queued", "paused", "failed", "warning", "info", "neutral"];

// Tailwind utilities for a status kind: the 10% wash and the ink. Spelled
// out so the Tailwind scanner sees every class.
export const STATUS_CLASS: Record<StatusKind, string> = {
  running: "bg-status-running-soft text-status-running",
  done: "bg-status-done-soft text-status-done",
  waits: "bg-status-waits-soft text-status-waits",
  queued: "bg-status-neutral-soft text-status-neutral",
  paused: "bg-status-neutral-soft text-status-neutral",
  failed: "bg-status-failed-soft text-status-failed",
  warning: "bg-status-warning-soft text-status-warning",
  info: "bg-status-info-soft text-status-info",
  neutral: "bg-status-neutral-soft text-status-neutral",
};

const EXACT: Record<string, StatusKind> = {
  // running
  running: "running",
  active: "running",
  in_progress: "running",
  working: "running",
  leased: "running",
  starting: "running",
  provisioning: "running",
  cancelling: "running",
  reviewing: "running",
  resumed: "running",
  live: "running",
  // done
  done: "done",
  completed: "done",
  complete: "done",
  finished: "done",
  succeeded: "done",
  success: "done",
  landed: "done",
  merged: "done",
  accepted: "done",
  approved: "done",
  closed: "done",
  answered: "done",
  delivered: "done",
  passed: "done",
  agreed: "done",
  eligible: "done",
  proceeded: "done",
  submitted: "done",
  online: "done",
  connected: "done",
  enabled: "done",
  ready: "done",
  ok: "done",
  healthy: "done",
  // waits on you
  waiting: "waits",
  awaiting: "waits",
  asked: "waits",
  review: "waits",
  for_owner: "waits",
  owner: "waits",
  held: "waits",
  hold: "waits",
  needs_owner: "waits",
  needs_you: "waits",
  proposed: "waits",
  suggested: "waits",
  needs_votes: "waits",
  "waiting for you": "waits",
  // queued
  queued: "queued",
  planned: "queued",
  open: "queued",
  pending: "queued",
  scheduled: "queued",
  idle: "queued",
  draft: "queued",
  // paused
  paused: "paused",
  drained: "paused",
  disabled: "paused",
  off: "paused",
  deferred: "paused",
  inactive: "paused",
  // failed
  failed: "failed",
  error: "failed",
  rejected: "failed",
  cancelled: "failed",
  canceled: "failed",
  denied: "failed",
  declined: "failed",
  blocked: "failed",
  stopped: "failed",
  dead: "failed",
  unreachable: "failed",
  provider_error: "failed",
  malformed: "failed",
  ambiguous: "failed",
  lost: "failed",
  withdrawn: "failed",
  // needs fixing
  stale: "warning",
  expired: "warning",
  silent: "warning",
  quiet: "warning",
  partially: "warning",
  step_limit: "warning",
  time_limit: "warning",
  format_errors: "warning",
  not_signed_in: "warning",
  unchecked: "warning",
  short: "warning",
  "no expiry": "warning",
  degraded: "warning",
  // info
  superseded: "info",
  released: "info",
  legacy: "info",
  pruned: "info",
  retired: "info",
  unknown: "neutral",
};

// Map a raw store state onto a status kind. Exact words first, then a few
// suffix rules ("*_error" fails, "*_limit" needs fixing), then neutral.
export function statusKind(raw: string | null | undefined): StatusKind {
  const v = (raw || "").trim().toLowerCase();
  if (!v) return "neutral";
  const key = v.replace(/[\s-]+/g, "_");
  if (EXACT[key]) return EXACT[key];
  if (EXACT[v]) return EXACT[v];
  if (/(^|_)(error|failed|failure)$/.test(key)) return "failed";
  if (/(^|_)limit$/.test(key) || /^needs_/.test(key)) return "warning";
  if (/ing$/.test(key)) return "running";
  return "neutral";
}
