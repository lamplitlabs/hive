// The live feed's words for the store's events: what an event's action reads
// as after the actor's name ("daemon found pull request #41 conflicts with
// main"), and the second line some events carry. Pure: no React and no
// runtime imports, so node --test can load it (the -state.ts pattern);
// components/pulse/live-feed.tsx renders it. The few sentences that need the
// setting labels or the status words stay there.
import type { StoreEvent } from "./api";

// What a post of each kind reads as in a sentence: "aurora picked up a thread".
export const POSTED: Record<string, string> = {
  claim: "picked up a thread",
  handoff: "handed work over",
  finding: "posted a finding",
  question: "asked a question",
  answer: "answered a question",
  suggestion: "made a suggestion",
  opinion: "gave an opinion",
  vote: "voted",
  amendment: "proposed a rule change",
  result: "posted a result",
  notice: "posted a notice",
  proposal: "made a proposal",
};

// A task's new status as what the agent did: "started on a task".
export const MOVED: Record<string, string> = {
  needs_owner: "asked you about a task",
  review: "asked you about a task",
  in_progress: "started on a task",
  planned: "planned a task",
  done: "finished a task",
  cancelled: "cancelled a task",
  open: "reopened a task",
};

// The rest of the store's actions, as verbs. Unknown actions read as their
// own words ("thing.happened" reads "thing happened"). For the landing and
// review events "a pull request" becomes "pull request #41" and "main" the
// branch it targets when the event names them (see eventVerb).
export const VERB: Record<string, string> = {
  "compute.requested": "asked for an extra machine",
  "compute.approved": "approved an extra machine",
  "compute.denied": "turned down an extra machine",
  "compute.released": "released an extra machine",
  "job.add": "queued a job",
  "job.done": "finished a job",
  "job.failed": "stopped a job that failed",
  "job.cancelled": "cancelled a job",
  // pulse-board jobs.rs and the runner's recover.rs: a job that ended badly
  // was tried again; the outcome picks the sentence (RECOVERED).
  "job.recover": "picked up a job that ended badly",
  // yield_lane.rs: a fleet run held so your own queued job gets its place.
  "job.yield": "paused a job to make room for yours",
  "message.sent": "sent a message",
  "record.created": "added a record",
  "record.updated": "updated a record",
  "record.revised": "updated a record",
  "executor.add": "added a machine",
  "executor.remove": "removed a machine",
  "executor.removed": "removed a machine",
  "task.created": "created a task",
  "task.amended": "changed a task",
  "task.cancelled": "cancelled a task",
  "land.landed": "merged finished work",
  "land.batch": "merged several finished branches, checked as one",
  "land.batch-failed": "could not merge several finished branches as one",
  "land.pushed": "pushed merged work",
  "land.held": "held back a merge",
  "land.reverted": "saw merged work reverted",
  // delivery.rs: the finished work's pull request was opened, or updated
  // (LAND_PR picks the sentence from the event's detail).
  "land.pr": "opened a pull request",
  "land.pr-checks": "saw the checks fail on a pull request",
  "land.pr-closed": "saw a pull request closed without being merged",
  "land.pr-conflict": "found a pull request conflicts with main and cannot merge",
  "land.pr-held": "held back a pull request until the work changes",
  "land.pr-refreshed": "brought a pull request up to date with main",
  "land.pr-rebuilt": "rebuilt a pull request without the commits of the one it built on, which was closed",
  "land.rebase": "asked a worker to redo finished work that no longer fits on main",
  "land.merged": "saw a pull request merged",
  "land.evidence": "read which files a merged pull request changed",
  "land.missing": "found a finished job's branch missing, so it never arrived",
  "land.measured": "measured how a merged change affected the work after it",
  // review.rs: the pull request review loop.
  "review.queued": "asked an agent to review a pull request",
  "review.changes": "asked for changes on a pull request",
  "review.merged": "merged a pull request after its review",
  "review.failed": "could not finish a pull request's review",
  "review.owner": "asked you to decide on a pull request",
  "hook.failed": "had a check fail",
  "project.add": "added the project",
  "project.removed": "removed the project",
  "project.restored": "restored the project",
  "settings.set": "changed a setting",
  "memory.remembered": "kept a memory for later runs",
  "memory.forgotten": "forgot a memory",
  "agent.cleanup": "cleaned up after a run",
  "agent.retire": "retired a worker after its run",
  "daemon.sweep": "ran the runner's cleanup",
  // stage_budget.rs: one step of the runner's pass ran past its time.
  "daemon.slow-stage": "took longer than planned on one step of its pass",
  // awake.rs: the computer slept; runs in flight lost that time.
  "daemon.slept": "noticed the computer was asleep",
  // The self-improvement loop (pulse-evolve): a fixture is one measurement
  // the loop scores every experiment with.
  "evolution.archived": "recorded the result of an improvement experiment",
  "evolution.feedback": "asked an agent to reflect on a finished improvement experiment",
  "evolution.fixture.added": "added a measurement to the improvement loop",
  "evolution.fixture.derived": "added or adjusted measurements the improvement loop read off the project",
  "evolution.fixture.derive_failed": "could not read new measurements off the project for the improvement loop",
  "evolution.fixture.enabled": "switched a measurement of the improvement loop on or off",
  "evolution.fixture.rearmed": "raised the bar of a measurement the project had outgrown",
  "evolution.fixture.retired": "switched off a measurement that kept disagreeing with itself",
  "evolution.fixture.seeded": "gave the improvement loop its starter measurements",
  "evolution.fixture.seed_failed": "could not give the improvement loop its starter measurements",
  "evolution.flat.resumed": "let the improvement loop carry on, as its stalled question no longer holds",
  "evolution.flat.self_reviewed": "let the improvement loop carry on after a question went unanswered",
  "evolution.healed": "added measurements so the stalled improvement loop can tell changes apart",
  "evolution.records.seeded": "wrote the improvement loop\u2019s rules and evaluators for the project",
  "evolution.records.seed_failed": "could not write the improvement loop\u2019s rules and evaluators",
  "evolution.fleet.prepared": "prepared workspaces for an improvement experiment",
  "evolution.fleet.cleaned": "cleared away an improvement experiment\u2019s workspaces",
  // Schedules (pulse-tasks schedules.rs): a repeating job, check or watch.
  "schedule.added": "added a schedule",
  "schedule.updated": "changed a schedule",
  "schedule.ended": "finished a schedule after its last run",
  "schedule.fired": "queued a job for a schedule",
  "schedule.ran": "ran a scheduled check",
  "schedule.run": "asked a schedule to run now",
  "schedule.red": "saw a scheduled check start failing",
  // Leads (pulse-tasks leads.rs, pulse-runner lead.rs): the agent that plans
  // a project's tasks into jobs.
  "lead.run": "asked a lead to plan now",
  "lead.seated": "gave a project a lead",
  "task.followup": "followed up on a task whose job failed",
  "task.lead": "handed a task to its lead to plan",
  "task.nudged": "moved a task\u2019s jobs up the queue because you asked about it",
  // Suggested work (pulse-tasks suggestions.rs): a task an agent proposes
  // and another agent reviews.
  "suggestion.new": "suggested a task",
  "suggestion.review": "reviewed a suggested task",
  "suggestion.reviewing": "started reviewing a suggested task",
  "suggestion.unreviewed": "put a suggested task back for another review",
  "suggestion.withdrawn": "withdrew a suggested task",
  // Tool connections (pulse-store mcp_servers.rs, pulse-hub mcp.rs): an MCP
  // server the agents call. The event's `name` replaces "a tool connection"
  // (TOOL_OR_SKILL).
  "mcp.added": "added a tool connection",
  "mcp.updated": "changed a tool connection",
  "mcp.enabled": "switched on a tool connection",
  "mcp.disabled": "switched off a tool connection",
  "mcp.checked": "checked a tool connection",
  "mcp.signed_in": "signed in to a tool connection",
  "mcp.signed_out": "signed out of a tool connection",
  // Skills (pulse-store skills.rs): instructions every worker reads before
  // the work they cover.
  "skill.added": "added a skill",
  "skill.updated": "changed a skill",
  "skill.enabled": "switched on a skill",
  "skill.disabled": "switched off a skill",
  // Records (pulse-store records.rs, pulse-records drafts, the runner's
  // auto_adopt.rs, pulse-board projects.rs).
  "record.status": "changed a record\u2019s status",
  "record.draft": "drafted a change to a record for you to adopt",
  "record.auto_adopted": "adopted a drafted record change while you were away",
  "record.imported": "imported a record from the project\u2019s files",
  // Projects (pulse-board projects.rs, pulse-store forget.rs, pulse-hub
  // lifecycle.rs).
  "project.moved": "moved the project to a new folder",
  "project.renamed": "renamed the project",
  "project.forgotten": "forgot the project and everything kept about it",
  "project.hook_failed": "could not finish setting up the project",
  // Agents (pulse-board agents.rs): the actor is the agent itself, except
  // for a prune, whose subject is the agent let go.
  "agent.join": "joined as a new agent",
  "agent.resume": "came back for another session",
  "agent.prune": "let go of an agent that had done nothing useful for a while",
  // pulse-cred-setup setup.rs: the push credential (push.credential).
  "cred.set": "saved the credential PULSE pushes with",
  "cred.clear": "removed the credential PULSE pushes with",
  // pulse-store vault.rs and pulse-runner secrets.rs: the team secret vault
  // (ADR-070 rule 4). Names only; a value is never in an event.
  "secret.key": "created the team's vault key",
  "secret.set": "saved a secret in the team vault",
  "secret.removed": "removed a secret from the team vault",
  "secret.granted": "gave a job the vault secrets its project declares",
  "secret.refused": "kept vault secrets from a job",
  "secret.read": "opened a vault secret for a job",
  // pulse-harness conversation.rs WAKE_ACTION/FINISH_ACTION: a message for a
  // conversation (`pulse chat wake`), its pull request merged.
  "chat.wake": "woke a conversation with a message",
  "chat.finish": "finished a conversation whose pull request was merged",
  // pulse-goals review.rs AMEND_ACTION: a decided goal proposal applied.
  "goal.amend": "set the project\u2019s goal from an accepted proposal",
  // pulse-tally amend.rs: a voted amendment written into the rules.
  "constitution.amended": "changed the project\u2019s rules after a vote",
  // pulse-hooks hooks.rs: a script or web address run on an event.
  "hook.added": "added a hook that runs when something happens",
  // pulse-store store.rs: a setting PULSE dropped is removed on upgrade.
  "setting.retired": "removed a setting PULSE no longer uses",
  // The project's own PULSE files (.pulse/automations, .pulse/pulse.yaml,
  // skill folders) read off the default branch: pulse-tasks manifest.rs and
  // the runner's manifest.rs.
  "manifest.synced": "applied the project\u2019s PULSE files from its default branch",
  "manifest.problems": "found problems in the project\u2019s PULSE files",
  "manifest.problems_cleared": "saw the problems in the project\u2019s PULSE files fixed",
  // Store upgrades and upkeep (pulse-store store.rs, records.rs, chat.rs,
  // archive.rs): written by PULSE itself when a new version opens the store.
  "schema.records": "made room in the store for records after an upgrade",
  "schema.superseded": "set aside an old store table that an upgrade replaced",
  "schema.widened": "rebuilt a store table to take new kinds of entries after an upgrade",
  "store.archived": "moved old history to the archive file to keep the store small",
  // Record seeding and migration (pulse-records bundle.rs and import.rs,
  // pulse-evolve starter.rs).
  "records.framework.seeded": "added PULSE\u2019s built-in records",
  "records.framework.seed_failed": "could not add PULSE\u2019s built-in records for the project",
  "records.migrated": "imported the project\u2019s records from its repository",
  // pulse-board remove.rs and the runner's pair.rs: a machine's projects or
  // address changed; pulse-tasks messages.rs: an agent read its messages;
  // pulse-board remove.rs: repeated board posts cleared out.
  "executor.update": "changed the settings of a machine",
  "message.read": "read its messages",
  "post.remove.duplicates": "removed repeated posts from the board",
};

