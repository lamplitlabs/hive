# Before/after reports

When an agent's change has a face - a page, a view, a stylesheet, a native
screen - the owner should not have to build the branch to see it. The agent
shows it instead: a picture of each changed screen as it was before the
change and one as it is after, a "Was" and a "Now" line for each, the checks
it ran and where the work stands, on one page the hub serves beside the job
or the session. Hive builds that page itself (`hive report`), so every
agent's report reads the same, and `landing.ui_report` decides what a change
without one costs.

Every other piece of work gets a report too: once a job or a session is
done, Hive builds the same page from what it knows of the work (see
[Hive's report](#hives-report-for-every-job-and-session)), and the console
shows it to the owner by itself.

What the agent leaves in its folder, step by step, and how it reaches the
owner:

```text
 the agent                            in $HIVE_ARTIFACTS

┌──────────────────────┐             ┌───────────────────────┐
│ pictures each screen │ ──────────→ │ before-<screen>.png   │
├──────────────────────┤             │                       │
│ makes the change     │             │                       │
├──────────────────────┤             │                       │
│ pictures them again  │ ──────────→ │ after-<screen>.png    │
├──────────────────────┤             │                       │
│ writes the manifest  │ ──────────→ │ report.json           │
├──────────────────────┤             │                       │
│ runs hive report     │ ──────────→ │ report.html           │
└──────────────────────┘             └───────────┬───────────┘
                       the hub adds Hive's part  │
                       and serves the page       │
                                                 ↓
                                     ┌───────────────────────┐
                                     │ the owner opens       │
                                     │ See the report        │
                                     └───────────────────────┘
```

## What the agent does

Rule 10 of a worker's brief, and the same paragraph of a session's brief,
ask for it in this order:

1. **Before** changing a screen, picture it as it is: run the project's dev
   server or test harness and save `before-<screen>.png` in
   `$HIVE_ARTIFACTS` at 1440x900, plus 390x844 for a screen people open on
   a phone, plus the dark theme when the change touches colour.
2. **After** the change, the same screens the same way as
   `after-<screen>.png`.
3. **`report.json`** beside them, then **`hive report`**, which builds
   `report.html` there and names anything missing; the agent runs it until
   it names nothing and names `report.html` in its summary.

`hive report --example` prints the manifest's shape:

```json
{
  "title": "Sidebar fits the owner's name",
  "summary": "What changed for the person looking at the screen, and why.",
  "changes": [
    {
      "area": "Sidebar",
      "title": "Owner row",
      "was": "The name was cut to \"Mani...\" next to the presence switch.",
      "now": "The full name shows; the switch sits after it.",
      "before": "before-sidebar.png",
      "after": "after-sidebar.png",
      "pairs": [
        { "label": "Phone", "before": "before-sidebar-phone.png", "after": "after-sidebar-phone.png" }
      ]
    }
  ],
  "checks": ["`npm test`: 516 passed"],
  "status": "Committed on the job's branch; it lands at the next landing pass.",
  "notes": ["Changes that are not pictured, one line each."]
}
```

A change names one pair (`before`, `after`) and any number of labelled
`pairs` more. Pictures are plain file names in the same folder (the hub
serves no subfolders), and they are read by their first bytes: PNG, JPEG,
WebP, GIF or AVIF. A field the manifest does not know is an error that names
it. Two pictures that are byte for byte the same are shown with a note that
the screen did not change. `hive report --check` says what the report shows
without writing the page, `--dir` points it at another folder, and `--json`
prints the result for a script. The command needs no store, so it runs on a
remote hand too.

The page needs no script and no font from elsewhere: the hub serves an
agent's HTML sandboxed (`Content-Security-Policy: sandbox allow-scripts`), and
the pictures load from the same folder by their names. Opening a picture
shows it full size; going back returns to the page.

## What a change without one costs

`landing.ui_report` (default `require`) is set for the machine, and for one
project with `landing.ui_report.<slug>` (`hive settings set
landing.ui_report.web warn`); forgetting a project removes its setting. A
change counts as having a face when the branch touches a `.tsx`, `.jsx`,
`.vue`, `.svelte`, `.astro`, `.html` or stylesheet file, an Android layout,
an iOS storyboard or xib, or a Dart, Kotlin, Swift or Java screen, view,
page, activity or fragment; tests and built bundles do not count. A report
shows the change when at least one pair has both pictures in the folder.

Under `require`, on a project with an upstream, a change with a face and no
report goes this way:

```text
                  ┌────────────────────────┐
                  │ UI change, no report   │
                  └────────────┬───────────┘
             ┌─────────────────┴─────────────────┐
             │ a worker's job                    │ a session's branch
             ↓                                   ↓
┌────────────────────────┐          ┌────────────────────────┐
│ one more turn,         │          │ the session is woken,  │
│ up to 20 minutes       │          │ waits up to an hour    │
└────────────┬───────────┘          └────────────┬───────────┘
             └─────────────────┬─────────────────┘
                               ↓
                  ┌────────────────────────┐
                  │ the pull request opens │
                  └────────────┬───────────┘
             ┌─────────────────┴─────────────────┐
             │ shows the change                  │ still no report
             ↓                                   ↓
  reviewed like any other                Needs your review
```

| | `require` | `warn` | `off` |
| --- | --- | --- | --- |
| The brief | asks for the report | asks for the report | asks for a screenshot or a report, as before |
| A worker that finished without one | gets one more turn of its session, for the report alone (up to 20 minutes; its branch is already taken, so nothing it commits then lands) | - | - |
| A local landing (no upstream) | holds the branch, looks again every 30 minutes and lands it once the report shows the change; you are asked once | lands, and the landing post says it has no report | lands |
| A pull request | is yours to merge: the review's reasons name the missing report ([The complexity rule](review.md#the-complexity-rule)) | the reviewer merges it as usual | the reviewer merges it as usual |
| A session's branch | its pull request waits: the session is woken once with what to do, and the pull request opens when the report shows the change, or after an hour for you to merge; `hive land --branch` from the session's agent says the same and proposes nothing | opens as usual | opens as usual |

Where nothing can be shown, nothing waits: a job that ran on a remote hand
(no folder the hub serves; its summary describes the screen instead), an
experiment's `evolve/` branch, a branch you land or propose by hand
(`hive land --project <slug> --branch <branch>` from your own shell), and an
archived session.

## Hive's report for every job and session

`report.html` in the work's folder is Hive's page of the whole piece of
work, built by the hub (`crates/hive-runner/src/work_report.rs`, the page in
`crates/hive-harness/src/report.rs` `build_full`):

- **Where it stands**, first and in words: merged as pull request #N, landed
  on the default branch, in review, held and why, done with nothing landed,
  failed, or - for a session - stopped by a restart.
- **What it did**: the agent's `report.json` title and summary when there is
  one; otherwise the task's or session's title and the worker's summary
  (without its `done -` outcome code) or the session's last word.
- **Who and how**: the agent, the model, the steps, how long it took, the
  cost, the job or session id.
- **The before/after pairs** of `report.json`, as above, and **every other
  picture** in the folder under Screenshots.
- **What changed**: the commits - a job's are the ones that carry its
  `Hive-Job:` trailer, so the list holds after the branch is gone; a
  session's come from its branch or its pull request's merge - and the files
  they changed with the lines added and removed.
- **The checks**: the agent's, and the verify step the landing ran.
- **For a session**: its sub-sessions and how each ended, and the tasks it
  filed and where they stand.

The hub builds the page the first time a finished job's folder or a finished
session is listed without one, and again each time the page is opened, so it
says where the work stands now rather than where it stood. An agent's own
`hive report` writes the same file without Hive's part; the next opening
puts it back.

## Where the owner sees it

- A done task opens on its report: the canvas shows the page of the step that
  did the work (the finished step whose run made the most commits), and the
  task's "done" line says **See the report** and **Open it in a new tab**. A
  done task's card on the board offers **Report** where a working one offers
  **Session**.
- A finished session opens its report on the canvas too, and its "finished"
  line links it the same way.
- A step's files on the canvas open on the report first.

- A run's result post has a `- Report:` line linking the page, or saying
  there is none and what that costs; the landing post ends with a link to it.
  Links to files the hub serves open in a new tab.
- **See the report** sits under a worker's report in a task, beside its
  evidence files; above a session's **Artifacts** list; and in a run's panel
  on the **Runs** page.
- The page itself lists the files of the job's or the session's folder like
  any other artifact, so it also opens on the canvas.

The folder is `HIVE_HOME/work/jobs/<job>/artifacts` for a job and
`HIVE_HOME/work/chat/<id>/artifacts` for a session; a job's folder is kept
`daemon.artifact_keep_days` after the job finishes.
