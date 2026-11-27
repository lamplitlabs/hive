#!/usr/bin/env node
// What a stranger meets: the checks `npm run check` cannot make, because they
// are about the public internet and not about this folder.
//
//   node scripts/check-live.mjs [--site https://hivefleet.dev]
//                               [--repo lamplitlabs/hive]
//                               [--tap lamplitlabs/homebrew-tap]
//
// --site may name a preview of a deploy, or this machine while it serves
// out/, to look at a build before the domain points at it.
//
// It asks, without any login, for what the site promises a reader:
//
// - the site: the domain serves this site (not a registrar's parked page),
//   with its sitemap, robots.txt, llms.txt, search index, a guide and a
//   picture of the docs; www goes to the same place;
// - the installer: /install.sh is a shell script, served as text, and the
//   same bytes as scripts/install.sh of this checkout, because the install
//   line hands that address to a shell;
// - the repository: public, so the links to GitHub open and the installer
//   can download a release;
// - the release: the latest one carries the four archives and SHA256SUMS
//   that the installer and `hive update` read (docs/release.md, "What a
//   release must carry"), and a download needs no login;
// - Homebrew: the tap holds the formula the install page names.
//
// Each line says "ok" or "NOT YET" with what is missing and where the step
// is written down. Nothing here can be fixed by a commit: these are the
// owner's steps (docs/development.md, "Going public"). It exits 1 while one
// of them is open, 0 when a stranger can install Hive from the site.
// It is not part of `npm run check`: a build must not depend on the network.
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 && args[i + 1] ? args[i + 1] : fallback;
};
if (args.includes("--help") || args.includes("-h")) {
  console.log("usage: node scripts/check-live.mjs [--site URL] [--repo OWNER/NAME] [--tap OWNER/homebrew-NAME]");
  process.exit(0);
}

/** The address every page names as its own, whatever address it is read at. */
const HOME = "https://hivefleet.dev";
/** Where to look: the public address, or a preview of a deploy, or `npx serve out` on this machine. */
const SITE = opt("site", HOME).replace(/\/+$/, "");
const REPO = opt("repo", "lamplitlabs/hive");
const TAP = opt("tap", "lamplitlabs/homebrew-tap");
const TARGETS = ["aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];
const here = path.dirname(fileURLToPath(import.meta.url));
const installer = path.resolve(here, "../../../scripts/install.sh");

const results = [];
const ok = (name, detail) => results.push({ name, ok: true, detail });
const open = (name, detail, fix) => results.push({ name, ok: false, detail, fix });
const unknown = (name, detail) => results.push({ name, ok: null, detail });

async function get(url, init = {}) {
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), 20_000);
  try {
    const res = await fetch(url, { redirect: "follow", ...init, signal: ctl.signal, headers: { "user-agent": "hive-site-check-live", ...(init.headers ?? {}) } });
    const body = init.method === "HEAD" ? "" : await res.text();
    return { status: res.status, type: res.headers.get("content-type") ?? "", body, url: res.url, location: res.headers.get("location") ?? "" };
  } catch (e) {
    return { status: 0, type: "", body: "", url, location: "", error: e.name === "AbortError" ? "no answer in 20 seconds" : String(e.cause?.code ?? e.message) };
  } finally {
    clearTimeout(timer);
  }
}
const sha256 = (text) => crypto.createHash("sha256").update(text).digest("hex");
const domainStep = 'docs/development.md, "Putting the domain on the site"';

// ---- the site ---------------------------------------------------------------

const home = await get(`${SITE}/`);
const isSite = home.status === 200 && home.type.includes("text/html") && home.body.includes(`<link rel="canonical" href="${HOME}/"`);
if (isSite) ok("the site", `${SITE}/ serves this site`);
else {
  const what = home.error ? `no answer (${home.error})` : home.status !== 200 ? `answers ${home.status}` : "answers with a page that is not this site (a parked domain's page, for one)";
  open("the site", `${SITE}/ ${what}`, `add the domain to the Vercel project and set its two DNS records: ${domainStep}`);
}

