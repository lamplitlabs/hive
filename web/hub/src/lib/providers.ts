// What the console knows about each model provider, for the Connect a
// provider wizard (components/settings/provider-wizard.tsx) and the provider
// card in Settings -> Harness: how a provider signs in, where its key comes
// from, what its address is, and a one-line pitch in plain words. The hub's
// own list (GET harness: `providers`) says which providers this hive build
// has and whether each offers a sign-in or a pasted key; this file only adds
// the words and the links, so a provider the catalog doesn't know still
// shows, under its own label. Kept free of `@/` imports so `node --test`
// can load it (providers.test.mjs).

// How a provider gets its credentials, which also groups the picker:
// `account` signs in through the browser (no key to copy), `key` takes a
// pasted API key, `local` is a model server on this machine that needs
// none, `custom` is any other OpenAI-compatible server.
// Opens the Connect a provider wizard on the Settings page already open
// (the command palette); elsewhere `/settings/?connect=1#harness` does it.
export const CONNECT_PROVIDER_EVENT = "hive:connect-provider";

export type ProviderKind = "account" | "key" | "local" | "custom";

export interface ProviderInfo {
  name: string;
  // The name people know it by (the hub's label says "OpenAI-compatible").
  title: string;
  kind: ProviderKind;
  // One sentence: what you get and what it costs.
  blurb: string;
  // Who stands behind the sign-in button ("GitHub", "Microsoft", "Anthropic").
  idp?: string;
  // Where to get a key, as a link and the words for it.
  keyUrl?: string;
  keySite?: string;
  // What a key looks like, for the field's placeholder.
  keyHint?: string;
  // The address setting and the address a fresh install uses ("" = none:
  // the owner must give one).
  baseUrlSetting?: string;
  defaultBaseUrl?: string;
  // True when the address can't stay empty (Azure, custom).
  needsAddress?: boolean;
  // A sign-in that runs a program on the machine Hive runs on.
  signInNote?: string;
  // Shown under the picker card as a short badge.
  badge?: string;
}

