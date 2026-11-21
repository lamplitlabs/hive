import { CopyButton } from "@/components/copy-button";
import { highlightCode } from "@/lib/markdown";

/**
 * A sample of code on a page written by hand: highlighted at build time,
 * scrolling inside its own box. With a title, or with `copy` (the bare command
 * to put on the clipboard), the box gets a bar across its top.
 */
export function CodeBlock({ code, lang = "bash", title, copy, copyWhat }: { code: string; lang?: string; title?: string; copy?: string; copyWhat?: string }) {
  const pre = (
    <pre className={title || copy ? "code-block rounded-t-none border-t-0" : "code-block"} tabIndex={0}>
      <code dangerouslySetInnerHTML={{ __html: highlightCode(code, lang) }} />
    </pre>
  );
  if (!title && !copy) return pre;
  return (
    <figure>
      <figcaption className="flex min-h-11 items-center justify-between gap-3 rounded-t-xl border border-border bg-muted/60 py-1 pr-1.5 pl-4 text-xs font-medium text-muted-foreground">
        <span>{title}</span>
        {copy && <CopyButton text={copy} what={copyWhat} className="h-8" />}
      </figcaption>
      {pre}
    </figure>
  );
}

/** One command to run, with a button that copies it. On a narrow screen the command wraps and the button sits under it, so its end is never hidden. */
export function CopyLine({ command, what, className = "" }: { command: string; what?: string; className?: string }) {
  return (
    <div className={`flex flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border border-border bg-code p-2 pl-4 ${className}`}>
      <div className="flex min-w-0 flex-1 basis-full items-baseline gap-3 sm:basis-0">
        <span className="font-mono text-[13px] text-teal select-none" aria-hidden="true">
          $
        </span>
        <code className="min-w-0 flex-1 py-1 font-mono text-xs leading-relaxed break-words whitespace-pre-wrap sm:text-[13px]">{command}</code>
      </div>
      <CopyButton text={command} what={what} className="w-full justify-center sm:w-auto" />
    </div>
  );
}
