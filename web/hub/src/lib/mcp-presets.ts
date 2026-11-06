// Well-known tools (MCP servers) the Add a tool wizard offers as a start,
// with everything but the secret filled in. The list mirrors the toolkit's
// catalogue in crates/pulse-runner/src/toolkit/tools.rs (the hub's GET
// toolkit lists only the names and words of that catalogue, so the details
// that start a server live here too); a project's own suggestions from GET
// toolkit come first and win over a preset of the same name. No React and
// no `@/` imports, so `node --test` can load it.

import type { McpAuth, McpTransport, ToolSuggestion } from "./api";
import type { McpForm } from "./mcp";

export interface McpPreset {
  name: string;
  title: string;
  about: string;
  docs: string;
  transport: McpTransport;
  command: string;
  args: string[];
  url: string;
  auth: McpAuth;
  auth_header: string;
  // What the secret is, in words; "" when the server needs none.
  secret: string;
  // For every project, or for one.
  shared: boolean;
  // Why it is offered (a project's suggestion), or "".
  why?: string;
  project?: string;
}

export const MCP_PRESETS: McpPreset[] = [
  { name: "github", title: "GitHub", about: "Issues, pull requests, reviews, Actions runs and code search on GitHub, read and written as you.", docs: "https://github.com/github/github-mcp-server", transport: "http", command: "", args: [], url: "https://api.githubcopilot.com/mcp/", auth: "oauth", auth_header: "", secret: "", shared: true },
  { name: "playwright", title: "Playwright browser", about: "A real browser: open the app, click through it, read the page and take screenshots, so UI work is seen before it is called done.", docs: "https://github.com/microsoft/playwright-mcp", transport: "stdio", command: "npx", args: ["-y", "@playwright/mcp@latest"], url: "", auth: "none", auth_header: "", secret: "", shared: true },
  { name: "context7", title: "Context7 docs", about: "Current documentation and code examples for the libraries a project depends on, so agents stop guessing at APIs.", docs: "https://github.com/upstash/context7", transport: "http", command: "", args: [], url: "https://mcp.context7.com/mcp", auth: "none", auth_header: "", secret: "", shared: true },
  { name: "linear", title: "Linear", about: "Issues, projects and cycles in Linear, as you.", docs: "https://linear.app/docs/mcp", transport: "http", command: "", args: [], url: "https://mcp.linear.app/mcp", auth: "oauth", auth_header: "", secret: "", shared: true },
  { name: "atlassian", title: "Jira and Confluence", about: "Jira issues and Confluence pages, as you.", docs: "https://www.atlassian.com/platform/remote-mcp-server", transport: "sse", command: "", args: [], url: "https://mcp.atlassian.com/v1/sse", auth: "oauth", auth_header: "", secret: "", shared: true },
  { name: "notion", title: "Notion", about: "Pages and databases in the Notion workspace you sign into.", docs: "https://developers.notion.com/docs/mcp", transport: "http", command: "", args: [], url: "https://mcp.notion.com/mcp", auth: "oauth", auth_header: "", secret: "", shared: true },
  { name: "figma", title: "Figma", about: "Design files and components from Figma, for building what was designed.", docs: "https://help.figma.com/hc/en-us/articles/32132100833559", transport: "http", command: "", args: [], url: "https://mcp.figma.com/mcp", auth: "oauth", auth_header: "", secret: "", shared: true },
  { name: "sentry", title: "Sentry", about: "Issues, stack traces and releases from Sentry, so a fix starts from the real error.", docs: "https://docs.sentry.io/product/sentry-mcp/", transport: "http", command: "", args: [], url: "https://mcp.sentry.dev/mcp", auth: "oauth", auth_header: "", secret: "", shared: false },
  { name: "gitlab", title: "GitLab", about: "Issues, merge requests and pipelines on GitLab (18.3 or later), as you.", docs: "https://docs.gitlab.com/user/gitlab_duo/model_context_protocol/mcp_server/", transport: "http", command: "", args: [], url: "https://gitlab.com/api/v4/mcp", auth: "oauth", auth_header: "", secret: "", shared: true },
  { name: "chrome-devtools", title: "Chrome DevTools", about: "Chrome with DevTools open: performance traces, network and console, and the page itself.", docs: "https://github.com/ChromeDevTools/chrome-devtools-mcp", transport: "stdio", command: "npx", args: ["-y", "chrome-devtools-mcp@latest", "--headless"], url: "", auth: "none", auth_header: "", secret: "", shared: true },
  { name: "postgres", title: "Postgres", about: "Reads the schema, explains query plans and runs read-only SQL against a project's database.", docs: "https://github.com/crystaldba/postgres-mcp", transport: "stdio", command: "uvx", args: ["postgres-mcp", "--access-mode=restricted"], url: "", auth: "env", auth_header: "DATABASE_URI", secret: "the database address, postgresql://user:password@host/db", shared: false },
  { name: "supabase", title: "Supabase", about: "Tables, SQL, migrations, edge functions and logs of a Supabase project, as you.", docs: "https://github.com/supabase-community/supabase-mcp", transport: "http", command: "", args: [], url: "https://mcp.supabase.com/mcp", auth: "oauth", auth_header: "", secret: "", shared: false },
  { name: "stripe", title: "Stripe", about: "Customers, payments, subscriptions and the Stripe docs, from the account you sign into (test mode first).", docs: "https://docs.stripe.com/mcp", transport: "http", command: "", args: [], url: "https://mcp.stripe.com", auth: "oauth", auth_header: "", secret: "", shared: false },
  { name: "vercel", title: "Vercel", about: "Deployments, build logs and project settings on Vercel, as you.", docs: "https://vercel.com/docs/mcp/vercel-mcp", transport: "http", command: "", args: [], url: "https://mcp.vercel.com", auth: "oauth", auth_header: "", secret: "", shared: false },
  { name: "cloudflare", title: "Cloudflare", about: "Workers, KV, D1, R2, logs and the Cloudflare docs, as you.", docs: "https://github.com/cloudflare/mcp-server-cloudflare", transport: "http", command: "", args: [], url: "https://mcp.cloudflare.com/mcp", auth: "oauth", auth_header: "", secret: "", shared: false },
  { name: "aws-knowledge", title: "AWS knowledge", about: "AWS documentation, API references and architecture guidance; it never touches your account.", docs: "https://github.com/awslabs/mcp", transport: "http", command: "", args: [], url: "https://knowledge-mcp.global.api.aws", auth: "none", auth_header: "", secret: "", shared: true },
  { name: "kubernetes", title: "Kubernetes", about: "Pods, deployments, logs and events of the cluster kubectl already points at.", docs: "https://github.com/Flux159/mcp-server-kubernetes", transport: "stdio", command: "npx", args: ["-y", "mcp-server-kubernetes"], url: "", auth: "none", auth_header: "", secret: "", shared: false },
  { name: "huggingface", title: "Hugging Face", about: "Models, datasets, Spaces and papers on the Hugging Face Hub.", docs: "https://huggingface.co/settings/mcp", transport: "http", command: "", args: [], url: "https://huggingface.co/mcp", auth: "none", auth_header: "", secret: "", shared: true },
];

