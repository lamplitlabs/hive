/** One line of a terminal: a command you type (`more` when it carries on the line above), or a note. */
export type TerminalLine = { cmd: string; more?: boolean; note?: undefined } | { note: string; cmd?: undefined; more?: undefined };

/** A terminal drawn in markup: real commands, no screenshot to keep fresh. A line too long for a narrow screen wraps, as it would in a narrow terminal. */
export function Terminal({ title, lines }: { title: string; lines: readonly TerminalLine[] }) {
  return (
    <figure className="night overflow-hidden rounded-2xl border border-border bg-[#070b1a]/85 shadow-pop backdrop-blur-sm">
      <figcaption className="flex items-center gap-2 border-b border-border px-4 py-3">
        <span className="size-2.5 rounded-full bg-white/15" aria-hidden="true" />
        <span className="size-2.5 rounded-full bg-white/15" aria-hidden="true" />
        <span className="size-2.5 rounded-full bg-white/15" aria-hidden="true" />
        <span className="ml-2 truncate font-mono text-xs text-muted-foreground">{title}</span>
      </figcaption>
      <pre className="px-4 py-4 font-mono text-[12.5px] leading-7 break-words whitespace-pre-wrap sm:px-5 sm:text-[13px]">
        <code>
          {lines.map((l, i) => (
            <span key={i} className={l.cmd !== undefined && !l.more ? "block pl-[2ch] -indent-[2ch]" : "block pl-[4ch] -indent-[4ch]"}>
              {l.cmd !== undefined ? (
                <>
                  <span className="text-teal select-none" aria-hidden="true">
                    {l.more ? "  " : "$ "}
                  </span>
                  <span className="text-foreground">{l.cmd}</span>
                </>
              ) : (
                <span className="text-[var(--code-comment)] italic">
                  {"  "}# {l.note}
                </span>
              )}
              {"\n"}
            </span>
          ))}
        </code>
      </pre>
    </figure>
  );
}
