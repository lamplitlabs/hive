// The search over the site, as the browser runs it, from the index the build
// writes: titles and headings first, then the commands and settings a
// section names, then the words of its text. Pure and small on purpose: this
// is nearly all the code the search box ships.

/**
 * One section of a page in the index: its id, its heading, the words of its
 * text that its heading and its page's title do not show (one string, space
 * between, the ones it says most first), and the code it names as written
 * (commands, settings, flags; a newline between). The last two are left out
 * when empty.
 */
export type SearchSection = [id: string, text: string, terms?: string, code?: string];

/**
 * One page in the index. Short keys keep the file small: u the path, t the
 * title, g the group it is listed under, h its sections; k the words a
 * reader may type for this page that its title lacks ("quickstart"), x and c
 * the words and the code of what stands before its first heading; r marks a
 * record of why something was decided or designed, which is found after the
 * pages that say what holds today.
 */
export type SearchEntry = { u: string; t: string; g: string; h: SearchSection[]; k?: string; x?: string; c?: string; r?: 1 };

/** One result. `why` is the command or setting that was asked for, when that is how the place was found. */
export type SearchHit = { href: string; title: string; where: string; why?: string };

/** A query of one word shorter than this is looked for as a whole word. */
export const MIN_PREFIX = 3;

/** What a word costs a section that only says it in its text, before its place there is counted: a heading costs 0, the page's title 1. */
const IN_TEXT = 4;

/** The kind of find each rank is, best first: by a title, by a heading, by the words kept for a page, by code, by text. */
const KIND = [0, 0, 0, 1, 1, 1, 1, 2, 3, 3, 4, 4];

/** The words of a text as the search compares them: lower case, letters and digits only. */
export const searchWords = (text: string): string[] => text.toLowerCase().match(/[a-z0-9]+/g) ?? [];

/**
 * The words of grammar: articles, pronouns, prepositions, conjunctions and
 * helping verbs. They stand in nearly every section and name nothing, so the
 * index leaves them out of a section's text and a query is not held to them.
 * A word that can be what a reader looks for ("done", "yes", "use", "get")
 * is not one of them.
 */
