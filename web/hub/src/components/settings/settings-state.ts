// Pure helpers of the Settings page (spec "Settings"): the lucide icon name
// of each group's heading, a model's family badge and its context-window
// chip. No React, no "@/" imports, so settings-state.test.mjs can drive it
// under node --test.

// The icon on each group's SectionHeading, by group id. The page maps the
// name to the lucide component; unknown ids get the generic sliders.
export type GroupIcon = "Settings2" | "Palette" | "Cpu" | "Play" | "Bell" | "KeyRound" | "Cloud" | "ShieldAlert" | "Gauge" | "Server" | "Sparkles" | "Plug" | "SlidersHorizontal" | "UserRound";

const GROUP_ICON: Record<string, GroupIcon> = {
  general: "Settings2",
  owner: "UserRound",
  appearance: "Palette",
  harness: "Cpu",
  daemon: "Play",
  running: "Play",
  notify: "Bell",
  notifications: "Bell",
  credential: "KeyRound",
  credentials: "KeyRound",
  secrets: "KeyRound",
  cloud: "Cloud",
  compute: "Cloud",
  stop: "ShieldAlert",
  danger: "ShieldAlert",
  runner: "Gauge",
  evolve: "Sparkles",
  provider: "Plug",
  other: "SlidersHorizontal",
};

export function groupIcon(id: string): GroupIcon {
  return GROUP_ICON[id] ?? "SlidersHorizontal";
}

// The family badge of a model picker: the word people know the model by.
export type ModelFamily = "Opus" | "Sonnet" | "Haiku" | "GPT" | "Gemini" | "Llama" | "Other";

export function modelFamily(modelId: string | null | undefined): ModelFamily {
  const id = (modelId || "").toLowerCase();
  if (!id) return "Other";
  if (id.includes("opus")) return "Opus";
  if (id.includes("sonnet")) return "Sonnet";
  if (id.includes("haiku")) return "Haiku";
  if (/(^|[^a-z])(gpt|o[1-9](-|$))/.test(id)) return "GPT";
  if (id.includes("gemini")) return "Gemini";
  if (id.includes("llama")) return "Llama";
  return "Other";
}

// The context-window chip: tokens as "200k" or "1M"; nothing for 0/unknown.
export function contextChip(tokens: number | null | undefined): string {
  if (!tokens || tokens <= 0 || !Number.isFinite(tokens)) return "";
  if (tokens >= 1_000_000) {
    const m = tokens / 1_000_000;
    return (Number.isInteger(m) ? m : Number(m.toFixed(1))) + "M";
  }
  if (tokens >= 1000) return Math.round(tokens / 1000) + "k";
  return String(tokens);
}
