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

  let phase;
  let waits = "nobody";
  let note = "";
  if (t.status === "done") phase = "done";
  else if (t.status === "cancelled") phase = "cancelled";
  else if (t.status === "open" && !t.lead) {
    phase = "intake";
    waits = "lead";
    note = "waits for the project's lead to take it up";
  } else if (!jobs.length) {
    phase = "shape";
    waits = "lead";
    note = "the lead plans its first step";
  } else if (d && d.state === "merged") {
    phase = "done";
    note = "pull request #" + d.number + " merged";
  } else if (jobs.some((j) => RUNNING.has(j.status))) {
    phase = "build";
    const at = jobs.find((j) => j.status === "leased" || j.status === "running");
    const blocked = jobs.find((j) => j.status === "blocked");
    waits = at ? "agent" : "runner";
    note = at ? at.id + " is at work" : blocked ? blocked.id + " is blocked" : jobs.filter((j) => j.status === "queued").map((j) => j.id).join(", ") + " waits for a free slot";
  } else if (!d || !d.state || d.state === "none") {
    phase = "verify";
    waits = "runner";
    note = "the runner checks the commits of " + jobs.filter((j) => j.status === "done").map((j) => j.id).join(", ");
  } else if (d.state === "merge_waiting" || d.state === "merge_asked") {
    phase = "deliver";
    waits = d.state === "merge_waiting" ? "github" : "runner";
    note = "pull request #" + d.number + (d.state === "merge_waiting" ? ": the merge waits for its checks or GitHub" : ": the owner asked for the merge; the runner merges it");
  } else {
    phase = "review";
    const n = "pull request #" + d.number;
    [waits, note] =
      d.state === "needs_owner" || d.state === "review_failed" || d.state === "held"
        ? ["owner", n + " waits for the owner"]
        : d.state === "changes_requested"
          ? ["lead", n + ": the reviewer asked for changes"]
          : ["reviewer", n + (d.state === "in_review" ? " is under review" : " is open for review")];
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
