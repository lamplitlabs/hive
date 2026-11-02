// Steering versus queueing on the Chat page's composer: the rules and the
// words, kept apart from the component so compose-mode.test.mjs can drive
// them without a browser.
//
// While the agent's turn runs, a message can go two ways. STEER pushes it
// into the running turn: the agent reads it before its next model call
// (`POST chat/{id}/steer`), and its row is in the transcript at once, marked
// `extra.steered`. QUEUE puts it behind the turn (`POST chat/{id}/message`):
// it starts a turn of its own once this one ends, and the hub lists it as
// queued until then. Idle, there is only SEND, which starts a turn.
//
// The owner's choice: while a turn runs, plain Enter steers and ⌘/Ctrl+Enter
// queues; idle, Enter sends as always. Both also exist as buttons.

export type ComposeMode = "send" | "steer" | "queue";

// Whether a turn is open: the agent is working, or waiting on a question
// or an approval. The composer's two ways exist only then.
export function turnOpen(status: string): boolean {
  return status === "running" || status === "waiting";
}

// What Enter does given the live status and whether ⌘ or Ctrl was held:
// idle it sends (the modifier changes nothing, as before); with a turn open
// it steers, or queues with the modifier.
export function composeMode(status: string, modifier: boolean): ComposeMode {
  if (!turnOpen(status)) return "send";
  return modifier ? "queue" : "steer";
}

// The live state the hub answers a steer with, as far as deciding what to
// show needs it.
export interface SteerAnswer {
  queued?: string[];
  steered?: string[];
}

// Whether the message the owner just sent goes to the transcript at once,
// so the page shows it before the poll brings its row. A steer writes the
// row immediately - unless the hub could not steer (the turn ended just
// before it arrived) and queued it instead, which its answer says by
// listing the words under `queued`; then the queued list below the
// transcript is its place. A send or a queue shows now only when the
// agent was idle (sent-messages.ts, sentShowsNow).
export function showsNow(
  mode: ComposeMode,
  status: string,
  answer: SteerAnswer | null | undefined,
  words: string,
): boolean {
  if (mode === "steer") return !(answer?.queued ?? []).includes(words);
  return !turnOpen(status);
}

// The steered list an older hub's live state does not carry.
export function steeredOf(live: { steered?: string[] } | null | undefined): string[] {
  return live?.steered ?? [];
}

// The buttons' names and tooltips, which name the shortcuts.
export const STEER_LABEL = "Steer the running turn";
export const STEER_TITLE =
  "Steer: the agent reads it before its next step (Enter)";
export const QUEUE_LABEL = "Queue message";
export const QUEUE_TITLE = "Queue: it goes after this turn (⌘/Ctrl+Enter)";
export const SEND_LABEL = "Send";
export const SEND_TITLE = "Send (Enter)";

// What the empty composer says.
export function composePlaceholder(running: boolean): string {
  return running
    ? "Steer the agent now; ⌘/Ctrl+Enter queues it for after this turn."
    : "Tell the agent what to do next…";
}

// The line under the composer about the keys.
export function composeKeysHint(running: boolean): string {
  return running
    ? "Enter steers the running turn (the agent reads it before its next step), ⌘/Ctrl+Enter queues it for after, Shift+Enter for a new line."
    : "Enter sends, Shift+Enter for a new line.";
}

// The word under one of the owner's rows: "Sent" until the agent answered,
// "Answered" after; a steered row says so, since it reached the agent
// mid-turn rather than starting one.
export function sentLabel(status: "sent" | "answered", steered: boolean): string {
  if (steered) return status === "answered" ? "Answered · steered" : "Steered";
  return status === "answered" ? "Answered" : "Sent";
}

// The hint beside a steered message the agent has not taken up yet.
export const STEERED_HINT = "reaches the agent at its next step";
