// The Memory page's pure parts (spec "Memory"): which file the URL names,
// the files in the order the left pane lists them, the icon an entry's
// source gets, and the entries of the chosen file. No React, only type
// imports and the pure lib modules, so node --test covers it.

import type { MemoryEntry, MemoryFile } from "../../lib/api";

// The file's name as a person reads it (lib/memory.ts fileTitle, repeated
// here because node resolves no extensionless import between .ts files).
function titleOf(f: Pick<MemoryFile, "key" | "kind" | "project">): string {
  if (f.kind === "user") return "You";
  if (f.kind === "conversations") return "Sessions without a project";
  return f.project || f.key;
}

// ----- which file ---------------------------------------------------------

// The URL names the chosen file as ?file=user | ?file=none (the sessions
// without a project) | ?file=<slug>. The older ?key=<key> still resolves,
// so links made before the redesign keep working.
export const FILE_USER = "user";
export const FILE_NONE = "none";

export function fileParam(f: Pick<MemoryFile, "key" | "kind">): string {
  if (f.kind === "user") return FILE_USER;
  if (f.kind === "conversations") return FILE_NONE;
  return f.key;
}

// The file the URL names, the user's when it names none (or one that is
// not there). `legacyKey` is the older ?key= parameter.
export function chosenFile(files: MemoryFile[], param: string | null | undefined, legacyKey?: string | null): MemoryFile | null {
  if (!files.length) return null;
  const want = param || legacyKey || FILE_USER;
  const byParam = files.find((f) => fileParam(f) === want);
  if (byParam) return byParam;
  const byKey = files.find((f) => f.key === want);
  if (byKey) return byKey;
  return files.find((f) => f.kind === "user") ?? files[0];
}

// ----- the left pane --------------------------------------------------------

export interface FileChoice {
  file: MemoryFile;
  param: string;
  title: string;
  count: number;
  // What the row's icon stands for: the owner, the shared sessions file, a
  // project (its avatar), or a file whose project is gone.
  icon: "user" | "sessions" | "project" | "orphan";
}

// The files as the left pane lists them: "You", "Sessions without a
// project", then every registered project (the ones with entries first),
// then files whose project is gone. Empty files stay, so the owner can add
// to them.
export function fileChoices(files: MemoryFile[]): FileChoice[] {
  const rank = (f: MemoryFile) => (f.kind === "user" ? 0 : f.kind === "conversations" ? 1 : f.project ? 2 : 3);
  const ordered = [...files].sort((a, b) => {
    const r = rank(a) - rank(b);
    if (r) return r;
    if (rank(a) === 2) {
      const e = (b.entries.length > 0 ? 1 : 0) - (a.entries.length > 0 ? 1 : 0);
      if (e) return e;
    }
    return titleOf(a).localeCompare(titleOf(b));
  });
  return ordered.map((f) => ({
    file: f,
    param: fileParam(f),
    title: titleOf(f),
    count: f.entries.length,
    icon: f.kind === "user" ? "user" : f.kind === "conversations" ? "sessions" : f.project ? "project" : "orphan",
  }));
}

// ----- the entries ----------------------------------------------------------

export type EntrySource = "you" | "agent" | "session";

// Who wrote an entry. The file carries no author: a bullet is "- [day]
// text" whoever kept it. So: a line the owner kept from this page (the hub
// remembers them for the session) or wrote by hand (no day in front) is
// theirs; a line in the sessions file came from a session; the rest an
// agent's run kept.
export function sourceOf(entry: Pick<MemoryEntry, "day" | "line">, file: Pick<MemoryFile, "kind">, mine?: ReadonlySet<string>): EntrySource {
  if (mine?.has(entry.line) || !entry.day) return "you";
  if (file.kind === "conversations") return "session";
  return "agent";
}

export function sourceWords(s: EntrySource): string {
  return s === "you" ? "Kept by you" : s === "session" ? "Kept by a session" : "Kept by an agent";
}

// The chosen file's entries as the right pane shows them: newest first,
// narrowed by the search (every word must appear).
export function shownEntries(file: Pick<MemoryFile, "entries">, search: string): MemoryEntry[] {
  const words = search.toLowerCase().split(/\s+/).filter(Boolean);
  const hit = words.length ? file.entries.filter((e) => words.every((w) => (e.day + " " + e.text).toLowerCase().includes(w))) : file.entries;
  return [...hit].reverse();
}

// The one line under the file's title: how many entries, and when the file
// last changed when there is one.
export function fileNote(f: Pick<MemoryFile, "exists" | "entries" | "path">): string {
  const n = f.entries.length;
  const count = n === 0 ? "Nothing yet" : n === 1 ? "1 memory" : n.toLocaleString("en-US") + " memories";
  return f.exists ? count + " in " + f.path : count + "; no file yet";
}

// The words of the empty state for a file with nothing in it.
export function emptyWords(f: Pick<MemoryFile, "kind">): string {
  if (f.kind === "user") return "Nothing yet. A fact kept here reaches every project: how you like replies, what you never want done, which tools you prefer.";
  if (f.kind === "conversations") return "Nothing yet. Sessions that are not attached to a project keep what they learn here.";
  return "Nothing yet. Agents keep a fact here when they learn something the project's files do not say.";
}

// What forgetting and then undoing does to an entry: the text comes back
// under today's day, since the file keeps no other record. Pure so the test
// states it; the component shows it in the toast.
export function undoNote(entry: Pick<MemoryEntry, "day">, today: string): string {
  return entry.day && entry.day !== today ? "Back in the file, dated today instead of " + entry.day + "." : "Back in the file.";
}
