#!/usr/bin/env node
// After a build: read out/ the way a crawler and a reader meet it, and fail
// on what would be broken for them.
//
//   node scripts/check-out.mjs [out]
//
// - every internal link, image, stylesheet and script of every exported page
//   leads to a file of the export, and every #anchor to an element with that
//   id on the page it points at (a link a doc's Markdown got wrong shows up
//   here by page and address; src/lib/docs.test.mjs names the Markdown file);
//   every link has text, and every link that leaves the site carries
//   rel="noopener noreferrer";
// - every file a stylesheet names (the fonts) is in the export, and a page
//   preloads only a font the stylesheets use;
// - every page has one <title> (at most 60 characters), one description (70
//   to 160), its own canonical address, one h1, the link preview tags and
//   JSON-LD that parses; no two pages share a title or a description;
// - sitemap.xml lists exactly the pages that were exported, robots.txt names
//   it, llms.txt lists every page, the manifest's icons exist;
// - the pages the site writes itself keep to the words it was given: no em
//   dash, no hype, nothing of the hosted product.
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { toString } from "hast-util-to-string";
import rehypeParse from "rehype-parse";
import { unified } from "unified";
import { visit } from "unist-util-visit";

const OUT = path.resolve(process.argv[2] ?? "out");
const SITE = "https://hivefleet.dev";
const TITLE_MAX = 60;
const DESCRIPTION = [70, 160];

const errors = [];
const notes = [];
const fail = (where, what) => errors.push(`${where}: ${what}`);

if (!fs.existsSync(path.join(OUT, "index.html"))) {
  console.error(`check-out: ${OUT} holds no index.html. Run \`npm run build\` first.`);
  process.exit(2);
}

function walk(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const p = path.join(dir, e.name);
    return e.isDirectory() ? walk(p) : [p];
  });
}

const rel = (file) => path.relative(OUT, file).split(path.sep).join("/");
const isFile = (p) => {
  try {
    return fs.statSync(p).isFile();
  } catch {
    return false;
  }
};

/** out/docs/harness/index.html is the page /docs/harness/. Next's own 404 files are not pages to find. */
function pagePath(file) {
  const r = rel(file);
  if (r === "404.html" || r.startsWith("404/") || r.startsWith("_not-found/")) return null;
  if (r === "index.html") return "/";
  return r.endsWith("/index.html") ? `/${r.slice(0, -"index.html".length)}` : null;
}

const parser = unified().use(rehypeParse);
const attr = (node, name) => {
  const v = node.properties?.[name];
  return Array.isArray(v) ? v.join(" ") : typeof v === "string" ? v : v === true ? "" : undefined;
};

function read(file) {
  const tree = parser.parse(fs.readFileSync(file, "utf8"));
  const page = { file, path: pagePath(file), ids: new Map(), links: [], assets: [], titles: [], h1: [], meta: [], jsonld: [], imgs: [], lang: undefined, canonical: [], text: "", styles: [], fontPreloads: [] };
  visit(tree, "element", (node) => {
    const id = attr(node, "id");
    if (id) page.ids.set(id, (page.ids.get(id) ?? 0) + 1);
    switch (node.tagName) {
      case "html":
        page.lang = attr(node, "lang");
        break;
      case "title":
        // An <svg> may hold a <title> of its own; the page's is the one in <head>.
        page.titles.push(toString(node));
        break;
      case "h1":
        page.h1.push(toString(node).trim());
        break;
      case "a":
        if (attr(node, "href") !== undefined) {
          page.links.push({
            href: attr(node, "href"),
            rel: attr(node, "rel") ?? "",
            name: toString(node).trim() || (attr(node, "ariaLabel") ?? ""),
            hidden: attr(node, "ariaHidden") === "true",
          });
        }
        if (attr(node, "name")) page.ids.set(attr(node, "name"), 1);
        break;
      case "img":
        page.imgs.push({ src: attr(node, "src"), alt: attr(node, "alt") });
        if (attr(node, "src")) page.assets.push(attr(node, "src"));
        break;
      case "script":
        if (attr(node, "src")) page.assets.push(attr(node, "src"));
        else if (attr(node, "type") === "application/ld+json") page.jsonld.push(toString(node));
        break;
      case "link": {
        const relValue = attr(node, "rel") ?? "";
        const href = attr(node, "href");
        if (relValue.includes("canonical")) page.canonical.push(href);
        else if (href) page.assets.push(href);
        if (href && relValue.includes("preload") && attr(node, "as") === "font") page.fontPreloads.push(href);
        break;
      }
      case "style":
        page.styles.push(toString(node));
        break;
      case "meta":
        page.meta.push({ name: attr(node, "name") ?? attr(node, "property"), content: attr(node, "content") ?? "" });
        break;
      case "body": {
        const copy = structuredClone(node);
        visit(copy, "element", (n, i, parent) => {
          if ((n.tagName === "script" || n.tagName === "style") && parent && i !== undefined) {
            parent.children.splice(i, 1);
            return i;
          }
        });
        page.text = toString(copy);
        break;
      }
    }
  });
  return page;
}