export const PROVIDERS: ProviderInfo[] = [
  {
    name: "copilot",
    title: "GitHub Copilot",
    kind: "account",
    idp: "GitHub",
    blurb: "Uses your Copilot plan. Sign in with GitHub, no key to copy. The models include Claude, GPT and Gemini.",
    badge: "Recommended",
  },
  {
    name: "anthropic",
    title: "Anthropic Claude",
    kind: "key",
    idp: "Anthropic",
    blurb: "Claude models, straight from Anthropic. Sign in with your Anthropic Console account or paste an API key. Billed per token.",
    keyUrl: "https://console.anthropic.com/settings/keys",
    keySite: "console.anthropic.com",
    keyHint: "sk-ant-api03-…",
    baseUrlSetting: "provider.anthropic.base_url",
    defaultBaseUrl: "https://api.anthropic.com",
    signInNote: "Hive runs Anthropic's own command-line tool (ant) on the machine Hive runs on, and a browser window opens there for you to approve.",
  },
  {
    name: "openai",
    title: "OpenAI",
    kind: "key",
    blurb: "GPT models with an OpenAI API key. Billed per token.",
    keyUrl: "https://platform.openai.com/api-keys",
    keySite: "platform.openai.com",
    keyHint: "sk-…",
    baseUrlSetting: "provider.openai.base_url",
    defaultBaseUrl: "https://api.openai.com/v1",
  },
  {
    name: "gemini",
    title: "Google Gemini",
    kind: "key",
    blurb: "Gemini models with a Google AI Studio key. A free tier is available.",
    keyUrl: "https://aistudio.google.com/apikey",
    keySite: "aistudio.google.com",
    keyHint: "AIza…",
    baseUrlSetting: "provider.gemini.base_url",
    defaultBaseUrl: "https://generativelanguage.googleapis.com",
  },
  {
    name: "github-models",
    title: "GitHub Models",
    kind: "account",
    idp: "GitHub",
    blurb: "Models hosted by GitHub. Sign in with GitHub or paste a token. Free within GitHub's limits.",
    keyUrl: "https://github.com/settings/personal-access-tokens",
    keySite: "github.com",
    keyHint: "github_pat_…",
    baseUrlSetting: "provider.github-models.base_url",
    defaultBaseUrl: "https://models.github.ai/inference",
  },
  {
    name: "azure-openai",
    title: "Azure OpenAI",
    kind: "account",
    idp: "Microsoft",
    blurb: "Your company's Azure OpenAI resource. Sign in with Microsoft or paste a key. Needs the resource's address.",
    keyUrl: "https://portal.azure.com/#view/Microsoft_Azure_ProjectOxford/CognitiveServicesHub/~/OpenAI",
    keySite: "the Azure portal",
    keyHint: "key or bearer token",
    baseUrlSetting: "provider.azure-openai.base_url",
    defaultBaseUrl: "",
    needsAddress: true,
  },
  {
    name: "openrouter",
    title: "OpenRouter",
    kind: "key",
    blurb: "One key for hundreds of models from many vendors.",
    keyUrl: "https://openrouter.ai/keys",
    keySite: "openrouter.ai",
    keyHint: "sk-or-…",
    baseUrlSetting: "provider.openrouter.base_url",
    defaultBaseUrl: "https://openrouter.ai/api/v1",
  },
  {
    name: "groq",
    title: "Groq",
    kind: "key",
    blurb: "Open models on very fast hardware.",
    keyUrl: "https://console.groq.com/keys",
    keySite: "console.groq.com",
    keyHint: "gsk_…",
    baseUrlSetting: "provider.groq.base_url",
    defaultBaseUrl: "https://api.groq.com/openai/v1",
  },
  {
    name: "mistral",
    title: "Mistral",
    kind: "key",
    blurb: "Mistral's own models, including Codestral for code.",
    keyUrl: "https://console.mistral.ai/api-keys",
    keySite: "console.mistral.ai",
    baseUrlSetting: "provider.mistral.base_url",
    defaultBaseUrl: "https://api.mistral.ai/v1",
  },
  {
    name: "deepseek",
    title: "DeepSeek",
    kind: "key",
    blurb: "DeepSeek's chat and reasoning models at a low price.",
    keyUrl: "https://platform.deepseek.com/api_keys",
    keySite: "platform.deepseek.com",
    keyHint: "sk-…",
    baseUrlSetting: "provider.deepseek.base_url",
    defaultBaseUrl: "https://api.deepseek.com/v1",
  },
  {
    name: "xai",
    title: "xAI Grok",
    kind: "key",
    blurb: "xAI's Grok models.",
    keyUrl: "https://console.x.ai",
    keySite: "console.x.ai",
    keyHint: "xai-…",
    baseUrlSetting: "provider.xai.base_url",
    defaultBaseUrl: "https://api.x.ai/v1",
  },
  {
    name: "together",
    title: "Together AI",
    kind: "key",
    blurb: "Open models such as Llama, Qwen and DeepSeek in the cloud.",
    keyUrl: "https://api.together.ai/settings/api-keys",
    keySite: "together.ai",
    baseUrlSetting: "provider.together.base_url",
    defaultBaseUrl: "https://api.together.xyz/v1",
  },
  {
    name: "fireworks",
    title: "Fireworks AI",
    kind: "key",
    blurb: "Fast hosted open models.",
    keyUrl: "https://fireworks.ai/account/api-keys",
    keySite: "fireworks.ai",
    keyHint: "fw_…",
    baseUrlSetting: "provider.fireworks.base_url",
    defaultBaseUrl: "https://api.fireworks.ai/inference/v1",
  },
  {
    name: "cerebras",
    title: "Cerebras",
    kind: "key",
    blurb: "Open models on Cerebras chips, built for speed.",
    keyUrl: "https://cloud.cerebras.ai",
    keySite: "cloud.cerebras.ai",
    keyHint: "csk-…",
    baseUrlSetting: "provider.cerebras.base_url",
    defaultBaseUrl: "https://api.cerebras.ai/v1",
  },
  {
    name: "ollama",
    title: "Ollama",
    kind: "local",
    blurb: "Models running on this computer with Ollama. Free and private; no key.",
    baseUrlSetting: "provider.ollama.base_url",
    defaultBaseUrl: "http://127.0.0.1:11434/v1",
  },
  {
    name: "lmstudio",
    title: "LM Studio",
    kind: "local",
    blurb: "Models running on this computer with LM Studio. Free and private; no key.",
    baseUrlSetting: "provider.lmstudio.base_url",
    defaultBaseUrl: "http://127.0.0.1:1234/v1",
  },
  {
    name: "vllm",
    title: "vLLM",
    kind: "local",
    blurb: "Your own vLLM server. No key unless you set one.",
    baseUrlSetting: "provider.vllm.base_url",
    defaultBaseUrl: "http://127.0.0.1:8000/v1",
  },
  {
    name: "custom",
    title: "Another OpenAI-compatible server",
    kind: "custom",
    blurb: "A gateway, a proxy or any server that speaks the OpenAI API.",
    keyHint: "the server's key",
    baseUrlSetting: "provider.custom.base_url",
    defaultBaseUrl: "",
    needsAddress: true,
  },
];

const BY_NAME = new Map(PROVIDERS.map((p) => [p.name, p]));

