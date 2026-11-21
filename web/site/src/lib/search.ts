// The search over the docs, as the browser runs it: titles and headings only,
// from the index the build writes. Pure and tiny on purpose: this is nearly
// all the code the search box ships.

/** One page in the index. Short keys keep the file small: u the path, t the title, g the sidebar group, h its headings. */
export type SearchEntry = { u: string; t: string; g: string; h: [id: string, text: string][] };

export type SearchHit = { href: string; title: string; where: string };

/**
 * A page matches when its title holds every word of the query; a heading
 * matches by its own words, or by its words and its page's title together
 * ("harness hooks" finds Hooks in The harness). Titles come before headings,
 * and within each an exact match before one that starts with the query before
 * one that only holds it; ties keep the sidebar's order.
 */
export function searchDocs(entries: readonly SearchEntry[], query: string, max = 10): SearchHit[] {
  const q = query.toLowerCase().trim().replace(/\s+/g, " ");
  if (!q) return [];
  const words = q.split(" ");
  const all = (text: string) => words.every((w) => text.includes(w));
  const rank = (text: string, base: number) => base + (text === q ? 0 : text.startsWith(q) ? 1 : 2);
  const found: { hit: SearchHit; rank: number }[] = [];
  for (const e of entries) {
    const title = e.t.toLowerCase();
    if (all(title)) found.push({ hit: { href: e.u, title: e.t, where: e.g }, rank: rank(title, 0) });
    for (const [id, text] of e.h) {
      const heading = text.toLowerCase();
      const hit = { href: `${e.u}#${id}`, title: text, where: e.t };
      if (all(heading)) found.push({ hit, rank: rank(heading, 3) });
      else if (words.some((w) => heading.includes(w)) && all(`${title} ${heading}`)) found.push({ hit, rank: 6 });
    }
  }
  return found
    .sort((a, b) => a.rank - b.rank)
    .slice(0, max)
    .map((f) => f.hit);
}
