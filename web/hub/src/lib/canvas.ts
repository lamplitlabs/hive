// The canvas beside a conversation or a task: the tabs the agent's steps
// open on their own (the file it edits, the terminal it runs commands in,
// the page it fetched or drove in a browser, its plan) and the ones you
// open from the "+" menu.
// Pure rules, so the component stays a view; components/chat/canvas.tsx
// draws them.

import type { HarnessStep } from "@/lib/api";

export type CanvasKind =
  | "terminal"
  | "file"
  | "page"
  | "plan"
  | "changes"
  | "diff"
  | "artifacts"
  | "artifact"
  | "subs"
  | "project"
  | "links"
  | "steps"
  | "brief";

export interface CanvasTab {
  /** `kind` alone, or `kind:key` for a file, a page or an artifact. */
  id: string;
  kind: CanvasKind;
  /** The path, URL, query or artifact name; empty for the one-of-a-kind tabs. */
  key: string;
  label: string;
}

// The tabs that read better with room: the canvas column grows while one
// of these is in front, the way the Copilot app widens for a file.
export const WIDE_KINDS: ReadonlySet<CanvasKind> = new Set<CanvasKind>([
  "terminal",
  "file",
  "page",
  "diff",
  "artifact",
]);

export const KIND_LABEL: Record<CanvasKind, string> = {
  terminal: "Terminal",
  file: "File",
  page: "Page",
  plan: "Plan",
  changes: "Changes",
  diff: "All changes",
  artifacts: "Artifacts",
  artifact: "Artifact",
  subs: "Sub-conversations",
  project: "Project",
  links: "Links",
  steps: "Steps",
  brief: "Brief",
};

export const tabId = (kind: CanvasKind, key = ""): string =>
  key ? kind + ":" + key : kind;

export function fileName(path: string): string {
  const p = path.replace(/\/+$/, "");
  return p.slice(p.lastIndexOf("/") + 1) || p;
}

// "docs.rs/reqwest" for a URL, "Search: retry after" for a query.
export function pageLabel(key: string): string {
  if (!/^https?:\/\//i.test(key)) return "Search: " + key;
  try {
    const u = new URL(key);
    const host = u.hostname.replace(/^www\./, "");
    const first = u.pathname.split("/").filter(Boolean)[0];
    return first ? host + "/" + first : host;
  } catch {
    return key;
  }
}

export function makeTab(kind: CanvasKind, key = ""): CanvasTab {
  const label =
    kind === "file"
      ? fileName(key)
      : kind === "page"
        ? pageLabel(key)
        : kind === "artifact"
          ? key
          : KIND_LABEL[kind];
  return { id: tabId(kind, key), kind, key, label };
}

// ----- what a step touches --------------------------------------------------

const ownStep = (s: HarnessStep) => !s.depth;

// The one file an edit or a write changes: the first line of the step,
// without the write's "(N bytes)" or a fixture's ": 1 block replaced".
export function stepPath(s: HarnessStep): string {
  if (s.kind !== "edit" && s.kind !== "write") return "";
  const first = (s.content || s.command || "").split("\n")[0].trim();
  return first
    .replace(/\s+\(\d+ bytes\)$/, "")
    .split(/:\s/)[0]
    .trim();
}

// The files a read step opens, without their line ranges.
export function readPaths(s: HarnessStep): string[] {
  if (s.kind !== "read") return [];
  return (s.content || s.command || "")
    .split("\n")
    .map((l) => l.trim().replace(/:\d+(-\d+)?$/, ""))
    .filter(Boolean);
}

// The URL a fetch, the query a search, or the page a browser tool opened
// (an MCP `browser_navigate` call: Playwright's) that a page tab shows.
export function stepPage(s: HarnessStep): string {
  if (s.kind === "mcp") return browserPage(s);
  if (s.kind !== "fetch" && s.kind !== "search") return "";
  return (s.content || s.command || "").split("\n")[0].trim();
}

// The URL an MCP browser tool was sent to. The step reads `mcp__<server>__
// browser_navigate {"url":"…"}`; the arguments may be cut short, so the url
// is read on its own rather than parsed as a whole.
export function browserPage(s: HarnessStep): string {
  if (s.kind !== "mcp" || !/__browser_navigate$/.test(s.command || "")) return "";
  const m = /"url"\s*:\s*"((?:[^"\\]|\\.)*)"/.exec(s.content || "");
  if (!m) return "";
  try {
    const url = JSON.parse('"' + m[1] + '"') as string;
    return /^https?:\/\//i.test(url) ? url : "";
  } catch {
    return "";
  }
}

