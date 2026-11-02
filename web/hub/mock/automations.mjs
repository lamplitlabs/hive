// Automations mock for `automations` (crates/pulse-hub/src/automations.rs):
// the standing jobs of each project, kept in the repository (one file per
// automation under `.pulse/automations/`, or an item of the older
// `schedules:` list in `.pulse/pulse.yaml`) or in the store, with the checks
// the hub runs on a draft and the writes the console makes. Pure functions
// over in-memory state, so the shape is testable without a server:
// createAutomationsMock({...}) mirrors mock/api/automations.json plus the
// deps server.mjs injects.
//
// The list body is { projects, limits, kinds, script_kinds, script_job_kinds };
// check answers { yaml, problems, warning, path, branch, form }; a write
// answers { where, schedule, yaml, warning, path?, branch?, commit?, synced? }
// (201 for a new one, 200 for an edit, 400 with `problems` when a check
// fails); remove and the actions answer the same shape plus `removed` where
// an automation left the repository. Those keys are the contract the owner
// console reads - freeze them here.
const pad = (n, w) => String(n).padStart(w, "0");
const SLUG = /^[a-z0-9][a-z0-9-]*$/;
const SCRIPT_KINDS = ["check", "watch"];
const DIR = ".pulse/automations";
const scriptInPulse = (path) =>
  String(path).replace(/^\.\//, "").startsWith(".pulse/") &&
  String(path).replace(/^\.\//, "").length > ".pulse/".length;
const scriptHome = (name) => DIR + "/" + (name || "<name>") + ".sh";

export const KINDS = ["task", "improve", "clean", "reflect"];
export const SCRIPT_JOB_KINDS = ["task", "improve"];

export function everyMinutes(every) {
  const t = String(every || "")
    .trim()
    .toLowerCase();
  if (t === "hourly") return 60;
  if (t === "daily") return 1440;
  if (t === "weekly") return 10080;
  const m = /^(\d+)\s*([mhdw])?$/.exec(t);
  if (!m || !Number(m[1])) return null;
  const unit = m[2] || "m";
  return (
    Number(m[1]) *
    (unit === "m" ? 1 : unit === "h" ? 60 : unit === "d" ? 1440 : 10080)
  );
}

export function everyWords(minutes, at) {
  const one = (n, a, b) => (n === 1 ? "every " + a : "every " + n + " " + b);
  let w;
  if (minutes % 10080 === 0) w = one(minutes / 10080, "week", "weeks");
  else if (minutes % 1440 === 0) w = one(minutes / 1440, "day", "days");
  else if (minutes % 60 === 0) w = one(minutes / 60, "hour", "hours");
  else w = one(minutes, "minute", "minutes");
  return at && minutes % 1440 === 0 ? w + " at " + at + " UTC" : w;
}

// A scalar the way the hub writes it: bare when it is plainly a word, quoted
// otherwise (the hub is stricter; this is a mock).
function scalar(s) {
  const t = String(s);
  const lower = t.toLowerCase();
  const numeric =
    /^0[xob]/.test(lower) ||
    (/^[0-9e._+-]+$/.test(lower) && /[0-9]/.test(lower)) ||
    [".inf", "-.inf", "+.inf", ".nan"].includes(lower);
  if (
    /^[A-Za-z0-9][A-Za-z0-9 ._/()',-]*$/.test(t) &&
    !t.endsWith(" ") &&
    /[A-Za-z]/.test(t) &&
    !/^(yes|no|on|off|true|false|null|y|n|~)$/i.test(t) &&
    !numeric
  )
    return t;
  return JSON.stringify(t);
}

// The fields after the name, one `key: value` line each at `indent`, in the
// hub's order, defaults left out.
function yamlFields(e, indent) {
  const out = [];
  const line = (k, v) => out.push(indent + k + ": " + v);
  if (e.title) line("title", scalar(e.title));
  if (e.every) line("every", scalar(e.every));
  if (e.at) line("at", JSON.stringify(e.at));
  if (e.kind && e.kind !== "task") line("kind", e.kind);
  if (e.brief) {
    if (e.brief.includes("\n")) {
      line("brief", "|-");
      for (const l of e.brief.split("\n")) out.push(l ? indent + "  " + l : "");
    } else line("brief", scalar(e.brief));
  }
  if (e.skill) line("skill", scalar(e.skill));
  if (e.script) line("script", scalar(e.script));
  if (SCRIPT_KINDS.includes(e.kind) && e.job_kind && e.job_kind !== "task")
    line("job_kind", e.job_kind);
  if (e.tier !== undefined && e.tier !== 1) line("tier", String(e.tier));
  if (e.priority) line("priority", String(e.priority));
  if (e.run_as) line("run_as", scalar(e.run_as));
  if (e.times) line("times", String(e.times));
  if (e.paused) line("paused", "true");
  return out.join("\n");
}

// One list item of `schedules:`, in the hub's field order, defaults left out.
export function yamlItem(e, indent = "  ") {
  const fields = yamlFields(e, indent + "  ");
  return indent + "- name: " + scalar(e.name) + "\n" + fields + "\n";
}

// One file of `.pulse/automations/`: the fields at the top level; the
// file's name is the automation's.
export function yamlFile(e) {
  return yamlFields(e, "") + "\n";
}

// deps: { data, now, emit } - data is the fixture (mutated in place), now()
// the clock, emit the activity log.
export function createAutomationsMock({
  data,
  now = () => new Date().toISOString(),
  emit = () => {},
}) {
  const failure = (status, error, why, fix, extra = {}) => ({
    status,
    body: { error, why, fix, ...extra },
  });
  const ok = (status, body) => ({ status, body });
  const projectOf = (slug) => data.projects.find((p) => p.slug === slug);
  const allRows = () => data.projects.flatMap((p) => p.schedules);
  const nextId = () =>
    "SCHED-" +
    pad(
      allRows().reduce(
        (m, r) => Math.max(m, Number(r.id.split("-").pop()) || 0),
        0,
      ) + 1,
      3,
    );
  const nextCommit = () =>
    Array.from(
      { length: 40 },
      () => "0123456789abcdef"[Math.floor(Math.random() * 16)],
    ).join("");
  const filesOf = (p) => p.automations?.files || [];
  const fileOf = (p, name) => filesOf(p).find((f) => f.name === name);
  const listEntryOf = (p, name) =>
    (p.file?.entries || []).find((e) => e.name === name);
  // Where a name lives: its own file first, else the list.
  const holder = (p, name) => {
    const f = fileOf(p, name);
    if (f) return { form: "file", path: f.path, file: f };
    const e = listEntryOf(p, name);
    return e ? { form: "list", path: p.file.path, entry: e } : null;
  };
  const liveNames = (p) => {
    const names = (p.file?.entries || []).map((e) => e.name);
    for (const f of filesOf(p)) if (!names.includes(f.name)) names.push(f.name);
    return names;
  };
  const pathOf = (name) => DIR + "/" + name + ".yaml";
  // A check or watch whose script sits outside .pulse/ is flagged with the
  // place it belongs, the way the hub's schedule_json does.
  const flagScript = (s) => {
    const outside = SCRIPT_KINDS.includes(s.kind) && !!s.script && !scriptInPulse(s.script);
    s.script_outside = outside;
    s.script_home = outside
      ? scriptHome(
          s.origin_name ||
            String(s.title || s.id)
              .toLowerCase()
              .replace(/[^a-z0-9]+/g, "-")
              .replace(/^-+|-+$/g, ""),
        )
      : "";
    return s;
  };
  // The script the automation a draft edits already runs, or "".
  const keptScript = (p, body) => {
    if (body.id) return p.schedules.find((x) => x.id === String(body.id))?.script || "";
    if (body.replace) {
      const h = holder(p, String(body.replace));
      return h?.file?.entry?.script || h?.entry?.script || "";
    }
    return "";
  };

  function list(query) {
    const slug = query.get("project") || "";
    const projects = slug
      ? data.projects.filter((p) => p.slug === slug)
      : data.projects;
    for (const p of projects) for (const s of p.schedules) flagScript(s);
    return ok(200, {
      projects,
      limits: data.limits,
      kinds: data.kinds,
      script_kinds: data.script_kinds,
      script_job_kinds: data.script_job_kinds,
    });
  }

  // The draft, trimmed and defaulted the way the hub's Draft::from_payload does.
  function draft(body) {
    const e = body.entry && typeof body.entry === "object" ? body.entry : {};
    const kind = String(e.kind || "task").trim();
    const script = SCRIPT_KINDS.includes(kind);
    return {
      name: String(e.name || "").trim(),
      title: String(e.title || "").trim(),
      every: String(e.every || "").trim(),
      at: String(e.at || "").trim(),
      kind,
      brief: script
        ? ""
        : String(e.brief || "")
            .replace(/\r\n/g, "\n")
            .trimEnd(),
      skill: script ? "" : String(e.skill || "").trim(),
      script: script ? String(e.script || "").trim() : "",
      job_kind: script ? String(e.job_kind || "task").trim() : "",
      tier: Number(e.tier ?? 1),
      priority: Number(e.priority ?? 0),
      run_as: String(e.run_as || "").trim(),
      times: Number(e.times ?? 0),
      paused: !!e.paused,
    };
  }

  // What is wrong with an entry's own fields, the way the hub's static and
  // store checks word it. `kept` is the script the automation already
  // runs: it may stay where it is, even outside .pulse/.
  function fieldProblems(p, e, kept = "") {
    const problems = [];
    if (!e.name) problems.push("name: a lowercase name is required");
    else if (!SLUG.test(e.name))
      problems.push(
        "name: " +
          JSON.stringify(e.name) +
          " is not lowercase letters, digits and dashes",
      );
    const minutes = everyMinutes(e.every);
    if (!e.every)
      problems.push(
        "needs `every`, the cadence (30m, 6h, 1d, 2w, hourly, daily, weekly)",
      );
    else if (minutes === null)
      problems.push(
        "every: " +
          JSON.stringify(e.every) +
          " does not read as 30m, 6h, 1d, 2w, hourly, daily or weekly",
      );
    else if (minutes < data.limits.min_minutes)
      problems.push(
        "every: " +
          e.every +
          " is under the floor of " +
          data.limits.min_minutes +
          " minutes (schedule.min_minutes)",
      );
    if (e.at && !/^([01]\d|2[0-3]):[0-5]\d$/.test(e.at))
      problems.push("at: " + JSON.stringify(e.at) + " is not HH:MM");
    if (![...data.kinds, ...data.script_kinds].includes(e.kind))
      problems.push(
        "kind: " +
          JSON.stringify(e.kind) +
          " is not one of " +
          [...data.kinds, ...data.script_kinds].join(", "),
      );
    if (SCRIPT_KINDS.includes(e.kind)) {
      if (!e.script)
        problems.push(
          "a " +
            e.kind +
            " needs `script`, the path of the script in the repository",
        );
      else if (e.script.startsWith("/") || e.script.split("/").includes(".."))
        problems.push(
          "script: " + e.script + " must be a path inside the repository",
        );
      else if (!scriptInPulse(e.script) && e.script !== kept)
        problems.push(
          "The script " +
            JSON.stringify(e.script) +
            " is outside .pulse/",
        );
      else if (!/^\.pulse\/automations\//.test(e.script) && e.script !== kept)
        problems.push(
          "script: " +
            e.script +
            " is not on " +
            (p.repo?.branch || "the default branch"),
        );
      if (!data.script_job_kinds.includes(e.job_kind))
        problems.push(
          "job_kind: " +
            JSON.stringify(e.job_kind) +
            " is not one of " +
            data.script_job_kinds.join(", "),
        );
    } else if (!e.brief && !e.skill)
      problems.push(
        "needs a `brief` (what each job does) or a `skill` to follow",
      );
    if (e.skill && !p.options.skills.includes(e.skill))
      problems.push("skill: " + e.skill + " is not on for " + p.slug);
    if (e.run_as && !p.options.agents.includes(e.run_as))
      problems.push(
        "run_as: " + e.run_as + " is not a custom agent of " + p.slug,
      );
    if (e.tier !== 0 && e.tier !== 1)
      problems.push("tier: " + e.tier + " is not 0 or 1");
    if (e.times < 0) problems.push("times: must be 0 or more");
    return problems;
  }

  function check(body) {
    const slug = String(body.project || "").trim();
    const p = projectOf(slug);
    if (!p)
      return failure(
        404,
        "No such project",
        "no project named " + slug + ".",
        "Pick a project from the list.",
      );
    const where = body.where === "store" ? "store" : "repo";
    const e = draft(body);
    const problems = fieldProblems(p, e, keptScript(p, body));
    let path = pathOf(e.name);
    let form = "file";
    let branch = "";
    const warning = "";
    if (where === "repo") {
      if (!p.repo)
        problems.push(
          slug +
            " has no checkout on this machine, so its files cannot be written here; keep the automation in the store instead",
        );
      else {
        branch = p.repo.branch;
        if (p.file && !p.file.entries) {
          problems.push(...p.file.problems);
          problems.push(
            "fix " +
              p.file.path +
              " by hand before the console writes to the repository",
          );
        }
        const taken = () => {
          const h = holder(p, e.name);
          if (h)
            problems.push(
              "`" +
                e.name +
                "` is already an automation (" +
                h.path +
                "); pick another name",
            );
        };
        const replace = body.replace ? String(body.replace) : "";
        if (replace) {
          const old = holder(p, replace);
          if (!old)
            problems.push(
              slug +
                " on " +
                branch +
                " has no automation named `" +
                replace +
                "` to replace",
            );
          else if (old.form === "list") {
            form = "list";
            path = old.path;
            if (replace !== e.name) taken();
          } else if (replace === e.name) path = old.path;
          else taken();
        } else {
          const h = holder(p, e.name);
          if (h)
            problems.push(
              h.path +
                " already holds an automation named `" +
                e.name +
                "`; edit that one or pick another name",
            );
        }
      }
    } else {
      const id = body.id ? String(body.id) : "";
      if (id) {
        const s = p.schedules.find((x) => x.id === id);
        if (!s) problems.push("no schedule " + id + " in " + slug);
        else if (s.origin)
          problems.push(
            id +
              " is kept by " +
              s.origin.slice(5).split("#")[0] +
              "; edit it there",
          );
        else if (s.status === "ended")
          problems.push(id + " has ended; add a new automation instead");
      } else if (
        p.schedules.filter((s) => s.status !== "ended").length >=
        data.limits.max_per_project
      )
        problems.push(
          slug +
            " already has " +
            data.limits.max_per_project +
            " live schedules (schedule.max_per_project)",
        );
    }
    return ok(200, {
      yaml: form === "list" ? yamlItem(e) : yamlFile(e),
      problems,
      warning,
      path,
      branch,
      form,
      _draft: e,
      _project: p,
      _where: where,
    });
  }

  function row(p, e, over = {}) {
    const minutes = everyMinutes(e.every) || 1440;
    const t = now();
    return {
      id: nextId(),
      project: p.slug,
      title: e.title || e.name,
      kind: e.kind,
      brief: e.brief,
      tier: e.tier,
      priority: e.priority,
      every_minutes: minutes,
      at_time: minutes % 1440 === 0 ? e.at : "",
      status: e.paused ? "paused" : "active",
      next_at: t,
      last_at: "",
      last_job: "",
      runs: 0,
      max_runs: e.times,
      note: e.paused ? "paused" : "",
      created_by: "human",
      created_at: t,
      updated_at: t,
      script: e.script,
      job_kind: e.job_kind,
      state: "",
      state_since: "",
      last_rc: null,
      last_output: "",
      fails: 0,
      agent: e.run_as,
      skill: e.skill,
      origin: "",
      every: e.every,
      every_words: everyWords(minutes, e.at),
      origin_name: "",
      origin_path: "",
      ...over,
    };
  }

  // The repository's entry, from the draft, defaults dropped the way a hand
  // would leave them out.
  function fileEntry(e) {
    const out = { name: e.name, title: e.title, every: e.every };
    if (e.at) out.at = e.at;
    if (e.kind !== "task") out.kind = e.kind;
    if (e.brief) out.brief = e.brief;
    if (e.skill) out.skill = e.skill;
    if (e.script) out.script = e.script;
    if (SCRIPT_KINDS.includes(e.kind)) out.job_kind = e.job_kind;
    if (e.tier !== 1) out.tier = e.tier;
    if (e.priority) out.priority = e.priority;
    if (e.run_as) out.run_as = e.run_as;
    if (e.times) out.times = e.times;
    if (e.paused) out.paused = true;
    return out;
  }

  const withDefaults = (e) => ({
    kind: "task",
    tier: 1,
    priority: 0,
    times: 0,
    ...e,
  });

  function rewriteList(p) {
    const head = p.file.text.split("\nschedules:")[0];
    const items = p.file.entries.map((e) => yamlItem(withDefaults(e))).join("");
    p.file.text =
      (head.startsWith("schedules:") ? "" : head + "\n") +
      (p.file.entries.length ? "schedules:\n" + items : "schedules: []\n");
  }

  // A file of the folder from an entry, as the hub writes it.
  function makeFile(e) {
    return {
      path: pathOf(e.name),
      name: e.name,
      text: yamlFile(withDefaults(e)),
      entry: e,
      problems: [],
    };
  }

  function ensureFolder(p) {
    if (!p.automations) p.automations = { dir: DIR, files: [] };
    return p.automations;
  }

  // Apply the repository to the rows the way the daemon's sync does: a live
  // row per automation that passes, ended rows for ones that left, held
  // ones reported and left as they were.
  function sync(p, note) {
    const t = now();
    const sources = [];
    for (const e of p.file?.entries || [])
      sources.push({ entry: e, path: p.file.path });
    for (const f of filesOf(p)) {
      if (!f.entry) continue;
      if (listEntryOf(p, f.name)) continue; // the daemon holds a name in both
      sources.push({ entry: f.entry, path: f.path });
    }
    const problems = [];
    const held = new Set();
    for (const f of filesOf(p)) {
      if (!f.entry) {
        problems.push(...f.problems);
        held.add(f.name);
        continue;
      }
      if (listEntryOf(p, f.name)) {
        problems.push(
          "schedule `" +
            f.name +
            "`: named in both " +
            p.file.path +
            " and " +
            f.path +
            "; keep one of them",
        );
        held.add(f.name);
        continue;
      }
      const d = draft({ entry: f.entry });
      for (const x of fieldProblems(p, d)) {
        problems.push("schedule `" + f.name + "`: " + x);
        held.add(f.name);
      }
    }
    const applied = [];
    const present = new Set(sources.map((s) => s.entry.name));
    for (const s of p.schedules) {
      if (
        s.origin_path &&
        s.status !== "ended" &&
        !present.has(s.origin_name) &&
        !held.has(s.origin_name)
      ) {
        Object.assign(s, {
          status: "ended",
          note: note || "removed from the project's PULSE file",
          next_at: "",
          updated_at: t,
        });
      }
    }
    for (const { entry: e, path } of sources) {
      if (held.has(e.name)) continue;
      const d = draft({ entry: e });
      const minutes = everyMinutes(d.every) || 1440;
      const live = p.schedules.find(
        (s) => s.origin_name === e.name && s.status !== "ended",
      );
      const fields = {
        title: d.title || d.name,
        kind: d.kind,
        brief: d.brief,
        tier: d.tier,
        priority: d.priority,
        every_minutes: minutes,
        at_time: minutes % 1440 === 0 ? d.at : "",
        max_runs: d.times,
        script: d.script,
        job_kind: d.job_kind,
        agent: d.run_as,
        skill: d.skill,
        every: d.every,
        every_words: everyWords(minutes, d.at),
        origin: "repo:" + path + "#" + e.name,
        origin_path: path,
        updated_at: t,
      };
      if (live) {
        Object.assign(live, fields);
        if (d.paused && live.status === "active")
          Object.assign(live, { status: "paused", note: "paused by " + path });
        if (
          !d.paused &&
          live.status === "paused" &&
          live.note.startsWith("paused by ")
        )
          Object.assign(live, { status: "active", note: "" });
      } else {
        p.schedules.push(
          row(p, d, { ...fields, created_by: "daemon", origin_name: e.name }),
        );
      }
      applied.push(e.name);
    }
    p.applied = {
      path: p.file?.path || "",
      files: filesOf(p).map((f) => f.path),
      commit: p.repo.commit,
      at: t,
      problems,
      schedules: applied,
    };
    const parts = [];
    if (p.file) parts.push(p.file.path);
    if (filesOf(p).length) parts.push(DIR + "/");
    return {
      line:
        parts.join(" and ") +
        ": " +
        applied.length +
        " applied" +
        (held.size ? ", " + held.size + " held" : ""),
      problems,
    };
  }

  function write(body) {
    const c = check(body);
    if (c.status !== 200) return c;
    const { _draft: e, _project: p, _where: where, ...rest } = c.body;
    if (rest.problems.length)
      return failure(
        400,
        "The automation has problems",
        rest.problems.join("; "),
        "Fix what the problems name and try again.",
        rest,
      );
    const t = now();
    if (where === "store") {
      const id = body.id ? String(body.id) : "";
      if (id) {
        const s = p.schedules.find((x) => x.id === id);
        const minutes = everyMinutes(e.every);
        Object.assign(s, {
          title: e.title || e.name,
          kind: e.kind,
          brief: e.brief,
          tier: e.tier,
          priority: e.priority,
          every_minutes: minutes,
          at_time: minutes % 1440 === 0 ? e.at : "",
          max_runs: e.times,
          script: e.script,
          job_kind: e.job_kind,
          agent: e.run_as,
          skill: e.skill,
          every: e.every,
          every_words: everyWords(minutes, e.at),
          updated_at: t,
        });
        if (e.paused && s.status === "active")
          Object.assign(s, { status: "paused", note: "paused" });
        if (!e.paused && s.status === "paused")
          Object.assign(s, { status: "active", note: "", next_at: t });
        record(s.id, "updated", { changed: ["title", "every"] });
        emit("human:console", "schedule.update", s.id, { project: p.slug });
        return ok(200, {
          where,
          schedule: flagScript(s),
          yaml: rest.yaml,
          warning: rest.warning,
        });
      }
      const s = row(p, e);
      p.schedules.push(s);
      record(s.id, "added");
      emit("human:console", "schedule.add", s.id, { project: p.slug });
      return ok(201, {
        where,
        schedule: flagScript(s),
        yaml: rest.yaml,
        warning: rest.warning,
      });
    }
    const replace = body.replace ? String(body.replace) : "";
    const entry = fileEntry(e);
    const folder = ensureFolder(p);
    const before = JSON.stringify([p.file?.entries, folder.files]);
    if (rest.form === "list") {
      // A list item is edited in place, under its new name if it changed.
      p.file.entries = p.file.entries.map((x) =>
        x.name === replace ? entry : x,
      );
      if (JSON.stringify(p.file.entries) !== before) rewriteList(p);
    } else {
      if (replace && replace !== e.name)
        folder.files = folder.files.filter((f) => f.name !== replace);
      const made = makeFile(entry);
      const i = folder.files.findIndex((f) => f.name === e.name);
      if (i >= 0) folder.files[i] = made;
      else folder.files.push(made);
      folder.files.sort((a, b) => (a.name < b.name ? -1 : 1));
    }
    let commit = null;
    if (JSON.stringify([p.file?.entries, folder.files]) !== before) {
      commit = nextCommit();
      p.repo.commit = commit;
    }
    const synced = sync(p);
    if (body.move) {
      const moved = p.schedules.find((s) => s.id === String(body.move));
      if (moved && !moved.origin && moved.status !== "ended")
        Object.assign(moved, {
          status: "ended",
          note: "moved to " + rest.path + " as " + e.name,
          next_at: "",
          updated_at: t,
        });
    }
    const schedule =
      p.schedules.find(
        (s) => s.status !== "ended" && s.origin_name === e.name,
      ) || null;
    if (schedule) {
      const origin = "repo:" + rest.path + "#" + e.name;
      if (replace)
        record(schedule.id, "updated", { actor: "daemon", changed: ["title"], origin });
      else record(schedule.id, "added", { actor: "daemon", origin });
      flagScript(schedule);
    }
    emit(
      "human:console",
      replace ? "schedule.update" : "schedule.add",
      schedule?.id || e.name,
      { project: p.slug, path: rest.path },
    );
    return ok(replace ? 200 : 201, {
      where,
      path: rest.path,
      branch: p.repo.branch,
      commit,
      yaml: rest.yaml,
      warning: rest.warning,
      synced,
      schedule,
    });
  }

  function remove(body) {
    const slug = String(body.project || "").trim();
    const name = String(body.name || "").trim();
    const p = projectOf(slug);
    if (!p)
      return failure(
        404,
        "No such project",
        "no project named " + slug + ".",
        "Pick a project from the list.",
      );
    if (!p.repo)
      return failure(
        404,
        "No checkout",
        slug + " has no checkout on this machine",
        "pulse schedule list --project " + slug,
      );
    const h = holder(p, name);
    if (!h)
      return failure(
        400,
        "No automation " + name + " in " + slug,
        p.repo.branch +
          " holds " +
          (liveNames(p).join(", ") || "no automation"),
        "refresh the list",
      );
    if (h.form === "list") {
      p.file.entries = p.file.entries.filter((x) => x.name !== name);
      rewriteList(p);
    } else {
      p.automations.files = p.automations.files.filter((f) => f.name !== name);
    }
    const commit = nextCommit();
    p.repo.commit = commit;
    const synced = sync(p);
    emit("human:console", "schedule.end", name, {
      project: slug,
      path: h.path,
    });
    return ok(200, {
      where: "repo",
      path: h.path,
      branch: p.repo.branch,
      commit,
      removed: name,
      synced,
      schedule: null,
      yaml: "",
      warning: "",
    });
  }

  function action(id, verb, body) {
    const p = data.projects.find((x) => x.schedules.some((s) => s.id === id));
    const s = p?.schedules.find((x) => x.id === id);
    if (!s)
      return failure(
        404,
        "No schedule " + id,
        "nothing in the store has that id",
        "pulse schedule list",
      );
    const t = now();
    const note = String(body.note || "").trim();
    if (verb === "pause")
      Object.assign(s, { status: "paused", note, updated_at: t });
    else if (verb === "resume")
      Object.assign(s, {
        status: "active",
        note: "",
        next_at: t,
        updated_at: t,
      });
    else if (verb === "run") Object.assign(s, { next_at: t, updated_at: t });
    else if (verb === "end") {
      if (s.origin_path && p.repo && holder(p, s.origin_name)) {
        const r = remove({ project: p.slug, name: s.origin_name });
        if (note) s.note = note;
        record(s.id, "ended", { note: note || s.note });
        return r;
      }
      Object.assign(s, { status: "ended", note, next_at: "", updated_at: t });
    } else
      return failure(
        404,
        "No such action",
        verb + " is not pause, resume, end or run",
        "pulse schedule --help",
      );
    record(s.id, verb === "end" ? "ended" : verb === "pause" ? "paused" : verb === "resume" ? "resumed" : "run", { note });
    emit("human:console", "schedule." + verb, s.id, { project: s.project });
    return ok(200, { where: "store", schedule: flagScript(s), yaml: "", warning: "" });
  }

  // ----- history -----------------------------------------------------------

  // The event log, one entry per thing that happened to a schedule, newest
  // last; the fixture seeds it (`history` keyed by id) and the actions and
  // writes append to it the way the hub's events table grows.
  const log = () => (data.history ??= {});
  let seq = Object.values(log())
    .flat()
    .reduce((m, e) => Math.max(m, e.seq || 0), 0);
  const blank = () => ({
    seq: 0,
    at: "",
    actor: "",
    what: "",
    run: 0,
    rc: null,
    timeout: false,
    seconds: 0,
    state: "",
    previous: "",
    job: "",
    job_status: "",
    job_agent: "",
    job_finished_at: "",
    queued: false,
    why: "",
    note: "",
    changed: [],
    origin: "",
  });
  function record(id, what, over = {}) {
    const entries = (log()[id] ??= []);
    entries.push({
      ...blank(),
      seq: ++seq,
      at: now(),
      actor: "human",
      what,
      ...over,
    });
  }

  // GET automations/<id>/history?before=SEQ&limit=N: newest first, with the
  // jobs it queued and the last output, the shape the hub answers.
  function history(id, query) {
    const p = data.projects.find((x) => x.schedules.some((s) => s.id === id));
    const s = p?.schedules.find((x) => x.id === id);
    if (!s)
      return failure(
        404,
        "No schedule " + id,
        "The id is not in this store.",
        "Refresh the list.",
      );
    const before = Number(query?.get?.("before") || 0) || 0;
    const limit = Math.min(500, Math.max(1, Number(query?.get?.("limit") || 50) || 50));
    const all = [...(log()[id] || [])]
      .map((e) => ({ ...blank(), ...e }))
      .filter((e) => !before || e.seq < before)
      .sort((a, b) => b.seq - a.seq);
    const entries = all.slice(0, limit);
    const jobs = (data.jobs || []).filter((j) => j.source === "schedule:" + id);
    return ok(200, {
      schedule: flagScript(s),
      entries,
      more: all.length > limit,
      jobs,
      last_output: s.last_output || "",
    });
  }

  return {
    list,
    check: (body) => strip(check(body)),
    write,
    remove,
    action,
    history,
  };
}

// The check's private fields never leave the mock.
function strip(r) {
  if (r.status !== 200) return r;
  const { _draft, _project, _where, ...body } = r.body;
  void _draft;
  void _project;
  void _where;
  return { status: r.status, body };
}
