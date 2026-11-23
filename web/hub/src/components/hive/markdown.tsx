"use client";

import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { diagramLabel, isDiagram } from "@/lib/diagram";
import { cn } from "@/lib/utils";

// The text of a hast node: the characters of a code block, whatever it nests.
type HastNode = { type?: string; value?: string; tagName?: string; properties?: { className?: unknown }; children?: HastNode[] };
function textOf(node: HastNode | undefined): string {
  if (!node) return "";
  if (node.type === "text") return node.value || "";
  return (node.children || []).map(textOf).join("");
}

// Record bodies, briefs and chat messages are Markdown stored in SQL. Links
// to console pages open in the same tab; absolute links, and files the hub
// serves under /api/ (a job's before/after report), open in a new tab. Raw
// HTML is never rendered. A code block drawn with box-drawing characters is a
// diagram (lib/diagram.ts): its lines are closed up (.prose-hive pre.diagram)
// and a screen reader gets its words.
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
          pre: ({ node, children }) => {
            const code = ((node as HastNode | undefined)?.children || []).find((c) => c.tagName === "code");
            const classes = Array.isArray(code?.properties?.className) ? code.properties.className.map(String) : [];
            const lang = classes.find((c) => c.startsWith("language-"))?.slice("language-".length) ?? "";
            const text = textOf(code);
            if (!code || !isDiagram(text, lang)) return <pre>{children}</pre>;
            return (
              <pre className="diagram" role="img" aria-label={diagramLabel(text)} tabIndex={0}>
                {children}
              </pre>
            );
          },
        }}
      >
        {source}
      </ReactMarkdown>
    </div>
  );
}
