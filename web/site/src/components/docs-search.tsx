"use client";

// The search box over the docs: titles and headings, from the small index the
// build writes to /docs/search.json. The index is fetched the first time the
// box is used, never before, and a result is a plain link.
import { useId, useRef, useState } from "react";
import { searchDocs } from "@/lib/search";
import type { SearchEntry } from "@/lib/search";

const MAX = 10;
let index: Promise<SearchEntry[] | null> | undefined;

/** The index, fetched once per page load; null when it did not arrive (the next try asks again). */
function load(): Promise<SearchEntry[] | null> {
  index ??= fetch("/docs/search.json")
    .then((r) => (r.ok ? (r.json() as Promise<SearchEntry[]>) : Promise.reject(new Error(String(r.status)))))
    .catch(() => {
      index = undefined;
      return null;
    });
  return index;
}

export function DocsSearch({ className = "" }: { className?: string }) {
  const id = useId();
  const [query, setQuery] = useState("");
  const [entries, setEntries] = useState<SearchEntry[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const closing = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const hits = entries ? searchDocs(entries, query, MAX) : [];
  const asked = query.trim().length > 0;
  const show = open && asked;
  const listId = `${id}-results`;

  function ready() {
    clearTimeout(closing.current);
    setOpen(true);
    if (!entries) {
      void load().then((found) => {
        setEntries(found);
        setFailed(found === null);
      });
    }
  }

  function onKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === "Escape") {
      setOpen(false);
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      if (hits.length === 0) return;
      e.preventDefault();
      setOpen(true);
      setActive((a) => (a + (e.key === "ArrowDown" ? 1 : hits.length - 1)) % hits.length);
    } else if (e.key === "Enter" && show && hits[active]) {
      e.preventDefault();
      setOpen(false);
      window.location.assign(hits[active].href);
    }
  }

  return (
    <div className={`relative ${className}`} role="search">
      <label htmlFor={id} className="sr-only">
        Search the documentation
      </label>
      <svg
        viewBox="0 0 24 24"
        className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        aria-hidden="true"
      >
        <circle cx="11" cy="11" r="7" />
        <path d="m20 20-3.5-3.5" />
      </svg>
      <input
        id={id}
        type="search"
        role="combobox"
        aria-expanded={show}
        aria-controls={listId}
        aria-autocomplete="list"
        aria-activedescendant={show && hits[active] ? `${id}-hit-${active}` : undefined}
        autoComplete="off"
        spellCheck={false}
        placeholder="Search the docs"
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          setActive(0);
          ready();
        }}
        onFocus={ready}
        onBlur={() => {
          closing.current = setTimeout(() => setOpen(false), 150);
        }}
        onKeyDown={onKeyDown}
        className="h-10 w-full rounded-xl border border-border bg-card pr-3 pl-9 text-sm text-foreground placeholder:text-muted-foreground"
      />
      <div
        id={listId}
        role="listbox"
        aria-label="Search results"
        hidden={!show}
        // A press on a result must not take the focus from the box, or the list would close under the pointer.
        onMouseDown={(e) => e.preventDefault()}
        className="absolute top-12 right-0 left-0 z-40 max-h-[60dvh] overflow-y-auto rounded-xl border border-border bg-background p-1.5 shadow-pop"
      >
        {hits.map((h, i) => (
          <a
            key={h.href}
            id={`${id}-hit-${i}`}
            role="option"
            aria-selected={i === active}
            href={h.href}
            tabIndex={-1}
            // A heading of the page you are on is only a jump: the list must not stay open over it.
            onClick={() => setOpen(false)}
            onMouseEnter={() => setActive(i)}
            className="block rounded-lg px-3 py-2 text-sm aria-selected:bg-muted"
          >
            <span className="block font-medium text-foreground">{h.title}</span>
            <span className="block text-xs text-muted-foreground">{h.where}</span>
          </a>
        ))}
        {show && hits.length === 0 && (
          <p className="px-3 py-2 text-sm text-muted-foreground">
            {entries ? "No title or heading in the docs has these words." : failed ? "The search index did not load. Try again in a moment." : "Loading the index."}
          </p>
        )}
      </div>
      <p role="status" className="sr-only">
        {show && entries ? `${hits.length} ${hits.length === 1 ? "result" : "results"}.` : ""}
      </p>
    </div>
  );
}
