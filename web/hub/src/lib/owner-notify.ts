// Notifications for the owner, the part that needs no browser: what counts as
// something to tell the owner about, which of it is new since this browser
// last looked, how it is worded and where a click on it goes, and the
// choices kept in localStorage. Pure, so owner-notify.test.mjs can drive it;
// components/shell/owner-notifier.tsx owns React, the Notification API, the
// chime and the real localStorage (and only type imports appear here, so
// `node --test` loads this file as it is).
//
// Two kinds of news:
// - waits: something nothing happens on until the owner answers - every item
//   the "For you" count counts (a proposed decision, a proposal, a question,
//   a compute request, suggested work the agents could not settle, a draft to
//   adopt, a gate step), a task whose lead waits for a reply, and a
//   conversation whose agent asks a question or asks to run a tool;
// - finished: a task done, a conversation its agent finished, and a reply
//   that ended a conversation's turn.
//
// Every item has a stable id. An id this browser has seen never notifies
// again (the ids persist in localStorage, so a reload or a second tab stays
// quiet), and the first time a source loads its items are taken as already
// seen, so turning notifications on never replays the backlog.

import type { ChatListResponse, ChatTask, ComputeRequest, Conversation, OwnerQuestion, Proposal, RecordRow, WorkSuggestion } from "./api";
import type { GateStep } from "../components/hive/evolve";
import type { DraftState } from "../components/records/draft-panel";

export type NotifyGroup = "waits" | "finished";

// Where an item comes from: each loads on its own, and an item only counts
// once its source has loaded (and has been primed - see planNotices). "gate"
// (drafts and gate steps) needs the project registry besides the org payload.
export type NotifySource = "org" | "gate" | "records" | "compute" | "suggested" | "chat";
export const NOTIFY_SOURCES: NotifySource[] = ["org", "gate", "records", "compute", "suggested", "chat"];

export type NotifyKind =
  | "decision"
  | "proposal"
  | "question"
  | "compute"
  | "work"
  | "draft"
  | "step"
  | "task-review"
  | "chat-ask"
  | "task-done"
  | "chat-done"
  | "chat-reply";

export interface NotifyItem {
  id: string;
  kind: NotifyKind;
  group: NotifyGroup;
  source: NotifySource;
  title: string;
  body: string;
  href: string; // a console path, e.g. /chat/?id=chat-1
  // The conversation or task the item is about, so the page that already
  // shows it in front of the owner can stay quiet.
  chat?: string;
  task?: string;
}

// What to tell the owner: one notification (or one note on the page).
export interface Notice {
  tag: string; // the notification's tag: a second one with the same tag replaces the first
  title: string;
  body: string;
  href: string;
  count: number; // 1, or how many items a summary stands for
}

// The data each source contributes; undefined while that source has not loaded.
export interface NotifyInput {
  brain?: string; // posts without a slug belong to the brain project
  decisions?: RecordRow[];
  proposals?: Proposal[];
  questions?: OwnerQuestion[];
  drafts?: { slug: string; r: Pick<DraftState, "id" | "draft"> }[];
  steps?: Pick<GateStep, "kind" | "slug" | "slugs" | "title" | "label">[];
  compute?: ComputeRequest[];
  work?: WorkSuggestion[];
  chat?: Pick<ChatListResponse, "conversations" | "tasks">;
}

// ----- links ---------------------------------------------------------------

function qs(params: Record<string, string | undefined>): string {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) if (v) q.set(k, v);
  const s = q.toString();
  return s ? "?" + s : "";
}

// The same paths src/lib/nav.ts builds (hrefs.project/.record/.task/.conversations).
export const notifyHref = {
  forYou: (slug: string) => (slug ? "/project/" + qs({ slug, tab: "for-you" }) : "/#for-you"),
  record: (id: string, project?: string) => "/records/" + qs({ id, project }),
  compute: () => "/compute/",
  task: (id: string) => "/chat/" + qs({ task: id }),
  chat: (id: string) => "/chat/" + qs({ id }),
};

// ----- what there is to tell -----------------------------------------------

const clip = (s: string, n: number) => {
  const one = (s || "").replace(/\s+/g, " ").trim();
  return one.length > n ? one.slice(0, n - 1).trimEnd() + "…" : one;
};

const isOwner = (who: string) => who === "human" || who === "owner";

