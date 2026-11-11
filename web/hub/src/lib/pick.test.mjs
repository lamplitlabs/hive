import assert from "node:assert/strict";
import { test } from "node:test";
import { PICK_DEFAULT_TEXT, askForHelper, elementName, pickBlock, pickLabel, pickPlace, pickRequest, pickSnippet, readPickMessage, splitPicks, withPicks } from "./pick.ts";

const picked = (element = {}, over = {}) => ({
  type: "hive-pick-picked",
  url: "http://localhost:3000/settings",
  title: "Settings",
  element: { tag: "BUTTON", id: "save", classes: ["btn", "primary"], text: "  Save\n changes ", html: '<button id="save" class="btn primary">Save changes</button>', selector: "main > form > button", role: "", label: "", href: "", src: "", components: ["SaveButton", "SettingsForm"], rect: { x: 812.4, y: 340, w: 160, h: 40 }, ...element },
  ...over,
});

test("readPickMessage accepts only the helper's messages and cuts every field", () => {
  assert.equal(readPickMessage(null), null);
  assert.equal(readPickMessage("hive-pick-picked"), null);
  assert.equal(readPickMessage({ type: "something-else" }), null);
  assert.equal(readPickMessage({ type: "hive-pick-picked" }), null, "a pick without an element is nothing");
  assert.equal(readPickMessage(picked({ tag: "" })), null, "an element without a tag is nothing");
  assert.deepEqual(readPickMessage({ type: "hive-pick-hello", url: "http://localhost:3000/", title: "Blog" }), { type: "hello", url: "http://localhost:3000/", title: "Blog" });
  assert.deepEqual(readPickMessage({ type: "hive-pick-cancel" }), { type: "cancel" });

  const m = readPickMessage(picked());
  assert.equal(m.type, "picked");
  const p = m.pick;
  assert.equal(p.tag, "button");
  assert.equal(p.id, "save");
  assert.deepEqual(p.classes, ["btn", "primary"]);
  assert.equal(p.text, "Save changes", "text is one line");
  assert.equal(p.url, "http://localhost:3000/settings");
  assert.deepEqual(p.components, ["SaveButton", "SettingsForm"]);
  assert.deepEqual(p.rect, { x: 812, y: 340, w: 160, h: 40 });
  assert.ok(p.key.startsWith("http://localhost:3000/settings|main > form > button|"));

  // Long and odd values are cut, never thrown on.
  const long = readPickMessage(picked({ text: "x".repeat(1000), classes: [1, "ok", null], components: "nope", rect: "nope" })).pick;
  assert.equal(long.text.length, 160);
  assert.ok(long.text.endsWith("…"));
  assert.deepEqual(long.classes, ["ok"]);
  assert.deepEqual(long.components, []);
  assert.deepEqual(long.rect, { x: 0, y: 0, w: 0, h: 0 });
});

test("names, labels and places read like the Copilot app's chips", () => {
  const p = readPickMessage(picked()).pick;
  assert.equal(elementName(p), "button#save.btn.primary");
  assert.equal(pickLabel(p), "button#save.btn.primary “Save changes”");
  assert.equal(pickLabel({ tag: "img", id: "", classes: [], text: "", label: "Logo" }), "img “Logo”");
  assert.equal(pickLabel({ tag: "div", id: "", classes: ["a", "b", "c", "d"], text: "", label: "" }), "div.a.b.c", "three classes at most");
  assert.equal(pickLabel({ tag: "p", id: "", classes: [], text: "a".repeat(60), label: "" }), "p “" + "a".repeat(39) + "…”");
  assert.equal(pickPlace("http://localhost:3000/settings/"), "localhost:3000/settings");
  assert.equal(pickPlace("http://localhost:3000/"), "localhost:3000");
  assert.equal(pickPlace("not a url"), "not a url");
  assert.deepEqual(pickRequest(true), { type: "hive-pick", on: true });
});

test("a pick travels as a fenced block the agent reads and the transcript folds back", () => {
  const p = readPickMessage(picked()).pick;
  const block = pickBlock(p);
  assert.equal(
    block,
    ["```pick", "url: http://localhost:3000/settings", "element: button#save.btn.primary", "text: Save changes", "components: SaveButton < SettingsForm", "selector: main > form > button", "rect: 812,340 160x40", 'html: <button id="save" class="btn primary">Save changes</button>', "```"].join("\n"),
  );
  // Without words the message asks for a polish; with words they come first.
  assert.equal(withPicks("", [p]), PICK_DEFAULT_TEXT + "\n\n" + block);
  assert.equal(withPicks("  make it teal ", [p]), "make it teal\n\n" + block);
  assert.equal(withPicks("plain", []), "plain");
  // Two picks are two blocks.
  const q = readPickMessage(picked({ tag: "a", id: "", classes: [], text: "Docs", href: "/docs", components: [] })).pick;
  const both = withPicks("fix", [p, q]);
  const back = splitPicks(both);
  assert.equal(back.text, "fix");
  assert.deepEqual(back.picks, [
    { url: "http://localhost:3000/settings", element: "button#save.btn.primary", text: "Save changes", selector: "main > form > button" },
    { url: "http://localhost:3000/settings", element: "a", text: "Docs", selector: "main > form > button" },
  ]);
  // A message without picks is untouched; a fence inside the html cannot close the block early.
  assert.deepEqual(splitPicks("hello\n\nworld"), { text: "hello\n\nworld", picks: [] });
  const tricky = readPickMessage(picked({ html: "<pre>```</pre>" })).pick;
  assert.equal(splitPicks(withPicks("", [tricky])).picks.length, 1);
});

test("the helper's snippet names the hub and the ask says where it goes", () => {
  assert.equal(pickSnippet("http://127.0.0.1:8787/"), '<script src="http://127.0.0.1:8787/pick.js"></script>');
  const ask = askForHelper("http://127.0.0.1:8787", "http://localhost:3000/");
  assert.ok(ask.includes('<script src="http://127.0.0.1:8787/pick.js"></script>'));
  assert.ok(ask.includes("http://localhost:3000/"));
  assert.ok(ask.includes("development only"));
});
