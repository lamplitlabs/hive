import { cn } from "@/lib/utils";

// The brand mark: the logo's hive cell - a hexagon with a dot at its centre -
// on the mint-sky-violet tile. The only place the gradient appears (Pip, the
// mascot, wears the same one). Inside a `.brand-link` the cell redraws itself
// on hover. The same shape is drawn by the Mac app (HiveMarkView), its icon
// (apps/macos/Resources/AppIcon.svg), the favicon (app/icon.svg) and the README
// logo (.github/assets/hive-logo.svg).
export const HIVE_CELL_PATH = "M128 36l78 45v90l-78 45-78-45V81z";

export function HiveMark({ className }: { className?: string }) {
  return (
    <span aria-hidden className={cn("inline-flex size-8 shrink-0 items-center justify-center rounded-[9px] bg-brand shadow-[inset_0_0_0_1px_oklch(1_0_0/.25)]", className)}>
      <svg viewBox="0 0 256 256" className="size-[78%]" fill="none">
        <path d={HIVE_CELL_PATH} stroke="white" strokeWidth="22" strokeLinecap="round" strokeLinejoin="round" className="hive-mark-path" />
        <circle cx="128" cy="126" r="14" fill="white" />
      </svg>
    </span>
  );
}

// The same cell as a thin line, drawn once when it appears. Reduced motion
// shows it drawn.
export function HiveLine({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 28 28" fill="none" aria-hidden className={cn("size-7 text-primary", className)}>
      <path
        d="M14 3.5l8.5 4.9v9.8L14 23.1l-8.5-4.9V8.4z"
        stroke="currentColor"
        strokeWidth="2.25"
        strokeLinecap="round"
        strokeLinejoin="round"
        className="hive-draw [--hive-length:60]"
      />
      <circle cx="14" cy="13.5" r="1.6" fill="currentColor" />
    </svg>
  );
}
