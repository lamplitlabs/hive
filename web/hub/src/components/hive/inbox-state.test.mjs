import assert from "node:assert/strict";
import { register } from "node:module";
import { test } from "node:test";

// The one "for you" count every hub screen reads (home hero, sidebar tile,
// project hero, For-you tab), tested without React. inbox-state.ts imports
// only pure .ts modules, extensionless the way Next resolves them; the hub's
// test loader resolves those and node strips their types, so this suite
// needs no package and runs in a clone without node_modules too (the way
// hive evolve measure scores behaviour-hub-view-branches).
register("../project/tsx-test-loader.mjs", import.meta.url);

const { buildInbox, inboxBreakdown } = await import("./inbox-state.ts");

const loaded = (data, error = "") => ({ data, error, loading: false, updatedAt: 1, reload: async () => {} });
const DAY = 86400000;

// A running daemon under its service: no machine step.
const runner = { available: true, daemon: { state: "alive", pid: 1, at: "2026-11-09T09:00:00Z", interval: 300, loops: 1 }, runner: { allowed: true, reason: "" } };
const service = (loadedUnit = true) => ({
  service: { kind: "launchd", supported: true, installed: loadedUnit, units: [{ name: "daemon", path: "/d", present: true, loaded: loadedUnit }, { name: "hub", path: "/h", present: true, loaded: true }], binary: "/hive", logs: "/logs" },
  hub_is_service: true, hub_port: 8765, hub_url: "http://127.0.0.1:8765", command: "hive service install",
});
const settings = (stop = "0") => ({ available: true, can_edit: true, settings: [{ name: "stop", value: stop }] });

const ref = (id, draft) => ({ id, rev: 1, seeded: false, seeded_from: "", ...(draft ? { draft: { author: "iris", at: "2026-11-08T08:00:00Z" } } : {}) });
const gate = (evolve_next, extra = {}) => ({ fixtures_enabled: 6, baseline: true, evolve_level: "L1", evolve_next, ...extra });

const projects = [
  { slug: "demo-app", ...gate("hive evolve setup --heldout"), constitution: ref("EVO-constitution-demo-app", true), evaluators: ref("EVO-evaluators-demo-app", false) },
  { slug: "web", ...gate("hive evolve setup --heldout"), constitution: ref("EVO-constitution-web", false), evaluators: ref("EVO-evaluators-web", true) },
  { slug: "api", ...gate("none - the daemon measures") },
  { slug: "tool" }, // no gate fields on the org row: the registry row supplies them
];
const regRows = [{ slug: "tool", ...gate("hive evolve fixture seed tool") }];

const q = (id, slug, extra = {}) => ({ id, slug, summary: "Q " + id, date: "2026-11-0" + id.length + "T08:00:00Z", stop: false, stale: false, ...extra });

const org = {
  totals: { brain: "amistio" },
  projects,
  proposals: [
    { id: "P-1", slug: "demo-app", status: "awaiting", summary: "Raise the retry budget" },
    { id: "P-2", slug: "", status: "awaiting", summary: "Rename the brain crate" },
    { id: "P-3", slug: "web", status: "accepted", summary: "Already decided" },
  ],
  questions: [
    q("Q-1", "demo-app"),
    q("Q-22", "", { decider: "owner" }),
    q("Q-333", "demo-app", { decider: "lead" }),
    q("Q-4444", "web", { decider: "setting" }),
  ],
};

const sources = (over = {}) => ({
  org: loaded(org),
  proposed: loaded({ records: [{ id: "ADR-0042", kind: "adr", status: "Proposed", project: "demo-app", title: "Use SQLite" }, { id: "ADR-0041", kind: "adr", status: "Accepted", project: "demo-app" }, { id: "PLAN-1", kind: "plan", status: "Proposed", project: "web" }] }),
  reg: loaded({ projects: regRows }),
  compute: loaded({ requests: [{ id: "C-7", project: "web", status: "pending" }, { id: "C-8", project: "web", status: "approved" }] }),
  suggested: loaded({ suggestions: [{ id: "SUG-3", project: "demo-app", status: "owner" }, { id: "SUG-4", project: "api", status: "declined" }] }),
  runner: loaded(runner),
  service: loaded(service()),
  settings: loaded(settings()),
  harness: loaded({ provider: { name: "copilot", label: "GitHub Copilot", device_login: true, token_login: false, login: { logged_in: true } }, providers: [] }),
  ...over,
});

