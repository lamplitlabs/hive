// The hive cell: a hexagon with a dot at its centre, the same shape the
// console's sidebar, the favicon and the Mac app draw.
export function HiveMark({ className, draw = false }: { className?: string; draw?: boolean }) {
  return (
    <svg viewBox="0 0 32 32" aria-hidden="true" className={className}>
      <path
        d="M16 5l9.5 5.5v11L16 27l-9.5-5.5v-11z"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.4"
        strokeLinejoin="round"
        pathLength={1}
        className={draw ? "hive-draw" : undefined}
      />
      <circle cx="16" cy="16" r="1.8" fill="currentColor" />
    </svg>
  );
}

/** The mark on its brand tile, as the README's logo draws it. */
export function HiveTile({ className = "size-9" }: { className?: string }) {
  return (
    <span className={`bg-brand inline-flex items-center justify-center rounded-[28%] text-white shadow-card ${className}`}>
      <HiveMark className="size-[70%]" />
    </span>
  );
}
