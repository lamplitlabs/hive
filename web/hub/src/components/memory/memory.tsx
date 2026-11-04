"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { BrainIcon, CheckIcon, FolderIcon, Loader2Icon, MessagesSquareIcon, PlusIcon, SearchIcon, Trash2Icon, UserRoundIcon, XIcon } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { ConfirmDialog, showApiError } from "@/components/pulse/confirm";
import { Disclosure, EmptyState, ErrorNotice, Notice, PageTitle, Panel, SkeletonPage } from "@/components/pulse/panel";
import { Mono, ProjectLink, When } from "@/components/pulse/who";
import { query, writes, type MemoryEntry, type MemoryFile, type MemoryPayload } from "@/lib/api";
import { plural, sizeText } from "@/lib/format";
import { addTargets, countWords, dayWords, fileHint, fileTitle, matchEntries, orderFiles, summaryLine, textRoom } from "@/lib/memory";
import { hrefs } from "@/lib/nav";
import { invalidate, useApi } from "@/lib/use-api";
import { cn } from "@/lib/utils";

// Memory: what the agents kept for later - a Markdown file of dated bullets
// per project under ~/.pulse/memory/, one for the owner that every project
// reads, one shared by chats without a project. The harness reads them into
// every run's prompt, so what is here shapes what agents do next. The owner
// reads them file by file, searches across them, adds a fact the way an
// agent's `memory remember` does, and forgets one that no longer holds.

export const refreshMemory = () => invalidate((p) => p === "memory" || p.startsWith("memory?") || p.startsWith("events"));

const KIND_ICON = { user: UserRoundIcon, conversations: MessagesSquareIcon, project: FolderIcon } as const;

function updateSearch(pathname: string, router: ReturnType<typeof useRouter>, next: Record<string, string>) {
  const clean = Object.fromEntries(Object.entries(next).filter(([, v]) => v));
  router.replace(pathname + query(clean), { scroll: false });
}

// One remembered fact: its day on the left, the text, and a quiet Forget
// that asks first (the file is shared with every run on the project).
function Entry({ file, entry }: { file: MemoryFile; entry: MemoryEntry }) {
  const [confirm, setConfirm] = useState(false);
  const day = dayWords(entry.day);
  return (
    <li className="group/entry flex min-w-0 items-start gap-3 px-4 py-2.5 sm:px-5">
      <span className="tnum mt-0.5 w-24 shrink-0 text-[12px] leading-5 text-muted-foreground" title={entry.day ? "Remembered on " + entry.day : "Written by hand, no date"}>
        {day || "—"}
      </span>
      <p className="min-w-0 flex-1 text-[14px] leading-relaxed text-foreground anywhere">{entry.text}</p>
      <Button type="button" size="icon-sm" variant="ghost" className="-my-1 shrink-0 text-muted-foreground opacity-70 hover:text-bad group-hover/entry:opacity-100 focus-visible:opacity-100" aria-label="Forget this memory" title="Forget this memory" onClick={() => setConfirm(true)}>
        <Trash2Icon className="size-4" aria-hidden />
      </Button>
      <ConfirmDialog
        open={confirm}
        onOpenChange={setConfirm}
        title="Forget this memory?"
        description={
          <>
            It is removed from <Mono>{file.path}</Mono>; later runs {file.kind === "user" ? "on every project" : file.kind === "conversations" ? "of sessions without a project" : "on " + fileTitle(file)} no longer read it.
          </>
        }
        action="Forget"
        destructive
        success="Forgotten."
        invalidates={(p) => p === "memory" || p.startsWith("memory?") || p.startsWith("events")}
        onConfirm={() => writes.forgetMemory({ key: file.key, scope: file.scope, text: entry.line })}
      >
        <blockquote className="rounded-xl border bg-muted/40 px-4 py-3 text-sm leading-relaxed text-foreground anywhere">{entry.text}</blockquote>
      </ConfirmDialog>
    </li>
  );
}

