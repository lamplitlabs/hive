import assert from "node:assert/strict";
import { test } from "node:test";
import {
  RESTART_NOTE,
  SESSION_STATE_ICON,
  TIP_KEY,
  dismissTip,
  fileIcon,
  groupToolCalls,
  sessionState,
  tipDismissed,
  toolDuration,
  toolGroupLabel,
  toolIcon,
  toolSummary,
  unfinished,
} from "./session-state.ts";

const conv = (over = {}) => ({ status: "idle", live: null, pr: null, ...over });

test("a session's state: the live status wins, a waiting pull request asks", () => {
  assert.equal(sessionState(conv({ status: "running" })), "working");
  assert.equal(sessionState(conv({ status: "idle", live: { status: "running" } })), "working");
  assert.equal(sessionState(conv({ status: "waiting" })), "asks");
  assert.equal(sessionState(conv({ status: "running", live: { status: "waiting" } })), "asks");
  assert.equal(sessionState(conv({ status: "idle", pr: { state: "needs_owner" } })), "asks");
  assert.equal(sessionState(conv({ status: "running", pr: { state: "review_failed" } })), "asks");
  assert.equal(sessionState(conv({ status: "running", pr: { state: "open" } })), "working");
  assert.equal(sessionState(conv({ status: "closed" })), "done");
  assert.equal(sessionState(conv({ status: "archived" })), "done");
  assert.equal(sessionState(conv({ status: "idle" })), "open");
  assert.equal(sessionState(conv({ status: "ended" })), "open");
  assert.equal(sessionState(conv({ status: "idle", live: undefined })), "open");
});

test("each state names a status icon kind, open none", () => {
  assert.equal(SESSION_STATE_ICON.working, "running");
  assert.equal(SESSION_STATE_ICON.asks, "waiting");
  assert.equal(SESSION_STATE_ICON.stopped, "paused");
  assert.equal(SESSION_STATE_ICON.done, "done");
  assert.equal(SESSION_STATE_ICON.open, null);
});

test("a turn PULSE stopped to restart, with nothing carrying it on, reads as stopped", () => {
  assert.equal(sessionState(conv({ status: "idle", note: RESTART_NOTE })), "stopped");
  // The next turn clears the note; until then a live turn still wins.
  assert.equal(sessionState(conv({ status: "idle", note: RESTART_NOTE, live: { status: "running" } })), "working");
  assert.equal(sessionState(conv({ status: "closed", note: RESTART_NOTE })), "done");
  assert.equal(sessionState(conv({ status: "idle", note: "waiting on the reviewer" })), "open");
});

test("a finished session's unfinished sub-sessions: working, asking or stopped", () => {
  const subs = [
    conv({ id: "a", status: "closed" }),
    conv({ id: "b", status: "idle", note: RESTART_NOTE }),
    conv({ id: "c", status: "running" }),
    conv({ id: "d", status: "waiting" }),
    conv({ id: "e", status: "idle" }),
  ];
  assert.deepEqual(
    unfinished(subs).map((k) => k.id),
    ["b", "c", "d"],
  );
  assert.deepEqual(unfinished([]), []);
});

test("the tip stays until dismissed, in the injected storage under its key", () => {
  const store = new Map();
  const storage = {
    getItem: (k) => (store.has(k) ? store.get(k) : null),
    setItem: (k, v) => store.set(k, v),
  };
  assert.equal(tipDismissed(storage), false);
  dismissTip(storage);
  assert.equal(tipDismissed(storage), true);
  assert.equal(store.get(TIP_KEY), "1");
  assert.equal(TIP_KEY, "pulse.hub.tip.new-session");
  // No storage at all (no window): shown, and dismissing is a no-op.
  assert.equal(tipDismissed(null), false);
  assert.doesNotThrow(() => dismissTip(null));
  // A storage that throws (private mode) neither shows as dismissed nor breaks.
  const broken = {
    getItem: () => {
      throw new Error("denied");
    },
    setItem: () => {
      throw new Error("denied");
    },
  };
  assert.equal(tipDismissed(broken), false);
  assert.doesNotThrow(() => dismissTip(broken));
});

