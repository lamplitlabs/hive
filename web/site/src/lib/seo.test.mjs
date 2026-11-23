// What a search engine reads: every page's title and description, the
// structured data, the sitemap and /llms.txt. Checked on the same lists the
// pages are built from.
import test from "node:test";
import assert from "node:assert/strict";

const site = await import("./site.ts");
const seo = await import("./seo.ts");
const docs = await import("./docs.ts");
const pages = await import("./pages.ts");
const { FAQS, plainAnswer } = await import("./faq.ts");

const built = docs.loadDocs();
const all = pages.allPages(built);

test("every page has a title of at most 60 characters, and no two pages share one", () => {
  assert.ok(all.length >= site.STATIC_PAGES.length + 20);
  for (const p of all) {
    assert.ok(p.title.length > 0 && p.title.length <= seo.TITLE_MAX, `${p.path}: title is ${p.title.length} characters: ${p.title}`);
    assert.equal(p.title, p.title.trim());
  }
  const seen = new Map();
  for (const p of all) {
    assert.ok(!seen.has(p.title), `${p.path} and ${seen.get(p.title)} share the title "${p.title}"`);
    seen.set(p.title, p.path);
  }
  assert.ok(site.NOT_FOUND.title.length <= seo.TITLE_MAX && !seen.has(site.NOT_FOUND.title));
});

test("every page has a description of 70 to 160 characters, and no two pages share one", () => {
  const seen = new Map();
  for (const p of all) {
    const n = p.description.length;
    assert.ok(n >= seo.DESCRIPTION_MIN && n <= seo.DESCRIPTION_MAX, `${p.path}: description is ${n} characters: ${p.description}`);
    assert.doesNotMatch(p.description, /\s{2,}|\n/, `${p.path}: one line`);
    assert.ok(!seen.has(p.description), `${p.path} and ${seen.get(p.description)} share a description`);
    seen.set(p.description, p.path);
  }
  for (const d of built.docs) assert.ok(d.description.length <= docs.DESCRIPTION_MAX, `${d.path}: a docs description stops at 155`);
  const n = site.NOT_FOUND.description.length;
  assert.ok(n >= seo.DESCRIPTION_MIN && n <= seo.DESCRIPTION_MAX && !seen.has(site.NOT_FOUND.description));
});

test("every page has its own address: unique, under the site, ending in a slash", () => {
  const paths = all.map((p) => p.path);
  assert.equal(new Set(paths).size, paths.length);
  for (const p of paths) assert.match(p, /^\/([a-z0-9._~%-]+\/)*$/, p);
  assert.deepEqual(
    site.STATIC_PAGES.map((p) => p.path),
    ["/", "/install/", "/harness/", "/rsi/", "/machines/", "/docs/"],
  );
});

test("a page's metadata carries the canonical address, Open Graph and the large Twitter card", () => {
  for (const p of site.STATIC_PAGES) {
    const m = seo.pageMetadata(p);
    assert.deepEqual(m.title, { absolute: p.title });
    assert.equal(m.description, p.description);
    assert.equal(m.alternates.canonical, p.path);
    assert.equal(m.openGraph.type, "website");
    assert.equal(m.openGraph.url, p.path);
    assert.equal(m.openGraph.siteName, "Hive");
    assert.deepEqual(m.openGraph.images, [seo.OG_IMAGE]);
    assert.equal(m.twitter.card, "summary_large_image");
    assert.equal(m.twitter.images[0].url, "/og.png");
    assert.ok(m.twitter.images[0].alt.length > 20);
  }
  assert.deepEqual([seo.OG_IMAGE.url, seo.OG_IMAGE.width, seo.OG_IMAGE.height], ["/og.png", 1200, 630]);
  assert.ok(seo.OG_IMAGE.alt.length > 40);
  const doc = built.docs[0];
  const m = seo.pageMetadata({ path: doc.path, title: doc.metaTitle, description: doc.description, shareTitle: doc.title }, "article");
  assert.equal(m.openGraph.type, "article");
  assert.equal(m.openGraph.title, doc.title);
  assert.equal(m.alternates.canonical, doc.path);
});