// Edits and writes, newest first, one entry per path.
export function touchedFiles(steps: HarnessStep[]): string[] {
  const out: string[] = [];
  for (let i = steps.length - 1; i >= 0; i--) {
    const s = steps[i];
    const paths = s.kind === "read" ? readPaths(s) : [stepPath(s)];
    for (const p of paths) if (p && !out.includes(p)) out.push(p);
  }
  return out;
}

// Pages fetched and searches run, newest first.
export function visitedPages(steps: HarnessStep[]): string[] {
  const out: string[] = [];
  for (let i = steps.length - 1; i >= 0; i--) {
    const p = stepPage(steps[i]);
    if (p && !out.includes(p)) out.push(p);
  }
  return out;
}

export function latestPlanStep(steps: HarnessStep[]): HarnessStep | null {
  for (let i = steps.length - 1; i >= 0; i--)
    if (steps[i].kind === "plan" && ownStep(steps[i])) return steps[i];
  return null;
}

// ----- the terminal ---------------------------------------------------------

export interface TerminalEntry {
  n: number;
  at: string;
  /** The command line, or the process action ("output 3"). */
  command: string;
  /** What the agent said the command does, when it said. */
  title: string;
  kind: "command" | "process";
  output: string;
  exit_code: number | null;
  duration_ms: number;
  /** No observation yet: the command is still running. */
  running: boolean;
}

// Each command with the observation that followed it, in order. A step
// from a subagent stays out: its own terminal would be another story.
export function terminalEntries(
  steps: HarnessStep[],
  live: boolean,
): TerminalEntry[] {
  const out: TerminalEntry[] = [];
  for (let i = 0; i < steps.length; i++) {
    const s = steps[i];
    if (!ownStep(s) || (s.kind !== "command" && s.kind !== "process")) continue;
    let obs: HarnessStep | null = null;
    for (let j = i + 1; j < steps.length; j++) {
      const t = steps[j];
      if (!ownStep(t)) continue;
      if (t.kind === "observation") obs = t;
      break;
    }
    const last =
      i === steps.length - 1 || steps.slice(i + 1).every((t) => !ownStep(t));
    out.push({
      n: s.n,
      at: s.at,
      command: (s.command || s.content || "").trim(),
      title: s.title || "",
      kind: s.kind,
      output: obs?.content ?? "",
      exit_code: obs ? obs.exit_code : null,
      duration_ms: obs?.duration_ms ?? 0,
      running: !obs && live && last,
    });
  }
  return out;
}

// What a fetched page or a search left: the observation after the step.
export function pageText(
  steps: HarnessStep[],
  key: string,
): { text: string; at: string; exit_code: number | null } | null {
  for (let i = steps.length - 1; i >= 0; i--) {
    const s = steps[i];
    if (stepPage(s) !== key) continue;
    for (let j = i + 1; j < steps.length; j++) {
      const t = steps[j];
      if (!ownStep(t) && ownStep(s)) continue;
      if (t.kind === "observation")
        return { text: t.content, at: t.at, exit_code: t.exit_code };
      break;
    }
    return { text: "", at: s.at, exit_code: null };
  }
  return null;
}

// The search/replace blocks the agent wrote for one path, oldest first:
// what a file tab shows when the workspace is gone.
export function editBlocks(
  steps: HarnessStep[],
  path: string,
): { n: number; at: string; search: string; replace: string }[] {
  const out: { n: number; at: string; search: string; replace: string }[] = [];
  for (const s of steps) {
    if (s.kind !== "edit" || stepPath(s) !== path) continue;
    const re =
      /<<<<<<< SEARCH\n([\s\S]*?)\n=======\n([\s\S]*?)\n>>>>>>> REPLACE/g;
    let m: RegExpExecArray | null;
    while ((m = re.exec(s.content)) !== null)
      out.push({ n: s.n, at: s.at, search: m[1], replace: m[2] });
  }
  return out;
}

// ----- opening tabs from steps ----------------------------------------------

export interface AutoOpen {
  /** Tabs to add (or raise) in order. */
  open: CanvasTab[];
  /** The tab the latest step points at, to bring to the front. */
  front: string | null;
}

// How many files and pages the first look at an old run opens by itself;
// the rest stay a menu away.
const FIRST_FILES = 3;
const FIRST_PAGES = 1;

