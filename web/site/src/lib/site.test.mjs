// The site's pure facts, checked without a browser: node --test runs this
// against the TypeScript module through the strip-types loader.
import test from "node:test";
import assert from "node:assert/strict";

const site = await import("./site.ts");

test("the site names hivefleet.dev and the repository", () => {
  assert.equal(site.SITE.domain, "hivefleet.dev");
  assert.equal(site.SITE.url, "https://hivefleet.dev");
  assert.equal(site.SITE.repo, "https://github.com/lamplitlabs/hive");
  assert.equal(site.repoFile("/docs/harness.md"), "https://github.com/lamplitlabs/hive/blob/main/docs/harness.md");
  assert.equal(site.absoluteUrl("/install/"), "https://hivefleet.dev/install/");
});

test("the words are the open tool's: the category line and the one sentence", () => {
  assert.equal(site.SITE.tagline, "The self-improving agent harness");
  assert.match(site.SITE.sentence, /^Hive is an open-source harness for coding agents\./);
  assert.match(site.SITE.sentence, /your own machine as one binary/);
});

test("the install lines are the contract's, character for character", () => {
  assert.equal(site.INSTALL.script, "curl -fsSL https://hivefleet.dev/install.sh | sh");
  assert.equal(site.INSTALL.source, "cargo install --git https://github.com/lamplitlabs/hive --locked --root ~/.hive hive");
  assert.equal(site.INSTALL.blocks.script, "curl -fsSL https://hivefleet.dev/install.sh | sh      # macOS and Linux; installs ~/.hive/bin/hive");
  assert.equal(
    site.INSTALL.blocks.source,
    "cargo install --git https://github.com/lamplitlabs/hive --locked --root ~/.hive hive   # from source: Rust 1.95+, Node.js 22+",
  );
  assert.equal(
    site.INSTALL.blocks.after,
    [
      'export PATH="$HOME/.hive/bin:$PATH"    # add to your shell profile',
      "hive version",
      "hive hub                               # the console, at http://127.0.0.1:8765",
    ].join("\n"),
  );
  // The block is the bare command and its note, nothing else.
  assert.ok(site.INSTALL.blocks.script.startsWith(`${site.INSTALL.script} `));
  assert.ok(site.INSTALL.blocks.source.startsWith(`${site.INSTALL.source} `));
});

test("nothing of the hosted product is left in the site's own words", () => {
  const words = JSON.stringify([site.SITE, site.INSTALL, site.PAGES, site.NOT_FOUND]);
  assert.doesNotMatch(words, /clerk|sign[ -]?in|sign[ -]?up|\baccounts?\b|team brain|hosted/i);
});