if (isSite) {
  const want = [
    ["/sitemap.xml", (r) => r.body.includes("<urlset") && r.body.includes(`<loc>${HOME}/docs/how-to/</loc>`), "the sitemap, with the guides in it"],
    ["/robots.txt", (r) => r.body.includes(`Sitemap: ${HOME}/sitemap.xml`), "robots.txt, naming the sitemap"],
    ["/llms.txt", (r) => r.body.startsWith("# Hive") && r.body.includes("## Guides"), "llms.txt, with the guides listed"],
    ["/docs/search.json", (r) => r.body.trimStart().startsWith("["), "the search index"],
    ["/docs/how-to/", (r) => r.type.includes("text/html") && r.body.includes("<h1"), "the page of guides"],
    ["/docs-media/how-to/images/console-sessions.webp", (r) => r.type.includes("image/"), "a picture of the docs"],
  ];
  const missing = [];
  for (const [p, good, what] of want) {
    const r = await get(`${SITE}${p}`);
    if (r.status !== 200 || !good(r)) missing.push(`${p} (${what}): ${r.status === 200 ? "not what this commit builds" : r.error ?? `answers ${r.status}`}`);
  }
  if (missing.length === 0) ok("the site's files", `${want.length} of ${want.length} are served: ${want.map(([p]) => p).join(", ")}`);
  else open("the site's files", missing.join("; "), "the site there is older than this commit: push to main, or run the workflow deploy-site.yml by hand");

  const host = new URL(SITE).host;
  // Only the public name has a www beside it.
  if (SITE === HOME) {
    const www = await get(`${SITE.replace("://", "://www.")}/`, { redirect: "manual" });
    const to = www.location.replace(/\/+$/, "");
    if ([301, 302, 307, 308].includes(www.status) && to === SITE) ok("www", `www.${host} redirects to ${SITE}`);
    else if (www.status === 200) open("www", `www.${host} serves a page of its own instead of redirecting`, `in Vercel, make www.${host} redirect to ${host}: ${domainStep}, step 1`);
    else open("www", `www.${host} ${www.error ? `gives no answer (${www.error})` : `answers ${www.status}${to ? ` to ${to}` : ""}`}`, `add www.${host} to the Vercel project and its CNAME record: ${domainStep}`);
  }
}

// ---- the installer ----------------------------------------------------------

const script = await get(`${SITE}/install.sh`);
const isScript = script.status === 200 && script.body.startsWith("#!/bin/sh") && !script.type.includes("text/html");
if (!isScript) {
  open(
    "the installer",
    `${SITE}/install.sh ${script.error ? `gives no answer (${script.error})` : script.status !== 200 ? `answers ${script.status}` : "is not a shell script (a web page answers)"}, so the install line of the site fails`,
    `the domain must serve this site: ${domainStep}`,
  );
} else if (!fs.existsSync(installer)) {
  ok("the installer", `${SITE}/install.sh is a shell script (this checkout has no scripts/install.sh to compare it with)`);
} else if (sha256(script.body) === sha256(fs.readFileSync(installer, "utf8"))) {
  ok("the installer", `${SITE}/install.sh is scripts/install.sh of this checkout, byte for byte`);
} else {
  open("the installer", `${SITE}/install.sh is a shell script, and not the scripts/install.sh of this checkout`, "the site there was built from another commit: push to main, or run the workflow deploy-site.yml by hand");
}

// ---- the repository and the release -----------------------------------------

const api = (p) => get(`https://api.github.com/repos/${REPO}${p}`, { headers: { accept: "application/vnd.github+json" } });
const limited = (r) => r.status === 403 || r.status === 429;
const repo = await api("");
let isPublic = false;
if (repo.status === 200 && JSON.parse(repo.body).private === false) {
  isPublic = true;
  ok("the repository", `github.com/${REPO} is public`);
} else if (limited(repo)) unknown("the repository", "GitHub did not say (too many questions without a login from this address); try again in an hour");
else {
  open(
    "the repository",
    `github.com/${REPO} is not open to the public (GitHub answers ${repo.status || repo.error} without a login)`,
    "every link from the site to GitHub and every download of the installer needs it public: Settings, General, Danger Zone, Change repository visibility",
  );
}

