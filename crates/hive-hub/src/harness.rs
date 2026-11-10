use crate::httpx::{Request, ResponseWriter};
use crate::server::{Handler, JsonMap};
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct HarnessSessionRow {
    #[serde(flatten)]
    pub(crate) harness_session: hive_store::HarnessSession,
    pub(crate) job_title: String,
}

impl Handler {
    pub(crate) fn addition_harness(&self, w: &mut ResponseWriter, r: &Request, parts: Vec<String>) {
        if parts.is_empty() || parts[0] != "harness" || parts.len() > 4 {
            crate::addition_not_found(w);
            return;
        }
        if !self.addition_available(w, r, vec!["harness_sessions".to_string()]) {
            return;
        }
        match parts.as_slice() {
            [_] => {
                if crate::addition_method(w, r, vec!["GET".to_string()]) {
                    self.addition_harness_status(w, r);
                }
            }
            [_, sub] if sub == "models" => {
                if crate::addition_method(w, r, vec!["GET".to_string()]) {
                    self.addition_harness_models(w, r);
                }
            }
            [_, sub, name] if sub == "providers" => {
                if crate::addition_method(w, r, vec!["GET".to_string()]) {
                    self.addition_harness_provider(w, r, name.clone());
                }
            }
            [_, sub] if sub == "prompt" => {
                if crate::addition_method(w, r, vec!["GET".to_string()]) {
                    self.addition_harness_prompt(w, r);
                }
            }
            [_, sub] if sub == "login" => {
                if crate::addition_method(w, r, vec!["POST".to_string()]) {
                    self.addition_harness_login(w, r);
                }
            }
            [_, sub, tail] if sub == "login" && tail == "poll" => {
                if crate::addition_method(w, r, vec!["POST".to_string()]) {
                    self.addition_harness_login_poll(w, r);
                }
            }
            [_, sub, tail] if sub == "login" && tail == "token" => {
                if crate::addition_method(w, r, vec!["POST".to_string()]) {
                    self.addition_harness_token_login(w, r);
                }
            }
            [_, sub] if sub == "logout" => {
                if crate::addition_method(w, r, vec!["POST".to_string()]) {
                    self.addition_harness_logout(w, r);
                }
            }
            [_, sub] if sub == "sessions" => {
                if crate::addition_method(w, r, vec!["GET".to_string()]) {
                    self.addition_harness_sessions(w, r);
                }
            }
            [_, sub, id] if sub == "sessions" => {
                if crate::addition_method(w, r, vec!["GET".to_string()]) {
                    self.addition_harness_session(w, r, id.clone());
                }
            }
            [_, sub, id, action] if sub == "sessions" && action == "cancel" => {
                if crate::addition_method(w, r, vec!["POST".to_string()]) {
                    self.addition_harness_cancel(w, r, id.clone());
                }
            }
            [_, sub, id, action] if sub == "sessions" && action == "trajectory" => {
                if crate::addition_method(w, r, vec!["GET".to_string()]) {
                    self.addition_harness_trajectory(w, r, id.clone());
                }
            }
            [_, sub, id, action] if sub == "sessions" && action == "file" => {
                if crate::addition_method(w, r, vec!["GET".to_string()]) {
                    self.addition_harness_file(w, r, id.clone());
                }
            }
            _ => crate::addition_not_found(w),
        }
    }

    pub(crate) fn addition_harness_status(&self, w: &mut ResponseWriter, r: &Request) {
        let Some(store) = self.store.as_deref() else {
            crate::addition_failure(w, anyhow::anyhow!("store is not available"));
            return;
        };
        let p = match self.harness_provider(String::new()) {
            Ok(p) => p,
            Err(err) => {
                crate::addition_failure(w, err);
                return;
            }
        };
        let login = self.harness_provider_status(&r.ctx, p.clone(), Duration::from_secs(5));
        let counts = match store.harness_session_counts() {
            Ok(c) => c,
            Err(err) => {
                crate::addition_failure(w, err);
                return;
            }
        };
        let mut out = JsonMap::new();
        out.insert(
            "provider".into(),
            Value::Object(
                self.harness_provider_summary(p, login)
                    .into_iter()
                    .collect(),
            ),
        );
        out.insert(
            "providers".into(),
            maps_value(self.harness_provider_summaries()),
        );
        let mut models = JsonMap::new();
        models.insert(
            "worker".into(),
            Value::String(store.setting("worker_model", "")),
        );
        models.insert(
            "light".into(),
            Value::String(store.setting("light_model", "")),
        );
        models.insert(
            "measure".into(),
            Value::String(store.setting("measure_model", "")),
        );
        models.insert("ask".into(), Value::String(store.setting("ask.model", "")));
        out.insert("models".into(), Value::Object(models.into_iter().collect()));
        let mut limits = JsonMap::new();
        limits.insert(
            "max_steps".into(),
            Value::Number(Number::from(hive_harness::max_steps_setting(store))),
        );
        limits.insert(
            "step_timeout_seconds".into(),
            Value::Number(Number::from(store.setting_int("harness.step_timeout"))),
        );
        limits.insert(
            "output_limit".into(),
            Value::Number(Number::from(store.setting_int("harness.output_limit"))),
        );
        limits.insert(
            "minutes_per_job".into(),
            Value::Number(Number::from(store.setting_int("minutes_per_job"))),
        );
        out.insert("limits".into(), Value::Object(limits.into_iter().collect()));
        out.insert("sessions".into(), go_value(&counts));
        out.insert("settings".into(), maps_value(self.harness_settings()));
        crate::write_json(w, 200, Value::Object(out.into_iter().collect()));
    }

    pub(crate) fn addition_harness_prompt(&self, w: &mut ResponseWriter, _r: &Request) {
        let Some(store) = self.store.as_deref() else {
            crate::addition_failure(w, anyhow::anyhow!("store is not available"));
            return;
        };
        let tmpl = store.setting("harness.system_prompt", "");
        let render_template = if tmpl.is_empty() {
            hive_harness::DEFAULT_PROMPT_TEMPLATE.to_string()
        } else {
            tmpl.clone()
        };
        let data = hive_harness::PromptData {
            cwd: "<project clone>".to_string(),
            provider: store.setting("provider", hive_provider::DEFAULT),
            model: store.run_model("task"),
            step_timeout: Duration::from_secs(store.setting_int("harness.step_timeout") as u64)
                .as_secs()
                .to_string()
                + "s",
            output_limit: store.setting_int("harness.output_limit") as usize,
            max_steps: hive_harness::max_steps_setting(store),
            info: BTreeMap::new(),
            // The preview has no clone to read: it stands in for the
            // repository instructions a run would load.
            instructions: if store.setting("harness.instructions", "on") == "off" {
                String::new()
            } else {
                "<the clone's AGENTS.md, CLAUDE.md, GEMINI.md and .github/copilot-instructions.md, when it has them>".to_string()
            },
            memory: if store.setting("harness.memory", "on") == "off" {
                String::new()
            } else {
                "<what earlier runs remembered in ~/.hive/memory/<project>.md and ~/.hive/memory/user.md, when there is any>".to_string()
            },
            teach: store.setting("harness.teach", "off") == "on",
        };
        let rendered = hive_harness::render_prompt(&render_template, data);
        let mut out = JsonMap::new();
        out.insert("template".into(), Value::String(tmpl.clone()));
        out.insert(
            "default".into(),
            Value::String(hive_harness::DEFAULT_PROMPT_TEMPLATE.to_string()),
        );
        out.insert("custom".into(), Value::Bool(!tmpl.is_empty()));
        match rendered {
            Ok(text) => out.insert("rendered".into(), Value::String(text)),
            Err(err) => {
                out.insert("rendered".into(), Value::String(String::new()));
                out.insert("error".into(), Value::String(err.to_string()))
            }
        };
        crate::write_json(w, 200, Value::Object(out.into_iter().collect()));
    }

    /// `GET /harness/models[?provider=NAME]`: the models of the named
    /// provider, else of the active one.
    pub(crate) fn addition_harness_models(&self, w: &mut ResponseWriter, r: &Request) {
        let Some(p) = self.harness_provider_named(w, r.query("provider")) else {
            return;
        };
        let cancel = hive_provider::CancelToken::new().with_timeout(Duration::from_secs(15));
        match p.models(&cancel) {
            Ok(models) => {
                let mut out = JsonMap::new();
                out.insert("models".into(), go_value(&models));
                out.insert("fetched_at".into(), Value::String(hive_store::now()));
                crate::write_json(w, 200, Value::Object(out.into_iter().collect()));
            }
            Err(err) if hive_provider::is_not_logged_in(&err) => crate::write_error(
                w,
                503,
                "not signed in".to_string(),
                format!("The {} sign-in is missing on this machine.", p.label()),
                format!("Sign in to {} in Settings -> Harness.", p.label()),
            ),
            Err(err) => crate::write_error(
                w,
                502,
                "provider failed".to_string(),
                self.redacted_provider_error(err.to_string(), vec![]),
                "Retry, or sign in again.".to_string(),
            ),
        }
    }

