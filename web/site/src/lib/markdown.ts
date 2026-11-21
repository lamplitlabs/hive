// The repository's Markdown, rendered at build time to plain HTML: GitHub's
// flavour (tables, fenced code, task lists, autolinks, strikethrough), GitHub's
// heading ids, and links rewritten so a doc that points at another doc points
// at its page here and a doc that points into the repository points at GitHub.
// Nothing here runs in the browser. Pure apart from what `repoHas` is given, so
// the node tests import it as it is.
import { posix } from "node:path";
import GithubSlugger from "github-slugger";
import { toString as hastToString } from "hast-util-to-string";
import { toString as mdastToString } from "mdast-util-to-string";
import rehypeHighlight from "rehype-highlight";
import rehypeStringify from "rehype-stringify";
import remarkGfm from "remark-gfm";
import remarkParse from "remark-parse";
import remarkRehype from "remark-rehype";
import { unified } from "unified";
import { visit } from "unist-util-visit";
import type { Element, ElementContent, Root as HastRoot, RootContent as HastRootContent } from "hast";
import type { Root as MdastRoot } from "mdast";
import { SITE } from "./site.ts";

/** What a link in a doc turned out to be. */
export type LinkKind = "anchor" | "doc" | "site" | "repo" | "external" | "other";

export type Rewritten = {
  href: string;
  kind: LinkKind;
  /** For a link to another doc: that doc's file, relative to docs/. */
  doc?: string;
  /** The #fragment the link asked for, without the #. */
  anchor?: string;
  /** What is wrong with the link as the Markdown wrote it; the link is still rendered. */
  problem?: string;
};

export type LinkContext = {
  /** The doc being rendered, relative to docs/, with forward slashes: "harness.md". */
  file: string;
  /** Every doc that has a page: its file relative to docs/, and that page's path on the site. */
  pages: ReadonlyMap<string, string>;
  /** Whether the repository holds this path (relative to its root). Without it every path is trusted. */
  repoHas?: (path: string) => "file" | "dir" | null;
};

/** Sections of the README that a page of this site covers; a doc that links to one lands here instead. */
export const README_ON_SITE: Readonly<Record<string, string>> = {
  "five-minutes-to-the-first-task": "/install/",
  install: "/install/",
  installing: "/install/",
  "quick-start": "/install/",
};

const SCHEME = /^[a-zA-Z][a-zA-Z0-9+.-]*:/;

function decode(part: string): string {
  try {
    return decodeURIComponent(part);
  } catch {
    return part;
  }
}