// One file: its title and hint, where it lives, and its entries. A search
// narrows the entries and hides files with no match.
function FileCard({ file, search, newestFirst, open, onToggle }: { file: MemoryFile; search: string; newestFirst: boolean; open: boolean; onToggle: (open: boolean) => void }) {
  const entries = matchEntries(file, search, newestFirst);
  const Icon = KIND_ICON[file.kind];
  if (search && !entries.length) return null;
  const title = file.kind === "project" && file.project ? <ProjectLink slug={file.project} /> : fileTitle(file);
  return (
    <Panel
      icon={Icon}
      title={
        <span className="inline-flex min-w-0 flex-wrap items-baseline gap-x-2">
          <span className="min-w-0">{title}</span>
          <span className="text-[13px] font-normal text-muted-foreground">{search ? plural(entries.length, "match") + " of " + countWords(file.entries.length) : countWords(file.entries.length)}</span>
        </span>
      }
      actions={
        file.exists ? (
          <span className="inline-flex flex-wrap items-center gap-x-2">
            <span>
              Changed <When d={file.updated_at} />
            </span>
            <span aria-hidden>·</span>
            <span>{sizeText(file.size)}</span>
          </span>
        ) : (
          <span>No file yet</span>
        )
      }
      dense
      footer={
        <span className="inline-flex min-w-0 flex-wrap items-center gap-x-2">
          <span>{fileHint(file)}</span>
          <Mono className="text-[12px]">{file.path}</Mono>
        </span>
      }
    >
      {entries.length ? (
        <>
          <ul className={cn("divide-y divide-border/70", !open && entries.length > 8 && "max-h-[22rem] overflow-hidden [mask-image:linear-gradient(to_bottom,black_80%,transparent)]")}>
            {entries.map((e) => (
              <Entry key={e.line} file={file} entry={e} />
            ))}
          </ul>
          {entries.length > 8 ? (
            <div className="border-t border-border/70 px-4 py-2 sm:px-5">
              <Button type="button" size="sm" variant="ghost" onClick={() => onToggle(!open)}>
                {open ? "Show fewer" : "Show all " + countWords(entries.length)}
              </Button>
            </div>
          ) : null}
        </>
      ) : (
        <p className="px-4 py-4 text-sm leading-relaxed text-muted-foreground sm:px-5">
          {file.kind === "user" ? "Nothing yet. A fact kept here reaches every project: how you like replies, what you never want done, which tools you prefer." : "Nothing yet. Agents keep a fact here when they learn something the project's files do not say."}
        </p>
      )}
    </Panel>
  );
}

