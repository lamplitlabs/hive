"use client";

import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { followAfterScroll } from "./scroll";

// Keep a scrolling log at its newest line (lib/scroll.ts followAfterScroll):
// `box` is the ref of the scrolling element, `content` the ref of the one
// element inside it that holds every line, `onScroll` goes on the box. It
// opens at the end, follows the content as it grows and the box as it
// resizes (the canvas opening beside it rewraps every line) while the
// reader sits there, and lets go when they scroll up; `away` says so (the
// "Jump to the latest" pill) and `jump` comes back. A change of `key` (a
// new line) follows only while following; a change of `force` (a message
// just sent) pins it again. Both refs are callbacks, so a log that mounts
// after its data loads is watched too.
export function useFollowEnd(key?: unknown, force?: unknown) {
  const el = useRef<HTMLElement | null>(null);
  const inner = useRef<HTMLElement | null>(null);
  const ro = useRef<ResizeObserver | null>(null);
  const following = useRef(true);
  const lastTop = useRef(0);
  const [away, setAway] = useState(false);

  const pin = useCallback(() => {
    const b = el.current;
    if (!b || !following.current) return;
    b.scrollTop = b.scrollHeight;
    lastTop.current = b.scrollTop;
  }, []);

  const watch = useCallback(() => {
    ro.current?.disconnect();
    ro.current = null;
    if (!el.current || typeof ResizeObserver === "undefined") return;
    ro.current = new ResizeObserver(pin);
    ro.current.observe(el.current);
    if (inner.current) ro.current.observe(inner.current);
  }, [pin]);

  const box = useCallback(
    (node: HTMLElement | null) => {
      el.current = node;
      watch();
      pin();
    },
    [watch, pin],
  );
  const content = useCallback(
    (node: HTMLElement | null) => {
      inner.current = node;
      watch();
    },
    [watch],
  );

  useLayoutEffect(pin, [pin, key]);
  // Pinning scrolls the box, and its scroll event clears `away`.
  useLayoutEffect(() => {
    following.current = true;
    pin();
  }, [pin, force]);

  const onScroll = useCallback((e: { currentTarget: HTMLElement }) => {
    const b = e.currentTarget;
    const next = followAfterScroll(following.current, lastTop.current, b);
    lastTop.current = b.scrollTop;
    following.current = next;
    setAway(!next);
  }, []);

  const jump = useCallback(() => {
    const b = el.current;
    following.current = true;
    setAway(false);
    if (!b) return;
    const still =
      typeof matchMedia !== "undefined" &&
      matchMedia("(prefers-reduced-motion: reduce)").matches;
    b.scrollTo({ top: b.scrollHeight, behavior: still ? "auto" : "smooth" });
  }, []);

  return { box, content, away, onScroll, jump };
}
