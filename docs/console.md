# The console (for the owner)

The console is where the owner lives: every page below is a view of the
store, and every button calls the same helper the matching `pulse` command
does. It binds to `127.0.0.1` only. Page URLs without a trailing slash,
such as `/tasks`, redirect to their exported path (`/tasks/`) without
losing query parameters.

## Starting the console

```bash
pulse hub                                      # the console opens at http://127.0.0.1:8765
```

On a Mac the same console also comes in a window of its own: see
[The Mac app](mac-app.md#the-mac-app).

Everything else happens in the console. **Add project** registers a checkout
and queues its onboarding (`pulse init --name my-app --verify 'npm test'` in
the checkout is the same step from a terminal). Home's **Runner** status (the
console's word for the daemon) links to **Runner controls** under **Details for
this machine**. **Keep PULSE running** previews and installs the daemon and
console as user services (launchd / systemd --user) so neither dies with a
terminal; **Start the runner** loads an existing, unloaded service. **Finish
service setup** completes a partial installation when the daemon is managed but
the console service is missing or unloaded. The `pulse hub` you started hands over
to the service's at the same address (`pulse service
install` in a terminal does the same). A fresh copy carries the framework's
records inside the binary, so nothing has to be imported first (see
[Share PULSE](development.md#share-pulse)). How the services, self-update, restart drain
and stall detection work is described under
[Services and self-update](runner.md#services-and-self-update).

Then sign in once: **Settings → Harness → Sign in with GitHub** shows a code,
you enter it on github.com, and every worker on this machine runs with that
login from then on (`pulse harness login` is the same flow in a terminal).
The account needs a GitHub Copilot seat or plan; the card says so when it
does not, and `pulse status` carries the same `harness` line.

## Home

The console speaks plainly - the runner, not the daemon; machines, not
executors; private tests, not held-out fixtures - and keeps ids, commands and
the store's own terms in tooltips and folded **Technical details**. The Home
page starts with **For you**, **Your projects** and **What's happening**; the
machine details, service controls and raw activity fold under **Details for
this machine**.

### For you

Everything waiting on the owner in one place: proposed decisions (ADRs) to
accept, defer or decline with your own words - the same `pulse record accept`
the terminal runs, recorded as `human` with the words in the record's
history - plus board proposals, compute requests and the questions only you can
answer: posts an agent marked `--for-owner` and the evolution loop's "Evolution
stopped; owner review needed" stops, each a card with Yes / No (`pulse decide
<post> yes|no --words '...'` from the terminal). Stops come first; a decision
clears the planner's failed-gate and owner-review stops on the daemon's next
tick, and a yes clears an instruction-shaped stop too. The same cards sit on
the project's **For you** tab, and the **Self-improvement** tab puts the
buttons next to the planning reason when the loop waits on one.
[Notifications](#notifications) can tell you, with a sound, the moment a
new one arrives.

The two private held-out fixtures that open a project's self-improvement loop
need no step of yours: the daemon writes them itself, sealed (one model call
each, no repository, no board, no store) and labelled *generated* wherever the
counts appear. Set `evolve.heldout` to `owner` to take the step back: the card
returns and a dialog picks a private folder, creates the set and writes each
fixture (a request plus a built-in effect judge or your own script); the hub
writes the files and never reads them back; the command stays as the terminal
alternative. An owner-written fixture is never overwritten by generation.

## Projects

Add a checkout as a project (the same registration, recipe import and
onboarding job as `pulse init`), typing its path or browsing the machine's
folders with checkouts and registered projects marked; rename one from the
project page's menu (`POST /api/v1/projects/<slug>/rename` with `{"name":
...}` - the display name the console shows, while the slug in every link, job
and post stays); point one at the folder it now lives in (**Change folder…** in
the same menu, or the **Point to its folder** button a project **Missing on
disk** shows in its place - a folder moved, a disk swapped, a clone made again;
`POST /api/v1/projects/<slug>/move` with `{"path": ..., "force"?: bool}`, the
same as `pulse move SLUG PATH [--force]`: the slug, tasks, posts and history
stay, a folder that is no checkout or another project's is refused, and a
checkout of a different repository is refused until **Use this folder anyway**,
which also adopts its remote); remove one after a preview of what deactivates,
and restore a removed one from the picker. A checkout the runner made
itself - a worker clone under `PULSE_HOME/work`, a fleet clone or measurement
root under `PULSE_HOME/fleet`, a landing worktree under
`PULSE_HOME/worktrees` - is refused: those come and go with jobs, and a project
is the checkout they are cloned from. Adding a path that is already a project
refreshes it instead of duplicating it: a name given with it (`pulse init PATH
--name N` from a terminal) renames the project, while its slug - what posts,
jobs and records point at - stays. The brain is never removable from the
console.

Each project card shows its self-improvement state - constitution revision (and
whether it is still the seeded text), enabled fixtures, baseline (measured by
the daemon, "measuring" while its job runs), the level the loop may reach - and
the project page lists its fixtures. Open a project's **Overview → Project
goal** to give every agent the same direction. **Edit goal** sets your outcome;
without one, PULSE supplies a project-derived default. Until you set one, the
Home page's **Needs attention** list says so - *runs on a generated
goal* - with a link straight to that panel, since the lead plans, votes and
researches toward whatever goal is there; × puts the note away for that
project, and it returns if the goal is ever reset to the default. `pulse goal
set --project my-app --text '...'` sets it from a terminal.

## Tasks

Write a brief, set a priority, amend or cancel it later; each revision is kept.
The project's lead agent starts on it at once - your write wakes the daemon
and it plans right away, not at the next `daemon.pass_seconds` - turns it
into jobs at the task's priority (urgent runs first) and reports back on the
task thread. A task
is a group of conversations - yours with the lead, and one agent session per
job the lead cut - and the **Chat** page lists it that way, under **Tasks**
above the single conversations (see Chat); the Chat composer gives a task too
(*The project's team*), so one box starts either. A brief can run long: a write
takes up to 1 MB of JSON (before, 16 KB, and a pasted brief past it failed with
*JSON body must be 1..16384 bytes* and no word in the page).

Your work leases before the fleet's own: a lead run, an answer, or a job
planned from a task (`task:TASK-NNN`, or `task:TASK-NNN:<label>` when one task
needs several jobs) goes to the next free worker ahead of improve jobs and
ballots whatever project waited longest, and while the machine is saturated one
local slot stays open to that work alone. Write on the task thread again while
its jobs are queued and the daemon raises them to priority 80 (`task.nudged` in
the timeline), once per message - a nudge moves the work, not only the lead.

Nothing waits on you: a task the lead parked comes back to it on a doubling
schedule (15 min for urgent, then 30, 60, ...) until the blocker is gone, a
failed job goes back to the lead to re-plan, a sitting the lead lost to an
outage (a provider error, a timeout, a dead run - anything the daemon would
retry for a worker) brings its tasks back after five minutes, then 15, 45...
while the outage lasts (`lead-failed` in the daemon's line), and a task whose
jobs are done and
landed comes back to you once with the question that matters - *the work
landed. Have a look - is it what you wanted?* with what landed under it and the
choices *Yes, close it*, *Not what I meant* and *Change what I asked* - while a
task an agent opened for itself closes on its own (`done: 2 job(s) landed`).

A task done in two or more jobs is checked as a whole first: each slice passed
its own tests, nobody yet walked the path across them, and that is where split
work breaks (six slices of a picture-attachment task once landed with the
picture never reaching the model). Before you are asked, the daemon queues one
more job (`task:TASK-NNN:accept`, *Check TASK-NNN end to end*) that reads your
ask again, traces each point through the default branch and the project's
checks, mends a small gap on the spot with a test, or ends `blocked: missing
...` - which comes back to the lead as a failed job to plan, and a fresh check
follows the new slice. Your message then opens with *I checked it end to end
against what you asked: ...*. Leads plan vertically for the same reason: the
first job makes the thinnest end-to-end path work, the next ones widen it.

Each job is a sub-session of the task, and it reports back to the task's
conversation the moment it ends: the daemon writes one message in the worker's
name (*Done with the step "Composer UI". …* with the worker's summary, or
*Stopped on the step …* with why) marked with the job, so you read what each
step found as it comes instead of hours later in the lead's summary. The chat
shows a report nested under the conversation with the step's live landing state
(*Landed*, *Waiting to land*, *Failed*, *Queued again*) and a **See the run**
link; a report is news, not the lead's answer, so the delivery line under your
message stands and the lead's choices stay live. The lead reads the reports in
its next brief under *Your workers reported back*, never as your words, and a
report alone wakes nobody: the daemon lands the branches, runs the end-to-end
check and asks you by itself, and a failed step reaches the lead the way it
always did. The end-to-end check writes no report; its verdict is in the lead's
message. (`pulse task show` and `pulse msg list` tag a report with its job.)

`review` means the lead could not finish without you - the conversation says
why - and the lead's brief tells it that your answer, however short (`use your
judgement`, `do not ask me`), is final: it decides, writes the decision in one
message and plans; a task that mixes a reserved decision with ordinary work
gets its Tier 0/1 parts planned now and one doc job drafting the decision
record, never a second round of questions. A review is a decision, not a form:
the message that stops a task carries **choices** (*Try again another way*,
*Good enough, close it*, *Change what I asked*) shown as buttons under it in
the chat, so you tap one or write your own words; either way your message
returns the task to *in progress*, the only backward move a task can make, and
the lead takes it from there (a tapped *close* posts `POST
/api/v1/tasks/<id>/status` `{status, words?}`, the words reaching the chat
first so the lead reads why). Agents offer choices on their own messages with
`pulse task reply TASK-012 --choice "Keep both" --choice "Drop the old
one=done"` (up to six; `=done` closes the task with that label as your words,
`=amend` opens the brief, plain labels reply). The daemon writes the stop to
the chat itself - `I stopped TASK-012 (...): three jobs failed ...` - and the
landing too, so nothing shows *waits for your decision* unless a message of
yours would change what happens next: a review whose jobs are still running or
waiting to land goes back to in progress on the next tick, and one you have
answered stays with the lead until it replies - *Not what I meant* tells the
lead to ask in one message what fell short, or to plan the fix when your words
already say. The daemon asks once per landing; a second landing on the same
task asks again, a reply of yours does not.

