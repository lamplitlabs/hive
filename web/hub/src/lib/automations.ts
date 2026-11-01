import type {
  AutomationEntry,
  AutomationProject,
  AutomationRepoFile,
  AutomationSchedule,
} from "./api";

// Automations: the words and the quick checks the page needs before the hub
// answers. The hub is the judge - its /automations/check runs the same code
// the daemon applies the file with - so everything here is either display
// (cadence words, labels) or a hint the owner sees while typing, never the
// rule itself.

export const EMPTY_ENTRY: AutomationEntry = {
  name: "",
  title: "",
  every: "1d",
  at: "",
  kind: "task",
  brief: "",
  skill: "",
  script: "",
  job_kind: "task",
  tier: 1,
  priority: 0,
  run_as: "",
  times: 0,
  paused: false,
};

// What each kind is for, in the owner's words. The job kinds queue a job
// with the brief (or the skill); the script kinds run a script and queue a
// job only when it says something is wrong.
export const KIND_WORDS: Record<string, { label: string; hint: string }> = {
  task: {
    label: "Task",
    hint: "Queue a task with the brief: the usual standing job.",
  },
  improve: {
    label: "Improvement",
    hint: "Queue an improvement: a change measured by the project's own checks.",
  },
  clean: {
    label: "Clean-up",
    hint: "Queue a clean-up pass: strays, stale branches, leftovers.",
  },
  reflect: {
    label: "Reflection",
    hint: "Queue a reflection: what the last runs taught, written to the board.",
  },
  check: {
    label: "Check (script)",
    hint: "Run a script; a non-zero exit queues a job to fix what went red.",
  },
  watch: {
    label: "Watch (script)",
    hint: "Run a script that watches something; output means it fired, and a job follows.",
  },
};

export const SCRIPT_KINDS = ["check", "watch"];
export const isScriptKind = (kind: string): boolean =>
  SCRIPT_KINDS.includes(kind);

// The cadences the form offers; the owner can still type any `30m`, `6h`,
// `1d`, `2w`, `hourly`, `daily`, `weekly`.
export const EVERY_PRESETS: { value: string; label: string }[] = [
  { value: "30m", label: "Every 30 minutes" },
  { value: "1h", label: "Every hour" },
  { value: "6h", label: "Every 6 hours" },
  { value: "12h", label: "Every 12 hours" },
  { value: "1d", label: "Every day" },
  { value: "2d", label: "Every 2 days" },
  { value: "1w", label: "Every week" },
  { value: "2w", label: "Every 2 weeks" },
];

// A title into the slug the file needs: "Audit the dependencies" -> "audit-the-dependencies".
export function slugify(title: string): string {
  return title
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64)
    .replace(/-+$/g, "");
}

export const isSlug = (s: string): boolean => /^[a-z0-9][a-z0-9-]*$/.test(s);

// `1d`, `6h`, `2w`, `90m`, `hourly`, `daily`, `weekly`, or bare minutes, as
// minutes; null when it is none of those (the hub says why).
export function everyMinutes(every: string): number | null {
  const t = every.trim().toLowerCase();
  if (t === "hourly") return 60;
  if (t === "daily") return 1440;
  if (t === "weekly") return 10080;
  const m = /^(\d+)\s*([mhdw])?$/.exec(t);
  if (!m) return null;
  const n = Number(m[1]);
  if (!n) return null;
  const unit = m[2] ?? "m";
  return (
    n * (unit === "m" ? 1 : unit === "h" ? 60 : unit === "d" ? 1440 : 10080)
  );
}

// The cadence in words, the way the hub says it: "every day at 02:00 UTC".
export function cadenceWords(every: string, at = ""): string {
  const minutes = everyMinutes(every);
  if (minutes === null) return every.trim() ? "every " + every.trim() : "";
  const unit = (n: number, one: string, many: string) =>
    n === 1 ? "every " + one : "every " + n + " " + many;
  const day = 1440;
  let words: string;
  if (minutes % (7 * day) === 0)
    words = unit(minutes / (7 * day), "week", "weeks");
  else if (minutes % day === 0) words = unit(minutes / day, "day", "days");
  else if (minutes % 60 === 0) words = unit(minutes / 60, "hour", "hours");
  else words = unit(minutes, "minute", "minutes");
  return at.trim() ? words + " at " + at.trim() + " UTC" : words;
}