// The note the harness writes when the owner closes a conversation
// (crates/hive-harness/src/conversation.rs, CLOSED_BY_OWNER_NOTE): a
// conversation the owner closed is no news to the owner.
export const CLOSED_BY_OWNER_NOTE = "closed by you";

function askerName(q: Pick<OwnerQuestion, "name" | "agent">): string {
  return q.name || q.agent || "an agent";
}

function chatItems(c: Conversation): NotifyItem[] {
  // A sub-conversation reports to the conversation that started it, not to
  // the owner; its parent's own turn is what the owner hears about.
  if (c.parent) return [];
  const live = c.live;
  const status = live?.status || c.status;
  const title = clip(c.title || c.id, 80);
  const base = { source: "chat" as const, href: notifyHref.chat(c.id), chat: c.id };
  const pending = live?.pending;
  if (pending) {
    const approval = pending.kind === "approval";
    return [
      {
        ...base,
        id: "chat-ask:" + c.id + ":" + pending.kind + ":" + pending.id,
        kind: "chat-ask",
        group: "waits",
        title: approval ? "An agent asks for your approval" : "An agent asks you",
        body: title + ": " + clip(approval ? pending.summary || pending.tool : pending.question, 160),
      },
    ];
  }
  if (!c.turns || status === "running" || (live?.queued?.length ?? 0) > 0) return [];
  if (status === "closed") {
    if (c.note === CLOSED_BY_OWNER_NOTE) return [];
    return [{ ...base, id: "chat-done:" + c.id + ":" + c.turns, kind: "chat-done", group: "finished", title: "Session finished", body: title + (c.note ? ": " + clip(c.note, 160) : "") }];
  }
  if (status === "idle") return [{ ...base, id: "chat-reply:" + c.id + ":" + c.turns, kind: "chat-reply", group: "finished", title: "Reply ready", body: title }];
  return [];
}

function taskItems(t: ChatTask): NotifyItem[] {
  const base = { source: "chat" as const, href: notifyHref.task(t.id), task: t.id };
  const title = clip(t.title || t.id, 120);
  if (t.status === "done") return [{ ...base, id: "task-done:" + t.id, kind: "task-done", group: "finished", title: "Task finished", body: title }];
  // The lead's latest word asks for the owner: a later word from the lead
  // while the task still waits is news again, the owner's own reply is not.
  if (t.needs_you && !isOwner(t.last_author))
    return [{ ...base, id: "task-review:" + t.id + ":" + (t.last_message_at || t.updated_at), kind: "task-review", group: "waits", title: (t.lead || "The lead") + " waits for your reply", body: title }];
  return [];
}

// Every item there is to tell the owner about now, from the sources that loaded.
export function notifyItems(input: NotifyInput): NotifyItem[] {
  const brain = input.brain ?? "";
  const out: NotifyItem[] = [];
  for (const r of input.decisions ?? [])
    out.push({ id: "decision:" + r.project + ":" + r.id, kind: "decision", group: "waits", source: "records", title: "A decision waits for you", body: clip(r.title || r.id, 140) + " (" + r.id + (r.project ? ", " + r.project : "") + ")", href: notifyHref.record(r.id, r.project) });
  for (const p of input.proposals ?? []) {
    const slug = p.slug || brain;
    out.push({ id: "proposal:" + p.id, kind: "proposal", group: "waits", source: "org", title: "A proposal waits for your yes or no", body: clip(p.summary, 160) + (slug ? " (" + slug + ")" : ""), href: notifyHref.forYou(slug) });
  }
  for (const q of input.questions ?? []) {
    const slug = q.slug || brain;
    out.push({ id: "question:" + q.id, kind: "question", group: "waits", source: "org", title: q.stop ? "Self-improvement stopped: your review" : "Question from " + askerName(q), body: clip(q.summary, 160) + (slug ? " (" + slug + ")" : ""), href: notifyHref.forYou(slug) });
  }
  for (const d of input.drafts ?? []) {
    const what = d.r.id.includes("evaluators") ? "New evaluators" : "A new constitution";
    out.push({ id: "draft:" + d.slug + ":" + d.r.id + ":" + (d.r.draft?.at ?? ""), kind: "draft", group: "waits", source: "gate", title: "A draft waits for you", body: what + " for " + d.slug + (d.r.draft?.author ? ", drafted by " + d.r.draft.author : ""), href: notifyHref.forYou(d.slug) });
  }
  for (const s of input.steps ?? []) {
    const slugs = s.slugs?.length ? s.slugs : [s.slug];
    out.push({ id: "step:" + s.kind + ":" + [...slugs].sort().join(","), kind: "step", group: "waits", source: "gate", title: "A step only you can take", body: clip(s.title || s.label, 120) + " (" + slugs.join(", ") + ")", href: notifyHref.forYou(slugs.length === 1 ? s.slug : "") });
  }
  for (const r of input.compute ?? [])
    out.push({ id: "compute:" + r.id, kind: "compute", group: "waits", source: "compute", title: "A machine request waits for you", body: (r.requested_by || "An agent") + " asks for " + (r.size || "a machine") + (r.hours ? " for " + r.hours + " h" : "") + (r.reason ? ": " + clip(r.reason, 120) : ""), href: notifyHref.compute() });
  for (const s of input.work ?? [])
    out.push({ id: "work:" + s.id, kind: "work", group: "waits", source: "suggested", title: "Suggested work needs your call", body: clip(s.title, 140) + (s.project ? " (" + s.project + ")" : ""), href: notifyHref.forYou(s.project) });
  for (const t of input.chat?.tasks ?? []) out.push(...taskItems(t));
  for (const c of input.chat?.conversations ?? []) out.push(...chatItems(c));
  return out;
}