    pub(crate) fn addition_harness_login(&self, w: &mut ResponseWriter, r: &Request) {
        let (payload, ok) = crate::read_payload(w, r);
        if !ok {
            return;
        }
        let _guard = crate::ADDITION_WRITES
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let Some(p) = self.harness_provider_named(w, payload_provider(&payload)) else {
            return;
        };
        let Some(loginer) = p.as_device_login() else {
            crate::write_error(
                w,
                400,
                "device login is not supported".into(),
                "This provider does not implement device login.".into(),
                "Choose a provider that supports device login.".into(),
            );
            return;
        };
        let cancel = hive_provider::CancelToken::new().with_timeout(Duration::from_secs(15));
        match loginer.begin_device_login(&cancel) {
            Ok(flow) => crate::write_json(w, 200, go_value(&flow)),
            Err(err) => crate::write_error(
                w,
                502,
                "provider failed".into(),
                self.redacted_provider_error(err.to_string(), vec![]),
                "Retry, or sign in another way.".into(),
            ),
        }
    }

    pub(crate) fn addition_harness_login_poll(&self, w: &mut ResponseWriter, r: &Request) {
        let (payload, ok) = crate::read_payload(w, r);
        if !ok {
            return;
        }
        let device_code = payload
            .get("device_code")
            .map(str_value)
            .unwrap_or_default()
            .trim()
            .to_string();
        if device_code.is_empty() {
            crate::write_error(
                w,
                400,
                "missing device_code".into(),
                "The device login poll needs the device_code returned by /harness/login.".into(),
                "Send device_code.".into(),
            );
            return;
        }
        let _guard = crate::ADDITION_WRITES
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let Some(p) = self.harness_provider_named(w, payload_provider(&payload)) else {
            return;
        };
        let Some(loginer) = p.as_device_login() else {
            crate::write_error(
                w,
                400,
                "device login is not supported".into(),
                "This provider does not implement device login.".into(),
                "Choose a provider that supports device login.".into(),
            );
            return;
        };
        let cancel = hive_provider::CancelToken::new().with_timeout(Duration::from_secs(15));
        match loginer.poll_device_login(&cancel, &device_code) {
            Ok((status, mut login)) => {
                if status == hive_provider::DEVICE_COMPLETE {
                    self.invalidate_harness_provider(p.name().to_string());
                    if let Ok(next) = self.harness_provider(p.name().to_string()) {
                        login = provider_login_value(self.harness_provider_status(
                            &r.ctx,
                            next,
                            Duration::from_secs(5),
                        ));
                    }
                } else {
                    login = hive_provider::Login::default();
                }
                let mut out = JsonMap::new();
                out.insert("status".into(), Value::String(status));
                out.insert("login".into(), go_value(&login));
                crate::write_json(w, 200, Value::Object(out.into_iter().collect()));
            }
            Err(err) => crate::write_error(
                w,
                502,
                "provider failed".into(),
                self.redacted_provider_error(err.to_string(), vec![]),
                "Start the sign-in again, or sign in another way.".into(),
            ),
        }
    }

    pub(crate) fn addition_harness_token_login(&self, w: &mut ResponseWriter, r: &Request) {
        let (payload, ok) = crate::read_payload(w, r);
        if !ok {
            return;
        }
        let token = payload
            .get("token")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if token.is_empty() {
            crate::write_error(
                w,
                400,
                "missing token".into(),
                "Token login needs a non-empty token string.".into(),
                "Paste the API key or token.".into(),
            );
            return;
        }
        let _guard = crate::ADDITION_WRITES
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let Some(p) = self.harness_provider_named(w, payload_provider(&payload)) else {
            return;
        };
        let Some(loginer) = p.as_token_login() else {
            crate::write_error(
                w,
                400,
                "token login is not supported".into(),
                "This provider does not implement token login.".into(),
                "Choose a provider that supports token login.".into(),
            );
            return;
        };
        let cancel = hive_provider::CancelToken::new().with_timeout(Duration::from_secs(15));
        match loginer.login_with_token(&cancel, &token) {
            Ok(login) => {
                self.invalidate_harness_provider(p.name().to_string());
                let mut out = JsonMap::new();
                out.insert("login".into(), go_value(&login));
                crate::write_json(w, 200, Value::Object(out.into_iter().collect()));
            }
            Err(err) => crate::write_error(
                w,
                400,
                "token login failed".into(),
                self.redacted_provider_error(err.to_string(), vec![token]),
                "Paste a valid token.".into(),
            ),
        }
    }

    pub(crate) fn addition_harness_logout(&self, w: &mut ResponseWriter, r: &Request) {
        let (payload, ok) = crate::read_payload(w, r);
        if !ok {
            return;
        }
        let _guard = crate::ADDITION_WRITES
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let Some(p) = self.harness_provider_named(w, payload_provider(&payload)) else {
            return;
        };
        let Some(logouter) = p.as_logouter() else {
            crate::write_error(
                w,
                400,
                "logout is not supported".into(),
                "This provider does not implement logout.".into(),
                "Choose a provider that supports logout.".into(),
            );
            return;
        };
        if let Err(err) = logouter.logout() {
            crate::write_error(
                w,
                502,
                "provider failed".into(),
                self.redacted_provider_error(err.to_string(), vec![]),
                "Retry logout.".into(),
            );
            return;
        }
        self.invalidate_harness_provider(p.name().to_string());
        match self.harness_provider(p.name().to_string()) {
            Ok(next) => {
                let mut out = JsonMap::new();
                out.insert("ok".into(), Value::Bool(true));
                out.insert(
                    "login".into(),
                    self.harness_provider_status(&r.ctx, next, Duration::from_secs(5)),
                );
                crate::write_json(w, 200, Value::Object(out.into_iter().collect()));
            }
            Err(err) => crate::addition_failure(w, err),
        }
    }

    pub(crate) fn addition_harness_sessions(&self, w: &mut ResponseWriter, r: &Request) {
        let Some(store) = self.store.as_deref() else {
            crate::addition_failure(w, anyhow::anyhow!("store is not available"));
            return;
        };
        let (limit, ok) = addition_integer_local(w, r.query("limit"), "limit", 50, 500);
        if !ok {
            return;
        }
        let sessions = match store.harness_sessions(hive_store::HarnessSessionFilter {
            project: r.query("project"),
            agent: r.query("agent"),
            status: r.query("status"),
            job_id: r.query("job"),
            before: r.query("before"),
            limit,
        }) {
            Ok(s) => s,
            Err(err) => {
                crate::addition_failure(w, err);
                return;
            }
        };
        let titles = match self.harness_job_titles(&r.ctx, sessions.clone()) {
            Ok(t) => t,
            Err(err) => {
                crate::addition_failure(w, err);
                return;
            }
        };
        let title_map = titles.as_object().cloned().unwrap_or_default();
        let mut out_rows = Vec::new();
        for mut session in sessions.clone() {
            if session.submission.len() > 2000 {
                session.submission.truncate(2000);
            }
            out_rows.push(HarnessSessionRow {
                job_title: title_map
                    .get(&session.job_id)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                harness_session: session,
            });
        }
        let counts = match store.harness_session_counts() {
            Ok(c) => c,
            Err(err) => {
                crate::addition_failure(w, err);
                return;
            }
        };
        let mut payload = JsonMap::new();
        payload.insert("sessions".into(), go_value(&out_rows));
        payload.insert("counts".into(), go_value(&counts));
        if sessions.len() == limit as usize && !sessions.is_empty() {
            payload.insert(
                "next_before".into(),
                Value::String(hive_store::harness_session_cursor(sessions.last().unwrap())),
            );
        }
        crate::write_json(w, 200, Value::Object(payload.into_iter().collect()));
    }

