// Automations mock for `automations` (crates/pulse-hub/src/automations.rs):
// the standing jobs of each project, kept in `.pulse/pulse.yaml` or in the
// store, with the checks the hub runs on a draft and the writes the console
// makes. Pure functions over in-memory state, so the shape is testable
// without a server: createAutomationsMock({...}) mirrors
// mock/api/automations.json plus the deps server.mjs injects.
//
// The list body is { projects, limits, kinds, script_kinds, script_job_kinds };
// check answers { yaml, problems, warning, path, branch }; a write answers
// { where, schedule, yaml, warning, path?, branch?, commit?, synced? } (201 for
// a new one, 200 for an edit, 400 with `problems` when a check fails); remove
// and the actions answer the same shape plus `removed` where an entry left
// the file. Those keys are the contract the owner console reads - freeze them
// here.
const pad = (n, w) => String(n).padStart(w, "0");
const SLUG = /^[a-z0-9][a-z0-9-]*$/;
const SCRIPT_KINDS = ["check", "watch"];

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

// One list item of `schedules:`, in the hub's field order, defaults left out.
export function yamlItem(e, indent = "  ") {
  const lines = ["- name: " + scalar(e.name)];
  if (e.title) lines.push("  title: " + scalar(e.title));
  lines.push("  every: " + scalar(e.every));
  if (e.at) lines.push("  at: " + JSON.stringify(e.at));
  if (e.kind && e.kind !== "task") lines.push("  kind: " + e.kind);
  if (e.brief) {
    if (e.brief.includes("\n"))
      lines.push(
        "  brief: |-",
        ...e.brief.split("\n").map((l) => (l ? "    " + l : "")),
      );
    else lines.push("  brief: " + scalar(e.brief));
  }
  if (e.skill) lines.push("  skill: " + scalar(e.skill));
  if (e.script) lines.push("  script: " + scalar(e.script));
  if (SCRIPT_KINDS.includes(e.kind) && e.job_kind && e.job_kind !== "task")
    lines.push("  job_kind: " + e.job_kind);
  if (e.tier !== undefined && e.tier !== 1) lines.push("  tier: " + e.tier);
  if (e.priority) lines.push("  priority: " + e.priority);
  if (e.run_as) lines.push("  run_as: " + scalar(e.run_as));
  if (e.times) lines.push("  times: " + e.times);
  if (e.paused) lines.push("  paused: true");
  return lines.map((l) => (l ? indent + l : l)).join("\n") + "\n";
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
  const liveNames = (p) => (p.file?.entries || []).map((e) => e.name);

  function list(query) {
    const slug = query.get("project") || "";
    const projects = slug
      ? data.projects.filter((p) => p.slug === slug)
      : data.projects;
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
      problems.push("every: how often it runs is required (30m, 6h, 1d, 2w)");
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
        problems.push("script: a " + e.kind + " needs the script to run");
      else if (e.script.startsWith("/") || e.script.split("/").includes(".."))
        problems.push(
          "script: " + e.script + " must be a path inside the repository",
        );
      else if (!/^scripts\//.test(e.script))
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
      problems.push("brief: say what each job does, or name a skill");
    if (e.skill && !p.options.skills.includes(e.skill))
      problems.push("skill: " + e.skill + " is not on for " + slug);
    if (e.run_as && !p.options.agents.includes(e.run_as))
      problems.push(
        "run_as: " + e.run_as + " is not a custom agent of " + slug,
      );
    if (e.tier !== 0 && e.tier !== 1)
      problems.push("tier: " + e.tier + " is not 0 or 1");
    if (e.times < 0) problems.push("times: must be 0 or more");
    let path = "";
    let branch = "";
    let warning = "";
    if (where === "repo") {
      if (!p.repo)
        problems.push(
          slug +
            " has no checkout on this machine, so there is no file to write",
        );
      else {
        path = p.file?.path || ".pulse/pulse.yaml";
        branch = p.repo.branch;
        const names = liveNames(p);
        const replace = body.replace ? String(body.replace) : "";
        if (replace && !names.includes(replace))
          problems.push(path + " has no entry named " + replace);
        else if (replace && replace !== e.name && names.includes(e.name))
          problems.push(path + " already has an entry named " + e.name);
        else if (!replace && names.includes(e.name))
          problems.push(
            path +
              " already has an entry named " +
              e.name +
              "; edit that one or pick another name",
          );
        if (p.file?.problems?.length) problems.push(...p.file.problems);
        if (!p.file)
          warning =
            "There is no " +
            path +
            " on " +
            branch +
            " yet; it is created with this entry.";
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
      yaml: yamlItem(e),
      problems,
      warning,
      path,
      branch,
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

  // The file's entry, from the draft, defaults dropped the way the file keeps them.
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

  function rewriteFile(p) {
    const head = p.file.text.split("\nschedules:")[0];
    const items = p.file.entries
      .map((e) =>
        yamlItem({ kind: "task", tier: 1, priority: 0, times: 0, ...e }),
      )
      .join("");
    p.file.text =
      (head.startsWith("schedules:") ? "" : head + "\n") +
      (p.file.entries.length ? "schedules:\n" + items : "schedules: []\n");
  }

  // Apply the file to the rows the way the daemon's sync does: a live row per
  // entry, ended rows for entries that left.
  function sync(p, note) {
    const t = now();
    const path = p.file.path;
    const names = new Set(liveNames(p));
    for (const s of p.schedules) {
      if (
        s.origin_path === path &&
        s.status !== "ended" &&
        !names.has(s.origin_name)
      ) {
        Object.assign(s, {
          status: "ended",
          note: note || "removed from " + path,
          next_at: "",
          updated_at: t,
        });
      }
    }
    for (const e of p.file.entries) {
      const d = draft({ entry: e });
      const minutes = everyMinutes(d.every) || 1440;
      const live = p.schedules.find(
        (s) =>
          s.origin_name === e.name &&
          s.origin_path === path &&
          s.status !== "ended",
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
        updated_at: t,
      };
      if (live) {
        Object.assign(live, fields);
        if (d.paused && live.status === "active")
          Object.assign(live, { status: "paused", note: "paused by " + path });
        if (
          !d.paused &&
          live.status === "paused" &&
          live.note === "paused by " + path
        )
          Object.assign(live, { status: "active", note: "" });
      } else {
        p.schedules.push(
          row(p, d, {
            ...fields,
            created_by: "daemon",
            origin: "repo:" + path + "#" + e.name,
            origin_name: e.name,
            origin_path: path,
          }),
        );
      }
    }
    p.applied = {
      path,
      commit: p.repo.commit,
      at: t,
      problems: [],
      schedules: p.file.entries.map((e) => e.name),
    };
    return {
      line: path + ": " + p.file.entries.length + " entries applied",
      problems: [],
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
        emit("human:console", "schedule.update", s.id, { project: p.slug });
        return ok(200, {
          where,
          schedule: s,
          yaml: rest.yaml,
          warning: rest.warning,
        });
      }
      const s = row(p, e);
      p.schedules.push(s);
      emit("human:console", "schedule.add", s.id, { project: p.slug });
      return ok(201, {
        where,
        schedule: s,
        yaml: rest.yaml,
        warning: rest.warning,
      });
    }
    const replace = body.replace ? String(body.replace) : "";
    if (!p.file)
      p.file = {
        path: ".pulse/pulse.yaml",
        text: "schedules: []\n",
        entries: [],
        needs: [],
        problems: [],
      };
    const entry = fileEntry(e);
    const before = JSON.stringify(p.file.entries);
    if (replace)
      p.file.entries = p.file.entries.map((x) =>
        x.name === replace ? entry : x,
      );
    else p.file.entries.push(entry);
    let commit = null;
    if (JSON.stringify(p.file.entries) !== before) {
      rewriteFile(p);
      commit = nextCommit();
      p.repo.commit = commit;
    }
    const synced = sync(p);
    if (body.move) {
      const moved = p.schedules.find((s) => s.id === String(body.move));
      if (moved && !moved.origin && moved.status !== "ended")
        Object.assign(moved, {
          status: "ended",
          note: "moved to " + p.file.path + " as " + e.name,
          next_at: "",
          updated_at: t,
        });
    }
    const schedule =
      p.schedules.find(
        (s) =>
          s.status !== "ended" &&
          s.origin_name === e.name &&
          s.origin_path === p.file.path,
      ) || null;
    emit(
      "human:console",
      replace ? "schedule.update" : "schedule.add",
      schedule?.id || e.name,
      { project: p.slug, path: p.file.path },
    );
    return ok(replace ? 200 : 201, {
      where,
      path: p.file.path,
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
    if (!p.repo || !p.file)
      return failure(
        404,
        "No file",
        slug +
          " has no " +
          (p.file?.path || ".pulse/pulse.yaml") +
          " on this machine",
        "pulse schedule list --project " + slug,
      );
    if (!liveNames(p).includes(name))
      return failure(
        404,
        "No such entry",
        p.file.path + " has no entry named " + name,
        "pulse manifest check --project " + slug,
      );
    p.file.entries = p.file.entries.filter((x) => x.name !== name);
    rewriteFile(p);
    const commit = nextCommit();
    p.repo.commit = commit;
    const synced = sync(p);
    emit("human:console", "schedule.end", name, {
      project: slug,
      path: p.file.path,
    });
    return ok(200, {
      where: "repo",
      path: p.file.path,
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
      if (s.origin_path && p.file && liveNames(p).includes(s.origin_name)) {
        const r = remove({ project: p.slug, name: s.origin_name });
        if (note) s.note = note;
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
    emit("human:console", "schedule." + verb, s.id, { project: s.project });
    return ok(200, { where: "store", schedule: s, yaml: "", warning: "" });
  }

  return { list, check: (body) => strip(check(body)), write, remove, action };
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
