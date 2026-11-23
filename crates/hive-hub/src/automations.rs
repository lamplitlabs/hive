//! `/api/v1/automations`: the standing jobs of every project in one place -
//! the ones its repository keeps, one file each under `.hive/automations/`
//! (or an item of the older `.hive/hive.yaml` list), and the ones set by
//! hand - and the form that writes a new one. The console composes a YAML
//! entry; this module checks it with the code the daemon applies the files
//! with (`hive_tasks::manifest`, `hive_tasks::schedules`), commits it to
//! the project's default branch without touching any work in progress, and
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
use hive_tasks::manifest::{self, Entry, Manifest};
use hive_tasks::schedules::{self, Schedule, ScheduleInput};
use serde_json::{Value, json};

const ACTIONS: &[&str] = &["pause", "resume", "end", "run"];
/// History entries per page when the console names no limit.
const HISTORY_PAGE: usize = 50;

/// The identity a commit carries when the repository names none.
const FALLBACK_IDENTITY: (&str, &str) = ("Hive", "hive@localhost");

enum Route {
    List,
    Add,
    Check,
    Remove,
    History(String),
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
            3 if parts[2] == "history" => Route::History(parts[1].clone()),
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
            Route::History(_) => vec!["GET".to_string()],
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
            Route::History(id) => self.automations_history(w, r, &id),
            Route::Action(id, action) => self.automations_action(w, r, &id, &action),
        }
    }

    /// `GET /api/v1/automations/<id>/history?before=SEQ&limit=N`: what
    /// happened to one automation, newest first - its script runs with
    /// exit code, time and state, the jobs it queued or skipped and where
    /// they stand, its red/green turns, and every change by hand or by the
    /// file - with the last script output in full.
    fn automations_history(&self, w: &mut ResponseWriter, r: &Request, id: &str) {
        let Some(store) = self.store.as_deref() else {
            return;
        };
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
        let before = r.query("before").trim().parse::<i64>().unwrap_or(0);
        let limit = r
            .query("limit")
            .trim()
            .parse::<usize>()
            .unwrap_or(HISTORY_PAGE)
            .clamp(1, 500);
        // One more than asked tells the console whether older entries exist.
        let mut entries = match schedules::history(store, id, before, limit + 1) {
            Ok(v) => v,
            Err(e) => {
                addition_failure(w, e);
                return;
            }
        };
        let more = entries.len() > limit;
        entries.truncate(limit);
        let jobs = match schedules::jobs(store, id) {
            Ok(v) => v,
            Err(e) => {
                addition_failure(w, e);
                return;
            }
        };
        write_json(
            w,
            200,
            json!({
                "schedule": schedule_json(&sched),
                "entries": entries,
                "more": more,
                "jobs": jobs,
                "last_output": sched.last_output,
            }),
        );
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
        let who = hive_store::HUMAN_AGENT;
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
        match remove_from_repo(store, &project, name.trim(), hive_store::HUMAN_AGENT) {
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
        let who = hive_store::HUMAN_AGENT;
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

/// Whether the project's repository, on its default branch, still holds
/// an automation named `name` - its own file or an item of the list.
fn file_holds(store: &hive_store::Store, project: &str, name: &str) -> bool {
    match manifest::repo_of(store, project) {
        Ok(Some((repo, branch, _))) => RepoView::read(&repo, &branch).holder(name).is_some(),
        _ => false,
    }
}

/// One project's automations as the console lists them: its schedules
/// (with the cadence in words), the files on its default branch as they
/// are now, what the daemon last applied, and the names the form can
/// offer. `None` for a project with neither schedules nor a checkout here.
fn project_view(
    store: &hive_store::Store,
    p: &hive_store::Project,
) -> anyhow::Result<Option<Value>> {
    let rows = schedules::list(store, &p.slug, "")?;
    let repo = manifest::repo_of(store, &p.slug)?;
    if rows.is_empty() && repo.is_none() {
        return Ok(None);
    }
    let view = repo
        .as_ref()
        .map(|(path, branch, _)| RepoView::read(path, branch));
    let applied = manifest::status(store, &p.slug)?.map(|st| {
        json!({
            "path": st.path,
            "files": st.files,
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
        hive_harness::agents::load_agent_specs(&p.path, &store.home, Some(store), &p.slug)
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
        "file": view.as_ref().and_then(|v| v.list.as_ref()).map(RepoFile::json),
        "automations": view.as_ref().map(RepoView::files_json),
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
        m.insert("script_outside".into(), json!(s.script_outside()));
        m.insert(
            "script_home".into(),
            json!(if s.script_outside() {
                schedules::script_home(
                    manifest::origin_name(&s.origin).unwrap_or(&slug_of(&s.title, &s.id)),
                )
            } else {
                String::new()
            }),
        );
    }
    v
}

/// A file-name-shaped stem for a hand-set schedule: its title as a slug,
/// or its id.
fn slug_of(title: &str, id: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in title.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let out = out
        .trim_end_matches('-')
        .chars()
        .take(64)
        .collect::<String>();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        id.to_lowercase()
    } else {
        out
    }
}

/// The file path in a schedule's `origin` (`repo:<path>#<name>`).
fn origin_path(origin: &str) -> Option<&str> {
    manifest::origin_parts(origin).map(|(path, _)| path)
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

/// The list file of the default branch (`.hive/hive.yaml`), read the way
/// the daemon reads it.
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

/// `None` when the branch has no list file.
fn read_list(repo: &str, branch: &str) -> Option<RepoFile> {
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

/// Where an automation lives on the branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Holder {
    /// An item of the list file.
    List,
    /// Its own file, by index in `RepoView::files`.
    File(usize),
}

/// What the default branch keeps: the list file, when there is one, and
/// the per-automation files of `.hive/automations/`, each with the
/// problems the daemon would report on it.
struct RepoView {
    list: Option<RepoFile>,
    files: Vec<manifest::AutomationFile>,
}

impl RepoView {
    fn read(repo: &str, branch: &str) -> RepoView {
        let list = read_list(repo, branch);
        let mut files = manifest::read_automations(repo, branch);
        for f in &mut files {
            let Some(e) = &f.entry else {
                continue;
            };
            if let Some(l) = &list
                && l.entry(&f.name).is_some()
            {
                f.problems.push(format!(
                    "named in both {} and {}; keep one of them",
                    l.path, f.path
                ));
            }
            f.problems.extend(
                manifest::static_problems(&Manifest {
                    needs: None,
                    schedules: vec![e.clone()],
                })
                .iter()
                .map(|p| strip_label(p)),
            );
        }
        RepoView { list, files }
    }

    /// Where `name` lives, if anywhere: its own file first (a name in both
    /// is a problem the daemon holds), else the list.
    fn holder(&self, name: &str) -> Option<Holder> {
        if let Some(i) = self.files.iter().position(|f| f.name == name) {
            return Some(Holder::File(i));
        }
        self.list.as_ref()?.entry(name).map(|_| Holder::List)
    }

    /// The path that holds `name`.
    fn path_of(&self, name: &str) -> Option<&str> {
        match self.holder(name)? {
            Holder::List => self.list.as_ref().map(|l| l.path.as_str()),
            Holder::File(i) => Some(self.files[i].path.as_str()),
        }
    }

    /// Every name the branch holds: the list's, then the files'.
    fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = self.list.as_ref().map(RepoFile::names).unwrap_or_default();
        for f in &self.files {
            if !out.contains(&f.name) {
                out.push(f.name.clone());
            }
        }
        out
    }

    /// The `automations` key of a project: the folder and its files.
    fn files_json(&self) -> Value {
        json!({
            "dir": manifest::AUTOMATIONS_DIR,
            "files": self.files.iter().map(|f| json!({
                "path": f.path,
                "name": f.name,
                "text": f.text,
                "entry": f.entry,
                "problems": f.problems,
            })).collect::<Vec<_>>(),
        })
    }
}

/// What the console sends: the entry, where it goes, and what it replaces.
struct Draft {
    entry: Entry,
    /// `where: "repo"` (the default) writes the file; `"store"` keeps the
    /// schedule in the store alone, the way `hive schedule add` does.
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
                "The entry takes the fields of an automation: name, title, every, at, kind, brief, skill, script, job_kind, tier, priority, run_as, times, paused.".to_string(),
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
                    "An automation is kept in the repository (.hive/automations/<name>.yaml) or in the store."
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
    /// Where the entry goes on the branch: its own file under
    /// `.hive/automations/`, or the list file when it replaces an item
    /// there. The file form's path for a store-kept draft, as a preview.
    path: String,
    /// `yaml` is an item of the list file at `path`, not a whole file.
    in_list: bool,
    /// The branch as it is, when the draft goes there.
    view: Option<RepoView>,
    repo: Option<(String, String, String)>,
}

impl Checked {
    fn json(&self) -> Value {
        json!({
            "yaml": self.yaml,
            "problems": self.problems,
            "warning": self.warning,
            "path": self.path,
            "form": if self.in_list { "list" } else { "file" },
            "branch": self.repo.as_ref().map(|(_, b, _)| b.as_str()).unwrap_or(""),
        })
    }
}

/// Checks a draft the way `hive manifest check` checks a file: the YAML it
/// composes parses, the static checks pass, the store's checks pass (the
/// cadence, the skill, the custom agent, the script on the branch), and
/// the place it goes can take it.
fn check_draft(store: &hive_store::Store, project: &str, draft: &Draft) -> anyhow::Result<Checked> {
    let Some(p) = store.project(project)? else {
        anyhow::bail!(
            "No project {project:?}\n  why: it is not registered in this store\n  fix: pick a project of the list"
        );
    };
    let name = draft.entry.name.as_str();
    let mut problems = Vec::new();
    let mut warning = String::new();
    let mut path = manifest::automation_path(name);
    let mut in_list = false;
    let (repo, view) = if draft.in_repo {
        let repo = manifest::repo_of(store, &p.slug)?;
        match &repo {
            None => {
                problems.push(if p.path.trim().is_empty() || !Path::new(&p.path).is_dir() {
                    format!(
                        "{} has no checkout on this machine, so its files cannot be written here; keep the automation in the store instead",
                        p.slug
                    )
                } else {
                    format!(
                        "{} has no {} branch in {}, so its files cannot be written here; keep the automation in the store instead",
                        p.slug,
                        default_branch(&p),
                        p.path
                    )
                });
                (None, None)
            }
            Some((dir, branch, _)) => {
                let view = RepoView::read(dir, branch);
                // A list file that does not read may hold any name: nothing
                // is written beside it until it does.
                if let Some(l) = &view.list
                    && l.manifest.is_none()
                {
                    problems.extend(l.problems.iter().cloned());
                    problems.push(format!(
                        "fix {} by hand before the console writes to the repository",
                        l.path
                    ));
                }
                let taken = |problems: &mut Vec<String>| {
                    if let Some(held) = view.path_of(name) {
                        problems.push(format!(
                            "`{name}` is already an automation ({held}); pick another name"
                        ));
                    }
                };
                match &draft.replace {
                    Some(old) => match view.holder(old) {
                        None => problems.push(format!(
                            "{} on {branch} has no automation named `{old}` to replace",
                            p.slug
                        )),
                        Some(Holder::List) => {
                            in_list = true;
                            path = view
                                .list
                                .as_ref()
                                .map(|l| l.path.clone())
                                .unwrap_or_default();
                            if old != name {
                                taken(&mut problems);
                            }
                        }
                        Some(Holder::File(i)) => {
                            if old == name {
                                path = view.files[i].path.clone();
                            } else {
                                taken(&mut problems);
                            }
                        }
                    },
                    None => {
                        if let Some(held) = view.path_of(name) {
                            problems.push(format!(
                                "{held} already holds an automation named `{name}`; edit that one or pick another name"
                            ));
                        }
                    }
                }
                (repo, Some(view))
            }
        }
    } else {
        if let Some(id) = &draft.id {
            match schedules::get(store, id) {
                Ok(s) if s.project != p.slug => {
                    problems.push(format!("{id} belongs to {}, not {}", s.project, p.slug));
                }
                Ok(s) if s.from_repo() => problems.push(format!(
                    "{id} is kept by {}; edit it there",
                    origin_path(&s.origin).unwrap_or("the repository")
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
    // The YAML as it will be written, read back the way the daemon reads it.
    let yaml = if in_list {
        yaml_item(&draft.entry, 2)
    } else {
        yaml_file(&draft.entry)
    };
    let parsed = if in_list {
        match manifest::parse(&format!("schedules:\n{yaml}")) {
            Ok(m) => m.schedules.into_iter().next(),
            Err(e) => {
                problems.push(format!("the entry does not read back as YAML: {e}"));
                None
            }
        }
    } else {
        match manifest::parse_automation(name, &yaml) {
            Ok(e) => Some(e),
            Err(e) => {
                problems.push(format!("the file does not read back as YAML: {e}"));
                None
            }
        }
    };
    if let Some(e) = parsed {
        let statics = manifest::static_problems(&Manifest {
            needs: None,
            schedules: vec![e.clone()],
        });
        let held = !statics.is_empty();
        problems.extend(statics.iter().map(|p| strip_label(p)));
        if !held {
            let mut input = e.input(&p.slug, &path, hive_store::HUMAN_AGENT);
            if !draft.in_repo {
                input.origin = String::new();
            }
            // An edit may keep the script it already runs, even outside
            // .hive/; a new automation and a moved script follow the rule.
            let kept = kept_script(store, &p.slug, draft);
            match schedules::check_update(store, &input, &kept) {
                Ok(w) => warning = w,
                Err(err) => problems.push(first_line(&err.to_string())),
            }
        }
    }
    Ok(Checked {
        yaml,
        problems,
        warning,
        path,
        in_list,
        view,
        repo,
    })
}

/// The script the automation a draft edits already runs, or "" for a new
/// one: a store edit names its id, a repository edit the entry it replaces
/// (whose live row carries `origin repo:<path>#<name>`).
fn kept_script(store: &hive_store::Store, slug: &str, draft: &Draft) -> String {
    if let Some(id) = &draft.id
        && let Ok(s) = schedules::get(store, id)
        && s.project == slug
    {
        return s.script;
    }
    if let Some(old) = &draft.replace
        && let Ok(rows) = schedules::list(store, slug, "")
    {
        return rows
            .into_iter()
            .filter(|s| s.status != "ended")
            .find(|s| manifest::origin_name(&s.origin) == Some(old.as_str()))
            .map(|s| s.script)
            .unwrap_or_default();
    }
    String::new()
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

fn default_branch(p: &hive_store::Project) -> String {
    if p.default_branch.trim().is_empty() {
        "main".to_string()
    } else {
        p.default_branch.trim().to_string()
    }
}

/// Keeps the draft in the store alone: `hive schedule add`, or an update
/// of the schedule it names.
fn write_store(
    store: &hive_store::Store,
    project: &str,
    draft: &Draft,
    who: &str,
) -> anyhow::Result<Schedule> {
    let mut input: ScheduleInput = draft.entry.input(project, "", who);
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

/// Writes the draft to the default branch - its own file under
/// `.hive/automations/`, or in place of the list item it replaces -
/// commits it and applies the files. A rename moves the file in the same
/// commit.
fn write_repo(
    store: &hive_store::Store,
    project: &str,
    draft: &Draft,
    checked: &Checked,
    who: &str,
) -> anyhow::Result<Value> {
    let (Some((repo, branch, _)), Some(view)) = (&checked.repo, &checked.view) else {
        anyhow::bail!(
            "No checkout of {project} here\n  why: the files live in the repository, which this machine has no checkout of\n  fix: keep the automation in the store instead"
        );
    };
    let name = &draft.entry.name;
    let path = checked.path.clone();
    // What the commit changes: a text to write, or `None` to remove.
    let mut changes: Vec<(String, Option<String>)> = Vec::new();
    let (yaml, message) = if checked.in_list {
        let Some(list) = &view.list else {
            anyhow::bail!(
                "No list file in {project}\n  why: {branch} has no {path} any more\n  fix: refresh the list"
            );
        };
        let old = draft.replace.as_deref().unwrap_or(name);
        let indent = list_span(&list.text).map(|s| s.item_indent).unwrap_or(2);
        let item = yaml_item(&draft.entry, indent);
        let new_text = replace_item(&list.text, old, &item)?;
        let mut expect: BTreeSet<String> = list.names().into_iter().collect();
        expect.remove(old);
        expect.insert(name.clone());
        verify_names(&path, &new_text, &expect)?;
        if new_text != list.text {
            changes.push((path.clone(), Some(new_text)));
        }
        let message = if old == name {
            format!("hive: update automation {name} in {path}")
        } else {
            format!("hive: replace automation {old} with {name} in {path}")
        };
        (item, message)
    } else {
        let text = yaml_file(&draft.entry);
        verify_file(&path, name, &text)?;
        let old_path = draft
            .replace
            .as_deref()
            .and_then(|old| view.path_of(old))
            .map(str::to_string);
        if let Some(old) = &old_path
            && old != &path
        {
            changes.push((old.clone(), None));
        }
        let as_is = view
            .files
            .iter()
            .find(|f| f.path == path)
            .map(|f| f.text.as_str());
        if as_is != Some(text.as_str()) {
            changes.push((path.clone(), Some(text.clone())));
        }
        let message = match &draft.replace {
            Some(old) if old == name => format!("hive: update automation {name} ({path})"),
            Some(old) => format!("hive: rename automation {old} to {name} ({path})"),
            None => format!("hive: add automation {name} ({path})"),
        };
        (text, message)
    };
    let commit = if changes.is_empty() {
        None
    } else {
        Some(commit_files(repo, branch, &changes, &message)?)
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
        "yaml": yaml,
        "warning": checked.warning,
        "synced": synced.map(|s| json!({"line": s.line(), "problems": s.problems})),
        "schedule": schedule.map(|s| schedule_json(&s)),
    }))
}

/// Takes an automation out of the repository - its file removed, or its
/// item taken out of the list - commits and applies: the schedule ends
/// with the daemon's "removed" note.
fn remove_from_repo(
    store: &hive_store::Store,
    project: &str,
    name: &str,
    who: &str,
) -> anyhow::Result<Value> {
    let Some((repo, branch, _)) = manifest::repo_of(store, project)? else {
        anyhow::bail!(
            "No checkout of {project} here\n  why: the files live in the repository, which this machine has no checkout of\n  fix: remove the automation where the repository is checked out"
        );
    };
    let view = RepoView::read(&repo, &branch);
    let (path, changes, message) = match view.holder(name) {
        None => {
            let names = view.names();
            anyhow::bail!(
                "No automation {name} in {project}\n  why: {branch} holds {}\n  fix: refresh the list",
                if names.is_empty() {
                    "no automation".to_string()
                } else {
                    names.join(", ")
                }
            );
        }
        Some(Holder::File(i)) => {
            let path = view.files[i].path.clone();
            let message = format!("hive: remove automation {name} ({path})");
            (path.clone(), vec![(path, None)], message)
        }
        Some(Holder::List) => {
            let Some(list) = &view.list else {
                unreachable!("a list holder has a list")
            };
            if list.manifest.is_none() {
                anyhow::bail!(
                    "{} could not be read\n  why: {}\n  fix: fix the file by hand first",
                    list.path,
                    list.problems.join("; ")
                );
            }
            let new_text = remove_item(&list.text, name)?;
            let mut expect: BTreeSet<String> = list.names().into_iter().collect();
            expect.remove(name);
            verify_names(&list.path, &new_text, &expect)?;
            let message = format!("hive: remove automation {name} from {}", list.path);
            (
                list.path.clone(),
                vec![(list.path.clone(), Some(new_text))],
                message,
            )
        }
    };
    let commit = commit_files(&repo, &branch, &changes, &message)?;
    let synced = manifest::sync(store, project, who, true)?;
    Ok(json!({
        "where": "repo",
        "path": path,
        "branch": branch,
        "commit": commit,
        "removed": name,
        "synced": synced.map(|s| json!({"line": s.line(), "problems": s.problems})),
    }))
}

/// The merged list must read back with exactly the names expected; the
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

/// A per-automation file must read back as the automation it is meant to
/// hold and pass the daemon's static checks.
fn verify_file(path: &str, name: &str, text: &str) -> anyhow::Result<()> {
    let e = manifest::parse_automation(name, text).map_err(|e| {
        anyhow::anyhow!(
            "The file would not read back\n  why: {path}: {e}\n  fix: edit the file by hand; the console did not write it"
        )
    })?;
    let problems = manifest::static_problems(&Manifest {
        needs: None,
        schedules: vec![e],
    });
    if !problems.is_empty() {
        anyhow::bail!(
            "The file would not pass the daemon's checks\n  why: {path}: {}\n  fix: fix what the problems name; the console did not write it",
            problems
                .iter()
                .map(|p| strip_label(p))
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    Ok(())
}

// ---- YAML -------------------------------------------------------------

/// Words YAML reads as something other than a string when left bare.
const YAML_WORDS: &[&str] = &[
    "true", "false", "yes", "no", "on", "off", "null", "y", "n", "~",
];

/// A scalar as YAML: bare when it is plainly a word, a sentence or a path
/// (`.hive/automations/check.sh` included), else a double-quoted string
/// (JSON's quoting is valid YAML).
pub(crate) fn yaml_scalar(s: &str) -> String {
    let mut chars = s.chars();
    let starts_plain = match (chars.next(), chars.next()) {
        (Some(c), _) if c.is_ascii_alphanumeric() => true,
        (Some('.'), Some(c)) => c.is_ascii_alphabetic(),
        _ => false,
    };
    let plain = starts_plain
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

/// One `- name: ...` item of `schedules:`, its dash at `indent` spaces.
pub(crate) fn yaml_item(e: &Entry, indent: usize) -> String {
    format!(
        "{}- name: {}\n{}",
        " ".repeat(indent),
        yaml_scalar(&e.name),
        yaml_fields(e, indent + 2)
    )
}

/// A per-automation file, `.hive/automations/<name>.yaml`: the fields at
/// the top level; the file's name is the automation's.
pub(crate) fn yaml_file(e: &Entry) -> String {
    yaml_fields(e, 0)
}

/// The fields of an entry after its name, one `key: value` line each at
/// `indent` spaces, the defaults left out so the file stays as short as a
/// hand would write it. Fields come in the order the README's example uses.
fn yaml_fields(e: &Entry, indent: usize) -> String {
    let field = " ".repeat(indent);
    let mut out = String::new();
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
            let block = " ".repeat(indent + 2);
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
    /// Indent of the `- ` items (2 when there are none yet).
    item_indent: usize,
    /// Each item: its first line, the line after its last, its name.
    items: Vec<(usize, usize, String)>,
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
                item_indent: 2,
                items: Vec::new(),
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
            item_indent: 2,
            items: Vec::new(),
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
        } else {
            j += 1;
        }
    }
    Ok(Some(ListSpan {
        key,
        item_indent,
        items,
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
    let mut cmd = Command::new(hive_gitenv::git_program());
    cmd.arg("-C").arg(dir).args(args);
    cmd.env_clear();
    for kv in hive_gitenv::env() {
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
/// configuration); Hive's own when it gives none.
fn identity(repo: &str) -> (String, String) {
    let read = |key: &str| -> String {
        let mut cmd = Command::new(hive_gitenv::git_program());
        cmd.args(["-C", repo, "config", "--get", key]);
        for kv in std::env::vars().map(|(k, v)| format!("{k}={v}")) {
            if hive_gitenv::hook_var(&kv)
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

/// Commits `changes` - a text to write at each path, or `None` to remove
/// it - on `branch` of `repo` in one commit, without touching any work in
/// progress: through the checkout that has the branch out when the paths
/// are clean there, else straight into the branch with a temporary index.
/// Never pushes. Returns the new commit.
pub(crate) fn commit_files(
    repo: &str,
    branch: &str,
    changes: &[(String, Option<String>)],
    message: &str,
) -> anyhow::Result<String> {
    let (name, email) = identity(repo);
    let user_name = format!("user.name={name}");
    let user_email = format!("user.email={email}");
    let paths: Vec<&str> = changes.iter().map(|(p, _)| p.as_str()).collect();
    let refuse = |what: String, why: String, fix: &str| {
        anyhow::anyhow!("{what}\n  why: {why}\n  fix: {fix}")
    };
    if let Some(wt) = checkout_of(repo, branch) {
        let mut args = vec!["status", "--porcelain", "--untracked-files=all", "--"];
        args.extend(paths.iter().copied());
        let dirty = git(&wt, &args, &[], None).map_err(|e| {
            refuse(
                "The checkout could not be read".into(),
                e,
                "check the repository",
            )
        })?;
        if !dirty.trim().is_empty() {
            let mut named: Vec<&str> = paths
                .iter()
                .copied()
                .filter(|p| {
                    dirty
                        .lines()
                        .any(|l| l.get(3..).is_some_and(|rest| rest.ends_with(p)))
                })
                .collect();
            if named.is_empty() {
                named = paths.clone();
            }
            return Err(refuse(
                format!(
                    "{} {} uncommitted changes",
                    named.join(", "),
                    if named.len() == 1 { "has" } else { "have" }
                ),
                format!(
                    "the checkout of {branch} at {wt} has edits to the file the console would write"
                ),
                "commit or discard those edits first, then try again",
            ));
        }
        for (path, text) in changes {
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
                    // A folder the removal empties goes too: git keeps none.
                    if let Some(parent) = full.parent() {
                        let _ = std::fs::remove_dir(parent);
                    }
                }
            }
        }
        let mut args = vec!["add", "-A", "--"];
        args.extend(paths.iter().copied());
        git(&wt, &args, &[], None).map_err(|e| {
            refuse(
                "The file could not be staged".into(),
                e,
                "check the checkout",
            )
        })?;
        let mut args = vec![
            "-c",
            user_name.as_str(),
            "-c",
            user_email.as_str(),
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--no-verify",
            "-m",
            message,
            "--",
        ];
        args.extend(paths.iter().copied());
        git(&wt, &args, &[], None).map_err(|e| {
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
        "hive-index-{}-{}",
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
        for (path, text) in changes {
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
        assert_eq!(
            yaml_scalar(".hive/automations/check.sh"),
            ".hive/automations/check.sh"
        );
        assert_eq!(yaml_scalar(".inf"), "\".inf\"");
        assert_eq!(yaml_scalar(".5"), "\".5\"");
        assert_eq!(yaml_scalar(". a"), "\". a\"");
        assert_eq!(yaml_scalar("."), "\".\"");
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
            script: ".hive/automations/check.sh".to_string(),
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
            "  - name: nightly\n    title: Audit the dependencies\n    every: 1d\n    at: \"02:00\"\n    kind: check\n    brief: |-\n      First line.\n\n        indented second\n      third: with colon\n    script: .hive/automations/check.sh\n    job_kind: improve\n    tier: 0\n    priority: 2\n    run_as: release-manager\n    times: 5\n    paused: true\n"
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
    fn replace_and_remove_keep_the_rest_of_the_list_file_as_written() {
        let item = yaml_item(&entry("sweep"), 2);
        let replaced = replace_item(FILE, "deps-audit", &item).unwrap();
        assert_eq!(
            replaced,
            "# the project's file\nneeds: [macos]\nschedules:\n  - name: sweep\n    title: Audit the dependencies\n    every: 1d\n    at: \"02:00\"\n    brief: Run the audit and fix what it names.\n\n  - name: release-notes\n    skill: release\n    every: weekly\n\n# trailing words\n"
        );
        assert!(replace_item(FILE, "nope", &item).is_err());
        assert!(replace_item("schedules: [a, b]\n", "a", &item).is_err());
        assert!(replace_item("schedules:\n  key: value\n", "a", &item).is_err());

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
        let margin = "schedules:\n- name: one\n  every: 1d\n  brief: A.\n- name: two\n  every: 1d\n  brief: B.\n";
        assert_eq!(
            remove_item(margin, "one").unwrap(),
            "schedules:\n- name: two\n  every: 1d\n  brief: B.\n"
        );
    }

    #[test]
    fn yaml_file_puts_the_fields_at_the_top_level_and_reads_back_by_its_name() {
        let e = Entry {
            skill: "release".to_string(),
            brief: "Cut it.\nThen tell.".to_string(),
            ..entry("release-notes")
        };
        let text = yaml_file(&e);
        assert_eq!(
            text,
            "title: Audit the dependencies\nevery: 1d\nat: \"02:00\"\nbrief: |-\n  Cut it.\n  Then tell.\nskill: release\n"
        );
        assert!(!text.contains("name:"), "the file's name is the name");
        assert_eq!(
            manifest::parse_automation("release-notes", &text).unwrap(),
            e
        );
        // The file's stem wins; a different `name:` inside is refused.
        assert_eq!(
            manifest::parse_automation("other", &text).unwrap().name,
            "other"
        );
        let err =
            manifest::parse_automation("other", "name: release-notes\nevery: 1d\n").unwrap_err();
        assert!(err.contains("does not match the file name other"), "{err}");
        let err = manifest::parse_automation("x", "schedules:\n  - name: x\n").unwrap_err();
        assert!(err.contains("one automation per file"), "{err}");
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
        hive_testutil::git(&full).unwrap()
    }

    /// A project with a checkout whose `main` has scripts and no Hive file.
    fn project_with_repo(store: &hive_store::Store, slug: &str) -> String {
        http::additions_project(store, slug);
        let dir = Path::new(&store.home).join(slug).display().to_string();
        std::fs::create_dir_all(Path::new(&dir).join(".hive/automations")).unwrap();
        std::fs::write(
            Path::new(&dir).join(".hive/automations/check.sh"),
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

    fn post(store: &hive_store::Store, path: &str, body: Value) -> (u16, Value) {
        let (code, map, _) =
            http::request(http::handler(store), "POST", path, &body.to_string(), &[]);
        (code, Value::Object(map.into_iter().collect()))
    }

    fn get(store: &hive_store::Store, path: &str) -> (u16, Value) {
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
            "title: Audit\nevery: 1d\nat: \"02:00\"\nbrief: Audit.\n"
        );
        assert_eq!(body["problems"], json!([]));
        assert_eq!(body["path"], ".hive/automations/deps-audit.yaml");
        assert_eq!(body["form"], "file");
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
            json!({"project": "demo", "entry": {"name": "missing", "every": "1d", "kind": "check", "script": ".hive/automations/nope.sh"}}),
        );
        let problems: Vec<String> = serde_json::from_value(body["problems"].clone()).unwrap();
        assert!(
            problems
                .iter()
                .any(|p| p.contains(".hive/automations/nope.sh")),
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

    /// A new check's script lives under .hive/; a check set before the
    /// rule is listed with `script_outside` and the place its script
    /// belongs, keeps running, and may be edited as long as the script
    /// stays put - a move anywhere else outside .hive/ is refused.
    #[test]
    fn scripts_outside_hive_are_refused_for_new_automations_and_flagged_on_old_ones() {
        let _serial = crate::test_helpers::serial();
        let t = http::additions_store(false);
        let s = &t.store;
        let dir = project_with_repo(s, "demo");
        std::fs::create_dir_all(Path::new(&dir).join("scripts")).unwrap();
        std::fs::write(
            Path::new(&dir).join("scripts/site.sh"),
            "#!/bin/sh\nexit 0\n",
        )
        .unwrap();
        git_in(&dir, &["add", "."]);
        git_in(&dir, &["commit", "-q", "-m", "old script"]);
        let (_, body) = post(
            s,
            "/api/v1/automations/check",
            json!({"project": "demo", "entry": {"name": "site", "every": "1d", "kind": "check", "script": "scripts/site.sh"}}),
        );
        let problems: Vec<String> = serde_json::from_value(body["problems"].clone()).unwrap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("\"scripts/site.sh\" is outside .hive/"),
            "{problems:?}"
        );
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "where": "store", "entry": {"name": "site", "every": "1d", "kind": "check", "script": "scripts/site.sh"}}),
        );
        assert_eq!(code, 400, "{body}");
        assert!(
            body["why"].as_str().unwrap().contains("is outside .hive/"),
            "{body}"
        );

        // A row from before the rule.
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "where": "store", "entry": {"name": "site", "title": "Check the site", "every": "1d", "kind": "check", "script": ".hive/automations/check.sh"}}),
        );
        assert_eq!(code, 201, "{body}");
        let id = body["schedule"]["id"].as_str().unwrap().to_string();
        assert_eq!(body["schedule"]["script_outside"], false);
        assert_eq!(body["schedule"]["script_home"], "");
        s.conn()
            .unwrap()
            .execute(
                "UPDATE schedules SET script = 'scripts/site.sh' WHERE id = ?",
                [&id],
            )
            .unwrap();
        let (_, body) = get(s, "/api/v1/automations");
        let row = body["projects"][0]["schedules"][0].clone();
        assert_eq!(row["id"], id);
        assert_eq!(row["script_outside"], true);
        assert_eq!(row["script_home"], ".hive/automations/check-the-site.sh");

        // An edit that keeps the script passes the check and the write.
        let edit = json!({"project": "demo", "where": "store", "id": id, "entry": {"name": "site", "title": "Check the site", "every": "2d", "kind": "check", "script": "scripts/site.sh"}});
        let (code, body) = post(s, "/api/v1/automations/check", edit.clone());
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["problems"], json!([]));
        let (code, body) = post(s, "/api/v1/automations", edit);
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["schedule"]["every"], "2d");
        assert_eq!(body["schedule"]["script_outside"], true);
        // Another path outside .hive/ does not.
        let (_, body) = post(
            s,
            "/api/v1/automations/check",
            json!({"project": "demo", "where": "store", "id": id, "entry": {"name": "site", "title": "Check the site", "every": "2d", "kind": "check", "script": "scripts/other.sh"}}),
        );
        let problems: Vec<String> = serde_json::from_value(body["problems"].clone()).unwrap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("is outside .hive/"), "{problems:?}");
    }

    /// The history of one automation: its runs, the jobs it queued (with
    /// their status now), its turns and the changes by hand, newest first,
    /// paged by `before`, with the last output in full.
    #[test]
    fn history_lists_what_happened_to_one_automation_newest_first() {
        let _serial = crate::test_helpers::serial();
        let t = http::additions_store(false);
        let s = &t.store;
        project_with_repo(s, "demo");
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "where": "store", "entry": {"name": "site", "title": "Check the site", "every": "1d", "kind": "check", "script": ".hive/automations/check.sh"}}),
        );
        assert_eq!(code, 201, "{body}");
        let id = body["schedule"]["id"].as_str().unwrap().to_string();
        let sched = schedules::get(s, &id).unwrap();
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO jobs (id, project, kind, title, brief, source, status, created_at, created_by, lease_agent, finished_at) VALUES ('job-site-1', 'demo', 'task', 'Fix the site', 'fix', ?, 'done', ?, 'daemon', 'demo-dev', ?)",
                [&format!("{}{id}", schedules::SOURCE_PREFIX), &s.now(), &s.now()],
            )
            .unwrap();
        let run = schedules::ScriptRun {
            rc: Some(1),
            seconds: 2.0,
            output: "curl: (7) Failed to connect\n".into(),
            verdict: schedules::verdict(&sched, 1, ""),
            job: "job-site-1".into(),
            queued: true,
            post: "POST-7".into(),
            ..Default::default()
        };
        schedules::record_script_run(s, &sched, &s.now(), "daemon", &run).unwrap();
        let (code, body) = post(
            s,
            &format!("/api/v1/automations/{id}/pause"),
            json!({"note": "known"}),
        );
        assert_eq!(code, 200, "{body}");

        let (code, body) = get(s, &format!("/api/v1/automations/{id}/history"));
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["schedule"]["id"], id);
        assert_eq!(body["schedule"]["status"], "paused");
        assert_eq!(body["last_output"], "curl: (7) Failed to connect\n");
        assert_eq!(body["more"], false);
        let entries = body["entries"].as_array().unwrap();
        let whats: Vec<&str> = entries
            .iter()
            .map(|e| e["what"].as_str().unwrap())
            .collect();
        assert_eq!(whats, vec!["paused", "fired", "red", "ran", "added"]);
        assert_eq!(entries[0]["note"], "known");
        assert_eq!(entries[0]["actor"], "human");
        assert_eq!(entries[1]["job"], "job-site-1");
        assert_eq!(entries[1]["job_status"], "done");
        assert_eq!(entries[1]["job_agent"], "demo-dev");
        assert_eq!(entries[1]["queued"], true);
        assert_eq!(entries[2]["previous"], "");
        assert_eq!(entries[2]["state"], "red");
        assert_eq!(entries[2]["note"], "POST-7");
        assert_eq!(entries[3]["rc"], 1);
        assert_eq!(entries[3]["seconds"], 2.0);
        assert_eq!(entries[3]["run"], 1);
        assert_eq!(entries[4]["actor"], "human");
        assert_eq!(entries[4]["origin"], "", "a hand-set row names no file");
        let jobs = body["jobs"].as_array().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0]["id"], "job-site-1");

        // Paging: two at a time, then the rest from the oldest seen.
        let (_, body) = get(s, &format!("/api/v1/automations/{id}/history?limit=2"));
        assert_eq!(body["more"], true);
        let entries = body["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        let before = entries[1]["seq"].as_i64().unwrap();
        let (_, body) = get(
            s,
            &format!("/api/v1/automations/{id}/history?limit=50&before={before}"),
        );
        assert_eq!(body["more"], false);
        let whats: Vec<&str> = body["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["what"].as_str().unwrap())
            .collect();
        assert_eq!(whats, vec!["red", "ran", "added"]);

        let (code, _) = post(s, &format!("/api/v1/automations/{id}/history"), json!({}));
        assert_eq!(code, 405);
    }

    fn read(dir: &str, path: &str) -> String {
        std::fs::read_to_string(Path::new(dir).join(path)).unwrap()
    }

    #[test]
    fn adding_to_the_repository_commits_one_file_per_automation_and_applies_it_at_once() {
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
        assert_eq!(body["path"], ".hive/automations/deps-audit.yaml");
        let commit = body["commit"].as_str().unwrap().to_string();
        assert_eq!(git_in(&dir, &["rev-parse", "HEAD"]).trim(), commit);
        let subject = git_in(&dir, &["log", "-1", "--format=%s"]);
        assert_eq!(
            subject.trim(),
            "hive: add automation deps-audit (.hive/automations/deps-audit.yaml)"
        );
        assert_eq!(
            read(&dir, ".hive/automations/deps-audit.yaml"),
            "title: Audit the dependencies\nevery: 1d\nat: \"02:00\"\nbrief: Audit.\n"
        );
        assert!(
            !Path::new(&dir).join(".hive/hive.yaml").exists(),
            "no list file is written"
        );
        assert_eq!(
            git_in(&dir, &["status", "--porcelain"]).trim(),
            "",
            "the checkout is clean after the commit"
        );
        // Applied at once: the schedule is in the store, kept by the file.
        let sched = &body["schedule"];
        assert_eq!(
            sched["origin"],
            "repo:.hive/automations/deps-audit.yaml#deps-audit"
        );
        assert_eq!(sched["origin_name"], "deps-audit");
        assert_eq!(sched["origin_path"], ".hive/automations/deps-audit.yaml");
        assert_eq!(sched["every"], "1d");
        assert_eq!(sched["every_words"], "every day at 02:00 UTC");
        let id = sched["id"].as_str().unwrap().to_string();

        // A second automation is a second file; the same name again is
        // refused before anything is written.
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "entry": {"name": "deps-audit", "every": "1d", "brief": "Again."}}),
        );
        assert_eq!(code, 400, "{body}");
        assert!(
            body["why"].as_str().unwrap().contains(
                ".hive/automations/deps-audit.yaml already holds an automation named `deps-audit`"
            ),
            "{body}"
        );
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "entry": {"name": "nightly", "kind": "check", "script": ".hive/automations/check.sh", "every": "daily", "job_kind": "improve", "paused": true}}),
        );
        assert_eq!(code, 201, "{body}");
        assert_eq!(
            read(&dir, ".hive/automations/nightly.yaml"),
            "every: daily\nkind: check\nscript: .hive/automations/check.sh\njob_kind: improve\npaused: true\n"
        );
        assert_eq!(body["schedule"]["status"], "paused");

        // An edit rewrites the file and keeps the row.
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "replace": "deps-audit", "entry": {"name": "deps-audit", "title": "Audit the dependencies", "every": "2d", "at": "03:00", "brief": "Audit, then fix."}}),
        );
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["schedule"]["id"], id);
        assert_eq!(body["schedule"]["every"], "2d");
        assert_eq!(body["schedule"]["at_time"], "03:00");
        assert_eq!(
            read(&dir, ".hive/automations/deps-audit.yaml"),
            "title: Audit the dependencies\nevery: 2d\nat: \"03:00\"\nbrief: Audit, then fix.\n"
        );
        assert_eq!(
            git_in(&dir, &["log", "-1", "--format=%s"]).trim(),
            "hive: update automation deps-audit (.hive/automations/deps-audit.yaml)"
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

        // A rename moves the file in one commit: the old row ends, a new
        // one starts.
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "replace": "deps-audit", "entry": {"name": "audit", "title": "Audit the dependencies", "every": "2d", "at": "03:00", "brief": "Audit, then fix."}}),
        );
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["path"], ".hive/automations/audit.yaml");
        assert!(
            !Path::new(&dir)
                .join(".hive/automations/deps-audit.yaml")
                .exists()
        );
        assert!(
            Path::new(&dir)
                .join(".hive/automations/audit.yaml")
                .exists()
        );
        assert_eq!(
            git_in(&dir, &["log", "-1", "--format=%s"]).trim(),
            "hive: rename automation deps-audit to audit (.hive/automations/audit.yaml)"
        );
        assert_eq!(
            git_in(&dir, &["log", "--format=%s"]).lines().count(),
            5,
            "scripts, add, add, update, rename"
        );
        assert_eq!(schedules::get(s, &id).unwrap().status, "ended");
        assert_ne!(body["schedule"]["id"], id);
        assert_eq!(
            body["schedule"]["origin"],
            "repo:.hive/automations/audit.yaml#audit"
        );
        let id = body["schedule"]["id"].as_str().unwrap().to_string();

        // The list shows the folder's files, what was applied and the rows.
        let (code, body) = get(s, "/api/v1/automations?project=demo");
        assert_eq!(code, 200, "{body}");
        let p = &body["projects"][0];
        assert_eq!(p["slug"], "demo");
        assert_eq!(p["repo"]["branch"], "main");
        assert_eq!(p["file"], Value::Null);
        assert_eq!(p["automations"]["dir"], ".hive/automations");
        let files = p["automations"]["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0]["name"], "audit");
        assert_eq!(files[0]["path"], ".hive/automations/audit.yaml");
        assert_eq!(files[0]["entry"]["every"], "2d");
        assert_eq!(files[0]["problems"], json!([]));
        assert_eq!(files[1]["name"], "nightly");
        assert_eq!(p["applied"]["schedules"], json!(["audit", "nightly"]));
        assert_eq!(
            p["applied"]["files"],
            json!([
                ".hive/automations/audit.yaml",
                ".hive/automations/nightly.yaml"
            ])
        );
        assert_eq!(p["schedules"].as_array().unwrap().len(), 3);
        assert_eq!(body["limits"]["max_per_project"], 10);

        // Ending a file-kept schedule removes its file.
        let (code, body) = post(s, &format!("/api/v1/automations/{id}/end"), json!({}));
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["removed"], "audit");
        assert_eq!(body["path"], ".hive/automations/audit.yaml");
        assert_eq!(body["schedule"]["status"], "ended");
        assert_eq!(body["schedule"]["note"], manifest::ENDED_NOTE);
        assert!(
            !Path::new(&dir)
                .join(".hive/automations/audit.yaml")
                .exists()
        );
        assert_eq!(
            git_in(&dir, &["log", "-1", "--format=%s"]).trim(),
            "hive: remove automation audit (.hive/automations/audit.yaml)"
        );
        // And the last one leaves no folder behind.
        let (code, body) = post(
            s,
            "/api/v1/automations/remove",
            json!({"project": "demo", "name": "nightly"}),
        );
        assert_eq!(code, 200, "{body}");
        // The folder stays for the script that lives beside the files.
        assert!(
            !Path::new(&dir)
                .join(".hive/automations/nightly.yaml")
                .exists()
        );
        assert!(Path::new(&dir).join(".hive/automations/check.sh").exists());
        assert_eq!(git_in(&dir, &["status", "--porcelain"]).trim(), "");
        let (code, body) = post(
            s,
            "/api/v1/automations/remove",
            json!({"project": "demo", "name": "nightly"}),
        );
        assert_eq!(code, 400, "{body}");
        assert_eq!(body["error"], "No automation nightly in demo");
    }

    /// A repository from before the rename keeps its automations under
    /// `.pulse/automations`. The console lists them, an edit rewrites the
    /// file where it is, a new automation goes under `.hive/`, and a rename
    /// moves the file there.
    #[test]
    fn an_automation_under_the_older_folder_is_edited_where_it_is() {
        let _serial = crate::test_helpers::serial();
        let t = http::additions_store(false);
        let s = &t.store;
        let dir = project_with_repo(s, "demo");
        for (path, text) in [
            (
                ".pulse/automations/old-watch.yaml",
                "title: The older watch\nkind: watch\nscript: .pulse/automations/old-watch.sh\nevery: daily\nbrief: Follow the skill.\n",
            ),
            (".pulse/automations/old-watch.sh", "#!/bin/sh\nexit 0\n"),
        ] {
            let full = Path::new(&dir).join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }
        git_in(&dir, &["add", "."]);
        git_in(&dir, &["commit", "-q", "-m", "the older folder"]);
        manifest::sync(s, "demo", "human", true).unwrap().unwrap();

        // Listed with the path it has, and its script is in its place.
        let (code, body) = get(s, "/api/v1/automations");
        assert_eq!(code, 200, "{body}");
        let project = body["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["slug"] == "demo")
            .unwrap()
            .clone();
        assert_eq!(
            project["automations"]["files"][0]["path"],
            ".pulse/automations/old-watch.yaml"
        );
        assert_eq!(project["automations"]["dir"], ".hive/automations");
        let row = &project["schedules"][0];
        assert_eq!(
            row["origin"],
            "repo:.pulse/automations/old-watch.yaml#old-watch"
        );
        assert_eq!(row["script_outside"], false);
        let id = row["id"].as_str().unwrap().to_string();

        // The same name again is taken, wherever a new file would go.
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "entry": {"name": "old-watch", "every": "1d", "brief": "Again."}}),
        );
        assert_eq!(code, 400, "{body}");
        assert!(
            body["why"].as_str().unwrap().contains(
                ".pulse/automations/old-watch.yaml already holds an automation named `old-watch`"
            ),
            "{body}"
        );

        // An edit rewrites the file under the older folder and keeps the row.
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "replace": "old-watch", "entry": {"name": "old-watch", "title": "The older watch", "kind": "watch", "script": ".pulse/automations/old-watch.sh", "every": "weekly", "brief": "Follow the skill."}}),
        );
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["path"], ".pulse/automations/old-watch.yaml");
        assert_eq!(body["schedule"]["id"], id);
        assert_eq!(body["schedule"]["every"], "1w");
        assert_eq!(
            git_in(&dir, &["log", "-1", "--format=%s"]).trim(),
            "hive: update automation old-watch (.pulse/automations/old-watch.yaml)"
        );
        assert!(
            !Path::new(&dir)
                .join(".hive/automations/old-watch.yaml")
                .exists(),
            "an edit does not move the file"
        );

        // A new automation goes under .hive/, beside the older folder.
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "entry": {"name": "fresh", "every": "1d", "brief": "New work."}}),
        );
        assert_eq!(code, 201, "{body}");
        assert_eq!(body["path"], ".hive/automations/fresh.yaml");

        // A rename writes the new name under .hive/ and removes the old file.
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "replace": "old-watch", "entry": {"name": "new-watch", "title": "The older watch", "kind": "watch", "script": ".pulse/automations/old-watch.sh", "every": "weekly", "brief": "Follow the skill."}}),
        );
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["path"], ".hive/automations/new-watch.yaml");
        assert!(
            !Path::new(&dir)
                .join(".pulse/automations/old-watch.yaml")
                .exists()
        );
        assert!(
            Path::new(&dir)
                .join(".pulse/automations/old-watch.sh")
                .exists(),
            "the script it still runs stays where it is"
        );
        assert_eq!(
            body["schedule"]["script"],
            ".pulse/automations/old-watch.sh"
        );
        assert_eq!(body["schedule"]["script_outside"], false);
    }

    #[test]
    fn a_list_file_is_edited_in_place_and_new_automations_go_beside_it() {
        let _serial = crate::test_helpers::serial();
        let t = http::additions_store(false);
        let s = &t.store;
        let dir = project_with_repo(s, "demo");
        std::fs::create_dir_all(Path::new(&dir).join(".hive/automations")).unwrap();
        std::fs::write(Path::new(&dir).join(".hive/hive.yaml"), FILE).unwrap();
        // A file that doubles a list item, and one that does not read.
        std::fs::write(
            Path::new(&dir).join(".hive/automations/release-notes.yaml"),
            "skill: release\nevery: weekly\n",
        )
        .unwrap();
        std::fs::write(
            Path::new(&dir).join(".hive/automations/broken.yaml"),
            "every: [\n",
        )
        .unwrap();
        git_in(&dir, &["add", "."]);
        git_in(&dir, &["commit", "-q", "-m", "hive files"]);

        let (code, body) = get(s, "/api/v1/automations?project=demo");
        assert_eq!(code, 200, "{body}");
        let p = &body["projects"][0];
        assert_eq!(p["file"]["path"], ".hive/hive.yaml");
        assert_eq!(p["file"]["entries"].as_array().unwrap().len(), 2);
        let files = p["automations"]["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0]["name"], "broken");
        assert_eq!(files[0]["entry"], Value::Null);
        assert!(
            files[0]["problems"][0]
                .as_str()
                .unwrap()
                .starts_with(".hive/automations/broken.yaml could not be read"),
            "{files:?}"
        );
        assert_eq!(files[1]["name"], "release-notes");
        assert_eq!(
            files[1]["problems"],
            json!([
                "named in both .hive/hive.yaml and .hive/automations/release-notes.yaml; keep one of them"
            ])
        );

        // Editing a list item keeps it in the list, in place.
        let (code, body) = post(
            s,
            "/api/v1/automations/check",
            json!({"project": "demo", "replace": "deps-audit", "entry": {"name": "deps-audit", "every": "2d", "brief": "Audit twice."}}),
        );
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["form"], "list");
        assert_eq!(body["path"], ".hive/hive.yaml");
        assert_eq!(
            body["yaml"],
            "  - name: deps-audit\n    every: 2d\n    brief: Audit twice.\n"
        );
        assert_eq!(body["problems"], json!([]));
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "replace": "deps-audit", "entry": {"name": "deps-audit", "every": "2d", "brief": "Audit twice."}}),
        );
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["path"], ".hive/hive.yaml");
        assert!(
            read(&dir, ".hive/hive.yaml").starts_with(
                "# the project's file\nneeds: [macos]\nschedules:\n  - name: deps-audit\n    every: 2d\n    brief: Audit twice.\n\n  - name: release-notes\n"
            ),
            "{}",
            read(&dir, ".hive/hive.yaml")
        );
        assert_eq!(
            git_in(&dir, &["log", "-1", "--format=%s"]).trim(),
            "hive: update automation deps-audit in .hive/hive.yaml"
        );
        assert_eq!(
            body["schedule"]["origin"],
            "repo:.hive/hive.yaml#deps-audit"
        );

        // A new automation goes to its own file; a name the list holds is
        // taken, and so is one a file holds.
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "entry": {"name": "deps-audit", "every": "1d", "brief": "x"}}),
        );
        assert_eq!(code, 400, "{body}");
        assert!(
            body["why"]
                .as_str()
                .unwrap()
                .contains(".hive/hive.yaml already holds an automation named `deps-audit`"),
            "{body}"
        );
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "replace": "deps-audit", "entry": {"name": "broken", "every": "1d", "brief": "x"}}),
        );
        assert_eq!(code, 400, "{body}");
        assert!(
            body["why"]
                .as_str()
                .unwrap()
                .contains("`broken` is already an automation (.hive/automations/broken.yaml)"),
            "{body}"
        );
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "entry": {"name": "sweep", "every": "1w", "brief": "Sweep."}}),
        );
        assert_eq!(code, 201, "{body}");
        assert_eq!(body["path"], ".hive/automations/sweep.yaml");
        assert_eq!(
            read(&dir, ".hive/automations/sweep.yaml"),
            "every: 1w\nbrief: Sweep.\n"
        );
        assert!(read(&dir, ".hive/hive.yaml").contains("- name: deps-audit\n"));

        // Rewriting the broken file fixes it; the daemon then applies it.
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "replace": "broken", "entry": {"name": "broken", "every": "1d", "brief": "Fixed."}}),
        );
        assert_eq!(code, 200, "{body}");
        assert_eq!(
            body["schedule"]["origin"],
            "repo:.hive/automations/broken.yaml#broken"
        );

        // Removing a list item edits the list; the doubled name is held by
        // the daemon until one of its two places goes, and removing takes
        // the file first.
        let (code, body) = post(
            s,
            "/api/v1/automations/remove",
            json!({"project": "demo", "name": "deps-audit"}),
        );
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["path"], ".hive/hive.yaml");
        assert_eq!(
            git_in(&dir, &["log", "-1", "--format=%s"]).trim(),
            "hive: remove automation deps-audit from .hive/hive.yaml"
        );
        assert!(!read(&dir, ".hive/hive.yaml").contains("deps-audit"));
        let (code, body) = post(
            s,
            "/api/v1/automations/remove",
            json!({"project": "demo", "name": "release-notes"}),
        );
        assert_eq!(code, 200, "{body}");
        assert_eq!(body["path"], ".hive/automations/release-notes.yaml");
        assert!(
            !Path::new(&dir)
                .join(".hive/automations/release-notes.yaml")
                .exists()
        );
        assert!(read(&dir, ".hive/hive.yaml").contains("- name: release-notes\n"));
        let (code, body) = get(s, "/api/v1/automations?project=demo");
        assert_eq!(code, 200, "{body}");
        let p = &body["projects"][0];
        assert_eq!(
            p["automations"]["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["broken", "sweep"]
        );
        assert_eq!(
            p["applied"]["schedules"],
            json!(["release-notes", "broken", "sweep"])
        );
        // The list's own entry still names a skill the project lacks; the
        // daemon holds it and says so, the way it would for any file.
        assert_eq!(
            p["applied"]["problems"],
            json!(["schedule `release-notes`: No enabled skill \"release\" for demo"])
        );
        assert!(
            p["schedules"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["origin_name"] != "release-notes" || r["status"] == "ended"),
            "a held entry has no live row"
        );
    }

    #[test]
    fn a_dirty_file_in_the_checkout_is_never_overwritten() {
        let _serial = crate::test_helpers::serial();
        let t = http::additions_store(false);
        let s = &t.store;
        let dir = project_with_repo(s, "demo");
        std::fs::create_dir_all(Path::new(&dir).join(".hive/automations")).unwrap();
        std::fs::write(
            Path::new(&dir).join(".hive/automations/deps-audit.yaml"),
            "every: 1d\nbrief: Not committed yet.\n",
        )
        .unwrap();
        let (code, body) = post(
            s,
            "/api/v1/automations",
            json!({"project": "demo", "entry": {"name": "deps-audit", "every": "1d", "brief": "Audit."}}),
        );
        assert_eq!(code, 400, "{body}");
        assert_eq!(
            body["error"],
            ".hive/automations/deps-audit.yaml has uncommitted changes"
        );
        let text = read(&dir, ".hive/automations/deps-audit.yaml");
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
            git_in(&dir, &["show", "main:.hive/automations/deps-audit.yaml"]),
            "every: 1d\nbrief: Audit.\n"
        );
        // The working tree and the feature branch are as they were.
        assert_eq!(
            git_in(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
            "feature"
        );
        assert!(
            !Path::new(&dir)
                .join(".hive/automations/deps-audit.yaml")
                .exists()
        );
        assert_eq!(
            git_in(&dir, &["status", "--porcelain"]).trim(),
            "?? wip.txt"
        );
        assert_eq!(
            body["schedule"]["origin"],
            "repo:.hive/automations/deps-audit.yaml#deps-audit"
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
        assert!(
            !Path::new(&dir)
                .join(".hive/automations/sweep.yaml")
                .exists()
        );

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
        assert_eq!(
            body["schedule"]["origin"],
            "repo:.hive/automations/sweep.yaml#sweep"
        );
        let old = schedules::get(s, &id).unwrap();
        assert_eq!(old.status, "ended");
        assert_eq!(old.note, "moved to .hive/automations/sweep.yaml as sweep");
        let (code, body) = get(s, "/api/v1/automations");
        assert_eq!(code, 200);
        let rows = body["projects"][0]["schedules"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows.iter().filter(|r| r["status"] == "active").count(), 1);

        let (code, _) = post(s, "/api/v1/automations/SCHED-999/pause", json!({}));
        assert_eq!(code, 404);
        let (code, _) = get(s, "/api/v1/automations/SCHED-999/history");
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
