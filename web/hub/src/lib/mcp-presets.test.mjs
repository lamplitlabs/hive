import assert from "node:assert/strict";
import { test } from "node:test";
import { MCP_PRESETS, formFromPreset, presetAccess, presetFromSuggestion, toolChoices } from "./mcp-presets.ts";

const suggestion = (over = {}) => ({
  project: "demo-app",
  name: "sentry",
  title: "Sentry (demo-app)",
  about: "The errors demo-app reports.",
  docs: "https://docs.sentry.io/product/sentry-mcp/",
  why: "demo-app ships @sentry/node",
  transport: "http",
  command: "",
  args: [],
  url: "https://mcp.sentry.dev/mcp",
  auth: "oauth",
  auth_header: "",
  secret: "",
  scope: "project",
  add: "hive mcp add sentry",
  after: "",
  ...over,
});

test("the presets have unique names and everything a server needs to start", () => {
  const names = MCP_PRESETS.map((p) => p.name);
  assert.equal(new Set(names).size, names.length);
  for (const p of MCP_PRESETS) {
    assert.ok(p.title && p.about && p.docs.startsWith("https://"), p.name);
    if (p.transport === "stdio") assert.ok(p.command && !p.url, p.name);
    else assert.ok(p.url.startsWith("https://") && !p.command, p.name);
    // A secret is described exactly when the server signs in with one.
    assert.equal(p.secret !== "", p.auth !== "none" && p.auth !== "oauth", p.name);
  }
});

test("presetFromSuggestion keeps the suggestion's details, why and project", () => {
  const p = presetFromSuggestion(suggestion());
  assert.deepEqual(p, {
    name: "sentry",
    title: "Sentry (demo-app)",
    about: "The errors demo-app reports.",
    docs: "https://docs.sentry.io/product/sentry-mcp/",
    transport: "http",
    command: "",
    args: [],
    url: "https://mcp.sentry.dev/mcp",
    auth: "oauth",
    auth_header: "",
    secret: "",
    shared: false,
    why: "demo-app ships @sentry/node",
    project: "demo-app",
  });
  assert.equal(presetFromSuggestion(suggestion({ scope: "shared" })).shared, true);
});

test("toolChoices puts suggestions first, once per name, and drops what they override", () => {
  const { suggested, popular } = toolChoices(
    [suggestion(), suggestion({ project: "other", title: "Sentry again" }), suggestion({ name: "custom-db", title: "Custom DB" })],
    [],
  );
  assert.deepEqual(suggested.map((p) => [p.name, p.title]), [
    ["sentry", "Sentry (demo-app)"],
    ["custom-db", "Custom DB"],
  ]);
  assert.ok(!popular.some((p) => p.name === "sentry"), "the suggestion wins over the sentry preset");
  assert.equal(popular.length, MCP_PRESETS.length - 1);
  assert.deepEqual(popular.map((p) => p.name), MCP_PRESETS.filter((p) => p.name !== "sentry").map((p) => p.name));
});

test("toolChoices leaves out every name already connected", () => {
  const { suggested, popular } = toolChoices([suggestion(), suggestion({ name: "custom-db" })], ["sentry", "github", "playwright"]);
  assert.deepEqual(suggested.map((p) => p.name), ["custom-db"]);
  const names = popular.map((p) => p.name);
  for (const gone of ["sentry", "github", "playwright"]) assert.ok(!names.includes(gone), gone);
  assert.equal(names.length, MCP_PRESETS.length - 3);
});

test("toolChoices with nothing suggested or taken offers every preset", () => {
  const { suggested, popular } = toolChoices([], []);
  assert.deepEqual(suggested, []);
  assert.deepEqual(popular, MCP_PRESETS);
});

test("presetAccess says how a preset signs in", () => {
  assert.equal(presetAccess({ auth: "oauth", transport: "http" }), "Signs in once");
  assert.equal(presetAccess({ auth: "none", transport: "stdio" }), "Runs here, no sign-in");
  assert.equal(presetAccess({ auth: "none", transport: "http" }), "No sign-in");
  assert.equal(presetAccess({ auth: "env", transport: "stdio" }), "Needs a secret");
  assert.equal(presetAccess({ auth: "bearer", transport: "http" }), "Needs a secret");
});

test("formFromPreset starts the details step with the preset's server", () => {
  const playwright = MCP_PRESETS.find((p) => p.name === "playwright");
  assert.deepEqual(formFromPreset(playwright, "demo-app"), {
    name: "playwright",
    project: "",
    transport: "stdio",
    command: "npx",
    args: "-y\n@playwright/mcp@latest",
    url: "",
    auth: "none",
    auth_header: "",
    note: playwright.about,
  });
});

test("formFromPreset scopes a per-project preset to its own project, else the one given", () => {
  const postgres = MCP_PRESETS.find((p) => p.name === "postgres");
  assert.equal(postgres.shared, false);
  assert.equal(formFromPreset(postgres, "demo-app").project, "demo-app");
  assert.equal(formFromPreset(postgres).project, "");
  assert.equal(formFromPreset(postgres, "demo-app").auth_header, "DATABASE_URI");
  const suggested = presetFromSuggestion(suggestion());
  assert.equal(formFromPreset(suggested, "elsewhere").project, "demo-app");
  assert.equal(formFromPreset({ ...suggested, project: "" }, "elsewhere").project, "elsewhere");
});
