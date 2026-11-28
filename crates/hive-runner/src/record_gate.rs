//! Work that builds a decision record the owner has not accepted.
//!
//! A task whose work needs a Tier 2 decision gets its ADR drafted as a
//! Proposed record, and its pull request may be ready before the owner has
//! said yes. On 2026-11-27 the reviewer merged pull request 280 (TASK-141)
//! by itself although ADR-073, the decision it builds, was still Proposed:
//! the complexity rule knew nothing of records, so the brief said "You may
//! merge it yourself". [`reasons`] gives the rule that knowledge: a task
//! pull request whose task names a Tier 2 ADR the owner has not accepted
//! (or whose ADR names the task) goes to the owner.
//!
//! The other half is the owner's no. When the owner declines such a record
//! after its work was merged, [`tend_declined`] queues one job that takes
//! that work out (a revert, delivered as its own pull request), once per
//! record; [`merged_work`] is what the console shows before the owner picks
//! Decline, so the answer says what declining will do.

use serde_json::{Value, json};

use crate::util::Row;

/// The event that records the job queued to take a declined record's
/// merged work out; one per record, so a later pass queues no second job.
pub const REVERT_ACTION: &str = "record.revert";
/// The source prefix of that job: `revert:<slug>:<record>`.
pub const REVERT_SOURCE_PREFIX: &str = "revert:";
const REVERT_PRIORITY: i32 = 60;
const ACTOR: &str = "daemon";

/// The statuses of an ADR the owner has not accepted: still asked, put off,
/// or turned down. `Rejected` is said as declined.
fn unaccepted(status: &str) -> Option<&'static str> {
    let s = status.trim();
    if s.eq_ignore_ascii_case("Proposed") || s.eq_ignore_ascii_case("Deferred") {
        Some("which the owner has not accepted")
    } else if s.eq_ignore_ascii_case("Rejected") {
        Some("which the owner declined")
    } else {
        None
    }
}

/// Whether `text` names `id` as a whole word: `ADR-07` is not named by
/// `ADR-073`, nor `TASK-14` by `TASK-141`.
pub fn names(text: &str, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(i) = text[from..].find(id) {
        let start = from + i;
        let end = start + id.len();
        let before = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let after = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before && after {
            return true;
        }
        from = end;
    }
    false
}

/// The task's own words: its title and every revision of its brief.
fn task_text(s: &hive_store::Store, task: &str) -> String {
    let Ok(conn) = s.conn() else {
        return String::new();
    };
    let mut text = crate::util::query_one(&*conn, "SELECT title FROM tasks WHERE id = ?", &[&task])
        .ok()
        .flatten()
        .map(|r| r.str("title"))
        .unwrap_or_default();
    for r in crate::util::query_rows(
        &*conn,
        "SELECT title, brief FROM task_revisions WHERE task_id = ? ORDER BY rev",
        &[&task],
    )
    .unwrap_or_default()
    {
        text.push('\n');
        text.push_str(&r.str("title"));
        text.push('\n');
        text.push_str(&r.str("brief"));
    }
    text
}

fn body_of(s: &hive_store::Store, id: &str) -> String {
    s.get_record(id, 0)
        .ok()
        .and_then(|(_, rev)| rev)
        .map(|r| r.body)
        .unwrap_or_default()
}

/// Whether a record and a task belong together: the task names the record,
/// or the record names the task.
fn linked(s: &hive_store::Store, rec: &hive_store::Record, task: &str, text: &str) -> bool {
    names(text, &rec.id) || names(&body_of(s, &rec.id), task)
}

/// The Tier 2 ADRs of `project` the owner has not accepted that the task
/// builds, each with the words the reason uses.
pub fn unaccepted_records(
    s: &hive_store::Store,
    project: &str,
    task: &str,
) -> Vec<(hive_store::Record, &'static str)> {
    if project.is_empty() || task.is_empty() {
        return Vec::new();
    }
    let Ok(adrs) = s.list_records(hive_store::RecordFilter {
        project: project.into(),
        kind: "adr".into(),
        ..Default::default()
    }) else {
        return Vec::new();
    };
    let open: Vec<_> = adrs
        .into_iter()
        .filter(|r| r.tier == 2)
        .filter_map(|r| unaccepted(&r.status).map(|w| (r, w)))
        .collect();
    if open.is_empty() {
        return Vec::new();
    }
    let text = task_text(s, task);
    open.into_iter()
        .filter(|(r, _)| linked(s, r, task, &text))
        .collect()
}