// The sources whose data is in the input: only their items count.
export function loadedSources(input: NotifyInput): NotifySource[] {
  const has: Record<NotifySource, boolean> = {
    org: input.proposals !== undefined && input.questions !== undefined,
    gate: input.drafts !== undefined && input.steps !== undefined,
    records: input.decisions !== undefined,
    compute: input.compute !== undefined,
    suggested: input.work !== undefined,
    chat: input.chat !== undefined,
  };
  return NOTIFY_SOURCES.filter((s) => has[s]);
}

// ----- what is new ---------------------------------------------------------

// What this browser has already been told about: id -> when it was first seen
// (ms), and the sources whose backlog has been taken in.
export interface NotifySeen {
  primed: NotifySource[];
  ids: Record<string, number>;
}

export const EMPTY_SEEN: NotifySeen = { primed: [], ids: {} };

// More new items than this in one look become one summary notification.
export const MAX_AT_ONCE = 3;
// Seen ids that no longer show are kept this long (so an item that drops out
// of a list for a moment and comes back stays quiet), and at most this many.
export const SEEN_KEEP_MS = 30 * 24 * 3600 * 1000;
export const SEEN_MAX = 2000;

export interface NotifyPrefs {
  waits: boolean; // tell me when something waits for me
  finished: boolean; // tell me when a task or conversation finishes
  sound: boolean; // play the chime (in Hive.app: the Mac's notification sound)
}

export const DEFAULT_PREFS: NotifyPrefs = { waits: false, finished: false, sound: true };

export const notifyOn = (p: NotifyPrefs) => p.waits || p.finished;

export interface NotifyPlan {
  notices: Notice[];
  seen: NotifySeen;
  changed: boolean; // whether `seen` differs from the one passed in
}

export interface PlanArgs {
  items: NotifyItem[];
  loaded: readonly NotifySource[];
  seen: NotifySeen;
  now: number;
  prefs: Pick<NotifyPrefs, "waits" | "finished">;
  // The conversation or task the page in front of the owner shows (only
  // when the page has focus): news about it is already on screen.
  watching?: { chat?: string; task?: string } | null;
}

