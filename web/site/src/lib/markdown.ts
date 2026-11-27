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
import { SKIP, visit } from "unist-util-visit";
import type { Element, ElementContent, Root as HastRoot, RootContent as HastRootContent } from "hast";
import type { Root as MdastRoot } from "mdast";
import type { ImageSize } from "./image-size.ts";
import { findDiagrams, isDiagram } from "./diagram.ts";
import type { FoundDiagram } from "./diagram.ts";
import { diagramElement } from "./doc-diagram.ts";
import { loneShot, shotElement, shotFigure } from "./doc-shot.ts";
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
  /** For a file of the repository: its path from the repository's root. */
  repoPath?: string;
};

export type LinkContext = {
  /** The doc being rendered, relative to docs/, with forward slashes: "harness.md". */
  file: string;
  /** Every doc that has a page: its file relative to docs/, and that page's path on the site. */
  pages: ReadonlyMap<string, string>;
  /** Whether the repository holds this path (relative to its root). Without it every path is trusted. */
  repoHas?: (path: string) => "file" | "dir" | null;
  /** The pixel size of a picture of the repository, when it can be read, so the page keeps the picture's place while it loads. */
  imageSize?: (repoPath: string) => ImageSize | null;
};

/** Where the export keeps the pictures of docs/ (scripts/prebuild.mjs copies them there). */
export const DOC_MEDIA = "/docs-media";
// Raster formats only. An SVG can carry script, and opened by its address it would run that script as this site;
// one in docs/ is shown from GitHub's raw host, as every picture outside docs/ is.
const PICTURE = /\.(png|webp|jpe?g|gif|avif)$/i;

/** Whether a file of docs/ is a picture the export carries. */
export function isDocPicture(file: string): boolean {
  return PICTURE.test(file);
}

/**
 * A picture kept under docs/ is served by the site itself, so a page needs
 * nothing from GitHub to show it: docs/how-to/images/home.webp is
 * /docs-media/how-to/images/home.webp. Any other file of the repository is null.
 */
export function docMediaPath(repoPath: string): string | null {
  if (!repoPath.startsWith("docs/") || !isDocPicture(repoPath)) return null;
  return `${DOC_MEDIA}/${repoPath.slice("docs/".length).split("/").map(encodeURIComponent).join("/")}`;
}