const files = walk(OUT);
const htmlFiles = files.filter((f) => f.endsWith(".html"));
const all = htmlFiles.map(read);
const pages = all.filter((p) => p.path !== null);
const byPath = new Map(pages.map((p) => [p.path, p]));

// ---- links, anchors, assets ------------------------------------------------

let linkCount = 0;
let anchorCount = 0;
let externalCount = 0;

function resolve(page, href, what) {
  const where = page.path ?? `/${rel(page.file)}`;
  if (/^(mailto:|tel:)/i.test(href) || (what === "asset" && /^data:/i.test(href))) return;
  let url;
  try {
    url = new URL(href, `${SITE}${page.path ?? "/"}`);
  } catch {
    fail(where, `${what} ${href} is not an address`);
    return;
  }
  if (url.origin !== SITE) {
    if (/^https?:$/.test(url.protocol)) externalCount += 1;
    else fail(where, `${what} ${href} has a scheme this site does not link to`);
    return;
  }
  linkCount += 1;
  let pathname;
  try {
    pathname = decodeURIComponent(url.pathname);
  } catch {
    pathname = url.pathname;
  }
  const hash = url.hash ? decodeURIComponent(url.hash.slice(1)) : "";

  let target = byPath.get(pathname);
  if (!target) {
    const asFile = path.join(OUT, pathname);
    if (!pathname.endsWith("/") && isFile(asFile)) {
      if (hash && asFile.endsWith(".html")) fail(where, `${what} ${href}: anchors are checked on pages, and ${pathname} is a file`);
      return;
    }
    if (!pathname.endsWith("/") && byPath.has(`${pathname}/`)) {
      fail(where, `${what} ${href} lacks the trailing slash of ${pathname}/ (the host would have to redirect)`);
      return;
    }
    fail(where, `${what} ${href} leads nowhere: the export has no ${pathname}`);
    return;
  }
  if (hash) {
    anchorCount += 1;
    if (!target.ids.has(hash)) fail(where, `${what} ${href}: ${target.path} has no element with the id "${hash}"`);
  }
}

for (const page of all) {
  const where = page.path ?? `/${rel(page.file)}`;
  for (const link of page.links) {
    resolve(page, link.href, "link");
    if (!link.hidden && !link.name) fail(where, `link ${link.href} has no text`);
    // A link that leaves the site tells the other side nothing about where the reader came from.
    if (/^https?:\/\//i.test(link.href) && !link.href.startsWith(`${SITE}/`) && link.href !== SITE) {
      if (!/\bnoopener\b/.test(link.rel) || !/\bnoreferrer\b/.test(link.rel)) fail(where, `external link ${link.href} lacks rel="noopener noreferrer"`);
    }
  }
  for (const src of page.assets) resolve(page, src, "asset");
}

// ---- what the stylesheets load ----------------------------------------------

