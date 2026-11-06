"use client";

import { useMemo, useSyncExternalStore } from "react";
import { useTheme } from "next-themes";
import { SectionHeading } from "@/components/pulse/item-card";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { applyAppearance, DEFAULT_APPEARANCE, readAppearance, writeAppearance, type Appearance } from "@/components/shell/appearance-state";

// Settings > Appearance: how the console reads in this browser - the colour
// theme (next-themes), density, font and text size (appearance-state). None
// of it is a PULSE setting: each choice lives in localStorage and takes
// effect at once, so the rows have no Save and no confirm.
//
// `icon` is the lucide component the page picks for the heading (groupIcon
// "Palette" mapped in page.tsx) so this file stays free of the icon map.

type Icon = React.ComponentType<{ className?: string; strokeWidth?: number; "aria-hidden"?: boolean }>;

const subscribe = () => () => {};
const getClientSnapshot = () => true;
const getServerSnapshot = () => false;

// The stored choices as an external store: the snapshot is a string (stable
// by value, so useSyncExternalStore never loops), rebuilt when this section
// writes a choice or another tab changes localStorage. The server snapshot
// is the defaults, which is also what the inline script applied before paint.
const listeners = new Set<() => void>();
const keyOf = (a: Appearance) => a.density + " " + a.font + " " + a.text;
const SERVER_KEY = keyOf(DEFAULT_APPEARANCE);
function subscribeAppearance(cb: () => void) {
  listeners.add(cb);
  window.addEventListener("storage", cb);
  return () => {
    listeners.delete(cb);
    window.removeEventListener("storage", cb);
  };
}
const readKey = () => keyOf(readAppearance(window.localStorage));
const serverKey = () => SERVER_KEY;
function parseKey(key: string): Appearance {
  const [density, font, text] = key.split(" ") as [Appearance["density"], Appearance["font"], Appearance["text"]];
  return { density, font, text };
}

// One segmented choice: the label and help on the left, the control right.
function Choice<T extends string>({
  id,
  label,
  help,
  value,
  options,
  onChange,
}: {
  id: string;
  label: string;
  help: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
}) {
  return (
    <div className="grid gap-3 px-4 py-3 sm:px-5 md:grid-cols-[minmax(0,1fr)_auto] md:items-center md:gap-x-8">
      <div className="flex min-w-0 flex-col gap-1">
        <span id={id} className="text-sm font-medium leading-snug text-foreground">
          {label}
        </span>
        <p className="max-w-2xl text-[13px] leading-relaxed text-muted-foreground">{help}</p>
      </div>
      <Tabs value={value} onValueChange={(v) => onChange(v as T)} className="md:justify-self-end">
        <TabsList variant="segmented" aria-labelledby={id}>
          {options.map((o) => (
            <TabsTrigger key={o.value} value={o.value}>
              {o.label}
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>
    </div>
  );
}

export function AppearanceSection({ icon }: { icon?: Icon }) {
  const mounted = useSyncExternalStore(subscribe, getClientSnapshot, getServerSnapshot);
  const { theme, setTheme } = useTheme();
  const key = useSyncExternalStore(subscribeAppearance, readKey, serverKey);
  const appearance = useMemo(() => parseKey(key), [key]);

  const change = (patch: Partial<Appearance>) => {
    const next = { ...appearance, ...patch };
    writeAppearance(window.localStorage, next);
    applyAppearance(document.documentElement, next);
    for (const cb of listeners) cb();
  };

  const themeValue = mounted ? theme || "system" : "system";

  return (
    <>
      <SectionHeading icon={icon} note="How the console looks in this browser. Each choice takes effect at once.">
        Appearance
      </SectionHeading>
      <div className="min-w-0 divide-y overflow-hidden rounded-lg border bg-card">
        <Choice
          id="appearance-theme"
          label="Theme"
          help="System follows your device. Light and Dark stay put."
          value={themeValue}
          options={[
            { value: "system", label: "System" },
            { value: "light", label: "Light" },
            { value: "dark", label: "Dark" },
          ]}
          onChange={setTheme}
        />
        <Choice
          id="appearance-density"
          label="Density"
          help="Compact tightens rows and lists so more fits on the screen."
          value={appearance.density}
          options={[
            { value: "comfortable", label: "Comfortable" },
            { value: "compact", label: "Compact" },
          ]}
          onChange={(density) => change({ density })}
        />
        <Choice
          id="appearance-font"
          label="Font"
          help="Readable is Atkinson Hyperlegible, shaped so letters are hard to confuse."
          value={appearance.font}
          options={[
            { value: "geist", label: "Geist" },
            { value: "readable", label: "Readable" },
          ]}
          onChange={(font) => change({ font })}
        />
        <Choice
          id="appearance-text"
          label="Text size"
          help="Larger sets the base text to 16 px instead of 14."
          value={appearance.text}
          options={[
            { value: "default", label: "Default" },
            { value: "larger", label: "Larger" },
          ]}
          onChange={(text) => change({ text })}
        />
      </div>
    </>
  );
}
