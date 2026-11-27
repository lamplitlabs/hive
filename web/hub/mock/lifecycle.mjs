// The mock hub's GET tasks/{id}/lifecycle and GET tasks/lifecycle?project=P
// (TASK-141): a small reading of a fixture task's status, jobs and
// delivery into the phases crates/hive-tasks/src/lifecycle.rs derives, so
// the console's Lifecycle tab and Phases view render without a store. The
// rules are the derivation's in brief; the hub's answer is the real one.

export const PHASES = [
  ["intake", "Intake", "a lead takes the task up: the owner's task passes no other gate, and suggested work came with a different agent's verdict"],
  ["shape", "Shape and plan", "a job exists for the task, Tier 0 or Tier 1"],
  ["build", "Build", "every step's run has ended"],
  ["verify", "Verify", "each step's own commits passed the project's check, and none of the task's jobs is queued, running or blocked"],
  ["review", "Review", "the task's pull request is open and the reviewer (or the owner) says merge, with GitHub's own gate passed"],
  ["deliver", "Deliver", "the pull request is merged on the upstream"],
];
const ENDS = [
  ["done", "Done", ""],
  ["cancelled", "Cancelled", ""],
];
const info = ([phase, label, gate]) => ({ phase, label, gate });
const PATH = PHASES.map(([p]) => p);

const RUNNING = new Set(["queued", "leased", "running", "blocked"]);

// Where one task stands: { task, phase, waits_on, note, rework, steps, phases }.
export function taskLifecycle(detail) {
  const t = detail.task;
  const jobs = (detail.jobs || []).filter((j) => j.kind !== "lead");
  const leads = (detail.jobs || []).filter((j) => j.kind === "lead");
  const d = detail.delivery || null;
  const left = Object.fromEntries(PATH.map((p) => [p, []]));
  left.intake.push({ kind: "task", id: t.id, at: t.created_at });
  if (t.origin) left.intake.push({ kind: "origin", id: t.origin });
  for (const j of leads) left.shape.push({ kind: "lead", id: j.id, note: j.status, at: j.created_at });
  for (const j of jobs) {
    left.build.push({ kind: "job", id: j.id, note: j.status, at: j.created_at, ...(j.harness_session ? { session: j.harness_session } : {}) });
    if (j.branch && j.status === "done") left.verify.push({ kind: "branch", id: j.branch, note: j.id, at: j.finished_at });
  }
  if (d && d.number) left.review.push({ kind: "pr", id: String(d.number), url: d.url, at: d.at });
  if (d && d.state && d.state !== "none" && d.state !== "merged") left.review.push({ kind: "verdict", id: d.state, note: d.note || "", at: d.at });
  if (d && d.state === "merged") left.deliver.push({ kind: "merge", id: String(d.number), url: d.url, at: d.at });

  // The hub's order (derive_lifecycle): a merge, the task's own end, a step
  // at work, the pull request, a finished branch, the lead's sitting, the
  // lead's turn; then the task's status has the last word.
  const st = d && d.state && d.state !== "none" ? d.state : "";
  const n = "pull request #" + (d ? d.number : "");
  const active = jobs.filter((j) => RUNNING.has(j.status));
  const delivering = jobs.some((j) => j.status === "done" && j.branch);
  let phase;
  let waits = "nobody";
  let note = "";
  if (st === "merged") {
    phase = "done";
    note = n + " merged";
  } else if (t.status === "done") phase = "done";
  else if (active.length) {
    phase = "build";
    const at = active.find((j) => j.status === "leased" || j.status === "running");
    const blocked = active.find((j) => j.status === "blocked");
    waits = at ? "agent" : "runner";
    note = at ? at.id + " is at work" : blocked ? blocked.id + " is blocked" : active.map((j) => j.id).join(", ") + " waits for a free slot";
  } else if (st === "merge_waiting" || st === "merge_asked") {
    phase = "deliver";
    waits = st === "merge_waiting" ? "github" : "runner";
    note = n + (st === "merge_waiting" ? ": the merge waits for its checks or GitHub" : ": the owner asked for the merge; the runner merges it");
  } else if (st && st !== "held" && st !== "closed") {
    phase = "review";
    // The one rule for "waits on you", the Sessions list's (PR_NEEDS_OWNER).
    [waits, note] =
      st === "needs_owner" || st === "review_failed"
        ? ["owner", n + " waits for the owner"]
        : st === "changes_requested"
          ? ["lead", n + ": the reviewer asked for changes"]
          : ["reviewer", n + (st === "in_review" ? " is under review" : " is open for review")];
  } else if (st === "closed") {
    phase = "shape";
    waits = "lead";
    note = n + " was closed unmerged; the lead plans the next step";
  } else if (delivering) {
    // A held pull request is the runner's to try again, not the owner's turn.
    phase = "review";
    waits = "runner";
    note = st === "held" ? "the pull request could not be opened" + (d.note ? ": " + d.note : "") : "the runner opens the task's pull request";
  } else if (leads.some((j) => RUNNING.has(j.status))) {
    phase = "shape";
    waits = "lead";
    note = "the lead plans it in " + leads.find((j) => RUNNING.has(j.status)).id;
  } else if (t.status === "open" && !t.lead && !(detail.jobs || []).length) {
    phase = "intake";
    waits = "lead";
    note = "waits for the lead to take it up";
  } else {
    phase = "shape";
    waits = "lead";
    const failed = jobs.filter((j) => j.status === "failed").pop();
    note = failed ? failed.id + " failed; the lead plans the next step" : "the lead plans its jobs";
  }
  if (t.status === "review" && phase !== "done") {
    waits = "owner";
    note = "the lead asked the owner";
  }
  if (t.status === "cancelled") {
    phase = "cancelled";
    waits = "nobody";
    note = "cancelled";
  }
  const end = phase === "done" || phase === "cancelled";
  const idx = PATH.indexOf(phase);
  const steps = PHASES.map(([p, , gate], i) => {
    const state = end
      ? left[p].length || phase === "done"
        ? "passed"
        : "skipped"
      : i < idx
        ? "passed"
        : i === idx
          ? "current"
          : "ahead";
    const first = left[p].map((m) => m.at).filter(Boolean).sort()[0];
    return { phase: p, state, ...(first && state !== "ahead" ? { at: first } : {}), left: left[p], gate };
  });
  const rework = (detail.jobs || []).some((j) => j.kind === "follow-up" || /follow-up/i.test(j.title || "")) || (d && d.state === "changes_requested");
  return { task: t.id, phase, waits_on: waits, note: note || (phase === "done" ? "closed" : phase === "cancelled" ? "cancelled" : ""), rework: !!rework, steps, phases: PHASES.map(info) };
}

// A project's tasks by phase: { project, phases: [{phase,label,gate,tasks}], tasks: [...] }.
export function projectLifecycle(project, details) {
  const rows = details.map(taskLifecycle).map(({ task, phase, waits_on, note, rework }) => ({ task, phase, waits_on, note, rework }));
  return {
    project,
    phases: [...PHASES, ...ENDS].map((p) => ({ ...info(p), tasks: rows.filter((r) => r.phase === p[0]).map((r) => r.task) })),
    tasks: rows,
  };
}
