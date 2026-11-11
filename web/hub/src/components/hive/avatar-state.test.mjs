import assert from "node:assert/strict";
import { test } from "node:test";
import { AVATAR_HUES, avatarColors, hashOf, hueOf, initialsOf } from "./avatar-state.ts";

test("initialsOf takes one letter from each of the first two words", () => {
  assert.equal(initialsOf("demo-app"), "DA");
  assert.equal(initialsOf("hive"), "H");
  assert.equal(initialsOf("Amistio Template"), "AT");
  assert.equal(initialsOf("x_y_z"), "XY");
  assert.equal(initialsOf("my.project"), "MP");
  assert.equal(initialsOf("acme/web"), "AW");
});

test("initialsOf reads a camelCase word's capitals and keeps digits", () => {
  assert.equal(initialsOf("demoApp"), "DA");
  assert.equal(initialsOf("HIVE"), "H", "an all-caps word is one word");
  assert.equal(initialsOf("app2"), "A");
  assert.equal(initialsOf("3d-viewer"), "3V");
});

test("initialsOf never returns empty", () => {
  assert.equal(initialsOf(""), "?");
  assert.equal(initialsOf("   "), "?");
  assert.equal(initialsOf("---"), "?");
  assert.equal(initialsOf(null), "?");
  assert.equal(initialsOf(undefined), "?");
});

test("hashOf is stable and spreads", () => {
  assert.equal(hashOf("demo-app"), hashOf("demo-app"));
  assert.notEqual(hashOf("demo-app"), hashOf("demo-apq"));
  assert.ok(hashOf("") >= 0);
});

test("hueOf picks one of the eight hues, case-insensitively, and is stable", () => {
  const seen = new Set();
  for (const s of ["demo-app", "hive", "amistio-template", "blog", "shop", "api", "docs", "infra", "web", "mobile", "data", "ml"]) {
    const h = hueOf(s);
    assert.ok(AVATAR_HUES.includes(h), s + " -> " + h);
    assert.equal(hueOf(s), h);
    assert.equal(hueOf(s.toUpperCase()), h);
    seen.add(h);
  }
  assert.ok(seen.size >= 4, "a dozen slugs should land on several hues, got " + seen.size);
  assert.equal(hueOf(""), AVATAR_HUES[0]);
  assert.equal(hueOf(null), AVATAR_HUES[0]);
});

test("avatarColors gives an oklch background in the slug's hue and a contrasting ink", () => {
  for (const s of ["demo-app", "hive", "blog", "api"]) {
    const c = avatarColors(s);
    assert.match(c.background, new RegExp("^oklch\\(0\\.\\d+ 0\\.\\d+ " + hueOf(s) + "\\)$"));
    assert.ok(c.color === "#fff" || c.color.startsWith("oklch("));
  }
});
