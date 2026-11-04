// Memory: the pure parts of the Memory page - what the hub returns, how the
// files are named and grouped, how the search narrows the entries - so the
// node tests cover them without React.

import type { MemoryFile, MemoryPayload } from "@/lib/api";

// The file's name as a person reads it: "You, in every project" for the
// user's file, "Conversations" for the shared one, the project's slug else.
export function fileTitle(f: Pick<MemoryFile, "key" | "kind" | "project">): string {
  if (f.kind === "user") return "You, in every project";
  if (f.kind === "conversations") return "Sessions without a project";
  return f.project || f.key;
}

// What the file is for, in one line under its title.
export function fileHint(f: Pick<MemoryFile, "kind" | "project">): string {
  switch (f.kind) {
    case "user":
      return "Read by every run on every project: how you like things done.";
    case "conversations":
      return "Shared by sessions that are not attached to a project.";
    default:
      return f.project ? "Read by every run and session on this project." : "No project goes by this key any more; the file is still on disk.";
  }
}

// How many entries, as words: "nothing yet", "1 memory", "12 memories".
export function countWords(n: number): string {
  if (n === 0) return "nothing yet";
  return n === 1 ? "1 memory" : n.toLocaleString("en-US") + " memories";
}

// The files in the order the page shows them: the user's, the shared one,
// the registered projects (ones with entries first, then alphabetical),
// then files whose project is gone. `onlyWithEntries` drops the empty ones
// (the choices stay in the Add box).
export function orderFiles(files: MemoryFile[], onlyWithEntries: boolean): MemoryFile[] {
  const rank = (f: MemoryFile) => (f.kind === "user" ? 0 : f.kind === "conversations" ? 1 : f.project ? 2 : 3);
  const out = files.filter((f) => !onlyWithEntries || f.entries.length > 0 || f.kind === "user");
  return [...out].sort((a, b) => {
    const r = rank(a) - rank(b);
    if (r) return r;
    if (rank(a) === 2) {
      const e = (b.entries.length > 0 ? 1 : 0) - (a.entries.length > 0 ? 1 : 0);
      if (e) return e;
    }
    return fileTitle(a).localeCompare(fileTitle(b));
  });
}

// The entries of one file that match the search (case-insensitive, every
// word must appear), newest first when asked. Empty search keeps them all.
export function matchEntries(f: MemoryFile, search: string, newestFirst: boolean): MemoryFile["entries"] {
  const words = search.toLowerCase().split(/\s+/).filter(Boolean);
  const hit = words.length ? f.entries.filter((e) => words.every((w) => (e.day + " " + e.text).toLowerCase().includes(w))) : f.entries;
  return newestFirst ? [...hit].reverse() : hit;
}

// The one sentence at the top: how much is remembered and where.
export function summaryLine(p: Pick<MemoryPayload, "on" | "total" | "files" | "dir">): string {
  const withEntries = p.files.filter((f) => f.entries.length > 0).length;
  const where = " in " + p.dir + ".";
  if (!p.on) return "Memory is off: runs do not read or write these files (Settings, Harness). " + (p.total ? countWords(p.total) + " still on disk" + where : "Nothing is stored" + where);
  if (!p.total) return "Nothing remembered yet. Agents keep a fact here when they learn something the files do not say; it is read into every later run's prompt. The files live" + where;
  const files = withEntries === 1 ? "1 file" : withEntries + " files";
  return countWords(p.total) + " across " + files + ", read into every run's prompt" + where;
}

// The Add box's choices: the user's file first, then each file by title;
// `value` is the key the hub expects.
export function addTargets(files: MemoryFile[]): { value: string; label: string }[] {
  return orderFiles(files, false)
    .filter((f) => f.kind !== "project" || f.project || f.exists)
    .map((f) => ({ value: f.key, label: fileTitle(f) }));
}

// The remaining room for one memory, as the hub enforces it.
export function textRoom(text: string, limit: number): { left: number; over: boolean } {
  const n = Array.from(text.trim().split(/\s+/).join(" ")).length;
  return { left: limit - n, over: n > limit };
}

// A day as the entry carries it ("2026-01-02") or nothing for a hand-written line.
export function dayWords(day: string): string {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(day)) return "";
  const d = new Date(day + "T00:00:00Z");
  return d.toLocaleDateString("en-US", { year: "numeric", month: "short", day: "numeric", timeZone: "UTC" });
}