    pub(crate) fn addition_harness_session(&self, w: &mut ResponseWriter, r: &Request, id: String) {
        let Some(store) = self.store.as_deref() else {
            crate::addition_failure(w, anyhow::anyhow!("store is not available"));
            return;
        };
        if !r.query_all("after").is_empty() {
            let (after, ok) = addition_integer_local(w, r.query("after"), "after", 0, 0);
            if !ok {
                return;
            }
            let session = match store.harness_session_head(&id) {
                Ok(Some(s)) => s,
                Ok(None) => {
                    harness_not_found(w);
                    return;
                }
                Err(err) => {
                    crate::addition_failure(w, err);
                    return;
                }
            };
            let title = match self.harness_job_title(&r.ctx, session.job_id.clone()) {
                Ok(t) => t,
                Err(err) => {
                    crate::addition_failure(w, err);
                    return;
                }
            };
            let steps = match store.harness_steps(&id, after) {
                Ok(s) => s,
                Err(err) => {
                    crate::addition_failure(w, err);
                    return;
                }
            };
            let mut out = JsonMap::new();
            out.insert(
                "session".into(),
                go_value(&HarnessSessionRow {
                    harness_session: session,
                    job_title: title,
                }),
            );
            out.insert("steps".into(), go_value(&steps));
            out.insert("after".into(), Value::Number(Number::from(after)));
            crate::write_json(w, 200, Value::Object(out.into_iter().collect()));
            return;
        }
        let (session, steps) = match store.harness_session(&id) {
            Ok(v) => v,
            Err(err) => {
                crate::addition_failure(w, err);
                return;
            }
        };
        let Some(session) = session else {
            harness_not_found(w);
            return;
        };
        let title = match self.harness_job_title(&r.ctx, session.job_id.clone()) {
            Ok(t) => t,
            Err(err) => {
                crate::addition_failure(w, err);
                return;
            }
        };
        let mut out = JsonMap::new();
        out.insert(
            "session".into(),
            go_value(&HarnessSessionRow {
                harness_session: session,
                job_title: title,
            }),
        );
        out.insert("steps".into(), go_value(&steps));
        crate::write_json(w, 200, Value::Object(out.into_iter().collect()));
    }

    pub(crate) fn addition_harness_cancel(&self, w: &mut ResponseWriter, _r: &Request, id: String) {
        let Some(store) = self.store.as_deref() else {
            crate::addition_failure(w, anyhow::anyhow!("store is not available"));
            return;
        };
        let _guard = crate::ADDITION_WRITES
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut pending = false;
        match store.request_harness_cancel(&id) {
            Ok(true) => {}
            Ok(false) => {
                crate::write_error(
                    w,
                    409,
                    "session is not running".into(),
                    "The harness session is not running.".into(),
                    "Select a running session.".into(),
                );
                return;
            }
            Err(err)
                if err
                    .downcast_ref::<hive_store::ErrHarnessCancelPending>()
                    .is_some() =>
            {
                pending = true;
            }
            Err(err) => {
                if matches!(
                    err.downcast_ref::<rusqlite::Error>(),
                    Some(rusqlite::Error::QueryReturnedNoRows)
                ) {
                    harness_not_found(w);
                    return;
                }
                crate::addition_failure(w, err);
                return;
            }
        }
        let session = match store.harness_session_head(&id) {
            Ok(Some(s)) => s,
            Ok(None) => {
                harness_not_found(w);
                return;
            }
            Err(err) => {
                crate::addition_failure(w, err);
                return;
            }
        };
        if !pending {
            let mut detail = serde_json::Map::new();
            detail.insert("session".to_string(), Value::String(id.clone()));
            detail.insert("job".to_string(), Value::String(session.job_id.clone()));
            if let Err(err) = store.event(
                hive_store::HUMAN_AGENT,
                "harness.cancel",
                &id,
                Value::Object(detail),
            ) {
                crate::addition_failure(w, err);
                return;
            }
        }
        let mut out = JsonMap::new();
        out.insert("cancelled".into(), Value::Bool(true));
        out.insert("pending".into(), Value::Bool(pending));
        out.insert("session".into(), go_value(&session));
        crate::write_json(w, 200, Value::Object(out.into_iter().collect()));
    }

    pub(crate) fn addition_harness_trajectory(
        &self,
        w: &mut ResponseWriter,
        _r: &Request,
        id: String,
    ) {
        let Some(store) = self.store.as_deref() else {
            crate::addition_failure(w, anyhow::anyhow!("store is not available"));
            return;
        };
        let session = match store.harness_session_head(&id) {
            Ok(Some(s)) if !s.trajectory.is_empty() => s,
            _ => {
                harness_not_found(w);
                return;
            }
        };
        let (path, ok) = self.safe_harness_trajectory(session.trajectory);
        if !ok {
            harness_not_found(w);
            return;
        }
        let mut data = Vec::new();
        if std::fs::File::open(&path)
            .and_then(|mut f| f.read_to_end(&mut data))
            .is_err()
        {
            harness_not_found(w);
            return;
        }
        w.header_mut().insert(
            hyper::header::CONTENT_TYPE,
            hyper::header::HeaderValue::from_static("application/json"),
        );
        let disposition = format!("attachment; filename=\"{}.json\"", id.replace('"', ""));
        if let Ok(v) = hyper::header::HeaderValue::from_str(&disposition) {
            w.header_mut().insert(hyper::header::CONTENT_DISPOSITION, v);
        }
        w.header_mut().insert(
            hyper::header::CACHE_CONTROL,
            hyper::header::HeaderValue::from_static("no-store"),
        );
        w.write_header(200);
        let _ = w.write(&data);
    }

    /// `GET /api/v1/harness/sessions/{id}/file?path=REL`: one text file of
    /// the run's workspace, the way the canvas opens a file the agent is
    /// editing. The path is relative to the session's `cwd` and must stay
    /// inside it (symlinks resolved); the hub's own private folder and the
    /// git store are not files to read. Big files come back cut at
    /// [`SESSION_FILE_LIMIT`] with `truncated: true`; a binary one says so
    /// instead of its bytes.
    pub(crate) fn addition_harness_file(&self, w: &mut ResponseWriter, r: &Request, id: String) {
        let Some(store) = self.store.as_deref() else {
            crate::addition_failure(w, anyhow::anyhow!("store is not available"));
            return;
        };
        let session = match store.harness_session_head(&id) {
            Ok(Some(s)) => s,
            Ok(None) => {
                harness_not_found(w);
                return;
            }
            Err(err) => {
                crate::addition_failure(w, err);
                return;
            }
        };
        let rel = r.query("path");
        if rel.trim().is_empty() {
            crate::write_error(
                w,
                400,
                "Missing path".to_string(),
                "The file route needs the file's path inside the run's workspace.".to_string(),
                "Send GET /api/v1/harness/sessions/ID/file?path=src/lib.rs.".to_string(),
            );
            return;
        }
        if session.cwd.is_empty() || !Path::new(&session.cwd).is_dir() {
            crate::write_error(
                w,
                404,
                "Workspace gone".to_string(),
                format!(
                    "The run worked in {}, which is no longer on this machine.",
                    if session.cwd.is_empty() {
                        "no folder".to_string()
                    } else {
                        session.cwd.clone()
                    }
                ),
                "Read the file in the branch's diff, or open the project's checkout.".to_string(),
            );
            return;
        }
        let path = match session_file_path(&session.cwd, &rel) {
            Ok(p) => p,
            Err(why) => {
                crate::write_error(
                    w,
                    404,
                    "No such file".to_string(),
                    format!("{rel}: {why}."),
                    "Pick a file from the run's edits or the branch's changes.".to_string(),
                );
                return;
            }
        };
        let Ok(mut file) = std::fs::File::open(&path) else {
            crate::write_error(
                w,
                404,
                "No such file".to_string(),
                format!("{rel} could not be opened in the run's workspace."),
                "Pick a file from the run's edits or the branch's changes.".to_string(),
            );
            return;
        };
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        let mut data = Vec::new();
        if file
            .by_ref()
            .take(SESSION_FILE_LIMIT as u64 + 1)
            .read_to_end(&mut data)
            .is_err()
        {
            harness_not_found(w);
            return;
        }
        let binary = !crate::chat::looks_like_text(&data);
        let truncated = data.len() > SESSION_FILE_LIMIT;
        if truncated {
            data.truncate(SESSION_FILE_LIMIT);
        }
        let text = if binary {
            String::new()
        } else {
            String::from_utf8_lossy(&data).into_owned()
        };
        crate::write_json(
            w,
            200,
            serde_json::json!({
                "session": id,
                "path": rel,
                "size": size,
                "text": text,
                "truncated": truncated,
                "binary": binary,
            }),
        );
    }

