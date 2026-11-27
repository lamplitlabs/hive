// The Hive mark: a hexagon cut into three brackets around a core, with a node
// in each gap - the brain, and the machines paired with it. The geometry is
// scripts/brand.py's (`python3 scripts/brand.py --print`); the console, the
// favicon, the README banner and the Mac app draw the same shape.
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

/** The mark in the text colour. */
export function HiveMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 256 256" aria-hidden="true" className={className}>
      <g fill="none" stroke="currentColor" strokeWidth="22" strokeLinejoin="miter" strokeMiterlimit="6">
        {HIVE_BRACKETS.map((d) => (
          <path key={d} d={d} />
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

/** The mark in white on its brand tile, as the console's sidebar draws it. */
export function HiveTile({ className = "size-9" }: { className?: string }) {
  return (
    <span className={`bg-brand inline-flex items-center justify-center rounded-[26%] text-white shadow-card ${className}`}>
      <HiveMark className="size-[78%]" />
    </span>
  );
}
