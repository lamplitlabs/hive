// The search index, as the build makes it: for every page its title and its
// sections, and for every section the words and the code a reader may type
// to find it. Runs at build time only; the browser gets the result
// (/docs/search.json) and src/lib/search.ts to ask it. Pure.
import { searchWords } from "./search.ts";
import type { SearchEntry, SearchSection } from "./search.ts";

export type SectionSource = { id: string; heading: string; text: string; code: readonly string[] };
export type PageSource = {
  path: string;
  title: string;
  group: string;
  /** Words a reader may type for this page that its title lacks. */
  keywords?: string;
  /** What stands before the first heading. */
  lead?: { text: string; code: readonly string[] };
  sections: readonly SectionSource[];
};

/** Words that say nothing about where they stand. */
const STOP = new Set(
  (
    "a about above after again against all also am an and any are as at be because been before being below between both but by can cannot could did do does doing done down during each either else ever every few for from further get gets had has have having he her here hers him his how however i if in into is it its itself just let lets like may me might more most much must my no nor not now of off on once one ones only or other others our out over own per same she should since so some such than that the their them then there these they this those through thus to too two under until up upon us use used uses using very via was we were what when where whether which while who whom whose why will with within without would yes yet you your yours"
  ).split(" "),
);

/** A word is kept when it is not everywhere: found in at most this share of all sections. */
const COMMON_SHARE = 0.12;
/** At most this many words and this much code per section: the rest of a very long section adds size, not finds. */
const MAX_TERMS = 160;
const MAX_CODE = 24;

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

function terms(text: string, shown: ReadonlySet<string>, common: ReadonlySet<string>): string[] {
  const out = new Set<string>();
  for (const w of searchWords(text)) {
    if (out.size >= MAX_TERMS) break;
    if (w.length < 3 || /^\d+$/.test(w) || STOP.has(w) || shown.has(w) || common.has(w)) continue;
    out.add(w);
  }
  return [...out];
}

function codeTerms(code: readonly string[]): string[] {
  return [...new Set(code.filter(isCodeTerm))].slice(0, MAX_CODE);
}

export function buildSearchIndex(pages: readonly PageSource[]): SearchEntry[] {
  // How many sections each word stands in, to leave out the ones that stand everywhere.
  const inSections = new Map<string, number>();
  let count = 0;
  const seen = (text: string) => {
    count += 1;
    for (const w of new Set(searchWords(text))) inSections.set(w, (inSections.get(w) ?? 0) + 1);
  };
  for (const p of pages) {
    if (p.lead) seen(p.lead.text);
    for (const s of p.sections) seen(s.text);
  }
  const limit = Math.max(8, Math.floor(count * COMMON_SHARE));
  const common = new Set([...inSections].filter(([, n]) => n > limit).map(([w]) => w));

  return pages.map((p) => {
    const title = new Set(searchWords(p.title));
    const h = p.sections.map((s): SearchSection => {
      const t = terms(s.text, new Set([...title, ...searchWords(s.heading)]), common).join(" ");
      const c = codeTerms(s.code).join("\n");
      return c ? [s.id, s.heading, t, c] : t ? [s.id, s.heading, t] : [s.id, s.heading];
    });
    const entry: SearchEntry = { u: p.path, t: p.title, g: p.group, h };
    if (p.keywords) entry.k = p.keywords;
    if (p.lead) {
      const x = terms(p.lead.text, title, common).join(" ");
      const c = codeTerms(p.lead.code).join("\n");
      if (x) entry.x = x;
      if (c) entry.c = c;
    }
    return entry;
  });
}
