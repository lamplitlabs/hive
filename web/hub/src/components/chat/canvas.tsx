"use client";

// The canvas: the column beside a conversation or a task where the agent's
// work opens by itself - the file it is editing, the terminal its commands
// run in, the page it fetched, its plan - and where you open the rest from
// the "+" menu: the branch's changes, artifacts, the project, the
// sub-conversations, links it mentioned. One tab strip, the way the Copilot
// app keeps a session's files and terminal beside the chat; a tab that needs
// room widens the column, and the × on a tab puts it away. The rules (which
// step opens what, what comes to the front) live in lib/canvas.ts.

import { useEffect, useId, useMemo, useRef, useState, type ReactNode } from "react";
import {
  AppWindowIcon,
  CircleCheckIcon,
  CircleIcon,
  CornerDownRightIcon,
  DownloadIcon,
  ExternalLinkIcon,
  FileDiffIcon,
  FileTextIcon,
  FolderGitIcon,
  GitCompareIcon,
  GlobeIcon,
  ImagesIcon,
  LinkIcon,
  ListChecksIcon,
  ListTodoIcon,
  LoaderCircleIcon,
  Maximize2Icon,
  Minimize2Icon,
  PanelRightIcon,
  PaperclipIcon,
  PlusIcon,
  RefreshCwIcon,
  RouteIcon,
  ScrollTextIcon,
  SearchIcon,
  TerminalIcon,
  XIcon,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useWidth } from "@/components/chat/split";
import { BrowserTab, type PickSink } from "@/components/chat/browser";
import {
  ArtifactBody,
  ArtifactList,
  ChangesBody,
  DiffTable,
  FileKindIcon,
  bytesWord,
  fileKind,
  type ChatDiff,
} from "@/components/chat/changes";
import {
  evidenceEmptyText,
  jobEvidenceLabel,
  keepSelection,
} from "@/components/chat/job-evidence-state";
import { exitWords, planItems } from "@/components/harness/harness";
import { EmptyState, Notice, SkeletonRows } from "@/components/hive/panel";
import { When } from "@/components/hive/who";
import {
  artifactHref,
  harnessFilePath,
  jobArtifactsPath,
  type ArtifactSource,
  type ChatArtifact,
  type ChatChanges,
  type HarnessFileResponse,
  type HarnessStep,
  type JobArtifactsResponse,
  type TaskJob,
} from "@/lib/api";
import {
  KIND_LABEL,
  WIDE_KINDS,
  addTabs,
  afterClose,
  autoOpen,
  changedNewLines,
  editBlocks,
  extractLinks,
  latestPlanStep,
  localPages,
  makeTab,
  pageText,
  stepPath,
  terminalEntries,
  touchedFiles,
  visitedPages,
  type CanvasKind,
  type CanvasTab,
  type TerminalEntry,
} from "@/lib/canvas";
import { useApi } from "@/lib/use-api";
import { num, plural, span } from "@/lib/format";
import { revealIn } from "@/lib/scroll";
import { cn } from "@/lib/utils";

// ----- the state --------------------------------------------------------------

export interface CanvasControl {
  tabs: CanvasTab[];
  active: string;
  /** The column is wide: a terminal, a file, a page or a diff is in front. */
  wide: boolean;
  /** Opens a tab (or brings it forward) because you asked. */
  open: (kind: CanvasKind, key?: string) => void;
  /** The same for a tab made elsewhere (a job's evidence, labelled by its step). */
  openTab: (tab: CanvasTab) => void;
  select: (id: string) => void;
  close: (id: string) => void;
  setWide: (wide: boolean) => void;
}

// How long a tab you picked stays in front while the agent opens others.
const PIN_MS = 30000;

// The tabs of one canvas. `initial` opens at the start; `ensure` are tabs
// that should be there whenever they can be (the Changes tab once the
// conversation has a branch) unless you closed them; `steps` are the
// current run's, and each new one may open a tab of its own (lib/canvas's
// autoOpen). A tab you picked stays in front for a while; after that the
// agent's newest file or command comes forward again. Opening a finished
// run adds its tabs without bringing one forward: you came to read, not
// to watch.
export function useCanvas({
  session,
  steps,
  live = false,
  initial,
  ensure = [],
}: {
  session: string;
  steps: HarnessStep[];
  live?: boolean;
  initial: CanvasTab[];
  ensure?: CanvasTab[];
}): CanvasControl {
  const [tabs, setTabs] = useState<CanvasTab[]>(initial);
  const [active, setActive] = useState<string>(initial[0]?.id ?? "");
  const [dismissed, setDismissed] = useState<string[]>([]);
  // A tab you picked stays in front: `pin` counts the picks so each one
  // starts the clock again.
  const [pin, setPin] = useState({ held: false, n: 0 });
  const [userWide, setUserWide] = useState<boolean | null>(null);
  const [seenSession, setSeenSession] = useState(session);
  const [handled, setHandled] = useState(0);

  // Derived during render, the way the transcript tracks its own state: a
  // new session starts the step cursor over; new steps open their tabs.
  if (session !== seenSession) {
    setSeenSession(session);
    setHandled(0);
  } else {
    const lastN = steps.reduce((m, s) => Math.max(m, s.n), 0);
    if (lastN > handled) {
      const { open, front } = autoOpen(steps, handled);
      setHandled(lastN);
      if (open.length) setTabs((cur) => addTabs(cur, open));
      if (front && (live || handled > 0) && !pin.held) setActive(front);
    }
  }
  const missing = ensure.filter(
    (t) => !tabs.some((o) => o.id === t.id) && !dismissed.includes(t.id),
  );
  if (missing.length) setTabs((cur) => addTabs(cur, missing));

  useEffect(() => {
    if (!pin.held) return;
    const t = setTimeout(
      () => setPin((p) => (p.n === pin.n ? { held: false, n: p.n } : p)),
      PIN_MS,
    );
    return () => clearTimeout(t);
  }, [pin]);
  const hold = () => setPin((p) => ({ held: true, n: p.n + 1 }));

  const activeTab = tabs.find((t) => t.id === active) ?? tabs[0];
  const wide = userWide ?? (!!activeTab && WIDE_KINDS.has(activeTab.kind));

  const openTab = (t: CanvasTab) => {
    setTabs((cur) => addTabs(cur, [t]));
    setActive(t.id);
    hold();
    setUserWide(null);
  };

  return {
    tabs,
    active: activeTab?.id ?? "",
    wide,
    open: (kind, key = "") => openTab(makeTab(kind, key)),
    openTab,
    select: (id) => {
      setActive(id);
      hold();
      setUserWide(null);
    },
    close: (id) => {
      const next = afterClose(tabs, activeTab?.id ?? "", id);
      setTabs(next.tabs);
      setActive(next.active);
      setDismissed((d) => (d.includes(id) ? d : [...d, id]));
      hold();
    },
    setWide: setUserWide,
  };
}

