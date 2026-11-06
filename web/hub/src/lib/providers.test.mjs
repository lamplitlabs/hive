import assert from "node:assert/strict";
import { test } from "node:test";
import { groupProviders, keyProblem, providerInfo, suggestLightModel, suggestModel } from "./providers.ts";

const hub = (name, label, device_login = false) => ({ name, label, device_login, token_login: true });

test("a Claude subscription token is refused before it reaches Anthropic", () => {
  assert.match(keyProblem("anthropic", "sk-ant-oat01-abc"), /Pro or Max/);
  assert.match(keyProblem("anthropic", "sk-ant-admin01-abc"), /admin key/);
  assert.equal(keyProblem("anthropic", "sk-ant-api03-abc"), "");
});

test("a key for another provider is caught by its shape", () => {
  assert.match(keyProblem("anthropic", "sk-proj-abc"), /another provider/);
  assert.match(keyProblem("openai", "sk-ant-api03-abc"), /another provider/);
  assert.match(keyProblem("gemini", "sk-abc"), /another provider/);
  assert.match(keyProblem("copilot", "sk-abc"), /GitHub token/);
  assert.match(keyProblem("openai", "sk-abc def"), /no spaces/);
  assert.equal(keyProblem("openai", "sk-abc"), "");
  assert.equal(keyProblem("groq", ""), "");
});

test("providers group by how they connect, in catalog order, with a sign-in counting as an account", () => {
  const groups = groupProviders([hub("openai", "OpenAI-compatible"), hub("ollama", "Ollama (local)"), hub("anthropic", "Anthropic Claude", true), hub("copilot", "GitHub Copilot", true), hub("future", "Future AI")]);
  assert.deepEqual(
    groups.map((g) => [g.kind, g.items.map((i) => i.hub.name)]),
    [
      ["account", ["copilot", "anthropic"]],
      ["key", ["openai", "future"]],
      ["local", ["ollama"]],
    ],
  );
  // The hub's label stands in for a provider the catalog doesn't know.
  assert.equal(groups[1].items[1].info.title, "Future AI");
  assert.equal(providerInfo("openai", "OpenAI-compatible").title, "OpenAI");
});

test("the search narrows by name, words and kind", () => {
  const all = [hub("openai", "OpenAI-compatible"), hub("ollama", "Ollama (local)"), hub("gemini", "Google Gemini")];
  assert.deepEqual(groupProviders(all, "free").flatMap((g) => g.items.map((i) => i.hub.name)), ["gemini", "ollama"]);
  assert.deepEqual(groupProviders(all, "nothing like this"), []);
});

test("model picks keep a listed current model, else the default, and find a light sibling", () => {
  const models = [{ id: "claude-opus-5-5" }, { id: "claude-sonnet-5-5", default: true }, { id: "claude-haiku-4-5" }];
  assert.equal(suggestModel(models, "claude-opus-5-5"), "claude-opus-5-5");
  assert.equal(suggestModel(models, "gpt-5.5"), "claude-sonnet-5-5");
  assert.equal(suggestModel([], "x"), "");
  assert.equal(suggestLightModel(models, ""), "claude-haiku-4-5");
  assert.equal(suggestLightModel([{ id: "gpt-5.5" }], ""), "");
});
