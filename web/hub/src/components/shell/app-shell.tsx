"use client";

import Link from "next/link";
import { usePathname, useSearchParams } from "next/navigation";
import { Suspense, useEffect } from "react";
import {
  BookOpenIcon,
  BrainIcon,
  CalendarClockIcon,
  ChevronDownIcon,
  FolderIcon,
  HomeIcon,
  ListTodoIcon,
  MessageSquareIcon,
  MoreHorizontalIcon,
  PanelLeftCloseIcon,
  PanelLeftOpenIcon,
  PlugIcon,
  ScrollTextIcon,
  ServerIcon,
  SettingsIcon,
  UsersIcon,
  VoteIcon,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { hrefs } from "@/lib/nav";
import { useApi, useEventStream } from "@/lib/use-api";
import { COMMONS_HINT, projectLabel, type Org } from "@/lib/api";
import { useInbox } from "@/components/pulse/inbox";
import { ProjectMascot } from "@/components/pulse/mascots";
import { AskHost, AskTrigger } from "@/components/ask/ask-panel";
import { CommandPalette, CommandPaletteHost } from "./command-palette";
import { PulseMark } from "./pulse-mark";
import { ThemeToggle } from "./theme-toggle";
import { AwakeStatus, AwakeTheme } from "./awake";
import { useSidebarCollapsed } from "./use-sidebar";
import { sidebarToggleLabel } from "./sidebar-state";
import { RailTip } from "./rail-tip";
import { OwnerNotifier } from "./owner-notifier";

type NavItem = {
  href: string;
  label: string;
  hint: string;
  icon: React.ComponentType<{ className?: string }>;
  match: (p: string) => boolean;
};

// Two groups: the everyday work, and the machinery behind it. The labels are
// what each page is about, not what the store calls it (Runs is the harness's
// log, Machines is compute).
const WORK: NavItem[] = [
  {
    href: "/",
    label: "Home",
    hint: "What waits for you, and how your projects are doing",
    icon: HomeIcon,
    match: (p) => p === "/",
  },
  {
    href: "/chat/",
    label: "Sessions",
    hint: "Ask an agent to work, watch it, answer its questions",
    icon: MessageSquareIcon,
    match: (p) => p.startsWith("/chat"),
  },
  {
    href: "/tasks/",
    label: "Tasks",
    hint: "The work you gave your agents",
    icon: ListTodoIcon,
    match: (p) => p.startsWith("/tasks"),
  },
  {
    href: "/votes/",
    label: "Votes",
    hint: "Ideas your agents are voting on",
    icon: VoteIcon,
    match: (p) => p.startsWith("/votes"),
  },
  {
    href: "/records/",
    label: "Records",
    hint: "Decisions, plans, guides and notes",
    icon: BookOpenIcon,
    match: (p) => p.startsWith("/records") || p.startsWith("/code-deliveries"),
  },
];
const SYSTEM: NavItem[] = [
  {
    href: "/harness/",
    label: "Runs",
    hint: "Every agent run, step by step",
    icon: ScrollTextIcon,
    match: (p) => p.startsWith("/harness"),
  },
  {
    href: "/compute/",
    label: "Machines",
    hint: "Where agents run, and requests for more",
    icon: ServerIcon,
    match: (p) => p.startsWith("/compute"),
  },
  {
    href: "/connections/",
    label: "Connections",
    hint: "Tools and skills agents can use",
    icon: PlugIcon,
    match: (p) => p.startsWith("/connections"),
  },
  {
    href: "/automations/",
    label: "Automations",
    hint: "Standing jobs on a schedule, kept in each project's PULSE file",
    icon: CalendarClockIcon,
    match: (p) => p.startsWith("/automations"),
  },
  {
    href: "/memory/",
    label: "Memory",
    hint: "What agents remembered across runs, file by file",
    icon: BrainIcon,
    match: (p) => p.startsWith("/memory"),
  },
  {
    href: "/settings/",
    label: "Settings",
    hint: "How PULSE behaves on this machine",
    icon: SettingsIcon,
    match: (p) => p.startsWith("/settings"),
  },
];

const focusRing =
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background";

// The desktop sidebar folds to a rail of icons (the `collapsed:` variant in
// globals.css; the choice lives in use-sidebar.ts). On the rail a count sits
// in the item's corner instead of at its end, and a row's name moves off
// screen for sighted users while screen readers still get it.
const railCount =
  "collapsed:absolute collapsed:top-0.5 collapsed:right-0.5 collapsed:h-4 collapsed:min-w-4 collapsed:px-1 collapsed:text-[10px]";
const railName = "collapsed:sr-only";

// The one button that folds the sidebar to its rail and back. It sits at the
// end of the brand row; on the rail it drops under the mark.
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

// The number of things waiting for the owner, in the one warm colour. With
// `attn` a soft marigold ring breathes around it (the Home item only, so the
// sidebar does not blink all over).
function YouCount({
  n,
  className,
  attn,
}: {
  n: number;
  className?: string;
  attn?: boolean;
}) {
  if (!n) return null;
  return (
    <span
      key={n}
      className={cn(
        "tnum pop inline-flex h-5 min-w-5 items-center justify-center rounded-full bg-you px-1.5 text-[11.5px] font-semibold text-[oklch(0.3_0.06_60)]",
        attn && "attn",
        className,
      )}
    >
      {n}
    </span>
  );
}

// Each project is a little folder with a face, in the project's colour.
function ProjectDot({ slug, className }: { slug: string; className?: string }) {
  return <ProjectMascot slug={slug} size={20} className={className} />;
}

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
  return (
    <RailTip label={item.label} hint={item.hint}>
      <Link
        href={item.href}
        aria-current={active ? "page" : undefined}
        title={collapsed ? undefined : item.hint}
        className={cn(
          "group nav-item relative flex h-9 items-center gap-3 rounded-lg px-2.5 text-sm font-medium text-muted-foreground transition-colors hover:bg-muted hover:text-foreground motion-reduce:transition-none collapsed:justify-center collapsed:px-0",
          focusRing,
          active &&
            "bg-card text-foreground shadow-[0_1px_2px_oklch(0.26_0.04_262/.06)] ring-1 ring-border hover:bg-card",
        )}
      >
        <item.icon
          aria-hidden
          className={cn(
            "nav-icon size-[18px] shrink-0",
            active
              ? "text-primary"
              : "text-muted-foreground group-hover:text-foreground",
          )}
        />
        <span className={cn("min-w-0 flex-1 truncate", railName)}>
          {item.label}
        </span>
        {count ? <YouCount n={count} attn className={railCount} /> : null}
      </Link>
    </RailTip>
  );
}

