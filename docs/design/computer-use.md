# Computer use: the tool shape and where it plugs in

Status: design note (Tier 1). Nothing here is built or enabled. Whether
PULSE agents may control the desktop at all, and under which guards, is
decided by **ADR-048** ("Agents Control the Desktop Only Where the Owner
Turned It On"). ADR-048 is a record in the store, not a file under
`docs/decisions/`; read it with `pulse record show ADR-048`. The owner
accepted it on 2026-11-02 and its delivery is "not started". This note
maps the tool that ADR describes onto the harness as it stands, so the work
that delivers it is plumbing, not new design. Where this note and the ADR
differ, the ADR wins.

Task: TASK-052 (the owner asked that agents can work the computer:
screenshot, mouse, keyboard).

## 1. The tool

One native harness tool, `computer`, with an `action` argument. It is one
tool rather than four so that a single permission rule (`computer`,
`computer(click:*)`) and a single approval card cover it, the way `memory`
and `process` work today (`crates/pulse-harness/src/memory.rs`
`memory_tool`, `crates/pulse-harness/src/background.rs` `process_tool`).

```json
{
  "name": "computer",
  "description": "Look at and work the owner's screen, inside the apps the owner allowed for this project. screenshot returns an image of the allowed apps' windows (the rest is blank) with its size in points; click presses the mouse at x,y in that screenshot's points (left, right or double); type types text into the frontmost app; key presses one key or chord (return, escape, tab, cmd+l). Every action is shown to the owner and recorded. If the owner is using the machine, stop and ask.",
  "parameters": {
    "type": "object",
    "properties": {
      "action": {"type": "string", "enum": ["screenshot", "click", "type", "key"], "description": "What to do."},
      "x": {"type": "integer", "description": "For click: points from the screenshot's left edge."},
      "y": {"type": "integer", "description": "For click: points from the screenshot's top edge."},
      "button": {"type": "string", "enum": ["left", "right", "double"], "description": "For click (default left)."},
      "text": {"type": "string", "description": "For type: the text to type, at most 2000 characters."},
      "key": {"type": "string", "description": "For key: a key name or chord such as return, tab, escape, cmd+l, shift+tab."}
    },
    "required": ["action"]
  }
}
```

| action | needs | does | result text |
|---|---|---|---|
| `screenshot` | Screen Recording | captures the main display, keeps the allowlisted apps' windows and blanks the rest, scales to at most 1280 px on the long side, PNG | `screenshot 1280x800 points, frontmost com.apple.Safari; image attached below` |
| `click` | Accessibility | checks the frontmost app against the allowlist, maps the points back to display coordinates, posts mouse down/up (twice for `double`) | `clicked left at 412,233 in com.apple.Safari` |
| `type` | Accessibility | checks the frontmost app, posts the text as Unicode keyboard events | `typed 18 characters into com.apple.Safari` |
| `key` | Accessibility | checks the frontmost app, posts the chord | `pressed cmd+l in com.apple.Safari` |

Every action other than `screenshot` ends with a short settle (about 300
ms) and does **not** return an image. The model asks for a screenshot when
it wants to see the result. That keeps one image per look instead of one
per keystroke, which matters for the request budget (section 4).

Coordinates are in the last screenshot's points, never raw display pixels.
The backend keeps the scale and origin of the last screenshot and refuses
a click when no screenshot has been taken in this session ("take a
screenshot first"). A Retina display and the downscale are then the
backend's concern, not the model's.

Scroll, drag and move can join the same `action` enum later without a new
ADR (ADR-048 Scope). Clipboard reads, launching apps by path and system
settings are out of scope.

## 2. Where it plugs into the harness

All paths are `crates/pulse-harness/src/harness.rs` unless named.

| Concern | Existing seam | What computer use adds |
|---|---|---|
| Tool name and schema | `NATIVE_TOOL_NAMES`, `process_tool`, `memory_tool` | `pub const COMPUTER_TOOL: &str = "computer"` and `computer_tool()` in a new module `crates/pulse-harness/src/computer.rs` |
| Offering it | `Harness::tools()` and `tool_names()` push optional tools behind `offers_*` gates (`offers_question` = a `Ui` and depth 0) | `offers_computer()`: `owner.computer_use != off` **and** the project is on `owner.computer_use.allow` **and** `offers_question()` (an interactive session with a human; never a job, never a subagent at depth > 0) **and** `!cfg.read_only` **and** `!locked_down()` **and** the backend reports a GUI session **and** the model accepts images (section 5). Both `tools()` and `tool_names()` get the same line, like `fetch` and `web_search`. |
| Dispatch | `dispatch_tool_call_inner` matches on `call.name` | an arm `COMPUTER_TOOL => self.computer_call(cancel, call)` returning a `ToolOutcome` plus images (section 3) |
| Permission rules | `subjects_of(tool, arguments, cwd)`; `gates` refuses on `harness.deny_tools`; `permissions.withdraws` drops a tool denied whole | `subjects_of` gains an arm `COMPUTER_TOOL => vec![format!("{action}:{bundle_id}")]`, so `computer(type:*)` or `computer(*:com.apple.Terminal)` match. A deny rule wins as it does today. |
| Hooks | `pre_tool_hooks` / `with_post_tool_hooks` run for every tool | nothing new: a `pre_tool` hook matching `computer` can block an action (exit `HOOK_BLOCK_EXIT`) |
| Approval | `approval(tool, summary, detail, reason)` builds an `ApprovalRequest { tool, summary, detail, reason, always }` for the session's `Ui`; the conversation's `approve` (`conversation.rs`) returns `Allow` when **Auto** is on | `computer_call` calls `approval` for every action under `owner.computer_use = ask` with summary `click left at 412,233 in Safari`, detail the last screenshot's artifact path, reason `desktop control (ADR-048)`. The conversation's `approve` gets one guard: a `computer` request is never auto-allowed, and the `always` pattern it offers is `computer(<action>:<bundle_id>)`, never `computer(*)`. |
| Steps the hub shows | `StepKind` (`Command`, `Fetch`, `Mcp`, ...) emitted through `self.emit(Step {...})` | `StepKind::Computer`, with `command` = the one-line summary and the screenshot's artifact name, so the conversation shows each action and its image |
| Audit | `pulse_bus::emit(store, actor, action, target, json)` (as `harness.start` in `commands.rs`); events are append-only by trigger (`crates/pulse-store/src/ddl.rs`) | one `computer.action` event per action and per refusal. It is **not** added to `PRUNABLE_EVENT_ACTIONS`, so `daemon.transcript_keep_days` never removes it. Typed text goes through `crates/pulse-bus/src/redact.rs` and is cut to 200 characters. |
| Kill switch | the `stop` setting (`pulse stop`; read by the runner in `crates/pulse-runner/src/run.rs`); the harness `CancelToken` (`watch_harness_cancel` in `commands.rs`) | `computer_call` reads `stop` and `owner.computer_use` from the store before **every** action and refuses on `stop = 1` or `off`; it checks `cancel` before posting events; and it refuses when the owner's HID idle time is shorter than the time since the last action ("the owner is using the machine") |
| Settings | `crates/pulse-store/src/settings.rs` defaults and rules; `owner.` names refused for agents in `crates/pulse-inspect/src/settings.rs` | `owner.computer_use` (choice `off`/`ask`/`on`, default `off`) and `owner.computer_use.allow` (text, `project=bundle|bundle` entries; `*` and the never-allowed apps refused at validation). Each needs a label in `web/hub/src/lib/setting-labels.ts` in the same commit (`setting_labels_cover_catalogue`). |

The backend sits behind a trait so the harness logic is tested without a
screen:

```rust
pub trait Desktop: Send {
    /// GUI session present and both grants given; Err names what is missing.
    fn ready(&self) -> Result<(), String>;
    fn frontmost(&self) -> Option<String>; // bundle id
    fn idle_seconds(&self) -> f64;          // HID idle time
    fn screenshot(&mut self, allow: &[String]) -> anyhow::Result<Shot>;
    fn click(&mut self, x: f64, y: f64, button: Button) -> anyhow::Result<()>;
    fn type_text(&mut self, text: &str) -> anyhow::Result<()>;
    fn key(&mut self, chord: &str) -> anyhow::Result<()>;
}
pub struct Shot { pub png: Vec<u8>, pub width: u32, pub height: u32, pub scale: f64 }
```

`FakeDesktop` (a recorded list of calls and a fixed PNG) drives the unit
tests. `MacDesktop` is the only real backend in the first delivery
(ADR-048 Scope), and it uses no new crate:

- `screenshot`: `/usr/sbin/screencapture -x -l <window>` per allowlisted
  window or `-R` for the display, then `/usr/bin/sips -Z 1280` to scale.
  The window list comes from `CGWindowListCopyWindowInfo` through the
  `CoreGraphics` framework linked with `#[link(name = "CoreGraphics", kind = "framework")]`.
- input: `CGEventCreateMouseEvent` / `CGEventCreateKeyboardEvent` /
  `CGEventKeyboardSetUnicodeString` / `CGEventPost` from the same framework.
- grants: `CGPreflightScreenCaptureAccess` and `AXIsProcessTrusted`
  (`ApplicationServices`), which check and never prompt. The prompt comes
  only from the owner's own click in the console's setting card, which
  opens System Settings > Privacy & Security.
- frontmost app: `NSWorkspace.frontmostApplication` is AppKit; the
  equivalent without AppKit is the owning PID of the topmost on-screen
  window from `CGWindowListCopyWindowInfo`, mapped to a bundle id.
- idle time: `CGEventSourceSecondsSinceLastEventType` with the HID state.

A crate such as `core-graphics` would be shorter but is a dependency
(Tier 2), so it is left to its own decision.

## 3. How a screenshot reaches the model (`Message.images`)

Tool results carry text only today. `tool_message(call, &text)` builds a
`Role::Tool` message from `ToolOutcome.text`, and the providers map a tool
message to text alone:

- anthropic: `Role::Tool` becomes `Block::ToolResult { content: String }`
  (`crates/pulse-provider-anthropic/src/lib.rs`).
- gemini: `functionResponse.response.output` (`crates/pulse-provider-gemini/src/lib.rs`).
- openai and copilot: the `tool` role's `content` is a string.

The path that already carries images to every provider is a **user**
message's `images` (`pulse_provider::Message.images: Vec<ImageContent>`,
with `ImageContent { media_type, data }` base64). openai and copilot map it
to `image_url` data-URL parts, anthropic to `Block::Image` before the text,
and gemini to `inlineData` parts. The hub's attached images already use it
(`Harness::turn_with_images` -> `push_user_with_images`).

So a `screenshot` result reaches the model in two messages, pushed in this
order after the tool call:

1. `Role::Tool`, `tool_call_id` = the call: the result text from the table
   in section 1 (`screenshot 1280x800 points, ...; image attached below`).
2. `Role::User` built by `push_user_with_images`: content
   `[computer screenshot 3 for call <id>]` and `images: vec![ImageContent { media_type: "image/png", data }]`.

Every provider accepts a user message right after the tool results. The
anthropic mapper already merges consecutive same-role messages, so the
`tool_result` block and the image land in one user turn as Anthropic
expects. No provider needs a change.

The change in the harness is small: `ToolOutcome` gets a field
`images: Vec<ImageContent>` (empty for every existing tool). In
`handle_tool_calls`, after `self.msgs.push(tool_message(call, &outcome.text))`
and once all the tool messages of the reply are in place, the harness
pushes one user message holding every image the calls returned. The images
must come after **all** the tool results of the reply: openai, copilot and
anthropic reject a request where a tool call's result does not directly
follow the assistant message.

A model-native "computer use" tool type (Anthropic's `computer_20250124`,
OpenAI's `computer_use_preview`) is deliberately not used. Each vendor's is
a different shape and model list, and the plain function tool above works
on every provider the harness speaks.

## 4. Size, history and compaction

- **Per image:** at most 1280 px on the long side, PNG. A typical desktop
  screenshot is 200-600 KB base64 at that size. The backend refuses
  anything over `IMAGE_LIMIT` (5 MB, `conversation.rs`).
- **Request budget:** `estimate_request_bytes` (`crates/pulse-harness/src/compact.rs`)
  already counts `images`, so `provider.max_request_mb` (default 1 MiB)
  already compacts a conversation that grows past it. One screenshot is a
  large part of that budget. The harness therefore keeps **only the
  newest screenshot** in the history: when it pushes a new one, it replaces
  the `images` of the earlier screenshot messages with nothing and changes
  their text to `[computer screenshot 2 - replaced by a newer one; the file is <artifact>]`.
  `prune_history` collapses old observations the same way.
- **Compaction summaries:** `render_transcript` (compact.rs) renders text
  only, so a screenshot never reaches the summarizer model. The `[computer
  screenshot N ...]` line it does see is enough for the summary to say a
  screenshot was taken.
- **Persistence:** `Message` serializes `images` (serde, skipped only when
  empty), so a trajectory or a resumed session would keep every
  screenshot's base64. The harness writes each screenshot to the
  conversation's artifacts folder (`PULSE_ARTIFACTS`, the same folder the
  `playwright` connection uses; `crates/pulse-mcp/src/browser.rs`) and
  stores the message with the file name and SHA-256 in its text and **no**
  bytes. `resume_messages` then gives the model the text placeholder, and
  the model takes a fresh screenshot, which is what it should see after a
  restart anyway.

## 5. Models

The `Model` record (`crates/pulse-provider/src/lib.rs`) has `chat` and
`tool_calls` but no vision flag, and Copilot's `ModelSupports` reads only
`tool_calls`. A model that cannot see images would click blind, and some
gateways reject an image part outright. The first delivery adds
`vision: bool` to `Model`:

- Copilot: `capabilities.supports.vision` from `/models`.
- Anthropic and Gemini: true for their current chat models.
- OpenAI-compatible vendors: false unless the vendor's model list says
  otherwise, or the owner names the model in a new `harness.vision_models`
  setting.

`offers_computer()` requires it. This is the same gate the hub's image
attachment should use; it currently sends images to any model.

## 6. What the owner sees

- **Settings (the owner settings beside `owner.presence`):** `Computer use` (off / ask / on) with the two grant
  states ("Screen Recording: not granted - Open System Settings") and the
  per-project app list, with a picker of running apps by name that writes
  bundle ids.
- **Conversation:** each action is a step (`StepKind::Computer`) with its
  screenshot thumbnail from the artifacts folder. Under `ask`, each action
  is an approval card showing the latest screenshot with the click point
  marked.
- **Activity:** `computer.action` events with project, agent, action, app
  and outcome.
- **macOS:** the system's own screen-recording indicator while `pulse`
  captures. PULSE does not hide it.

## 7. Tests the implementation will need

All against `FakeDesktop`, no screen and no TCC:

- the tool is not offered when `owner.computer_use = off` (the default), in
  a job (no `Ui`), in a subagent, read-only, under lockdown, for a project
  missing from the allowlist, or for a model without vision;
- an agent's `pulse settings set owner.computer_use on` is refused (the
  existing `owner.` guard);
