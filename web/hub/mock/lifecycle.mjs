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

// A step at work or waiting for one (LifecycleJob::active).
const ACTIVE = new Set(["queued", "leased", "running", "blocked"]);
// Pull request states that are the owner's turn: the Sessions list's set
// (PR_NEEDS_OWNER in src/components/chat/task-groups.ts) and the hub's
// (OWNER_PR_STATES); lifecycle.test.mjs holds all three to the shared cases.
const OWNER_STATES = new Set(["needs_owner", "review_failed", "held_for_owner"]);
const REVIEW_STATES = new Set(["pr_open", "in_review", "needs_owner", "review_failed", "changes_requested", "merge_waiting", "merge_asked"]);

// A mark as the hub serializes it: empty note, at and url are left out.
const mark = (kind, id, extra = {}) => {
  const m = { kind, id: String(id) };
  for (const [k, v] of Object.entries(extra)) if (v) m[k] = v;
  return m;
};

// The task's pull request as the hub reads it from its events (task_pr),
// from the task detail's delivery: `last` names the newest pull-request
// event, `state` the review state of an open one, `hold` the hold's word.
function taskPr(d) {
  if (!d || !d.state || d.state === "none") return null;
  const st = d.state;
  const last = st === "merged" ? "land.merged" : st === "closed" ? "land.pr-closed" : st === "held" || st === "held_for_owner" ? "land.pr-held" : REVIEW_STATES.has(st) ? "land.pr" : "";
  if (!last) return null;
  const number = Number(d.number || 0);
  const at = d.at || "";
  return {
    last,
    number,
    url: number ? d.url || "" : "",
    state: last === "land.pr" ? st : "",
    hold: last === "land.pr-held" ? st : "",
    note: d.note || "",
    follow_up: last === "land.pr" ? d.follow_up || "" : "",
    opened_at: number ? at : "",
    merged_at: last === "land.merged" ? at : "",
    at,
    changes_asked: st === "changes_requested",
  };
}

