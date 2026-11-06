import assert from "node:assert/strict";
import { test } from "node:test";
import { LOGOS, LOGO_IDS, isLogoId, logoFor } from "./logos.ts";

test("every logo has a title, a 6-digit hex and a non-empty 24x24 path", () => {
  assert.equal(LOGO_IDS.length, 28);
  for (const id of LOGO_IDS) {
    const l = LOGOS[id];
    assert.ok(l, id);
    assert.ok(l.title.length > 0, id + " title");
    assert.match(l.hex, /^[0-9A-F]{6}$/, id + " hex");
    assert.match(l.path, /^[Mm]/, id + " path starts with a move");
    assert.ok(l.path.length > 10, id + " path");
    assert.ok(!/["<>]/.test(l.path), id + " path is plain path data");
  }
  assert.equal(Object.keys(LOGOS).length, LOGO_IDS.length);
});

test("brand hexes follow the brief", () => {
  assert.equal(LOGOS.anthropic.hex, "191919");
  assert.equal(LOGOS.claude.hex, "D97757");
  assert.equal(LOGOS.github.hex, "181717");
  assert.equal(LOGOS.googlegemini.hex, "8E75B2");
  assert.equal(LOGOS.mistralai.hex, "FA520F");
  assert.equal(LOGOS.vllm.hex, "2D6BFF");
  assert.equal(LOGOS.linux.hex, "FCC624");
  assert.equal(LOGOS.googlecloud.hex, "4285F4");
});

test("logoFor maps provider names", () => {
  assert.equal(logoFor("anthropic"), "anthropic");
  assert.equal(logoFor("Anthropic Claude"), "claude", "the Claude mark beside Claude models");
  assert.equal(logoFor("claude-sonnet-4"), "claude");
  assert.equal(logoFor("GitHub Copilot"), "githubcopilot");
  assert.equal(logoFor("copilot"), "githubcopilot");
  assert.equal(logoFor("github"), "github");
  assert.equal(logoFor("GitHub Models"), "github");
  assert.equal(logoFor("gemini-2.5-pro"), "googlegemini");
  assert.equal(logoFor("Google Gemini"), "googlegemini");
  assert.equal(logoFor("google"), "googlegemini");
  assert.equal(logoFor("ollama"), "ollama");
  assert.equal(logoFor("Mistral AI"), "mistralai");
  assert.equal(logoFor("mistral-large"), "mistralai");
  assert.equal(logoFor("OpenRouter"), "openrouter");
  assert.equal(logoFor("deepseek-r1"), "deepseek");
  assert.equal(logoFor("vLLM"), "vllm");
  assert.equal(logoFor("LM Studio"), "lmstudio");
  assert.equal(logoFor("lmstudio"), "lmstudio");
});

test("logoFor maps tool names", () => {
  assert.equal(logoFor("mcp"), "modelcontextprotocol");
  assert.equal(logoFor("Model Context Protocol server"), "modelcontextprotocol");
  assert.equal(logoFor("Linear"), "linear");
  assert.equal(logoFor("notion"), "notion");
  assert.equal(logoFor("Figma"), "figma");
  assert.equal(logoFor("jira"), "jira");
  assert.equal(logoFor("Confluence"), "confluence");
  assert.equal(logoFor("postgres"), "postgresql");
  assert.equal(logoFor("PostgreSQL"), "postgresql");
  assert.equal(logoFor("sentry"), "sentry");
  assert.equal(logoFor("docker"), "docker");
  assert.equal(logoFor("Vercel"), "vercel");
  assert.equal(logoFor("cloudflare"), "cloudflare");
  assert.equal(logoFor("Stripe"), "stripe");
});

test("logoFor maps machines by their system", () => {
  assert.equal(logoFor("mac"), "apple");
  assert.equal(logoFor("macOS 15"), "apple");
  assert.equal(logoFor("darwin"), "apple");
  assert.equal(logoFor("Apple M3"), "apple");
  assert.equal(logoFor("ubuntu 24.04"), "ubuntu");
  assert.equal(logoFor("Debian"), "debian");
  assert.equal(logoFor("linux"), "linux");
  assert.equal(logoFor("gcp"), "googlecloud");
  assert.equal(logoFor("Google Cloud"), "googlecloud");
  assert.equal(logoFor("google-cloud-e2"), "googlecloud");
});

test("logoFor is null for names without a mark", () => {
  for (const s of ["openai", "gpt-4o", "azure", "aws", "groq", "xai", "slack", "playwright", "context7", "", "   ", null, undefined]) {
    assert.equal(logoFor(s), null, JSON.stringify(s));
  }
  assert.equal(logoFor("macro"), null, "mac needs a word boundary");
  assert.equal(logoFor("maclean"), null);
});

test("isLogoId guards free text", () => {
  assert.equal(isLogoId("github"), true);
  assert.equal(isLogoId("openai"), false);
  assert.equal(isLogoId("toString"), false);
});
