//! `/api/v1/automations`: the standing jobs of every project in one place -
//! the ones its `.pulse/pulse.yaml` keeps and the ones set by hand - and
//! the form that writes a new one. The console composes a YAML entry; this
//! module checks it with the code the daemon applies the file with
//! (`pulse_tasks::manifest`, `pulse_tasks::schedules`), commits it to the
//! project's default branch without touching any work in progress, and
//! applies it at once with `manifest::sync`. Nothing is pushed: the commit
//! travels with the owner's next push like any other.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::additions::{
    ADDITION_WRITES, JsonMap, addition_failure, addition_method, addition_not_found,
    addition_string,
};
use crate::httpx::{Request, ResponseWriter};
use crate::server::{Handler, read_payload, write_error, write_json};
use pulse_tasks::manifest::{self, Entry, Manifest};
use pulse_tasks::schedules::{self, Schedule, ScheduleInput};
use serde_json::{Value, json};

const ACTIONS: &[&str] = &["pause", "resume", "end", "run"];

/// The file a project without one gets.
const DEFAULT_PATH: &str = ".pulse/pulse.yaml";

/// The identity a commit carries when the repository names none.
const FALLBACK_IDENTITY: (&str, &str) = ("PULSE", "pulse@localhost");

enum Route {
    List,
    Add,
    Check,
    Remove,
    Action(String, String),
}

impl Handler {
    pub(crate) fn addition_automations(
        &self,
        w: &mut ResponseWriter,
        r: &Request,
        parts: Vec<String>,
    ) {
        let route = match parts.len() {
            1 if r.method == "GET" => Route::List,
            1 => Route::Add,
            2 if parts[1] == "check" => Route::Check,
            2 if parts[1] == "remove" => Route::Remove,
            3 if ACTIONS.contains(&parts[2].as_str()) => {
                Route::Action(parts[1].clone(), parts[2].clone())
            }
            _ => {
                addition_not_found(w);
                return;
            }
        };
        let methods = match route {
            Route::List | Route::Add => vec!["GET".to_string(), "POST".to_string()],
            _ => vec!["POST".to_string()],
        };
        if !addition_method(w, r, methods)
            || !self.addition_available(w, r, vec!["schedules".to_string(), "projects".to_string()])
        {
            return;
        }
        match route {
            Route::List => self.automations_list(w, r),
            Route::Add => self.automations_add(w, r),
            Route::Check => self.automations_check(w, r),
            Route::Remove => self.automations_remove(w, r),
            Route::Action(id, action) => self.automations_action(w, r, &id, &action),
        }
    }

    fn automations_list(&self, w: &mut ResponseWriter, r: &Request) {
        let Some(store) = self.store.as_deref() else {
            return;
        };
        let only = self.addition_project(r);
        let projects = match store.projects(true) {
            Ok(v) => v,
            Err(e) => {
                addition_failure(w, e);
                return;
            }
        };
        let mut out = Vec::new();
        for p in projects {
            if !only.is_empty() && p.slug != only {
                continue;
            }
            match project_view(store, &p) {
                Ok(Some(v)) => out.push(v),
                Ok(None) => {}
                Err(e) => {
                    addition_failure(w, e);
                    return;
                }
            }
        }
        write_json(
            w,
            200,
            json!({
                "projects": out,
                "limits": {
                    "max_per_project": store.setting_int("schedule.max_per_project"),
                    "min_minutes": store.setting_int("schedule.min_minutes"),
                },
                "kinds": schedules::SCHEDULE_KINDS,
                "script_kinds": schedules::SCRIPT_KINDS,
                "script_job_kinds": schedules::SCRIPT_JOB_KINDS,
            }),
        );
    }

    /// The project a write names, checked against the one this hub serves.
    fn automations_project(&self, w: &mut ResponseWriter, payload: &JsonMap) -> Option<String> {
        let (mut project, ok) = addition_string(w, payload, "project", self.project.is_empty());
        if !ok {
            return None;
        }
        if project.is_empty() {
            project = self.project.clone();
        }
        if !self.project.is_empty() && project != self.project {
            write_error(
                w,
                400,
                "Project mismatch".to_string(),
                "This hub serves a different project.".to_string(),
                "Write the automation in the selected project.".to_string(),
            );
            return None;
        }
        Some(project)
    }

    fn automations_check(&self, w: &mut ResponseWriter, r: &Request) {
        let (payload, ok) = read_payload(w, r);
        if !ok {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            return;
        };
        let Some(project) = self.automations_project(w, &payload) else {
            return;
        };
        let draft = match Draft::from_payload(&payload) {
            Ok(d) => d,
            Err(e) => {
                write_error(w, 400, e.0, e.1, e.2);
                return;
            }
        };
        let checked = match check_draft(store, &project, &draft) {
            Ok(c) => c,
            Err(e) => {
                addition_failure(w, e);
                return;
            }
        };
        write_json(w, 200, checked.json());
    }

    fn automations_add(&self, w: &mut ResponseWriter, r: &Request) {
        let (payload, ok) = read_payload(w, r);
        if !ok {
            return;
        }
        let _lock = ADDITION_WRITES.lock().unwrap_or_else(|e| e.into_inner());
        let Some(store) = self.store.as_deref() else {
            return;
        };
        let Some(project) = self.automations_project(w, &payload) else {
            return;
        };
        let draft = match Draft::from_payload(&payload) {
            Ok(d) => d,
            Err(e) => {
                write_error(w, 400, e.0, e.1, e.2);
                return;
            }
        };
        let checked = match check_draft(store, &project, &draft) {
            Ok(c) => c,
            Err(e) => {
                addition_failure(w, e);
                return;
            }
        };
        if !checked.problems.is_empty() {
            let mut body = checked.json();
            body["error"] = json!("The automation has problems");
            body["why"] = json!(checked.problems.join("; "));
            body["fix"] = json!("Fix what the problems name and try again.");
            write_json(w, 400, body);
            return;
        }
        let who = pulse_store::HUMAN_AGENT;
        let result = if draft.in_repo {
            write_repo(store, &project, &draft, &checked, who)
        } else {
            write_store(store, &project, &draft, who).map(|schedule| {
                json!({
                    "where": "store",
                    "schedule": schedule_json(&schedule),
                    "yaml": checked.yaml,
                    "warning": checked.warning,
                })
            })
        };
        match result {
            Ok(body) => write_json(
                w,
                if draft.replace.is_some() || draft.id.is_some() {
                    200
                } else {
                    201
                },
                body,
            ),
            Err(e) => addition_failure(w, e),
        }
    }

    fn automations_remove(&self, w: &mut ResponseWriter, r: &Request) {
        let (payload, ok) = read_payload(w, r);
        if !ok {
            return;
        }
        let _lock = ADDITION_WRITES.lock().unwrap_or_else(|e| e.into_inner());
        let Some(store) = self.store.as_deref() else {
            return;
        };
        let Some(project) = self.automations_project(w, &payload) else {
            return;
        };
        let (name, ok) = addition_string(w, &payload, "name", true);
        if !ok {
            return;
        }
        match remove_from_repo(store, &project, name.trim(), pulse_store::HUMAN_AGENT) {
            Ok(body) => write_json(w, 200, body),
            Err(e) => addition_failure(w, e),
        }
    }

    fn automations_action(&self, w: &mut ResponseWriter, r: &Request, id: &str, action: &str) {
        let (payload, ok) = read_payload(w, r);
        if !ok {
            return;
        }
        let (note, ok) = addition_string(w, &payload, "note", false);
        if !ok {
            return;
        }
        let _lock = ADDITION_WRITES.lock().unwrap_or_else(|e| e.into_inner());
        let Some(store) = self.store.as_deref() else {
            return;
        };
        let who = pulse_store::HUMAN_AGENT;
        let sched = match schedules::get(store, id) {
            Ok(s) if self.project.is_empty() || s.project == self.project => s,
            _ => {
                write_error(
                    w,
                    404,
                    format!("No schedule {id}"),
                    "The id is not in this store.".to_string(),
                    "Refresh the list.".to_string(),
                );
                return;
            }
        };
        // Ending a schedule the file keeps means taking it out of the file:
        // ended in the store alone, the next sync would add it back.
        if action == "end"
            && sched.from_repo()
            && sched.status != "ended"
            && let Some(name) = manifest::origin_name(&sched.origin)
            && file_holds(store, &sched.project, name)
        {
            match remove_from_repo(store, &sched.project, name, who) {
                Ok(mut body) => {
                    if let Ok(now) = schedules::get(store, id) {
                        body["schedule"] = schedule_json(&now);
                    }
                    write_json(w, 200, body);
                }
                Err(e) => addition_failure(w, e),
            }
            return;
        }
        let result = match action {
            "pause" => schedules::pause(store, id, &note, who),
            "resume" => schedules::resume(store, id, who),
            "end" => schedules::end(store, id, &note, who),
            _ => schedules::run_now(store, id, who),
        };
        match result {
            Ok(s) => write_json(w, 200, json!({"schedule": schedule_json(&s)})),
            Err(e) => addition_failure(w, e),
        }
    }
}

