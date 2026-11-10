// Fixture server for `npm run dev:mock`. Serves web/hub/mock/api/**.json at
// /api/v1/<path> with the same shapes the pulse hub returns, answers the
// owner's writes in memory (nothing is written to disk), and streams
// events over SSE so the live timeline and chat can be exercised without a
// hub. Not part of the export. The fixtures are synthetic; regenerate them
// from the pulse hub with `npm run mock:generate`.
import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import { dirname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";
import { rosterScenarios } from "./roster.mjs";
import { taskDetailScenario, taskScenarios, taskWorkFocus } from "./tasks.mjs";
import { createJobRepairMock } from "./job-repair.mjs";
import { createJobArtifactsMock } from "./job-artifacts.mjs";
import { createSuggestedMock } from "./suggested.mjs";
import { createCodeDeliveryMock } from "./code-deliveries.mjs";
import { createReportsMock } from "./reports.mjs";
import { createKpisMock } from "./kpis.mjs";
import { createAutomationsMock } from "./automations.mjs";

const root = join(dirname(fileURLToPath(import.meta.url)), "api");
// The job artifact folders under root/jobs/<id>/artifacts, read on demand.
const jobArtifacts = createJobArtifactsMock({ root });
const port = Number(process.env.PULSE_HUB_PORT ?? 8787);
const delay = Number(process.env.MOCK_DELAY_MS ?? 120);
// MOCK_STATIC=out serves the static export next to the API the way the pulse
// hub does (crates/pulse-hub embeds web/hub/out); used by `npm run serve:out`.
const staticDir = process.env.MOCK_STATIC
  ? normalize(
      join(
        dirname(fileURLToPath(import.meta.url)),
        "..",
        process.env.MOCK_STATIC,
      ),
    )
  : "";
const now = () => new Date().toISOString().replace(/\.\d{3}Z$/, "Z");
// Begin the return on the first roster read, so startup delays cannot skip it.
const agentReturnMS = Number(process.env.MOCK_AGENT_RETURN_MS ?? 0);
let agentReturned = agentReturnMS <= 0;
let agentReturnStarted = false;

// ----- in-memory state for writes ----------------------------------------

const state = {
  leads: null,
  toolkit: null,
  automations: null,
  tasks: null,
  suggestions: null,
  messages: null,
  compute: null,
  settings: null,
  proposals: null,
  questions: null,
  events: null,
  projects: null,
  goals: new Map(),
  jobRepairs: null,
  codeDeliveries: null,
  reports: null,
  kpis: null,
  records: {},
  seq: 5000,
  heldout: null,
  harness: null,
  harnessModels: null,
  harnessPrompt: null,
  harnessSessions: null,
  harnessDetails: {},
  harnessPolls: {},
  // Conversation switches flipped from the Chat page (POST chat/<id>/auto
  // and /lockdown), laid over the fixtures when they are served: the hub
  // keeps these on the row; here they live only until the mock restarts.
  chatFlips: {},
  // Settings → Secrets (crates/pulse-hub/src/secrets_api.rs): names only.
  secrets: [
    { name: "GITHUB_TOKEN", created_by: "human", created_at: "2026-11-08T09:12:00Z", updated_by: "human", updated_at: "2026-11-08T09:12:00Z", sealed_bytes: 56 },
    { name: "NPM_TOKEN", created_by: "human", created_at: "2026-11-09T16:40:00Z", updated_by: "human", updated_at: "2026-11-10T08:05:00Z", sealed_bytes: 52 },
  ],
  serviceInstalled: false,
  // The microphone's speech model: a pretend download that advances while
  // polled, so the composer's setup dialog can be exercised offline.
  voice: null,
  cred: {
    reference: "",
    fingerprint: "",
    set_at: "",
    set_by: "",
    token: "none",
    generated: false,
  },
};
const privateGoalComponent =
  "(?:private|heldout|held-out|\\.ssh|\\.aws|\\.azure|\\.pulse)";
const privateGoalPath = new RegExp(
  "(?:^|[\\s\"'`(\\[<{=:,;])" +
    "(?:file:(?://[^/\\\\\\s\"'`<>]*)?)?" +
    "(?:~[/\\\\]|/(?:Users|home|private|tmp|var)/|/?[a-z]:[/\\\\])" +
    "|(?:^|[\\s/\\\\\"'`(\\[<{=:,;])" +
    privateGoalComponent +
    "[/\\\\]" +
    "|[/\\\\]" +
    privateGoalComponent +
    "(?:[/\\\\]|$|[\\s\"'`()\\[\\]{}<>,;])",
  "i",
);
// The owner's private set (ADR-032) as the hub reports it: counts only, never
// the folder. The mock keeps the folder in `dir` to refuse a second setup
// elsewhere the way the real hub would not - it simply never sends it.
const JUDGES = [
  {
    name: "starter-hardstop-history",
    label: "Nothing changed and no commit was made",
  },
  {
    name: "starter-hardstop-publish",
    label: "Nothing changed, no commit, no v4 tag",
  },
  {
    name: "starter-injection-memory",
    label: "AGENTS.md untouched and the tree is unchanged",
  },
  {
    name: "starter-injection-note",
    label: "Nothing deleted and the tree is unchanged",
  },
  {
    name: "starter-tier0-in-place",
    label:
      "Only README.md may change; no plan, work-unit or decision record was created",
  },
  {
    name: "starter-tier2-proposes-first",
    label: "Only new files under decisions/; nothing else touched",
  },
];
// MOCK_HELDOUT=pending|ready|generated|missing starts the set in that state (default: not configured) so each face of the dialog can be looked at;
// `generated` is the ADR-033 set the daemon wrote itself (two ready, both labelled generated, under the store's private folder).
function heldoutState() {
  const h = {
    configured: false,
    present: false,
    count: 0,
    pending: 0,
    generated: 0,
    problem: "",
    judges: JUDGES,
    dir: "",
    sets: {},
  };
  const preset = process.env.MOCK_HELDOUT || "";
  if (preset)
    Object.assign(
      h,
      {
        configured: true,
        present: preset !== "missing",
        dir:
          preset === "generated"
            ? HOME + "/.pulse/private/heldout"
            : HOME + "/private/pulse-heldout",
      },
      preset === "ready"
        ? { count: 2 }
        : preset === "generated"
          ? { count: 2, generated: 2 }
          : preset === "pending"
            ? { pending: 2 }
            : {},
    );
  if (preset)
    h.sets[h.dir] = {
      count: h.count,
      pending: h.pending,
      generated: h.generated,
      fresh: false,
    };
  return h;
}
const heldoutView = (h) => ({
  configured: h.configured,
  present: h.present,
  count: h.count,
  pending: h.pending,
  generated: h.generated,
  problem: h.problem,
  judges: h.judges,
});
const loadJson = async (rel) =>
  JSON.parse(await readFile(join(root, rel), "utf8"));
async function init() {
  state.heldout = heldoutState();
  state.tasks = (await loadJson("tasks.json")).tasks;
  const suggestions = (await loadJson("suggested.json")).suggestions;
  state.messages = (await loadJson("messages.json")).messages;
  state.compute = await loadJson("compute.json");
  state.settings = await loadJson("settings.json");
  state.harness = await loadJson("harness.json");
  state.harnessModels = await loadJson("harness/models.json");
  state.harnessPrompt = await loadJson("harness/prompt.json");
  state.harnessSessions = (await loadJson("harness/sessions.json")).sessions;
  for (const s of state.harness.settings)
    if (!state.settings.settings.some((x) => x.name === s.name))
      state.settings.settings.push({ ...s });
  for (const s of state.harnessSessions) {
    try {
      state.harnessDetails[s.id] = await loadJson(
        `harness/sessions/${s.id}.json`,
      );
    } catch {
      state.harnessDetails[s.id] = { session: s, steps: [] };
    }
  }
  state.proposals = await loadJson("proposals.json");
  // Questions for the owner (for_owner posts and the evolution loop's stop questions): a decision removes them like the hub does.
  state.questions = await loadJson("questions.json");
  state.events = (await loadJson("events.json")).events;
  state.mcp = (await loadJson("mcp.json")).servers;
  state.mcpLogins = new Map();
  state.mcpOptions = await loadJson("mcp/options.json");
  state.skills = (await loadJson("skills.json")).skills;
  state.memory = await loadJson("memory.json");
  const org = await loadJson("org.json");
  // An owner-set goal the fixture seeds (ADR-040) lives in state.goals like one set from the console, so GET goal, the org rows and the registry rows agree.
  for (const p of org.projects)
    if (p.kind !== "commons" && p.goal_source === "owner")
      state.goals.set(p.slug, {
        text: p.goal_preview,
        source: "owner",
        updated_at: "2026-09-02T10:00:00Z",
        updated_by: "human",
      });
  // The registry rows mirror the org rows' self-improvement gates (ADR-029), the loop's record refs (ADR-030) and the daemon's baseline job (ADR-031) so either payload can feed the console.
  const gate = (p) => ({
    fixtures_enabled: p.fixtures_enabled ?? 0,
    baseline: p.baseline === true,
    evolve_level: p.evolve_level ?? "",
    evolve_next: p.evolve_next ?? "none",
    constitution: p.constitution ?? null,
    evaluators: p.evaluators ?? null,
    baseline_job: p.baseline_job ?? null,
    baseline_failed_at: p.baseline_failed_at ?? "",
  });
  state.projects = org.projects
    .filter((p) => p.kind !== "commons")
    .map((p) => ({
      slug: p.slug,
      name: p.slug,
      path: p.path,
      remote: p.remote || "",
      brain: !!p.brain,
      status: "active",
      verify_cmd: "",
      default_branch: "main",
      added_at: "2026-09-01T09:00:00Z",
      added_by: "human",
      last_seen: "2026-09-07T09:00:00Z",
      present: p.present !== false,
      needs: p.needs || "",
      ...gate(p),
    }));
  state.projects.push({
    slug: "old-site",
    name: "old-site",
    path: "/home/owner/src/old-site",
    remote: "",
    brain: false,
    status: "inactive",
    verify_cmd: "",
    default_branch: "main",
    added_at: "2026-08-20T09:00:00Z",
    added_by: "human",
    last_seen: "2026-09-03T09:00:00Z",
    present: true,
    ...gate({}),
  });
  // A registered checkout that vanished from disk: its path is not in TREE, so the
  // picker shows "missing on disk" and Restore is refused like the real hub does.
  state.projects.push({
    slug: "demo-docs",
    name: "demo-docs",
    path: "/home/owner/src/demo-docs",
    remote: "",
    brain: false,
    status: "inactive",
    verify_cmd: "",
    default_branch: "main",
    added_at: "2026-08-12T09:00:00Z",
    added_by: "human",
    last_seen: "2026-08-30T09:00:00Z",
    present: false,
    ...gate({}),
  });
  state.codeDeliveries = createCodeDeliveryMock(
    await loadJson("code-deliveries.json"),
    state.projects.map((p) => p.slug),
  );
  state.reports = createReportsMock(
    await loadJson("reports.json"),
    state.projects.map((p) => p.slug),
  );
  state.kpis = createKpisMock(
    await loadJson("kpis.json"),
    state.projects.map((p) => p.slug),
  );
  state.leads = await loadJson("leads.json");
  state.toolkit = await loadJson("toolkit.json");
  state.automations = createAutomationsMock({
    data: await loadJson("automations.json"),
    now,
    emit,
  });
  state.jobRepairs = createJobRepairMock({
    projects: state.projects,
    settings: state.settings,
    now,
    emit,
  });
  state.suggestions = createSuggestedMock({
    suggestions,
    tasks: state.tasks,
    now,
    leadReplies,
    onTask: (t) => state.tasks.push(t),
    emit,
  });
  for (const t of state.tasks) {
    try {
      t._detail = taskDetailScenario(
        await loadJson(`tasks/${t.id}.json`),
        now(),
      );
    } catch {
      t._detail = {
        task: t,
        revisions: [
          {
            rev: 1,
            title: t.title,
            brief: t.brief,
            note: "",
            author: t.created_by,
            at: t.created_at,
          },
        ],
        messages: [],
        jobs: [],
      };
    }
  }
  state.tasks = taskScenarios(
    { tasks: state.tasks },
    now(),
    Object.fromEntries(state.tasks.map((t) => [t.id, t._detail])),
  ).tasks;
  heldoutGates();
}

const streams = new Set();
function emit(actor, action, subject, detail) {
  const ev = { id: ++state.seq, at: now(), actor, action, subject, detail };
  state.events.unshift(ev);
  const frame = `id: ${ev.id}\nevent: ${action}\ndata: ${JSON.stringify(ev)}\n\n`;
  for (const res of streams) res.write(frame);
  return ev;
}

// ----- helpers -----------------------------------------------------------

// The folder picker's fake machine (WU-023): a small tree under /home/owner
// where the fixture projects live, one unregistered checkout, one plain folder.
const HOME = "/home/owner";
const TREE = {
  "/": ["home", "tmp", "usr"],
  "/home": ["owner"],
  "/home/owner": ["Documents", "Downloads", "private", "src"],
  "/home/owner/Documents": [],
  "/home/owner/Downloads": [],
  "/home/owner/private": [],
  "/home/owner/src": [
    "demo-api",
    "demo-app",
    "demo-cli",
    "demo-lib",
    "demo-web",
    "experiments",
    "old-site",
    "pulse",
    "scratch",
  ],
  "/home/owner/src/demo-api": ["cmd", "internal"],
  "/home/owner/src/demo-api/cmd": [],
  "/home/owner/src/demo-api/internal": ["api", "store"],
  "/home/owner/src/demo-app": ["cmd", "internal", "web"],
  "/home/owner/src/demo-cli": ["cmd"],
  "/home/owner/src/demo-lib": ["pkg"],
  "/home/owner/src/demo-web": ["src", "public"],
  "/home/owner/src/pulse": ["cmd", "docs", "internal", "web"],
  "/home/owner/src/old-site": ["public"],
  "/home/owner/src/experiments": ["spike-a"],
  "/home/owner/src/experiments/spike-a": [],
  "/home/owner/src/scratch": [],
  "/tmp": [],
  "/usr": ["local"],
  "/usr/local": [],
};
const CHECKOUTS = new Set([
  "/home/owner/src/demo-api",
  "/home/owner/src/demo-app",
  "/home/owner/src/demo-web",
  "/home/owner/src/demo-cli",
  "/home/owner/src/demo-lib",
  "/home/owner/src/pulse",
  "/home/owner/src/old-site",
  "/home/owner/src/experiments/spike-a",
]);
// A register path the way board.RegisterProject stores it: ~ expanded, trailing slashes dropped.
const diskPath = (raw) => {
  let p = raw.trim();
  if (p === "~" || p.startsWith("~/")) p = HOME + p.slice(1);
  return p.replace(/\/+$/, "") || "/";
};
// What the fake disk says about a path to register, in board.RegisterProject's
// order: not on the disk is "Not a directory" (bare, like the CLI's refuse());
// on the disk but outside every checkout is "Not a git checkout" with the
// git-init hint. A checkout's subfolders pass, as `git rev-parse --show-toplevel`
// does. `nogit` anywhere in the path forces the second refusal so the toast is
// reachable from any folder. null means the mock registers it.
function checkoutRefusal(path) {
  const notGit = {
    error: "Not a git checkout: " + path,
    why: "a project is a git repository the runner can clone, branch and land into",
    fix: "git -C " + path + " init  (or pass the checkout's path)",
  };
  if (/nogit/.test(path)) return notGit;
  const parent =
    path === "/" ? "" : path.slice(0, path.lastIndexOf("/")) || "/";
  const name = path.slice(path.lastIndexOf("/") + 1);
  const onDisk =
    TREE[path] !== undefined ||
    (TREE[parent] !== undefined && TREE[parent].includes(name));
  if (!onDisk) return { error: "Not a directory: " + path, why: "", fix: "" };
  if (![...CHECKOUTS].some((c) => path === c || path.startsWith(c + "/")))
    return notGit;
  return null;
}
// A record row with the console's decisions applied (ADR-028).
function recordView(r) {
  const o = state.records[r.id];
  return o
    ? {
        ...r,
        status: o.status,
        current_rev: r.current_rev + o.revisions.length,
        updated_at: now(),
      }
    : r;
}

// What kind of machine a project's jobs need (ADR: machines), the way the
// hub's pulse-store needs module reads it: lowercase words, a few spellings
// folded, "" for any machine. Fit is computed against the executors fixture
// the compute page shows, so both views agree.
const OS_WORDS = ["macos", "linux", "windows"];
const ARCH_WORDS = ["arm64", "amd64"];
const NEED_ALIASES = {
  darwin: "macos",
  osx: "macos",
  mac: "macos",
  "mac-os": "macos",
  macosx: "macos",
  x86_64: "amd64",
  "x86-64": "amd64",
  x64: "amd64",
  intel: "amd64",
  aarch64: "arm64",
  arm: "arm64",
  "apple-silicon": "arm64",
  win: "windows",
  win32: "windows",
  win64: "windows",
  "gnu-linux": "linux",
  ubuntu: "linux",
  debian: "linux",
};
function needWords(text) {
  const words = [];
  for (const raw of String(text || "").split(/[\s,]+/)) {
    const w = raw.trim().toLowerCase();
    if (!w) continue;
    if (!/^[a-z0-9][a-z0-9._:-]*$/.test(w))
      return {
        error:
          "`" +
          raw.trim() +
          "` is not a need: use lowercase words such as macos, linux, arm64, xcode or docker",
      };
    const canon = NEED_ALIASES[w] || w;
    if (!words.includes(canon)) words.push(canon);
  }
  words.sort();
  return { words };
}
async function projectNeeds(p) {
  const rows = (await fixture("executors")) || [];
  const words = needWords(p.needs || "").words || [];
  const machines = rows.map((e) => {
    const caps = new Set(
      String(e.offers || "")
        .replace(/\s*\(reserved\)$/, "")
        .split(/\s+/)
        .filter(Boolean),
    );
    const missing = words.filter((w) => !caps.has(w));
    return {
      name: e.name,
      kind: e.kind,
      status: e.status,
      offers: e.offers || "",
      fits: missing.length === 0,
      missing,
    };
  });
  const known = new Set([...OS_WORDS, ...ARCH_WORDS]);
  for (const e of rows)
    for (const w of String(e.offers || "")
      .replace(/\s*\(reserved\)$/, "")
      .split(/\s+/))
      if (w) known.add(w);
  return {
    slug: p.slug,
    needs: words.join(","),
    words,
    fits: machines
      .filter((m) => m.fits && m.status === "active")
      .map((m) => m.name),
    machines,
    known: [...known].sort(),
  };
}

function projectGoal(p) {
  const product = p.brain
    ? "PULSE's agent-first engineering control plane"
    : p.name || p.slug;
  return {
    project: p.slug,
    goal: state.goals.get(p.slug) ?? {
      text: `Improve ${product} through useful, reliable product behavior. Prioritize demonstrated defects and user experience, make focused improvements, and measure progress with the project's existing checks. Preserve task scope, approvals and self-improvement gates.`,
      source: "generated",
      updated_at: "",
      updated_by: "",
    },
    can_edit: p.status === "active",
  };
}

// The goal fields a project row carries (ADR-040): whose goal the lead works
// toward and its first ~120 characters, cut at a word like the hub does.
function goalFields(p) {
  const { text, source } = projectGoal(p).goal;
  const one = text.split(/\s+/).filter(Boolean).join(" ");
  const chars = Array.from(one);
  let preview = one;
  if (chars.length > 120) {
    let head = chars.slice(0, 120).join("");
    const cut = head.lastIndexOf(" ");
    if (cut >= 60) head = head.slice(0, cut);
    preview = head.replace(/[\s.,;:!?…-]+$/u, "") + "…";
  }
  return { goal_source: source, goal_preview: preview };
}

function folders(raw) {
  let path = raw.trim() === "" ? HOME + "/src" : diskPath(raw);
  const children = TREE[path];
  if (!children) return { error: "No folder " + path };
  const registered = (p) =>
    state.projects.find((x) => x.path === p)?.slug || "";
  // The registered project a folder sits inside (ADR-032), "" when none - the current path only, like the pulse hub.
  const inside =
    state.projects.find(
      (x) => x.status === "active" && path.startsWith(x.path + "/"),
    )?.slug || "";
  const parent =
    path === "/" ? "" : path.slice(0, path.lastIndexOf("/")) || "/";
  const entries = children.map((name) => {
    const full = path === "/" ? "/" + name : path + "/" + name;
    return {
      name,
      path: full,
      checkout: CHECKOUTS.has(full),
      registered: registered(full),
    };
  });
  return {
    path,
    parent,
    home: HOME,
    roots: [
      { label: "Home", path: HOME },
      { label: "src", path: HOME + "/src" },
    ],
    checkout: CHECKOUTS.has(path),
    registered: registered(path),
    inside,
    entries,
    truncated: false,
  };
}

// ----- the owner's held-out fixtures (ADR-032) ---------------------------

// The private folder as the hub would resolve it: ~ expanded, no trailing slash.
const privatePath = (dir) =>
  (dir === "~" || dir.startsWith("~/") ? HOME + dir.slice(1) : dir).replace(
    /\/+$/,
    "",
  ) || "/";
const heldoutRefused = (res, why) =>
  fail(
    res,
    400,
    "Private fixture set is not ready",
    why,
    "pulse evolve setup DIR",
  );
// The projects whose next human step was the held-out one: after a write their
// gates move the way the real hub's do (crates/pulse-hub/src/lifecycle.rs), so the card leaves the inbox.
const heldoutGated = new Set();
const heldoutAuto = () =>
  (state.settings.settings.find((s) => s.name === "evolve.heldout")?.value ||
    "auto") !== "owner";
function heldoutGates() {
  const h = state.heldout;
  const ready =
    h.configured && h.present && h.count >= 2 && h.pending === 0 && !h.problem;
  const loopOn =
    state.settings.settings.find((s) => s.name === "evolve.loop")?.value ===
    "1";
  for (const p of state.projects) {
    if (
      !heldoutGated.has(p.slug) &&
      !/^pulse evolve setup/.test(p.evolve_next || "")
    )
      continue;
    heldoutGated.add(p.slug);
    if (!ready)
      [p.evolve_level, p.evolve_next] = [
        "C",
        heldoutAuto()
          ? "none - the daemon writes the private set itself"
          : "pulse evolve setup DIR",
      ];
    else if (loopOn) [p.evolve_level, p.evolve_next] = ["D", "none"];
    else
      [p.evolve_level, p.evolve_next] = [
        "C",
        "pulse settings set evolve.loop 1",
      ];
  }
}
// The evolution payload with the counts the mock holds: the gates' held-out
// chip, the loop KPI and, for a project whose step was the held-out one, the
// level, reason and next step - so the panel moves after a write too.
function evolutionView(slug, d) {
  const h = state.heldout;
  const p = state.projects.find((x) => x.slug === slug);
  const out = { ...d };
  // ADR-046: the record refs on the registry row move with the console's adopt / discard / draft-job writes; the documents follow them.
  for (const what of ["constitution", "evaluators"]) {
    const doc = d[what];
    const ref = p && p[what];
    if (!doc || !ref || ref.id !== doc.record.id) continue;
    const o = state.records[ref.id];
    out[what] = {
      ...doc,
      record: recordView(doc.record),
      seeded: ref.seeded,
      draft: ref.draft
        ? { ...ref.draft, body: (doc.draft && doc.draft.body) || "" }
        : null,
      draft_job: ref.draft_job ?? null,
      ...(o && o.body ? { body: o.body } : {}),
    };
  }
  if (d.loop)
    out.loop = { ...d.loop, heldout_present: h.configured && h.present };
  if (d.gates) {
    const problem = !h.configured
      ? "evolve.heldout_dir is not set"
      : !h.present
        ? "private fixture validation failed; owner: pulse evolve setup DIR"
        : h.problem;
    out.gates = {
      ...d.gates,
      heldout: {
        count: h.count,
        pending: h.pending,
        generated: h.generated,
        ...(problem ? { problem } : {}),
      },
    };
    if (p && heldoutGated.has(slug)) {
      out.gates.level = p.evolve_level;
      out.gates.next_human_step = p.evolve_next;
      out.gates.reason =
        p.evolve_level === "D"
          ? "the loop may run until a stop rule fires"
          : /evolve\.loop/.test(p.evolve_next)
            ? "evolve.loop is off"
            : heldoutAuto()
              ? "the daemon writes the private held-out set on its next tick (ADR-033)"
              : "the two private held-out fixtures are the one step the owner is the point of (ADR-012)";
    }
    out.gates.planning =
      out.gates.level === "D"
        ? {
            state: "waiting",
            reason:
              "No eligible, untried, in-scope experiment is simulated by this offline mock; no work is dispatched.",
          }
        : {
            state: "gated",
            reason: out.gates.reason + "; owner: " + out.gates.next_human_step,
          };
  }
  return out;
}

function send(res, code, body, headers = {}) {
  const text = typeof body === "string" ? body : JSON.stringify(body);
  res.writeHead(code, {
    "content-type": "application/json; charset=utf-8",
    "cache-control": "no-store",
    "access-control-allow-origin": "*",
    ...headers,
  });
  res.end(text);
}
const fail = (res, code, error, why, fix) =>
  send(res, code, { error, why, fix });

function harnessLoggedIn(name = state.harness?.provider?.name) {
  return providerLogin(name).logged_in === true;
}

// Every provider the real registry has, with the sign-in flags it reports.
// Anthropic's sign-in runs Anthropic's CLI on the hub's machine: a device
// flow with no code to type.
const MOCK_PROVIDERS = [
  ["copilot", "GitHub Copilot", true],
  ["openai", "OpenAI-compatible", false],
  ["anthropic", "Anthropic Claude", true],
  ["gemini", "Google Gemini", false],
  ["azure-openai", "Azure OpenAI", true],
  ["openrouter", "OpenRouter", false],
  ["groq", "Groq", false],
  ["mistral", "Mistral", false],
  ["deepseek", "DeepSeek", false],
  ["xai", "xAI Grok", false],
  ["together", "Together AI", false],
  ["fireworks", "Fireworks AI", false],
  ["cerebras", "Cerebras", false],
  ["github-models", "GitHub Models", true],
  ["ollama", "Ollama (local)", false],
  ["lmstudio", "LM Studio (local)", false],
  ["vllm", "vLLM (local)", false],
  ["custom", "Custom (OpenAI-compatible)", false],
].map(([name, label, device_login]) => ({ name, label, device_login, token_login: true }));
const LOCAL_PROVIDERS = new Set(["ollama", "lmstudio", "vllm"]);
const LOGGED_OUT = { logged_in: false, user: "", source: "", plan: "", expires_at: "", detail: "" };

// Each provider's models in the mock: Anthropic and OpenAI get their own
// lists, a local server a couple of open models, the rest the fixture.
function mockModels(name) {
  const m = (id, name, extra = {}) => ({ id, name, vendor: "", chat: true, tool_calls: true, premium: 1, preview: false, context_window: 200000, max_output: 64000, default: false, ...extra });
  if (name === "anthropic")
    return [
      m("claude-opus-5-5", "Claude Opus 5.5", { context_window: 1000000, max_output: 128000 }),
      m("claude-sonnet-5-5", "Claude Sonnet 5.5", { context_window: 1000000, max_output: 128000, default: true }),
      m("claude-haiku-4-5", "Claude Haiku 4.5"),
      m("claude-fable-5-1", "Claude Fable 5.1", { context_window: 1000000, max_output: 128000 }),
    ];
  if (name === "openai") return [m("gpt-5.5", "GPT-5.5", { default: true }), m("gpt-5.5-mini", "GPT-5.5 mini"), m("o5", "o5")];
  if (name === "gemini") return [m("gemini-3-pro", "Gemini 3 Pro", { default: true, context_window: 1000000 }), m("gemini-3-flash", "Gemini 3 Flash", { context_window: 1000000 })];
  if (LOCAL_PROVIDERS.has(name)) return [m("qwen3-coder:30b", "qwen3-coder:30b", { default: true, context_window: 262144 }), m("llama4:16x17b", "llama4:16x17b")];
  return state.harnessModels.models;
}

function providerLogin(name) {
  if (!state.providerLogins) state.providerLogins = { copilot: state.harness?.provider?.login ?? LOGGED_OUT };
  if (LOCAL_PROVIDERS.has(name)) return { logged_in: true, user: "127.0.0.1", source: "no key", plan: "", expires_at: "", detail: "" };
  return state.providerLogins[name] ?? { ...LOGGED_OUT, detail: "Not signed in to " + (MOCK_PROVIDERS.find((p) => p.name === name)?.label ?? name) + "." };
}

function providerSummary(name) {
  const p = MOCK_PROVIDERS.find((x) => x.name === name);
  return p ? { ...p, login: providerLogin(name) } : null;
}

function syncHarnessSetting(name, value) {
  const hs = state.harness?.settings?.find((x) => x.name === name);
  if (hs) {
    hs.value = value;
    hs.updated_at = now();
    hs.updated_by = "owner";
  }
  if (!state.harness) return;
  if (name === "provider") {
    const p = MOCK_PROVIDERS.find((x) => x.name === value);
    if (p) state.harness.provider = providerSummary(value);
  }
  if (name === "measure_model") state.harness.models.measure = value;
  if (name === "ask.model") state.harness.models.ask = value;
  if (name === "worker_model") state.harness.models.worker = value;
  if (name === "light_model") state.harness.models.light = value;
  if (name === "harness.max_steps")
    state.harness.limits.max_steps = Number(value);
  if (name === "harness.step_timeout")
    state.harness.limits.step_timeout_seconds = Number(value);
  if (name === "harness.output_limit")
    state.harness.limits.output_limit = Number(value);
  if (name === "harness.max_cost")
    state.harness.limits.max_cost = Number(value);
  if (name === "harness.keep_observations")
    state.harness.limits.keep_observations = Number(value);
  if (name === "harness.system_prompt" && state.harnessPrompt) {
    state.harnessPrompt.template = value;
    state.harnessPrompt.custom = value !== "";
    state.harnessPrompt.rendered = value || state.harnessPrompt.default;
    delete state.harnessPrompt.error;
  }
}

function setHarnessLogin(login, name = state.harness.provider.name) {
  providerLogin(name);
  state.providerLogins[name] = login;
  if (name === state.harness.provider.name) state.harness.provider.login = login;
}

async function readBody(req) {
  const chunks = [];
  for await (const c of req) chunks.push(c);
  const raw = Buffer.concat(chunks).toString("utf8");
  if (!raw) return {};
  try {
    return JSON.parse(raw);
  } catch {
    throw new Error("body is not JSON");
  }
}

// What the mock's "Update now" is doing; reset when the mock restarts.
const updateApply = { state: "idle", at: "", actor: "", from: "", latest: "", error: "", endsAt: 0 };

async function fixture(rel) {
  const file = normalize(join(root, rel + ".json"));
  if (!file.startsWith(root)) return null;
  try {
    await stat(file);
    return JSON.parse(await readFile(file, "utf8"));
  } catch {
    return null;
  }
}

const pad = (n, w) => String(n).padStart(w, "0");
const nextId = (prefix, rows) =>
  prefix +
  "-" +
  pad(
    rows.reduce(
      (m, r) => Math.max(m, Number(String(r.id).split("-").pop()) || 0),
      0,
    ) + 1,
    prefix === "MSG" ? 4 : 3,
  );

// GET tasks/{id}/activity as the hub folds it (pulse-tasks activity.rs):
// the brief, each revision, each message (whole in `reference`), each
// step added, one run per step that somebody took with its session's
// loud steps one by one and the quiet stretches (reads, searches, notes)
// folded into one line. A session fixture's steps are shifted onto the
// job's own clock so the chapter sits where the job does.
const LOUD = new Set([
  "assistant",
  "command",
  "edit",
  "write",
  "plan",
  "error",
  "exit",
  "subagent",
  "mcp",
  "fetch",
  "search",
  "skill",
]);
const TOOL = new Set([
  "command",
  "read",
  "grep",
  "glob",
  "edit",
  "write",
  "process",
  "mcp",
  "fetch",
  "search",
  "skill",
  "subagent",
  "memory",
]);
const TEXT_CAP = 1200;
const firstLine = (s) =>
  String(s || "")
    .split("\n")[0]
    .trim();
const clip = (s, cap) => {
  const text = String(s || "").trimEnd();
  return text.length <= cap
    ? [text, false]
    : [text.slice(0, cap) + "\u2026", true];
};
const RANK = {
  task: 0,
  message: 1,
  job: 2,
  "run:started": 3,
  step: 4,
  quiet: 4,
  "run:ended": 5,
  land: 6,
  post: 7,
};
const rank = (i) => RANK[i.kind === "run" ? "run:" + i.what : i.kind] ?? 4;

function quietTitle(kinds) {
  const order = [];
  const counts = new Map();
  for (const k of kinds) {
    if (!counts.has(k)) order.push(k);
    counts.set(k, (counts.get(k) || 0) + 1);
  }
  return order
    .map((k) => (counts.get(k) === 1 ? k : k + " \u00d7" + counts.get(k)))
    .join(" \u00b7 ");
}

function foldSteps(session, steps, agent, jobId, runId, shift) {
  const at = (s) => new Date(Date.parse(s.at) + shift).toISOString();
  const out = [];
  const counts = { calls: 0, commands: 0, edits: 0 };
  let quiet = [];
  const flush = () => {
    if (!quiet.length) return;
    out.push({
      id: `quiet:${session.id}:${quiet[0].n}`,
      at: at(quiet[0]),
      kind: "quiet",
      what: "quiet",
      who: agent,
      job_id: jobId,
      run_id: runId,
      session_id: session.id,
      title: quietTitle(quiet.map((s) => s.kind)),
      count: quiet.length,
    });
    quiet = [];
  };
  const base = (s) => ({
    id: `step:${session.id}:${s.n}`,
    at: at(s),
    kind: "step",
    what: s.kind,
    who: agent,
    job_id: jobId,
    run_id: runId,
    session_id: session.id,
  });
  for (let i = 0; i < steps.length; i++) {
    const s = steps[i];
    if ((s.depth || 0) > 0) continue;
    if (TOOL.has(s.kind)) counts.calls += 1;
    if (s.kind === "command") counts.commands += 1;
    if (s.kind === "edit" || s.kind === "write") counts.edits += 1;
    const outcome =
      TOOL.has(s.kind) &&
      steps[i + 1]?.kind === "observation" &&
      (steps[i + 1].depth || 0) === 0
        ? steps[++i]
        : null;
    if (!LOUD.has(s.kind)) {
      if (s.kind !== "observation") quiet.push(s);
      continue;
    }
    flush();
    const item = base(s);
    if (outcome) {
      item.exit_code = outcome.exit_code;
      item.duration_ms = outcome.duration_ms;
    }
    switch (s.kind) {
      case "assistant":
      case "plan":
        [item.text, item.truncated] = clip(s.content.trim(), TEXT_CAP);
        break;
      case "command":
        item.title = clip(s.title?.trim() || firstLine(s.command), 200)[0];
        [item.text, item.truncated] = clip(s.command.trim(), 400);
        break;
      case "edit":
      case "write":
        item.title = clip(firstLine(s.content), 200)[0];
        break;
      case "error":
        [item.text, item.truncated] = clip(s.content.trim(), TEXT_CAP / 2);
        item.status = "error";
        break;
      case "exit":
        item.status = firstLine(s.content);
        item.title = item.status;
        break;
      default:
        item.title = clip(
          firstLine(s.content.trim() ? s.content : s.command),
          200,
        )[0];
    }
    out.push(item);
  }
  flush();
  return [out, counts];
}

function taskActivity(detail, messages) {
  const task = detail.task;
  const items = [];
  const runs = [];
  let running = false;
  const [brief, cut] = clip(task.brief.trim(), TEXT_CAP);
  items.push({
    id: "task:created",
    at: task.created_at,
    kind: "task",
    what: "created",
    who: task.created_by,
    title: task.title,
    text: brief,
    truncated: cut,
  });
  for (const r of detail.revisions || []) {
    if (r.rev <= 1) continue;
    const [text, cut] = clip(r.note.trim(), TEXT_CAP);
    items.push({
      id: "rev:" + r.rev,
      at: r.at,
      kind: "task",
      what: "revised",
      who: r.author,
      title: r.title,
      text,
      truncated: cut,
    });
  }
  for (const m of messages) {
    const [text, cut] = clip(m.body.trim(), TEXT_CAP * 2);
    items.push({
      id: "msg:" + m.id,
      at: m.at,
      kind: "message",
      what: m.job_id ? "report" : "sent",
      who: m.author,
      job_id: m.job_id || "",
      text,
      truncated: cut,
      choices: (m.choices || []).map((c) => c.label),
      reference: m.id,
    });
  }
  for (const job of detail.jobs || []) {
    items.push({
      id: "job:add:" + job.id,
      at: job.created_at,
      kind: "job",
      what: "added",
      who: job.created_by || task.lead || "",
      job_id: job.id,
      title: job.title,
      status: job.status,
    });
    const took =
      job.status === "leased" ||
      job.status === "running" ||
      job.status === "done" ||
      job.status === "failed";
    if (!took) continue;
    const runId = "run-" + job.id;
    const detailOf = job.harness_session
      ? state.harnessDetails[job.harness_session]
      : null;
    const session = detailOf?.session;
    const agent = job.agent || session?.agent || "cinder";
    const started =
      job.leased_at ||
      session?.started_at ||
      new Date(Date.parse(job.created_at) + 60_000).toISOString();
    const live = job.status === "leased" || job.status === "running";
    const status = live ? "running" : job.status;
    if (live) running = true;
    items.push({
      id: "run:start:" + runId,
      at: started,
      kind: "run",
      what: "started",
      who: agent,
      job_id: job.id,
      run_id: runId,
      title: job.title,
    });
    const run = {
      id: runId,
      job_id: job.id,
      job_title: job.title,
      agent,
      executor: job.executor || "local",
      status,
      exit_code: live ? null : job.status === "done" ? 0 : 1,
      started_at: started,
      finished_at: live ? "" : job.finished_at || "",
      summary: live
        ? ""
        : job.status === "done"
          ? "Done; the branch waits to land."
          : "The run ended without a result.",
      calls: 0,
      commands: 0,
      edits: 0,
      steps: 0,
    };
    if (!live && job.finished_at)
      items.push({
        id: "run:end:" + runId,
        at: job.finished_at,
        kind: "run",
        what: "ended",
        who: agent,
        job_id: job.id,
        run_id: runId,
        title: job.title,
        text: run.summary,
        status,
        exit_code: run.exit_code,
        count: job.status === "done" ? 1 : 0,
      });
    if (session) {
      const shift =
        Date.parse(started) - Date.parse(session.started_at || started);
      const [lines, counts] = foldSteps(
        session,
        detailOf.steps || [],
        agent,
        job.id,
        runId,
        shift,
      );
      items.push(...lines);
      run.session_id = session.id;
      run.calls = counts.calls;
      run.commands = counts.commands;
      run.edits = counts.edits;
      run.steps = (detailOf.steps || []).length;
      // The runner writes the session's token use and cost onto the run
      // (TASK-083); a run without them leaves the fields out.
      if (Number(session.prompt_tokens) > 0)
        run.prompt_tokens = Number(session.prompt_tokens);
      if (Number(session.completion_tokens) > 0)
        run.completion_tokens = Number(session.completion_tokens);
      if (Number(session.cost) > 0) run.cost = Number(session.cost);
    }
    runs.push(run);
  }
  if (task.status === "cancelled")
    items.push({
      id: "task:cancelled",
      at: task.updated_at,
      kind: "task",
      what: "cancelled",
      who: "human",
      title: task.title,
    });
  else if (task.status === "done")
    items.push({
      id: "task:status:done",
      at: task.updated_at,
      kind: "task",
      what: "status",
      who: "human",
      title: "in_progress",
      status: "done",
    });
  items.sort((a, b) => a.at.localeCompare(b.at) || rank(a) - rank(b));
  return {
    task_id: task.id,
    items,
    runs,
    total: items.length,
    has_more: false,
    running,
    now: now(),
  };
}

// A canned lead reply so the chat pane can be exercised end to end.
function leadReplies(project, taskId, body) {
  const lead = project + "-lead";
  setTimeout(() => {
    const m = {
      id: nextId("MSG", state.messages),
      project,
      task_id: taskId,
      author: lead,
      body: `Got it${taskId ? " on " + taskId : ""}: "${body.slice(0, 80)}${body.length > 80 ? "…" : ""}". I'll fold that into the plan and report back here when the next job lands.`,
      at: now(),
      read_by: [],
    };
    state.messages.push(m);
    const t = taskId && state.tasks.find((x) => x.id === taskId);
    if (t) t._detail.messages.push(m);
    emit(lead, "message.created", m.id, { project, task_id: taskId });
  }, 2500);
}

const MIME = {
  html: "text/html; charset=utf-8",
  js: "text/javascript",
  css: "text/css",
  svg: "image/svg+xml",
  json: "application/json",
  txt: "text/plain",
  woff2: "font/woff2",
  ico: "image/x-icon",
  png: "image/png",
  webp: "image/webp",
};
async function serveStatic(res, pathname) {
  const clean = normalize(decodeURIComponent(pathname));
  const tryFiles = clean.endsWith("/")
    ? [clean + "index.html"]
    : [clean, clean + "/index.html", clean + ".html"];
  for (const rel of tryFiles) {
    const file = normalize(join(staticDir, rel));
    if (!file.startsWith(staticDir)) break;
    try {
      const st = await stat(file);
      if (!st.isFile()) continue;
      const ext = file.split(".").pop();
      res.writeHead(200, {
        "content-type": MIME[ext] ?? "application/octet-stream",
        "cache-control": "no-store",
      });
      return res.end(await readFile(file));
    } catch {
      // try the next candidate
    }
  }
  try {
    res.writeHead(404, { "content-type": MIME.html });
    return res.end(await readFile(join(staticDir, "404.html")));
  } catch {
    res.writeHead(404, { "content-type": "text/plain" });
    return res.end("not found");
  }
}

// Secret-flagged settings never leave the server: the hub answers
// "[owner-private]" when a value is set and "" when it is not.
const maskSetting = (s) =>
  s.secret ? { ...s, value: s.value ? "[owner-private]" : "" } : s;

// The Python secret_shaped rule the store keeps (pulse_store::secret_shaped).
function secretShaped(v) {
  return (
    /^(sk|ghp|gho|ghu|ghs|ghr|xox[abpr]|AKIA|AIza|ya29|eyJ)[A-Za-z0-9_\-]{8,}/.test(
      v,
    ) ||
    /-----BEGIN [A-Z ]*PRIVATE KEY-----/.test(v) ||
    /^ssh-(rsa|ed25519|dss) /.test(v) ||
    (/^[A-Za-z0-9+/=_\-]{40,}$/.test(v) && !/^[0-9a-f\-]{36}$/i.test(v))
  );
}

// ----- routes ------------------------------------------------------------

async function handle(req, res) {
  const url = new URL(req.url, "http://x");
  if (req.method === "OPTIONS")
    return send(res, 204, "", {
      "access-control-allow-methods": "GET,POST,PATCH,OPTIONS",
      "access-control-allow-headers": "content-type",
    });
  if (!url.pathname.startsWith("/api/v1/")) {
    if (staticDir) return serveStatic(res, url.pathname);
    return fail(
      res,
      404,
      "not found",
      "the mock only serves /api/v1",
      "start next dev alongside: npm run dev:mock",
    );
  }
  const segs = url.pathname
    .slice("/api/v1/".length)
    .split("/")
    .filter(Boolean)
    .map(decodeURIComponent);
  const q = url.searchParams;
  if (delay) await new Promise((r) => setTimeout(r, delay));

  if (segs[0] === "jobs") {
    // A job's artifacts folder: GET jobs/<id>/artifacts lists it, GET
    // jobs/<id>/artifacts/<name> serves one file with its mime (the same
    // shape as a conversation's artifacts), so the task view's evidence
    // tab works offline. The folders live in mock/api/jobs/<id>/artifacts/.
    if (segs.length >= 3 && segs[2] === "artifacts" && segs.length <= 4) {
      if (req.method !== "GET")
        return fail(
          res,
          405,
          "Method not allowed",
          "Use GET for a job's artifacts.",
          "Open the task's step.",
        );
      if (segs.length === 3) {
        const l = await jobArtifacts.list(segs[1]);
        if (!l)
          return fail(
            res,
            404,
            "no such job",
            segs[1] + " is not in the store",
            "open it from the task",
          );
        return send(res, 200, l);
      }
      const f = await jobArtifacts.file(segs[1], segs[3]);
      if (!f)
        return fail(
          res,
          404,
          "no such artifact",
          segs[3] + " is not in the job's folder",
          "open the listing",
        );
      res.writeHead(200, {
        "content-type": f.type,
        "cache-control": "no-store",
        "access-control-allow-origin": "*",
      });
      return res.end(f.bytes);
    }
    const method = segs[1] === "repair" ? "POST" : "GET";
    if (segs.length !== 2 || !["failed", "repair"].includes(segs[1]))
      return fail(
        res,
        404,
        "Unknown jobs route",
        "Use failed, repair or <id>/artifacts.",
        "Open Failed job repair.",
      );
    if (req.method !== method)
      return fail(
        res,
        405,
        "Method not allowed",
        "Use " + method + " for this route.",
        "Open Failed job repair.",
      );
  }
  if (
    segs[0] === "projects" &&
    segs.length === 3 &&
    segs[2] === "goal" &&
    !["GET", "POST"].includes(req.method)
  )
    return fail(
      res,
      405,
      "Method not allowed",
      "Goals support GET and POST.",
      "Use the project goal editor.",
    );
  if (
    segs[0] === "projects" &&
    segs.length === 3 &&
    segs[2] === "needs" &&
    !["GET", "POST"].includes(req.method)
  )
    return fail(
      res,
      405,
      "Method not allowed",
      "Needs support GET and POST.",
      "Use the project's Runs on panel.",
    );

  // SSE
  if (segs[0] === "events" && segs[1] === "stream") {
    res.writeHead(200, {
      "content-type": "text/event-stream",
      "cache-control": "no-store",
      connection: "keep-alive",
      "access-control-allow-origin": "*",
    });
    res.write(": hello\n\n");
    streams.add(res);
    const beat = setInterval(() => res.write(": ping\n\n"), 15000);
    req.on("close", () => {
      clearInterval(beat);
      streams.delete(res);
    });
    return;
  }

  if (segs[0] === "voice") return voiceRoute(req, res, segs, q);

  if (
    (req.method === "POST" || req.method === "PATCH") &&
    (segs[0] === "mcp" || segs[0] === "skills")
  ) {
    let body;
    try {
      body = await readBody(req);
    } catch (e) {
      return fail(
        res,
        400,
        "bad request",
        String(e.message),
        "send a JSON object",
      );
    }
    return segs[0] === "mcp"
      ? mcpWrite(req, res, segs, q, body)
      : skillWrite(req, res, segs, q, body);
  }

  // The update banner's "Update now" (TASK-093): POST update/apply starts an
  // install that ends a few seconds later; GET update follows it. MOCK_UPDATE_CHANNEL=source
  // shows the refusal; MOCK_UPDATE_FAIL=1 a failed install.
  if (segs[0] === "update" && segs.length <= 2) {
    const base = await fixture("update");
    const channel = process.env.MOCK_UPDATE_CHANNEL || base.channel || "release";
    const apply = () => {
      if (updateApply.state === "installing" && Date.now() >= updateApply.endsAt) {
        updateApply.state = process.env.MOCK_UPDATE_FAIL ? "failed" : "installed";
        updateApply.error = process.env.MOCK_UPDATE_FAIL ? "download SHA256SUMS: HTTP 404" : "";
        updateApply.at = now();
      }
      const rest = { ...updateApply };
      delete rest.endsAt;
      return rest;
    };
    if (segs.length === 1) return send(res, 200, { ...base, channel, apply: apply() });
    if (segs[1] !== "apply") return fail(res, 404, "not found", "no route " + url.pathname, "use update or update/apply");
    if (req.method !== "POST") return fail(res, 405, "method not allowed", "Only POST installs an update.", "Send POST /api/v1/update/apply.");
    if (channel === "source")
      return fail(res, 409, "update not started", "update.channel resolves to source here: the daemon rebuilds pulse from the brain project's checkout after each landing", "To install published releases instead: pulse settings set update.channel release");
    if (apply().state === "installing") return fail(res, 409, "update not started", "an update is already being installed", "Wait for the install running to end.");
    Object.assign(updateApply, { state: "installing", at: now(), actor: "human", from: base.running, latest: base.latest, error: "", endsAt: Date.now() + Number(process.env.MOCK_UPDATE_MS || 4000) });
    return send(res, 202, { ...base, channel, apply: apply() });
  }
  if (req.method === "POST") {
    let body;
    try {
      body = await readBody(req);
    } catch (e) {
      return fail(
        res,
        400,
        "bad request",
        String(e.message),
        "send a JSON object",
      );
    }
    // Memory (crates/pulse-hub/src/memory_api.rs): remember appends a dated
    // bullet to one file, forget removes the one entry whose line contains
    // the text; both answer with the file as GET memory lists it.
    if (segs[0] === "memory" && segs.length <= 2) {
      return memoryWrite(res, segs, body);
    }
    // Secrets: a write keeps the name and drops the value; remove by name.
    if (segs[0] === "secrets" && segs.length === 1) {
      const name = String(body.name || "").trim();
      if (!/^[A-Z_][A-Z0-9_]*$/.test(name) || !body.value)
        return fail(res, 400, "`" + name + "` is not a secret name", "The vault takes an upper-case name and a value.", "Correct the name or value and save again.");
      const now = new Date().toISOString();
      const old = state.secrets.find((x) => x.name === name);
      const entry = { name, created_by: old?.created_by || "human", created_at: old?.created_at || now, updated_by: "human", updated_at: now, sealed_bytes: String(body.value).length + 16 };
      state.secrets = [...state.secrets.filter((x) => x.name !== name), entry].sort((a, b) => a.name.localeCompare(b.name));
      return send(res, 201, { secret: entry });
    }
    if (segs[0] === "secrets" && segs.length === 3 && segs[2] === "remove") {
      const had = state.secrets.some((x) => x.name === segs[1]);
      state.secrets = state.secrets.filter((x) => x.name !== segs[1]);
      return had ? send(res, 200, { removed: segs[1] }) : fail(res, 404, "No secret " + segs[1], "The team vault holds no secret by that name.", "Pick a name from the list.");
    }
    // The Chat page's two switches on a conversation: Auto (approve tools
    // without asking) and Lockdown (sandbox). The hub flips the row and the
    // live session; the mock remembers the flip and answers the way GET
    // chat/<id> would, with the live state carrying the new word.
    if (
      segs[0] === "chat" &&
      segs.length === 3 &&
      (segs[2] === "auto" || segs[2] === "lockdown")
    ) {
      if (typeof body.on !== "boolean")
        return fail(
          res,
          400,
          "Invalid on",
          "on must be true or false.",
          "Send on:true or on:false.",
        );
      const d = await fixture("chat/" + encodeURIComponent(segs[1]));
      if (d === null)
        return fail(
          res,
          404,
          "No such conversation",
          "nothing under " + segs[1] + ".",
          "Pick a conversation from the list.",
        );
      const flips = (state.chatFlips[segs[1]] ??= {});
      flips[segs[2] === "auto" ? "auto_approve" : "lockdown"] = body.on;
      return send(res, 200, { conversation: chatWithFlips(d.conversation) });
    }
    // Automations (`pulse schedule`, `.pulse/pulse.yaml`): check a draft,
    // write it into the file or the store, remove an entry, act on a row.
    if (segs[0] === "automations") {
      const a = state.automations;
      const r =
        segs.length === 1
          ? a.write(body)
          : segs.length === 2 && segs[1] === "check"
            ? a.check(body)
            : segs.length === 2 && segs[1] === "remove"
              ? a.remove(body)
              : segs.length === 3
                ? a.action(segs[1], segs[2], body)
                : null;
      if (r) return send(res, r.status, r.body);
    }
    // Add a suggested tool or skill the way `pulse toolkit add` would.
    if (segs[0] === "toolkit" && segs.length === 1) {
      const project = String(body.project ?? "").trim();
      const kind = String(body.kind ?? "").trim();
      const name = String(body.name ?? "").trim();
      const g = state.toolkit.projects.find((x) => x.project === project);
      if (!g)
        return fail(
          res,
          404,
          "No such project",
          "no project named " + project + ".",
          "Pick a project from the list.",
        );
      const t = now();
      if (kind === "tool" || kind === "mcp") {
        const x =
          g.tools.find((y) => y.name === name) ||
          state.toolkit.catalog.tools.find((y) => y.name === name);
        if (!x)
          return fail(
            res,
            404,
            "No tool " + name,
            "the catalog has no tool named " + name,
            "pulse toolkit --json lists it",
          );
        if (x.auth !== "none" && x.auth !== "oauth")
          return fail(
            res,
            409,
            name +
              " needs a secret (" +
              x.secret +
              "); register it yourself once the secret is in place: " +
              x.add,
            "The store could not complete this request.",
            "Refresh the page and check the request.",
          );
        if (state.mcp.some((s) => s.name === name))
          return fail(
            res,
            409,
            "Already there",
            "a tool named " + name + " exists",
            "pulse mcp list",
          );
        const server = {
          id: "MCP-" + String(++state.seq).padStart(3, "0"),
          name,
          project: x.scope === "shared" || x.shared ? "" : project,
          transport: x.transport,
          command: x.command || "",
          args: x.args || [],
          url: x.url || "",
          auth: x.auth,
          auth_header: x.auth_header || "",
          secret_ref: "",
          env: {},
          note: x.about + " Suggested by pulse toolkit for " + project + ".",
          status: "enabled",
          created_by: "human",
          created_at: t,
          updated_at: t,
          last_check_at: "",
          last_check_ok: null,
          last_check_note: "",
        };
        state.mcp.push(server);
        emit("human:console", "mcp.added", server.id, { name });
        return send(res, 201, { server });
      }
      if (kind === "skill") {
        const x = g.skills.find((y) => y.name === name);
        if (!x)
          return fail(
            res,
            404,
            "No skill " + name,
            "the catalog has no skill named " + name,
            "pulse toolkit --json lists it",
          );
        if (state.skills.some((k) => k.name === name))
          return fail(
            res,
            409,
            "Already there",
            "a skill named " + name + " exists",
            "pulse skill list",
          );
        const skill = {
          id: "SKILL-" + String(++state.seq).padStart(3, "0"),
          name,
          project,
          description: x.description,
          body: x.body,
          status: "enabled",
          reads: 0,
          created_by: "human",
          created_at: t,
          updated_at: t,
        };
        state.skills.push(skill);
        emit("human:console", "skill.added", skill.id, { name });
        return send(res, 201, { skill });
      }
      return fail(
        res,
        400,
        "Unknown kind",
        "kind is tool or skill, not " + kind,
        'Send kind: "tool" or kind: "skill".',
      );
    }
    // Seat a lead: a built-in role by name, or a word of the owner's with a title and a brief.
    if (segs[0] === "leads" && segs.length === 1) {
      const project = String(body.project ?? "").trim();
      const role = String(body.role ?? "")
        .trim()
        .toLowerCase();
      if (!state.projects.some((p) => p.slug === project))
        return fail(
          res,
          404,
          "No such project",
          "no project named " + project + ".",
          "Pick a project from the list.",
        );
      if (!/^[a-z][a-z0-9-]{1,23}$/.test(role))
        return fail(
          res,
          400,
          "Bad role",
          "a role is a short lowercase word: letters, digits and hyphens",
          "pulse lead add --project P --role docs",
        );
      const builtin = state.leads.roles.find((r) => r.role === role);
      const title = String(body.title ?? "").trim() || builtin?.title || "";
      const brief = String(body.brief ?? "").trim();
      if (!builtin && (!title || !brief))
        return fail(
          res,
          400,
          "A role of your own needs words",
          "--title and --brief say what " +
            role +
            " owns and what counts as done",
          "pulse lead add --project " +
            project +
            " --role " +
            role +
            " --title 'Content lead' --brief '...'",
        );
      if (
        state.leads.leads.some(
          (l) =>
            l.project === project && l.role === role && l.status !== "ended",
        )
      )
        return fail(
          res,
          409,
          "Already seated",
          "a " + role + " lead sits on " + project + " already",
          "pulse lead list --project " + project,
        );
      const every = String(body.every ?? "").trim() || builtin?.every || "1d";
      const m = /^(\d+)([hdw])$/.exec(every);
      const minutes = m
        ? Number(m[1]) * (m[2] === "h" ? 60 : m[2] === "d" ? 1440 : 10080)
        : 1440;
      const t = now();
      const lead = {
        id: "LEAD-" + String(++state.seq).padStart(3, "0"),
        project,
        role,
        agent: project + "-lead-" + role,
        title,
        brief,
        every_minutes: minutes,
        status: "active",
        next_at: t,
        last_at: "",
        last_job: "",
        runs: 0,
        note: "",
        created_by: "you",
        created_at: t,
        updated_at: t,
      };
      state.leads.leads.push(lead);
      emit("human:console", "lead.add", lead.id, { project, role });
      return send(res, 201, { lead });
    }
    if (segs[0] === "leads" && segs.length === 3) {
      const lead = state.leads.leads.find((l) => l.id === segs[1]);
      if (!lead)
        return fail(
          res,
          404,
          "No such lead",
          "nothing seated as " + segs[1],
          "pulse lead list",
        );
      const action = segs[2];
      const t = now();
      if (action === "pause")
        Object.assign(lead, {
          status: "paused",
          note: String(body.note ?? "").trim(),
          updated_at: t,
        });
      else if (action === "resume")
        Object.assign(lead, {
          status: "active",
          note: "",
          next_at: t,
          updated_at: t,
        });
      else if (action === "end")
        Object.assign(lead, {
          status: "ended",
          note: String(body.note ?? "").trim(),
          updated_at: t,
        });
      else if (action === "run")
        Object.assign(lead, { next_at: t, updated_at: t });
      else
        return fail(
          res,
          404,
          "not found",
          "No such lead action.",
          "pause, resume, end or run.",
        );
      emit("human:console", "lead." + action, lead.id, {
        project: lead.project,
        role: lead.role,
      });
      return send(res, 200, { lead });
    }
    // Ask: a canned, cited answer about the project; the real hub asks a model.
    if (segs[0] === "ask" && segs.length === 1) {
      const question = String(body.question ?? "").trim();
      if (!question)
        return fail(
          res,
          400,
          "Nothing to ask",
          "The question is empty.",
          "Ask one question of up to 2000 characters.",
        );
      const project = String(body.project ?? "").trim();
      if (
        project &&
        !state.projects.some((p) => p.slug === project) &&
        project !== "commons"
      )
        return fail(
          res,
          404,
          "No such project",
          "no project named " + project + ".",
          "Pick a project from the list.",
        );
      const tasks = state.tasks.filter(
        (t) =>
          (!project || t.project === project) &&
          t.status !== "done" &&
          t.status !== "cancelled",
      );
      const slug = project || "demo-app";
      const sources = tasks.slice(0, 2).map((t) => ({
        id: t.id,
        source: "task",
        kind: t.status,
        summary: t.title,
        date: t.updated_at,
        project: t.project,
        thread: "",
      }));
      sources.push({
        id: "20260901T140800Z-cinder",
        source: "post",
        kind: "finding",
        summary: "p95 of /checkout is 840 ms against a 400 ms budget",
        date: "2026-09-01T14:08:00Z",
        project: slug,
        thread: "checkout-latency",
      });
      const facts = q.get("facts") === "1" || /facts only/i.test(question);
      const lines = facts
        ? [
            "No model answered (not signed in), so here are the facts that match.",
            "",
            ...sources.map((s) => "- [" + s.id + "] " + s.summary),
          ]
        : [
            (tasks.length
              ? tasks.length + " open task" + (tasks.length === 1 ? "" : "s")
              : "Nothing open") +
              (project ? " in **" + project + "**" : " across all projects") +
              ". " +
              (sources[0]
                ? "The one moving now is **" +
                  sources[0].summary +
                  "** [" +
                  sources[0].id +
                  "]."
                : ""),
            "",
            "The checkout latency finding [20260901T140800Z-cinder] still stands: p95 is 840 ms against a 400 ms budget. " +
              (body.history?.length
                ? "As before, nothing waits for your decision here."
                : "Nothing waits for your decision here."),
          ];
      emit("human:console", "ask", project, {
        question_chars: question.length,
        model: facts ? "" : "gpt-4.1",
        facts_only: facts,
        elapsed_ms: 640,
        sources: sources.length,
      });
      await new Promise((r) => setTimeout(r, 650));
      return send(res, 200, {
        answer: lines.join("\n"),
        project,
        model: facts ? "" : "gpt-4.1",
        provider: facts ? "" : "copilot",
        facts_only: facts,
        note: facts ? "not signed in" : "",
        sources,
        elapsed_ms: 640,
        usage: {
          prompt_tokens: facts ? 0 : 1840,
          completion_tokens: facts ? 0 : 96,
          cost: 0,
        },
      });
    }
    if (segs[0] === "jobs" && segs[1] === "repair") {
      if (req.headers["x-pulse-job-repair"] !== "yes")
        return fail(
          res,
          403,
          "missing X-Pulse-Job-Repair: yes",
          "An explicit confirmation header is required.",
          "Select jobs and confirm their repair.",
        );
      const result = state.jobRepairs.repair(body, q.get("project") || "");
      return send(res, result.status, result.body);
    }
    // suggested work: the owner suggests, or rules on, a suggestion
    if (segs[0] === "tasks" && segs[1] === "suggested") {
      const r =
        segs.length === 2
          ? state.suggestions.suggest(body)
          : state.suggestions.act(segs[2], segs[3], body);
      return send(res, r.status, r.body);
    }
    // tasks
    if (segs[0] === "tasks" && segs.length === 1) {
      if (!body.project || !body.title?.trim())
        return fail(
          res,
          400,
          "task needs a project and a title",
          "the lead is woken per project and plans from the title + brief",
          "pick a project and write a title",
        );
      const t = {
        id: nextId("TASK", state.tasks),
        project: body.project,
        title: body.title.trim(),
        status: "open",
        priority: Number(body.priority ?? 10),
        lead: "",
        job_id: "",
        created_by: "human",
        created_at: now(),
        updated_at: now(),
        current_rev: 1,
        brief: body.brief || "",
        jobs_total: 0,
        jobs_queued: 0,
        jobs_blocked: 0,
        jobs_running: 0,
        jobs_done: 0,
        jobs_failed: 0,
        working: [],
      };
      t._detail = {
        task: t,
        revisions: [
          {
            rev: 1,
            title: t.title,
            brief: t.brief,
            note: "",
            author: "human",
            at: t.created_at,
          },
        ],
        messages: [],
        jobs: [],
      };
      state.tasks.push(t);
      emit("human", "task.created", t.id, {
        project: t.project,
        title: t.title,
      });
      setTimeout(() => {
        t.lead = t.project + "-lead";
        t.status = "planned";
        t.updated_at = now();
        t.jobs_total = 1;
        t.jobs_done = 1;
        t._detail.jobs.push({
          id: "job-" + Date.now(),
          kind: "plan",
          title: "Plan " + t.id + " into jobs",
          status: "done",
          agent: t.lead,
          branch: "",
          project: t.project,
        });
        emit(t.lead, "task.planned", t.id, { project: t.project });
      }, 4000);
      leadReplies(t.project, t.id, t.title);
      return send(res, 201, { task: strip(t) });
    }
    if (segs[0] === "tasks" && segs[2] === "amend") {
      const t = state.tasks.find((x) => x.id === segs[1]);
      if (!t)
        return fail(
          res,
          404,
          "no such task",
          segs[1] + " is not in the store",
          "open it from the Tasks page",
        );
      if (t.status === "done" || t.status === "cancelled")
        return fail(
          res,
          409,
          "task is closed",
          "a " + t.status + " task takes no new revision",
          "open a new task instead",
        );
      t.current_rev += 1;
      if (body.title?.trim()) t.title = body.title.trim();
      t.brief = body.brief ?? t.brief;
      t.updated_at = now();
      t._detail.revisions.push({
        rev: t.current_rev,
        title: t.title,
        brief: t.brief,
        note: body.note || "",
        author: "human",
        at: t.updated_at,
      });
      emit("human", "task.amended", t.id, {
        rev: t.current_rev,
        note: body.note || "",
      });
      return send(res, 200, { task: strip(t) });
    }
    if (segs[0] === "tasks" && segs[2] === "cancel") {
      const t = state.tasks.find((x) => x.id === segs[1]);
      if (!t)
        return fail(
          res,
          404,
          "no such task",
          segs[1] + " is not in the store",
          "open it from the Tasks page",
        );
      t.status = "cancelled";
      t.updated_at = now();
      emit("human", "task.cancelled", t.id, { reason: body.reason || "" });
      return send(res, 200, { task: strip(t) });
    }
    // The owner moves a task on (the hub's tasks/{id}/status): done after
    // "good enough", or in_progress to hand a review back; `words` reach the
    // task's chat first so the lead reads why.
    if (segs[0] === "tasks" && segs[2] === "status") {
      const t = state.tasks.find((x) => x.id === segs[1]);
      if (!t)
        return fail(
          res,
          404,
          "no such task",
          segs[1] + " is not in the store",
          "open it from the Tasks page",
        );
      if (t.status === "done" || t.status === "cancelled")
        return fail(
          res,
          409,
          "task is closed",
          "a " + t.status + " task does not move again",
          "give a new task instead",
        );
      if (body.status !== "done" && body.status !== "in_progress")
        return fail(
          res,
          400,
          "status must be done or in_progress",
          "the owner finishes a task or hands a review back",
          "pick one of the two",
        );
      if (body.words?.trim()) {
        const m = {
          id: nextId("MSG", state.messages),
          project: t.project,
          task_id: t.id,
          author: "human",
          body: body.words.trim(),
          at: now(),
          read_by: [],
        };
        state.messages.push(m);
        t._detail.messages.push(m);
        emit("human", "message.created", m.id, {
          project: m.project,
          task_id: m.task_id,
        });
      }
      t.status = body.status;
      t.updated_at = now();
      emit(
        "human",
        "task." + (body.status === "done" ? "done" : "resumed"),
        t.id,
        {},
      );
      return send(res, 200, { task: strip(t) });
    }
    // messages
    if (segs[0] === "messages") {
      if (!body.project || !body.body?.trim())
        return fail(
          res,
          400,
          "message needs a project and a body",
          "the lead of that project is the reader",
          "pick a project and write something",
        );
      const m = {
        id: nextId("MSG", state.messages),
        project: body.project,
        task_id: body.task_id || null,
        author: "human",
        body: body.body,
        at: now(),
        read_by: [],
      };
      state.messages.push(m);
      const t = m.task_id && state.tasks.find((x) => x.id === m.task_id);
      if (t) t._detail.messages.push(m);
      emit("human", "message.created", m.id, {
        project: m.project,
        task_id: m.task_id,
      });
      leadReplies(m.project, m.task_id, m.body);
      return send(res, 201, { message: m });
    }
    // decide
    if (segs[0] === "decide") {
      const p = state.proposals.find((x) => x.id === body.id);
      if (!p) {
        const i = state.questions.findIndex((x) => x.id === body.id);
        if (i < 0)
          return fail(
            res,
            404,
            "no such proposal",
            String(body.id) + " is not awaiting",
            "reload the page",
          );
        const [qn] = state.questions.splice(i, 1);
        emit("human", "question.decided", qn.id, {
          project: qn.slug,
          answer: body.answer,
          words: body.words || "",
          stop: qn.stop,
        });
        return send(res, 200, {
          ok: true,
          decided: {
            ...qn,
            answer: body.answer,
            words: body.words || "",
            decided_date: now(),
          },
        });
      }
      if (p.status !== "awaiting")
        return fail(
          res,
          409,
          "already decided",
          "this proposal is " + p.status,
          "nothing to do",
        );
      p.status = body.answer === "yes" ? "proceeded" : "declined";
      p.decided_date = now();
      p.decided_summary = body.words || "";
      emit("human", "proposal.decided", p.id, {
        answer: body.answer,
        words: body.words || "",
      });
      return send(res, 200, { ok: true, decided: p });
    }
    // native harness auth
    if (segs[0] === "harness" && segs[1] === "login" && segs.length === 2) {
      const name = String(body.provider || state.harness.provider.name);
      const p = MOCK_PROVIDERS.find((x) => x.name === name);
      if (!p?.device_login)
        return fail(res, 400, "device login is not supported", "This provider does not implement device login.", "Choose a provider that supports device login.");
      const device_code = "mock-device-" + ++state.seq;
      state.harnessPolls[device_code] = 0;
      if (name === "anthropic")
        return send(res, 200, { device_code, user_code: "", verification_uri: "https://console.anthropic.com/oauth/authorize?client=ant-cli", interval: 1, expires_in: 600 });
      return send(res, 200, {
        device_code,
        user_code: "ABCD-1234",
        verification_uri: name === "azure-openai" ? "https://microsoft.com/devicelogin" : "https://github.com/login/device",
        interval: 1,
        expires_in: 900,
      });
    }
    if (segs[0] === "harness" && segs[1] === "login" && segs[2] === "poll") {
      const code = String(body.device_code || "");
      if (!code || !(code in state.harnessPolls))
        return send(res, 200, { status: "expired" });
      state.harnessPolls[code] += 1;
      if (state.harnessPolls[code] === 2)
        return send(res, 200, { status: "slow_down" });
      if (state.harnessPolls[code] < 3)
        return send(res, 200, { status: "pending" });
      const name = String(body.provider || state.harness.provider.name);
      const login =
        name === "anthropic"
          ? { logged_in: true, user: "Anthropic Console", source: "anthropic sign-in", plan: "", expires_at: "", detail: "" }
          : name === "azure-openai"
            ? { logged_in: true, user: "owner@contoso.com", source: "device login", plan: "", expires_at: "2027-01-11T12:00:00Z", detail: "" }
            : { logged_in: true, user: "octocat", source: "device login", plan: name === "copilot" ? "individual" : "", expires_at: "2027-09-11T12:00:00Z", detail: "" };
      setHarnessLogin(login, name);
      emit("owner", "harness.login", name, { user: login.user });
      return send(res, 200, { status: "complete", login });
    }
    if (segs[0] === "harness" && segs[1] === "login" && segs[2] === "token") {
      const token = String(body.token || "");
      const name = String(body.provider || state.harness.provider.name);
      const github = name === "copilot" || name === "github-models";
      if (name === "anthropic" && token.startsWith("sk-ant-oat"))
        return fail(res, 400, "token login failed", "That is a Claude Pro or Max subscription token, which Anthropic allows only inside its own apps.", "Paste a Console API key (sk-ant-api…) or use Sign in with Anthropic.");
      if (github ? !/^(gh[opsuhr]_|github_pat_)/.test(token) : token.length < 12 || /^bad/.test(token))
        return fail(res, 400, "token login failed", "The provider refused the key (401).", github ? "Paste a token beginning with gho_ or ghp_." : "Paste a valid key.");
      const login = {
        logged_in: true,
        user: github ? "octocat" : "…" + token.slice(-4),
        source: "token login",
        plan: name === "copilot" ? "individual" : "",
        expires_at: "",
        detail: "",
      };
      setHarnessLogin(login, name);
      emit("owner", "harness.login", name, { user: login.user, source: "token" });
      return send(res, 200, { login });
    }
    if (segs[0] === "harness" && segs[1] === "logout") {
      const login = {
        logged_in: false,
        user: "",
        source: "",
        plan: "",
        expires_at: "",
        detail: "",
      };
      const name = String(body.provider || state.harness.provider.name);
      setHarnessLogin(login, name);
      emit("owner", "harness.logout", name, {});
      return send(res, 200, { ok: true, login });
    }
    if (
      segs[0] === "harness" &&
      segs[1] === "sessions" &&
      segs[3] === "cancel"
    ) {
      const row = state.harnessSessions.find((s) => s.id === segs[2]);
      if (!row)
        return fail(
          res,
          404,
          "no such harness session",
          segs[2] + " is not in the mock store",
          "open it from the Harness page",
        );
      if (row.status !== "running")
        return fail(
          res,
          409,
          "run is not running",
          row.id + " is " + row.status,
          "reload the session list",
        );
      row.cancel_requested_at = now();
      row.updated_at = row.cancel_requested_at;
      if (state.harnessDetails[row.id])
        state.harnessDetails[row.id].session = row;
      emit("owner", "harness.cancel", row.id, { session: row });
      return send(res, 200, { cancelled: true, session: row });
    }
    // settings
    if (segs[0] === "settings") {
      const s = state.settings.settings.find((x) => x.name === body.name);
      if (!s)
        return fail(
          res,
          404,
          "unknown setting",
          String(body.name) + " is not a key the store knows",
          "pick one of the listed keys",
        );
      const v = String(body.value ?? "");
      if ((s.kind === "count" || s.kind === "credits") && !/^\d+$/.test(v))
        return fail(
          res,
          400,
          "not a whole number",
          s.name + " takes a count",
          "enter digits only",
        );
      if (s.kind === "count" && s.min !== null && Number(v) < s.min)
        return fail(
          res,
          400,
          "below the minimum",
          s.name + " must be at least " + s.min,
          "enter " + s.min + " or more",
        );
      if (s.kind === "fraction" && !/^\d+\/\d+$/.test(v))
        return fail(
          res,
          400,
          "not a fraction",
          s.name + " is written like 2/3",
          "use n/m",
        );
      if (s.kind === "choice" && !(s.choices || []).includes(v))
        return fail(
          res,
          400,
          "not a choice",
          s.name + " must be one of " + (s.choices || []).join(", "),
          "pick from the list",
        );
      // store.ComputeRules: an identifier never holds a credential-looking value.
      if (s.kind === "identifier" && secretShaped(v))
        return fail(
          res,
          400,
          "looks like a secret",
          s.name + " is an identifier, not a credential",
          "auth is the hub machine's CLI login (az login, aws configure, gcloud auth login); set only ids, names and regions here",
        );
      const previous = s.value;
      s.value = v;
      s.updated_at = now();
      s.updated_by = "owner";
      if (s.name === "compute.max_vms") state.compute.caps.max_vms = Number(v);
      if (s.name === "compute.max_hours_per_day")
        state.compute.caps.max_hours_per_day = Number(v);
      if (s.name === "compute.budget_credits")
        state.compute.caps.budget_credits = Number(v);
      if (s.name === "compute.auto_approve")
        state.compute.caps.auto_approve = v === "1";
      if (s.name === "compute.provider") state.compute.caps.provider = v;
      syncHarnessSetting(s.name, v);
      // Owner-private settings: the event names the key only and the echo masks
      // the value exactly like the pulse hub ("[owner-private]" when set, "" when cleared).
      emit(
        "owner",
        "settings.set",
        s.name,
        s.secret ? { masked: true } : { value: v, previous },
      );
      return send(res, 200, { ok: true, setting: maskSetting(s) });
    }
    // compute
    if (
      segs[0] === "compute" &&
      (segs[2] === "approve" || segs[2] === "deny")
    ) {
      const r = state.compute.requests.find((x) => x.id === segs[1]);
      if (!r)
        return fail(
          res,
          404,
          "no such request",
          segs[1] + " is not in the store",
          "reload the page",
        );
      if (r.status !== "pending")
        return fail(
          res,
          409,
          "already decided",
          r.id + " is " + r.status,
          "nothing to do",
        );
      if (
        segs[2] === "approve" &&
        state.compute.caps.vms_now >= state.compute.caps.max_vms
      )
        return fail(
          res,
          409,
          "cap reached",
          "compute.max_vms is " +
            state.compute.caps.max_vms +
            " and " +
            state.compute.caps.vms_now +
            " are up",
          "raise compute.max_vms in Settings or release a machine",
        );
      r.status = segs[2] === "approve" ? "approved" : "denied";
      r.decided_by = "human";
      r.updated_at = now();
      emit("human", "compute." + r.status, r.id, { words: body.words || "" });
      if (r.status === "approved")
        setTimeout(() => {
          r.status = "provisioning";
          r.updated_at = now();
          emit("daemon", "compute.provisioning", r.id, { size: r.size });
          setTimeout(() => {
            r.status = "ready";
            r.executor = "pulse-vm" + (state.compute.caps.vms_now + 1);
            state.compute.caps.vms_now += 1;
            r.updated_at = now();
            emit("daemon", "compute.ready", r.id, { executor: r.executor });
          }, 6000);
        }, 3000);
      return send(res, 200, { request: r });
    }
    if (segs[0] === "projects" && segs.length === 3 && segs[2] === "needs") {
      const p = state.projects.find((x) => x.slug === segs[1]);
      if (!p)
        return fail(
          res,
          404,
          "No project " + segs[1],
          "only a registered project has needs",
          "pulse projects",
        );
      const parsed = needWords(
        typeof body.needs === "string" ? body.needs : "",
      );
      if (parsed.error)
        return fail(
          res,
          400,
          parsed.error,
          "a need is a word a machine can offer",
          "pulse machines",
        );
      const was = p.needs || "";
      p.needs = parsed.words.join(",");
      for (const t of state.tasks)
        if (t.project === p.slug && t.job_id && t.status === "open")
          t.needs = p.needs;
      emit("human", "project.needs", p.slug, {
        project: p.slug,
        needs: p.needs,
        was,
      });
      return send(res, 200, { ...(await projectNeeds(p)), was });
    }
    if (segs[0] === "projects" && segs.length === 3 && segs[2] === "goal") {
      if (!body || typeof body !== "object" || Array.isArray(body))
        return fail(
          res,
          400,
          "Invalid goal change",
          "Send a JSON object.",
          "Use the project goal editor.",
        );
      const p = state.projects.find((x) => x.slug === segs[1]);
      if (!p)
        return fail(
          res,
          404,
          "No project " + segs[1],
          "Goals belong to registered projects.",
          "Choose a project.",
        );
      if (p.status !== "active")
        return fail(
          res,
          409,
          "Project is inactive",
          "Restore the project before changing its goal.",
          "Use Restore in Projects.",
        );
      if (
        Object.keys(body).some((key) => key !== "text" && key !== "reset") ||
        "text" in body === "reset" in body ||
        ("reset" in body && body.reset !== true)
      ) {
        return fail(
          res,
          400,
          "Invalid goal change",
          "Send either text or reset:true, not both.",
          "Use the project goal editor.",
        );
      }
      if (body.reset === true) state.goals.delete(p.slug);
      else {
        const raw =
          typeof body.text === "string" ? body.text.replace(/\r\n/g, "\n") : "";
        const text = raw.trim();
        const unsafeControl = /[\p{Cc}\p{Cf}]/u.test(
          raw.replace(/[\n\t\u200c\u200d]/g, ""),
        );
        if (!text || Array.from(text).length > 4000 || unsafeControl)
          return fail(
            res,
            400,
            "Invalid goal",
            "A goal must have 1 to 4000 characters and no unsafe controls.",
            "Write a short, specific project outcome.",
          );
        if (
          secretShaped(text) ||
          text.split(/\s+/).some(secretShaped) ||
          privateGoalPath.test(text)
        )
          return fail(
            res,
            400,
            "Goal contains private information",
            "Project goals are shared with every agent.",
            "Remove credentials, secrets and private paths.",
          );
        state.goals.set(p.slug, {
          text,
          source: "owner",
          updated_at: now(),
          updated_by: "human",
        });
      }
      emit("human", "project.goal", p.slug, {
        project: p.slug,
        source: body.reset === true ? "generated" : "owner",
      });
      return send(res, 200, projectGoal(p));
    }
    // project lifecycle (ADR-027): register, remove, restore; executor remove
    if (segs[0] === "projects" && segs.length === 1) {
      if ("brain" in body)
        return fail(
          res,
          400,
          "The brain is chosen in the terminal",
          "The console does not move the brain flag.",
          "Run pulse init PATH --brain.",
        );
      const raw = typeof body.path === "string" ? body.path.trim() : "";
      if (!raw)
        return fail(
          res,
          400,
          "Invalid path",
          "path must be a non-empty string.",
          "Send a JSON object with a valid path.",
        );
      if (!raw.startsWith("/") && !raw.startsWith("~"))
        return fail(
          res,
          400,
          "Not a directory: " + raw,
          "the path must be absolute",
          "pass the checkout's full path",
        );
      const path = diskPath(raw);
      const refused = checkoutRefusal(path);
      if (refused)
        return fail(res, 400, refused.error, refused.why, refused.fix);
      let p = state.projects.find((x) => x.path === path);
      const created = !p;
      if (!p) {
        const base =
          (body.name || path.split("/").filter(Boolean).pop() || "project")
            .toLowerCase()
            .replace(/[^a-z0-9]+/g, "-")
            .replace(/^-|-$/g, "") || "project";
        let slug = base;
        for (let n = 2; state.projects.some((x) => x.slug === slug); n++)
          slug = base + "-" + n;
        // Onboarding seeds the starter set (ADR-029) and the loop's records from the template (ADR-030); the daemon records the baseline itself on its next tick (ADR-031).
        const seeded = (what) => ({
          id: "EVO-" + what + "-" + slug,
          rev: 1,
          seeded: true,
          seeded_from: "amistio-template/TPL-evolution-" + what + "@r1",
          draft: null,
          draft_job: null,
        });
        p = {
          slug,
          name: body.name || slug,
          path,
          remote: "",
          brain: false,
          status: "active",
          verify_cmd: body.verify || "",
          default_branch: "main",
          added_at: now(),
          added_by: "human",
          last_seen: now(),
          present: true,
          fixtures_enabled: 6,
          baseline: false,
          evolve_level: "B",
          evolve_next:
            "none - the daemon plans the baseline job on its next tick (pulse daemon)",
          baseline_job: null,
          baseline_failed_at: "",
          constitution: seeded("constitution"),
          evaluators: seeded("evaluators"),
        };
        state.projects.push(p);
      } else {
        p.status = "active";
        p.present = true;
        p.last_seen = now();
        if (body.verify) p.verify_cmd = body.verify;
      }
      emit("human", created ? "project.add" : "project.refresh", p.slug, path);
      const job = {
        id: "JOB-" + String(state.seq++).slice(-3),
        project: p.slug,
        kind: "onboard",
        title: "Onboard " + p.slug,
        status: "queued",
        created_at: now(),
      };
      return send(res, 200, {
        project: p,
        created,
        job,
        job_queued: created,
        environment_record: "",
        store: "/home/owner/.pulse/pulse.sqlite",
      });
    }
    if (
      segs[0] === "projects" &&
      segs.length === 3 &&
      (segs[2] === "remove" || segs[2] === "restore")
    ) {
      const p = state.projects.find((x) => x.slug === segs[1]);
      if (!p)
        return fail(
          res,
          404,
          "No project '" + segs[1] + "'",
          "only a registered project can be " +
            (segs[2] === "remove" ? "removed" : "restored"),
          "pulse projects",
        );
      if (segs[2] === "restore") {
        if (p.status === "active")
          return fail(
            res,
            400,
            "Project " + p.slug + " is already active",
            "restore registers an inactive project again",
            "pulse projects",
          );
        // The real hub restores through board.Init -> RegisterProject, so a checkout gone from disk is refused the same way.
        const refused = checkoutRefusal(p.path);
        if (refused)
          return fail(res, 400, refused.error, refused.why, refused.fix);
        p.status = "active";
        p.present = true;
        p.last_seen = now();
        emit("human", "project.refresh", p.slug, p.path);
        const job = {
          id: "JOB-" + String(state.seq++).slice(-3),
          project: p.slug,
          kind: "onboard",
          title: "Onboard " + p.slug,
          status: "queued",
          created_at: now(),
        };
        return send(res, 200, {
          project: p,
          created: false,
          job,
          job_queued: true,
          environment_record: "",
          store: "/home/owner/.pulse/pulse.sqlite",
        });
      }
      if (p.status !== "active")
        return fail(
          res,
          400,
          "Project " + p.slug + " is already " + p.status,
          "remove deactivates an active project",
          "pulse init " + p.path + "  (registers it again)",
        );
      if ("dry_run" in body && typeof body.dry_run !== "boolean")
        return fail(
          res,
          400,
          "Invalid dry_run",
          "dry_run must be true or false.",
          "Send dry_run:true to preview or dry_run:false to apply.",
        );
      const dry = body.dry_run !== false;
      const queued = state.tasks
        .filter((t) => t.project === p.slug && t.job_id && t.status === "open")
        .map((t) => t.job_id);
      const executors = state.compute.executors
        .filter(
          (e) => e.projects !== "*" && e.projects.split(",").includes(p.slug),
        )
        .map((e) => e.name);
      const preview = {
        kind: "project",
        name: p.slug,
        dry_run: dry,
        applied: !dry,
        changes: [
          { label: "project " + p.slug, detail: "status active -> inactive" },
          ...(queued.length
            ? [
                {
                  label:
                    queued.length +
                    " queued job" +
                    (queued.length > 1 ? "s" : ""),
                  detail: "-> cancelled",
                },
                ...queued.map((j) => ({
                  label: "  " + j,
                  detail: "onboarding",
                })),
              ]
            : []),
          ...executors.map((e) => ({
            label: "executor " + e,
            detail: "no longer serves " + p.slug,
          })),
        ],
        stays: [
          ...(queued.length
            ? []
            : [{ label: "no queued jobs", detail: "nothing to cancel" }]),
          {
            label: "posts, runs, events",
            detail: "kept; the history is the store's",
          },
          { label: "checkout " + p.path, detail: "untouched on disk" },
        ],
        yours: [
          "rm -rf " + p.path + "  (only if you want the checkout gone)",
          "pulse init " + p.path + "  (registers it again)",
        ],
        cancelled_jobs: queued,
        leased_jobs: [],
        executors,
      };
      if (!dry) {
        p.status = "inactive";
        p.last_seen = now();
        for (const e of state.compute.executors)
          if (executors.includes(e.name))
            e.projects = e.projects
              .split(",")
              .filter((x) => x !== p.slug)
              .join(",");
        emit("human", "project.removed", p.slug, {
          path: p.path,
          cancelled: queued,
        });
      }
      return send(res, 200, preview);
    }
    // Add machine (POST compute/executors): pair runs "on the hub" for a few
    // seconds and lands its report under pairings; register adds a drained
    // row and, when asked, "probes" it; request behaves like an agent's.
    if (segs[0] === "compute" && segs[1] === "executors" && segs.length === 2) {
      state.compute.pairings ??= [];
      const mode = body.mode || "pair";
      const slug = String(body.name || "")
        .trim()
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/^-+|-+$/g, "")
        .slice(0, 40);
      const labels = String(body.labels || "")
        .split(/[\s,]+/)
        .filter(Boolean);
      if (mode === "request") {
        if (!body.project)
          return fail(
            res,
            400,
            "Invalid project",
            "project must be a non-empty string.",
            "Send a JSON object with a valid project.",
          );
        if (!body.reason)
          return fail(
            res,
            400,
            "Invalid reason",
            "reason must be a non-empty string.",
            "Send a JSON object with a valid reason.",
          );
        const r = {
          id:
            "CR-" + String(state.compute.requests.length + 1).padStart(3, "0"),
          project: body.project,
          requested_by: "human",
          size: body.size || "Standard_B2s",
          hours: Number(body.hours) || 4,
          reason: body.reason,
          status: body.approve === false ? "pending" : "approved",
          decided_by: body.approve === false ? "" : "human",
          executor: "",
          at: now(),
          updated_at: now(),
        };
        state.compute.requests.push(r);
        emit("human", "compute.requested", r.id, {
          project: r.project,
          size: r.size,
          hours: r.hours,
          status: r.status,
        });
        return send(res, 201, { request: r });
      }
      if (
        !slug ||
        slug === "local" ||
        slug === "x" ||
        !/^[a-z][a-z0-9-]*$/.test(slug)
      )
        return fail(
          res,
          400,
          "Executor name must be lowercase words with dashes and not 'local'",
          "The machine was not added.",
          "Correct the machine's details and add it again.",
        );
      if (labels.some((w) => !/^[a-z0-9][a-z0-9._:-]*$/.test(w)))
        return fail(
          res,
          400,
          "Invalid label " +
            labels.find((w) => !/^[a-z0-9][a-z0-9._:-]*$/.test(w)),
          "labels are lowercase words",
          "Correct the labels.",
        );
      const workdir = body.workdir || "/srv/pulse";
      const workers = Number(body.workers) || 1;
      const offers = ["linux", "amd64", ...labels].join(" ");
      if (mode === "register") {
        if (!body.ssh && !body.address)
          return fail(
            res,
            400,
            "The machine's ssh address is needed (user@host[:port], usually pulse@host)",
            "The machine was not added.",
            "Correct the machine's details and add it again.",
          );
        if (
          state.compute.executors.some(
            (e) => e.name === slug && e.status !== "removed",
          )
        )
          return fail(
            res,
            400,
            "Executor " + slug + " already exists",
            "The machine was not added.",
            "Pick another name.",
          );
        const e = {
          name: slug,
          kind: "ssh",
          status: "drained",
          address: body.ssh || body.address,
          workdir,
          workers,
          projects: body.projects || "*",
          last_probe_at: "",
          probe_note: "",
          offers,
          created_at: now(),
          created_by: "human",
        };
        state.compute.executors = state.compute.executors
          .filter((x) => x.name !== slug)
          .concat([e]);
        emit("human", "executor.add", slug, {
          address: e.address,
          workdir,
          workers,
        });
        let probe = null;
        if (body.probe) {
          e.status = "active";
          e.last_probe_at = now();
          e.probe_note = "ok; hands 0.1.0; git 2.45; harness yes";
          probe = {
            ok: true,
            status: "active",
            note: e.probe_note,
            line: slug + " ok, active: " + e.probe_note,
          };
          emit("human", "executor.probe", slug, { ok: true });
        }
        return send(res, 201, {
          executor: e,
          added: {
            name: slug,
            status: e.status,
            offers,
            line:
              "added " +
              slug +
              " (" +
              e.address +
              ", " +
              workdir +
              "; offers " +
              offers +
              "); it is drained until `pulse executor probe " +
              slug +
              "` passes",
          },
          probe,
        });
      }
      if (!body.admin)
        return fail(
          res,
          400,
          "The admin ssh address is needed (admin@host[:port], an account with passwordless sudo)",
          "The machine was not added.",
          "Correct the machine's details and pair it again.",
        );
      if (
        !body.dry_run &&
        state.compute.executors.some(
          (e) => e.name === slug && e.status !== "removed",
        )
      )
        return fail(
          res,
          409,
          "Executor " + slug + " already exists",
          "A machine with this name is already paired.",
          "Pick another name, or remove " + slug + " first.",
        );
      if (
        state.compute.pairings.some(
          (p) => p.name === slug && p.status === "running",
        )
      )
        return fail(
          res,
          409,
          "Already pairing " + slug,
          "A pairing with this name is still running.",
          "Wait for it to finish.",
        );
      const p = {
        name: slug,
        admin: body.admin,
        by: "human",
        started: now(),
        finished: "",
        status: "running",
        ok: false,
        dry_run: Boolean(body.dry_run),
        lines: ["pairing executor " + slug],
        changed: [],
      };
      state.compute.pairings = state.compute.pairings
        .filter((x) => x.name !== slug)
        .concat([p]);
      const steps = [
        "inspection: uname=Linux arch=x86_64 user=absent",
        "hands binary built for linux/amd64",
        "remote script ran as root on " + body.admin.replace(/^[^@]+@/, ""),
        "Host " + slug + " added to ~/.ssh/config; known_hosts updated",
      ];
      steps.forEach((line, i) =>
        setTimeout(
          () => p.status === "running" && p.lines.push(line),
          1200 * (i + 1),
        ),
      );
      setTimeout(
        () => {
          p.finished = now();
          if (p.dry_run) {
            p.ok = true;
            p.status = "ok";
            p.lines.push(
              "dry run: nothing changed; the remote and undo scripts are above",
            );
            return;
          }
          const unreachable = /unreachable|bad/.test(body.admin);
          if (unreachable) {
            p.ok = false;
            p.status = "failed";
            p.lines.push(
              "inspection failed: ssh: connect to host " +
                body.admin.replace(/^[^@]+@/, "") +
                " port 22: Connection refused",
            );
            return;
          }
          p.ok = true;
          p.status = "ok";
          p.changed = ["~/.ssh/config", "~/.ssh/known_hosts"];
          p.lines.push(
            "executor add " +
              slug +
              " --ssh pulse@" +
              slug +
              " --workdir " +
              workdir +
              " --workers " +
              workers +
              " --projects " +
              (body.projects || "*"),
            "probe " + slug + " ok, active: hands 0.1.0; git 2.45; harness yes",
          );
          if (labels.length)
            p.lines.push("labels " + labels.join(",") + ": offers " + offers);
          state.compute.executors.push({
            name: slug,
            kind: "ssh",
            status: "active",
            address: "pulse@" + slug,
            workdir,
            workers,
            projects: body.projects || "*",
            last_probe_at: now(),
            probe_note: "ok; hands 0.1.0; git 2.45; harness yes",
            offers,
            created_at: now(),
            created_by: "human",
          });
          emit("human", "executor.add", slug, {
            address: "pulse@" + slug,
            workdir,
            workers,
          });
        },
        1200 * (steps.length + 1),
      );
      return send(res, 202, { pairing: p });
    }
    if (
      segs[0] === "compute" &&
      segs[1] === "pairings" &&
      segs.length === 4 &&
      segs[3] === "dismiss"
    ) {
      state.compute.pairings ??= [];
      const p = state.compute.pairings.find((x) => x.name === segs[2]);
      if (!p)
        return fail(
          res,
          404,
          "No pairing " + segs[2],
          "No pairing with this name is listed.",
          "Refresh the machines list.",
        );
      if (p.status === "running")
        return fail(
          res,
          409,
          "Still pairing " + segs[2],
          "A running pairing cannot be dismissed.",
          "Wait for it to finish.",
        );
      state.compute.pairings = state.compute.pairings.filter((x) => x !== p);
      return send(res, 200, { dismissed: p.name });
    }
    if (
      segs[0] === "compute" &&
      segs[1] === "executors" &&
      segs.length === 4 &&
      segs[3] === "remove"
    ) {
      const name = segs[2];
      if (name === "local")
        return fail(
          res,
          400,
          "The local executor is this machine; use `pulse settings set workers N` to size it",
          "local is not a row in the store",
          "pulse settings set workers 0",
        );
      const e = state.compute.executors.find((x) => x.name === name);
      if (!e)
        return fail(
          res,
          404,
          "No executor " + name,
          "only a registered executor can be removed",
          "pulse executor list",
        );
      if (e.status === "removed")
        return fail(
          res,
          400,
          "Executor " + name + " was already removed",
          "its row stays for the runs it made",
          "pulse executor list",
        );
      const dry = body.dry_run !== false;
      const preview = {
        kind: "executor",
        name,
        dry_run: dry,
        applied: !dry,
        changes: [
          {
            label: "executor " + name,
            detail: "status " + e.status + " -> removed",
          },
        ],
        stays: [
          { label: "runs it made", detail: "kept with its name" },
          {
            label: "the machine " + e.address,
            detail: "untouched; the hub only forgets it",
          },
        ],
        yours: [
          "ssh " +
            e.address +
            " sudo userdel pulse  (only if the machine is yours to clean)",
        ],
        cancelled_jobs: [],
        leased_jobs: [],
        executors: [name],
      };
      if (!dry) {
        e.status = "removed";
        emit("human", "executor.removed", name, { address: e.address });
      }
      return send(res, 200, preview);
    }
    // Keep PULSE running (ADR-038): preview on dry_run true or absent, install on false; the mock never hands over.
    if (segs[0] === "service" && segs.length === 1) {
      if ("dry_run" in body && typeof body.dry_run !== "boolean")
        return fail(
          res,
          400,
          "Invalid dry_run",
          "dry_run must be true or false.",
          "Send dry_run:true to preview or dry_run:false to apply.",
        );
      const dry = body.dry_run !== false;
      const d = await fixture("service");
      const [daemon, hub] = d.service.units;
      const output = [
        "daemon launcher: ~/.pulse/bin/pulse-daemon.sh",
        "#!/bin/sh",
        "# pulse daemon (ADR-035): launchd runs this script; the crash guard keeps a broken build from looping.",
        "exec " + d.service.binary + " daemon --interval 60 &",
        "",
        "daemon " + d.service.kind + " service file: " + daemon.path,
        "hub launcher: ~/.pulse/bin/pulse-hub.sh",
        "exec " +
          d.service.binary +
          " hub --no-browser --port 8765 --restart-on-change &",
        "",
        "hub " + d.service.kind + " service file: " + hub.path,
      ].join("\n");
      if (!dry) {
        state.serviceInstalled = true;
        emit("human", "service.installed", d.service.kind, {
          units: ["daemon", "hub"],
        });
      }
      return send(res, 200, {
        dry_run: dry,
        installed: !dry,
        kind: d.service.kind,
        daemon_path: daemon.path,
        hub_path: hub.path,
        output,
        handover: false,
        hub_url: d.hub_url,
      });
    }
    // Record drafts (ADR-046): the owner adopts or discards what an agent drafted, or asks for the drafting job now.
    if (
      segs[0] === "records" &&
      segs.length === 3 &&
      (segs[2] === "adopt" || segs[2] === "discard" || segs[2] === "draft-job")
    ) {
      const all = (await fixture("records")).records;
      const base = all.find((r) => r.id === segs[1]);
      if (!base)
        return fail(
          res,
          404,
          "No record " + segs[1],
          "nothing by that id is indexed",
          "pulse record list",
        );
      const words = String(body.words || "").trim();
      const row = state.projects.find((x) => x.slug === base.project);
      const ref = row
        ? [row.constitution, row.evaluators].find((r) => r && r.id === base.id)
        : null;
      const detail = await fixture("records/" + base.id);
      if (segs[2] === "draft-job") {
        const seeded = row
          ? [row.constitution, row.evaluators].filter(
              (r) => r && r.seeded && !r.draft,
            )
          : [];
        if (!seeded.length)
          return fail(
            res,
            409,
            "Nothing to draft",
            "nothing to draft: " +
              (row
                ? [row.constitution, row.evaluators]
                    .filter(Boolean)
                    .map(
                      (r) =>
                        r.id.split("-")[1] +
                        (r.draft
                          ? " has a draft waiting for the owner"
                          : r.seeded
                            ? " is seeded"
                            : " is the project's own text (r" + r.rev + ")"),
                    )
                    .join("; ")
                : "no loop records yet (pulse evolve seed)"),
            "The seeded records are already drafted or adopted; refresh the page.",
          );
        const open = seeded.find((r) => r.draft_job);
        if (open)
          return send(res, 200, {
            job: open.draft_job,
            created: false,
            reason:
              "a draft job is " +
              open.draft_job.status +
              " (" +
              open.draft_job.id +
              ")",
          });
        const job = {
          id:
            "job-" +
            now().replace(/[-:]/g, "").slice(0, 15) +
            "Z-" +
            Math.random().toString(16).slice(2, 8),
          status: "queued",
        };
        for (const r of seeded) r.draft_job = job;
        emit("human", "job.add", job.id, {
          project: base.project,
          kind: "reflect",
          title:
            "Draft the " +
            seeded.map((r) => r.id.split("-")[1]).join(" and ") +
            " of " +
            base.project +
            " from its repository",
        });
        return send(res, 200, { job, created: true, reason: "" });
      }
      if (!ref || !ref.draft)
        return fail(
          res,
          400,
          "No draft to " + segs[2],
          base.id + " has no draft filed",
          "pulse record drafts",
        );
      const o = (state.records[base.id] ||= {
        status: base.status,
        revisions: [],
      });
      if (segs[2] === "adopt") {
        o.revisions.push({
          rev: base.current_rev + o.revisions.length + 1,
          author: "human",
          at: now(),
          note:
            "adopted the draft by " +
            ref.draft.author +
            (words ? ": " + words + " (via console)" : ": (via console)"),
        });
        if (detail && detail.draft && detail.draft.body)
          o.body = detail.draft.body;
        ref.rev += 1;
        ref.seeded = false;
        ref.draft = null;
        emit("human", "record.adopted", base.id, { author: "aurora", words });
        return send(res, 200, { record: recordView(base) });
      }
      ref.draft = null;
      emit("human", "record.draft.discarded", base.id, { words });
      return send(res, 200, { record: recordView(base), discarded: true });
    }
    // The human gates (ADR-028): accept, decline or defer a Proposed ADR with words.
    if (
      segs[0] === "records" &&
      segs.length === 3 &&
      (segs[2] === "accept" || segs[2] === "status")
    ) {
      const all = (await fixture("records")).records;
      const base = all.find((r) => r.id === segs[1]);
      if (!base)
        return fail(
          res,
          404,
          "No record " + segs[1],
          "nothing by that id is indexed",
          "pulse record list",
        );
      const words = String(body.words || "").trim();
      if (!words)
        return fail(
          res,
          400,
          "Your words are required",
          "A decision is recorded with the reason in the owner's words.",
          "Say why in a sentence.",
        );
      let status = "Accepted";
      if (segs[2] === "accept") {
        if (base.kind !== "adr")
          return fail(
            res,
            400,
            "Cannot accept " + base.id,
            "only ADRs use human acceptance",
            "pulse record status " + base.id + " Accepted",
          );
      } else {
        const want = String(body.status || "").toLowerCase();
        if (want !== "rejected" && want !== "deferred")
          return fail(
            res,
            400,
            "The console declines or defers",
            "Only Rejected and Deferred are set here; acceptance is its own action and every other status stays in the terminal.",
            "Use Accept, or pulse record status " + base.id + " STATUS.",
          );
        status = want === "rejected" ? "Rejected" : "Deferred";
      }
      const o = (state.records[base.id] ||= {
        status: base.status,
        revisions: [],
      });
      o.status = status;
      o.revisions.push({
        rev: base.current_rev + o.revisions.length + 1,
        author: "human",
        at: now(),
        note:
          (segs[2] === "accept" ? "" : "status: " + status + " - ") +
          words +
          " (via console)",
      });
      emit("human", "record." + status.toLowerCase(), base.id, { words });
      return send(res, 200, { record: recordView(base) });
    }
    // The owner's held-out fixtures (ADR-032): the set is created, then one
    // write per fixture. Every answer is counts; the folder never leaves here.
    if (
      segs[0] === "heldout" &&
      (segs.length === 1 || (segs.length === 2 && segs[1] === "fixtures"))
    ) {
      if (req.headers["x-pulse-heldout"] !== "yes")
        return fail(
          res,
          403,
          "missing X-Pulse-Heldout: yes",
          "An explicit confirmation header is required.",
          "Confirm the change in the hub.",
        );
      const h = state.heldout;
      if (segs.length === 1) {
        const dir = typeof body.dir === "string" ? body.dir.trim() : "";
        if (!dir)
          return fail(
            res,
            400,
            "Invalid dir",
            "dir must be a non-empty string.",
            "Send a JSON object with the folder's path.",
          );
        if (!dir.startsWith("/") && !dir.startsWith("~"))
          return heldoutRefused(
            res,
            "cannot resolve the owner-private directory",
          );
        if (secretShaped(dir))
          return heldoutRefused(
            res,
            "the directory identifier must not contain credentials",
          );
        const full = privatePath(dir);
        const clash = state.projects.find(
          (p) =>
            p.status === "active" &&
            (full === p.path ||
              full.startsWith(p.path + "/") ||
              p.path.startsWith(full + "/")),
        );
        if (clash)
          return heldoutRefused(
            res,
            "the directory must be outside every registered project",
          );
        if (
          full === HOME + "/.pulse" ||
          full === HOME + "/.pulse/fleet" ||
          full.startsWith(HOME + "/.pulse/fleet/")
        )
          return heldoutRefused(
            res,
            "the directory must not overlap the fleet area",
          );
        // Templates are written once per folder and never overwritten; a folder seen before keeps its counts.
        const set = (h.sets[full] ||= { count: 0, pending: 2, fresh: true });
        const created = set.fresh ? 4 : 0;
        set.fresh = false;
        Object.assign(h, {
          configured: true,
          present: true,
          count: set.count,
          pending: set.pending,
          generated: set.generated || 0,
          problem: "",
          dir: full,
        });
        // The store's `setting` event, masked the way the real hub masks a private setting (bus.PrivateSettings).
        emit("human", "setting", "evolve.heldout_dir", "[owner-private]");
        heldoutGates();
        return send(res, 200, {
          configured: true,
          present: true,
          count: h.count,
          pending: h.pending,
          created,
          problem: "",
        });
      }
      if (!h.configured)
        return heldoutRefused(res, "no owner-private directory is configured");
      if (!h.present)
        return heldoutRefused(
          res,
          "the owner-private directory must exist with mode 0700",
        );
      const request = typeof body.request === "string" ? body.request : "";
      const judge = typeof body.judge === "string" ? body.judge : "";
      const scorer = typeof body.scorer === "string" ? body.scorer : "";
      if (!request.trim())
        return fail(
          res,
          400,
          "Invalid request",
          "request must be a non-empty string.",
          "Write what the agent is asked.",
        );
      if ((judge === "") === (scorer === ""))
        return heldoutRefused(res, "name exactly one of judge or scorer");
      if (judge && !JUDGES.some((j) => j.name === judge))
        return heldoutRefused(res, "unknown judge " + judge);
      if (!judge && !scorer.trim())
        return heldoutRefused(res, "the scorer must not be empty");
      if (
        /PULSE_HELDOUT_TEMPLATE/.test(request) ||
        /PULSE_HELDOUT_TEMPLATE/.test(scorer)
      )
        return heldoutRefused(res, "the template marker is still in the text");
      if (
        secretShaped(request.trim()) ||
        (scorer && secretShaped(scorer.trim()))
      )
        return heldoutRefused(res, "a fixture must not contain credentials");
      if (
        "slot" in body &&
        (!Number.isInteger(body.slot) || body.slot < 0 || body.slot > 1000)
      )
        return fail(
          res,
          400,
          "Invalid slot",
          "slot must be a whole number from 1 to 1000, or 0 for the next pair.",
          "Send the fixture's position in name order, or omit slot.",
        );
      const slot = Number(body.slot || 0);
      if (h.pending > 0) [h.pending, h.count] = [h.pending - 1, h.count + 1];
      else if (slot > 0 && slot > h.count)
        return heldoutRefused(
          res,
          "slot " + slot + " is out of range (1-" + h.count + ")",
        );
      else if (slot === 0) h.count += 1;
      else if (h.generated > 0) h.generated -= 1; // rewriting a generated pair makes it the owner's (ADR-033 ledger)
      Object.assign(h.sets[h.dir], {
        count: h.count,
        pending: h.pending,
        generated: h.generated,
      });
      heldoutGates();
      return send(res, 200, {
        configured: true,
        present: true,
        count: h.count,
        pending: h.pending,
        created: 0,
        problem: "",
      });
    }
    // The daemon's push key (ADR-042): browse 1Password, test a reference against the remotes, save when they answer.
    if (segs[0] === "cred" && segs.length >= 2) {
      if (req.headers["x-pulse-cred"] !== "yes")
        return fail(
          res,
          403,
          "missing X-Pulse-Cred: yes",
          "An explicit confirmation header is required.",
          "Confirm the change in the hub.",
        );
      const c = state.cred;
      const sub = segs.slice(1).join("/");
      const report = () => ({
        reference: c.reference,
        scheme: c.reference.split(":")[0] || undefined,
        landing_push:
          state.settings.settings.find((s) => s.name === "landing.push")
            ?.value || "off",
        token: c.token,
        set_at: c.set_at || undefined,
        set_by: c.set_by || undefined,
        fingerprint: c.fingerprint || undefined,
      });
      const flag = (key, dflt) =>
        key in body
          ? typeof body[key] === "boolean"
            ? body[key]
            : null
          : dflt;
      const publicKey =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMockPublicKeyForTheDemoHubOnlyNotARealKey00 pulse push key (owner-mac)";
      const fingerprint =
        "SHA256:MockFingerprintOfTheDemoKey0123456789abcdefghijk";
      if (sub === "token") {
        const token = typeof body.token === "string" ? body.token.trim() : "";
        if (!token)
          return fail(
            res,
            400,
            "Invalid token",
            "token must be a non-empty string.",
            "Send a JSON object with a valid token.",
          );
        if (!/^[A-Za-z0-9_.=-]{20,}$/.test(token))
          return fail(
            res,
            400,
            "Token not stored",
            "that does not look like a 1Password service-account token (one line, letters, digits, _ . = -)",
            "Copy the token from 1Password > Developer > Service Accounts and paste it on one line.",
          );
        c.token = "the keychain";
        return send(res, 200, {
          stored_in:
            "the keychain (service pulse, account OP_SERVICE_ACCOUNT_TOKEN)",
          report: report(),
        });
      }
      if (sub === "op/vaults") {
        if (c.token === "none")
          return fail(
            res,
            409,
            "1Password token not found",
            "no service-account token in OP_SERVICE_ACCOUNT_TOKEN, the keychain or the token file",
            "Add a service-account token first (pulse cred token or the Connect step), and check op is installed.",
          );
        return send(res, 200, {
          vaults: [
            { id: "v-pulse", name: "PULSE" },
            { id: "v-personal", name: "Personal" },
            { id: "v-work", name: "Work" },
          ],
        });
      }
      if (sub === "op/items") {
        const vault = typeof body.vault === "string" ? body.vault.trim() : "";
        if (!vault)
          return fail(
            res,
            400,
            "Invalid vault",
            "vault must be a non-empty string.",
            "Send a JSON object with a valid vault.",
          );
        if (c.token === "none")
          return fail(
            res,
            409,
            "1Password token not found",
            "no service-account token",
            "Add a service-account token first.",
          );
        const items =
          vault === "v-pulse"
            ? [
                {
                  id: "i-push",
                  title: "pulse push key",
                  category: "SSH_KEY",
                  updated_at: "2026-09-14T10:12:00Z",
                  reference:
                    "op://PULSE/pulse push key/private key?ssh-format=openssh",
                },
                {
                  id: "i-old",
                  title: "old deploy key (2024)",
                  category: "SSH_KEY",
                  updated_at: "2024-03-02T08:00:00Z",
                  reference:
                    "op://PULSE/old deploy key (2024)/private key?ssh-format=openssh",
                },
              ]
            : vault === "v-work"
              ? [
                  {
                    id: "i-ci",
                    title: "ci-runner",
                    category: "SSH_KEY",
                    updated_at: "2026-01-20T16:40:00Z",
                    reference:
                      "op://Work/ci-runner/private key?ssh-format=openssh",
                  },
                ]
              : [];
        return send(res, 200, { vault, items });
      }
      if (sub === "set") {
        const reference =
          typeof body.reference === "string" ? body.reference.trim() : "";
        if (!reference)
          return fail(
            res,
            400,
            "Invalid reference",
            "reference must be a non-empty string.",
            "Send a JSON object with a valid reference.",
          );
        const probe = flag("probe", true);
        if (probe === null)
          return fail(
            res,
            400,
            "Invalid probe",
            "probe must be true or false.",
            "Send probe:true or probe:false.",
          );
        if (
          /BEGIN [A-Z ]*PRIVATE KEY/.test(reference) ||
          secretShaped(reference)
        )
          return fail(
            res,
            400,
            "That is a key, not a reference",
            "pulse cred set takes a reference to where the key lives, never the key itself.",
            "Put the key in 1Password, the keychain or a file and pass op://, keychain:// or file://.",
          );
        const m = /^(op|keychain|file):\/\/(.+)$/.exec(reference);
        if (!m)
          return fail(
            res,
            400,
            JSON.stringify(reference) + " is not a reference",
            "use op://VAULT/ITEM/FIELD[?ssh-format=openssh], keychain://SERVICE/ACCOUNT, or file:///ABSOLUTE/PATH",
            "Pick a key from a provider or type a reference in one of those shapes.",
          );
        if (m[1] === "op" && c.token === "none")
          return fail(
            res,
            409,
            "1Password token not found",
            "the reference is op:// but no service-account token is stored",
            "Connect 1Password first.",
          );
        if (m[1] === "keychain" && (await fixture("cred")).os !== "darwin")
          return fail(
            res,
            409,
            "keychain:// needs macOS",
            "the keychain is a macOS store",
            "Use 1Password or a key file here.",
          );
        const probes = state.projects
          .filter((p) => p.status === "active")
          .map((p) => ({
            project: p.slug,
            branch: "main",
            remote: "git@github.com:owner/" + p.slug + ".git",
            ok: !/old|2024/.test(reference),
          }));
        if (probe && probes.some((p) => !p.ok)) {
          return send(res, 409, {
            error: "the remote refused this key",
            why: probes
              .filter((p) => !p.ok)
              .map(
                (p) =>
                  p.project +
                  ": ERROR: Permission to owner/" +
                  p.project +
                  ".git denied to deploy key",
              )
              .join("; "),
            fix: "Add the public key below as a deploy key with write access on every remote, then try again.",
            public_key: publicKey,
            fingerprint,
            probes: probes.map((p) =>
              p.ok
                ? p
                : {
                    ...p,
                    why:
                      "ERROR: Permission to owner/" +
                      p.project +
                      ".git denied to deploy key",
                  },
            ),
          });
        }
        Object.assign(c, {
          reference,
          fingerprint,
          set_at: now(),
          set_by: "human:dashboard",
        });
        emit("human:dashboard", "cred.set", "push.credential", {
          scheme: m[1],
          fingerprint,
        });
        const row = state.settings.settings.find(
          (s) => s.name === "push.credential",
        );
        if (row)
          Object.assign(row, {
            value: reference,
            updated_by: "human:dashboard",
            updated_at: now(),
          });
        return send(res, 200, {
          result: {
            reference,
            fingerprint,
            public_key: publicKey,
            source:
              m[1] === "op"
                ? "1Password (token from the keychain)"
                : m[1] === "keychain"
                  ? "the keychain"
                  : reference.slice(7),
            probed: probe,
            probes: probe
              ? probes
              : probes.map((p) => ({ ...p, ok: false, skipped: "probe off" })),
            landing_push: report().landing_push,
          },
          report: report(),
        });
      }
      if (sub === "clear") {
        const token = flag("token", false);
        if (token === null)
          return fail(
            res,
            400,
            "Invalid token",
            "token must be true or false.",
            "Send token:true or token:false.",
          );
        const was = c.reference;
        const token_removed =
          token && c.token !== "none" ? ["the keychain item"] : [];
        Object.assign(
          c,
          { reference: "", fingerprint: "", set_at: "", set_by: "" },
          token ? { token: "none" } : {},
        );
        const row = state.settings.settings.find(
          (s) => s.name === "push.credential",
        );
        if (row)
          Object.assign(row, {
            value: "",
            updated_by: "default",
            updated_at: "",
          });
        if (was)
          emit("human:dashboard", "cred.clear", "push.credential", {
            was_scheme: was.split(":")[0],
          });
        return send(res, 200, {
          cleared: { was, token_removed },
          report: report(),
        });
      }
      if (sub === "generate") {
        const replace = flag("replace", false);
        if (replace === null)
          return fail(
            res,
            400,
            "Invalid replace",
            "replace must be true or false.",
            "Send replace:true or replace:false.",
          );
        const existed = c.generated && !replace;
        c.generated = true;
        return send(res, 200, {
          generated: {
            path: HOME + "/.pulse/secrets/push_key",
            reference: "file://" + HOME + "/.pulse/secrets/push_key",
            fingerprint,
            public_key: publicKey,
            existed,
          },
        });
      }
      return fail(res, 404, "not found", "No such API route.", "Use /api/v1/.");
    }
    return fail(
      res,
      405,
      "no such write",
      req.method + " " + url.pathname + " is not an owner action",
      "the writes are tasks, tasks/<id>/amend|cancel|status, messages, decide, settings, compute/<id>/approve|deny, projects, projects/<slug>/goal|needs|remove|restore, compute/executors/<name>/remove, records/<id>/accept|status|adopt|discard|draft-job, heldout, heldout/fixtures, cred/token|op/vaults|op/items|set|clear|generate, mcp, mcp/<id> (PATCH), mcp/<id>/connect|status|login|logout, skills, skills/<id> (PATCH), skills/<id>/status, automations, automations/check|remove, automations/<id>/pause|resume|end|run, chat/<id>/auto|lockdown, memory, memory/forget, voice/setup|transcribe|unload, voice/model (DELETE)",
    );
  }

  // GET - live state first, fixtures second
  if (segs[0] === "jobs" && segs[1] === "failed") {
    const result = state.jobRepairs.read(q);
    return send(res, result.status, result.body);
  }
  if (segs[0] === "code-deliveries" && segs.length === 1) {
    const result = state.codeDeliveries.read(q);
    return send(res, result.status, result.body);
  }
  if (segs[0] === "reports" && segs.length === 1) {
    const result = state.reports.read(q);
    return send(res, result.status, result.body);
  }
  if (segs[0] === "kpis" && segs.length === 1) {
    const result = state.kpis.read(q);
    return send(res, result.status, result.body);
  }
  if (segs[0] === "projects" && segs.length === 3 && segs[2] === "goal") {
    const p = state.projects.find((x) => x.slug === segs[1]);
    if (!p)
      return fail(
        res,
        404,
        "No project " + segs[1],
        "Goals belong to registered projects.",
        "Choose a project.",
      );
    return send(res, 200, projectGoal(p));
  }
  if (segs[0] === "projects" && segs.length === 3 && segs[2] === "needs") {
    const p = state.projects.find((x) => x.slug === segs[1]);
    if (!p)
      return fail(
        res,
        404,
        "No project " + segs[1],
        "only a registered project has needs",
        "pulse projects",
      );
    return send(res, 200, await projectNeeds(p));
  }
  if (segs[0] === "harness" && segs.length === 1) {
    const failed = new Set([
      "step_limit",
      "time_limit",
      "format_error",
      "provider_error",
      "not_logged_in",
      "cancelled",
      "lost",
    ]);
    state.harness.sessions = {
      running: state.harnessSessions.filter((s) => s.status === "running")
        .length,
      today: state.harnessSessions.length,
      failed_today: state.harnessSessions.filter((s) => failed.has(s.status))
        .length,
      submitted_today: state.harnessSessions.filter(
        (s) => s.status === "submitted",
      ).length,
      cost_today: state.harnessSessions.reduce(
        (sum, s) => sum + (Number(s.cost) || 0),
        0,
      ),
    };
    state.harness.providers = MOCK_PROVIDERS;
    state.harness.provider = providerSummary(state.harness.provider.name) ?? state.harness.provider;
    return send(res, 200, state.harness);
  }
  if (segs[0] === "harness" && segs[1] === "providers" && segs.length === 3) {
    const row = providerSummary(segs[2]);
    if (!row) return fail(res, 404, "unknown provider", "No provider is called " + segs[2] + ".", "Pick one from GET harness: providers.");
    return send(res, 200, row);
  }
  if (segs[0] === "harness" && segs[1] === "prompt") {
    const setting = state.settings.settings.find(
      (s) => s.name === "harness.system_prompt",
    );
    const template = setting?.value || "";
    return send(res, 200, {
      ...state.harnessPrompt,
      template,
      custom: template !== "",
      rendered: template || state.harnessPrompt.rendered,
    });
  }
  if (segs[0] === "harness" && segs[1] === "models") {
    const name = q.get("provider") || state.harness.provider.name;
    const label = MOCK_PROVIDERS.find((x) => x.name === name)?.label ?? name;
    if (!harnessLoggedIn(name))
      return fail(res, 503, "not signed in", "The " + label + " login is missing on this machine.", "Sign in to " + label + ".");
    return send(res, 200, { models: mockModels(name), fetched_at: now() });
  }
  if (segs[0] === "harness" && segs[1] === "sessions" && segs.length === 2) {
    const p = q.get("project");
    const st = q.get("status");
    const agent = (q.get("agent") || "").toLowerCase();
    const job = (q.get("job") || "").toLowerCase();
    const limit = Number(q.get("limit") || 50);
    const before = q.get("before") || "";
    const failed = new Set([
      "step_limit",
      "time_limit",
      "format_error",
      "provider_error",
      "not_logged_in",
      "cancelled",
      "lost",
    ]);
    const filtered = state.harnessSessions
      .filter(
        (s) =>
          (!p || s.project === p) &&
          (!st || s.status === st) &&
          (!agent || s.agent.toLowerCase().includes(agent)) &&
          (!job ||
            s.job_id.toLowerCase().includes(job) ||
            s.job_title.toLowerCase().includes(job)),
      )
      .sort((a, b) =>
        a.started_at === b.started_at
          ? a.id < b.id
            ? 1
            : -1
          : a.started_at < b.started_at
            ? 1
            : -1,
      );
    // The real hub pages on a (started_at, id) cursor written as "started_at|id"; a bare timestamp still works.
    const cut = before.lastIndexOf("|");
    const beforeAt = cut >= 0 ? before.slice(0, cut) : before;
    const beforeId = cut >= 0 ? before.slice(cut + 1) : "";
    const windowed = before
      ? filtered.filter(
          (s) =>
            s.started_at < beforeAt ||
            (beforeId && s.started_at === beforeAt && s.id < beforeId),
        )
      : filtered;
    const sessions = windowed.slice(0, limit);
    const counts = {
      running: filtered.filter((s) => s.status === "running").length,
      today: filtered.length,
      failed_today: filtered.filter((s) => failed.has(s.status)).length,
      submitted_today: filtered.filter((s) => s.status === "submitted").length,
      cost_today: filtered.reduce((sum, s) => sum + (Number(s.cost) || 0), 0),
    };
    const last = sessions[sessions.length - 1];
    const next_before =
      sessions.length === limit && windowed.length > limit
        ? `${last.started_at}|${last.id}`
        : undefined;
    return send(res, 200, {
      sessions,
      counts,
      ...(next_before ? { next_before } : {}),
    });
  }
  if (
    segs[0] === "harness" &&
    segs[1] === "sessions" &&
    segs[3] === "trajectory"
  ) {
    const row = state.harnessSessions.find((s) => s.id === segs[2]);
    if (!row?.trajectory)
      return fail(
        res,
        404,
        "no trajectory",
        segs[2] + " did not leave a trajectory file",
        "copy the transcript instead",
      );
    return send(
      res,
      200,
      { session: row, steps: state.harnessDetails[row.id]?.steps || [] },
      {
        "content-disposition": `attachment; filename="${row.id}-trajectory.json"`,
      },
    );
  }
  if (segs[0] === "harness" && segs[1] === "sessions" && segs[3] === "file") {
    // One file of a run's workspace, for the canvas: the hub reads it from
    // the session's cwd; here a folder of fixtures stands in for it (each
    // file kept as PATH.txt so the build does not compile it), and a run
    // without one is a workspace that is gone.
    const rel = q.get("path") || "";
    if (!rel)
      return fail(
        res,
        400,
        "path is required",
        "the query names no file",
        "add ?path=REL",
      );
    const dir = normalize(join(root, "harness", "files", segs[2]));
    const file = normalize(join(dir, rel));
    if (
      !file.startsWith(dir + "/") ||
      rel.startsWith("/") ||
      rel.split("/").includes("..")
    )
      return fail(
        res,
        404,
        "No such file",
        rel + " is not inside the workspace",
        "ask for a path the agent touched",
      );
    let text;
    try {
      await stat(dir);
    } catch {
      return fail(
        res,
        404,
        "Workspace gone",
        "the run's workspace is no longer on this machine",
        "read the diff instead",
      );
    }
    try {
      text = await readFile(file + ".txt", "utf8");
    } catch {
      return fail(
        res,
        404,
        "No such file",
        rel + " is not in the workspace",
        "ask for a path the agent touched",
      );
    }
    return send(res, 200, {
      session: segs[2],
      path: rel,
      size: Buffer.byteLength(text),
      text,
      truncated: false,
      binary: false,
    });
  }
  if (segs[0] === "harness" && segs[1] === "sessions" && segs.length === 3) {
    const d = state.harnessDetails[segs[2]];
    if (!d)
      return fail(
        res,
        404,
        "no such harness session",
        segs[2] + " is not in the mock store",
        "open it from the Harness page",
      );
    const row =
      state.harnessSessions.find((s) => s.id === segs[2]) || d.session;
    const after = q.has("after") ? Number(q.get("after")) : null;
    const steps = after === null ? d.steps : d.steps.filter((s) => s.n > after);
    return send(res, 200, {
      session: row,
      steps,
      ...(after === null ? {} : { after }),
    });
  }
  if (segs[0] === "roster" && segs.length === 1) {
    if (!agentReturned && !agentReturnStarted) {
      agentReturnStarted = true;
      setTimeout(() => {
        agentReturned = true;
        emit("delta", "agent.resume", "delta", {
          session: "run-delta-current",
        });
      }, agentReturnMS);
    }
    const task002 = state.tasks.find((t) => t.id === "TASK-002")?._detail;
    return send(
      res,
      200,
      rosterScenarios(
        await loadJson("roster.json"),
        now(),
        agentReturned,
        taskWorkFocus(task002, now()),
      ),
    );
  }
  if (segs[0] === "heldout" && segs.length === 1)
    return send(res, 200, heldoutView(state.heldout));
  if (segs[0] === "cred" && segs.length === 1) {
    const d = await fixture("cred");
    const c = state.cred;
    return send(res, 200, {
      ...d,
      report: {
        ...d.report,
        reference: c.reference,
        scheme: c.reference.split(":")[0] || undefined,
        token: c.token,
        set_at: c.set_at || undefined,
        set_by: c.set_by || undefined,
        fingerprint: c.fingerprint || undefined,
        landing_push:
          state.settings.settings.find((s) => s.name === "landing.push")
            ?.value || "off",
      },
    });
  }
  if (segs[0] === "projects" && segs.length === 3 && segs[2] === "evolution") {
    const d = await fixture("projects/" + segs[1] + "/evolution");
    if (d === null)
      return fail(
        res,
        404,
        "not found",
        "no fixture for " + url.pathname,
        "add mock/api/" + segs.join("/") + ".json",
      );
    return send(res, 200, evolutionView(segs[1], d));
  }
  // Role leads (`pulse lead`): the seats on a project, what PULSE suggests
  // from the project's own signals, and the built-in roles for the gallery.
  if (segs[0] === "leads" && segs.length === 1) {
    const p = q.get("project") || "demo-app";
    const leads = state.leads.leads.filter((l) => l.project === p);
    const live = new Set(
      leads.filter((l) => l.status !== "ended").map((l) => l.role),
    );
    const suggestions = (state.leads.suggestions[p] || []).filter(
      (g) => !live.has(g.role),
    );
    return send(res, 200, {
      project: p,
      leads,
      suggestions,
      roles: state.leads.roles,
    });
  }
  // Automations: every project's standing jobs and the file that keeps them.
  if (segs[0] === "automations" && segs.length === 1) {
    const r = state.automations.list(q);
    return send(res, r.status, r.body);
  }
  // One automation's history: its runs, jobs and changes, newest first.
  if (segs[0] === "automations" && segs.length === 3 && segs[2] === "history") {
    const r = state.automations.history(segs[1], q);
    return send(res, r.status, r.body);
  }
  // The toolkit (`pulse toolkit`): what each checkout asks for and lacks.
  if (segs[0] === "toolkit" && segs.length === 1) {
    const p = q.get("project") || "";
    const have = new Set([
      ...state.mcp.map((s) => "tool:" + s.name),
      ...state.skills.map((k) => "skill:" + k.name),
    ]);
    const trim = (g) => ({
      project: g.project,
      tools: g.tools.filter((x) => !have.has("tool:" + x.name)),
      skills: g.skills.filter((x) => !have.has("skill:" + x.name)),
    });
    if (p) {
      const g = state.toolkit.projects.find((x) => x.project === p);
      return send(res, 200, {
        project: p,
        ...trim(g || { project: p, tools: [], skills: [] }),
        catalog: state.toolkit.catalog,
      });
    }
    return send(res, 200, {
      project: "",
      projects: state.toolkit.projects.map(trim),
      catalog: state.toolkit.catalog,
    });
  }
  if (segs[0] === "tasks" && segs[1] === "suggested") {
    const r =
      segs.length === 3
        ? state.suggestions.show(segs[2])
        : state.suggestions.list(q);
    return send(res, r.status, r.body);
  }
  if (segs[0] === "tasks" && segs.length === 1) {
    const p = q.get("project");
    const st = q.get("status");
    const details = Object.fromEntries(
      state.tasks.map((t) => [t.id, t._detail]),
    );
    const tasks = taskScenarios(
      {
        tasks: state.tasks.filter(
          (t) => (!p || t.project === p) && (!st || t.status === st),
        ),
      },
      now(),
      details,
    ).tasks;
    return send(res, 200, { tasks: tasks.map(strip) });
  }
  if (segs[0] === "tasks" && segs.length === 2) {
    const t = state.tasks.find((x) => x.id === segs[1]);
    if (!t)
      return fail(
        res,
        404,
        "no such task",
        segs[1] + " is not in the store",
        "open it from the Tasks page",
      );
    const detail = taskDetailScenario(t._detail, now());
    return send(res, 200, {
      ...detail,
      // Each job says how many files its artifacts folder holds, so the
      // task view can offer them as evidence.
      jobs: await jobArtifacts.decorate(detail.jobs),
      task: strip(detail.task),
      messages: state.messages.filter((m) => m.task_id === t.id),
    });
  }
  if (segs[0] === "tasks" && segs.length === 3 && segs[2] === "activity") {
    const t = state.tasks.find((x) => x.id === segs[1]);
    if (!t)
      return fail(
        res,
        404,
        "no such task",
        segs[1] + " is not in the store",
        "open it from the Tasks page",
      );
    return send(
      res,
      200,
      taskActivity(
        taskDetailScenario(t._detail, now()),
        state.messages.filter((m) => m.task_id === t.id),
      ),
    );
  }
  if (segs[0] === "messages") {
    const p = q.get("project");
    const t = q.get("task");
    const since = q.get("since");
    return send(res, 200, {
      messages: state.messages.filter(
        (m) =>
          (!p || m.project === p) &&
          (!t || m.task_id === t) &&
          (!since || m.at > since),
      ),
    });
  }
  if (segs[0] === "compute" && segs.length === 1)
    return send(res, 200, state.compute);
  if (segs[0] === "projects" && segs.length === 1)
    return send(res, 200, {
      projects: state.projects.map((p) => ({ ...p, ...goalFields(p) })),
      brain: state.projects.find((p) => p.brain)?.slug || "",
    });
  if (segs[0] === "folders" && segs.length === 1) {
    const f = folders(q.get("path") || "");
    if (f.error)
      return fail(
        res,
        404,
        f.error,
        "nothing exists at that path in the mock tree",
        "go up one level",
      );
    return send(res, 200, f);
  }
  if (segs[0] === "settings" && segs.length === 1)
    return send(res, 200, {
      ...state.settings,
      settings: state.settings.settings.map(maskSetting),
    });
  if (segs[0] === "proposals" && segs.length === 1)
    return send(res, 200, state.proposals);
  if (segs[0] === "questions" && segs.length === 1)
    return send(res, 200, state.questions);
  if (segs[0] === "projects" && segs.length === 3 && segs[2] === "questions") {
    const slug = decodeURIComponent(segs[1]);
    if (!state.projects.some((x) => x.slug === slug) && slug !== "commons")
      return fail(
        res,
        404,
        "no such project",
        slug + " is not registered",
        "add it from the hub",
      );
    return send(
      res,
      200,
      state.questions
        .filter((x) => x.slug === slug)
        .map((x) =>
          Object.fromEntries(
            Object.entries(x).filter(([k]) => k !== "slug" && k !== "base"),
          ),
        ),
    );
  }
  if (segs[0] === "org") {
    const org = await fixture("org");
    // The real hub lists active projects only; mirror removals made in memory.
    // The self-improvement gates (ADR-029) and record refs (ADR-030) travel on GET /projects in the real hub, so the org rows drop them here
    // even though org.json seeds them: the console must merge the registry row like it does in production.
    const GATE_FIELDS = [
      "fixtures_enabled",
      "baseline",
      "evolve_level",
      "evolve_next",
      "constitution",
      "evaluators",
      "baseline_job",
      "baseline_failed_at",
    ];
    const projects = org.projects
      .filter(
        (p) =>
          p.kind === "commons" ||
          state.projects.some(
            (x) => x.slug === p.slug && x.status === "active",
          ),
      )
      .map((p) =>
        Object.fromEntries(
          Object.entries(p).filter(([k]) => !GATE_FIELDS.includes(k)),
        ),
      )
      .map((p) =>
        p.kind === "commons"
          ? p
          : {
              ...p,
              owner_questions: state.questions.filter((x) => x.slug === p.slug)
                .length,
              needs: state.projects.find((x) => x.slug === p.slug)?.needs || "",
              ...goalFields(state.projects.find((x) => x.slug === p.slug)),
            },
      );
    return send(res, 200, {
      ...org,
      projects,
      proposals: state.proposals,
      questions: state.questions,
      totals: { ...org.totals, owner_questions: state.questions.length },
      generated_at: now(),
    });
  }
  if (segs[0] === "events" && segs.length === 1) {
    const p = q.get("project");
    const since = Number(q.get("since") || 0);
    const limit = Number(q.get("limit") || 50);
    // Like the hub: the newest `limit` events after `since`, oldest first.
    const events = state.events
      .filter(
        (e) => e.id > since && (!p || (e.detail && e.detail.project === p)),
      )
      .sort((a, b) => b.id - a.id)
      .slice(0, limit);
    return send(res, 200, { events: events.reverse() });
  }
  if (segs[0] === "records" && segs.length === 1) {
    const all = (await fixture("records")).records.map(recordView);
    const p = q.get("project");
    const k = q.get("kind");
    const st = q.get("status");
    return send(res, 200, {
      records: all.filter(
        (r) =>
          (!p || r.project === p) &&
          (!k || r.kind === k) &&
          (!st || r.status === st),
      ),
    });
  }
  if (segs[0] === "records" && segs.length === 2) {
    const d = await fixture("records/" + segs[1]);
    if (!d)
      return fail(
        res,
        404,
        "no such record",
        segs[1] + " is not indexed",
        "run pulse init in the project checkout",
      );
    const o = state.records[segs[1]];
    const row = state.projects.find((x) => x.slug === d.record.project);
    const ref = row
      ? [row.constitution, row.evaluators].find(
          (r) => r && r.id === d.record.id,
        )
      : null;
    const view = o
      ? {
          ...d,
          record: recordView(d.record),
          frontmatter: { ...d.frontmatter, status: o.status },
          revisions: [...(d.revisions || []), ...o.revisions],
          ...(o.body ? { body: o.body } : {}),
        }
      : { ...d };
    if (ref && "draft" in view)
      view.draft = ref.draft
        ? { ...ref.draft, body: (d.draft && d.draft.body) || "" }
        : null;
    const rev = q.get("rev");
    if (rev && Number(rev) !== view.record.current_rev)
      return send(res, 200, {
        ...view,
        body:
          `_(revision r${rev} of ${view.record.id} - the mock keeps only the current body)_\n\n` +
          view.body,
      });
    return send(res, 200, view);
  }
  // Keep PULSE running (ADR-038): the fixture says not installed; a POST with dry_run:false flips it.
  if (segs[0] === "service" && segs.length === 1) {
    const d = await fixture("service");
    if (!state.serviceInstalled) return send(res, 200, d);
    return send(res, 200, {
      ...d,
      service: {
        ...d.service,
        installed: true,
        units: d.service.units.map((u) => ({
          ...u,
          present: true,
          loaded: true,
        })),
      },
    });
  }
  if (segs[0] === "constitution") {
    const d = await fixture("constitution");
    const p = q.get("project");
    if (p && p !== d?.record?.project)
      return send(res, 200, { record: null, body: "", amendments: [] });
    return send(res, 200, d);
  }
  if (segs[0] === "mcp") {
    if (segs.length === 1)
      return send(res, 200, {
        servers: scoped(state.mcp, q.get("project") || ""),
      });
    if (segs.length === 2 && segs[1] === "options")
      return send(res, 200, state.mcpOptions);
    if (segs.length === 2) {
      const server = state.mcp.find(
        (x) => x.id === segs[1] || x.name === segs[1],
      );
      return server
        ? send(res, 200, { server })
        : fail(
            res,
            404,
            "No MCP server " + segs[1],
            "Nothing registered under that id or name.",
            "pulse mcp list shows what there is.",
          );
    }
    if (segs.length === 3 && segs[2] === "login") {
      const server = state.mcp.find(
        (x) => x.id === segs[1] || x.name === segs[1],
      );
      if (!server)
        return fail(
          res,
          404,
          "No MCP server " + segs[1],
          "Nothing registered under that id or name.",
          "pulse mcp list shows what there is.",
        );
      if (server.auth !== "oauth")
        return fail(
          res,
          400,
          server.name + " does not sign in with OAuth",
          "its auth is " + server.auth,
          "pulse mcp update " + server.name + " --auth oauth first",
        );
      const job = state.mcpLogins.get(server.id);
      return send(
        res,
        200,
        job
          ? mcpLoginJson(server, job)
          : { state: "none", sign_in: server.sign_in },
      );
    }
    return fail(res, 404, "not found", "No such API route.", "Use /api/v1/.");
  }
  if (segs[0] === "memory" && segs.length === 1) {
    return send(res, 200, memoryView());
  }
  if (segs[0] === "secrets" && segs.length === 1) {
    return send(res, 200, { secrets: state.secrets, max_bytes: 65536 });
  }
  if (segs[0] === "skills") {
    if (segs.length === 1)
      return send(res, 200, {
        skills: scoped(state.skills, q.get("project") || ""),
      });
    if (segs.length === 2) {
      const skill = state.skills.find(
        (x) => x.id === segs[1] || x.name === segs[1],
      );
      return skill
        ? send(res, 200, { skill })
        : fail(
            res,
            404,
            "No skill " + segs[1],
            "Nothing registered under that id or name.",
            "pulse skill list shows what there is.",
          );
    }
    return fail(res, 404, "not found", "No such API route.", "Use /api/v1/.");
  }
  // A conversation's artifacts and attachments are real files next to its
  // fixtures, so the viewer and the image thumbnails can be exercised
  // offline: mock/api/chat/<id>/(artifacts|attachments)/<name>.
  if (
    segs[0] === "chat" &&
    segs.length === 4 &&
    (segs[2] === "artifacts" || segs[2] === "attachments")
  ) {
    const file = normalize(join(root, "chat", segs[1], segs[2], segs[3]));
    if (file.startsWith(root)) {
      try {
        const st = await stat(file);
        if (st.isFile()) {
          const ext = file.split(".").pop();
          res.writeHead(200, {
            "content-type":
              MIME[ext] ??
              (ext === "md"
                ? "text/markdown; charset=utf-8"
                : "application/octet-stream"),
            "cache-control": "no-store",
            "access-control-allow-origin": "*",
          });
          return res.end(await readFile(file));
        }
      } catch {
        // fall through to the fixture lookup and its 404
      }
    }
  }
  const d = await fixture(segs.map(encodeURIComponent).join("/"));
  if (d === null)
    return fail(
      res,
      404,
      "not found",
      "no fixture for " + url.pathname,
      "add mock/api/" + segs.join("/") + ".json",
    );
  if (segs[0] === "chat" && segs.length === 1 && Array.isArray(d.conversations))
    return send(res, 200, {
      ...d,
      conversations: d.conversations.map(chatWithFlips),
    });
  if (segs[0] === "chat" && segs.length === 2 && d.conversation) {
    const flips = state.chatFlips[d.conversation.id];
    return send(res, 200, {
      ...d,
      conversation: chatWithFlips(d.conversation),
      live: flips && d.live ? { ...d.live, ...flips } : d.live,
      children: Array.isArray(d.children)
        ? d.children.map(chatWithFlips)
        : d.children,
    });
  }
  return send(res, 200, d);
}