// Where one task stands: { task, phase, waits_on, note, rework, steps, phases },
// the hub's derive_lifecycle (crates/hive-tasks/src/lifecycle.rs) read from a
// task detail instead of the store.
export function taskLifecycle(detail) {
  const t = detail.task;
  const all = (detail.jobs || [])
    .filter((j) => j.status !== "cancelled")
    .sort((a, b) => (a.created_at || "").localeCompare(b.created_at || "") || a.id.localeCompare(b.id));
  const jobs = all.filter((j) => j.kind !== "lead");
  const leads = all.filter((j) => j.kind === "lead");
  const pr = taskPr(detail.delivery || null);
  const active = jobs.filter((j) => ACTIVE.has(j.status));
  const delivering = jobs.some((j) => j.status === "done" && j.branch);
  const failed = jobs.filter((j) => j.status === "failed");
  const planning = leads.filter((j) => ACTIVE.has(j.status));
  // The lead column stands for the hub's `task.lead` event.
  const takenUp = !!t.lead || t.status !== "open" || all.length > 0;
  const rework = !!pr && (pr.changes_asked || (!!pr.opened_at && jobs.some((j) => j.created_at > pr.opened_at && (!pr.merged_at || j.created_at < pr.merged_at))));
  const ids = (js) => js.map((j) => j.id).join(", ");

  let phase, waits, note;
  if (pr && (pr.last === "land.merged" || pr.state === "merged")) [phase, waits, note] = ["done", "nobody", `pull request #${pr.number} merged`];
  else if (t.status === "done") [phase, waits, note] = ["done", "nobody", "closed"];
  else if (active.length) {
    const building = active.filter((j) => j.status !== "done");
    const at = building.find((j) => j.status === "leased" || j.status === "running");
    const blocked = building.find((j) => j.status === "blocked");
    if (!building.length) [phase, waits, note] = ["verify", "runner", "the runner checks the commits of " + ids(active)];
    else if (at) [phase, waits, note] = ["build", "agent", at.id + " is at work"];
    else if (blocked) [phase, waits, note] = ["build", "runner", blocked.id + " is blocked" + (blocked.result ? ": " + blocked.result : "")];
    else [phase, waits, note] = ["build", "runner", ids(building) + " waits for a free slot"];
  } else if (pr && pr.last === "land.pr") {
    const n = "pull request #" + pr.number;
    const st = pr.state;
    if (st === "merge_waiting") [phase, waits, note] = ["deliver", "github", n + ": the merge waits for its checks or GitHub"];
    else if (st === "merge_asked") [phase, waits, note] = ["deliver", "runner", n + ": the owner asked for the merge; the runner merges it"];
    else if (OWNER_STATES.has(st)) [phase, waits, note] = ["review", "owner", n + " waits for the owner"];
    else if (st === "changes_requested") [phase, waits, note] = ["review", "lead", n + ": the reviewer asked for changes"];
    else if (st === "in_review") [phase, waits, note] = ["review", "reviewer", n + " is under review"];
    else [phase, waits, note] = ["review", "reviewer", n + " is open for review"];
  } else if (pr && pr.last === "land.pr-closed" && !jobs.some((j) => j.status === "done" && j.branch && (j.finished_at || "") > pr.at)) {
    [phase, waits, note] = ["shape", "lead", `pull request #${pr.number} was closed unmerged; the lead plans the next step`];
  } else if (delivering) {
    const held = pr && pr.last === "land.pr-held";
    [phase, waits, note] = ["review", "runner", held ? "the pull request could not be opened" + (pr.note ? ": " + pr.note : "") : "the runner opens the task's pull request"];
  } else if (planning.length) [phase, waits, note] = ["shape", "lead", "the lead plans it in " + planning[0].id];
  else if (!takenUp) [phase, waits, note] = ["intake", "lead", "waits for the lead to take it up"];
  else if (failed.length) [phase, waits, note] = ["shape", "lead", failed[failed.length - 1].id + " failed; the lead plans the next step"];
  else [phase, waits, note] = ["shape", "lead", "the lead plans its jobs"];

  // The owner's turn outranks whatever the agents do (owner_turn); the
  // phase stays where the work is.
  const prNow = !pr ? "" : pr.last === "land.pr" ? pr.state : pr.last === "land.pr-held" && delivering ? pr.hold : "";
  if (phase !== "done" && (t.status === "review" || OWNER_STATES.has(prNow))) {
    waits = "owner";
    if (t.status === "review") note = "the lead asked the owner";
    else if (prNow !== "held_for_owner") note = `pull request #${pr.number} waits for the owner`;
    else note = "the pull request could not be opened; the owner decides" + (pr.note ? ": " + pr.note : "");
  }
  let stoppedIn = null;
  if (t.status === "cancelled") {
    stoppedIn = phase;
    [phase, waits, note] = ["cancelled", "nobody", "cancelled"];
  }

  // What each phase left behind, and when it began.
  const left = Object.fromEntries(PATH.map((p) => [p, { at: "", marks: [] }]));
  left.intake = { at: t.created_at || "", marks: [mark("task", t.id, { at: t.created_at }), ...(t.origin ? [mark("origin", t.origin)] : [])] };
  left.shape.marks = leads.map((j) => mark("lead", j.id, { note: j.status, at: j.created_at }));
  left.build = { at: jobs.length ? jobs[0].created_at || "" : "", marks: jobs.map((j) => mark("job", j.id, { note: j.status, at: j.created_at })) };
  const verified = jobs.filter((j) => j.status === "done" && j.branch);
  left.verify = {
    at: verified.map((j) => j.finished_at).filter(Boolean).sort()[0] || "",
    marks: verified.map((j) => mark("branch", j.branch, { note: j.id, at: j.finished_at })),
  };
  if (pr) {
    const review = [];
    if (pr.number) review.push(mark("pr", pr.number, { url: pr.url, at: pr.opened_at }));
    if (pr.state && pr.state !== "pr_open") review.push(mark("verdict", pr.state, { note: pr.note, at: pr.at }));
    else if (pr.last === "land.merged") review.push(mark("verdict", "merged", { at: pr.at }));
    if (pr.follow_up) review.push(mark("job", pr.follow_up, { note: "follow-up" }));
    if (pr.last === "land.pr-held" || pr.last === "land.pr-closed") review.push(mark("verdict", pr.last === "land.pr-held" ? pr.hold : "closed", { note: pr.note, at: pr.at }));
    left.review = { at: pr.opened_at, marks: review };
    if (pr.last === "land.merged" || pr.state === "merged") left.deliver = { at: pr.at, marks: [mark("merge", pr.number, { url: pr.url, at: pr.at })] };
    else if (pr.state === "merge_waiting" || pr.state === "merge_asked") left.deliver = { at: pr.at, marks: [] };
  }
  // The console links a job to its run (the hub's lifecycle_decorate).
  for (const j of all) {
    if (!j.harness_session) continue;
    for (const p of PATH) for (const m of left[p].marks) if ((m.kind === "job" || m.kind === "lead") && m.id === j.id) m.session = j.harness_session;
  }

  const reached =
    phase === "cancelled" ? (stoppedIn === "done" ? PATH.length : Math.max(0, PATH.indexOf(stoppedIn))) : phase === "done" ? PATH.length : PATH.indexOf(phase);
  const steps = PHASES.map(([p, , gate], i) => {
    const { at, marks } = left[p];
    const state =
      phase === "cancelled"
        ? i < reached
          ? "passed"
          : "skipped"
        : i < reached
          ? i >= PATH.indexOf("build") && !marks.length
            ? "skipped"
            : "passed"
          : i === reached
            ? "current"
            : "ahead";
    return { phase: p, state, ...(at ? { at } : {}), left: marks, gate };
  });
  return { task: t.id, phase, waits_on: waits, note, rework, steps, phases: PHASES.map(info) };
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
