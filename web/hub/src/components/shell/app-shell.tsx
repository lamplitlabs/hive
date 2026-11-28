"use client";

import Link from "next/link";
import { usePathname, useSearchParams } from "next/navigation";
import { Fragment, Suspense, useCallback, useEffect, useRef, useState } from "react";
import {
  MessageSquarePlusIcon,
  MoreHorizontalIcon,
  PanelLeftCloseIcon,
  PanelLeftOpenIcon,
  PlusIcon,
  SearchIcon,
  UsersIcon,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { AskHost } from "@/components/ask/ask-panel";
import { ProjectAvatar } from "@/components/hive/avatar";
import { useInbox } from "@/components/hive/inbox";
import { useHardStops } from "@/components/sessions/hard-stops";
import {
  Kbd,
  ShortcutHelp,
  useGlobalShortcuts,
} from "@/components/hive/shortcut-help";
import { AddProjectDialog } from "@/components/project/project-dialogs";
import { NewTaskDialog } from "@/components/tasks/task-dialogs";
import { openAsk } from "@/components/ask/ask-panel";
import { COMMONS_HINT, projectLabel, type Org } from "@/lib/api";
import { hrefs } from "@/lib/nav";
import { useApi } from "@/lib/use-api";
import { cn } from "@/lib/utils";
import { AwakeTheme } from "./awake";
import { BottomBar } from "./bottom-bar";
import { CommandPaletteHost, setPaletteOpen } from "./command-palette";
import { CrumbsProvider, useCurrentCrumbs } from "./crumbs";
import { NAV_ICON } from "./nav-icons";
import {
  machineName,
  NAV_GROUPS,
  shortcutWords,
  type NavItem,
} from "./nav-items";
import { OwnerNotifier } from "./owner-notifier";
import { OwnerRow } from "./owner-row";
import { HiveMark } from "./hive-mark";
import { RailTip } from "./rail-tip";
import { sidebarToggleLabel } from "./sidebar-state";
import { SystemStatus } from "./system-status";
import { PhoneBar, TopBar, TOPBAR_H } from "./top-bar";
import { UpdateBanner } from "./update-banner";
import { useSidebarCollapsed } from "./use-sidebar";

const focusRing =
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

// ----- header -----------------------------------------------------------------

// The one button that folds the sidebar to its rail and back.
function SidebarToggle() {
  const [collapsed, setCollapsed] = useSidebarCollapsed();
  const label = sidebarToggleLabel(collapsed);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          className="text-muted-foreground hover:text-foreground"
          aria-label={label}
          aria-expanded={!collapsed}
          aria-controls="sidebar"
          onClick={() => setCollapsed(!collapsed)}
        >
          {/* Both icons render and CSS picks one, so a collapsed reload shows the right arrow before React hydrates. */}
          <PanelLeftCloseIcon aria-hidden className="size-4 collapsed:hidden" />
          <PanelLeftOpenIcon
            aria-hidden
            className="hidden size-4 collapsed:block"
          />
        </Button>
      </TooltipTrigger>
      <TooltipContent side={collapsed ? "right" : "bottom"} sideOffset={8}>
        {label}
      </TooltipContent>
    </Tooltip>
  );
}

// The mark, the machine's name and the fold button. On the rail the name goes
// and the button drops under the mark.
function SidebarHeader() {
  const { data: org } = useApi<Org>("org");
  const name = machineName(org?.host);
  return (
    <div className="mx-3 mt-3 mb-2 flex shrink-0 items-center gap-1 collapsed:mx-2 collapsed:flex-col collapsed:items-stretch collapsed:gap-1">
      <Link
        href="/"
        className={cn(
          "brand-link flex h-10 min-w-0 flex-1 items-center gap-2.5 rounded-md px-1.5 collapsed:justify-center collapsed:px-0",
          focusRing,
        )}
        // The name holds the two lines the link shows, in their order
        // (WCAG 2.5.3), then where it leads.
        aria-label={"Hive, " + name + ": home"}
      >
        <HiveMark />
        <span className="min-w-0 collapsed:sr-only">
          <span className="block truncate text-base font-semibold leading-tight tracking-tight">
            Hive
          </span>{" "}
          <span
            className="block truncate text-xs leading-tight text-muted-foreground"
            title={org?.host || undefined}
          >
            {name}
          </span>
        </span>
      </Link>
      <div className="flex shrink-0 justify-center">
        <SidebarToggle />
      </div>
    </div>
  );
}

// ----- New -----------------------------------------------------------------

