// A loop record's draft state, pure: no React, no DOM, only type imports, so
// node --test reaches it (the inbox counts drafts with it, inbox-state.ts).
// draft-panel.tsx re-exports both names for its callers.
import type { EvolveRecordRef, JobRef, RecordDraft } from "../../lib/api";

// What one loop record is waiting on, from a project row or the evolution document.
export interface DraftState {
  id: string;
  rev: number;
  seeded: boolean;
  seeded_from: string;
  draft?: RecordDraft | null;
  draft_job?: JobRef | null;
}

export const draftState = (r: EvolveRecordRef | null | undefined): DraftState | null => (r ? { id: r.id, rev: r.rev, seeded: r.seeded, seeded_from: r.seeded_from, draft: r.draft, draft_job: r.draft_job } : null);