test("total counts each kind once: one decision, two proposals, two owner questions, a request, a suggestion, two drafts and the folded steps", () => {
  const inbox = buildInbox(sources());
  assert.equal(inbox.ready, true);
  assert.deepEqual(inbox.decisions.map((r) => r.id), ["ADR-0042"]);
  assert.deepEqual(inbox.proposals.map((p) => p.id), ["P-1", "P-2"]);
  assert.deepEqual(inbox.questions.map((x) => x.id).sort(), ["Q-1", "Q-22"]);
  assert.deepEqual(inbox.compute.map((r) => r.id), ["C-7"]);
  assert.deepEqual(inbox.work.map((s) => s.id), ["SUG-3"]);
  assert.deepEqual(inbox.drafts.map((d) => d.slug + ":" + d.r.id), ["demo-app:EVO-constitution-demo-app", "web:EVO-evaluators-web"]);
  // The private tests are one per machine: demo-app and web share one card; tool's seed step stays its own.
  assert.deepEqual(inbox.steps.map((s) => s.kind + ":" + (s.slugs ?? [s.slug]).join("+")), ["heldout:demo-app+web", "seed:tool"]);
  assert.equal(inbox.machine, false);
  assert.equal(inbox.provider, null);
  assert.equal(inbox.total, 1 + 2 + 2 + 1 + 1 + 2 + 2);
  assert.deepEqual(inbox.checking, []);
  assert.deepEqual(inbox.down, []);
  assert.deepEqual(inbox.downWhy, []);
  assert.equal(inbox.daemon.pause, "off");
});

test("agentsDeciding names the questions the agents settle but never adds them to total", () => {
  const inbox = buildInbox(sources());
  assert.equal(inbox.agentsDeciding, 2);
  const none = buildInbox(sources({ org: loaded({ ...org, questions: org.questions.filter((x) => !x.decider || x.decider === "owner") }) }));
  assert.equal(none.agentsDeciding, 0);
  assert.equal(none.total, inbox.total);
});

test("a paused daemon and an unconnected provider add one step each", () => {
  const base = buildInbox(sources()).total;
  const paused = buildInbox(sources({ settings: loaded(settings("1")) }));
  assert.equal(paused.machine, true);
  assert.equal(paused.total, base + 1);
  const unset = buildInbox(sources({ service: loaded(service(false)) }));
  assert.equal(unset.machine, true, "a daemon service still to start is the owner's step too");
  assert.equal(unset.total, base + 1);
  const signedOut = buildInbox(sources({ settings: loaded(settings("1")), harness: loaded({ provider: { name: "openai", label: "OpenAI", device_login: false, token_login: true }, providers: [] }) }));
  assert.deepEqual(signedOut.provider, { name: "openai", label: "OpenAI", expired: false });
  assert.equal(signedOut.total, base + 2);
  const expiredAt = new Date(Date.now() - DAY).toISOString();
  const expired = buildInbox(sources({ harness: loaded({ provider: { name: "copilot", label: "GitHub Copilot", device_login: true, token_login: false, login: { logged_in: true, expires_at: expiredAt } }, providers: [] }) }));
  assert.deepEqual(expired.provider, { name: "copilot", label: "GitHub Copilot", expired: true });
  assert.equal(expired.total, base + 1);
  const fresh = buildInbox(sources({ harness: loaded({ provider: { name: "copilot", label: "GitHub Copilot", device_login: true, token_login: false, login: { logged_in: true, expires_at: new Date(Date.now() + DAY).toISOString() } }, providers: [] }) }));
  assert.equal(fresh.provider, null);
  assert.equal(fresh.total, base);
});

