"use client";

import { useMemo } from "react";
import { ExternalLinkIcon } from "lucide-react";
import { linkDocs } from "@/lib/site";

// A text of Hive's own that may name a file of its documentation, such as a
// setting's description ending in "(docs/security.md)": the words as they
// are, with each such name a link to its page on the website (lib/site.ts).
// A text that names none is drawn as plain text.
export function DocText({ children }: { children: string }) {
  const parts = useMemo(() => linkDocs(children), [children]);
  if (!parts.some((p) => p.href)) return <>{children}</>;
  return (
    <>
      {parts.map((p, i) =>
        p.href ? (
          // Inline, not a flex box: the name wraps with the sentence and keeps its bracket.
          <a key={i} href={p.href} target="_blank" rel="noreferrer" title="Opens on hivefleet.dev" className="font-medium text-link underline decoration-current/30 underline-offset-2 hover:decoration-current">
            {p.text}
            <ExternalLinkIcon className="ml-0.5 inline size-3 align-[-1px]" aria-hidden />
          </a>
        ) : (
          p.text
        ),
      )}
    </>
  );
}