// The hints the form shows while the owner types; the hub's check adds the
// rest (the cadence floor, the skill, the agent, the script on the branch).
export function entryHints(e: AutomationEntry, minMinutes = 30): string[] {
  const out: string[] = [];
  if (!e.name.trim())
    out.push("Give it a name: lowercase letters, digits and dashes.");
  else if (!isSlug(e.name.trim()))
    out.push(
      "The name must be lowercase letters, digits and dashes, starting with a letter or digit.",
    );
  const minutes = everyMinutes(e.every);
  if (!e.every.trim()) out.push("Say how often it runs.");
  else if (minutes === null)
    out.push("The cadence reads as 30m, 6h, 1d, 2w, hourly, daily or weekly.");
  else if (minMinutes > 0 && minutes < minMinutes)
    out.push(
      "The shortest cadence is " +
        minMinutes +
        " minutes (the setting schedule.min_minutes).",
    );
  if (e.at.trim() && !/^([01]\d|2[0-3]):[0-5]\d$/.test(e.at.trim()))
    out.push("The time of day reads as HH:MM, in UTC.");
  if (e.at.trim() && minutes !== null && minutes % 1440 !== 0)
    out.push(
      "A time of day anchors a cadence of whole days; this one is shorter, so the time is ignored.",
    );
  if (isScriptKind(e.kind)) {
    if (!e.script.trim())
      out.push(
        "A " +
          e.kind +
          " needs the script to run, relative to the repository root.",
      );
    else if (e.script.trim().startsWith("/") || e.script.includes(".."))
      out.push(
        "The script path is relative to the repository root, without a leading slash or ..",
      );
  } else if (!e.brief.trim() && !e.skill.trim())
    out.push("Say what each job does (a brief), or name the skill it follows.");
  if (e.tier !== 0 && e.tier !== 1) out.push("The tier is 0 or 1.");
  if (e.times < 0) out.push("Times is 0 (keep going) or how many jobs to run.");
  return out;
}

// A hand-set schedule as the entry that would keep it in the file.
export function entryFromSchedule(s: AutomationSchedule): AutomationEntry {
  return {
    name: s.origin_name || slugify(s.title) || s.id.toLowerCase(),
    title: s.title,
    every: s.every,
    at: s.at_time,
    kind: s.kind || "task",
    brief: s.brief,
    skill: s.skill,
    script: s.script,
    job_kind: isScriptKind(s.kind) ? s.job_kind || "task" : "task",
    tier: s.tier,
    priority: s.priority,
    run_as: s.agent,
    times: s.max_runs,
    paused: s.status === "paused",
  };
}

// A file entry with the file's defaults filled in, so the form has a value
// for every field.
export function entryFromFile(e: Partial<AutomationEntry>): AutomationEntry {
  return {
    ...EMPTY_ENTRY,
    ...e,
    kind: e.kind || "task",
    job_kind: e.job_kind || "task",
    tier: e.tier ?? 1,
    priority: e.priority ?? 0,
    times: e.times ?? 0,
    paused: !!e.paused,
  };
}

// The entry's own fields trimmed, defaults restored, for the hub.
export function cleanEntry(e: AutomationEntry): AutomationEntry {
  const script = isScriptKind(e.kind);
  return {
    name: e.name.trim(),
    title: e.title.trim(),
    every: e.every.trim(),
    at: e.at.trim(),
    kind: e.kind.trim() || "task",
    brief: script ? "" : e.brief.replace(/\r\n/g, "\n").trimEnd(),
    skill: script ? "" : e.skill.trim(),
    script: script ? e.script.trim() : "",
    job_kind: script ? e.job_kind.trim() || "task" : "",
    tier: e.tier,
    priority: e.priority,
    run_as: e.run_as.trim(),
    times: e.times,
    paused: e.paused,
  };
}