// A font the CSS names and the export lacks is an error nowhere else: the page
// would be set in another typeface. A page should preload only a font it will
// use: one whose face holds plain Latin letters, not the face kept for other
// scripts.
const cssUrls = new Set();
const latinFonts = new Set();
const urlsIn = (css) => [...css.matchAll(/url\(\s*(["']?)([^"')]+)\1\s*\)/g)].map((m) => m[2]).filter((ref) => !/^(data:|#)/i.test(ref));
/** Whether a unicode-range (U+61, U+0-FF, U+4??) holds the code point. */
function rangeHolds(range, point) {
  return range.split(",").some((part) => {
    const [from, to = from] = part.trim().replace(/^u\+/i, "").split("-");
    return parseInt(from.replace(/\?/g, "0"), 16) <= point && point <= parseInt(to.replace(/\?/g, "F"), 16);
  });
}
function readCss(css, where, base) {
  const missing = new Set();
  const local = (ref) => {
    const url = new URL(ref, base);
    if (url.origin !== SITE) {
      fail(where, `a stylesheet loads ${ref} from another site`);
      return null;
    }
    const pathname = decodeURIComponent(url.pathname);
    cssUrls.add(pathname);
    if (!isFile(path.join(OUT, pathname))) missing.add(ref);
    return pathname;
  };
  for (const ref of urlsIn(css)) local(ref);
  for (const [, face] of css.matchAll(/@font-face\s*{([^}]*)}/g)) {
    const range = /unicode-range\s*:\s*([^;]+)/i.exec(face)?.[1];
    if (range && !rangeHolds(range, 0x61)) continue;
    for (const ref of urlsIn(face)) {
      const pathname = local(ref);
      if (pathname) latinFonts.add(pathname);
    }
  }
  for (const ref of missing) fail(where, `a stylesheet names ${ref}, which is not in the export`);
}
for (const file of files.filter((f) => f.endsWith(".css"))) readCss(fs.readFileSync(file, "utf8"), `/${rel(file)}`, `${SITE}/${rel(file)}`);
for (const page of all) for (const css of page.styles) readCss(css, page.path ?? `/${rel(page.file)}`, `${SITE}${page.path ?? "/"}`);
for (const page of all) {
  for (const href of page.fontPreloads) {
    const pathname = decodeURIComponent(new URL(href, `${SITE}${page.path ?? "/"}`).pathname);
    if (!latinFonts.has(pathname)) fail(page.path ?? `/${rel(page.file)}`, `preloads the font ${href}, which no stylesheet uses for Latin text`);
  }
}

// ---- each page, as a search engine reads it ---------------------------------

const metaOf = (page, name) => page.meta.filter((m) => m.name === name).map((m) => m.content);
const one = (page, name) => {
  const values = metaOf(page, name);
  if (values.length !== 1) fail(page.path, `has ${values.length} ${name} tags, wants one`);
  return values[0] ?? "";
};

const titles = new Map();
const descriptions = new Map();
const types = (page) =>
  page.jsonld.flatMap((text) => {
    try {
      const data = JSON.parse(text);
      return (data["@graph"] ?? [data]).map((n) => n["@type"]);
    } catch (e) {
      fail(page.path, `JSON-LD does not parse: ${e.message}`);
      return [];
    }
  });

for (const page of pages) {
  const where = page.path;
  const url = `${SITE}${page.path}`;
  const isDoc = page.path.startsWith("/docs/") && page.path !== "/docs/";

  if (page.lang !== "en") fail(where, `<html lang> is ${page.lang}`);
  const title = page.titles[0] ?? "";
  if (!title) fail(where, "has no <title>");
  if (title.length > TITLE_MAX) fail(where, `title is ${title.length} characters (at most ${TITLE_MAX}): ${title}`);
  if (titles.has(title)) fail(where, `shares its title with ${titles.get(title)}: ${title}`);
  titles.set(title, where);

  const description = one(page, "description");
  if (description.length < DESCRIPTION[0] || description.length > DESCRIPTION[1]) {
    fail(where, `description is ${description.length} characters (${DESCRIPTION[0]} to ${DESCRIPTION[1]}): ${description}`);
  }
  if (descriptions.has(description)) fail(where, `shares its description with ${descriptions.get(description)}`);
  descriptions.set(description, where);

  if (page.canonical.length !== 1 || page.canonical[0] !== url) fail(where, `canonical is ${JSON.stringify(page.canonical)}, wants ${url}`);
  if (page.h1.length !== 1) fail(where, `has ${page.h1.length} h1 headings, wants exactly one: ${JSON.stringify(page.h1)}`);
  if (metaOf(page, "robots").some((r) => /noindex/i.test(r))) fail(where, "is marked noindex");

  if (one(page, "og:url") !== url) fail(where, `og:url is ${metaOf(page, "og:url")}, wants ${url}`);
  if (!one(page, "og:title")) fail(where, "og:title is empty");
  if (one(page, "og:description") !== description) fail(where, "og:description differs from the description");
  if (one(page, "og:type") !== (isDoc ? "article" : "website")) fail(where, `og:type is ${metaOf(page, "og:type")}`);
  if (one(page, "og:image") !== `${SITE}/og.png`) fail(where, `og:image is ${metaOf(page, "og:image")}`);
  if (one(page, "og:image:width") !== "1200" || one(page, "og:image:height") !== "630") fail(where, "og:image is not declared 1200 x 630");
  if (one(page, "og:image:alt").length < 20) fail(where, "og:image:alt is missing");
  if (one(page, "twitter:card") !== "summary_large_image") fail(where, `twitter:card is ${metaOf(page, "twitter:card")}`);
  if (one(page, "twitter:image") !== `${SITE}/og.png`) fail(where, `twitter:image is ${metaOf(page, "twitter:image")}`);

  for (const img of page.imgs) if (img.alt === undefined) fail(where, `image ${img.src} has no alt text`);
  for (const [id, n] of page.ids) if (n > 1) fail(where, `the id "${id}" is on ${n} elements, so an anchor to it is ambiguous`);

  const found = new Set(types(page));
  const wanted = ["WebSite", "Organization", ...(page.path === "/" ? ["SoftwareApplication", "FAQPage"] : []), ...(isDoc ? ["BreadcrumbList", "TechArticle"] : [])];
  for (const t of wanted) if (!found.has(t)) fail(where, `has no ${t} in its JSON-LD`);

  // The words: only where the site wrote them. A docs page is the repository's Markdown.
  if (!isDoc) {
    const text = page.path === "/docs/" ? "" : page.text;
    if (/\u2014/.test(text)) fail(where, "has an em dash");
    const hype = /\b(revolutionary|seamless(ly)?|powerful|effortless(ly)?|next-generation)\b/i.exec(text);
    if (hype) fail(where, `uses the word "${hype[0]}"`);
    const hosted = /\b(clerk|sign[ -]?in|sign[ -]?up|signed in|accounts?|team brain)\b/i.exec(text);
    if (hosted) fail(where, `speaks of the hosted product: "${hosted[0]}"`);
  }
}

// The 404 page: there, and never indexed.
const notFound = all.find((p) => rel(p.file) === "404.html");
if (!notFound) fail("/404.html", "is missing");
else {
  if (!metaOf(notFound, "robots").some((r) => /noindex/i.test(r))) fail("/404.html", "is not marked noindex");
  if (notFound.h1.length !== 1) fail("/404.html", `has ${notFound.h1.length} h1 headings`);
  if (notFound.canonical.length > 0) fail("/404.html", `has a canonical address: ${notFound.canonical}`);
}

// ---- the files beside the pages ---------------------------------------------

const text = (name) => (isFile(path.join(OUT, name)) ? fs.readFileSync(path.join(OUT, name), "utf8") : null);
const exported = pages.map((p) => `${SITE}${p.path}`).sort();

const sitemap = text("sitemap.xml");
if (!sitemap) fail("/sitemap.xml", "is missing");
else {
  const listed = [...sitemap.matchAll(/<loc>([^<]+)<\/loc>/g)].map((m) => m[1]).sort();
  for (const u of exported) if (!listed.includes(u)) fail("/sitemap.xml", `does not list ${u}`);
  for (const u of listed) if (!exported.includes(u)) fail("/sitemap.xml", `lists ${u}, which was not exported`);
  if (new Set(listed).size !== listed.length) fail("/sitemap.xml", "lists an address twice");
}

const robots = text("robots.txt");
if (!robots) fail("/robots.txt", "is missing");
else {
  if (!robots.split("\n").some((l) => l.trim() === `Sitemap: ${SITE}/sitemap.xml`)) fail("/robots.txt", "does not name the sitemap");
  if (!/^Allow: \/$/m.test(robots)) fail("/robots.txt", "does not allow everything");
  if (/^Disallow: \S/m.test(robots)) fail("/robots.txt", "disallows something");
}

const llms = text("llms.txt");
if (!llms) fail("/llms.txt", "is missing");
else for (const u of exported) if (!llms.includes(`](${u}): `)) fail("/llms.txt", `does not list ${u}`);

const manifest = text("manifest.webmanifest");
if (!manifest) fail("/manifest.webmanifest", "is missing");
else {
  try {
    const m = JSON.parse(manifest);
    for (const key of ["name", "short_name", "theme_color", "background_color"]) if (!m[key]) fail("/manifest.webmanifest", `has no ${key}`);
    const icons = (m.icons ?? []).map((i) => i.src);
    for (const icon of ["/icon-192.png", "/icon-512.png"]) {
      if (!icons.includes(icon)) fail("/manifest.webmanifest", `does not list ${icon}`);
      if (!isFile(path.join(OUT, icon))) fail("/manifest.webmanifest", `${icon} is not in the export`);
    }
  } catch (e) {
    fail("/manifest.webmanifest", `does not parse: ${e.message}`);
  }
}

for (const name of ["og.png", "icon.svg", "apple-icon.png", "docs/search.json"]) if (!isFile(path.join(OUT, name))) fail(`/${name}`, "is missing");
notes.push(isFile(path.join(OUT, "install.sh")) ? "/install.sh is in the export." : "/install.sh is not in the export: scripts/install.sh was not there when the site was built.");

const hostedDocs = pages.filter((p) => p.path.startsWith("/docs/") && /\bclerk\b/i.test(p.text)).map((p) => p.path);
if (hostedDocs.length) notes.push(`docs pages whose Markdown still names Clerk (the words are the repository's, not the site's): ${hostedDocs.join(", ")}`);

// ---- the verdict -------------------------------------------------------------

for (const n of notes) console.log(`check-out: note: ${n}`);
if (errors.length) {
  console.error(`check-out: ${errors.length} ${errors.length === 1 ? "problem" : "problems"} in ${rel(OUT) || OUT}:`);
  for (const e of errors) console.error(`  - ${e}`);
  process.exit(1);
}
console.log(
  `check-out: ${pages.length} pages, ${linkCount} internal links and assets, ${anchorCount} anchors, ${cssUrls.size} files the stylesheets name: every one resolves (${externalCount} external links were not followed). Titles, descriptions, canonical addresses, JSON-LD, sitemap, robots, llms.txt and the manifest are in order.`,
);
