"use client";

import { useState } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { BookOpenIcon, KeyRoundIcon, LogOutIcon, MoreHorizontalIcon, PencilIcon, PlugZapIcon, PlusIcon, PowerIcon, PowerOffIcon } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { ProjectAvatar } from "@/components/hive/avatar";
import { showApiError } from "@/components/hive/confirm";
import { CardGrid, ItemCard, ItemCardTitle } from "@/components/hive/item-card";
import { KindTile } from "@/components/hive/kind-tile";
import { BrandLogo, LogoTile } from "@/components/hive/logo";
import { PageHeader } from "@/components/hive/page-header";
import { Disclosure, EmptyState, ErrorNotice, Kv, SkeletonRows } from "@/components/hive/panel";
import { SectionHeading } from "@/components/hive/section-heading";
import { SidePanel, useOpenItem } from "@/components/hive/side-panel";
import { StatusBadge } from "@/components/hive/status-badge";
import { Mono, When } from "@/components/hive/who";
import { ConnectDialog, McpServerDialog, SignInDialog, refreshMcp } from "@/components/connections/mcp-dialogs";
import { SkillDialog, SkillViewDialog, refreshSkills } from "@/components/connections/skill-dialogs";
import { connectionState, dedupeSuggestions, servesProjects, skillUseText, toolCountText, unconnectedSuggestions } from "@/components/connections/connections-state";
import { ToolkitGallery, refreshToolkit } from "@/components/project/toolkit";
import { query, skillRepoPath, writes, type McpAuth, type McpOptions, type McpServer, type McpServersPayload, type McpTransport, type ProjectsPayload, type Skill, type SkillsPayload, type ToolSuggestion, type ToolkitPayload, type ToolkitProject } from "@/lib/api";
import { num, plural } from "@/lib/format";
import { DEFAULT_OPTIONS, TRANSPORT_SHORT, authLabel, checkReason, endpoint, lastCheck, scopeLabel, scopeTitle, sortScoped, toolNames, type McpForm } from "@/lib/mcp";
import { useApi } from "@/lib/use-api";
import { cn } from "@/lib/utils";

// A tool with no brand mark of its own wears the MCP mark (spec "Logos" -
// tools with no logo), not its initials: every tool here is an MCP server.
const MCP_40 = <BrandLogo id="modelcontextprotocol" size={20} title="MCP server" />;
const MCP_28 = <BrandLogo id="modelcontextprotocol" size={16} title="MCP server" />;

// Connections (spec "Connections"): the tools and skills agents can use.
// A tool connection (an MCP server) is the bridge to another system -
// issues, feedback, docs, environments; a skill is a reusable set of
// instructions. Both are for every project unless one project is named -
// the Add dialogs start at "All projects" whatever the page filter says -
// and both reach agents through their brief. The page: what Hive suggests
// first (one click adds it), the tool cards, then the skills; a card opens
// its details beside the list.

// A quiet fold for the terminal side of things.
function Fold({ summary, children }: { summary: string; children: React.ReactNode }) {
  return (
    <Disclosure summary={summary} contentClassName="grid max-w-3xl gap-2 text-sm leading-relaxed text-muted-foreground anywhere">
      {children}
    </Disclosure>
  );
}

// The wizard's details step, filled from a suggestion (the same shape
// project/toolkit.tsx builds; kept here so this page can open it directly).
function toolForm(t: ToolSuggestion): Partial<McpForm> {
  return {
    name: t.name,
    project: t.scope === "shared" ? "" : t.project,
    transport: t.transport as McpTransport,
    command: t.command,
    args: t.args.join("\n"),
    url: t.url,
    auth: t.auth as McpAuth,
    auth_header: t.auth_header,
    note: t.about + " Suggested by hive toolkit for " + t.project + ".",
  };
}

