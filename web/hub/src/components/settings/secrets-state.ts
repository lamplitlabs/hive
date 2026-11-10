// Pure helpers of Settings → Secrets, the team secret vault (ADR-070 rule 4,
// crates/pulse-hub/src/secrets_api.rs). The page lists names, who set each
// and when; it never shows a value back. No React, no "@/" imports, so
// secrets-state.test.mjs can drive it under node --test.

// A secret name as the vault and a recipe's `secrets:` take it: the
// variable the job's worker sees, so upper case like an env name. Mirrors
// pulse-hand-core vault.rs name_problem.
const NAME_RE = /^[A-Z_][A-Z0-9_]*$/;
const TAKEN = new Set(["PATH", "HOME", "TMPDIR", "GIT_DIR", "GIT_INDEX_FILE"]);

// Why the typed name cannot be saved, in plain words; "" when it can.
export function secretNameProblem(name: string): string {
  const n = name.trim();
  if (!n) return "";
  if (n.length > 64) return "At most 64 characters.";
  if (!NAME_RE.test(n)) return "Use upper-case letters, digits and _, starting with a letter or _ (like GITHUB_TOKEN).";
  if (n.startsWith("PULSE_") || TAKEN.has(n)) return "PULSE sets " + n + " for every job; pick another name.";
  return "";
}

// What a typed name becomes: trimmed, spaces and dashes to _, upper case,
// so "npm token" reads as NPM_TOKEN before the owner saves.
export function normalizeSecretName(name: string): string {
  return name.trim().replace(/[\s-]+/g, "_").toUpperCase();
}

// Whether the form may save: a valid name and a value of 1 byte to max.
export function canSaveSecret(name: string, value: string, maxBytes: number): boolean {
  if (!name.trim() || secretNameProblem(name)) return false;
  const bytes = new TextEncoder().encode(value).length;
  return bytes > 0 && (maxBytes <= 0 || bytes <= maxBytes);
}

// The one line under a secret's name: who set it last and the dot of
// time, "set by you" for the owner's own writes.
export function setByWords(updatedBy: string, owner = "human"): string {
  const who = !updatedBy || updatedBy === owner ? "you" : updatedBy;
  return "set by " + who;
}

// The masked value a row shows in place of the secret: never derived from
// the value, so nothing about it (not even its length) leaks to the page.
export const MASK = "••••••••";

// The line under the heading: how many secrets, and how a job gets one.
export function vaultNote(count: number): string {
  const lead =
    count === 0
      ? "No secrets yet."
      : count === 1
        ? "1 secret."
        : count + " secrets.";
  return lead + " To give a project's jobs one, list its name under secrets: in that project's environment recipe.";
}
