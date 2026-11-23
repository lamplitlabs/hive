// The search over the docs, as the browser runs it: titles and headings only,
// from the index the build writes. Pure and tiny on purpose: this is nearly
// all the code the search box ships.

/** One page in the index. Short keys keep the file small: u the path, t the title, g the sidebar group, h its headings. */
export type SearchEntry = { u: string; t: string; g: string; h: [id: string, text: string][] };

export type SearchHit = { href: string; title: string; where: string };

const words = (text: string) => text.toLowerCase().match(/[a-z0-9]+/g) ?? [];

/**
 * A page matches when its title holds every word of the query; a heading
 * matches by its own words, or by its words and its page's title together
 * ("harness hooks" finds Hooks in The harness). A word of the query matches a
 * word that starts with it, so "kill" does not find "skills". Titles come
 * before headings, and within each an exact match before one that starts with
 * the query before one that only holds it; ties keep the sidebar's order.
 */
export function searchDocs(entries: readonly SearchEntry[], query: string, max = 10): SearchHit[] {
  const asked = words(query);
  if (asked.length === 0) return [];
  const q = asked.join(" ");
  const has = (tokens: string[]) => asked.every((w) => tokens.some((t) => t.startsWith(w)));
  const rank = (tokens: string[], base: number) => {
    const text = tokens.join(" ");
    return base + (text === q ? 0 : text.startsWith(q) ? 1 : 2);
  };
  const found: { hit: SearchHit; rank: number }[] = [];
  for (const e of entries) {
    const title = words(e.t);
    if (has(title)) found.push({ hit: { href: e.u, title: e.t, where: e.g }, rank: rank(title, 0) });
    for (const [id, text] of e.h) {
      const heading = words(text);
      const hit = { href: `${e.u}#${id}`, title: text, where: e.t };
      if (has(heading)) found.push({ hit, rank: rank(heading, 3) });
      else if (asked.some((w) => heading.some((t) => t.startsWith(w))) && has([...title, ...heading])) found.push({ hit, rank: 6 });
    }
  }
  return found
    .sort((a, b) => a.rank - b.rank)
    .slice(0, max)
    .map((f) => f.hit);
}
