// The Tasks page's logic without its markup, so node --test can read it:
// which board column a task sits in, which arrow its priority gets, the
// progress ring's dash, the list's sort and the New task review line.

export type TaskColumn = "planned" | "in_progress" | "review" | "done";

export const TASK_COLUMNS_ORDER: TaskColumn[] = ["planned", "in_progress", "review", "done"];

export const COLUMN_LABEL: Record<TaskColumn, string> = {
  planned: "Planned",
  in_progress: "In progress",
  review: "Asked you",
  done: "Done",
};

// Which of the four board columns a task belongs in. `open` is a task the
// lead has not planned yet, so it sits with Planned; `review` is the lead
// stopped on a question for you; done and cancelled both rest in Done.
export function columnOf(t: { status: string }): TaskColumn {
  switch (t.status) {
    case "in_progress":
      return "in_progress";
    case "review":
      return "review";
    case "done":
    case "cancelled":
      return "done";
    default:
      return "planned";
  }
}

// The tasks of a list sorted into the four columns, keeping the list's
// order inside each. Every column is present, even when empty, so the
// board always shows four headings with a count.
export function groupByColumn<T extends { status: string }>(tasks: T[]): Record<TaskColumn, T[]> {
  const out: Record<TaskColumn, T[]> = { planned: [], in_progress: [], review: [], done: [] };
  for (const t of tasks) out[columnOf(t)].push(t);
  return out;
}

export type PriorityIcon = "up" | "flat" | "down";

// Which mark stands for a priority: an up arrow for high and urgent (30+),
// a flat bar for normal (10-29), a down arrow for low. Normal is never an
// arrow pointing right: beside a card's title that reads as "open". The
// word next to it comes from priorityWord(); the icon never carries the
// meaning alone.
export function priorityIcon(p: number): PriorityIcon {
  if (p >= 30) return "up";
  if (p >= 10) return "flat";
  return "down";
}

// The stroke-dasharray for a progress ring of radius `r`: the done arc,
// then the gap. Clamped so a ring never overflows (done > total) or goes
// negative; an empty total shows an empty ring.
export function ringDash(done: number, total: number, r: number): { dash: string; circumference: number; fraction: number } {
  const circumference = 2 * Math.PI * r;
  const fraction = total > 0 ? Math.min(1, Math.max(0, done / total)) : 0;
  const arc = circumference * fraction;
  const round = (n: number) => Math.round(n * 100) / 100;
  return { dash: round(arc) + " " + round(circumference - arc), circumference: round(circumference), fraction };
}

// "1 of 2 steps", "No steps yet" - the ring's label.
export function stepsLine(done: number, total: number): string {
  if (!total) return "No steps yet";
  return done + " of " + total + (total === 1 ? " step" : " steps");
}

// Which of the status filter's words applies: the page keeps today's
// `status` search param (active = anything not closed; a status name; all).
export function matchesStatus(t: { status: string }, filter: string): boolean {
  if (!filter || filter === "active") return t.status !== "done" && t.status !== "cancelled";
  if (filter === "all") return true;
  return t.status === filter;
}

// The one-line review at the end of the New task wizard: "A high-priority
// task for demo-app, led by Pip." The lead's name is what the picker shows
// or, when the project has no named lead yet, "its lead agent".
export function reviewLine(input: { priorityWord: string; project: string; lead?: string }): string {
  const level = input.priorityWord.toLowerCase();
  const prio = level === "normal" ? "A task" : (level === "urgent" ? "An " : "A ") + level + "-priority task";
  const where = input.project ? " for " + input.project : "";
  const who = input.lead ? ", led by " + input.lead : ", led by its lead agent";
  return prio + where + who + ".";
}

// The "new" search param opens the New task dialog on load (/tasks/?new=1);
// other pages link here when they want the dialog without a button of
// their own. Anything truthy but "0" counts.
export function wantsNewTask(param: string | null | undefined): boolean {
  if (param === null || param === undefined) return false;
  return param !== "0" && param !== "false" && param !== "";
}