// The tool connections and skills an event names: "added the skill
// pnpm-v11-overrides" rather than "added a skill".
const TOOL_OR_SKILL = /\ba (tool connection|skill)$/;

// suggestion.review's statuses (suggestions.rs).
const REVIEWED: Record<string, string> = {
  accepted: "accepted a suggested task",
  declined: "declined a suggested task",
  owner: "asked you to decide on a suggested task",
};

// evolution.archived's outcomes (archive.rs ArchiveOutcome).
const ARCHIVED: Record<string, string> = {
  improved: "kept an improvement experiment that made things better",
  neutral: "recorded an improvement experiment that changed nothing measurable",
  discarded: "discarded an improvement experiment that made things worse",
};

// A schedule's state after a run: the bad and good states of a check
// (red/green) and of a watch (broken/ok).
const FAILING = new Set(["red", "broken"]);
const PASSING = new Set(["green", "ok"]);

// job.recover's outcomes (recover.rs, pulse-board jobs.rs).
const RECOVERED: Record<string, string> = {
  retried: "tried a job again",
  replanned: "planned a job again from the start",
  requeued: "put a job back in the queue",
  held: "held a job back for you to look at",
};

// land.pr: one event for a new pull request and for each update of it.
function landPr(d: Record<string, unknown>): string {
  if (d.opened !== false) return VERB["land.pr"];
  if (d.refreshed === true) return "brought a pull request up to date with main";
  if (str(d.retargeted_from)) return "moved a pull request onto main";
  return "updated a pull request with new work";
}