    pub(crate) fn safe_harness_trajectory(&self, path: String) -> (String, bool) {
        let Some(store) = self.store.as_deref() else {
            return (String::new(), false);
        };
        let Ok(base) = std::fs::canonicalize(Path::new(&store.home).join("harness")) else {
            return (String::new(), false);
        };
        let Ok(abs) = std::fs::canonicalize(Path::new(&path)) else {
            return (String::new(), false);
        };
        if !abs.starts_with(&base) || abs == base {
            return (String::new(), false);
        }
        (abs.display().to_string(), true)
    }

    /// `GET /harness/providers/{name}`: one provider's sign-in ways and its
    /// login on this machine, active or not, so the console can connect a
    /// provider before switching to it.
    pub(crate) fn addition_harness_provider(
        &self,
        w: &mut ResponseWriter,
        r: &Request,
        name: String,
    ) {
        let name = name.trim().to_string();
        if !hive_provider::names().contains(&name) {
            crate::write_error(
                w,
                404,
                "unknown provider".into(),
                format!("No model provider is called {name:?}."),
                format!("Use one of: {}.", hive_provider::names().join(", ")),
            );
            return;
        }
        let p = match self.harness_provider(name) {
            Ok(p) => p,
            Err(err) => {
                crate::addition_failure(w, err);
                return;
            }
        };
        let login = self.harness_provider_status(&r.ctx, p.clone(), Duration::from_secs(5));
        crate::write_json(
            w,
            200,
            Value::Object(
                self.harness_provider_summary(p, login)
                    .into_iter()
                    .collect(),
            ),
        );
    }

    /// The provider a call names (`provider` in its payload or query), else
    /// the active one. An unknown name answers 400 and gives `None`.
    fn harness_provider_named(
        &self,
        w: &mut ResponseWriter,
        name: String,
    ) -> Option<Arc<dyn hive_provider::Provider>> {
        let name = name.trim().to_string();
        if !name.is_empty() && !hive_provider::names().contains(&name) {
            crate::write_error(
                w,
                400,
                "unknown provider".into(),
                format!("No model provider is called {name:?}."),
                format!("Use one of: {}.", hive_provider::names().join(", ")),
            );
            return None;
        }
        match self.harness_provider(name) {
            Ok(p) => Some(p),
            Err(err) => {
                crate::addition_failure(w, err);
                None
            }
        }
    }

    pub(crate) fn harness_provider(
        &self,
        name: String,
    ) -> Result<Arc<dyn hive_provider::Provider>> {
        let store = self
            .store
            .as_deref()
            .ok_or_else(|| anyhow!("store is not available"))?;
        let mut name = name.trim().to_string();
        if name.is_empty() {
            name = store.setting("provider", "");
        }
        if name.trim().is_empty() {
            name = hive_provider::DEFAULT.to_string();
        }
        let settings = store.provider_settings();
        let key = format!(
            "{name}\0{}",
            provider_settings_fingerprint(settings.clone())
        );
        let mut cache = self
            .provider_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(p) = cache.get(&key) {
            return Ok(p.clone());
        }
        let p = hive_provider::open(
            &name,
            hive_provider::Options {
                home: store.home.clone(),
                settings,
                http: None,
                env: None,
                log: None,
            },
        )?;
        let p: Arc<dyn hive_provider::Provider> = Arc::from(p);
        cache.insert(key, p.clone());
        Ok(p)
    }

    pub(crate) fn invalidate_harness_provider(&self, name: String) {
        let name = name.trim();
        let prefix = format!("{name}\0");
        let mut cache = self
            .provider_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        cache.retain(|key, _| key != name && !key.starts_with(&prefix));
    }

    pub(crate) fn harness_provider_status(
        &self,
        _ctx: &hive_bus::CancellationToken,
        p: Arc<dyn hive_provider::Provider>,
        timeout: Duration,
    ) -> Value {
        let cancel = hive_provider::CancelToken::new().with_timeout(timeout);
        match p.status(&cancel) {
            Ok(login) => go_value(&login),
            Err(err) => {
                let login = hive_provider::Login {
                    logged_in: false,
                    detail: self.redacted_provider_error(err.to_string(), vec![]),
                    ..hive_provider::Login::default()
                };
                go_value(&login)
            }
        }
    }

    pub(crate) fn harness_provider_summary(
        &self,
        p: Arc<dyn hive_provider::Provider>,
        login: Value,
    ) -> JsonMap {
        let mut out = JsonMap::new();
        out.insert("name".into(), Value::String(p.name().to_string()));
        out.insert("label".into(), Value::String(p.label().to_string()));
        out.insert(
            "device_login".into(),
            Value::Bool(implements_device_login(p.clone())),
        );
        out.insert("token_login".into(), Value::Bool(implements_token_login(p)));
        out.insert("login".into(), login);
        out
    }

    pub(crate) fn harness_provider_summaries(&self) -> Vec<JsonMap> {
        hive_provider::names()
            .into_iter()
            .map(|name| match self.harness_provider(name.clone()) {
                Ok(p) => {
                    let mut row = JsonMap::new();
                    row.insert("name".into(), Value::String(p.name().to_string()));
                    row.insert("label".into(), Value::String(p.label().to_string()));
                    row.insert(
                        "device_login".into(),
                        Value::Bool(implements_device_login(p.clone())),
                    );
                    row.insert("token_login".into(), Value::Bool(implements_token_login(p)));
                    row
                }
                Err(_) => {
                    let mut row = JsonMap::new();
                    row.insert("name".into(), Value::String(name.clone()));
                    row.insert(
                        "label".into(),
                        Value::String(hive_provider::label_of(&name)),
                    );
                    row.insert("device_login".into(), Value::Bool(false));
                    row.insert("token_login".into(), Value::Bool(false));
                    row
                }
            })
            .collect()
    }

    pub(crate) fn harness_settings(&self) -> Vec<JsonMap> {
        let Some(store) = self.store.as_deref() else {
            return Vec::new();
        };
        // The endpoint setting of every provider choice that has one, in
        // catalogue order, so the payload follows the registry as it grows.
        let endpoints: Vec<String> = hive_store::HARNESS_RULES
            .get("provider")
            .map(|rule| {
                rule.choices
                    .iter()
                    .map(|name| format!("provider.{name}.base_url"))
                    .filter(|key| hive_store::HARNESS_RULES.contains_key(key))
                    .collect()
            })
            .unwrap_or_default();
        let mut names = vec!["provider".to_string()];
        names.extend(endpoints);
        names.extend(
            [
                // The tenant of the azure-openai Microsoft sign-in sits by
                // the endpoints so the console shows it with that provider.
                "provider.azure-openai.tenant",
                "harness.max_steps",
                "minutes_per_job",
                "harness.max_cost",
                "harness.keep_observations",
                "harness.context_window",
                "harness.system_prompt",
                "harness.instructions",
                "harness.mcp_tools",
                "harness.skills",
                "harness.hooks",
                "harness.subagents",
                "harness.memory",
                "harness.tasks",
                "harness.web_search",
                "harness.fetch_allow",
                "harness.allow_tools",
                "harness.deny_tools",
                "harness.compact_at",
                "harness.trim_keep_recent",
                "harness.trim_over",
                "harness.reasoning",
                "harness.step_timeout",
                "harness.commit_timeout",
                "harness.output_limit",
                "worker_model",
                "light_model",
                "measure_model",
                "ask.model",
                "worker_reasoning",
                "light_reasoning",
                "measure_reasoning",
            ]
            .into_iter()
            .map(str::to_string),
        );
        names
            .iter()
            .map(|name| {
                let name = name.as_str();
                let rule = hive_store::SETTING_RULES
                    .get(name)
                    .or_else(|| hive_store::MODEL_RULES.get(name))
                    .or_else(|| hive_store::HARNESS_RULES.get(name))
                    .cloned()
                    .unwrap_or_default();
                let mut row = JsonMap::new();
                row.insert("name".into(), Value::String(name.to_string()));
                row.insert("value".into(), Value::String(store.setting(name, "")));
                row.insert("kind".into(), Value::String(rule.kind));
                // `choices` is null for a rule without choices, never [].
                row.insert(
                    "choices".into(),
                    if rule.choices.is_empty() {
                        Value::Null
                    } else {
                        go_value(&rule.choices)
                    },
                );
                row.insert("description".into(), Value::String(rule.description));
                row
            })
            .collect()
    }

