// The site's pure facts, checked without a browser: node --test runs this
// against the TypeScript module through the strip-types loader.
import test from "node:test";
import assert from "node:assert/strict";

const site = await import("./site.ts");

test("the site names hivefleet.dev and nothing of GitHub", () => {
  assert.equal(site.SITE.domain, "hivefleet.dev");
  assert.equal(site.SITE.url, "https://hivefleet.dev");
  for (const v of Object.values(site.SITE)) assert.doesNotMatch(v, /github/i);
});

test("without a Clerk key the sign-in is not ready", () => {
  if (!process.env.NEXT_PUBLIC_CLERK_PUBLISHABLE_KEY) {
    assert.equal(site.SIGN_IN_READY, false);
    assert.equal(site.CLERK_PUBLISHABLE_KEY, "");
  } else {
    assert.equal(site.SIGN_IN_READY, true);
  }
});
