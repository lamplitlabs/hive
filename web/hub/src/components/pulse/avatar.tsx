"use client";

import { cn } from "@/lib/utils";
import { avatarColors, initialsOf } from "./avatar-state";

export { initialsOf, hueOf, avatarColors } from "./avatar-state";

// Projects get clean avatars instead of folder faces (spec "Icons and
// logos"): a rounded square with 1-2 initials on a hue hashed from the slug,
// or the project's own image when it has one. Agents keep their robot faces
// (who.tsx Avatar).
const PROJECT_SIZE = {
  20: "size-5 rounded-[5px] text-[9px]",
  32: "size-8 rounded-md text-xs",
  48: "size-12 rounded-lg text-base",
} as const;

export function ProjectAvatar({ slug, name, size = 32, src, className }: { slug: string; name?: string | null; size?: 20 | 32 | 48; src?: string | null; className?: string }) {
  const label = name || slug;
  const base = cn("inline-flex shrink-0 select-none items-center justify-center overflow-hidden font-semibold leading-none tracking-tight", PROJECT_SIZE[size], className);
  if (src) {
    return (
      // eslint-disable-next-line @next/next/no-img-element -- static export, local or remote image of the owner's choosing
      <img src={src} alt={label} title={label} width={size} height={size} className={cn(base, "object-cover")} />
    );
  }
  return (
    <span className={base} style={avatarColors(slug)} role="img" aria-label={label} title={label} data-slug={slug}>
      {initialsOf(label)}
    </span>
  );
}

// The owner: round, initials on ink. One person, one mark.
const OWNER_SIZE = {
  20: "size-5 text-[9px]",
  24: "size-6 text-[10px]",
  32: "size-8 text-xs",
  48: "size-12 text-base",
} as const;

export function OwnerAvatar({ name, size = 32, src, className }: { name?: string | null; size?: 20 | 24 | 32 | 48; src?: string | null; className?: string }) {
  const label = name || "You";
  const base = cn("inline-flex shrink-0 select-none items-center justify-center overflow-hidden rounded-full font-semibold leading-none", OWNER_SIZE[size], className);
  if (src) {
    return (
      // eslint-disable-next-line @next/next/no-img-element -- static export
      <img src={src} alt={label} title={label} width={size} height={size} className={cn(base, "object-cover")} />
    );
  }
  return (
    <span className={cn(base, "bg-primary text-primary-foreground")} role="img" aria-label={label} title={label}>
      {initialsOf(label)}
    </span>
  );
}
