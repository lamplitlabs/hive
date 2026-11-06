"use client";

import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { usePathname, useSearchParams } from "next/navigation";
import type { Org } from "@/lib/api";
import { useApi } from "@/lib/use-api";
import { NAV_ITEMS, SETTINGS_ITEM } from "./nav-items";
import { defaultCrumbs, type Crumb } from "./crumbs-state";

export type { Crumb } from "./crumbs-state";

// The top bar's breadcrumbs. A page calls useCrumbs([{label, href}]) and the
// shell shows them (the phone's title and back arrow read them too); on
// unmount they clear and the shell falls back to defaultCrumbs() from the
// route. The provider sits in the shell above the top bar.
type Ctx = [Crumb[] | null, (c: Crumb[] | null) => void];
const CrumbsContext = createContext<Ctx>([null, () => {}]);

export function CrumbsProvider({ children }: { children: ReactNode }) {
  const state = useState<Crumb[] | null>(null);
  return <CrumbsContext.Provider value={state}>{children}</CrumbsContext.Provider>;
}

// Set the crumbs for as long as the calling page is mounted. Pass the array
// freshly each render; the hook compares by content so it only writes on a
// real change.
export function useCrumbs(crumbs: Crumb[] | null | undefined) {
  const [, set] = useContext(CrumbsContext);
  const key = crumbs ? JSON.stringify(crumbs) : "";
  useEffect(() => {
    set(key ? (JSON.parse(key) as Crumb[]) : null);
    return () => set(null);
  }, [key, set]);
}

const PAGES = [...NAV_ITEMS, SETTINGS_ITEM].map((i) => ({ href: i.href, label: i.label }));

// The crumbs the bars show: the page's own, else the route's default.
export function useCurrentCrumbs(): Crumb[] {
  const [own] = useContext(CrumbsContext);
  const pathname = usePathname();
  const params = useSearchParams();
  const { data: org } = useApi<Org>("org");
  const search = params.toString();
  return useMemo(() => {
    if (own && own.length) return own;
    const projects = [...(org?.projects ?? []), ...(org?.commons ? [org.commons] : [])].map((p) => ({ slug: p.slug, name: p.name }));
    return defaultCrumbs(pathname, new URLSearchParams(search), projects, PAGES);
  }, [own, pathname, search, org]);
}