test("a tool call's icon follows its kind or its tool name", () => {
  assert.equal(toolIcon("bash"), "Terminal");
  assert.equal(toolIcon("shell"), "Terminal");
  assert.equal(toolIcon("command"), "Terminal");
  assert.equal(toolIcon("edit"), "FilePen");
  assert.equal(toolIcon("write"), "FilePen");
  assert.equal(toolIcon("read"), "FileText");
  assert.equal(toolIcon("fetch"), "Globe");
  assert.equal(toolIcon("web_search"), "Globe");
  assert.equal(toolIcon("browser"), "Globe");
  assert.equal(toolIcon("grep"), "Search");
  assert.equal(toolIcon("glob"), "Search");
  assert.equal(toolIcon("subagent"), "Bot");
  assert.equal(toolIcon("chat"), "Bot");
  assert.equal(toolIcon("mcp"), "Wrench");
  assert.equal(toolIcon("Bash"), "Terminal");
  assert.equal(toolIcon(""), "Wrench");
  assert.equal(toolIcon(null), "Wrench");
});

test("a tool call's summary is the command, the path, or the first line", () => {
  assert.equal(toolSummary({ kind: "command", command: "ls -la\n", content: "out" }), "ls -la");
  assert.equal(toolSummary({ kind: "command", command: "", content: "", title: "List files" }), "List files");
  assert.equal(toolSummary({ kind: "read", command: "src/a.ts", content: "" }), "src/a.ts");
  assert.equal(toolSummary({ kind: "mcp", command: "", content: "\n\n  first line\nsecond" }), "first line");
  assert.equal(toolSummary({ kind: "mcp", command: "", content: "" }), "");
});

test("durations print compactly and skip the trivial", () => {
  assert.equal(toolDuration(0), "");
  assert.equal(toolDuration(80), "");
  assert.equal(toolDuration(340), "340ms");
  assert.equal(toolDuration(1200), "1.2s");
  assert.equal(toolDuration(12400), "12s");
  assert.equal(toolDuration(65000), "1m 5s");
  assert.equal(toolDuration(120000), "2m");
  assert.equal(toolDuration(null), "");
});

const call = (kind, ms = 0, over = {}) => ({ step: { kind, duration_ms: ms }, result: null, ...over });

test("consecutive tool calls fold into one group; thoughts and notes stand alone", () => {
  const items = [
    call("command", 1000),
    call("read", 200, { result: { duration_ms: 500 } }),
    call("note"),
    call("edit", 100),
    call("assistant", 0, { thought: 3 }),
    call("grep", 50),
    call("glob", 50),
    call("fetch", 400),
  ];
  const g = groupToolCalls(items);
  assert.deepEqual(
    g.map((x) => (x.kind === "many" ? ["many", x.items.length, x.ms] : ["one", x.item.step.kind])),
    [["many", 2, 1500], ["one", "note"], ["one", "edit"], ["one", "assistant"], ["many", 3, 500]],
  );
  // The result's duration is the call's when it has one.
  assert.equal(g[0].ms, 1500);
  // A single call is never a group; an empty list folds to nothing.
  assert.deepEqual(groupToolCalls([call("command", 5)]).map((x) => x.kind), ["one"]);
  assert.deepEqual(groupToolCalls([]), []);
});

test("the group row says how many and how long", () => {
  assert.equal(toolGroupLabel(3, 4200), "3 tool calls · 4.2s");
  assert.equal(toolGroupLabel(1, 0), "1 tool call");
  assert.equal(toolGroupLabel(2, 50), "2 tool calls");
});

test("a changed file's icon follows its extension", () => {
  assert.equal(fileIcon("src/a.ts"), "FileCode");
  assert.equal(fileIcon("src/a.tsx"), "FileCode");
  assert.equal(fileIcon("crates/x/src/lib.rs"), "FileCode");
  assert.equal(fileIcon("a.js"), "FileCode");
  assert.equal(fileIcon("a.mjs"), "FileCode");
  assert.equal(fileIcon("a.py"), "FileCode");
  assert.equal(fileIcon("a.go"), "FileCode");
  assert.equal(fileIcon("package.json"), "FileJson");
  assert.equal(fileIcon("README.md"), "FileText");
  assert.equal(fileIcon("notes.txt"), "FileText");
  assert.equal(fileIcon("logo.png"), "Image");
  assert.equal(fileIcon("photo.JPG"), "Image");
  assert.equal(fileIcon("icon.svg"), "Image");
  assert.equal(fileIcon("Makefile"), "File");
  assert.equal(fileIcon("dir.v2/LICENSE"), "File");
  assert.equal(fileIcon(""), "File");
  assert.equal(fileIcon(null), "File");
});