// One look: takes in the backlog of a source seen for the first time, marks
// every new item seen, and says what to tell the owner about. A new item in
// a group the owner turned off is marked seen too, so turning that group on
// later starts from then, not from the backlog.
export function planNotices({ items, loaded, seen, now, prefs, watching }: PlanArgs): NotifyPlan {
  const ids: Record<string, number> = { ...seen.ids };
  const primedBefore = new Set(seen.primed);
  const primed = new Set(seen.primed);
  let changed = false;
  const fresh: NotifyItem[] = [];
  const current = new Set<string>();
  for (const it of items) {
    if (!loaded.includes(it.source)) continue;
    current.add(it.id);
    if (ids[it.id] !== undefined) continue;
    ids[it.id] = now;
    changed = true;
    if (primedBefore.has(it.source)) fresh.push(it);
  }
  for (const s of loaded)
    if (!primed.has(s)) {
      primed.add(s);
      changed = true;
    }
  const wanted = fresh.filter((it) => prefs[it.group] && !(watching && ((it.chat && it.chat === watching.chat) || (it.task && it.task === watching.task))));
  const notices = wanted.length > MAX_AT_ONCE ? [summaryNotice(wanted)] : wanted.map(noticeOf);
  const pruned = pruneSeen(ids, current, now, NOTIFY_SOURCES.every((s) => loaded.includes(s)));
  if (Object.keys(pruned).length !== Object.keys(ids).length) changed = true;
  return { notices, seen: { primed: NOTIFY_SOURCES.filter((s) => primed.has(s)), ids: pruned }, changed };
}

export const noticeOf = (it: NotifyItem): Notice => ({ tag: it.id, title: it.title, body: it.body, href: it.href, count: 1 });

export function summaryNotice(items: NotifyItem[]): Notice {
  const n = items.length;
  const waits = items.filter((it) => it.group === "waits").length;
  const title = waits === n ? n + " things wait for you" : waits === 0 ? n + " things finished" : n + " updates from Hive";
  const hrefs = new Set(items.map((it) => it.href));
  const allChat = items.every((it) => it.source === "chat");
  const href = hrefs.size === 1 ? items[0].href : allChat ? "/chat/" : waits === n ? "/#for-you" : "/";
  const named = items.slice(0, MAX_AT_ONCE).map((it) => it.title);
  const body = named.join("; ") + (n > named.length ? "; and " + (n - named.length) + " more" : "");
  return { tag: "summary:" + items[0].id, title, body, href, count: n };
}

// Keeps every id still shown, and the rest for SEEN_KEEP_MS - but only drops
// anything when every source loaded this time (an id a failed source did not
// list is not gone), newest first up to SEEN_MAX.
export function pruneSeen(ids: Record<string, number>, current: ReadonlySet<string>, now: number, complete: boolean): Record<string, number> {
  const entries = Object.entries(ids);
  if (!complete && entries.length <= SEEN_MAX) return ids;
  const keep = entries.filter(([id, at]) => current.has(id) || !complete || now - at < SEEN_KEEP_MS);
  if (keep.length <= SEEN_MAX) return keep.length === entries.length ? ids : Object.fromEntries(keep);
  const shown = keep.filter(([id]) => current.has(id));
  const rest = keep.filter(([id]) => !current.has(id)).sort((a, b) => b[1] - a[1]);
  return Object.fromEntries([...shown, ...rest.slice(0, Math.max(0, SEEN_MAX - shown.length))]);
}

// ----- how to deliver it ---------------------------------------------------

export interface DeliveryContext {
  app: boolean; // inside Hive.app, whose bridge posts Mac notifications
  focused: boolean; // the page is visible and has focus
  supported: boolean; // the browser has the Notification API
  // Notification.permission (granted, denied or default); in Hive.app what
  // macOS allows it, as the app last said (or unsupported outside its bundle).
  permission: string;
  sound: boolean;
}

export interface Delivery {
  toast: boolean; // a note on the page
  system: boolean; // a browser notification
  app: boolean; // handed to Hive.app, which posts a Mac notification (or, while it is in front, only plays the sound)
  chime: boolean; // the page plays its own chime
}

// In front, a note on the page; behind, the browser's notification - or the
// Mac's, in Hive.app, which plays the sound itself. The chime plays
// whenever sound is on, even where notifications are blocked (in Hive.app
// too, when macOS does not let it notify).
export function delivery(c: DeliveryContext): Delivery {
  if (c.app && c.permission === "granted") return { toast: c.focused, system: false, app: true, chime: false };
  return { toast: c.focused, system: !c.app && !c.focused && c.supported && c.permission === "granted", app: false, chime: c.sound };
}

// The browser's own notification sound is turned off when the chime plays,
// so the owner hears one sound, not two; when the chime could not play
// (no click on the page yet since it loaded) the browser's sound stands in.
export const silentNotification = (sound: boolean, chimed: boolean) => !sound || chimed;

