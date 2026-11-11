// Breadcrumbs for the top bar (spec "Top bar": "demo-app › Board › Rate limit
// fixtures", each part a link). A page sets its own through useCrumbs; when
// it sets none the shell derives a sensible default from the route here.
// Pure and self-contained (no runtime imports), so node --test can load it:
// the shell passes the nav items in as `pages`.

export type Crumb = { label: string; href?: string };

export type CrumbProject = { slug: string; name?: string | null };

export type CrumbPage = { href: string; label: string };

// A route's canonical form: "/records" and "/records/" are one page.
export function normalizePath(pathname: string): string {
  const p = (pathname || "/").split("?")[0].split("#")[0];
  if (p === "" || p === "/") return "/";
  return p.endsWith("/") ? p : p + "/";
}

function projectName(slug: string, projects: CrumbProject[]): string {
  const p = projects.find((x) => x.slug === slug);
  const name = (p?.name || "").trim();
  return name || slug;
}

// The project page's tabs and their sub-pages, as the `tab`/`sub` params
// name them (lib/nav.ts resolves the aliases the same way). Overview is the
// project itself and adds no crumb.
const TAB_LABEL: Record<string, string> = {
  "for-you": "For you",
  work: "Work",
  tasks: "Work",
  chat: "Work",
  board: "Board",
  threads: "Threads",
  suggestions: "Suggestions",
  amendments: "Amendments",
  notices: "Notices",
  proposals: "Board",
  evolution: "Self-improvement",
  history: "History",
  timeline: "Timeline",
  team: "Team",
  env: "Environment",
};

const enc = encodeURIComponent;

function projectHref(slug: string, tab?: string | null, sub?: string | null): string {
  let h = "/project/?slug=" + enc(slug);
  if (tab && tab !== "overview") h += "&tab=" + enc(tab);
  if (sub) h += "&sub=" + enc(sub);
  return h;
}

// What the top bar shows when the page says nothing: Home on "/", the page's
// nav label elsewhere, "project › Tab" on a project page and "Page not
// found" on a path the console doesn't have. A detail id in
// the query (a record, a task, a run, a conversation) adds a last crumb so
// the phone's back arrow has somewhere to go.
export function defaultCrumbs(pathname: string, search: URLSearchParams, projects: CrumbProject[] = [], pages: CrumbPage[] = []): Crumb[] {
  const path = normalizePath(pathname);
  if (path === "/") return [{ label: "Home", href: "/" }];

  if (path === "/project/") {
    const slug = search.get("slug") || "";
    if (!slug) return [{ label: "Projects", href: "/" }];
    const crumbs: Crumb[] = [{ label: projectName(slug, projects), href: projectHref(slug) }];
    const tab = search.get("tab");
    const sub = search.get("sub");
    const label = (sub && TAB_LABEL[sub]) || (tab && TAB_LABEL[tab]) || null;
    if (label) crumbs.push({ label, href: projectHref(slug, tab, sub) });
    if (search.get("thread")) crumbs.push({ label: "Thread" });
    else if (search.get("post")) crumbs.push({ label: "Post" });
    return crumbs;
  }

  if (path === "/code-deliveries/") {
    return [
      { label: "Records", href: "/records/" },
      { label: "Code delivered", href: "/code-deliveries/" },
    ];
  }

  // The passkey pages (ADR-070 hosted front) sit outside the nav; they set
  // the same crumbs through useCrumbs, so the first render and the page's
  // own crumbs agree instead of flashing "Page not found".
  if (path === "/passkey/") return [{ label: "Passkey" }];
  if (path === "/passkey/register/") return [{ label: "Passkey", href: "/passkey/" }, { label: "Register" }];

  // Every page the console has is in `pages` or handled above, so a path
  // that matches none is the 404 page; naming it from the path read "Nope"
  // for /nope/.
  const page = pages.find((p) => normalizePath(p.href) === path);
  if (!page) return [{ label: "Page not found" }];
  const crumbs: Crumb[] = [{ label: page.label, href: path }];
  const project = search.get("project");
  if (project) crumbs.push({ label: projectName(project, projects), href: path + "?project=" + enc(project) });

  // One detail open on the page.
  const task = search.get("task");
  const detail = search.get("id") || task || search.get("key") || search.get("open");
  if (detail) crumbs.push({ label: path === "/chat/" && !task ? "Session" : detail });
  return crumbs;
}

// The phone's title: the last crumb, or the document title minus " · Hive".
export function phoneTitle(crumbs: Crumb[], documentTitle: string): string {
  const last = crumbs[crumbs.length - 1];
  if (last?.label) return last.label;
  const t = (documentTitle || "").replace(/\s*·\s*Hive\s*$/, "").trim();
  return t || "Hive";
}

// Where the phone's back arrow goes: the previous crumb that has a link.
export function backHref(crumbs: Crumb[]): string | null {
  if (crumbs.length < 2) return null;
  for (let i = crumbs.length - 2; i >= 0; i--) {
    const h = crumbs[i].href;
    if (h) return h;
  }
  return null;
}