/** Sections of the README that a page of this site covers; a doc that links to one lands here instead. */
export const README_ON_SITE: Readonly<Record<string, string>> = {
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

/**
 * The addresses a Markdown text links to, as they are written and in the
 * order they stand: `[a](b.md)`, and `[a][b]` with its `[b]: b.md` wherever
 * that stands. Code is not read for links.
 */
export function markdownLinks(source: string): string[] {
  const tree = unified().use(remarkParse).use(remarkGfm).parse(splitFrontMatter(source).body);
  // Of two definitions of one name the first holds, as in the page.
  const defined = new Map<string, string>();
  visit(tree, "definition", (node) => {
    if (!defined.has(node.identifier)) defined.set(node.identifier, node.url);
  });
  const out: string[] = [];
  visit(tree, (node) => {
    if (node.type === "link") out.push(node.url);
    else if (node.type === "linkReference" && defined.has(node.identifier)) out.push(defined.get(node.identifier) as string);
  });
  return out;
}

/** Where a link written in docs/<ctx.file> leads on the site. */
export function rewriteHref(href: string, ctx: LinkContext): Rewritten {
  if (href === "" || href.startsWith("#")) {
    return { href, kind: "anchor", anchor: decode(href.slice(1)) };
  }
  if (href.startsWith("//")) return { href, kind: "external" };
  if (SCHEME.test(href)) {
    if (/^(mailto|tel):/i.test(href)) return { href, kind: "other" };
    // javascript:, data:, file: and the like are never linked from a public page, whatever a doc says.
    if (!/^https?:/i.test(href)) return { href: "", kind: "other", problem: `${href.slice(0, 60)} uses a scheme the site does not link to` };
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
  if (!ctx.repoHas) return { href: github(path, resolved.endsWith("/")), kind: "repo", anchor, repoPath: path };

  const found = ctx.repoHas(path);
  if (found) return { href: github(path, found === "dir"), kind: "repo", anchor, repoPath: path };
  // A path written from the repository's root without the leading slash (scripts/install.sh in a doc).
  const asWritten = posix.normalize(written).replace(/^(\.\/)+/, "").replace(/\/+$/, "");
  const alt = fromRoot || asWritten.startsWith("..") ? null : ctx.repoHas(asWritten);
  if (alt) return { href: github(asWritten, alt === "dir"), kind: "repo", anchor, repoPath: asWritten };
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

/** What stands under one h2 or h3 heading (or before the first, with the id ""), for the search. */
export type Section = {
  id: string;
  /** Everything under the heading as plain text, code blocks included. */
  text: string;
  /** The inline code of the section as written: commands, settings, flags, paths. */
  code: string[];
};
export type DocLink = Rewritten & { written: string };
/** One diagram drawn in text: where it starts in the file, its size, its words, and what is badly drawn (lib/diagram.ts). */
export type DocDiagram = FoundDiagram;

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
  /** The body cut at those headings, in order; the first is what stands before any of them. */
  sections: Section[];
  /** Every id a link may point at: each heading's, the title's first. */
  ids: string[];
  /** The top-level paragraphs as plain text, in order (the description is cut from the first). */
  paragraphs: string[];
  /** Every link and image the doc holds, as written and as rendered. */
  links: DocLink[];
  /** Every diagram drawn in text, in order. */
  diagrams: DocDiagram[];
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
  // The diagrams are read from the file as it is, so each is named by its line there, front matter included.
  const out: Rendered = { title: "", titleHtml: "", titleId: "", html: "", toc: [], sections: [], ids: [], paragraphs: [], links: [], diagrams: findDiagrams(source), frontMatter: data };

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
      // A permalink for the eye and the mouse. It stays out of the heading's text and out of the Tab order:
      // the table of contents already links to every heading, by name.
      node.children.push(el("a", { className: ["anchor"], href: `#${id}`, ariaHidden: "true", tabIndex: -1 }, []));
    });
    if (first.title) {
      out.titleHtml = stringifyChildren(first.title.node.children);
      const siblings = first.title.parent.children as HastRootContent[];
      siblings.splice(siblings.indexOf(first.title.node), 1);
    }

    // Links and images.
    visit(tree, "element", (node, index, parent) => {
      if (node.tagName === "a" && typeof node.properties.href === "string" && !isAnchorLink(node)) {
        const written = node.properties.href;
        const to = rewriteHref(written, ctx);
        out.links.push({ ...to, written });
        if (to.href === "" && to.problem) delete node.properties.href;
        else node.properties.href = to.href;
        if (to.kind === "external" || to.kind === "repo") node.properties.rel = ["noopener", "noreferrer"];
      } else if (node.tagName === "img" && typeof node.properties.src === "string") {
        const written = node.properties.src;
        const to = rewriteHref(written, ctx);
        out.links.push({ ...to, written });
        // A picture under docs/ is the site's own file; one kept elsewhere in the repository is served by GitHub as the file itself.
        const media = to.kind === "repo" && to.repoPath && !to.problem ? docMediaPath(to.repoPath) : null;
        if (media && to.repoPath) {
          node.properties.src = media;
          const size = ctx.imageSize?.(to.repoPath);
          if (size) {
            node.properties.width = size.width;
            node.properties.height = size.height;
          }
        } else {
          node.properties.src = to.kind === "repo" ? to.href.replace(`${SITE.repo}/blob/`, `${SITE.repo}/raw/`) : to.href;
        }
        node.properties.alt = typeof node.properties.alt === "string" ? node.properties.alt : "";
        node.properties.loading = "lazy";
        node.properties.decoding = "async";
        // A picture of the docs is framed, and a click opens it at its own size (doc-shot.ts).
        const inLink = parent?.type === "element" && parent.tagName === "a";
        if (media && parent && index !== undefined && !inLink) {
          parent.children[index] = shotElement(node, media);
          return SKIP;
        }
      }
    });
    // A picture that stands alone in its paragraph is a figure, with its words under it: a <figure> may not stand inside a <p>, so it takes the paragraph's place.
    visit(tree, "element", (node, index, parent) => {
      if (node.tagName !== "p" || !parent || index === undefined) return;
      const shot = loneShot(node);
      if (shot) {
        parent.children[index] = shotFigure(shot);
        return SKIP;
      }
    });

    // The body cut at its h2 and h3 headings, as text: what the search index is made of. The cut follows every such
    // heading, as the table of contents does: one inside a quote, a list or the footnotes starts a section too.
    let section: Section = { id: "", text: "", code: [] };
    out.sections.push(section);
    const cuts = (node: Element) => node.tagName === "h2" || node.tagName === "h3";
    const holdsCut = (node: Element): boolean => node.children.some((c) => c.type === "element" && (cuts(c) || holdsCut(c)));
    const say = (text: string) => {
      const words = text.replace(/\s+/g, " ").trim();
      if (words) section.text += `${section.text ? " " : ""}${words}`;
    };
    const cut = (nodes: readonly (HastRootContent | ElementContent)[]) => {
      for (const node of nodes) {
        if (node.type === "text") say(node.value);
        if (node.type !== "element") continue;
        if (cuts(node)) {
          section = { id: typeof node.properties.id === "string" ? node.properties.id : "", text: "", code: [] };
          out.sections.push(section);
        } else if (holdsCut(node)) {
          cut(node.children);
        } else {
          say(hastToString(node));
          const into = section;
          visit(node, "element", (n, _i, parent) => {
            if (n.tagName === "code" && !(parent?.type === "element" && parent.tagName === "pre")) into.code.push(hastToString(n).replace(/\s+/g, " ").trim());
          });
        }
      }
    };
    cut(tree.children);

    // Wide things scroll inside their own box, and a keyboard can reach them.
    visit(tree, "element", (node, index, parent) => {
      if (node.tagName === "pre") {
        node.properties.tabIndex = 0;
        const code = node.children.find((child): child is Element => child.type === "element" && child.tagName === "code");
        const classes = Array.isArray(code?.properties.className) ? code.properties.className.map(String) : [];
        const lang = classes.find((c) => c.startsWith("language-"))?.slice("language-".length) ?? "";
        const text = code ? hastToString(code) : "";
        if (code && isDiagram(text, lang)) {
          // A diagram drawn in text has markup of its own (doc-diagram.ts); when that is another element, it takes the block's place.
          const drawn = diagramElement(node, text);
          if (drawn !== node && parent && index !== undefined) {
            parent.children[index] = drawn;
            return SKIP;
          }
        }
      } else if (node.tagName === "table" && parent && index !== undefined) {
        nameRows(node);
        keepWords(node);
        // Focusable, so a keyboard can scroll it; no landmark of its own, or a page of tables would be a page of regions.
        parent.children[index] = el("div", { className: ["table-scroll"], tabIndex: 0 }, [node]);
      } else if (node.tagName === "input" && node.properties.type === "checkbox") {
        // The box of a task list item says whether the item is done; without a name a screen reader reads a bare checkbox.
        node.properties.ariaLabel = "Done";
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

const isElement = (node: ElementContent): node is Element => node.type === "element";

/** The rows of a table's head or body, each as its cells. */
function rowsOf(table: Element, section: "thead" | "tbody"): Element[][] {
  return table.children
    .filter(isElement)
    .filter((part) => part.tagName === section)
    .flatMap((part) => part.children.filter(isElement))
    .map((row) => row.children.filter(isElement));
}

/**
 * A table whose first heading is empty names its rows in its first column
 * (a setting down the side, its values across the top). Those cells become
 * the rows' headers, so a screen reader says which row a value belongs to,
 * and the empty corner stops being a heading that says nothing. The
 * stylesheet keeps all of them looking as they did.
 */
function nameRows(table: Element): void {
  const corner = rowsOf(table, "thead")[0]?.[0];
  if (!corner || corner.tagName !== "th" || hastToString(corner).trim() !== "") return;
  corner.tagName = "td";
  for (const [first] of rowsOf(table, "tbody")) {
    if (first?.tagName === "td") {
      first.tagName = "th";
      first.properties.scope = "row";
    }
  }
}

/** The longest one-word code a table keeps on one line: a setting, a flag, a command. An address or a path may still break. */
const WHOLE_WORD = 20;

/**
 * A narrow column would break a setting's name in the middle
 * (`worker_mode` and `l`). A short code of one word is marked, and the
 * stylesheet keeps a marked one whole; the table's box scrolls if it must.
 */
function keepWords(table: Element): void {
  visit(table, "element", (node) => {
    if (node.tagName !== "code") return;
    const text = hastToString(node);
    if (text.length > 0 && text.length <= WHOLE_WORD && !/\s/.test(text)) node.properties.className = ["word"];
  });
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