// The first row of the page: tools the projects' checkouts ask for and do
// not have yet, one logo card each, with a one-click Add that opens the
// wizard at its details step, filled in. Hidden when nothing is suggested.
function SuggestedTools({ project, servers, onAdd }: { project: string; servers: McpServer[]; onAdd: (t: ToolSuggestion) => void }) {
  const q = useApi<ToolkitPayload>(project ? "toolkit?project=" + encodeURIComponent(project) : "toolkit", { refresh: 120000 });
  const d = q.data;
  if (!d) return null;
  const groups: ToolkitProject[] = d.projects ?? [{ project: d.project, tools: d.tools ?? [], skills: d.skills ?? [] }];
  const rows = dedupeSuggestions(unconnectedSuggestions(groups.flatMap((g) => g.tools), servers));
  if (!rows.length) return null;
  return (
    <section aria-label="Suggested for your projects" className="min-w-0">
      <SectionHeading count={rows.length} note="What the projects' files name and nothing reaches yet. One click adds it.">
        Suggested for your projects
      </SectionHeading>
      <CardGrid cols={3}>
        {rows.map((t) => (
          <article key={t.name} className="flex min-w-0 items-start gap-3 rounded-lg border border-dashed bg-card/60 p-4" aria-label={"Suggested tool " + t.title}>
            <LogoTile name={t.name} size={40} title={t.title} fallback={MCP_40} />
            <div className="min-w-0 flex-1">
              <h3 className="truncate text-sm font-semibold leading-5">{t.title || t.name}</h3>
              <p className="mt-0.5 line-clamp-2 text-[13px] leading-5 text-muted-foreground anywhere" title={t.why}>
                {t.why || t.about}
              </p>
              <p className="mt-1 flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                <ProjectAvatar slug={t.project} size={20} />
                <span className="truncate">{t.project + (t.also.length ? " and " + plural(t.also.length, "other project") : "")}</span>
              </p>
            </div>
            <Button size="sm" variant="outline" className="shrink-0" onClick={() => onAdd(t)} aria-label={"Add " + (t.title || t.name)}>
              <PlusIcon /> Add
            </Button>
          </article>
        ))}
      </CardGrid>
    </section>
  );
}

