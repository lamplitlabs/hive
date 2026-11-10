use crate::store::Column;

pub const EVOLVE_DDL: &[&str] = &[
    r####"CREATE TABLE IF NOT EXISTS evolution_archive (
  id TEXT NOT NULL,
  project TEXT NOT NULL,
  suggestion_post TEXT NOT NULL DEFAULT '',
  hypothesis TEXT NOT NULL DEFAULT '',
  branch TEXT NOT NULL DEFAULT '',
  baseline_score REAL NOT NULL,
  variant_score REAL NOT NULL,
  tolerance REAL NOT NULL,
  kept INTEGER NOT NULL CHECK (kept IN (0, 1)),
  fixtures_json TEXT NOT NULL DEFAULT '[]',
  sandbox TEXT NOT NULL DEFAULT '',
  run_id TEXT NOT NULL DEFAULT '',
  at TEXT NOT NULL,
  note TEXT NOT NULL DEFAULT '',
  PRIMARY KEY (id, project)
)"####,
    r####"CREATE INDEX IF NOT EXISTS evolution_archive_project_at ON evolution_archive (project, at)"####,
    r####"CREATE TABLE IF NOT EXISTS evolution_baselines (
  project TEXT NOT NULL,
  "commit" TEXT NOT NULL,
  fixture_hash TEXT NOT NULL,
  score REAL NOT NULL,
  fixtures_json TEXT NOT NULL,
  run_id TEXT NOT NULL,
  at TEXT NOT NULL,
  PRIMARY KEY (project, "commit", fixture_hash)
)"####,
    r####"CREATE INDEX IF NOT EXISTS evolution_baselines_project_at ON evolution_baselines (project, at)"####,
    r####"CREATE TRIGGER IF NOT EXISTS evolution_archive_no_update BEFORE UPDATE ON evolution_archive
BEGIN SELECT RAISE(ABORT, 'archive rows are immutable'); END"####,
    r####"CREATE TRIGGER IF NOT EXISTS evolution_archive_no_delete BEFORE DELETE ON evolution_archive
BEGIN SELECT RAISE(ABORT, 'archive rows are immutable'); END"####,
    r####"CREATE TABLE IF NOT EXISTS evolution_fixtures (
  id TEXT NOT NULL,
  project TEXT NOT NULL,
  name TEXT NOT NULL,
  request TEXT NOT NULL,
  scorer_cmd TEXT NOT NULL,
  weight REAL NOT NULL DEFAULT 1,
  kind TEXT NOT NULL DEFAULT 'public' CHECK (kind IN ('public', 'signal')),
  enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  created_by TEXT NOT NULL,
  at TEXT NOT NULL,
  PRIMARY KEY (id),
  UNIQUE (project, name)
)"####,
    r####"CREATE INDEX IF NOT EXISTS evolution_fixtures_project_enabled ON evolution_fixtures (project, enabled, name)"####,
    r####"CREATE TABLE IF NOT EXISTS evolution_fixture_keys (
  fixture_id TEXT NOT NULL REFERENCES evolution_fixtures(id),
  path TEXT NOT NULL,
  PRIMARY KEY (fixture_id, path)
)"####,
    r####"CREATE TABLE IF NOT EXISTS evolution_fleets (
  id TEXT PRIMARY KEY,
  project TEXT NOT NULL,
  ref TEXT NOT NULL,
  workers INTEGER NOT NULL CHECK (workers > 0),
  stripped_json TEXT NOT NULL,
  at TEXT NOT NULL,
  created_by TEXT NOT NULL,
  cleaned_at TEXT NOT NULL DEFAULT ''
)"####,
    r####"CREATE INDEX IF NOT EXISTS evolution_fleets_project_at ON evolution_fleets (project, at)"####,
    r####"CREATE TRIGGER IF NOT EXISTS evolution_fixtures_no_delete BEFORE DELETE ON evolution_fixtures
BEGIN SELECT RAISE(ABORT, 'fixtures are never deleted; disable instead'); END"####,
    r####"CREATE TRIGGER IF NOT EXISTS evolution_fleets_no_delete BEFORE DELETE ON evolution_fleets
