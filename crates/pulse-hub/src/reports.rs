//! Work reports: every task the fleet finished, with what landed for it and
//! the evidence it left, on one page of the hub (`/reports`).
//!
//! `GET /api/v1/reports[?project=SLUG][&limit=N]` lists the done tasks,
//! newest first, each with the jobs that carried it (`source = task:ID` or
//! `task:ID:label`, and the job the lead queued for it), and for each job:
//!
//! - its last run: the agent, the SUMMARY line, the check it ran (`verify`)
//!   and how many commits it made;
//! - its landing, from the landing ledger: a pull request merged
//!   (`land.merged`), a branch merged locally (`land.landed`) or a pull
//!   request still open (`land.pr`) - the commit, the pull request's number
//!   and link, and the check the landing ran;
//! - the release that carries the landed commit: the project's nearest tag
//!   that contains it (`git name-rev --tags`), empty while no tag does;
//! - its evidence: the files in `PULSE_HOME/work/jobs/<id>/artifacts`
//!   (screenshots, report.json, report.html, CSVs; served by
//!   `/api/v1/jobs/<id>/artifacts/<name>`) and the title, summary and checks
//!   of the agent's own report.json when it left one.
//!
//! Nothing here writes: the work report page itself (report.html) is built
//! when it is opened, by the job artifacts route.

use crate::httpx::{Request, ResponseWriter};
use crate::server::Handler;
use pulse_harness::artifacts::Artifact;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const REPORTS_DEFAULT_LIMIT: i64 = 50;
pub const REPORTS_MAX_LIMIT: i64 = 200;
/// A task's brief as the list shows it: the first few sentences.
const BRIEF_CHARS: usize = 400;
/// The most evidence files listed per job (the folder may hold many shots).
const ARTIFACTS_SHOWN: usize = 40;

/// The landing actions that say where a job's commits went, best first.
const LANDING_ACTIONS: [&str; 3] = ["land.merged", "land.landed", "land.pr"];

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ReportsScope {
    /// The project filter, "" for every project.
    pub project: String,
    pub limit: i64,
    /// How many done tasks there are in scope.
    pub total: i64,
    pub has_more: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ReportsPayload {
    pub scope: ReportsScope,
    pub reports: Vec<TaskReport>,
}

/// One done task and everything it delivered.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TaskReport {
    pub id: String,
    pub project: String,
    pub title: String,
    pub brief: String,
    pub priority: i64,
    pub created_by: String,
    pub created_at: String,
    /// When the task was last updated: for a done task, when it was done.
    pub done_at: String,
    pub jobs: Vec<JobReport>,
}

/// One job of a done task: its run, its landing, its release and evidence.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct JobReport {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub status: String,
    pub agent: String,
    pub finished_at: String,
    /// The worker's SUMMARY line from its last run.
    pub summary: String,
    /// The check the run ran before it finished ("verify exit 0").
    pub verify: String,
    pub commits: i64,
    pub landing: Option<LandingReport>,
    /// The release tag that carries the landed commit; "" when none does yet.
    pub release: String,
    pub artifacts: Vec<Artifact>,
    pub artifacts_total: usize,
    /// The agent's own report.json, when it left one.
    pub report: Option<ManifestBrief>,
}

/// Where a job's commits went, from the newest landing event of the best kind.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct LandingReport {
    /// land.merged, land.landed or land.pr.
    pub action: String,
    pub at: String,
    /// The commit on the default branch ("" for a pull request not yet merged
    /// whose merge commit has not been read).
    pub sha: String,
    /// The pull request's number and link (0 and "" for a local landing).
    pub number: i64,
    pub url: String,
    /// The check the landing ran on the merged tree.
    pub verify: String,
}

/// The headline of an agent's report.json.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ManifestBrief {
    pub title: String,
    pub summary: String,
    pub status: String,
    pub checks: Vec<String>,
    pub changes: usize,
}