// The repository entry a schedule row came from, when the repository still
// has it: its own file under .pulse/automations/ first, else the item of the
// `schedules:` list.
export function fileEntryOf(
  p: AutomationProject,
  s: AutomationSchedule,
): AutomationEntry | null {
  if (!s.origin_name) return null;
  const f = p.automations?.files.find((x) => x.name === s.origin_name);
  if (f?.entry) return entryFromFile(f.entry);
  const e = p.file?.entries?.find((x) => x.name === s.origin_name);
  return e ? entryFromFile(e) : null;
}

// True for a path under the automations folder.
export const inFolder = (path: string): boolean =>
  path.startsWith(".pulse/automations/");

// The label the daemon puts before a problem of one automation.
const LABEL = /^schedule `([^`]*)`: /;

// Everything wrong with one automation file: what the file itself shows and
// what the daemon said about it on its last pass, without the label.
export function fileProblems(
  p: AutomationProject,
  f: AutomationRepoFile,
): string[] {
  const out = [...f.problems];
  for (const x of p.applied?.problems ?? []) {
    const m = LABEL.exec(x);
    if (m && m[1] === f.name) {
      const text = x.slice(m[0].length);
      if (!out.includes(text)) out.push(text);
    }
  }
  return out;
}

// Where one automation file stands: live (a schedule runs from it), paused,
// held (something is wrong with it, so the daemon leaves it), or pending
// (the daemon has not applied it yet).
export function fileState(
  p: AutomationProject,
  f: AutomationRepoFile,
): {
  state: "live" | "paused" | "held" | "pending";
  words: string;
  row: AutomationSchedule | null;
} {
  const row =
    p.schedules.find((s) => s.status !== "ended" && s.origin_path === f.path) ??
    null;
  if (!f.entry || fileProblems(p, f).length)
    return { state: "held", words: "held", row };
  if (row)
    return row.status === "paused"
      ? { state: "paused", words: "paused", row }
      : { state: "live", words: "live", row };
  return { state: "pending", words: "not applied yet", row: null };
}

// The daemon's problems that no file of the page already shows: what it
// said about the list file and about each automation file sits beside
// them, so only the rest (a file it could not read, a name it found twice
// in the list) needs its own notice.
export function daemonProblems(p: AutomationProject): string[] {
  const shown = new Set<string>(p.file?.problems ?? []);
  const names = new Set<string>();
  for (const f of p.automations?.files ?? []) {
    names.add(f.name);
    for (const x of f.problems) shown.add(x);
  }
  return (p.applied?.problems ?? []).filter((x) => {
    if (shown.has(x) || shown.has(x.replace(LABEL, ""))) return false;
    const m = LABEL.exec(x);
    return !(m && names.has(m[1]));
  });
}

// Where a schedule is kept, in the owner's words.
export function keptWords(
  s: Pick<AutomationSchedule, "origin_path" | "created_by">,
): string {
  if (s.origin_path) return "in " + s.origin_path;
  return s.created_by === "human" ? "set by you" : "set by " + s.created_by;
}

// What a check or a watch last said, for the row.
export function stateWords(
  s: Pick<AutomationSchedule, "kind" | "state" | "fails">,
): { text: string; tone: "ok" | "bad" | "neutral" } | null {
  if (!isScriptKind(s.kind) || !s.state) return null;
  if (s.state === "green" || s.state === "ok")
    return { text: s.kind === "check" ? "green" : "quiet", tone: "ok" };
  return {
    text:
      (s.state === "red" ? "red" : s.state) +
      (s.fails > 1 ? " ×" + s.fails : ""),
    tone: "bad",
  };
}

// Rows the page shows: live ones first, ended last, each group as the hub
// ordered it (by what fires next).
export function sortRows(rows: AutomationSchedule[]): AutomationSchedule[] {
  const rank = (s: AutomationSchedule) =>
    s.status === "active" ? 0 : s.status === "paused" ? 1 : 2;
  return [...rows].sort((a, b) => rank(a) - rank(b));
}
