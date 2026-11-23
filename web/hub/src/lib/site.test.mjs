import assert from "node:assert/strict";
import { test } from "node:test";
import { SITE_URL, docUrl, linkDocs } from "./site.ts";

test("a file under docs/ is its page on the site, the path in lowercase", () => {
  assert.equal(SITE_URL, "https://hivefleet.dev");
  assert.equal(docUrl("docs/landing.md"), "https://hivefleet.dev/docs/landing/");
  assert.equal(docUrl("docs/decisions/ADR-070-web-first-hive-hosted-brain-open-hand.md"), "https://hivefleet.dev/docs/decisions/adr-070-web-first-hive-hosted-brain-open-hand/");
  // The same file however it is named: without docs/, without .md, from the repository's root.
  for (const ref of ["security.md", "security", "docs/security", "./docs/security.md", "/docs/security.md", "Docs/Security.MD", " docs/security.md "]) assert.equal(docUrl(ref), "https://hivefleet.dev/docs/security/", ref);
  assert.equal(docUrl("docs/context/environment.md"), "https://hivefleet.dev/docs/context/environment/");
  // The folder itself is the documentation's first page.
  for (const ref of ["docs", "docs/", ""]) assert.equal(docUrl(ref), "https://hivefleet.dev/docs/", JSON.stringify(ref));
});

test("an anchor keeps its GitHub-style slug", () => {
  assert.equal(docUrl("docs/recovery.md#failed-jobs-and-recovery"), "https://hivefleet.dev/docs/recovery/#failed-jobs-and-recovery");
  assert.equal(docUrl("console.md#reaching-the-console-from-anywhere"), "https://hivefleet.dev/docs/console/#reaching-the-console-from-anywhere");
  assert.equal(docUrl("docs/Release.md#Channels"), "https://hivefleet.dev/docs/release/#Channels", "only the path is lowercased");
});

test("a GitHub link to a documentation file becomes its page; other GitHub links stay", () => {
  assert.equal(docUrl("https://github.com/lamplitlabs/hive/blob/main/docs/landing.md"), "https://hivefleet.dev/docs/landing/");
  assert.equal(docUrl("https://github.com/lamplitlabs/hive/blob/v0.1.4/docs/decisions/ADR-071-hive-is-an-open-source-tool.md#decision"), "https://hivefleet.dev/docs/decisions/adr-071-hive-is-an-open-source-tool/#decision");
  assert.equal(docUrl("https://github.com/lamplitlabs/hive/blob/main/docs/security.md?plain=1#what-leaves-the-machine"), "https://hivefleet.dev/docs/security/#what-leaves-the-machine");
  assert.equal(docUrl("https://github.com/lamplitlabs/hive/tree/main/docs"), "https://hivefleet.dev/docs/");
  for (const stays of [
    "https://github.com/lamplitlabs/hive/blob/main/crates/hive-hub/src/hands.rs",
    "https://github.com/lamplitlabs/hive/blob/main/README.md",
    "https://github.com/lamplitlabs/hive/blob/main/docsify/x.md",
    "https://github.com/lamplitlabs/hive/issues/12",
    "https://github.com/lamplitlabs/hive/releases/tag/v0.2.0",
  ])
    assert.equal(docUrl(stays), stays);
});

test("a text that names a documentation file gets a link for it and keeps its words", () => {
  const text = "The choice for a brain on a server that should hold no code (docs/security.md).";
  const parts = linkDocs(text);
  assert.deepEqual(parts, [
    { text: "The choice for a brain on a server that should hold no code (" },
    { text: "docs/security.md", href: "https://hivefleet.dev/docs/security/" },
    { text: ")." },
  ]);
  assert.equal(parts.map((p) => p.text).join(""), text);
  assert.deepEqual(linkDocs("hive update installs the release (update.channel, docs/release.md#channels). Empty: no check"), [
    { text: "hive update installs the release (update.channel, " },
    { text: "docs/release.md#channels", href: "https://hivefleet.dev/docs/release/#channels" },
    { text: "). Empty: no check" },
  ]);
  assert.deepEqual(linkDocs("See https://github.com/lamplitlabs/hive/blob/main/docs/machines.md#joining."), [
    { text: "See " },
    { text: "https://github.com/lamplitlabs/hive/blob/main/docs/machines.md#joining", href: "https://hivefleet.dev/docs/machines/#joining" },
    { text: "." },
  ]);
  assert.deepEqual(linkDocs("docs/faq.md"), [{ text: "docs/faq.md", href: "https://hivefleet.dev/docs/faq/" }]);
});

test("files of the reader's own project, and other names, are left as words", () => {
  for (const text of [
    "",
    "Whether Hive updates and restarts itself.",
    "agents read docs/context/environment.md to learn how to build the project",
    "the constitution (docs/evolution/constitution.md) and docs/decisions/",
    "kept in mydocs/notes.md and in project/docs/notes.md",
    "a page named docs/notes.mdx or docs/notes.md-old",
    "the source is at https://github.com/lamplitlabs/hive/blob/main/crates/hive-hub/src/hands.rs",
    "releases are at https://github.com/lamplitlabs/hive/releases/tag/v0.2.0",
  ])
    assert.deepEqual(linkDocs(text), [{ text }], text);
});