    pub(crate) fn harness_job_title(
        &self,
        _ctx: &hive_bus::CancellationToken,
        job_id: String,
    ) -> Result<String> {
        if job_id.trim().is_empty() {
            return Ok(String::new());
        }
        let store = self
            .store
            .as_deref()
            .ok_or_else(|| anyhow!("store is not available"))?;
        let conn = store.conn()?;
        match conn.query_row("SELECT title FROM jobs WHERE id = ?", [&job_id], |r| {
            r.get(0)
        }) {
            Ok(title) => Ok(title),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(String::new()),
            Err(err) => Err(err.into()),
        }
    }

    pub(crate) fn harness_job_titles(
        &self,
        _ctx: &hive_bus::CancellationToken,
        sessions: Vec<hive_store::HarnessSession>,
    ) -> Result<Value> {
        let store = self
            .store
            .as_deref()
            .ok_or_else(|| anyhow!("store is not available"))?;
        let mut ids = Vec::new();
        for session in sessions {
            if !session.job_id.is_empty() && !ids.contains(&session.job_id) {
                ids.push(session.job_id);
            }
        }
        let mut titles = JsonMap::new();
        if ids.is_empty() {
            return Ok(Value::Object(titles.into_iter().collect()));
        }
        let placeholders = vec!["?"; ids.len()].join(",");
        let sql = format!("SELECT id, title FROM jobs WHERE id IN ({placeholders})");
        let conn = store.conn()?;
        let mut stmt = conn.prepare(&sql)?;
        let args = ids
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect::<Vec<_>>();
        let mut rows = stmt.query(args.as_slice())?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            titles.insert(id, Value::String(title));
        }
        Ok(Value::Object(titles.into_iter().collect()))
    }

    pub(crate) fn redacted_provider_error(&self, err: String, secrets: Vec<String>) -> String {
        let mut message = err.split_whitespace().collect::<Vec<_>>().join(" ");
        if message.is_empty() {
            return String::new();
        }
        for secret in secrets {
            let secret = secret.trim();
            if !secret.is_empty() {
                message = message.replace(secret, hive_bus::PRIVATE_VALUE);
            }
        }
        message
    }
}

fn harness_not_found(w: &mut ResponseWriter) {
    crate::write_error(
        w,
        404,
        "not found".into(),
        "No harness session matches the request.".into(),
        "Refresh the harness session list.".into(),
    );
}

/// How much of a workspace file the hub sends to the canvas.
pub(crate) const SESSION_FILE_LIMIT: usize = 512 * 1024;

/// The absolute path of `rel` inside `cwd`, or why it is not a file the hub
/// reads: an absolute or `..` path, a path through `.git` or the hub's
/// private folder, a symlink that leaves the workspace, or no file there.
pub(crate) fn session_file_path(
    cwd: &str,
    rel: &str,
) -> std::result::Result<std::path::PathBuf, &'static str> {
    use std::path::Component;
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err("the path must be relative to the run's workspace");
    }
    for c in rel_path.components() {
        match c {
            Component::Normal(part) => {
                let part = part.to_string_lossy();
                if part == ".git" {
                    return Err("the git store is not a file to read");
                }
            }
            Component::CurDir => {}
            _ => return Err("the path must stay inside the run's workspace"),
        }
    }
    let norm = rel.replace('\\', "/");
    if norm.starts_with(".hive/private/") || norm.contains("/.hive/private/") {
        return Err("the hub's private folder is not a file to read");
    }
    let base = std::fs::canonicalize(cwd).map_err(|_| "the workspace is gone")?;
    let abs = std::fs::canonicalize(base.join(rel_path))
        .map_err(|_| "no such file in the run's workspace")?;
    if !abs.starts_with(&base) || abs == base {
        return Err("the path must stay inside the run's workspace");
    }
    if !abs.is_file() {
        return Err("that is a folder, not a file");
    }
    Ok(abs)
}

pub(crate) fn provider_settings_fingerprint(settings: BTreeMap<String, String>) -> String {
    settings
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\0")
}

pub(crate) fn implements_device_login(p: Arc<dyn hive_provider::Provider>) -> bool {
    p.as_device_login().is_some()
}

pub(crate) fn implements_token_login(p: Arc<dyn hive_provider::Provider>) -> bool {
    p.as_token_login().is_some()
}

fn go_value<T: serde::Serialize>(v: &T) -> Value {
    let text = hive_store::gojson::to_string(v).expect("serialize hub payload");
    serde_json::from_str(&text).expect("reparse hub payload")
}

fn maps_value(rows: Vec<JsonMap>) -> Value {
    Value::Array(
        rows.into_iter()
            .map(|m| Value::Object(m.into_iter().collect()))
            .collect(),
    )
}

