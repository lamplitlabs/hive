import type { AskOption, OwnerQuestion } from "./api";

// The inbox order for questions that wait for the owner: the evolution
// loop's stop questions first (the loop is halted until one is answered),
// live threads before stale ones, newest first within each group.
export function sortOwnerQuestions<T extends Pick<OwnerQuestion, "stop" | "stale" | "date" | "id">>(qs: readonly T[]): T[] {
  return [...qs].sort((a, b) => Number(!!b.stop) - Number(!!a.stop) || Number(!!a.stale) - Number(!!b.stale) || (a.date < b.date ? 1 : a.date > b.date ? -1 : a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
}

// The labels a post gives the owner's two buttons, from its own `Yes: ...`
// and `No: ...` lines (the security watcher writes "Yes: Upgrade it" /
// "No: Leave it to me"); plain Yes / No when the post names none. Only a
// short label counts: a sentence is the explanation under it, not a button.
export function decisionLabels(body: string | undefined, fallback: { yes: string; no: string } = { yes: "Yes", no: "No" }): { yes: string; no: string } {
  const pick = (word: "Yes" | "No"): string | null => {
    const m = new RegExp("^\\s*\\*{0,2}" + word + "\\*{0,2}:\\s*(.+?)\\s*$", "m").exec(body || "");
    const label = m ? m[1].replace(/[.!]+$/, "").trim() : "";
    return label && label.length <= 40 ? label : null;
  };
  return { yes: pick("Yes") ?? fallback.yes, no: pick("No") ?? fallback.no };
}

// The longest label an answer carries (it is a button) and the most answers
// one question offers; the hub's parser (hive-board ask_options) agrees.
export const MAX_OPTION_LABEL = 60;
export const MAX_OPTIONS = 6;

function splitLabel(text: string): { label: string; effect: string } {
  const t = text.trim().replace(/^\*+|\*+$/g, "").trim();
  let cut = -1;
  let len = 0;
  for (const sep of [" - ", " \u2014 ", " \u2013 "]) {
    const i = t.indexOf(sep);
    if (i >= 0 && (cut < 0 || i < cut)) {
      cut = i;
      len = sep.length;
    }
  }
  const label = (cut >= 0 ? t.slice(0, cut) : t).trim().replace(/^[`"]+|[`"]+$/g, "").replace(/[.!]+$/, "").trim();
  return { label, effect: cut >= 0 ? t.slice(cut + len).trim() : "" };
}

const ANSWER_LINE = /^[ \t]*[-*]?[ \t]*\**(option|yes|no|recommended)\**[ \t]*:[ \t]*(.+?)[ \t]*$/i;

// The answer lines of a body, from its last block of such lines: lines in
// fenced code blocks are never answers (a diff's `-no: cache`), and only
// blank lines may sit between the lines of one block (TASK-149 item 12; the
// hub's hive-board answer_lines reads it the same way).
function answerLines(body: string): Array<[string, string]> {
  const blocks: Array<Array<[string, string]>> = [];
  let current: Array<[string, string]> = [];
  let fence = false;
  for (const line of body.split(/\r?\n/)) {
    if (/^ {0,3}(```|~~~)/.test(line)) {
      fence = !fence;
      if (current.length) {
        blocks.push(current);
        current = [];
      }
      continue;
    }
    if (fence || !line.trim()) continue;
    const m = ANSWER_LINE.exec(line);
    if (m) current.push([m[1].toLowerCase(), m[2]]);
    else if (current.length) {
      blocks.push(current);
      current = [];
    }
  }
  if (current.length) blocks.push(current);
  return blocks.pop() ?? [];
}

// The answers a post names in its body - `Option: <label> - <what it does>`
// lines, or `Yes:` and `No:` - and the one a `Recommended: <label> - <why>`
// line marks, recommended first. The same reading as the hub's (TASK-129),
// for an older hub whose rows carry no `options`.
export function askOptions(body: string | undefined): { options: AskOption[]; why: string } {
  const options: AskOption[] = [];
  let recommended: { label: string; effect: string } | null = null;
  for (const [word, text] of answerLines(body || "")) {
    const { label, effect } = splitLabel(text);
    if (!label) continue;
    if (word === "recommended") {
      recommended ??= { label, effect };
      continue;
    }
    if (label.length > MAX_OPTION_LABEL) continue;
    const answer = word === "option" ? "" : (word as "yes" | "no");
    if (answer && options.some((o) => o.answer === answer)) continue;
    if (options.some((o) => o.label.toLowerCase() === label.toLowerCase())) continue;
    if (options.length >= MAX_OPTIONS) continue;
    options.push({ label, effect, answer, recommended: false });
  }
  let why = "";
  if (recommended) {
    const want = recommended.label.toLowerCase();
    const pick = options.find((o) => o.label.toLowerCase() === want) ?? options.find((o) => o.answer === want);
    if (pick) {
      pick.recommended = true;
      why = recommended.effect;
      options.sort((a, b) => Number(b.recommended) - Number(a.recommended));
    }
  }
  return { options, why };
}

// How the owner answers a question (TASK-129: "always give option no
// text.. and add recommended one as well.. always"):
// - `pick`: the answers to pick from, the recommended one first and marked;
//   picking one records it. `byLabel` sends the label (the hub reads what it
//   means from the post); otherwise the yes or no is sent.
// - `with-lead`: the post names no answers, or none recommended - it is not
//   the owner's to answer until its lead names them, and nothing is typed.
// A stop of the self-improvement loop always offers carry on / keep paused;
// keeping it paused is recommended unless the post says otherwise.
export type QuestionAnswers =
  | { kind: "pick"; options: AskOption[]; why: string; byLabel: boolean }
  | { kind: "with-lead"; missing: string };

export function questionAnswers(q: Pick<OwnerQuestion, "body" | "options" | "recommended_why" | "needs_options">, stop: boolean, stopLabels: { yes: string; no: string } = { yes: "Carry on", no: "Keep paused" }): QuestionAnswers {
  const fromHub = q.options && q.options.length ? { options: [...q.options], why: q.recommended_why ?? "" } : null;
  const read = fromHub ?? askOptions(q.body);
  const marked = read.options.filter((o) => o.recommended).length;
  if (read.options.length >= 2 && marked === 1) return { kind: "pick", options: read.options, why: read.why, byLabel: true };
  if (stop) {
    return {
      kind: "pick",
      options: [
        { label: stopLabels.no, effect: "the loop stays stopped until the gate is resolved", answer: "no", recommended: true },
        { label: stopLabels.yes, effect: "the loop resumes", answer: "yes", recommended: false },
      ],
      why: "",
      byLabel: false,
    };
  }
  const missing = q.needs_options || (read.options.length < 2 ? "it names no answers to pick" : marked === 0 ? "it names no recommended answer" : "it marks more than one answer recommended");
  return { kind: "with-lead", missing };
}

// A question the owner answers in words rather than with yes or no: it asks
// which, what, how, where, who, when or why, and the post names no `Yes:` /
// `No:` buttons of its own. "Which payment sandbox should the fixtures hit?"
// can't be answered with "Yes"; "Should we upgrade?" can.
export function isOpenQuestion(summary: string | undefined, body: string | undefined): boolean {
  if (/^\s*\*{0,2}(Yes|No|Option)\*{0,2}:/im.test(body || "")) return false;
  return /^\s*(which|what|how|where|who|whom|whose|when|why)\b/i.test(summary || "");
}

// What the card calls a post that waits for the owner, by its kind: a
// question is asked, a finding or a handoff is put to the owner for a call.
export function askTitle(kind: string | undefined, asker: string): string {
  switch (kind) {
    case "finding":
      return "Your call, from " + asker;
    case "handoff":
      return "Handed to you by " + asker;
    case "suggestion":
      return "Proposed to you by " + asker;
    default:
      return "Question from " + asker;
  }
}
