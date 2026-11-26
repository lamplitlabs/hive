// The commit a build may name in public/commit.txt: HEAD of the checkout
// the site lives in, and only when that tree is exactly what the commit
// holds. scripts/prebuild.mjs writes the file; the check
// .hive/automations/site-shows-main.sh reads it from the public address to
// tell which commit the site shows, so a wrong stamp would make an older or
// a changed site pass for main.
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

/**
 * `{ commit }` for the site folder `site` (web/site of a checkout), or
 * `{ reason }` when the build is not to be stamped:
 * - git is not there, or `site` is in no checkout;
 * - the checkout is not the one `site` is web/site of (a copy of the tree
 *   inside another repository would take that repository's HEAD);
 * - a tracked file is changed, or a file git does not ignore is untracked
 *   (docs/ is read from the folder, so an untracked page is built too).
 */
export function commitStamp(site) {
  const git = (...args) => {
    try {
      return execFileSync("git", args, { cwd: site, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
    } catch {
      return null;
    }
  };
  const real = (p) => {
    try {
      return fs.realpathSync(p);
    } catch {
      return path.resolve(p);
    }
  };
  const top = git("rev-parse", "--show-toplevel");
  const commit = git("rev-parse", "HEAD");
  if (top === null || commit === null || !/^[0-9a-f]{40}$/.test(commit)) {
    return { reason: "this is not a git checkout (or git is not on PATH)" };
  }
  if (real(top) !== real(path.resolve(site, "../.."))) {
    return { reason: "the site is not at web/site of its own git checkout" };
  }
  // Untracked files count whatever this machine's git settings say (status.showUntrackedFiles).
  const changed = git("status", "--porcelain", "--untracked-files=normal");
  if (changed === null) return { reason: "git could not say whether this tree is clean" };
  if (changed !== "") return { reason: "this tree has changed or untracked files" };
  return { commit };
}
