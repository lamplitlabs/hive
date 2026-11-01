//! The `task` tool: an agent files work for the fleet on PULSE's board,
//! the way the owner does with `pulse task new`. A hub conversation's
//! agent, a job's worker or an RSI run that finds work bigger than its own
//! turn - a feature to build, a module to fix, a slice another project
//! needs - files a task; the project's lead plans it on its next pass, as
//! it plans the owner's. The task remembers who filed it and from where
//! (`created_by`, `origin`), so the board and the lead can tell an agent's
//! task from the owner's.
//!
//! The guard rails live in [`pulse_tasks::tasks::file`]: an agent's task
//! needs a brief, may not repeat an open title, and an author may have
//! [`pulse_tasks::tasks::AGENT_OPEN_CAP`] open per project. The tool adds
//! what only the run knows: the actor's name, the origin, the project the
//! run works in, and the store.
use pulse_provider::Tool;
use pulse_store::Store;
use pulse_tasks::tasks::{self, Origin, Task};
use serde_json::{Value, json};
use std::fmt::Write as _;

/// The tool's name.
pub const TASK_TOOL: &str = "task";

/// How many characters of a brief the `show` action returns.
const SHOW_BRIEF_LIMIT: usize = 4000;
/// How many tasks `list` returns at most.
const LIST_LIMIT: usize = 50;

/// What a run needs to file tasks: the store, who files, from where, and
/// the project the run works in (empty when it has none: then every call
/// names its project).
#[derive(Clone)]
pub struct TaskDesk {
    store: Store,
    actor: String,
    origin: Origin,
    project: String,
}

impl TaskDesk {
    pub fn new(
        store: Store,
        actor: impl Into<String>,
        origin: Origin,
        project: impl Into<String>,
    ) -> Self {
        TaskDesk {
            store,
            actor: actor.into(),
            origin,
            project: project.into(),
        }
    }

    /// Who the desk files as.
    pub fn actor(&self) -> &str {
        &self.actor
    }

    /// The project a call without one files into; empty when there is none.
    pub fn project(&self) -> &str {
        &self.project
    }

    /// The origin the desk records on every task it files.
    pub fn origin(&self) -> &Origin {
        &self.origin
    }

