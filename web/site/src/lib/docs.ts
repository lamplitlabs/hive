// The repository's docs/ as the site's /docs/ pages: which files get a page,
// in which order the sidebar lists them, and each page's title, description
// and neighbours. `planDocs` and `buildDocs` are pure (the tests feed them
// lists and strings); `loadDocs` reads ../../docs once per build process.
import fs from "node:fs";
import path from "node:path";
import { cutClean, cutTitle, renderMarkdown } from "./markdown.ts";
import type { DocLink, TocEntry } from "./markdown.ts";
import type { SearchEntry } from "./search.ts";
import { SITE } from "./site.ts";

/** Files under docs/ that get no page: the repository's own environment recipe is for its runner, not for a reader. */
export const DOCS_EXCLUDED: ReadonlySet<string> = new Set(["context/environment.md"]);

type GroupSpec = { title: string; files?: readonly string[]; dirs?: readonly string[] };

/**
 * The sidebar, in order. A name is a file of docs/ without `.md`; one that
 * does not exist (yet) is skipped, and a file no group names lands in "More",
 * so a new doc is never dropped silently.
 */
export const DOC_GROUPS: readonly GroupSpec[] = [
  { title: "Start", files: ["concepts", "faq", "console", "mac-app"] },
  { title: "The agents", files: ["board", "harness", "self-improvement", "evolve", "review", "landing"] },
  { title: "Machines and running", files: ["machines", "runner", "release", "recovery", "security"] },
  { title: "Reference", files: ["commands", "settings", "reports", "owner-reservation", "development", "windows-port"] },
  { title: "Decisions and design", dirs: ["decisions", "design"] },
];
export const MORE_GROUP = "More";

/** Sidebar words that differ from the start of the file's own heading. */
const LABELS: Readonly<Record<string, string>> = {
  landing: "Delivery",
  recovery: "Recovery and hygiene",
};

/**
 * Descriptions the site words itself. A page's description is its first
 * paragraph; where a later decision overtook the page and that paragraph no
 * longer says what holds, the description says so. The Markdown is left as
 * the repository has it.
 */
const DESCRIPTIONS: Readonly<Record<string, string>> = {
  "decisions/adr-070-web-first-hive-hosted-brain-open-hand":
    "Superseded in part by ADR-071: the hosted product is withdrawn. What this ADR built stays as options for someone who runs Hive themselves.",
};

export type PlannedDoc = {
  /** Relative to docs/, forward slashes: "decisions/ADR-069-….md". */
  file: string;
  /** The same without `.md`, lowercased: "decisions/adr-069-…". */
  slug: string;
  /** The page: "/docs/decisions/adr-069-…/". */
  path: string;
  group: string;
};

export type Plan = {
  /** Every page, in sidebar order. */
  docs: PlannedDoc[];
  groups: { title: string; docs: PlannedDoc[] }[];
  /** The files left out on purpose. */
  excluded: string[];
};

/** File names in one order on every machine: by their lowercased letters, whatever the locale. */
function byName(a: string, b: string): number {
  const x = a.toLowerCase();
  const y = b.toLowerCase();
  return x < y ? -1 : x > y ? 1 : a < b ? -1 : a > b ? 1 : 0;
}

export function docSlug(file: string): string {
  return file.replace(/\.md$/i, "").toLowerCase();
}

export function docPath(file: string): string {
  return `/docs/${docSlug(file).split("/").map(encodeURIComponent).join("/")}/`;
}

/** Which files get a page, and where each sits in the sidebar. */
export function planDocs(files: readonly string[]): Plan {
  const all = [...new Set(files)].filter((f) => /\.md$/i.test(f)).sort(byName);
  const excluded = all.filter((f) => DOCS_EXCLUDED.has(f));
  const kept = all.filter((f) => !DOCS_EXCLUDED.has(f));

  const bySlug = new Map<string, string>();
  for (const file of kept) {
    const slug = docSlug(file);
    const twin = bySlug.get(slug);
    if (twin) throw new Error(`docs/${twin} and docs/${file} would both be /docs/${slug}/: the addresses are lowercased, so rename one`);
    bySlug.set(slug, file);
  }

  const taken = new Set<string>();
  const groups: Plan["groups"] = [];
  const add = (title: string, picked: string[]) => {
    if (picked.length === 0) return;
    for (const f of picked) taken.add(f);
    groups.push({ title, docs: picked.map((file) => ({ file, slug: docSlug(file), path: docPath(file), group: title })) });
  };
  for (const spec of DOC_GROUPS) {
    const picked: string[] = [];
    for (const name of spec.files ?? []) {
      const file = bySlug.get(name);
      if (file && !taken.has(file)) picked.push(file);
    }
    for (const dir of spec.dirs ?? []) {
      picked.push(...kept.filter((f) => docSlug(f).startsWith(`${dir}/`) && !taken.has(f) && !picked.includes(f)));
    }
    add(spec.title, picked);
  }
  add(
    MORE_GROUP,
    kept.filter((f) => !taken.has(f)),
  );
  return { docs: groups.flatMap((g) => g.docs), groups, excluded };
}