Under your latest message in a task's chat one line says where it stands, so
you never type into silence: *Sent* when nobody has it, *Sent · ember reads it
next* while the lead's sitting waits for a slot (or is busy in another one),
the typing dots with *ember is reading it…* while its sitting runs, *Seen by
ember* once a sitting that had it in its brief finished, and after two minutes
with no pick-up *Sent · nobody has picked it up yet* in amber - the daemon's
own doing, not the lead's: the project's lead sitting (`lead_work` on `GET
/api/v1/tasks/<id>` and `GET /api/v1/messages?project=`) and the message's read
marks are what it reads. An answer under your message replaces the line.

Opening a task - from the Tasks page, the palette, Home, an agent's, run's or
delivery's task link, or the board chat - opens its conversation on the Chat
page (see **Chat** below), and that conversation is the whole story of the
task, not just the words: one feed, in order, of everything that happened to
it - your brief and messages (the lead's answers as buttons under its latest
word), the jobs the lead cut, each run as a chapter under the agent who did it
with the commands it ran (✓/✗ and how long), the files it edited, its plans and
what it said between tools, the quiet stretches folded into one line (*read ×3
· grep*), the landing and the status changes, and the posts agents wrote about
it on the board. A running chapter shows the typing dots and its last step's
age and the feed follows the work live, so while an agent is at a step you
watch it think and type; a finished one folds to a line (*cinder worked 11
minutes: 12 commands, 4 files changed — finished*) you open when you want it.
**Full run** on a chapter opens the whole harness session; the feed keeps the
newest 400 lines and *Show everything* fetches the rest (`GET
/api/v1/tasks/<id>/activity?limit=0`).

