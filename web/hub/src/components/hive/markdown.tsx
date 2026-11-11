"use client";

import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { cn } from "@/lib/utils";

// Record bodies, briefs and chat messages are Markdown stored in SQL. Links
// to console pages open in the same tab; absolute links, and files the hub
// serves under /api/ (a job's before/after report), open in a new tab. Raw
// HTML is never rendered.
export function Markdown({ source, className }: { source: string; className?: string }) {
  if (!source?.trim()) return <p className={cn("text-sm text-muted-foreground", className)}>No text yet.</p>;
  return (
    <div className={cn("prose-hive", className)}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        skipHtml
        components={{
          a: ({ href, children }) => {
            const away = /^https?:\/\//.test(href || "") || /^\/api\//.test(href || "");
            return (
              <a href={href} target={away ? "_blank" : undefined} rel={away ? "noreferrer noopener" : undefined}>
                {children}
              </a>
            );
          },
        }}
      >
        {source}
      </ReactMarkdown>
    </div>
  );
}