export const STOP_WORDS: ReadonlySet<string> = new Set(
  (
    "a about above after again against all also am an and any are as at be because been before being below between both but by can cannot could did do does doing down during each either else ever every few for from further had has have having he her here hers him his how however i if in into is it its itself just may me might more most much must my no nor not now of off on once one ones only or other others our out over own per same she should since so some such than that the their them then there these they this those through thus to too two under until up upon us very via was we were what when where whether which while who whom whose why will with within without would yet you your yours"
  ).split(" "),
);

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
 *  10   a section whose text holds the words: the one that is most about
 *       them first (a word in its heading counts most, then one in its
 *       page's title, then the words its text says most often);
 *  11   a page whose opening holds them.
 *
 * A word of the query matches a word that starts with it, so "kill" does not
 * find "skills". One or two letters on their own are too little to start a
 * word with: "pr" finds the word "pr" and not every "project". Within titles
 * and headings an exact match comes before one that starts with the query,
 * before one that only holds it.
 *
 * A place is held to the words of the query that say what is looked for.
 * The words of grammar and single letters do not count beside them: "how do
 * I add a machine" looks for "add" and "machine". A query made of nothing
 * else ("for you") is looked for as it is typed.
 *
 * Among finds of one kind (by a title, by a heading, by the words kept for
 * a page, by code, by text):
 *
 *  - a record of a decision or a design comes last: it says why something
 *    was chosen, and a reader looks first for what holds;
 *  - then the place whose title or heading also holds the words of grammar
 *    that were typed comes first: "for you page" is nearer to the heading
 *    "Read the For you page" than to the heading "Page".
 */
export function searchDocs(entries: readonly SearchEntry[], query: string, max = 10): SearchHit[] {
  const asked = searchWords(query);
  if (asked.length === 0) return [];
  const telling = asked.filter((w) => w.length > 1 && !STOP_WORDS.has(w));
  const held = telling.length > 0 ? telling : asked;
  // The query as it is typed and as the words it is held to: a heading or a command that is either, word for word, is what was asked for.
  const forms = [...new Set([asked.join(" "), held.join(" ")])];
  // The words a place is not held to: how many of them its title or heading lacks says how near it is to what was typed.
  const beside = [...new Set(asked.filter((w) => !held.includes(w)))];
  const lacks = (...lists: (readonly string[])[]) => beside.filter((w) => !lists.some((tokens) => tokens.includes(w))).length;
  // Only a query that is one short word: the second letter of a word in a longer one is a word still being typed.
  const whole = asked.length === 1 && asked[0].length < MIN_PREFIX;
  const starts = (tokens: readonly string[], w: string) => (whole ? tokens.includes(w) : tokens.some((t) => t.startsWith(w)));
  const has = (...lists: (readonly string[])[]) => held.every((w) => lists.some((tokens) => starts(tokens, w)));
  const some = (...lists: (readonly string[])[]) => held.some((w) => lists.some((tokens) => starts(tokens, w)));
  // How far a section found by its text is from being about the words. A word in its heading is as near as can be
  // and one in its page's title next. Any other counts by where it stands among the words of the text, where the
  // index puts the most said first: a step further each time the place doubles, so that words about as prominent
  // count the same and the order of the index (the sidebar's) decides between their sections.
  const far = (title: readonly string[], s: Place) =>
    held.reduce((sum, w) => {
      if (starts(s.words, w)) return sum;
      if (starts(title, w)) return sum + 1;
      const at = s.terms.findIndex((t) => (whole ? t === w : t.startsWith(w)));
      return sum + IN_TEXT + Math.floor(Math.log2(at + 1));
    }, 0);
  const rank = (tokens: string[], base: number) => {
    const text = tokens.join(" ");
    return base + (forms.includes(text) ? 0 : forms.some((f) => text.startsWith(f)) ? 1 : 2);
  };
  // The code a place names that the query spells: whole, or from its start at a word's edge.
  const named = (p: Place): { why: string; rank: number } | null => {
    let best: { why: string; rank: number } | null = null;
    for (const c of p.code) {
      const r = forms.includes(c.flat) ? 8 : forms.some((f) => c.flat.startsWith(`${f} `) || (f.includes(" ") && c.flat.startsWith(f))) ? 9 : 0;
      if (r && (!best || r < best.rank)) best = { why: c.as, rank: r };
    }
    return best;
  };

  const found: { hit: SearchHit; rank: number; far: number; late: number; lacks: number }[] = [];
  for (const p of prepare(entries)) {
    const e = p.entry;
    const of = { far: 0, late: e.r ? 1 : 0, lacks: lacks(p.title) };
    const page = { href: e.u, title: e.t, where: e.g };
    const leadCode = named(p.lead);
    if (has(p.title)) found.push({ hit: page, rank: rank(p.title, 0), ...of });
    else if (p.keys.length > 0 && some(p.keys) && has(p.title, p.keys)) found.push({ hit: page, rank: 7, ...of });
    else if (leadCode) found.push({ hit: { ...page, why: leadCode.why }, rank: leadCode.rank, ...of });
    else if (some(p.lead.terms) && has(p.title, p.lead.terms)) found.push({ hit: page, rank: 11, ...of });

    e.h.forEach(([id, text], i) => {
      const s = p.sections[i];
      const hit = { href: `${e.u}#${id}`, title: text, where: e.t };
      const at = { ...of, lacks: lacks(s.words, p.title) };
      if (has(s.words)) return void found.push({ hit, rank: rank(s.words, 3), ...at });
      if (some(s.words) && has(p.title, s.words)) return void found.push({ hit, rank: 6, ...at });
      const code = named(s);
      if (code) return void found.push({ hit: { ...hit, why: code.why }, rank: code.rank, ...at });
      if (some(s.terms) && has(p.title, s.words, s.terms)) found.push({ hit, rank: 10, ...at, far: far(p.title, s) });
    });
  }
  return found
    .sort((a, b) => KIND[a.rank] - KIND[b.rank] || a.late - b.late || a.lacks - b.lacks || a.rank - b.rank || a.far - b.far)
    .slice(0, max)
    .map((f) => f.hit);
}
