"use client";

// The one button on the site that needs the browser: it puts a command on the
// clipboard. Kept apart and small, so a page that has none ships none of it.
import { useEffect, useRef, useState } from "react";

export function CopyButton({ text, what = "the command", primary = false, className = "" }: { text: string; what?: string; /** The page's main action: drawn as the primary button. */ primary?: boolean; className?: string }) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);

  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // No clipboard here (an old browser, a page not served over https): the line stays on the page to select.
      return;
    }
    setCopied(true);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setCopied(false), 2000);
  }

  return (
    <>
      <button
        type="button"
        onClick={copy}
        className={`inline-flex h-9 shrink-0 items-center gap-1.5 rounded-lg border px-3 text-xs font-medium transition-colors ${
          primary ? "border-transparent bg-primary text-primary-foreground hover:bg-primary-strong" : "border-border bg-card text-foreground hover:bg-muted"
        } ${className}`}
      >
        <svg viewBox="0 0 24 24" className="size-3.5" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
          {copied ? (
            <path d="M20 6 9 17l-5-5" />
          ) : (
            <>
              <rect x="8" y="8" width="13" height="13" rx="2" />
              <path d="M4 16V5a1 1 0 0 1 1-1h11" />
            </>
          )}
        </svg>
        <span className="min-w-[3.1em] text-left">{copied ? "Copied" : "Copy"}</span>
        <span className="sr-only"> {what}</span>
      </button>
      <span role="status" className="sr-only">
        {copied ? "Copied to the clipboard." : ""}
      </span>
    </>
  );
}
