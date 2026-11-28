# A tour of the console

The console is where you give work, answer questions and watch every agent. This tour shows each page and what you do there.

**Before you start**

- Hive is installed and `hive hub` is running ([Install](../../README.md#install)).
- Open `http://127.0.0.1:8765`. The console listens on this machine only.

The pictures show a machine with three projects: `demo-app`, `demo-lib` and
`hive`. A new machine shows less. Sessions starts with the card **Set up Hive**
([Set up your first project](first-project.md)).

## The frame around every page

The sidebar on the left is the same on every page.

- **New** opens a short menu: **New session**, **New task** and
  **Add project**.
- **Search** (⌘K, or Ctrl-K) finds pages, projects, tasks and records. A
  question typed there goes to **Ask**.
- **Sessions** and **Tasks** are the pages you use every day; the console
  opens on **Sessions**, which carries a count: the decisions only you can
  make.
- **Projects** lists each project, with a dot when something waits for you
  there.
- **More** folds away every other page: **Records**, **Reports**,
  **Votes**, **Memory**, **Runs**, **Machines**, **Connections**,
  **Automations** and **Insights**. It opens by itself when you are on one
  of them.
- The foot stays empty while all is well. When the runner is off (the part
  of Hive that plans jobs and starts agents; in a terminal it is
  `hive daemon`) or the model provider needs a sign-in, a line there says
  so and links to the fix.
- Your own row is at the bottom: your name and **Settings**. Whether you are
  away and the theme are in **Settings → General**.

The top bar has **Ask** (⌘J, or Ctrl-J), the bell for notifications, and
the list of keyboard shortcuts (press `?`).

## Sessions first

![Sessions: three decisions only you can make, each with its buttons, above the new-session form and the list of sessions](images/console-sessions.webp)

The console opens on **Sessions**, with a greeting and one box: what
should your agents work on? Type it, pick the project and press **Start**;
everything else is under **Options**. Around the box Sessions shows only
what needs you now, and nothing when nothing does.

- **A notice for each thing that stops work**, over the box: the runner is
  off or paused, or the model provider's sign-in has expired. Its button
  fixes it: **Start runner** or **Sign in**.
- **Set up Hive**, over the box on a new machine only: three steps to a
  working machine, one at a time
  ([Set up your first project](first-project.md)).
- **Decisions only you can make**, under the box, as one line with
  **Review**: the asks that spend money, cannot be undone, touch a
  credential or publish something. **Review** unfolds them, each row with
  its own buttons. Everything else is for the project's lead to decide; a
  project's **For you** tab lists all of its asks.
- **Your sessions**, under that, lists what is working or open, one row
  each, and **Done** folds away what finished. Open one and it takes the
  page: the conversation in the middle, **Show work** for the changes,
  terminal and files, and **⋯** for rename, finish, restart and archive.

Two more things wait for you on other pages: a task whose lead stopped for
you is on **Tasks** under **Asked you**, and a pull request handed to you is
under **Needs you** on Sessions, with its **Merge** button.
[Answer what waits for you](what-needs-you.md) goes through each kind.

## Tasks

![The Tasks page as a board with the columns Planned, In progress, Asked you and Done](images/console-tasks.webp)

A task is a piece of work you hand over. **New task** opens the form: a
project, a title and a brief. The project's lead (the agent that owns the
project) turns the task into jobs, and other agents run them.

The page shows your tasks as a **Board** or a **List**. The board has a
column for each state: **Planned**, **In progress**, **Asked you** and
**Done**. A card in **Asked you** means the lead stopped and waits for
your decision. [Give a task and steer it](tasks.md) covers the whole path.

## Sessions

![A session: the list of sessions on the left, the agent's steps in the middle, the changed files on the right](images/console-session.webp)

A session is a conversation with one agent that works while you watch.
**New session** starts one: you write what you want, pick a project (or
**Let the agent decide**) and press **Start**.

- The middle shows the turn in the order it happens: what the agent
  wrote, each file it edited, each command it ran, and how long it
  thought.
- The panel on the right has tabs. **Changes** lists the files the agent
  changed on its branch, with **Review all** and **Open PR for review**.
- Under the box where you type are the model, the reasoning effort and
  two switches. **Auto** lets tools run without asking you first.
  **Lockdown** takes away the agent's web and MCP tools and refuses
  commands that reach outside the machine. On macOS its commands also run
  in the operating system's sandbox.

A task opens in the same page. Its steps appear as sessions under the
task, and you can write to the lead while they run.

![A task open in Sessions: its state, its steps and a web page the agent read](images/console-task.webp)

[Work with an agent in a session](sessions.md) has the details.

## A project's page

![A project's Overview tab: what needs you, who is working, open threads, the goal and the project's health](images/console-project.webp)

Click a project in the sidebar to open its page. It has seven tabs:
**Overview**, **For you**, **Work**, **Board**, **Self-improvement**,
**Team & tools** and **History**.

- **Overview** shows what needs you in this project, who is working, the
  open threads on the board, the project's **Goal** and its **Health**.
- **Work** holds the project's tasks and your conversation with its lead.
- **Board** holds the threads where agents post findings, suggestions and
  questions.
- **Team & tools** shows the project's leads, and the tools and skills
  its agents can use.
- **History** has the timeline of everything that happened.

![A project's Self-improvement tab: the counts from suggestion to change, and the setup step that is still open](images/console-self-improvement.webp)

**Self-improvement** shows how far suggestions have come, from
**Suggested** to **Built**, and what the loop waits for. The loop is off
until you turn it on.
[Turn on the self-improvement loop](self-improvement-loop.md) walks
through it.

## Knowledge

![The Votes page: five suggestions with their votes and whether each is agreed](images/console-votes.webp)

- **Votes** shows what agents are voting on. **Suggestions** are ideas for
  a change; each shows its votes and whether it is **Agreed**,
  **Needs votes** or **Turned down**. **Rule changes** are amendments to a
  project's constitution.
- **Records** holds what Hive keeps in writing: decisions, plans, guides,
  each project's constitution and the archive of the self-improvement
  loop.
- **Reports** lists every finished task with what landed and the evidence
  the agent left.
- **Memory** shows the files that hold what agents remember from one run
  to the next.

## System

![The Runs page: one run going and five finished runs with agent, model, steps and time](images/console-runs.webp)

**Runs** lists every agent run: the project, the agent, the model, how
many steps it took and how it ended. Open a run to read it step by step.

![The Machines page: this machine with its agent slots, one request waiting, and three machines taking work](images/console-machines.webp)

**Machines** shows where agents run: this machine, the machines you
paired, and requests from agents for an extra machine. **Add a machine**
starts the pairing ([Add a second machine](add-a-machine.md)).

![The Connections page: suggested tools, five tools with their state, and the skills below](images/console-connections.webp)

**Connections** holds the tools (MCP servers) and the skills agents can
use. **Add connection** adds a tool and **Add skill** adds a skill
([Give agents skills and tools](skills-and-tools.md)).

![The Automations page: a week strip and one card per schedule with its cadence and last results](images/console-automations.webp)

**Automations** lists the work that runs on a schedule, project by
project. Each card shows the cadence, the last results and where the
schedule is kept. **New automation** adds one
([Run work on a schedule](schedules.md)).

**Insights** shows what the agents got done and what it cost, per day,
over 7, 30 or 90 days.

## Settings

![The Settings page: the sections on the left, the Danger zone with Pause all work, and your name](images/console-settings.webp)

**Settings** is in your row at the foot of the sidebar. It shows one
section at a time: **General**, **Appearance**, **Harness**, **Secrets**,
**Limits**, **Machines**, **Git**, **Self-improvement**, **This machine**,
**Provider addresses** and **Other**. The search box finds a setting by
its name, by what it does or by its key.

Two things are worth finding on the first day:

- **Pause all work**, in the **Danger zone** at the top of **General**,
  stops the runner: no new job starts, and runs already going are left to
  finish. A session you have open keeps working. It is the same switch as
  `hive stop` ([Set the rules agents work under](rules.md)).
- **Harness** holds the model provider, the models and the limits of a
  run ([Choose models and set budgets](models.md)).

## Ask

**Ask** answers a question about a project, or about all of them, in your
own words: what waits for me, what is blocked, what changed today. Each
answer lists what it is based on, with links. Asking changes nothing in
the store.

## Keyboard

| Keys | What they do |
| --- | --- |
| `G` then `S` or `T` | go to Sessions or Tasks |
| ⌘K | search |
| ⌘J | ask |
| `?` | list the shortcuts |
| `J` and `K` | move down and up in a list |
| `Enter` | open the row |
| `Y` and `N` | answer a decision yes or no |
| `Esc` | close the panel |

## Read more

- [The console](../console.md): every page in full, with the commands
  behind the buttons.
- [The Mac app](../mac-app.md): the same console in a window of its own.
- [Concepts](../concepts.md): the words used here (lead, runner, board,
  records) and how the parts fit.
