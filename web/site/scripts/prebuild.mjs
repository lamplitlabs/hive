#!/usr/bin/env node
// Before a build: the installer the site serves at /install.sh is the
// repository's own scripts/install.sh, copied as it is, so the line on the
// front page and the file in the repository can never be two different
// scripts. The copy is a build product (public/install.sh is git-ignored).
// A checkout without the installer still builds; it serves no /install.sh.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const site = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const source = path.resolve(site, "../../scripts/install.sh");
const target = path.join(site, "public", "install.sh");

const size = fs.existsSync(source) ? fs.statSync(source).size : 0;
if (size > 0) {
  fs.copyFileSync(source, target);
  console.log(`prebuild: copied scripts/install.sh (${size} bytes) to public/install.sh; the site serves it at /install.sh.`);
} else {
  fs.rmSync(target, { force: true });
  console.log("prebuild: scripts/install.sh is not in this checkout (or is empty), so this build serves no /install.sh. The build goes on.");
}