export const rec = (d: unknown): Record<string, unknown> | null => (d && typeof d === "object" && !Array.isArray(d) ? (d as Record<string, unknown>) : null);
export const str = (v: unknown) => (typeof v === "string" ? v : "");
export const scalar = (v: unknown) => (typeof v === "string" || typeof v === "number" || typeof v === "boolean" ? String(v) : "");

// The author a post id ends with: "20260901T203300Z-cinder" -> "cinder".
const postAuthor = (id: string) => id.replace(/^\d{8}T\d{6}Z-?/, "");
const isMachine = (a: string) => a.startsWith("executor.");

// A duration in words, from milliseconds: "40 seconds", "3 minutes",
// "1 hour 5 minutes".
export function durationWords(ms: number): string {
  const unit = (n: number, word: string) => n + " " + word + (n === 1 ? "" : "s");
  const sec = Math.round(ms / 1000);
  if (sec < 1) return "under a second";
  if (sec < 60) return unit(sec, "second");
  const min = Math.round(sec / 60);
  if (min < 60) return unit(min, "minute");
  const h = Math.floor(min / 60);
  return unit(h, "hour") + (min % 60 ? " " + unit(min % 60, "minute") : "");
}

// Names a pull request and its target branch when the event carries them.
function namePr(sentence: string, d: Record<string, unknown> | null): string {
  let s = sentence;
  const n = d?.number;
  if (typeof n === "number" && n > 0) s = s.replace("a pull request", "pull request #" + n);
  const base = str(d?.base).trim();
  if (base && base !== "main") s = s.replace(/\bmain\b/, base);
  return s;
}