// ----- the component ----------------------------------------------------------

const KIND_ICON: Record<CanvasKind, typeof TerminalIcon> = {
  terminal: TerminalIcon,
  file: FileTextIcon,
  page: GlobeIcon,
  browser: GlobeIcon,
  plan: ListChecksIcon,
  changes: GitCompareIcon,
  diff: FileDiffIcon,
  artifacts: PaperclipIcon,
  artifact: PaperclipIcon,
  subs: CornerDownRightIcon,
  project: FolderGitIcon,
  links: LinkIcon,
  steps: ListTodoIcon,
  brief: ScrollTextIcon,
  lifecycle: RouteIcon,
  evidence: ImagesIcon,
};

// Brings the canvas into view: beside the transcript on a wide screen it is
// already there; on a phone it sits below, so a link that opens a tab on it
// (the report) scrolls down to it.
export function revealCanvas() {
  if (typeof window === "undefined") return;
  window.requestAnimationFrame(() =>
    document
      .querySelector("[data-canvas]")
      ?.scrollIntoView({ behavior: "smooth", block: "nearest" }),
  );
}

export function Canvas({
  ctl,
  label,
  session,
  steps,
  live,
  diff,
  changes,
  chatId,
  artifacts = [],
  jobs = [],
  texts = [],
  panels,
  menu,
  counts = {},
  picks,
  onAsk,
}: {
  ctl: CanvasControl;
  label: string;
  /** The run whose steps feed the terminal, the files and the pages. */
  session: string;
  steps: HarnessStep[];
  live: boolean;
  diff?: ChatDiff | null;
  changes?: ChatChanges | null;
  /** The conversation whose artifacts the artifact tabs show. */
  chatId?: string;
  artifacts?: ChatArtifact[];
  /** The task's steps, for the evidence tabs (a job's artifacts folder). */
  jobs?: TaskJob[];
  /** What the Links tab reads: messages and observations. */
  texts?: string[];
  /** Tab bodies the page supplies (changes, artifacts, project, subs, steps, brief). */
  panels: Partial<Record<CanvasKind, ReactNode>>;
  /** The one-of-a-kind tabs the "+" menu offers, in order. */
  menu: CanvasKind[];
  /** A number beside a menu entry (changed files, artifacts, sub-conversations). */
  counts?: Partial<Record<CanvasKind, number>>;
  /** The composer's picked elements, for the Browser tab's Pick; absent where there is no composer. */
  picks?: PickSink;
  /** Puts words into the composer (the Browser tab's ask for its helper). */
  onAsk?: (text: string) => void;
}) {
  const entries = useMemo(() => terminalEntries(steps, live), [steps, live]);
  const running = entries.some((e) => e.running);
  const files = useMemo(() => {
    const out = touchedFiles(steps);
    for (const f of diff?.files ?? [])
      if (f.path && !out.includes(f.path)) out.push(f.path);
    return out;
  }, [steps, diff]);
  const pages = useMemo(() => visitedPages(steps), [steps]);
  const local = useMemo(() => localPages(steps), [steps]);
  const active = ctl.tabs.find((t) => t.id === ctl.active);
  const strip = useRef<HTMLDivElement | null>(null);
  const closeHint = useId();
  // Delete on a tab closes it (the way a tab list is closed from the
  // keyboard). Once the closed tab is gone from the page, the tab now in
  // front takes the focus it held; with no tab left, the "+" beside the
  // strip. The page may draw the whole canvas again when a tab closes, so
  // this looks in the document, not in this strip, and gives up after a
  // moment rather than wait for ever.
  const closeFromKeyboard = (id: string) => {
    ctl.close(id);
    let frames = 0;
    const settle = () => {
      const canvas = document.querySelector<HTMLElement>("[data-canvas]");
      const gone = !Array.from(document.querySelectorAll<HTMLElement>("[data-canvas] [data-tab]")).some((el) => el.dataset.tab === id);
      if (canvas && gone) {
        (canvas.querySelector<HTMLElement>('[data-active="true"] [role="tab"]') ?? canvas.querySelector<HTMLElement>("[data-canvas-open]"))?.focus();
        return;
      }
      if (++frames < 30) requestAnimationFrame(settle);
    };
    requestAnimationFrame(settle);
  };
  // When the tabs would not all fit with their names, the ones behind keep
  // only their icons; the one in front always has its name.
  const stripWidth = useWidth(strip);
  const compact = stripWidth > 0 && stripWidth < ctl.tabs.length * 104;
  // The tab in front stays in view when the strip overflows. Scrolled on
  // the strip alone: scrollIntoView would also scroll every ancestor that
  // overflows, and on a desktop the canvas sits in a column that shares
  // its scroller with the conversations list, which must not move.
  useEffect(() => {
    const el = strip.current;
    const tab = el?.querySelector<HTMLElement>('[data-active="true"]');
    if (el && tab) revealIn(el, tab, "nearest", "auto", "x");
  }, [ctl.active]);

  return (
    <section
      className="enter flex min-h-0 min-w-0 flex-col overflow-hidden rounded-xl border bg-card shadow-card lg:h-full"
      style={{ animationDelay: "60ms" }}
      aria-label={label}
      data-canvas
    >
      <div className="flex min-w-0 items-center gap-1 border-b px-1.5">
        <div
          ref={strip}
          className="scroll-strip flex min-w-0 flex-1 items-center gap-0.5"
          role="tablist"
          aria-label="Open on the canvas"
        >
          {ctl.tabs.map((t) => {
            const Icon = KIND_ICON[t.kind];
            const on = t.id === ctl.active;
            const iconOnly = compact && !on;
            // The count beside a one-of-a-kind tab: changed files, artifacts,
            // sub-sessions, steps - the same numbers the "+" menu shows.
            const n = t.key ? undefined : counts[t.kind];
            return (
              // The tab is the button that brings it in front. The cross beside
              // it is the mouse's way to close; a tab may hold no control of
              // its own, so the keyboard closes with Delete (the hint under
              // the strip says so to a screen reader).
              <div
                key={t.id}
                data-tab={t.id}
                data-active={on}
                className={cn(
                  "group/tab relative flex h-9 max-w-[12rem] shrink-0 items-center text-[13px] transition-colors",
                  "after:absolute after:inset-x-1 after:bottom-0 after:h-0.5 after:rounded-full after:bg-transparent",
                  iconOnly ? "px-0.5" : "pl-2",
                  on
                    ? "font-medium text-foreground after:bg-link"
                    : "text-muted-foreground hover:text-foreground",
                )}
              >
                <button
                  type="button"
                  role="tab"
                  aria-selected={on}
                  aria-describedby={closeHint}
                  onClick={() => ctl.select(t.id)}
                  onKeyDown={(e) => {
                    if (e.key !== "Delete" || e.metaKey || e.ctrlKey || e.altKey) return;
                    e.preventDefault();
                    closeFromKeyboard(t.id);
                  }}
                  className={cn(
                    "flex min-w-0 items-center gap-1.5 py-1 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-md",
                    iconOnly && "px-1.5",
                  )}
                  title={t.key || t.label}
                  aria-label={iconOnly ? t.label : undefined}
                >
                  {t.kind === "file" ? (
                    <FileKindIcon name={t.key} className="size-4 shrink-0" />
                  ) : (
                    <Icon className="size-4 shrink-0" aria-hidden />
                  )}
                  {iconOnly ? null : (
                    <span className="min-w-0 truncate">{t.label}</span>
                  )}
                  {!iconOnly && n ? (
                    <span className="tnum inline-flex h-5 shrink-0 items-center rounded-md bg-muted px-1.5 text-xs font-medium text-muted-foreground">
                      {num(n)}
                    </span>
                  ) : null}
                  {t.kind === "terminal" && running ? (
                    <span
                      className="live-dot size-1.5 shrink-0 rounded-full bg-primary"
                      data-live="true"
                      role="img"
                      aria-label="a command is running"
                    />
                  ) : null}
                </button>
                {iconOnly ? null : (
                  <button
                    type="button"
                    tabIndex={-1}
                    aria-hidden="true"
                    onClick={() => ctl.close(t.id)}
                    className={cn(
                      "ml-0.5 mr-1 rounded p-0.5 text-muted-foreground transition-opacity hover:bg-muted hover:text-foreground",
                      !on &&
                        "opacity-0 group-hover/tab:opacity-100 focus-visible:opacity-100",
                    )}
                    title={"Close " + t.label}
                  >
                    <XIcon className="size-3.5" />
                  </button>
                )}
              </div>
            );
          })}
        </div>
        <span id={closeHint} className="sr-only">
          Press Delete to close this tab.
        </span>
        <OpenMenu
          ctl={ctl}
          menu={menu}
          files={files}
          pages={pages}
          local={local}
          counts={counts}
        />
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              type="button"
              size="icon-sm"
              variant="ghost"
              className="hidden shrink-0 lg:inline-flex"
              onClick={() => ctl.setWide(!ctl.wide)}
              aria-label={
                ctl.wide ? "Make the canvas narrower" : "Make the canvas wider"
              }
            >
              {ctl.wide ? <Minimize2Icon /> : <Maximize2Icon />}
            </Button>
          </TooltipTrigger>
          <TooltipContent>{ctl.wide ? "Narrower" : "Wider"}</TooltipContent>
        </Tooltip>
      </div>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col" role="tabpanel">
        {!active ? (
          <Pane>
            <EmptyState
              icon={<PanelRightIcon className="size-[18px]" />}
              hint="Files the agent edits, its terminal and the pages it reads open here by themselves."
            >
              Nothing open. Use + to open the terminal, a file, the plan or the
              changes.
            </EmptyState>
          </Pane>
        ) : active.kind === "terminal" ? (
          <Terminal entries={entries} />
        ) : active.kind === "file" ? (
          <FileTab
            key={active.key}
            session={session}
            path={active.key}
            steps={steps}
            diff={diff ?? null}
          />
        ) : active.kind === "page" ? (
          <PageTab key={active.key} steps={steps} page={active.key} />
        ) : active.kind === "browser" ? (
          <BrowserTab
            key={active.id}
            url={active.key}
            sink={picks}
            onAsk={onAsk}
            onNavigate={(url) => {
              // The tab is named after its address: a new address is a new
              // tab in the old one's place.
              if (url === active.key) return;
              ctl.close(active.id);
              ctl.open("browser", url);
            }}
          />
        ) : active.kind === "plan" ? (
          <PlanTab steps={steps} />
        ) : active.kind === "diff" ? (
          diff && changes ? (
            <ChangesBody diff={diff} className="max-h-[60dvh] lg:max-h-none" />
          ) : (
            <Pane>
              <p className="text-[13px] leading-relaxed text-muted-foreground">
                The diff appears once the session has a project and a
                branch with changes.
              </p>
            </Pane>
          )
        ) : active.kind === "artifact" ? (
          <ArtifactTab
            source={{ kind: "chat", id: chatId ?? "" }}
            artifact={artifacts.find((a) => a.name === active.key)}
            name={active.key}
          />
        ) : active.kind === "evidence" ? (
          <EvidenceTab
            key={active.key}
            jobId={active.key}
            job={jobs.find((j) => j.id === active.key)}
          />
        ) : active.kind === "links" ? (
          <Links texts={texts} />
        ) : panels[active.kind] ? (
          <Pane>{panels[active.kind]}</Pane>
        ) : (
          <Pane>
            <p className="text-[13px] text-muted-foreground">
              Nothing to show here.
            </p>
          </Pane>
        )}
      </div>
    </section>
  );
}