// The one ink button: start a session, give a task, add a project.
function NewMenu({ onAddProject }: { onAddProject: () => void }) {
  const [task, setTask] = useState(false);
  const pathname = usePathname();
  const params = useSearchParams();
  const project = pathname.startsWith("/project")
    ? params.get("slug") || undefined
    : undefined;
  return (
    <>
      <DropdownMenu>
        <RailTip label="New" hint="A session, a task or a project">
          <DropdownMenuTrigger asChild>
            <Button
              className="h-9 w-full justify-start gap-2 px-3 collapsed:w-9 collapsed:justify-center collapsed:px-0"
              aria-label="New"
            >
              <PlusIcon aria-hidden className="size-4" strokeWidth={2.25} />
              <span className="collapsed:sr-only">New</span>
            </Button>
          </DropdownMenuTrigger>
        </RailTip>
        <DropdownMenuContent align="start" className="w-60 p-1.5">
          <DropdownMenuItem asChild className="min-h-10 gap-2.5 rounded-md">
            <Link href={hrefs.conversations()}>
              <MessageSquarePlusIcon
                aria-hidden
                className="size-4 text-muted-foreground"
              />
              <span className="min-w-0">
                <span className="block font-medium">New session</span>
                <span className="block truncate text-xs text-muted-foreground">
                  Ask an agent to do something
                </span>
              </span>
            </Link>
          </DropdownMenuItem>
          <DropdownMenuItem
            className="min-h-10 gap-2.5 rounded-md"
            onSelect={() => setTask(true)}
          >
            <NAV_ICON.tasks
              aria-hidden
              className="size-4 text-muted-foreground"
            />
            <span className="min-w-0">
              <span className="block font-medium">New task</span>
              <span className="block truncate text-xs text-muted-foreground">
                Work for a project&apos;s lead to plan
              </span>
            </span>
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            className="min-h-10 gap-2.5 rounded-md"
            onSelect={onAddProject}
          >
            <PlusIcon aria-hidden className="size-4 text-muted-foreground" />
            <span className="min-w-0">
              <span className="block font-medium">Add project</span>
              <span className="block truncate text-xs text-muted-foreground">
                A repository on this machine
              </span>
            </span>
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <NewTaskDialog open={task} onOpenChange={setTask} project={project} />
    </>
  );
}

// A muted row that opens the command palette.
function SearchRow() {
  return (
    <RailTip label="Search" hint="⌘K">
      <button
        type="button"
        data-palette-trigger=""
        onClick={(e) => setPaletteOpen(true, e.currentTarget)}
        // Named by its word, "Search"; the shortcut is in aria-keyshortcuts,
        // and the hint drawn beside the word is for the eye only.
        aria-haspopup="dialog"
        aria-keyshortcuts="Control+k Meta+k"
        className={cn(
          "flex h-9 w-full items-center gap-2 rounded-md border border-input bg-background px-2.5 text-sm text-muted-foreground transition-colors hover:bg-muted hover:text-foreground motion-reduce:transition-none collapsed:w-9 collapsed:justify-center collapsed:border-0 collapsed:bg-transparent collapsed:px-0",
          focusRing,
        )}
      >
        <SearchIcon
          aria-hidden
          className="size-4 shrink-0"
          strokeWidth={1.75}
        />
        <span className="min-w-0 flex-1 truncate text-left collapsed:sr-only">
          Search
        </span>
        <Kbd className="collapsed:hidden" aria-hidden>
          ⌘K
        </Kbd>
      </button>
    </RailTip>
  );
}

// ----- nav ------------------------------------------------------------------

// The count of things waiting for you, in the one warm colour; on the rail a dot.
function Waiting({ n, className }: { n: number; className?: string }) {
  if (!n) return null;
  return (
    <>
      <span
        key={n}
        className={cn(
          "tnum pop inline-flex h-5 min-w-5 items-center justify-center rounded-full bg-you px-1.5 text-xs font-semibold text-[oklch(0.3_0.06_60)] collapsed:hidden",
          className,
        )}
        aria-label={n + " waiting"}
      >
        {n}
      </span>
      <span
        aria-hidden
        className="absolute top-1.5 right-1.5 hidden size-1.5 rounded-full bg-you collapsed:block"
      />
    </>
  );
}

const navRow =
  "group relative flex h-8 items-center gap-3 rounded-md px-2.5 text-sm font-medium text-muted-foreground transition-colors duration-120 ease-out-soft hover:bg-muted hover:text-foreground motion-reduce:transition-none collapsed:w-9 collapsed:justify-center collapsed:px-0";
const navOn = "bg-accent text-foreground hover:bg-accent";

