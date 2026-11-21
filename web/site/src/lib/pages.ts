// Every page the site exports, in one list: the sitemap, /llms.txt and the
// tests all read it, so a page can never be in one and missing from another.
import { loadDocs } from "./docs.ts";
import type { Docs } from "./docs.ts";
import { absoluteUrl, INSTALL, SITE, STATIC_PAGES } from "./site.ts";

export type AnyPage = {
  path: string;
  /** The whole <title>. */
  title: string;
  description: string;
  /** The name a list shows for it. */
  name: string;
  /** One line that says what is on it. */
  summary: string;
  kind: "page" | "doc";
};

/** The static pages, then every docs page in sidebar order. The 404 page is not a page to find. */
export function allPages(docs: Docs = loadDocs()): AnyPage[] {
  return [
    ...STATIC_PAGES.map((p) => ({
      path: p.path,
      title: p.title,
      description: p.description,
      name: p.title,
      summary: p.summary,
      kind: "page" as const,
    })),
    ...docs.docs.map((d) => ({
      path: d.path,
      title: d.metaTitle,
      description: d.description,
      name: d.title,
      summary: d.description,
      kind: "doc" as const,
    })),
  ];
}

export type SitemapEntry = { url: string; changeFrequency: "weekly" | "monthly"; priority: number };

export function sitemapEntries(docs: Docs = loadDocs()): SitemapEntry[] {
  return allPages(docs).map((p) => ({
    url: absoluteUrl(p.path),
    changeFrequency: p.kind === "doc" ? "weekly" : "monthly",
    priority: p.path === "/" ? 1 : p.kind === "page" ? 0.8 : p.path.startsWith("/docs/decisions/") || p.path.startsWith("/docs/design/") ? 0.4 : 0.6,
  }));
}

/** /llms.txt: what Hive is and where everything is, in plain text, for a language model that reads the site. */
export function llmsText(docs: Docs = loadDocs()): string {
  const pages = allPages(docs);
  const line = (p: AnyPage) => `- [${p.name}](${absoluteUrl(p.path)}): ${p.summary}`;
  const out = [
    `# ${SITE.name}`,
    "",
    `> ${SITE.sentence}`,
    "",
    `${SITE.name} is ${SITE.tagline.charAt(0).toLowerCase()}${SITE.tagline.slice(1)}. The facts:`,
    "",
    "- One binary (`hive`), one SQLite file (`~/.hive/hive.sqlite`), one console (`http://127.0.0.1:8765`, loopback only). Nothing leaves the machine except the model calls and the git pushes you allowed.",
    "- A harness of its own: a tool loop with limits, deny rules, hooks, MCP servers, skills, helpers and sub-sessions. Models: GitHub Copilot by default; OpenAI, Anthropic, Gemini, a dozen OpenAI-compatible vendors and local servers by key.",
    "- Tasks as conversations: you write what you want, a project's lead agent cuts it into steps, step agents work in throwaway clones, you watch and steer.",
    "- Code becomes pull requests: each step must pass the project's own check, a finished branch becomes a pull request, a review agent reads it and merges it or hands it to you.",
    "- Self-improvement under a constitution (RSI, recursive self-improvement): agents post suggestions and vote; a suggestion must name a check that fails on the default branch today and passes with the change; the loop builds the top voted suggestion on an experiment branch, measures it against the project's fixtures and private tests, archives the result and delivers what was kept as a pull request. The constitution is the owner's: hard stops, paths the loop may never touch, a kill switch (`hive stop`).",
    "- Many machines: `hive pair ADMIN@HOST` pairs a Linux machine over SSH; any machine with `hive-hand` can enrol itself with a six-word join code and dial out. A project says what its jobs need and the queue sends each job to a machine that offers it.",
    `- Platforms: macOS and Linux. Licence: ${SITE.license}, copyright ${SITE.year} ${SITE.holder}. Source: ${SITE.repo}`,
    "",
    "## Install",
    "",
    "```bash",
    INSTALL.blocks.script,
    INSTALL.blocks.source,
    INSTALL.blocks.after,
    "```",
    "",
    "## Pages",
    "",
    ...pages.filter((p) => p.kind === "page").map(line),
    "",
    "## Documentation",
    "",
    ...pages.filter((p) => p.kind === "doc").map(line),
    "",
    "## Source",
    "",
    `- [The repository](${SITE.repo}): the code, the README and the Markdown these pages are rendered from (docs/).`,
    "",
  ];
  return out.join("\n");
}
