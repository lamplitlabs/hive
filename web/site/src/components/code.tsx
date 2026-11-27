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
      <figcaption className="flex min-h-11 items-center justify-between gap-3 rounded-t-xl border border-border bg-muted/60 py-1 pr-1.5 pl-4 text-[13px] font-medium text-muted-foreground">
        {/* A title that is a path is set as one: any title with a slash or a dot in it and no space. */}
        <span className={title && /^[^\s]*[./][^\s]*$/.test(title) ? "font-mono text-[12.5px]" : undefined}>{title}</span>
        {copy && <CopyButton text={copy} what={copyWhat} className="h-8" />}
      </figcaption>
      {pre}
    </figure>
  );
}

/** One command to run, with a button that copies it. On a narrow screen the command wraps and the button sits under it, so its end is never hidden. */
export function CopyLine({ command, what, primary = false, className = "" }: { command: string; what?: string; /** Where copying the command is what the page asks for. */ primary?: boolean; className?: string }) {
  return (
    // On a narrow screen the button is a row of its own, as far from the left edge of the box as from the right; the command above it keeps its inset.
    <div className={`flex flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border border-border bg-code p-2 sm:pl-4 ${className}`}>
      <div className="flex min-w-0 flex-1 basis-full items-baseline gap-3 pl-2 sm:basis-0 sm:pl-0">
        <span className="font-mono text-[13px] text-teal select-none" aria-hidden="true">
          $
        </span>
        {/* Each word is a box of its own: a narrow screen breaks the line between words, never inside "--role" or an address, unless a word alone is too long. */}
        <code className="min-w-0 flex-1 py-1 font-mono text-xs leading-relaxed break-words sm:text-[13px]">
          {command.split(" ").map((word, i) => (
            <span key={i}>
              {i > 0 && " "}
              <span className="inline-block max-w-full">{word}</span>
            </span>
          ))}
        </code>
      </div>
      <CopyButton text={command} what={what} primary={primary} className="w-full justify-center sm:w-auto" />
    </div>
  );
}
