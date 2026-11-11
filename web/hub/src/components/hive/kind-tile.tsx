"use client";

import {
  Activity,
  BookOpen,
  Brain,
  CalendarClock,
  FileCheck,
  FileText,
  Folder,
  Gavel,
  Lightbulb,
  ListChecks,
  Map,
  MessageCircleQuestion,
  MessagesSquare,
  Package,
  Plug,
  Scale,
  Server,
  Sparkles,
  SquareCheck,
  Vote,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { KIND_LABEL, KIND_TINT, TINT_CLASS, tintFor, type Kind, type Tint } from "./kind-tile-state";

export { kindFor, tintFor, isKind, KINDS, KIND_TINT, KIND_LABEL, TINT_CLASS } from "./kind-tile-state";
export type { Kind, Tint } from "./kind-tile-state";

// One icon and one tint per kind of object, everywhere (spec "Icons and
// logos"). `tint` is the resting tint; a waiting decision turns marigold
// through tintFor().
export const KIND_ICON: Record<Kind, LucideIcon> = {
  session: MessagesSquare,
  task: ListChecks,
  decision: Scale,
  proposal: FileCheck,
  question: MessageCircleQuestion,
  suggestion: Lightbulb,
  rule: Gavel,
  run: Activity,
  machine: Server,
  tool: Plug,
  skill: Sparkles,
  automation: CalendarClock,
  memory: Brain,
  plan: Map,
  guide: BookOpen,
  feature: Package,
  work: SquareCheck,
  record: FileText,
  vote: Vote,
  project: Folder,
};

export const KIND: Record<Kind, { icon: LucideIcon; tint: Tint; label: string }> = Object.fromEntries(
  (Object.keys(KIND_ICON) as Kind[]).map((k) => [k, { icon: KIND_ICON[k], tint: KIND_TINT[k], label: KIND_LABEL[k] }]),
) as Record<Kind, { icon: LucideIcon; tint: Tint; label: string }>;

const TILE_SIZE = {
  20: "size-5 rounded-[5px]",
  28: "size-7 rounded-md",
  40: "size-10 rounded-md",
} as const;
const ICON_SIZE = {
  20: "size-3",
  28: "size-4",
  40: "size-5",
} as const;

// A rounded square with the kind's tinted wash and its icon in the kind's
// ink. 28 px in lists, 20 px inline, 40 px in a panel header. `icon`
// overrides the glyph (a tool's logo in place of Plug); `waiting` turns a
// decision marigold.
export function KindTile({
  kind,
  size = 28,
  waiting,
  icon,
  className,
  title,
}: {
  kind: Kind;
  size?: 20 | 28 | 40;
  waiting?: boolean;
  icon?: LucideIcon | React.ReactNode;
  className?: string;
  title?: string;
}) {
  const tint = tintFor(kind, waiting);
  const label = title ?? KIND_LABEL[kind];
  let glyph: React.ReactNode;
  if (icon === undefined || icon === null) {
    const Icon = KIND_ICON[kind];
    glyph = <Icon className={ICON_SIZE[size]} strokeWidth={1.75} aria-hidden />;
  } else if (typeof icon === "function" || (typeof icon === "object" && "render" in (icon as object))) {
    const Icon = icon as LucideIcon;
    glyph = <Icon className={ICON_SIZE[size]} strokeWidth={1.75} aria-hidden />;
  } else {
    glyph = icon as React.ReactNode;
  }
  return (
    <span
      className={cn("inline-flex shrink-0 items-center justify-center", TILE_SIZE[size], TINT_CLASS[tint], className)}
      role="img"
      aria-label={label}
      title={label}
      data-kind={kind}
    >
      {glyph}
    </span>
  );
}

// The kind's icon alone, in the kind's ink, for a place that already has a
// surface (a heading, a chip).
export function KindIcon({ kind, waiting, className }: { kind: Kind; waiting?: boolean; className?: string }) {
  const Icon = KIND_ICON[kind];
  const tint = tintFor(kind, waiting);
  return <Icon className={cn("size-4 shrink-0", TINT_CLASS[tint].split(" ")[1], className)} strokeWidth={1.75} aria-hidden />;
}