/// Whether the project's file on its default branch still names `name`.
fn file_holds(store: &pulse_store::Store, project: &str, name: &str) -> bool {
    match manifest::repo_of(store, project) {
        Ok(Some((repo, branch, _))) => read_file(&repo, &branch)
            .map(|f| f.entry(name).is_some())
            .unwrap_or(false),
        _ => false,
    }
}

/// One project's automations as the console lists them: its schedules
/// (with the cadence in words), the file on its default branch as it is
/// now, what the daemon last applied, and the names the form can offer.
/// `None` for a project with neither schedules nor a checkout here.
fn project_view(
    store: &pulse_store::Store,
    p: &pulse_store::Project,
) -> anyhow::Result<Option<Value>> {
    let rows = schedules::list(store, &p.slug, "")?;
    let repo = manifest::repo_of(store, &p.slug)?;
    if rows.is_empty() && repo.is_none() {
        return Ok(None);
    }
    let file = match &repo {
        Some((path, branch, _)) => read_file(path, branch).map(|f| f.json()),
        None => None,
    };
    let applied = manifest::status(store, &p.slug)?.map(|st| {
        json!({
            "path": st.path,
            "commit": st.commit,
            "at": st.at,
            "problems": st.problems,
            "schedules": st.schedules,
        })
    });
    let mut skills: Vec<String> = store
        .skills(&p.slug)?
        .into_iter()
        .filter(|k| k.status == "enabled")
        .map(|k| k.name)
        .collect();
    skills.sort();
    skills.dedup();
    let agents: Vec<String> = if Path::new(&p.path).is_dir() {
        pulse_harness::agents::load_agent_specs(&p.path, &store.home, Some(store), &p.slug)
            .into_iter()
            .filter(|a| a.loads())
            .map(|a| a.name)
            .collect()
    } else {
        Vec::new()
    };
    Ok(Some(json!({
        "slug": p.slug,
        "name": p.name,
        "repo": repo.as_ref().map(|(path, branch, commit)| json!({
            "path": path,
            "branch": branch,
            "commit": commit.chars().take(12).collect::<String>(),
        })),
        "file": file,
        "applied": applied,
        "schedules": rows.iter().map(schedule_json).collect::<Vec<_>>(),
        "options": {"skills": skills, "agents": agents},
    })))
}

fn schedule_json(s: &Schedule) -> Value {
    let mut v = serde_json::to_value(s).unwrap_or(Value::Null);
    if let Some(m) = v.as_object_mut() {
        m.insert(
            "every".into(),
            json!(schedules::format_every(i64::from(s.every_minutes))),
        );
        m.insert(
            "every_words".into(),
            json!(every_words(i64::from(s.every_minutes), &s.at_time)),
        );
        m.insert(
            "origin_name".into(),
            json!(manifest::origin_name(&s.origin).unwrap_or("")),
        );
        m.insert(
            "origin_path".into(),
            json!(origin_path(&s.origin).unwrap_or("")),
        );
    }
    v
}

/// The file path in a schedule's `origin` (`repo:<path>#<name>`).
fn origin_path(origin: &str) -> Option<&str> {
    origin
        .strip_prefix(pulse_store::REPO_ORIGIN)?
        .split_once('#')
        .map(|(path, _)| path)
}

/// The cadence as the console says it: "every day at 02:00 UTC", "every
/// 6 hours", "every 2 weeks", "every 90 minutes".
pub(crate) fn every_words(minutes: i64, at: &str) -> String {
    let day = 24 * 60;
    let unit = |n: i64, one: &str, many: &str| {
        if n == 1 {
            format!("every {one}")
        } else {
            format!("every {n} {many}")
        }
    };
    let mut words = if minutes > 0 && minutes % (7 * day) == 0 {
        unit(minutes / (7 * day), "week", "weeks")
    } else if minutes > 0 && minutes % day == 0 {
        unit(minutes / day, "day", "days")
    } else if minutes > 0 && minutes % 60 == 0 {
        unit(minutes / 60, "hour", "hours")
    } else {
        unit(minutes, "minute", "minutes")
    };
    if !at.trim().is_empty() {
        words.push_str(&format!(" at {} UTC", at.trim()));
    }
    words
}

/// The file of the default branch, read the way the daemon reads it.
struct RepoFile {
    path: String,
    text: String,
    /// The parsed file; `None` when the YAML is at fault (`problems` says).
    manifest: Option<Manifest>,
    problems: Vec<String>,
}