/// The `provider` a sign-in payload names; empty means the active one.
fn payload_provider(payload: &JsonMap) -> String {
    payload
        .get("provider")
        .map(str_value)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn str_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn provider_login_value(v: Value) -> hive_provider::Login {
    let text = hive_store::gojson::to_string(&v).unwrap_or_default();
    serde_json::from_str(&text).unwrap_or_default()
}

fn addition_integer_local(
    w: &mut ResponseWriter,
    value: String,
    name: &str,
    fallback: i64,
    maximum: i64,
) -> (i64, bool) {
    if value.is_empty() {
        return (fallback, true);
    }
    match value.parse::<i64>() {
        Ok(n) if n >= 0 && (maximum == 0 || n <= maximum) => (n, true),
        _ => {
            crate::write_error(
                w,
                400,
                format!("Invalid {name}"),
                format!("{name} must be a valid integer."),
                format!("Use a valid {name} query parameter."),
            );
            (0, false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    #[derive(Default)]
    struct HubFakeProvider;

    impl hive_provider::Provider for HubFakeProvider {
        fn name(&self) -> &str {
            "hubfake"
        }
        fn label(&self) -> &str {
            "Hub Fake"
        }
        fn chat(
            &self,
            _cancel: &hive_provider::CancelToken,
            _req: hive_provider::Request,
        ) -> anyhow::Result<hive_provider::Response> {
            Ok(hive_provider::Response::default())
        }
        fn models(
            &self,
            _cancel: &hive_provider::CancelToken,
        ) -> anyhow::Result<Vec<hive_provider::Model>> {
            Ok(vec![hive_provider::Model {
                id: "hub-worker".into(),
                name: "Hub Worker".into(),
                vendor: "fake".into(),
                chat: true,
                tool_calls: true,
                vision: false,
                premium: 0.0,
                preview: false,
                context_window: 0,
                max_output: 0,
                default: false,
            }])
        }
        fn status(
            &self,
            _cancel: &hive_provider::CancelToken,
        ) -> anyhow::Result<hive_provider::Login> {
            Ok(hive_provider::Login {
                logged_in: true,
                user: "octocat".into(),
                source: "device login".into(),
                plan: "individual".into(),
                expires_at: "2026-09-07T12:00:00Z".into(),
                ..hive_provider::Login::default()
            })
        }
        fn as_device_login(&self) -> Option<&dyn hive_provider::DeviceLogin> {
            Some(self)
        }
        fn as_token_login(&self) -> Option<&dyn hive_provider::TokenLogin> {
            Some(self)
        }
        fn as_logouter(&self) -> Option<&dyn hive_provider::Logouter> {
            Some(self)
        }
    }

    impl hive_provider::DeviceLogin for HubFakeProvider {
        fn begin_device_login(
            &self,
            _cancel: &hive_provider::CancelToken,
        ) -> anyhow::Result<hive_provider::DeviceFlow> {
            Ok(hive_provider::DeviceFlow {
                device_code: "device-1".into(),
                user_code: "USER-1".into(),
                verification_uri: "https://example.test/device".into(),
                interval: 5,
                expires_in: 600,
            })
        }
        fn poll_device_login(
            &self,
            _cancel: &hive_provider::CancelToken,
            device_code: &str,
        ) -> anyhow::Result<(String, hive_provider::Login)> {
            if device_code != "device-1" {
                return Ok((
                    hive_provider::DEVICE_DENIED.into(),
                    hive_provider::Login::default(),
                ));
            }
            Ok((
                hive_provider::DEVICE_COMPLETE.into(),
                hive_provider::Login {
                    logged_in: true,
                    user: "octocat".into(),
                    source: "device login".into(),
                    plan: "individual".into(),
                    ..hive_provider::Login::default()
                },
            ))
        }
    }

    impl hive_provider::TokenLogin for HubFakeProvider {
        fn login_with_token(
            &self,
            _cancel: &hive_provider::CancelToken,
            token: &str,
        ) -> anyhow::Result<hive_provider::Login> {
            if token == "bad-token" {
                anyhow::bail!("hubfake: invalid token");
            }
            Ok(hive_provider::Login {
                logged_in: true,
                user: "token-user".into(),
                source: "token login".into(),
                plan: "individual".into(),
                ..hive_provider::Login::default()
            })
        }
    }

    impl hive_provider::Logouter for HubFakeProvider {
        fn logout(&self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    fn register_hub_fake_provider() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            hive_provider::register("hubfake", "Hub Fake", |_options| {
                Ok(Box::new(HubFakeProvider))
            });
        });
    }

    fn harness_store() -> crate::test_helpers_http::TestStore {
        register_hub_fake_provider();
        let s = crate::test_helpers_http::additions_store(false);
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO settings (name, value, updated_at, updated_by) VALUES ('provider', 'hubfake', ?, 'human') ON CONFLICT(name) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at, updated_by = excluded.updated_by",
                [&s.now()],
            )
            .unwrap();
        s
    }

    #[test]
    fn addition_harness_status_and_settings() {
        let _serial = crate::test_helpers::serial();
        let s = harness_store();
        s.begin_harness_session(hive_store::HarnessSession {
            id: "HS-running".into(),
            job_id: "JOB-1".into(),
            agent: "alice".into(),
            project: "demo".into(),
            ..hive_store::HarnessSession::default()
        })
        .unwrap();
        s.begin_harness_session(hive_store::HarnessSession {
            id: "HS-failed".into(),
            job_id: "JOB-2".into(),
            agent: "bob".into(),
            project: "demo".into(),
            ..hive_store::HarnessSession::default()
        })
        .unwrap();
        s.finish_harness_session(
            "HS-failed",
            "failed",
            "error",
            "failed submission",
            "boom",
            0.0,
            "",
        )
        .unwrap();
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "GET",
            "/api/v1/harness",
            "",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        // The run-mode switch is gone: the built-in harness is the only one.
        assert!(!result.contains_key("harness"), "{raw}");
        assert_eq!(result["provider"]["name"], "hubfake");
        assert_eq!(result["provider"]["label"], "Hub Fake");
        assert_eq!(result["provider"]["device_login"], true);
        assert_eq!(result["provider"]["token_login"], true);
        assert_eq!(result["provider"]["login"]["user"], "octocat");
        assert_eq!(result["limits"]["max_steps"], 60);
        assert_eq!(result["limits"]["step_timeout_seconds"], 300);
        assert_eq!(result["limits"]["output_limit"], 20000);
        assert_eq!(result["limits"]["minutes_per_job"], 10);
        assert_eq!(result["sessions"]["running"], 1);
        assert_eq!(result["sessions"]["today"], 2);
        assert_eq!(result["sessions"]["failed_today"], 1);
        assert_eq!(result["sessions"]["submitted_today"], 0);
        assert_eq!(result["sessions"]["cost_today"], 0.0);
        let settings = result["settings"].as_array().unwrap();
        for name in [
            "harness.max_steps",
            "minutes_per_job",
            "harness.max_cost",
            "harness.keep_observations",
            "harness.system_prompt",
        ] {
            assert!(
                settings.iter().any(|r| r["name"] == name
                    && !r["value"].is_null()
                    && !r["kind"].is_null()
                    && r["description"].as_str().unwrap_or_default() != ""),
                "{name} missing from /harness settings: {settings:?}"
            );
        }
        // Every provider choice's endpoint rides along, in catalogue order
        // right after the provider row, so a console can show the one
        // that matches the chosen provider.
        let names: Vec<&str> = settings
            .iter()
            .map(|r| r["name"].as_str().unwrap_or_default())
            .collect();
        let provider_at = names.iter().position(|n| *n == "provider").unwrap();
        let endpoints: Vec<String> = hive_store::HARNESS_RULES["provider"]
            .choices
            .iter()
            .filter(|name| *name != "copilot")
            .map(|name| format!("provider.{name}.base_url"))
            .collect();
        assert!(endpoints.len() >= 16, "{endpoints:?}");
        assert_eq!(
            names[provider_at + 1..provider_at + 1 + endpoints.len()],
            endpoints[..]
        );
        let azure = settings
            .iter()
            .find(|r| r["name"] == "provider.azure-openai.base_url")
            .unwrap();
        assert_eq!(azure["value"], "");
        assert_eq!(azure["kind"], "text");
        assert!(azure["choices"].is_null());
        assert_eq!(
            names[provider_at + 1 + endpoints.len()],
            "provider.azure-openai.tenant"
        );
        let tenant = settings
            .iter()
            .find(|r| r["name"] == "provider.azure-openai.tenant")
            .unwrap();
        assert_eq!(tenant["value"], "organizations");
        assert_eq!(tenant["kind"], "text");
        let window = settings
            .iter()
            .find(|r| r["name"] == "harness.context_window")
            .unwrap();
        assert_eq!(window["value"], "0");
        assert_eq!(window["kind"], "count");
        assert!(
            window["description"]
                .as_str()
                .unwrap_or_default()
                .contains("model list")
        );
        let ollama = settings
            .iter()
            .find(|r| r["name"] == "provider.ollama.base_url")
            .unwrap();
        assert_eq!(ollama["value"], "http://127.0.0.1:11434/v1");
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "GET",
            "/api/v1/settings",
            "",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        assert!(
            result["settings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["name"] == "harness.max_steps")
        );
    }

    #[test]
    fn harness_provider_summaries_include_open_ai_token_login() {
        let _serial = crate::test_helpers::serial();
        hive_provider_openai::register();
        hive_provider_anthropic::register();
        hive_provider_gemini::register();
        let s = harness_store();
        let providers = crate::test_helpers_http::handler(&s).harness_provider_summaries();
        let row = providers
            .iter()
            .find(|row| row["name"] == "openai")
            .expect("openai provider missing");
        assert_eq!(row["label"], "OpenAI-compatible");
        assert_eq!(row["device_login"], false);
        assert_eq!(row["token_login"], true);
        // The two vendors with a device code offer both ways in; the
        // console reads these flags to show the matching sign-in button.
        for (name, label) in [
            ("github-models", "GitHub Models"),
            ("azure-openai", "Azure OpenAI"),
        ] {
            let row = providers
                .iter()
                .find(|row| row["name"] == name)
                .unwrap_or_else(|| panic!("{name} provider missing"));
            assert_eq!(row["label"], label);
            assert_eq!(row["device_login"], true, "{name}");
            assert_eq!(row["token_login"], true, "{name}");
        }
        for name in ["gemini", "ollama"] {
            let row = providers
                .iter()
                .find(|row| row["name"] == name)
                .unwrap_or_else(|| panic!("{name} provider missing"));
            assert_eq!(row["device_login"], false, "{name}");
            assert_eq!(row["token_login"], true, "{name}");
        }
        // Anthropic signs in through its own CLI in the browser, or takes a key.
        let row = providers
            .iter()
            .find(|row| row["name"] == "anthropic")
            .expect("anthropic provider missing");
        assert_eq!(row["device_login"], true);
        assert_eq!(row["token_login"], true);
    }

    #[test]
    fn harness_provider_cache_uses_provider_settings_fingerprint() {
        let _serial = crate::test_helpers::serial();
        let s = harness_store();
        let h = crate::test_helpers_http::handler(&s);
        assert!(
            provider_settings_fingerprint(s.provider_settings())
                .contains("provider.openai.base_url=https://api.openai.com/v1")
        );
        let first = h.harness_provider("hubfake".into()).unwrap();
        let again = h.harness_provider("hubfake".into()).unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        assert_eq!(first.name(), "hubfake");
        assert_eq!(again.label(), "Hub Fake");
        s.set_setting("human", "provider.openai.base_url", "https://one.test/v1")
            .unwrap();
        assert!(
            provider_settings_fingerprint(s.provider_settings())
                .contains("provider.openai.base_url=https://one.test/v1")
        );
        let with_one = h.harness_provider("hubfake".into()).unwrap();
        assert!(!Arc::ptr_eq(&first, &with_one));
        let with_one_again = h.harness_provider("hubfake".into()).unwrap();
        assert!(Arc::ptr_eq(&with_one, &with_one_again));
        s.set_setting("human", "provider.openai.base_url", "https://two.test/v1")
            .unwrap();
        let second = h.harness_provider("hubfake".into()).unwrap();
        assert!(!Arc::ptr_eq(&with_one, &second));
        assert_eq!(second.name(), "hubfake");
        h.invalidate_harness_provider("hubfake".into());
        let third = h.harness_provider("hubfake".into()).unwrap();
        assert!(!Arc::ptr_eq(&second, &third));
        assert_eq!(third.label(), "Hub Fake");
    }

    #[test]
    fn addition_harness_models() {
        let _serial = crate::test_helpers::serial();
        let s = harness_store();
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "GET",
            "/api/v1/harness/models",
            "",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["models"].as_array().unwrap().len(), 1);
        assert_ne!(result["fetched_at"], "");
    }

    #[test]
    fn addition_harness_login_flow_token_and_logout() {
        let _serial = crate::test_helpers::serial();
        let s = harness_store();
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "POST",
            "/api/v1/harness/login",
            r#"{"provider":"hubfake"}"#,
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["device_code"], "device-1");
        let (status, result, _) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "POST",
            "/api/v1/harness/login/poll",
            "{}",
            &[],
        );
        assert_eq!(status, 400);
        assert_eq!(result["error"], "missing device_code");
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "POST",
            "/api/v1/harness/login/token",
            r#"{"token":"good-token"}"#,
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["login"]["source"], "token login");
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "POST",
            "/api/v1/harness/logout",
            "{}",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["ok"], true);
    }

    #[test]
    fn addition_harness_sign_in_calls_act_on_the_named_provider() {
        let _serial = crate::test_helpers::serial();
        hive_provider_openai::register();
        let s = harness_store();
        // The active provider has no device code and no key: every call
        // below would fail if it went to the active provider.
        s.set_setting("human", "provider", "openai").unwrap();
        let call = |method: &str, path: &str, body: &str| {
            crate::test_helpers_http::request(
                crate::test_helpers_http::handler(&s),
                method,
                path,
                body,
                &[],
            )
        };
        let (status, result, raw) = call("GET", "/api/v1/harness/providers/hubfake", "");
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["name"], "hubfake");
        assert_eq!(result["label"], "Hub Fake");
        assert_eq!(result["device_login"], true);
        assert_eq!(result["token_login"], true);
        assert_eq!(result["login"]["logged_in"], true);
        assert_eq!(result["login"]["user"], "octocat");
        let (status, result, raw) = call("GET", "/api/v1/harness/providers/nope", "");
        assert_eq!(status, 404, "{raw}");
        assert_eq!(result["error"], "unknown provider");
        let (status, _, raw) = call("POST", "/api/v1/harness/providers/hubfake", "{}");
        assert_eq!(status, 405, "{raw}");

        let (status, result, raw) = call("GET", "/api/v1/harness/models?provider=hubfake", "");
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["models"][0]["id"], "hub-worker");
        let (status, result, raw) = call("GET", "/api/v1/harness/models?provider=nope", "");
        assert_eq!(status, 400, "{raw}");
        assert_eq!(result["error"], "unknown provider");

        let (status, result, raw) =
            call("POST", "/api/v1/harness/login", r#"{"provider":"hubfake"}"#);
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["device_code"], "device-1");
        let (status, result, raw) = call(
            "POST",
            "/api/v1/harness/login/poll",
            r#"{"provider":"hubfake","device_code":"device-1"}"#,
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["status"], "complete");
        assert_eq!(result["login"]["user"], "octocat");
        let (status, result, raw) = call(
            "POST",
            "/api/v1/harness/login/poll",
            r#"{"provider":"nope","device_code":"device-1"}"#,
        );
        assert_eq!(status, 400, "{raw}");
        assert_eq!(result["error"], "unknown provider");
        let (status, result, raw) = call(
            "POST",
            "/api/v1/harness/login/token",
            r#"{"provider":"hubfake","token":"good-token"}"#,
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["login"]["user"], "token-user");
        let (status, result, raw) = call(
            "POST",
            "/api/v1/harness/logout",
            r#"{"provider":"hubfake"}"#,
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["ok"], true);
        assert_eq!(result["login"]["user"], "octocat");
        // None of it switched the active provider.
        assert_eq!(s.setting("provider", ""), "openai");
    }

    #[test]
    fn addition_harness_sessions() {
        let _serial = crate::test_helpers::serial();
        let s = harness_store();
        s.conn().unwrap().execute("INSERT INTO jobs(id,project,kind,title,created_at) VALUES('JOB-1','demo','task','Fix hub API',?)", [&s.now()]).unwrap();
        s.begin_harness_session(hive_store::HarnessSession {
            id: "HS-1".into(),
            run_id: "RUN-1".into(),
            job_id: "JOB-1".into(),
            agent: "alice".into(),
            project: "demo".into(),
            provider: "hubfake".into(),
            model: "hub-worker".into(),
            submission: "x".repeat(2100),
            ..hive_store::HarnessSession::default()
        })
        .unwrap();
        s.append_harness_step(hive_store::HarnessStep {
            session_id: "HS-1".into(),
            n: 1,
            kind: "assistant".into(),
            content: "hello".into(),
            ..hive_store::HarnessStep::default()
        })
        .unwrap();
        s.finish_harness_session(
            "HS-1",
            "failed",
            "error",
            &"x".repeat(2100),
            "boom",
            1.25,
            "trajectory",
        )
        .unwrap();
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "GET",
            "/api/v1/harness/sessions?project=demo&agent=alice&status=failed&job=JOB-1&limit=5",
            "",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        assert!(!result["counts"].is_null());
        assert_eq!(result["sessions"].as_array().unwrap().len(), 1);
        assert_eq!(result["sessions"][0]["id"], "HS-1");
        assert_eq!(result["sessions"][0]["job_title"], "Fix hub API");
        assert_eq!(
            result["sessions"][0]["submission"].as_str().unwrap().len(),
            2000
        );
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "GET",
            "/api/v1/harness/sessions/HS-1",
            "",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["session"]["job_title"], "Fix hub API");
        assert_eq!(
            result["session"]["submission"].as_str().unwrap().len(),
            2100
        );
        assert_eq!(result["steps"].as_array().unwrap().len(), 1);
        assert_eq!(result["steps"][0]["kind"], "assistant");
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "GET",
            "/api/v1/harness/sessions/missing",
            "",
            &[],
        );
        assert_eq!(status, 404, "{raw}");
        assert_eq!(result["error"], "not found");
    }

    #[test]
    fn addition_harness_sessions_paging() {
        let _serial = crate::test_helpers::serial();
        let s = harness_store();
        for (id, at) in [
            ("HS-new", "2026-09-06T04:00:00Z"),
            ("HS-mid", "2026-09-06T03:00:00Z"),
            ("HS-old", "2026-09-06T02:00:00Z"),
        ] {
            s.begin_harness_session(hive_store::HarnessSession {
                id: id.into(),
                started_at: at.into(),
                updated_at: at.into(),
                ..hive_store::HarnessSession::default()
            })
            .unwrap();
        }
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "GET",
            "/api/v1/harness/sessions?limit=2",
            "",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["next_before"], "2026-09-06T03:00:00Z|HS-mid");
    }

    #[test]
    fn addition_harness_cancel() {
        let _serial = crate::test_helpers::serial();
        let s = harness_store();
        s.begin_harness_session(hive_store::HarnessSession {
            id: "HS-run".into(),
            job_id: "JOB-1".into(),
            ..hive_store::HarnessSession::default()
        })
        .unwrap();
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "POST",
            "/api/v1/harness/sessions/HS-run/cancel",
            "{}",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["cancelled"], true);
        assert_ne!(result["session"]["cancel_requested_at"], "");
        let detail: String = s
            .conn()
            .unwrap()
            .query_row(
                "SELECT detail FROM events WHERE action = 'harness.cancel' AND subject = 'HS-run'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let event: Value = serde_json::from_str(&detail).unwrap();
        assert_eq!(event["session"], "HS-run");
        assert_eq!(event["job"], "JOB-1");
        let (status, result, _) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "POST",
            "/api/v1/harness/sessions/HS-run/cancel",
            "{}",
            &[],
        );
        assert_eq!(status, 200);
        assert_eq!(result["pending"], true);
        let cancel_events: i64 = s.conn().unwrap().query_row(
            "SELECT COUNT(*) FROM events WHERE action = 'harness.cancel' AND subject = 'HS-run'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(cancel_events, 1);
        s.finish_harness_session(
            "HS-run",
            "cancelled",
            "cancelled",
            "",
            "cancelled by the owner",
            0.0,
            "",
        )
        .unwrap();
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "POST",
            "/api/v1/harness/sessions/HS-run/cancel",
            "{}",
            &[],
        );
        assert_eq!(status, 409, "{raw}");
        assert_eq!(result["error"], "session is not running");
        let (status, _, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "POST",
            "/api/v1/harness/sessions/missing/cancel",
            "{}",
            &[],
        );
        assert_eq!(status, 404, "{raw}");
    }

    #[test]
    fn addition_harness_prompt() {
        let _serial = crate::test_helpers::serial();
        let s = harness_store();
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "GET",
            "/api/v1/harness/prompt",
            "",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["template"], "");
        assert_eq!(result["custom"], false);
        s.set_setting(
            "human",
            "harness.system_prompt",
            "Hello {{.Cwd}} {{.Provider}}",
        )
        .unwrap();
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "GET",
            "/api/v1/harness/prompt",
            "",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        // The preview stands in for the clone's instruction files; a
        // template without the key gets them appended.
        let rendered = result["rendered"].as_str().unwrap();
        assert!(
            rendered.starts_with("Hello <project clone> hubfake\nRepository instructions follow"),
            "{rendered}"
        );
        assert!(rendered.contains("<the clone's AGENTS.md"), "{rendered}");
        assert!(
            rendered.contains("What you remembered in earlier runs")
                && rendered.ends_with("<what earlier runs remembered in ~/.hive/memory/<project>.md and ~/.hive/memory/user.md, when there is any>"),
            "memory stands in after the instructions: {rendered}"
        );
        s.set_setting("human", "harness.instructions", "off")
            .unwrap();
        s.set_setting("human", "harness.memory", "off").unwrap();
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "GET",
            "/api/v1/harness/prompt",
            "",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["rendered"], "Hello <project clone> hubfake");
    }

    #[test]
    fn addition_harness_trajectory_download() {
        let _serial = crate::test_helpers::serial();
        let s = harness_store();
        let dir = std::path::Path::new(&s.home).join("harness");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("HS-traj.json");
        std::fs::write(&path, "{\"ok\":true}\n").unwrap();
        s.begin_harness_session(hive_store::HarnessSession {
            id: "HS-traj".into(),
            trajectory: path.display().to_string(),
            ..hive_store::HarnessSession::default()
        })
        .unwrap();
        let srv = crate::test_helpers_http::server(crate::test_helpers_http::handler(&s));
        let resp = crate::test_helpers::client()
            .get(format!(
                "{}/api/v1/harness/sessions/HS-traj/trajectory",
                srv.url
            ))
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        assert_eq!(
            resp.headers()
                .get("Content-Type")
                .unwrap()
                .to_str()
                .unwrap(),
            "application/json"
        );
        assert_eq!(
            resp.headers()
                .get("Content-Disposition")
                .unwrap()
                .to_str()
                .unwrap(),
            "attachment; filename=\"HS-traj.json\""
        );
        assert_eq!(resp.text().unwrap(), "{\"ok\":true}\n");
        let outside = tempfile::tempdir().unwrap().path().join("x.json");
        s.begin_harness_session(hive_store::HarnessSession {
            id: "HS-outside".into(),
            trajectory: outside.display().to_string(),
            ..hive_store::HarnessSession::default()
        })
        .unwrap();
        let (status, _, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "GET",
            "/api/v1/harness/sessions/HS-outside/trajectory",
            "",
            &[],
        );
        assert_eq!(status, 404, "{raw}");
    }

    #[test]
    fn session_file_reads_inside_the_workspace_only() {
        let _serial = crate::test_helpers::serial();
        let s = harness_store();
        let work = tempfile::tempdir().unwrap();
        let src = work.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("lib.rs"), "fn main() {}\n").unwrap();
        std::fs::write(
            work.path().join("logo.png"),
            [0x89u8, b'P', b'N', b'G', 0, 1, 2],
        )
        .unwrap();
        let private = work.path().join(".hive").join("private");
        std::fs::create_dir_all(&private).unwrap();
        std::fs::write(private.join("token"), "secret\n").unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("other.txt"), "elsewhere\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            outside.path().join("other.txt"),
            work.path().join("link.txt"),
        )
        .unwrap();
        s.begin_harness_session(hive_store::HarnessSession {
            id: "HS-file".into(),
            cwd: work.path().display().to_string(),
            ..hive_store::HarnessSession::default()
        })
        .unwrap();
        s.begin_harness_session(hive_store::HarnessSession {
            id: "HS-nowhere".into(),
            cwd: work.path().join("gone").display().to_string(),
            ..hive_store::HarnessSession::default()
        })
        .unwrap();
        let get = |path: &str| {
            crate::test_helpers_http::request(
                crate::test_helpers_http::handler(&s),
                "GET",
                path,
                "",
                &[],
            )
        };

        let (status, _, raw) = get("/api/v1/harness/sessions/HS-file/file?path=src/lib.rs");
        assert_eq!(status, 200, "{raw}");
        let body: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(body["text"], "fn main() {}\n");
        assert_eq!(body["path"], "src/lib.rs");
        assert_eq!(body["size"], 13);
        assert_eq!(body["truncated"], false);
        assert_eq!(body["binary"], false);

        let (status, _, raw) = get("/api/v1/harness/sessions/HS-file/file?path=logo.png");
        assert_eq!(status, 200, "{raw}");
        let body: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(body["binary"], true);
        assert_eq!(body["text"], "");

        for bad in [
            "../other.txt",
            "src/../../other.txt",
            "/etc/hosts",
            ".hive/private/token",
            ".git/config",
            "src",
            "missing.rs",
        ] {
            let (status, _, raw) = get(&format!(
                "/api/v1/harness/sessions/HS-file/file?path={}",
                bad.replace('/', "%2F")
            ));
            assert_eq!(status, 404, "{bad}: {raw}");
        }
        #[cfg(unix)]
        {
            let (status, _, raw) = get("/api/v1/harness/sessions/HS-file/file?path=link.txt");
            assert_eq!(status, 404, "symlink out of the workspace: {raw}");
        }

        let (status, _, raw) = get("/api/v1/harness/sessions/HS-file/file");
        assert_eq!(status, 400, "{raw}");
        let (status, _, raw) = get("/api/v1/harness/sessions/HS-nowhere/file?path=src/lib.rs");
        assert_eq!(status, 404, "{raw}");
        assert!(raw.contains("Workspace gone"), "{raw}");
        let (status, _, raw) = get("/api/v1/harness/sessions/HS-none/file?path=src/lib.rs");
        assert_eq!(status, 404, "{raw}");
    }

    #[test]
    fn session_after() {
        let _serial = crate::test_helpers::serial();
        let s = harness_store();
        s.begin_harness_session(hive_store::HarnessSession {
            id: "HS-after".into(),
            job_id: "JOB-after".into(),
            agent: "alice".into(),
            project: "demo".into(),
            ..hive_store::HarnessSession::default()
        })
        .unwrap();
        for (i, kind) in ["assistant", "command", "observation"]
            .into_iter()
            .enumerate()
        {
            s.append_harness_step(hive_store::HarnessStep {
                session_id: "HS-after".into(),
                n: i as i64 + 1,
                kind: kind.into(),
                content: kind.into(),
                ..hive_store::HarnessStep::default()
            })
            .unwrap();
        }
        let (status, result, raw) = crate::test_helpers_http::request(
            crate::test_helpers_http::handler(&s),
            "GET",
            "/api/v1/harness/sessions/HS-after?after=1",
            "",
            &[],
        );
        assert_eq!(status, 200, "{raw}");
        assert_eq!(result["steps"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn addition_harness_methods() {
        let _serial = crate::test_helpers::serial();
        let s = harness_store();
        for path in [
            "/api/v1/harness",
            "/api/v1/harness/models",
            "/api/v1/harness/sessions",
        ] {
            let (status, _, raw) = crate::test_helpers_http::request(
                crate::test_helpers_http::handler(&s),
                "POST",
                path,
                "{}",
                &[],
            );
            assert_eq!(status, 405, "{path}: {raw}");
        }
        for path in [
            "/api/v1/harness/login",
            "/api/v1/harness/login/poll",
            "/api/v1/harness/login/token",
            "/api/v1/harness/logout",
        ] {
            let (status, _, raw) = crate::test_helpers_http::request(
                crate::test_helpers_http::handler(&s),
                "GET",
                path,
                "",
                &[],
            );
            assert_eq!(status, 405, "{path}: {raw}");
        }
    }
}
