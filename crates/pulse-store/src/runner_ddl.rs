pub const RUNNER_DDL: &[&str] = &[
    r####"CREATE TABLE IF NOT EXISTS briefs (
  run_id TEXT NOT NULL,
  job_id TEXT NOT NULL,
  agent TEXT NOT NULL,
  text TEXT NOT NULL,
  at TEXT NOT NULL,
  PRIMARY KEY (run_id)
)"####,
    r####"CREATE TABLE IF NOT EXISTS compute_machines (
  request_id TEXT NOT NULL,
  provider TEXT NOT NULL,
  name TEXT NOT NULL,
  executor TEXT NOT NULL DEFAULT '',
  address TEXT NOT NULL DEFAULT '',
  detail TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL,
  released_at TEXT NOT NULL DEFAULT '',
  attempts INTEGER NOT NULL DEFAULT 0,
  note TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (request_id)
)"####,
    r####"CREATE TABLE IF NOT EXISTS retired_vms (
  executor TEXT NOT NULL,
  provider TEXT NOT NULL DEFAULT '',
  resource_group TEXT NOT NULL DEFAULT '',
  name TEXT NOT NULL DEFAULT '',
  deallocated_at TEXT NOT NULL DEFAULT '',
  delete_after TEXT NOT NULL DEFAULT '',
  deleted_at TEXT NOT NULL DEFAULT '',
  note TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (executor)
)"####,
    r####"CREATE INDEX IF NOT EXISTS events_action_subject ON events (action, subject, seq)"####,
    r####"CREATE INDEX IF NOT EXISTS events_subject_seq ON events (subject, seq)"####,
    r####"CREATE INDEX IF NOT EXISTS runs_agent_recent ON runs (agent, mode, started_at DESC, id DESC)"####,
    r####"CREATE INDEX IF NOT EXISTS runs_status_started ON runs (status, started_at DESC)"####,
    r####"CREATE INDEX IF NOT EXISTS runs_executor_status ON runs (executor, status, started_at DESC)"####,
    r####"CREATE INDEX IF NOT EXISTS jobs_source_status ON jobs (source, status, created_at DESC)"####,
    r####"CREATE INDEX IF NOT EXISTS jobs_source_status_kind_id ON jobs (source, status, kind, id)"####,
    r####"CREATE INDEX IF NOT EXISTS jobs_status_finished ON jobs (status, finished_at DESC, id)"####,
    r####"CREATE INDEX IF NOT EXISTS posts_kind_expires ON posts (kind, expires)"####,
    r####"CREATE INDEX IF NOT EXISTS posts_kind_date ON posts (kind, date DESC)"####,
    r####"CREATE INDEX IF NOT EXISTS posts_project_kind ON posts (project, kind, date DESC)"####,
    r####"CREATE INDEX IF NOT EXISTS posts_thread_answers ON posts (project, area, thread, agent, date DESC)"####,
    r####"CREATE INDEX IF NOT EXISTS posts_project_date ON posts (project, date)"####,
    r####"CREATE INDEX IF NOT EXISTS posts_project_area_thread_id ON posts (project, area, thread, id)"####,
    // "Did this run leave a suggestion card?" is a lookup by the post's
    // session; as a scan of every suggestion post it cost the planning pass
    // 28 of its 45 seconds on a store with 78,000 posts.
    r####"CREATE INDEX IF NOT EXISTS posts_session_kind ON posts (session, kind)"####,
    // The unanswered-mention check reads only the posts that carry a
    // `mentions` list in their extra (1,180 of 78,000 on the measured store),
    // so a partial index over exactly those spares it the JSON parse of
    // every other post; `mentions_for` repeats the index's WHERE terms
    // verbatim, which is what lets SQLite choose it.
    r####"CREATE INDEX IF NOT EXISTS posts_mentioning ON posts (project, date DESC) WHERE json_valid(extra) AND json_extract(extra, '$.mentions') IS NOT NULL"####,
    // Likewise for the questions one project addresses to another: the
    // open-asks query in `asks.rs` repeats these terms.
    r####"CREATE INDEX IF NOT EXISTS posts_asking ON posts (kind, date DESC) WHERE json_valid(extra) AND json_extract(extra, '$.projects') IS NOT NULL"####,
    // The pending-landing scan asks for a project's done jobs with a branch;
    // with branch in the index SQLite answers from the index entries instead
    // of reading each done job's row (brief included) to find none.
    r####"CREATE INDEX IF NOT EXISTS jobs_project_status_branch ON jobs (project, status, branch)"####,
    r####"CREATE INDEX IF NOT EXISTS agents_status_active ON agents (status, last_active_at DESC)"####,
];
use crate::Column;
pub const RUNNER_COLUMNS: &[Column] = &[
    Column {
        table: r####"executors"####,
        name: r####"admin"####,
        ddl: r####"TEXT NOT NULL DEFAULT ''"####,
    },
    Column {
        table: r####"executors"####,
        name: r####"healed_at"####,
        ddl: r####"TEXT NOT NULL DEFAULT ''"####,
    },
    Column {
        table: r####"retired_vms"####,
        name: r####"detail"####,
        ddl: r####"TEXT NOT NULL DEFAULT '{}'"####,
    },
    Column {
        table: r####"runs"####,
        name: r####"work_mb"####,
        ddl: r####"INTEGER NOT NULL DEFAULT 0"####,
    },
    // ADR-049: the agent-program variant a run ran under (a `variants.id`);
    // empty while `evolve.mode=fixtures`, which is every run until the owner
    // turns variants on.
    Column {
        table: r####"runs"####,
        name: r####"variant"####,
        ddl: r####"TEXT NOT NULL DEFAULT ''"####,
    },
];
