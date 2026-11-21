"use client";

import Link from "next/link";
import { createContext, useCallback, useContext, useRef } from "react";
import { cn } from "@/lib/utils";
import { Skeleton } from "@/components/ui/skeleton";
import { inTextField, nextIndex, openerName, rowKey } from "./list-row-state";

export { nextIndex, rowKey, opensRow } from "./list-row-state";

// One item in a list (spec "Components" - List row): a 44 px row (36 in
// compact density) with a kind tile or avatar on the left, a title, a few
// chips of meta, and status and time on the right. Actions show on hover or
// focus (always, on a touch screen). The whole row opens the item.
//
// What opens it, for the keyboard and for a screen reader, is the title: a
// link when the row has `href`, else a button that calls `onOpen` (the
// "opener", marked data-row-open). The row around it is a plain box, so the
// buttons and links a row carries (Accept, Decline, a chip's link) are
// beside the opener and never inside it: a control inside a control has no
// place in a page's outline, and its words would be read as part of the
// row's name. A pointer may still press anywhere on the row: a click that
// lands on no control is passed to the opener. The focus ring is drawn on
// the row while its opener holds the keyboard's focus.
//
// Hover-only actions take no room of their own: with a mouse they float over
// the row's right end (its status and time) while the row is hovered or holds
// focus, on the row's own background (--row-bg), so the time stays lined up
// at the right edge and the title keeps its width the rest of the time. On a
// touch screen there is no hover, so they stay in sight in the row (under it
// on a phone), like `actionsVisible` ones.
//
// Rows sit inside <ListRows>, which draws the dividers and moves focus with
// J/K, and there each row is one item of the list. A row that carries more
// than itself (its details open under it) goes inside a <ListItem>, which is
// then the item.
const CONTROL = "a,button,input,select,textarea,summary,label,[role=button],[role=menuitem],[role=option],[role=checkbox],[role=switch]";

// Where a row stands: straight inside a list (the row is the list's item),
// inside an item of its own (<ListItem>), or in no list at all.
const RowPlace = createContext<"list" | "item" | null>(null);

// Hover-only actions under a mouse: out of the flow, over the row's right
// end, shown while the row is hovered or holds focus. The strip wears the
// row's colour (the card's, while the row has none of its own), and a short
// fade on its left edge lets a long title slide under it. It takes the row's
// own vertical padding and alignment, so in a top-aligned row (a memory) the
// buttons sit on the first line, where the time was.
const FLOATING = cn(
  "pointer-fine:pointer-events-none pointer-fine:absolute pointer-fine:inset-y-0 pointer-fine:right-0 pointer-fine:[align-items:inherit] pointer-fine:px-3 pointer-fine:py-[inherit] pointer-fine:opacity-0",
  "pointer-fine:bg-[var(--row-bg,var(--card))]",
  "pointer-fine:before:pointer-events-none pointer-fine:before:absolute pointer-fine:before:inset-y-0 pointer-fine:before:right-full pointer-fine:before:w-4 pointer-fine:before:bg-[linear-gradient(to_right,transparent,var(--row-bg,var(--card)))] pointer-fine:before:content-['']",
  "pointer-fine:group-hover/row:pointer-events-auto pointer-fine:group-hover/row:opacity-100 pointer-fine:group-focus-within/row:pointer-events-auto pointer-fine:group-focus-within/row:opacity-100",
);

const TITLE = "min-w-0 truncate text-sm font-medium leading-5 text-foreground";

