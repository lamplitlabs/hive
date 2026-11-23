// The search the browser runs: titles and headings first, then the commands
// and settings a section names, then the words of its text.
import test from "node:test";
import assert from "node:assert/strict";

const { searchDocs } = await import("./search.ts");
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

test("on the real index: the words a reader types reach the page that answers", () => {
  const real = docs.searchIndex(docs.loadDocs());
  const first = (q) => searchDocs(real, q)[0]?.href;
  assert.equal(first("hooks"), "/docs/harness/#hooks");
  assert.equal(first("pairing hands"), "/docs/machines/#pairing-hands");
  assert.equal(first("evolve settings"), "/docs/evolve/");
  assert.equal(first("complexity rule"), "/docs/review/#the-complexity-rule");
  // What the titles and headings alone never found.
  assert.equal(first("quickstart"), "/install/");
  assert.equal(first("getting started"), "/install/");
  assert.equal(first("hive pair"), "/machines/");
  assert.equal(first("tour"), "/docs/how-to/console-tour/");
  const names = (q) => searchDocs(real, q, 50).filter((h) => h.why);
  assert.ok(names("evolve.loop").length >= 3);
  assert.ok(names("evolve.loop").every((h) => h.why === "evolve.loop"));
  assert.ok(searchDocs(real, "ollama").length >= 2);
  assert.ok(searchDocs(real, "docker").length >= 2);
});

test("on the real index: a word a newcomer types reaches the guide that covers it", () => {
  const real = docs.searchIndex(docs.loadDocs());
  const first = (q) => searchDocs(real, q)[0]?.href;
  for (const [q, guide] of [
    ["tutorial", ""],
    ["uninstall", "operations"],
    ["restore", "operations"],
    ["cron", "schedules"],
    ["kill switch", "rules"],
    ["ollama", "models"],
    ["pr", "pull-requests"],
    ["dashboard", "console-tour"],
    ["stuck", "troubleshooting"],
  ]) {
    // The guide itself, or the section of it that is named for the word.
    const page = guide ? `/docs/how-to/${guide}/` : "/docs/how-to/";
    assert.equal(first(q)?.split("#")[0], page, `"${q}" finds ${first(q)}`);
  }
  // The name of a command or of a family of settings still finds that first, and the guide right after it.
  assert.deepEqual(
    searchDocs(real, "inbox", 2).map((h) => h.href),
    ["/docs/settings/#inbox", "/docs/how-to/what-needs-you/"],
  );
  // Every guide has words kept for it, and every word kept is one a guide of that name exists for.
  const guides = docs.loadDocs().docs.filter((d) => d.group === docs.GUIDES_GROUP).map((d) => d.slug);
  assert.deepEqual(Object.keys(docs.GUIDE_KEYWORDS).sort(), [...guides].sort());
});
