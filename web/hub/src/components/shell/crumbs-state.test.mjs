import assert from "node:assert/strict";
import { test } from "node:test";
import { backHref, defaultCrumbs as derive, normalizePath, phoneTitle } from "./crumbs-state.ts";
import { NAV_ITEMS, SETTINGS_ITEM } from "./nav-items.ts";

const pages = [...NAV_ITEMS, SETTINGS_ITEM];
const defaultCrumbs = (path, search, projects = []) => derive(path, search, projects, pages);

const q = (s = "") => new URLSearchParams(s);
const projects = [{ slug: "demo-app", name: "Demo App" }, { slug: "plain" }];

test("home and plain pages name themselves", () => {
  assert.deepEqual(defaultCrumbs("/", q()), [{ label: "Home", href: "/" }]);
  assert.deepEqual(defaultCrumbs("/records/", q()), [{ label: "Records", href: "/records/" }]);
  assert.deepEqual(defaultCrumbs("/records", q()), [{ label: "Records", href: "/records/" }], "a missing trailing slash is the same page");
  assert.deepEqual(defaultCrumbs("/for-you/", q()), [{ label: "For you", href: "/for-you/" }]);
  assert.deepEqual(defaultCrumbs("/insights/", q()), [{ label: "Insights", href: "/insights/" }]);
  assert.deepEqual(defaultCrumbs("/settings/", q()), [{ label: "Settings", href: "/settings/" }]);
  assert.deepEqual(defaultCrumbs("/harness/", q()).map((c) => c.label), ["Runs"]);
});

test("a project page reads project › Tab, with the project's name when it has one", () => {
  assert.deepEqual(defaultCrumbs("/project/", q("slug=demo-app&tab=board"), projects), [
    { label: "Demo App", href: "/project/?slug=demo-app" },
    { label: "Board", href: "/project/?slug=demo-app&tab=board" },
  ]);
  assert.deepEqual(defaultCrumbs("/project/", q("slug=plain"), projects), [{ label: "plain", href: "/project/?slug=plain" }]);
  assert.deepEqual(
    defaultCrumbs("/project/", q("slug=demo-app&tab=board&sub=threads&thread=abc"), projects).map((c) => c.label),
    ["Demo App", "Threads", "Thread"],
  );
  assert.deepEqual(defaultCrumbs("/project/", q("slug=unknown"), projects)[0].label, "unknown", "an unregistered slug still shows");
  assert.deepEqual(defaultCrumbs("/project/", q(""), projects), [{ label: "Projects", href: "/" }]);
});

test("a filter or an open detail adds a crumb", () => {
  assert.deepEqual(defaultCrumbs("/tasks/", q("project=demo-app"), projects).map((c) => c.label), ["Tasks", "Demo App"]);
  assert.deepEqual(defaultCrumbs("/records/", q("id=ADR-0042"), projects).map((c) => c.label), ["Records", "ADR-0042"]);
  assert.deepEqual(defaultCrumbs("/chat/", q("task=TASK-007")).map((c) => c.label), ["Sessions", "TASK-007"]);
  assert.deepEqual(defaultCrumbs("/chat/", q("id=conv-1")).map((c) => c.label), ["Sessions", "Session"]);
  assert.deepEqual(defaultCrumbs("/code-deliveries/", q()).map((c) => c.label), ["Records", "Code delivered"]);
  assert.deepEqual(defaultCrumbs("/reports/", q("project=demo-app")).map((c) => c.label), ["Reports", "demo-app"]);
});

test("a route the console doesn't have is the 404 page", () => {
  assert.deepEqual(defaultCrumbs("/nope/", q()), [{ label: "Page not found" }]);
  assert.deepEqual(defaultCrumbs("/some-thing/", q()), [{ label: "Page not found" }], "never a word made from the path");
  assert.equal(normalizePath("/x?y=1"), "/x/");
});

test("the phone title is the last crumb, else the document title without the suffix", () => {
  assert.equal(phoneTitle([{ label: "Records" }, { label: "ADR-1" }], "Records · PULSE"), "ADR-1");
  assert.equal(phoneTitle([], "Tasks · PULSE"), "Tasks");
  assert.equal(phoneTitle([], ""), "PULSE");
});

test("back goes to the nearest earlier crumb with a link", () => {
  assert.equal(backHref([{ label: "Home", href: "/" }]), null);
  assert.equal(backHref([{ label: "Records", href: "/records/" }, { label: "ADR-1" }]), "/records/");
  assert.equal(backHref([{ label: "A", href: "/a/" }, { label: "B" }, { label: "C" }]), "/a/");
});
