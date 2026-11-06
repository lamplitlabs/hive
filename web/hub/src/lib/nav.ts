// Href builders. Every route is a static page; state lives in search params
// so the export has no dynamic segments and the pulse hub serves one
// index.html per route.

export type ProjectCanonicalTab =
  | "overview"
  | "for-you"
  | "work"
  | "board"
  | "evolution"
  | "team"
  | "history";
export type ProjectLegacyTab =
  | "tasks"
  | "chat"
  | "threads"
  | "proposals"
  | "suggestions"
  | "amendments"
  | "timeline"
  | "notices"
  | "env";
export type ProjectTab = ProjectCanonicalTab | ProjectLegacyTab;
export type ProjectTabResolution = { tab: ProjectCanonicalTab; sub?: string };

export const PROJECT_TABS: { id: ProjectTab; label: string }[] = [
  { id: "overview", label: "Overview" },
  { id: "for-you", label: "For you" },
  { id: "work", label: "Work" },
  { id: "board", label: "Board" },
  { id: "evolution", label: "Self-improvement" },
  { id: "team", label: "Team & tools" },
  { id: "history", label: "History" },
];

export function isProjectTab(s: string | null): s is ProjectTab {
  return !!s && !!resolveProjectTab(s);
}

export function resolveProjectTab(raw: string | null): ProjectTabResolution {
  switch (raw) {
    case "tasks":
    case "chat":
      return { tab: "work" };
    case "threads":
      return { tab: "board", sub: "threads" };
    case "suggestions":
      return { tab: "board", sub: "suggestions" };
    case "amendments":
      return { tab: "board", sub: "amendments" };
    case "notices":
      return { tab: "board", sub: "notices" };
    case "proposals":
      return { tab: "for-you" };
    case "timeline":
      return { tab: "history", sub: "timeline" };
    // The build-and-run notes and records sit with the leads and the toolkit now.
    case "env":
      return { tab: "team", sub: "env" };
    case "for-you":
    case "work":
    case "board":
    case "evolution":
    case "team":
    case "history":
    case "overview":
      return { tab: raw };
    default:
      return { tab: "overview" };
  }
}

function qs(
  params: Record<string, string | number | undefined | null>,
): string {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(params))
    if (v !== undefined && v !== null && v !== "") q.set(k, String(v));
  const s = q.toString();
  return s ? "?" + s : "";
}

export const hrefs = {
  home: () => "/",
  // Everything that waits on the owner, across projects; `project` narrows to one.
  forYou: (f: { project?: string } = {}) => "/for-you/" + qs({ project: f.project }),
  // The over-time metrics that used to sit at the foot of Home.
  insights: (f: { project?: string; range?: string } = {}) =>
    "/insights/" + qs({ project: f.project, range: f.range }),
  project: (
    slug: string,
    tab?: ProjectTab | string,
    extra: Record<string, string | undefined> = {},
  ) => {
    const r = resolveProjectTab(tab ?? null);
    return (
      "/project/" +
      qs({
        slug,
        tab: r.tab !== "overview" ? r.tab : undefined,
        sub: r.sub,
        ...extra,
      })
    );
  },
  thread: (slug: string, key: string) =>
    "/project/" + qs({ slug, tab: "board", sub: "threads", thread: key }),
  post: (slug: string, id: string) =>
    "/project/" + qs({ slug, tab: "board", sub: "threads", post: id }),
  // Opening a task means its conversation on the Chat page: one feed of
  // everything that happened to it (task ids are unique across projects).
  task: (id: string) => "/chat/" + qs({ task: id }),
  records: (f: { kind?: string; status?: string; project?: string } = {}) =>
    "/records/" + qs(f),
  record: (id: string, project?: string) => "/records/" + qs({ id, project }),
  codeDeliveries: (f: { project?: string } = {}) =>
    "/code-deliveries/" + qs({ project: f.project }),
  compute: () => "/compute/",
  settings: (section?: string) => "/settings/" + (section ? "#" + section : ""),
  conversations: (id?: string) => "/chat/" + qs({ id }),
  harness: (
    f: { id?: string; project?: string; status?: string; agent?: string } = {},
  ) =>
    "/harness/" +
    qs({ id: f.id, project: f.project, status: f.status, agent: f.agent }),
  // MCP servers and skills (TASK-022); `tab` opens the skills half, `project` filters both.
  connections: (f: { project?: string; tab?: "mcp" | "skills" } = {}) =>
    "/connections/" +
    qs({ project: f.project, tab: f.tab !== "mcp" ? f.tab : undefined }),
  votes: (f: { project?: string; status?: string; kind?: string } = {}) =>
    "/votes/" + qs(f),
  // Standing jobs and the `.pulse/pulse.yaml` entries that keep them; `project` narrows to one.
  automations: (f: { project?: string } = {}) =>
    "/automations/" + qs({ project: f.project }),
  // What agents remembered across runs; `key` opens one file, `q` searches.
  memory: (f: { key?: string; q?: string } = {}) =>
    "/memory/" + qs({ key: f.key, q: f.q }),
  tasks: (f: { project?: string; status?: string } = {}) => "/tasks/" + qs(f),
};
