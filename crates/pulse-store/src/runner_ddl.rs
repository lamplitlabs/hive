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
    // The ledger's hot JSON reads: a project's landings and verify times
    // (the lease's lane count, `jobs::project_lanes`, on every lease), a
    // job's landing, deferral and recovery events, and a recovery chain by
    // its root. Without these each one walked every event of its action and
    // parsed the detail (2,600 landings, 4,000 recoveries on the measured
    // store of 515,000 events). Partial like `posts_mentioning`: only rows
    // whose detail is JSON naming the field are indexed, so a query uses one
    // when it repeats `json_valid(detail)` with an equality on the field.
    r####"CREATE INDEX IF NOT EXISTS events_detail_project ON events (action, json_extract(detail, '$.project')) WHERE json_valid(detail) AND json_extract(detail, '$.project') IS NOT NULL"####,
    r####"CREATE INDEX IF NOT EXISTS events_detail_job ON events (action, json_extract(detail, '$.job')) WHERE json_valid(detail) AND json_extract(detail, '$.job') IS NOT NULL"####,
    r####"CREATE INDEX IF NOT EXISTS events_detail_root ON events (action, json_extract(detail, '$.root')) WHERE json_valid(detail) AND json_extract(detail, '$.root') IS NOT NULL"####,
    // The worker pick behind every lease orders the free workers by their
    // last `job.lease` (`run::pick_worker_on`): 0.9 s a pick over 22,000
    // leases as a walk of the action, 15 ms from this index.
    r####"CREATE INDEX IF NOT EXISTS events_action_actor ON events (action, actor, seq)"####,
    // The pending-landing scan asks for a project's done jobs with a branch;
    // with branch in the index SQLite answers from the index entries instead
    // of reading each done job's row (brief included) to find none.
    r####"CREATE INDEX IF NOT EXISTS jobs_project_status_branch ON jobs (project, status, branch)"####,
    r####"CREATE INDEX IF NOT EXISTS agents_status_active ON agents (status, last_active_at DESC)"####,
    // ADR-070 step 2: a hand enrols once with a join code and then dials
    // out. The code is kept as its SHA-256 only, valid ten minutes, used
    // once (`used_at`, `device`).
    r####"CREATE TABLE IF NOT EXISTS hand_join_codes (
  code_hash TEXT NOT NULL,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  created_by TEXT NOT NULL DEFAULT '',
  used_at TEXT NOT NULL DEFAULT '',
  device TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (code_hash)
)"####,
    // The device registry: one row per enrolled hand, its public key (an
    // OpenSSH ed25519 key line), the `executors` row (kind `dial`) it works
    // as, and when it last dialled in. Revoking sets `revoked_at` and drops
    // the key, so the hand's next connect is refused.
    r####"CREATE TABLE IF NOT EXISTS hand_devices (
  id TEXT NOT NULL,
  executor TEXT NOT NULL,
  public_key TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  created_by TEXT NOT NULL DEFAULT '',
  last_seen_at TEXT NOT NULL DEFAULT '',
  revoked_at TEXT NOT NULL DEFAULT '',
  revoked_by TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (id)
)"####,
    // A connect is a signed answer to a nonce the brain handed out: each
    // nonce answers once and lapses after two minutes.
    r####"CREATE TABLE IF NOT EXISTS hand_nonces (
  nonce TEXT NOT NULL,
  device TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  PRIMARY KEY (nonce)
)"####,
    // A connected hand's session: the bearer token's SHA-256.
    r####"CREATE TABLE IF NOT EXISTS hand_sessions (
  token_hash TEXT NOT NULL,
  device TEXT NOT NULL,
  created_at TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  PRIMARY KEY (token_hash)
)"####,
    // The dial channel's mailbox: the brain queues one hands verb per row,
    // the hand's long poll takes it, runs it and posts the outcome back.
    // The hub and the daemon are two processes on one store, so the
    // mailbox is a table.
    r####"CREATE TABLE IF NOT EXISTS hand_calls (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  device TEXT NOT NULL,
  args TEXT NOT NULL,
  stdin BLOB NOT NULL DEFAULT x'',
  timeout_secs INTEGER NOT NULL DEFAULT 0,
  status TEXT NOT NULL DEFAULT 'queued',
  created_at TEXT NOT NULL,
  taken_at TEXT NOT NULL DEFAULT '',
  done_at TEXT NOT NULL DEFAULT '',
  rc INTEGER NOT NULL DEFAULT 0,
  stdout BLOB NOT NULL DEFAULT x'',
  stderr BLOB NOT NULL DEFAULT x''
)"####,
    r####"CREATE INDEX IF NOT EXISTS hand_calls_device_status ON hand_calls (device, status, id)"####,
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
    // What a run cost: the tokens and the provider's priced cost of the
    // harness sessions it ran (harness_sessions.run_id), summed onto the run
    // as it ends; zero for a worker that keeps no harness session.
    Column {
        table: r####"runs"####,
        name: r####"prompt_tokens"####,
        ddl: r####"INTEGER NOT NULL DEFAULT 0"####,
    },
    Column {
        table: r####"runs"####,
        name: r####"completion_tokens"####,
        ddl: r####"INTEGER NOT NULL DEFAULT 0"####,
    },
    Column {
        table: r####"runs"####,
        name: r####"cost"####,
        ddl: r####"REAL NOT NULL DEFAULT 0"####,
    },
    // The outcome code the worker's SUMMARY line opened with (done, blocked,
    // failed, nothing-to-do; empty without one) and what classified the run:
    // `code` when that code did, `text` when the runner fell back to reading
    // the summary's words, `none` when the worker wrote no summary.
    Column {
        table: r####"runs"####,
        name: r####"outcome"####,
        ddl: r####"TEXT NOT NULL DEFAULT ''"####,
    },
    Column {
        table: r####"runs"####,
        name: r####"outcome_source"####,
        ddl: r####"TEXT NOT NULL DEFAULT ''"####,
    },
];