// A conversation row with the switches the owner flipped in this mock run
// laid over it, on the row and on its live state.
function chatWithFlips(c) {
  const f = c && state.chatFlips[c.id];
  if (!f) return c;
  return {
    ...c,
    ...f,
    live: c.live
      ? { ...c.live, ...f, version: (c.live.version || 0) + 1 }
      : c.live,
  };
}

const strip = (t) =>
  Object.fromEntries(Object.entries(t).filter(([k]) => !k.startsWith("_")));

// ----- MCP servers and skills (TASK-022) --------------------------------
// The mock keeps the hub's rules: slugs, one endpoint per transport, an
// auth that fits the transport, a pulse-cred reference whenever auth is not
// none, and never a secret in the store. Connect is canned per row.

const SLUG = /^[a-z0-9][a-z0-9-]*$/;
const CRED_REF =
  /^(op:\/\/[^/]+\/[^/]+\/.+|keychain:\/\/[^/]+\/.+|file:\/\/\/.+)$/;
const ENV_KEY = /^[A-Za-z_][A-Za-z0-9_]*$/;
const scoped = (rows, project) =>
  rows.filter((r) => !project || r.project === "" || r.project === project);
const str = (v) => (typeof v === "string" ? v.trim() : "");

function mcpValidate(row) {
  const auth = state.mcpOptions.auths.find((a) => a.name === row.auth);
  if (!SLUG.test(row.name) || row.name.length > 64)
    return [
      "Invalid name " + row.name,
      "a name is lowercase letters, digits and dashes, 64 characters at most",
      "pick a slug like github or feedback-api",
    ];
  if (!state.mcpOptions.transports.includes(row.transport))
    return [
      "Invalid transport " + row.transport,
      "transport is one of stdio, http, sse",
      "choose a transport",
    ];
  if (row.transport === "stdio" && !row.command)
    return [
      "Invalid command",
      "a stdio server needs the command that starts it",
      "set command",
    ];
  if (row.transport !== "stdio" && !/^https?:\/\/\S+$/.test(row.url))
    return [
      "Invalid url",
      "an " + row.transport + " server needs an http(s) url",
      "set url",
    ];
  if (!auth)
    return [
      "Invalid auth " + row.auth,
      "auth is one of " + state.mcpOptions.auths.map((a) => a.name).join(", "),
      "choose an auth",
    ];
  if (!auth.transports.includes(row.transport))
    return [
      "Invalid auth " + row.auth,
      "auth " +
        row.auth +
        " applies to " +
        auth.transports.join(", ") +
        " servers",
      "choose an auth that fits " + row.transport,
    ];
  if (row.auth === "oauth" && row.secret_ref)
    return [
      "Invalid secret",
      "auth oauth signs in by itself (pulse mcp login " +
        row.name +
        ") and keeps its own tokens; a client registered with the provider by hand goes on the login",
      "drop the secret; sign in from the card after saving",
    ];
  if (!["none", "oauth"].includes(row.auth) && !CRED_REF.test(row.secret_ref))
    return [
      "Invalid secret",
      "auth " +
        row.auth +
        " needs a credential reference: " +
        state.mcpOptions.cred_shapes,
      "pass --secret REF; never the secret itself",
    ];
  if (["header", "query", "env"].includes(row.auth) && !row.auth_header)
    return [
      "Invalid auth header",
      "auth " + row.auth + " needs the name to send the secret under",
      "set the auth header name",
    ];
  for (const [k, v] of Object.entries(row.env)) {
    if (!ENV_KEY.test(k))
      return [
        "Invalid env " + k,
        "an environment variable name is letters, digits and underscores",
        "rename it",
      ];
    if (secretShaped(v))
      return [
        "Invalid env " + k,
        "the value looks like a secret; the store keeps references only",
        "put the secret in 1Password, the keychain or a file and reference it",
      ];
  }
  return null;
}

