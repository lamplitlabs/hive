// Project and owner avatars: the initials and the hue, with no React so
// node --test covers them.

// One or two letters for a name: "demo-app" -> "DA", "pulse" -> "P",
// "Amistio Template" -> "AT", "x_y_z" -> "XY". Digits count as letters;
// an empty name gives "?".
export function initialsOf(name: string | null | undefined): string {
  const words = (name || "")
    .trim()
    .split(/[\s\-_./:]+/)
    .map((w) => w.replace(/[^\p{L}\p{N}]/gu, ""))
    .filter(Boolean);
  if (words.length === 0) return "?";
  if (words.length === 1) {
    const w = words[0];
    // A single camelCase word takes its first letter and its first inner
    // capital ("demoApp" -> "DA"); an all-caps word is one word ("PULSE" -> "P").
    const inner = w.slice(1).match(/\p{Lu}/gu) ?? [];
    if (inner.length >= 1 && inner.length < w.length - 1) return (w[0] + inner[0]).toUpperCase();
    return w.slice(0, 1).toUpperCase();
  }
  return (words[0].slice(0, 1) + words[1].slice(0, 1)).toUpperCase();
}

// Eight pleasant hues (oklch degrees) spaced around the wheel, skipping the
// muddy yellow-greens: teal, blue, indigo, violet, pink, coral, amber, green.
export const AVATAR_HUES = [185, 235, 265, 300, 345, 25, 70, 145] as const;

// A stable 32-bit hash (FNV-1a) of the slug so one project keeps one colour.
export function hashOf(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h >>> 0;
}

export function hueOf(slug: string | null | undefined): number {
  const key = (slug || "").trim().toLowerCase();
  if (!key) return AVATAR_HUES[0];
  return AVATAR_HUES[hashOf(key) % AVATAR_HUES.length];
}

// The avatar's colours as CSS: a mid-light background in the hue and ink
// chosen for contrast. Every hue sits at L=0.52 C=0.12, dark enough for
// white text (>= 4.5:1 against white at that lightness for all 8 hues
// except amber, which gets ink text).
export function avatarColors(slug: string | null | undefined): { background: string; color: string } {
  const h = hueOf(slug);
  const light = h === 70; // amber reads better with dark ink
  return {
    background: light ? `oklch(0.82 0.13 ${h})` : `oklch(0.52 0.12 ${h})`,
    color: light ? "oklch(0.26 0.04 262)" : "#fff",
  };
}
