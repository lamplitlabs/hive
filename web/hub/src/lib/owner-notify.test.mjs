import assert from "node:assert/strict";
import { test } from "node:test";
import {
  DEFAULT_PREFS,
  EMPTY_SEEN,
  MAX_AT_ONCE,
  NOTIFY_PREFS_KEY,
  NOTIFY_SEEN_KEY,
  NOTIFY_SOURCES,
  SEEN_KEEP_MS,
  SEEN_MAX,
  delivery,
  loadedSources,
  notifyItems,
  notifyStatus,
  planNotices,
  pruneSeen,
  readPrefs,
  readSeen,
  silentNotification,
  writePrefs,
  writeSeen,
} from "./owner-notify.ts";

const memory = (entries = {}) => {
  const m = new Map(Object.entries(entries));
  return { getItem: (k) => (m.has(k) ? m.get(k) : null), setItem: (k, v) => void m.set(k, v), removeItem: (k) => void m.delete(k), keys: () => [...m.keys()] };
};

const conv = (id, extra = {}) => ({ id, title: "Title " + id, project: "demo", status: "idle", turns: 1, note: "", parent: "", created_by: "human", ...extra });
const task = (id, extra = {}) => ({ id, project: "demo", title: "Task " + id, status: "in_progress", lead: "demo-lead", last_author: "demo-lead", needs_you: false, updated_at: "2026-11-03T00:00:00Z", last_message_at: "2026-11-03T00:00:00Z", ...extra });
const question = (id, extra = {}) => ({ id, summary: "Question " + id, slug: "demo", stop: false, decider: "owner", agent: "granite", name: "", ...extra });
const ALL = { proposals: [], questions: [], drafts: [], steps: [], decisions: [], compute: [], work: [], chat: { conversations: [], tasks: [] } };
const ON = { waits: true, finished: true };

// One look at `input`, seen and loaded from the input itself.
const look = (input, seen, prefs = ON, extra = {}) => planNotices({ items: notifyItems(input), loaded: loadedSources(input), seen, now: 1000, prefs, ...extra });

test("everything the For-you count counts is something that waits", () => {
  const items = notifyItems({
    brain: "pulse",
    decisions: [{ id: "ADR-007", project: "demo", title: "Use SQLite" }],
    proposals: [{ id: "p1", summary: "Raise the cap", slug: "" }],
    questions: [question("q1"), question("q2", { stop: true })],
    drafts: [{ slug: "demo", r: { id: "CTX-evaluators", draft: { author: "iris", at: "2026-11-01" } } }],
    steps: [{ kind: "held_out", slug: "demo", slugs: ["demo", "other"], title: "Set up private tests", label: "x" }],
    compute: [{ id: "c1", requested_by: "quartz", size: "Standard_D4s_v5", hours: 2, reason: "bench" }],
    work: [{ id: "SUG-001", title: "A feature", project: "demo" }],
  });
  assert.deepEqual(items.map((i) => i.kind), ["decision", "proposal", "question", "question", "draft", "step", "compute", "work"]);
  assert.ok(items.every((i) => i.group === "waits"));
  assert.equal(items[0].href, "/records/?id=ADR-007&project=demo");
  assert.equal(items[1].href, "/project/?slug=pulse&tab=for-you", "a post without a slug belongs to the brain");
  assert.match(items[1].body, /\(pulse\)$/);
  assert.equal(items[2].title, "Question from granite");
  assert.equal(items[3].title, "Self-improvement stopped: your review");
  assert.equal(items[4].body, "New evaluators for demo, drafted by iris");
  assert.equal(items[5].id, "step:held_out:demo,other");
  assert.equal(items[5].href, "/#for-you", "a step shared by projects opens the home For you");
  assert.equal(items[6].body, "quartz asks for Standard_D4s_v5 for 2 h: bench");
  assert.equal(items[6].href, "/compute/");
  assert.equal(items[7].href, "/project/?slug=demo&tab=for-you");
});