if (isPublic) {
  const latest = await api("/releases/latest");
  if (latest.status !== 200) {
    if (limited(latest)) unknown("the release", "GitHub did not say (too many questions without a login from this address)");
    else open("the release", `no published release (GitHub answers ${latest.status})`, 'cut one: docs/release.md, "How a release is cut"');
  } else {
    const release = JSON.parse(latest.body);
    const names = new Set((release.assets ?? []).map((a) => a.name));
    const wanted = [...TARGETS.map((t) => `hive-${release.tag_name}-${t}.tar.gz`), "SHA256SUMS"];
    const absent = wanted.filter((n) => !names.has(n));
    const sums = await get(`https://github.com/${REPO}/releases/download/${release.tag_name}/SHA256SUMS`);
    if (absent.length > 0) open("the release", `${release.tag_name} lacks ${absent.join(", ")}`, 'build the tag again: docs/release.md, "What a release must carry"');
    else if (sums.status !== 200 || !/^[0-9a-f]{64} {2}\S+/m.test(sums.body)) open("the release", `${release.tag_name}: SHA256SUMS does not download without a login (${sums.status || sums.error})`, "the release must be published, not a draft");
    else ok("the release", `${release.tag_name} carries the four archives and SHA256SUMS, and they download without a login`);

    const formula = await get(`https://raw.githubusercontent.com/${TAP}/HEAD/Formula/hivefleet.rb`);
    const version = release.tag_name.replace(/^v/, "");
    if (formula.status !== 200) {
      open(
        "Homebrew",
        `github.com/${TAP} has no Formula/hivefleet.rb (${formula.status || formula.error}), so \`brew install ${TAP.replace("/homebrew-", "/")}/hivefleet\` fails`,
        'create the tap and the secret HOMEBREW_TAP_TOKEN, then build the tag again: docs/release.md, "The Homebrew tap"',
      );
    } else if (!formula.body.includes(version)) {
      open("Homebrew", `the formula in ${TAP} is not for ${release.tag_name}`, `gh workflow run release.yml -f tag=${release.tag_name} writes it (docs/release.md, "The Homebrew tap")`);
    } else ok("Homebrew", `${TAP} holds the formula for ${release.tag_name}`);
  }
} else if (limited(repo)) {
  unknown("the release", "not looked at: GitHub did not say whether the repository is public");
  unknown("Homebrew", "not looked at: the formula is compared with the latest release");
} else {
  unknown("the release", "not looked at: a release of a repository that is not public cannot be downloaded");
  unknown("Homebrew", "not looked at: the formula points at that release");
}

// ---- the verdict ------------------------------------------------------------

const width = Math.max(...results.map((r) => r.name.length));
for (const r of results) {
  console.log(`${r.ok === true ? "ok      " : r.ok === false ? "NOT YET " : "?       "}${r.name.padEnd(width)}  ${r.detail}`);
  if (r.fix) console.log(`${" ".repeat(8)}${" ".repeat(width)}  to do: ${r.fix}`);
}
const waiting = results.filter((r) => r.ok === false).length;
const unsure = results.filter((r) => r.ok === null).length;
console.log(
  waiting === 0 && unsure === 0
    ? `\ncheck-live: a stranger can read the site at ${SITE} and install Hive from it.`
    : `\ncheck-live: ${waiting} ${waiting === 1 ? "step is" : "steps are"} open${unsure ? `, ${unsure} could not be looked at` : ""}. These are the owner's steps, not something a commit can do.`,
);
process.exit(waiting === 0 && unsure === 0 ? 0 : 1);