function NavLink({
  item,
  active,
  count,
}: {
  item: NavItem;
  active: boolean;
  count?: number;
}) {
  const [collapsed] = useSidebarCollapsed();
  const Icon = NAV_ICON[item.icon];
  const words = shortcutWords(item);
  return (
    <RailTip
      label={item.label}
      hint={words ? item.hint + " · " + words : item.hint}
    >
      <Link
        href={item.href}
        aria-current={active ? "page" : undefined}
        title={
          collapsed
            ? undefined
            : words
              ? item.hint + " (" + words + ")"
              : item.hint
        }
        className={cn(navRow, focusRing, active && navOn)}
      >
        {active ? (
          <span aria-hidden className="brand-bar absolute inset-y-1.5 left-0" />
        ) : null}
        <Icon
          aria-hidden
          className={cn(
            "size-[18px] shrink-0",
            active
              ? "text-link"
              : "text-muted-foreground group-hover:text-foreground",
          )}
          strokeWidth={1.75}
        />
        <span className="min-w-0 flex-1 truncate collapsed:sr-only">
          {item.label}
        </span>
        {count ? <Waiting n={count} /> : null}
      </Link>
    </RailTip>
  );
}

// The groups in the spec's order - no label, Knowledge, Projects (rendered
// live from the org, passed in as `projects`), System.
function NavGroups({
  pathname,
  projects,
}: {
  pathname: string;
  projects: React.ReactNode;
}) {
  // Sessions counts the asks only the owner may answer (the hard-stop strip on it).
  const total = useHardStops().rows.length;
  return (
    <>
      {NAV_GROUPS.map((g, gi) => (
        <Fragment key={g.label ?? "main"}>
          {g.label === "System" ? projects : null}
          <nav aria-label={g.label ?? "Your work"}>
            {g.label ? (
              <p className="mb-1 px-2.5 text-xs font-medium text-muted-foreground collapsed:sr-only">
                {g.label}
              </p>
            ) : null}
            {gi > 0 ? (
              <div
                aria-hidden
                className="mx-2 mb-2 hidden border-t collapsed:block"
              />
            ) : null}
            <ul className="grid gap-0.5">
              {g.items.map((n) => (
                <li key={n.href}>
                  <NavLink
                    item={n}
                    active={n.match(pathname)}
                    count={n.icon === "sessions" ? total : undefined}
                  />
                </li>
              ))}
            </ul>
          </nav>
        </Fragment>
      ))}
    </>
  );
}

// ----- projects -----------------------------------------------------------

function ProjectRow({
  slug,
  name,
  current,
  waiting,
}: {
  slug: string;
  name: string;
  current: boolean;
  waiting: number;
}) {
  const [task, setTask] = useState(false);
  return (
    <li>
      <div
        className={cn(
          "group relative flex h-8 items-center rounded-md text-sm text-muted-foreground transition-colors hover:bg-muted hover:text-foreground motion-reduce:transition-none collapsed:w-9 collapsed:justify-center",
          current && navOn,
        )}
      >
        {current ? (
          <span aria-hidden className="brand-bar absolute inset-y-1.5 left-0" />
        ) : null}
        <RailTip
          label={name}
          hint={waiting ? waiting + " waiting for you" : undefined}
        >
          <Link
            href={hrefs.project(slug)}
            aria-current={current ? "page" : undefined}
            title={name !== slug ? slug : undefined}
            className={cn(
              "flex h-8 min-w-0 flex-1 items-center gap-2.5 rounded-md px-2.5 collapsed:justify-center collapsed:px-0",
              focusRing,
            )}
          >
            <ProjectAvatar slug={slug} name={name} size={20} />
            <span className="min-w-0 flex-1 truncate collapsed:sr-only">
              {name}
            </span>
            {waiting ? <Waiting n={waiting} /> : null}
          </Link>
        </RailTip>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon-xs"
              className="mr-1 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 data-[state=open]:opacity-100 collapsed:hidden"
              aria-label={"More for " + name}
            >
              <MoreHorizontalIcon aria-hidden className="size-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="w-48 p-1.5">
            <DropdownMenuItem onSelect={() => setTask(true)}>
              New task
            </DropdownMenuItem>
            <DropdownMenuItem asChild>
              <Link href={hrefs.project(slug, "board")}>Open board</Link>
            </DropdownMenuItem>
            <DropdownMenuItem asChild>
              <Link href={hrefs.project(slug, "team")}>Project settings</Link>
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      <NewTaskDialog open={task} onOpenChange={setTask} project={slug} />
    </li>
  );
}

