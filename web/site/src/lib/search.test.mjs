// The search the browser runs: titles and headings first, then the commands
// and settings a section names, then the words of its text.
import test from "node:test";
import assert from "node:assert/strict";

const { STOP_WORDS, searchDocs, searchWords } = await import("./search.ts");
const { isCodeTerm } = await import("./search-index.ts");
const docs = await import("./docs.ts");

const index = [
  { u: "/docs/harness/", t: "The harness (how an agent runs)", g: "The agents", h: [["the-tools", "The tools"], ["hooks", "Hooks"], ["skills-as-tools", "Skills as tools"]] },
  { u: "/docs/board/", t: "The board (for the agents)", g: "The agents", h: [["events-hooks-and-channels", "Events, hooks and channels"], ["shared-skills-hive-skill", "Shared skills: hive skill"]] },
  { u: "/docs/machines/", t: "Machines and paired hands", g: "Machines and running", h: [["pairing-hands", "Pairing hands"], ["the-hand", "The hand"]] },
];

test("a page whose title holds every word comes first, then the headings that do", () => {
  assert.deepEqual(searchDocs(index, "harness"), [{ href: "/docs/harness/", title: "The harness (how an agent runs)", where: "The agents" }]);
  assert.deepEqual(
    searchDocs(index, "hooks").map((h) => h.href),
    ["/docs/harness/#hooks", "/docs/board/#events-hooks-and-channels"],
  );
  assert.deepEqual(searchDocs(index, "  HAND  ")[0], { href: "/docs/machines/", title: "Machines and paired hands", where: "Machines and running" });
});

test("a heading is found by its words and its page's together, and a page's name alone does not list all its headings", () => {
  assert.deepEqual(searchDocs(index, "harness hooks"), [{ href: "/docs/harness/#hooks", title: "Hooks", where: "The harness (how an agent runs)" }]);
  assert.deepEqual(
    searchDocs(index, "board skills").map((h) => h.href),
    ["/docs/board/#shared-skills-hive-skill"],
  );
  assert.equal(searchDocs(index, "harness").length, 1);
});

test("a word of the query matches from the start of a word, whatever the punctuation around it", () => {
  // "kill" is not in "Skills"; "skill" is.
  assert.deepEqual(searchDocs(index, "kill"), []);
  assert.deepEqual(
    searchDocs(index, "skill").map((h) => h.href),
    ["/docs/harness/#skills-as-tools", "/docs/board/#shared-skills-hive-skill"],
  );
  assert.equal(searchDocs(index, "pair")[0].href, "/docs/machines/");
  assert.equal(searchDocs(index, "(how an agent runs)")[0].href, "/docs/harness/");
  assert.equal(searchDocs(index, "hive-skill")[0].href, "/docs/board/#shared-skills-hive-skill");
});

test("one or two letters on their own find a whole word, not every word that starts with them", () => {
  const short = [
    { u: "/docs/how-to/first-project/", t: "Set up your first project", g: "Guides", h: [["3-add-your-project", "3. Add your project"]] },
    { u: "/docs/how-to/pull-requests/", t: "Review and merge the work", g: "Guides", k: "pr prs reviewer", h: [["ci", "What CI runs"]] },
  ];
  const found = (q) => searchDocs(short, q).map((h) => h.href);
  assert.deepEqual(found("pr"), ["/docs/how-to/pull-requests/"]);
  assert.deepEqual(found("ci"), ["/docs/how-to/pull-requests/#ci"]);
  assert.deepEqual(found("p"), []);
  // From three letters on, and beside another word, the start of a word is enough.
  assert.deepEqual(found("pro"), ["/docs/how-to/first-project/", "/docs/how-to/first-project/#3-add-your-project"]);
  assert.deepEqual(found("add pr"), ["/docs/how-to/first-project/#3-add-your-project"]);
  // Two letters after a word of grammar are a word still being typed: the list must not empty while a question is written.
  assert.deepEqual(found("the pr"), ["/docs/how-to/first-project/", "/docs/how-to/first-project/#3-add-your-project", "/docs/how-to/pull-requests/"]);
  assert.deepEqual(found("what is ci"), ["/docs/how-to/pull-requests/#ci"]);
});

test("nothing typed finds nothing, and the list stops at its limit", () => {
  assert.deepEqual(searchDocs(index, "   "), []);
  assert.deepEqual(searchDocs(index, " - "), []);
  assert.deepEqual(searchDocs(index, "zzz"), []);
  assert.equal(searchDocs(index, "the", 2).length, 2);
  assert.ok(searchDocs(docs.searchIndex(docs.loadDocs()), "the").length <= 10);
});

