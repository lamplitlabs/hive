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

test("the passkey pages outside the nav name themselves, not the 404 page", () => {
  // The same crumbs the pages set through useCrumbs (app/passkey/**/page.tsx).
  assert.deepEqual(defaultCrumbs("/passkey/", q()), [{ label: "Passkey" }]);
  assert.deepEqual(defaultCrumbs("/passkey", q()), [{ label: "Passkey" }], "a missing trailing slash is the same page");
  assert.deepEqual(defaultCrumbs("/passkey/register/", q()), [{ label: "Passkey", href: "/passkey/" }, { label: "Register" }]);
  assert.equal(backHref(defaultCrumbs("/passkey/register/", q())), "/passkey/", "the phone's back arrow returns to Passkey");
  assert.deepEqual(defaultCrumbs("/passkey/other/", q()), [{ label: "Page not found" }], "only the two passkey pages exist");
});

test("every page the console exports has crumbs other than the 404 page", async () => {
  const { readdir } = await import("node:fs/promises");
  const { fileURLToPath } = await import("node:url");
  const app = fileURLToPath(new URL("../../app/", import.meta.url));
  const routes = [];
  const walk = async (dir, route) => {
    for (const e of await readdir(dir, { withFileTypes: true })) {
      if (e.isDirectory()) await walk(dir + e.name + "/", route + e.name + "/");
      else if (e.name === "page.tsx") routes.push(route);
    }
  };
  await walk(app, "/");
  assert.ok(routes.includes("/passkey/register/"), "found the app routes: " + routes.join(" "));
  for (const route of routes) {
    const search = route === "/project/" ? q("slug=demo-app") : q();
    assert.notDeepEqual(defaultCrumbs(route, search, projects), [{ label: "Page not found" }], route + " has no default crumbs");
  }
});

test("a route the console doesn't have is the 404 page", () => {
  assert.deepEqual(defaultCrumbs("/nope/", q()), [{ label: "Page not found" }]);
  assert.deepEqual(defaultCrumbs("/some-thing/", q()), [{ label: "Page not found" }], "never a word made from the path");
  assert.equal(normalizePath("/x?y=1"), "/x/");
});

test("the phone title is the last crumb, else the document title without the suffix", () => {
  assert.equal(phoneTitle([{ label: "Records" }, { label: "ADR-1" }], "Records · Hive"), "ADR-1");
  assert.equal(phoneTitle([], "Tasks · Hive"), "Tasks");
  assert.equal(phoneTitle([], ""), "Hive");
});

test("back goes to the nearest earlier crumb with a link", () => {
  assert.equal(backHref([{ label: "Home", href: "/" }]), null);
  assert.equal(backHref([{ label: "Records", href: "/records/" }, { label: "ADR-1" }]), "/records/");
  assert.equal(backHref([{ label: "A", href: "/a/" }, { label: "B" }, { label: "C" }]), "/a/");
});