test("forProject narrows every kind to one slug, the brain takes the unslugged ones, and the machine never counts there", () => {
  const inbox = buildInbox(sources({ settings: loaded(settings("1")), harness: undefined }));
  const demo = inbox.forProject("demo-app");
  assert.deepEqual(demo.decisions.map((r) => r.id), ["ADR-0042"]);
  assert.deepEqual(demo.proposals.map((p) => p.id), ["P-1"]);
  assert.deepEqual(demo.questions.map((x) => x.id), ["Q-1"]);
  assert.equal(demo.agentsDeciding, 1);
  assert.deepEqual(demo.compute, []);
  assert.deepEqual(demo.work.map((s) => s.id), ["SUG-3"]);
  assert.deepEqual(demo.drafts.map((d) => d.r.id), ["EVO-constitution-demo-app"]);
  // A project's own count keeps its own (unfolded) private-tests step.
  assert.deepEqual(demo.steps.map((s) => s.kind + ":" + s.slug + ":" + (s.slugs ?? []).length), ["heldout:demo-app:0"]);
  assert.equal(demo.machine, false);
  assert.equal(demo.provider, null);
  assert.equal(demo.total, 1 + 1 + 1 + 1 + 1 + 1);
  const web = inbox.forProject("web");
  assert.equal(web.total, 1 + 1 + 1, "web: one request, one draft, one step");
  assert.equal(web.agentsDeciding, 1);
  const brain = inbox.forProject("amistio");
  assert.deepEqual(brain.proposals.map((p) => p.id), ["P-2"]);
  assert.deepEqual(brain.questions.map((x) => x.id), ["Q-22"]);
  assert.equal(brain.total, 2);
  assert.equal(inbox.forProject("tool").steps[0].kind, "seed");
  assert.equal(inbox.forProject("nowhere").total, 0);
  assert.equal(inbox.gateOf(projects[3]).evolve_next, "hive evolve fixture seed tool");
  assert.equal(inbox.gateOf(projects[2]).evolve_next, "none - the daemon measures");
});

test("before anything loads the count is not ready and names what it is still checking", () => {
  const empty = loaded(undefined);
  const inbox = buildInbox({ org: empty, proposed: empty, reg: empty, compute: empty, runner: empty, service: empty, settings: empty });
  assert.equal(inbox.ready, false);
  assert.equal(inbox.total, 0);
  assert.deepEqual(inbox.checking, ["decisions", "compute requests"]);
  assert.deepEqual(inbox.down, []);
  assert.equal(inbox.forProject("demo-app").total, 0);
});

test("a source that failed to load is named down with its reason, not counted as zero waiting", () => {
  const inbox = buildInbox(sources({ proposed: loaded(undefined, "HTTP 500"), compute: loaded(undefined, "timeout"), suggested: undefined }));
  assert.deepEqual(inbox.checking, []);
  assert.deepEqual(inbox.down, ["decisions could not be loaded", "compute requests could not be loaded"]);
  assert.deepEqual(inbox.downWhy, ["decisions could not be loaded (HTTP 500)", "compute requests could not be loaded (timeout)"]);
  assert.deepEqual(inbox.decisions, []);
  assert.deepEqual(inbox.work, []);
  // A stale error beside data still loaded keeps the data: nothing is down.
  const stale = buildInbox(sources({ proposed: loaded({ records: [] }, "HTTP 500"), compute: loaded({ requests: [] }, "timeout") }));
  assert.deepEqual(stale.down, []);
});

test("inboxBreakdown says nothing waits, or lists each kind pluralised with the machine and provider folded into steps", () => {
  const zero = { decisions: [], proposals: [], questions: [], agentsDeciding: 3, compute: [], work: [], drafts: [], steps: [], machine: false, provider: null, total: 0 };
  assert.equal(inboxBreakdown(zero), "nothing waits");
  assert.equal(inboxBreakdown(zero, "all clear"), "all clear");
  assert.equal(inboxBreakdown({ ...zero, proposals: [{}], questions: [{}, {}] }), "1 proposal and 2 questions");
  assert.equal(inboxBreakdown({ ...zero, machine: true }), "1 step");
  assert.equal(
    inboxBreakdown({ ...zero, decisions: [{}, {}], proposals: [{}], questions: [{}], compute: [{}, {}], work: [{}], drafts: [{}, {}], steps: [{}], machine: true, provider: { name: "openai", label: "OpenAI", expired: false } }),
    "2 decisions, 1 proposal, 1 question, 2 requests, 1 suggestion, 2 drafts and 3 steps",
  );
  assert.equal(inboxBreakdown(buildInbox(sources())), "1 decision, 2 proposals, 2 questions, 1 request, 1 suggestion, 2 drafts and 2 steps");
});
