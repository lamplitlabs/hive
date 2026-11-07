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
  // running: work actually under way right now - the only kind whose icon
  // spins. Something switched on but idle (an active lead, an automation, a
  // record in force) is "done"'s calm tick, never a spinner.
  running: "running",
  in_progress: "running",
  working: "running",
  leased: "running",
  starting: "running",
  provisioning: "running",
  cancelling: "running",
  reviewing: "running",
  stopping: "running",
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
  active: "done",
  on: "done",
  resumed: "done",
  live: "done",
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
  missing: "warning",
  expiring: "warning",
  lagging: "warning",
  failing: "failed",
  // info
  superseded: "info",
  released: "info",
  legacy: "info",
  pruned: "info",
  retired: "info",
  unknown: "neutral",
};

// Map a raw store state onto a status kind. One of the nine kinds itself
// ("warning", "waits", "info") stands for itself; then exact words, then a
// few suffix rules ("*_error" fails, "*_limit" needs fixing, a leftover
// "*ing" is work under way), then neutral.
export function statusKind(raw: string | null | undefined): StatusKind {
  const v = (raw || "").trim().toLowerCase();
  if (!v) return "neutral";
  const key = v.replace(/[\s-]+/g, "_");
  if ((STATUS_KINDS as string[]).includes(key)) return key as StatusKind;
  if (EXACT[key]) return EXACT[key];
  if (EXACT[v]) return EXACT[v];
  if (/(^|_)(error|failed|failure)$/.test(key)) return "failed";
  if (/(^|_)limit$/.test(key) || /^needs_/.test(key)) return "warning";
  if (/ing$/.test(key)) return "running";
  return "neutral";
}