impl Handler {
    pub(crate) fn addition_reports(&self, w: &mut ResponseWriter, r: &Request, parts: Vec<String>) {
        if parts.len() != 1 {
            crate::addition_not_found(w);
            return;
        }
        if !crate::addition_method(w, r, vec!["GET".to_string()])
            || !self.addition_available(
                w,
                r,
                vec![
                    "tasks".to_string(),
                    "task_revisions".to_string(),
                    "jobs".to_string(),
                    "runs".to_string(),
                    "events".to_string(),
                ],
            )
        {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            crate::addition_failure(w, anyhow::anyhow!("store is not available"));
            return;
        };
        let project = self.addition_project(r);
        if !self.valid_code_delivery_project(w, project.clone()) {
            return;
        }
        let (limit, ok) = crate::code_deliveries::code_delivery_positive_int(
            w,
            r.query("limit"),
            "limit".to_string(),
            REPORTS_DEFAULT_LIMIT,
            REPORTS_MAX_LIMIT,
        );
        if !ok {
            return;
        }
        match work_reports(store, &project, limit) {
            Ok(payload) => match serde_json::to_value(&payload) {
                Ok(v) => crate::write_json(w, 200, v),
                Err(err) => crate::addition_failure(w, anyhow::anyhow!(err)),
            },
            Err(err) => crate::addition_failure(w, err),
        }
    }
}

/// The done tasks in scope, newest first, with their jobs' landings,
/// releases and evidence.
pub fn work_reports(
    store: &pulse_store::Store,
    project: &str,
    limit: i64,
) -> anyhow::Result<ReportsPayload> {
    let limit = limit.clamp(1, REPORTS_MAX_LIMIT);
    let conn = store.conn()?;
    let total: i64 = conn.query_row(
        "SELECT count(*) FROM tasks WHERE status = 'done' AND (?1 = '' OR project = ?1)",
        [project],
        |r| r.get(0),
    )?;
    let mut tasks = done_tasks(&conn, project, limit)?;
    let landings = landing_index(&conn, project)?;
    for (task, lead_job) in &mut tasks {
        task.jobs = task_jobs(&conn, store, task, lead_job, &landings)?;
    }
    let mut reports: Vec<TaskReport> = tasks.into_iter().map(|(t, _)| t).collect();
    fill_releases(store, &mut reports);
    Ok(ReportsPayload {
        scope: ReportsScope {
            project: project.to_string(),
            limit,
            total,
            has_more: total > reports.len() as i64,
        },
        reports,
    })
}

