// The search index, as the build makes it: for every page its title and its
// sections, and for every section the words and the code a reader may type
// to find it. Runs at build time only; the browser gets the result
// (/docs/search.json) and src/lib/search.ts to ask it. Pure.
import { STOP_WORDS, searchWords } from "./search.ts";
import type { SearchEntry, SearchSection } from "./search.ts";

export type SectionSource = { id: string; heading: string; text: string; code: readonly string[] };
export type PageSource = {
  path: string;
  title: string;
  group: string;
  /** Words a reader may type for this page that its title lacks. */
  keywords?: string;
  /** A record of a decision or a design: found after the pages that say what holds today. */
  record?: boolean;
  /** What stands before the first heading. */
  lead?: { text: string; code: readonly string[] };
  sections: readonly SectionSource[];
};

/**
 * Inline code worth finding by typing it: a `hive` command, a flag, a setting
 * (dotted or with an underscore), a file of Hive's, an environment variable.
 * Not a bare word, a placeholder, or a line of prose in code marks.
 */
export function isCodeTerm(code: string): boolean {
  if (code.length < 3 || code.length > 48) return false;
  if (/[<>{}|]|\.\.\./.test(code)) return false;
  return (
    /^hive(-hand)?( [a-z][\w-]*){1,3}( --[a-z][\w-]*)?$/.test(code) ||
    /^--[a-z][\w-]*$/.test(code) ||
    /^[a-z][a-z0-9]*([._][a-z0-9]+)+$/.test(code) ||
    /^[A-Z][A-Z0-9]*(_[A-Z0-9]+)+$/.test(code) ||
    /^(~\/)?\.hive(\/[\w.-]+)*\/?$/.test(code)
  );
}

/**
 * Every word of a text once, but the words of grammar, single letters, and
 * the words its heading and its page's title already show. Nothing else is
 * left out, however often it is said and however long the text: a word the
 * index lacks is a word the search says the docs do not have.
 *
 * The words a text says most come first, and among equals the one said
 * first: where a word stands is how much the text is about it, which is
 * what the search orders its finds by.
 */
function terms(text: string, shown: ReadonlySet<string>): string[] {
  const said = new Map<string, number>();
  for (const w of searchWords(text)) {
    if (w.length > 1 && !STOP_WORDS.has(w) && !shown.has(w)) said.set(w, (said.get(w) ?? 0) + 1);
  }
  return [...said].sort((a, b) => b[1] - a[1]).map(([w]) => w);
}

function codeTerms(code: readonly string[]): string[] {
  return [...new Set(code.filter(isCodeTerm))];
}

export function buildSearchIndex(pages: readonly PageSource[]): SearchEntry[] {
  return pages.map((p) => {
    const title = new Set(searchWords(p.title));
    const h = p.sections.map((s): SearchSection => {
      const t = terms(s.text, new Set([...title, ...searchWords(s.heading)])).join(" ");
      const c = codeTerms(s.code).join("\n");
      return c ? [s.id, s.heading, t, c] : t ? [s.id, s.heading, t] : [s.id, s.heading];
    });
    const entry: SearchEntry = { u: p.path, t: p.title, g: p.group, h };
    if (p.keywords) entry.k = p.keywords;
    if (p.record) entry.r = 1;
    if (p.lead) {
      const x = terms(p.lead.text, title).join(" ");
      const c = codeTerms(p.lead.code).join("\n");
      if (x) entry.x = x;
      if (c) entry.c = c;
    }
    return entry;
  });
}