// A tab body that scrolls on its own; on a phone it stops at a sensible
// height so the page below stays reachable.
function Pane({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "scroll-thin grid max-h-[70dvh] min-h-0 min-w-0 flex-1 content-start gap-3 overflow-y-auto px-3 py-3 lg:max-h-none",
        className,
      )}
    >
      {children}
    </div>
  );
}

function OpenMenu({
  ctl,
  menu,
  files,
  pages,
  local,
  counts,
}: {
  ctl: CanvasControl;
  menu: CanvasKind[];
  files: string[];
  pages: string[];
  /** Pages served on this machine the run visited or announced, newest first. */
  local: string[];
  counts: Partial<Record<CanvasKind, number>>;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          size="icon-sm"
          variant="ghost"
          className="shrink-0"
          aria-label="Open something on the canvas"
          title="Open…"
          data-canvas-open
        >
          <PlusIcon />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-60">
        <DropdownMenuLabel className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
          Open
        </DropdownMenuLabel>
        {menu.map((kind) => {
          const Icon = KIND_ICON[kind];
          const n = counts[kind];
          if (kind === "browser" && local.length > 1) {
            return (
              <DropdownMenuSub key={kind}>
                <DropdownMenuSubTrigger>
                  <Icon /> {KIND_LABEL[kind]}
                  <span className="ml-auto text-xs text-muted-foreground tnum">
                    {num(local.length)}
                  </span>
                </DropdownMenuSubTrigger>
                <DropdownMenuSubContent className="scroll-thin max-h-80 w-72 overflow-y-auto">
                  {local.slice(0, 20).map((u) => (
                    <DropdownMenuItem
                      key={u}
                      onSelect={() => ctl.open("browser", u)}
                      title={u}
                    >
                      <AppWindowIcon className="size-3.5 shrink-0 text-muted-foreground" />
                      <span className="min-w-0 truncate font-mono text-xs">
                        {makeTab("browser", u).label}
                      </span>
                    </DropdownMenuItem>
                  ))}
                  <DropdownMenuSeparator />
                  <DropdownMenuItem onSelect={() => ctl.open("browser")}>
                    <PlusIcon className="size-3.5 shrink-0 text-muted-foreground" />
                    <span className="text-xs">Another address…</span>
                  </DropdownMenuItem>
                </DropdownMenuSubContent>
              </DropdownMenuSub>
            );
          }
          return (
            <DropdownMenuItem
              key={kind}
              onSelect={() =>
                ctl.open(kind, kind === "browser" ? (local[0] ?? "") : "")
              }
            >
              <Icon /> {KIND_LABEL[kind]}
              {kind === "browser" && local[0] ? (
                <span className="ml-auto min-w-0 max-w-28 truncate font-mono text-xs text-muted-foreground">
                  {makeTab("browser", local[0]).label}
                </span>
              ) : n ? (
                <span className="ml-auto text-xs text-muted-foreground tnum">
                  {num(n)}
                </span>
              ) : null}
            </DropdownMenuItem>
          );
        })}
        {files.length || pages.length ? <DropdownMenuSeparator /> : null}
        {files.length ? (
          <DropdownMenuSub>
            <DropdownMenuSubTrigger>
              <FileTextIcon /> Files{" "}
              <span className="ml-auto text-xs text-muted-foreground tnum">
                {num(files.length)}
              </span>
            </DropdownMenuSubTrigger>
            <DropdownMenuSubContent className="scroll-thin max-h-80 w-72 overflow-y-auto">
              {files.slice(0, 40).map((p) => (
                <DropdownMenuItem
                  key={p}
                  onSelect={() => ctl.open("file", p)}
                  title={p}
                >
                  <FileKindIcon
                    name={p}
                    className="size-3.5 shrink-0 text-muted-foreground"
                  />
                  <span className="min-w-0 truncate font-mono text-xs">
                    {p}
                  </span>
                </DropdownMenuItem>
              ))}
            </DropdownMenuSubContent>
          </DropdownMenuSub>
        ) : null}
        {pages.length ? (
          <DropdownMenuSub>
            <DropdownMenuSubTrigger>
              <GlobeIcon /> Pages{" "}
              <span className="ml-auto text-xs text-muted-foreground tnum">
                {num(pages.length)}
              </span>
            </DropdownMenuSubTrigger>
            <DropdownMenuSubContent className="scroll-thin max-h-80 w-72 overflow-y-auto">
              {pages.slice(0, 40).map((p) => (
                <DropdownMenuItem
                  key={p}
                  onSelect={() => ctl.open("page", p)}
                  title={p}
                >
                  {/^https?:\/\//i.test(p) ? (
                    <GlobeIcon className="size-3.5 shrink-0 text-muted-foreground" />
                  ) : (
                    <SearchIcon className="size-3.5 shrink-0 text-muted-foreground" />
                  )}
                  <span className="min-w-0 truncate text-xs">
                    {makeTab("page", p).label}
                  </span>
                </DropdownMenuItem>
              ))}
            </DropdownMenuSubContent>
          </DropdownMenuSub>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

// ----- terminal -----------------------------------------------------------------

// Every command of the run with what it printed, in a terminal that follows
// the newest line while you let it (scroll up and it stays put).
function Terminal({ entries }: { entries: TerminalEntry[] }) {
  const box = useRef<HTMLDivElement | null>(null);
  const [stick, setStick] = useState(true);
  const last = entries[entries.length - 1];
  const key =
    entries.length +
    ":" +
    (last ? last.output.length + ":" + String(last.running) : "");
  useEffect(() => {
    if (stick && box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [key, stick]);
  return (
    <div
      ref={box}
      onScroll={(e) => {
        const el = e.currentTarget;
        setStick(el.scrollHeight - el.scrollTop - el.clientHeight < 60);
      }}
      className="scroll-thin max-h-[60dvh] min-h-0 min-w-0 flex-1 overflow-auto bg-zinc-950 px-3 py-2.5 font-mono text-xs leading-[1.55] text-zinc-200 lg:max-h-none dark:bg-zinc-950"
      aria-label="Terminal"
      aria-live="polite"
    >
      {!entries.length ? (
        <p className="text-zinc-400">
          No commands yet. Each command the agent runs shows here with what it
          printed.
        </p>
      ) : null}
      {entries.map((e) => {
        const exit = exitWords(e.exit_code);
        return (
          <div key={e.n} className="mb-3 min-w-0 last:mb-0">
            {e.title ? (
              <p className="text-zinc-400 anywhere"># {e.title}</p>
            ) : null}
            <p className="flex min-w-0 gap-2">
              <span
                className="shrink-0 text-emerald-400 select-none"
                aria-hidden
              >
                {e.kind === "process" ? "%" : "$"}
              </span>
              <span className="min-w-0 font-medium whitespace-pre-wrap text-zinc-50 anywhere">
                {e.command}
              </span>
            </p>
            {e.output ? (
              <pre className="mt-0.5 min-w-0 whitespace-pre-wrap text-zinc-300 anywhere">
                {e.output}
              </pre>
            ) : null}
            <p className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 text-xs text-zinc-400">
              {e.running ? (
                <span className="inline-flex items-center gap-1 text-primary">
                  <LoaderCircleIcon className="size-3 animate-spin" /> running
                </span>
              ) : exit ? (
                <span
                  className={cn(exit.bad ? "text-red-400" : "text-zinc-400")}
                >
                  {exit.text}
                </span>
              ) : null}
              {e.duration_ms ? (
                <span className="tnum">
                  {span(Math.round(e.duration_ms / 1000))}
                </span>
              ) : null}
            </p>
          </div>
        );
      })}
    </div>
  );
}

// ----- file ---------------------------------------------------------------------

const LINES_AT_ONCE = 3000;

// One file of the run's workspace, as it is now, with the lines the branch
// added marked; the Changes view shows the same file's hunks instead. When
// the workspace is gone (a finished job's clone), the hunks or the blocks
// the agent wrote are what remain to read.
function FileTab({
  session,
  path,
  steps,
  diff,
}: {
  session: string;
  path: string;
  steps: HarnessStep[];
  diff: ChatDiff | null;
}) {
  const file = useApi<HarnessFileResponse>(
    session ? harnessFilePath(session, path) : null,
    { refresh: false },
  );
  const edits = steps.filter((s) => stepPath(s) === path).length;
  const reload = file.reload;
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    void reload();
  }, [edits, reload]);
  const df = diff?.files?.find((f) => f.path === path) ?? null;
  const marks = useMemo(
    () => (df ? changedNewLines(df.lines) : new Set<number>()),
    [df],
  );
  const blocks = useMemo(() => editBlocks(steps, path), [steps, path]);
  // A file the agent edited in this run opens as it is now; one only the
  // branch changed (opened from the Changes list) opens on its hunks.
  const [mode, setMode] = useState<"file" | "diff">(() =>
    df && !edits ? "diff" : "file",
  );
  const [all, setAll] = useState(false);
  const text = file.data?.text ?? "";
  const lines = useMemo(() => {
    const ls = text.split("\n");
    if (ls.length > 1 && ls[ls.length - 1] === "") ls.pop();
    return ls;
  }, [text]);
  const shown = all ? lines : lines.slice(0, LINES_AT_ONCE);
  // The newest edit's first line is where the eye should land; failing
  // that, the first line the branch added.
  const target = useMemo(() => {
    const lastBlock = blocks[blocks.length - 1];
    const want = lastBlock
      ? lastBlock.replace.split("\n").find((l) => l.trim())
      : undefined;
    if (want) {
      const i = lines.findIndex((l) => l.trim() === want.trim());
      if (i >= 0) return i + 1;
    }
    let min = 0;
    for (const n of marks) if (!min || n < min) min = n;
    return min;
  }, [blocks, lines, marks]);
  const box = useRef<HTMLDivElement | null>(null);
  const loaded = !!file.data;
  useEffect(() => {
    if (!loaded || mode !== "file" || !target) return;
    const el = box.current;
    const line = el?.querySelector<HTMLElement>('[data-line="' + target + '"]');
    if (el && line) revealIn(el, line, "center");
  }, [loaded, mode, target, edits]);
  const gone = !!file.error && !file.data;
  const showDiff = mode === "diff" || (gone && !!df);

  return (
    <>
      <div className="flex min-w-0 items-center gap-2 border-b px-3 py-1.5 text-xs">
        <code
          className="min-w-0 flex-1 truncate font-mono text-foreground"
          title={path}
        >
          {path}
        </code>
        {file.data ? (
          <span className="shrink-0 text-muted-foreground tnum">
            {bytesWord(file.data.size)}
          </span>
        ) : null}
        {df ? (
          <div
            className="flex shrink-0 rounded-md border p-px"
            role="group"
            aria-label="View"
          >
            <button
              type="button"
              onClick={() => setMode("file")}
              aria-pressed={!showDiff}
              className={cn(
                "rounded-[5px] px-2 py-0.5",
                !showDiff
                  ? "bg-muted font-medium text-foreground"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              File
            </button>
            <button
              type="button"
              onClick={() => setMode("diff")}
              aria-pressed={showDiff}
              className={cn(
                "rounded-[5px] px-2 py-0.5",
                showDiff
                  ? "bg-muted font-medium text-foreground"
                  : "text-muted-foreground hover:text-foreground",
              )}
            >
              Changes
            </button>
          </div>
        ) : null}
        <Button
          type="button"
          size="icon-xs"
          variant="ghost"
          onClick={() => void reload()}
          disabled={file.loading || !session}
          aria-label="Read the file again"
          title="Read the file again"
        >
          <RefreshCwIcon className={cn(file.loading && "animate-spin")} />
        </Button>
      </div>
      <div
        ref={box}
        className="scroll-thin max-h-[60dvh] min-h-0 min-w-0 flex-1 overflow-auto bg-background/50 lg:max-h-none"
      >
        {showDiff && df ? (
          <DiffTable f={df} />
        ) : gone ? (
          <div className="grid gap-3 p-3">
            <Notice tone="info">{file.error}</Notice>
            {blocks.length ? <EditBlocks blocks={blocks} /> : null}
          </div>
        ) : !file.data ? (
          !session ? (
            <div className="grid gap-3 p-3">
              <p className="text-[13px] text-muted-foreground">
                No run to read the file from yet.
              </p>
              {blocks.length ? <EditBlocks blocks={blocks} /> : null}
            </div>
          ) : (
            <SkeletonRows n={8} className="p-3" />
          )
        ) : file.data.binary ? (
          <p className="p-4 text-sm text-muted-foreground">
            A binary file - nothing to show line by line.
          </p>
        ) : (
          <>
            <table
              className="w-full border-collapse font-mono text-xs leading-[1.6]"
              style={{ tabSize: 4 }}
            >
              <tbody>
                {shown.map((l, i) => {
                  const n = i + 1;
                  const hit = marks.has(n);
                  return (
                    <tr
                      key={n}
                      data-line={n}
                      data-mark={hit || undefined}
                      className={cn(hit && "bg-ok/[0.1]")}
                    >
                      <td className="w-[1%] min-w-[3ch] pr-2 pl-2.5 text-right align-top text-muted-foreground select-none tnum">
                        {n}
                      </td>
                      <td
                        className={cn(
                          "w-[1%] px-1 text-center align-top select-none",
                          hit ? "text-ok" : "text-transparent",
                        )}
                      >
                        {hit ? "+" : " "}
                      </td>
                      <td className="pr-4 whitespace-pre text-foreground/90">
                        {l || " "}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            {lines.length > shown.length ? (
              <button
                type="button"
                onClick={() => setAll(true)}
                className="m-3 rounded-md border px-2.5 py-1 text-xs text-muted-foreground hover:bg-muted/60 hover:text-foreground"
              >
                Show the other {num(lines.length - shown.length)} lines
              </button>
            ) : null}
            {file.data.truncated ? (
              <p className="px-3 pb-3 text-xs text-muted-foreground">
                The hub stops at 512 KB; the file goes on.
              </p>
            ) : null}
          </>
        )}
      </div>
    </>
  );
}

function EditBlocks({
  blocks,
}: {
  blocks: { n: number; at: string; search: string; replace: string }[];
}) {
  return (
    <div className="grid gap-2" aria-label="What the agent changed">
      <p className="text-xs font-medium text-muted-foreground">
        {plural(blocks.length, "edit")} the agent made here
      </p>
      {blocks.map((b, i) => (
        <div
          key={b.n + ":" + i}
          className="overflow-hidden rounded-lg border font-mono text-xs leading-[1.55]"
        >
          {b.search ? (
            <pre className="bg-bad/[0.07] px-3 py-1.5 whitespace-pre-wrap text-foreground/80 anywhere">
              {b.search}
            </pre>
          ) : null}
          <pre className="bg-ok/[0.09] px-3 py-1.5 whitespace-pre-wrap text-foreground anywhere">
            {b.replace}
          </pre>
        </div>
      ))}
    </div>
  );
}

// ----- page ---------------------------------------------------------------------

// What a fetch brought back or a search found, with the link to the real
// page; nothing is framed.
function PageTab({ steps, page }: { steps: HarnessStep[]; page: string }) {
  const r = pageText(steps, page);
  const isUrl = /^https?:\/\//i.test(page);
  return (
    <>
      <div className="flex min-w-0 items-center gap-2 border-b px-3 py-1.5 text-xs">
        {isUrl ? (
          <GlobeIcon
            className="size-3.5 shrink-0 text-muted-foreground"
            aria-hidden
          />
        ) : (
          <SearchIcon
            className="size-3.5 shrink-0 text-muted-foreground"
            aria-hidden
          />
        )}
        <span className="min-w-0 flex-1 truncate text-foreground" title={page}>
          {page}
        </span>
        {r?.at ? (
          <When
            d={r.at}
            className="hidden shrink-0 text-muted-foreground sm:inline"
          />
        ) : null}
        {isUrl ? (
          <Button asChild size="xs" variant="outline">
            <a href={page} target="_blank" rel="noreferrer noopener">
              <ExternalLinkIcon /> Open
            </a>
          </Button>
        ) : null}
      </div>
      <div className="scroll-thin max-h-[60dvh] min-h-0 min-w-0 flex-1 overflow-auto bg-background/50 lg:max-h-none">
        {r?.text ? (
          <pre className="px-4 py-3 text-[13px] leading-relaxed whitespace-pre-wrap text-foreground/90 anywhere">
            {r.text}
          </pre>
        ) : (
          <p className="p-4 text-sm text-muted-foreground">
            {r
              ? "Nothing came back yet."
              : "This page is not in the current run."}
          </p>
        )}
      </div>
    </>
  );
}

// ----- plan ---------------------------------------------------------------------

export function PlanList({
  step,
  compact = false,
}: {
  step: HarnessStep;
  compact?: boolean;
}) {
  const items = planItems(step.content);
  if (!items.length) return null;
  const done = items.filter((it) => it.status === "done").length;
  return (
    <div
      className={cn(
        "grid min-w-0 gap-1.5 rounded-xl border bg-muted/30 px-3 py-2.5",
        !compact && "border-primary/25 bg-primary/[0.04]",
      )}
    >
      <p className="flex min-w-0 items-center gap-1.5 text-xs font-medium text-muted-foreground">
        <ListChecksIcon
          className="size-3.5 shrink-0 text-primary"
          aria-hidden
        />
        Plan
        <span className="tnum">
          {done}/{items.length} done
        </span>
      </p>
      <ul className="grid gap-1 text-[13px] leading-snug">
        {items.map((it, i) => (
          <li key={i} className="flex min-w-0 items-start gap-2">
            {it.status === "done" ? (
              <CircleCheckIcon
                className="mt-0.5 size-3.5 shrink-0 text-ok"
                aria-label="done"
              />
            ) : it.status === "doing" ? (
              <LoaderCircleIcon
                className="mt-0.5 size-3.5 shrink-0 animate-spin text-primary"
                aria-label="in progress"
              />
            ) : (
              <CircleIcon
                className="mt-0.5 size-3.5 shrink-0 text-muted-foreground/60"
                aria-label="to do"
              />
            )}
            <span
              className={cn(
                "min-w-0 anywhere",
                it.status === "done" && "text-muted-foreground line-through",
                it.status === "doing" && "font-medium text-foreground",
              )}
            >
              {it.text}
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function PlanTab({ steps }: { steps: HarnessStep[] }) {
  const plan = latestPlanStep(steps);
  return (
    <Pane>
      {plan ? (
        <>
          <PlanList step={plan} />
          <p className="px-1 text-xs text-muted-foreground">
            Updated <When d={plan.at} />
          </p>
        </>
      ) : (
        <EmptyState
          icon={<ListChecksIcon className="size-[18px]" />}
          hint="It keeps one when the work has more than a step or two."
        >
          The agent has not written a plan in this run.
        </EmptyState>
      )}
    </Pane>
  );
}

// ----- artifact -----------------------------------------------------------------

// One file of a conversation's or a job's folder, with its bar.
function ArtifactTab({
  source,
  artifact,
  name,
}: {
  source: ArtifactSource;
  artifact?: ChatArtifact;
  name: string;
}) {
  if (!artifact || !source.id) {
    return (
      <Pane>
        <p className="text-[13px] text-muted-foreground">
          {name +
            " is not in " +
            (source.kind === "job" ? "this step's" : "this session's") +
            " artifacts any more."}
        </p>
      </Pane>
    );
  }
  const kind = fileKind(artifact.name);
  const href = artifactHref(source, artifact.name);
  return (
    <>
      <div className="flex min-w-0 items-center gap-2 border-b px-3 py-1.5 text-xs">
        <FileKindIcon
          name={artifact.name}
          className="size-3.5 shrink-0 text-muted-foreground"
        />
        <span
          className="min-w-0 flex-1 truncate font-medium text-foreground"
          title={artifact.name}
        >
          {artifact.name}
        </span>
        <span className="shrink-0 text-muted-foreground tnum">
          {bytesWord(artifact.size)}
        </span>
        <When
          d={artifact.modified}
          className="hidden shrink-0 text-muted-foreground sm:inline"
        />
        <Button asChild size="xs" variant="outline">
          <a href={href} download={artifact.name}>
            <DownloadIcon /> Download
          </a>
        </Button>
      </div>
      <div
        className={cn(
          "scroll-thin min-h-0 min-w-0 flex-1 overflow-auto bg-background/50",
          kind === "html" || kind === "pdf"
            ? "max-h-[60dvh] p-0 lg:max-h-none"
            : "max-h-[60dvh] p-3 lg:max-h-none",
        )}
      >
        <ArtifactBody key={href} href={href} name={artifact.name} kind={kind} />
      </div>
    </>
  );
}

// ----- evidence -----------------------------------------------------------------

// The files a fleet job left in its artifacts folder - the screenshots and
// reports that show what it did - listed the way a conversation's Artifacts
// panel lists its own, with the chosen one shown below: an image inline, a
// page in a sandboxed frame, text as text. The listing comes from GET
// jobs/{id}/artifacts and refreshes while the tab is open, so a running
// step's new screenshot appears on its own; the newest image opens first.
function EvidenceTab({ jobId, job }: { jobId: string; job?: TaskJob }) {
  const { data, error, loading, reload } = useApi<JobArtifactsResponse>(
    jobId ? jobArtifactsPath(jobId) : null,
    { refresh: job?.status === "leased" ? 10000 : 60000 },
  );
  const artifacts = useMemo(() => data?.artifacts ?? [], [data]);
  const [picked, setPicked] = useState("");
  const selected = keepSelection(picked, artifacts);
  const source = useMemo<ArtifactSource>(
    () => ({ kind: "job", id: jobId }),
    [jobId],
  );
  const title = jobEvidenceLabel(job?.title ?? "");
  if (!data && loading) return <SkeletonRows n={3} className="p-3" />;
  if (!artifacts.length) {
    return (
      <Pane>
        <EmptyState
          icon={<ImagesIcon className="size-[18px]" />}
          hint={"Job " + jobId + ". It knows the folder as $HIVE_ARTIFACTS."}
        >
          {evidenceEmptyText(error)}
        </EmptyState>
        {error ? (
          <div>
            <Button
              type="button"
              size="xs"
              variant="outline"
              onClick={() => void reload()}
            >
              <RefreshCwIcon /> Try again
            </Button>
          </div>
        ) : null}
      </Pane>
    );
  }
  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col"
      aria-label={title}
      data-evidence={jobId}
    >
      <div className="scroll-thin max-h-[30dvh] shrink-0 overflow-y-auto border-b px-1.5 py-1.5 lg:max-h-[14rem]">
        <p
          className="flex items-center gap-1.5 px-1.5 pb-1 text-xs text-muted-foreground"
          title={"Job " + jobId}
        >
          <ImagesIcon className="size-3" aria-hidden />
          <span className="min-w-0 flex-1 truncate">
            {plural(artifacts.length, "file") +
              " this step left" +
              (job?.title ? " for \u201c" + job.title + "\u201d" : "")}
          </span>
          {error ? (
            <span className="shrink-0 text-warn" title={error}>
              stale
            </span>
          ) : null}
        </p>
        <ArtifactList
          id={jobId}
          source={source}
          artifacts={artifacts}
          view={selected ? { kind: "artifact", name: selected } : null}
          onView={(v) => {
            if (v.kind === "artifact") setPicked(v.name);
          }}
        />
      </div>
      <ArtifactTab
        source={source}
        artifact={artifacts.find((a) => a.name === selected)}
        name={selected}
      />
    </div>
  );
}

// ----- links --------------------------------------------------------------------

// The links the agent and you mentioned, newest first - pull requests,
// pages it read, places it pointed at.
function Links({ texts }: { texts: string[] }) {
  const links = useMemo(
    () =>
      extractLinks(
        texts,
        typeof window !== "undefined" ? window.location.host : "",
      ),
    [texts],
  );
  return (
    <Pane>
      {links.length ? (
        <ul className="grid min-w-0 gap-px" role="list" aria-label="Links">
          {links.map((l) => (
            <li key={l.url} className="min-w-0">
              <a
                href={l.url}
                target="_blank"
                rel="noreferrer noopener"
                className="flex min-w-0 items-center gap-2 rounded-md px-1.5 py-1 text-[13px] text-foreground transition-colors hover:bg-muted/60"
                title={l.url}
              >
                <LinkIcon
                  className="size-3.5 shrink-0 text-muted-foreground"
                  aria-hidden
                />
                <span className="min-w-0 flex-1 truncate">{l.label}</span>
                <ExternalLinkIcon
                  className="size-3 shrink-0 text-muted-foreground"
                  aria-hidden
                />
              </a>
            </li>
          ))}
        </ul>
      ) : (
        <EmptyState
          icon={<LinkIcon className="size-[18px]" />}
          hint="Pull requests, pages and places the agent names in its replies."
        >
          No links mentioned yet.
        </EmptyState>
      )}
    </Pane>
  );
}
