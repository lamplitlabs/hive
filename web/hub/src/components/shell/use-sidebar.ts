"use client";

import { useCallback, useSyncExternalStore } from "react";
import { applySidebarClass, readSidebarCollapsed, writeSidebarCollapsed } from "./sidebar-state";

// Whether the desktop sidebar is collapsed to its rail, and the one way to
// change it. The server snapshot is "open", so the static export hydrates the
// same markup every time; the boot script in layout.tsx has already put the
// class on <html> when the stored choice is collapsed, and the first client
// render agrees with it.
const listeners = new Set<() => void>();
function read(): boolean {
  try {
    return readSidebarCollapsed(window.localStorage);
  } catch {
    return false;
  }
}
// Another tab's change arrives as a storage event: the class follows the
// stored choice here too, so two tabs never disagree.
function onStorage() {
  applySidebarClass(document.documentElement.classList, read());
  listeners.forEach((l) => l());
}
function subscribe(cb: () => void) {
  listeners.add(cb);
  if (listeners.size === 1) window.addEventListener("storage", onStorage);
  return () => {
    listeners.delete(cb);
    if (!listeners.size) window.removeEventListener("storage", onStorage);
  };
}

export function useSidebarCollapsed(): [boolean, (collapsed: boolean) => void] {
  const collapsed = useSyncExternalStore(subscribe, read, () => false);
  const set = useCallback((next: boolean) => {
    try {
      writeSidebarCollapsed(window.localStorage, next);
    } catch {
      // private mode or storage disabled: the choice lasts for this page only
    }
    applySidebarClass(document.documentElement.classList, next);
    listeners.forEach((l) => l());
  }, []);
  return [collapsed, set];
}