// A project's suggestion as a preset (it carries everything a preset does).
export function presetFromSuggestion(t: ToolSuggestion): McpPreset {
  return {
    name: t.name,
    title: t.title,
    about: t.about,
    docs: t.docs,
    transport: t.transport as McpTransport,
    command: t.command,
    args: t.args,
    url: t.url,
    auth: t.auth as McpAuth,
    auth_header: t.auth_header,
    secret: t.secret,
    shared: t.scope === "shared",
    why: t.why,
    project: t.project,
  };
}

// The picker's list: the projects' suggestions first (one per name), then
// the presets they don't already cover; a name already connected drops out.
export function toolChoices(suggestions: ToolSuggestion[], taken: string[]): { suggested: McpPreset[]; popular: McpPreset[] } {
  const have = new Set(taken);
  const seen = new Set<string>();
  const suggested: McpPreset[] = [];
  for (const s of suggestions) {
    if (have.has(s.name) || seen.has(s.name)) continue;
    seen.add(s.name);
    suggested.push(presetFromSuggestion(s));
  }
  const popular = MCP_PRESETS.filter((p) => !have.has(p.name) && !seen.has(p.name));
  return { suggested, popular };
}

// How a preset signs in, in a couple of words, for its card.
export function presetAccess(p: Pick<McpPreset, "auth" | "transport">): string {
  if (p.auth === "oauth") return "Signs in once";
  if (p.auth === "none") return p.transport === "stdio" ? "Runs here, no sign-in" : "No sign-in";
  return "Needs a secret";
}

// The form a preset starts the details step with.
export function formFromPreset(p: McpPreset, project = ""): Partial<McpForm> {
  return {
    name: p.name,
    project: p.shared ? "" : p.project || project,
    transport: p.transport,
    command: p.command,
    args: p.args.join("\n"),
    url: p.url,
    auth: p.auth,
    auth_header: p.auth_header,
    note: p.about,
  };
}
