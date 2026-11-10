// The Sessions page's pure logic (spec "Sessions"): which state icon a row
// in the list shows, the one-time tip under the new-session composer,
// which icon a tool call and a changed file get, and how consecutive tool
// calls fold into one row. Plain TypeScript without React or icons so
// node --test imports it directly; the components map the names returned
// here to lucide icons.

import type { HarnessStep } from "@/lib/api";

// ----- the list -------------------------------------------------------------

// A row's state: working (the spinner), asks (a marigold dot - the agent
// waits on you, or its pull request does), stopped (a marigold pause - Hive
// stopped its turn to restart and nothing carried it on, so it waits for a
// message), done (a tick), open (the quiet speech bubble). Status by icon and
// word, never colour alone.
export type SessionState = "working" | "asks" | "stopped" | "done" | "open";

// The row note Hive leaves on a conversation whose turn it stopped itself
// (RESTART_NOTE in crates/hive-harness/src/conversation.rs); the next turn
// clears it.
export const RESTART_NOTE = "Hive restarted during the last turn";

export interface SessionLike {
  status: string;
  note?: string;
  live?: { status: string } | null;
  pr?: { state: string } | null;
}

// Pull request states that are the owner's turn (task-groups.ts
// PR_NEEDS_OWNER), spelled out here so this file imports nothing at runtime.
const PR_ASKS = new Set(["needs_owner", "review_failed"]);

export function sessionState(c: SessionLike): SessionState {
  const status = c.live?.status || c.status;
  if (c.pr && PR_ASKS.has(c.pr.state)) return "asks";
  if (status === "waiting") return "asks";
  if (status === "running") return "working";
  if (status === "closed" || status === "archived") return "done";
  if (status === "idle" && c.note === RESTART_NOTE) return "stopped";
  return "open";
}

// The sub-sessions a finished session leaves behind: still working, waiting
// on you, or stopped by a restart with nothing carrying them on. A quiet
// (open) or finished one is not counted.
export function unfinished<T extends SessionLike>(subs: T[]): T[] {
  return subs.filter((k) => {
    const s = sessionState(k);
    return s === "working" || s === "asks" || s === "stopped";
  });
}

// The StatusIcon kind each state draws (status-badge.tsx); "open" has no
// status kind and draws MessageSquare muted instead.
export const SESSION_STATE_ICON: Record<SessionState, string | null> = {
  working: "running",
  asks: "waiting",
  stopped: "paused",
  done: "done",
  open: null,
};

export const SESSION_STATE_WORD: Record<SessionState, string> = {
  working: "Working",
  asks: "Asked you",
  stopped: "Stopped by a restart",
  done: "Finished",
  open: "Open",
};

// ----- the one-time tip -----------------------------------------------------

// Where the dismissal lives; `storage` is injectable so tests (and a hub
// rendered without a window) need no localStorage.
export const TIP_KEY = "hive.hub.tip.new-session";

export interface TipStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