/// The done tasks, newest first, each with the job its lead queued.
fn done_tasks(
    conn: &Connection,
    project: &str,
    limit: i64,
) -> anyhow::Result<Vec<(TaskReport, String)>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.project, t.title, rv.brief, t.priority, t.created_by, t.created_at, t.updated_at, t.job_id
         FROM tasks t LEFT JOIN task_revisions rv ON rv.task_id = t.id AND rv.rev = t.current_rev
         WHERE t.status = 'done' AND (?1 = '' OR t.project = ?1)
         ORDER BY t.updated_at DESC, t.id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![project, limit], |r| {
        Ok((
            TaskReport {
                id: r.get(0)?,
                project: r.get(1)?,
                title: r.get(2)?,
                brief: clip(
                    &r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    BRIEF_CHARS,
                ),
                priority: r.get(4)?,
                created_by: r.get(5)?,
                created_at: r.get(6)?,
                done_at: r.get(7)?,
                jobs: Vec::new(),
            },
            r.get::<_, String>(8)?,
        ))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// One landing event, as read from the ledger.
#[derive(Debug, Clone)]
struct Landing {
    seq: i64,
    action: String,
    at: String,
    detail: serde_json::Map<String, Value>,
}

impl Landing {
    fn str(&self, key: &str) -> String {
        self.detail
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    fn number(&self) -> i64 {
        self.detail
            .get("number")
            .and_then(Value::as_i64)
            .unwrap_or(0)
    }
}

/// The landing events in scope, by the jobs they name (`job` and `jobs`),
/// plus the merge commit of each merged pull request (`land.evidence`
/// reads it after the merge) by project and number.
struct LandingIndex {
    by_job: BTreeMap<String, Vec<Landing>>,
    merge_sha: BTreeMap<(String, i64), String>,
}

fn landing_index(conn: &Connection, project: &str) -> anyhow::Result<LandingIndex> {
    let mut stmt = conn.prepare(
        "SELECT seq, action, at, detail FROM events
         WHERE action IN ('land.merged', 'land.landed', 'land.pr', ?1)
           AND (?2 = '' OR (json_valid(detail) AND json_extract(detail, '$.project') = ?2))
         ORDER BY seq",
    )?;
    let rows = stmt.query_map(
        params![pulse_codedelivery::MERGE_EVIDENCE_ACTION, project],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        },
    )?;
    let mut index = LandingIndex {
        by_job: BTreeMap::new(),
        merge_sha: BTreeMap::new(),
    };
    for row in rows {
        let (seq, action, at, detail) = row?;
        let Ok(Value::Object(detail)) = serde_json::from_str::<Value>(&detail) else {
            continue;
        };
        let l = Landing {
            seq,
            action,
            at,
            detail,
        };
        if l.action == pulse_codedelivery::MERGE_EVIDENCE_ACTION {
            let sha = l.str("sha");
            if l.number() > 0 && !sha.is_empty() {
                index.merge_sha.insert((l.str("project"), l.number()), sha);
            }
            continue;
        }
        let mut jobs: BTreeSet<String> = BTreeSet::new();
        let job = l.str("job");
        if !job.is_empty() {
            jobs.insert(job);
        }
        if let Some(list) = l.detail.get("jobs").and_then(Value::as_array) {
            jobs.extend(list.iter().filter_map(Value::as_str).map(str::to_string));
        }
        for job in jobs {
            index.by_job.entry(job).or_default().push(l.clone());
        }
    }
    Ok(index)
}

/// The landing that says most about a job: merged, then landed, then an
/// open pull request - the newest of the first kind there is.
fn best_landing(found: &[Landing], index: &LandingIndex) -> Option<LandingReport> {
    let l = LANDING_ACTIONS.iter().find_map(|want| {
        found
            .iter()
            .filter(|l| l.action == *want)
            .max_by_key(|l| l.seq)
    })?;
    let mut sha = l.str("sha");
    if sha.is_empty() && l.action == "land.merged" {
        sha = index
            .merge_sha
            .get(&(l.str("project"), l.number()))
            .cloned()
            .unwrap_or_default();
    }
    Some(LandingReport {
        action: l.action.clone(),
        at: l.at.clone(),
        sha,
        number: l.number(),
        url: l.str("url"),
        verify: l.str("verify"),
    })
}

/// A task's jobs, oldest first: their sources name the task, or the lead
/// queued them for it.
fn task_jobs(
    conn: &Connection,
    store: &pulse_store::Store,
    task: &TaskReport,
    lead_job: &str,
    landings: &LandingIndex,
) -> anyhow::Result<Vec<JobReport>> {
    let (exact, like) = pulse_store::task_source_params(&task.id);
    let mut stmt = conn.prepare(&format!(
        "SELECT id, kind, title, status, finished_at FROM jobs
         WHERE project = ? AND ({} OR (? != '' AND id = ?)) ORDER BY created_at, id",
        pulse_store::task_source_clause("source")
    ))?;
    let rows = stmt.query_map(
        params![task.project, exact, like, lead_job, lead_job],
        |r| {
            Ok(JobReport {
                id: r.get(0)?,
                kind: r.get(1)?,
                title: r.get(2)?,
                status: r.get(3)?,
                finished_at: r.get(4)?,
                ..JobReport::default()
            })
        },
    )?;
    let mut jobs: Vec<JobReport> = rows.collect::<Result<_, _>>()?;
    let mut run = conn.prepare(
        "SELECT agent, summary, verify, commits FROM runs WHERE job_id = ?1
         ORDER BY started_at DESC, id DESC LIMIT 1",
    )?;
    for job in &mut jobs {
        if let Some((agent, summary, verify, commits)) = run
            .query_row([&job.id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .optional()?
        {
            job.agent = agent;
            job.summary = summary.trim().to_string();
            job.verify = verify.trim().to_string();
            job.commits = commits;
        }
        job.landing = landings
            .by_job
            .get(&job.id)
            .and_then(|found| best_landing(found, landings));
        let dir = pulse_harness::artifacts::job_artifacts_dir(&store.home, &job.id);
        let files = pulse_harness::artifacts::list(&dir);
        job.artifacts_total = files.len();
        job.artifacts = files.into_iter().take(ARTIFACTS_SHOWN).collect();
        job.report = pulse_harness::report::read_manifest(&dir)
            .ok()
            .flatten()
            .map(|m| ManifestBrief {
                title: m.title,
                summary: clip(&m.summary, BRIEF_CHARS),
                status: m.status,
                checks: m.checks,
                changes: m.changes.len(),
            });
    }
    Ok(jobs)
}

/// The release that carries each landed commit: per project, one
/// `git name-rev --tags` over every commit, the tag part of its answer
/// ("v5~3" is in v5). A project with no checkout or no tags leaves "".
fn fill_releases(store: &pulse_store::Store, reports: &mut [TaskReport]) {
    let mut wanted: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for t in reports.iter() {
        for j in &t.jobs {
            if let Some(l) = &j.landing
                && plain_sha(&l.sha)
                && l.action != "land.pr"
            {
                wanted
                    .entry(t.project.clone())
                    .or_default()
                    .insert(l.sha.clone());
            }
        }
    }
    let mut found: BTreeMap<(String, String), String> = BTreeMap::new();
    for (project, shas) in wanted {
        let Ok(Some(p)) = store.project(&project) else {
            continue;
        };
        if p.path.is_empty() {
            continue;
        }
        let mut args = vec!["name-rev", "--tags"];
        args.extend(shas.iter().map(String::as_str));
        let out = crate::board::git_value(&p.path, &args);
        for (sha, tag) in parse_name_rev(&out) {
            found.insert((project.clone(), sha), tag);
        }
    }
    for t in reports.iter_mut() {
        for j in &mut t.jobs {
            if let Some(l) = &j.landing
                && let Some(tag) = found.get(&(t.project.clone(), l.sha.clone()))
            {
                j.release = tag.clone();
            }
        }
    }
}

/// `git name-rev --tags` lines ("<sha> v5~3", "<sha> tags/v5^0",
/// "<sha> undefined") as (sha, tag) for the commits a tag carries.
pub(crate) fn parse_name_rev(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|line| {
            let (sha, name) = line.trim().split_once(' ')?;
            let name = name.trim();
            if name.is_empty() || name == "undefined" {
                return None;
            }
            let name = name.strip_prefix("tags/").unwrap_or(name);
            let tag = name.split(['~', '^']).next().unwrap_or_default();
            (!tag.is_empty()).then(|| (sha.to_string(), tag.to_string()))
        })
        .collect()
}

fn plain_sha(sha: &str) -> bool {
    (7..=64).contains(&sha.len()) && sha.chars().all(|c| c.is_ascii_hexdigit())
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers_http as http;
    use serde_json::json;
    use std::path::Path;

    fn git_in(dir: &str, args: &[&str]) -> String {
        let mut full = vec!["-C", dir];
        full.extend_from_slice(args);
        pulse_testutil::git(&full).unwrap()
    }

    fn job(s: &pulse_store::Store, id: &str, project: &str, source: &str, status: &str) {
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO jobs (id, project, kind, title, source, status, created_at, finished_at)
                 VALUES (?, ?, 'task', ?, ?, ?, ?, '2026-01-02T00:00:00Z')",
                params![id, project, format!("Job {id}"), source, status, format!("2026-01-01T00:00:0{}Z", id.len() % 10)],
            )
            .unwrap();
    }

    fn landing(s: &pulse_store::Store, action: &str, detail: Value) {
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO events (at, actor, action, subject, detail) VALUES ('2026-01-03T00:00:00Z', 'daemon', ?, 'b', ?)",
                params![action, detail.to_string()],
            )
            .unwrap();
    }

    #[test]
    fn name_rev_lines_read_as_tags() {
        let got =
            parse_name_rev("aaa1111 v5~3\nbbb2222 tags/v1.2^0\nccc3333 undefined\nddd4444 v2\n\n");
        assert_eq!(
            got,
            vec![
                ("aaa1111".to_string(), "v5".to_string()),
                ("bbb2222".to_string(), "v1.2".to_string()),
                ("ddd4444".to_string(), "v2".to_string()),
            ]
        );
    }

    #[test]
    fn done_tasks_list_their_landings_releases_and_evidence() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        http::additions_project(&s, "one");
        http::additions_project(&s, "two");
        // The project's checkout: a commit the v1 tag carries, then a newer
        // one no release carries yet.
        let dir = Path::new(&s.home).join("one").display().to_string();
        git_in(&dir, &["init", "-q", "-b", "main"]);
        git_in(&dir, &["config", "user.email", "t@example.com"]);
        git_in(&dir, &["config", "user.name", "t"]);
        git_in(&dir, &["commit", "-q", "--allow-empty", "-m", "first"]);
        let released = git_in(&dir, &["rev-parse", "HEAD"]).trim().to_string();
        git_in(&dir, &["tag", "v1"]);
        git_in(&dir, &["commit", "-q", "--allow-empty", "-m", "second"]);
        let unreleased = git_in(&dir, &["rev-parse", "HEAD"]).trim().to_string();

        let done =
            pulse_tasks::new(&s, "one", "Ship reports", "Show every report", 0, "human").unwrap();
        let later = pulse_tasks::new(&s, "one", "Polish", "the brief", 0, "human").unwrap();
        let open = pulse_tasks::new(&s, "one", "Still open", "the brief", 0, "human").unwrap();
        let other = pulse_tasks::new(&s, "two", "Elsewhere", "the brief", 0, "human").unwrap();
        for id in [&done.id, &later.id, &other.id] {
            s.conn()
                .unwrap()
                .execute("UPDATE tasks SET status = 'done' WHERE id = ?", [id])
                .unwrap();
        }
        for (id, at) in [
            (&done.id, "2026-01-04T00:00:00Z"),
            (&later.id, "2026-01-05T00:00:00Z"),
        ] {
            s.conn()
                .unwrap()
                .execute("UPDATE tasks SET updated_at = ? WHERE id = ?", [at, id])
                .unwrap();
        }
        job(&s, "job-a", "one", &format!("task:{}", done.id), "done");
        job(&s, "job-b", "one", &format!("task:{}:hub", done.id), "done");
        job(&s, "job-near", "one", &format!("task:{}0", done.id), "done");
        job(&s, "job-c", "one", &format!("task:{}", later.id), "done");
        job(
            &s,
            "job-open",
            "one",
            &format!("task:{}", open.id),
            "queued",
        );
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO runs (id, job_id, agent, project, started_at, status, summary, verify, commits)
                 VALUES ('run-a', 'job-a', 'iris', 'one', '2026-01-02T00:00:00Z', 'done', 'SUMMARY: done - built it', 'verify exit 0', 2)",
                [],
            )
            .unwrap();
        // job-a landed locally (released in v1), job-b's pull request merged
        // (its merge commit read afterwards, not released yet), job-c's pull
        // request is still open.
        landing(
            &s,
            "land.landed",
            json!({"project": "one", "job": "job-a", "sha": released, "verify": "verify exit 0"}),
        );
        landing(
            &s,
            "land.pr",
            json!({"project": "one", "number": 7, "url": "https://example.com/pr/7", "jobs": ["job-b"]}),
        );
        landing(
            &s,
            "land.merged",
            json!({"project": "one", "number": 7, "url": "https://example.com/pr/7", "jobs": ["job-b"]}),
        );
        landing(
            &s,
            pulse_codedelivery::MERGE_EVIDENCE_ACTION,
            json!({"project": "one", "number": 7, "sha": unreleased}),
        );
        landing(
            &s,
            "land.pr",
            json!({"project": "one", "number": 8, "url": "https://example.com/pr/8", "job": "job-c"}),
        );
        // The evidence job-a's worker left: a screenshot and its report.json.
        let evidence = pulse_harness::artifacts::job_artifacts_dir(&s.home, "job-a");
        std::fs::create_dir_all(&evidence).unwrap();
        std::fs::write(evidence.join("after-reports.png"), b"png").unwrap();
        std::fs::write(
            evidence.join("report.json"),
            json!({"title": "Reports page", "summary": "A page of reports.", "checks": ["npm test"], "changes": [{"title": "List"}]}).to_string(),
        )
        .unwrap();

        let (status, body) =
            http::additions_request(http::handler(&s), "GET", "/api/v1/reports?project=one", "");
        assert_eq!(status, 200, "{body:?}");
        assert_eq!(body["scope"]["total"], 2, "{body:?}");
        assert_eq!(body["scope"]["has_more"], false);
        let reports = body["reports"].as_array().unwrap();
        let ids: Vec<&str> = reports.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(
            ids,
            vec![later.id.as_str(), done.id.as_str()],
            "newest done first"
        );

        let polish = &reports[0]["jobs"][0];
        assert_eq!(polish["id"], "job-c");
        assert_eq!(polish["landing"]["action"], "land.pr");
        assert_eq!(polish["landing"]["number"], 8);
        assert_eq!(
            polish["release"], "",
            "an open pull request is in no release"
        );

        let jobs = reports[1]["jobs"].as_array().unwrap();
        let ids: Vec<&str> = jobs.iter().map(|j| j["id"].as_str().unwrap()).collect();
        assert_eq!(
            ids,
            vec!["job-a", "job-b"],
            "a longer task id's job is not this task's"
        );
        assert_eq!(jobs[0]["agent"], "iris");
        assert_eq!(jobs[0]["summary"], "SUMMARY: done - built it");
        assert_eq!(jobs[0]["verify"], "verify exit 0");
        assert_eq!(jobs[0]["commits"], 2);
        assert_eq!(jobs[0]["landing"]["action"], "land.landed");
        assert_eq!(jobs[0]["landing"]["sha"], released.as_str());
        assert_eq!(jobs[0]["landing"]["verify"], "verify exit 0");
        assert_eq!(jobs[0]["release"], "v1");
        assert_eq!(jobs[0]["artifacts_total"], 2);
        assert_eq!(jobs[0]["report"]["title"], "Reports page");
        assert_eq!(jobs[0]["report"]["checks"][0], "npm test");
        assert_eq!(jobs[0]["report"]["changes"], 1);
        assert_eq!(
            jobs[1]["landing"]["action"], "land.merged",
            "merged beats the pull request"
        );
        assert_eq!(jobs[1]["landing"]["sha"], unreleased.as_str());
        assert_eq!(jobs[1]["landing"]["url"], "https://example.com/pr/7");
        assert_eq!(jobs[1]["release"], "");
        assert!(jobs[1]["report"].is_null());
        assert_eq!(jobs[1]["artifacts"].as_array().unwrap().len(), 0);

        // Every project: the other project's done task too; limit pages.
        let (status, body) =
            http::additions_request(http::handler(&s), "GET", "/api/v1/reports?limit=1", "");
        assert_eq!(status, 200, "{body:?}");
        assert_eq!(body["scope"]["total"], 3);
        assert_eq!(body["scope"]["has_more"], true);
        assert_eq!(body["reports"].as_array().unwrap().len(), 1);

        for (path, want) in [
            ("/api/v1/reports?project=nope", 404),
            ("/api/v1/reports?limit=0", 400),
            ("/api/v1/reports?limit=999", 400),
            ("/api/v1/reports/extra", 404),
        ] {
            let (status, body) = http::additions_request(http::handler(&s), "GET", path, "");
            assert_eq!(status, want, "{path}: {body:?}");
        }
        let (status, _) =
            http::additions_request(http::handler(&s), "POST", "/api/v1/reports", "{}");
        assert_eq!(status, 405);
    }
}