// A second index, with what the build adds to each section: the words of its text, and the code it names.
const full = [
  {
    u: "/docs/how-to/operations/",
    t: "Keep Hive running, updated and backed up",
    g: "Guides",
    k: "uninstall service launchd systemd upgrade",
    x: "survive reboot",
    h: [
      ["1-run-as-services", "1. Run as services", "launchd systemd terminal", "hive service install\nhive service status"],
      ["2-update", "2. Update", "release checksum homebrew", "hive update\nhive update --check\nupdate.channel"],
    ],
  },
  { u: "/install/", t: "Install Hive and run your first task", g: "This site", k: "quickstart getting started homebrew", h: [] },
  {
    u: "/docs/settings/",
    t: "Settings",
    g: "Reference",
    c: "hive settings set",
    h: [
      ["update", "update", "channel release source", "update.channel\nupdate.latest_url"],
      ["evolve", "evolve", "loop gates tolerance", "evolve.loop\nevolve.tolerance"],
    ],
  },
];
const hrefs = (q, max) => searchDocs(full, q, max).map((h) => h.href);

test("a setting or a command typed as it is written finds the sections that name it, and says so", () => {
  assert.deepEqual(searchDocs(full, "evolve.loop"), [{ href: "/docs/settings/#evolve", title: "evolve", where: "Settings", why: "evolve.loop" }]);
  const named = (q) => searchDocs(full, q, 20).filter((h) => h.why).map((h) => [h.href, h.why]);
  assert.deepEqual(named("update.channel"), [
    ["/docs/how-to/operations/#2-update", "update.channel"],
    ["/docs/settings/#update", "update.channel"],
  ]);
  // The punctuation and the case a reader types do not matter.
  assert.deepEqual(named("HIVE  service  status"), [["/docs/how-to/operations/#1-run-as-services", "hive service status"]]);
  // The start of a command is enough when it ends at a word, and the whole command comes before its start.
  assert.deepEqual(named("update latest"), [["/docs/settings/#update", "update.latest_url"]]);
  assert.deepEqual(named("hive update check"), [["/docs/how-to/operations/#2-update", "hive update --check"]]);
  // Code a page names before its first heading finds the page.
  assert.deepEqual(searchDocs(full, "hive settings set"), [{ href: "/docs/settings/", title: "Settings", where: "Reference", why: "hive settings set" }]);
  // A heading or a title that holds the words still comes first, and needs no reason beside it.
  assert.deepEqual(searchDocs(full, "hive update")[0], { href: "/docs/how-to/operations/", title: "Keep Hive running, updated and backed up", where: "Guides" });
});

test("the words kept for a page find it, and a title or a heading still comes first", () => {
  assert.deepEqual(hrefs("quickstart"), ["/install/"]);
  assert.deepEqual(hrefs("getting started"), ["/install/"]);
  assert.deepEqual(hrefs("uninstall"), ["/docs/how-to/operations/"]);
  // "update" starts a word of a title and is a heading twice: the title, the heading that is the word, the heading that holds it.
  assert.deepEqual(hrefs("update"), ["/docs/how-to/operations/", "/docs/settings/#update", "/docs/how-to/operations/#2-update"]);
  // A page's own words and its title together.
  assert.deepEqual(hrefs("install homebrew"), ["/install/"]);
});

test("the words of a section's text find the section, after everything that names the thing", () => {
  assert.deepEqual(hrefs("checksum"), ["/docs/how-to/operations/#2-update"]);
  // Words from the text and the heading together, and from the text and the page's title together.
  assert.deepEqual(hrefs("services launchd"), ["/docs/how-to/operations/#1-run-as-services"]);
  assert.deepEqual(hrefs("settings tolerance"), ["/docs/settings/#evolve"]);
  // A page's opening finds the page, last of all.
  assert.deepEqual(hrefs("reboot"), ["/docs/how-to/operations/"]);
  // A word of a title alone never lists every section of that page.
  assert.deepEqual(hrefs("settings"), ["/docs/settings/"]);
  // Every word must be found in the same place.
  assert.deepEqual(hrefs("checksum tolerance"), []);
  // "launchd" is a word kept for the guide and a word of one of its sections: the page first.
  assert.deepEqual(hrefs("launchd"), ["/docs/how-to/operations/", "/docs/how-to/operations/#1-run-as-services"]);
});

test("a query is held to the words that say what is looked for, not to the words of grammar around them", () => {
  // "how do i" names nothing: the finds are those of "update", in the same order.
  assert.deepEqual(hrefs("how do i update"), hrefs("update"));
  assert.deepEqual(hrefs("the checksum of a release"), ["/docs/how-to/operations/#2-update"]);
  // A setting asked about in a sentence is still the setting.
  assert.deepEqual(searchDocs(full, "what is evolve.loop")[0], { href: "/docs/settings/#evolve", title: "evolve", where: "Settings", why: "evolve.loop" });
  // A query made of nothing else is looked for as it is typed, and the heading that is the query word for word comes first.
  const pages = [{ u: "/docs/console/", t: "The console", g: "Start", h: [["runs", "Runs for you"], ["for-you", "For you"]] }];
  assert.deepEqual(searchDocs(pages, "for you").map((h) => h.href), ["/docs/console/#for-you", "/docs/console/#runs"]);
  assert.deepEqual(searchDocs(pages, "a"), []);
  // A word that can be what a reader looks for is not a word of grammar.
  for (const w of ["done", "yes", "use", "get", "run", "stop", "job"]) assert.ok(!STOP_WORDS.has(w), w);
});

