import assert from "node:assert/strict";
import { test } from "node:test";
import {
  SIDEBAR_BOOT_SCRIPT,
  SIDEBAR_CLASS,
  SIDEBAR_COLLAPSED,
  SIDEBAR_KEY,
  applySidebarClass,
  readSidebarCollapsed,
  sidebarToggleLabel,
  writeSidebarCollapsed,
} from "./sidebar-state.ts";

const memory = (entries = {}) => {
  const m = new Map(Object.entries(entries));
  return { getItem: (k) => (m.has(k) ? m.get(k) : null), setItem: (k, v) => void m.set(k, v), removeItem: (k) => void m.delete(k), keys: () => [...m.keys()] };
};

test("the sidebar starts open and only the one stored word collapses it", () => {
  assert.equal(readSidebarCollapsed(memory()), false);
  assert.equal(readSidebarCollapsed(memory({ [SIDEBAR_KEY]: SIDEBAR_COLLAPSED })), true);
  assert.equal(readSidebarCollapsed(memory({ [SIDEBAR_KEY]: "open" })), false);
  assert.equal(readSidebarCollapsed(memory({ [SIDEBAR_KEY]: "yes" })), false, "an unknown value reads as the default");
});

test("collapsing writes the key and opening removes it, so open and untouched read the same", () => {
  const storage = memory();
  writeSidebarCollapsed(storage, true);
  assert.deepEqual(storage.keys(), ["pulse.sidebar"]);
  assert.equal(readSidebarCollapsed(storage), true);
  writeSidebarCollapsed(storage, false);
  assert.deepEqual(storage.keys(), []);
  assert.equal(readSidebarCollapsed(storage), false);
});

test("the html class follows the choice", () => {
  const classes = new Set();
  const list = { add: (t) => void classes.add(t), remove: (t) => void classes.delete(t) };
  applySidebarClass(list, true);
  assert.deepEqual([...classes], [SIDEBAR_CLASS]);
  applySidebarClass(list, false);
  assert.deepEqual([...classes], []);
  applySidebarClass(list, false);
  assert.deepEqual([...classes], [], "removing twice is harmless");
});

test("the boot script marks <html> before paint only when the stored choice is collapsed, and survives a broken storage", () => {
  const run = (storage) => {
    const classes = new Set();
    const fn = new Function("localStorage", "document", SIDEBAR_BOOT_SCRIPT);
    fn(storage, { documentElement: { classList: { add: (t) => void classes.add(t) } } });
    return [...classes];
  };
  assert.deepEqual(run(memory()), []);
  assert.deepEqual(run(memory({ [SIDEBAR_KEY]: SIDEBAR_COLLAPSED })), [SIDEBAR_CLASS]);
  assert.deepEqual(
    run({
      getItem() {
        throw new Error("SecurityError");
      },
    }),
    [],
    "private mode: the sidebar starts open",
  );
  assert.ok(!SIDEBAR_BOOT_SCRIPT.includes("</script"), "safe to inline");
});

test("the toggle names what a press does", () => {
  assert.equal(sidebarToggleLabel(false), "Collapse the sidebar");
  assert.equal(sidebarToggleLabel(true), "Expand the sidebar");
});