- `click` and `type` are refused when the frontmost app is not allowlisted
  and for a never-allowed app (Terminal, System Settings, Keychain Access,
  1Password), and `*` in the allowlist is refused at validation;
- `stop = 1` set between two actions refuses the second; a cancel ends a
  pending action; an owner's input since the last action refuses the next;
- `ask` produces an `ApprovalRequest` for every action even with the
  conversation on **Auto**, and **Always** never yields `computer(*)`;
- a screenshot result becomes a tool message plus one user message with
  `images` after **all** the reply's tool results, and only the newest
  screenshot keeps its bytes in the history;
- each action writes one `computer.action` event, typed secrets come out
  redacted, and the retention sweep leaves the events in place;
- `estimate_request_bytes` stays under `provider.max_request_mb` across a
  run of ten screenshots.

## 8. Order of work after ADR-048 is accepted

1. Settings `owner.computer_use` and `owner.computer_use.allow` (default
   off) with hub labels; `Desktop` trait, `FakeDesktop`, `computer_tool()`,
   `offers_computer()`, dispatch, `subjects_of`, approval guard, audit
   event, kill-switch checks; `ToolOutcome.images` and the user-message
   push. All tests above. Nothing reachable with the default setting.
2. `MacDesktop` (screencapture/sips, CoreGraphics events, window list,
   grants, idle time) and `Model.vision`.
3. Console: the setting card with grant states and app picker, the
   `Computer` step with thumbnails, and approval cards with the marked
   screenshot.