// The Add box: a fact for one file, with the hub's limit shown as it is typed.
function AddBox({ data, initialKey, onDone }: { data: MemoryPayload; initialKey: string; onDone: () => void }) {
  const targets = useMemo(() => addTargets(data.files), [data.files]);
  const [key, setKey] = useState(() => (targets.some((t) => t.value === initialKey) ? initialKey : "user"));
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const room = textRoom(text, data.limits.text);
  const ready = !!text.trim() && !room.over && !busy;
  const file = data.files.find((f) => f.key === key);
  const submit = async () => {
    if (!ready || !file) return;
    setBusy(true);
    const id = toast.loading("Remembering…");
    try {
      const r = await writes.rememberMemory({ key: file.key, scope: file.scope, text: text.trim() });
      toast.success("Remembered for " + fileTitle(r.file).toLowerCase() + ".", { id, description: countWords(r.count) + " in " + r.file.path + "; runs read it from their next start." });
      setText("");
      refreshMemory();
      onDone();
    } catch (e) {
      toast.dismiss(id);
      showApiError(e, "The memory wasn't kept");
    } finally {
      setBusy(false);
    }
  };
  return (
    <Panel title="Remember something" icon={PlusIcon} className="border-dashed">
      <div className="grid gap-3">
        <div className="grid gap-3 sm:grid-cols-[minmax(0,18rem)_1fr]">
          <div className="grid content-start gap-1.5">
            <Label htmlFor="memory-key">For</Label>
            <Select value={key} onValueChange={setKey}>
              <SelectTrigger id="memory-key" aria-label="Which file to remember it in">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {targets.map((t) => (
                  <SelectItem key={t.value} value={t.value}>
                    {t.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="text-xs text-muted-foreground">{file ? fileHint(file) : ""}</p>
          </div>
          <div className="grid content-start gap-1.5">
            <Label htmlFor="memory-text">The fact</Label>
            <Textarea
              id="memory-text"
              value={text}
              onChange={(e) => setText(e.target.value)}
              rows={3}
              placeholder="One short fact agents should know next time: a convention the files do not state, a gotcha, how you like things done."
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && ready) void submit();
              }}
            />
            <p className={cn("text-xs", room.over ? "text-bad" : "text-muted-foreground")}>
              {room.over ? plural(-room.left, "character") + " over the limit of " + data.limits.text + ". A memory is one fact, not an essay." : "Under " + data.limits.text + " characters, one line. Not for secrets, or what the repository's files already say."}
              <span className="pointer-coarse:hidden"> Press ⌘↩ to keep it.</span>
            </p>
          </div>
        </div>
        <div className="flex flex-wrap items-center justify-end gap-2">
          <Button type="button" variant="outline" onClick={onDone} disabled={busy}>
            Close
          </Button>
          <Button type="button" onClick={() => void submit()} disabled={!ready}>
            {busy ? <Loader2Icon className="animate-spin" /> : <CheckIcon />}
            Remember
          </Button>
        </div>
      </div>
    </Panel>
  );
}

export function MemoryPageClient() {
  const sp = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const search = sp.get("q") || "";
  const key = sp.get("key") || "";
  const q = useApi<MemoryPayload>("memory", { refresh: 15000 });
  const [adding, setAdding] = useState(false);
  const [newestFirst, setNewestFirst] = useState(true);
  const [opened, setOpened] = useState<Record<string, boolean>>({});
  const files = useMemo(() => (q.data ? orderFiles(q.data.files, !search) : []), [q.data, search]);
  const shown = key ? files.filter((f) => f.key === key) : files;
  const setSearch = (v: string) => updateSearch(pathname, router, { q: v, key });
  return (
    <>
      <PageTitle
        icon={BrainIcon}
        sub="What agents kept for later, file by file. Every run reads these into its prompt, so a stale line here steers the next job wrong: forget what no longer holds."
        actions={
          <>
            <Button asChild variant="outline">
              <Link href={hrefs.settings("harness")} title="Settings, Harness: turn memory on or off">
                Memory setting
              </Link>
            </Button>
            <Button onClick={() => setAdding(true)} disabled={!q.data}>
              <PlusIcon /> Remember something
            </Button>
          </>
        }
        bar={
          q.data ? (
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <span className="relative min-w-0 flex-1 basis-64 sm:max-w-sm">
                <SearchIcon className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" aria-hidden />
                <Input type="search" aria-label="Search memories" placeholder="Search memories…" value={search} onChange={(e) => setSearch(e.target.value)} className="h-9 pl-8" />
              </span>
              <Button type="button" size="sm" variant="outline" onClick={() => setNewestFirst((v) => !v)} title="Which end of each file to read first">
                {newestFirst ? "Newest first" : "Oldest first"}
              </Button>
              {key ? (
                <Button type="button" size="sm" variant="ghost" onClick={() => updateSearch(pathname, router, { q: search })}>
                  <XIcon /> All files
                </Button>
              ) : null}
            </div>
          ) : null
        }
      >
        Memory
      </PageTitle>
      {q.error && !q.data ? (
        <ErrorNotice error={q.error} onRetry={() => void q.reload()}>
          {"The memory files didn't load (" + q.error + "). Check that PULSE is running, then try again."}
        </ErrorNotice>
      ) : null}
      {!q.data ? (
        q.error ? null : (
          <SkeletonPage rows={3} />
        )
      ) : (
        <div className="grid min-w-0 gap-5">
          <p className="max-w-3xl text-[15px] leading-relaxed text-foreground">{summaryLine(q.data)}</p>
          {!q.data.on ? (
            <Notice tone="warn">
              Memory is off (<Mono>harness.memory</Mono>), so runs neither read nor write these files. Turn it on under <Link href={hrefs.settings("harness")} className="underline underline-offset-4">Settings, Harness</Link>.
            </Notice>
          ) : null}
          {adding ? <AddBox data={q.data} initialKey={key} onDone={() => setAdding(false)} /> : null}
          {shown.length === 0 ? (
            search ? (
              <EmptyState
                title="No memory matches"
                icon={<SearchIcon className="size-[18px]" />}
                action={
                  <Button size="sm" variant="outline" onClick={() => setSearch("")}>
                    <XIcon /> Clear the search
                  </Button>
                }
              >
                Try other words; every word you type must appear in the entry.
              </EmptyState>
            ) : (
              <EmptyState title="No such file" icon={<BrainIcon className="size-[18px]" />}>
                Nothing is remembered under <Mono>{key}</Mono>.
              </EmptyState>
            )
          ) : (
            shown.map((f) => <FileCard key={f.key} file={f} search={search} newestFirst={newestFirst} open={!!opened[f.key]} onToggle={(o) => setOpened((cur) => ({ ...cur, [f.key]: o }))} />)
          )}
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
    </>
  );
}