test("the Google verification token is used when the build has one, and only then", () => {
  assert.equal(seo.googleVerification({}), undefined);
  assert.equal(seo.googleVerification({ NEXT_PUBLIC_GOOGLE_SITE_VERIFICATION: "  " }), undefined);
  assert.equal(seo.googleVerification({ NEXT_PUBLIC_GOOGLE_SITE_VERIFICATION: " abc123 " }), "abc123");
});

/** JSON-LD as a browser would read it out of the script element. */
const roundTrip = (data) => {
  const text = seo.serializeJsonLd(data);
  assert.doesNotMatch(text, /</, "no < can close the script element early");
  return JSON.parse(text);
};
const nodes = (data) => (data["@graph"] ? data["@graph"] : [data]);
const byType = (data, type) => nodes(data).find((n) => n["@type"] === type);

test("JSON-LD on every page: the WebSite and the Organization", () => {
  const data = roundTrip(seo.siteJsonLd());
  assert.equal(data["@context"], "https://schema.org");
  const web = byType(data, "WebSite");
  assert.equal(web.name, "Hive");
  assert.equal(web.url, "https://hivefleet.dev/");
  assert.ok(web["@id"] && web.description.length > 0 && web.inLanguage === "en");
  const org = byType(data, "Organization");
  assert.equal(org.name, "Lamplit Labs");
  assert.match(org.url, /^https:\/\//);
  assert.equal(web.publisher["@id"], org["@id"]);
});

test("JSON-LD on the landing page: the SoftwareApplication, free, MIT, with its repository", () => {
  const data = roundTrip(seo.softwareJsonLd());
  const app = byType(data, "SoftwareApplication");
  assert.equal(app.name, "Hive");
  assert.equal(app.applicationCategory, "DeveloperApplication");
  assert.equal(app.operatingSystem, "macOS, Linux");
  assert.deepEqual(app.offers, { "@type": "Offer", price: "0", priceCurrency: "USD" });
  assert.match(app.license, /opensource\.org\/license\/mit$/);
  assert.ok(app.sameAs.includes("https://github.com/lamplitlabs/hive"));
  assert.equal(app.url, "https://hivefleet.dev/");
  assert.equal(app.installUrl, "https://hivefleet.dev/install/");
  assert.equal(app.softwareHelp.url, "https://hivefleet.dev/docs/");
  assert.doesNotMatch(app.operatingSystem, /windows/i);
  const code = byType(data, "SoftwareSourceCode");
  assert.equal(code.codeRepository, "https://github.com/lamplitlabs/hive");
  assert.equal(code.targetProduct["@id"], app["@id"]);
});

test("JSON-LD for the questions: six to eight, each with its answer in plain text", () => {
  assert.ok(FAQS.length >= 6 && FAQS.length <= 8, `${FAQS.length} questions`);
  const data = roundTrip(seo.faqJsonLd(FAQS.map((f) => ({ q: f.q, a: plainAnswer(f.a) }))));
  assert.equal(data["@type"], "FAQPage");
  assert.equal(data.mainEntity.length, FAQS.length);
  for (const [i, q] of data.mainEntity.entries()) {
    assert.equal(q["@type"], "Question");
    assert.equal(q.name, FAQS[i].q);
    assert.match(q.name, /\?$/);
    assert.equal(q.acceptedAnswer["@type"], "Answer");
    assert.ok(q.acceptedAnswer.text.length > 40);
    assert.doesNotMatch(q.acceptedAnswer.text, /`|<|\n/);
  }
  assert.equal(new Set(FAQS.map((f) => f.q)).size, FAQS.length);
});

test("JSON-LD on a docs page: the breadcrumbs and the TechArticle", () => {
  for (const doc of built.docs) {
    const crumbs = roundTrip(
      seo.breadcrumbJsonLd([
        { name: "Hive", path: "/" },
        { name: "Docs", path: "/docs/" },
        { name: doc.label, path: doc.path },
      ]),
    );
    assert.equal(crumbs["@type"], "BreadcrumbList");
    assert.deepEqual(
      crumbs.itemListElement.map((e) => [e["@type"], e.position, e.item]),
      [
        ["ListItem", 1, "https://hivefleet.dev/"],
        ["ListItem", 2, "https://hivefleet.dev/docs/"],
        ["ListItem", 3, `https://hivefleet.dev${doc.path}`],
      ],
    );
    assert.ok(crumbs.itemListElement.every((e) => e.name.length > 0));

    const article = roundTrip(seo.techArticleJsonLd({ path: doc.path, title: doc.title, description: doc.description }));
    assert.equal(article["@type"], "TechArticle");
    assert.ok(article.headline.length > 0 && article.headline.length <= 110, doc.path);
    assert.equal(article.description, doc.description);
    assert.equal(article.url, `https://hivefleet.dev${doc.path}`);
    assert.equal(article.mainEntityOfPage["@id"], article.url);
    assert.ok(article.author["@id"] && article.publisher["@id"] && article.isPartOf["@id"]);
    assert.equal(article.image, "https://hivefleet.dev/og.png");
  }
});

