import { cn } from "@/lib/utils";

// The brand mark: a hexagon cut into three brackets around a core, with a node
// in each gap - the brain, and the machines paired with it - on the
// mint-sky-violet tile. The only place the gradient appears (Pip, the mascot,
// wears the same one). Inside a `.brand-link` the brackets redraw themselves
// on hover. The geometry is scripts/brand.py's (`--print`); the same shape is
// drawn by the Mac app (HiveMarkView), its icon
// (apps/macos/Resources/AppIcon.svg), the favicon (app/icon.svg) and the README
// logo (.github/assets/hive-logo.svg).
export const HIVE_BRACKETS = [
  "M77.77 61L128 32L178.23 61",
  "M211.14 118L211.14 176L160.91 205",
  "M95.09 205L44.86 176L44.86 118",
] as const;
export const HIVE_NODES = [
  "M211.14 66.5L222.83 73.25L222.83 86.75L211.14 93.5L199.45 86.75L199.45 73.25z",
  "M128 210.5L139.69 217.25L139.69 230.75L128 237.5L116.31 230.75L116.31 217.25z",
  "M44.86 66.5L56.55 73.25L56.55 86.75L44.86 93.5L33.17 86.75L33.17 73.25z",
] as const;
export const HIVE_CORE = "M128 100L152.25 114L152.25 142L128 156L103.75 142L103.75 114z";
/** The length of one bracket, for the draw animation's dash. */
export const HIVE_BRACKET_LENGTH = 116;

export function HiveMark({ className }: { className?: string }) {
  return (
    <span aria-hidden className={cn("inline-flex size-8 shrink-0 items-center justify-center rounded-[9px] bg-brand shadow-[inset_0_0_0_1px_oklch(1_0_0/.25)]", className)}>
      <svg viewBox="0 0 256 256" className="size-[78%]" fill="none">
        <g stroke="white" strokeWidth="22" strokeLinejoin="miter" strokeMiterlimit="6">
          {HIVE_BRACKETS.map((d) => (
            <path key={d} d={d} className="hive-mark-path" />
          ))}
        </g>
        <g fill="white">
          {HIVE_NODES.map((d) => (
            <path key={d} d={d} />
          ))}
          <path d={HIVE_CORE} />
        </g>
      </svg>
    </span>
  );
}

// The same mark as a thin line, drawn once when it appears. Reduced motion
// shows it drawn.
export function HiveLine({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 256 256" fill="none" aria-hidden className={cn("size-7 text-primary", className)}>
      <g stroke="currentColor" strokeWidth="19" strokeLinejoin="miter" strokeMiterlimit="6">
        {HIVE_BRACKETS.map((d) => (
          <path key={d} d={d} className="hive-draw [--hive-length:116]" />
        ))}
      </g>
      <g fill="currentColor">
        {HIVE_NODES.map((d) => (
          <path key={d} d={d} />
        ))}
        <path d={HIVE_CORE} />
      </g>
    </svg>
  );
}
