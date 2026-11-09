"use client";

import { useMemo } from "react";
import { buildInbox, type Inbox, type InboxSources } from "./inbox-state";
import type { Compute, HarnessStatus, Org, ProjectsPayload, RecordsPayload, Runner, ServicePayload, SettingsPayload, WorkSuggestionsPayload } from "@/lib/api";
import { useApi } from "@/lib/use-api";

// The hooks behind the one "for you" count. The count itself - what counts,
// per project and for the whole machine - is pure and lives in
// inbox-state.ts; every name it exports is re-exported here, so callers keep
// importing from "@/components/pulse/inbox". (Named, not `export *`: Next
// does not take a star re-export in a client module.)
export { buildInbox, inboxBreakdown, type Inbox, type InboxItems, type InboxSources } from "./inbox-state";

// Every path below is shared through useApi's cache, so a second caller on
// the same screen costs no extra request. The owner's notifications
// (components/shell/owner-notifier.tsx) read the sources too: an item only
// counts as news once the source it comes from has loaded.

export function useInboxSources(): InboxSources {
  const org = useApi<Org>("org");
  const proposed = useApi<RecordsPayload>("records?kind=adr&status=Proposed", { refresh: 60000 });
  const reg = useApi<ProjectsPayload>("projects", { refresh: 60000 });
  const compute = useApi<Compute>("compute");
  const suggested = useApi<WorkSuggestionsPayload>("tasks/suggested?status=owner", { refresh: 60000 });
  const runner = useApi<Runner>("runner");
  const service = useApi<ServicePayload>("service", { refresh: 60000 });
  const settings = useApi<SettingsPayload>("settings");
  const harness = useApi<HarnessStatus>("harness", { refresh: 60000 });
  return useMemo(() => ({ org, proposed, reg, compute, suggested, runner, service, settings, harness }), [org, proposed, reg, compute, suggested, runner, service, settings, harness]);
}

export function useInbox(): Inbox {
  const sources = useInboxSources();
  return useMemo(() => buildInbox(sources), [sources]);
}