test("the words of grammar decide nothing about what is found, and still say which find is nearer to what was typed", () => {
  const pages = [
    {
      u: "/docs/console/",
      t: "The console",
      g: "Start",
      h: [
        ["tabs", "Project tabs", "overview board"],
        ["keys", "Keyboard", "tab key"],
        ["for-you", "For you", "tab questions decisions"],
        ["page", "Page", "numbers"],
        ["read", "Read the For you page", "rows buttons"],
      ],
    },
  ];
  const found = (q) => searchDocs(pages, q).map((h) => h.href.split("#")[1]);
  // By a heading: the one that says "for you" too comes before the one that is the word itself.
  assert.deepEqual(found("for you page"), ["read", "page"]);
  assert.deepEqual(found("page"), ["page", "read"]);
  // By text: the section headed "For you" before the one that stands before it and says "tab" as often. A heading find still comes first.
  assert.deepEqual(found("for you tab"), ["tabs", "for-you", "keys"]);
  assert.deepEqual(found("tab"), ["tabs", "keys", "for-you"]);
  // A heading that is the question, word for word, before a heading that is its one telling word.
  const stop = [
    { u: "/docs/commands/", t: "Commands", g: "Reference", h: [["stop", "stop"]] },
    { u: "/docs/faq/", t: "Questions people ask", g: "Start", h: [["how-do-i-stop-it", "How do I stop it?"]] },
  ];
  assert.deepEqual(searchDocs(stop, "how do i stop it").map((h) => h.href), ["/docs/faq/#how-do-i-stop-it", "/docs/commands/#stop"]);
  assert.deepEqual(searchDocs(stop, "stop").map((h) => h.href), ["/docs/commands/#stop", "/docs/faq/#how-do-i-stop-it"]);
  // Holding some of them says nothing, and an article never does: neither "the" nor "what is" in a heading puts it
  // before the section that is about the word.
  const some = [
    {
      u: "/docs/release/",
      t: "Releases",
      g: "Reference",
      h: [
        ["the-console", "The console", "bundle assets page checksum"],
        ["what", "What a release is", "archives targets notes checksum"],
        ["update", "Update", "checksum release"],
      ],
    },
  ];
  const sections = (q) => searchDocs(some, q).map((h) => h.href.split("#")[1]);
  assert.deepEqual(sections("checksum"), ["update", "the-console", "what"]);
  assert.deepEqual(sections("the checksum"), sections("checksum"));
  assert.deepEqual(sections("what is the checksum"), ["what", "update", "the-console"]);
  assert.deepEqual(sections("what does the checksum do"), sections("checksum"));
});

test("among sections found by their text, the one most about the words comes first", () => {
  const about = [
    {
      u: "/docs/how-to/tasks/",
      t: "Give a task",
      g: "Guides",
      h: [
        // Says both words once, late in its text.
        ["brief", "A brief", "agent repository branch tests api key"],
        // Says them most.
        ["secrets", "Secrets", "key api paste provider"],
        // Has one of them in its heading.
        ["api", "The API", "endpoint token key"],
      ],
    },
    // Has one of them in its page's title.
    { u: "/docs/harness/", t: "Keys and sign-in", g: "The agents", h: [["providers", "Other providers", "openai api gemini"]] },
  ];
  assert.deepEqual(
    searchDocs(about, "api key").map((h) => h.href),
    ["/docs/how-to/tasks/#api", "/docs/harness/#providers", "/docs/how-to/tasks/#secrets", "/docs/how-to/tasks/#brief"],
  );
  // Words about as prominent count the same, and the order of the index decides.
  const even = [
    { u: "/docs/a/", t: "A", g: "Guides", h: [["one", "One", "alpha beta gamma delta token"]] },
    { u: "/docs/b/", t: "B", g: "Reference", h: [["two", "Two", "alpha beta gamma token"]] },
    { u: "/docs/c/", t: "C", g: "Reference", h: [["three", "Three", "token alpha"]] },
  ];
  assert.deepEqual(searchDocs(even, "token").map((h) => h.href), ["/docs/c/#three", "/docs/a/#one", "/docs/b/#two"]);
});

