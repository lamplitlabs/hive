// What the site knows about Hive and about itself. Pure (no I/O, no JSX), so
// the node tests import it as it is. The facts are the repository's: the
// README and docs/ are the source of truth, and every page's copy is checked
// against them by hand, its length and uniqueness by src/lib/*.test.mjs.

export const SITE = {
  name: "Hive",
  domain: "hivefleet.dev",
  url: "https://hivefleet.dev",
  /** The category line. */
  tagline: "The self-improving agent harness",
  /** The one sentence every package uses. */
  sentence:
    "Hive is an open-source harness for coding agents. It runs on your own machine as one binary, improves itself under rules you write, and pairs more machines when the work needs them.",
  repo: "https://github.com/lamplitlabs/hive",
  license: "MIT",
  holder: "Lamplit Labs",
  holderUrl: "https://www.lamplitlabs.com",
  year: 2026,
  console: "http://127.0.0.1:8765",
} as const;

/** A file of the repository on GitHub, as a reader follows it. */
export function repoFile(path: string): string {
  return `${SITE.repo}/blob/main/${path.replace(/^\/+/, "")}`;
}

/**
 * The install lines (the contract's section 4). `blocks` are the code blocks
 * as every package prints them, character for character; `script`, `brew`
 * and `source` are the bare commands a copy button puts on the clipboard.
 * The Homebrew formula is `hivefleet`: Homebrew's own list has another
 * `hive` (docs/release.md, "Homebrew").
 */
export const INSTALL = {
  script: "curl -fsSL https://hivefleet.dev/install.sh | sh",
  brew: "brew install lamplitlabs/tap/hivefleet",
  source: "cargo install --git https://github.com/lamplitlabs/hive --locked --root ~/.hive hive",
  blocks: {
    script: "curl -fsSL https://hivefleet.dev/install.sh | sh      # macOS, Linux and WSL; installs ~/.hive/bin/hive",
    brew: "brew install lamplitlabs/tap/hivefleet   # with Homebrew, on macOS, Linux and WSL; installs hive",
    source: "cargo install --git https://github.com/lamplitlabs/hive --locked --root ~/.hive hive   # from source: Rust 1.95+, Node.js 22+",
    after: [
      'export PATH="$HOME/.hive/bin:$PATH"    # add to your shell profile',
      "hive version",
      "hive hub                               # the console, at http://127.0.0.1:8765",
    ].join("\n"),
  },
} as const;

/** One static page: where it lives, what the tab and the search result say, and one line for llms.txt. */
export type SitePage = {
  path: string;
  /** The whole <title>, at most 60 characters. */
  title: string;
  /** The meta description, 70 to 160 characters. */
  description: string;
  /** The word the header and the footer use for it. */
  nav: string;
  /** One line for /llms.txt. */
  summary: string;
  /** Words a reader may type into the search for this page that the summary lacks. */
  keywords?: string;
};

export const PAGES = {
  home: {
    path: "/",
    title: "Hive: the self-improving agent harness",
    description:
      "An open-source harness for coding agents. One binary on your own machine, self-improvement under rules you write, and more machines when the work needs them.",
    nav: "Home",
    summary: "What Hive is, how a task goes, the self-improvement loop, pairing machines and the questions people ask first.",
  },
  install: {
    path: "/install/",
    title: "Install Hive and run your first task",
    description:
      "Install Hive on macOS, Linux or Windows with WSL, open the console on 127.0.0.1:8765, connect a model, add a project and give its lead agent a first task.",
    nav: "Install",
    summary: "The install line, building from source, and the first task step by step.",
    keywords: "quickstart quick start getting started download setup set up homebrew brew curl cargo tutorial windows wsl",
  },
  harness: {
    path: "/harness/",
    title: "The harness: how a Hive agent runs",
    description:
      "Hive runs every agent on its own harness: a tool loop with budgets, deny rules, hooks, MCP servers, skills, helpers and sub-sessions, on the model you choose.",
    nav: "Harness",
    summary: "The tool loop, limits, deny rules, hooks, MCP servers, skills, helpers, sub-sessions and model providers.",
  },
  rsi: {
    path: "/rsi/",
    title: "Self-improvement under a constitution (RSI)",
    description:
      "How Hive improves itself: agents suggest and vote, a check must fail today and pass with the change, results are measured, and you write the constitution.",
    nav: "Self-improvement",
    summary: "Suggestions, votes, the check that must fail today, experiments, private tests, the archive, the constitution, hard stops and the kill switch.",
  },
  machines: {
    path: "/machines/",
    title: "Pair machines: hive pair, join codes, hive-hand",
    description:
      "Pair a Linux machine over SSH with hive pair, or enrol any machine with a six-word join code. Jobs say what they need and go to a machine that offers it.",
    nav: "Machines",
    summary: "Pairing over SSH with hive pair, join codes and hive-hand, what a job needs and what a machine offers.",
  },
  docs: {
    path: "/docs/",
    title: "Hive documentation: guides and reference",
    description:
      "Guides that take one job from start to finish, and the reference for every part of Hive: the console, the harness, self-improvement, delivery and machines.",
    nav: "Docs",
    summary: "The index of the documentation: the guides, the reference for every part, and a search over both.",
  },
} as const satisfies Record<string, SitePage>;

/** The static pages in the order the header, the footer, the sitemap and llms.txt list them. */
export const STATIC_PAGES: readonly SitePage[] = [PAGES.home, PAGES.install, PAGES.harness, PAGES.rsi, PAGES.machines, PAGES.docs];

/** The 404 page: never indexed, never in the sitemap. */
export const NOT_FOUND = {
  title: "Page not found · Hive",
  description: "There is no page at this address. The front page says what Hive is, and the documentation index lists every page of the docs.",
} as const;

/** An absolute address on the site for a path that starts with a slash. */
export function absoluteUrl(path: string): string {
  return `${SITE.url}${path.startsWith("/") ? path : `/${path}`}`;
}