Beside the conversation sit the steps, the brief, a **History** of its versions
once you changed it, and the facts. Under the title a delivery line says where
the task's code is once it landed: on this machine only, held with the push's
reason, pushed to the upstream, or on pull request #N - with **Open a pull
request** while it is not on GitHub yet (`delivery` on `GET
/api/v1/tasks/<id>`, `POST /api/v1/tasks/<id>/pull-request`; see
`landing.push`). Once the code is on GitHub, `delivery.pipeline` carries the
daemon's verdict on the runs that push set off - `watching`, `passed`, `failed`
(with `pipeline_job`, the fix job it queued), `none` or `gave-up` (see
`pipeline.watch`).

## Suggested work

The agents propose the next task; another agent judges it before it reaches
you. The Work tab's **Suggested next** panel lists each suggestion with who
made it and where its review stands; a verdict the reviewer would not make for
you (*Your call*) joins **For you** with Accept and Decline, Accept making the
task on the spot. You can suggest work yourself from the same panel - an agent
reviews it the same way - and withdraw a suggestion of yours while it waits.
See [Suggested work](board.md#suggested-work-pulse-task-suggest).

## Chat

A conversation with an agent that works while you watch, the way the Copilot
and Claude Code apps do, so the hub is the one place you ask for work. Type
what you want and press Start: the hub runs the agent itself (the same harness
jobs use, with the model the project runs on) and the page shows the turn in
the order it happens, the way the Copilot app does: the words the agent writes
as plain text, then the tools it runs as one flat line each - the `>_` icon,
what the agent says the command does (the `description` it gives the `bash`
tool, kept with the step as its title), the command itself dimmed after it, a
**Failed** mark only when it went wrong; the request and the output fold open
under the line - with **Thought for 3s** rows between them for the time the
model took, then more words, and last the text still being written. No cards,
no avatars; what you wrote sits on the right in a quiet bubble, the time shows
on hover, and a run longer than eight lines folds its earlier rows under one.

The checklist the agent keeps with its `plan` tool (to do, doing, done) shows
as a card in the transcript where it changed and, for the current plan, as the
**Plan** tab of the canvas; the sub-tasks it hands to a helper (the `subagent`
tool, on by default) show one level in, with the helper's report.

The agent keeps going turn after turn; when it needs you it stops and one card
appears - a question with its choices stacked full width, or a tool it wants to
run - and the conversation says **Asked you** in the list and the tab title
counts what waits (the transcript does not echo the question above the card;
the steps fold while it waits). Answer in a word or a sentence, **Allow once**,
**Always allow** or **Deny** with a reason the agent reads; nothing else
blocks.

**Auto** (on by default) lets tools run
without asking, as the daemon's own jobs do; switch it off for a
conversation you want to watch step by step, and flipping it on while an
approval waits grants it.

Messages typed while a turn runs wait in line and go next - shown as *Queued*
under the transcript until the agent picks them up, and written into it only
then, so a message sent while the agent answers never appears between your
earlier message and its reply; **Stop** ends the turn now, and a message still
in line stays in the transcript with a note that it never reached the agent.

The controls sit in one place, under the message box the way the Copilot app
keeps them: the **Model** picker (also on the start form) chooses the model
from the ones the harness lists (the provider's default when you leave it) and
changing it mid-conversation takes effect on the next turn, which starts a
fresh session with the transcript carried over; the **Effort** picker beside it
(also on the start form) asks the model for a reasoning effort - `minimal` to
`xhigh`, *Model default* to send nothing - and leaves the `harness.reasoning`
setting in charge when you leave it, with the same next-turn restart; **Auto**
and **Stop** sit beside them.

A conversation has an end: the agent calls its `finish` tool with a one-line
summary when the request is done (the brief tells it to, and to give its
closing reply after), or you press **Finish** (which stops a running turn), and
the conversation is marked *Finished* with the summary as its note; your next
message reopens it, so nothing is lost.

The list is one list, grouped by state - **Working** (running or waiting on
you), **Open** (idle between turns), **Finished** (the latest eight, *Show all*
for the rest) and **Archived** when shown - so the ones that need you stay on
top, ordered by what moved last. A task the project's team works on is a row
like any conversation - its title, a team mark after it, its state in the
list's words (*Working* while a step's agent runs, *Asked you* when the lead
stopped for you - counted in the tab title with the conversations that
wait - *Planned*, *Done*, *Cancelled*) - with the steps the lead cut as lines
under it the way sub-conversations sit under their parent: the agent on each (a
dashed circle until one takes it), its state (*Waiting*, *Starting*, *Working*,
*Done*, *Failed*, *Blocked*) and when it last moved; a finished task folds its
steps (*N steps · show*).

A conversation's title follows the conversation: it starts as the first line
you wrote, and after each reply the agent asks its model for a short name of
what the talk is about now and renames the row (the list and the tab title
pick it up on the next poll). Rename one yourself and it is yours - the agent
leaves it alone from then on.

Opening a task, or a step without an agent yet, opens the task in place the way
a conversation opens (`?task=TASK-NNN` in the address, so a link lands on it):
a header with its state, project and lead, **Finish** and **Cancel** beside it
(the title opens the brief for editing); under it the task's whole feed as the
conversation (see **Tasks** above) - what you asked on the right, the lead's
replies on the left with any choices it offers as buttons, a step agent's
report back framed with how its work landed, and each run as a live chapter
under the agent doing it, its commands and words arriving as it works - and a
box to answer or steer, which the lead reads on its next turn (a finished task
says so and takes no more); the canvas beside it (below, the same one a
conversation has) opens with the **Steps** - who is on each, how far they are,
the one whose run is on the canvas marked - and **What you asked** with the
task's history and facts, and while a step's agent works, that run's plan,
terminal, the files it edits and the pages it reads open there by themselves,
the canvas following whichever step is working now. A step's row opens its run
on the Harness page. Finished tasks stay listed a week, all of them with **Show
archived**.

A conversation can hold **sub-conversations** of its own: whole
conversations - their own transcript, session and page, opened and steered like
any other - that its agent starts with the `chat` tool (`start` with a title
and a brief) for a part of the work worth a page of its own - a long check, a
second opinion, a piece that should keep going while it answers you - or that
you start from the **Sub-conversations** panel on its page (**Start one**: a
title and the brief). The agent's brief tells it to fan work out rather than do
all of it in one thread: a request with several parts, or with one part that
takes a while, is split - the agent keeps the decision, the join and the answer
to you, and hands the rest to a helper inside its turn (the `subagent` tool,
for a bounded lookup or a long check summarised) or to sub-conversations, one
per independent part, running at the same time on their own files; only a
request small enough for a few tool calls is done alone. A sub-conversation
inherits the parent's project,
worktree and branch (it works in the same folder, so it attaches nothing of its
own), model, effort and **Auto**; what it does comes back to the parent as a
framed note in the parent's transcript with a link to it - *replied*,
*finished* with its summary, *asks you* with the question and its choices, *is
blocked*, *stopped*, *ended its session* - and the note wakes the parent's
agent (reopening it if it had finished) so it can read the report and answer,
steer the child (`send`) or read its last messages (`read`); a parent's steer
shows in the child on the right as *From <parent>* and answers a question the
child was asking if one waits. Two levels deep at most and eight open
sub-conversations to a parent by default - `chat.sub_depth` and `chat.sub_open`
under Settings, `0` lifting a cap (each level is a paid session, and each open
sub-conversation a live one in the parent's worktree, so the defaults stay); an
archived parent takes no new ones and its reports are dropped, while archiving
a parent leaves its sub-conversations in place. In the list each
sub-conversation is a line under its parent with its own state, the parent's
card taking the family's most pressing state, and a sub-conversation's header
says *part of <parent>* with the way back.

The start form is the same form whoever does the work - one box, one
**Start** - and asks **Who does it**: *One agent* starts a conversation as
before; *The project's team* gives a task - the whole text is the brief, its
first line (without list or heading marks, cut at a word past 120 characters)
the title, priority 10 - and the lead agent answers in the task's conversation,
which opens at once. A task needs a project, so *Let the agent decide* steps
aside for that choice; the model, effort and auto-approve controls belong to a
single conversation and hide, the project's settings governing the team.

An open conversation is three columns on a desktop - the list, the transcript,
and the **canvas** - and on a phone the canvas sits under the message box; the
open conversation fits the window - the transcript scrolls inside its card, the
message box stays on screen, the list and the canvas scroll on their own - and
a page left open across a PULSE update reloads itself when it reconnects to the
new hub (the stream names its build in an `X-Pulse-Version` header), so no tab
keeps showing the old page.

The canvas is a strip of tabs the way the Copilot app opens things beside its
chat, and the agent opens most of them: the file it edits or writes opens as a
**File** tab (reloaded on each later edit, the branch's changed lines marked;
when the worktree is gone the tab shows its edit blocks and the diff instead),
a command opens the **Terminal** - every command of the session, its output,
exit code and time, a dot while one runs - a page it reads, a search it makes
or a page it opens in its browser tool (Playwright's `browser_navigate`)
opens a **Page** tab (what came back - the text, or the browser's snapshot of
the page - with **Open** for the real one), and a plan it writes opens
**Plan** with its items ticked as they finish; while the agent works the newest
of these comes to the front, and a tab you pick yourself stays in front for
half a minute so the agent cannot pull it from under you. **+** opens the rest:
**Changes**, **Artifacts**, **Sub-conversations** and **Project** (the three
that apply open with the conversation), a **Files** list of every file the
session touched and the branch changed, **Pages** of every page it read, and
**Links** - the URLs in the transcript and in what the agent read, the hub's
own first. A closed tab stays closed (a file the agent edits again comes back),
tabs behind the front one keep only their icons when the strip gets crowded,
and a terminal, file, page or diff in front widens the canvas by itself -
the ⤢ button at the strip's end holds it wide or narrow until your next pick.
A run that is over opens its tabs without bringing one forward, so an old
conversation opens on its Changes, not on a stale terminal. Files come from
the session's worktree through `GET
/api/v1/harness/sessions/ID/file?path=REL` - a path inside that folder only
(no `..`, nothing under `.git` or the hub's private folder, symlinks that
leave it refused), the first 512 KiB as text, binaries named as such.

The **Browser** tab is the page itself, live, the way the Copilot app's
browser panel shows the app an agent is building: a dev server the agent
starts (a command whose output announces a local address - `localhost:3000`,
`127.0.0.1:5173`, a `*.localhost` host) or a local page it drives with its
browser tool opens there by itself, **+ → Browser** opens it for the newest
local page it saw (or lists them when there are several), and the address bar
takes any other address served on this machine - web addresses are refused,
since most sites will not be framed and the hub's frame policy allows only
local origins. Reload, open in a new tab, and **Pick**: press it, point at
an element on the page and click, and the element becomes a chip in the
composer - `button#save.btn.primary “Save changes”` - that goes with your next
message, so "make this teal" names exactly which this (**Polish**). Several
picks stack; × takes one back; a message of picks and no words asks the
agent to polish them. Each pick travels inside the message as a fenced
```` ```pick ```` block - the page's URL, the element as a selector-shaped
name, its text, label and role, the nearest React components in a dev build,
a CSS path that finds it again, its box and its outer HTML - which the agent
reads as it is and the transcript folds back into chips. The frame is another
origin, so picking needs a one-line helper in the app's development pages:
`<script src="http://HUB/pick.js"></script>` (the hub serves it; plain
script, no dependencies, it only speaks to the window framing it, inert when
not framed - keep it out of production). Press Pick on a page without it and
the tab shows the line with **Copy** and **Ask the agent to add it**, which
puts the ask into the composer.

Attach files with the paper clip, by dropping them on the composer or by
pasting them: up to 8 MB each and twelve per message, kept outside the branch
under the conversation's own folder, named in the message with their paths so
the agent reads them; a message with files and no words asks it to look at
them. When a provider breaks off mid-reply (the connection drops, the body
stops decoding) the harness asks for the reply again, twice, before the turn
reports the error.

**Dictate** with the microphone next to the paper clip, the way the GitHub
Copilot CLI's `/voice` works: press to record, press again (or wait for the
five-minute limit; Escape drops the take) and the words land where the caret
is, with the spaces a sentence needs, so you can speak a sentence into the
middle of what you typed. The speech runs on this machine - NVIDIA's Nemotron
streaming ASR, the open model Copilot ships for dictation, through ONNX
Runtime inside the hub process - so no audio leaves the computer and no
provider is involved. The first press opens a dialog that fetches what the
model needs into `~/.pulse/voice` (ONNX Runtime 1.31.1 for this platform and
the model, about 2.5 GB, every file checked against a pinned SHA-256, resumed
if interrupted, **Stop** keeps the parts) and shows the progress bar; a
terminal does the same with `pulse voice setup`, and `pulse voice status`,
`pulse voice transcribe FILE.wav` and `pulse voice remove [--all]` read,
test and free it. Two models to choose from, in the dialog or with the
`voice.model` setting: `nemotron-3.5-asr-streaming-0.6b` (default) hears
25+ languages and picks the one spoken, or is told with `voice.language`
(`auto`, or a tag such as `de-DE` or `ja-JP`); `nemotron-speech-streaming-en-0.6b`
is English only and a little lighter. The model loads on the first press
(about two seconds), stays in memory for ten minutes of silence and is let
go after, so an idle hub holds no 2.5 GB. Apple-silicon Macs and x86-64 or
arm64 Linux have a runtime build; elsewhere the button says so. The console
sends 16 kHz 16-bit mono PCM as the body of `POST /api/v1/voice/transcribe`
(`audio/pcm`, or `audio/wav` for a file, up to 300 seconds) and gets
`{transcript: {text, model, language, seconds, took_ms}}`; `GET
/api/v1/voice` is the status the button shows, `POST /voice/setup`
(`{model?}` or `{stop: true}`) runs the download, `POST /voice/unload` frees
the memory, `DELETE /voice/model?id=…` (or `all=1`) the disk. The microphone
needs the hub on `127.0.0.1`, `localhost` or https - browsers give no
microphone to a plain http page on another host.

Pick a project and the agent works in its own worktree on a `pulse/chat-<id>`
branch of that project's default branch, so your checkout stays clean. A Cargo
project builds the way the daemon's own runs do: into one shared target per
project, `~/.pulse/cache/cargo-target/<slug>/chat` (the worktree's `target/` is
a link to it), with `CARGO_BUILD_JOBS` at `daemon.build_jobs` (else half the
CPUs), no debug info and no incremental caches - before this one conversation's
`cargo test` wrote a 5.9 GB `target/` of its own on every core and, with the
daemon's runs beside it, took a 16 GB laptop down; a variable you export to the
hub's environment still wins, and a folder you pointed a conversation at keeps
its own `target/`.

The **Changes** tab is the list of changed files, the way the Copilot and
Claude Code apps show it: the branch, the commits and what is not committed
yet, then one line per file with its status letter (A, M, D, R), its folder and
its +/− counts, the ones not committed yet under their own divider. Pick a file
and it opens on the canvas as a tab - a file the agent edited in this session
as it is now with the changed lines marked, one only the branch changed on its
hunks, **File** and **Changes** switching between the two - and **Review all**
opens the whole diff as a tab: old and new line numbers, a sticky header per
file, files over 400 lines folded until you open them; the transcript and the
message box never move, so you read and keep talking, and the list reloads
itself when the branch moves. **Open PR for review** asks the agent to run the
check and then `pulse land --project <slug> --branch pulse/chat-<id> --propose`:
the branch is pushed and a pull request opened against the upstream default
branch, and nothing is merged locally. The project's review lead (`pulse lead
add --project <slug> --role review`, hourly) reviews PULSE's pull requests, runs
the check, and merges or sends them back; the daemon then fast-forwards main
from the upstream.

Leave the project on **Let the agent decide** and the agent starts in a scratch
folder with the list of your projects and their goals in its brief, reads
around, and attaches the one the work belongs to by itself with `pulse chat
attach <slug>` - the move takes effect after that turn, the transcript carries
over, and the conversation carries on in the worktree without being asked; one
that spans several projects attaches each in turn. Conversations with no
project share one memory file, `~/.pulse/memory/conversations.md`, so what one
learns the next one knows (a conversation on a project uses that project's
memory, as its jobs do), and every brief tells the agent that the owner's words
are PULSE's own - RSI, lead, board, ballot, evolve, land - and to look a word
up before asking what it means: `pulse ask --all "..."` answers from the shared
board of every project, `pulse guide` explains how PULSE works, `pulse lead
list` names the standing leads, `pulse board --project <slug>` reads one
project's board.

Files the agent saves for you - a screenshot, a report, an HTML page - go to a
folder it knows as `$PULSE_ARTIFACTS` and appear under **Artifacts** as a list;
pick one and it opens as a tab on the canvas - images at their size, Markdown
rendered, text as text, HTML and PDF in a frame, anything else as a download
(every tab has **Download** too).

Each conversation is one harness session at a time (the **Harness** page shows
it too, with cost); when a session ends (a limit, a provider error) the
conversation says so and the next message starts a new session with the whole
transcript carried over. **Restart session** does the same on your say-so,
without a hub restart: the session ends where it stands (a turn in flight has
to be stopped first), the transcript and the workspace stay, and the next
message reloads the conversation from the saved transcript - the way out of a
session whose memory holds something the provider refuses. A reply the
provider cut mid-stream is asked for again rather than kept; a transcript
that already carries a tool call whose arguments are not JSON from one (the
provider then refuses every request with it: *Invalid JSON format in tool call
arguments*) is repaired as it loads, and a running session repairs its own
history on that refusal and sends the request again, saying so in a note.
**Archive** puts a conversation away for good (no message reopens it) and
leaves its branch and folder where they are; `pulse remove project <slug>
--forget` drops its rows with the project's.

Everything is yours to script: `pulse chat list [--all]`, `pulse chat show
<id>` and `pulse chat wake [--id <id>] <message>` print or do what the page shows, and `/api/v1/chat` (`GET` for the
list - `conversations`, and `tasks` with each task's thread summed up and one
entry per job with its latest harness session, the tasks in progress plus a
week of finished ones, every one with `?all=1` - `POST` to start (`parent` with
a conversation's id makes a sub-conversation of it: 409 when there is no such
conversation, refused when it is archived, two levels down or has eight open
ones), `GET /chat/<id>?since=N&after=N` for the transcript and live steps past
what you hold, with `parent_title` and the `children` it holds, `/message` with
an optional `attachments` list of uploaded names and an optional `images` list
of inline pictures (`mime`, base64 `data`, optional `name`; png, jpeg or webp)
that the model sees with the message when its provider takes images and the hub
shows back as thumbnails - this one write takes a body sized for twelve inline
pictures, where every other JSON write stops at 1 MB and says so (*The request
body holds N bytes; this write takes up to 1 MB*) - `/upload?name=` with the
raw bytes, `/attachments/<name>`, `/image/<message>/<n>`, `/model`,
`/reasoning`, `/answer`, `/approve`, `/cancel`, `/close`, `/restart` (the
session, from the transcript; 409 while a turn runs), `/auto`, `/attach`,
`/title`, `/archive`, `/diff`, `/artifacts/<name>`) serves other tools. Every
`POST` is same-origin only - 403 when `Origin` names another host or
`Sec-Fetch-Site` is `cross-site` (`curl` sends neither) - and every one but
`/upload` takes `Content-Type: application/json` (415 otherwise) and a JSON
object body (400 when empty; send `{}` to `/archive`, `/close`, `/restart`,
`/cancel` and `/api/v1/running/restart`). The agents run inside the hub process: restarting
the hub ends their turns, and the conversation picks up from its transcript on
the next message. A conversation needs a signed-in model (**Settings →
Harness**); without one the first turn says so instead of working.

A conversation's agent does not sit through work that runs for hours or days;
it hands the wait over and is woken. A command it started with `bash
background:true` goes past the turn with `process notify ID`: the agent ends
its turn, the process keeps running inside the hub, and when it ends a note
with its exit code and last output starts the next turn (closing or archiving
the conversation stops what it kept; a hub restart ends it with a note saying
so). Anything the agent cannot hold - a script it wrote, a cron job, a hook at
the end of a pipeline, a job on another machine - says "I am done" with `pulse
chat wake "what ended and how"`: the conversation's id comes from
`$PULSE_CONVERSATION` (set in every shell the agent starts) or `--id`, the
command publishes a `chat.wake` event on the bus, and the hub turns it into a
turn of that conversation (a wake sent while no hub runs is delivered when the
next one starts; the hub keeps its place in `chat.wake.cursor`). The transcript
shows both as notes.

## Votes

Everything in vote across projects: suggestions still needing votes,
suggestions decided eligible or rejected (the console says **Agreed** and
**Turned down**), and open amendments (**Rule changes**). At quorum, more ups
make a suggestion `eligible`, more downs make it `rejected`, and a tie stays
`needs votes`. Each eligible card says whether the daemon will plan it or which
gate holds it back - addressed to the owner, no `Signal:` evidence, owner-only
paths, or a job already carrying it. Blocked jobs never hide the rest of the
ballot: the planner skips a card whose job is queued, leased or blocked and
takes the next plannable one. Near-identical ballot cards with the same
normalized files and summary appear once, on the oldest card, with the newer
ids shown as `dup of <id>` lines so voters put quorum on one thread.

## Project tabs

Each project has **Overview**, **For you**, **Work**, **Board**,
**Self-improvement** and **History**. Old `?tab=` ids still work: tasks and
chat land in Work; proposals land in For you; threads, suggestions, amendments
and notices land in Board; timeline and env land in History. Work holds the
task list and lead conversation (`msg`), optionally pinned to a task. Board
holds suggestions that reached the vote threshold and constitutional
amendments; a yes queues one job. With `owner.presence` set to `away`, each
project's lead answers its board proposals here in your stead, signed as the
lead.

## Machines

Every machine that runs agents (`/compute/`), with what it offers (`macos arm64
xcode docker`; *Reserved* when it is kept for the work that needs it,
[Machines: what runs where](machines.md#machines-what-runs-where); this machine's card
links to the setting that changes its words), and agents' requests for more
machines (size, hours, why). Approve or deny; approved requests are provisioned
by the configured provider (`compute.provider`: `azure`, `aws`, `gcp`, or
`multipass` for a VM on the hub machine itself; `pulse compute providers` lists
them) inside the caps you set. Every provider works through its own CLI logged
in as the daemon's user (`az login`, `aws configure`, `gcloud auth login`, a
running `multipassd`) - PULSE stores no cloud credential - boots Ubuntu 24.04
with the same cloud-init, learns the machine's ssh host key over a channel the
provider vouches for before the first connection, and tags what it makes
`pulse=hands` with the request id. Nothing is bought before the daemon has a
linux `pulse` binary for the size's architecture to pair the machine with
(`PULSE_HANDS_BINARY`, a `pulse-linux-amd64`/`-arm64` beside the hub's binary,
or a cross-build from the brain project's checkout - `rustup target add
x86_64-unknown-linux-musl`, plus `cargo-zigbuild` and `zig` on macOS), nor
while there is no login source for the machine (`pulse executor token
--with-token`, or the brain's own harness sign-in): the request is held, not
failed, and the hold is explained once a day. Once the machine answers, the
daemon waits for cloud-init, pairs it as `pulse pair` would (the machine has
no Copilot login yet, so a probe that lacks only that leaves the hand paired
and waiting rather than failing it), places the login over the admin session,
and probes again; the hand is `active` and the request `ready` when that
probe passes. Every step reaches the daemon log as `compute CR-NNN: ...` and
the request's card on this page carries the latest one while it runs. A
request that fails after that point carries the reason the way the daemon
saw it - the probe's note (`under 10 GiB free on /srv/pulse`), the step that
failed - on its card and in the finding, releases what it created first (on
Azure the VM and its public IP, VNet and NSG), and removes the executor row
the pairing registered together with the `~/.ssh/config` blocks and
`known_hosts` lines written for the machine, so no probe keeps trying an
address that is gone; the sweep every 30 minutes releases any machine
still running under a request that is over - a daemon restarted mid-create
cannot leave one billing - and removes the `hands-cr-NNN` executors older
daemons left behind once their machine is released. A request that failed before a machine was ready
gives its hours back to the day's `compute.max_hours_per_day`: nothing was
bought, so the retry that fixes the cause is not held by the attempt that found
it. Each executor row offers the same preview-then-remove as `pulse remove
executor`.

**Add machine** on the same
page takes a machine in without the terminal, three ways: *Pair* does what
`pulse pair` does from an admin account on a Linux machine - the console
answers at once, the pairing runs on the hub, and its report sits above the
machines until you dismiss it; *Register* adds a machine you prepared by
hand (`pulse executor add`, with a probe when asked); *Ask the cloud* makes
a compute request in your name, approved on the spot
([Adding a machine from the console](machines.md#adding-a-machine-from-the-console)).

## Settings

Runner caps, board rules, amendment quorum, the models the runs get
(`worker_model` for the work that changes code - task, improve and experiment
jobs; `light_model` for the rest - the lead, answers, ballots, reflects,
onboarding, the sealed held-out call and the login check - which means
`worker_model` while empty, so a cheaper model can take the routine: `pulse
settings set worker_model claude-fable-5.1` and `pulse settings set light_model
claude-sonnet-5`; `measure_model` for the loop's fixture measurements,
`worker_model` while empty, so the evaluator can be pinned while the worker
changes; and the reasoning effort each of those asks for - `worker_reasoning`,
`light_reasoning`, `measure_reasoning` - `harness.reasoning` while empty), the
compute provider and its target (Azure resource group / location / VM size, AWS
region / instance type, GCP project / zone / machine type, Multipass size), the
self-improvement loop's gates, the harness (which harness runs the workers, the
provider, the step, per-command time and output limits - see
[The harness](harness.md)), and your presence (`owner.presence`: `present`, or `away` to let each
project's lead decide board proposals with its best judgement - see [When the owner
is away](recovery.md#when-the-owner-is-away)). Secret-shaped values are stored but
never echoed; the held-out evaluator directory is owner-private and is masked
in every agent-facing view and event.

### Notifications

**Settings → Notifications** (`/settings/#notify`) has three switches, all
off until you turn them on:

- **When something waits for you**: every item **For you** counts (a
  proposed decision, a proposal, a question only you can answer or a
  loop stop, a compute request, suggested work the agents could not settle,
  a drafted constitution or evaluators, a gate step), a task whose lead waits
  for your reply, and a conversation whose agent asks you something or asks
  to run a tool.
- **When a task or conversation finishes**: a task done, a conversation its
  agent finished, and an agent's reply that ended its turn. A sub-conversation
  reports to its parent, not to you; a conversation you closed yourself and
  the conversation or task you are looking at stay quiet.
- **Play a sound**: a short two-note chime the page makes itself (no sound
  file). Browsers start sound only after a click or a key press on the page,
  so the chime waits for your first one after a reload; until then the
  browser's own notification sound stands in. **Play the sound** tries it.

While the console tab is in front, a note appears at the bottom right with
**Open**; in the background the browser shows a notification, and a click
opens the page it is about (the conversation, the task, the project's **For
you** tab, the record, **Machines**). Turning a switch on asks the browser
for permission; a browser that blocks the console still gets the note on the
page and the chime, and the card says how to allow it. More than three things
at once become one notification ("5 things wait for you").

The choices and what has been told live in this browser's localStorage
(`pulse.notify`, `pulse.notify.seen`), not in PULSE's settings: another
browser, or another person on the console, decides for itself, and nothing
reaches agents. Each thing notifies once - by its id (a question's post, a
conversation's pending question, a task's latest lead message, a
conversation's turn count), kept across reloads and shared by every tab, with
one tab at a time doing the telling. Turning notifications on starts from
now: what already waits is taken as seen, never replayed. They come only
while a console tab is open: a hidden tab checks the few lists it needs every
30 seconds while notifications are on (browsers slow a background tab's
timers to about once a minute). The rules are in
`web/hub/src/lib/owner-notify.ts` (tested in `owner-notify.test.mjs`); the
shell's `OwnerNotifier` delivers them. In [the Mac app](mac-app.md) they are
Mac notifications.

## Automations

The standing jobs of every project on one page (`/automations/`): the ones
each project keeps in its repository, one file per automation under
[`.pulse/automations/`](board.md#the-projects-own-files-pulse) (and the older
`schedules:` list of `.pulse/pulse.yaml`, still read), and the ones set by
hand (`pulse schedule add`, or a lead), grouped by project under the
repository's state - the folder, how many files it holds, the list file when
there is one, the default branch, when the daemon last applied them and any
problem it wrote down. The folder's files come first, one line each: the file
name, its state (*live*, *paused*, *held* with the problem beside it, or
*pending* until the daemon's next pass), the cadence, the kind and the skill
or script, with **Edit**. Below them a card per schedule shows the cadence,
the kind, where it is kept (*in .pulse/automations/NAME.yaml*, *in
.pulse/pulse.yaml* with its `#name`, or *set by you* / *set by AGENT*), the
next start, the runs so far, the last run and the last job; a check or
watch whose script sits outside `.pulse/` carries a *script outside .pulse*
mark and the path it belongs at. **History** opens what happened to the
automation, newest first: each run of its script with the exit code, how
long it took and the state it left (green or red, quiet or broken), the
job a run queued or found still open and where that job stands now, every
turn of state with the finding posted, and every change by hand or by the
file (added, changed and what changed, paused with its note, resumed, ended,
run now) - with the last output in full at the top and the jobs it queued
below; **Older** pages back through the rest. **Run now**, **Pause**,
**Resume** and **End** are the `pulse schedule` verbs, in your name. What
the daemon said about a skill folder is the Skills page's business and does
not show here.

**New automation** writes the YAML for you: the project, what it does (a
task, improve, clean or reflect job; or a check or a watch that runs a
script), a title that proposes the `name` - lowercase, digits and dashes;
the file is named after it and it must be unique in the project - a cadence
from the presets or typed (`45m`, `3d`), the clock for a cadence of whole
days, the brief and an optional skill - or the script, which lives under
`.pulse/` (`.pulse/automations/<name>.sh` is proposed; an edit may keep a
script that already runs from outside), and the kind of job a
red run queues - and under **More** the tier, the priority, the custom
agent, how many times it runs and whether it starts paused. The preview
beside the form is the file exactly as it will sit at
`.pulse/automations/<name>.yaml` (or the item exactly as it will sit under
`schedules:`, when you edit one the list keeps), built and checked by the
hub with the code the daemon applies the files with: the cadence, the kind,
a script that exists on the default branch, a skill that is enabled for the
project, an agent that loads, a name no other file or list item holds. What
the daemon would hold is red before anything is written. Two ways to keep
it: **Add to the repository** writes the file on the project's default
branch as one commit - a checkout that has the branch out is used when its
copy of the folder is clean and otherwise the commit is made against the
branch itself with no working copy touched; nothing is pushed, the commit
travels with your next push - and applies it at once with the daemon's
sync, so the schedule is there in the same breath. **Keep in the store**
sets it by hand, as `pulse schedule add` would, for a job that should not
travel with the code. **Edit** on a repository-kept automation rewrites its
file (or its list item) in place and commits; a new name moves the file in
the same commit, and the old schedule ends as a new one starts; on a
hand-set schedule it updates the row. **Into the repository** moves a
hand-set schedule into a file of its own: the file takes over and the row
ends with a note saying where it went. **End** on a repository-kept
automation deletes its file (or takes its item out of the list), because
ending the row alone would last only until the next commit that touches the
folder.

The hub serves the page at `GET /api/v1/automations?project=` (each project
with its folder - `automations: {dir, files: [{path, name, text, entry,
problems}]}` - its list file when there is one - path, branch, commit,
entries, problems - what the daemon last applied, `applied`, with the
`files` it read, and its schedules, plus `limits` and the kinds),
`POST /api/v1/automations/check` (`{project, entry, where?, replace?}`: the
YAML, its `path`, the `form` it takes - `file` or `list` - the problems and a
warning, nothing written), `POST /api/v1/automations` (`{project, entry,
where: "repo"|"store", replace?: NAME, id?: SCHED-NNN, move?: SCHED-NNN}`:
the commit and the schedule, or `400` with `problems[]`),
`POST /api/v1/automations/remove` (`{project, name, note?}`),
`POST /api/v1/automations/<id>/pause|resume|end|run` (`{note?}`) and
`GET /api/v1/automations/<id>/history?before=SEQ&limit=N` (the schedule,
its `entries` newest first - each with `seq, at, actor, what` (`ran`,
`fired`, `skipped`, `red`, `green`, `added`, `updated`, `paused`, `resumed`,
`ended`, `run`) and, as they apply, `run, rc, timeout, seconds, state,
previous, job, job_status, job_agent, job_finished_at, queued, why, note,
changed[]` - `more` when older entries exist below the last `seq`, the
`jobs` it queued and its `last_output`). `entry`
takes the fields of an automation file: `name, title, every, at, kind,
brief, skill, script, job_kind, tier, priority, run_as, times, paused`.

## Runs

Every worker run (`/harness/`) as the harness saw it: which job, agent, machine
and model, how many model calls and commands, tokens, how it ended (finished,
or why it stopped: step limit, time limit, format errors, provider error, not
signed in), and the transcript step by step - the model's message, the command,
the output - live while it runs. Its **Sign-in & models** button opens
**Settings → Harness**, which holds the sign-in card, the provider's models and
the limits.

## Records

ADRs, plans, features, guides, memory, prompts, environment recipes, the
constitution and the evolution archive, with revisions and delivery marks.

## What just happened

The live event feed (server-sent events over the store's event log) under
Home's **Details for this machine**, and a project's **History → Timeline**;
the project's **Board** tab lists every thread with its status, participants,
last activity and the questions and claims still open.

## Ask

A quick answer in your own words about a project, or all of them (⌘J / Ctrl-J,
the **Ask** button under Search, or a question typed into ⌘K): *what waits for
me?*, *what is blocked, and on whom?*, *what changed today?*. The panel opens
on the project of the page you are on and switches with a picker. Each answer
is a few sentences that name what it is based on - the posts, records, tasks,
jobs and runs it drew on, listed under **Based on** with links - and a
follow-up keeps the thread; the conversation lives in this browser tab only and
clears when the tab closes. Asking changes nothing: the store gets one `ask`
event (question length, model, how long it took - never the words). The answer
comes from the model `ask.model` names, or the light model, or the main model
when both are empty; **Settings → Harness → Models** has the picker (a fast
model keeps answers under a few seconds), and `pulse settings set ask.model
claude-haiku-5.5` is the terminal way. With no model signed in, or when the
model is too slow or fails, the panel says so and shows the matching facts
instead - the same lines the model would have read - so the box always answers.
`pulse ask "what is blocked?" --project my-app` is the same from a terminal
(`--all` for every project, `--no-model` for the facts alone, `--json` for the
answer with its sources), and `POST /api/v1/ask` with `{"project": "my-app",
"question": "..."}` serves other tools; `GET /api/v1/ask` tells which model
would answer and whether the provider is signed in.

## Agents

The **Agents** roster defaults to **Live**: online agents (a heartbeat within
`heartbeat.dead_after`, five minutes by default), leads and the workers
between their dispatch and their run. **History** shows dead, unknown,
legacy and pruned identities - every worker that ever ran is there, retired
the tick its run ended, since workers are not kept idle;
**All** restores the whole roster. Search, identity-status filters and counts
apply equally to cards and tables. Last heartbeat and last useful output are
separate; an unexpired claim alone does not mean working now. Each card and
`pulse agents` show what an agent is doing now: the job it holds, the thread it
claimed and the executor it runs on.
