# The harness (how an agent runs)

Every job runs through Hive's own agent harness, `hive harness run` - a
small, readable loop in the lineage of
[mini-swe-agent](https://github.com/SWE-agent/mini-swe-agent): one model,
seven typed tools (`bash`, `read`, `grep`, `glob`, `edit`, `write`, `plan`)
plus `skill`, `fetch` and `subagent` where the owner allows them and the
project's MCP servers' tools, a linear history, and limits the owner sets.

```text
system prompt (+ the clone's AGENTS.md) + the job's brief
  ─► model answers (one or more tool calls, or exactly one fenced bash/read/grep/edit block)
  ─► every call runs in the clone root, in order; cd <root> is unnecessary
  ─► each result (cut in the middle past harness.output_limit) goes back as that call's observation
  ─► ... until the model answers with no call: that message is the final report
```

## The tools

### The tool set

Every run offers the same typed tools, each a JSON
schema the provider's tool-calling API sees, each confined to the clone
and subject to the same deny rules and output cap as a shell command:

| tool | what it does |
|------|--------------|
| `bash` | one non-interactive `/bin/sh -c` command, a fresh process each time, with a `description` - one plain sentence on what it does, kept as the step's title and shown to the owner in place of the command line; `background: true` starts it as a **background process** instead (below) |
| `read` | one file, every line numbered; `start`/`end` read a range, a binary file reports its size |
| `grep` | a Rust regex over the clone, a directory, a file or a glob; `path:line: text` lines back |
| `glob` | the files whose path matches a glob (`*` crosses `/`), at most 500, `.git` left out |
| `edit` | exact search/replace in one file, each search matching once; `edits` applies several in order; an empty search creates the file |
| `write` | a whole new file, parents created; refuses an existing file unless `overwrite` is true |
| `plan` | the task's checklist (todo / doing / done), shown in the transcript and echoed back |
| `process` | the background processes of the run: `output` reads what one has printed since the last look, `wait` gives it up to `seconds` to end, `stop` ends it, `list` names them |
| `memory` | what is kept for later runs: `remember` a fact for the project or the user, `forget` one, `list` them (**Memory**, below) |
| `skill` | one skill's `SKILL.md` by name, or one file of it (**Skills as tools**, below) |
| `fetch` | one web page from a host the owner allowed, as text (**Web pages**, below) |
| `web_search` | a web search, numbered results with title, address and snippet (**Web search**, below) |
| `subagent` | a bounded helper run on a self-contained sub-task, its report back as the observation (**Helpers**, below) |
| `chat` | in a hub session only: `start` a sub-session with a title and a brief, `send` it a message, `read` its last messages (**Sub-sessions** under **Sessions** in [The owner's path](console.md#the-console-for-the-owner)) |

A reply may carry **several calls**, and every one gets its own
observation, so a model reads three files in one turn instead of three.
The calls that only look - `read`, `grep`, `glob` and `fetch` - run **at
the same time** when a reply carries two or more of them (eight threads at
most), each confined to the clone as before; `bash`, `edit`, `write`,
`plan`, `skill`, `subagent` and MCP calls run one after another in the
order the model gave, after the batch, and the results go back in the
order of the calls, so the model sees nothing of the scheduling but the
speed. A lone `read` beside a `bash` call is no batch and still runs in
order, which is what a model that writes a file and reads it back
expects. A call to a tool that does not exist, or with arguments of the wrong
shape, comes back as an error naming the tools and counts like a prose-only
reply towards `format_error`; a call that ran but found nothing (a missing
file, a search that matched twice) is an ordinary observation with a
non-zero return code. The fenced blocks below are still accepted for models
without tool calling - the typed tools are the same operations.

### Step timeouts

A command that is still running after `harness.step_timeout` seconds
(300; `--step-timeout N` for one run) is killed with its whole process
group and comes back with return code -1, what it had printed by then -
cut with the tail favoured past `harness.output_limit`, since where a
stuck command got to is in its last lines - and a closing line that says
so and what to do instead: narrow the command, wrap it in `timeout <secs>`,
pipe long output through `head` rather than `tail` (which prints nothing
until its input ends; a `... | tail -40` that timed out used to come back
empty), or start it with `bash background:true` and read it with the
`process` tool. The system prompt says the same up front, with the scope
rule that `grep`, `find` and `ls` stay in the project tree and never scan
`~/.hive` or the home directory: in three days of owner task runs, 22
commands ran the full 900-second live timeout, among them `grep -rl`
over `~/.hive` and long measurements piped through `tail`, for three
hours of wall time that returned nothing.

### Background processes

A command runs to completion and whatever it
left behind - a `cmd &`, a forgotten watcher - is ended with it, so a
server or a watch-mode build goes through `bash` with `background: true`:
the command starts in its own process group, the call comes back after
`wait` seconds (2 by default, up to 600) with what was printed so far and
an id - `[bg-1 is running in the background (pid 4242) after 2.0s]` - or,
if it already ended within the wait, with its output and exit code as if
it had run in the foreground. From then on the `process` tool manages it:
`output` returns the new lines since the last read (the buffer keeps the
last 2 MiB and says how much it dropped), `wait` with `seconds` blocks
until it ends or the time is up and returns the output either way, `stop`
ends the whole group, `list` shows each one with its state. Eight may run
at once; every one still running is stopped when the run or session ends,
and a note in the trajectory says which. Jobs get the same tool, so an
`npm run dev` under a job's test step is no longer a hang; read-only runs
and plan mode do not offer it, since a process that outlives a command
cannot be covered by the snapshot they restore. In a session `/procs`
lists them, `/procs ID [LINES]` shows the tail of one, `/procs stop ID|all`
ends them ([Chatting with the harness](#chatting-with-the-harness), below).

## What a run brings with it

### Memory

What a run learns the hard way - the test command that
actually works, where the API lives, that the owner wants short replies -
is gone when it ends, so the `memory` tool keeps it: `remember` with a
`text` (one line, 600 characters at most) appends a dated bullet to the
project's file, `~/.hive/memory/<project>.md`, or with `scope: user` to
`~/.hive/memory/user.md`, which every project reads; `forget` with enough
of a bullet's text to match exactly one takes it out (several matches are
listed instead); `list` shows both files. The project is the fleet's
project slug when the run is a job or the directory belongs to a
registered project, else the `origin` remote as `github.com-owner-repo`,
else the path - so a chat in a project's checkout and the jobs on that
project share one file. The next run in the same project reads both files
into its system prompt after the repository instructions, each bullet
dated, 16 KB in all (`memory list` shows the rest), with a note in the
session saying which files it read; the files are plain Markdown, edited
by hand as well, and lines that are not bullets stay where they are.
Writing a memory needs no approval - it changes nothing in the clone -
but it shows as a `memory:` step and permission rules reach it by scope:
`--deny-tool 'memory(user)'`. A read-only run (an answer job, plan mode)
reads memory but is not offered the tool; a helper shares the parent's.
`harness.memory` (`on`) turns it off, `--no-memory` for one run, and
`--bare` leaves it out with the rest. In a session `/memory` lists both
files, `/memory add [user] TEXT` and `/memory forget TEXT` edit them
without a model turn, `/memory clear project|user` empties one
([Chatting with the harness](#chatting-with-the-harness), below).

### Teacher mode

An experienced developer reads a diff and knows why
the agent chose what it chose; a junior who never touches the code learns
nothing from it, and that is a cost for later. `harness.teach on` (or
`--teach` for one run or session, `--teach-off` to skip it once) keeps
the work as it is but asks the agent to teach as it goes: whenever it
makes a design or coding choice worth learning from - a pattern, a data
structure, where code lives, a test strategy, a trade-off, a fix and the
bug behind it - it says in a sentence or two what it chose, the
alternative it passed over and why, in the reply that makes the choice.
In a session it names the concept and the file and line, and closes a
piece of work with the lessons to take away. A job repeats the lessons
worth keeping as `LESSON:` lines in its report (three to seven; what any
developer knows is skipped); the runner posts each as a `lesson` post on
the project board, in the job's thread, under the `lessons` area, and
lists them in the run report, so the why of a run stands beside its
result where `hive board` and `hive ask` find it. The paragraph sits
in the system prompt at the template's `{{.Teach}}` key, after the
memory; a template without the key gets it appended.

### Repository instructions

A run reads the owner's own
`~/.hive/AGENTS.md` first, then the clone's `AGENTS.md`, `CLAUDE.md`,
`GEMINI.md` and `.github/copilot-instructions.md` - each once (a copy of
another is skipped), 16 KB per file with a marker where it was cut, 32 KB
in all - and places them in the system prompt where the template's
`{{.Instructions}}` key stands (the built-in template puts them just before
the task; a custom template without the key gets them appended), under the
sentence that the harness's own rules still win where they differ. The
rule layouts of the other harnesses are read as they are: every Markdown
file under `.hive/rules`, `.claude/rules` (Claude Code) and
`.github/instructions` (Copilot's `*.instructions.md`), recursively, in
path order. A rule without a scope - no `applyTo:` or `paths:` in its
front matter, or one that says `**` - joins the system prompt under its
path; a scoped rule (`applyTo: "src/**/*.ts"`, a list, `{a,b}`, a plain
directory name) waits and is handed to the model, once, the first time it
reads, edits or writes a file the scope covers, appended to that tool's
result with its source and scope named, and noted in the transcript
(`instructions for src/api/a.ts from .github/instructions/ts.instructions.md
(applies to src/**/*.ts; 212 bytes)`). An `AGENTS.md` or `CLAUDE.md` in a
directory below the root works the same way, for everything under that
directory. `harness.instructions` (`on`; Settings → Harness **Run mode and
transcript size**) turns all of it off for every run, `--no-instructions`
for one; `hive harness prompt -C <clone>` shows the prompt a run there
would start with, `/env` in a session lists the rules still waiting
(`path rules: …`). The transcript's first `note` steps say which files
were read and how many rules wait.

### Custom agents

The helper the `subagent` tool runs can be one the repository or the owner
described: a Markdown file with front matter under `.hive/agents`,
`.claude/agents` (Claude Code's `<name>.md`), `.github/agents` (Copilot's
`<name>.agent.md`) or `~/.hive/agents`, or an `agent` record of the store (the
importer makes one of each `.hive/agents` file). A name belongs to its most
trusted source - a store record, then `~/.hive/agents`, then `.hive/agents`,
`.claude/agents`, `.github/agents` - and a lower twin of the same name may only
narrow it: fewer `tools`, more denies, a smaller `max_steps`, read-only;
whatever else it says is dropped with a warning. `hive agents list` (and the
hub) show which source won. Its `description` tells the model when to pick it
(the tool lists them and takes `agent: NAME`), its body - up to 30,000
characters - becomes the helper's standing instructions ahead of the sub-task,
and its `tools` list - in any harness's spelling: `Read`, `view`, `shell`,
`WebFetch`, `Task`, `mcp__github__*`, Copilot's `github/*`, OpenCode's
`mcp_github_search`; `Bash(git *)` counts as `bash` - narrows what the helper
gets: a tool outside the list (or in `disallowedTools`) is neither offered to
it nor run for it, the same way a `deny` rule works. `model`, `maxTurns` (it
may only lower the step cap), `permissionMode: plan` and `readonly: true` apply
too; `permissionMode: bypassPermissions` keeps the file from loading, and so
does a `hooks:` block or an `allow: "*"` in a repository file. A helper also
inherits the run's permission rules and the path-scoped rules above. Under a
`hive:` key the file speaks to Hive itself: `kinds` and `select: {tags,
files}` name the job cards it runs - a job whose kind is listed and whose
`Tags:` (else `Class:`) words or `File:` paths match runs under that agent's
model, tools, step cap and standing instructions, the runner handing the worker
`hive harness run --agent NAME` (a remote hand reads the clone's files; a
store-only record is invisible to it); a job pinned to an agent by name (`hive
job add --run-as NAME`, or a schedule's or a job hook's `--run-as`) runs as it
whatever its `kinds` say, so long as it still loads; `max_steps` and
`max_subagents` cap the run and the helpers it may start; `brief`, `schedule`
(`every`, 60 minutes or more), `quorum` and `project_scope` are read and linted
for the phases that will consume them. `--agent NAME` runs any `hive harness
run` as that agent; its `model` applies only when `models.allowed` lets it
(empty = any model the provider accepts; a comma or newline list, `claude-*`
globs), else the run keeps the model it had and says so. `hive agents lint
[PATH…]` reads the files one by one - the keys, what each tool name became, the
limits (1024 characters of description), the template, the Tier 0/1 rules, the
twins - and exits 1 on an error; `hive agents schema` prints the front
matter's JSON Schema; `hive agents list` what loads and from where (`hive
agent`, singular, manages the running workers). `--no-agents` leaves the agents
out for one run; they follow `harness.subagents` otherwise.

### MCP servers as tools

With a store at hand, a run connects the
project's enabled [MCP servers](board.md#connecting-the-outside-world-hive-mcp)
when it starts and offers their tools to the model by name,
`mcp__<server>__<tool>` (letters, digits, `-` and `_`; longer names are
cut, clashes get a numeric suffix), described as the server describes them
and called through the same client as `hive mcp call`. Past 32 tools the
run offers `mcp_search` instead - the model looks tools up by words in their
name or description, the matches come back with their schema and are
advertised from the next reply on, and any known tool is callable by name
even before it was looked up. A server that does not answer within 45
seconds is noted in the transcript and skipped; the run goes on without it.
`harness.mcp_tools` (`on`) turns it off, `--no-mcp` for one run; the
`hive mcp` commands stay available from `bash` either way. Each MCP call is
an `mcp` step in the transcript with the arguments it sent and the text the
server returned.

A stdio server of a run does not start in the clone: a conversation's
run in its artifacts folder (`$HIVE_ARTIFACTS`, the one the hub shows
beside the conversation), a job's in the run's own `TMPDIR` beside the
clone, and `hive mcp call` from either shell does the same - so what a
server writes never lands in the checkout, and the sweep that ends what a
command leaves running under a job's clone leaves the servers alone. A
Playwright server (`@playwright/mcp`, `playwright-mcp`) also gets, unless
its registration says otherwise, `--headless`, `--isolated` (a fresh
profile each start), absolute file paths, that folder as its output
directory, and the browser this machine has - Chrome, Edge, a Chromium
such as Brave, or Playwright's own download - instead of Playwright's
assumption of Chrome; `hive mcp show playwright` prints the line. So an
agent's `browser_take_screenshot` lands in the conversation's artifacts
folder where the owner sees it, and a run on a machine with only Edge
works out of the box. When the run ends, a stdio server gets its stdin
closed and a few seconds to shut its browser down before it is killed.

### Skills as tools

A run finds the clone's skill folders - `.hive/skills`,
`.claude/skills`, `.github/skills`, `.agents/skills`, then
`~/.hive/skills` - each `<name>/SKILL.md` (front-matter `name:` and
`description:` read when present, the folder name otherwise; the first
folder to define a name wins) - and, with a store, the project's
[`hive skill`](board.md#shared-skills-hive-skill) registry for the names the
folders do not define. The model sees them as the `skill` tool, each name
with its one-line description in the tool's schema, and loads one when the
task calls for it: `skill(name)` returns the body (and the list of files
the folder carries), `skill(name, file)` one of those files. An unknown
name comes back with the names that exist. The system prompt gets a line
saying skills exist and how to load one; nothing is loaded that the model
did not ask for, so a project with forty skills costs the context a few
hundred tokens, not forty documents. `harness.skills` (`on`) turns it off,
`--no-skills` for one run; the brief still carries the skills the lead
chose, exactly as before. Each load is a `skill` step. With a daemon tending
the project, the same folders on the default branch are also registered as
`hive skill`s of the project, so their names reach the brief, remote workers
and schedules ([the project's own files](board.md#the-projects-own-files-hive)).

### Hooks

The clone's `.hive/hooks.json` and the owner's
`~/.hive/hooks.json` name commands the harness runs at six moments of a
run - `run_start`, `user_prompt`, `pre_tool`, `post_tool`, `pre_compact`,
`run_end` - and the clone's hooks run first:

```json
{"hooks": [
  {"event": "pre_tool",  "tools": ["bash"],          "command": "hive-guard --stdin", "timeout": 10},
  {"event": "post_tool", "tools": ["edit", "write"], "command": "npm run lint:changed"},
  {"event": "run_end",                               "command": "./scripts/after-run.sh"}
]}
```

(a bare array is accepted too; `tools` left out means every tool, `timeout`
defaults to 60 seconds, 600 at most). Each hook runs through `/bin/sh -c`
in the clone root with the event as JSON on stdin (`event`, `cwd`, `tool`,
`input` for the tool events; `task`, `model`, `depth` at the start;
`exit_status`, `calls`, `steps`, `submission` at the end) and
`HIVE_HOOK_EVENT` / `HIVE_TOOL` in the environment. A `pre_tool` hook
that exits `2` **blocks the call**: the model sees ``blocked by hook `cmd`:
<its stderr or stdout>`` as the observation and the tool never runs; any other
failure is noted and the call proceeds. A `post_tool` hook's output is
appended to the observation under ``[hook `cmd`]``, so a linter that runs
after every edit speaks to the model in the same turn. What a `run_start`
hook prints is appended to the system prompt (a build number, the branch
policy, today's date); `run_end` runs once, before the clone is released,
whatever the exit status. Two moments belong to sessions: a `user_prompt`
hook runs on every message you send in `hive harness chat` (payload
`prompt`, `turn`, `messages`, `model`) before the model sees it - what it
prints travels with the message under ``--- user_prompt hook `cmd` ---``
(the current branch, the ticket the folder belongs to, a reminder), and exit
`2` **keeps the message back**: the reply is the hook's words, nothing is
sent and the turn does not count; a `pre_compact` hook runs before the
conversation is summarised, by `/compact` or on its own (payload `trigger`
`manual`/`auto`/`size`/`oversize` - by hand, past the token threshold,
past the request-size budget, after the provider refused a request as too
large - `compacting`, `prompt_tokens`, `context_window`) - its
output is handed to the summariser as the owner's asks ("keep the list of
files touched"), and exit `2` declines the compaction, which `/compact`
reports and an automatic one retries later. Hooks run as the owner within
their own `timeout`, and never count as model calls or tool steps; each is
a `hook` step in the transcript with the event and the tool it answered.
`harness.hooks` (`on`) turns them off, `--no-hooks` for one run. These are
the model's in-run hooks; the daemon's own `harness.step` and job events
on the bus are a different thing.

### Helpers

A top-level run may delegate a self-contained sub-task - "find every caller of
X and report which pass a delay", "run the suite and summarise the
failures" - to the `subagent` tool: a second agent loop on the same model and
clone (`cwd` shared, nothing copied) with its own message history, the parent's
skills, hooks and fetch policy, no MCP servers and no helpers of its own (one
level deep). The parent chooses `max_steps` (20 by default); the child is given
at most the parent's remaining calls minus a reserve of four, the remaining
minutes minus one, and the remaining cost, and the parent is told plainly when
too little remains (`subagent: only 3 model calls remain in this run; do the
work yourself and finish`). The child's final report returns as the
observation - `subagent submitted: 2 model calls, 1 tool steps, 17s` and the
text - and every one of its steps lands in the parent's session tagged with
`depth: 1`, numbered after the parent's; the console indents them and the
transcript's `## 25 >1 grep` heading marks them. The child's calls, tool steps
and usage count against the parent's budgets; a child that loses the clone ends
the parent with `sandbox_lost` too. `harness.subagents` (`on`) turns it off,
`--no-subagents` for one run. A helper never sees the tool; a job's brief is
still written for one agent and the lead still plans with jobs, not helpers. A
hub session has the `chat` tool beside it for the other shape of
delegation: a **sub-session** is not bounded by the parent's turn - it is
a session of its own with its own harness session, page and budget, in the
parent's worktree, that the owner can open and steer, and it reports back to
the parent as notes that wake it (**Sessions** in [The owner's
path](console.md#the-console-for-the-owner)). A session's brief tells the agent to
split a request with several parts, or with one part that takes a while, and
which to pick for each: a helper for a bounded self-contained sub-task whose
answer it needs now (a survey of the code, a suite run summarised), a
sub-session for a part that should keep going and be watched on its own
while it answers the owner, one per independent part when there are several; a
sub-session's own brief says to do its part itself.

### Web pages

The `fetch` tool reads one `http(s)` page - a documentation
page, a changelog, an issue the model was pointed at - as text (HTML
reduced to its text with links kept, JSON and plain text as they are, 2 MB
read at most, 30 seconds, five redirects), cut past `harness.output_limit`
like any observation. It exists only where the owner says which hosts:
`harness.fetch_allow` (empty; Settings → Harness **Run mode and transcript
size**) is a comma-separated list of hosts, `*.` for subdomains -
`docs.rs, *.rust-lang.org, github.com` - and `--fetch-allow HOSTS` sets it
for one run. Empty means the tool is not offered; a URL whose host is not
listed, or that redirects off the list, is refused with the list named
(`refused by policy: host "x" is not in harness.fetch_allow …`) as an
`error` step. The deny rules for credentials do not apply to a page the
model reads, but a page's text is observation, never instruction - the
system prompt says so. Each fetch is a `fetch` step with the URL.

### Web search

The `web_search` tool answers a query with up to twenty
numbered results - title, address, snippet - the way a person opens a
search tab before reading a page: `8 results for "tokio select
cancellation" from duckduckgo:` then `1. … / https://… / …`, or `no
results for … ; try other words`. Without a key it asks DuckDuckGo's HTML
endpoint; with `BRAVE_SEARCH_API_KEY` in the environment, or a key saved
in `~/.hive/private/providers/brave.json` as `{"api_key": "…"}`, it asks
Brave Search instead (the key never goes into settings - Hive refuses
secret-shaped values there). `harness.web_search` says where it is
offered: `chat` (the default) in `hive harness chat`, where the owner is
there to read along, and not to jobs; `on` to jobs too; `off` nowhere -
and `--web-search` / `--no-web-search` decide for one run. Searching
changes nothing, so a read-only run keeps the tool, and a helper searches
through its parent's engine. The query is what a permission rule sees:
`--deny-tool 'web_search'` withdraws the tool, `--deny-tool
'web_search(* internal *)'` refuses only such queries. Results are
observation like a fetched page, searches run in parallel beside reads
and fetches, each is a `search` step with the query, and `/env` names the
engine (`web search: duckduckgo`, `brave` or `off`). To read a result the
model needs `fetch`, so its host has to be in `harness.fetch_allow`;
`HIVE_WEB_SEARCH_URL` points the engine at another endpoint (tests and
proxies).

### Read-only runs

The jobs that look but do not touch - `answer` and
`ballot` - run with `--read-only`, and `hive harness run --read-only`
gives any run the same discipline. "Read-only" is about the clone, not the
machine: the `edit` and `write` tools are not offered (the model is told
so in its prompt and in the run's first `note` step), and a `bash` command
that would change the clone is refused before it starts with an `error`
step and a result that names the first offending part - ``refused: this is
a read-only run and `git commit` makes a commit (a read-only run lands
nothing). Look, test and report; change nothing and commit nothing.
Scratch files go under /tmp.`` - while `cargo test`, `npm ci`, `git log`,
`cargo fmt --check`, a pipe into `head` or `/dev/null`, and anything that
writes under `/tmp`, `$TMPDIR` or after a `cd /tmp` run as usual. The
checker reads the command the way `/bin/sh` would - quotes, `&&`, pipes,
`>`/`>>`/`2>` redirections, `$(...)`, backticks, heredoc bodies (a `>`
inside one is text), `sh -c '…'`, `eval`, `xargs`, `find -delete`/`-exec`,
`sudo`/`env`/`timeout`/`npx`/`uv run` and the other wrappers - and knows
the writers: `git add|commit|checkout|stash|reset|push|clean|…`, `rm`,
`mkdir`, `touch`, `cp`/`mv`/`ln`/`rsync` destinations, `sed -i`, `perl -i`,
`patch` without `--dry-run`, `tar x`, `unzip`, `curl -o`/`wget`, the
formatters that rewrite (`cargo fmt`, `rustfmt`, `prettier --write`,
`eslint --fix`, `gofmt -w`, `black`, `ruff format`, …) and the package
managers' manifest-editing subcommands (`npm install <pkg>`, `cargo add`,
`cargo update`, `go get`, `go mod tidy`, `uv add`, `poetry add`, …; a
plain `npm ci`, `cargo build` or `uv sync` is fine). What the checker
cannot see - a Python one-liner that opens a file for writing - the tree
guard catches: every command in a read-only run is bracketed by a snapshot
of the clone's status, HEAD, branch and local refs, and whatever changed is
put back afterwards (new files removed with the directories they left
empty, tracked files restored from HEAD, a commit soft-reset and its files
restored, a new branch or tag deleted, a moved one moved back), with the
result ending in `[read-only run: the command changed the clone (made.txt
(new, removed); a.txt); the change was put back. …]` and an `error` step.
A read-only run's budget notice asks for the report without the commit, a
helper started by a read-only run is read-only too, and the fenced
`edit` block is refused like the tool.

### Team secrets (the vault)

A job that needs an API key or token gets it from the team vault (ADR-070
rule 4, step 4), never from a file in its clone. The brain keeps the
vault in its store: the `secrets` table holds only ciphertext
(ChaCha20-Poly1305 under one team data key, a fresh nonce per write and the
name bound as associated data). The data key is stored wrapped, once per
holder: X25519 to the holder's key, then HKDF-SHA256 and ChaCha20-Poly1305.
Today the brain is the only holder. Its key is a mode-0600 file in the
brain's private directory, outside the store. The step 2 device registry
adds a holder per paired device.

- `printf %s "$TOKEN" | hive secret set NAME` seals a value. The value
  comes on stdin only. It is refused from a terminal and from inside a job,
  so it never sits in shell history or a run log. `hive secret list
  [--json]` shows names, who set them and when, never values. `hive
  secret rm NAME` removes one. A name is an environment variable name, and
  a value is at most 64 KiB. The hub's Settings → Secrets panel does the
  same over `/api/v1/secrets`. It lists names, seals a new value and
  removes one, and it never answers a value.
- A project's environment recipe declares the names its jobs may have:
  `secrets: [DEPLOY_TOKEN, SENTRY_DSN]` beside `env:`. A recipe never holds
  a value.
- When a job runs, the brain grants every name the recipe declares and
  records `secret.granted`. A brief line `Secrets: A, B` asks for names.
  Each asked name the recipe does not declare (or that is not a valid
  name) is refused with the reason, and the refusal is recorded as
  `secret.refused`. The refusal is logged as `secret NAME refused: ...`,
  and the run carries on without that name.
- On the brain's own executor (LocalExec), the granted values are opened
  in memory and handed to the worker process as environment variables for
  that run only, with one `secret.read` event per name. They are wiped when
  the worker ends. Every value is replaced by `[secret NAME]` in the run
  log and in the parsed result before either is stored. Events carry names,
  never values. Setting and removing a value record `secret.set` and
  `secret.removed`.
- Remote hands cannot open the vault yet. A project whose recipe declares
  `secrets:` is marked unfit on them ("needs vault secrets, which only the
  brain opens yet"), so its jobs run on the brain.

Not built yet (TASK-097): a hand opening the team key with its own device
key (needs step 2's registry), provider logins as reserved vault names,
and `vault://NAME` references for MCP and cloud credentials. The vault adds
no setting.

## Budgets, context and compaction

### Reasoning effort and prompt caching

`harness.reasoning` (`default`;
Settings → Harness **Run mode and transcript size**, `--reasoning LEVEL` for
one run) asks the model for a reasoning effort - `minimal`, `low`,
`medium`, `high`, `xhigh` or `max` - sent as `reasoning_effort` to the
OpenAI-compatible providers and to Copilot for the models that take it;
`default` sends nothing and leaves the model's own setting. A model that
does not take the field rejects the call, so set it per project with the
model in mind (the gemini and anthropic providers ignore it this wave).
`worker_reasoning`, `light_reasoning` and `measure_reasoning` (Settings →
Harness **Models**) set it per role the way the model settings do - the
runs that change code, the light runs, the loop's measurements - and fall
back to `harness.reasoning` while empty; a conversation's **Effort** picker
sets it for that conversation alone.
Independently of any setting, the anthropic provider marks the system
prompt and the last block of the last message as cache breakpoints
(`cache_control: ephemeral`), so a long run's steady prefix - the prompt,
the instructions, the older turns - is read from Anthropic's cache on each
call instead of being billed and processed anew.

### Compaction

When a long run's prompt passes `harness.compact_at`
percent of the model's context window (`50`; Settings → Harness
**Transcript limits**, `--compact-at N` for one run, `0` turns it off, 95
at most), or the request it is about to send passes
`provider.max_request_mb` (`1` MiB; `--max-request-mb N`, `0` turns the
size trigger off), the harness asks the model itself for a summary of the
older part of the conversation - what was asked, what was learned, what
changed, what remains - and replaces those messages with it, keeping the
system prompt, the task and the last six messages verbatim; a `note` step
(`compacted 12 older messages into a 1480-character summary (31 messages
became 20)`) marks the moment, and says so when the request size was the
reason. It is the first thing the harness does about a full window, not
the last: it happens again each time the prompt grows back to the
threshold, and each summary is written with the earlier one in front of
it, so a run of any length carries its facts forward in a summary the
model wrote rather than losing them to the collapsing of old outputs
described under **Context discipline**, which now waits until compaction
has not made enough room. The summary call costs usage but not a model
call, reads at most about 30,000 tokens of transcript (the oldest part is
cut first, the earlier summary always travels whole), and may go to a
cheaper model than the run's (`harness.compact_model`; on the providers
that bill per token - openai, anthropic, gemini and the OpenAI-compatible
vendors - that saves directly, on Copilot, which bills per request, it
saves time); a run compacts again only after three more calls, and a
conversation too short to cut is left alone. The prompt size the token
threshold is measured against is the provider's own count for the last
call plus an estimate of what the history gained since, or an estimate
from the conversation's size when the provider reports none; the window
is the one the model lists, or 128,000 tokens when it lists none, unless
`harness.context_window` says otherwise. The request size is the harness's
own estimate of the body - the messages, their tool calls and images, the
tool schemas - and exists because a gateway gives up on a body long before
a 1M-token window is full: with the token threshold alone a Copilot
conversation grew until the gateway answered `Timed out reading request
body. Try again, or use a smaller request size.` (HTTP 408) and every retry
of the same body failed the same way. The trajectory JSON keeps the
compacted messages.

Two more moments compact. A request the provider refuses as too large all
the same - HTTP 408 or 413, or its words for it: OpenAI's
`context_length_exceeded` and `maximum context length`, Anthropic's
`prompt is too long`, Gemini's `input token count ... exceeds` - is not
sent again as it was: the harness compacts the conversation whatever the
three-call gap, asks again, and if that is still refused (or there was
nothing left to compact) collapses every older observation but one and asks
once more, each step a `note` in the transcript; only then does the turn
stop with the provider's error and a line on what helps (a new
conversation, which carries this one over compacted, or `/compact` in
`hive harness chat`). The
provider itself does not retry such a call - the body is the problem - so
the shrinking lives in the harness. And a session that resumes an earlier
one (a conversation whose agent was let go and comes back, `hive harness
chat --resume`, a held run carried on) compacts the carried-over history
before its first call when it is past either threshold, so a resumed
conversation starts lean instead of sending everything it ever saw.

The first-turn contract says that exactly one fenced block per reply is
executed, prose-only replies are not executed, and a command block can use
```` ```bash ````, ```` ```sh ````, ```` ```shell ````, ```` ```zsh ````,
```` ```console ````, or ```` ```terminal ````. The parser also accepts
`~~~` fences, a leading `$ ` prompt on the first command line, and a
single-line inline-code command when the whole reply is only that code.
Multiple command/action blocks are still refused as ambiguous. The final
report is what the runner has always read - `SUMMARY:`,
`FINDING:`, `SUGGESTION:`, `QUESTION:`, `VOTE:`, `DECIDE:` and, in
teacher mode, `LESSON:` lines - so
briefs, planning, delivery, lessons and the board are unchanged. A run that
hits a limit still reports: `SUMMARY: harness stopped: step_limit after 60
model calls and 41 commands; format_errors=0` and the model's last message
as a `FINDING:`. Report headings and report keywords are matched
case-insensitively, including Markdown heading levels. Exit status zero alone
is not a completion report: empty or unstructured output without an explicit
job-completion record is rejected. One prose-only reply receives a
`format_retry` corrective step without consuming `harness.max_steps`; later
consecutive prose-only replies count as format errors and the summary exposes
`format_errors=N`. A successful answer job writes its summary back to the original, still-unanswered worker
question; failed, owner-only or still-blocked answers do not close questions.
The `SUMMARY:` line opens with an **outcome code** - `done`, `nothing-to-do`,
`blocked` or `failed` (`SUMMARY: done - fixed the parser; cargo test green`,
`SUMMARY: nothing-to-do - 4f09ccbc covers it`, `SUMMARY: blocked: no login`)
- which both the job brief and the harness prompt ask for. The runner reads
that code before any of the summary's words (`classify_outcome` in
`crates/hive-runner/src/brief.rs`); only a summary without a code falls back
to the old text rules (`blocked:`/`failed:` before the first colon, the
"blocked" regex in recovery's failure classification). Every run writes the
code and the path that classified it to `runs.outcome` and
`runs.outcome_source` (`code`, `text` for the regex fallback, `none` without
a summary), says it in the run log (`outcome: blocked (from the worker's
SUMMARY outcome code)`) and on its `run.finish` event, and `hive status`
counts the day's runs by source - so the fallback rate can be watched fall.
A `blocked` or `failed` code also fails the
job, even when the worker exits zero, so unfinished work reaches recovery
instead of silently closing a task. A command's completion marker is accepted
only when that command succeeds without timing out; otherwise the error
returns to the agent for diagnosis.

**Three budgets** bound every run, all owner settings (Settings → Harness
**Budgets**, or `hive settings set`): `harness.max_steps` model calls (60), the wall
clock (`minutes_per_job`, the harness stops one minute before the runner
would kill it), and `harness.max_cost` - the provider-reported cost one run
may spend, `0` meaning no cap. `harness.max_steps none` (also `off` or
`unlimited`; `--max-steps none` for one run) lifts the call cap
altogether: a run then goes on until it finishes or the other two budgets
end it, with **Compaction** keeping its conversation inside the context
window however long it runs, and the model is told its calls are
unlimited rather than given a number - a chat turn is uncapped the same
way. A custom agent's own `max_steps` still caps a run it takes.
The step budget is per kind in the shape of
the per-kind model settings: `harness.task_max_steps` (empty by default)
raises it for `task`, `improve` and `experiment` runs only - the kinds that
write - while `answer`, `ballot`, `reflect` and `onboard` stay on
`harness.max_steps`, so an owner task can have a larger budget without every
short run getting one. The unit is the provider's: Copilot sends
`usage.cost` only for some models, so a session's cost column shows `—`
rather than a number when the vendor did not say - and a run whose provider
reports no cost is bounded by the other two budgets alone. Whichever budget
trips ends the
run with `cost_limit`, `step_limit` or `time_limit` and a one-line reason on
the session (`stopped at the cost budget: 2.10 of 2`). A run the step budget
stops with work in its clone - commits since the base, uncommitted edits or
new files - is not failed on a task cut off mid-way: the run settles `held`
the way a run the runner's restart holds does (below), its job returns to
the queue with the attempt refunded and the clone kept, and the job's next
run - the same agent, which the runner dispatches first - carries the
conversation on in that clone with a fresh step budget and a note saying it
stopped at the budget, that this is the only continuation the job gets, and
to finish from where it stands without widening the scope. A job gets that
hold once: a second stop at the budget is judged and recovered as before
(`step_limit` in the failure class, the corrected brief carrying the stop's
reason).
A stop with nothing in the clone, under a custom worker command or on a
remote executor is judged as it stands.

### Trimming older results

Command output is what fills a context window, and most of it is output
the model is no longer working from: measured over three days of owner
task runs (36 sessions, 2,513 model calls, 79,000 prompt tokens a call on
average), tool results made up about 124,000 characters of an average
request and 84% of those sat in results older than the eight most recent,
while the prompt grew by about a thousand tokens a call until the
request-size compaction at 164,000 tokens. So every request sends the
last `harness.trim_keep_recent` (8; `0` turns it off) tool results whole
and any older one longer than `harness.trim_over` characters (4,000, about
the 80th percentile of measured results; `0` turns it off) as its first
and last 750 characters around a marker
(`[... trimmed 41,200 of 48,000 characters of this older result; run the
command again if you need them ...]`). Only the request is trimmed: the
conversation the run keeps, the trajectory and the session's steps hold
every output in full, so a compaction summary is written from the whole
transcript and the console shows what the command printed. The system
prompt, the task, user and assistant messages, the compaction summary and
the latest results never change, nothing moves, and each result still
follows the call it answers, so every provider accepts the request; the
byte budget of `provider.max_request_mb` is measured on the trimmed
request. The first request a session trims gets a `note` step (`sending
older command outputs trimmed to their first and last lines from here on:
this request carries 61,000 of the conversation's 131,000 result
characters ...`). On the measured sessions the defaults take 45% of the
result characters out of an average request - about 14,000 tokens a call
at four characters a token, 18,000 at the three the measured results
tokenise to - and replaying five real sessions through the rule cut a
quarter of the estimated request size over the whole run.

### Context discipline

Trimming (above) is the everyday measure; what follows is for a window
that fills anyway. While the prompt is under
the compaction threshold and the request under its byte budget nothing
else is touched; once it is past either and **Compaction** (above) is off, waiting
out its three-call gap or has not made enough room, the harness keeps only the last
`harness.keep_observations` (5; Settings → Harness **Transcript limits**,
next to `step_timeout` and `output_limit`) outputs verbatim and older
ones collapse to their first and last lines around a marker
(`[1840 characters elided - rerun the command to see them]`) - the model is
told this in its prompt and simply reruns a command when it needs the full
output again. Past 80% of the model's context window the collapsed outputs
become one-liners and the kept count halves. The window is the one the
provider lists for the model - Copilot's per-model limit, Anthropic's
`max_input_tokens` (a million for Claude Sonnet 4.6+, Opus 4.6+, Haiku 5.5,
Fable and Mythos; 200k for the older models), Gemini's `inputTokenLimit`
(a million) - or 128,000 tokens when it lists none, and
`harness.context_window` (Settings → Harness **Run mode
and transcript size**, or `--context-window` on `hive harness run`)
overrides it when the list says nothing or less than the deployment really
takes, `0` meaning "whatever the model lists". Each collapse leaves a `note`
step in the transcript (`collapsed 4 older observations to make room in the
context window (level 1, keep 5; the prompt was about 91000 of 128000
tokens)`) so the console shows what the model no longer sees; notes never
count as calls or commands.

### Native action blocks

For models without tool calling, the same
`read`, `grep` and `edit` operations are accepted as fenced blocks - one
per reply - and use the same output truncation and clone-root confinement
as shell observations:

````text
```read
src/lib.rs:1-80
README.md
```
````

`read` takes one relative path per line, optionally with `:START-END` line
ranges, and returns numbered text lines; binary files report their size only.

````text
```grep
TODO|FIXME
crates/hive-harness/src
```
````

`grep` takes a Rust regex on the first line and an optional relative file,
directory, or glob on the second line, returning `path:line: text` matches.

````text
```edit
src/lib.rs
<<<<<<< SEARCH
old text
=======
new text
>>>>>>> REPLACE
```
````

`edit` takes a relative path and one or more exact SEARCH/REPLACE blocks.
Each SEARCH must match exactly once or the file is left unchanged with a
clear match-count error. An empty SEARCH creates a new file only when the
target does not exist. Absolute paths, `..`, symlinks that escape the clone,
and paths denied by the harness policy are refused.

The trajectory JSON under `~/.hive/harness/<session>.json` is rewritten
atomically every five steps and again at session end, on harness errors and on
cancellation. A crash between flushes may lose up to four steps from the JSON
file; the durable record is the per-step `harness_steps` store rows. Every
stored step also emits a `harness.step` event on the bus (session, step number,
kind, exit code, job) - the console's Harness page refreshes on it, `GET
/api/v1/harness/sessions/ID?after=N` returns only the steps after `N`, and
`/api/v1/events?kind=harness.step&subject=ID` (or the `…/events/stream` SSE
endpoint with the same filters) follows one session
cheaply. The dashboard's Live panel leaves step events out; they belong on
the Harness page.

## Running, stopping and reading a run

### Testing the loop without a login

`HIVE_HARNESS_FAKE=1` registers a
scripted provider, `fake`: `hive harness run --provider fake --model fake-1
-p 'say hi' -C "$(mktemp -d)" --no-store` walks the whole loop on a built-in
two-reply script, and `HIVE_HARNESS_FAKE_SCRIPT=path.json` supplies your own
(`{"replies":[{"command":"echo hi"},{"report":"SUMMARY: done"}]}`; add
`"loop":true` to repeat the last reply). It never appears in Settings; it is
for tests and demos.
Commands or native paths that would leave the clone (`git push`, changing
`git remote`, any `gh` call) or touch credentials (`~/.ssh`, `.netrc`, the hive home's
`private/` directory under any spelling - `~/.hive/private`,
`$HIVE_HOME/private`, a measurement home's `private/providers` -
`~/.copilot`, `~/.config/gh`) are refused without being run; the model is
told why and continues. Native actions reuse the same deny rules as shell so
the two paths cannot drift. The macOS filesystem root `/private/`, which
`/tmp` and `/var` resolve to, is not a credential path.
A run never holds a git login either. With a GitHub App configured
(Settings → Git, [Credentials](landing.md#credentials)), the brain mints a
one-hour installation token for the job's one repository after the run
has ended and its commits passed the identity check; the hand that pushes
installs a credential helper for that one push, outside the worktree
(under the hive home, wired in through `GIT_CONFIG_*` variables, never
`.git/config`), and removes it when the push returns. The token is in no
log line, no event and no file the worker can commit
(`jobs::tests::a_job_leased_to_a_local_hive_hand_pushes_with_the_app_token`).

Exit codes tell the runner what happened without parsing prose: `0` the
model submitted; `1` a limit (`step_limit`, `time_limit`, `cost_limit`,
`format_error`); `3` the provider failed after retries (`provider_error`); `4`
not signed in - one stderr line beginning `not logged in:` names the reason
(no login, a token GitHub rejected, an account without a Copilot seat, a
token that is not Copilot-enabled) and the fix. The runner marks a `4` as a
login fault of the machine, never of the job - and while this machine has no
login the daemon starts no worker here at all: the `capacity` line of `hive
status` and the console's **The machine** panel say why (`workers held: …`
with a **Sign in** link to Settings → Harness), one daily finding lands on
the brain's board, and remote hands keep working. `5` means the owner
cancelled the run (`cancelled`, below). `6` means the run's working
directory vanished under it (`sandbox_lost`: a sweep or a cleanup removed
the clone while the model was still working) - the harness notices at the
first command that cannot start, says so in one error step, and ends the
run instead of spending its remaining budget on commands that all fail the
same way; the runner counts it as transient, so the job runs again in a
fresh clone.

### Stopping a run

The **Cancel run** button on a session (a click, then a
confirm - it posts `POST /api/v1/harness/sessions/ID/cancel`) or `hive
harness cancel ID` writes `cancel_requested_at` on the session and emits a
`harness.cancel` event. The harness polls its own session row every two
seconds and stops between steps with exit status `cancelled` - the in-flight
command is killed, the session is finished with `cancelled by the owner`, and
the transcript stays readable. The daemon watches the row too, so a worker
that cannot reach the store is stopped from outside. The job fails as
`cancelled by the owner` and **recovery leaves it alone** (no automatic
retry; the recovery note counts it as `cancelled by the owner`) - cancel the
task, or queue it again, when you want. A second click while the first is
pending answers `pending`; a session that already ended answers `409`. Off
the store, `--cancel-file PATH` (or `HIVE_HARNESS_CANCEL=PATH`) stops a run
when the file appears.

### The system prompt is a setting

`harness.system_prompt` holds the
`{{.Field}}` template the harness renders before every run; empty means the
built-in one. `hive harness prompt --default` prints the template, `hive
harness prompt` the rendered prompt for the current settings and directory,
`--raw` the stored template, and `GET /api/v1/harness/prompt` returns all of
those for the console's **System prompt** editor (Settings → Harness:
Save, Reset to default, Undo, Preview). Available fields: `{{.Cwd}}`, `{{.Info.cwd}}`,
`{{.Info.os}}`, `{{.Info.shell}}`, `{{.Model}}`, `{{.Provider}}`,
`{{.StepTimeout}}`, `{{.OutputLimit}}`, `{{.MaxSteps}}`, and
`{{.Instructions}}` for where the clone's instruction files go (a template
without it gets them appended; their text is never scanned for keys, so an
`AGENTS.md` that documents `{{.Something}}` is safe). A template that does
not parse is refused when saved; one that fails to render fails the run with
the template error rather than a silent default. `--prompt-file PATH` (or
`HIVE_HARNESS_PROMPT_FILE`) overrides the setting for one run.

Each run is a **session** in the store (`harness_sessions`, `harness_steps`)
keyed by the run id, streamed step by step while it runs - the console's
**Harness** page and `hive harness sessions` / `hive harness show ID` read
them. A run on a remote hand streams the same way: the harness there appends
begin, step and finish JSONL events to a steps file (`--steps-file PATH`, or
`HIVE_HARNESS_STEPS`, which the runner sets for remote jobs; `--steps-file -`
streams them to stdout one object per line, with the command's own `--json`
result as the last line, for a program that drives a run or a chat), the hands
serve it by offset (`steps JOB SESSION OFFSET`, next to the heartbeat
snapshot), and the daemon mirrors the lines into the brain's store as they
arrive, tagged with the executor's name - so the Harness page shows every
executor's steps before the job finishes. A trajectory JSON under
`~/.hive/harness/<session>.json` holds the full messages for replay or
evaluation; **Download trajectory** on a session (`GET
/api/v1/harness/sessions/ID/trajectory`, confined to that folder) hands you
the file, and `GET /api/v1/harness/sessions/ID/file?path=REL` reads one file
of the session's worktree (the chat canvas's File tabs; a relative path inside
that folder only, text up to 512 KiB, `binary: true` for the rest, 404 when
the worktree is gone).

The **Harness** page opens with the day's numbers - running, today,
submitted, failed, and cost (a dash for providers that report none) - from
the `counts` object every sessions list carries (`hive harness status`
prints the same `submitted_today` and `cost_today`). The list pages backwards
on a `(started_at, id)` cursor (`?before=…` from the previous page's
`next_before`; **Load more**), so a fleet whose sessions all started in the
same second never loses a row between pages; the first page stays live,
older pages are snapshots. A session's transcript follows `harness.step`
events and fetches only the new steps (`?after=N`), shows whether it is live
or polling, and keeps following while you are at the bottom. Job rows on a
task carry `harness_session`, so a task links straight to the run that
worked it.

## Chatting with the harness

`hive harness chat` is the same agent as a
conversation: it runs in your terminal, in the directory you point it at
(`-C`), with the same tools, instruction files, skills, hooks, MCP servers
and deny rules as a job - but it speaks to a person. Each line you type is a
**turn**: the model reads, searches, runs commands and edits until it has an
answer, which it prints; then it waits for your next line, keeping the whole
conversation in context (compaction and context discipline apply as in a
run). The step budget and the wall-clock cap (`--max-steps`, `--minutes`)
are **per turn**; a turn that hits one stops with a note and the session
goes on - say `continue`. `--max-cost` is for the whole session. Replies
**stream** as they are written from the providers that stream (Copilot and
the OpenAI-compatible ones: OpenAI, Azure, OpenRouter, Groq, Mistral,
DeepSeek, xAI, Together, Fireworks, Cerebras, GitHub Models, Ollama, LM
Studio, vLLM); the others print each reply when it is whole. Streaming
needs `stream_options`/`stream` in the API; a provider that rejects either
is asked again without it, then without streaming, and stays there for the
session. A run's model calls stream the same way with nobody watching: a
streamed reply opens at once and keeps bytes coming while the model works,
so the transport timeout (two minutes between bytes) tells a dead connection
from a model that thinks for five. A whole reply had to be finished inside
that timeout, so a run whose model took longer over an answer was cut off
and asked again, up to five times, and then stopped with `provider_error:
error sending request` - the gateway finishing replies nobody read.

Calls that **change things are shown first**: an `edit` or `write` (with the
blocks or the file's head), a command that would change the clone or that
crosses the job policy (`git push`, `gh`, …), or an MCP tool. Answer `y`,
`n`, `a` (always: the prompt names the rule it adds - `bash(git commit *)`
for a command, `edit` or `write` for the file tools, `fetch(host)`, the
tool's name for MCP - and no call it covers asks again this session),
`all` (approve everything from now on) or type a reason, which the model
hears (`bash: declined by the user: not that file`). `--yes` approves
everything from the start and `/approve` toggles it. Reads, searches, the
`plan`, `skill`, `fetch` and `subagent` tools and commands that only look
run without asking. The model may also **ask you** through a `question`
tool (a question and optional choices; a number picks one). When the input
is a pipe rather than a terminal - `echo 'what does this build?' | hive
harness chat -C .` - every line is a turn, changes are declined unless
`--yes`, questions go unanswered (the model is told to decide), and stdout
carries only the model's replies, so the output is scriptable; `--json`
prints one JSON object per turn and one for the session, and `--steps-file
-` adds every step as it happens to the same stream, so a program that
drives the chat sees the reads, commands and edits as well as the replies.

In a terminal the prompt is a **line editor**: arrow keys and Emacs keys edit
the line, Up/Down and Ctrl-R walk the input history (kept across sessions in
`~/.hive/harness/chat_history`), Tab completes `/commands` at the start of a
line and file paths after `@`, and a line ending in `\` continues on the next
(the editor keeps asking until the backslash goes). Text typed while a turn
runs is kept for the next prompt. **`@PATH`** anywhere in a message attaches
that file after your words (a directory attaches its listing; files over 48 KiB
are cut, binary ones named but not attached), so `explain @src/main.rs` needs
no read step. **`#123`** or **`owner/repo#123`** attaches the GitHub issue or
pull request the same way - title, state, author, labels, body, the latest
comments, and for a pull request its branches, size and files - read through
the `gh` CLI you are signed in with, so `fix #42` starts from the report itself
(a reference `gh` cannot read stays as typed and the prompt says why - a
repository the active `gh` account cannot see points you at `gh auth switch`).
**`!CMD`** runs a shell command yourself, shows its output, and hands the same
output to the model with your next message - `!cargo test` then `fix the
failing one`.

Commands: `/help`; `/exit` (also Ctrl-D, or Ctrl-C twice while idle;
Ctrl-C during a turn stops that turn); `/new` ends this session and starts
a fresh one in the same terminal; `/sessions` lists earlier sessions from
this directory and `/resume ID` carries one on in a new session; `/plan`
(or `/plan on|off`) toggles **plan mode** - read-only: `edit` and `write`
are withdrawn and commands that would change the clone are refused, so the
model can explore and propose before `/plan off` lets it act; `/model
NAME` switches the model for the next turns; `/compact` summarises the
older conversation now; `/usage` shows calls, tokens, cost and time;
`/approve` toggles approving everything; `/permissions` lists the
permission rules in force, `/permissions allow PATTERN` or `/permissions
deny PATTERN` adds one for the session and `/permissions clear` drops the
session's (see below); `/procs` lists the background processes the model
started with `bash background:true` (state, pid, command), `/procs ID
[LINES]` shows the last lines one printed (20 by default) and `/procs stop
ID|all` ends them - everything still running is stopped when the session
ends; `/memory` shows what is remembered for the project and for you,
`/memory add [user] TEXT` remembers one more, `/memory forget TEXT` drops
the one that matches, `/memory clear project|user` empties a file (the
model sees the change from the next session on; **Memory** above);
`/rewind` lists the turns so far and `/rewind N` puts the files the model
edited and the conversation back to before turn N (**Rewind** below);
`/diff` shows `git status` and
the diff stat of the clone; `/review` has the model review the working
tree's changes against HEAD (untracked files are named for it to read) and
`/review REF` the branch since REF (`/review main`) - one read-only turn
that hands the diff over and asks for findings with `path:line`, most
serious first, then puts the mode back; `/pr [TITLE]` has the model read
the branch's commits and diff, write the title and description, push and
run `gh pr create` - the push and the create go through the usual command
approval, and it refuses on `main` or with a detached HEAD; `/export
[PATH]` writes the conversation as
Markdown (default `~/.hive/harness/ID.md`); `/env` prints what the agent
has in hand - model, context window, budgets, mode, tools, instruction
files, skills, hooks, MCP servers, deny rules, permission rules,
background processes; `/init` asks the model to
write or improve the project's `AGENTS.md` from what it finds; `/delegate
[TITLE]` has the model turn the conversation so far into a brief and puts
it on the board as a task for the fleet (the directory must be a
registered project), so a chat that found the work hands it off; `/session`
shows the session id. `-p TEXT` sends a first message before reading the
rest. Every session is a store session like a run (`hive harness
sessions`, the Harness page, `hive harness cancel ID` stops its current
turn) with status `ended` when you leave, and its trajectory under
`~/.hive/harness/ID.json` is rewritten after every turn with the full
conversation, so a session survives a closed terminal: `hive harness chat
--resume ID` carries the conversation into a new session (the model is
told time has passed and files may have changed) and `--continue` picks
the newest session that ran in this directory. Sessions use the built-in
chat prompt, not `harness.system_prompt`; `--prompt-file PATH` overrides
it. `--bare` starts a plain agent - no instruction files, memory, skills,
custom agents, hooks or MCP servers (`--no-instructions --no-memory
--no-skills --no-agents --no-hooks --no-mcp` in one).

### Rewind

A turn that went the wrong way can be taken back. Before an
`edit` or `write` runs, the session keeps a copy of the file as it was
(the first copy per turn wins, a file the turn creates is remembered as
absent), and `/rewind` lists the turns with what each changed: `2. make
the test pass - 2 files: src/lib.rs, tests/it.rs`. `/rewind N` puts those
files back to how they were before turn N - a file a turn created is
removed, a directory left empty with it - and drops turns N onward from
the conversation, so the next message continues from the answer to turn
N-1 as if the rest had not been said; `/rewind N files` puts only the
files back and tells the model with your next message, so the
conversation goes on; `/rewind N conversation` drops only the messages.
What a shell command changed is not recorded - `git` is the rewind for
that. A rewound turn's copies are forgotten; after `/rewind N files` the
turn stays in the list as `no file changes`, so the numbers do not shift.
The copies live beside the trajectory in
`~/.hive/harness/ID.rewind/` and follow a session through `--resume`
(`resumed session ID: 6 messages carried over; 2 turns can still be
rewound`). After compaction the messages of the turns folded into the
summary cannot be dropped; their files can (`/rewind 1 files`). The
rewind is a `note` step in the trajectory, so a later reader sees it
happened.

**Permission rules** say what runs without asking and what may not run at
all, in the `tool` / `tool(filter)` grammar Claude Code and Copilot CLI
users know. The filter is a glob (`*` any text, `?` one character) matched
against the whole **subject** of the call: the command line for `bash`,
the path relative to the working directory for `read`, `grep`, `glob`,
`edit` and `write`, the host for `fetch`, the arguments as compact JSON
for an MCP tool (whose name may itself be a glob: `mcp__github__*`), the
skill's name for `skill`; a bare tool name covers every call of it. **Deny
beats allow beats the default**: a denied call is refused before anything
runs - also under `--yes`, also as a fenced block or in a parallel batch -
and the model is told (``bash: refused by permission rule `deny bash(rm
*)` ``); a tool denied whole (`deny write`) is not offered to the model at
all; an allowed call runs without approval, including a policy-line
crossing like `bash(git push *)`; everything else does what it would do
anyway. Rules come from `--allow-tool` / `--deny-tool` (repeatable or
comma-separated: `--deny-tool 'bash(rm *), read(.env*), write(*.lock)'`),
from the settings `harness.allow_tools` and `harness.deny_tools`
(Settings → Harness; comma- or newline-separated) and, in a session, from
`a` at an approval or `/permissions allow|deny PATTERN`. `harness.deny_tools`
reaches jobs too - the runner passes it as `--deny-tool`, so a rule like
`bash(curl *)` holds fleet-wide - on top of the job policy line (no push,
no remotes, no `gh`, no credential paths), which stays as it is; allow
rules change nothing in a job, which asks nobody.

## Providers, sign-in and models

```bash
hive harness login                     # device flow: open the URL, type the code (GitHub for copilot and github-models, Microsoft for azure-openai); for anthropic, Anthropic's own CLI opens the browser and there is no code; --with-token reads a key or token on stdin; --provider <name> picks the provider
hive harness status                    # harness, provider, who is signed in, the models and limits in force
hive harness models                    # the models the provider offers this login (id, vendor, tools, premium, context)
hive harness run -p 'Reply with SUMMARY: ok' -C ~/src/my-app --max-steps 3   # one run by hand; --json for the result, --task-file - reads the brief from stdin, --steps-file PATH streams JSONL events (- for stdout, which implies --json), --max-cost 2 --keep-observations 3 override the budgets (--max-steps none lifts the call cap), --context-window 1000000 the window context discipline assumes, --compact-at 60 the percent of it at which the run summarises its older messages, again each time it fills back up (0 never), --no-instructions skips the clone's AGENTS.md and rules, --no-mcp offers no MCP tools, --no-skills / --no-agents / --no-hooks / --no-subagents / --no-memory leave those out, --web-search / --no-web-search offer or withhold the web_search tool whatever harness.web_search says, --fetch-allow 'docs.rs,*.rust-lang.org' offers the fetch tool for these hosts, --deny-tool 'bash(rm *),read(.env*)' refuses matching calls (a tool denied whole is withdrawn), --bare leaves out instruction files, memory, skills, custom agents, hooks and MCP, --read-only makes it a run that looks but does not touch (what answer and ballot jobs get), --reasoning high asks the model for that reasoning effort
hive harness chat -C ~/src/my-app        # a conversation with the same agent: each line a turn, changes shown for approval (--yes approves all; y/n/a/all or a reason at each), replies stream; line editing, history and Tab completion, @PATH attaches a file, #123 attaches a GitHub issue or pull request through gh, !CMD runs a shell command the model also sees; --allow-tool 'bash(git *),edit(src/*)' runs those without asking, --deny-tool 'bash(rm *)' refuses those; --resume ID / --continue carry an earlier session on, -p TEXT sends a first message; /plan /permissions /procs /memory /rewind /diff /review /pr /export /env /init /delegate /new /sessions /help inside
hive harness sessions --project my-app # recent sessions; hive harness show ID prints one transcript
hive harness cancel ID                 # stop a running session between steps (exit 5; the job is held from recovery)
hive harness prompt                    # the rendered system prompt; --default the built-in template, --raw the stored one
hive harness logout                    # forget the stored login
```

### Sign-in and where the token lives

The default provider is GitHub Copilot: the
console's *Sign in with GitHub* (or `hive harness login`) runs GitHub's
device flow with the Copilot editor app, checks that the account actually has
Copilot, and stores the GitHub token in `~/.hive/private/providers/copilot.json`
(mode 0600, never echoed; `hive harness logout` removes it). That file wins.
Without it the harness borrows, in order: `COPILOT_GITHUB_TOKEN`, `GH_TOKEN`
or `GITHUB_TOKEN` from the environment; the login the Copilot editor plugins
keep in `~/.config/github-copilot/apps.json` (or `hosts.json`); the Copilot
CLI's config. It does not borrow `gh auth token`: the gh OAuth app is not
Copilot-enabled, so GitHub refuses those tokens at Copilot's door - the status
card says exactly that rather than "signed in". The GitHub token is exchanged
for a short-lived Copilot session token on each run and refreshed before it
expires; a `401` mid-run refreshes once, `429`/`5xx` retry with ±20% jittered
backoff. On `429`/`503`, `Retry-After` and `x-ratelimit-reset*` headers are
honored up to `provider.retry_after_cap_s`, logging `rate limited by <provider>;
waiting <n>s (header)`. Error bodies are capped for logs, and oversized success
bodies fail clearly (`response exceeded 8 MiB`). Every log line and error is
redacted of token shapes.

### Models

`worker_model`, `light_model` and `measure_model` (Settings →
Harness, or `hive settings set`) choose among the models the provider lists
for the signed-in account; empty means the provider's default chat model. The console shows
each model's vendor, tool-call support, premium multiplier and context window.
Beside each sits its reasoning effort (`worker_reasoning`, `light_reasoning`,
`measure_reasoning`), `harness.reasoning` while empty. A model that does not
take the chosen effort gets the nearest one it offers instead of failing the
run - a model that offers `low`, `high` and `max` runs an `xhigh` setting as
`high`, and a model without `max` runs a `max` setting as its `xhigh` or
`high` - and the run's log says so; the choice is kept for that model while
the harness runs.

### Only harness

The built-in harness is the only way Hive runs workers,
leads and measurements; nothing besides the `hive` binary is installed on a
machine that runs agents. The settings of the retired `copilot-cli` run mode
(`harness`, `runner_cmd`, `max_ai_credits`) are swept out of an older store
the first time a newer binary opens it, each one logged as a `setting.retired`
event, and `hive settings set` refuses them as unknown.

**Adding a provider** is one crate under `crates/hive-provider-<name>`
that implements `hive_provider::Provider` (`chat`, `models`, `status`) and,
optionally, `DeviceLogin`, `TokenLogin` and `Logouter`, then registers
itself with `hive_provider::register` - the harness, the CLI verbs, the hub routes
and the console's sign-in card pick it up through the `provider` setting.
A vendor that speaks the OpenAI chat completions API is not a crate but one
`Vendor` entry in `hive-provider-openai` (name, label, default endpoint, the
key and base-URL environment variables, whether a key is optional, how the
key travels, and which device login it offers, if any), which the crate's
single `register()` turns into a provider; a native API (Anthropic, Gemini)
gets its own crate. The shared pieces - the pasted-key file, the environment
fallback, the status card wording, logout - live in `hive_provider::apikey`,
and the RFC 8628 device code flow (begin, poll, refresh, the JWT helpers) in
`hive_provider::device`, so every key-based vendor behaves the same way and
a vendor with a device login stores it in the same file with its refresh
token and expiry.
A provider's own settings are named `provider.<name>.*`; `Store::provider_settings()`
hands all of them to `hive_provider::open` in one map. A provider hands the
harness one assistant message per call: `hive_provider::merge_choices`
folds every `choices[i]` of a chat-completion reply into it, so a gateway
that puts the narration in one choice and the tool call in the next (Copilot
does) still yields a reply the harness acts on instead of a prose-only turn
that costs a `format_retry`.

### Other providers

Every provider but Copilot takes an API key:
`hive settings set provider <name>`, then paste the key in the console's
Harness card or run `hive harness login --provider <name> --with-token`
(reads stdin) - it lands in `~/.hive/private/providers/<name>.json` (0600,
never echoed; `hive harness logout --provider <name>` removes it). Without
the file the provider borrows the key from its environment variable, so
`OPENAI_API_KEY` and the like work for one-off runs. Each provider calls the
endpoint in `provider.<name>.base_url` (Settings → **Model provider**); while
that keeps its default, its `*_BASE_URL` variable stands in when set. `hive
harness models --provider <name>` lists what the endpoint serves. None of
them reports a cost, so `harness.max_cost` does not apply to them - the step
and time budgets do.

To point a provider at a proxy, a gateway or a local server, set its key
(`hive settings set provider.<name>.base_url <url>`), or leave the setting
at its default and export the environment variable beside it:

| Setting | Default | Environment variable |
|---|---|---|
| `provider.anthropic.base_url` | `https://api.anthropic.com` | `ANTHROPIC_BASE_URL` |
| `provider.openai.base_url` | `https://api.openai.com/v1` | `OPENAI_BASE_URL` |
| `provider.gemini.base_url` | `https://generativelanguage.googleapis.com` | `GEMINI_BASE_URL` |
| `provider.openrouter.base_url` | `https://openrouter.ai/api/v1` | `OPENROUTER_BASE_URL` |
| `provider.groq.base_url` | `https://api.groq.com/openai/v1` | `GROQ_BASE_URL` |
| `provider.mistral.base_url` | `https://api.mistral.ai/v1` | `MISTRAL_BASE_URL` |
| `provider.deepseek.base_url` | `https://api.deepseek.com/v1` | `DEEPSEEK_BASE_URL` |
| `provider.xai.base_url` | `https://api.x.ai/v1` | `XAI_BASE_URL` |
| `provider.together.base_url` | `https://api.together.xyz/v1` | `TOGETHER_BASE_URL` |
| `provider.fireworks.base_url` | `https://api.fireworks.ai/inference/v1` | `FIREWORKS_BASE_URL` |
| `provider.cerebras.base_url` | `https://api.cerebras.ai/v1` | `CEREBRAS_BASE_URL` |
| `provider.github-models.base_url` | `https://models.github.ai/inference` | `GITHUB_MODELS_BASE_URL` |
| `provider.ollama.base_url` | `http://127.0.0.1:11434/v1` | `OLLAMA_HOST` or `OLLAMA_BASE_URL` |
| `provider.lmstudio.base_url` | `http://127.0.0.1:1234/v1` | `LMSTUDIO_BASE_URL` |
| `provider.vllm.base_url` | `http://127.0.0.1:8000/v1` | `VLLM_BASE_URL` |

The variable is read only while the setting keeps its default; a value set
in the store wins. `provider.azure-openai.base_url` and
`provider.custom.base_url` have no default and read `AZURE_OPENAI_ENDPOINT`
and `HIVE_CUSTOM_BASE_URL` while they are empty - both are described below.

Two of them also sign in with a device code, the way Copilot does, so no key
has to be minted or pasted: the Harness card shows *Sign in with GitHub* for
`github-models` (GitHub's device flow with the GitHub CLI's OAuth app; the
login is checked against the Models catalog and stored like a pasted token)
and *Sign in with Microsoft* for `azure-openai` (the Microsoft Entra device
code flow with the Azure CLI's public client in the tenant
`provider.azure-openai.tenant`, default `organizations`; the account needs
the *Cognitive Services OpenAI User* role on the resource). `hive harness
login --provider github-models|azure-openai` is the same flow in a terminal.
A Microsoft sign-in stores its refresh token with the access token and
renews itself about five minutes before expiry, so workers never see an
expired bearer; a refresh that fails flips the card to "sign in again" and
the run stops with `not_logged_in`. A pasted token shaped like a JWT (for
instance `az account get-access-token --resource
https://cognitiveservices.azure.com --query accessToken -o tsv`) goes out as
a bearer token too, a plain key in the `api-key` header. Only providers whose
terms allow a third-party sign-in get one: Anthropic's consumer (claude.ai)
OAuth, OpenAI's consumer OAuth, Google's and OpenRouter's browser-callback
flows are deliberately not wired up, so those stay on API keys - except for
the Anthropic Console sign-in below, which goes through Anthropic's own CLI.

The OpenAI-compatible vendors all speak the chat completions API with the
same one bash tool and all live in `hive-provider-openai`: `openai`
(`OPENAI_API_KEY`, `OPENAI_BASE_URL`, default `https://api.openai.com/v1`),
`azure-openai` (`AZURE_OPENAI_API_KEY` or *Sign in with Microsoft*,
`AZURE_OPENAI_ENDPOINT`; no default - set `provider.azure-openai.base_url` to
`https://<resource>.openai.azure.com/openai/v1`, a bare resource root gets
`/openai/v1` added; a key travels in the `api-key` header, a sign-in as a
bearer token; `AZURE_OPENAI_TENANT_ID`, `AZURE_OPENAI_CLIENT_ID`,
`AZURE_OPENAI_AUTHORITY` and `AZURE_OPENAI_SCOPE` reshape the sign-in for a
sovereign cloud or your own app registration),
`openrouter` (`OPENROUTER_API_KEY`), `groq` (`GROQ_API_KEY`), `mistral`
(`MISTRAL_API_KEY`), `deepseek` (`DEEPSEEK_API_KEY`), `xai` (`XAI_API_KEY`),
`together` (`TOGETHER_API_KEY`), `fireworks` (`FIREWORKS_API_KEY`),
`cerebras` (`CEREBRAS_API_KEY`), `github-models` (*Sign in with GitHub*, or
a GitHub personal access token with the `models:read` scope:
`GITHUB_MODELS_TOKEN`, `GH_TOKEN` or `GITHUB_TOKEN`;
`https://models.github.ai/inference`), and the local servers
`ollama` (`http://127.0.0.1:11434/v1`; `OLLAMA_HOST` or `OLLAMA_BASE_URL`),
`lmstudio` (`http://127.0.0.1:1234/v1`) and `vllm` (`http://127.0.0.1:8000/v1`),
which need no key - their status card says `no key needed` - though one in
`OLLAMA_API_KEY`, `LMSTUDIO_API_KEY` or `VLLM_API_KEY` is sent when present.
Every other base-URL variable is `<VENDOR>_BASE_URL` (`GROQ_BASE_URL`,
`MISTRAL_BASE_URL`, …). Any other OpenAI-compatible server - a gateway, a
proxy, a hosted model none of the names above covers - is the `custom`
vendor, described by settings alone: `hive settings set
provider.custom.base_url https://<host>/v1` (or `HIVE_CUSTOM_BASE_URL`;
there is no default, and a call without one fails before it is made), a key
pasted in the Harness card, given to `hive harness login --provider custom
--with-token` or read from `HIVE_CUSTOM_API_KEY`; `provider.custom.no_key
on` makes it keyless like the local servers, and
`provider.custom.api_key_header x-api-key` sends the raw key in that header
instead of `Authorization: Bearer`. Pick it like any vendor, `provider
custom` in the settings or `--provider custom`, and name its models as the
server lists them. Most of these endpoints report no context window, so
with them context discipline stays at its first level (collapsed, never
one-liners).

`anthropic` (Anthropic Claude; *Sign in with Anthropic*, a pasted Console
API key, `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL`,
default `https://api.anthropic.com`; crate `hive-provider-anthropic`) speaks
the native Messages API: the bash tool goes out as `tool_use` and its output
comes back as `tool_result`, and `max_tokens` defaults to 8192. `gemini`
(Google Gemini; `GEMINI_API_KEY` or `GOOGLE_API_KEY`, `GEMINI_BASE_URL`,
default `https://generativelanguage.googleapis.com`; crate
`hive-provider-gemini`) speaks the native `generateContent` API with
`functionCall` / `functionResponse`, and sanitizes the tool schemas to the
OpenAPI subset Gemini accepts. Both report their models' context windows
from the model list - Anthropic's `max_input_tokens` and `max_tokens` (a
million tokens for Claude Sonnet 4.6 and later, Opus 4.6 and later, Haiku
5.5, Fable and Mythos; 200k for the older models; retired models are left
out), Gemini's `inputTokenLimit` (a million) - so context discipline
tightens past 80% of it exactly as it does with Copilot, and
`harness.context_window` overrides the figure when a proxy lists none.

### Signing in with Anthropic

*Sign in with Anthropic* (the Harness card, or `hive harness login
--provider anthropic`) is the Anthropic Console's OAuth sign-in, run by
Anthropic's own CLI, `ant` (`brew install anthropics/tap/ant`, or a release
from github.com/anthropics/anthropic-cli). Hive starts `ant auth login`,
which opens the browser on the machine Hive runs on and finishes by itself
once you approve; there is no code to type, and the console notices on its
own. `ant` keeps the OAuth profile under `~/.config/anthropic`; Hive keeps
only a marker, `~/.hive/private/providers/anthropic-signin.json` (the
profile - `ANTHROPIC_PROFILE` at sign-in, empty for the active one - and a
label, never a token). When a run needs a token Hive asks `ant auth
print-credentials --access-token`, which refreshes the profile itself, keeps
the answer in memory for four minutes and sends it as `Authorization: Bearer`
with `anthropic-beta: oauth-2025-04-20`. A token refused mid-run is fetched
again once; when `ant` can no longer hand one out, runs wait for a sign-in
like any provider without a login and the card says "The Anthropic sign-in
ended. Sign in again." The binary is found through `HIVE_ANT_BIN`, `PATH`,
`~/.local/bin`, `/opt/homebrew/bin` and `/usr/local/bin`, since a daemon
started by launchd runs with a thin `PATH`.

Usage bills to the Console organisation and workspace you signed in to, like
an API key from it. The order is: a pasted key, the sign-in,
`ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`; signing in removes a pasted key
and pasting a key removes the sign-in. *Sign out* forgets the marker and
leaves `ant`'s profile alone, because Claude Code and the SDKs share it (`ant
auth logout` ends it everywhere). A sign-in is never exported to another
machine - sign in there too.

A Claude Pro or Max subscription sign-in is not offered: Anthropic allows
claude.ai logins only inside its own apps (Claude Code, claude.ai) and does
not let third-party tools use them. A pasted `sk-ant-oat…` token - what
`claude setup-token` prints - is turned away before any request with a
sentence saying so.