// Skills the projects' shapes suggest, as the gallery toolkit.tsx draws.
function SuggestedSkills({ project }: { project: string }) {
  const q = useApi<ToolkitPayload>(project ? "toolkit?project=" + encodeURIComponent(project) : "toolkit", { refresh: 120000 });
  const d = q.data;
  if (!d) return null;
  const groups: ToolkitProject[] = d.projects ?? [{ project: d.project, tools: d.tools ?? [], skills: d.skills ?? [] }];
  const rows = groups.filter((g) => g.skills.length > 0);
  if (!rows.length) return null;
  const total = rows.reduce((n, g) => n + g.skills.length, 0);
  return (
    <Disclosure variant="row" summary={<span className="text-sm font-medium">{"Skills Hive suggests"}</span>} hint={plural(total, "drafted skill")} contentClassName="mt-3 grid gap-3">
      {rows.map((g) => (
        <div key={g.project} className="grid gap-2">
          {rows.length > 1 ? <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">{g.project}</p> : null}
          <ToolkitGallery project={g.project} tools={g.tools} skills={g.skills} canDecide only="skill" />
        </div>
      ))}
    </Disclosure>
  );
}

export function ConnectionsPageClient() {
  const sp = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const project = sp.get("project") || "";
  const servers = useApiServers(project);
  const options = useApiOptions();
  const skills = useApiSkills(project);
  const projects = useApiProjects();
  const [openId, setOpenId] = useOpenItem("open");
  const [addMcp, setAddMcp] = useState(false);
  const [preset, setPreset] = useState<Partial<McpForm> | null>(null);
  const [editMcp, setEditMcp] = useState<McpServer | null>(null);
  const [connecting, setConnecting] = useState<McpServer | null>(null);
  const [signingIn, setSigningIn] = useState<McpServer | null>(null);
  const [addSkill, setAddSkill] = useState(false);
  const [editSkill, setEditSkill] = useState<Skill | null>(null);
  const [viewSkill, setViewSkill] = useState<Skill | null>(null);
  const go = (next: { project?: string }) => {
    const p = next.project ?? project;
    // `tab` was the old split of this page; a link that still carries it lands on the same page.
    router.replace(pathname + query({ project: p || undefined, open: openId || undefined }), { scroll: false });
  };
  const opts = options.data ?? DEFAULT_OPTIONS;
  const serverRows = sortScoped(servers.data?.servers ?? []);
  const skillRows = sortScoped(skills.data?.skills ?? []);
  const enabledServers = serverRows.filter((s) => s.status === "enabled").length;
  const enabledSkills = skillRows.filter((k) => k.status === "enabled").length;
  const scopeNote = project ? " Showing " + project + "'s own and the ones every project shares." : "";
  const open = openId ? (serverRows.find((s) => s.id === openId) ?? null) : null;
  // The skills section is this page's second half; `?tab=skills` links from before scroll to it.
  const skillsFirst = sp.get("tab") === "skills";

  const filter = (
    <Select value={project || "all"} onValueChange={(v) => go({ project: v === "all" ? "" : v })}>
      <SelectTrigger id="connections-project" aria-label="Project" className="w-full sm:w-44">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="all">All projects</SelectItem>
        {(projects.data?.projects ?? []).map((p) => (
          <SelectItem key={p.slug} value={p.slug}>
            {p.slug}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );

  return (
    <div className="flex min-w-0 items-start gap-0">
      {/* The page's column beside its side panel. The shell holds the one <main>. */}
      <div className="min-w-0 flex-1">
        <PageHeader
          title="Connections"
          subtitle="Tools and skills your agents can use"
          action={
            <Button onClick={() => setAddMcp(true)}>
              <PlusIcon /> Add connection
            </Button>
          }
        />

        <div className="grid min-w-0 gap-8 [&>*]:min-w-0">
          <SuggestedTools
            project={project}
            servers={serverRows}
            onAdd={(t) => {
              setPreset(toolForm(t));
            }}
          />

          <section aria-label="Tools" id="tools" className={cn(skillsFirst && "order-2")}>
            <SectionHeading count={servers.data ? serverRows.length : undefined} note={serverRows.length ? (enabledServers === serverRows.length ? (serverRows.length === 1 ? "It is on." : "All on.") : num(enabledServers) + " of " + num(serverRows.length) + " on.") + scopeNote : undefined} action={filter}>
              Tools
            </SectionHeading>
            {servers.error && !servers.data ? (
              <ErrorNotice error={servers.error} onRetry={() => void servers.reload()} />
            ) : !servers.data ? (
              <SkeletonRows n={3} />
            ) : serverRows.length === 0 ? (
              <EmptyState
                title="No tool connection yet"
                action={
                  <Button onClick={() => setAddMcp(true)}>
                    <PlusIcon /> Add connection
                  </Button>
                }
              >
                Connect the systems your agents should reach: issues, docs, a browser, your database. Hive fills in the details for the usual ones.
              </EmptyState>
            ) : (
              <CardGrid cols={3}>
                {serverRows.map((s) => (
                  <ServerCard key={s.id} s={s} selected={s.id === openId} onOpen={() => setOpenId(s.id)} onConnect={setConnecting} onEdit={setEditMcp} onSignIn={setSigningIn} />
                ))}
              </CardGrid>
            )}
            {serverRows.length ? (
              <div className="mt-3">
                <Fold summary="How agents use these">
                  <p>
                    Agents call a tool with <Mono>hive mcp call NAME TOOL</Mono> and see what it offers with <Mono>hive mcp tools NAME</Mono>. The tools that are on are named in every agent&apos;s brief.
                  </p>
                </Fold>
              </div>
            ) : null}
          </section>

          <section aria-label="Skills" id="skills" className={cn(skillsFirst && "order-1")}>
            <SectionHeading
              count={skills.data ? skillRows.length : undefined}
              note={skillRows.length ? (enabledSkills === skillRows.length ? (skillRows.length === 1 ? "It is on." : "All on.") : num(enabledSkills) + " of " + num(skillRows.length) + " on.") + " Agents read one before work it covers." : undefined}
              action={
                <Button size="sm" variant="outline" onClick={() => setAddSkill(true)}>
                  <PlusIcon /> Add skill
                </Button>
              }
            >
              Skills
            </SectionHeading>
            {skills.error && !skills.data ? (
              <ErrorNotice error={skills.error} onRetry={() => void skills.reload()} />
            ) : !skills.data ? (
              <SkeletonRows n={3} />
            ) : skillRows.length === 0 ? (
              <EmptyState
                title="No skill yet"
                action={
                  <Button variant="outline" onClick={() => setAddSkill(true)}>
                    <PlusIcon /> Add skill
                  </Button>
                }
              >
                A skill is a page of instructions agents read before work it covers: how to ship, how to test the UI, what a good PR looks like here.
              </EmptyState>
            ) : (
              <CardGrid cols={3}>
                {skillRows.map((k) => (
                  <SkillCard key={k.id} k={k} onView={setViewSkill} onEdit={setEditSkill} />
                ))}
              </CardGrid>
            )}
            <div className="mt-3 grid gap-2">
              <SuggestedSkills project={project} />
              {skillRows.length ? (
                <Fold summary="How agents use these">
                  <p>
                    Agents read a skill with <Mono>hive skill show NAME</Mono>; each read is counted. They add their own with <Mono>hive skill add</Mono>.
                  </p>
                </Fold>
              ) : null}
            </div>
          </section>
        </div>
      </div>

      <SidePanel open={!!open} onClose={() => setOpenId(null)} width="detail" title={open?.name ?? "Tool"} subtitle={open ? scopeTitle(open.project) : undefined} lead={open ? <LogoTile name={open.name} size={28} fallback={MCP_28} /> : undefined}>
        {open ? <ServerDetails s={open} onConnect={setConnecting} onEdit={setEditMcp} onSignIn={setSigningIn} /> : null}
      </SidePanel>

      <McpServerDialog open={addMcp} onOpenChange={setAddMcp} options={opts} onSignIn={setSigningIn} />
      {preset ? (
        <McpServerDialog
          open
          onOpenChange={(o) => {
            if (!o) {
              setPreset(null);
              refreshToolkit();
              refreshMcp();
            }
          }}
          options={opts}
          initial={preset}
          onSignIn={setSigningIn}
        />
      ) : null}
      <McpServerDialog open={!!editMcp} onOpenChange={(o) => (o ? null : setEditMcp(null))} options={opts} server={editMcp} />
      <ConnectDialog
        server={connecting}
        onOpenChange={(o) => (o ? null : setConnecting(null))}
        onSignIn={(s) => {
          setConnecting(null);
          setSigningIn(s);
        }}
      />
      <SignInDialog
        server={signingIn}
        onOpenChange={(o) => (o ? null : setSigningIn(null))}
        onSignedIn={(s) => {
          // Signed in: the natural next step is the check that lists its tools.
          setSigningIn(null);
          setConnecting(s);
        }}
      />
      <SkillDialog open={addSkill} onOpenChange={setAddSkill} />
      <SkillDialog open={!!editSkill} onOpenChange={(o) => (o ? null : setEditSkill(null))} skill={editSkill} />
      <SkillViewDialog
        skill={viewSkill}
        onOpenChange={(o) => (o ? null : setViewSkill(null))}
        onEdit={
          viewSkill && skillRepoPath(viewSkill)
            ? undefined
            : (k) => {
                setViewSkill(null);
                setEditSkill(k);
              }
        }
      />
    </div>
  );
}

// ----- data ------------------------------------------------------------------

function useApiServers(project: string) {
  return useApi<McpServersPayload>("mcp" + query({ project: project || undefined }), { refresh: 30000 });
}
function useApiOptions() {
  return useApi<McpOptions>("mcp/options", { refresh: false });
}
function useApiSkills(project: string) {
  return useApi<SkillsPayload>("skills" + query({ project: project || undefined }), { refresh: 30000 });
}
function useApiProjects() {
  return useApi<ProjectsPayload>("projects", { refresh: 60000 });
}

// ----- tools -----------------------------------------------------------------

// Turn on / off and sign out, the writes a tool card and its details share.
function useServerWrites(s: McpServer) {
  const [busy, setBusy] = useState(false);
  const [signingOut, setSigningOut] = useState(false);
  const on = s.status === "enabled";
  const signOut = async () => {
    setSigningOut(true);
    try {
      const r = await writes.logoutMcp(s.id);
      toast.success("Signed out of " + s.name + ".", { description: r.note || (r.revoked ? "The provider was told to forget the tokens too." : "The tokens on this machine are gone.") });
      refreshMcp();
    } catch (e) {
      showApiError(e, "Couldn't sign out of " + s.name);
    } finally {
      setSigningOut(false);
    }
  };
  const toggle = async () => {
    const next = on ? "disabled" : "enabled";
    setBusy(true);
    try {
      await writes.setMcpStatus(s.id, next);
      toast.success(s.name + (next === "enabled" ? " is on." : " is off."), { description: next === "enabled" ? "Agents see it from their next job." : "Agents stop seeing it. Its settings and secret reference stay." });
      refreshMcp();
    } catch (e) {
      showApiError(e, next === "enabled" ? s.name + " wasn't turned on" : s.name + " wasn't turned off");
    } finally {
      setBusy(false);
    }
  };
  return { on, busy, signingOut, toggle, signOut };
}

// The projects a connection serves: a stack of small avatars, or the words
// "All projects".
function Serves({ project, className }: { project: string; className?: string }) {
  const slugs = servesProjects(project);
  if (!slugs.length) return <span className={className}>All projects</span>;
  return (
    <span className={cn("inline-flex min-w-0 items-center gap-1.5", className)} aria-label={scopeTitle(project)}>
      <span className="flex -space-x-1">
        {slugs.slice(0, 4).map((p) => (
          <Tooltip key={p}>
            <TooltipTrigger asChild>
              <span className="inline-flex rounded-[5px] ring-2 ring-card" role="img" aria-label={"Project " + p}>
                <ProjectAvatar slug={p} size={20} />
              </span>
            </TooltipTrigger>
            <TooltipContent>{p}</TooltipContent>
          </Tooltip>
        ))}
      </span>
      {slugs.length > 4 ? <span>{"+" + (slugs.length - 4)}</span> : slugs.length === 1 ? <span className="truncate">{slugs[0]}</span> : null}
    </span>
  );
}

// A switched-off tool or skill: a dashed edge on a paper-coloured card and a
// grey tile. Its words keep full strength (TASK-173: the 80% opacity it had
// put 13 px text at 4.05:1 and the "Off" chip at 3.52:1); the "Off" chip
// says the state.
const OFF_CARD = "border-dashed bg-background shadow-none";

// One tool connection as a card (spec "Connections"): its logo, name, state
// as an icon + word, the projects it serves and how many tools the last
// check found. Turn off / Edit live in the "…" menu; the card opens its
// details beside the list.
function ServerCard({ s, selected, onOpen, onConnect, onEdit, onSignIn }: { s: McpServer; selected: boolean; onOpen: () => void; onConnect: (s: McpServer) => void; onEdit: (s: McpServer) => void; onSignIn: (s: McpServer) => void }) {
  const { on, busy, signingOut, toggle, signOut } = useServerWrites(s);
  const state = connectionState(s);
  const oauth = s.auth === "oauth";
  const signedIn = !!s.sign_in?.signed_in;
  const tools = toolCountText(s.last_check_note);
  return (
    <ItemCard onOpen={onOpen} active={selected} ariaLabel={"Open " + s.name} className={cn("gap-3 rounded-lg p-4 sm:p-4", !on && OFF_CARD)}>
      <div className="flex min-w-0 items-start gap-3">
        <LogoTile name={s.name} size={40} fallback={MCP_40} className={cn(!on && "grayscale")} />
        <div className="min-w-0 flex-1">
          <ItemCardTitle className="text-base font-semibold leading-6 tracking-tight">
            <span title={"Id " + s.id}>{s.name}</span>
          </ItemCardTitle>
          <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[13px] text-muted-foreground">
            <StatusBadge status={state.status} label={state.label} size="sm" title={state.detail || undefined} />
            {tools ? <span className="tnum">{tools}</span> : null}
          </div>
        </div>
        <DropdownMenu>
          <Tooltip>
            <TooltipTrigger asChild>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="icon-sm" className="-my-1 -mr-2 shrink-0" aria-label="More">
                  <MoreHorizontalIcon />
                </Button>
              </DropdownMenuTrigger>
            </TooltipTrigger>
            <TooltipContent>More</TooltipContent>
          </Tooltip>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={() => onConnect(s)}>
              <PlugZapIcon /> {s.last_check_at ? "Check now" : "Connect now"}
            </DropdownMenuItem>
            {oauth && !signedIn ? (
              <DropdownMenuItem onSelect={() => onSignIn(s)}>
                <KeyRoundIcon /> Sign in
              </DropdownMenuItem>
            ) : null}
            {oauth && signedIn ? (
              <DropdownMenuItem onSelect={() => void signOut()} disabled={signingOut}>
                <LogOutIcon /> Sign out
              </DropdownMenuItem>
            ) : null}
            <DropdownMenuItem onSelect={() => onEdit(s)}>
              <PencilIcon /> Edit
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={() => void toggle()} disabled={busy}>
              {on ? <PowerOffIcon /> : <PowerIcon />} {on ? "Turn off" : "Turn on"}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      <p className="line-clamp-2 text-[13px] leading-5 text-muted-foreground anywhere">{s.note || "No description yet. Edit it to tell agents what this tool is for and when to use it."}</p>
      <div className="mt-auto flex min-w-0 items-center gap-3 text-[13px] text-muted-foreground">
        <Serves project={s.project} />
        {oauth && !signedIn && on ? (
          <Button size="xs" className="ml-auto" onClick={() => onSignIn(s)} aria-label={"Sign in to " + s.name}>
            <KeyRoundIcon /> Sign in
          </Button>
        ) : null}
      </div>
    </ItemCard>
  );
}

// A tool's details in the side panel: what it is, the last check, the
// sign-in, where it is reached, and the same actions as the card's menu.
function ServerDetails({ s, onConnect, onEdit, onSignIn }: { s: McpServer; onConnect: (s: McpServer) => void; onEdit: (s: McpServer) => void; onSignIn: (s: McpServer) => void }) {
  const { on, busy, signingOut, toggle, signOut } = useServerWrites(s);
  const state = connectionState(s);
  const oauth = s.auth === "oauth";
  const signedIn = !!s.sign_in?.signed_in;
  const where = endpoint(s);
  const auth = s.auth !== "none" ? authLabel(s.auth) + (s.auth_header ? " (" + s.auth_header + ")" : "") : authLabel("none");
  const tools = toolNames(s.last_check_note || "");
  return (
    <div className="grid min-w-0 gap-5 text-sm">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <StatusBadge status={state.status} label={state.label} title={state.detail || undefined} />
        {state.detail && state.kind !== "connected" ? <span className="min-w-0 text-[13px] text-muted-foreground anywhere">{checkReason(state.detail)}</span> : null}
      </div>
      <p className="leading-relaxed text-foreground anywhere">{s.note || "No description yet. Edit it to tell agents what this tool is for and when to use it."}</p>
      <div className="flex flex-wrap gap-2">
        {oauth && !signedIn ? (
          <Button size="sm" onClick={() => onSignIn(s)}>
            <KeyRoundIcon /> Sign in
          </Button>
        ) : null}
        <Button size="sm" variant="outline" onClick={() => onConnect(s)}>
          <PlugZapIcon /> {s.last_check_at ? "Check now" : "Connect now"}
        </Button>
        <Button size="sm" variant="ghost" onClick={() => onEdit(s)}>
          <PencilIcon /> Edit
        </Button>
        <Button size="sm" variant="ghost" onClick={() => void toggle()} disabled={busy}>
          {on ? <PowerOffIcon /> : <PowerIcon />} {on ? "Turn off" : "Turn on"}
        </Button>
        {oauth && signedIn ? (
          <Button size="sm" variant="ghost" onClick={() => void signOut()} disabled={signingOut}>
            <LogOutIcon /> Sign out
          </Button>
        ) : null}
      </div>
      <section className="min-w-0">
        <SectionHeading as="h3" className="mb-2">
          Last check
        </SectionHeading>
        <CheckLine server={s} />
        {tools.length ? (
          <ul className="mt-2 flex flex-wrap gap-1" aria-label="Tools">
            {tools.map((t) => (
              <li key={t} className="rounded-md bg-muted px-1.5 py-0.5 font-mono text-xs">
                {t}
              </li>
            ))}
          </ul>
        ) : null}
      </section>
      {oauth ? (
        <section className="min-w-0">
          <SectionHeading as="h3" className="mb-2">
            Sign-in
          </SectionHeading>
          <SignInLine server={s} />
        </section>
      ) : null}
      <section className="min-w-0">
        <SectionHeading as="h3" className="mb-2">
          How it is reached
        </SectionHeading>
        <Kv
          className="text-[13px]"
          rows={[
            ["Serves", <Serves key="p" project={s.project} />],
            ["Transport", TRANSPORT_SHORT[s.transport]],
            ["Address", where ? <Mono key="w">{where}</Mono> : <span key="w" className="text-muted-foreground">no address</span>],
            ["Sign-in", auth + (s.secret_ref ? ", secret from " + s.secret_ref : "")],
            ["Id", <Mono key="i">{s.id}</Mono>],
          ]}
        />
      </section>
    </div>
  );
}

// "Connected 1 month ago. 12 tools, including list_issues, get_issue and
// create_issue." / "Failed 1 month ago: Hive couldn't read the secret.";
// the raw note in the tooltip.
function CheckLine({ server }: { server: McpServer }) {
  const c = lastCheck(server);
  const tools = toolNames(server.last_check_note || "");
  const reason = c.tone === "bad" ? checkReason(c.detail) : "";
  const shown = tools.slice(0, 3);
  const rest = tools.length - shown.length;
  return (
    <p className="flex min-w-0 items-start gap-2 text-sm leading-relaxed" title={server.last_check_note || undefined}>
      <PlugZapIcon className={cn("mt-[3px] size-3.5 shrink-0", c.tone === "ok" ? "text-ok" : c.tone === "bad" ? "text-bad" : "text-muted-foreground/60")} aria-hidden />
      <span className="min-w-0 anywhere">
        {server.last_check_at ? (
          <>
            <span className={cn("font-medium", c.tone === "bad" ? "text-bad" : "text-foreground")}>
              {c.label} <When d={server.last_check_at} />
              {reason ? ":" : "."}
            </span>
            {tools.length ? <span className="text-muted-foreground">{" " + plural(tools.length, "tool") + (rest > 0 ? ", including " : ": ") + andList(shown) + "."}</span> : reason ? <span className="text-muted-foreground">{" " + reason.replace(/[.\s]*$/, ".")}</span> : null}
          </>
        ) : (
          <span className="text-muted-foreground">Not checked yet. Connect it to see what it offers.</span>
        )}
      </span>
    </p>
  );
}

// The sign-in an `auth: oauth` server has on this machine, from the grant
// file the daemon reads - not from the last check, which may be older.
function SignInLine({ server }: { server: McpServer }) {
  const signedIn = !!server.sign_in?.signed_in;
  // The daemon's note names the terminal command when nothing is signed in;
  // here the Sign in button is the step, so the line only states the fact.
  const raw = server.sign_in?.note || "";
  const note = signedIn ? raw.charAt(0).toUpperCase() + raw.slice(1) : /grant unreadable/.test(raw) ? "The saved sign-in can't be read. Sign in again." : "Not signed in on this machine yet.";
  return (
    <p className="flex min-w-0 items-start gap-2 text-sm leading-relaxed" title={raw || undefined}>
      <KeyRoundIcon className={cn("mt-[3px] size-3.5 shrink-0", signedIn ? "text-ok" : "text-muted-foreground/60")} aria-hidden />
      <span className={cn("min-w-0 anywhere", signedIn ? "text-foreground" : "text-muted-foreground")}>{note || (signedIn ? "Signed in on this machine." : "Not signed in on this machine yet.")}</span>
    </p>
  );
}

const andList = (xs: string[]) => (xs.length < 2 ? xs.join("") : xs.slice(0, -1).join(", ") + " and " + xs[xs.length - 1]);

// ----- skills ----------------------------------------------------------------

// One skill as a card: the skill tile, its name and state, where it applies
// and how often it was read. Edit / Turn off in the "…" menu; the card
// opens the skill to read.
function SkillCard({ k, onView, onEdit }: { k: Skill; onView: (k: Skill) => void; onEdit: (k: Skill) => void }) {
  const [busy, setBusy] = useState(false);
  const on = k.status === "enabled";
  const toggle = async () => {
    const next = on ? "disabled" : "enabled";
    setBusy(true);
    try {
      await writes.setSkillStatus(k.id, next);
      toast.success(k.name + (next === "enabled" ? " is on." : " is off."), {
        description: next === "enabled" ? "Agents of " + scopeLabel(k.project) + " hear about it from their next job." : "Agents stop hearing about it. The instructions stay.",
      });
      refreshSkills();
    } catch (e) {
      showApiError(e, next === "enabled" ? k.name + " wasn't turned on" : k.name + " wasn't turned off");
    } finally {
      setBusy(false);
    }
  };
  const by = k.created_by === "human" || k.created_by === "owner" ? "you" : k.created_by;
  const repoPath = skillRepoPath(k);
  return (
    <ItemCard onOpen={() => onView(k)} ariaLabel={"Read the skill " + k.name} className={cn("gap-3 rounded-lg p-4 sm:p-4", !on && OFF_CARD)}>
      <div className="flex min-w-0 items-start gap-3">
        <KindTile kind="skill" size={40} className={cn(!on && "grayscale")} />
        <div className="min-w-0 flex-1">
          <ItemCardTitle className="text-base font-semibold leading-6 tracking-tight">
            <span title={"Id " + k.id}>{k.name}</span>
          </ItemCardTitle>
          <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-[13px] text-muted-foreground">
            {on ? <StatusBadge status="enabled" label="On" size="sm" /> : <StatusBadge status="disabled" label="Off" size="sm" />}
            <span>{skillUseText(k)}</span>
          </div>
        </div>
        <DropdownMenu>
          <Tooltip>
            <TooltipTrigger asChild>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="icon-sm" className="-my-1 -mr-2 shrink-0" aria-label="More">
                  <MoreHorizontalIcon />
                </Button>
              </DropdownMenuTrigger>
            </TooltipTrigger>
            <TooltipContent>More</TooltipContent>
          </Tooltip>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={() => onView(k)}>
              <BookOpenIcon /> Read
            </DropdownMenuItem>
            {repoPath ? null : (
              <DropdownMenuItem onSelect={() => onEdit(k)}>
                <PencilIcon /> Edit
              </DropdownMenuItem>
            )}
            <DropdownMenuSeparator />
            <DropdownMenuItem onSelect={() => void toggle()} disabled={busy}>
              {on ? <PowerOffIcon /> : <PowerIcon />} {on ? "Turn off" : "Turn on"}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      <p className="line-clamp-2 text-[13px] leading-5 text-muted-foreground anywhere">{k.description}</p>
      <div className="mt-auto flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-[13px] text-muted-foreground">
        <Serves project={k.project} />
        <span className="ml-auto truncate" title={repoPath ? "Kept by the repository: " + repoPath : by ? "Added by " + by : undefined}>
          {"Updated "}
          <When d={k.updated_at} />
        </span>
      </div>
    </ItemCard>
  );
}