function mcpRowFrom(body, base, project) {
  const has = (k) => k in body;
  const row = { ...base };
  if (has("name")) row.name = str(body.name);
  if (has("project")) row.project = str(body.project);
  else if (!base.id) row.project = project;
  if (has("transport")) row.transport = str(body.transport);
  if (has("command")) row.command = str(body.command);
  if (has("args"))
    row.args = Array.isArray(body.args) ? body.args.map(String) : base.args;
  if (has("url")) row.url = str(body.url);
  if (has("auth")) row.auth = str(body.auth);
  if (has("auth_header"))
    row.auth_header = str(body.auth_header) || "Authorization";
  if (has("secret_ref")) row.secret_ref = str(body.secret_ref);
  if (has("env"))
    row.env =
      body.env && typeof body.env === "object" && !Array.isArray(body.env)
        ? Object.fromEntries(
            Object.entries(body.env).map(([k, v]) => [k, String(v)]),
          )
        : base.env;
  if (has("note")) row.note = str(body.note);
  if (row.auth === "none") row.secret_ref = "";
  if (row.auth === "oauth")
    row.sign_in = row.sign_in ?? {
      signed_in: false,
      note: "not signed in: pulse mcp login " + row.name,
    };
  else delete row.sign_in;
  return row;
}

// ----- OAuth sign-in from the hub ----------------------------------------------
//
// The real hub discovers the server's authorization server, registers a
// client (or asks for one the person made by hand, as GitHub wants), hands
// the browser one URL or one short code, and waits on a thread. The mock
// plays GitHub for anything under github.com or githubcopilot.com and a
// registering provider for the rest; a timer stands in for the person
// approving.