// Which tabs steps after `after` (a step number) open. Edits and writes
// open the file, commands the terminal, fetches, searches and a browser
// tool's navigations the page, a plan the plan; the newest such step is the
// one brought to the front. On
// the first pass over a run (`after` 0) only the last few files and pages
// open, so an old run does not greet you with twenty tabs.
export function autoOpen(steps: HarnessStep[], after: number): AutoOpen {
  const fresh = steps.filter((s) => s.n > after && ownStep(s));
  const first = after === 0;
  const open: CanvasTab[] = [];
  let front: string | null = null;
  const add = (t: CanvasTab) => {
    if (!open.some((o) => o.id === t.id)) open.push(t);
  };
  let files = 0;
  let pages = 0;
  for (let i = fresh.length - 1; i >= 0; i--) {
    const s = fresh[i];
    let tab: CanvasTab | null = null;
    if (s.kind === "edit" || s.kind === "write") {
      const p = stepPath(s);
      if (!p) continue;
      const isNew = !open.some((o) => o.id === tabId("file", p));
      if (first && isNew && files >= FIRST_FILES) continue;
      if (isNew) files++;
      tab = makeTab("file", p);
    } else if (s.kind === "command" || s.kind === "process") {
      tab = makeTab("terminal");
    } else if (s.kind === "fetch" || s.kind === "search" || s.kind === "mcp") {
      const p = stepPage(s);
      if (!p) continue;
      const isNew = !open.some((o) => o.id === tabId("page", p));
      if (first && isNew && pages >= FIRST_PAGES) continue;
      if (isNew) pages++;
      tab = makeTab("page", p);
    } else if (s.kind === "plan") {
      tab = makeTab("plan");
    }
    if (!tab) continue;
    if (front === null && tab.kind !== "plan") front = tab.id;
    add(tab);
  }
  // A plan alone (no file, command or page since) is what comes forward.
  if (front === null && open.some((o) => o.kind === "plan")) front = "plan";
  open.reverse();
  return { open, front };
}

// ----- links ----------------------------------------------------------------

export interface CanvasLink {
  url: string;
  label: string;
}

const URL_RE = /https?:\/\/[^\s<>"'`)\]}]+/g;

export function linkLabel(url: string): string {
  const gh =
    /^https?:\/\/github\.com\/([^/]+\/[^/]+)\/(pull|issues|discussions)\/(\d+)/.exec(
      url,
    );
  if (gh) return gh[1] + "#" + gh[3];
  try {
    const u = new URL(url);
    const host = u.hostname.replace(/^www\./, "");
    const path = u.pathname.replace(/\/+$/, "");
    const short = path.length > 40 ? "…" + path.slice(-39) : path;
    return host + short;
  } catch {
    return url;
  }
}

// Every link the texts mention, newest text first, each once; the pulse
// hub's own pages are left out (they are a click away already).
export function extractLinks(texts: string[], ownHost = ""): CanvasLink[] {
  const seen = new Set<string>();
  const out: CanvasLink[] = [];
  for (let i = texts.length - 1; i >= 0; i--) {
    const matches = (texts[i] || "").match(URL_RE) ?? [];
    for (const raw of matches) {
      const url = raw.replace(/[.,;:!?]+$/, "");
      if (seen.has(url)) continue;
      if (ownHost && url.includes("//" + ownHost)) continue;
      seen.add(url);
      out.push({ url, label: linkLabel(url) });
    }
  }
  return out;
}

// ----- the tab list ---------------------------------------------------------

// Adds the tabs that are not there yet, in place of nothing else: a tab the
// owner closed and the agent touches again comes back at the end.
export function addTabs(cur: CanvasTab[], add: CanvasTab[]): CanvasTab[] {
  const next = [...cur];
  for (const t of add) if (!next.some((o) => o.id === t.id)) next.push(t);
  return next;
}

// Which tab is in front after one closes: the neighbour on the left, or the
// first that remains.
export function afterClose(
  tabs: CanvasTab[],
  active: string,
  closing: string,
): { tabs: CanvasTab[]; active: string } {
  const i = tabs.findIndex((t) => t.id === closing);
  if (i < 0) return { tabs, active };
  const next = tabs.filter((t) => t.id !== closing);
  if (active !== closing) return { tabs: next, active };
  const pick = next[Math.max(0, i - 1)];
  return { tabs: next, active: pick ? pick.id : "" };
}

// The new-line counts a diff's hunks add, to mark those lines in a file tab.
// `lines` are a unified diff's lines of one file (lib/diff's DiffFile).
export function changedNewLines(
  lines: { kind: string; text: string }[],
): Set<number> {
  const out = new Set<number>();
  let n = 0;
  for (const l of lines) {
    if (l.kind === "hunk") {
      const m = /\+(\d+)/.exec(l.text);
      n = m ? Number(m[1]) : n;
      continue;
    }
    if (l.kind === "meta") continue;
    if (l.kind === "del") continue;
    if (l.kind === "add") out.add(n);
    n++;
  }
  return out;
}
