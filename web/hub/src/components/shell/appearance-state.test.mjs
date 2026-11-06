import assert from "node:assert/strict";
import { test } from "node:test";
import {
  APPEARANCE_BOOT_SCRIPT,
  APPEARANCE_KEYS,
  APPEARANCE_VALUES,
  DEFAULT_APPEARANCE,
  applyAppearance,
  readAppearance,
  writeAppearance,
} from "./appearance-state.ts";

const memory = (entries = {}) => {
  const m = new Map(Object.entries(entries));
  return {
    getItem: (k) => (m.has(k) ? m.get(k) : null),
    setItem: (k, v) => void m.set(k, v),
    removeItem: (k) => void m.delete(k),
    entries: () => Object.fromEntries(m),
  };
};

test("an empty or missing storage reads as the defaults", () => {
  assert.deepEqual(readAppearance(memory()), DEFAULT_APPEARANCE);
  assert.deepEqual(readAppearance(null), DEFAULT_APPEARANCE);
  assert.deepEqual(readAppearance(undefined), DEFAULT_APPEARANCE);
  assert.deepEqual(DEFAULT_APPEARANCE, { density: "comfortable", font: "geist", text: "default" });
});

test("stored choices are read back under their keys", () => {
  const stored = memory({
    [APPEARANCE_KEYS.density]: "compact",
    [APPEARANCE_KEYS.font]: "readable",
    [APPEARANCE_KEYS.text]: "larger",
  });
  assert.deepEqual(readAppearance(stored), { density: "compact", font: "readable", text: "larger" });
  assert.deepEqual(readAppearance(memory({ [APPEARANCE_KEYS.font]: "readable" })), { ...DEFAULT_APPEARANCE, font: "readable" });
});

test("an unknown stored word, or a storage that throws, falls back to the default", () => {
  const odd = memory({
    [APPEARANCE_KEYS.density]: "cosy",
    [APPEARANCE_KEYS.font]: "",
    [APPEARANCE_KEYS.text]: "LARGER",
  });
  assert.deepEqual(readAppearance(odd), DEFAULT_APPEARANCE);
  const throwing = {
    getItem() {
      throw new Error("blocked");
    },
  };
  assert.deepEqual(readAppearance(throwing), DEFAULT_APPEARANCE);
});

test("the keys and the accepted values are the three choices", () => {
  assert.deepEqual(APPEARANCE_KEYS, { density: "pulse.density", font: "pulse.font", text: "pulse.text" });
  assert.deepEqual(Object.keys(APPEARANCE_VALUES).sort(), ["density", "font", "text"]);
  for (const [choice, values] of Object.entries(APPEARANCE_VALUES)) {
    assert.ok(values.includes(DEFAULT_APPEARANCE[choice]), `${choice} default is accepted`);
  }
});

test("applying sets a data attribute for a choice and removes it for a default", () => {
  const root = { dataset: {} };
  applyAppearance(root, { density: "compact", font: "readable", text: "larger" });
  assert.deepEqual(root.dataset, { density: "compact", font: "readable", text: "larger" });
  applyAppearance(root, { density: "comfortable", font: "readable", text: "default" });
  assert.deepEqual(root.dataset, { font: "readable" });
  assert.equal("density" in root.dataset, false, "a default deletes its attribute rather than writing a word");
  applyAppearance(root, DEFAULT_APPEARANCE);
  assert.deepEqual(root.dataset, {});
});

test("applying leaves other data attributes on <html> alone", () => {
  const root = { dataset: { awake: "holding", theme: "dark" } };
  applyAppearance(root, { ...DEFAULT_APPEARANCE, density: "compact" });
  assert.deepEqual(root.dataset, { awake: "holding", theme: "dark", density: "compact" });
});

test("writing stores a choice and removes the key for a default", () => {
  const storage = memory({ [APPEARANCE_KEYS.density]: "compact", "pulse.sidebar": "collapsed" });
  writeAppearance(storage, { density: "comfortable", font: "readable", text: "larger" });
  assert.deepEqual(storage.entries(), {
    "pulse.sidebar": "collapsed",
    [APPEARANCE_KEYS.font]: "readable",
    [APPEARANCE_KEYS.text]: "larger",
  });
  writeAppearance(storage, DEFAULT_APPEARANCE);
  assert.deepEqual(storage.entries(), { "pulse.sidebar": "collapsed" });
});

test("what is written reads back the same", () => {
  const storage = memory();
  const chosen = { density: "compact", font: "geist", text: "larger" };
  writeAppearance(storage, chosen);
  assert.deepEqual(readAppearance(storage), chosen);
});

test("the boot script names each key and each data attribute, and swallows storage errors", () => {
  for (const key of Object.values(APPEARANCE_KEYS)) {
    assert.ok(APPEARANCE_BOOT_SCRIPT.includes(JSON.stringify(key)), `names ${key}`);
  }
  for (const attribute of ["density", "font", "text"]) {
    assert.ok(APPEARANCE_BOOT_SCRIPT.includes(JSON.stringify(attribute)), `names data-${attribute}`);
  }
  for (const value of ["compact", "readable", "larger"]) {
    assert.ok(APPEARANCE_BOOT_SCRIPT.includes(JSON.stringify(value)), `accepts ${value}`);
  }
  assert.ok(APPEARANCE_BOOT_SCRIPT.startsWith("(function(){try{"));
  assert.ok(APPEARANCE_BOOT_SCRIPT.endsWith("catch(e){}})()"));
  assert.ok(APPEARANCE_BOOT_SCRIPT.includes("document.documentElement"));
});

test("the boot script, run against a fake document, marks <html> like applyAppearance does", () => {
  const run = (stored) => {
    const dataset = {};
    const fn = new Function(
      "document",
      "localStorage",
      `${APPEARANCE_BOOT_SCRIPT}; return document.documentElement.dataset;`,
    );
    return fn({ documentElement: { dataset } }, memory(stored));
  };
  assert.deepEqual(run({}), {});
  assert.deepEqual(run({ [APPEARANCE_KEYS.density]: "compact", [APPEARANCE_KEYS.text]: "larger" }), { density: "compact", text: "larger" });
  assert.deepEqual(run({ [APPEARANCE_KEYS.font]: "comic" }), {}, "an unknown word is not applied");
  assert.deepEqual(run({ [APPEARANCE_KEYS.font]: "geist" }), {}, "a default word is not applied");
  const throwing = new Function("document", "localStorage", `${APPEARANCE_BOOT_SCRIPT}; return "survived";`);
  assert.equal(
    throwing(
      { documentElement: { dataset: {} } },
      {
        getItem() {
          throw new Error("blocked");
        },
      },
    ),
    "survived",
  );
});
