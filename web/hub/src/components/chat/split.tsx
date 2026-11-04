"use client";

// The Chat page's columns, shared by a conversation and a task: the list on
// the left, what is open in the middle, its work beside it.

import {
  createContext,
  useContext,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import { ArrowLeftIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

// On a phone it flows as one: the transcript, then the side panel (plan,
// changes, artifacts, project); the list gives way to the open conversation.
// On a desktop the conversations list takes a column on the left and the
// transcript the rest, the panel under the list. Given enough width the
// panel gets a column of its own on the right - the way the Copilot app
// keeps a session's changes beside the chat - so it sits at the top where
// you see it instead of under however many conversations the list holds.
// The width is read from the box itself, not the window: the hub's own
// sidebar and the page's margins take a share that differs by breakpoint.
// With `fill` (a conversation open on a desktop) the columns take the rest
// of the window: the transcript scrolls inside its card, the composer stays
// on screen, and each side column scrolls on its own, so a long diff never
// pushes the page about. With `wide` (a file, a terminal or a diff in front
// on the canvas) the right column takes the larger share and the transcript
// the smaller, the way the Copilot app narrows the chat when a file opens.
//
// The page renders one Split and keeps it: the list inside it stays mounted
// while conversations open and close, so its scroll position, its folds and
// its rows hold still - a click on a conversation changes what is beside
// the list, not the list. What is open renders a Pane inside the Split:
// its main content goes in the middle column and its panel is portalled
// into whichever slot the width gives the panel (under the list, or the
// right column), while the Pane tells the Split whether there is a panel
// and whether the canvas wants the wide share.
const THREE_COLUMNS_AT = 1100;
const ROOMY_AT = 1264;

export function useWidth(ref: RefObject<HTMLElement | null>): number {
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver((entries) => {
      const e = entries[0];
      if (e) setWidth(e.contentRect.width);
    });
    ro.observe(el);
    setWidth(el.getBoundingClientRect().width);
    return () => ro.disconnect();
  }, [ref]);
  return width;
}

type Layout = { panel: boolean; wide: boolean };
const NO_PANEL: Layout = { panel: false, wide: false };

type Frame = {
  // Where the open pane's panel renders; null until the Split has mounted.
  slot: HTMLElement | null;
  // The layout the Split has applied; a pane portals its panel only once
  // this matches what it asked for, so the panel never lands in the slot
  // of the layout before (under the list, say, for one paint, which would
  // grow and then shrink the list's column and jolt its scroll position).
  layout: Layout;
  setLayout: (layout: Layout) => void;
};
const FrameContext = createContext<Frame | null>(null);

export function Split({
  list,
  listOnMobile,
  fill,
  children,
}: {
  list: ReactNode;
  listOnMobile?: boolean;
  fill?: boolean;
  children: ReactNode;
}) {
  const box = useRef<HTMLDivElement>(null);
  const width = useWidth(box);
  const [layout, setLayout] = useState<Layout>(NO_PANEL);
  const [slot, setSlot] = useState<HTMLElement | null>(null);
  const frame = useMemo<Frame>(
    () => ({ slot, layout, setLayout }),
    [slot, layout],
  );
  const three = layout.panel && width >= THREE_COLUMNS_AT;
  const wide = three && layout.wide;
  const side = cn(
    "scroll-thin grid min-w-0 content-start gap-4 lg:order-none lg:overflow-y-auto lg:pb-1",
    fill ? "lg:min-h-0" : "lg:sticky lg:top-4 lg:max-h-[calc(100dvh-2rem)]",
  );
  // With `fill` the panel's box is a definite height, so a panel that
  // scrolls inside itself (the canvas) can take all of it.
  const panelBox = fill ? "lg:grid lg:grid-rows-[minmax(0,1fr)]" : "";
  return (
    <FrameContext.Provider value={frame}>
      <div
        ref={box}
        className={cn(
          "grid min-w-0 gap-4",
          three
            ? wide
              ? "lg:grid-cols-[14rem_minmax(24rem,1fr)_minmax(0,1.25fr)]"
              : width >= ROOMY_AT
                ? "lg:grid-cols-[15rem_minmax(0,1fr)_22rem]"
                : "lg:grid-cols-[14rem_minmax(0,1fr)_19rem]"
            : "lg:grid-cols-[17rem_minmax(0,1fr)] xl:grid-cols-[19rem_minmax(0,1fr)] 2xl:grid-cols-[21rem_minmax(0,1fr)]",
          fill
            ? "lg:min-h-0 lg:flex-1 lg:grid-rows-[minmax(0,1fr)]"
            : "lg:items-start",
        )}
      >
        <aside
          className={cn(
            side,
            "order-2 lg:pr-1",
            (!layout.panel || three) && !listOnMobile && "hidden lg:grid",
          )}
        >
          <div className={cn("min-w-0", !listOnMobile && "hidden lg:block")}>
            {list}
          </div>
          {!three ? (
            // The panel's slot under the list; an empty slot takes no room,
            // so the list's gap does not grow when nothing is open.
            <div
              ref={setSlot}
              className={cn(
                "min-w-0 empty:hidden",
                fill && "lg:h-[70dvh]",
                panelBox,
              )}
            />
          ) : null}
        </aside>
        <main
          className={cn(
            "order-1 min-w-0 lg:order-none",
            fill && "lg:flex lg:min-h-0 lg:flex-col",
          )}
        >
          {children}
        </main>
        {three ? (
          <aside
            ref={setSlot}
            className={cn(side, "order-3 lg:pl-1", panelBox)}
            aria-label="Work of this session"
          />
        ) : null}
      </div>
    </FrameContext.Provider>
  );
}

// What is open inside a Split: its main content, and the panel beside it.
// Rendered by the conversation or the task, so the panel keeps the view's
// state and context while its DOM lands in the Split's slot.
export function Pane({
  main,
  panel,
  wide,
}: {
  main: ReactNode;
  panel?: ReactNode;
  wide?: boolean;
}) {
  const frame = useContext(FrameContext);
  const setLayout = frame?.setLayout;
  const hasPanel = !!panel;
  const wantsWide = !!wide;
  useLayoutEffect(() => {
    setLayout?.({ panel: hasPanel, wide: wantsWide });
  }, [setLayout, hasPanel, wantsWide]);
  // Leaving (another view opens, or none) hands the Split back its two
  // columns until the next pane says otherwise.
  useLayoutEffect(() => () => setLayout?.(NO_PANEL), [setLayout]);
  const applied =
    !!frame &&
    frame.layout.panel === hasPanel &&
    frame.layout.wide === wantsWide;
  return (
    <>
      {main}
      {panel && applied && frame.slot ? createPortal(panel, frame.slot) : null}
    </>
  );
}

// On a phone the open conversation takes the list's place; this goes back.
export function BackButton({ onBack }: { onBack: () => void }) {
  return (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      className="-ml-2 justify-self-start lg:hidden"
      onClick={onBack}
    >
      <ArrowLeftIcon /> All sessions
    </Button>
  );
}
