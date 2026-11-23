// The search over the site, as the browser runs it, from the index the build
// writes: titles and headings first, then the commands and settings a
// section names, then the words of its text. Pure and small on purpose: this
// is nearly all the code the search box ships.

/**
 * One section of a page in the index: its id, its heading, the words of its
 * text that say something the heading does not (one string, space between),
 * and the code it names as written (commands, settings, flags; a newline
 * between). The last two are left out when empty.
 */
export type SearchSection = [id: string, text: string, terms?: string, code?: string];

/**
 * One page in the index. Short keys keep the file small: u the path, t the
 * title, g the group it is listed under, h its sections; k the words a
 * reader may type for this page that its title lacks ("quickstart"), x and c
 * the words and the code of what stands before its first heading.
 */
export type SearchEntry = { u: string; t: string; g: string; h: SearchSection[]; k?: string; x?: string; c?: string };

/** One result. `why` is the command or setting that was asked for, when that is how the place was found. */
export type SearchHit = { href: string; title: string; where: string; why?: string };

/** A query of one word shorter than this is looked for as a whole word. */
export const MIN_PREFIX = 3;

/** The words of a text as the search compares them: lower case, letters and digits only. */
export const searchWords = (text: string): string[] => text.toLowerCase().match(/[a-z0-9]+/g) ?? [];

type Place = { words: string[]; terms: string[]; code: { as: string; flat: string }[] };
type Prepared = { entry: SearchEntry; title: string[]; keys: string[]; lead: Place; sections: Place[] };

const flat = (code: string) => searchWords(code).join(" ");
const place = (text: string, terms?: string, code?: string): Place => ({
  words: searchWords(text),
  terms: terms ? terms.split(" ") : [],
  code: code ? code.split("\n").map((as) => ({ as, flat: flat(as) })) : [],
});

// The index is read once per page load and asked on every key: its words are cut once.
const prepared = new WeakMap<readonly SearchEntry[], Prepared[]>();
function prepare(entries: readonly SearchEntry[]): Prepared[] {
  let p = prepared.get(entries);
  if (!p) {
    p = entries.map((entry) => ({
      entry,
      title: searchWords(entry.t),
      keys: entry.k ? searchWords(entry.k) : [],
      lead: place("", entry.x, entry.c),
      sections: entry.h.map(([, text, terms, code]) => place(text, terms, code)),
    }));
    prepared.set(entries, p);
  }
  return p;
}

/**
 * What is found, best first; ties keep the order of the index (the sidebar's).
 *
 *  0-2  a page whose title holds every word of the query;
 *  3-5  a heading that holds every word;
 *  6    a heading that holds some and its page's title the rest ("harness
 *       hooks" finds Hooks in The harness);
 *  7    a page by the words kept for it ("quickstart");
 *  8-9  a section that names the command or the setting asked for
 *       (`evolve.loop`, `hive pair`): as a whole first, then as its start;
 *  10   a section whose text holds the words;
 *  11   a page whose opening holds them.
 *
 * A word of the query matches a word that starts with it, so "kill" does not
 * find "skills". One or two letters on their own are too little to start a
 * word with: "pr" finds the word "pr" and not every "project". Within titles
 * and headings an exact match comes before one that starts with the query,
 * before one that only holds it.
 */
export function searchDocs(entries: readonly SearchEntry[], query: string, max = 10): SearchHit[] {
  const asked = searchWords(query);
  if (asked.length === 0) return [];
  const q = asked.join(" ");
  const whole = asked.length === 1 && q.length < MIN_PREFIX;
  const starts = (tokens: readonly string[], w: string) => (whole ? tokens.includes(w) : tokens.some((t) => t.startsWith(w)));
  const has = (...lists: (readonly string[])[]) => asked.every((w) => lists.some((tokens) => starts(tokens, w)));
  const some = (...lists: (readonly string[])[]) => asked.some((w) => lists.some((tokens) => starts(tokens, w)));
  const rank = (tokens: string[], base: number) => {
    const text = tokens.join(" ");
    return base + (text === q ? 0 : text.startsWith(q) ? 1 : 2);
  };
  // The code a place names that the query spells: whole, or from its start at a word's edge.
  const named = (p: Place): { why: string; rank: number } | null => {
    let best: { why: string; rank: number } | null = null;
    for (const c of p.code) {
      const r = c.flat === q ? 8 : c.flat.startsWith(`${q} `) || (asked.length > 1 && c.flat.startsWith(q)) ? 9 : 0;
      if (r && (!best || r < best.rank)) best = { why: c.as, rank: r };
    }
    return best;
  };

  const found: { hit: SearchHit; rank: number }[] = [];
  for (const p of prepare(entries)) {
    const e = p.entry;
    const page = { href: e.u, title: e.t, where: e.g };
    const leadCode = named(p.lead);
    if (has(p.title)) found.push({ hit: page, rank: rank(p.title, 0) });
    else if (p.keys.length > 0 && some(p.keys) && has(p.title, p.keys)) found.push({ hit: page, rank: 7 });
    else if (leadCode) found.push({ hit: { ...page, why: leadCode.why }, rank: leadCode.rank });
    else if (some(p.lead.terms) && has(p.title, p.lead.terms)) found.push({ hit: page, rank: 11 });

    e.h.forEach(([id, text], i) => {
      const s = p.sections[i];
      const hit = { href: `${e.u}#${id}`, title: text, where: e.t };
      if (has(s.words)) return void found.push({ hit, rank: rank(s.words, 3) });
      if (some(s.words) && has(p.title, s.words)) return void found.push({ hit, rank: 6 });
      const code = named(s);
      if (code) return void found.push({ hit: { ...hit, why: code.why }, rank: code.rank });
      if (some(s.terms) && has(p.title, s.words, s.terms)) found.push({ hit, rank: 10 });
    });
  }
  return found
    .sort((a, b) => a.rank - b.rank)
    .slice(0, max)
    .map((f) => f.hit);
}