impl RepoFile {
    fn names(&self) -> Vec<String> {
        self.manifest
            .as_ref()
            .map(|m| {
                m.schedules
                    .iter()
                    .map(|e| e.name.trim().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn entry(&self, name: &str) -> Option<&Entry> {
        self.manifest
            .as_ref()?
            .schedules
            .iter()
            .find(|e| e.name.trim() == name)
    }

    fn json(&self) -> Value {
        json!({
            "path": self.path,
            "text": self.text,
            "entries": self.manifest.as_ref().map(|m| &m.schedules),
            "needs": self.manifest.as_ref().and_then(|m| m.needs.clone()),
            "problems": self.problems,
        })
    }
}

/// `None` when the branch has no PULSE file.
fn read_file(repo: &str, branch: &str) -> Option<RepoFile> {
    let path = manifest::MANIFEST_PATHS.iter().copied().find(|p| {
        git(
            repo,
            &["cat-file", "-t", &format!("{branch}:{p}")],
            &[],
            None,
        )
        .map(|t| t == "blob")
        .unwrap_or(false)
    })?;
    let mut problems = Vec::new();
    let text = match git_raw(repo, &["show", &format!("{branch}:{path}")], &[], None) {
        Ok(text) if text.len() > manifest::FILE_LIMIT => {
            problems.push(format!(
                "{path} is {} KB; the limit is {} KB",
                text.len() / 1024,
                manifest::FILE_LIMIT / 1024
            ));
            String::new()
        }
        Ok(text) => text,
        Err(e) => {
            problems.push(format!("{path} could not be read from {branch}: {e}"));
            String::new()
        }
    };
    let manifest = if problems.is_empty() {
        match manifest::parse(&text) {
            Ok(m) => {
                problems.extend(manifest::static_problems(&m));
                Some(m)
            }
            Err(e) => {
                problems.push(format!("{path} could not be read: {e}"));
                None
            }
        }
    } else {
        None
    };
    Some(RepoFile {
        path: path.to_string(),
        text,
        manifest,
        problems,
    })
}

/// What the console sends: the entry, where it goes, and what it replaces.
struct Draft {
    entry: Entry,
    /// `where: "repo"` (the default) writes the file; `"store"` keeps the
    /// schedule in the store alone, the way `pulse schedule add` does.
    in_repo: bool,
    /// The file entry this one replaces (an edit), by name.
    replace: Option<String>,
    /// The store schedule this one updates in place (an edit).
    id: Option<String>,
    /// A hand-set schedule that moves into the file: ended once the file
    /// holds the entry.
    moved: Option<String>,
}

type Refusal = (String, String, String);

impl Draft {
    fn from_payload(payload: &JsonMap) -> Result<Draft, Refusal> {
        let Some(raw) = payload.get("entry") else {
            return Err((
                "entry is required".to_string(),
                "The write names no automation.".to_string(),
                "Send entry: {name, title, every, ...} as the YAML entry's fields.".to_string(),
            ));
        };
        let mut entry: Entry = serde_json::from_value(raw.clone()).map_err(|e| {
            (
                "entry is not a schedule".to_string(),
                e.to_string(),
                "The entry takes the fields of a .pulse/pulse.yaml schedule: name, title, every, at, kind, brief, skill, script, job_kind, tier, priority, run_as, times, paused.".to_string(),
            )
        })?;
        for field in [
            &mut entry.name,
            &mut entry.title,
            &mut entry.every,
            &mut entry.at,
            &mut entry.kind,
            &mut entry.skill,
            &mut entry.script,
            &mut entry.job_kind,
            &mut entry.run_as,
        ] {
            *field = field.trim().to_string();
        }
        entry.brief = entry.brief.replace("\r\n", "\n").trim_end().to_string();
        let text = |key: &str| -> Option<String> {
            payload
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let in_repo = match text("where").as_deref() {
            None | Some("repo") => true,
            Some("store") => false,
            Some(other) => {
                return Err((
                    format!("where {other:?} is not a place"),
                    "An automation is kept in the repository's .pulse/pulse.yaml or in the store."
                        .to_string(),
                    "Send where: \"repo\" or where: \"store\".".to_string(),
                ));
            }
        };
        Ok(Draft {
            entry,
            in_repo,
            replace: text("replace"),
            id: text("id"),
            moved: text("move"),
        })
    }
}

/// The checks of a draft, in the order the console shows them.
struct Checked {
    yaml: String,
    problems: Vec<String>,
    warning: String,
    /// The file as it is on the branch, when the draft goes there.
    file: Option<RepoFile>,
    repo: Option<(String, String, String)>,
}

impl Checked {
    fn json(&self) -> Value {
        json!({
            "yaml": self.yaml,
            "problems": self.problems,
            "warning": self.warning,
            "path": self.file.as_ref().map(|f| f.path.as_str()).unwrap_or(DEFAULT_PATH),
            "branch": self.repo.as_ref().map(|(_, b, _)| b.as_str()).unwrap_or(""),
        })
    }
}

/// Checks a draft the way `pulse manifest check` checks a file: the YAML it
/// composes parses, the static checks pass, the store's checks pass (the
/// cadence, the skill, the custom agent, the script on the branch), and
/// the place it goes can take it.
fn check_draft(
    store: &pulse_store::Store,
    project: &str,
    draft: &Draft,
) -> anyhow::Result<Checked> {
    let Some(p) = store.project(project)? else {
        anyhow::bail!(
            "No project {project:?}\n  why: it is not registered in this store\n  fix: pick a project of the list"
        );
    };
    let yaml = yaml_item(&draft.entry, 2);
    let mut problems = Vec::new();
    let mut warning = String::new();
    let parsed = match manifest::parse(&format!("schedules:\n{yaml}")) {
        Ok(m) => Some(m),
        Err(e) => {
            problems.push(format!("the entry does not read back as YAML: {e}"));
            None
        }
    };
    let (repo, file) = if draft.in_repo {
        let repo = manifest::repo_of(store, &p.slug)?;
        match &repo {
            None => {
                problems.push(if p.path.trim().is_empty() || !Path::new(&p.path).is_dir() {
                    format!(
                        "{} has no checkout on this machine, so its file cannot be written here; keep the automation in the store instead",
                        p.slug
                    )
                } else {
                    format!(
                        "{} has no {} branch in {}, so its file cannot be written here; keep the automation in the store instead",
                        p.slug,
                        default_branch(&p),
                        p.path
                    )
                });
                (None, None)
            }
            Some((path, branch, _)) => {
                let file = read_file(path, branch);
                if let Some(f) = &file {
                    if f.manifest.is_none() {
                        problems.extend(f.problems.iter().cloned());
                        problems.push(format!(
                            "fix {} by hand before the console writes to it",
                            f.path
                        ));
                    }
                    let names = f.names();
                    let name = draft.entry.name.as_str();
                    match &draft.replace {
                        Some(old) if !names.iter().any(|n| n == old) => problems.push(format!(
                            "{} has no entry named `{old}` to replace",
                            f.path
                        )),
                        Some(old) if old != name && names.iter().any(|n| n == name) => problems
                            .push(format!(
                                "{} already has an entry named `{name}`; pick another name",
                                f.path
                            )),
                        None if names.iter().any(|n| n == name) => problems.push(format!(
                            "{} already has an entry named `{name}`; edit that one or pick another name",
                            f.path
                        )),
                        _ => {}
                    }
                } else if draft.replace.is_some() {
                    problems.push(format!(
                        "{} has no PULSE file on {branch}, so there is nothing to replace",
                        p.slug
                    ));
                }
                (repo, file)
            }
        }
    } else {
        if let Some(id) = &draft.id {
            match schedules::get(store, id) {
                Ok(s) if s.project != p.slug => {
                    problems.push(format!("{id} belongs to {}, not {}", s.project, p.slug));
                }
                Ok(s) if s.from_repo() => problems.push(format!(
                    "{id} is kept by {}; edit the file entry instead",
                    origin_path(&s.origin).unwrap_or("the project's PULSE file")
                )),
                Ok(s) if s.status == "ended" => {
                    problems.push(format!("{id} has ended; add a new automation instead"));
                }
                Ok(_) => {}
                Err(e) => problems.push(first_line(&e.to_string())),
            }
        } else {
            let cap = store.setting_int("schedule.max_per_project");
            let held = schedules::list(store, &p.slug, "")?
                .iter()
                .filter(|s| s.status != "ended")
                .count() as i64;
            if cap > 0 && held >= cap {
                problems.push(format!(
                    "{} already holds {held} schedules; the setting schedule.max_per_project caps them - end one first",
                    p.slug
                ));
            }
        }
        (None, None)
    };
    if let Some(m) = parsed {
        let statics = manifest::static_problems(&m);
        let held = !statics.is_empty();
        problems.extend(statics.into_iter().map(|p| strip_label(&p)));
        if !held && let Some(e) = m.schedules.first() {
            let path = file
                .as_ref()
                .map(|f| f.path.as_str())
                .unwrap_or(DEFAULT_PATH);
            let mut input = e.input(&p.slug, path, pulse_store::HUMAN_AGENT);
            if !draft.in_repo {
                input.origin = String::new();
            }
            match schedules::check(store, &input) {
                Ok(w) => warning = w,
                Err(err) => problems.push(first_line(&err.to_string())),
            }
        }
    }
    Ok(Checked {
        yaml,
        problems,
        warning,
        file,
        repo,
    })
}

/// A static problem without its "schedule `name`: " label: the console
/// shows one entry at a time.
fn strip_label(problem: &str) -> String {
    if let Some(rest) = problem.strip_prefix("schedule ")
        && let Some((_, rest)) = rest.split_once(": ")
    {
        return rest.to_string();
    }
    problem.to_string()
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").trim().to_string()
}

fn default_branch(p: &pulse_store::Project) -> String {
    if p.default_branch.trim().is_empty() {
        "main".to_string()
    } else {
        p.default_branch.trim().to_string()
    }
}

/// Keeps the draft in the store alone: `pulse schedule add`, or an update
/// of the schedule it names.
fn write_store(
    store: &pulse_store::Store,
    project: &str,
    draft: &Draft,
    who: &str,
) -> anyhow::Result<Schedule> {
    let mut input: ScheduleInput = draft.entry.input(project, DEFAULT_PATH, who);
    input.origin = String::new();
    let sched = match &draft.id {
        Some(id) => schedules::update(store, id, &input, who)?.0,
        None => schedules::add_checked(store, input)?.0,
    };
    let sched = if draft.entry.paused && sched.status == "active" {
        schedules::pause(store, &sched.id, "paused from the console", who)?
    } else if !draft.entry.paused && sched.status == "paused" && draft.id.is_some() {
        schedules::resume(store, &sched.id, who)?
    } else {
        sched
    };
    Ok(sched)
}

/// Writes the draft into the file on the default branch - appended, or in
/// place of the entry it replaces - commits it and applies the file.
fn write_repo(
    store: &pulse_store::Store,
    project: &str,
    draft: &Draft,
    checked: &Checked,
    who: &str,
) -> anyhow::Result<Value> {
    let Some((repo, branch, _)) = &checked.repo else {
        anyhow::bail!(
            "No checkout of {project} here\n  why: the file lives in the repository, which this machine has no checkout of\n  fix: keep the automation in the store instead"
        );
    };
    let (path, old_text) = match &checked.file {
        Some(f) => (f.path.clone(), f.text.clone()),
        None => (DEFAULT_PATH.to_string(), String::new()),
    };
    let indent = list_span(&old_text).map(|s| s.item_indent).unwrap_or(2);
    let item = yaml_item(&draft.entry, indent);
    let new_text = match &draft.replace {
        Some(old) => replace_item(&old_text, old, &item)?,
        None => append_item(&old_text, &item)?,
    };
    let mut expect: BTreeSet<String> = checked
        .file
        .as_ref()
        .map(|f| f.names().into_iter().collect())
        .unwrap_or_default();
    if let Some(old) = &draft.replace {
        expect.remove(old);
    }
    expect.insert(draft.entry.name.clone());
    verify_names(&path, &new_text, &expect)?;
    let name = &draft.entry.name;
    let message = match &draft.replace {
        Some(old) if old == name => format!("pulse: update schedule {name} in {path}"),
        Some(old) => format!("pulse: replace schedule {old} with {name} in {path}"),
        None => format!("pulse: add schedule {name} to {path}"),
    };
    let commit = if new_text == old_text {
        None
    } else {
        Some(commit_file(repo, branch, &path, Some(&new_text), &message)?)
    };
    let synced = manifest::sync(store, project, who, true)?;
    if let Some(id) = &draft.moved
        && let Ok(s) = schedules::get(store, id)
        && !s.from_repo()
        && s.status != "ended"
    {
        schedules::end(store, id, &format!("moved to {path} as {name}"), who)?;
    }
    let schedule = schedules::list(store, project, "")?
        .into_iter()
        .find(|s| s.status != "ended" && manifest::origin_name(&s.origin) == Some(name.as_str()));
    Ok(json!({
        "where": "repo",
        "path": path,
        "branch": branch,
        "commit": commit,
        "yaml": item,
        "warning": checked.warning,
        "synced": synced.map(|s| json!({"line": s.line(), "problems": s.problems})),
        "schedule": schedule.map(|s| schedule_json(&s)),
    }))
}

/// Takes an entry out of the file, commits and applies: the schedule ends
/// with the file's "removed" note.
fn remove_from_repo(
    store: &pulse_store::Store,
    project: &str,
    name: &str,
    who: &str,
) -> anyhow::Result<Value> {
    let Some((repo, branch, _)) = manifest::repo_of(store, project)? else {
        anyhow::bail!(
            "No checkout of {project} here\n  why: the file lives in the repository, which this machine has no checkout of\n  fix: remove the entry where the repository is checked out"
        );
    };
    let Some(file) = read_file(&repo, &branch) else {
        anyhow::bail!(
            "No PULSE file in {project}\n  why: {branch} has no .pulse/pulse.yaml\n  fix: nothing to remove"
        );
    };
    if file.manifest.is_none() {
        anyhow::bail!(
            "{} could not be read\n  why: {}\n  fix: fix the file by hand first",
            file.path,
            file.problems.join("; ")
        );
    }
    if file.entry(name).is_none() {
        anyhow::bail!(
            "No entry {name} in {}\n  why: the file on {branch} names {}\n  fix: refresh the list",
            file.path,
            if file.names().is_empty() {
                "no schedule".to_string()
            } else {
                file.names().join(", ")
            }
        );
    }
    let new_text = remove_item(&file.text, name)?;
    let mut expect: BTreeSet<String> = file.names().into_iter().collect();
    expect.remove(name);
    verify_names(&file.path, &new_text, &expect)?;
    let message = format!("pulse: remove schedule {name} from {}", file.path);
    let commit = commit_file(&repo, &branch, &file.path, Some(&new_text), &message)?;
    let synced = manifest::sync(store, project, who, true)?;
    Ok(json!({
        "where": "repo",
        "path": file.path,
        "branch": branch,
        "commit": commit,
        "removed": name,
        "synced": synced.map(|s| json!({"line": s.line(), "problems": s.problems})),
    }))
}

/// The merged file must read back with exactly the names expected; the
/// console never commits a file the daemon could not apply.
fn verify_names(path: &str, text: &str, expect: &BTreeSet<String>) -> anyhow::Result<()> {
    let m = manifest::parse(text).map_err(|e| {
        anyhow::anyhow!(
            "The file would not read back\n  why: {path} after the change: {e}\n  fix: edit the file by hand; the console did not write it"
        )
    })?;
    let got: BTreeSet<String> = m
        .schedules
        .iter()
        .map(|e| e.name.trim().to_string())
        .collect();
    if &got != expect {
        anyhow::bail!(
            "The file would not read back as expected\n  why: {path} after the change names {} where {} was expected\n  fix: edit the file by hand; the console did not write it",
            if got.is_empty() {
                "no schedule".to_string()
            } else {
                got.into_iter().collect::<Vec<_>>().join(", ")
            },
            if expect.is_empty() {
                "none".to_string()
            } else {
                expect.iter().cloned().collect::<Vec<_>>().join(", ")
            }
        );
    }
    Ok(())
}

// ---- YAML -------------------------------------------------------------

/// Words YAML reads as something other than a string when left bare.
const YAML_WORDS: &[&str] = &[
    "true", "false", "yes", "no", "on", "off", "null", "y", "n", "~",
];

/// A scalar as YAML: bare when it is plainly a word or a sentence, else a
/// double-quoted string (JSON's quoting is valid YAML).
pub(crate) fn yaml_scalar(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && !s.ends_with(' ')
        && s.chars().any(|c| c.is_ascii_alphabetic())
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, ' ' | '.' | '_' | '/' | '-' | '\'' | '(' | ')' | ',')
        })
        && !YAML_WORDS.contains(&s.to_ascii_lowercase().as_str())
        && !looks_numeric(s);
    if plain {
        s.to_string()
    } else {
        serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
    }
}

/// `1e5`, `0x1F`, `1_000`: digits with a letter YAML still reads as a
/// number.
fn looks_numeric(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.starts_with("0x")
        || lower.starts_with("0o")
        || lower.starts_with("0b")
        || (lower
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, 'e' | '.' | '_' | '-' | '+'))
            && lower.chars().any(|c| c.is_ascii_digit()))
        || matches!(lower.as_str(), ".inf" | "-.inf" | "+.inf" | ".nan")
}

