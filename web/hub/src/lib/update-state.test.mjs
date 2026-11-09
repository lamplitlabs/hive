import assert from "node:assert/strict";
import { test } from "node:test";
import { UPDATE_DISMISS_KEY, readDismissed, updateBanner, webLink, writeDismissed } from "./update-state.ts";

const base = {
  enabled: true,
  running: "v0.1.0",
  latest: "v0.2.0",
  url: "https://github.com/o/r/releases/tag/v0.2.0",
  checked_at: "2026-11-09T02:00:00Z",
  error: "",
  newer: true,
  notice: "A newer PULSE is available: v0.2.0 (running v0.1.0) - https://github.com/o/r/releases/tag/v0.2.0",
};

test("a newer release shows a banner with its link", () => {
  assert.deepEqual(updateBanner(base), {
    latest: "v0.2.0",
    title: "PULSE v0.2.0 is available",
    detail: "This machine runs v0.1.0.",
    href: "https://github.com/o/r/releases/tag/v0.2.0",
  });
});

test("no banner while the check is off, nothing is newer, or nothing is loaded", () => {
  assert.equal(updateBanner(null), null);
  assert.equal(updateBanner(undefined), null);
  assert.equal(updateBanner({ ...base, enabled: false }), null);
  assert.equal(updateBanner({ ...base, newer: false }), null);
  assert.equal(updateBanner({ ...base, latest: " " }), null);
});

test("a dismissed release stays hidden; the next one shows again", () => {
  assert.equal(updateBanner(base, "v0.2.0"), null);
  assert.equal(updateBanner({ ...base, latest: "v0.3.0" }, "v0.2.0")?.title, "PULSE v0.3.0 is available");
});

test("only http(s) links are offered", () => {
  assert.equal(updateBanner({ ...base, url: "" }).href, null);
  assert.equal(updateBanner({ ...base, url: "javascript:alert(1)" }).href, null);
  assert.equal(webLink("http://127.0.0.1:8080/r"), "http://127.0.0.1:8080/r");
  assert.equal(webLink("https://x.test/a b"), null);
  assert.equal(webLink(null), null);
});

test("the dismissed release lives in this browser's storage", () => {
  const m = new Map();
  const storage = { getItem: (k) => (m.has(k) ? m.get(k) : null), setItem: (k, v) => m.set(k, v) };
  assert.equal(readDismissed(storage), null);
  writeDismissed(storage, "v0.2.0");
  assert.equal(m.get(UPDATE_DISMISS_KEY), "v0.2.0");
  assert.equal(readDismissed(storage), "v0.2.0");
  const broken = {
    getItem() {
      throw new Error("denied");
    },
    setItem() {
      throw new Error("denied");
    },
  };
  assert.equal(readDismissed(broken), null);
  writeDismissed(broken, "v0.2.0");
  assert.equal(readDismissed(null), null);
});