    /// Runs one parsed call and returns the text the model reads, or the
    /// error it reads instead.
    pub fn run(&self, action: &TaskAction) -> Result<String, String> {
        match action {
            TaskAction::Create {
                project,
                title,
                brief,
                priority,
            } => {
                let project = self.project_for(project)?;
                let t = tasks::file(
                    &self.store,
                    &project,
                    title,
                    brief,
                    *priority,
                    &self.actor,
                    &self.origin,
                )
                .map_err(|e| e.to_string())?;
                Ok(format!(
                    "{} is on the board for {}: {}\n  filed by {} (origin {}); the project's lead plans it on its next pass. `pulse task show {}` follows it; the task tool's show action does too.",
                    t.id,
                    t.project,
                    t.title,
                    t.created_by,
                    if t.origin.is_empty() {
                        "owner"
                    } else {
                        &t.origin
                    },
                    t.id
                ))
            }
            TaskAction::List { project, status } => {
                let project = if project.is_empty() {
                    self.project.clone()
                } else {
                    project.clone()
                };
                let list = tasks::list(&self.store, &project, status).map_err(|e| e.to_string())?;
                let scope = match (project.is_empty(), status.is_empty()) {
                    (true, true) => "every project".to_string(),
                    (true, false) => format!("every project, status {status}"),
                    (false, true) => project.clone(),
                    (false, false) => format!("{project}, status {status}"),
                };
                if list.is_empty() {
                    return Ok(format!("No tasks in {scope}."));
                }
                let mut out = format!("{} in {scope}", plural(list.len(), "task", "tasks"));
                if list.len() > LIST_LIMIT {
                    let _ = write!(out, " (first {LIST_LIMIT})");
                }
                out.push('\n');
                for t in list.iter().take(LIST_LIMIT) {
                    let _ = writeln!(out, "{}", list_line(t));
                }
                Ok(out.trim_end().to_string())
            }
            TaskAction::Show { id } => {
                let (t, revs) = tasks::get(&self.store, id).map_err(|e| e.to_string())?;
                let jobs = tasks::jobs(&self.store, id).map_err(|e| e.to_string())?;
                let messages = pulse_tasks::messages(&self.store, "", id, "", "")
                    .map_err(|e| e.to_string())?;
                let mut out = format!("{} {} {}\n", t.id, t.status, t.title);
                let _ = writeln!(
                    out,
                    "project {}, priority {}, rev {}, by {}{}, created {}, updated {}",
                    t.project,
                    t.priority,
                    t.current_rev,
                    t.created_by,
                    if t.origin.is_empty() {
                        String::new()
                    } else {
                        format!(" (origin {})", t.origin)
                    },
                    t.created_at,
                    t.updated_at
                );
                if !t.lead.is_empty() || !t.job_id.is_empty() {
                    let _ = writeln!(out, "lead {}, job {}", t.lead, t.job_id);
                }
                let brief = t.brief.trim();
                if !brief.is_empty() {
                    out.push('\n');
                    if brief.len() > SHOW_BRIEF_LIMIT {
                        out.push_str(&brief[..brief.floor_char_boundary(SHOW_BRIEF_LIMIT)]);
                        let _ = write!(out, "\n[brief cut at {SHOW_BRIEF_LIMIT} characters]");
                    } else {
                        out.push_str(brief);
                    }
                    out.push('\n');
                }
                if revs.len() > 1 {
                    let _ = writeln!(
                        out,
                        "\n{} (latest first):",
                        plural(revs.len(), "revision", "revisions")
                    );
                    for r in revs.iter().rev() {
                        let _ = writeln!(
                            out,
                            "  rev {} {} by {}{}: {}",
                            r.rev,
                            r.at,
                            r.author,
                            if r.note.is_empty() {
                                String::new()
                            } else {
                                format!(" ({})", r.note)
                            },
                            r.title
                        );
                    }
                }
                if !jobs.is_empty() {
                    let _ = writeln!(out, "\n{}:", plural(jobs.len(), "job", "jobs"));
                    for j in &jobs {
                        let _ = writeln!(
                            out,
                            "  {} {} {}{}{}",
                            j.id,
                            j.status,
                            j.title,
                            if j.agent.is_empty() {
                                String::new()
                            } else {
                                format!(" ({})", j.agent)
                            },
                            if j.result.trim().is_empty() {
                                String::new()
                            } else {
                                format!(": {}", crate::harness::one_line(&j.result, 200))
                            }
                        );
                    }
                }
                if !messages.is_empty() {
                    let _ = writeln!(
                        out,
                        "\n{} on its thread:",
                        plural(messages.len(), "message", "messages")
                    );
                    for m in &messages {
                        let _ = writeln!(
                            out,
                            "  {} {}: {}",
                            m.at,
                            m.author,
                            crate::harness::one_line(&m.body, 300)
                        );
                    }
                }
                Ok(out.trim_end().to_string())
            }
        }
    }

    fn project_for(&self, given: &str) -> Result<String, String> {
        if !given.is_empty() {
            return Ok(given.to_string());
        }
        if !self.project.is_empty() {
            return Ok(self.project.clone());
        }
        let known = self
            .store
            .projects(true)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|p| p.slug)
            .collect::<Vec<_>>();
        Err(if known.is_empty() {
            "this run has no project and none is registered; a task belongs to one project"
                .to_string()
        } else {
            format!(
                "this run has no project; say which one the task is for (project: one of {})",
                known.join(", ")
            )
        })
    }
}