// The catalog entry for a provider, or one built from the hub's label for a
// provider this console doesn't know yet (a key provider by default).
export function providerInfo(name: string, label?: string): ProviderInfo {
  return BY_NAME.get(name) ?? { name, title: label || name, kind: "key", blurb: "" };
}

// The picker's groups, in order, with the words that head each one.
export const PROVIDER_GROUPS: { kind: ProviderKind; title: string; note: string }[] = [
  { kind: "account", title: "Sign in with an account", note: "No key to copy: approve Hive in your browser." },
  { kind: "key", title: "Use an API key", note: "Paste a key from the provider's site. It stays on this machine." },
  { kind: "local", title: "Run models on this computer", note: "Free and private. The model server must be running." },
  { kind: "custom", title: "Something else", note: "" },
];

export interface HubProvider {
  name: string;
  label: string;
  device_login: boolean;
  token_login: boolean;
}

// The hub's providers sorted into the picker's groups, in catalog order
// (the hub lists them alphabetically); unknown ones go last in their group.
// A query narrows by name, title, blurb or group.
export function groupProviders(hub: HubProvider[], query = ""): { kind: ProviderKind; title: string; note: string; items: { hub: HubProvider; info: ProviderInfo }[] }[] {
  const q = query.trim().toLowerCase();
  const order = (name: string) => {
    const i = PROVIDERS.findIndex((p) => p.name === name);
    return i < 0 ? PROVIDERS.length : i;
  };
  // A key provider whose hub also offers a browser sign-in (Anthropic, once
  // its CLI sign-in is in) belongs with the accounts: no key to copy.
  const rows = hub
    .map((h) => {
      const info = providerInfo(h.name, h.label);
      return { hub: h, info: h.device_login && info.kind === "key" ? { ...info, kind: "account" as const } : info };
    })
    .filter(({ hub: h, info }) => !q || [h.name, h.label, info.title, info.blurb, info.kind === "local" ? "local offline free private" : ""].join(" ").toLowerCase().includes(q))
    .sort((a, b) => order(a.hub.name) - order(b.hub.name));
  return PROVIDER_GROUPS.map((g) => ({ ...g, items: rows.filter((r) => r.info.kind === g.kind) })).filter((g) => g.items.length);
}

// A pasted key that is clearly the wrong thing, caught before it reaches
// the provider: the sentence to show, or "".
export function keyProblem(name: string, key: string): string {
  const k = key.trim();
  if (!k) return "";
  if (/\s/.test(k)) return "A key has no spaces. Copy it again, without the words around it.";
  if (name === "anthropic") {
    if (k.startsWith("sk-ant-oat"))
      return "That's a Claude Pro or Max sign-in token (from claude setup-token). Anthropic lets only its own apps use those. Paste a Console API key (sk-ant-api…) or use Sign in with Anthropic.";
    if (k.startsWith("sk-ant-admin")) return "That's an Anthropic admin key, which can't call models. Create a regular API key in the Console.";
    if (/^(sk-proj-|sk-or-|gsk_|AIza|xai-|gh[opsu]_|github_pat_)/.test(k)) return "That key is for another provider. Anthropic keys start with sk-ant-.";
  }
  if (name === "openai" && /^(sk-ant-|sk-or-|gsk_|AIza|xai-)/.test(k)) return "That key is for another provider. OpenAI keys start with sk-.";
  if (name === "gemini" && /^(sk-|gsk_|xai-)/.test(k)) return "That key is for another provider. Google AI Studio keys start with AIza.";
  if (name === "openrouter" && /^(sk-ant-|sk-proj-|gsk_|AIza)/.test(k)) return "That key is for another provider. OpenRouter keys start with sk-or-.";
  if ((name === "copilot" || name === "github-models") && /^(sk-|AIza|gsk_)/.test(k)) return "That's not a GitHub token. GitHub tokens start with gho_, ghp_ or github_pat_.";
  return "";
}

// The pick for a role from a provider's model list: the current id when the
// provider lists it, else the provider's default, else the first chat model.
export function suggestModel<M extends { id: string; default?: boolean; chat?: boolean }>(models: M[], current: string): string {
  if (current && models.some((m) => m.id === current)) return current;
  return (models.find((m) => m.default) ?? models.find((m) => m.chat !== false) ?? models[0])?.id ?? "";
}

// A cheaper, faster sibling for the light jobs, when the list has one by a
// well-known name (haiku, mini, flash, small); else "" (same as the main).
export function suggestLightModel<M extends { id: string }>(models: M[], current: string): string {
  if (current && models.some((m) => m.id === current)) return current;
  const light = /(haiku|mini|flash|small|lite|nano)/i;
  return models.find((m) => light.test(m.id))?.id ?? "";
}
