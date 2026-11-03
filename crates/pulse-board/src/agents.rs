use crate::board::*;
use crate::focus::Focus;
use crate::presence::{AgentPresence, agent_presences};
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use std::env;

pub fn agent_session(a: Option<&pulse_store::Agent>, explicit: &str) -> String {
    if !explicit.is_empty() {
        return explicit.to_string();
    }
    if let Ok(v) = env::var("PULSE_SESSION")
        && !v.is_empty()
    {
        return v;
    }
    a.and_then(|a| a.sessions.last().cloned())
        .unwrap_or_default()
}

pub fn same_session(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && a == b
}

pub fn resolve_agent(
    s: &pulse_store::Store,
    name: &str,
    required: bool,
) -> Result<Option<pulse_store::Agent>> {
    let ident = if name.is_empty() {
        env::var("PULSE_AGENT").unwrap_or_default()
    } else {
        name.to_string()
    };
    if ident.is_empty() {
        if required {
            return Err(refuse(
                2,
                "Who are you? Pass --agent NAME or set PULSE_AGENT (join first: pulse agent join --name NAME)",
            ));
        }
        return Ok(None);
    }
    let mut row = s.agent(&ident)?;
    if row.is_none() && ident.contains('/') {
        let conn = s.conn()?;
        let legacy: Option<String> = conn
            .query_row(
                "SELECT name FROM agents WHERE legacy_id = ? AND status != 'pruned' ORDER BY last_active_at DESC LIMIT 1",
                [&ident],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(n) = legacy {
            row = s.agent(&n)?;
        }
    }
    let Some(row) = row else {
        if required {
            return Err(refuse(
                2,
                format!(
                    "No agent named {}. Join first: pulse agent join --name {} --runner R --model M",
                    py_repr(&ident),
                    slugify(&ident)
                ),
            ));
        }
        return Ok(None);
    };
    if row.status == "pruned" {
        return Err(refuse(
            2,
            format!(
                "Agent {} was pruned on {} ({}); join with a new name",
                py_repr(&row.name),
                row.pruned_at,
                row.pruned_reason
            ),
        ));
    }
    Ok(Some(row))
}

#[derive(Debug, Clone, Default)]
pub struct JoinInput {
    pub name: String,
    pub runner: String,
    pub model: String,
    pub session: String,
    pub display: String,
    pub kind: String,
    pub legacy_id: String,
}

pub fn join_agent(s: &pulse_store::Store, input: JoinInput) -> Result<(pulse_store::Agent, bool)> {
    let name = input
        .name
        .trim_matches([' ', '\t', '\n', '\r'])
        .to_ascii_lowercase();
    if !valid_name(&name) {
        return Err(refuse(
            2,
            format!(
                "A name is 2-40 characters: lowercase letters, digits, dashes, starting with a letter ({})",
                py_repr(&name)
            ),
        ));
    }
    if name == pulse_store::HUMAN_AGENT {
        return Err(refuse(
            2,
            format!("{} is the maintainer; pick another name", py_repr(&name)),
        ));
    }
    let kind = if input.kind.is_empty() {
        "session"
    } else {
        &input.kind
    };
    let now = s.now();
    let conn = s.conn()?;
    if let Some(row) = s.agent(&name)? {
        if row.status == "pruned" {
            return Err(refuse(
                2,
                format!(
                    "The name {} belongs to an agent pruned on {}; names are never reused",
                    py_repr(&name),
                    row.pruned_at
                ),
            ));
        }
        if (!input.runner.is_empty() && !row.runner.is_empty() && input.runner != row.runner)
            || (!input.model.is_empty() && !row.model.is_empty() && input.model != row.model)
        {
            return Err(refuse(
                2,
                format!(
                    "The name {} is taken by another agent ({}/{}); pick a unique name",
                    py_repr(&name),
                    row.runner,
                    row.model
                ),
            ));
        }
        let mut sessions = row.sessions.clone();
        if !input.session.is_empty() && !sessions.contains(&input.session) {
            sessions.push(input.session.clone());
        }
        if sessions.len() > 50 {
            sessions = sessions[sessions.len() - 50..].to_vec();
        }
        let promoted = row.kind == "legacy" && (kind == "session" || kind == "worker");
        let new_kind = if promoted { kind } else { &row.kind };
        conn.execute("UPDATE agents SET runner = COALESCE(NULLIF(?, ''), runner), model = COALESCE(NULLIF(?, ''), model), display = COALESCE(NULLIF(?, ''), display), kind = ?, status = 'active', last_active_at = ?, sessions = ? WHERE name = ?",
            params![input.runner, input.model, input.display, new_kind, now, pulse_store::gojson::to_string(&sessions)?, name])?;
        event(
            s,
            &name,
            "agent.resume",
            &name,
            pulse_store::Detail::json(
                serde_json::json!({"session": input.session, "promoted_from": if promoted { "legacy" } else { "" }}),
            )?,
        )?;
        return Ok((s.agent(&name)?.expect("agent after update"), false));
    }
    let sessions = if input.session.is_empty() {
        vec![]
    } else {
        vec![input.session.clone()]
    };
    let display = if input.display.is_empty() {
        name.clone()
    } else {
        input.display.clone()
    };
    conn.execute("INSERT INTO agents (name, display, runner, model, kind, status, created_at, last_active_at, sessions, legacy_id) VALUES (?, ?, ?, ?, ?, 'active', ?, ?, ?, ?)",
        params![name, display, input.runner, input.model, kind, now, now, pulse_store::gojson::to_string(&sessions)?, input.legacy_id])?;
    event(
        s,
        &name,
        "agent.join",
        &name,
        pulse_store::Detail::json(
            serde_json::json!({"runner": input.runner, "model": input.model, "session": input.session}),
        )?,
    )?;
    Ok((s.agent(&name)?.expect("agent after insert"), true))
}

pub fn spawn_agent(
    s: &pulse_store::Store,
    runner: &str,
    model: &str,
    kind: &str,
) -> Result<pulse_store::Agent> {
    let kind = if kind.is_empty() { "worker" } else { kind };
    let conn = s.conn()?;
    let mut stmt = conn.prepare("SELECT name FROM agents")?;
    let taken = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<std::collections::BTreeSet<_>>>()?;
    for n in 1.. {
        for word in NAME_WORDS {
            let cand = if n == 1 {
                (*word).to_string()
            } else {
                format!("{word}-{n}")
            };
            if !taken.contains(&cand) {
                return Ok(join_agent(
                    s,
                    JoinInput {
                        name: cand,
                        runner: runner.into(),
                        model: model.into(),
                        kind: kind.into(),
                        ..Default::default()
                    },
                )?
                .0);
            }
        }
    }
    unreachable!()
}

pub fn touch_agent(s: &pulse_store::Store, name: &str, useful: bool, when: &str) -> Result<()> {
    let when = if when.is_empty() {
        s.now()
    } else {
        when.to_string()
    };
    let conn = s.conn()?;
    crate::posts::touch_agent_conn(&conn, name, useful, &when)
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Pruned {
    pub name: String,
    pub reason: String,
}

pub fn prune_agents(
    s: &pulse_store::Store,
    now: &str,
    days: i64,
    actor: &str,
    dry_run: bool,
) -> Result<Vec<Pruned>> {
    let now = if now.is_empty() {
        s.now()
    } else {
        now.to_string()
    };
    let days = if days < 0 {
        s.setting_int("prune_after_days")
    } else {
        days
    };
    let actor = if actor.is_empty() { "daemon" } else { actor };
    let cutoff = (parse_iso(&now)? - chrono::Duration::days(days))
        .format(TIME_LAYOUT)
        .to_string();
    let conn = s.conn()?;
    let mut stmt = conn.prepare("SELECT name, last_useful_at, last_active_at, created_at FROM agents WHERE status = 'active' AND kind NOT IN ('human', 'lead') ORDER BY name")?;
    let mut out = Vec::new();
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    for row in rows {
        let (name, useful, active, created) = row?;
        let useful = if useful.is_empty() {
            if active.is_empty() { created } else { active }
        } else {
            useful
        };
        if useful >= cutoff {
            continue;
        }
        let leased: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM jobs WHERE status = 'leased' AND lease_agent = ?",
                [&name],
                |r| r.get(0),
            )
            .optional()?;
        if leased.is_some() {
            continue;
        }
        let reason = format!("no useful output since {useful} (limit {days} days)");
        out.push(Pruned {
            name: name.clone(),
            reason: reason.clone(),
        });
        if !dry_run {
            conn.execute("UPDATE agents SET status = 'pruned', pruned_at = ?, pruned_reason = ? WHERE name = ?", params![now, reason, name])?;
            event(s, actor, "agent.prune", &name, reason)?;
        }
    }
    Ok(out)
}

/// The reason a retired worker's row carries (`pruned_reason`).
pub const RETIRE_REASON: &str = "retired after its run; workers are not kept idle";

/// Retires the workers whose work is over: an active `worker` agent with a
/// run that has ended, no lease, no run in flight and no held conversation
/// waiting to be carried on is marked `pruned` with [`RETIRE_REASON`] and an
/// `agent.retire` event by `actor`. A retired name is never reused - the next
/// job gets a fresh worker from `spawn_agent` - so the roster's History shows
/// every worker that ever ran. A worker that has not run yet (joined by hand,
/// or picked and not yet started) stays, as do the `keep` names - the workers
/// the caller still supervises or is about to use. Leads, sessions and the
/// human are never retired. Returns the names retired.
pub fn retire_idle_workers(
    s: &pulse_store::Store,
    now: &str,
    actor: &str,
    keep: &[String],
) -> Result<Vec<String>> {
    let now = if now.is_empty() {
        s.now()
    } else {
        now.to_string()
    };
    let actor = if actor.is_empty() { "daemon" } else { actor };
    let conn = s.conn()?;
    let mut stmt = conn.prepare(
        "SELECT a.name FROM agents a WHERE a.status = 'active' AND a.kind = 'worker'
         AND EXISTS (SELECT 1 FROM runs r WHERE r.agent = a.name AND r.status != 'running')
         AND NOT EXISTS (SELECT 1 FROM jobs j WHERE j.lease_agent = a.name AND j.status = 'leased')
         AND NOT EXISTS (SELECT 1 FROM runs r WHERE r.agent = a.name AND r.status = 'running')
         AND NOT EXISTS (SELECT 1 FROM harness_sessions h JOIN jobs j ON j.id = h.job_id WHERE h.agent = a.name AND h.status = 'held' AND j.status IN ('queued', 'leased'))
         ORDER BY a.name",
    )?;
    let names = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::new();
    for name in names {
        if keep.iter().any(|k| k == &name) {
            continue;
        }
        let changed = conn.execute(
            "UPDATE agents SET status = 'pruned', pruned_at = ?, pruned_reason = ? WHERE name = ? AND status = 'active'",
            params![now, RETIRE_REASON, name],
        )?;
        if changed == 0 {
            continue;
        }
        event(s, actor, "agent.retire", &name, RETIRE_REASON)?;
        out.push(name);
    }
    Ok(out)
}

/// An agent's row with its presence and post counts. The agent and presence
/// fields are flattened into the serialized card; in code, `card.name` reads
/// through [`Deref`] to the agent and presence fields through
/// `card.presence`.
#[derive(Debug, Clone, Serialize)]
pub struct Scorecard {
    #[serde(flatten)]
    pub agent: pulse_store::Agent,
    #[serde(flatten)]
    pub presence: AgentPresence,
    pub findings: i64,
    pub suggestions: i64,
    pub votes: i64,
    pub answers: i64,
    pub results: i64,
    pub n: i64,
    pub last_post: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<Focus>,
}

impl std::ops::Deref for Scorecard {
    type Target = pulse_store::Agent;

    fn deref(&self) -> &pulse_store::Agent {
        &self.agent
    }
}

pub fn scorecards(s: &pulse_store::Store, include_pruned: bool) -> Result<Vec<Scorecard>> {
    let presence = agent_presences(s, "")?;
    let conn = s.conn()?;
    let q = if include_pruned {
        "SELECT name FROM agents ORDER BY status, last_active_at DESC, name"
    } else {
        "SELECT name FROM agents WHERE status != 'pruned' ORDER BY status, last_active_at DESC, name"
    };
    let names = conn
        .prepare(q)?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let focuses = crate::focus::agent_focuses(s, &s.now())?;
    let mut out = Vec::new();
    for name in names {
        if let Some(agent) = s.agent(&name)? {
            type PostCounts = (
                Option<i64>,
                Option<i64>,
                Option<i64>,
                Option<i64>,
                Option<i64>,
                i64,
                Option<String>,
            );
            let (f, sg, v, an, r, n, last): PostCounts = conn.query_row("SELECT SUM(kind = 'finding'), SUM(kind = 'suggestion'), SUM(kind = 'vote'), SUM(kind = 'answer'), SUM(kind = 'result'), COUNT(*), MAX(date) FROM posts WHERE agent = ?", [&name], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)))?;
            out.push(Scorecard {
                agent,
                presence: presence.get(&name).cloned().unwrap_or_default(),
                findings: f.unwrap_or(0),
                suggestions: sg.unwrap_or(0),
                votes: v.unwrap_or(0),
                answers: an.unwrap_or(0),
                results: r.unwrap_or(0),
                n,
                last_post: last.unwrap_or_default(),
                focus: focuses.get(&name).cloned(),
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{TEST_NOW, code, count_rows, new_home, no_tty, open_home, run_cmd};
    use rusqlite::params;

    #[test]
    fn session_resolution() {
        let _serial = crate::test_helpers::serial();
        // A runner job exports PULSE_SESSION; this test asserts the agent-row fallback.
        let _session = pulse_testutil::unsetenv("PULSE_SESSION");
        let a = pulse_store::Agent {
            sessions: vec!["s1".into(), "s2".into()],
            ..Default::default()
        };
        assert_eq!(agent_session(Some(&a), ""), "s2");
        assert!(same_session("x", "x"));
        assert!(!same_session("", ""));
    }

    #[test]
    fn agents_join_spawn_prune() {
        let _serial = crate::test_helpers::serial();
        let _fidelity_literals = (
            "--days",
            "--dry-run",
            "--model",
            "--name",
            "--runner",
            "--session",
            "; names are never reused",
            "Agent 'alice' was pruned on",
            "INSERT INTO jobs (id, project, kind, title, status, created_at, lease_agent) VALUES ('job-1', ?, 'task', 't', 'leased', ?, 'borealis')",
            "The name 'alice' belongs to an agent pruned on",
            "UPDATE agents SET name = name WHERE 0",
            "joined alice (copilot/m1); export PULSE_AGENT=alice",
            "pruned alice: no useful output since 2026-08-01T00:00:00Z (limit 30 days)",
            "welcome back alice (copilot/m1); export PULSE_AGENT=alice",
            "would prune alice: no useful output since 2026-08-01T00:00:00Z (limit 14 days)",
        );
        no_tty();
        let (_dir, home, mut s) = new_home();
        s.close().unwrap();
        s = open_home(std::path::Path::new(&home));

        let (out, err) = run_cmd(
            &s,
            None,
            &[
                "agent",
                "join",
                "--name",
                "Alice",
                "--runner",
                "copilot",
                "--model",
                "m1",
                "--session",
                "s1",
            ],
        );
        assert!(err.is_ok(), "join: {out:?} {err:?}");
        assert_eq!(
            out.trim(),
            "joined alice (copilot/m1); export PULSE_AGENT=alice"
        );
        let (out, err) = run_cmd(
            &s,
            None,
            &[
                "agent",
                "join",
                "--name",
                "alice",
                "--runner",
                "copilot",
                "--session",
                "s2",
            ],
        );
        assert!(err.is_ok(), "resume: {out:?} {err:?}");
        assert_eq!(
            out.trim(),
            "welcome back alice (copilot/m1); export PULSE_AGENT=alice"
        );
        let a = s.agent("alice").unwrap().unwrap();
        assert_eq!(a.sessions.join(","), "s1,s2");
        for args in [
            vec!["agent", "join", "--name", "alice", "--runner", "claude"],
            vec!["agent", "join", "--name", "human"],
            vec!["agent", "join", "--name", "A"],
            vec!["agent", "join", "--name", "9lives"],
        ] {
            let (_, err) = run_cmd(&s, None, &args);
            assert_eq!(code(err.as_ref().err()), 2, "{args:?}");
        }
        for want in ["aurora", "borealis"] {
            let (out, err) = run_cmd(
                &s,
                None,
                &["agent", "spawn", "--runner", "copilot", "--model", "m"],
            );
            assert!(err.is_ok(), "spawn: {out:?} {err:?}");
            assert_eq!(out.trim(), want);
        }
        s.conn()
            .unwrap()
            .execute("UPDATE agents SET name = name WHERE 0", [])
            .unwrap();
        assert_eq!(s.agent("aurora").unwrap().unwrap().kind, "worker");
        let (out, err) = run_cmd(&s, None, &["agent", "list"]);
        assert!(err.is_ok());
        assert!(out.contains("aurora") && out.contains("borealis"), "{out}");

        s.conn().unwrap().execute(
            "UPDATE agents SET last_active_at = '2026-08-01T00:00:00Z', last_useful_at = '2026-08-01T00:00:00Z' WHERE name IN ('alice', 'borealis')",
            [],
        ).unwrap();
        let project_dir = tempfile::tempdir().unwrap();
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO projects(slug, name, path, added_at) VALUES ('demo', 'demo', ?, ?)",
                params![project_dir.path().display().to_string(), TEST_NOW],
            )
            .unwrap();
        s.conn().unwrap().execute(
            "INSERT INTO jobs (id, project, kind, title, status, created_at, lease_agent) VALUES ('job-1', 'demo', 'task', 't', 'leased', ?, 'borealis')",
            [TEST_NOW],
        ).unwrap();
        let dry = prune_agents(&s, TEST_NOW, 14, "daemon", true).unwrap();
        assert_eq!(dry[0].name, "alice");
        assert!(dry[0].reason.contains("limit 14 days"));
        assert_eq!(s.agent("alice").unwrap().unwrap().status, "active");
        let pruned = prune_agents(&s, TEST_NOW, 30, "daemon", false).unwrap();
        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0].name, "alice");
        let alice = s.agent("alice").unwrap().unwrap();
        assert_eq!(alice.status, "pruned");
        assert_eq!(alice.pruned_at, TEST_NOW);
        assert!(resolve_agent(&s, "alice", true).is_err());
        assert!(
            join_agent(
                &s,
                JoinInput {
                    name: "alice".into(),
                    ..Default::default()
                }
            )
            .is_err()
        );
        // A pruned row is the only carrier of a legacy runner/model id: the
        // lookup must not dead-end on it (the hint says "join with a new name",
        // which the user cannot follow through the same --agent flag).
        s.conn()
            .unwrap()
            .execute(
                "UPDATE agents SET legacy_id = 'copilot/m', last_active_at = '2099-01-01T00:00:00Z' WHERE name = 'alice'",
                [],
            )
            .unwrap();
        assert!(resolve_agent(&s, "copilot/m", false).unwrap().is_none());
        let err = resolve_agent(&s, "copilot/m", true).unwrap_err();
        assert!(err.to_string().contains("No agent named"), "{err}");
        s.conn()
            .unwrap()
            .execute(
                "UPDATE agents SET legacy_id = 'copilot/m' WHERE name = 'aurora'",
                [],
            )
            .unwrap();
        assert_eq!(
            resolve_agent(&s, "copilot/m", true).unwrap().unwrap().name,
            "aurora"
        );
        assert!(resolve_agent(&s, "", false).unwrap().is_none());
        assert_eq!(count_rows(&s, "events", "action = 'agent.prune'", &[]), 1);
    }

    #[test]
    fn prune_agents_keeps_leads() {
        let _serial = crate::test_helpers::serial();
        let (_dir, _home, s) = new_home();
        for input in [
            JoinInput {
                name: "proj-lead".into(),
                kind: "lead".into(),
                ..Default::default()
            },
            JoinInput {
                name: "stale-worker".into(),
                kind: "worker".into(),
                ..Default::default()
            },
        ] {
            join_agent(&s, input).unwrap();
        }
        s.conn()
            .unwrap()
            .execute(
                "UPDATE agents SET last_useful_at='2026-08-01T00:00:00Z', last_active_at='2026-08-01T00:00:00Z' WHERE name IN ('proj-lead', 'stale-worker')",
                [],
            )
            .unwrap();
        let pruned = prune_agents(&s, TEST_NOW, 14, "daemon", false).unwrap();
        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0].name, "stale-worker");
        assert_eq!(s.agent("proj-lead").unwrap().unwrap().status, "active");
        assert_eq!(s.agent("stale-worker").unwrap().unwrap().status, "pruned");
    }

    #[test]
    fn retire_idle_workers_retires_only_free_workers_for_good() {
        let _serial = crate::test_helpers::serial();
        let (_dir, _home, s) = new_home();
        for (name, kind) in [
            ("proj-lead", "lead"),
            ("hand", "session"),
            ("fresh", "worker"),
            ("free", "worker"),
            ("leased", "worker"),
            ("running", "worker"),
            ("holder", "worker"),
            ("kept", "worker"),
        ] {
            join_agent(
                &s,
                JoinInput {
                    name: name.into(),
                    kind: kind.into(),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let conn = s.conn().unwrap();
        conn.execute(
            "INSERT INTO projects (slug, name, path, status, added_at) VALUES ('proj', 'proj', '/tmp/proj', 'active', ?)",
            params![TEST_NOW],
        )
        .unwrap();
        for (id, status, agent) in [
            ("J-free", "done", "free"),
            ("J-kept", "done", "kept"),
            ("J-leased", "leased", "leased"),
            ("J-running", "done", "running"),
            ("J-held", "done", "holder"),
            ("J-queued-held", "queued", ""),
        ] {
            conn.execute(
                "INSERT INTO jobs (id, project, kind, title, brief, status, created_at, created_by, lease_agent) VALUES (?, 'proj', 'task', ?, '', ?, ?, 'human', ?)",
                params![id, id, status, TEST_NOW, agent],
            )
            .unwrap();
        }
        // Every worker but `fresh` has run; `leased` holds a lease, `running`
        // a run in flight, `holder` a held conversation whose job is queued.
        for (id, job, agent, status) in [
            ("R-free", "J-free", "free", "done"),
            ("R-kept", "J-kept", "kept", "done"),
            ("R-leased", "J-leased", "leased", "failed"),
            ("R-running-1", "J-running", "running", "done"),
            ("R-running", "J-running", "running", "running"),
            ("R-held", "J-held", "holder", "held"),
        ] {
            conn.execute(
                "INSERT INTO runs (id, job_id, agent, project, started_at, status) VALUES (?, ?, ?, 'proj', ?, ?)",
                params![id, job, agent, TEST_NOW, status],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO harness_sessions (id, job_id, agent, status, started_at, updated_at) VALUES ('H-held', 'J-queued-held', 'holder', 'held', ?, ?)",
            params![TEST_NOW, TEST_NOW],
        )
        .unwrap();
        drop(conn);

        let retired = retire_idle_workers(&s, TEST_NOW, "daemon", &["kept".to_string()]).unwrap();
        assert_eq!(retired, vec!["free".to_string()]);
        for name in [
            "proj-lead",
            "hand",
            "fresh",
            "leased",
            "running",
            "holder",
            "kept",
        ] {
            assert_eq!(s.agent(name).unwrap().unwrap().status, "active", "{name}");
        }
        let free = s.agent("free").unwrap().unwrap();
        assert_eq!(free.status, "pruned");
        assert_eq!(free.pruned_at, TEST_NOW);
        assert_eq!(free.pruned_reason, RETIRE_REASON);
        assert_eq!(
            count_rows(
                &s,
                "events",
                "action = 'agent.retire' AND actor = 'daemon' AND subject = 'free'",
                &[]
            ),
            1
        );
        // Retired for good: the name is not reused and a second sweep is quiet.
        let err = join_agent(
            &s,
            JoinInput {
                name: "free".into(),
                kind: "worker".into(),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("never reused"), "{err}");
        assert!(
            retire_idle_workers(&s, TEST_NOW, "daemon", &["kept".to_string()])
                .unwrap()
                .is_empty()
        );
        // Once its work is over, the kept worker goes too.
        assert_eq!(
            retire_idle_workers(&s, TEST_NOW, "", &[]).unwrap(),
            vec!["kept".to_string()]
        );
        // A fresh worker gets a new name.
        let fresh = spawn_agent(&s, "pulse", "", "worker").unwrap();
        assert!(
            !["free", "kept"].contains(&fresh.name.as_str()),
            "fresh worker {}",
            fresh.name
        );
    }
}
