import { HiveMark } from "@/components/hive-mark";

// The loop as a honeycomb: six cells around the constitution, read clockwise
// from the top left. Each cell sits where a hexagon's neighbour sits, so the
// percentages below are geometry, not taste: a pointy-top hexagon is a third
// of the figure wide and two fifths high.
const STEPS = [
  { name: "Suggest", sub: "an agent names a check", x: 33.333, y: 20 },
  { name: "Vote", sub: "other agents decide", x: 66.667, y: 20 },
  { name: "Build", sub: "on an experiment branch", x: 83.333, y: 50 },
  { name: "Measure", sub: "fixtures and private tests", x: 66.667, y: 80 },
  { name: "Archive", sub: "every result, kept or not", x: 33.333, y: 80 },
  { name: "Deliver", sub: "a pull request", x: 16.667, y: 50 },
] as const;

const cell = "absolute h-2/5 w-1/3 p-[1.2%]";
const at = (x: number, y: number) => ({ left: `${x - 16.667}%`, top: `${y - 20}%` });

/** The self-improvement loop, drawn in HTML and CSS: a list a screen reader reads in order, a honeycomb to the eye. */
export function LoopDiagram({ className = "" }: { className?: string }) {
  return (
    <figure className={className}>
      <div className="relative mx-auto aspect-[5.196/5] w-full max-w-[30rem]">
        {/* The utilities undo what a page's long-text styles give a list: this one is a figure. */}
        <ol aria-label="The self-improvement loop, step by step" className="m-0 max-w-none list-none p-0">
          {STEPS.map((s, i) => (
            <li key={s.name} className={`${cell} m-0`} style={at(s.x, s.y)}>
              <div className="hex h-full bg-border p-[1.5px]">
                <div className="hex flex h-full flex-col items-center justify-center bg-card px-[14%] text-center">
                  <span className="font-mono text-[10px] font-semibold text-teal sm:text-[11px]">
                    {i + 1}
                    <span className="sr-only">. </span>
                  </span>
                  <span className="text-[13px] leading-tight font-semibold tracking-tight sm:text-[15px]">
                    {s.name}
                    <span className="sr-only">: </span>
                  </span>
                  <span className="mt-0.5 text-[10px] leading-[1.25] text-muted-foreground sm:text-[11.5px]">{s.sub}</span>
                </div>
              </div>
            </li>
          ))}
        </ol>
        <div className={cell} style={at(50, 50)}>
          <div className="hex bg-brand h-full p-[2px]">
            <div className="night hex flex h-full flex-col items-center justify-center bg-[var(--night-1)] px-[12%] text-center">
              <HiveMark className="mark-sweep size-5 sm:size-6" />
              <span className="mt-1 max-w-[6.5em] text-[12px] leading-tight font-semibold sm:text-[14px]">Your constitution</span>
            </div>
          </div>
        </div>
      </div>
      <figcaption className="mx-auto mt-5 max-w-[30rem] text-center text-sm text-muted-foreground">
        The loop runs clockwise, inside the constitution you write: hard stops, paths it may never touch, a kill switch.
      </figcaption>
    </figure>
  );
}
