// Where a waiting row leads when it is not answered in place: a decision to
// its record, a compute request to Machines, a gate step to the project's
// self-improvement tab, the rest to the project's For you tab (questions and
// proposals are answered there) or, without a project, to the hard-stop strip
// on Sessions. Kept apart from asks-state.ts so that file stays import-free
// for node --test.

import { hrefs } from "@/lib/nav";
import type { ForYouRow } from "./asks-state";

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
      return r.project ? hrefs.project(r.project, "evolution") : hrefs.hardStops();
    default:
      return r.project ? hrefs.project(r.project, "for-you") : hrefs.hardStops();
  }
}
