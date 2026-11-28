"use client";

import { useCallback, useSyncExternalStore } from "react";

// Whether an open conversation or task shows its work panel (the changes,
// the terminal, the files and pages the agent opened) beside the
// transcript. Hidden by default, so a new owner meets a conversation and
// nothing else; "Show work" in the header opens it, and the choice stays in
// this browser for every session after. The server snapshot is hidden, so
// the static export hydrates the same markup every time.
const KEY = "hive.chat.work";
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

export function useWorkPanel(): [boolean, (open: boolean) => void] {
  const open = useSyncExternalStore(subscribe, read, () => false);
  const set = useCallback((next: boolean) => {
    try {
      window.localStorage.setItem(KEY, next ? "1" : "0");
    } catch {
      // private mode or storage disabled: the choice lasts for this page only
    }
    listeners.forEach((l) => l());
  }, []);
  return [open, set];
}
