"use client";

import { useCallback, useSyncExternalStore } from "react";

// Whether the sidebar's "More" fold is open. Folded by default, so a new
// owner sees Sessions, Tasks and the projects and nothing else; the choice
// stays in this browser. The server snapshot is folded, so the static export
// hydrates the same markup every time.
const KEY = "hive.sidebar.more";
const listeners = new Set<() => void>();
function read(): boolean {
  try {
    return window.localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}
function subscribe(cb: () => void) {
  listeners.add(cb);
  window.addEventListener("storage", cb);
  return () => {
    listeners.delete(cb);
    window.removeEventListener("storage", cb);
  };
}

export function useMoreOpen(): [boolean, (open: boolean) => void] {
  const open = useSyncExternalStore(subscribe, read, () => false);
  const set = useCallback((next: boolean) => {
    try {
      window.localStorage.setItem(KEY, next ? "1" : "0");
    } catch {
      // private mode or storage disabled: the fold lasts for this page only
    }
    listeners.forEach((l) => l());
  }, []);
  return [open, set];
}
