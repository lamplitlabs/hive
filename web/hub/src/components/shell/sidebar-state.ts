// The desktop sidebar's one setting: open (the default) or collapsed to a
// rail of icons. Pure, so sidebar-state.test.mjs can drive it; use-sidebar.ts
// owns the React hook and the real localStorage.
//
// The choice is this browser's, like the theme and the view toggles, so it
// lives in localStorage. The layout itself is CSS: `<html>` carries
// SIDEBAR_CLASS while the sidebar is collapsed and the `collapsed:` Tailwind
// variant (globals.css) styles the rail. A tiny script in the document head
// (SIDEBAR_BOOT_SCRIPT) sets the class before the first paint, the way
// next-themes sets the theme, so a reload never shows the wide sidebar for a
// frame and then folds it.

export const SIDEBAR_KEY = "pulse.sidebar";
export const SIDEBAR_COLLAPSED = "collapsed";
export const SIDEBAR_CLASS = "sidebar-collapsed";

// The subset of Storage the sidebar uses, so tests can pass a Map-backed one.
export interface SidebarStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

// The subset of DOMTokenList the sidebar touches on <html>.
export interface SidebarClassList {
  add(token: string): void;
  remove(token: string): void;
}

export function readSidebarCollapsed(storage: SidebarStorage): boolean {
  return storage.getItem(SIDEBAR_KEY) === SIDEBAR_COLLAPSED;
}

// Open is the default, so opening removes the key rather than writing "open":
// a cleared storage and a never-touched one read the same.
export function writeSidebarCollapsed(storage: SidebarStorage, collapsed: boolean): void {
  if (collapsed) storage.setItem(SIDEBAR_KEY, SIDEBAR_COLLAPSED);
  else storage.removeItem(SIDEBAR_KEY);
}

export function applySidebarClass(classList: SidebarClassList, collapsed: boolean): void {
  if (collapsed) classList.add(SIDEBAR_CLASS);
  else classList.remove(SIDEBAR_CLASS);
}

// Runs inline in <head> before React: reads the choice and marks <html>.
// Storage can throw (private mode, disabled); then the sidebar starts open.
export const SIDEBAR_BOOT_SCRIPT = `(function(){try{if(localStorage.getItem(${JSON.stringify(SIDEBAR_KEY)})===${JSON.stringify(SIDEBAR_COLLAPSED)})document.documentElement.classList.add(${JSON.stringify(SIDEBAR_CLASS)})}catch(e){}})()`;

// The toggle button says what a press does.
export function sidebarToggleLabel(collapsed: boolean): string {
  return collapsed ? "Expand the sidebar" : "Collapse the sidebar";
}