test("tasks: a reply the lead waits for, and a task done", () => {
  const items = notifyItems({ chat: { conversations: [], tasks: [task("TASK-001", { needs_you: true, status: "review" }), task("TASK-002", { status: "done" }), task("TASK-003"), task("TASK-004", { needs_you: true, last_author: "human" })] } });
  assert.deepEqual(items.map((i) => [i.id, i.group]), [
    ["task-review:TASK-001:2026-11-03T00:00:00Z", "waits"],
    ["task-done:TASK-002", "finished"],
  ], "the owner's own last word is no news to the owner");
  assert.equal(items[0].title, "demo-lead waits for your reply");
  assert.equal(items[0].href, "/chat/?task=TASK-001");
  assert.equal(items[0].task, "TASK-001");
  // A later word from the lead is a new item.
  const later = notifyItems({ chat: { conversations: [], tasks: [task("TASK-001", { needs_you: true, last_message_at: "2026-11-03T01:00:00Z" })] } });
  assert.notEqual(later[0].id, items[0].id);
});

test("conversations: a question or an approval waits, a finished turn or close is news", () => {
  const items = notifyItems({
    chat: {
      tasks: [],
      conversations: [
        conv("a", { status: "waiting", live: { status: "waiting", pending: { kind: "question", id: 3, question: "Which port?" }, queued: [] } }),
        conv("b", { status: "waiting", live: { status: "waiting", pending: { kind: "approval", id: 4, tool: "bash", summary: "rm -rf build" }, queued: [] } }),
        conv("c", { status: "idle", turns: 2 }),
        conv("d", { status: "closed", turns: 3, note: "Shipped the fix" }),
        conv("e", { status: "closed", turns: 3, note: "closed by you" }),
        conv("f", { status: "running", turns: 3 }),
        conv("g", { status: "idle", turns: 0 }),
        conv("h", { status: "idle", turns: 2, parent: "a", created_by: "a" }),
        conv("i", { status: "idle", turns: 2, live: { status: "running", pending: null, queued: [] } }),
        conv("j", { status: "idle", turns: 2, live: { status: "idle", pending: null, queued: ["next"] } }),
      ],
    },
  });
  assert.deepEqual(items.map((i) => i.id), ["chat-ask:a:question:3", "chat-ask:b:approval:4", "chat-reply:c:2", "chat-done:d:3"]);
  assert.equal(items[0].title, "An agent asks you");
  assert.equal(items[0].body, "Title a: Which port?");
  assert.equal(items[1].title, "An agent asks for your approval");
  assert.equal(items[1].body, "Title b: rm -rf build");
  assert.equal(items[2].group, "finished");
  assert.equal(items[3].body, "Title d: Shipped the fix");
  assert.equal(items[3].href, "/chat/?id=d");
  assert.equal(items[3].chat, "d");
});

test("the first look at a source takes its backlog in silently; the next look tells what is new, once", () => {
  const first = look({ ...ALL, questions: [question("q1")] }, EMPTY_SEEN);
  assert.deepEqual(first.notices, []);
  assert.deepEqual(first.seen.primed, NOTIFY_SOURCES);
  assert.ok(first.changed);
  assert.equal(first.seen.ids["question:q1"], 1000);

  const second = look({ ...ALL, questions: [question("q1"), question("q2")] }, first.seen);
  assert.deepEqual(second.notices.map((n) => n.tag), ["question:q2"]);
  assert.equal(second.notices[0].count, 1);

  const third = look({ ...ALL, questions: [question("q1"), question("q2")] }, second.seen);
  assert.deepEqual(third.notices, [], "a seen id never notifies again");
  assert.equal(third.changed, false, "nothing new, nothing to write");
});

test("a source that loads late is primed on its own look, not counted as new", () => {
  const orgOnly = { proposals: [], questions: [question("q1")] };
  const first = look(orgOnly, EMPTY_SEEN);
  assert.deepEqual(first.seen.primed, ["org"]);
  const withChat = look({ ...orgOnly, chat: { conversations: [conv("c", { turns: 4 })], tasks: [] } }, first.seen);
  assert.deepEqual(withChat.notices, [], "the chat backlog is not news");
  assert.deepEqual(withChat.seen.primed, ["org", "chat"]);
  const after = look({ ...orgOnly, chat: { conversations: [conv("c", { turns: 5 })], tasks: [] } }, withChat.seen);
  assert.deepEqual(after.notices.map((n) => n.tag), ["chat-reply:c:5"]);
});