// What the event's action reads as after the actor's name, or "" when the
// sentence needs words this module does not hold (a setting's label, a
// task's status) - live-feed.tsx says those.
export function eventVerb(e: Pick<StoreEvent, "action" | "subject" | "detail">): string {
  const d = rec(e.detail);
  const a = e.action;
  if (a === "mention") return "was mentioned" + (str(d?.by) ? " by " + str(d?.by) : "");
  // pulse-board remove.rs: repeated posts cleared out, not a new post.
  if (a === "post.remove.duplicates") {
    const n = typeof d?.removed === "number" ? d.removed : 0;
    return n > 0 ? "removed " + n + " repeated post" + (n === 1 ? "" : "s") + " from the board" : VERB[a];
  }
  if (a === "post" || a.startsWith("post.")) {
    const kind = a.startsWith("post.") ? a.slice(5) : str(d?.kind);
    return POSTED[kind] ?? "posted on the board";
  }
  if (a === "decide") {
    const answer = str(d?.answer);
    const who = postAuthor(e.subject || "");
    const what = who ? " to " + who + "\u2019s proposal" : "";
    return answer === "yes" ? "said yes" + what : answer === "no" ? "said no" + what : "answered" + (who ? " " + who + "\u2019s proposal" : "");
  }
  if (a === "message.sent") return "sent a message" + (str(d?.task_id) ? " about " + str(d?.task_id) : "");
  if (isMachine(a) && VERB[a] && e.subject) return VERB[a].replace(/a machine$/, "the machine " + e.subject);
  // The subject is the worker's name: "retired the worker cinder after its run".
  if (a === "agent.retire" && e.subject) return VERB[a].replace(/a worker/, "the worker " + e.subject);
  if (a === "project.renamed" && str(d?.from) && str(d?.to)) return "renamed the project from \u201c" + str(d?.from) + "\u201d to \u201c" + str(d?.to) + "\u201d";
  if ((a.startsWith("mcp.") || a.startsWith("skill.")) && VERB[a]) {
    const name = str(d?.name).trim();
    let said = name ? VERB[a].replace(TOOL_OR_SKILL, "the $1 " + name) : VERB[a];
    if (a === "mcp.checked" && typeof d?.ok === "boolean") said += d.ok ? ", which answered" : ", which did not answer";
    return said;
  }
  if (a === "job.recover") return RECOVERED[str(d?.outcome)] ?? VERB[a];
  if (a === "daemon.slept" && typeof d?.seconds === "number" && d.seconds > 0) return "noticed the computer was asleep for about " + durationWords(d.seconds * 1000);
  if (a === "land.pr") return namePr(d ? landPr(d) : VERB[a], d);
  if (a === "land.measured" && d?.regressed === true) return "measured a merged change and found the work after it got slower";
  if (a === "suggestion.review") return REVIEWED[str(d?.status)] ?? VERB[a];
  if (a === "evolution.archived") return ARCHIVED[str(d?.outcome)] ?? VERB[a];
  if (a === "evolution.fixture.enabled" && typeof d?.enabled === "boolean") return (d.enabled ? "switched on" : "switched off") + " a measurement of the improvement loop";
  if (a === "schedule.ran" && FAILING.has(str(d?.state))) return "ran a scheduled check, which failed";
  if (a === "schedule.ran" && PASSING.has(str(d?.state))) return "ran a scheduled check, which passed";
  if (a === "schedule.red" && str(d?.kind) === "watch") return "saw a watched script stop working";
  if (a === "goal.amend" && d?.applied === false) return "could not set the project\u2019s goal from an accepted proposal";
  if (a === "manifest.synced" && names(d?.problems).length) return VERB[a] + ", some with problems";
  if (VERB[a]) return a.startsWith("land.") || a.startsWith("review.") ? namePr(VERB[a], d) : VERB[a];
  return a
    .replace(/[._-]+/g, " ")
    .replace(/\bdaemon\b/g, "runner")
    .replace(/\bexecutor\b/g, "machine")
    .trim();
}