/** Where a link written in docs/<ctx.file> leads on the site. */
export function rewriteHref(href: string, ctx: LinkContext): Rewritten {
  if (href === "" || href.startsWith("#")) {
    return { href, kind: "anchor", anchor: decode(href.slice(1)) };
  }
  if (href.startsWith("//")) return { href, kind: "external" };
  if (SCHEME.test(href)) {
    if (!/^https?:/i.test(href)) return { href, kind: "other" };
    // The site's own address, written out in full: keep the visitor here.
    if (href === SITE.url || href.startsWith(`${SITE.url}/`)) {
      const rest = href.slice(SITE.url.length) || "/";
      return { href: rest, kind: "site" };
    }
    return { href, kind: "external" };
  }

  const parts = /^([^?#]*)(\?[^#]*)?(#.*)?$/.exec(href);
  const rawPath = parts?.[1] ?? href;
  const hash = parts?.[3] ?? "";
  const anchor = hash ? decode(hash.slice(1)) : undefined;
  const written = decode(rawPath);

  // GitHub reads a leading slash as the repository's root; everything else is relative to the doc's folder.
  const fromRoot = written.startsWith("/");
  const resolved = posix.normalize(fromRoot ? written.replace(/^\/+/, "") : posix.join("docs", posix.dirname(ctx.file), written));
  const path = resolved.replace(/\/+$/, "");

  if (path.startsWith("docs/")) {
    const page = ctx.pages.get(path.slice("docs/".length));
    if (page) return { href: `${page}${hash}`, kind: "doc", doc: path.slice("docs/".length), anchor };
  }
  if (path === "README.md" && anchor && README_ON_SITE[anchor]) {
    return { href: README_ON_SITE[anchor], kind: "site" };
  }

  const github = (p: string, dir: boolean) => `${SITE.repo}/${dir ? "tree" : "blob"}/main/${encodeURI(p)}${hash}`;
  if (path === "" || path === "." || path.startsWith("..")) {
    return { href: github(path.replace(/^(\.\.\/)+/, ""), true), kind: "repo", anchor, problem: `${href} leaves the repository` };
  }
  if (!ctx.repoHas) return { href: github(path, resolved.endsWith("/")), kind: "repo", anchor };

  const found = ctx.repoHas(path);
  if (found) return { href: github(path, found === "dir"), kind: "repo", anchor };
  // A path written from the repository's root without the leading slash (scripts/install.sh in a doc).
  const asWritten = posix.normalize(written).replace(/^(\.\/)+/, "").replace(/\/+$/, "");
  const alt = fromRoot || asWritten.startsWith("..") ? null : ctx.repoHas(asWritten);
  if (alt) return { href: github(asWritten, alt === "dir"), kind: "repo", anchor };
  return { href: github(path, false), kind: "repo", anchor, problem: `${href} names ${path}, which is not in the repository` };
}

/** The YAML block some docs open with (the ADRs): its plain `key: value` lines, and the Markdown after it. */
export function splitFrontMatter(source: string): { data: Record<string, string>; body: string } {
  const text = source.replace(/^\uFEFF/, "");
  if (!/^---[ \t]*\r?\n/.test(text)) return { data: {}, body: text };
  const end = /\r?\n---[ \t]*(\r?\n|$)/.exec(text.slice(3));
  if (!end) return { data: {}, body: text };
  const block = text.slice(text.indexOf("\n") + 1, 3 + end.index);
  const data: Record<string, string> = {};
  for (const line of block.split(/\r?\n/)) {
    const m = /^([A-Za-z_][\w-]*):[ \t]+(\S.*)$/.exec(line);
    if (m) data[m[1]] = m[2].trim().replace(/^(['"])(.*)\1$/, "$2");
  }
  return { data, body: text.slice(3 + end.index + end[0].length) };
}

/** One line of text, cut to at most `max` characters at the end of a sentence, else at a word with an ellipsis. */
export function cutClean(text: string, max: number, min = 0): string {
  const t = text.replace(/\s+/g, " ").trim();
  if (t.length <= max) return t;
  const head = t.slice(0, max + 1);
  let sentence = -1;
  for (const m of head.matchAll(/[.!?](?=\s)/g)) {
    const at = m.index ?? -1;
    // "e.g." and "i.e." end no sentence.
    if (!/(?:\be\.g|\bi\.e)$/i.test(head.slice(0, at))) sentence = at;
  }
  if (sentence + 1 >= Math.max(min, Math.floor(max * 0.45))) return t.slice(0, sentence + 1);
  const room = max - 1;
  let cut = head.lastIndexOf(" ", room);
  if (cut < room * 0.5) cut = room;
  return `${t.slice(0, cut).replace(/[\s,;:([{-]+$/, "")}…`;
}

/** A title cut to at most `max` characters: at a clause when one ends late enough, else at a word with an ellipsis. */
export function cutTitle(title: string, max: number): string {
  const t = title.replace(/\s+/g, " ").trim();
  if (t.length <= max) return t;
  let best = -1;
  for (const mark of [" - ", ": ", ", ", " ("]) {
    for (let i = t.indexOf(mark); i !== -1 && i <= max; i = t.indexOf(mark, i + 1)) best = Math.max(best, i);
  }
  if (best >= 20) return t.slice(0, best);
  const cut = t.lastIndexOf(" ", max - 1);
  return `${t.slice(0, cut > 0 ? cut : max - 1).replace(/[\s,;:([{-]+$/, "")}…`;
}

export type TocEntry = { id: string; text: string; level: 2 | 3 };
export type DocLink = Rewritten & { written: string };

export type Rendered = {
  /** The first `# heading` as text; empty when the file has none. */
  title: string;
  /** The same heading's inner HTML (it may hold code), and the id GitHub gives it. */
  titleHtml: string;
  titleId: string;
  /** The body without that heading. */
  html: string;
  /** The h2 and h3 headings, in order. */
  toc: TocEntry[];
  /** Every id a link may point at: each heading's, the title's first. */
  ids: string[];
  /** The top-level paragraphs as plain text, in order (the description is cut from the first). */
  paragraphs: string[];
  /** Every link and image the doc holds, as written and as rendered. */
  links: DocLink[];
  frontMatter: Record<string, string>;
};

const HEADING = /^h([1-6])$/;

function el(tagName: string, properties: Element["properties"], children: ElementContent[]): Element {
  return { type: "element", tagName, properties, children };
}

/** Raw HTML in a doc is almost always a placeholder (`<slug>`, `<job>`): show it as the text it is, never run it, never drop it. */
function remarkLiteralHtml() {
  return (tree: MdastRoot) => {
    visit(tree, "html", (node, index, parent) => {
      if (!parent || index === undefined) return;
      if (/^<!--[\s\S]*-->$/.test(node.value.trim())) {
        parent.children.splice(index, 1);
        return index;
      }
      const text = { type: "text" as const, value: node.value };
      // At block level a bare text node has no paragraph around it.
      const block = parent.type === "root" || parent.type === "blockquote" || parent.type === "listItem";
      parent.children.splice(index, 1, block ? { type: "paragraph" as const, children: [text] } : text);
      return index + 1;
    });
  };
}

function stringifyChildren(children: ElementContent[]): string {
  return unified().use(rehypeStringify).stringify({ type: "root", children });
}

/** One doc: Markdown in, the pieces a page is built from out. */
export function renderMarkdown(source: string, ctx: LinkContext): Rendered {
  const { data, body } = splitFrontMatter(source);
  const out: Rendered = { title: "", titleHtml: "", titleId: "", html: "", toc: [], ids: [], paragraphs: [], links: [], frontMatter: data };

  const collectText = () => (tree: MdastRoot) => {
    for (const node of tree.children) {
      if (node.type === "paragraph") out.paragraphs.push(mdastToString(node).replace(/\s+/g, " ").trim());
    }
  };

  const shape = () => (tree: HastRoot) => {
    // Headings: GitHub's ids in document order, so the anchors links already use keep working.
    const slugger = new GithubSlugger();
    const first: { title?: { node: Element; parent: HastRoot | Element } } = {};
    visit(tree, "element", (node, _index, parent) => {
      const m = HEADING.exec(node.tagName);
      if (!m || !parent) return;
      const text = hastToString(node).replace(/\s+/g, " ").trim();
      const given = typeof node.properties.id === "string" ? node.properties.id : "";
      const id = given || slugger.slug(text);
      node.properties.id = id;
      out.ids.push(id);
      let level = Number(m[1]);
      if (level === 1) {
        if (!first.title) {
          first.title = { node, parent };
          out.title = text;
          out.titleId = id;
          return;
        }
        // A page has one h1: a second top heading in a file becomes a section.
        node.tagName = "h2";
        level = 2;
      }
      if (level === 2 || level === 3) out.toc.push({ id, text, level });
      node.children.push(
        el("a", { className: ["anchor"], href: `#${id}`, ariaLabel: `Permalink: ${text}` }, [{ type: "text", value: "#" }]),
      );
    });
    if (first.title) {
      out.titleHtml = stringifyChildren(first.title.node.children);
      const siblings = first.title.parent.children as HastRootContent[];
      siblings.splice(siblings.indexOf(first.title.node), 1);
    }

    // Links and images.
    visit(tree, "element", (node) => {
      if (node.tagName === "a" && typeof node.properties.href === "string" && !isAnchorLink(node)) {
        const written = node.properties.href;
        const to = rewriteHref(written, ctx);
        out.links.push({ ...to, written });
        node.properties.href = to.href;
        if (to.kind === "external" || to.kind === "repo") node.properties.rel = ["noopener", "noreferrer"];
      } else if (node.tagName === "img" && typeof node.properties.src === "string") {
        const written = node.properties.src;
        const to = rewriteHref(written, ctx);
        out.links.push({ ...to, written });
        // A picture kept in the repository is served by GitHub as the file itself.
        node.properties.src = to.kind === "repo" ? to.href.replace(`${SITE.repo}/blob/`, `${SITE.repo}/raw/`) : to.href;
        node.properties.alt = typeof node.properties.alt === "string" ? node.properties.alt : "";
        node.properties.loading = "lazy";
        node.properties.decoding = "async";
      }
    });

    // Wide things scroll inside their own box, and a keyboard can reach them.
    visit(tree, "element", (node, index, parent) => {
      if (node.tagName === "pre") {
        node.properties.tabIndex = 0;
      } else if (node.tagName === "table" && parent && index !== undefined) {
        parent.children[index] = el("div", { className: ["table-scroll"], role: "region", ariaLabel: "Table", tabIndex: 0 }, [node]);
      }
    });
  };

  const file = unified()
    .use(remarkParse)
    .use(remarkGfm)
    .use(remarkLiteralHtml)
    .use(collectText)
    .use(remarkRehype)
    .use(shape)
    .use(rehypeHighlight, { plainText: ["text", "txt", "plain"] })
    .use(rehypeStringify)
    .processSync(body);
  out.html = String(file);
  return out;
}

function isAnchorLink(node: Element): boolean {
  const cls = node.properties.className;
  return Array.isArray(cls) && cls.includes("anchor");
}

/** One code sample for a page written by hand: the same highlighter the docs get, the text left exactly as it is. */
export function highlightCode(code: string, lang: string): string {
  const tree: HastRoot = {
    type: "root",
    children: [el("pre", {}, [el("code", { className: [`language-${lang}`] }, [{ type: "text", value: code }])])],
  };
  const processor = unified().use(rehypeHighlight, { plainText: ["text", "txt", "plain"] }).use(rehypeStringify);
  const done = processor.runSync(tree) as HastRoot;
  const pre = done.children[0] as Element;
  const inner = pre.children[0] as Element;
  return stringifyChildren(inner.children);
}
