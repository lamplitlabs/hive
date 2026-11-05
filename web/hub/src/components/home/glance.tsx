"use client";

// The strip under the greeting: four quiet tiles that answer the owner's
// first four questions in one glance - is anything waiting for me, is anyone
// working, is the machine alright, how are my projects. Each tile is a link
// to where the answer lives. The words come from glance-state.ts.
import Link from "next/link";
import { ArrowRightIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { glanceTiles, type GlanceInput, type GlanceTile, type GlanceTone } from "./glance-state";

const DOT: Record<GlanceTone, string> = {
  neutral: "bg-muted-foreground/40",
  ok: "bg-ok",
  warn: "bg-warn",
  bad: "bg-bad",
  info: "bg-primary",
  accent: "bg-accent-2",
  you: "bg-you",
};

const VALUE: Record<GlanceTone, string> = {
  neutral: "text-foreground",
  ok: "text-foreground",
  warn: "text-warn",
  bad: "text-bad",
  info: "text-foreground",
  accent: "text-foreground",
  you: "text-foreground",
};

const tile = "group/tile flex min-w-0 flex-col gap-1 rounded-2xl border bg-card px-4 py-3.5 text-left transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none";

function Tile({ t, href, onClick }: { t: GlanceTile; href?: string; onClick?: () => void }) {
  const body = (
    <>
      <span className="flex items-center justify-between gap-2 text-[13px] leading-snug text-muted-foreground">
        <span className="min-w-0 truncate">{t.label}</span>
        <ArrowRightIcon className="size-3.5 shrink-0 opacity-0 transition-opacity group-hover/tile:opacity-100 group-focus-visible/tile:opacity-100 motion-reduce:transition-none" aria-hidden />
      </span>
      <span className="flex min-w-0 items-center gap-2">
        <span className={cn("size-2 shrink-0 rounded-full", DOT[t.tone])} aria-hidden />
        <span key={t.value} className={cn("tick min-w-0 truncate text-[22px] font-semibold leading-tight tracking-tight", VALUE[t.tone])} title={t.value}>
          {t.value}
        </span>
      </span>
      <span className="min-w-0 truncate text-xs leading-relaxed text-muted-foreground" title={t.sub}>
        {t.sub}
      </span>
    </>
  );
  const highlight = t.tone === "you" ? "surface-you" : "";
  if (onClick)
    return (
      <button type="button" onClick={onClick} className={cn(tile, highlight)} aria-label={t.label + ": " + t.value + ". " + t.sub}>
        {body}
      </button>
    );
  return (
    <Link href={href ?? "/"} className={cn(tile, highlight)} aria-label={t.label + ": " + t.value + ". " + t.sub}>
      {body}
    </Link>
  );
}

export function GlanceStrip({ input, hrefs, onRunner, className }: { input: GlanceInput; hrefs: { waiting: string; working: string; projects: string }; onRunner: () => void; className?: string }) {
  const tiles = glanceTiles(input);
  return (
    <div className={cn("stagger grid min-w-0 grid-cols-2 gap-3 xl:grid-cols-4", className)} aria-label="At a glance">
      {tiles.map((t) => (t.id === "runner" ? <Tile key={t.id} t={t} onClick={onRunner} /> : <Tile key={t.id} t={t} href={t.id === "waiting" ? hrefs.waiting : t.id === "working" ? hrefs.working : hrefs.projects} />))}
    </div>
  );
}
