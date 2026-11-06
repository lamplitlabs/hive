"use client";

import { Server } from "lucide-react";
import { cn } from "@/lib/utils";
import { LOGOS, logoFor, type LogoId } from "@/lib/logos";
import { initialsOf } from "./avatar-state";

export { logoFor, LOGOS, LOGO_IDS, isLogoId } from "@/lib/logos";
export type { LogoId } from "@/lib/logos";

const LOGO_SIZE = { 16: "size-4", 20: "size-5", 24: "size-6" } as const;

// A brand mark as an inline SVG (spec "Logo rules"): one colour - the text
// colour - at 16 to 20 px inside a neutral tile; full brand colour only in
// the provider wizard and on connection cards (`color="brand"`).
export function BrandLogo({ id, size = 16, color = "mono", className, title }: { id: LogoId; size?: 16 | 20 | 24; color?: "mono" | "brand"; className?: string; title?: string }) {
  const logo = LOGOS[id];
  const label = title ?? logo.title;
  return (
    <svg viewBox="0 0 24 24" role="img" aria-label={label} className={cn("shrink-0", LOGO_SIZE[size], className)} fill={color === "brand" ? `#${logo.hex}` : "currentColor"} data-logo={id}>
      <title>{label}</title>
      <path d={logo.path} />
    </svg>
  );
}

const TILE = {
  28: { tile: "size-7 rounded-md text-[11px]", logo: 16 as const },
  40: { tile: "size-10 rounded-md text-sm", logo: 20 as const },
} as const;

// A neutral tile holding a brand mark: the shape a tool connection, a
// provider or a machine takes in a list.
export function LogoTile({ name, id, size = 28, color = "mono", fallback, className, title }: { name?: string | null; id?: LogoId | null; size?: 28 | 40; color?: "mono" | "brand"; fallback?: React.ReactNode; className?: string; title?: string }) {
  const logo = id ?? logoFor(name);
  const label = title ?? (logo ? LOGOS[logo].title : name || "");
  if (!logo) {
    if (fallback !== undefined) return <span className={cn("inline-flex shrink-0 items-center justify-center bg-muted text-muted-foreground", TILE[size].tile, className)} title={label}>{fallback}</span>;
    return <MonogramTile name={name} size={size} className={className} title={title} />;
  }
  return (
    <span className={cn("inline-flex shrink-0 items-center justify-center bg-muted text-foreground", TILE[size].tile, className)} title={label} data-logo={logo}>
      <BrandLogo id={logo} size={TILE[size].logo} color={color} title={label} />
    </span>
  );
}

// The tile for a thing with no mark: its initials on slate.
export function MonogramTile({ name, size = 28, className, title }: { name?: string | null; size?: 28 | 40; className?: string; title?: string }) {
  const label = title ?? name ?? "";
  return (
    <span className={cn("inline-flex shrink-0 select-none items-center justify-center bg-kind-slate-soft font-semibold leading-none tracking-tight text-kind-slate", TILE[size].tile, className)} role="img" aria-label={label || undefined} title={label || undefined}>
      {initialsOf(name)}
    </span>
  );
}

// A machine by its system: Apple, Ubuntu, Debian or Linux marks, else the
// Server icon. Mono, 16 px, for a row lead or beside a hostname.
export function MachineLogo({ os, size = 16, className }: { os?: string | null; size?: 16 | 20 | 24; className?: string }) {
  const id = logoFor(os);
  if (id === "apple" || id === "ubuntu" || id === "debian" || id === "linux") return <BrandLogo id={id} size={size} className={className} title={os || undefined} />;
  return <Server className={cn("shrink-0", LOGO_SIZE[size], className)} strokeWidth={1.75} aria-label={os || "Machine"} role="img" />;
}
