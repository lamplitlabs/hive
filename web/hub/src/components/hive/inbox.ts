"use client";

import { useMemo } from "react";
import { buildInbox, INBOX_SOURCE_PATHS as P, type Inbox, type InboxSources } from "./inbox-state";
import type { Compute, HarnessStatus, Org, ProjectsPayload, RecordsPayload, Runner, ServicePayload, SettingsPayload, WorkSuggestionsPayload } from "@/lib/api";
import { useApi } from "@/lib/use-api";

// The hooks behind the one "for you" count. The count itself - what counts,
// per project and for the whole machine - is pure and lives in
// inbox-state.ts; every name it exports is re-exported here, so callers keep
// importing from "@/components/hive/inbox". (Named, not `export *`: Next
// does not take a star re-export in a client module.)
export { buildInbox, inboxBreakdown, INBOX_SOURCE_PATHS, type Inbox, type InboxItems, type InboxSourcePath, type InboxSources } from "./inbox-state";

// Every path (INBOX_SOURCE_PATHS, inbox-state.ts) is shared through useApi's cache, so a second caller on
// the same screen costs no extra request. The owner's notifications
// (components/shell/owner-notifier.tsx) read the sources too: an item only
// counts as news once the source it comes from has loaded.

export function useInboxSources(): InboxSources {
  const org = useApi<Org>(P.org.path, P.org);
  const proposed = useApi<RecordsPayload>(P.proposed.path, P.proposed);
  const reg = useApi<ProjectsPayload>(P.reg.path, P.reg);
  const compute = useApi<Compute>(P.compute.path, P.compute);
  const suggested = useApi<WorkSuggestionsPayload>(P.suggested.path, P.suggested);
  const runner = useApi<Runner>(P.runner.path, P.runner);
  const service = useApi<ServicePayload>(P.service.path, P.service);
  const settings = useApi<SettingsPayload>(P.settings.path, P.settings);
  const harness = useApi<HarnessStatus>(P.harness.path, P.harness);
  return useMemo(() => ({ org, proposed, reg, compute, suggested, runner, service, settings, harness }), [org, proposed, reg, compute, suggested, runner, service, settings, harness]);
}

export function useInbox(): Inbox {
  const sources = useInboxSources();
  return useMemo(() => buildInbox(sources), [sources]);
}
