// Where one For you row leads when it is not answered in place: the record
// itself, the Machines page, a project's evolution tab, or the For you page
// filtered to the row's project (questions and proposals are answered
// there). Kept apart from for-you-state.ts so that file stays import-free
// for node --test.

import { hrefs } from "@/lib/nav";
import type { ForYouRow } from "./for-you-state";

export function rowHref(r: ForYouRow): string {
  if (r.href) return r.href;
  const [kind, ...rest] = r.key.split(":");
  const id = rest.join(":");
  switch (kind) {
    case "decision":
      return hrefs.record(id, r.project || undefined);
    case "compute":
      return hrefs.compute();
    case "draft":
    case "step":
      return r.project ? hrefs.project(r.project, "evolution") : hrefs.forYou();
    default:
      return hrefs.forYou({ project: r.project || undefined });
  }
}