/// The project and task a pull request delivers: the newest `land.pr` of
/// its number and head, else the task a `hive/task-<id>` head names.
fn pr_task(s: &hive_store::Store, pr: &crate::review::PrView) -> Option<(String, String)> {
    if pr.head.is_empty() {
        return None;
    }
    let conn = s.conn().ok()?;
    let row = crate::util::query_one(
        &*conn,
        "SELECT detail FROM events WHERE action = 'land.pr'
         AND json_extract(detail, '$.number') = ? AND json_extract(detail, '$.head') = ?
         ORDER BY seq DESC LIMIT 1",
        &[&pr.number, &pr.head],
    )
    .ok()
    .flatten();
    if let Some(row) = row {
        let d: Value = serde_json::from_str(&row.str("detail")).unwrap_or(Value::Null);
        let project = d["project"].as_str().unwrap_or_default().to_string();
        let task = d["task"].as_str().unwrap_or_default().to_string();
        if !project.is_empty() && !task.is_empty() {
            return Some((project, task));
        }
    }
    let task = pr.head.strip_prefix("hive/task-")?.to_string();
    let project =
        crate::util::query_one(&*conn, "SELECT project FROM tasks WHERE id = ?", &[&task])
            .ok()
            .flatten()?
            .str("project");
    (!project.is_empty()).then_some((project, task))
}

/// The complexity rule's reasons from records: `builds ADR-N, which the
/// owner has not accepted` for each Tier 2 ADR the pull request's task
/// builds while the owner has not said yes to it.
pub(crate) fn reasons(s: &hive_store::Store, pr: &crate::review::PrView) -> Vec<String> {
    let Some((project, task)) = pr_task(s, pr) else {
        return Vec::new();
    };
    unaccepted_records(s, &project, &task)
        .into_iter()
        .map(|(r, words)| format!("builds {}, {words}", r.id))
        .collect()
}

/// One merged pull request of a task that builds a record.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MergedPr {
    pub task: String,
    pub number: i64,
    pub url: String,
    pub head: String,
    /// When Hive recorded the merge (`land.merged`).
    pub at: String,
}

/// The tasks of the record's project the record and they name each other.
fn linked_tasks(s: &hive_store::Store, rec: &hive_store::Record) -> Vec<String> {
    let Ok(tasks) = hive_tasks::list(s, &rec.project, "") else {
        return Vec::new();
    };
    let body = body_of(s, &rec.id);
    tasks
        .into_iter()
        .filter(|t| names(&body, &t.id) || names(&task_text(s, &t.id), &rec.id))
        .map(|t| t.id)
        .collect()
}

/// The pull requests already merged that deliver the work of the tasks
/// building `rec`, oldest first. Empty for a record no merged work builds.
pub fn merged_work(s: &hive_store::Store, rec: &hive_store::Record) -> Vec<MergedPr> {
    if rec.kind != "adr" {
        return Vec::new();
    }
    let tasks = linked_tasks(s, rec);
    if tasks.is_empty() {
        return Vec::new();
    }
    let Ok(conn) = s.conn() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for r in crate::util::query_rows(
        &*conn,
        "SELECT at, detail FROM events WHERE action = 'land.merged' AND subject = ? ORDER BY seq",
        &[&rec.project],
    )
    .unwrap_or_default()
    {
        let d: Value = serde_json::from_str(&r.str("detail")).unwrap_or(Value::Null);
        let task = d["task"].as_str().unwrap_or_default();
        if task.is_empty() || !tasks.iter().any(|t| t == task) {
            continue;
        }
        let number = d["number"].as_i64().unwrap_or(0);
        if out.iter().any(|m: &MergedPr| m.number == number) {
            continue;
        }
        out.push(MergedPr {
            task: task.to_string(),
            number,
            url: d["url"].as_str().unwrap_or_default().to_string(),
            head: d["head"].as_str().unwrap_or_default().to_string(),
            at: r.str("at"),
        });
    }
    out
}