/// One `- name: ...` item of `schedules:`, its dash at `indent` spaces,
/// the defaults left out so the file stays as short as a hand would write
/// it. Fields come in the order the README's example uses.
pub(crate) fn yaml_item(e: &Entry, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let field = " ".repeat(indent + 2);
    let mut out = format!("{pad}- name: {}\n", yaml_scalar(&e.name));
    let mut line = |key: &str, value: String| {
        out.push_str(&format!("{field}{key}: {value}\n"));
    };
    if !e.title.is_empty() {
        line("title", yaml_scalar(&e.title));
    }
    if !e.every.is_empty() {
        line("every", yaml_scalar(&e.every));
    }
    if !e.at.is_empty() {
        line(
            "at",
            serde_json::to_string(&e.at).unwrap_or_else(|_| "\"\"".to_string()),
        );
    }
    if !e.kind.is_empty() && e.kind != "task" {
        line("kind", yaml_scalar(&e.kind));
    }
    if !e.brief.is_empty() {
        if e.brief.contains('\n') && !e.brief.starts_with(' ') && !e.brief.starts_with('\t') {
            let block = " ".repeat(indent + 4);
            let mut text = String::from("|-\n");
            for l in e.brief.lines() {
                if l.trim().is_empty() {
                    text.push('\n');
                } else {
                    text.push_str(&format!("{block}{l}\n"));
                }
            }
            text.truncate(text.len() - 1);
            line("brief", text);
        } else {
            line("brief", yaml_scalar(&e.brief));
        }
    }
    if !e.skill.is_empty() {
        line("skill", yaml_scalar(&e.skill));
    }
    if !e.script.is_empty() {
        line("script", yaml_scalar(&e.script));
    }
    if !e.job_kind.is_empty() && e.job_kind != "task" {
        line("job_kind", yaml_scalar(&e.job_kind));
    }
    if e.tier != 1 {
        line("tier", e.tier.to_string());
    }
    if e.priority != 0 {
        line("priority", e.priority.to_string());
    }
    if !e.run_as.is_empty() {
        line("run_as", yaml_scalar(&e.run_as));
    }
    if e.times != 0 {
        line("times", e.times.to_string());
    }
    if e.paused {
        line("paused", "true".to_string());
    }
    out
}

/// Where the `schedules:` list sits in the file's lines.
struct ListSpan {
    /// The `schedules:` line.
    key: usize,
    /// `schedules: []` or `schedules:` with nothing under it.
    empty: bool,
    /// Indent of the `- ` items (2 when there are none yet).
    item_indent: usize,
    /// Each item: its first line, the line after its last, its name.
    items: Vec<(usize, usize, String)>,
    /// The line after the last item's last content line: where a new one
    /// goes.
    end: usize,
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn is_blank_or_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.is_empty() || t.starts_with('#')
}

/// The value of `name:` on a line (`- name: x`, `name: "x"`), if it is one.
fn name_on(line: &str) -> Option<String> {
    let t = line.trim_start();
    let t = t.strip_prefix('-').map(str::trim_start).unwrap_or(t);
    let rest = t.strip_prefix("name:")?;
    let rest = rest.trim();
    let rest = match rest.find(" #") {
        Some(i) if !rest.starts_with('"') && !rest.starts_with('\'') => rest[..i].trim(),
        _ => rest,
    };
    let unquoted = rest
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| rest.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(rest);
    Some(unquoted.to_string())
}

/// Finds the block list under a top-level `schedules:`; `None` when the
/// file has no such key, `Err` when it is written in a form the console
/// does not rewrite (a flow list with items, a mapping).
fn list_span(text: &str) -> Option<ListSpan> {
    list_span_checked(text).ok().flatten()
}