test("the sitemap lists every page that is exported, and nothing else", () => {
  const entries = pages.sitemapEntries(built);
  const expected = [...site.STATIC_PAGES.map((p) => p.path), ...built.docs.map((d) => d.path)].map((p) => `https://hivefleet.dev${p}`);
  assert.deepEqual(
    entries.map((e) => e.url),
    expected,
  );
  assert.equal(new Set(entries.map((e) => e.url)).size, entries.length);
  assert.equal(entries[0].priority, 1);
  for (const e of entries) {
    assert.ok(e.priority > 0 && e.priority <= 1);
    assert.doesNotMatch(e.url, /404|_not-found|search\.json/);
  }
  // A guide counts for more than a page of the reference, and less than a page the site wrote.
  const of = (path) => entries.find((e) => e.url === `https://hivefleet.dev${path}`);
  assert.deepEqual([of("/install/").priority, of("/docs/how-to/").priority, of("/docs/how-to/console-tour/").priority, of("/docs/console/").priority], [0.8, 0.7, 0.7, 0.6]);
  assert.equal(of("/docs/how-to/console-tour/").changeFrequency, "weekly");
});

test("llms.txt says what Hive is and lists every page once, each with one line", () => {
  const text = pages.llmsText(built);
  assert.ok(text.startsWith("# Hive\n\n> Hive is an open-source harness for coding agents."));
  assert.match(text, /curl -fsSL https:\/\/hivefleet\.dev\/install\.sh \| sh/);
  for (const p of all) {
    const url = `https://hivefleet.dev${p.path}`;
    const lines = text.split("\n").filter((l) => l.includes(`](${url}): `));
    assert.equal(lines.length, 1, `${url} is listed once`);
    assert.match(lines[0], /^- \[[^\]]+\]\([^)]+\): \S.{20,}$/);
  }
  assert.doesNotMatch(text, /\t|\r/);
  // The guides have a section of their own, before the reference.
  const section = (name) => text.slice(text.indexOf(`\n## ${name}\n`), text.indexOf("\n## ", text.indexOf(`\n## ${name}\n`) + 4));
  assert.ok(text.indexOf("\n## Guides\n") > 0 && text.indexOf("\n## Guides\n") < text.indexOf("\n## Documentation\n"));
  const guides = built.docs.filter((d) => d.path.startsWith("/docs/how-to/"));
  assert.ok(guides.length >= 10);
  for (const d of built.docs) {
    const inGuides = section("Guides").includes(`](https://hivefleet.dev${d.path}): `);
    assert.equal(inGuides, guides.includes(d), `${d.path} is listed under ${guides.includes(d) ? "Guides" : "Documentation"}`);
  }
});

test("the site's own words keep to the rules: no em dash, no hype, nothing of the hosted product", () => {
  const words = JSON.stringify([site.SITE, site.PAGES, site.NOT_FOUND, site.INSTALL, FAQS, seo.OG_IMAGE.alt, seo.siteJsonLd(), seo.softwareJsonLd()]);
  assert.doesNotMatch(words, /\u2014/, "no em dash");
  assert.doesNotMatch(words, /revolutionary|seamless|powerful|effortless|next-generation/i);
  assert.doesNotMatch(words, /clerk|sign[ -]?in|sign[ -]?up|\baccounts?\b|team brain/i);
  // RSI is spelled out once where it is used.
  assert.match(site.PAGES.rsi.title, /RSI/);
});