export function ListRow({
  lead,
  title,
  meta,
  status,
  time,
  actions,
  selected,
  href,
  onOpen,
  className,
  children,
  ariaLabel,
  glow,
  actionsVisible,
  expanded,
}: {
  lead?: React.ReactNode;
  title: React.ReactNode;
  meta?: React.ReactNode;
  status?: React.ReactNode;
  time?: React.ReactNode;
  actions?: React.ReactNode;
  selected?: boolean;
  href?: string;
  onOpen?: () => void;
  className?: string;
  children?: React.ReactNode;
  // What the opener is called aloud. It should hold the title's words (the
  // title with the row's kind before it, say); when it does not, the title is
  // put first (openerName). Without it the title names the opener.
  ariaLabel?: string;
  // A row that just arrived or changed glows for 1.5 s (row-glow in globals.css).
  glow?: boolean;
  // Actions that must stay in sight (a decision's Accept / Decline) instead of showing on hover.
  actionsVisible?: boolean;
  // A row that opens in place (its details drawn under it) says so to a screen reader.
  expanded?: boolean;
}) {
  const place = useContext(RowPlace);
  const opener = useRef<HTMLElement | null>(null);
  const interactive = Boolean(href || onOpen);
  // --row-bg is the row's background in its current state (unset: the
  // card's); the floating actions wear the same colour, so they sit on the
  // row without a seam. The hover tint is opaque (muted mixed into the card).
  const cls = cn(
    "group/row relative flex min-h-(--row-h) min-w-0 items-center gap-3 bg-[var(--row-bg,transparent)] px-3 py-1.5 text-left outline-none transition-colors duration-120 ease-out-soft motion-reduce:transition-none max-sm:flex-wrap",
    interactive &&
      "cursor-pointer hover:[--row-bg:color-mix(in_oklab,var(--muted)_60%,var(--card))] has-[[data-row-open]:focus-visible]:[--row-bg:color-mix(in_oklab,var(--muted)_60%,var(--card))] has-[[data-row-open]:focus-visible]:rounded-[4px] has-[[data-row-open]:focus-visible]:ring-2 has-[[data-row-open]:focus-visible]:ring-inset has-[[data-row-open]:focus-visible]:ring-ring",
    selected && "[--row-bg:var(--accent)] before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-link before:content-['']",
    glow && "row-glow",
    className,
  );
  const text = typeof title === "string" ? title : undefined;
  const name = openerName(ariaLabel, text);
  const head = href ? (
    <Link
      ref={(el) => {
        opener.current = el;
      }}
      href={href}
      data-row-open
      aria-current={selected ? "true" : undefined}
      aria-label={name}
      title={text}
      className={cn(TITLE, "outline-none")}
    >
      {title}
    </Link>
  ) : onOpen ? (
    <button
      ref={(el) => {
        opener.current = el;
      }}
      type="button"
      data-row-open
      aria-current={selected ? "true" : undefined}
      aria-expanded={expanded}
      aria-label={name}
      title={text}
      className={cn(TITLE, "cursor-pointer text-left outline-none")}
      // Called bare, as before: a caller's onOpen may take an argument of its own, and must not get the click.
      onClick={() => onOpen()}
    >
      {title}
    </button>
  ) : (
    <span className={TITLE} title={text}>
      {title}
    </span>
  );
  // A press anywhere on the row that lands on no control of its own is a
  // press on the opener. With a modifier, or the middle button, a link opens
  // in a new tab, as the link itself would.
  const press = (e: React.MouseEvent<HTMLDivElement>, aux = false) => {
    const hit = (e.target as HTMLElement).closest(CONTROL);
    if (hit && e.currentTarget.contains(hit)) return;
    const el = opener.current;
    if (!el) return;
    if (el instanceof HTMLAnchorElement && (aux || e.metaKey || e.ctrlKey || e.shiftKey)) {
      window.open(el.href, "_blank", "noopener");
      return;
    }
    if (!aux) el.click();
  };
  return (
    <div
      role={place === "list" ? "listitem" : undefined}
      className={cls}
      data-row
      aria-current={!interactive && selected ? "true" : undefined}
      onClick={interactive ? (e) => press(e) : undefined}
      onAuxClick={
        href
          ? (e) => {
              if (e.button === 1) press(e, true);
            }
          : undefined
      }
    >
      {lead ? <span className="flex shrink-0 items-center" aria-hidden={typeof title === "string" ? true : undefined}>{lead}</span> : null}
      <span className="flex min-w-0 flex-1 flex-col justify-center gap-0.5">
        <span className="flex min-w-0 items-center gap-2">
          {head}
          {meta ? <span className="hidden min-w-0 shrink items-center gap-1.5 truncate text-xs text-muted-foreground sm:flex">{meta}</span> : null}
        </span>
        {children ? <span className="min-w-0 truncate text-xs leading-4 text-muted-foreground">{children}</span> : null}
        {meta ? <span className="flex min-w-0 items-center gap-1.5 truncate text-xs text-muted-foreground sm:hidden">{meta}</span> : null}
      </span>
      {status || time ? (
        // Under floating actions the status and time step aside, so no edge of them shows past the buttons.
        <span className={cn("flex shrink-0 items-center gap-2 text-xs tabular-nums text-muted-foreground", actions && !actionsVisible && "transition-opacity duration-120 motion-reduce:transition-none pointer-fine:group-hover/row:opacity-0 pointer-fine:group-focus-within/row:opacity-0")}>
          {status}
          {time ? <span className="hidden whitespace-nowrap sm:inline">{time}</span> : null}
        </span>
      ) : null}
      {actions ? (
        <span
          className={cn(
            "flex shrink-0 items-center gap-1",
            actionsVisible
              ? "max-sm:basis-full max-sm:justify-end"
              : cn("transition-opacity duration-120 motion-reduce:transition-none pointer-coarse:max-sm:basis-full pointer-coarse:max-sm:justify-end", FLOATING),
          )}
          // The space between the row's buttons is theirs: a press there
          // opens nothing. A cue (<RowCue>) is the exception: it stands for the opener.
          onClick={(e) => {
            if (!(e.target as HTMLElement).closest("[data-row-cue]")) e.stopPropagation();
          }}
        >
          {actions}
        </span>
      ) : null}
    </div>
  );
}

