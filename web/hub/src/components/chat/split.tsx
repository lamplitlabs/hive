"use client";

// The Chat page's columns, shared by a conversation and a task: the list on
// the left, what is open in the middle, its work beside it.

import {
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
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

export function Split({
  list,
  main,
  panel,
  listOnMobile,
  fill,
  wide,
}: {
  list: ReactNode;
  main: ReactNode;
  panel?: ReactNode;
  listOnMobile?: boolean;
  fill?: boolean;
  wide?: boolean;
}) {
  const box = useRef<HTMLDivElement>(null);
  const width = useWidth(box);
  const three = !!panel && width >= THREE_COLUMNS_AT;
  const side = cn(
    "scroll-thin grid min-w-0 content-start gap-4 lg:order-none lg:overflow-y-auto lg:pb-1",
    fill ? "lg:min-h-0" : "lg:sticky lg:top-4 lg:max-h-[calc(100dvh-2rem)]",
  );
  // With `fill` the panel's box is a definite height, so a panel that
  // scrolls inside itself (the canvas) can take all of it.
  const panelBox = fill ? "lg:grid lg:grid-rows-[minmax(0,1fr)]" : "";
  return (
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
          (!panel || three) && !listOnMobile && "hidden lg:grid",
        )}
      >
        <div className={cn("min-w-0", !listOnMobile && "hidden lg:block")}>
          {list}
        </div>
        {panel && !three ? (
          <div className={cn("min-w-0", fill && "lg:h-[70dvh]", panelBox)}>
            {panel}
          </div>
        ) : null}
      </aside>
      <main
        className={cn(
          "order-1 min-w-0 lg:order-none",
          fill && "lg:flex lg:min-h-0 lg:flex-col",
        )}
      >
        {main}
      </main>
      {panel && three ? (
        <aside
          className={cn(side, "order-3 lg:pl-1", panelBox)}
          aria-label="Work of this conversation"
        >
          {panel}
        </aside>
      ) : null}
    </div>
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
      <ArrowLeftIcon /> All conversations
    </Button>
  );
}