fn list_line(t: &Task) -> String {
    let mut line = format!("- {} {} p{} {}", t.id, t.status, t.priority, t.title);
    let mut tail = Vec::new();
    if !t.lead.is_empty() {
        tail.push(format!("lead {}", t.lead));
    }
    tail.push(format!("by {}", t.created_by));
    if !t.origin.is_empty() {
        tail.push(format!("origin {}", t.origin));
    }
    tail.push(format!("updated {}", t.updated_at));
    let _ = write!(line, " ({})", tail.join(", "));
    line
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The `task` tool as the model sees it. `project` names the run's own
/// project (the default of `create` and `list`) when it has one.
pub fn task_tool(project: &str) -> Tool {
    let default_project = if project.is_empty() {
        "This run has no project of its own, so create needs project.".to_string()
    } else {
        format!("Without project, create and list use this run's project, {project}.")
    };
    Tool {
        name: TASK_TOOL.to_string(),
        description: format!(
            "File work for the fleet on PULSE's board, the way the owner does: a task the project's lead plans into jobs on its next pass. Use it for work bigger than this turn or belonging to another project - a feature to build, a module to fix, a follow-up the owner should not have to type - not for what you can do here now (the subagent and chat tools are for that). Give a title that names the outcome and a brief that says what to do, why (the failing check, the hand step, the promise the README makes), and how the work is proved; the lead reads nothing else. An open task with the same title is refused and named - add to it instead; you may have {} open per project. The task records you as its author and this run as its origin, and the owner sees it on the board. list shows a project's tasks (status: open, planned, in_progress, review, done, cancelled); show follows one. {default_project}",
            tasks::AGENT_OPEN_CAP
        ),
        parameters: json!({"type":"object","properties":{
            "action":{"type":"string","enum":["create","list","show"],"description":"create files a task; list shows tasks; show reads one with its revisions, jobs and thread."},
            "title":{"type":"string","description":"create: the outcome, in one line (under 200 characters)."},
            "brief":{"type":"string","description":"create: what to do, why, and how it is proved - the lead plans from this alone."},
            "project":{"type":"string","description":"create, list: the project slug (default: this run's project)."},
            "priority":{"type":"integer","description":"create: higher is sooner; 0 is the default."},
            "status":{"type":"string","description":"list: only tasks in this status."},
            "id":{"type":"string","description":"show: the task id (TASK-001)."}
        },"required":["action"]}),
    }
}

/// A parsed `task` call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskAction {
    Create {
        project: String,
        title: String,
        brief: String,
        priority: i32,
    },
    List {
        project: String,
        status: String,
    },
    Show {
        id: String,
    },
}

impl TaskAction {
    pub fn parse(arguments: &str) -> Result<TaskAction, String> {
        let args: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
        if !args.is_object() {
            return Err("arguments must be a JSON object with an action".into());
        }
        let str_of = |key: &str| -> Result<String, String> {
            match args.get(key) {
                None | Some(Value::Null) => Ok(String::new()),
                Some(Value::String(s)) => Ok(s.trim().to_string()),
                Some(Value::Number(n)) => Ok(n.to_string()),
                Some(other) => Err(format!(
                    "argument {key} must be a string, not {}",
                    match other {
                        Value::Bool(_) => "a boolean",
                        Value::Array(_) => "an array",
                        _ => "an object",
                    }
                )),
            }
        };
        let action = str_of("action")?.to_ascii_lowercase();
        let project = str_of("project")?;
        match action.as_str() {
            "create" | "new" | "file" | "add" => {
                let title = str_of("title")?;
                if title.is_empty() {
                    return Err("create needs a title".into());
                }
                let brief = match args.get("brief") {
                    None | Some(Value::Null) => String::new(),
                    Some(Value::String(s)) => s.trim().to_string(),
                    Some(_) => return Err("argument brief must be a string".into()),
                };
                let priority = match args.get("priority") {
                    None | Some(Value::Null) => 0,
                    Some(Value::Number(n)) => n
                        .as_i64()
                        .ok_or_else(|| "argument priority must be a whole number".to_string())?
                        .clamp(i64::from(i32::MIN), i64::from(i32::MAX))
                        as i32,
                    Some(Value::String(s)) => s.trim().parse::<i32>().map_err(|_| {
                        format!("argument priority must be a whole number, not {s:?}")
                    })?,
                    Some(_) => return Err("argument priority must be a whole number".into()),
                };
                Ok(TaskAction::Create {
                    project,
                    title,
                    brief,
                    priority,
                })
            }
            "list" | "ls" => {
                let status = str_of("status")?.to_ascii_lowercase();
                if !status.is_empty() && !tasks::STATUSES.contains(&status.as_str()) {
                    return Err(format!(
                        "unknown status {status:?}; statuses are {}",
                        tasks::STATUSES.join(", ")
                    ));
                }
                Ok(TaskAction::List { project, status })
            }
            "show" | "get" | "read" => {
                let id = str_of("id")?;
                if id.is_empty() {
                    return Err("show needs id, the task's id".into());
                }
                Ok(TaskAction::Show { id })
            }
            "" => Err("an action is needed: create, list or show".into()),
            other => Err(format!("unknown action {other}; use create, list or show")),
        }
    }