test("drafts and gate steps wait for the project registry and prime on their own", () => {
  const orgOnly = { proposals: [], questions: [] };
  const first = look(orgOnly, EMPTY_SEEN);
  assert.deepEqual(first.seen.primed, ["org"]);
  const draft = { slug: "demo", r: { id: "CTX-constitution", draft: { author: "iris", at: "2026-11-01" } } };
  const withGate = look({ ...orgOnly, drafts: [draft], steps: [] }, first.seen);
  assert.deepEqual(withGate.notices, [], "a draft already there when the registry loads is no news");
  assert.deepEqual(withGate.seen.primed, ["org", "gate"]);
  const next = look({ ...orgOnly, drafts: [draft, { slug: "lib", r: { id: "CTX-evaluators", draft: { author: "iris", at: "2026-11-02" } } }], steps: [] }, withGate.seen);
  assert.deepEqual(next.notices.map((n) => n.tag), ["draft:lib:CTX-evaluators:2026-11-02"]);
});

test("a group turned off is marked seen but not told", () => {
  const primed = look(ALL, EMPTY_SEEN).seen;
  const input = { ...ALL, questions: [question("q1")], chat: { conversations: [], tasks: [task("TASK-9", { status: "done" })] } };
  const waitsOnly = look(input, primed, { waits: true, finished: false });
  assert.deepEqual(waitsOnly.notices.map((n) => n.tag), ["question:q1"]);
  assert.ok(waitsOnly.seen.ids["task-done:TASK-9"], "turning finished on later does not replay this");
});

test("news about the conversation or task on screen stays quiet", () => {
  const primed = look(ALL, EMPTY_SEEN).seen;
  const input = { ...ALL, chat: { conversations: [conv("c", { turns: 2 }), conv("d", { turns: 2 })], tasks: [task("TASK-1", { status: "done" })] } };
  const r = look(input, primed, ON, { watching: { chat: "c", task: "TASK-1" } });
  assert.deepEqual(r.notices.map((n) => n.tag), ["chat-reply:d:2"]);
  assert.ok(r.seen.ids["chat-reply:c:2"]);
});

test("more than a few at once become one summary", () => {
  const primed = look(ALL, EMPTY_SEEN).seen;
  const qs = Array.from({ length: MAX_AT_ONCE + 2 }, (_, i) => question("q" + i));
  const r = look({ ...ALL, questions: qs }, primed);
  assert.equal(r.notices.length, 1);
  const [n] = r.notices;
  assert.equal(n.count, MAX_AT_ONCE + 2);
  assert.equal(n.title, MAX_AT_ONCE + 2 + " things wait for you");
  assert.equal(n.href, "/project/?slug=demo&tab=for-you", "one place for them all is where it goes");
  assert.match(n.body, /and 2 more$/);

  const mixed = look({ ...ALL, questions: qs.slice(0, 2), chat: { conversations: [conv("c", { turns: 2 }), conv("d", { turns: 2 })], tasks: [] } }, primed);
  assert.equal(mixed.notices[0].title, "4 updates from PULSE");
  assert.equal(mixed.notices[0].href, "/");
  const chats = look({ ...ALL, chat: { conversations: ["a", "b", "c", "d"].map((id) => conv(id, { turns: 2 })), tasks: [] } }, primed);
  assert.equal(chats.notices[0].title, "4 things finished");
  assert.equal(chats.notices[0].href, "/chat/");
});

test("seen ids are pruned only when every source loaded, and kept while they show", () => {
  const now = SEEN_KEEP_MS * 3;
  const ids = { old: 0, shown: 0, recent: now - 1000 };
  assert.equal(pruneSeen(ids, new Set(["shown"]), now, false), ids, "a partial look drops nothing");
  assert.deepEqual(pruneSeen(ids, new Set(["shown"]), now, true), { shown: 0, recent: now - 1000 });
  const many = Object.fromEntries(Array.from({ length: SEEN_MAX + 10 }, (_, i) => ["id" + i, now - i]));
  const kept = pruneSeen(many, new Set(["id" + (SEEN_MAX + 5)]), now, true);
  assert.equal(Object.keys(kept).length, SEEN_MAX);
  assert.ok("id0" in kept, "the newest stay");
  assert.ok("id" + (SEEN_MAX + 5) in kept, "an id still shown stays");
  assert.ok(!("id" + (SEEN_MAX + 9) in kept));
});