// The chime: two short rising sine notes (A5, E6), synthesised by the page
// so it needs no sound file. `at` and `dur` in seconds.
export const CHIME_NOTES: { freq: number; at: number; dur: number }[] = [
  { freq: 880, at: 0, dur: 0.16 },
  { freq: 1318.51, at: 0.13, dur: 0.34 },
];
export const CHIME_GAIN = 0.12;

// ----- the choices, in localStorage ----------------------------------------

export const NOTIFY_PREFS_KEY = "hive.notify";
export const NOTIFY_SEEN_KEY = "hive.notify.seen";

// The subset of Storage used here, so tests can pass a Map-backed one.
export interface NotifyStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

function parse(s: string | null): unknown {
  if (!s) return null;
  try {
    return JSON.parse(s);
  } catch {
    return null;
  }
}

export function readPrefs(storage: NotifyStorage): NotifyPrefs {
  const v = parse(storage.getItem(NOTIFY_PREFS_KEY));
  if (!v || typeof v !== "object") return DEFAULT_PREFS;
  const o = v as Record<string, unknown>;
  const flag = (k: keyof NotifyPrefs) => (typeof o[k] === "boolean" ? (o[k] as boolean) : DEFAULT_PREFS[k]);
  return { waits: flag("waits"), finished: flag("finished"), sound: flag("sound") };
}

// Saves the choices. Turning notifications on from off forgets what was
// seen, so the next look takes the backlog in silently: "from now on".
export function writePrefs(storage: NotifyStorage, next: NotifyPrefs): void {
  const was = readPrefs(storage);
  storage.setItem(NOTIFY_PREFS_KEY, JSON.stringify(next));
  if (!notifyOn(was) && notifyOn(next)) storage.removeItem(NOTIFY_SEEN_KEY);
}

export function readSeen(storage: NotifyStorage): NotifySeen {
  const v = parse(storage.getItem(NOTIFY_SEEN_KEY));
  if (!v || typeof v !== "object") return EMPTY_SEEN;
  const o = v as Record<string, unknown>;
  const primed = Array.isArray(o.primed) ? NOTIFY_SOURCES.filter((s) => (o.primed as unknown[]).includes(s)) : [];
  const ids: Record<string, number> = {};
  if (o.ids && typeof o.ids === "object")
    for (const [k, at] of Object.entries(o.ids as Record<string, unknown>)) if (typeof at === "number" && Number.isFinite(at)) ids[k] = at;
  return { primed, ids };
}

export function writeSeen(storage: NotifyStorage, seen: NotifySeen): void {
  storage.setItem(NOTIFY_SEEN_KEY, JSON.stringify(seen));
}

// ----- the settings card ---------------------------------------------------

export interface NotifyStatus {
  tone: "ok" | "info" | "warn";
  text: string;
}

// One line under the switches: what the owner will get here, and what to do
// when the browser stands in the way.
export function notifyStatus(c: { app: boolean; supported: boolean; permission: string; on: boolean }): NotifyStatus {
  if (c.app) {
    if (c.permission === "denied") return { tone: "warn", text: "macOS doesn't let Hive notify you. Allow it in System Settings → Notifications → Hive. Until then: a note on the page while Hive is in front." };
    if (c.permission === "unsupported") return { tone: "warn", text: "This copy of Hive runs outside its app bundle, so macOS won't take its notifications. While Hive is in front you still get a note on the page." };
    if (c.permission !== "granted") return { tone: "info", text: c.on ? "macOS hasn't been asked yet: turn a switch off and on again to ask it." : "Turning this on asks macOS whether Hive may notify you." };
    return { tone: "ok", text: "Hive shows them as Mac notifications, with the Mac's notification sound; while its window is in front, a note on the page instead. System Settings → Notifications → Hive decides how they look." };
  }
  if (!c.supported) return { tone: "warn", text: "This browser can't show notifications. While the console is in front you still get a note on the page and the sound." };
  if (c.permission === "denied")
    return { tone: "warn", text: "This browser blocks notifications from the console. Allow them in the site's settings (the icon left of the address), then reload. Until then: a note on the page while it is in front, and the sound." };
  if (c.permission !== "granted") return { tone: "info", text: c.on ? "The browser hasn't been asked yet: turn a switch off and on again to ask it." : "Turning this on asks the browser for permission." };
  return { tone: "ok", text: "In front, a note appears on the page; in the background, the browser shows a notification. Either way only while a console tab is open." };
}