test("a record of a decision comes after the pages that say what holds, among finds of one kind", () => {
  const records = [
    { u: "/docs/decisions/adr-069/", t: "ADR-069: Delivery by pull request", g: "Decisions and design", r: 1, h: [["option-2", "Option 2: a pull request for every branch, merged by the owner", "reviewer"]] },
    { u: "/docs/how-to/pull-requests/", t: "Review and merge the work", g: "Guides", h: [["2-find", "2. Find the pull request", "console task"], ["5-merge", "5. Merge it", "button pull request reviewer"]] },
  ];
  const found = (q) => searchDocs(records, q).map((h) => h.href);
  // The record's heading holds all three words and the guide's needs its page's title for one: the guide's is first all the same.
  assert.deepEqual(found("merge pull request"), ["/docs/how-to/pull-requests/#2-find", "/docs/decisions/adr-069/#option-2", "/docs/how-to/pull-requests/#5-merge"]);
  // The kind of find still counts first: a record found by its title stands before any heading.
  assert.deepEqual(found("pull request"), ["/docs/decisions/adr-069/", "/docs/how-to/pull-requests/#2-find", "/docs/decisions/adr-069/#option-2", "/docs/how-to/pull-requests/#5-merge"]);
  assert.deepEqual(found("adr 069"), ["/docs/decisions/adr-069/"]);
});

// On the real docs the tests below hold whatever the docs say: a page reworded, a heading renamed or a page added
// must not fail one. Which find comes first is pinned above, on indexes made for it.
const real = docs.searchIndex(docs.loadDocs());
const all = (q) => new Set(searchDocs(real, q, Infinity).map((h) => h.href));

test("on the real index: every title finds its page and every heading its section", () => {
  let asked = 0;
  for (const doc of docs.loadDocs().docs) {
    for (const [text, href] of [[doc.title, doc.path], ...doc.toc.map((h) => [h.text, `${doc.path}#${h.id}`])]) {
      // A heading of signs alone has no word to be found by.
      if (searchWords(text).length === 0) continue;
      asked += 1;
      assert.ok(all(text).has(href), `"${text}" does not find ${href}`);
    }
  }
  assert.ok(asked > 0);
});

test("on the real index: every word kept for a page finds that page", () => {
  // The guides: "uninstall", "cron", "pr".
  const guides = new Map(docs.loadDocs().docs.filter((d) => d.group === docs.GUIDES_GROUP).map((d) => [d.slug, d.path]));
  assert.deepEqual(Object.keys(docs.GUIDE_KEYWORDS).filter((slug) => !guides.has(slug)), [], "words are kept for guides that exist; a guide needs none");
  for (const [slug, words] of Object.entries(docs.GUIDE_KEYWORDS)) {
    for (const w of searchWords(words)) assert.ok(all(w).has(guides.get(slug)), `"${w}" is kept for ${slug} and does not find it`);
  }
  // The site's own pages: "quickstart".
  const pages = real.filter((e) => e.g === "This site");
  assert.ok(pages.length > 0);
  for (const e of pages) {
    for (const w of searchWords(e.k)) assert.ok(all(w).has(e.u), `"${w}" is kept for ${e.u} and does not find it`);
  }
});

test("on the real index: nothing the docs say is out of reach, however common the word and wherever it stands", () => {
  // Every command and setting the docs name finds every place that names it.
  const named = new Map();
  // Every section is found by a word of its own, alone and beside the word of its text that most sections say.
  const said = new Map();
  const places = [];
  for (const doc of docs.loadDocs().docs) {
    doc.sections.forEach((section, i) => {
      const href = i === 0 ? doc.path : `${doc.path}#${section.id}`;
      for (const code of new Set(section.code.filter(isCodeTerm))) named.set(code, [...(named.get(code) ?? []), href]);
      const words = [...new Set(searchWords(section.text).filter((w) => w.length > 1 && !STOP_WORDS.has(w)))];
      for (const w of words) said.set(w, (said.get(w) ?? 0) + 1);
      if (words.length > 0) places.push({ href, page: doc.path, words });
    });
  }
  assert.ok(named.size > 0 && places.length > 0);
  for (const [code, hrefs] of named) {
    const found = all(code);
    assert.deepEqual(hrefs.filter((h) => !found.has(h)), [], `\`${code}\` is named there and not found there`);
  }
  for (const { href, page, words } of places) {
    const byUse = [...words].sort((a, b) => said.get(a) - said.get(b));
    // The same asked as a question: the words of grammar around them must not lose the place.
    for (const q of [byUse[0], `${byUse.at(-1)} ${byUse[0]}`, `how do I ${byUse[0]} the ${byUse.at(-1)}`]) {
      const found = all(q);
      // A word that only the page's title shows finds the page, not each of its sections.
      assert.ok(found.has(href) || found.has(page), `"${q}" is said in ${href} and does not find it`);
    }
  }
});