export type DocRef = { path: string; label: string };

export type Doc = PlannedDoc & {
  /** The file's first `# heading`. */
  title: string;
  titleHtml: string;
  titleId: string;
  /** The sidebar's word for it. */
  label: string;
  /** The whole <title>: at most 60 characters. */
  metaTitle: string;
  /** From the first paragraph, or the site's own words for a page a later decision overtook: 70 to 155 characters. */
  description: string;
  html: string;
  toc: TocEntry[];
  ids: string[];
  links: DocLink[];
  frontMatter: Record<string, string>;
  editUrl: string;
  prev?: DocRef;
  next?: DocRef;
};

export type Docs = {
  docs: Doc[];
  groups: { title: string; docs: Doc[] }[];
  byPath: ReadonlyMap<string, Doc>;
  bySlug: ReadonlyMap<string, Doc>;
  excluded: string[];
  /** Links that are broken in the Markdown itself. They are rendered as written and reported, never hidden. */
  problems: string[];
};

/** "windows-port" reads "Windows port" when a file has no heading of its own. */
function titleFromSlug(slug: string): string {
  const words = (slug.split("/").pop() ?? slug).replace(/[-_]+/g, " ").trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

export function docLabel(slug: string, title: string): string {
  if (LABELS[slug]) return LABELS[slug];
  if (/^ADR-\d+\b/.test(title)) return cutTitle(title, 64);
  const cut = title.search(/ \(|: /);
  return cut > 0 ? title.slice(0, cut) : title;
}

const TITLE_MAX = 60;

/** The <title> of a docs page: the heading and the site's name, as much of both as 60 characters hold. */
export function docMetaTitle(title: string): string {
  for (const tail of [" · Hive docs", " · Hive"]) {
    if (title.length + tail.length <= TITLE_MAX) return title + tail;
  }
  if (title.length <= TITLE_MAX) return title;
  return `${cutTitle(title, TITLE_MAX - " · Hive".length)} · Hive`;
}

export const DESCRIPTION_MIN = 70;
export const DESCRIPTION_MAX = 155;

/** The meta description: the first paragraph, more of the page when that is too short, cut cleanly. */
export function describeDoc(title: string, paragraphs: readonly string[]): string {
  let text = "";
  for (const p of paragraphs) {
    if (!p) continue;
    text = text ? `${text} ${p}` : p;
    if (text.length >= DESCRIPTION_MIN) break;
  }
  if (text.length < DESCRIPTION_MIN) text = text ? `${title}. ${text}` : title;
  if (text.length < DESCRIPTION_MIN) text = `${text.replace(/[\s.]+$/, "")}. A page of the Hive documentation, rendered from the repository.`;
  return cutClean(text, DESCRIPTION_MAX, DESCRIPTION_MIN);
}

export type DocSource = { file: string; source: string };

/** Every doc rendered, titled, described and linked to its neighbours. */
export function buildDocs(sources: readonly DocSource[], repoHas?: (path: string) => "file" | "dir" | null): Docs {
  const plan = planDocs(sources.map((s) => s.file));
  const text = new Map(sources.map((s) => [s.file, s.source]));
  const pages = new Map(plan.docs.map((d) => [d.file, d.path]));

  const docs: Doc[] = plan.docs.map((planned) => {
    const r = renderMarkdown(text.get(planned.file) ?? "", { file: planned.file, pages, repoHas });
    const title = r.title || titleFromSlug(planned.slug);
    return {
      ...planned,
      title,
      titleHtml: r.titleHtml || title.replace(/&/g, "&amp;").replace(/</g, "&lt;"),
      titleId: r.titleId,
      label: docLabel(planned.slug, title),
      metaTitle: docMetaTitle(title),
      description: DESCRIPTIONS[planned.slug] ?? describeDoc(title, r.paragraphs),
      html: r.html,
      toc: r.toc,
      ids: r.ids,
      links: r.links,
      frontMatter: r.frontMatter,
      editUrl: `${SITE.repo}/edit/main/docs/${planned.file.split("/").map(encodeURIComponent).join("/")}`,
    };
  });

  // Two pages may never share a title or a description: the later one says which page it is.
  const titles = new Set<string>();
  const descriptions = new Set<string>();
  for (const doc of docs) {
    if (titles.has(doc.metaTitle)) doc.metaTitle = `${cutTitle(doc.title, TITLE_MAX - doc.slug.length - 3)} (${doc.slug})`.slice(0, TITLE_MAX);
    titles.add(doc.metaTitle);
    if (descriptions.has(doc.description)) doc.description = cutClean(`${doc.title}: ${doc.description}`, DESCRIPTION_MAX, DESCRIPTION_MIN);
    descriptions.add(doc.description);
  }

  docs.forEach((doc, i) => {
    const before = docs[i - 1];
    const after = docs[i + 1];
    if (before) doc.prev = { path: before.path, label: before.label };
    if (after) doc.next = { path: after.path, label: after.label };
  });

  const byFile = new Map(docs.map((d) => [d.file, d]));
  const problems: string[] = [];
  for (const doc of docs) {
    for (const link of doc.links) {
      const where = `docs/${doc.file}: [${link.written}]`;
      if (link.problem) problems.push(`${where} ${link.problem}`);
      if (!link.anchor) continue;
      if (link.kind === "anchor" && !doc.ids.includes(link.anchor)) {
        problems.push(`${where} points at #${link.anchor}, and this page has no such heading`);
      } else if (link.kind === "doc" && link.doc) {
        const target = byFile.get(link.doc);
        if (target && !target.ids.includes(link.anchor)) {
          problems.push(`${where} points at #${link.anchor}, and docs/${link.doc} has no such heading`);
        }
      }
    }
  }

  const groups = plan.groups.map((g) => ({ title: g.title, docs: g.docs.map((p) => byFile.get(p.file) as Doc) }));
  return {
    docs,
    groups,
    byPath: new Map(docs.map((d) => [d.path, d])),
    bySlug: new Map(docs.map((d) => [d.slug, d])),
    excluded: plan.excluded,
    problems,
  };
}

/** Where the repository's docs/ is: beside web/, two folders up from web/site, where every build runs. */
export function docsRoot(): string {
  return process.env.HIVE_DOCS_DIR ? path.resolve(process.env.HIVE_DOCS_DIR) : path.resolve(process.cwd(), "../../docs");
}

/** Every .md file under a folder, relative to it, forward slashes, sorted. */
export function listDocFiles(root: string): string[] {
  const out: string[] = [];
  const walk = (dir: string, prefix: string) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (entry.name.startsWith(".")) continue;
      const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (entry.isDirectory()) walk(path.join(dir, entry.name), rel);
      else if (entry.isFile() && /\.md$/i.test(entry.name)) out.push(rel);
    }
  };
  walk(root, "");
  return out.sort(byName);
}