// A cue among a row's actions, for a row whose only action is to open:
// drawn like a button (the caller's classes), but a span, because the row's
// opener is the control. A press on it opens the row; a screen reader skips
// it and reads the opener.
export function RowCue({ className, children }: { className?: string; children: React.ReactNode }) {
  return (
    <span data-row-cue aria-hidden className={className}>
      {children}
    </span>
  );
}

// One item of a <ListRows> that holds more than a row: the row and what
// opens under it (a question's text, an editor). The list's items are then
// these, and the row inside is a plain box.
export function ListItem({ children, ...props }: React.ComponentProps<"div">) {
  return (
    <RowPlace.Provider value="item">
      <div role="listitem" {...props}>
        {children}
      </div>
    </RowPlace.Provider>
  );
}

// The opener of a row, or the row itself when it has none (a row of text).
function rowStop(row: HTMLElement): HTMLElement {
  return row.querySelector<HTMLElement>("[data-row-open]") ?? row;
}

// The list: a bordered card with a divider between rows. J/K (or the
// arrows) move focus between rows, Enter opens the focused one (the
// opener's own doing), Esc lets go. Keys typed in a field inside a row are
// left alone. Each child is one item of the list: a <ListRow>, or a
// <ListItem> around a row and its details.
export function ListRows({ children, className, ariaLabel }: { children: React.ReactNode; className?: string; ariaLabel?: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const onKeyDown = useCallback((e: React.KeyboardEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement;
    if (inTextField(target.tagName, target.isContentEditable)) return;
    const k = rowKey(e.key, { meta: e.metaKey, ctrl: e.ctrlKey, alt: e.altKey });
    if (!k || k === "open") return;
    const root = ref.current;
    if (!root) return;
    const rows = Array.from(root.querySelectorAll<HTMLElement>("[data-row]")).filter((r) => r.closest("[data-rows]") === root);
    if (k === "blur") {
      if (rows.some((r) => rowStop(r) === target)) target.blur();
      return;
    }
    const current = rows.findIndex((r) => r === target || r.contains(target));
    const next = nextIndex(e.key, current, rows.length);
    if (next < 0 || next === current) {
      if (next >= 0) e.preventDefault();
      return;
    }
    e.preventDefault();
    rowStop(rows[next]).focus();
    rows[next].scrollIntoView?.({ block: "nearest" });
  }, []);
  return (
    <RowPlace.Provider value="list">
      <div ref={ref} data-rows role="list" aria-label={ariaLabel} className={cn("divide-y divide-border overflow-hidden rounded-lg border bg-card", className)} onKeyDown={onKeyDown}>
        {children}
      </div>
    </RowPlace.Provider>
  );
}

// The row's shape while the list loads: a lead square and two text bars.
export function RowSkeleton({ rows = 3, className }: { rows?: number; className?: string }) {
  return (
    <div className={cn("divide-y divide-border overflow-hidden rounded-lg border bg-card", className)} aria-busy aria-label="Loading">
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="flex min-h-(--row-h) items-center gap-3 px-3 py-1.5">
          <Skeleton className="size-7 shrink-0 rounded-md" />
          <div className="flex min-w-0 flex-1 flex-col gap-1.5">
            <Skeleton className="h-3.5 w-[min(60%,16rem)]" />
            <Skeleton className="h-3 w-[min(35%,9rem)]" />
          </div>
          <Skeleton className="h-3 w-10 shrink-0" />
        </div>
      ))}
    </div>
  );
}