// Whether the hub's live event stream is up: a steady teal dot, or amber while it reconnects.
function LiveStatus() {
  const { state } = useEventStream();
  const live = state === "live";
  const label = live
    ? "Live: changes show up as they happen"
    : state === "polling"
      ? "Reconnecting: refreshing every 10 seconds until the live stream is back"
      : "Connecting to PULSE…";
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          tabIndex={0}
          role="status"
          aria-live="polite"
          aria-atomic="true"
          aria-label={label}
          className={cn(
            "inline-flex h-8 items-center gap-2 rounded-lg px-2 text-[13px] text-muted-foreground outline-none collapsed:size-8 collapsed:justify-center collapsed:px-0",
            focusRing,
          )}
        >
          <span
            aria-hidden
            data-live={live}
            className={cn(
              "live-dot size-2 shrink-0 rounded-full",
              live
                ? "bg-ok"
                : state === "polling"
                  ? "bg-warn"
                  : "bg-muted-foreground/50",
            )}
          />
          <span className={railName}>
            {live
              ? "Live"
              : state === "polling"
                ? "Reconnecting"
                : "Connecting"}
          </span>
        </span>
      </TooltipTrigger>
      <TooltipContent side="top">{label}</TooltipContent>
    </Tooltip>
  );
}

// The projects on this machine, in the sidebar, each with its share of what waits for you.
function SidebarProjects() {
  const params = useSearchParams();
  const pathname = usePathname();
  const current = pathname.startsWith("/project")
    ? params.get("slug") || ""
    : "";
  const { data, error } = useApi<Org>("org");
  const inbox = useInbox();
  const [collapsed] = useSidebarCollapsed();
  const projects = data?.projects ?? [];
  const commons = data?.commons;
  const row =
    "group relative flex h-8 items-center gap-2.5 rounded-lg px-2.5 text-sm text-muted-foreground transition-colors hover:bg-muted hover:text-foreground motion-reduce:transition-none collapsed:justify-center collapsed:px-0";
  const on = "bg-card text-foreground ring-1 ring-border hover:bg-card";
  return (
    <div>
      <p className="mb-1 px-2.5 text-xs font-medium text-muted-foreground collapsed:sr-only">
        Projects
      </p>
      {/* The rail has no room for words: a rule stands in for the heading. */}
      <div
        aria-hidden
        className="mx-2 mb-2 hidden border-t collapsed:block"
      />
      {error && !data ? (
        <p className="px-2.5 py-1 text-xs text-muted-foreground collapsed:sr-only">
          Projects didn&apos;t load.
        </p>
      ) : null}
      {data && !projects.length ? (
        <p className="px-2.5 py-1 text-xs leading-relaxed text-muted-foreground collapsed:sr-only">
          No projects yet. Add one from Home.
        </p>
      ) : null}
      <ul className="grid gap-0.5">
        {projects.map((p) => {
          const forYou = inbox.forProject(p.slug).total;
          const active = current === p.slug;
          const label = projectLabel(p);
          const missing = p.present ? "" : label + " is missing on disk";
          return (
            <li key={p.slug}>
              <RailTip
                label={label}
                hint={missing || (label !== p.slug ? p.slug : undefined)}
              >
                <Link
                  href={hrefs.project(p.slug)}
                  aria-current={active ? "page" : undefined}
                  className={cn(row, focusRing, active && on)}
                  title={collapsed ? undefined : missing || (label !== p.slug ? label + " (" + p.slug + ")" : p.slug)}
                >
                  <ProjectDot slug={p.slug} />
                  <span className={cn("min-w-0 flex-1 truncate", railName)}>
                    {label}
                  </span>
                  {!p.present ? (
                    <span
                      className="size-2 shrink-0 rounded-full bg-bad collapsed:absolute collapsed:top-1 collapsed:right-1"
                      aria-label="missing on disk"
                      role="img"
                    />
                  ) : null}
                  <YouCount n={forYou} className={railCount} />
                </Link>
              </RailTip>
            </li>
          );
        })}
        {commons ? (
          <li>
            <RailTip label="Shared board" hint={COMMONS_HINT}>
              <Link
                href={hrefs.project(commons.slug)}
                aria-current={current === commons.slug ? "page" : undefined}
                title={collapsed ? undefined : COMMONS_HINT}
                className={cn(row, focusRing, current === commons.slug && on)}
              >
                <span
                  aria-hidden
                  className="inline-flex size-5 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground"
                >
                  <UsersIcon className="size-3" />
                </span>
                <span className={cn("min-w-0 flex-1 truncate", railName)}>
                  Shared board
                </span>
                {commons.questions_open ? (
                  <span
                    className={cn(
                      "tnum text-xs text-muted-foreground",
                      railName,
                    )}
                  >
                    {commons.questions_open}
                  </span>
                ) : null}
              </Link>
            </RailTip>
          </li>
        ) : null}
      </ul>
    </div>
  );
}