export function readDocSources(root: string): DocSource[] {
  if (!fs.existsSync(root)) {
    throw new Error(
      `The docs folder is missing: ${root}. The site renders the repository's docs/, so build it from a checkout of the whole repository (or point HIVE_DOCS_DIR at the folder).`,
    );
  }
  return listDocFiles(root).map((file) => ({ file, source: fs.readFileSync(path.join(root, file), "utf8") }));
}

let cache: Docs | undefined;

/** The docs as the build sees them, read and rendered once per process. */
export function loadDocs(): Docs {
  if (cache) return cache;
  const root = docsRoot();
  const repo = path.dirname(root);
  const repoHas = (p: string): "file" | "dir" | null => {
    try {
      return fs.statSync(path.join(repo, p)).isDirectory() ? "dir" : "file";
    } catch {
      return null;
    }
  };
  cache = buildDocs(readDocSources(root), repoHas);
  return cache;
}

/** One doc by its slug, for a page of the site that leans on it; a doc that is gone fails the build with its name. */
export function requireDoc(slug: string): Doc {
  const doc = loadDocs().bySlug.get(slug);
  if (!doc) throw new Error(`The site links to docs/${slug}.md, and docs/ has no such file any more.`);
  return doc;
}

/** One doc by its slug when it exists: the docs other packages add later (concepts, faq, commands, settings). */
export function optionalDoc(slug: string): Doc | undefined {
  return loadDocs().bySlug.get(slug);
}

/** What the search box loads: every page's title and its h2 and h3 headings. */
export function searchIndex(docs: Docs): SearchEntry[] {
  return docs.docs.map((d) => ({ u: d.path, t: d.title, g: d.group, h: d.toc.map((e) => [e.id, e.text]) }));
}