fn list_span_checked(text: &str) -> Result<Option<ListSpan>, String> {
    let lines: Vec<&str> = text.lines().collect();
    let Some(key) = lines.iter().position(|l| {
        l.starts_with("schedules:")
            && l["schedules:".len()..]
                .chars()
                .next()
                .is_none_or(|c| c == ' ' || c == '#')
    }) else {
        return Ok(None);
    };
    let after = lines[key]["schedules:".len()..]
        .split('#')
        .next()
        .unwrap_or("")
        .trim();
    if !after.is_empty() {
        if after == "[]" {
            return Ok(Some(ListSpan {
                key,
                empty: true,
                item_indent: 2,
                items: Vec::new(),
                end: key + 1,
            }));
        }
        return Err(format!(
            "schedules: is written on one line (`schedules: {after}`); write it as a block list, one `- name:` per entry, before the console edits it"
        ));
    }
    let mut i = key + 1;
    while i < lines.len() && is_blank_or_comment(lines[i]) {
        i += 1;
    }
    if i >= lines.len() || indent_of(lines[i]) == 0 && !lines[i].starts_with('-') {
        return Ok(Some(ListSpan {
            key,
            empty: true,
            item_indent: 2,
            items: Vec::new(),
            end: key + 1,
        }));
    }
    let item_indent = indent_of(lines[i]);
    if !lines[i].trim_start().starts_with('-') {
        return Err(
            "schedules: is not a list; write it as a block list, one `- name:` per entry"
                .to_string(),
        );
    }
    let starts_item = |l: &str| indent_of(l) == item_indent && l.trim_start().starts_with('-');
    let in_list = |l: &str| is_blank_or_comment(l) || indent_of(l) > item_indent || starts_item(l);
    let mut items = Vec::new();
    let mut end = i;
    let mut j = i;
    while j < lines.len() && in_list(lines[j]) {
        if starts_item(lines[j]) {
            let start = j;
            let mut name = name_on(lines[j]).filter(|n| !n.is_empty());
            j += 1;
            while j < lines.len() && in_list(lines[j]) && !starts_item(lines[j]) {
                if name.is_none() && indent_of(lines[j]) > item_indent {
                    name = name_on(lines[j]).filter(|n| !n.is_empty());
                }
                j += 1;
            }
            // The item ends after its last content line; blank lines and
            // comments that follow belong to the file, not to it.
            let mut stop = j;
            while stop > start + 1 && is_blank_or_comment(lines[stop - 1]) {
                stop -= 1;
            }
            items.push((start, stop, name.unwrap_or_default()));
            end = stop;
        } else {
            j += 1;
        }
    }
    Ok(Some(ListSpan {
        key,
        empty: items.is_empty(),
        item_indent,
        items,
        end,
    }))
}

