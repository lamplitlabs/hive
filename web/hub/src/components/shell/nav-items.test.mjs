import assert from "node:assert/strict";
import { test } from "node:test";
import { MORE_GROUPS, NAV_GROUPS, NAV_ITEMS, PRIMARY_ITEMS, SETTINGS_ITEM, activeItem, machineName, shortcutWords } from "./nav-items.ts";
import { GO } from "../../lib/shortcuts.ts";

test("four groups in the spec's order: no label, Knowledge, Projects is rendered live, System", () => {
  assert.deepEqual(
    NAV_GROUPS.map((g) => g.label),
    [null, "Knowledge", "System"],
  );
  assert.deepEqual(
    NAV_GROUPS[0].items.map((i) => i.label),
    ["Home", "For you", "Sessions", "Tasks"],
  );
  assert.deepEqual(
    NAV_GROUPS[1].items.map((i) => i.label),
    ["Records", "Reports", "Votes", "Memory"],
  );
  assert.deepEqual(
    NAV_GROUPS[2].items.map((i) => i.label),
    ["Runs", "Machines", "Connections", "Automations", "Insights"],
  );
});

test("every href is unique, trailing-slashed (static export) and every item has a hint and an icon", () => {
  const hrefs = [...NAV_ITEMS, SETTINGS_ITEM].map((i) => i.href);
  assert.equal(new Set(hrefs).size, hrefs.length, "no two items share an href");
  for (const i of [...NAV_ITEMS, SETTINGS_ITEM]) {
    assert.ok(i.href === "/" || i.href.endsWith("/"), i.href + " ends with a slash");
    assert.ok(i.hint.length > 10, i.label + " has a hint");
    assert.ok(i.icon, i.label + " names an icon");
  }
});

test("the four G-shortcuts map to the hrefs lib/shortcuts.ts navigates to", () => {
  const byKey = Object.fromEntries(NAV_ITEMS.filter((i) => i.shortcut).map((i) => [i.shortcut.toLowerCase(), i.href]));
  assert.deepEqual(byKey, GO);
  assert.equal(shortcutWords(NAV_ITEMS[0]), "G then H");
  assert.equal(shortcutWords({ shortcut: undefined }), null);
});

test("each item matches its own route and nothing else's", () => {
  for (const i of NAV_ITEMS) {
    assert.ok(i.match(i.href), i.label + " matches its own href");
    assert.ok(i.match(i.href.replace(/\/$/, "") || "/"), i.label + " matches without the slash");
    for (const other of NAV_ITEMS) {
      if (other === i) continue;
      assert.ok(!other.match(i.href), other.label + " does not match " + i.href);
    }
  }
  assert.equal(activeItem("/records/")?.label, "Records");
  assert.equal(activeItem("/code-deliveries/")?.label, "Records", "code deliveries live under Records");
  assert.equal(activeItem("/reports/")?.label, "Reports");
  assert.equal(activeItem("/reports/?project=demo-app")?.label, "Reports");
  assert.equal(activeItem("/settings/")?.label, "Settings");
  assert.equal(activeItem("/project/"), undefined, "a project page lights no item");
  assert.equal(activeItem("/")?.label, "Home");
});

test("the phone bar takes the first group and More the rest", () => {
  assert.deepEqual(PRIMARY_ITEMS.map((i) => i.label), ["Home", "For you", "Sessions", "Tasks"]);
  assert.deepEqual(MORE_GROUPS.map((g) => g.label), ["Knowledge", "System"]);
});

test("the machine name tidies the host and falls back", () => {
  assert.equal(machineName("manishs-mac"), "manishs mac");
  assert.equal(machineName("Manishs-MacBook-Pro.local"), "Manishs MacBook Pro");
  assert.equal(machineName(""), "This machine");
  assert.equal(machineName(undefined), "This machine");
  assert.equal(machineName("  "), "This machine");
});