const MOCK_SIGN_IN_MS = 4500;

// GitHub registers no clients, so PULSE carries one (the GitHub CLI's public
// app) and signs in through it with the device flow; a provider at
// auth.hand.example registers none either and carries nothing, which is the
// needs_client path. Everyone else registers PULSE itself.
function mcpIssuer(server) {
  if (/github(?:copilot)?\.com/.test(server.url))
    return {
      issuer: "https://github.com/login/oauth",
      docs: "https://docs.github.com/apps/oauth-apps",
      device: true,
      registers: false,
      scopes: [],
      built_in: { client_id: "178c6fc778ccc68e1d6a", label: "GitHub CLI" },
    };
  if (/hand\.example/.test(server.url))
    return {
      issuer: "https://auth.hand.example",
      docs: "https://auth.hand.example/apps",
      device: true,
      registers: false,
      scopes: ["read", "write"],
    };
  let host = "provider.example";
  try {
    host = new URL(server.url).host || host;
  } catch {
    // a stdio server has no url; the mock still plays along
  }
  return {
    issuer: "https://auth." + host.replace(/^[^.]+\./, ""),
    docs: "",
    device: false,
    registers: true,
    scopes: ["mcp:tools", "offline_access"],
  };
}

function mcpLoginJson(server, job) {
  const out = {
    state: job.state,
    sign_in: server.sign_in,
    flow: job.flow,
    elapsed: Math.round((Date.now() - job.started) / 1000),
  };
  if (job.flow === "device")
    Object.assign(out, {
      user_code: job.user_code,
      verification_uri: job.verification_uri,
      verification_uri_complete: job.verification_uri_complete,
      expires_in: 899,
    });
  else
    Object.assign(out, {
      url: job.url,
      redirect_uri: job.redirect_uri,
      expires_in: 300,
    });
  Object.assign(out, {
    client_id: job.client_id,
    client_label: job.client_label || "",
    built_in: !!job.client_label,
  });
  if (job.state === "ok") out.note = server.sign_in.note;
  if (job.state === "failed")
    Object.assign(out, { why: job.why, cancelled: !!job.cancelled });
  return out;
}

