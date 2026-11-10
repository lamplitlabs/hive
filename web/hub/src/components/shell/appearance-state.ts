// Appearance: the three choices under Settings > Appearance that change how
// the console reads rather than what it shows - density (comfortable rows or
// compact ones), font (Geist or the Readable option, Atkinson Hyperlegible)
// and text size (14px or 16px). Pure, so appearance-state.test.mjs can drive
// it; the Settings page owns the React hook and the real localStorage.
//
// Each choice is this browser's, like the theme and the sidebar, so it lives
// in localStorage. The look itself is CSS: <html> carries data-density,
// data-font and data-text and globals.css styles each one. A default carries
// no attribute and no stored key, so a cleared storage and a never-touched
// one read the same. A tiny script in the document head (APPEARANCE_BOOT_SCRIPT)
// sets the attributes before the first paint, the way next-themes sets the
// theme, so a reload never shows the default for a frame and then reflows.

export const APPEARANCE_KEYS = {
  density: "hive.density",
  font: "hive.font",
  text: "hive.text",
} as const;

export type Density = "comfortable" | "compact";
export type FontChoice = "geist" | "readable";
export type TextSize = "default" | "larger";

export interface Appearance {
  density: Density;
  font: FontChoice;
  text: TextSize;
}

export const DEFAULT_APPEARANCE: Appearance = {
  density: "comfortable",
  font: "geist",
  text: "default",
};

// The values each choice accepts; anything else stored reads as the default.
export const APPEARANCE_VALUES: { [K in keyof Appearance]: readonly Appearance[K][] } = {
  density: ["comfortable", "compact"],
  font: ["geist", "readable"],
  text: ["default", "larger"],
};

// The subset of Storage the appearance uses, so tests can pass a Map-backed one.
export interface AppearanceReadStorage {
  getItem(key: string): string | null;
}
export interface AppearanceWriteStorage {
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

// The subset of HTMLElement touched on <html>: its dataset, where
// data-density / data-font / data-text live.
export interface AppearanceRoot {
  dataset: Record<string, string | undefined>;
}

const CHOICES = Object.keys(APPEARANCE_KEYS) as (keyof Appearance)[];

function pick<K extends keyof Appearance>(choice: K, value: string | null | undefined): Appearance[K] {
  const allowed = APPEARANCE_VALUES[choice] as readonly string[];
  return value != null && allowed.includes(value) ? (value as Appearance[K]) : DEFAULT_APPEARANCE[choice];
}

// Reads the three choices; a missing storage (SSR, disabled) or an unknown
// stored word gives the default for that choice.
export function readAppearance(storage: AppearanceReadStorage | null | undefined): Appearance {
  const out: Appearance = { ...DEFAULT_APPEARANCE };
  if (!storage) return out;
  for (const choice of CHOICES) {
    let raw: string | null = null;
    try {
      raw = storage.getItem(APPEARANCE_KEYS[choice]);
    } catch {
      raw = null;
    }
    (out as Record<keyof Appearance, string>)[choice] = pick(choice, raw);
  }
  return out;
}

// Marks <html>: a non-default choice sets its data attribute, a default
// removes it, so the CSS selectors stay `html[data-x="value"]` for the
// non-default states alone.
export function applyAppearance(root: AppearanceRoot, appearance: Appearance): void {
  for (const choice of CHOICES) {
    const value = appearance[choice];
    if (value === DEFAULT_APPEARANCE[choice]) delete root.dataset[choice];
    else root.dataset[choice] = value;
  }
}

// Stores the three choices; a default removes its key rather than writing it.
export function writeAppearance(storage: AppearanceWriteStorage, appearance: Appearance): void {
  for (const choice of CHOICES) {
    const value = appearance[choice];
    if (value === DEFAULT_APPEARANCE[choice]) storage.removeItem(APPEARANCE_KEYS[choice]);
    else storage.setItem(APPEARANCE_KEYS[choice], value);
  }
}

// Runs inline in <head> before React: reads each choice and marks <html>.
// Storage can throw (private mode, disabled); then the defaults stand. Only a
// known non-default word is applied, so a stale or foreign value is ignored.
export const APPEARANCE_BOOT_SCRIPT = `(function(){try{var d=document.documentElement.dataset;var c=${JSON.stringify(
  CHOICES.map((choice) => [
    APPEARANCE_KEYS[choice],
    choice,
    APPEARANCE_VALUES[choice].filter((value) => value !== DEFAULT_APPEARANCE[choice]),
  ]),
)};for(var i=0;i<c.length;i++){var v=localStorage.getItem(c[i][0]);if(v&&c[i][2].indexOf(v)>=0)d[c[i][1]]=v}}catch(e){}})()`;
