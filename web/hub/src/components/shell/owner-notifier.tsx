"use client";

import { useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import { useRouter } from "next/navigation";
import { toast } from "sonner";
import { buildInbox, useInboxSources } from "@/components/pulse/inbox";
import type { ChatListResponse } from "@/lib/api";
import {
  CHIME_GAIN,
  CHIME_NOTES,
  DEFAULT_PREFS,
  delivery,
  NOTIFY_PREFS_KEY,
  loadedSources,
  notifyItems,
  notifyOn,
  planNotices,
  readPrefs,
  readSeen,
  silentNotification,
  writePrefs,
  writeSeen,
  type Notice,
  type NotifyInput,
  type NotifyPrefs,
  type NotifyStorage,
} from "@/lib/owner-notify";
import { load, useApi } from "@/lib/use-api";

// The owner's notifications: mounted once in the shell, it watches what the
// "For you" count watches plus the chat list, and tells the owner - a note on
// the page while it is in front, the browser's notification (or PULSE.app's
// Mac notification) behind - with a chime. What counts, what is new and the
// wording live in lib/owner-notify.ts; this file owns the browser.
//
// While the page shows, the event stream and each list's own refresh keep the
// data fresh. A hidden page stops both (lib/use-api.ts), so while
// notifications are on a hidden page polls the few lists it needs itself,
// every HIDDEN_POLL_MS (browsers slow a background tab's timers to about once
// a minute after a while).
const HIDDEN_POLL_MS = 30000;
const CHAT_REFRESH_MS = 15000;
// The note on the page stays long enough to be read on a glance back.
const TOAST_MS = 12000;
const POLLED = ["org", "projects", "records?kind=adr&status=Proposed", "compute", "tasks/suggested?status=owner", "chat"];

// ----- the choices, shared with the settings card ---------------------------

const listeners = new Set<() => void>();
function subscribe(cb: () => void) {
  listeners.add(cb);
  window.addEventListener("storage", cb);
  return () => {
    listeners.delete(cb);
    window.removeEventListener("storage", cb);
  };
}

function storage(): NotifyStorage | null {
  try {
    return window.localStorage;
  } catch {
    return null; // private mode or storage disabled: no notifications
  }
}

// The snapshot is the stored JSON itself, so an unchanged value keeps its identity.
const prefsSnapshot = () => storage()?.getItem(NOTIFY_PREFS_KEY) ?? "";

export function useNotifyPrefs(): NotifyPrefs {
  const raw = useSyncExternalStore(subscribe, prefsSnapshot, () => null);
  return useMemo(() => {
    if (raw === null) return DEFAULT_PREFS;
    const s = storage();
    return s ? readPrefs(s) : DEFAULT_PREFS;
  }, [raw]);
}

export function setNotifyPrefs(next: NotifyPrefs) {
  const s = storage();
  if (!s) return;
  writePrefs(s, next);
  listeners.forEach((l) => l());
}

// ----- PULSE.app ------------------------------------------------------------

// PULSE.app (apps/macos) registers this handler for the console's own page:
// it posts Mac notifications and answers what macOS allows. A browser has none.
interface AppHandler {
  postMessage(message: unknown): unknown;
}

export function appNotifier(): AppHandler | null {
  if (typeof window === "undefined") return null;
  const w = window as unknown as { webkit?: { messageHandlers?: Record<string, AppHandler | undefined> } };
  return w.webkit?.messageHandlers?.pulseNotify ?? null;
}

// The app's answer: WebKit returns a promise for a handler that replies.
function toApp(app: AppHandler, message: Record<string, unknown>): Promise<unknown> {
  try {
    return Promise.resolve(app.postMessage(message));
  } catch (e) {
    return Promise.reject(e);
  }
}

// What may notify here: the browser's Notification.permission, or what macOS
// allows PULSE.app ("granted", "denied" or "default" for not asked yet).
// `ask` asks the owner when nobody asked yet; call it from a click.
// What macOS lets PULSE.app do, as the app last said.
let appPermission = "default";

export async function notifyPermission(ask: boolean): Promise<string> {
  const app = appNotifier();
  if (app) {
    try {
      const answer = await toApp(app, { kind: "permission", ask });
      appPermission = typeof answer === "string" ? answer : "default";
    } catch {
      appPermission = "default";
    }
    return appPermission;
  }
  if (typeof Notification === "undefined") return "unsupported";
  if (ask && Notification.permission === "default") {
    try {
      return await Notification.requestPermission();
    } catch {
      return Notification.permission;
    }
  }
  return Notification.permission;
}

// ----- the chime --------------------------------------------------------------

// Browsers start sound only after the owner touched the page, so the context
// is made (or resumed) on the first click or key press and kept.
let audio: AudioContext | null = null;
function unlockAudio() {
  try {
    if (!audio) audio = new AudioContext();
    if (audio.state === "suspended") void audio.resume();
  } catch {
    audio = null;
  }
}

// True when the chime played; false when the browser would not let it.
export function playChime(): boolean {
  if (!audio || audio.state !== "running") return false;
  notes(audio);
  return true;
}

// The settings card's "Play the sound": from a click, so the sound may start.
export function tryChime() {
  unlockAudio();
  if (!audio) return;
  if (audio.state === "running") notes(audio);
  else void audio.resume().then(() => audio && notes(audio)).catch(() => undefined);
}

function notes(audio: AudioContext) {
  const t0 = audio.currentTime + 0.02;
  for (const n of CHIME_NOTES) {
    const osc = audio.createOscillator();
    const gain = audio.createGain();
    osc.type = "sine";
    osc.frequency.value = n.freq;
    gain.gain.setValueAtTime(0.0001, t0 + n.at);
    gain.gain.exponentialRampToValueAtTime(CHIME_GAIN, t0 + n.at + 0.015);
    gain.gain.exponentialRampToValueAtTime(0.0001, t0 + n.at + n.dur);
    osc.connect(gain).connect(audio.destination);
    osc.start(t0 + n.at);
    osc.stop(t0 + n.at + n.dur + 0.05);
  }
}

// ----- the notifier -----------------------------------------------------------

// The conversation or task the page in front shows: news about it is on screen.
function watching(): { chat?: string; task?: string } | null {
  if (document.visibilityState !== "visible" || !document.hasFocus() || !location.pathname.startsWith("/chat")) return null;
  const q = new URLSearchParams(location.search);
  return { chat: q.get("id") || undefined, task: q.get("task") || undefined };
}

// One tab at a time reads and writes what was seen, so two open tabs never
// both tell the owner about the same thing.
function exclusive(run: () => void) {
  const locks = typeof navigator !== "undefined" ? (navigator as Navigator & { locks?: LockManager }).locks : undefined;
  if (!locks) return run();
  void locks.request("pulse.notify", async () => run()).catch(() => undefined);
}

export function OwnerNotifier() {
  const prefs = useNotifyPrefs();
  const on = notifyOn(prefs);
  const router = useRouter();
  const sources = useInboxSources();
  const chat = useApi<ChatListResponse>(on ? "chat" : null, { refresh: CHAT_REFRESH_MS });

  // The input only changes when a source's data does: an item is news once
  // its source loaded. Drafts and gate steps need the project registry too.
  const input = useMemo<NotifyInput>(() => {
    const o = sources.org.data;
    const inbox = buildInbox(sources);
    return {
      brain: o?.totals.brain ?? "",
      ...(o ? { proposals: inbox.proposals, questions: inbox.questions } : {}),
      ...(o && sources.reg.data ? { drafts: inbox.drafts, steps: inbox.steps } : {}),
      ...(sources.proposed.data ? { decisions: inbox.decisions } : {}),
      ...(sources.compute.data ? { compute: inbox.compute } : {}),
      ...(sources.suggested?.data ? { work: inbox.work } : {}),
      ...(chat.data ? { chat: chat.data } : {}),
    };
    // Only the data matters here, not each source's loading flags.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sources.org.data, sources.reg.data, sources.proposed.data, sources.compute.data, sources.suggested?.data, chat.data]);

  // The newest choices and router for the delivery below, without re-running the look.
  const live = useRef({ prefs, router });
  useEffect(() => {
    live.current = { prefs, router };
  }, [prefs, router]);

  // What PULSE.app may do, asked once notifications are on and again
  // whenever the owner comes back (they may have changed it in System Settings).
  useEffect(() => {
    if (!on || !appNotifier()) return;
    const read = () => void notifyPermission(false);
    read();
    window.addEventListener("focus", read);
    return () => window.removeEventListener("focus", read);
  }, [on]);

  useEffect(() => {
    if (!on) return;
    const unlock = () => unlockAudio();
    window.addEventListener("pointerdown", unlock, { capture: true });
    window.addEventListener("keydown", unlock, { capture: true });
    const poll = window.setInterval(() => {
      if (document.hidden) for (const p of POLLED) void load(p);
    }, HIDDEN_POLL_MS);
    return () => {
      window.removeEventListener("pointerdown", unlock, { capture: true });
      window.removeEventListener("keydown", unlock, { capture: true });
      window.clearInterval(poll);
    };
  }, [on]);

  useEffect(() => {
    if (!on) return;
    const loaded = loadedSources(input);
    if (!loaded.length) return;
    const items = notifyItems(input);
    exclusive(() => {
      const s = storage();
      if (!s) return;
      const { prefs: p, router: r } = live.current;
      const plan = planNotices({ items, loaded, seen: readSeen(s), now: Date.now(), prefs: p, watching: watching() });
      if (plan.changed) writeSeen(s, plan.seen);
      if (plan.notices.length) deliver(plan.notices, p, (href) => r.push(href));
    });
  }, [on, input]);

  return null;
}

function deliver(notices: Notice[], prefs: NotifyPrefs, open: (href: string) => void) {
  const app = appNotifier();
  const supported = typeof Notification !== "undefined";
  const focused = document.visibilityState === "visible" && document.hasFocus();
  const permission = app ? appPermission : supported ? Notification.permission : "unsupported";
  const how = delivery({ app: !!app, focused, supported, permission, sound: prefs.sound });
  // One chime for one look, however many notices it brings.
  const chimed = how.chime ? playChime() : false;
  for (const n of notices) {
    if (how.toast) toast(n.title, { id: n.tag, description: n.body, duration: TOAST_MS, action: { label: "Open", onClick: () => open(n.href) } });
    if (how.app && app) void toApp(app, { kind: "notify", tag: n.tag, title: n.title, body: n.body, path: n.href, sound: prefs.sound, focused }).catch(() => undefined);
    if (how.system) {
      try {
        const shown = new Notification(n.title, { body: n.body, tag: n.tag, icon: "/icon.svg", silent: silentNotification(prefs.sound, chimed) });
        shown.onclick = () => {
          window.focus();
          open(n.href);
          shown.close();
        };
      } catch {
        // Some browsers (Chrome on Android) only notify from a service
        // worker; there the chime is all the owner gets in the background.
      }
    }
  }
}