// The projects on this machine, each with its share of what waits for you.
function SidebarProjects({ onAdd }: { onAdd: () => void }) {
  const params = useSearchParams();
  const pathname = usePathname();
  const current = pathname.startsWith("/project")
    ? params.get("slug") || ""
    : "";
  const { data, error } = useApi<Org>("org");
  const inbox = useInbox();
  const projects = data?.projects ?? [];
  const commons = data?.commons;
  return (
    <nav aria-label="Projects">
      <div className="mb-1 flex items-center justify-between pr-1 pl-2.5 collapsed:sr-only">
        <p className="text-xs font-medium text-muted-foreground">Projects</p>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon-xs"
              className="text-muted-foreground hover:text-foreground"
              aria-label="Add a project"
              onClick={onAdd}
            >
              <PlusIcon aria-hidden className="size-3.5" />
            </Button>
          </TooltipTrigger>
          <TooltipContent side="right">Add a project</TooltipContent>
        </Tooltip>
      </div>
      <div aria-hidden className="mx-2 mb-2 hidden border-t collapsed:block" />
      {error && !data ? (
        <div className="px-2.5 py-1.5 text-xs text-muted-foreground collapsed:hidden">
          Projects didn&apos;t load.
        </div>
      ) : null}
      {data && !projects.length ? (
        <div className="px-2.5 py-1.5 text-xs text-muted-foreground collapsed:hidden">
          No projects yet.{" "}
          <button
            type="button"
            className="font-medium text-link underline-offset-2 hover:underline"
            onClick={onAdd}
          >
            Add one
          </button>
        </div>
      ) : null}
      <ul className="grid gap-0.5">
        {projects.map((p) => (
          <ProjectRow
            key={p.slug}
            slug={p.slug}
            name={projectLabel(p)}
            current={current === p.slug}
            waiting={inbox.forProject(p.slug).total}
          />
        ))}
        {commons ? (
          <li>
            <RailTip label="Shared board" hint={COMMONS_HINT}>
              <Link
                href={hrefs.project(commons.slug)}
                aria-current={current === commons.slug ? "page" : undefined}
                title={COMMONS_HINT}
                className={cn(
                  "group relative flex h-8 items-center gap-2.5 rounded-md px-2.5 text-sm text-muted-foreground transition-colors hover:bg-muted hover:text-foreground motion-reduce:transition-none collapsed:w-9 collapsed:justify-center collapsed:px-0",
                  focusRing,
                  current === commons.slug && navOn,
                )}
              >
                {current === commons.slug ? (
                  <span
                    aria-hidden
                    className="brand-bar absolute inset-y-1.5 left-0"
                  />
                ) : null}
                <span
                  aria-hidden
                  className="inline-flex size-5 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground"
                >
                  <UsersIcon className="size-3" />
                </span>
                <span className="min-w-0 flex-1 truncate collapsed:sr-only">
                  Shared board
                </span>
                {commons.questions_open ? (
                  <Waiting n={commons.questions_open} />
                ) : null}
              </Link>
            </RailTip>
          </li>
        ) : null}
      </ul>
    </nav>
  );
}

// ----- title ----------------------------------------------------------------

// Each page names itself in the browser tab ("Tasks · Hive", "demo-app · Hive")
// from its crumbs, so tabs, history and screen readers can tell pages apart.
function DocumentTitle() {
  const crumbs = useCurrentCrumbs();
  const last = crumbs[crumbs.length - 1];
  const name = last && last.href !== "/" ? last.label : "";
  useEffect(() => {
    document.title = name ? name + " · Hive" : "Hive";
  }, [name]);
  return null;
}

// ----- shell ----------------------------------------------------------------

