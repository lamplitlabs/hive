use crate::httpx::{Request, ResponseWriter};
use crate::server::{Handler, UiFs};
use anyhow::{Result, anyhow};
use rusqlite::types::Value as SqlValue;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
pub(crate) type JsonMap = BTreeMap<String, serde_json::Value>;
impl Handler {
    pub(crate) fn running(&self, r: &crate::httpx::Request) -> anyhow::Result<JsonMap> {
        let mut hub = JsonMap::new();
        hub.insert("version".into(), json!(hive_cli::version()));
        hub.insert("revision".into(), json!(hive_cli::revision()));
        hub.insert("pid".into(), json!(std::process::id()));
        hub.insert(
            "restart_pending".into(),
            json!(
                self.binary_changed
                    .load(std::sync::atomic::Ordering::SeqCst)
            ),
        );
        let (ok, applicable) = hive_cli::code_signed(&hive_cli::executable());
        let signed = if applicable { json!(ok) } else { Value::Null };
        let Some(store) = &self.store else {
            return Ok(map_from_pairs(vec![
                ("available", json!(false)),
                ("self_update", json!("")),
                ("retry_minutes", json!(0)),
                ("hub", json!(hub)),
                ("daemon", Value::Null),
                ("brain", Value::Null),
                ("last", Value::Null),
                ("state", json!("no-store")),
                ("signed", signed),
                ("restart_ask", Value::Null),
            ]));
        };
        let self_update = store.setting("daemon.self_update", "on");
        let mut retry = store.setting_int("daemon.self_update_retry_minutes");
        if retry <= 0 {
            retry = 60;
        }
        let daemon = match store.meta("daemon") {
            Ok((raw, true)) => serde_json::from_str::<Value>(&raw).ok().map(|beat| {
                json!(map_from_pairs(vec![
                    ("pid", beat.get("pid").cloned().unwrap_or(Value::Null)),
                    (
                        "at",
                        json!(beat.get("at").and_then(Value::as_str).unwrap_or(""))
                    ),
                    (
                        "version",
                        json!(beat.get("version").and_then(Value::as_str).unwrap_or(""))
                    ),
                    (
                        "revision",
                        json!(beat.get("revision").and_then(Value::as_str).unwrap_or(""))
                    ),
                    ("bus", beat.get("bus").cloned().unwrap_or(Value::Null)),
                    (
                        "note",
                        json!(beat.get("note").and_then(Value::as_str).unwrap_or(""))
                    ),
                    (
                        "started_at",
                        json!(beat.get("started_at").and_then(Value::as_str).unwrap_or(""))
                    ),
                    (
                        "restart",
                        beat.get("restart").cloned().unwrap_or(Value::Null)
                    ),
                    // The keep-awake hold and the last sleep noticed
                    // (daemon.keep_awake, awake.rs).
                    ("awake", beat.get("awake").cloned().unwrap_or(Value::Null)),
                    ("slept", beat.get("slept").cloned().unwrap_or(Value::Null)),
                ]))
            }),
            Ok(_) => None,
            Err(err) => return Err(err),
        };
        let brain = store.brain()?.map(|p| {
            let head = crate::board::git_value(&p.path, &["rev-parse", "HEAD"]);
            let branch = crate::board::git_value(&p.path, &["rev-parse", "--abbrev-ref", "HEAD"]);
            let dirty = !crate::board::git_value(&p.path, &["status", "--porcelain"]).is_empty();
            json!(map_from_pairs(vec![
                ("slug", json!(p.slug)),
                ("default_branch", json!(p.default_branch)),
                ("branch", json!(branch)),
                ("head", json!(head)),
                ("dirty", json!(dirty)),
            ]))
        });
        let mut last = Value::Null;
        if let Ok(rows) = self.addition_rows(
            r,
            "SELECT at, detail FROM events WHERE action = 'daemon.self_update' ORDER BY seq DESC LIMIT 1",
            Vec::new(),
        ) && let Some(row) = rows.first() {
            let detail = row
                .get("detail")
                .and_then(Value::as_str)
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or(Value::Null);
            last = json!(map_from_pairs(vec![
                ("at", row.get("at").cloned().unwrap_or(Value::String(String::new()))),
                ("from", json!(detail.get("from").and_then(Value::as_str).unwrap_or(""))),
                ("to", json!(detail.get("to").and_then(Value::as_str).unwrap_or(""))),
                ("inflight", json!(detail.get("inflight").and_then(Value::as_i64).unwrap_or(0))),
            ]));
        }
        // The latest ask for a restart now, and whether the running daemon
        // has it still ahead of it: an ask older than the daemon's start is
        // the one that restarted the daemon before it.
        let mut restart_ask = Value::Null;
        if let Ok(rows) = self.addition_rows(
            r,
            "SELECT at, actor FROM events WHERE action = ? ORDER BY seq DESC LIMIT 1",
            vec![SqlValue::Text(
                hive_runner::daemon::RESTART_ACTION.to_string(),
            )],
        ) && let Some(row) = rows.first()
        {
            let at = row
                .get("at")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let started = daemon
                .as_ref()
                .and_then(|d| d.get("started_at"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let pending = !at.is_empty() && !started.is_empty() && at.as_str() > started;
            restart_ask = json!(map_from_pairs(vec![
                ("at", json!(at)),
                (
                    "actor",
                    json!(row.get("actor").and_then(Value::as_str).unwrap_or(""))
                ),
                ("pending", json!(pending)),
            ]));
        }
        let daemon_revision = daemon
            .as_ref()
            .and_then(|d| d.get("revision"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let state = if self_update != "on" {
            "off"
        } else if daemon.is_none() {
            "no-daemon"
        } else if brain.is_none() {
            "no-brain"
        } else if daemon_revision.is_empty() {
            "unstamped"
        } else if let Some(b) = brain.as_ref() {
            let head = b.get("head").and_then(Value::as_str).unwrap_or("");
            let branch = b.get("branch").and_then(Value::as_str).unwrap_or("");
            let default_branch = b
                .get("default_branch")
                .and_then(Value::as_str)
                .unwrap_or("");
            let dirty = b.get("dirty").and_then(Value::as_bool).unwrap_or(false);
            if !head.is_empty() && head != daemon_revision {
                if dirty || (branch != default_branch && !branch.starts_with("HEAD")) {
                    "waiting"
                } else {
                    "behind"
                }
            } else {
                "current"
            }
        } else {
            "no-brain"
        };
        Ok(map_from_pairs(vec![
            ("available", json!(true)),
            ("self_update", json!(self_update)),
            ("retry_minutes", json!(retry)),
            ("hub", json!(hub)),
            ("daemon", daemon.unwrap_or(Value::Null)),
            ("brain", brain.unwrap_or(Value::Null)),
            ("last", last),
            ("state", json!(state)),
            ("signed", signed),
            ("restart_ask", restart_ask),
        ]))
    }

    /// `POST /api/v1/running/restart`: the owner wants the daemon restarted.
    /// By default the ask drains like a self-update: the daemon stops
    /// starting runs and restarts once the runs in flight have ended or
    /// `daemon.restart_drain_minutes` is up, so nothing is held or lost. A
    /// payload of `{"now": true}` - which the console sends only after the
    /// operator confirms, having seen the runs in flight - asks for the fast
    /// path: the runs in flight hold and carry on under the next daemon, and
    /// `daemon.hold_seconds` later the rest are stopped and their jobs
    /// return to the queue. A hub whose binary changed goes either way, its
    /// conversations carrying on under the next hub.
    pub(crate) fn addition_running_restart(
        &self,
        w: &mut crate::httpx::ResponseWriter,
        r: &crate::httpx::Request,
    ) {
        if r.method != "POST" {
            crate::server::write_error(
                w,
                405,
                "method not allowed".into(),
                "Only POST restarts Hive.".into(),
                "Send POST /api/v1/running/restart.".into(),
            );
            return;
        }
        let (payload, ok) = crate::server::read_payload(w, r);
        if !ok {
            return;
        }
        let _lock = crate::additions::ADDITION_WRITES
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let Some(store) = self.store.as_deref() else {
            crate::server::write_error(
                w,
                404,
                "not found".into(),
                "This hub has no store.".into(),
                "Run the hub with a Hive home.".into(),
            );
            return;
        };
        let daemon = matches!(store.meta("daemon"), Ok((_, true)));
        let now = payload.get("now").and_then(Value::as_bool).unwrap_or(false);
        let detail = json!({"from": "console", "hub_pid": std::process::id(), "now": now});
        if let Err(err) = hive_runner::daemon::ask_restart(store, hive_store::HUMAN_AGENT, detail) {
            crate::server::write_error(
                w,
                500,
                "the ask was not written".into(),
                err.to_string(),
                "Try again.".into(),
            );
            return;
        }
        let hub = self.service && self.restart_now.as_ref().is_some_and(|go| go());
        crate::server::write_json(
            w,
            200,
            json!({"asked": true, "daemon": daemon, "hub": hub, "now": now, "at": store.now()}),
        );
    }

    /// `GET /api/v1/update`: what the daemon's daily update check last
    /// found (TASK-087), compared with this console's own build. `enabled`
    /// is false while `update.latest_url` is empty; `newer` is true only
    /// when the release it named outranks the running version, and `notice`
    /// is then the line `hive version` prints. `channel` is where a newer
    /// Hive comes from here (`update.channel`, `auto` resolved);
    /// `can_install` says whether `hive update` (and the banner's Update
    /// now) can install the release on this machine, `install_command` is
    /// then the command to run in a terminal, and `install_blocked` otherwise
    /// says why not - the same words the apply refusal would give. Where a
    /// package manager installed this program (`hive_cli::managed`),
    /// `managed_by` names it, `can_install` is false and `install_command`
    /// is that manager's upgrade command; both are empty everywhere else.
    pub(crate) fn update_status(&self) -> JsonMap {
        let running = hive_cli::version();
        let enabled = self.store.as_ref().is_some_and(|s| {
            !s.setting(hive_cli::update::LATEST_URL_SETTING, "")
                .trim()
                .is_empty()
        });
        let st = self
            .store
            .as_deref()
            .and_then(hive_cli::update::stored_state)
            .unwrap_or_default();
        let notice = hive_cli::update::notice(&running, &st);
        let (channel, apply) = match self.store.as_deref() {
            Some(s) => (
                hive_runner::release_update::update_channel(s).as_str(),
                json!(hive_runner::release_update::apply_state(s)),
            ),
            None => (
                "",
                json!(hive_runner::release_update::ApplyState {
                    state: "idle".into(),
                    ..Default::default()
                }),
            ),
        };
        let blocked = match self.store.as_deref() {
            Some(s) => install_blocked(s),
            None => Some("this hub has no store".to_string()),
        };
        let binary = apply_target().binary;
        let managed = hive_cli::managed::beside(std::path::Path::new(&binary));
        let command = match &managed {
            Some(m) => m.upgrade.clone(),
            None if blocked.is_none() => "hive update".to_string(),
            None => String::new(),
        };
        map_from_pairs(vec![
            ("channel", json!(channel)),
            ("apply", apply),
            ("can_install", json!(blocked.is_none())),
            ("install_command", json!(command)),
            ("install_blocked", json!(blocked.unwrap_or_default())),
            (
                "managed_by",
                json!(managed.as_ref().map(|m| m.owner()).unwrap_or_default()),
            ),
            ("enabled", json!(enabled)),
            ("running", json!(running)),
            ("latest", json!(st.latest)),
            ("url", json!(st.url)),
            ("checked_at", json!(st.checked_at)),
            ("error", json!(st.error)),
            ("newer", json!(st.newer_than(&running))),
            ("notice", json!(notice)),
        ])
    }

    pub(crate) fn addition_update(
        &self,
        w: &mut crate::httpx::ResponseWriter,
        r: &crate::httpx::Request,
    ) {
        if !crate::server::read_method(w, r) {
            return;
        }
        self.respond(w, json!(self.update_status()), String::new());
    }

    /// `POST /api/v1/update/apply` (TASK-093): the banner's "Update now".
    /// Installs the release `update.latest_url` names the way `hive update`
    /// does (checksum-verified, signed, smoke-tested), on a thread of its
    /// own; answers 202 with the update status, whose `apply` says
    /// `installing` and later how it ended. 409 while `update.channel`
    /// resolves to source (the daemon builds from the brain's checkout),
    /// while no release source is set, or while an install already runs.
    pub(crate) fn addition_update_apply(
        &self,
        w: &mut crate::httpx::ResponseWriter,
        r: &crate::httpx::Request,
    ) {
        use hive_runner::release_update::{ApplyRefusal, start_apply};
        if r.method != "POST" {
            crate::server::write_error(
                w,
                405,
                "method not allowed".into(),
                "Only POST installs an update.".into(),
                "Send POST /api/v1/update/apply.".into(),
            );
            return;
        }
        if !crate::server::same_origin_write(w, r) {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            crate::server::write_error(
                w,
                404,
                "not found".into(),
                "This hub has no store.".into(),
                "Run the hub with a Hive home.".into(),
            );
            return;
        };
        match start_apply(store, hive_store::HUMAN_AGENT, apply_target()) {
            Ok(_running) => crate::server::write_json(w, 202, json!(self.update_status())),
            Err(refusal) => {
                let (status, hint) = match refusal {
                    ApplyRefusal::SourceChannel => (
                        409,
                        "To install published releases instead: hive settings set update.channel release",
                    ),
                    ApplyRefusal::NoSource => (
                        409,
                        "Set update.latest_url to the release document, e.g. https://api.github.com/repos/OWNER/REPO/releases/latest",
                    ),
                    ApplyRefusal::Busy => (409, "Wait for the install running to end."),
                    ApplyRefusal::NoTarget => (409, "Build hive from source on this machine."),
                    ApplyRefusal::Spawn(_) => (500, "Try again, or run hive update."),
                    ApplyRefusal::Managed(_) => (409, "Run that command in a terminal."),
                };
                crate::server::write_error(
                    w,
                    status,
                    "update not started".into(),
                    refusal.to_string(),
                    hint.into(),
                );
            }
        }
    }

    pub(crate) fn addition_running(
        &self,
        w: &mut crate::httpx::ResponseWriter,
        r: &crate::httpx::Request,
    ) {
        if !crate::server::read_method(w, r) {
            return;
        }
        match self.running(r) {
            Ok(data) => self.respond(w, json!(data), String::new()),
            Err(err) => self.respond(w, Value::Null, err.to_string()),
        }
    }
}

/// Tests install into a scratch binary instead of the hub's own.
#[cfg(test)]
pub(crate) static APPLY_TARGET: Mutex<Option<hive_runner::release_update::ApplyTarget>> =
    Mutex::new(None);

fn apply_target() -> hive_runner::release_update::ApplyTarget {
    #[cfg(test)]
    if let Some(t) = APPLY_TARGET
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
    {
        return t;
    }
    hive_runner::release_update::ApplyTarget::this_process()
}

/// Why `hive update` cannot install a release on this machine, or `None`
/// when it can: the checks [`hive_runner::release_update::start_apply`]
/// makes before it starts, short of an install already running.
fn install_blocked(s: &hive_store::Store) -> Option<String> {
    use hive_runner::release_update::{ApplyRefusal, managed_refusal, update_channel};
    // A package manager's program first: whatever else is set, it is that
    // manager's to replace.
    if let Some(why) = managed_refusal(&apply_target().binary) {
        return Some(why);
    }
    if s.setting(hive_cli::update::LATEST_URL_SETTING, "")
        .trim()
        .is_empty()
    {
        return Some(ApplyRefusal::NoSource.to_string());
    }
    if update_channel(s).as_str() == "source" {
        return Some(ApplyRefusal::SourceChannel.to_string());
    }
    if hive_cli::update::running_target().is_none() {
        return Some(ApplyRefusal::NoTarget.to_string());
    }
    None
}

fn map_from_pairs(pairs: Vec<(&str, Value)>) -> JsonMap {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers_http as http;
    use serde_json::{Value, json};
    use std::process::Command;

    fn lifecycle_repo() -> String {
        let dir = crate::test_helpers::hold_tempdir(tempfile::tempdir().expect("temp repo"));
        let dir = std::fs::canonicalize(dir).expect("canonical temp repo");
        let run = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .envs(hive_gitenv::env().into_iter().filter_map(|kv| {
                    kv.split_once('=')
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                }))
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@x")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@x")
                .output()
                .expect("run git");
            assert!(
                out.status.success(),
                "git {args:?}: {}\n{}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("README.md"), "# t\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "init"]);
        dir.display().to_string()
    }

    fn must_output(cmd: &mut Command) -> Vec<u8> {
        cmd.envs(hive_gitenv::env().into_iter().filter_map(|kv| {
            kv.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        }));
        let out = cmd.output().expect("run command");
        assert!(
            out.status.success(),
            "{:?}: {}\n{}",
            cmd,
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    }

    fn documented_difference(slug: &str, route: &str, path: &str) -> &'static str {
        if route == "roster" && path.starts_with("$[") {
            let field = path.split_once("].").map(|(_, f)| f).unwrap_or("");
            match field {
                "presence"
                | "presence_reason"
                | "last_heartbeat_at"
                | "heartbeat_source"
                | "heartbeat_age_seconds"
                | "dead_after_seconds"
                | "presence_session"
                | "presence_checked_at"
                | "presence_expires_at" => {
                    return "WU-024 adds supervised presence separately from unchanged identity, score and history fields; the heartbeat feature adds the beat's source, age and the dead-after threshold";
                }
                "current_job" | "current_job_kind" | "current_job_title" | "current_thread"
                | "executor" | "focus_since" | "current_task" | "current_task_title"
                | "current_project" | "current_source" => {
                    return "agents-focus-roster adds current work/task fields separately from identity, score and history fields";
                }
                _ => {}
            }
        }
        if slug == "org" && route == "roster" && path.contains("active_claims[0].presence") {
            return "";
        }
        ""
    }

    fn roster_golden(value: Value, legacy: bool) -> Value {
        let mut rows = value.as_array().cloned().expect("roster array");
        for row in &mut rows {
            if legacy
                && row.get("status").and_then(Value::as_str) == Some("working")
                && row.get("agent_status").is_some()
            {
                let status = row["agent_status"].clone();
                row.as_object_mut().unwrap().insert("status".into(), status);
            }
        }
        rows.sort_by(|a, b| {
            a.get("agent")
                .and_then(Value::as_str)
                .unwrap_or("")
                .cmp(b.get("agent").and_then(Value::as_str).unwrap_or(""))
        });
        Value::Array(rows)
    }

    #[test]
    fn restart_now_asks_the_daemon_and_the_console_sees_the_ask_until_the_daemon_restarted() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(true);
        let mut h = http::handler(&s);
        let went = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = went.clone();
        h.restart_now = Some(Arc::new(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            true
        }));
        // Only POST, and only JSON.
        let (status, _) = http::additions_request(h.clone(), "GET", "/api/v1/running/restart", "");
        assert_eq!(status, 405);
        // Without a daemon the ask is written for the one that comes; a hub
        // not under the service never restarts itself.
        let (status, result) =
            http::additions_request(h.clone(), "POST", "/api/v1/running/restart", "{}");
        assert_eq!(status, 200, "{result:?}");
        assert_eq!(result["asked"], true);
        assert_eq!(result["daemon"], false);
        assert_eq!(result["hub"], false);
        assert_eq!(went.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(result["now"], false, "the default ask drains");
        // "now" is echoed and written into the ask's detail for the daemon.
        let (status, result) = http::additions_request(
            h.clone(),
            "POST",
            "/api/v1/running/restart",
            "{\"now\": true}",
        );
        assert_eq!(status, 200, "{result:?}");
        assert_eq!(result["now"], true);
        let detail: String = s
            .conn()
            .unwrap()
            .query_row(
                "SELECT detail FROM events WHERE action = ? ORDER BY seq DESC LIMIT 1",
                [hive_runner::daemon::RESTART_ACTION],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&detail).unwrap()["now"],
            true
        );
        let (actor, at): (String, String) = s
            .conn()
            .unwrap()
            .query_row(
                "SELECT actor, at FROM events WHERE action = ? ORDER BY seq DESC LIMIT 1",
                [hive_runner::daemon::RESTART_ACTION],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(actor, hive_store::HUMAN_AGENT);
        let (_, result) = http::additions_request(h.clone(), "GET", "/api/v1/running", "");
        assert_eq!(result["restart_ask"]["at"], at);
        assert_eq!(result["restart_ask"]["actor"], "human");
        assert_eq!(
            result["restart_ask"]["pending"], false,
            "no daemon to act on it"
        );
        assert_eq!(result["hub"]["restart_pending"], false);
        // A daemon that started before the ask has it ahead; one that
        // started after it is the daemon the ask restarted.
        let beat = |started_at: &str| {
            s.conn().unwrap().execute("INSERT INTO meta (key, value) VALUES ('daemon', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [format!(r#"{{"pid":4242,"at":"2026-09-06T11:59:00Z","loops":3,"interval":300,"note":"restarting: waits for 1 run","started_at":"{started_at}","version":"dev","revision":"","restart":{{"since":"2026-09-06T11:50:00Z","inflight":1,"dispatching":true,"waits_for":[]}}}}"#)]).unwrap();
        };
        beat("2020-01-01T00:00:00Z");
        let (_, result) = http::additions_request(h.clone(), "GET", "/api/v1/running", "");
        assert_eq!(result["restart_ask"]["pending"], true);
        assert_eq!(result["daemon"]["note"], "restarting: waits for 1 run");
        assert_eq!(result["daemon"]["started_at"], "2020-01-01T00:00:00Z");
        assert_eq!(result["daemon"]["restart"]["inflight"], 1);
        beat("2999-01-01T00:00:00Z");
        let (_, result) = http::additions_request(h.clone(), "GET", "/api/v1/running", "");
        assert_eq!(result["restart_ask"]["pending"], false);
        // Under the service, the hub whose binary changed goes too.
        h.service = true;
        h.binary_changed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let (status, result) =
            http::additions_request(h.clone(), "POST", "/api/v1/running/restart", "{}");
        assert_eq!(status, 200, "{result:?}");
        assert_eq!(result["daemon"], true);
        assert_eq!(result["hub"], true);
        assert_eq!(went.load(std::sync::atomic::Ordering::SeqCst), 1);
        let (_, result) = http::additions_request(h.clone(), "GET", "/api/v1/running", "");
        assert_eq!(result["hub"]["restart_pending"], true);
    }

    #[test]
    fn update_says_what_the_daily_check_found() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(true);
        let h = http::handler(&s);
        let (status, result) = http::additions_request(h.clone(), "GET", "/api/v1/update", "");
        assert_eq!(status, 200, "{result:?}");
        assert_eq!(result["enabled"], false);
        assert_eq!(result["newer"], false);
        assert_eq!(result["notice"], Value::Null);
        assert_eq!(result["running"], json!(hive_cli::version()));
        let st = hive_cli::update::UpdateState {
            checked_at: "2026-11-09T02:00:00Z".into(),
            source: "https://releases.test/latest.json".into(),
            latest: "v999.0.0".into(),
            url: "https://releases.test/v999.0.0".into(),
            error: String::new(),
        };
        s.set_meta(
            hive_cli::update::META_KEY,
            &serde_json::to_string(&st).unwrap(),
        )
        .unwrap();
        // Off: an old result stays hidden.
        let (_, result) = http::additions_request(h.clone(), "GET", "/api/v1/update", "");
        assert_eq!(result["latest"], "");
        s.set_setting("human", hive_cli::update::LATEST_URL_SETTING, &st.source)
            .unwrap();
        let (_, result) = http::additions_request(h.clone(), "GET", "/api/v1/update", "");
        assert_eq!(result["enabled"], true);
        assert_eq!(result["latest"], "v999.0.0");
        assert_eq!(result["url"], "https://releases.test/v999.0.0");
        assert_eq!(result["checked_at"], "2026-11-09T02:00:00Z");
        let semver = hive_cli::update::parse_semver(&hive_cli::version()).is_some();
        assert_eq!(result["newer"], semver);
        let (status, _) = http::additions_request(h, "POST", "/api/v1/update", "{}");
        assert_ne!(status, 200);
        assert_eq!(result["channel"], "release");
        assert_eq!(result["apply"]["state"], "idle");
    }

    #[test]
    fn update_says_whether_hive_update_can_install_here() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(true);
        let h = http::handler(&s);
        // No release source: nothing to install from, and the payload says so.
        let (status, result) = http::additions_request(h.clone(), "GET", "/api/v1/update", "");
        assert_eq!(status, 200, "{result:?}");
        assert_eq!(result["can_install"], false, "{result:?}");
        assert_eq!(result["install_command"], "");
        assert!(
            result["install_blocked"]
                .as_str()
                .unwrap_or_default()
                .contains(hive_cli::update::LATEST_URL_SETTING),
            "{result:?}"
        );
        s.set_setting(
            "human",
            hive_cli::update::LATEST_URL_SETTING,
            "https://releases.test/latest.json",
        )
        .unwrap();
        // On the source channel the daemon rebuilds; hive update is not the way.
        s.set_setting("human", hive_cli::update::CHANNEL_SETTING, "source")
            .unwrap();
        let (_, result) = http::additions_request(h.clone(), "GET", "/api/v1/update", "");
        assert_eq!(result["channel"], "source");
        assert_eq!(result["can_install"], false, "{result:?}");
        assert_eq!(result["install_command"], "");
        assert!(
            result["install_blocked"]
                .as_str()
                .unwrap_or_default()
                .contains("update.channel"),
            "{result:?}"
        );
        // On the release channel the banner can tell the owner what to run.
        s.set_setting("human", hive_cli::update::CHANNEL_SETTING, "release")
            .unwrap();
        let (_, result) = http::additions_request(h, "GET", "/api/v1/update", "");
        assert_eq!(result["channel"], "release");
        let here = hive_cli::update::running_target().is_some();
        assert_eq!(result["can_install"], here, "{result:?}");
        if here {
            assert_eq!(result["install_command"], "hive update");
            assert_eq!(result["install_blocked"], "");
        } else {
            assert_eq!(result["install_command"], "");
            assert_ne!(result["install_blocked"], "");
        }
    }

    /// A release folder served over HTTP the way release_update.rs's tests
    /// build one: a fake `hive` printing `v0.2.0`, packed as this
    /// machine's archive, its SHA256SUMS line and the release document.
    /// Returns the document's URL and keeps the server alive.
    fn served_release(dir: &std::path::Path) -> (String, hive_testutil::httptest::Server) {
        use hive_testutil::httptest::{Response, Server};
        let root = dir.to_path_buf();
        let server = Server::new(move |req| {
            match std::fs::read(root.join(req.path().trim_start_matches('/'))) {
                Ok(bytes) => Response::new(200).body(bytes),
                Err(_) => Response::text(404, "missing"),
            }
        });
        let base = server.url();
        let target = hive_cli::update::running_target().expect("a release target for this machine");
        let stage = dir.join("stage");
        std::fs::create_dir_all(&stage).unwrap();
        let hive = stage.join("hive");
        std::fs::write(
            &hive,
            "#!/bin/sh\ncase \"$1\" in version) echo '{\"version\":\"v0.2.0\"}';; help) exit 0;; *) exit 1;; esac\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hive, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let name = hive_cli::update::archive_name("v0.2.0", &target);
        let packed = Command::new("tar")
            .arg("-czf")
            .arg(dir.join(&name))
            .arg("-C")
            .arg(&stage)
            .arg("hive")
            .status()
            .unwrap();
        assert!(packed.success());
        let digest =
            hive_runner::release_update::sha256_hex(&std::fs::read(dir.join(&name)).unwrap());
        std::fs::write(dir.join("SHA256SUMS"), format!("{digest}  {name}\n")).unwrap();
        let doc = json!({
            "tag_name": "v0.2.0",
            "html_url": format!("{base}/page"),
            "assets": [
                {"name": name, "browser_download_url": format!("{base}/{name}")},
                {"name": "SHA256SUMS", "browser_download_url": format!("{base}/SHA256SUMS")},
            ]
        });
        std::fs::write(dir.join("latest.json"), doc.to_string()).unwrap();
        (format!("{base}/latest.json"), server)
    }

    /// Restores the hub's own apply target when a test ends.
    struct ScratchTarget;
    impl Drop for ScratchTarget {
        fn drop(&mut self) {
            *APPLY_TARGET.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }

    #[test]
    fn update_apply_is_refused_off_the_release_channel_and_for_other_sites() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(true);
        let h = http::handler(&s);
        // No release source yet.
        let (status, result) =
            http::additions_request(h.clone_for_test(), "POST", "/api/v1/update/apply", "{}");
        assert_eq!(status, 409, "{result:?}");
        s.set_setting(
            "human",
            hive_cli::update::LATEST_URL_SETTING,
            "https://releases.test/latest.json",
        )
        .unwrap();
        s.set_setting("human", hive_cli::update::CHANNEL_SETTING, "source")
            .unwrap();
        let (status, result) =
            http::additions_request(h.clone_for_test(), "POST", "/api/v1/update/apply", "{}");
        assert_eq!(status, 409, "{result:?}");
        assert!(
            result["error"]
                .as_str()
                .unwrap_or_default()
                .contains("update not started"),
            "{result:?}"
        );
        let (_, result) = http::additions_request(h.clone_for_test(), "GET", "/api/v1/update", "");
        assert_eq!(result["channel"], "source");
        assert_eq!(result["apply"]["state"], "idle");
        // GET is not an install.
        let (status, _) =
            http::additions_request(h.clone_for_test(), "GET", "/api/v1/update/apply", "");
        assert_eq!(status, 405);
        // A page from another site may not start one.
        s.set_setting("human", hive_cli::update::CHANNEL_SETTING, "release")
            .unwrap();
        let server = http::server(h.clone_for_test());
        let response = crate::test_helpers::client()
            .post(format!("{}/api/v1/update/apply", server.url))
            .header("Origin", "https://evil.example")
            .header("Content-Type", "application/json")
            .body("{}")
            .send()
            .unwrap();
        assert_eq!(response.status().as_u16(), 403);
        assert!(!matches!(
            s.meta(hive_runner::release_update::APPLY_KEY),
            Ok((_, true))
        ));
    }

    #[test]
    fn update_apply_installs_the_release_in_the_background_and_reports_it() {
        let _serial = crate::test_helpers::serial();
        let _restore = ScratchTarget;
        let s = http::additions_store(true);
        let h = http::handler(&s);
        let dir = tempfile::tempdir().unwrap();
        let (url, _server) = served_release(dir.path());
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let binary = bin.join("hive").display().to_string();
        std::fs::write(&binary, "old binary").unwrap();
        *APPLY_TARGET.lock().unwrap() = Some(hive_runner::release_update::ApplyTarget {
            binary: binary.clone(),
            running: "v0.1.0".into(),
        });
        s.set_setting("human", hive_cli::update::LATEST_URL_SETTING, &url)
            .unwrap();
        s.set_setting("human", hive_cli::update::CHANNEL_SETTING, "release")
            .unwrap();

        let (status, result) =
            http::additions_request(h.clone_for_test(), "POST", "/api/v1/update/apply", "{}");
        assert_eq!(status, 202, "{result:?}");
        assert_eq!(result["channel"], "release");
        let first = result["apply"]["state"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        assert!(first == "installing" || first == "installed", "{result:?}");
        assert_eq!(result["apply"]["from"], "v0.1.0");

        let deadline = std::time::Instant::now() + Duration::from_secs(90);
        let result = loop {
            let (_, result) =
                http::additions_request(h.clone_for_test(), "GET", "/api/v1/update", "");
            if result["apply"]["state"] != "installing" {
                break result;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the install did not end: {result:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(result["apply"]["state"], "installed", "{result:?}");
        assert_eq!(result["apply"]["latest"], "v0.2.0");
        assert_eq!(result["apply"]["error"], "");
        let now = std::fs::read_to_string(&binary).unwrap();
        assert!(now.contains("\"version\":\"v0.2.0\""), "{now}");
        assert_eq!(
            std::fs::read_to_string(format!("{binary}.prev")).unwrap(),
            "old binary"
        );
        // The daily check's state now names the release it installed.
        assert_eq!(result["latest"], "v0.2.0");
    }

    /// A package manager's program (`hive_cli::managed`) is that manager's
    /// to replace: the status names it and its command, Update now is
    /// refused in the same words, and the file is as it was.
    #[test]
    fn update_leaves_a_program_a_package_manager_installed_to_that_manager() {
        let _serial = crate::test_helpers::serial();
        let _restore = ScratchTarget;
        let s = http::additions_store(true);
        let h = http::handler(&s);
        let dir = tempfile::tempdir().unwrap();
        let (url, _server) = served_release(dir.path());
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let binary = bin.join("hive").display().to_string();
        std::fs::write(&binary, "old binary").unwrap();
        let marker = format!("{binary}{}", hive_cli::managed::SUFFIX);
        std::fs::write(&marker, "by=Homebrew\nupgrade=brew upgrade hivefleet\n").unwrap();
        *APPLY_TARGET.lock().unwrap() = Some(hive_runner::release_update::ApplyTarget {
            binary: binary.clone(),
            running: "v0.1.0".into(),
        });
        s.set_setting("human", hive_cli::update::LATEST_URL_SETTING, &url)
            .unwrap();
        s.set_setting("human", hive_cli::update::CHANNEL_SETTING, "release")
            .unwrap();
        let refusal = format!(
            "Homebrew installed this hive and owns {binary}, so Hive does not replace that file. Upgrade it with: brew upgrade hivefleet"
        );

        let (status, result) =
            http::additions_request(h.clone_for_test(), "GET", "/api/v1/update", "");
        assert_eq!(status, 200, "{result:?}");
        assert_eq!(result["managed_by"], "Homebrew", "{result:?}");
        assert_eq!(result["can_install"], false, "{result:?}");
        assert_eq!(result["install_command"], "brew upgrade hivefleet");
        assert_eq!(result["install_blocked"], refusal.as_str());

        let (status, result) =
            http::additions_request(h.clone_for_test(), "POST", "/api/v1/update/apply", "{}");
        assert_eq!(status, 409, "{result:?}");
        assert_eq!(result["error"], "update not started");
        assert_eq!(result["why"], refusal.as_str());
        assert_eq!(result["fix"], "Run that command in a terminal.");
        assert_eq!(std::fs::read_to_string(&binary).unwrap(), "old binary");
        assert!(!std::path::Path::new(&format!("{binary}.prev")).exists());
        assert!(!matches!(
            s.meta(hive_runner::release_update::APPLY_KEY),
            Ok((_, true))
        ));

        // The marker gone, the same machine is one hive update installs on.
        std::fs::remove_file(&marker).unwrap();
        let (_, result) = http::additions_request(h, "GET", "/api/v1/update", "");
        assert_eq!(result["managed_by"], "", "{result:?}");
        if hive_cli::update::running_target().is_some() {
            assert_eq!(result["can_install"], true, "{result:?}");
            assert_eq!(result["install_command"], "hive update");
        }
    }

    #[test]
    fn running_names_the_builds_and_the_state() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(true);
        let h = http::handler(&s);
        let (status, result) = http::additions_request(h.clone(), "GET", "/api/v1/running", "");
        assert_eq!(status, 200);
        assert_eq!(result["available"], true);
        assert_eq!(result["state"], "no-daemon");
        assert_eq!(result["self_update"], "on");
        assert_eq!(result["retry_minutes"], 60);
        let hub = result["hub"].as_object().unwrap();
        assert_ne!(hub["version"], "");
        assert!(hub["pid"] != Value::Null);
        assert_eq!(result["daemon"], Value::Null);
        assert_eq!(result["brain"], Value::Null);
        assert_eq!(result["last"], Value::Null);
        let repo = lifecycle_repo();
        let head = String::from_utf8(must_output(
            Command::new("git")
                .arg("-C")
                .arg(&repo)
                .arg("rev-parse")
                .arg("HEAD"),
        ))
        .unwrap()
        .trim()
        .to_string();
        s.conn().unwrap().execute("INSERT INTO projects(slug,name,path,brain,default_branch,added_at) VALUES('brain','brain',?,1,'main',?)", rusqlite::params![repo, s.now()]).unwrap();
        let beat = |revision: &str| {
            s.conn().unwrap().execute("INSERT INTO meta (key, value) VALUES ('daemon', ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [format!(r#"{{"pid":4242,"at":"2026-09-06T11:59:00Z","loops":3,"interval":300,"note":"","version":"dev-{}","revision":"{}","bus":{{"subscribers":2,"dropped":7,"deduped":3}}}}"#, &revision[..std::cmp::min(12, revision.len())], revision)]).unwrap();
        };
        beat("");
        let (_, result) = http::additions_request(h.clone(), "GET", "/api/v1/running", "");
        assert_eq!(result["state"], "unstamped");
        let brain = result["brain"].as_object().unwrap();
        assert_eq!(brain["slug"], "brain");
        assert_eq!(brain["branch"], "main");
        assert_eq!(brain["head"], head);
        assert_eq!(brain["dirty"], false);
        beat(&head);
        let (_, result) = http::additions_request(h.clone(), "GET", "/api/v1/running", "");
        assert_eq!(result["state"], "current");
        let daemon = result["daemon"].as_object().unwrap();
        assert_eq!(daemon["pid"], 4242);
        assert_eq!(daemon["revision"], head);
        assert_eq!(daemon["version"], format!("dev-{}", &head[..12]));
        let bus = daemon["bus"].as_object().unwrap();
        assert_eq!(bus["subscribers"], 2);
        assert_eq!(bus["dropped"], 7);
        assert_eq!(bus["deduped"], 3);
        beat("0123456789abcdef0123456789abcdef01234567");
        let (_, result) = http::additions_request(h.clone(), "GET", "/api/v1/running", "");
        assert_eq!(result["state"], "behind");
        must_output(
            Command::new("git")
                .arg("-C")
                .arg(&repo)
                .arg("checkout")
                .arg("-q")
                .arg("-b")
                .arg("topic"),
        );
        let (_, result) = http::additions_request(h.clone(), "GET", "/api/v1/running", "");
        assert_eq!(result["state"], "waiting");
        must_output(
            Command::new("git")
                .arg("-C")
                .arg(&repo)
                .arg("checkout")
                .arg("-q")
                .arg("main"),
        );
        s.conn().unwrap().execute("INSERT INTO events (at, actor, action, subject, detail) VALUES ('2026-09-06T11:58:00Z','daemon','daemon.self_update','hive',?)", [format!(r#"{{"from":"0123456789ab","to":"{}","binary":"/x/hive","inflight":2}}"#, head)]).unwrap();
        let (_, result) = http::additions_request(h.clone(), "GET", "/api/v1/running", "");
        assert_ne!(result["last"], Value::Null);
        let last = result["last"].as_object().unwrap();
        assert_eq!(last["to"], head);
        assert_eq!(last["inflight"], 2);
        assert_eq!(last["at"], "2026-09-06T11:58:00Z");
        s.conn().unwrap().execute("INSERT INTO settings (name, value, updated_at, updated_by) VALUES ('daemon.self_update','off',?, 'owner') ON CONFLICT(name) DO UPDATE SET value = excluded.value", [s.now()]).unwrap();
        let (_, result) = http::additions_request(h.clone(), "GET", "/api/v1/running", "");
        assert_eq!(result["state"], "off");
        assert_eq!(result["self_update"], "off");
        let (status, _) = http::additions_request(h, "POST", "/api/v1/running", "{}");
        assert_eq!(status, 405);
    }

    #[test]
    fn roster_presence_history_claims_and_shared_clock() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(false);
        let exec = |query: &str, args: &[&dyn rusqlite::ToSql]| {
            s.conn().unwrap().execute(query, args).unwrap();
        };
        // "offline" is a worker whose supervised heartbeat went silent six minutes ago: dead.
        // "unknown" is a session agent with no sign of life on record at all.
        let states = std::collections::BTreeMap::from([
            ("live", "online"),
            ("offline", "dead"),
            ("idle-worker", "idle"),
            ("idle-lead", "idle"),
            ("unknown", "unknown"),
            ("legacy", "legacy"),
            ("pruned", "pruned"),
        ]);
        for name in states.keys() {
            let kind = match *name {
                "unknown" => "session",
                "legacy" => "legacy",
                "idle-lead" => "lead",
                _ => "worker",
            };
            hive_board::join_agent(
                &s,
                hive_board::JoinInput {
                    name: (*name).into(),
                    kind: kind.into(),
                    session: format!("run-{name}"),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        exec(
            "UPDATE agents SET status='pruned',pruned_at=?,pruned_reason='no useful output for 14 days' WHERE name='pruned'",
            &[&s.now()],
        );
        // Joining counts as activity; these joined long before this clock reading, so the
        // supervised heartbeat is their latest sign of life.
        exec(
            "UPDATE agents SET last_active_at='2026-09-06T11:50:00Z' WHERE name IN ('live','offline')",
            &[],
        );
        exec(
            "UPDATE agents SET last_active_at='' WHERE name='unknown'",
            &[],
        );
        exec(
            "UPDATE agents SET jobs_done=3,posts=4,last_useful_at='2026-09-01T00:00:00Z' WHERE name='live'",
            &[],
        );
        exec(
            "INSERT INTO tasks(id,project,title,status,priority,created_by,created_at,updated_at,current_rev) VALUES('TASK-live','commons','Live owner task','open',0,'owner',?,?,1)",
            &[&s.now(), &s.now()],
        );
        let mut tokens = std::collections::BTreeMap::new();
        for name in ["live", "offline"] {
            let id = hive_board::ExecutionIdentity {
                agent: name.into(),
                session: format!("run-{name}"),
                run_id: format!("run-{name}"),
                job_id: format!("job-{name}"),
                generation: 2,
                executor: "local".into(),
            };
            tokens.insert(name.to_string(), id.clone());
            let source = if name == "live" { "task:TASK-live" } else { "" };
            exec(
                "INSERT INTO jobs(id,project,kind,title,source,created_at,status,lease_agent,generation) VALUES(?,'commons','task','presence',?,?,'leased',?,2)",
                &[&id.job_id, &source, &s.now(), &name],
            );
            exec(
                "INSERT INTO runs(id,job_id,agent,project,started_at,status) VALUES(?,?,?,'commons','2026-09-06T11:50:00Z','running')",
                &[&id.run_id, &id.job_id, &name],
            );
            if name == "live" {
                exec(
                    "INSERT INTO posts(id,project,area,thread,kind,agent,date,expires,summary,created_at) VALUES('live-claim','commons','agents','live','claim','live',?,'2026-09-06T12:30:00Z','working here',?)",
                    &[&s.now(), &s.now()],
                );
            }
            let at = if name == "offline" {
                "2026-09-06T11:54:00Z"
            } else {
                "2026-09-06T11:59:00Z"
            };
            assert!(
                hive_board::record_heartbeat(
                    &s,
                    hive_board::ExecutionHeartbeat {
                        identity: id,
                        at: at.into(),
                        ..Default::default()
                    }
                )
                .unwrap()
            );
        }
        let views = vec![crate::read::ProjectView {
            project: hive_store::Project {
                slug: hive_store::COMMONS_SLUG.into(),
                ..Default::default()
            },
            data: Some(std::sync::Arc::new(crate::board::BoardData {
                params: std::collections::BTreeMap::from([(
                    "now".into(),
                    json!("2026-09-06T11:00:00Z"),
                )]),
                posts: vec![
                    crate::board::PostRow::from_map(&crate::board::object(
                        "id agent kind date",
                        vec![
                            json!("old-post"),
                            json!("live"),
                            json!("finding"),
                            json!("2026-09-01T00:00:00Z"),
                        ],
                    )),
                    crate::board::PostRow::from_map(&crate::board::object(
                        "id agent kind date",
                        vec![
                            json!("imported-post"),
                            json!("imported"),
                            json!("finding"),
                            json!("2026-08-01T00:00:00Z"),
                        ],
                    )),
                ],
                claims: vec![
                    crate::board::object(
                        "id agent status thread_key",
                        vec![
                            json!("live-claim"),
                            json!("live"),
                            json!("active"),
                            json!("agents/live"),
                        ],
                    ),
                    crate::board::object(
                        "id agent status thread_key",
                        vec![
                            json!("stale-claim"),
                            json!("offline"),
                            json!("active"),
                            json!("agents/offline"),
                        ],
                    ),
                ],
                ..Default::default()
            })),
            ..Default::default()
        }];
        let h = http::handler(&s);
        let rows = h.roster(&views).unwrap();
        let by_name = rows
            .iter()
            .map(|r| (r["agent"].as_str().unwrap().to_string(), r.clone()))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut states = states;
        states.insert("imported", "legacy");
        for (name, want) in states {
            let r = by_name
                .get(name)
                .unwrap_or_else(|| panic!("missing {name}"));
            assert_eq!(r["presence"], want, "{name}");
            assert_eq!(r["presence_checked_at"], s.now(), "{name}");
        }
        assert_eq!(by_name["live"]["status"], "working");
        assert_eq!(by_name["live"]["agent_status"], "active");
        assert_ne!(by_name["offline"]["status"], "working");
        assert_eq!(
            by_name["offline"]["active_claims"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let live = &by_name["live"];
        assert_eq!(live["last_useful_at"], "2026-09-01T00:00:00Z");
        assert_eq!(live["last_heartbeat_at"], "2026-09-06T11:59:00Z");
        assert_eq!(live["presence_expires_at"], "2026-09-06T12:04:00Z");
        assert_eq!(live["jobs_done"], 3);
        assert_eq!(live["posts"], 4);
        assert_eq!(live["current_job"], "job-live");
        assert_eq!(live["current_job_kind"], "task");
        assert_eq!(live["current_job_title"], "presence");
        assert_eq!(live["current_thread"], "agents/live");
        assert_eq!(live["executor"], "local");
        assert_eq!(live["focus_since"], "2026-09-06T11:50:00Z");
        assert_eq!(live["current_task"], "TASK-live");
        assert_eq!(live["current_task_title"], "Live owner task");
        assert_eq!(live["current_project"], "commons");
        assert_eq!(live["current_source"], "task:TASK-live");
        exec(
            "UPDATE agents SET last_useful_at='2026-09-06 11:56:24' WHERE name='live'",
            &[],
        );
        let tie_rows = h.roster(&views).unwrap();
        let tie_live = tie_rows
            .iter()
            .find(|r| r["agent"] == "live")
            .expect("live row after space-separated timestamp");
        assert_eq!(tie_live["last_useful_at"], "2026-09-06 11:56:24");
        assert_eq!(tie_live["days_since"], json!(0.002));
        let cards = hive_board::scorecards(&s, true).unwrap();
        let inspect_cards = hive_inspect::agent_scorecards(&s, true).unwrap();
        for list in [
            serde_json::to_value(&cards).unwrap(),
            serde_json::to_value(&inspect_cards).unwrap(),
        ] {
            for card in list.as_array().unwrap() {
                let r = &by_name[card["name"].as_str().unwrap()];
                assert_eq!(r["presence"], card["presence"]);
                assert_eq!(r["last_heartbeat_at"], card["last_heartbeat_at"]);
                assert_eq!(r["presence_expires_at"], card["presence_expires_at"]);
            }
        }
        let later = hive_store::Store::open(hive_store::Options {
            home: s.home.clone(),
            now: "2026-09-06T12:04:00Z".into(),
            ..Default::default()
        })
        .unwrap();
        let expired = http::handler(&later).roster(&views).unwrap();
        for r in expired {
            if r["agent"] == "live" {
                assert_eq!(r["presence"], "dead");
                assert_ne!(r["status"], "working");
            }
        }
        let before = s.agent("live").unwrap();
        let ok = hive_board::record_heartbeat(
            &later,
            hive_board::ExecutionHeartbeat {
                identity: tokens["live"].clone(),
                at: later.now(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(ok);
        let returned = http::handler(&later).roster(&views).unwrap();
        assert_eq!(returned.len(), rows.len());
        for r in returned {
            if r["agent"] == "live" {
                assert_eq!(r["presence"], "online");
            }
        }
        let after = s.agent("live").unwrap();
        assert_eq!(
            serde_json::to_value(&before).unwrap(),
            serde_json::to_value(&after).unwrap(),
            "roster filtering or heartbeat changed identity/history"
        );
    }

    #[test]
    fn roster_golden_keeps_identity_and_history_assertions() {
        let _serial = crate::test_helpers::serial();
        let legacy = json!([{"agent":"zz","status":"working","agent_status":"active","jobs_done":3},{"agent":"aa","status":"pruned","agent_status":"pruned","jobs_done":4}]);
        let rows = roster_golden(legacy.clone(), true);
        let rows = rows.as_array().unwrap();
        assert_eq!(rows[0]["status"], "pruned");
        assert_eq!(rows[1]["status"], "active");
        assert_eq!(rows[1]["jobs_done"], 3);
        assert_eq!(legacy[0]["status"], "working");
        for field in [
            "presence",
            "presence_reason",
            "last_heartbeat_at",
            "heartbeat_source",
            "heartbeat_age_seconds",
            "dead_after_seconds",
            "presence_session",
            "presence_checked_at",
            "presence_expires_at",
            "current_job",
            "current_job_kind",
            "current_job_title",
            "current_thread",
            "executor",
            "focus_since",
            "current_task",
            "current_task_title",
            "current_project",
            "current_source",
        ] {
            assert_ne!(
                documented_difference("org", "roster", &format!("$[0].{field}")),
                "",
                "added field {field} is undocumented"
            );
        }
        for field in [
            "status",
            "agent_status",
            "jobs_done",
            "pruned_reason",
            "sessions",
            "active_claims[0].presence",
            "length",
        ] {
            assert_eq!(
                documented_difference("org", "roster", &format!("$[0].{field}")),
                "",
                "legacy field {field} must still compare exactly"
            );
        }
    }

    #[test]
    fn runner_view_carries_landing_holds() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(true);
        let h = http::handler(&s);
        let (code, body) = http::additions_request(h.clone(), "GET", "/api/v1/runner", "");
        assert_eq!(code, 200);
        assert_eq!(body["available"], true);
        assert_eq!(body["landing"].as_array().unwrap().len(), 0);
        s.conn().unwrap().execute(r#"INSERT INTO meta(key,value) VALUES
        ('daemon','{"pid":123,"at":"2026-09-06T11:59:30Z","loops":4,"interval":30,"note":""}'),
        ('landing','{"proj":{"at":"2026-09-06T11:59:31Z","note":"3 branches waiting; deferred: the checkout of /w/proj has uncommitted changes"}}')"#, []).unwrap();
        let (code, body) = http::additions_request(h, "GET", "/api/v1/runner", "");
        assert_eq!(code, 200);
        let held = body["landing"].as_array().unwrap();
        assert_eq!(held.len(), 1);
        let note = held[0].as_object().unwrap();
        assert_eq!(note["project"], "proj");
        assert_eq!(note["stale"], false);
        assert_eq!(
            note["note"],
            "3 branches waiting; deferred: the checkout of /w/proj has uncommitted changes"
        );
    }

    // Workers live for one run and retire when it ends (hive_board::
    // retire_idle_workers), so the summary counts what exists now - the
    // active workers, split by whether a lease holds them - and names no
    // target: nothing is kept ready. A retired worker is a pruned row with
    // the retire reason and is not an active worker any more.
    #[test]
    fn runner_view_counts_workers_without_a_target() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(true);
        let h = http::handler(&s);
        for (name, kind) in [("busy", "worker"), ("between", "worker"), ("lead", "lead")] {
            hive_board::join_agent(
                &s,
                hive_board::JoinInput {
                    name: name.into(),
                    kind: kind.into(),
                    session: format!("run-{name}"),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        s.conn().unwrap().execute(
            "INSERT INTO jobs(id,project,kind,title,source,created_at,status,lease_agent,generation) VALUES('job-busy','commons','task','t','',?,'leased','busy',1)",
            [s.now()],
        ).unwrap();
        s.conn().unwrap().execute(
            "INSERT INTO agents(name,kind,status,created_at,pruned_at,pruned_reason) VALUES('retired','worker','pruned',?,?,?)",
            [s.now(), s.now(), hive_board::RETIRE_REASON.to_string()],
        ).unwrap();
        let (code, body) = http::additions_request(h, "GET", "/api/v1/runner", "");
        assert_eq!(code, 200);
        let workers = body["workers"].as_object().unwrap();
        assert_eq!(workers["pool"], 2);
        assert_eq!(workers["busy"], 1);
        assert_eq!(workers["idle"], 1);
        assert_eq!(workers["idle_names"], json!(["between"]));
        assert!(
            !workers.contains_key("target"),
            "workers are not kept idle, so the summary has no target: {workers:?}"
        );
        assert!(workers.contains_key("prune_after_days"));
    }

    // runs_today is what the daily cap (jobs_per_day) stops at: distinct jobs
    // that ran today, so a job whose run was held or lost and ran again is one
    // job, an errored run is none, a dry run is none and yesterday is gone.
    #[test]
    fn runner_view_counts_the_distinct_jobs_the_daily_cap_counts() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(true);
        let h = http::handler(&s);
        let exec = |sql: &str| s.conn().unwrap().execute(sql, []).unwrap();
        for (id, job, status, started, mode) in [
            ("r1", "job-a", "done", "2026-09-06T08:00:00Z", "real"),
            ("r2", "job-a", "done", "2026-09-06T09:00:00Z", "real"),
            ("r3", "job-b", "held", "2026-09-06T09:10:00Z", "real"),
            ("r4", "job-b", "running", "2026-09-06T09:20:00Z", "real"),
            ("r5", "job-c", "lost", "2026-09-06T10:00:00Z", "real"),
            ("r6", "job-d", "error", "2026-09-06T10:30:00Z", "real"),
            ("r7", "job-e", "done", "2026-09-06T11:00:00Z", "dry-run"),
            ("r8", "job-f", "done", "2026-09-05T23:59:00Z", "real"),
        ] {
            exec(&format!(
                "INSERT INTO runs(id,job_id,agent,project,started_at,status,mode) VALUES('{id}','{job}','w','commons','{started}','{status}','{mode}')"
            ));
        }
        s.set_setting("human", "jobs_per_day", "10").unwrap();
        let (code, body) = http::additions_request(h, "GET", "/api/v1/runner", "");
        assert_eq!(code, 200);
        let jobs = body["jobs"].as_object().unwrap();
        assert_eq!(jobs["runs_today"], 2, "job-a once, job-b once: {jobs:?}");
        assert_eq!(jobs["cap"], 10);
    }

    #[test]
    fn runner_view_carries_capacity_hold() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(true);
        let h = http::handler(&s);
        const HOLD: &str = "workers on this machine wait for a GitHub Copilot login: Sign in with GitHub (Settings -> Harness) or run hive harness login";
        s.conn().unwrap().execute(&format!(r#"INSERT INTO meta(key,value) VALUES ('capacity','{{"queued":2,"slots":0,"inflight":0,"starved_since":"","held":"{HOLD}"}}')"#), []).unwrap();
        let (code, body) = http::additions_request(h, "GET", "/api/v1/runner", "");
        assert_eq!(code, 200);
        let want_capacity = format!("2 job(s) queued, 0 slot(s), 0 in flight; {HOLD}");
        assert_eq!(body["capacity"], want_capacity);
        assert_eq!(body["hold"], HOLD);
    }

    #[test]
    fn setting_labels_cover_catalogue() {
        let _serial = crate::test_helpers::serial();
        let src = match std::fs::read_to_string("../../web/hub/src/lib/setting-labels.ts") {
            Ok(src) => src,
            Err(err) => {
                eprintln!("setting-labels.ts not in this checkout: {err}");
                return;
            }
        };
        let label_re =
            regex::Regex::new(r#"(?m)^\s*"?([a-z_][a-z0-9_.-]*)"?:\s*"[^"]+",?$"#).unwrap();
        let labels = label_re
            .captures_iter(&src)
            .map(|m| m[1].to_string())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(
            labels.len() >= 10,
            "parsed only {} labels from setting-labels.ts; the LABELS map changed shape",
            labels.len()
        );
        let mut missing = Vec::new();
        let mut names = hive_store::setting_names();
        names.extend(hive_store::extra_setting_names());
        for name in names {
            if !labels.contains(&name) {
                missing.push(name);
            }
        }
        assert!(
            missing.is_empty(),
            "settings without a human label in web/hub/src/lib/setting-labels.ts: {}",
            missing.join(", ")
        );
    }
}