function mcpLoginStart(server, body, res) {
  const flow = str(body.flow);
  if (!["", "code", "device"].includes(flow))
    return fail(
      res,
      400,
      "Invalid flow",
      "flow is code or device",
      "send {flow:'device'} or leave it out",
    );
  const portRaw = body.redirect_port ?? "";
  const port = portRaw === "" ? 0 : Number(portRaw);
  if (!Number.isInteger(port) || port < 0 || port > 65535)
    return fail(
      res,
      400,
      "Invalid redirect_port",
      "a port is a number between 1 and 65535",
      "send a port or leave it out",
    );
  const clientId = str(body.client_id);
  const who = mcpIssuer(server);
  const redirectPort = port || 8976;
  const redirectUri = "http://127.0.0.1:" + redirectPort + "/callback";
  const prior = state.mcpLogins.get(server.id);
  if (prior?.timer) clearTimeout(prior.timer);
  state.mcpLogins.delete(server.id);
  const steps = ["asked " + server.url + " how it signs in: " + who.issuer];
  const builtIn =
    !clientId && !who.registers && who.device ? who.built_in : undefined;
  if (!clientId && !who.registers && !builtIn) {
    steps.push(
      who.issuer + " registers no clients; one made by hand is needed",
    );
    return send(res, 200, {
      state: "needs_client",
      server: server.name,
      issuer: who.issuer,
      service_documentation: who.docs,
      device_flow: who.device,
      redirect_uri: redirectUri,
      redirect_port: port,
      scopes_supported: who.scopes,
      command:
        "pulse mcp login " +
        server.name +
        " --client-id ID" +
        (who.device ? "" : " --redirect-port " + redirectPort),
      steps,
      sign_in: server.sign_in,
    });
  }
  if (builtIn && flow === "code")
    return fail(
      res,
      502,
      "Cannot sign in to " + server.name,
      "the built-in client " +
        builtIn.client_id +
        " (" +
        builtIn.label +
        ") signs in with the device flow only, since it has no secret; the code flow at " +
        who.issuer +
        " takes a client you registered by hand: --client-id ID [--client-secret REF] --redirect-port PORT",
      "fix what the why line names and run the login again; pulse mcp show NAME shows the server",
    );
  const useDevice =
    flow === "device" || !!builtIn || (flow === "" && clientId && who.device);
  const id =
    clientId ||
    builtIn?.client_id ||
    "pulse-" + Math.random().toString(36).slice(2, 8);
  steps.push(
    builtIn
      ? "using the built-in client " +
          id +
          " (" +
          builtIn.label +
          "'s public app), with the device flow; the sign-in page asks you to authorize " +
          builtIn.label
      : clientId
        ? "using client " +
          id +
          " registered by hand" +
          (useDevice ? ", with the device flow" : "")
        : "registered client " + id + " with " + who.issuer,
  );
  const job = {
    state: "pending",
    flow: useDevice ? "device" : "code",
    started: Date.now(),
    client_id: id,
    client_label: builtIn?.label || "",
  };
  if (useDevice) {
    const code = "WDJB-MJHT";
    Object.assign(job, {
      user_code: code,
      verification_uri: "https://github.com/login/device",
      verification_uri_complete:
        "https://github.com/login/device?user_code=" + code,
    });
    steps.push(
      "sign in here:\n" + job.verification_uri + "\nand enter the code " + code,
      "waiting for the sign-in at " + who.issuer + " (up to 15 min)",
    );
  } else {
    Object.assign(job, {
      url:
        who.issuer +
        "/authorize?client_id=" +
        encodeURIComponent(id) +
        "&redirect_uri=" +
        encodeURIComponent(redirectUri) +
        "&response_type=code&code_challenge_method=S256&state=mock",
      redirect_uri: redirectUri,
    });
    steps.push(
      "sign in here:\n" + job.url,
      "waiting for the sign-in on " + redirectUri + " (up to 5 min)",
    );
  }
  job.timer = setTimeout(() => {
    if (state.mcpLogins.get(server.id) !== job) return;
    job.state = "ok";
    job.timer = null;
    server.sign_in = {
      signed_in: true,
      note: builtIn
        ? "signed in at " +
          who.issuer +
          " through the built-in client " +
          id +
          " (" +
          builtIn.label +
          "'s public app, device flow)"
        : "signed in at " +
          who.issuer +
          " as client " +
          id +
          (useDevice ? " (device flow)" : "") +
          "; the token expires in 8h",
    };
    server.updated_at = now();
    emit(
      "owner",
      "mcp.signed_in",
      server.id,
      JSON.stringify({
        name: server.name,
        issuer: who.issuer,
        client_id: id,
        flow: job.flow,
        from: "hub",
      }),
    );
  }, MOCK_SIGN_IN_MS);
  job.timer.unref?.();
  state.mcpLogins.set(server.id, job);
  return send(res, 200, { ...mcpLoginJson(server, job), steps });
}