function Sidebar({ pathname }: { pathname: string }) {
  const [adding, setAdding] = useState(false);
  const settingsActive = pathname.startsWith("/settings");
  const onAdd = useCallback(() => setAdding(true), []);
  const slug = useSearchParams().get("slug") || "";
  const navBox = useRef<HTMLDivElement>(null);
  // On a short window the list scrolls; keep the page you are on in sight,
  // so Automations or Insights don't open with their own item below the
  // fold. The projects arrive after the first paint and push System down,
  // so it looks again as the list fills in - for a few seconds, and never
  // once the owner has scrolled it. The list scrolls itself, never the page.
  useEffect(() => {
    const box = navBox.current;
    if (!box) return;
    let touched = false;
    const reveal = () => {
      const on = box.querySelector<HTMLElement>('[aria-current="page"]');
      if (touched || !on) return;
      const b = box.getBoundingClientRect();
      const r = on.getBoundingClientRect();
      if (r.top < b.top) box.scrollTop -= b.top - r.top + 12;
      else if (r.bottom > b.bottom) box.scrollTop += r.bottom - b.bottom + 12;
    };
    const touch = () => {
      touched = true;
    };
    reveal();
    const mo = new MutationObserver(reveal);
    mo.observe(box, { childList: true, subtree: true });
    const stop = window.setTimeout(() => mo.disconnect(), 4000);
    box.addEventListener("wheel", touch, { passive: true });
    box.addEventListener("touchmove", touch, { passive: true });
    return () => {
      mo.disconnect();
      window.clearTimeout(stop);
      box.removeEventListener("wheel", touch);
      box.removeEventListener("touchmove", touch);
    };
  }, [pathname, slug]);
  return (
    <aside
      id="sidebar"
      aria-label="Sidebar"
      className="fixed inset-y-0 left-0 z-40 hidden w-[264px] flex-col border-r border-sidebar-border bg-sidebar pt-[env(safe-area-inset-top)] transition-[width] duration-200 ease-out-soft motion-reduce:transition-none lg:flex collapsed:w-16"
    >
      <SidebarHeader />
      <div className="grid gap-2 px-3 pb-3 collapsed:justify-items-center collapsed:px-2">
        <Suspense
          fallback={
            <div className="h-9 rounded-md bg-primary/80 collapsed:w-9" />
          }
        >
          <NewMenu onAddProject={onAdd} />
        </Suspense>
        <SearchRow />
      </div>
      <div ref={navBox} className="scroll-thin scroll-shade-y flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-3 pt-1 pb-4 collapsed:items-center collapsed:gap-3 collapsed:px-2">
        <NavGroups
          pathname={pathname}
          projects={
            <Suspense fallback={null}>
              <SidebarProjects onAdd={onAdd} />
            </Suspense>
          }
        />
      </div>
      <div className="grid shrink-0 gap-2 border-t border-sidebar-border px-3 pt-3 pb-2.5 collapsed:justify-items-center collapsed:px-2">
        <SystemStatus />
        <OwnerRow active={settingsActive} />
      </div>
      <AddProjectDialog open={adding} onOpenChange={setAdding} />
    </aside>
  );
}

export function AppShell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  const [help, setHelp] = useState(false);
  const onSearch = useCallback(() => setPaletteOpen(true), []);
  const onAsk = useCallback(() => openAsk(), []);
  const onHelp = useCallback(() => setHelp(true), []);
  useGlobalShortcuts({ onSearch, onAsk, onHelp });
  const chat = pathname === "/chat" || pathname.startsWith("/chat/");
  return (
    <CrumbsProvider>
      <div
        className="flex min-h-dvh min-w-0 flex-1 flex-col pl-[env(safe-area-inset-left)] pr-[env(safe-area-inset-right)] lg:pr-0 lg:pl-[264px] lg:transition-[padding-left] lg:duration-200 lg:ease-out-soft motion-reduce:transition-none collapsed:lg:pl-16"
        style={{ "--topbar-h": TOPBAR_H } as React.CSSProperties}
      >
        <Suspense fallback={null}>
          <CommandPaletteHost />
          <AskHost />
          <DocumentTitle />
          <AwakeTheme />
          <OwnerNotifier />
        </Suspense>
        <a
          href="#main"
          className="sr-only focus:not-sr-only focus:fixed focus:top-3 focus:left-3 focus:z-[60] focus:rounded-md focus:bg-primary focus:px-4 focus:py-3 focus:text-primary-foreground focus:shadow-pop"
        >
          Skip to content
        </a>

        <Suspense fallback={null}>
          <Sidebar pathname={pathname} />
        </Suspense>

        <Suspense fallback={<div className="hidden h-(--topbar-h) lg:block" />}>
          <TopBar onHelp={onHelp} />
          <PhoneBar />
        </Suspense>

        {/* Chat reads like an app, not a page: a transcript between two side columns wants the room (tighter margins, a wider cap), and it fits the window. */}
        <main
          id="main"
          tabIndex={-1}
          className={cn(
            "mx-auto w-full min-w-0 flex-1 px-4 pb-[calc(6rem+env(safe-area-inset-bottom))] outline-none sm:px-6 lg:px-10 lg:pb-14",
            chat ? "max-w-[1680px] lg:px-6 lg:pb-4" : "max-w-[1240px]",
          )}
        >
          {/* A newer Hive release, when the daily check found one (update.latest_url). Chat fits the window, so it goes without. */}
          {chat ? null : <UpdateBanner />}
          {children}
        </main>

        <BottomBar />
        <ShortcutHelp open={help} onOpenChange={setHelp} />
      </div>
    </CrumbsProvider>
  );
}
