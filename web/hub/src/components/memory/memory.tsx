"use client";

import Link from "next/link";
import { useEffect, useMemo, useRef, useState } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { BotIcon, BrainIcon, CheckIcon, FolderXIcon, Loader2Icon, MessageSquareIcon, PencilIcon, PlusIcon, SearchIcon, Trash2Icon, UserIcon, XIcon, type LucideIcon } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { showApiError } from "@/components/pulse/confirm";
import { ProjectAvatar } from "@/components/pulse/avatar";
import { ListRow, ListRows } from "@/components/pulse/list-row";
import { PageHeader } from "@/components/pulse/page-header";
import { Disclosure, EmptyState, ErrorNotice, Notice, SkeletonRows } from "@/components/pulse/panel";
import { SectionHeading } from "@/components/pulse/section-heading";
import { Mono } from "@/components/pulse/who";
import { query, writes, type MemoryEntry, type MemoryFile, type MemoryPayload } from "@/lib/api";
import { plural, rel } from "@/lib/format";
import { dayWords, fileHint, fileTitle, textRoom } from "@/lib/memory";
import { hrefs } from "@/lib/nav";
import { invalidate, useApi, useNow } from "@/lib/use-api";
import { cn } from "@/lib/utils";
import { chosenFile, emptyWords, fileChoices, fileNote, shownEntries, sourceOf, sourceWords, undoNote, type EntrySource, type FileChoice } from "./memory-state";

// Memory (spec "Memory"): what the agents kept for later - a Markdown file
// of dated bullets per project under ~/.pulse/memory/, one for the owner
// that every project reads, one shared by sessions without a project. The
// harness reads them into every run's prompt, so what is here shapes what
// agents do next. Two panes: the files on the left (the choice goes in the
// URL as ?file=), the chosen file's entries on the right, each with who
// kept it and when. The owner remembers a fact from an inline composer at
// the top of the pane, edits one in place, and forgets one with an Undo.

export const refreshMemory = () => invalidate((p) => p === "memory" || p.startsWith("memory?") || p.startsWith("events"));

const SOURCE_ICON: Record<EntrySource, LucideIcon> = { you: UserIcon, agent: BotIcon, session: MessageSquareIcon };

const todayIso = () => new Date().toISOString().slice(0, 10);