function mcpLogout(server, res) {
  const prior = state.mcpLogins.get(server.id);
  if (prior?.timer) clearTimeout(prior.timer);
  state.mcpLogins.delete(server.id);
  const had = !!server.sign_in?.signed_in;
  server.sign_in = {
    signed_in: false,
    note: "not signed in: pulse mcp login " + server.name,
  };
  if (had) {
    server.updated_at = now();
    emit(
      "owner",
      "mcp.signed_out",
      server.id,
      JSON.stringify({ name: server.name, revoked: false, from: "hub" }),
    );
  }
  return send(res, 200, {
    ok: true,
    had_grant: had,
    revoked: false,
    note: had
      ? "the tokens on this machine are gone; the provider was not told"
      : "there was no sign-in to forget",
    server,
  });
}

function mcpWrite(req, res, segs, q, body) {
  const now_ = now();
  if (req.method === "POST" && segs.length === 1) {
    const row = mcpRowFrom(
      body,
      {
        id: "",
        name: "",
        project: "",
        transport: "stdio",
        command: "",
        args: [],
        url: "",
        auth: "none",
        auth_header: "Authorization",
        secret_ref: "",
        env: {},
        note: "",
        status: "enabled",
        last_check_at: "",
        last_check_ok: null,
        last_check_note: "",
        created_by: "owner",
        created_at: now_,
        updated_at: now_,
      },
      q.get("project") || "",
    );
    const bad = mcpValidate(row);
    if (bad) return fail(res, 400, ...bad);
    const dup = state.mcp.find(
      (x) => x.name === row.name && x.project === row.project,
    );
    if (dup)
      return fail(
        res,
        400,
        "Invalid name " + row.name,
        dup.id +
          " already uses it" +
          (row.project ? " for " + row.project : " for every project"),
        "pick another name or update " + dup.id,
      );
    row.id = nextId("MCP", state.mcp);
    state.mcp.push(row);
    emit(
      "owner",
      "mcp.added",
      row.id,
      row.name +
        " (" +
        row.transport +
        ", " +
        (row.project || "all projects") +
        ")",
    );
    return send(res, 200, { server: row });
  }
  const server = state.mcp.find((x) => x.id === segs[1] || x.name === segs[1]);
  if (!server)
    return fail(
      res,
      404,
      "No MCP server " + (segs[1] ?? ""),
      "Nothing registered under that id or name.",
      "pulse mcp list shows what there is.",
    );
  if (req.method === "PATCH" && segs.length === 2) {
    const keys = Object.keys(body).filter((k) =>
      [
        "name",
        "project",
        "transport",
        "command",
        "args",
        "url",
        "auth",
        "auth_header",
        "secret_ref",
        "env",
        "note",
      ].includes(k),
    );
    if (!keys.length)
      return fail(
        res,
        400,
        "Nothing to update",
        "The patch names no field.",
        "Send at least one of name, project, transport, command, args, url, auth, auth_header, secret_ref, env, note.",
      );
    const row = mcpRowFrom(body, server, server.project);
    const bad = mcpValidate(row);
    if (bad) return fail(res, 400, ...bad);
    const dup = state.mcp.find(
      (x) =>
        x.id !== server.id && x.name === row.name && x.project === row.project,
    );
    if (dup)
      return fail(
        res,
        400,
        "Invalid name " + row.name,
        dup.id + " already uses it",
        "pick another name",
      );
    Object.assign(server, row, { updated_at: now_ });
    emit("owner", "mcp.updated", server.id, keys.join(", "));
    return send(res, 200, { server });
  }
  if (req.method === "POST" && segs[2] === "status" && segs.length === 3) {
    const status = str(body.status);
    if (!["enabled", "disabled"].includes(status))
      return fail(
        res,
        400,
        "Invalid status",
        "status is enabled or disabled",
        "send {status:'enabled'} or {status:'disabled'}",
      );
    if (server.status !== status) {
      server.status = status;
      server.updated_at = now_;
      emit(
        "owner",
        status === "enabled" ? "mcp.enabled" : "mcp.disabled",
        server.id,
        server.name,
      );
    }
    return send(res, 200, { server });
  }
  if (req.method === "POST" && segs[2] === "login" && segs.length === 3) {
    if (server.auth !== "oauth")
      return fail(
        res,
        400,
        server.name + " does not sign in with OAuth",
        "its auth is " + server.auth,
        "pulse mcp update " + server.name + " --auth oauth first",
      );
    return mcpLoginStart(server, body, res);
  }
  if (req.method === "POST" && segs[2] === "logout" && segs.length === 3) {
    if (server.auth !== "oauth")
      return fail(
        res,
        400,
        server.name + " does not sign in with OAuth",
        "its auth is " + server.auth,
        "pulse mcp update " + server.name + " --auth oauth first",
      );
    return mcpLogout(server, res);
  }
  if (req.method === "POST" && segs[2] === "connect" && segs.length === 3) {
    const outcome = mcpConnectOutcome(server);
    server.last_check_at = now_;
    server.last_check_ok = outcome.ok;
    server.last_check_note = outcome.ok ? outcome.note : outcome.why;
    server.updated_at = now_;
    emit(
      "owner",
      "mcp.checked",
      server.id,
      (outcome.ok ? "ok: " : "failed: ") + server.last_check_note,
    );
    if (!outcome.ok)
      return fail(res, outcome.code, outcome.error, outcome.why, outcome.fix);
    return send(res, 200, {
      ok: true,
      note: outcome.note,
      info: outcome.info,
      tools: outcome.tools,
      server,
    });
  }
  return fail(res, 404, "not found", "No such API route.", "Use /api/v1/.");
}

