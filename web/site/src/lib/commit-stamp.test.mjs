// public/commit.txt tells the world which commit the site was built from,
// and the check that keeps the site on main believes it. So the stamp is
// given only to a tree that is exactly a commit of the site's own checkout.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const { commitStamp } = await import("../../scripts/commit-stamp.mjs");

// Git sees the scratch repositories alone: not the repository this test
// runs in (a commit hook exports GIT_DIR), and not this machine's settings.
for (const name of ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY", "GIT_COMMON_DIR", "GIT_PREFIX"]) {
  delete process.env[name];
}
process.env.GIT_CONFIG_GLOBAL = os.devNull;
process.env.GIT_CONFIG_NOSYSTEM = "1";

const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "commit-stamp-"));
test.after(() => fs.rmSync(scratch, { recursive: true, force: true }));

const git = (cwd, ...args) =>
  execFileSync("git", ["-c", "user.name=test", "-c", "user.email=test@localhost", "-c", "commit.gpgsign=false", ...args], {
    cwd,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  }).trim();

/** A checkout with web/site, docs/ and one commit; returns its root, its site folder and the commit. */
function checkout(name) {
  const root = path.join(scratch, name);
  const site = path.join(root, "web", "site");
  fs.mkdirSync(path.join(site, "public"), { recursive: true });
  fs.mkdirSync(path.join(root, "docs"), { recursive: true });
  fs.writeFileSync(path.join(site, ".gitignore"), "/public/commit.txt\n/out/\n");
  fs.writeFileSync(path.join(site, "package.json"), "{}\n");
  fs.writeFileSync(path.join(root, "docs", "a.md"), "# A\n");
  git(root, "init", "--quiet");
  git(root, "add", "-A");
  git(root, "commit", "--quiet", "-m", "one");
  return { root, site, commit: git(root, "rev-parse", "HEAD") };
}

test("a clean checkout is stamped with its HEAD, ignored build products or not", () => {
  const { site, commit } = checkout("clean");
  assert.deepEqual(commitStamp(site), { commit });
  fs.writeFileSync(path.join(site, "public", "commit.txt"), "old\n");
  fs.mkdirSync(path.join(site, "out"));
  fs.writeFileSync(path.join(site, "out", "index.html"), "<p>built</p>\n");
  assert.deepEqual(commitStamp(site), { commit }, "what .gitignore names does not count");
});

test("a changed file or an untracked one takes the stamp away", () => {
  const { root, site } = checkout("touched");
  fs.writeFileSync(path.join(root, "docs", "a.md"), "# A, changed\n");
  assert.match(commitStamp(site).reason ?? "", /changed or untracked/);
  git(root, "checkout", "--quiet", "--", "docs/a.md");
  fs.writeFileSync(path.join(root, "docs", "new-page.md"), "# Not committed\n");
  assert.match(commitStamp(site).reason ?? "", /changed or untracked/, "docs/ is read from the folder: an untracked page would be built");
});

test("a copy of the tree inside another repository does not take that repository's HEAD", () => {
  const { root } = checkout("outer");
  const copy = path.join(root, "vendor", "hive", "web", "site");
  fs.mkdirSync(copy, { recursive: true });
  const stamp = commitStamp(copy);
  assert.equal(stamp.commit, undefined);
  assert.match(stamp.reason ?? "", /not at web\/site of its own git checkout/);
});

test("a folder in no checkout is not stamped", () => {
  const site = path.join(scratch, "plain", "web", "site");
  fs.mkdirSync(site, { recursive: true });
  const stamp = commitStamp(site);
  assert.equal(stamp.commit, undefined);
  assert.match(stamp.reason ?? "", /not a git checkout/);
});
