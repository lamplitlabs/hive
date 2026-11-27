import { ArrowDownIcon, ArrowLeftIcon, ArrowRightIcon, ArrowUpIcon } from "lucide-react";
import { HiveMark } from "@/components/hive-mark";

type Way = "right" | "down" | "left" | "up";

// The loop is a ring of six steps, read clockwise from the top left. On a
// wide screen it is three steps across and two down; on a phone, two across
// and three down. `wide` and `small` say where a step stands in each and
// which way its arrow leaves it for the next step.
const STEPS: readonly { name: string; sub: string; wide: readonly [string, Way]; small: readonly [string, Way] }[] = [
  { name: "Suggest", sub: "an agent names a check", wide: ["sm:col-start-1 sm:row-start-1", "right"], small: ["col-start-1 row-start-1", "right"] },
  { name: "Vote", sub: "other agents decide", wide: ["sm:col-start-2 sm:row-start-1", "right"], small: ["col-start-2 row-start-1", "down"] },
  { name: "Build", sub: "on an experiment branch", wide: ["sm:col-start-3 sm:row-start-1", "down"], small: ["col-start-2 row-start-2", "down"] },
  { name: "Measure", sub: "fixtures and private tests", wide: ["sm:col-start-3 sm:row-start-2", "left"], small: ["col-start-2 row-start-3", "left"] },
  { name: "Archive", sub: "every result, kept or not", wide: ["sm:col-start-2 sm:row-start-2", "left"], small: ["col-start-1 row-start-3", "up"] },
  { name: "Deliver", sub: "a pull request", wide: ["sm:col-start-1 sm:row-start-2", "up"], small: ["col-start-1 row-start-2", "up"] },
];

/** The rules the loop works inside, as the constitution names them. */
const RULES = ["hard stops", "paths it may never touch", "a kill switch"];

const ICON = { right: ArrowRightIcon, down: ArrowDownIcon, left: ArrowLeftIcon, up: ArrowUpIcon } as const;

// An arrow stands in the gap between its step and the next one, in the middle of that gap.
const PLACE: Record<Way, string> = {
  right: "left-full top-0 h-full w-[var(--gap)] items-center justify-center",
  left: "right-full top-0 h-full w-[var(--gap)] items-center justify-center",
  down: "top-full left-0 h-[var(--gap)] w-full items-center justify-center",
  up: "bottom-full left-0 h-[var(--gap)] w-full items-center justify-center",
};

function Arrow({ way, className }: { way: Way; className: string }) {
  const Icon = ICON[way];
  return (
    <span className={`absolute text-teal ${PLACE[way]} ${className}`} aria-hidden="true">
      <Icon className="size-4" strokeWidth={2.25} />
    </span>
  );
}

/**
 * The self-improvement loop, drawn in HTML and CSS: a list a screen reader
 * reads in order, a ring with arrows to the eye. The ring stands inside the
 * constitution's frame, because that is the fact it draws: the loop works
 * inside the rules its owner wrote. The caller says where the figure stands
 * (`mx-auto` in a column of its own, nothing in running text).
 */
export function LoopDiagram({ className = "" }: { className?: string }) {
  return (
    <figure className={`w-full max-w-[34rem] ${className}`}>
      <div className="bg-brand rounded-[1.35rem] p-[1.5px]">
        <div className="rounded-[1.25rem] bg-background px-4 pt-4 pb-5 sm:px-6 sm:pt-5 sm:pb-6">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-2 sm:gap-x-3">
            <span className="flex items-center gap-2 text-[15px] font-semibold tracking-tight">
              <HiveMark className="mark-sweep size-5" /> Your constitution
            </span>
            {/* On a narrow screen the rules flow in one line with the name, so none of them is left on a row of its own. */}
            <span className="contents sm:flex sm:flex-wrap sm:gap-1.5">
              <span className="sr-only">: </span>
              {RULES.map((rule, i) => (
                <span key={rule} className="rounded-full border border-border bg-muted px-2.5 py-0.5 text-[13px] text-muted-foreground">
                  {rule}
                  {i < RULES.length - 1 && <span className="sr-only">, </span>}
                </span>
              ))}
            </span>
          </div>
          {/* The utilities undo what a page's long-text styles give a list: this one is a figure. */}
          <ol
            aria-label="The self-improvement loop, step by step. After the last step it starts again."
            className="m-0 mt-5 grid max-w-none list-none auto-rows-fr grid-cols-2 gap-[var(--gap)] p-0 [--gap:1.75rem] sm:grid-cols-3 sm:[--gap:2rem]"
          >
            {STEPS.map((s, i) => (
              <li key={s.name} className={`relative m-0 flex flex-col rounded-xl border border-border bg-card p-3 shadow-card sm:p-3.5 ${s.small[0]} ${s.wide[0]}`}>
                <span className="flex items-center gap-2">
                  <span className="hex inline-flex h-6 w-[1.3rem] shrink-0 items-center justify-center bg-teal font-mono text-xs font-semibold text-background" aria-hidden="true">
                    {i + 1}
                  </span>
                  <span className="text-[15px] font-semibold tracking-tight">
                    {s.name}
                    <span className="sr-only">: </span>
                  </span>
                </span>
                <span className="mt-1.5 text-sm leading-snug text-balance text-muted-foreground">{s.sub}</span>
                <Arrow way={s.small[1]} className="flex sm:hidden" />
                <Arrow way={s.wide[1]} className="hidden sm:flex" />
              </li>
            ))}
          </ol>
        </div>
      </div>
      <figcaption className="mx-auto mt-4 max-w-[30rem] text-center text-sm text-muted-foreground">
        The loop runs clockwise and starts again after it delivers. It never leaves the constitution you write.
      </figcaption>
    </figure>
  );
}
