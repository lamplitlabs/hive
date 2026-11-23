#!/usr/bin/env node
// Before a build, two things of the repository are copied into public/, both
// build products (git-ignored):
//
// - the installer the site serves at /install.sh is the repository's own
//   scripts/install.sh, copied as it is, so the line on the front page and
//   the file in the repository can never be two different scripts. A checkout
//   without the installer still builds; it serves no /install.sh;
// - the pictures of the docs: every raster picture under docs/ goes to
//   public/docs-media/ with its path, so a docs page shows a picture from
//   this site and needs nothing from GitHub (src/lib/markdown.ts writes the
//   address, scripts/check-out.mjs fails on one that is missing).
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

// The same extensions as isDocPicture in src/lib/markdown.ts: raster pictures only. An SVG can carry script and
// is never copied to the site's own address (scripts/check-out.mjs fails on one that is there).
const PICTURE = /\.(png|webp|jpe?g|gif|avif)$/i;
const docs = path.resolve(site, "../../docs");
const media = path.join(site, "public", "docs-media");
fs.rmSync(media, { recursive: true, force: true });
let pictures = 0;
let bytes = 0;
const copy = (dir, rel) => {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.name.startsWith(".")) continue;
    const from = path.join(dir, entry.name);
    const to = rel ? `${rel}/${entry.name}` : entry.name;
    if (entry.isDirectory()) copy(from, to);
    else if (entry.isFile() && PICTURE.test(entry.name)) {
      const target = path.join(media, to);
      fs.mkdirSync(path.dirname(target), { recursive: true });
      fs.copyFileSync(from, target);
      pictures += 1;
      bytes += fs.statSync(from).size;
    }
  }
};
if (fs.existsSync(docs)) copy(docs, "");
console.log(
  pictures > 0
    ? `prebuild: copied ${pictures} ${pictures === 1 ? "picture" : "pictures"} of docs/ (${Math.round(bytes / 1024)} KB) to public/docs-media/; the site serves them at /docs-media/.`
    : "prebuild: docs/ holds no pictures, so this build serves no /docs-media/.",
);