const names = (v: unknown): string[] => (Array.isArray(v) ? v.filter((f): f is string => typeof f === "string" && f.trim() !== "") : []);
const firstFive = (items: string[]) => items.slice(0, 5).join(", ") + (items.length > 5 ? " +" + (items.length - 5) + " more" : "");

// The second line an event's own fields make, or "" to let the feed fall
// back to the detail's summary, title, note or text.
export function eventDetailText(e: Pick<StoreEvent, "action" | "detail">): string {
  const o = rec(e.detail);
  if (!o) return "";
  const a = e.action;
  // A red base names the checks main itself fails (the runner's `failures`),
  // not its 1500-char verify tail.
  if (a === "land.base-red") {
    const failures = names(o.failures);
    if (failures.length) return (str(o.note) ? str(o.note) + ": " : "") + firstFive(failures);
  }
  // stage_budget.rs: {"stage", "ms", "budget_ms", "ran", "skipped"}.
  if (a === "daemon.slow-stage" && (str(o.stage) || typeof o.ms === "number")) {
    const stage = str(o.stage) || "A step";
    let line = stage.charAt(0).toUpperCase() + stage.slice(1);
    if (typeof o.ms === "number") line += " took " + durationWords(o.ms);
    if (typeof o.budget_ms === "number" && o.budget_ms > 0) line += (typeof o.ms === "number" ? ", over its budget of " : " ran over its budget of ") + durationWords(o.budget_ms);
    const skipped = names(o.skipped);
    if (skipped.length) line += "; " + firstFive(skipped) + " wait" + (skipped.length === 1 ? "s" : "") + " for the next pass";
    return line;
  }
  if (a === "land.pr-checks") {
    const checks = names(o.checks);
    if (checks.length) return "Failing: " + firstFive(checks);
  }
  if (a === "land.pr-conflict" || a === "land.rebase") {
    const files = names(o.files);
    if (files.length) return "Both changed " + firstFive(files);
  }
  if (a === "review.failed" && str(o.error)) return str(o.error);
  if (a === "project.hook_failed" && str(o.error)) return str(o.error);
  if (a === "project.moved" && str(o.to)) return "Now in " + str(o.to);
  // A changed tool connection or skill names what changed.
  if ((a === "mcp.updated" || a === "skill.updated") && names(o.changed).length) return "Changed: " + firstFive(names(o.changed));
  if (a === "project.forgotten" && typeof o.total === "number") return o.total + " stored row" + (o.total === 1 ? "" : "s") + " removed";
  if (a === "goal.amend" && o.applied === false && str(o.why)) return str(o.why);
  if (a === "manifest.synced" || a === "manifest.problems") {
    const problems = names(o.problems);
    if (problems.length) return firstFive(problems);
  }
  return "";
}