    /// The short form a step shows: `create demo: title`.
    pub fn spec(&self) -> String {
        match self {
            TaskAction::Create { project, title, .. } => {
                if project.is_empty() {
                    format!("create: {}", crate::harness::one_line(title, 120))
                } else {
                    format!("create {project}: {}", crate::harness::one_line(title, 120))
                }
            }
            TaskAction::List { project, status } => {
                let mut s = "list".to_string();
                if !project.is_empty() {
                    let _ = write!(s, " {project}");
                }
                if !status.is_empty() {
                    let _ = write!(s, " ({status})");
                }
                s
            }
            TaskAction::Show { id } => format!("show {id}"),
        }
    }

    /// What a permission rule's filter matches: the action word, so
    /// `deny task(create)` leaves list and show.
    pub fn subject(&self) -> String {
        match self {
            TaskAction::Create { .. } => "create".to_string(),
            TaskAction::List { .. } => "list".to_string(),
            TaskAction::Show { .. } => "show".to_string(),
        }
    }

    /// Whether the call changes the board (create) rather than reads it.
    pub fn changes(&self) -> bool {
        matches!(self, TaskAction::Create { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        pulse_testutil::neutralize_runner_environment();
        let home = tempfile::tempdir().unwrap();
        let s = Store::open(pulse_store::Options {
            home: home.path().display().to_string(),
            create: true,
            now: "2026-11-01T12:00:00Z".into(),
            ..pulse_store::Options::default()
        })
        .unwrap();
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO projects (slug, name, path, added_at) VALUES ('demo', 'Demo', ?, ?)",
                rusqlite::params![home.path().join("demo").display().to_string(), s.now()],
            )
            .unwrap();
        (home, s)
    }

    #[test]
    fn parse_reads_every_spelling_and_refuses_the_malformed() {
        assert_eq!(
            TaskAction::parse(
                r#"{"action":"create","title":" Ship it ","brief":"why","priority":3}"#
            )
            .unwrap(),
            TaskAction::Create {
                project: String::new(),
                title: "Ship it".into(),
                brief: "why".into(),
                priority: 3
            }
        );
        assert_eq!(
            TaskAction::parse(r#"{"action":"new","title":"x","project":"demo","priority":"2"}"#)
                .unwrap(),
            TaskAction::Create {
                project: "demo".into(),
                title: "x".into(),
                brief: String::new(),
                priority: 2
            }
        );
        assert_eq!(
            TaskAction::parse(r#"{"action":"list","status":"OPEN"}"#).unwrap(),
            TaskAction::List {
                project: String::new(),
                status: "open".into()
            }
        );
        assert_eq!(
            TaskAction::parse(r#"{"action":"show","id":"TASK-001"}"#).unwrap(),
            TaskAction::Show {
                id: "TASK-001".into()
            }
        );
        assert!(TaskAction::parse("[]").unwrap_err().contains("JSON object"));
        assert!(
            TaskAction::parse(r#"{"action":"create"}"#)
                .unwrap_err()
                .contains("needs a title")
        );
        assert!(
            TaskAction::parse(r#"{"action":"show"}"#)
                .unwrap_err()
                .contains("needs id")
        );
        assert!(
            TaskAction::parse(r#"{"action":"list","status":"soon"}"#)
                .unwrap_err()
                .contains("unknown status")
        );
        assert!(
            TaskAction::parse(r#"{"action":"dance"}"#)
                .unwrap_err()
                .contains("unknown action")
        );
        assert!(
            TaskAction::parse(r#"{"action":"create","title":"x","priority":"soon"}"#)
                .unwrap_err()
                .contains("whole number")
        );
        assert!(
            TaskAction::parse(r#"{"action":"create","title":["x"]}"#)
                .unwrap_err()
                .contains("must be a string")
        );
        let create =
            TaskAction::parse(r#"{"action":"create","title":"Ship it","project":"demo"}"#).unwrap();
        assert_eq!(create.spec(), "create demo: Ship it");
        assert_eq!(create.subject(), "create");
        assert!(create.changes());
        let list =
            TaskAction::parse(r#"{"action":"list","project":"demo","status":"open"}"#).unwrap();
        assert_eq!(list.spec(), "list demo (open)");
        assert!(!list.changes());
        assert_eq!(
            TaskAction::parse(r#"{"action":"show","id":"TASK-001"}"#)
                .unwrap()
                .spec(),
            "show TASK-001"
        );
    }

    #[test]
    fn desk_files_lists_and_shows_with_the_runs_identity() {
        let (_home, s) = store();
        let desk = TaskDesk::new(
            s.clone(),
            "scout",
            Origin::Conversation("chat-1".into()),
            "demo",
        );
        let created = desk
            .run(&TaskAction::Create {
                project: String::new(),
                title: "Ship the composer".into(),
                brief: "A composer with a route; prove it with the hub tests.".into(),
                priority: 2,
            })
            .unwrap();
        assert!(
            created.starts_with("TASK-001 is on the board for demo: Ship the composer"),
            "{created}"
        );
        assert!(
            created.contains("filed by scout (origin chat:chat-1)"),
            "{created}"
        );
        let t = pulse_tasks::tasks::get(&s, "TASK-001").unwrap().0;
        assert_eq!(
            (t.created_by.as_str(), t.origin.as_str(), t.priority),
            ("scout", "chat:chat-1", 2)
        );
        // The guard rails speak through the tool.
        let err = desk
            .run(&TaskAction::Create {
                project: String::new(),
                title: "Another".into(),
                brief: String::new(),
                priority: 0,
            })
            .unwrap_err();
        assert!(err.contains("needs a brief"), "{err}");
        let err = desk
            .run(&TaskAction::Create {
                project: String::new(),
                title: "ship the COMPOSER".into(),
                brief: "again".into(),
                priority: 0,
            })
            .unwrap_err();
        assert!(err.contains("Already a task: TASK-001"), "{err}");
        let err = desk
            .run(&TaskAction::Create {
                project: "nope".into(),
                title: "Elsewhere".into(),
                brief: "why".into(),
                priority: 0,
            })
            .unwrap_err();
        assert!(err.contains("No project \"nope\""), "{err}");
        let list = desk
            .run(&TaskAction::List {
                project: String::new(),
                status: "open".into(),
            })
            .unwrap();
        assert!(list.starts_with("1 task in demo, status open\n"), "{list}");
        assert!(
            list.contains(
                "- TASK-001 open p2 Ship the composer (by scout, origin chat:chat-1, updated"
            ),
            "{list}"
        );
        let none = desk
            .run(&TaskAction::List {
                project: String::new(),
                status: "done".into(),
            })
            .unwrap();
        assert_eq!(none, "No tasks in demo, status done.");
        pulse_tasks::amend(&s, "TASK-001", "", "Also the toolbar.", "widened", "human").unwrap();
        pulse_tasks::reply(&s, "TASK-001", "demo-lead", "planning it").unwrap();
        let shown = desk
            .run(&TaskAction::Show {
                id: "TASK-001".into(),
            })
            .unwrap();
        assert!(
            shown.starts_with("TASK-001 open Ship the composer\n"),
            "{shown}"
        );
        assert!(shown.contains("by scout (origin chat:chat-1)"), "{shown}");
        assert!(shown.contains("Also the toolbar."), "{shown}");
        assert!(
            shown.contains("2 revisions (latest first):\n  rev 2 "),
            "{shown}"
        );
        assert!(shown.contains("(widened)"), "{shown}");
        assert!(
            shown.contains(
                "1 message on its thread:\n  2026-11-01T12:00:00Z demo-lead: planning it"
            ),
            "{shown}"
        );
        assert!(
            desk.run(&TaskAction::Show {
                id: "TASK-404".into()
            })
            .unwrap_err()
            .contains("No task")
        );
    }

    #[test]
    fn a_desk_without_a_project_needs_one_per_call() {
        let (_home, s) = store();
        let desk = TaskDesk::new(s.clone(), "scout", Origin::Job("job-1".into()), "");
        let err = desk
            .run(&TaskAction::Create {
                project: String::new(),
                title: "Somewhere".into(),
                brief: "why".into(),
                priority: 0,
            })
            .unwrap_err();
        assert!(
            err.contains("say which one the task is for (project: one of demo)"),
            "{err}"
        );
        let ok = desk
            .run(&TaskAction::Create {
                project: "demo".into(),
                title: "Somewhere".into(),
                brief: "why".into(),
                priority: 0,
            })
            .unwrap();
        assert!(ok.contains("origin job:job-1"), "{ok}");
        // list without a project lists every project.
        let list = desk
            .run(&TaskAction::List {
                project: String::new(),
                status: String::new(),
            })
            .unwrap();
        assert!(list.starts_with("1 task in every project\n"), "{list}");
        let tool = task_tool("");
        assert!(
            tool.description.contains("create needs project"),
            "{}",
            tool.description
        );
        assert!(
            task_tool("demo")
                .description
                .contains("this run's project, demo")
        );
    }
}
