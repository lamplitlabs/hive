// Lockdown (sandbox) mode on the Chat page: the words and the rules, kept
// apart from the components so they can be tested without a browser.
//
// A conversation in lockdown runs its agent with every outside thing
// blocked: no network (localhost stays, for a dev server or a test), writes
// confined to its worktree and the temp/build folders, no fetch, web_search
// or MCP tool, credentials unreadable, nothing pushed or landed from inside.
// Only work inside the sandbox is possible. The page says so with the vault
// theme (`lockdown-*` classes in globals.css) wherever the conversation shows.

import type { Conversation, ConversationLive } from "@/lib/api";

// Whether the conversation is in lockdown now: the live state is the
// session's own word (it follows a flip at once); the row is the fallback
// for a list entry or an older hub without `live`.
export function lockdownOf(
  c: Pick<Conversation, "lockdown">,
  live?: Pick<ConversationLive, "lockdown"> | null,
): boolean {
  if (live && typeof live.lockdown === "boolean") return live.lockdown;
  return !!c.lockdown;
}

// The one-line explanation beside the switch, on the New conversation form
// and in a conversation's composer.
export function lockdownHint(on: boolean): string {
  return on
    ? "Sandboxed: no network, writes stay in its worktree, no fetch, web search or MCP, no credentials. Only work inside."
    : "Works as usual: reaches the web and the tools you connected.";
}

// What the header's bar says while the sandbox holds.
export const LOCKDOWN_BAR =
  "Lockdown: this agent works in a sandbox. Nothing reaches outside this machine - no network, no pushes, no credentials - and its files stay in the worktree. Landing is yours, from here.";

// The toast after a flip. A session restarts inside (or outside) the
// sandbox on its next turn; a turn that is running finishes the way it
// started.
export function lockdownFlipMessage(on: boolean, running: boolean): string {
  const when = running
    ? "from the next turn (this one finishes as it started)"
    : "from the next turn";
  return on
    ? `Lockdown on ${when}: the agent's session restarts inside the sandbox.`
    : `Lockdown off ${when}: the agent's session restarts with the outside open again.`;
}

// The badge's label where the conversation is listed.
export const LOCKDOWN_BADGE = "in lockdown (sandbox)";
