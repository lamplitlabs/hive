// What the owner just sent, shown in the transcript before the hub's poll
// brings the row back. Without it the page showed the old transcript and a
// bare "Working…" for as long as the poll took - a message that had gone
// nowhere, to the eye. Pure: no React and no DOM, so sent-messages.test.mjs
// can drive it.

export interface SentAttachment {
  name: string;
  size: number;
  path: string;
}

export interface SentMessage {
  // A key for the list; unique within the page.
  key: string;
  // The words the row will carry (the hub's own fallback for a message
  // that was only files).
  text: string;
  attachments: SentAttachment[];
  // When it was sent, ms since the epoch. A poll requested before then
  // cannot know about it, so its answer says nothing about whether the
  // row arrived.
  at: number;
  // Steered into the running turn (compose-mode.ts) rather than sent to
  // start one: the row under it says so.
  steered?: boolean;
}

// The live state a poll answers with, as far as reconciling needs it.
export interface SentLive {
  status: string;
  queued: string[];
}

// A sent message is given up after this long without its row: the hub
// dropped it (a close, a restart) and said so with a note of its own.
export const SENT_TTL_MS = 2 * 60 * 1000;

let seq = 0;

// The words the hub writes for a message: the owner's, or its own line for
// a message that was only files (Manager::send_with).
export function sentWords(text: string, attachments: number): string {
  const t = text.trim();
  if (t) return t;
  if (attachments === 1) return "Look at the attached file.";
  if (attachments > 1) return "Look at the attached files.";
  return "";
}

export function sentMessage(
  text: string,
  attachments: SentAttachment[],
  at: number = Date.now(),
  steered = false,
): SentMessage {
  seq += 1;
  return {
    key: "sent-" + at + "-" + seq,
    text: sentWords(text, attachments.length),
    attachments,
    at,
    ...(steered ? { steered: true } : {}),
  };
}

// Whether a message sent while the agent was in this state goes to the
// transcript at once. Idle, the hub writes the row before it answers and
// the turn starts on it; running or waiting on a question, the row waits
// behind the turn in flight and the hub lists the message as queued, which
// the page already shows in its place.
export function sentShowsNow(status: string): boolean {
  return status !== "running" && status !== "waiting";
}

// The sent messages still worth showing after a poll answered: not the
// ones whose row arrived (a user row with the same words), and not the
// ones the hub will never write - the turn is over with nothing queued, or
// the row is overdue.
export function reconcileSent(
  sent: SentMessage[],
  arrived: { role: string; content: string }[],
  live: SentLive,
  requestedAt: number,
): SentMessage[] {
  if (!sent.length) return sent;
  const settled =
    live.status !== "running" &&
    live.status !== "waiting" &&
    !live.queued.length;
  const kept = sent.filter((s) => {
    if (arrived.some((m) => m.role === "user" && m.content === s.text))
      return false;
    // The poll went out before the send: it cannot say.
    if (requestedAt < s.at) return true;
    if (settled) return false;
    return requestedAt - s.at <= SENT_TTL_MS;
  });
  return kept.length === sent.length ? sent : kept;
}

// The queued list the hub reports, less what the page already shows as a
// sent message: right after a send the hub's snapshot can still list the
// message the worker is about to take up.
export function queuedBesides(queued: string[], sent: SentMessage[]): string[] {
  if (!sent.length || !queued.length) return queued;
  const shown = new Set(sent.map((s) => s.text));
  const rest = queued.filter((q) => !shown.has(q));
  return rest.length === queued.length ? queued : rest;
}