test("in front a note on the page, behind the browser's notification, in the app the Mac's", () => {
  const base = { app: false, focused: true, supported: true, permission: "granted", sound: true };
  assert.deepEqual(delivery(base), { toast: true, system: false, app: false, chime: true });
  assert.deepEqual(delivery({ ...base, focused: false }), { toast: false, system: true, app: false, chime: true });
  assert.deepEqual(delivery({ ...base, focused: false, permission: "denied" }), { toast: false, system: false, app: false, chime: true }, "blocked: the chime still plays");
  assert.deepEqual(delivery({ ...base, focused: false, supported: false, sound: false }), { toast: false, system: false, app: false, chime: false });
  assert.deepEqual(delivery({ ...base, app: true, focused: false }), { toast: false, system: false, app: true, chime: false }, "the app plays the Mac's sound itself");
  assert.deepEqual(delivery({ ...base, app: true }), { toast: true, system: false, app: true, chime: false });
  assert.deepEqual(delivery({ ...base, app: true, focused: false, permission: "denied" }), { toast: false, system: false, app: false, chime: true }, "macOS said no: the page's chime");
  assert.deepEqual(delivery({ ...base, app: true, focused: false, permission: "unsupported", sound: false }), { toast: false, system: false, app: false, chime: false });
  assert.equal(silentNotification(true, true), true, "the chime played: no second sound");
  assert.equal(silentNotification(true, false), false, "the chime could not play: the browser's sound stands in");
  assert.equal(silentNotification(false, false), true);
});

test("the choices read back, default off, and turning on forgets the seen ids", () => {
  const storage = memory();
  assert.deepEqual(readPrefs(storage), DEFAULT_PREFS);
  assert.deepEqual(DEFAULT_PREFS, { waits: false, finished: false, sound: true });
  writeSeen(storage, { primed: ["org"], ids: { "question:q1": 5 } });
  writePrefs(storage, { waits: true, finished: false, sound: true });
  assert.equal(storage.getItem(NOTIFY_SEEN_KEY), null, "on from off starts from now");
  assert.deepEqual(readPrefs(storage), { waits: true, finished: false, sound: true });
  writeSeen(storage, { primed: ["org"], ids: { "question:q1": 5 } });
  writePrefs(storage, { waits: true, finished: true, sound: false });
  assert.deepEqual(readSeen(storage), { primed: ["org"], ids: { "question:q1": 5 } }, "on stays on: the seen ids stay");
  storage.setItem(NOTIFY_PREFS_KEY, "{not json");
  assert.deepEqual(readPrefs(storage), DEFAULT_PREFS);
  storage.setItem(NOTIFY_PREFS_KEY, JSON.stringify({ waits: "yes", finished: true }));
  assert.deepEqual(readPrefs(storage), { waits: false, finished: true, sound: true });
  storage.setItem(NOTIFY_SEEN_KEY, JSON.stringify({ primed: ["org", "bogus"], ids: { a: 1, b: "x" } }));
  assert.deepEqual(readSeen(storage), { primed: ["org"], ids: { a: 1 } });
  storage.setItem(NOTIFY_SEEN_KEY, "[]");
  assert.deepEqual(readSeen(storage), { primed: [], ids: {} });
});

test("the settings line says what the owner gets here", () => {
  assert.equal(notifyStatus({ app: false, supported: true, permission: "granted", on: true }).tone, "ok");
  assert.equal(notifyStatus({ app: false, supported: true, permission: "denied", on: true }).tone, "warn");
  assert.equal(notifyStatus({ app: false, supported: false, permission: "default", on: false }).tone, "warn");
  assert.match(notifyStatus({ app: false, supported: true, permission: "default", on: false }).text, /asks the browser/);
  assert.match(notifyStatus({ app: true, supported: true, permission: "granted", on: true }).text, /Mac notifications/);
  assert.equal(notifyStatus({ app: true, supported: true, permission: "denied", on: true }).tone, "warn");
  assert.match(notifyStatus({ app: true, supported: true, permission: "denied", on: true }).text, /System Settings/);
  assert.match(notifyStatus({ app: true, supported: false, permission: "unsupported", on: true }).text, /outside its app bundle/);
  assert.match(notifyStatus({ app: true, supported: true, permission: "default", on: false }).text, /asks macOS/);
});