BEGIN SELECT RAISE(ABORT, 'fleet history is never deleted; clean the clones instead'); END"####,
    r####"CREATE TABLE IF NOT EXISTS evolution_cycle_leases (
  project TEXT PRIMARY KEY,
  owner TEXT NOT NULL,
  expires_at TEXT NOT NULL
)"####,
    // ADR-049 / PLAN-050: the agent-program population. A variant is an
    // overlay on the agent program (agents/, skills/, instructions.md,
    // allowlisted settings.toml under `dir`); the archive is append-only and
    // never pruned, so a retired variant stays a stepping stone. Scopes and
    // evidence are JSON arrays. Which variant a project runs is the pointer
    // setting `evolve.incumbent.<slug>` (empty or unset: `base`), not a state
    // here. Nothing reads or writes these rows while `evolve.mode=fixtures`.
    r####"CREATE TABLE IF NOT EXISTS variants (
  id TEXT PRIMARY KEY,
  project_scope TEXT NOT NULL DEFAULT '[]',
  kinds_scope TEXT NOT NULL DEFAULT '[]',
  parent TEXT NOT NULL DEFAULT '',
  hypothesis TEXT NOT NULL DEFAULT '',
  state TEXT NOT NULL DEFAULT 'proposed' CHECK (state IN ('proposed', 'replay', 'canary', 'promoted', 'retired')),
  proposer_job TEXT NOT NULL DEFAULT '',
  evidence TEXT NOT NULL DEFAULT '[]',
  dir TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  retired_at TEXT NOT NULL DEFAULT '',
  promoted_at TEXT NOT NULL DEFAULT ''
)"####,
    r####"CREATE INDEX IF NOT EXISTS variants_state ON variants (state, created_at)"####,
    r####"CREATE INDEX IF NOT EXISTS variants_parent ON variants (parent)"####,
    r####"CREATE TRIGGER IF NOT EXISTS variants_no_delete BEFORE DELETE ON variants
BEGIN SELECT RAISE(ABORT, 'variants are never deleted; retire instead'); END"####,
    // The outcome vector of one variant on one island (a project slug, or
    // `replay` for the replay cascade) for one job kind, recomputed by the
    // daemon from runs, harness sessions, jobs, landings and reverts:
    // trials, landed, kept (landed and not reverted after evolve.keep_days),
    // tokens, failures (and by cause, a JSON object), owner interventions,
    // verify flakes, hard-stop breaches, the posterior of kept landings per
    // 1e6 tokens (JSON) and the last failed trials' result lines (JSON).
    r####"CREATE TABLE IF NOT EXISTS variant_outcomes (
  variant TEXT NOT NULL REFERENCES variants(id),
  island TEXT NOT NULL,
  kind TEXT NOT NULL DEFAULT '',
  trials INTEGER NOT NULL DEFAULT 0,
  landed INTEGER NOT NULL DEFAULT 0,
  kept INTEGER NOT NULL DEFAULT 0,
  tokens INTEGER NOT NULL DEFAULT 0,
  failed INTEGER NOT NULL DEFAULT 0,
  failed_by_cause TEXT NOT NULL DEFAULT '{}',
  interventions INTEGER NOT NULL DEFAULT 0,
  flakes INTEGER NOT NULL DEFAULT 0,
  breaches INTEGER NOT NULL DEFAULT 0,
  posterior_json TEXT NOT NULL DEFAULT '{}',
  feedback TEXT NOT NULL DEFAULT '[]',
  computed_at TEXT NOT NULL,
  PRIMARY KEY (variant, island, kind)
)"####,
    r####"CREATE INDEX IF NOT EXISTS variant_outcomes_island ON variant_outcomes (island, kind)"####,
];

/// Columns added after the evolution tables shipped. `ensure_records` adds
/// each one that is missing, so an older store gains them on first open.
pub const EVOLVE_COLUMNS: &[Column] = &[
    // A metric fixture scores the tree itself - no agent request runs first;
    // its scorer prints a continuous value in [0,1] with headroom on both
    // sides of the baseline.
    Column {
        table: r####"evolution_fixtures"####,
        name: r####"metric"####,
        ddl: r####"INTEGER NOT NULL DEFAULT 0 CHECK (metric IN (0, 1))"####,
    },
    // Lineage: the archive row this experiment was built on (empty: the
    // default branch), the patch a discarded variant left behind so a later
    // experiment can build on it, and the default-branch commit it forked from.
    Column {
        table: r####"evolution_archive"####,
        name: r####"parent"####,
        ddl: r####"TEXT NOT NULL DEFAULT ''"####,
    },
    Column {
        table: r####"evolution_archive"####,
        name: r####"patch"####,
        ddl: r####"TEXT NOT NULL DEFAULT ''"####,
    },
    Column {
        table: r####"evolution_archive"####,
        name: r####"base_commit"####,
        ddl: r####"TEXT NOT NULL DEFAULT ''"####,
    },
];