fn join_lines(lines: &[String]) -> String {
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn item_lines(item: &str) -> Vec<String> {
    item.lines().map(str::to_string).collect()
}

/// The file with `item` added at the end of its `schedules:` list; a file
/// without the key gets one at its end, an empty file becomes the list.
pub(crate) fn append_item(text: &str, item: &str) -> anyhow::Result<String> {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    match list_span_checked(text).map_err(|e| anyhow::anyhow!("{e}"))? {
        None => {
            if !lines.is_empty() && !lines.last().is_some_and(|l| l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("schedules:".to_string());
            lines.extend(item_lines(item));
        }
        Some(span) if span.empty => {
            lines[span.key] = format!(
                "schedules:{}",
                lines[span.key]["schedules:".len()..]
                    .split_once('#')
                    .map(|(_, c)| format!(" #{c}"))
                    .unwrap_or_default()
            );
            let at = span.key + 1;
            for (k, l) in item_lines(item).into_iter().enumerate() {
                lines.insert(at + k, l);
            }
        }
        Some(span) => {
            for (k, l) in item_lines(item).into_iter().enumerate() {
                lines.insert(span.end + k, l);
            }
        }
    }
    Ok(join_lines(&lines))
}

/// The file with the entry named `old` replaced by `item`, in its place.
pub(crate) fn replace_item(text: &str, old: &str, item: &str) -> anyhow::Result<String> {
    let span = list_span_checked(text)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .ok_or_else(|| anyhow::anyhow!("the file has no schedules: list"))?;
    let (start, stop, _) = span
        .items
        .iter()
        .find(|(_, _, n)| n == old)
        .ok_or_else(|| anyhow::anyhow!("the file has no entry named `{old}`"))?;
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    lines.splice(*start..*stop, item_lines(item));
    Ok(join_lines(&lines))
}

/// The file without the entry named `name`; the last entry leaves
/// `schedules: []` behind so the file still reads.
pub(crate) fn remove_item(text: &str, name: &str) -> anyhow::Result<String> {
    let span = list_span_checked(text)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .ok_or_else(|| anyhow::anyhow!("the file has no schedules: list"))?;
    let (start, stop, _) = span
        .items
        .iter()
        .find(|(_, _, n)| n == name)
        .ok_or_else(|| anyhow::anyhow!("the file has no entry named `{name}`"))?;
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    // A blank line that separated this entry from the next goes with it.
    let mut stop = *stop;
    if span.items.len() > 1 && stop < lines.len() && lines[stop].trim().is_empty() {
        let next_starts_item = span.items.iter().any(|(s, _, _)| *s > stop);
        if next_starts_item {
            stop += 1;
        }
    }
    lines.drain(*start..stop);
    if span.items.len() == 1 {
        lines[span.key] = format!(
            "schedules: []{}",
            lines[span.key]["schedules:".len()..]
                .split_once('#')
                .map(|(_, c)| format!(" #{c}"))
                .unwrap_or_default()
        );
    }
    Ok(join_lines(&lines))
}

// ---- git ----------------------------------------------------------------

/// Runs git in `dir` with the hub's hermetic environment; stdout trimmed,
/// or stderr as the error.
fn git(
    dir: &str,
    args: &[&str],
    envs: &[(&str, &str)],
    stdin: Option<&[u8]>,
) -> Result<String, String> {
    git_raw(dir, args, envs, stdin).map(|out| out.trim_end().to_string())
}

/// `git` with its output as written - `show` keeps the file's last newline,
/// so a rewrite that changes nothing compares equal.
fn git_raw(
    dir: &str,
    args: &[&str],
    envs: &[(&str, &str)],
    stdin: Option<&[u8]>,
) -> Result<String, String> {
    let mut cmd = Command::new(pulse_gitenv::git_program());
    cmd.arg("-C").arg(dir).args(args);
    cmd.env_clear();
    for kv in pulse_gitenv::env() {
        if let Some((k, v)) = kv.split_once('=') {
            cmd.env(k, v);
        }
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("git could not start: {e}"))?;
    if let Some(bytes) = stdin
        && let Some(mut pipe) = child.stdin.take()
    {
        pipe.write_all(bytes)
            .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The working tree that has `branch` checked out, if any.
fn checkout_of(repo: &str, branch: &str) -> Option<String> {
    let out = git(repo, &["worktree", "list", "--porcelain"], &[], None).ok()?;
    let want = format!("refs/heads/{branch}");
    let mut path: Option<String> = None;
    for line in out.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(p.to_string());
        } else if line.strip_prefix("branch ") == Some(want.as_str()) {
            return path;
        } else if line.is_empty() {
            path = None;
        }
    }
    None
}

/// The identity the repository's configuration gives its commits, read
/// with the owner's own environment (the hermetic one hides their global
/// configuration); PULSE's own when it gives none.
fn identity(repo: &str) -> (String, String) {
    let read = |key: &str| -> String {
        let mut cmd = Command::new(pulse_gitenv::git_program());
        cmd.args(["-C", repo, "config", "--get", key]);
        for kv in std::env::vars().map(|(k, v)| format!("{k}={v}")) {
            if pulse_gitenv::hook_var(&kv)
                && let Some((k, _)) = kv.split_once('=')
            {
                cmd.env_remove(k);
            }
        }
        cmd.output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    let (name, email) = (read("user.name"), read("user.email"));
    if name.is_empty() || email.is_empty() {
        (
            FALLBACK_IDENTITY.0.to_string(),
            FALLBACK_IDENTITY.1.to_string(),
        )
    } else {
        (name, email)
    }
}

/// Commits `text` at `path` on `branch` of `repo` (a removal when `None`)
/// without touching any work in progress: through the checkout that has
/// the branch out when the file is clean there, else straight into the
/// branch with a temporary index. Never pushes. Returns the new commit.
pub(crate) fn commit_file(
    repo: &str,
    branch: &str,
    path: &str,
    text: Option<&str>,
    message: &str,
) -> anyhow::Result<String> {
    let (name, email) = identity(repo);
    let user_name = format!("user.name={name}");
    let user_email = format!("user.email={email}");
    let refuse = |what: String, why: String, fix: &str| {
        anyhow::anyhow!("{what}\n  why: {why}\n  fix: {fix}")
    };
    if let Some(wt) = checkout_of(repo, branch) {
        let dirty = git(
            &wt,
            &["status", "--porcelain", "--untracked-files=all", "--", path],
            &[],
            None,
        )
        .map_err(|e| {
            refuse(
                "The checkout could not be read".into(),
                e,
                "check the repository",
            )
        })?;
        if !dirty.trim().is_empty() {
            return Err(refuse(
                format!("{path} has uncommitted changes"),
                format!(
                    "the checkout of {branch} at {wt} has edits to the file the console would write"
                ),
                "commit or discard those edits first, then try again",
            ));
        }
        let full = Path::new(&wt).join(path);
        match text {
            Some(t) => {
                if let Some(parent) = full.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&full, t)?;
            }
            None => {
                let _ = std::fs::remove_file(&full);
            }
        }
        git(&wt, &["add", "-A", "--", path], &[], None).map_err(|e| {
            refuse(
                "The file could not be staged".into(),
                e,
                "check the checkout",
            )
        })?;
        git(
            &wt,
            &[
                "-c",
                &user_name,
                "-c",
                &user_email,
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "--no-verify",
                "-m",
                message,
                "--",
                path,
            ],
            &[],
            None,
        )
        .map_err(|e| {
            refuse(
                "The file could not be committed".into(),
                e,
                "finish what the checkout is in the middle of (a merge, a rebase), then try again",
            )
        })?;
        return git(&wt, &["rev-parse", "HEAD"], &[], None).map_err(|e| anyhow::anyhow!(e));
    }
    let reference = format!("refs/heads/{branch}");
    let old = git(repo, &["rev-parse", "--verify", &reference], &[], None).map_err(|e| {
        refuse(
            format!("No branch {branch}"),
            e,
            "check the project's default branch",
        )
    })?;
    let index = std::env::temp_dir().join(format!(
        "pulse-index-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let index_str = index.display().to_string();
    let env = [("GIT_INDEX_FILE", index_str.as_str())];
    let result = (|| -> Result<String, String> {
        git(repo, &["read-tree", &old], &env, None)?;
        match text {
            Some(t) => {
                let blob = git(
                    repo,
                    &["hash-object", "-w", "--stdin"],
                    &[],
                    Some(t.as_bytes()),
                )?;
                git(
                    repo,
                    &[
                        "update-index",
                        "--add",
                        "--cacheinfo",
                        &format!("100644,{blob},{path}"),
                    ],
                    &env,
                    None,
                )?;
            }
            None => {
                git(
                    repo,
                    &["update-index", "--force-remove", "--", path],
                    &env,
                    None,
                )?;
            }
        }
        let tree = git(repo, &["write-tree"], &env, None)?;
        let new = git(
            repo,
            &[
                "-c",
                &user_name,
                "-c",
                &user_email,
                "commit-tree",
                &tree,
                "-p",
                &old,
                "-m",
                message,
            ],
            &[],
            None,
        )?;
        git(repo, &["update-ref", &reference, &new, &old], &[], None)?;
        Ok(new)
    })();
    let _ = std::fs::remove_file(&index);
    result.map_err(|e| {
        refuse(
            "The file could not be committed".into(),
            e,
            "check the repository, or write the file where it is checked out",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers_http as http;

    fn entry(name: &str) -> Entry {
        Entry {
            name: name.to_string(),
            title: "Audit the dependencies".to_string(),
            every: "1d".to_string(),
            at: "02:00".to_string(),
            brief: "Run the audit and fix what it names.".to_string(),
            ..Entry::default()
        }
    }

    #[test]
    fn yaml_scalars_stay_bare_only_when_yaml_reads_them_back_as_the_same_string() {
        assert_eq!(yaml_scalar("deps-audit"), "deps-audit");
        assert_eq!(
            yaml_scalar("Audit the dependencies"),
            "Audit the dependencies"
        );
        assert_eq!(yaml_scalar("scripts/check.sh"), "scripts/check.sh");
        assert_eq!(yaml_scalar("09:00"), "\"09:00\"");
        assert_eq!(yaml_scalar("yes"), "\"yes\"");
        assert_eq!(yaml_scalar("1e5"), "\"1e5\"");
        assert_eq!(yaml_scalar("12"), "\"12\"");
        assert_eq!(yaml_scalar("a: b"), "\"a: b\"");
        assert_eq!(yaml_scalar("# not a comment"), "\"# not a comment\"");
        assert_eq!(yaml_scalar(" lead"), "\" lead\"");
        assert_eq!(yaml_scalar("trail "), "\"trail \"");
        assert_eq!(yaml_scalar(""), "\"\"");
    }

    #[test]
    fn yaml_item_leaves_defaults_out_and_reads_back_as_the_same_entry() {
        let e = Entry {
            kind: "check".to_string(),
            script: "scripts/check.sh".to_string(),
            job_kind: "improve".to_string(),
            tier: 0,
            priority: 2,
            run_as: "release-manager".to_string(),
            times: 5,
            paused: true,
            brief: "First line.\n\n  indented second\nthird: with colon".to_string(),
            skill: String::new(),
            ..entry("nightly")
        };
        let item = yaml_item(&e, 2);
        assert_eq!(
            item,
            "  - name: nightly\n    title: Audit the dependencies\n    every: 1d\n    at: \"02:00\"\n    kind: check\n    brief: |-\n      First line.\n\n        indented second\n      third: with colon\n    script: scripts/check.sh\n    job_kind: improve\n    tier: 0\n    priority: 2\n    run_as: release-manager\n    times: 5\n    paused: true\n"
        );
        let m = manifest::parse(&format!("schedules:\n{item}")).unwrap();
        assert_eq!(m.schedules, vec![e]);
        let plain = yaml_item(&entry("deps-audit"), 0);
        assert_eq!(
            plain,
            "- name: deps-audit\n  title: Audit the dependencies\n  every: 1d\n  at: \"02:00\"\n  brief: Run the audit and fix what it names.\n"
        );
        let m = manifest::parse(&format!("schedules:\n{plain}")).unwrap();
        assert_eq!(m.schedules, vec![entry("deps-audit")]);
    }

    const FILE: &str = "# the project's file\nneeds: [macos]\nschedules:\n  - name: deps-audit    # first\n    every: 1d\n    brief: Audit.\n\n  - name: release-notes\n    skill: release\n    every: weekly\n\n# trailing words\n";

    #[test]
    fn append_replace_and_remove_keep_the_rest_of_the_file_as_written() {
        let item = yaml_item(&entry("sweep"), 2);
        let appended = append_item(FILE, &item).unwrap();
        assert_eq!(
            appended,
            "# the project's file\nneeds: [macos]\nschedules:\n  - name: deps-audit    # first\n    every: 1d\n    brief: Audit.\n\n  - name: release-notes\n    skill: release\n    every: weekly\n  - name: sweep\n    title: Audit the dependencies\n    every: 1d\n    at: \"02:00\"\n    brief: Run the audit and fix what it names.\n\n# trailing words\n"
        );
        let m = manifest::parse(&appended).unwrap();
        assert_eq!(
            m.schedules
                .iter()
                .map(|e| e.name.as_str())
                .collect::<Vec<_>>(),
            vec!["deps-audit", "release-notes", "sweep"]
        );

        let replaced = replace_item(FILE, "deps-audit", &item).unwrap();
        assert_eq!(
            replaced,
            "# the project's file\nneeds: [macos]\nschedules:\n  - name: sweep\n    title: Audit the dependencies\n    every: 1d\n    at: \"02:00\"\n    brief: Run the audit and fix what it names.\n\n  - name: release-notes\n    skill: release\n    every: weekly\n\n# trailing words\n"
        );
        assert!(replace_item(FILE, "nope", &item).is_err());

        let removed = remove_item(FILE, "deps-audit").unwrap();
        assert_eq!(
            removed,
            "# the project's file\nneeds: [macos]\nschedules:\n  - name: release-notes\n    skill: release\n    every: weekly\n\n# trailing words\n"
        );
        let last = remove_item(&removed, "release-notes").unwrap();
        assert_eq!(
            last,
            "# the project's file\nneeds: [macos]\nschedules: []\n\n# trailing words\n"
        );
        assert!(manifest::parse(&last).unwrap().schedules.is_empty());
    }

    #[test]
    fn append_handles_files_without_a_list_and_items_at_the_margin() {
        let item = yaml_item(&entry("sweep"), 2);
        assert_eq!(
            append_item("", &item).unwrap(),
            format!("schedules:\n{item}")
        );
        assert_eq!(
            append_item("needs: [macos]\n", &item).unwrap(),
            format!("needs: [macos]\n\nschedules:\n{item}")
        );
        assert_eq!(
            append_item("schedules: []   # none yet\n", &item).unwrap(),
            format!("schedules: # none yet\n{item}")
        );
        assert_eq!(
            append_item("schedules:\nneeds: [macos]\n", &item).unwrap(),
            format!("schedules:\n{item}needs: [macos]\n")
        );
        let margin = "schedules:\n- name: one\n  every: 1d\n  brief: A.\n";
        let got = append_item(margin, &yaml_item(&entry("two"), 0)).unwrap();
        assert_eq!(
            got,
            "schedules:\n- name: one\n  every: 1d\n  brief: A.\n- name: two\n  title: Audit the dependencies\n  every: 1d\n  at: \"02:00\"\n  brief: Run the audit and fix what it names.\n"
        );
        assert_eq!(manifest::parse(&got).unwrap().schedules.len(), 2);
        assert!(append_item("schedules: [a, b]\n", &item).is_err());
        assert!(append_item("schedules:\n  key: value\n", &item).is_err());
    }

    #[test]
    fn every_words_say_the_cadence_the_way_the_console_does() {
        assert_eq!(every_words(1440, "02:00"), "every day at 02:00 UTC");
        assert_eq!(every_words(2 * 7 * 1440, ""), "every 2 weeks");
        assert_eq!(every_words(360, ""), "every 6 hours");
        assert_eq!(every_words(90, ""), "every 90 minutes");
        assert_eq!(every_words(60, ""), "every hour");
    }

    // ---- routes -------------------------------------------------------

    fn git_in(dir: &str, args: &[&str]) -> String {
        let mut full = vec!["-C", dir];
        full.extend_from_slice(args);
        pulse_testutil::git(&full).unwrap()
    }

    /// A project with a checkout whose `main` has scripts and no PULSE file.
    fn project_with_repo(store: &pulse_store::Store, slug: &str) -> String {
        http::additions_project(store, slug);
        let dir = Path::new(&store.home).join(slug).display().to_string();
        std::fs::create_dir_all(Path::new(&dir).join("scripts")).unwrap();
        std::fs::write(
            Path::new(&dir).join("scripts/check.sh"),
            "#!/bin/sh\nexit 0\n",
        )
        .unwrap();
        git_in(&dir, &["init", "-q", "-b", "main"]);
        git_in(&dir, &["config", "user.email", "t@example.com"]);
        git_in(&dir, &["config", "user.name", "t"]);
        git_in(&dir, &["add", "."]);
        git_in(&dir, &["commit", "-q", "-m", "scripts"]);
        store
            .conn()
            .unwrap()
            .execute(
                "UPDATE projects SET default_branch = 'main' WHERE slug = ?",
                [slug],
            )
            .unwrap();
        dir
    }

    fn post(store: &pulse_store::Store, path: &str, body: Value) -> (u16, Value) {
        let (code, map, _) =
            http::request(http::handler(store), "POST", path, &body.to_string(), &[]);
        (code, Value::Object(map.into_iter().collect()))
    }

    fn get(store: &pulse_store::Store, path: &str) -> (u16, Value) {
        let (code, map, _) = http::request(http::handler(store), "GET", path, "", &[]);
        (code, Value::Object(map.into_iter().collect()))
    }

    #[test]
    fn check_composes_the_yaml_and_names_what_the_daemon_would_refuse() {
        let t = http::additions_store(false);
        let s = &t.store;
        project_with_repo(s, "demo");
        let (code, body) = post(
            s,
            "/api/v1/automations/check",
            json!({"project": "demo", "entry": {"name": "deps-audit", "title": "Audit", "every": "1d", "at": "02:00", "brief": "Audit."}}),
        );
        assert_eq!(code, 200, "{body}");
        assert_eq!(
            body["yaml"],
            "  - name: deps-audit\n    title: Audit\n    every: 1d\n    at: \"02:00\"\n    brief: Audit.\n"
        );
        assert_eq!(body["problems"], json!([]));
        assert_eq!(body["path"], ".pulse/pulse.yaml");
        assert_eq!(body["branch"], "main");

        let (code, body) = post(
            s,
            "/api/v1/automations/check",
            json!({"project": "demo", "entry": {"name": "Bad Name", "every": "5m", "kind": "check", "brief": "x"}}),
        );
        assert_eq!(code, 200, "{body}");
        let problems: Vec<String> = serde_json::from_value(body["problems"].clone()).unwrap();
        assert!(
            problems.iter().any(|p| p.contains("not a slug")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("needs `script`")),
            "{problems:?}"
        );

        // The store's checks run when the static ones pass: a cadence under
        // the floor, a script the branch lacks.
        let (_, body) = post(
            s,
            "/api/v1/automations/check",
            json!({"project": "demo", "entry": {"name": "fast", "every": "5m", "brief": "x"}}),
        );
        let problems: Vec<String> = serde_json::from_value(body["problems"].clone()).unwrap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("5m") || problems[0].contains("30"),
            "{problems:?}"
        );
        let (_, body) = post(
            s,
            "/api/v1/automations/check",
            json!({"project": "demo", "entry": {"name": "missing", "every": "1d", "kind": "check", "script": "scripts/nope.sh"}}),
        );
        let problems: Vec<String> = serde_json::from_value(body["problems"].clone()).unwrap();
        assert!(
            problems.iter().any(|p| p.contains("scripts/nope.sh")),
            "{problems:?}"
        );

        let (code, body) = post(
            s,
            "/api/v1/automations/check",
            json!({"project": "demo", "entry": {"name": "x", "colour": "red"}}),
        );
        assert_eq!(code, 400, "{body}");
        assert_eq!(body["error"], "entry is not a schedule");
        let (code, _) = post(
            s,
            "/api/v1/automations/check",
            json!({"project": "demo", "entry": {"name": "x"}, "where": "cloud"}),
        );
        assert_eq!(code, 400);
    }

    #[test]
    fn adding_to_the_repository_commits_the_file_and_applies_it_at_once() {
        let _serial = crate::test_helpers::serial();
        let t = http::additions_store(false);
        let s = &t.store;
        let dir = project_with_repo(s, "demo");
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "where": "repo", "entry": {"name": "deps-audit", "title": "Audit the dependencies", "every": "1d", "at": "02:00", "brief": "Audit."}}),
        );
        assert_eq!(code, 201, "{body}");
        assert_eq!(body["where"], "repo");
        assert_eq!(body["path"], ".pulse/pulse.yaml");
        let commit = body["commit"].as_str().unwrap().to_string();
        assert_eq!(git_in(&dir, &["rev-parse", "HEAD"]).trim(), commit);
        let subject = git_in(&dir, &["log", "-1", "--format=%s"]);
        assert_eq!(
            subject.trim(),
            "pulse: add schedule deps-audit to .pulse/pulse.yaml"
        );
        let text = std::fs::read_to_string(Path::new(&dir).join(".pulse/pulse.yaml")).unwrap();
        assert_eq!(
            text,
            "schedules:\n  - name: deps-audit\n    title: Audit the dependencies\n    every: 1d\n    at: \"02:00\"\n    brief: Audit.\n"
        );
        assert_eq!(
            git_in(&dir, &["status", "--porcelain"]).trim(),
            "",
            "the checkout is clean after the commit"
        );
        // Applied at once: the schedule is in the store, kept by the file.
        let sched = &body["schedule"];
        assert_eq!(sched["origin"], "repo:.pulse/pulse.yaml#deps-audit");
        assert_eq!(sched["origin_name"], "deps-audit");
        assert_eq!(sched["every"], "1d");
        assert_eq!(sched["every_words"], "every day at 02:00 UTC");
        let id = sched["id"].as_str().unwrap().to_string();

        // A second entry appends; the same name again is refused before
        // anything is written.
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "entry": {"name": "deps-audit", "every": "1d", "brief": "Again."}}),
        );
        assert_eq!(code, 400, "{body}");
        assert!(
            body["why"]
                .as_str()
                .unwrap()
                .contains("already has an entry named `deps-audit`"),
            "{body}"
        );
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "entry": {"name": "nightly", "kind": "check", "script": "scripts/check.sh", "every": "daily", "job_kind": "improve", "paused": true}}),
        );
        assert_eq!(code, 201, "{body}");
        let text = std::fs::read_to_string(Path::new(&dir).join(".pulse/pulse.yaml")).unwrap();
        assert!(
            text.ends_with(
                "  - name: nightly\n    every: daily\n    kind: check\n    script: scripts/check.sh\n    job_kind: improve\n    paused: true\n"
            ),
            "{text}"
        );
        assert_eq!(body["schedule"]["status"], "paused");

        // An edit replaces the entry in place and keeps the row.
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "replace": "deps-audit", "entry": {"name": "deps-audit", "title": "Audit the dependencies", "every": "2d", "at": "03:00", "brief": "Audit, then fix."}}),
        );
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["schedule"]["id"], id);
        assert_eq!(body["schedule"]["every"], "2d");
        assert_eq!(body["schedule"]["at_time"], "03:00");
        let text = std::fs::read_to_string(Path::new(&dir).join(".pulse/pulse.yaml")).unwrap();
        assert!(
            text.starts_with(
                "schedules:\n  - name: deps-audit\n    title: Audit the dependencies\n    every: 2d\n    at: \"03:00\"\n    brief: Audit, then fix.\n  - name: nightly\n"
            ),
            "{text}"
        );
        // Nothing to change writes no commit.
        let head = git_in(&dir, &["rev-parse", "HEAD"]);
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "replace": "deps-audit", "entry": {"name": "deps-audit", "title": "Audit the dependencies", "every": "2d", "at": "03:00", "brief": "Audit, then fix."}}),
        );
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["commit"], Value::Null);
        assert_eq!(git_in(&dir, &["rev-parse", "HEAD"]), head);

        // The list shows the file, what was applied and the rows.
        let (code, body) = get(s, "/api/v1/automations?project=demo");
        assert_eq!(code, 200, "{body}");
        let p = &body["projects"][0];
        assert_eq!(p["slug"], "demo");
        assert_eq!(p["repo"]["branch"], "main");
        assert_eq!(p["file"]["path"], ".pulse/pulse.yaml");
        assert_eq!(p["file"]["entries"].as_array().unwrap().len(), 2);
        assert_eq!(p["file"]["problems"], json!([]));
        assert_eq!(p["applied"]["schedules"], json!(["deps-audit", "nightly"]));
        assert_eq!(p["schedules"].as_array().unwrap().len(), 2);
        assert_eq!(body["limits"]["max_per_project"], 10);

        // Ending a file-kept schedule takes it out of the file.
        let (code, body) = post(s, &format!("/api/v1/automations/{id}/end"), json!({}));
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["removed"], "deps-audit");
        assert_eq!(body["schedule"]["status"], "ended");
        assert_eq!(body["schedule"]["note"], manifest::ENDED_NOTE);
        let text = std::fs::read_to_string(Path::new(&dir).join(".pulse/pulse.yaml")).unwrap();
        assert!(
            text.starts_with("schedules:\n  - name: nightly\n"),
            "{text}"
        );
        let subject = git_in(&dir, &["log", "-1", "--format=%s"]);
        assert_eq!(
            subject.trim(),
            "pulse: remove schedule deps-audit from .pulse/pulse.yaml"
        );
        // And the last one leaves an empty list.
        let (code, body) = post(
            s,
            "/api/v1/automations/remove",
            json!({"project": "demo", "name": "nightly"}),
        );
        assert_eq!(code, 200, "{body}");
        let text = std::fs::read_to_string(Path::new(&dir).join(".pulse/pulse.yaml")).unwrap();
        assert_eq!(text, "schedules: []\n");
        let (code, body) = post(
            s,
            "/api/v1/automations/remove",
            json!({"project": "demo", "name": "nightly"}),
        );
        assert_eq!(code, 400, "{body}");
    }

    #[test]
    fn a_dirty_file_in_the_checkout_is_never_overwritten() {
        let _serial = crate::test_helpers::serial();
        let t = http::additions_store(false);
        let s = &t.store;
        let dir = project_with_repo(s, "demo");
        std::fs::create_dir_all(Path::new(&dir).join(".pulse")).unwrap();
        std::fs::write(
            Path::new(&dir).join(".pulse/pulse.yaml"),
            "schedules:\n  - name: draft\n    every: 1d\n    brief: Not committed yet.\n",
        )
        .unwrap();
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "entry": {"name": "deps-audit", "every": "1d", "brief": "Audit."}}),
        );
        assert_eq!(code, 400, "{body}");
        assert_eq!(body["error"], ".pulse/pulse.yaml has uncommitted changes");
        let text = std::fs::read_to_string(Path::new(&dir).join(".pulse/pulse.yaml")).unwrap();
        assert!(text.contains("Not committed yet."), "{text}");
        assert_eq!(git_in(&dir, &["log", "--format=%s"]).trim(), "scripts");
    }

    #[test]
    fn a_branch_checked_out_nowhere_is_written_through_the_index() {
        let _serial = crate::test_helpers::serial();
        let t = http::additions_store(false);
        let s = &t.store;
        let dir = project_with_repo(s, "demo");
        // The checkout sits on a feature branch; main is only a ref.
        git_in(&dir, &["checkout", "-q", "-b", "feature"]);
        std::fs::write(Path::new(&dir).join("wip.txt"), "work in progress\n").unwrap();
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "entry": {"name": "deps-audit", "every": "1d", "brief": "Audit."}}),
        );
        assert_eq!(code, 201, "{body}");
        let commit = body["commit"].as_str().unwrap().to_string();
        assert_eq!(git_in(&dir, &["rev-parse", "main"]).trim(), commit);
        assert_eq!(
            git_in(&dir, &["show", "main:.pulse/pulse.yaml"]),
            "schedules:\n  - name: deps-audit\n    every: 1d\n    brief: Audit.\n"
        );
        // The working tree and the feature branch are as they were.
        assert_eq!(
            git_in(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
            "feature"
        );
        assert!(!Path::new(&dir).join(".pulse/pulse.yaml").exists());
        assert_eq!(
            git_in(&dir, &["status", "--porcelain"]).trim(),
            "?? wip.txt"
        );
        assert_eq!(
            body["schedule"]["origin"],
            "repo:.pulse/pulse.yaml#deps-audit"
        );
    }

    #[test]
    fn keeping_it_in_the_store_adds_updates_moves_and_acts_like_the_cli() {
        let t = http::additions_store(false);
        let s = &t.store;
        let dir = project_with_repo(s, "demo");
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "where": "store", "entry": {"name": "sweep", "title": "Sweep the findings", "every": "1w", "brief": "Walk the open findings."}}),
        );
        assert_eq!(code, 201, "{body}");
        assert_eq!(body["where"], "store");
        let sched = body["schedule"].clone();
        assert_eq!(sched["origin"], "");
        assert_eq!(sched["created_by"], "human");
        assert_eq!(sched["every"], "1w");
        let id = sched["id"].as_str().unwrap().to_string();
        assert!(!Path::new(&dir).join(".pulse").exists());

        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "where": "store", "id": id, "entry": {"name": "sweep", "title": "Sweep the findings", "every": "2w", "brief": "Walk the open findings.", "paused": true}}),
        );
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["schedule"]["id"], id);
        assert_eq!(body["schedule"]["every"], "2w");
        assert_eq!(body["schedule"]["status"], "paused");

        for (action, status) in [
            ("resume", "active"),
            ("pause", "paused"),
            ("resume", "active"),
        ] {
            let (code, body) = post(
                s,
                &format!("/api/v1/automations/{id}/{action}"),
                json!({"note": "from the test"}),
            );
            assert_eq!(code, 200, "{action}: {body}");
            assert_eq!(body["schedule"]["status"], status);
        }
        let (code, body) = post(s, &format!("/api/v1/automations/{id}/run"), json!({}));
        assert_eq!(code, 200, "{body}");
        assert!(body["schedule"]["next_at"].as_str().unwrap() <= s.now().as_str());

        // Moving it into the file ends the hand-set row once the file holds it.
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "where": "repo", "move": id, "entry": {"name": "sweep", "title": "Sweep the findings", "every": "2w", "brief": "Walk the open findings."}}),
        );
        assert_eq!(code, 201, "{body}");
        assert_eq!(body["schedule"]["origin"], "repo:.pulse/pulse.yaml#sweep");
        let old = schedules::get(s, &id).unwrap();
        assert_eq!(old.status, "ended");
        assert_eq!(old.note, "moved to .pulse/pulse.yaml as sweep");
        let (code, body) = get(s, "/api/v1/automations");
        assert_eq!(code, 200);
        let rows = body["projects"][0]["schedules"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows.iter().filter(|r| r["status"] == "active").count(), 1);

        let (code, _) = post(s, "/api/v1/automations/SCHED-999/pause", json!({}));
        assert_eq!(code, 404);
        let (code, _) = get(s, "/api/v1/automations/SCHED-1/history");
        assert_eq!(code, 404);
    }

    #[test]
    fn a_project_without_a_checkout_can_only_keep_its_automations_in_the_store() {
        let t = http::additions_store(false);
        let s = &t.store;
        http::additions_project(s, "bare");
        let (code, body) = post(
            s,
            "/api/v1/automations/check",
            json!({"project": "bare", "entry": {"name": "sweep", "every": "1w", "brief": "Walk."}}),
        );
        assert_eq!(code, 200, "{body}");
        let problems: Vec<String> = serde_json::from_value(body["problems"].clone()).unwrap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("no main branch"), "{problems:?}");
        let (code, body) = post(
            s,
            "/api/v1/automations/check",
            json!({"project": "bare", "where": "store", "entry": {"name": "sweep", "every": "1w", "brief": "Walk."}}),
        );
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["problems"], json!([]));
        let (code, body) = get(s, "/api/v1/automations");
        assert_eq!(code, 200);
        assert_eq!(
            body["projects"],
            json!([]),
            "no schedules, no checkout: nothing to list"
        );
    }
}