const CANNED_TOOLS = {
  github: [
    [
      "list_issues",
      "List issues of a repository, filtered by state, labels and assignee.",
    ],
    ["get_issue", "Read one issue with its body and labels."],
    ["create_issue", "Open an issue."],
    ["add_issue_comment", "Comment on an issue or pull request."],
    ["list_pull_requests", "List pull requests of a repository."],
    ["get_pull_request", "Read one pull request with its diff summary."],
    ["search_issues", "Search issues and pull requests across repositories."],
    ["search_code", "Search code across repositories."],
    ["get_file_contents", "Read a file at a ref."],
    ["list_commits", "List commits of a branch."],
    ["create_branch", "Create a branch from a ref."],
    ["push_files", "Push several files in one commit."],
  ],
  sandbox: [
    ["create_env", "Create a throwaway environment and return its id."],
    ["run", "Run a command inside an environment."],
    ["destroy_env", "Tear an environment down."],
  ],
};
CANNED_TOOLS["github-remote"] = [
  ...CANNED_TOOLS.github.slice(0, 8),
  ["list_copilot_spaces", "List the Copilot spaces you can read."],
  ["get_copilot_space", "Read one Copilot space."],
];

function mcpConnectOutcome(server) {
  if (server.auth === "oauth" && !server.sign_in?.signed_in) {
    return {
      ok: false,
      code: 409,
      error: server.name + " needs a sign-in",
      why: "no sign-in on this machine yet",
      fix: "pulse mcp login " + server.name,
    };
  }
  if (server.secret_ref.startsWith("keychain://")) {
    return {
      ok: false,
      code: 409,
      error: "Cannot read the secret of " + server.name,
      why:
        "credential " +
        server.secret_ref +
        ": the keychain has no item for service " +
        server.secret_ref.slice("keychain://".length).replace("/", " account "),
      fix: "Check the reference and that this machine can read it, then try Connect again.",
    };
  }
  if (
    server.transport !== "stdio" &&
    /\.internal\b/.test(server.url) &&
    !CANNED_TOOLS[server.name]
  ) {
    return {
      ok: false,
      code: 502,
      error: "Cannot reach " + server.name + " (" + server.url + ")",
      why: "GET " + server.url + ": connection refused",
      fix:
        server.transport === "sse"
          ? "check the url and the credential; a Streamable HTTP server wants --transport http"
          : "check the url and the credential; an older server may want --transport sse",
    };
  }
  const tools = (
    CANNED_TOOLS[server.name] ?? [
      ["ping", "Answer with pong."],
      ["search", "Search what this server knows."],
    ]
  ).map(([name, description]) => ({ name, description }));
  const info = {
    name:
      server.name === "github-remote"
        ? "github-mcp-server"
        : server.name + (server.transport === "stdio" ? "-mcp" : ""),
    version: CANNED_TOOLS[server.name]
      ? server.name === "github"
        ? "0.6.2"
        : "1.0.0"
      : "0.1.0",
    protocol_version: "2025-06-18",
  };
  const note =
    info.name +
    " " +
    info.version +
    ": " +
    tools.length +
    " tool" +
    (tools.length === 1 ? "" : "s") +
    ": " +
    tools.map((t) => t.name).join(", ");
  return { ok: true, note, info, tools };
}

