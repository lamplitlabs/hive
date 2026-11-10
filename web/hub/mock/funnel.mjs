// The `funnel` field of GET /api/v1/projects/<slug>/evolution offline: the
// counts the hub derives from a project's suggestions (how many were posted,
// agreed, are ready for the planner, in flight, built) and the stall alerts
// it words when nothing moves. Mirrors the hub's rules over the mock's
// suggestions fixture so the Self-improvement tab shows the same strip.

// A gate that means the suggestion is already work, not a hold.
const IN_FLIGHT_GATE = /^(queued|leased|blocked) as job\b/;
const OPEN_JOB = new Set(["queued", "leased", "blocked"]);

// The long gate reasons a suggestion row carries, by their short label.
const GATE_LABELS = [
  ["net votes", "net votes"],
  ["names no check", "names no check"],
  ["addressed to the owner", "for the owner"],
  ["without evidence", "no evidence"],
  ["points at owner-only paths", "owner-only paths"],
  ["Signal path is absent", "waits for landing"],
  ["duplicate of", "duplicate"],
  ["operating-layer cap", "operating-layer cap"],
  ["loop cap", "loop cap"],
  ["check selects no test", "check selects no test"],
  ["File: names a path", "file gone"],
  ["File holds none", "file stale"],
];

// The short label of a gate reason: a known prefix, else its first 24 chars.
export function gateLabel(gate) {
  const g = String(gate || "").trim();
  for (const [prefix, label] of GATE_LABELS) if (g.startsWith(prefix)) return label;
  return g.slice(0, 24);
}

const join = (items) =>
  items
    .filter(([n]) => n > 0)
    .map(([n, word]) => n + " " + word)
    .join(", ");

// The funnel over a project's suggestion rows (open and closed).
export function funnelOf(rows, { require_check = true, check_skips_quorum = true } = {}) {
  const list = Array.isArray(rows) ? rows : [];
  const by = (status) => list.filter((r) => r.status === status).length;
  const inFlight = list.filter((r) => r.job && OPEN_JOB.has(r.job_status));
  const blocked = inFlight.filter((r) => r.job_status === "blocked").length;
  const heldBy = new Map();
  for (const r of list) {
    if (r.status !== "eligible" || !r.gate || IN_FLIGHT_GATE.test(r.gate)) continue;
    const label = gateLabel(r.gate);
    heldBy.set(label, (heldBy.get(label) || 0) + 1);
  }
  const held = [...heldBy]
    .map(([gate, count]) => ({ gate, count }))
    .sort((a, b) => b.count - a.count || a.gate.localeCompare(b.gate));
  const f = {
    posted: list.length,
    checked: list.filter((r) => r.checked === true).length,
    needs_votes: by("needs votes"),
    eligible: by("eligible"),
    plannable: list.filter((r) => r.status === "eligible" && r.plannable === true).length,
    held,
    in_flight: inFlight.length,
    blocked,
    carded: by("carded"),
    built: by("built"),
    tried: by("tried"),
    rejected: by("rejected"),
    expired: by("expired"),
    closed: by("closed"),
    require_check,
    check_skips_quorum,
    alerts: [],
  };
  const heldLine = held.map((h) => h.count + " " + h.gate).join(", ");
  const settledLine = join([
    [f.built, "built"],
    [f.tried, "tried"],
    [f.rejected, "turned down"],
    [f.expired, "expired"],
  ]);
  const stalled = f.plannable === 0 && f.in_flight === 0;
  if (blocked > 0) f.alerts.push({ code: "blocked", count: blocked, text: blocked + " suggestion job(s) are blocked; see the project's jobs." });
  if (stalled && f.eligible > 0) f.alerts.push({ code: "held", count: f.eligible, text: "Every agreed suggestion is held (" + heldLine + "); nothing is ready to plan." });
  else if (stalled && f.needs_votes > 0) f.alerts.push({ code: "votes", count: f.needs_votes, text: f.needs_votes + " suggestion(s) wait for votes and none is agreed; the next ballot decides." });
  else if (stalled && f.posted === 0) f.alerts.push({ code: "empty", count: 0, text: "No suggestions yet: research posts them, agents vote, and the planner turns the agreed ones into jobs." });
  else if (stalled) f.alerts.push({ code: "settled", count: f.posted, text: "Every suggestion is settled (" + settledLine + ") and nothing new is waiting." });
  return f;
}
