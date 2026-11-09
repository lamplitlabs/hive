"use client";

// Pick earlier sessions and tasks as references for a new session or task:
// the hub puts their summaries in front of the new first message, so the
// agent starts with that context instead of gathering it again.

import { useMemo, useState } from "react";
import { LinkIcon, XIcon } from "lucide-react";
import { Input } from "@/components/ui/input";
import type { ChatListResponse } from "@/lib/api";
import { useApi } from "@/lib/use-api";
import { referenceOptions, MAX_REFERENCES } from "@/lib/reference-state";

export function ReferencePicker({
  value,
  onChange,
}: {
  value: string[];
  onChange: (ids: string[]) => void;
}) {
  const list = useApi<ChatListResponse>("chat?all=1", { refresh: 30000 });
  const [query, setQuery] = useState("");
  const all = useMemo(() => referenceOptions(list.data, ""), [list.data]);
  const matches = useMemo(
    () => referenceOptions(list.data, query).filter((o) => !value.includes(o.id)).slice(0, 6),
    [list.data, query, value],
  );
  const label = (id: string) => all.find((o) => o.id === id)?.label ?? id;
  const full = value.length >= MAX_REFERENCES;

  return (
    <div className="grid min-w-0 gap-1.5">
      <div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
        <LinkIcon className="size-3.5" /> Build on earlier work (optional)
      </div>
      {value.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {value.map((id) => (
            <span key={id} className="inline-flex max-w-full items-center gap-1 rounded-md border bg-muted px-2 py-0.5 text-xs">
              <span className="truncate">{label(id)}</span>
              <button type="button" aria-label={`Remove ${label(id)}`} onClick={() => onChange(value.filter((v) => v !== id))}>
                <XIcon className="size-3" />
              </button>
            </span>
          ))}
        </div>
      )}
      {!full && (
        <Input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search sessions and tasks to reference…"
          aria-label="Reference sessions and tasks"
          className="h-8 text-sm"
        />
      )}
      {!full && query.trim() && (
        <div className="grid gap-0.5 rounded-md border p-1">
          {matches.length === 0 ? (
            <div className="px-2 py-1 text-xs text-muted-foreground">Nothing matches.</div>
          ) : (
            matches.map((o) => (
              <button
                type="button"
                key={o.id}
                className="truncate rounded px-2 py-1 text-left text-sm hover:bg-muted"
                onClick={() => {
                  onChange([...value, o.id]);
                  setQuery("");
                }}
              >
                {o.label}
              </button>
            ))
          )}
        </div>
      )}
      <p className="text-xs text-muted-foreground">
        Their summaries go to the agent with your message, so it does not gather that context again.
      </p>
    </div>
  );
}
