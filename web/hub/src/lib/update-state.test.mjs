import assert from "node:assert/strict";
import { test } from "node:test";
import { UPDATE_DISMISS_KEY, readDismissed, updateApplyView, updateBanner, webLink, writeDismissed } from "./update-state.ts";

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

const idle = { state: "idle", at: "", actor: "", from: "", latest: "", error: "" };
const onRelease = { ...base, channel: "release", apply: idle };

test("on the release channel the banner offers Update now", () => {
  const v = updateApplyView(onRelease);
  assert.equal(v.button, "Update now");
  assert.equal(v.busy, false);
  assert.equal(v.line, null);
  assert.equal(v.installing, false);
  assert.match(v.buttonTitle, /SHA256SUMS/);
  assert.match(v.buttonTitle, /v0\.2\.0/);
});

test("no button on the source channel, from an older hub, or with nothing loaded", () => {
  const source = updateApplyView({ ...onRelease, channel: "source" });
  assert.equal(source.button, null);
  assert.match(source.line, /builds PULSE from source/);
  assert.equal(updateApplyView({ ...base }).button, null);
  assert.equal(updateApplyView(null).button, null);
  assert.equal(updateApplyView(undefined).line, null);
});

test("while the POST is out or the hub installs, the button waits and polling speeds up", () => {
  const posting = updateApplyView(onRelease, { posting: true });
  assert.deepEqual([posting.button, posting.busy, posting.installing, posting.tone], ["Installing…", true, true, "info"]);
  const installing = updateApplyView({ ...onRelease, apply: { ...idle, state: "installing", latest: "v0.2.0", from: "v0.1.0" } });
  assert.equal(installing.busy, true);
  assert.equal(installing.line, "Downloading and checking v0.2.0…");
});

test("once installed the banner hands over to the runner's own wait words", () => {
  const installed = { ...onRelease, apply: { ...idle, state: "installed", latest: "v0.2.0" } };
  const waiting = updateApplyView(installed, { waitText: "A new version waits for 2 runs in progress" });
  assert.equal(waiting.button, null);
  assert.equal(waiting.tone, "ok");
  assert.equal(waiting.line, "Installed v0.2.0. A new version waits for 2 runs in progress.");
  assert.equal(
    updateApplyView(installed).line,
    "Installed v0.2.0. The runner and this console restart on it once their runs in flight finish.",
  );
});

test("a refusal or a failed install says why and offers to try again on release", () => {
  const refused = updateApplyView(onRelease, { postError: "update.channel resolves to source here" });
  assert.equal(refused.button, "Try again");
  assert.equal(refused.tone, "bad");
  assert.equal(refused.line, "Not started: update.channel resolves to source here");
  const failed = updateApplyView({ ...onRelease, apply: { ...idle, state: "failed", latest: "v0.2.0", error: "download SHA256SUMS: HTTP 404" } });
  assert.equal(failed.button, "Try again");
  assert.equal(failed.line, "The update did not install: download SHA256SUMS: HTTP 404");
  assert.match(updateApplyView({ ...onRelease, apply: { ...idle, state: "failed" } }).line, /without saying why/);
  assert.equal(updateApplyView({ ...onRelease, channel: "source", apply: { ...idle, state: "failed", error: "x" } }).button, null);
});

test("an apply about an older release reads as idle; current says nothing to install", () => {
  const old = updateApplyView({ ...onRelease, apply: { ...idle, state: "failed", latest: "v0.1.5", error: "old" } });
  assert.equal(old.button, "Update now");
  assert.equal(old.line, null);
  const current = updateApplyView({ ...onRelease, apply: { ...idle, state: "current", latest: "v0.2.0" } });
  assert.equal(current.button, null);
  assert.match(current.line, /already runs/);
});