/// When the owner declined the record: the newest revision whose note
/// sets `Rejected`, else the record's last change.
fn declined_at(s: &hive_store::Store, rec: &hive_store::Record) -> String {
    s.revisions(&rec.id)
        .unwrap_or_default()
        .into_iter()
        .rev()
        .find(|r| r.note.trim_start().starts_with("status: Rejected"))
        .map(|r| r.at)
        .unwrap_or_else(|| rec.updated_at.clone())
}

fn revert_queued(s: &hive_store::Store, id: &str) -> bool {
    let Ok(conn) = s.conn() else {
        return true;
    };
    crate::util::query_one(
        &*conn,
        "SELECT 1 AS hit FROM events WHERE action = ? AND subject = ? LIMIT 1",
        &[&REVERT_ACTION, &id],
    )
    .ok()
    .flatten()
    .is_some()
}

fn revert_brief(rec: &hive_store::Record, work: &[MergedPr]) -> String {
    let prs = work
        .iter()
        .map(|m| {
            format!(
                "- #{} ({}, branch `{}`): {}",
                m.number, m.task, m.head, m.url
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "The owner declined {id} ({title}). The work that builds it was already merged:\n\n{prs}\n\n\
         Take that work out, and nothing else. For each pull request above, oldest last, find its merge commit on the default branch \
         (`git log --merges --oneline --grep 'Merge pull request #<n> '`) and run `git revert -m 1 <commit>`; \
         resolve a conflict with what was merged after it by keeping the later work. \
         Run the project's check and commit. Do not change {id} or any other record, and do not rebuild the work another way: \
         the decision is the owner's (`hive record show {id}` has the owner's words).",
        id = rec.id,
        title = rec.title,
    )
}

/// The daemon's pass over the records the owner declined: a Rejected ADR
/// whose work was merged before the decline gets one job that takes the
/// work out, recorded as `record.revert` so no pass queues a second one,
/// and the task hears of it.
pub(crate) fn tend_declined(s: &hive_store::Store, project: &Row, now: &str) -> Vec<String> {
    let slug = project.str("slug");
    let Ok(declined) = s.list_records(hive_store::RecordFilter {
        project: slug.clone(),
        kind: "adr".into(),
        status: "Rejected".into(),
        ..Default::default()
    }) else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for rec in declined {
        if revert_queued(s, &rec.id) {
            continue;
        }
        let when = declined_at(s, &rec);
        // Work merged after the no was merged by the owner's own hand: the
        // reviewer hands such a pull request over and may not merge it.
        let work: Vec<MergedPr> = merged_work(s, &rec)
            .into_iter()
            .filter(|m| m.at.as_str() <= when.as_str())
            .collect();
        if work.is_empty() {
            continue;
        }
        let numbers = work
            .iter()
            .map(|m| format!("#{}", m.number))
            .collect::<Vec<_>>()
            .join(", ");
        let job = match crate::jobs::add_job(
            s,
            slug.clone(),
            "improve".into(),
            format!(
                "Take out the merged work of {}, which the owner declined",
                rec.id
            ),
            revert_brief(&rec, &work),
            1,
            REVERT_PRIORITY,
            format!("{REVERT_SOURCE_PREFIX}{slug}:{}", rec.id),
            ACTOR.into(),
            true,
        ) {
            Ok((job, _)) => job,
            Err(e) => {
                lines.push(format!("records {slug}: {} declined; the job to take out {numbers} was not queued: {e}", rec.id));
                continue;
            }
        };
        let id = job.str("id");
        let tasks: Vec<&str> = work.iter().map(|m| m.task.as_str()).collect();
        if let Ok(conn) = s.conn() {
            crate::util::event_at(
                &*conn,
                now,
                ACTOR,
                REVERT_ACTION,
                &rec.id,
                json!({"project": slug, "record": rec.id, "job": id,
                    "prs": work.iter().map(|m| m.number).collect::<Vec<_>>(), "tasks": tasks}),
            );
        }
        let mut told: Vec<&str> = Vec::new();
        for t in tasks {
            if told.contains(&t) {
                continue;
            }
            told.push(t);
            let _ = hive_tasks::send_message(
                s,
                &slug,
                t,
                ACTOR,
                &format!(
                    "The owner declined {}. Its work was already merged ({numbers}), so job {id} takes it out as a new pull request.",
                    rec.id
                ),
            );
        }
        lines.push(format!(
            "records {slug}: {} declined; {id} queued to take out {numbers}",
            rec.id
        ));
    }
    lines
}

/// [`merged_work`] as the console reads it beside a record.
pub fn merged_work_json(s: &hive_store::Store, rec: &hive_store::Record) -> Value {
    json!(merged_work(s, rec))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::*;

    fn adr(s: &hive_store::Store, title: &str, body: &str, tier: i64) -> String {
        hive_records::new_record(
            s,
            "adr",
            "proj",
            title,
            hive_records::NewOptions {
                status: "Proposed".into(),
                areas: vec![title.to_lowercase().replace(' ', "-")],
                tier,
                tier_set: true,
                body: body.into(),
                body_set: true,
                author: "ember-93".into(),
                ..Default::default()
            },
        )
        .unwrap()
        .id
    }

    fn land(s: &hive_store::Store, action: &str, number: i64, task: &str, at: &str) {
        let conn = s.conn().unwrap();
        crate::util::event_at(
            &*conn,
            at,
            "daemon",
            action,
            "proj",
            json!({"project": "proj", "kind": "task", "number": number, "task": task,
                "head": format!("hive/task-{task}"), "base": "main",
                "url": format!("https://github.com/o/r/pull/{number}")}),
        );
    }

    fn view(number: i64, task: &str) -> crate::review::PrView {
        crate::review::PrView {
            number,
            head: format!("hive/task-{task}"),
            ..Default::default()
        }
    }

    #[test]
    fn names_a_whole_id_only() {
        assert!(names("builds ADR-073.", "ADR-073"));
        assert!(names("ADR-073", "ADR-073"));
        assert!(!names("ADR-0731 and XADR-073", "ADR-073"));
        assert!(names("see ADR-0731 then (ADR-073)", "ADR-073"));
        assert!(!names("TASK-1410", "TASK-141"));
        assert!(!names("anything", ""));
    }

    #[test]
    fn a_pull_request_building_a_proposed_tier_2_record_goes_to_the_owner() {
        let _serial = serial();
        let s = new_store();
        let (_proj, _) = new_repo(&s);
        // The record names the task (PR 280: ADR-073 names TASK-141).
        let named_by_record = hive_tasks::new(
            &s,
            "proj",
            "Show the lifecycle",
            "b",
            0,
            hive_store::HUMAN_AGENT,
        )
        .unwrap();
        let id = adr(
            &s,
            "Lifecycle",
            &format!(
                "# Lifecycle\n\nThe owner asked for it ({}).\n",
                named_by_record.id
            ),
            2,
        );
        land(&s, "land.pr", 280, &named_by_record.id, TEST_NOW);
        let reasons = crate::review::reasons_for(&s, &view(280, &named_by_record.id));
        assert_eq!(
            reasons,
            vec![format!("builds {id}, which the owner has not accepted")]
        );
        let brief = crate::review::review_brief(
            "proj",
            &crate::review::OpenPr {
                number: 280,
                ..Default::default()
            },
            &view(280, &named_by_record.id),
            &reasons,
            &[],
            "",
        );
        assert!(!brief.contains("You may merge it yourself"), "{brief}");

        // The task names the record, and the head alone finds the task.
        let names_record = hive_tasks::new(
            &s,
            "proj",
            "Build the other one",
            &format!("Waits on {id} for the owner's yes."),
            0,
            hive_store::HUMAN_AGENT,
        )
        .unwrap();
        assert_eq!(
            crate::review::reasons_for(&s, &view(281, &names_record.id)),
            vec![format!("builds {id}, which the owner has not accepted")]
        );

        // A task neither names nor is named by: no reason.
        let other =
            hive_tasks::new(&s, "proj", "Unrelated", "b", 0, hive_store::HUMAN_AGENT).unwrap();
        assert!(crate::review::reasons_for(&s, &view(282, &other.id)).is_empty());

        // A Tier 1 record is the agents' to build.
        let tier1 =
            hive_tasks::new(&s, "proj", "Small one", "b", 0, hive_store::HUMAN_AGENT).unwrap();
        adr(&s, "Small", &format!("For {}.", tier1.id), 1);
        assert!(crate::review::reasons_for(&s, &view(283, &tier1.id)).is_empty());

        // Once the owner accepts the record the reason is gone.
        hive_records::accept(&s, &id, "Yes, go ahead.").unwrap();
        assert!(crate::review::reasons_for(&s, &view(280, &named_by_record.id)).is_empty());
        assert!(crate::review::reasons_for(&s, &view(281, &names_record.id)).is_empty());
    }

    #[test]
    fn a_declined_record_with_merged_work_gets_one_job_that_takes_it_out() {
        let _serial = serial();
        let s = new_store();
        let (proj, _) = new_repo(&s);
        let task = hive_tasks::new(
            &s,
            "proj",
            "Show the lifecycle",
            "b",
            0,
            hive_store::HUMAN_AGENT,
        )
        .unwrap();
        let id = adr(&s, "Lifecycle", &format!("Asked in {}.", task.id), 2);
        land(&s, "land.pr", 280, &task.id, "2026-01-01T00:00:00Z");
        land(&s, "land.merged", 280, &task.id, "2026-01-01T00:00:00Z");

        // What the console shows before the owner picks Decline.
        let (rec, _) = s.get_record(&id, 0).unwrap();
        let work = merged_work(&s, &rec);
        assert_eq!(work.len(), 1);
        assert_eq!(work[0].number, 280);
        assert_eq!(work[0].task, task.id);

        // Proposed: nothing to take out yet.
        assert!(tend_declined(&s, &proj, TEST_NOW).is_empty());

        hive_records::status_note(
            &s,
            &id,
            "Rejected",
            hive_store::HUMAN_AGENT,
            "Not this way (via console)",
        )
        .unwrap();
        let lines = tend_declined(&s, &proj, TEST_NOW);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("to take out #280"), "{lines:?}");
        let jobs = crate::util::must_rows(
            &*s.conn().unwrap(),
            "SELECT id, kind, tier, title, brief FROM jobs WHERE source = ?",
            &[&format!("revert:proj:{id}")],
        );
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].str("kind"), "improve");
        assert_eq!(jobs[0].int("tier"), 1);
        assert!(jobs[0].str("brief").contains("git revert -m 1"));
        assert!(
            jobs[0]
                .str("brief")
                .contains("https://github.com/o/r/pull/280")
        );
        let told = hive_tasks::messages(&s, "proj", &task.id, "", "").unwrap();
        assert!(
            told.iter()
                .any(|m| m.body.contains(&jobs[0].str("id")) && m.body.contains("#280")),
            "{told:?}"
        );

        // One job per record: the next passes queue nothing, even once the
        // first job is done.
        assert!(tend_declined(&s, &proj, TEST_NOW).is_empty());
        crate::util::exec1(
            &*s.conn().unwrap(),
            "UPDATE jobs SET status = 'done' WHERE source = ?",
            &[&format!("revert:proj:{id}")],
        );
        assert!(tend_declined(&s, &proj, TEST_NOW).is_empty());

        // The reviewer hands over a pull request of a declined record too.
        assert_eq!(
            crate::review::reasons_for(&s, &view(280, &task.id)),
            vec![format!("builds {id}, which the owner declined")]
        );
    }

    #[test]
    fn a_declined_record_with_nothing_merged_queues_nothing() {
        let _serial = serial();
        let s = new_store();
        let (proj, _) = new_repo(&s);
        let task = hive_tasks::new(
            &s,
            "proj",
            "Show the lifecycle",
            "b",
            0,
            hive_store::HUMAN_AGENT,
        )
        .unwrap();
        let id = adr(&s, "Lifecycle", &format!("Asked in {}.", task.id), 2);
        land(&s, "land.pr", 280, &task.id, "2026-01-01T00:00:00Z");
        hive_records::status_note(&s, &id, "Rejected", hive_store::HUMAN_AGENT, "No").unwrap();
        assert!(tend_declined(&s, &proj, TEST_NOW).is_empty());
        // Merged after the no: the owner's own merge, not taken out.
        land(&s, "land.merged", 280, &task.id, "2099-01-01T00:00:00Z");
        assert!(tend_declined(&s, &proj, TEST_NOW).is_empty());
    }
}