// An entry's `code` spans as code, the rest as plain text: agents write
// memory in Markdown, and raw backticks read as noise.
function InlineText({ text }: { text: string }) {
  const parts = text.split(/(`[^`\n]+`)/g);
  return (
    <>
      {parts.map((p, i) =>
        p.length > 2 && p.startsWith("`") && p.endsWith("`") ? (
          <code key={i} className="rounded-sm bg-muted px-1 py-px font-mono text-xs">
            {p.slice(1, -1)}
          </code>
        ) : (
          p.replace(/\*\*([^*]+)\*\*/g, "$1")
        ),
      )}
    </>
  );
}

// The writes the page makes, all through the hub's two memory routes (the
// same ones an agent's `memory remember` / `memory forget` use). There is
// no update and no undelete route: an edit forgets the old line and
// remembers the new text; Undo after a forget remembers the text again
// (dated today, since the file keeps no other record).
const remember = (file: MemoryFile, text: string) => writes.rememberMemory({ key: file.key, scope: file.scope, text });
const forget = (file: MemoryFile, entry: MemoryEntry) => writes.forgetMemory({ key: file.key, scope: file.scope, text: entry.line });

export function MemoryPageClient() {
  const sp = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const search = sp.get("q") || "";
  const fileParam = sp.get("file");
  const legacyKey = sp.get("key");
  const q = useApi<MemoryPayload>("memory", { refresh: 15000 });
  const [composing, setComposing] = useState(false);
  // Lines the owner kept from this page in this session, so their rows wear
  // the owner's icon rather than an agent's (the file itself keeps no author).
  const [mine, setMine] = useState<ReadonlySet<string>>(() => new Set());
  const keepMine = (line: string) => setMine((cur) => new Set(cur).add(line));

  const files = useMemo(() => q.data?.files ?? [], [q.data]);
  const choices = useMemo(() => fileChoices(files), [files]);
  const file = useMemo(() => chosenFile(files, fileParam, legacyKey), [files, fileParam, legacyKey]);
  const chosenParam = file ? (choices.find((c) => c.file.key === file.key)?.param ?? file.key) : "";

  const update = (next: { file?: string; q?: string }) => {
    const params: Record<string, string> = {};
    for (const [k, v] of sp.entries()) if (k !== "key") params[k] = v; // the older ?key= becomes ?file=
    if (next.file !== undefined) params.file = next.file;
    if (next.q !== undefined) params.q = next.q;
    const clean = Object.fromEntries(Object.entries(params).filter(([, v]) => v));
    router.replace(pathname + query(clean), { scroll: false });
  };
  const choose = (param: string) => {
    setComposing(false);
    update({ file: param });
  };

  return (
    <div className="min-w-0">
      <PageHeader
        title="Memory"
        subtitle="What your agents remember"
        action={
          <Button onClick={() => setComposing(true)} disabled={!file}>
            <PlusIcon /> Remember something
          </Button>
        }
      />
      {q.error && !q.data ? (
        <ErrorNotice error={q.error} onRetry={() => void q.reload()}>
          {"The memory files didn't load (" + q.error + "). Check that PULSE is running, then try again."}
        </ErrorNotice>
      ) : null}
      {!q.data ? (
        q.error ? null : <SkeletonRows n={5} />
      ) : (
        <div className="grid min-w-0 gap-5">
          {!q.data.on ? (
            <Notice tone="warn">
              Memory is off (<Mono>harness.memory</Mono>), so runs neither read nor write these files. Turn it on under{" "}
              <Link href={hrefs.settings("harness")} className="underline underline-offset-4">
                Settings, Harness
              </Link>
              .
            </Notice>
          ) : null}
          <div className="grid min-w-0 gap-5 lg:grid-cols-12">
            <div className="min-w-0 lg:col-span-4">
              <FilesPane choices={choices} chosen={chosenParam} onChoose={choose} />
            </div>
            <div className="min-w-0 lg:col-span-8">
              {file ? (
                <EntriesPane
                  key={file.key}
                  data={q.data}
                  file={file}
                  search={search}
                  onSearch={(v) => update({ q: v })}
                  composing={composing}
                  onComposed={() => setComposing(false)}
                  mine={mine}
                  keepMine={keepMine}
                />
              ) : (
                <EmptyState title="No memory files" icon={<BrainIcon className="size-[18px]" />}>
                  Nothing is remembered yet and no project is registered to remember for.
                </EmptyState>
              )}
            </div>
          </div>
          <Disclosure summary="How memory works" contentClassName="grid max-w-3xl gap-2 text-sm leading-relaxed text-muted-foreground anywhere">
            <p>
              Each file is plain Markdown; every line that starts with <Mono>- </Mono> is one memory, and the <Mono>[date]</Mono> in front is the day it was kept. You may edit the files by hand too.
            </p>
            <p>
              A run on a project reads that project&apos;s file and yours into its system prompt (up to 16 KB of each), and may call its <Mono>memory</Mono> tool to remember, forget or list. A chat not attached to a project shares the sessions file. The <Mono>harness.memory</Mono> setting turns all of it off.
            </p>
            <p>
              In a terminal: <Mono>/memory</Mono> inside <Mono>pulse chat</Mono> lists, adds and forgets the same way.
            </p>
          </Disclosure>
        </div>
      )}
    </div>
  );
}

// ----- the left pane --------------------------------------------------------

// The files: You, the sessions without a project, then each project with
// its avatar and how many entries it holds. The chosen one wears the teal
// left bar, like the nav. On a phone the list folds into a Select.
function FilesPane({ choices, chosen, onChoose }: { choices: FileChoice[]; chosen: string; onChoose: (param: string) => void }) {
  return (
    <>
      <div className="lg:hidden">
        <Label htmlFor="memory-file" className="sr-only">
          Which file
        </Label>
        <Select value={chosen} onValueChange={onChoose}>
          <SelectTrigger id="memory-file" aria-label="Which file" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {choices.map((c) => (
              <SelectItem key={c.param} value={c.param}>
                {c.title + (c.count ? " (" + c.count + ")" : "")}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <nav className="hidden min-w-0 lg:block" aria-label="Memory files">
        <ListRows ariaLabel="Memory files">
          {choices.map((c) => (
            <ListRow
              key={c.param}
              lead={<FileIcon c={c} />}
              title={c.title}
              selected={c.param === chosen}
              onOpen={() => onChoose(c.param)}
              ariaLabel={c.title + ", " + (c.count ? plural(c.count, "memory").replace("memorys", "memories") : "nothing yet")}
              status={
                <Badge variant={c.count ? "secondary" : "outline"} className="tnum" aria-hidden>
                  {c.count}
                </Badge>
              }
            >
              {c.icon === "orphan" ? "No project goes by this key any more" : c.icon === "project" && c.count === 0 ? "Nothing yet" : null}
            </ListRow>
          ))}
        </ListRows>
      </nav>
    </>
  );
}

function FileIcon({ c }: { c: FileChoice }) {
  if (c.icon === "project") return <ProjectAvatar slug={c.file.project} name={c.title} size={20} />;
  const Icon = c.icon === "user" ? UserIcon : c.icon === "sessions" ? MessageSquareIcon : FolderXIcon;
  return (
    <span className="inline-flex size-5 items-center justify-center text-muted-foreground">
      <Icon className="size-4" strokeWidth={1.75} aria-hidden />
    </span>
  );
}

// ----- the right pane -------------------------------------------------------

function EntriesPane({
  data,
  file,
  search,
  onSearch,
  composing,
  onComposed,
  mine,
  keepMine,
}: {
  data: MemoryPayload;
  file: MemoryFile;
  search: string;
  onSearch: (q: string) => void;
  composing: boolean;
  onComposed: () => void;
  mine: ReadonlySet<string>;
  keepMine: (line: string) => void;
}) {
  const now = useNow();
  const entries = useMemo(() => shownEntries(file, search), [file, search]);
  const [editing, setEditing] = useState<string | null>(null);
  const title = file.kind === "user" ? "You" : fileTitle(file);
  const FileHeadIcon = file.kind === "user" ? UserIcon : file.kind === "conversations" ? MessageSquareIcon : undefined;
  return (
    <section className="grid min-w-0 gap-3" aria-label={title}>
      <SectionHeading
        icon={FileHeadIcon}
        count={file.entries.length}
        note={fileNote(file)}
        action={
          file.project ? (
            <Link href={hrefs.project(file.project)} className="text-sm">
              Open the project
            </Link>
          ) : file.kind === "user" ? (
            <Link href={hrefs.settings("harness")} className="text-sm" title="Settings, Harness: turn memory on or off">
              Memory setting
            </Link>
          ) : null
        }
      >
        {file.kind === "project" && file.project ? (
          <span className="inline-flex items-center gap-2">
            <ProjectAvatar slug={file.project} name={title} size={20} />
            {title}
          </span>
        ) : (
          title
        )}
      </SectionHeading>
      <p className="-mt-2 text-[13px] leading-5 text-muted-foreground">{fileHint(file)}</p>
      {composing ? <Composer data={data} file={file} onDone={onComposed} keepMine={keepMine} /> : null}
      {file.entries.length > 5 || search ? (
        <span className="relative min-w-0 sm:max-w-sm">
          <SearchIcon className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden />
          <Input type="search" aria-label={"Search " + title} placeholder="Search this file…" value={search} onChange={(e) => onSearch(e.target.value)} className="h-9 pl-8" />
        </span>
      ) : null}
      {file.entries.length === 0 ? (
        <EmptyState
          title="Nothing remembered here"
          icon={<BrainIcon className="size-[18px]" />}
          compact
        >
          {emptyWords(file)}
        </EmptyState>
      ) : entries.length === 0 ? (
        <EmptyState
          title="No memory matches"
          icon={<SearchIcon className="size-[18px]" />}
          compact
          action={
            <Button size="sm" variant="outline" onClick={() => onSearch("")}>
              <XIcon /> Clear the search
            </Button>
          }
        >
          Try other words; every word you type must appear in the entry.
        </EmptyState>
      ) : (
        <ListRows ariaLabel={"Memories of " + title}>
          {entries.map((e) => (
            <EntryRow key={e.line} file={file} entry={e} now={now} source={sourceOf(e, file, mine)} editing={editing === e.line} onEdit={(on) => setEditing(on ? e.line : null)} keepMine={keepMine} />
          ))}
        </ListRows>
      )}
      {search && entries.length ? <p className="text-xs text-muted-foreground">{plural(entries.length, "match") + " of " + plural(file.entries.length, "memory").replace("memorys", "memories")}</p> : null}
    </section>
  );
}

// ----- one entry ----------------------------------------------------------

// A remembered fact: who kept it, the text, and when. The pencil turns the
// row into a textarea with Save and Cancel; the bin forgets it with an Undo
// in the toast.
function EntryRow({
  file,
  entry,
  now,
  source,
  editing,
  onEdit,
  keepMine,
}: {
  file: MemoryFile;
  entry: MemoryEntry;
  now: number;
  source: EntrySource;
  editing: boolean;
  onEdit: (on: boolean) => void;
  keepMine: (line: string) => void;
}) {
  const [busy, setBusy] = useState<"forget" | "save" | null>(null);
  const Icon = SOURCE_ICON[source];
  const who = sourceWords(source);
  const when = entry.day ? rel(entry.day, now) : "";
  const doForget = async () => {
    setBusy("forget");
    try {
      await forget(file, entry);
      refreshMemory();
      const text = entry.text;
      toast.success("Forgotten", {
        description: undoNote(entry, todayIso()).replace("Back in the file", "Undo puts it back in the file"),
        action: {
          label: "Undo",
          onClick: () => {
            void remember(file, text)
              .then(() => {
                keepMine("[" + todayIso() + "] " + text);
                refreshMemory();
                toast.success("Remembered again.", { description: undoNote(entry, todayIso()) });
              })
              .catch((e) => showApiError(e, "The memory didn't come back"));
          },
        },
      });
    } catch (e) {
      showApiError(e, "That didn't work");
    } finally {
      setBusy(null);
    }
  };
  if (editing) {
    return (
      <div className="px-3 py-2" data-row>
        <EntryEditor
          file={file}
          entry={entry}
          onDone={() => onEdit(false)}
          keepMine={keepMine}
          onBusy={(b) => setBusy(b ? "save" : null)}
        />
      </div>
    );
  }
  return (
    <ListRow
      lead={
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="inline-flex size-5 items-center justify-center text-muted-foreground" role="img" aria-label={who}>
              <Icon className="size-4" strokeWidth={1.75} aria-hidden />
            </span>
          </TooltipTrigger>
          <TooltipContent>{who}</TooltipContent>
        </Tooltip>
      }
      title={
        <span className="block whitespace-normal font-normal leading-5 anywhere" title={entry.day ? "Kept on " + dayWords(entry.day) : "Written by hand, no date"}>
          <InlineText text={entry.text} />
        </span>
      }
      time={when || undefined}
      className="items-start py-2.5"
      actions={
        <>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button type="button" size="icon-xs" variant="ghost" aria-label="Edit this memory" disabled={!!busy} onClick={() => onEdit(true)}>
                <PencilIcon aria-hidden />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Edit</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button type="button" size="icon-xs" variant="ghost" className="hover:text-status-failed" aria-label="Forget this memory" disabled={!!busy} onClick={() => void doForget()}>
                {busy === "forget" ? <Loader2Icon className="animate-spin" aria-hidden /> : <Trash2Icon aria-hidden />}
              </Button>
            </TooltipTrigger>
            <TooltipContent>Forget</TooltipContent>
          </Tooltip>
        </>
      }
    />
  );
}

// The row in edit: a textarea with the text, Save and Cancel. Saving
// forgets the old line and remembers the new text (there is no update
// route), so the entry comes back dated today.
function EntryEditor({ file, entry, onDone, keepMine, onBusy }: { file: MemoryFile; entry: MemoryEntry; onDone: () => void; keepMine: (line: string) => void; onBusy: (b: boolean) => void }) {
  const [text, setText] = useState(entry.text);
  const [busy, setBusy] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.focus();
    el.setSelectionRange(el.value.length, el.value.length);
  }, []);
  const next = text.trim().split(/\s+/).join(" ");
  const changed = next !== entry.text;
  const ready = !!next && changed && !busy;
  const save = async () => {
    if (!ready) return;
    setBusy(true);
    onBusy(true);
    try {
      await forget(file, entry);
      try {
        await remember(file, next);
      } catch (e) {
        // The old line is gone and the new one did not land: put the old
        // one back so nothing is lost, then say what failed.
        await remember(file, entry.text).catch(() => undefined);
        throw e;
      }
      keepMine("[" + todayIso() + "] " + next);
      refreshMemory();
      toast.success("Changed.", { description: entry.day && entry.day !== todayIso() ? "The memory is dated today now, instead of " + entry.day + "." : undefined });
      onDone();
    } catch (e) {
      showApiError(e, "The change wasn't kept");
    } finally {
      setBusy(false);
      onBusy(false);
    }
  };
  return (
    <div className="grid gap-2">
      <Label htmlFor={"memory-edit-" + entry.line} className="sr-only">
        The memory
      </Label>
      <Textarea
        ref={ref}
        id={"memory-edit-" + entry.line}
        value={text}
        onChange={(e) => setText(e.target.value)}
        rows={3}
        disabled={busy}
        onKeyDown={(e) => {
          if (e.key === "Escape") onDone();
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void save();
        }}
      />
      <div className="flex items-center gap-2">
        <Button type="button" size="sm" onClick={() => void save()} disabled={!ready}>
          {busy ? <Loader2Icon className="animate-spin" aria-hidden /> : <CheckIcon aria-hidden />} Save
        </Button>
        <Button type="button" size="sm" variant="ghost" onClick={onDone} disabled={busy}>
          Cancel
        </Button>
        <span className="text-xs text-muted-foreground pointer-coarse:hidden">⌘↩ saves, Esc cancels</span>
      </div>
    </div>
  );
}

// ----- the composer ---------------------------------------------------------

// Remember something: a textarea at the top of the chosen file's pane, with
// the hub's limit shown as it is typed. Saving is the same write an agent's
// `memory remember` makes.
function Composer({ data, file, onDone, keepMine }: { data: MemoryPayload; file: MemoryFile; onDone: () => void; keepMine: (line: string) => void }) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const ref = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    ref.current?.focus();
  }, []);
  const room = textRoom(text, data.limits.text);
  const ready = !!text.trim() && !room.over && !busy;
  const title = file.kind === "user" ? "you" : fileTitle(file);
  const submit = async () => {
    if (!ready) return;
    setBusy(true);
    const clean = text.trim().split(/\s+/).join(" ");
    try {
      const r = await remember(file, clean);
      keepMine("[" + todayIso() + "] " + clean);
      toast.success("Remembered for " + title + ".", { description: plural(r.count, "memory").replace("memorys", "memories") + " in " + r.file.path + "; runs read it from their next start." });
      setText("");
      refreshMemory();
      onDone();
    } catch (e) {
      showApiError(e, "The memory wasn't kept");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="grid gap-2 rounded-lg border border-dashed bg-card p-4" aria-label={"Remember something for " + title}>
      <Label htmlFor="memory-text">
        Remember something for {title}
      </Label>
      <Textarea
        ref={ref}
        id="memory-text"
        value={text}
        onChange={(e) => setText(e.target.value)}
        rows={3}
        disabled={busy}
        placeholder="One short fact agents should know next time: a convention the files do not state, a gotcha, how you like things done."
        onKeyDown={(e) => {
          if (e.key === "Escape") onDone();
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && ready) void submit();
        }}
      />
      <p className={cn("text-xs", room.over ? "text-status-failed" : "text-muted-foreground")}>
        {room.over ? plural(-room.left, "character") + " over the limit of " + data.limits.text + ". A memory is one fact, not an essay." : "Under " + data.limits.text + " characters, one line. Not for secrets, or what the repository's files already say."}
        <span className="pointer-coarse:hidden"> Press ⌘↩ to keep it.</span>
      </p>
      <div className="flex items-center gap-2">
        <Button type="button" size="sm" onClick={() => void submit()} disabled={!ready}>
          {busy ? <Loader2Icon className="animate-spin" aria-hidden /> : <CheckIcon aria-hidden />} Save
        </Button>
        <Button type="button" size="sm" variant="ghost" onClick={onDone} disabled={busy}>
          Cancel
        </Button>
      </div>
    </div>
  );
}
