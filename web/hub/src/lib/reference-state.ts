// Options for the reference picker (components/reference-picker.tsx):
// sessions and tasks from GET /chat, newest first, filtered by a query.

import type { ChatListResponse } from "./api";

export const MAX_REFERENCES = 10;

export interface ReferenceOption {
  id: string;
  label: string;
}

export function referenceOptions(
  data: ChatListResponse | undefined | null,
  query: string,
): ReferenceOption[] {
  if (!data) return [];
  const q = query.trim().toLowerCase();
  const tasks = (data.tasks ?? []).map((t) => ({
    id: t.id,
    label: `${t.id} · ${t.title}`,
    at: t.created_at || "",
  }));
  const sessions = (data.conversations ?? []).map((c) => ({
    id: c.id,
    label: `Session · ${c.title || c.id}`,
    at: c.last_at || c.created_at || "",
  }));
  return [...tasks, ...sessions]
    .filter((o) => !q || o.label.toLowerCase().includes(q) || o.id.toLowerCase().includes(q))
    .sort((a, b) => (a.at < b.at ? 1 : a.at > b.at ? -1 : 0))
    .map(({ id, label }) => ({ id, label }));
}