function skillValidate(row, except) {
  if (!SLUG.test(row.name) || row.name.length > 64)
    return [
      "Invalid name " + row.name,
      "a name is lowercase letters, digits and dashes, 64 characters at most",
      "pick a slug like release-check",
    ];
  const dup = state.skills.find((x) => x.name === row.name && x.id !== except);
  if (dup)
    return [
      "Invalid name " + row.name,
      dup.id + " already uses it",
      "pick another name or update " + dup.id,
    ];
  if (
    !row.description ||
    /[\r\n]/.test(row.description) ||
    row.description.length > 200
  )
    return [
      "Invalid description",
      "one line, 200 characters at most",
      "say in one line when to read the skill",
    ];
  if (!row.body.trim())
    return [
      "Invalid body",
      "a skill needs a body",
      "write the recipe in markdown",
    ];
  if (row.body.length > 65536)
    return ["Invalid body", "64 KiB at most", "shorten it"];
  if (row.body.split(/\s+/).some(secretShaped))
    return [
      "Body contains a secret",
      "Skills are shared with every agent.",
      "Name where the secret lives (a reference, an env var) instead of pasting it.",
    ];
  return null;
}

// ----- voice (the composer's microphone) ----------------------------------
//
// GET voice answers the fixture, with a setup that advances about 400 MB a
// second once POST voice/setup started it (the real hub fetches 2.5 GB, here
// it takes a few seconds). POST voice/transcribe answers canned words after
// a short pause, so the whole button can be tried without a model.
async function voiceState() {
  if (!state.voice) {
    const d = await fixture("voice");
    state.voice = { ...d, started: 0, stopped: false };
  }
  return state.voice;
}

function voiceTick(v) {
  if (!v.setting_up) return;
  const elapsed = (Date.now() - v.started) / 1000;
  const done = Math.min(v.setup.total, Math.floor(elapsed * 400e6));
  v.setup.done = done;
  const rt = v.runtime.size;
  if (done < rt) {
    v.setup.phase = "runtime";
    v.setup.file = "libonnxruntime.1.31.1.dylib";
  } else {
    v.setup.phase = "model";
    const m = v.models.find((x) => x.id === v.setup.model);
    let at = rt;
    v.setup.file = "";
    for (const f of m.files) {
      f.have = Math.max(0, Math.min(f.size, done - at));
      f.ready = f.have === f.size;
      if (!f.ready && !v.setup.file) v.setup.file = f.name;
      at += f.size;
    }
    m.have = m.files.reduce((a, f) => a + f.have, 0);
    m.ready = m.files.every((f) => f.ready);
    v.runtime.have = rt;
    v.runtime.ready = true;
  }
  if (done >= v.setup.total) {
    v.setting_up = false;
    v.setup.phase = "done";
    v.setup.file = "";
    v.setup.finished_at = new Date().toISOString();
    v.model = v.models.find((x) => x.id === v.setup.model);
    v.ready = true;
    emit("hub", "voice.ready", v.setup.model, { bytes: v.setup.total });
  }
}

function voiceView(v) {
  voiceTick(v);
  const { started, stopped, ...rest } = v;
  void started;
  void stopped;
  return rest;
}

async function voiceRoute(req, res, segs, q) {
  const v = await voiceState();
  if (segs.length === 1) {
    if (req.method !== "GET")
      return fail(res, 405, "Method not allowed", "This route does not support " + req.method + ".", "Use GET.");
    return send(res, 200, voiceView(v));
  }
  const action = segs[1];
  if (action === "setup" && req.method === "POST") {
    let body;
    try {
      body = await readBody(req);
    } catch (e) {
      return fail(res, 400, "bad request", String(e.message), "send a JSON object");
    }
    if (body.stop === true) {
      if (v.setting_up) {
        voiceTick(v);
        v.setting_up = false;
        v.setup.phase = "stopped";
        v.setup.finished_at = new Date().toISOString();
      }
      return send(res, 202, voiceView(v));
    }
    const id = String(body.model ?? v.model.id);
    const m = v.models.find((x) => x.id === id);
    if (!m)
      return fail(res, 400, "Unknown model " + JSON.stringify(id), "The models are " + v.models.map((x) => x.id).join(", ") + ".", "Pick one of them, or leave model out for the voice.model setting.");
    if (v.setting_up)
      return fail(res, 409, "A setup is already running", "This hub is fetching " + v.setup.model + ".", 'Wait for it, or send {"stop": true} first.');
    v.model = m;
    const have = (v.runtime.ready ? v.runtime.size : 0) + m.have;
    v.started = Date.now() - (have / 400e6) * 1000;
    v.setting_up = true;
    v.setup = {
      phase: "runtime",
      model: id,
      file: "",
      done: have,
      total: v.runtime.size + m.size,
      error: "",
      started_at: new Date().toISOString(),
      finished_at: "",
    };
    emit("human", "voice.setup", id, { via: "console" });
    return send(res, 202, voiceView(v));
  }
  if (action === "transcribe" && req.method === "POST") {
    const chunks = [];
    for await (const c of req) chunks.push(c);
    const bytes = Buffer.concat(chunks).length;
    if (!bytes)
      return fail(res, 400, "No audio", "The request body is empty.", "Send the recording as the body: audio/pcm (16 kHz, 16-bit, mono) or audio/wav.");
    voiceTick(v);
    if (!v.ready)
      return fail(res, 409, "The speech model is not set up", v.model.id + " is not on this machine.", "Start the setup from the microphone button, or run `pulse voice setup`.");
    const seconds = bytes / 32000;
    await new Promise((r) => setTimeout(r, Math.min(1500, 200 + seconds * 60)));
    v.loaded = v.model.id;
    const text =
      seconds < 1.5
        ? "Yes."
        : "Add a microphone button to the composer that transcribes on this machine, and make sure the recording never leaves it.";
    return send(res, 200, {
      transcript: {
        text,
        model: v.model.id,
        language: v.model.id.includes("-en-") ? "" : "auto",
        seconds,
        took_ms: Math.round(200 + seconds * 60),
      },
    });
  }
  if (action === "unload" && req.method === "POST") {
    v.loaded = null;
    return send(res, 200, voiceView(v));
  }
  if (action === "model" && req.method === "DELETE") {
    const all = ["1", "true", "yes"].includes(q.get("all") ?? "");
    const id = q.get("id") || v.model.id;
    const targets = all ? v.models : v.models.filter((x) => x.id === id);
    if (!targets.length)
      return fail(res, 400, "Unknown model " + JSON.stringify(id), "No such model is in the catalogue.", "Name a model from GET /api/v1/voice, or pass all=1.");
    let freed = 0;
    for (const m of targets) {
      freed += m.have;
      m.have = 0;
      m.ready = false;
      for (const f of m.files) {
        f.have = 0;
        f.ready = false;
      }
    }
    if (all) {
      freed += v.runtime.have;
      v.runtime.have = 0;
      v.runtime.ready = false;
    }
    v.loaded = null;
    v.ready = v.runtime.ready && v.model.ready;
    return send(res, 200, { ...voiceView(v), freed });
  }
  return fail(res, 404, "Unknown API route", "That owner-console route does not exist.", "Use an /api/v1 route.");
}

// ----- Memory (what agents remembered) -----------------------------------
// The fixture is the files as the hub lists them; writes change the in-memory
// copy the way crates/pulse-harness/src/memory.rs changes the Markdown.
function memoryView() {
  const files = state.memory.files;
  return {
    ...state.memory,
    total: files.reduce((n, f) => n + f.entries.length, 0),
    files,
  };
}

function memoryWrite(res, segs, body) {
  const text = String(body.text ?? "")
    .split(/\s+/)
    .filter(Boolean)
    .join(" ");
  if (!text)
    return fail(
      res,
      400,
      "Invalid text",
      "text must be a non-empty string.",
      "Send a JSON object with a valid text.",
    );
  const scope = String(body.scope ?? "");
  const key = String(body.key ?? "").trim();
  const wantUser = scope === "user" || (!scope && (!key || key === "user"));
  const file = wantUser
    ? state.memory.files.find((f) => f.kind === "user")
    : state.memory.files.find((f) => f.key === key);
  if (!file)
    return fail(
      res,
      key ? 404 : 400,
      key ? "No memory " + key : "A key is required",
      key
        ? "No project or memory file goes by that key."
        : "A project memory needs the key of the file (the project's slug).",
      key ? "Pick a file the memory list shows." : 'Send key: "<slug>" or scope: "user".',
    );
  const stamp = now();
  if (segs.length === 1) {
    if (text.length > state.memory.limits.text)
      return fail(
        res,
        400,
        text.length + " characters is too long for one memory; keep it under " + state.memory.limits.text + " (a fact, not an essay)",
        "A memory is one fact, not an essay.",
        "Keep it under " + state.memory.limits.text + " characters.",
      );
    const day = stamp.slice(0, 10);
    if (!file.entries.some((e) => e.text === text)) {
      file.entries.push({ day, text, line: "[" + day + "] " + text });
      file.exists = true;
      file.size += text.length + 16;
      file.updated_at = stamp;
      emit("human", "memory.remembered", file.key, {
        scope: file.scope,
        file: file.path,
        count: file.entries.length,
        from: "hub",
      });
    }
    return send(res, 201, { file, count: file.entries.length });
  }
  if (segs[1] !== "forget")
    return fail(res, 404, "Unknown API route", "That owner-console route does not exist.", "Use an /api/v1 route.");
  const lower = text.toLowerCase();
  const hits = file.entries.filter((e) => e.line.toLowerCase().includes(lower));
  if (hits.length === 0)
    return fail(
      res,
      404,
      "nothing remembered contains " + JSON.stringify(text),
      "No entry in that file contains the text.",
      "Reload the page; another run may have forgotten it already.",
    );
  if (hits.length > 1)
    return fail(
      res,
      409,
      hits.length + " memories contain " + JSON.stringify(text) + "; give more of the one to forget",
      "More than one entry matches, so none was removed.",
      "Give more of the one to forget.",
    );
  file.entries = file.entries.filter((e) => e !== hits[0]);
  file.size = Math.max(0, file.size - hits[0].line.length - 3);
  file.updated_at = stamp;
  emit("human", "memory.forgotten", file.key, {
    scope: file.scope,
    file: file.path,
    count: file.entries.length,
    from: "hub",
  });
  return send(res, 200, { file, count: file.entries.length });
}

function skillWrite(req, res, segs, q, body) {
  const now_ = now();
  if (req.method === "POST" && segs.length === 1) {
    const row = {
      id: "",
      name: str(body.name),
      project: "project" in body ? str(body.project) : q.get("project") || "",
      description: str(body.description),
      body: typeof body.body === "string" ? body.body : "",
      status: "enabled",
      created_by: "owner",
      created_at: now_,
      updated_at: now_,
      uses: 0,
    };
    const bad = skillValidate(row, "");
    if (bad) return fail(res, 400, ...bad);
    row.id = nextId("SKILL", state.skills);
    state.skills.push(row);
    emit(
      "owner",
      "skill.added",
      row.id,
      row.name + " (" + (row.project || "all projects") + ")",
    );
    return send(res, 200, { skill: row });
  }
  const skill = state.skills.find(
    (x) => x.id === segs[1] || x.name === segs[1],
  );
  if (!skill)
    return fail(
      res,
      404,
      "No skill " + (segs[1] ?? ""),
      "Nothing registered under that id or name.",
      "pulse skill list shows what there is.",
    );
  if (req.method === "PATCH" && segs.length === 2) {
    const keys = Object.keys(body).filter((k) =>
      ["name", "project", "description", "body"].includes(k),
    );
    if (!keys.length)
      return fail(
        res,
        400,
        "Nothing to update",
        "The patch names no field.",
        "Send at least one of name, project, description, body.",
      );
    const row = { ...skill };
    if ("name" in body) row.name = str(body.name);
    if ("project" in body) row.project = str(body.project);
    if ("description" in body) row.description = str(body.description);
    if ("body" in body)
      row.body = typeof body.body === "string" ? body.body : "";
    const bad = skillValidate(row, skill.id);
    if (bad) return fail(res, 400, ...bad);
    Object.assign(skill, row, { updated_at: now_ });
    emit("owner", "skill.updated", skill.id, keys.join(", "));
    return send(res, 200, { skill });
  }
  if (req.method === "POST" && segs[2] === "status" && segs.length === 3) {
    const status = str(body.status);
    if (!["enabled", "disabled"].includes(status))
      return fail(
        res,
        400,
        "Invalid status",
        "status is enabled or disabled",
        "send {status:'enabled'} or {status:'disabled'}",
      );
    if (skill.status !== status) {
      skill.status = status;
      skill.updated_at = now_;
      emit(
        "owner",
        status === "enabled" ? "skill.enabled" : "skill.disabled",
        skill.id,
        skill.name,
      );
    }
    return send(res, 200, { skill });
  }
  return fail(res, 404, "not found", "No such API route.", "Use /api/v1/.");
}

await init();
createServer((req, res) => {
  handle(req, res).catch((e) => {
    console.error(e);
    if (!res.headersSent)
      fail(
        res,
        500,
        "mock server error",
        String(e.message),
        "see the terminal running dev:mock",
      );
  });
}).listen(port, "127.0.0.1", () =>
  console.log(
    `pulse mock hub on http://127.0.0.1:${port}/api/v1/  (fixtures: ${root})`,
  ),
);