// Each page names itself in the browser tab ("Tasks · PULSE", "demo-app · PULSE"),
// so tabs, history and screen readers can tell pages apart.
function DocumentTitle() {
  const pathname = usePathname();
  const params = useSearchParams();
  const { data } = useApi<Org>("org");
  const slug = pathname.startsWith("/project") ? params.get("slug") || "" : "";
  const page = [...WORK, ...SYSTEM].find((n) => n.match(pathname));
  const name = slug
    ? slug === data?.commons?.slug
      ? "Shared board"
      : projectLabel(data?.projects.find((p) => p.slug === slug) ?? { slug })
    : pathname.startsWith("/code-deliveries")
      ? "Code delivered"
      : page && page.href !== "/"
        ? page.label
        : "";
  useEffect(() => {
    document.title = name ? name + " · PULSE" : "PULSE";
  }, [name]);
  return null;
}

function HomeLink({ active }: { active: boolean }) {
  const { total } = useInbox();
  return <NavLink item={WORK[0]} active={active} count={total} />;
}

// On phones and tablets the projects live in a menu at the top.
function ProjectMenu() {
  const params = useSearchParams();
  const pathname = usePathname();
  const current = pathname.startsWith("/project")
    ? params.get("slug") || ""
    : "";
  const { data, error, loading } = useApi<Org>("org");
  const inbox = useInbox();
  const projects = data?.projects ?? [];
  const commons = data?.commons;
  const currentLabel =
    current && current === commons?.slug
      ? "Shared board"
      : current
        ? projectLabel(projects.find((p) => p.slug === current) ?? { slug: current })
        : "Projects";
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          className="h-10 min-w-0 max-w-full justify-start gap-2 px-2"
          aria-label={
            current
              ? `Switch project, now on ${currentLabel}`
              : "Go to a project"
          }
        >
          {current ? (
            <ProjectDot slug={current} />
          ) : (
            <FolderIcon aria-hidden className="size-4 text-muted-foreground" />
          )}
          <span className="min-w-0 truncate text-[15px] font-semibold">
            {currentLabel}
          </span>
          <ChevronDownIcon
            aria-hidden
            className="size-4 shrink-0 text-muted-foreground"
          />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        sideOffset={6}
        collisionPadding={12}
        className="w-72 max-w-[calc(100vw-1.5rem)] rounded-xl p-1.5 shadow-pop"
      >
        <DropdownMenuLabel className="px-2.5 py-1.5 text-xs font-medium text-muted-foreground">
          Projects on this machine
        </DropdownMenuLabel>
        {error ? (
          <div role="status" className="px-2.5 py-1.5 text-xs text-bad">
            Projects didn&apos;t load: {error}
          </div>
        ) : null}
        {loading && !data ? (
          <div
            role="status"
            className="px-2.5 py-1.5 text-xs text-muted-foreground"
          >
            Loading projects…
          </div>
        ) : null}
        {data && !projects.length ? (
          <div className="px-2.5 py-1.5 text-xs text-muted-foreground">
            No projects yet. Add one from Home.
          </div>
        ) : null}
        {projects.map((p) => (
          <DropdownMenuItem
            key={p.slug}
            asChild
            className={cn(
              "min-h-10 gap-2.5 rounded-lg px-2.5",
              current === p.slug && "bg-accent",
            )}
          >
            <Link
              href={hrefs.project(p.slug)}
              aria-current={current === p.slug ? "page" : undefined}
              title={projectLabel(p) !== p.slug ? p.slug : undefined}
            >
              <ProjectDot slug={p.slug} />
              <span className="min-w-0 flex-1 truncate">{projectLabel(p)}</span>
              {!p.present ? (
                <span className="text-xs text-bad">missing</span>
              ) : null}
              <YouCount n={inbox.forProject(p.slug).total} />
            </Link>
          </DropdownMenuItem>
        ))}
        {commons ? (
          <DropdownMenuItem
            asChild
            className={cn(
              "min-h-10 gap-2.5 rounded-lg px-2.5",
              current === commons.slug && "bg-accent",
            )}
          >
            <Link href={hrefs.project(commons.slug)} title={COMMONS_HINT}>
              <span
                aria-hidden
                className="inline-flex size-5 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground"
              >
                <UsersIcon className="size-3" />
              </span>
              <span className="min-w-0 flex-1 truncate">Shared board</span>
            </Link>
          </DropdownMenuItem>
        ) : null}
        <DropdownMenuSeparator className="my-1.5" />
        <DropdownMenuItem
          asChild
          className="min-h-10 gap-2.5 rounded-lg px-2.5"
        >
          <Link href="/">
            <HomeIcon aria-hidden className="size-4 text-muted-foreground" />
            <span>All projects</span>
          </Link>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function MobileHomeTab({ active }: { active: boolean }) {
  const { total } = useInbox();
  return (
    <Link
      href="/"
      aria-current={active ? "page" : undefined}
      aria-label={total ? `Home, ${total} waiting for you` : "Home"}
      className={cn(
        "nav-item relative flex min-w-0 flex-col items-center justify-center gap-0.5 rounded-lg text-[11px] font-medium text-muted-foreground",
        focusRing,
        active && "text-foreground",
      )}
    >
      <span
        className={cn(
          "relative inline-flex h-7 w-12 items-center justify-center rounded-full",
          active && "bg-primary/12 text-primary",
        )}
      >
        <HomeIcon aria-hidden className="nav-icon size-[18px]" />
        {total ? (
          <span
            aria-hidden
            className="absolute -top-0.5 right-1.5 size-2.5 rounded-full bg-you ring-2 ring-card"
          />
        ) : null}
      </span>
      Home
    </Link>
  );
}

export function AppShell({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  const activeSystem = SYSTEM.find((n) => n.match(pathname));
  return (
    <div className="flex min-h-dvh min-w-0 flex-1 flex-col pl-[env(safe-area-inset-left)] pr-[env(safe-area-inset-right)] lg:pr-0 lg:pl-64 lg:transition-[padding-left] lg:duration-200 lg:ease-out-soft motion-reduce:transition-none collapsed:lg:pl-16">
      <Suspense fallback={null}>
        <CommandPaletteHost />
        <AskHost />
        <DocumentTitle />
        <AwakeTheme />
        <OwnerNotifier />
      </Suspense>
      <a
        href="#main"
        className="sr-only focus:not-sr-only focus:fixed focus:top-3 focus:left-3 focus:z-[60] focus:rounded-lg focus:bg-primary focus:px-4 focus:py-3 focus:text-primary-foreground focus:shadow-pop"
      >
        Skip to content
      </a>

      <aside
        id="sidebar"
        aria-label="Sidebar"
        className="fixed inset-y-0 left-0 z-40 hidden w-64 flex-col border-r bg-sidebar pt-[env(safe-area-inset-top)] transition-[width] duration-200 ease-out-soft motion-reduce:transition-none lg:flex collapsed:w-16"
      >
        {/* The brand row: the mark, the name and the fold button. On the rail the name goes and the button drops under the mark. */}
        <div className="mx-3 mt-4 mb-3 flex shrink-0 items-center gap-1 collapsed:mx-2 collapsed:flex-col collapsed:items-stretch collapsed:gap-2">
          <Link
            href="/"
            className={cn(
              "brand-link flex h-10 min-w-0 flex-1 items-center gap-2.5 rounded-lg px-1.5 collapsed:justify-center collapsed:px-0",
              focusRing,
            )}
            aria-label="PULSE home"
          >
            <PulseMark />
            <span
              className={cn(
                "text-[17px] font-semibold tracking-tight",
                railName,
              )}
            >
              PULSE
            </span>
          </Link>
          <div className="flex shrink-0 justify-center">
            <SidebarToggle />
          </div>
        </div>
        <div className="grid gap-2 px-3 pb-3 collapsed:px-2">
          <Suspense
            fallback={<div className="h-9 rounded-lg border bg-card" />}
          >
            <RailTip label="Search" hint="⌘K">
              <CommandPalette variant="sidebar" />
            </RailTip>
          </Suspense>
          <RailTip label="Ask" hint="⌘J">
            <AskTrigger variant="sidebar" />
          </RailTip>
        </div>
        <div className="scroll-thin flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto px-3 pt-1 pb-4 collapsed:gap-4 collapsed:px-2">
          <nav aria-label="Your work">
            <ul className="grid gap-0.5">
              <li>
                <HomeLink active={WORK[0].match(pathname)} />
              </li>
              {WORK.slice(1).map((n) => (
                <li key={n.href}>
                  <NavLink item={n} active={n.match(pathname)} />
                </li>
              ))}
            </ul>
          </nav>
          <Suspense fallback={null}>
            <SidebarProjects />
          </Suspense>
          <nav aria-label="Behind the scenes">
            <p className="mb-1 px-2.5 text-xs font-medium text-muted-foreground collapsed:sr-only">
              Behind the scenes
            </p>
            <div
              aria-hidden
              className="mx-2 mb-2 hidden border-t collapsed:block"
            />
            <ul className="grid gap-0.5">
              {SYSTEM.map((n) => (
                <li key={n.href}>
                  <NavLink item={n} active={n.match(pathname)} />
                </li>
              ))}
            </ul>
          </nav>
        </div>
        <div className="flex shrink-0 items-center justify-between gap-2 border-t px-3 py-2.5 collapsed:flex-col collapsed:items-center collapsed:gap-1 collapsed:px-2">
          <LiveStatus />
          <div className="flex items-center gap-1 collapsed:flex-col">
            <AwakeStatus />
            <ThemeToggle />
          </div>
        </div>
      </aside>

      <header className="sticky top-0 z-40 shrink-0 border-b bg-background/95 pt-[env(safe-area-inset-top)] backdrop-blur-sm supports-[backdrop-filter]:bg-background/85 lg:hidden">
        <div className="mx-auto flex h-14 max-w-3xl items-center gap-1 px-2 sm:px-4">
          <Link
            href="/"
            className={cn(
              "brand-link flex size-10 shrink-0 items-center justify-center rounded-lg",
              focusRing,
            )}
            aria-label="PULSE home"
          >
            <PulseMark className="size-7" />
          </Link>
          <div className="min-w-0 flex-1">
            <Suspense fallback={<div className="h-10" />}>
              <ProjectMenu />
            </Suspense>
          </div>
          <Suspense fallback={<div className="size-10" />}>
            <CommandPalette variant="icon" />
          </Suspense>
          <AskTrigger variant="icon" />
          <AwakeStatus compact />
          <ThemeToggle />
        </div>
      </header>

      {/* Chat reads like an app, not a page: a transcript between two side columns wants the room (tighter margins, a wider cap), and it fits the window (the page sizes itself to it). */}
      <main
        id="main"
        tabIndex={-1}
        className={cn(
          "mx-auto w-full min-w-0 flex-1 px-4 pb-[calc(6rem+env(safe-area-inset-bottom))] outline-none sm:px-6 lg:px-10 lg:pb-14",
          pathname === "/chat" || pathname.startsWith("/chat/")
            ? "max-w-[1680px] lg:px-6 lg:pb-4"
            : "max-w-[1240px]",
        )}
      >
        {children}
      </main>

      <nav
        aria-label="Main (compact)"
        className="fixed inset-x-0 bottom-0 z-40 border-t bg-card/95 backdrop-blur-sm supports-[backdrop-filter]:bg-card/90 lg:hidden"
        style={{
          paddingBottom: "env(safe-area-inset-bottom)",
          paddingLeft: "env(safe-area-inset-left)",
          paddingRight: "env(safe-area-inset-right)",
        }}
      >
        <div className="mx-auto grid h-16 max-w-xl grid-cols-5 gap-1 px-2">
          <MobileHomeTab active={pathname === "/"} />
          {WORK.slice(1).map((n) => {
            const active = n.match(pathname);
            return (
              <Link
                key={n.href}
                href={n.href}
                aria-current={active ? "page" : undefined}
                className={cn(
                  "nav-item flex min-w-0 flex-col items-center justify-center gap-0.5 rounded-lg text-[11px] font-medium text-muted-foreground",
                  focusRing,
                  active && "text-foreground",
                )}
              >
                <span
                  className={cn(
                    "inline-flex h-7 w-12 items-center justify-center rounded-full",
                    active && "bg-primary/12 text-primary",
                  )}
                >
                  <n.icon aria-hidden className="nav-icon size-[18px]" />
                </span>
                {n.label}
              </Link>
            );
          })}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <button
                type="button"
                aria-label={
                  activeSystem
                    ? `More pages, now on ${activeSystem.label}`
                    : "More pages"
                }
                className={cn(
                  "flex min-w-0 flex-col items-center justify-center gap-0.5 rounded-lg text-[11px] font-medium text-muted-foreground data-[state=open]:text-foreground",
                  focusRing,
                  activeSystem && "text-foreground",
                )}
              >
                <span
                  className={cn(
                    "inline-flex h-7 w-12 items-center justify-center rounded-full",
                    activeSystem && "bg-primary/12 text-primary",
                  )}
                >
                  {activeSystem ? (
                    <activeSystem.icon aria-hidden className="size-[18px]" />
                  ) : (
                    <MoreHorizontalIcon aria-hidden className="size-[18px]" />
                  )}
                </span>
                {activeSystem ? activeSystem.label : "More"}
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent
              aria-label="More pages"
              align="end"
              side="top"
              sideOffset={10}
              collisionPadding={12}
              className="w-72 max-w-[calc(100vw-1.5rem)] rounded-xl p-1.5 shadow-pop"
            >
              <DropdownMenuLabel className="px-2.5 py-1.5 text-xs font-medium text-muted-foreground">
                Behind the scenes
              </DropdownMenuLabel>
              {SYSTEM.map((n) => {
                const active = n.match(pathname);
                return (
                  <DropdownMenuItem
                    key={n.href}
                    asChild
                    className={cn(
                      "min-h-11 gap-3 rounded-lg px-2.5",
                      active && "bg-accent",
                    )}
                  >
                    <Link
                      href={n.href}
                      aria-current={active ? "page" : undefined}
                    >
                      <n.icon
                        aria-hidden
                        className="size-[18px] text-muted-foreground"
                      />
                      <span className="min-w-0">
                        <span className="block font-medium">{n.label}</span>
                        <span className="block truncate text-xs text-muted-foreground">
                          {n.hint}
                        </span>
                      </span>
                    </Link>
                  </DropdownMenuItem>
                );
              })}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </nav>
    </div>
  );
}