function tipStorage(storage?: TipStorage | null): TipStorage | null {
  if (storage) return storage;
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

export function tipDismissed(storage?: TipStorage | null): boolean {
  const s = tipStorage(storage);
  if (!s) return false;
  try {
    return s.getItem(TIP_KEY) === "1";
  } catch {
    return false;
  }
}

export function dismissTip(storage?: TipStorage | null): void {
  const s = tipStorage(storage);
  if (!s) return;
  try {
    s.setItem(TIP_KEY, "1");
  } catch {
    // Private mode or a full store: the tip comes back next time, no harm.
  }
}

// ----- tool calls -----------------------------------------------------------

// The icon name a tool call row shows, by the step's kind (the harness
// records "command" for bash, "fetch" for fetch and web_search, "subagent"
// for subagent and chat) or by the tool's name when a step kind is not
// known. Names are lucide icons; the component resolves them.
export type ToolIconName =
  | "Terminal"
  | "FilePen"
  | "FileText"
  | "Globe"
  | "Search"
  | "Bot"
  | "Wrench";

export function toolIcon(name: string | null | undefined): ToolIconName {
  const k = (name || "").trim().toLowerCase();
  switch (k) {
    case "bash":
    case "shell":
    case "command":
    case "observation":
    case "process":
      return "Terminal";
    case "edit":
    case "write":
      return "FilePen";
    case "read":
      return "FileText";
    case "fetch":
    case "web_search":
    case "browser":
    case "page":
      return "Globe";
    case "grep":
    case "glob":
    case "search":
      return "Search";
    case "subagent":
    case "chat":
    case "task":
      return "Bot";
    default:
      return "Wrench";
  }
}

// What a tool call row says in mono: the command for a shell step, the
// path for a file step, the first non-empty line of the content otherwise.
export function toolSummary(s: {
  kind: string;
  command: string;
  content: string;
  title?: string;
}): string {
  const first = (t: string) =>
    (t || "").split("\n").find((l) => l.trim())?.trim() ?? "";
  if (s.kind === "command") return first(s.command) || s.title || "";
  return first(s.command) || first(s.content) || s.title || "";
}

// "1.2s", "340ms", "2m 5s": how long a call took, from its duration or
// from the result's; nothing under 100 ms is worth printing.
export function toolDuration(ms: number | null | undefined): string {
  const n = Math.max(0, Math.round(ms ?? 0));
  if (n < 100) return "";
  if (n < 1000) return n + "ms";
  const s = Math.round(n / 1000);
  if (s < 60) return (n < 10000 ? (n / 1000).toFixed(1) : String(s)) + "s";
  const m = Math.floor(s / 60);
  return s % 60 ? m + "m " + (s % 60) + "s" : m + "m";
}

// Consecutive tool calls fold under one "N tool calls · Xs" row: a run of
// two or more items that are plain tool calls (not a thought, not a note)
// becomes a group; anything else stands alone. The caller keeps the items'
// rendering; this only decides the folds.
export interface ToolCallLike {
  step: { kind: string; duration_ms: number };
  result: { duration_ms: number } | null;
  thought?: number;
}

export type ToolGroup<T extends ToolCallLike> =
  | { kind: "one"; item: T }
  | { kind: "many"; items: T[]; ms: number };

const TOOL_KINDS = new Set([
  "command",
  "read",
  "grep",
  "glob",
  "edit",
  "write",
  "mcp",
  "skill",
  "fetch",
  "process",
  "memory",
  "search",
  "subagent",
  "task",
  "hook",
]);

export function isToolCall(it: ToolCallLike): boolean {
  return !it.thought && TOOL_KINDS.has(it.step.kind);
}

export function callMs(it: ToolCallLike): number {
  return Math.max(0, it.result?.duration_ms || it.step.duration_ms || 0);
}

export function groupToolCalls<T extends ToolCallLike>(
  items: T[],
  min = 2,
): ToolGroup<T>[] {
  const out: ToolGroup<T>[] = [];
  let run: T[] = [];
  const flush = () => {
    if (run.length >= min)
      out.push({
        kind: "many",
        items: run,
        ms: run.reduce((n, it) => n + callMs(it), 0),
      });
    else for (const it of run) out.push({ kind: "one", item: it });
    run = [];
  };
  for (const it of items) {
    if (isToolCall(it)) {
      run.push(it);
      continue;
    }
    flush();
    out.push({ kind: "one", item: it });
  }
  flush();
  return out;
}

// "3 tool calls · 4.2s"
export function toolGroupLabel(n: number, ms: number): string {
  const d = toolDuration(ms);
  return `${n} tool call${n === 1 ? "" : "s"}` + (d ? ` · ${d}` : "");
}

// ----- changed files --------------------------------------------------------

export type FileIconName =
  | "FileCode"
  | "FileJson"
  | "FileText"
  | "Image"
  | "File";

const CODE = new Set([
  "ts",
  "tsx",
  "rs",
  "js",
  "mjs",
  "cjs",
  "jsx",
  "py",
  "go",
  "sh",
  "css",
  "sql",
  "html",
  "htm",
  "toml",
  "yaml",
  "yml",
]);
const IMAGE = new Set(["png", "jpg", "jpeg", "gif", "webp", "svg", "avif"]);
const TEXT = new Set(["md", "markdown", "txt", "log", "csv", "tsv"]);

export function fileIcon(path: string | null | undefined): FileIconName {
  const name = (path || "").split("/").pop() || "";
  const dot = name.lastIndexOf(".");
  const ext = dot >= 0 ? name.slice(dot + 1).toLowerCase() : "";
  if (ext === "json") return "FileJson";
  if (CODE.has(ext)) return "FileCode";
  if (IMAGE.has(ext)) return "Image";
  if (TEXT.has(ext)) return "FileText";
  return "File";
}

// Steps that count as a HarnessStep tool call, for callers typed on the
// wire shape.
export function stepIsToolCall(s: Pick<HarnessStep, "kind">): boolean {
  return TOOL_KINDS.has(s.kind);
}
