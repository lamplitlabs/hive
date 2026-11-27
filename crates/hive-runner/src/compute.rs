#![allow(dead_code, clippy::type_complexity)]

use crate::util::Row;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, LazyLock, Mutex, RwLock};
use std::time::Duration;
pub(crate) const COMPUTE_IDLE_MINUTES: i32 = 30;
pub(crate) const COMPUTE_PROVISION_BUDGET: Duration = Duration::from_secs(45 * 60);
pub(crate) const COMPUTE_RELEASE_BUDGET: Duration = Duration::from_secs(20 * 60);
/// How often the worker looks for machines left behind: leftovers whose release failed, and
/// machines at the provider under a request of this store that is no longer live.
pub(crate) const COMPUTE_SWEEP_MINUTES: i64 = 30;
pub(crate) const COMPUTE_AREA: &str = "runner";
pub(crate) const COMPUTE_THREAD: &str = "compute";
/// Where a provisioning's lines go: the compute worker's daemon-log line and the request's note.
pub type LogSink = Arc<dyn Fn(&str) + Send + Sync>;
#[derive(Clone, Default)]
pub struct ProvisionRequest {
    pub id: String,
    pub project: String,
    pub size: String,
    pub hours: i32,
    pub reason: String,
    pub executor: String,
    pub store: Option<hive_store::Store>,
    pub actor: String,
    pub hub_home: String,
    pub log: Option<LogSink>,
}
impl std::fmt::Debug for ProvisionRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProvisionRequest")
            .field("id", &self.id)
            .field("project", &self.project)
            .field("executor", &self.executor)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Machine {
    pub provider: String,
    pub name: String,
    pub executor: String,
    pub address: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub detail: BTreeMap<String, String>,
}
pub trait Provider: Send + Sync {
    fn name(&self) -> String;
    fn kind(&self) -> String;
    fn provision(
        &self,
        cancel: &hive_bus::CancellationToken,
        req: ProvisionRequest,
    ) -> Result<Machine>;
    fn release(&self, cancel: &hive_bus::CancellationToken, m: &Machine) -> Result<()>;
    fn probe(&self, cancel: &hive_bus::CancellationToken, m: &Machine) -> Result<()>;
    fn find(
        &self,
        cancel: &hive_bus::CancellationToken,
        s: &hive_store::Store,
        executor: &str,
        host: &str,
    ) -> Result<Option<Machine>>;
    fn stop(&self, cancel: &hive_bus::CancellationToken, m: &Machine) -> Result<()>;
    fn delete(
        &self,
        cancel: &hive_bus::CancellationToken,
        m: &Machine,
    ) -> Result<(bool, Vec<String>)>;
    fn place(&self, m: &Machine) -> String;
    fn start_hint(&self, m: &Machine) -> String;
    /// Machines at the provider carrying this hub's compute tags, for the sweep; a provider
    /// without a cheap listing reports none.
    fn hands_machines(
        &self,
        _cancel: &hive_bus::CancellationToken,
        _s: &hive_store::Store,
    ) -> Result<Vec<Machine>> {
        Ok(vec![])
    }
}

pub(crate) static PROVIDER_SET: LazyLock<RwLock<BTreeMap<String, Arc<dyn Provider>>>> =
    LazyLock::new(|| RwLock::new(BTreeMap::new()));
pub(crate) fn init() {
    *hive_tasks::LOGIN_SOURCE_CHECK.write().unwrap() = Some(crate::token::has_login_source);
    *hive_tasks::COMPUTE_PROVIDERS.write().unwrap() = Some(crate::commands::cmd_compute_providers);
    // The hands-binary probe, so `hive compute request` (hive-tasks) asks
    // the runner's resolution before any provider call and refuses early when
    // the toolchain is missing.
    let _ = hive_tasks::hands::set_hands_binary_probe(Some(Box::new(|arch: &str| {
        match crate::pair::hands_binary_plan(arch) {
            crate::pair::HandsBinaryPlan::Missing(_) => {
                Some(crate::pair::hands_binary_plan_line(arch))
            }
            _ => None,
        }
    })));
}
pub fn register_provider<P: Provider + 'static>(p: P) {
    PROVIDER_SET.write().unwrap().insert(p.name(), Arc::new(p));
}
pub(crate) fn providers() -> Vec<Arc<dyn Provider>> {
    PROVIDER_SET.read().unwrap().values().cloned().collect()
}
pub(crate) fn provider_named(name: &str) -> Option<Arc<dyn Provider>> {
    PROVIDER_SET.read().unwrap().get(name).cloned()
}
pub fn provider_names() -> Vec<String> {
    PROVIDER_SET.read().unwrap().keys().cloned().collect()
}
pub(crate) fn compute_executor_name(request_id: &str) -> String {
    crate::util::slugify(&format!("hands-{}", request_id.to_lowercase()), 40)
}
/// The linux architecture a provider size boots, read from its family name, so the hands binary
/// is checked before a machine is bought: Azure's `p` sizes (Standard_D2ps_v5, Standard_B2pts_v2),
/// AWS Graviton (t4g, m7g, c6gn, a1) and GCP t2a/c4a are arm64; multipass runs the hub's own
/// architecture; everything else is amd64.
pub(crate) fn hands_arch_for_size(provider: &str, size: &str) -> String {
    static AZURE_ARM: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^standard_[a-z]+\d+p[a-z]*_v\d+$").unwrap());
    static AWS_ARM: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^([a-z]+\d+g[a-z]*|a1)\.").unwrap());
    let size = size.trim().to_ascii_lowercase();
    let arm = match provider {
        "azure" => AZURE_ARM.is_match(&size),
        "aws" => AWS_ARM.is_match(&size),
        "gcp" => size.starts_with("t2a-") || size.starts_with("c4a-"),
        "multipass" => std::env::consts::ARCH == "aarch64",
        _ => false,
    };
    if arm { "arm64" } else { "amd64" }.into()
}
/// The checkout the daemon builds itself from: the hands binary can be cross-built there.
pub(crate) fn brain_source_root(s: &hive_store::Store) -> String {
    s.conn()
        .ok()
        .map(|conn| crate::board::brain_project(&conn).str("path"))
        .unwrap_or_default()
}
pub(crate) fn redact_secrets(text: &str) -> String {
    static WORD_RE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\S+").unwrap());
    static STAR_SECRET_RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\*{6,}").unwrap());
    let text = crate::executor::redact_tokens(text);
    let text = STAR_SECRET_RE.replace_all(&text, "[redacted]").to_string();
    WORD_RE
        .replace_all(&text, |caps: &regex::Captures<'_>| {
            let word = caps.get(0).unwrap().as_str();
            let bare = word.trim_matches(&['"', '\'', '(', ')', ',', ';', ':', '`'][..]);
            if !bare.is_empty() && hive_store::secret_shaped(bare) {
                "[redacted]".to_string()
            } else {
                word.to_string()
            }
        })
        .to_string()
}
/// Records a request's machine. `now` stamps `created_at` only the first time: the row is saved
/// again when the machine is ready and when the sweep releases it as a leftover, and its hours
/// (the day's cap, `hive_tasks::compute::hours_today`; when its hours are up, `hours_up`) count
/// from when the provider made it. Moving the stamp to the ready step dropped the spin-up, and
/// moving it to the sweep made a leftover's run zero hours.
pub(crate) fn save_machine(s: &hive_store::Store, request_id: &str, m: &Machine, now: &str) {
    let detail =
        hive_store::gojson::to_string(&json!({"tags": m.tags, "detail": m.detail})).unwrap();
    let conn = s.conn().unwrap();
    crate::util::exec1(
        &conn,
        "INSERT INTO compute_machines (request_id, provider, name, executor, address, detail, created_at) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(request_id) DO UPDATE SET provider = excluded.provider, name = excluded.name, executor = excluded.executor, address = excluded.address, detail = excluded.detail, created_at = CASE WHEN compute_machines.created_at = '' THEN excluded.created_at ELSE compute_machines.created_at END, released_at = '', attempts = 0, note = ''",
        &[
            &request_id,
            &m.provider,
            &m.name,
            &m.executor,
            &m.address,
            &detail,
            &now,
        ],
    );
}
pub(crate) fn load_machine(s: &hive_store::Store, request_id: &str) -> (Option<Machine>, Row) {
    let conn = s.conn().unwrap();
    let r = crate::util::must_one(
        &conn,
        "SELECT * FROM compute_machines WHERE request_id = ?",
        &[&request_id],
    );
    if r.0.is_empty() {
        return (None, Row::new());
    }
    #[derive(Default, Deserialize)]
    struct Detail {
        tags: BTreeMap<String, String>,
        detail: BTreeMap<String, String>,
    }
    let d: Detail = serde_json::from_str(&r.str("detail")).unwrap_or_default();
    (
        Some(Machine {
            provider: r.str("provider"),
            name: r.str("name"),
            executor: r.str("executor"),
            address: r.str("address"),
            tags: d.tags,
            detail: d.detail,
        }),
        r,
    )
}
pub(crate) fn machine_released(s: &hive_store::Store, request_id: &str, now: &str) {
    let conn = s.conn().unwrap();
    crate::util::exec1(
        &conn,
        "UPDATE compute_machines SET released_at = ? WHERE request_id = ?",
        &[&now, &request_id],
    );
}
pub(crate) fn machine_release_failed(s: &hive_store::Store, request_id: &str, note: &str) -> i32 {
    let conn = s.conn().unwrap();
    let note = crate::util::trunc(note, 500);
    crate::util::exec1(
        &conn,
        "UPDATE compute_machines SET attempts = attempts + 1, note = ? WHERE request_id = ?",
        &[&note, &request_id],
    );
    crate::util::must_one(
        &conn,
        "SELECT attempts FROM compute_machines WHERE request_id = ?",
        &[&request_id],
    )
    .int("attempts")
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ComputeOpts {
    pub(crate) actor: String,
    pub(crate) hub_home: String,
}
#[derive(Debug, Clone)]
pub(crate) struct ComputeWorker {
    pub(crate) s: hive_store::Store,
    pub(crate) o: ComputeOpts,
    pub(crate) inflight: Arc<Mutex<BTreeMap<String, bool>>>,
    pub(crate) no_source_day: Arc<Mutex<String>>,
    pub(crate) swept_at: Arc<Mutex<String>>,
    wg: Arc<(Mutex<usize>, Condvar)>,
    wake_tx: crossbeam_channel::Sender<()>,
    wake_rx: crossbeam_channel::Receiver<()>,
    lines_tx: crossbeam_channel::Sender<String>,
    lines_rx: crossbeam_channel::Receiver<String>,
}
pub(crate) fn new_compute_worker(
    cancel: &hive_bus::CancellationToken,
    s: &hive_store::Store,
    o: ComputeOpts,
) -> ComputeWorker {
    let mut o = o;
    if o.actor.is_empty() {
        o.actor = "daemon".into();
    }
    let (wake_tx, wake_rx) = crossbeam_channel::bounded(1);
    // Room for the lines a provisioning writes between two ticks (a pair report alone is
    // twenty-odd) with more than one in flight; a full channel drops lines, never blocks.
    let (lines_tx, lines_rx) = crossbeam_channel::bounded(512);
    let w = ComputeWorker {
        s: s.clone(),
        o,
        inflight: Arc::new(Mutex::new(BTreeMap::new())),
        no_source_day: Arc::new(Mutex::new(String::new())),
        swept_at: Arc::new(Mutex::new(String::new())),
        wg: Arc::new((Mutex::new(0), Condvar::new())),
        wake_tx,
        wake_rx,
        lines_tx,
        lines_rx,
    };
    let (events, cancel_sub) = hive_bus::default().subscribe(
        hive_bus::Filter {
            actions: vec!["compute.approved".into()],
            ..Default::default()
        },
        16,
    );
    let child = cancel.child();
    let nudger = w.clone();
    std::thread::Builder::new()
        .name("compute-wake".into())
        .spawn(move || {
            let done = child.done();
            loop {
                crossbeam_channel::select! {
                    recv(done) -> _ => break,
                    recv(events) -> msg => {
                        if msg.is_err() {
                            break;
                        }
                        nudger.nudge();
                    }
                }
            }
            cancel_sub();
        })
        .expect("spawn compute wake");
    w
}
impl ComputeWorker {
    pub(crate) fn wake_rx(&self) -> crossbeam_channel::Receiver<()> {
        self.wake_rx.clone()
    }
    pub(crate) fn nudge(&self) {
        let _ = self.wake_tx.try_send(());
    }
    pub(crate) fn say(&self, line: &str) {
        self.say_line(line.to_string());
    }
    pub(crate) fn say_line(&self, line: String) {
        let _ = self.lines_tx.try_send(line);
    }
    pub(crate) fn drain(&self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(line) = self.lines_rx.try_recv() {
            out.push(line);
        }
        out
    }
    pub(crate) fn claim(&self, id: &str) -> bool {
        let mut m = self.inflight.lock().unwrap();
        if m.contains_key(id) {
            return false;
        }
        m.insert(id.to_string(), true);
        true
    }
    pub(crate) fn done(&self, id: &str) {
        self.inflight.lock().unwrap().remove(id);
    }
    pub(crate) fn busy(&self) -> i32 {
        self.inflight.lock().unwrap().len() as i32
    }
    pub(crate) fn start<F: FnOnce() + Send + 'static>(&self, id: &str, fn_: F) {
        if !self.claim(id) {
            return;
        }
        {
            let (mu, _) = &*self.wg;
            *mu.lock().unwrap() += 1;
        }
        let id = id.to_string();
        let worker = self.clone();
        std::thread::Builder::new()
            .name(format!("compute-{id}"))
            .spawn(move || {
                let id_for_panic = id.clone();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(fn_));
                if let Err(panic) = result {
                    let msg = panic
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_string())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown panic".into());
                    worker.say_line(format!("compute {id_for_panic}: daemon error: {msg}"));
                }
                worker.done(&id);
                let (mu, cv) = &*worker.wg;
                let mut n = mu.lock().unwrap();
                *n = n.saturating_sub(1);
                cv.notify_all();
            })
            .expect("spawn compute worker");
    }
    pub(crate) fn wait(&self, grace: Duration) {
        self.wait_beating(grace, &|| {});
    }
    /// [`wait`](Self::wait), calling `beat` about once a minute while
    /// operations still run inside their own budgets
    /// (`COMPUTE_PROVISION_BUDGET`, `COMPUTE_RELEASE_BUDGET`): the daemon's
    /// watchdog takes the wait for a wedge only once it outlasts them.
    pub(crate) fn wait_beating(&self, grace: Duration, beat: &dyn Fn()) {
        let budget = COMPUTE_PROVISION_BUDGET.max(COMPUTE_RELEASE_BUDGET);
        let started = std::time::Instant::now();
        let until = (!grace.is_zero()).then(|| started + grace);
        let (mu, cv) = &*self.wg;
        let mut n = mu.lock().unwrap();
        while *n > 0 {
            let now = std::time::Instant::now();
            if until.is_some_and(|at| now >= at) {
                return;
            }
            let step = until
                .map(|at| at - now)
                .unwrap_or(Duration::MAX)
                .min(Duration::from_secs(60));
            n = cv.wait_timeout_while(n, step, |n| *n > 0).unwrap().0;
            if *n > 0 && started.elapsed() < budget {
                beat();
            }
        }
    }
    pub(crate) fn tick(&self, cancel: &hive_bus::CancellationToken, now: &str) -> Vec<String> {
        if cancel.is_cancelled() {
            return self.drain();
        }
        match hive_tasks::list_compute(&self.s, "", "approved") {
            Ok(approved) => {
                for req in approved {
                    let c = cancel.clone();
                    let worker = self.clone();
                    let id = req.id.clone();
                    self.start(&id, move || worker.provision(&c, req));
                }
            }
            Err(e) => self.say_line(format!("compute: {}", crate::run::err_message(&e))),
        }
        if let Ok(stale) = hive_tasks::list_compute(&self.s, "", "provisioning") {
            for req in stale {
                let worker = self.clone();
                let id = req.id.clone();
                self.start(&id, move || worker.fail_stale(req));
            }
        }
        if self.sweep_due(now) {
            self.sweep(cancel);
        }
        if self.s.setting("compute.auto_approve", "0") == "1"
            && let Ok(pending) = hive_tasks::list_compute(&self.s, "", "pending")
            && let Some(req) = pending.into_iter().next()
            && !crate::token::has_login_source(&self.s)
        {
            self.no_source_finding(req, now);
        }
        if let Ok(ready) = hive_tasks::list_compute(&self.s, "", "ready") {
            for req in ready {
                let why = self.release_due(req.clone(), now);
                if !why.is_empty() {
                    let c = cancel.clone();
                    let worker = self.clone();
                    let id = req.id.clone();
                    self.start(&id, move || worker.release(&c, req, &why));
                } else {
                    self.say_past_hours(&req, now);
                }
            }
        }
        self.drain()
    }
    pub(crate) fn finding(&self, req: hive_tasks::ComputeRequest, summary: &str, body: &str) {
        if let Err(e) = compute_finding(&self.s, &self.o.actor, req.clone(), summary, body) {
            self.say_line(format!(
                "compute {}: finding not posted: {}",
                req.id,
                crate::run::err_message(&e)
            ));
        }
    }
    pub(crate) fn fail(&self, req: hive_tasks::ComputeRequest, why: &str) {
        let why = redact_secrets(why);
        if let Err(e) =
            hive_tasks::set_compute_status(&self.s, &req.id, "failed", "", &self.o.actor)
        {
            self.say_line(format!(
                "compute {}: {}",
                req.id,
                crate::run::err_message(&e)
            ));
        }
        // the reason, where the Machines page shows it
        let _ = hive_tasks::note_compute(&self.s, &req.id, &why);
        self.finding(
            req.clone(),
            &format!("compute {} failed: {}", req.id, why),
            &format!(
                "The request by {} for one {} machine for {} hour(s) ({}) could not be served.\n\nReason: {}\n\nNothing was registered. A new request needs `hive compute request`; the owner may adjust the caps with `hive settings set compute.max_vms N` / `compute.max_hours_per_day N`.\n",
                req.requested_by, req.size, req.hours, req.reason, why
            ),
        );
        self.say_line(format!("compute {} failed: {}", req.id, why));
    }
    /// A request a rental limit refused: not rented, said as such on its row ("Not rented:
    /// today's limit of 24 hours is used (12 used, this asks 12)") and in one finding, with the
    /// owner's answers on the Machines page - Rent it anyway, Rent it and raise the limit to N,
    /// Do not rent.
    pub(crate) fn not_rented(&self, req: hive_tasks::ComputeRequest, hit: &hive_tasks::LimitHit) {
        let words = hit.words();
        if let Err(e) = hive_tasks::not_rented(&self.s, &req.id, hit, &self.o.actor) {
            self.say_line(format!(
                "compute {}: {}",
                req.id,
                crate::run::err_message(&e)
            ));
        }
        let lower = words.trim_start_matches("Not rented: ");
        self.finding(
            req.clone(),
            &format!("compute {} not rented: {lower}", req.id),
            &format!(
                "The request by {} for one {} machine for {} hour(s) ({}) was not rented: {lower}. The limits bound what agents and the daemon rent by themselves.\n\nOn the Machines page the request offers Rent it anyway, Rent it and raise the limit to {} (`{}`), or Do not rent. A machine you ask for yourself is never refused by a limit.\n",
                req.requested_by, req.size, req.hours, req.reason, hit.raise_to, hit.setting
            ),
        );
        self.say_line(format!("compute {} not rented: {lower}", req.id));
    }
    pub(crate) fn no_source_finding(&self, req: hive_tasks::ComputeRequest, now: &str) {
        let day = now.get(..10).unwrap_or(now);
        if *self.no_source_day.lock().unwrap() == day {
            return;
        }
        let conn = self.s.conn().unwrap();
        let last = crate::util::must_one(
            &conn,
            "SELECT at FROM events WHERE action = 'compute.no-login-source' ORDER BY seq DESC LIMIT 1",
            &[],
        );
        if !last.0.is_empty() && last.str("at").starts_with(day) {
            *self.no_source_day.lock().unwrap() = day.into();
            return;
        }
        *self.no_source_day.lock().unwrap() = day.into();
        crate::util::event_at(
            &conn,
            now,
            &self.o.actor,
            "compute.no-login-source",
            &req.id,
            json!({"project": req.project}),
        );
        self.finding(
            req.clone(),
            &format!("{}: compute {} waits; place a Copilot-only token with `hive executor token --with-token`", hive_tasks::NO_LOGIN_SOURCE, req.id),
            &format!(
                "Compute request {} ({}, by {}) is held: a machine Hive provisions would have no Copilot login and drain at its first launch, so none is bought (ADR-034). The hub's own `gh` login is not used because it reaches repositories. Place a fine-grained token with only the Copilot Requests permission with `hive executor token --with-token` (stdin); the daemon then logs every new machine in before it works. This note is posted once a day while the hold lasts.\n",
                req.id, req.size, req.requested_by
            ),
        );
    }
    pub(crate) fn provision(
        &self,
        cancel: &hive_bus::CancellationToken,
        req: hive_tasks::ComputeRequest,
    ) {
        let Ok(cur) = hive_tasks::get_compute(&self.s, &req.id) else {
            return;
        };
        if cur.status != "approved" {
            return;
        }
        // A limit refuses an agent's or the daemon's request in plain words, with the owner's
        // buttons on its row; the owner's own call never gets here (`caps_hit`).
        if let Ok(caps) = hive_tasks::get_caps(&self.s)
            && let Some(hit) = caps_hit(&caps, &cur)
        {
            self.not_rented(cur, &hit);
            return;
        }
        let p = match checks_before_provider(&self.s, &req, &self.s.now()) {
            Ok(p) => p,
            Err(why) if why == hive_tasks::NO_LOGIN_SOURCE => {
                self.no_source_finding(req.clone(), &self.s.now());
                self.say_line(format!("compute {} held: {}", req.id, why));
                return;
            }
            Err(why) => {
                self.fail(req, &why);
                return;
            }
        };
        if let Err(e) =
            hive_tasks::set_compute_status(&self.s, &req.id, "provisioning", "", &self.o.actor)
        {
            self.say_line(format!(
                "compute {}: {}",
                req.id,
                crate::run::err_message(&e)
            ));
            return;
        }
        self.say_line(format!(
            "compute {}: provisioning one {} machine through {}",
            req.id,
            req.size,
            p.name()
        ));
        let pctx = cancel.with_timeout(COMPUTE_PROVISION_BUDGET);
        let req_for_provider = ProvisionRequest {
            id: req.id.clone(),
            project: req.project.clone(),
            size: req.size.clone(),
            hours: req.hours,
            reason: req.reason.clone(),
            executor: compute_executor_name(&req.id),
            store: Some(self.s.clone()),
            actor: self.o.actor.clone(),
            hub_home: self.o.hub_home.clone(),
            log: Some(self.provision_log(&req.id)),
        };
        let mut m = match p.provision(&pctx, req_for_provider) {
            Ok(m) => m,
            Err(e) => {
                let mut why = crate::run::err_message(&e);
                if pctx.deadline_exceeded() {
                    why = format!("gave up after {:?}: {why}", COMPUTE_PROVISION_BUDGET);
                }
                why.push_str(&self.release_leftover(&req.id).clause());
                for line in self.forget_hands(&req.id) {
                    self.say_line(format!("compute {}: {line}", req.id));
                }
                self.fail(req, &why);
                return;
            }
        };
        if m.provider.is_empty() {
            m.provider = p.name();
        }
        let now = self.s.now();
        save_machine(&self.s, &req.id, &m, &now);
        if let Err(e) =
            hive_tasks::set_compute_status(&self.s, &req.id, "ready", &m.executor, &self.o.actor)
        {
            self.say_line(format!(
                "compute {}: {}",
                req.id,
                crate::run::err_message(&e)
            ));
            return;
        }
        // a ready machine has nothing to explain; the steps stay in the daemon log
        let _ = hive_tasks::note_compute(&self.s, &req.id, "");
        self.finding(
            req.clone(),
            &format!(
                "compute {} ready: executor {} ({} {}) for {} hour(s)",
                req.id, m.executor, m.provider, m.name, req.hours
            ),
            &format!(
                "The machine {} asked for is up: `{}` on {}, registered as executor `{}` for {} (approved by {}). The daemon releases it when its {} hour(s) are over or after {} idle minutes, whichever comes first; once the hours are over it takes no new work, and a run in flight then is finished first. `hive executor list` shows it meanwhile.\n\nReason given: {}\n",
                req.requested_by, m.name, m.provider, m.executor, hive_tasks::machine_projects_words(&req.project), req.decided_by, req.hours, COMPUTE_IDLE_MINUTES, req.reason
            ),
        );
        self.say_line(format!("compute {} ready: executor {}", req.id, m.executor));
    }
    pub(crate) fn release_due(&self, req: hive_tasks::ComputeRequest, now: &str) -> String {
        let (executor, ready_at) = rented(&self.s, &req);
        if !executor.is_empty() && crate::executor::executor_running(&self.s, &executor, now) > 0 {
            return String::new();
        }
        if ready_at.is_empty() {
            return String::new();
        }
        let kept = if executor.is_empty() {
            Vec::new()
        } else {
            crate::executor::kept_work(&self.s, &executor)
        };
        if hours_up(&req, &ready_at, now) {
            // The approved hours bound what the machine costs; what it
            // still holds goes with it, and the reason says so.
            if !kept.is_empty() {
                return format!(
                    "its {} hour(s) are up; the clone(s) of {} kept on it go with it (their commits were never fetched)",
                    req.hours,
                    kept_jobs(&kept)
                );
            }
            return format!("its {} hour(s) are up", req.hours);
        }
        // A clone whose commits could not be fetched is the only copy of its
        // run's work: an idle machine holding one stays until the daemon's
        // fetch probe brings the commits back or its hours are up (TASK-138).
        if !kept.is_empty() {
            return String::new();
        }
        // Idle is counted from when the machine came up ready (the request's last change), not
        // from when it was made: a spin-up longer than the idle minutes is not idleness.
        let mut last = if req.updated_at > ready_at {
            req.updated_at.clone()
        } else {
            ready_at
        };
        if !executor.is_empty() {
            let conn = self.s.conn().unwrap();
            let r = crate::util::must_one(
                &conn,
                "SELECT MAX(finished_at) AS at FROM runs WHERE executor = ? AND finished_at IS NOT NULL AND finished_at != ''",
                &[&executor],
            );
            if !r.0.is_empty() && r.str("at") > last {
                last = r.str("at");
            }
        }
        if crate::util::must_iso(now) - crate::util::must_iso(&last)
            >= chrono::Duration::minutes(COMPUTE_IDLE_MINUTES as i64)
        {
            // Why it sat idle, in the words its row shows (TASK-165 item 4).
            return rental_fit(&self.s, &req.project, &offered_caps(&self.s, &executor))
                .idle_release_words();
        }
        String::new()
    }
    /// A rental past its hours that a run in flight keeps up: said once, in the daemon log and
    /// on the request's card, since the machine stays and takes nothing from the queue. A
    /// rental kept up inside its hours by clones whose commits were not fetched says that.
    fn say_past_hours(&self, req: &hive_tasks::ComputeRequest, now: &str) {
        let (executor, ready_at) = rented(&self.s, req);
        if !hours_up(req, &ready_at, now) {
            let kept = if executor.is_empty() {
                Vec::new()
            } else {
                crate::executor::kept_work(&self.s, &executor)
            };
            if kept.is_empty() {
                return;
            }
            let note = format!(
                "{executor} holds the clone(s) of {} whose commits could not be fetched: it is not released while idle, and the daemon tries the fetch again every {} minutes",
                kept_jobs(&kept),
                crate::executor::KEPT_FETCH_MINUTES
            );
            if req.note != note {
                let _ = hive_tasks::note_compute(&self.s, &req.id, &note);
                self.say_line(format!("compute {}: {note}", req.id));
            }
            return;
        }
        let note = format!(
            "its {} hour(s) are up: {executor} takes no new work and is released when its last run ends",
            req.hours
        );
        if req.note != note {
            let _ = hive_tasks::note_compute(&self.s, &req.id, &note);
            self.say_line(format!("compute {}: {note}", req.id));
        }
    }
    pub(crate) fn release(
        &self,
        _cancel: &hive_bus::CancellationToken,
        req: hive_tasks::ComputeRequest,
        why: &str,
    ) {
        release_compute(&self.s, &self.o.actor, &self.o.hub_home, req, why, |line| {
            self.say_line(line.to_string())
        });
    }
    /// A request still `provisioning` when no worker holds it: the daemon that was buying the
    /// machine stopped. Whatever it got as far as making is released; a machine it never
    /// recorded is left to the sweep, which finds it by its hive-request tag.
    fn fail_stale(&self, req: hive_tasks::ComputeRequest) {
        let mut why = "the daemon stopped while this machine was being provisioned".to_string();
        match self.release_leftover(&req.id) {
            Leftover::Nothing => why.push_str(&format!(
                "; a machine tagged hive-request={} at the provider is released by the sweep the daemon runs every {} minutes",
                req.id, COMPUTE_SWEEP_MINUTES
            )),
            left => why.push_str(&left.clause()),
        }
        for line in self.forget_hands(&req.id) {
            self.say_line(format!("compute {}: {line}", req.id));
        }
        self.fail(req, &why);
    }
    /// The provider's log sink for one request: each step goes to the daemon log as
    /// `compute CR-N: ...` and, as the latest word on the request, to its note, so the Machines
    /// page shows where a provisioning stands and the daemon log says how it went.
    fn provision_log(&self, request_id: &str) -> LogSink {
        let worker = self.clone();
        let id = request_id.to_string();
        Arc::new(move |line: &str| {
            worker.say_line(format!("compute {id}: {line}"));
            let _ = hive_tasks::note_compute(&worker.s, &id, line);
        })
    }
    /// What a provisioning that did not end in a ready machine leaves on the hub: the executor
    /// row the pairing registered (every probe of it afterwards would time out against an
    /// address that is gone), and the ssh aliases and host key lines written for the machine.
    /// Both go, as `hive remove executor` and a released request do; the lines say what went.
    pub(crate) fn forget_hands(&self, request_id: &str) -> Vec<String> {
        let mut lines = Vec::new();
        let executor = compute_executor_name(request_id);
        if !crate::executor::executor_row(&self.s, &executor)
            .0
            .is_empty()
        {
            match hive_board::remove_executor(&self.s, &executor, &self.o.actor, false) {
                Ok(_) => lines.push(format!("removed executor {executor}")),
                Err(e) => lines.push(format!(
                    "executor {executor} not removed: {}",
                    crate::run::err_message(&e)
                )),
            }
        }
        let (m, _) = load_machine(&self.s, request_id);
        let m = m.unwrap_or_else(|| Machine {
            name: crate::provider::machine_name(request_id.to_string()),
            executor,
            ..Default::default()
        });
        lines.extend(forget_machine_ssh(&self.o.hub_home, &m));
        lines
    }
    /// Executor rows of machines Hive bought that are gone: a `hands-cr-N` row whose compute
    /// machine is released while no live request names it. A failed provisioning now removes its
    /// row at once; the rows older daemons left behind (every probe of them timing out, dozens a
    /// cycle) go with the first sweep, with the ssh files written for their machines.
    pub(crate) fn forget_stale_hands(&self) -> Vec<String> {
        let conn = self.s.conn().unwrap();
        let rows = crate::util::must_rows(
            &conn,
            "SELECT e.name AS name, m.request_id AS request_id FROM executors e \
             JOIN compute_machines m ON m.executor = e.name \
             LEFT JOIN compute_requests r ON r.id = m.request_id \
             WHERE e.status != 'removed' AND e.name LIKE 'hands-cr-%' AND m.released_at != '' \
             AND COALESCE(r.status, '') NOT IN ('approved', 'provisioning', 'ready') \
             ORDER BY e.name, m.created_at DESC",
            &[],
        );
        drop(conn);
        let mut lines = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for r in rows {
            let name = r.str("name");
            if !seen.insert(name.clone()) {
                continue;
            }
            let request_id = r.str("request_id");
            match hive_board::remove_executor(&self.s, &name, &self.o.actor, false) {
                Ok(_) => lines.push(format!(
                    "compute {request_id}: removed executor {name}; its machine was released"
                )),
                Err(e) => {
                    lines.push(format!(
                        "compute {request_id}: executor {name} not removed: {}",
                        crate::run::err_message(&e)
                    ));
                    continue;
                }
            }
            if let (Some(m), _) = load_machine(&self.s, &request_id) {
                lines.extend(
                    forget_machine_ssh(&self.o.hub_home, &m)
                        .into_iter()
                        .map(|l| format!("compute {request_id}: {l}")),
                );
            }
        }
        lines
    }
    /// Releases the machine a provisioning left behind: the provider remembers a machine the
    /// moment it exists (machine_created), so a failure after that point still ends with nothing
    /// running. The outcome is recorded on the machine row for the sweep to retry.
    pub(crate) fn release_leftover(&self, request_id: &str) -> Leftover {
        let (Some(m), row) = load_machine(&self.s, request_id) else {
            return Leftover::Nothing;
        };
        if !row.str("released_at").is_empty() {
            return Leftover::Released(m);
        }
        let Some(p) = provider_named(&m.provider) else {
            let why = format!("no compute provider {} is built in", m.provider);
            machine_release_failed(&self.s, request_id, &why);
            return Leftover::Failed(m, why);
        };
        let rctx = hive_bus::CancellationToken::new().with_timeout(COMPUTE_RELEASE_BUDGET);
        match p.release(&rctx, &m) {
            Ok(()) => {
                machine_released(&self.s, request_id, &self.s.now());
                Leftover::Released(m)
            }
            Err(err) => {
                let why = redact_secrets(&crate::run::err_message(&err));
                machine_release_failed(&self.s, request_id, &why);
                Leftover::Failed(m, why)
            }
        }
    }
    /// Once per COMPUTE_SWEEP_MINUTES, and only on a hub that has bought a machine at least once.
    pub(crate) fn sweep_due(&self, now: &str) -> bool {
        let mut at = self.swept_at.lock().unwrap();
        if !at.is_empty()
            && crate::util::must_iso(now) - crate::util::must_iso(&at)
                < chrono::Duration::minutes(COMPUTE_SWEEP_MINUTES)
        {
            return false;
        }
        let conn = self.s.conn().unwrap();
        let tried = crate::util::must_one(
            &conn,
            "SELECT COUNT(*) AS n FROM compute_requests WHERE status IN ('provisioning', 'ready', 'failed', 'released')",
            &[],
        )
        .int("n");
        if tried == 0 {
            return false;
        }
        *at = now.to_string();
        true
    }
    /// What the daemon does for machines nobody is using: a leftover whose release failed is
    /// released again, and a machine the provider lists under a request of this store that is
    /// over (the daemon was stopped mid-create, or an older build forgot it) is released once and
    /// reported. Machines under another store's request, or in the retire list, are left alone.
    pub(crate) fn sweep(&self, cancel: &hive_bus::CancellationToken) {
        let now = self.s.now();
        for line in self.forget_stale_hands() {
            self.say_line(line);
        }
        let conn = self.s.conn().unwrap();
        let rows = crate::util::must_rows(
            &conn,
            "SELECT request_id FROM compute_machines WHERE released_at = '' ORDER BY request_id",
            &[],
        );
        drop(conn);
        let mut seen = std::collections::BTreeSet::new();
        for r in rows {
            let id = r.str("request_id");
            let Ok(req) = hive_tasks::get_compute(&self.s, &id) else {
                continue;
            };
            // ready ones are release_due's, provisioning ones are inflight or stale
            if req.status != "failed" {
                continue;
            }
            seen.insert(id.clone());
            let worker = self.clone();
            self.start(&id, move || worker.release_leftover_again(req));
        }
        for p in providers() {
            let found = match p.hands_machines(cancel, &self.s) {
                Ok(found) => found,
                Err(e) => {
                    self.say_line(format!(
                        "compute: {} sweep: {}",
                        p.name(),
                        redact_secrets(&crate::run::err_message(&e))
                    ));
                    continue;
                }
            };
            for m in found {
                let Some(id) = m
                    .tags
                    .get(crate::provider::TAG_REQUEST)
                    .cloned()
                    .filter(|id| !id.is_empty())
                else {
                    continue;
                };
                if seen.contains(&id) {
                    continue;
                }
                let Ok(req) = hive_tasks::get_compute(&self.s, &id) else {
                    continue;
                };
                if matches!(req.status.as_str(), "approved" | "provisioning" | "ready")
                    || self.retired(&m.name)
                {
                    continue;
                }
                // a provider's list lags behind its delete; one released this sweep interval
                // is not an orphan yet
                let released_at = load_machine(&self.s, &id).1.str("released_at");
                if !released_at.is_empty()
                    && crate::util::must_iso(&now) - crate::util::must_iso(&released_at)
                        < chrono::Duration::minutes(COMPUTE_SWEEP_MINUTES)
                {
                    continue;
                }
                seen.insert(id.clone());
                let worker = self.clone();
                let p = p.clone();
                self.start(&id, move || worker.release_orphan(p, req, m));
            }
        }
    }
    fn retired(&self, name: &str) -> bool {
        let conn = self.s.conn().unwrap();
        crate::util::must_one(
            &conn,
            "SELECT COUNT(*) AS n FROM retired_vms WHERE name = ?",
            &[&name],
        )
        .int("n")
            > 0
    }
    fn release_leftover_again(&self, req: hive_tasks::ComputeRequest) {
        match self.release_leftover(&req.id) {
            Leftover::Released(m) => self.leftover_released(req, m, "its request failed"),
            Leftover::Failed(m, why) => self.say_line(format!(
                "compute {}: {} still not released: {}",
                req.id, m.name, why
            )),
            Leftover::Nothing => {}
        }
    }
    fn release_orphan(&self, p: Arc<dyn Provider>, req: hive_tasks::ComputeRequest, m: Machine) {
        let now = self.s.now();
        // remembered first, so a failed release is retried as a leftover and counted; a row
        // already there keeps the time its machine was made, so the hours it ran still count
        save_machine(&self.s, &req.id, &m, &now);
        let rctx = hive_bus::CancellationToken::new().with_timeout(COMPUTE_RELEASE_BUDGET);
        match p.release(&rctx, &m) {
            Ok(()) => {
                machine_released(&self.s, &req.id, &now);
                self.leftover_released(
                    req.clone(),
                    m,
                    &format!(
                        "the provider still listed it while the request was {}",
                        req.status
                    ),
                );
            }
            Err(err) => {
                let why = redact_secrets(&crate::run::err_message(&err));
                let attempts = machine_release_failed(&self.s, &req.id, &why);
                self.say_line(format!(
                    "compute {}: {} found at {} and not released (attempt {}): {}",
                    req.id, m.name, m.provider, attempts, why
                ));
                if attempts == 1 {
                    self.finding(
                        req.clone(),
                        &format!(
                            "compute {}: {} is still running at {} and could not be released",
                            req.id, m.name, m.provider
                        ),
                        &format!(
                            "`{}` on {} carries the tag hive-request={} but the request is {}, so it should not exist; the provider refused to release it: {}\n\nThe daemon tries again every {} minutes; a human may delete it by hand.\n",
                            m.name, m.provider, req.id, req.status, why, COMPUTE_SWEEP_MINUTES
                        ),
                    );
                }
            }
        }
    }
    fn leftover_released(&self, req: hive_tasks::ComputeRequest, m: Machine, because: &str) {
        let now = self.s.now();
        let conn = self.s.conn().unwrap();
        crate::util::event_at(
            &conn,
            &now,
            &self.o.actor,
            "compute.leftover-released",
            &req.id,
            json!({"project": req.project, "provider": m.provider, "name": m.name, "because": because}),
        );
        drop(conn);
        self.finding(
            req.clone(),
            &format!(
                "compute {}: released {} ({}) that was left running; {}",
                req.id, m.name, m.provider, because
            ),
            &format!(
                "`{}` on {} was still there after request {} ({}) ended: {}. The daemon released it; nothing of it costs money now. A failed provisioning releases what it created at once, and this sweep runs every {} minutes for anything that slipped through.\n",
                m.name, m.provider, req.id, req.status, because, COMPUTE_SWEEP_MINUTES
            ),
        );
        self.say_line(format!(
            "compute {}: released {} ({}) left running; {}",
            req.id, m.name, m.provider, because
        ));
    }
}
/// What release_leftover found for a request: no machine row, a machine now (or already)
/// released, or one the provider refused to release with the reason.
pub(crate) enum Leftover {
    Nothing,
    Released(Machine),
    Failed(Machine, String),
}
impl Leftover {
    /// The clause appended to a failure note.
    pub(crate) fn clause(&self) -> String {
        match self {
            Leftover::Nothing => String::new(),
            Leftover::Released(_) => "; what was created is released".into(),
            Leftover::Failed(_, why) => format!(
                "; releasing what was created failed too: {why}; the daemon tries again every {COMPUTE_SWEEP_MINUTES} minutes"
            ),
        }
    }
}
/// The executor a ready request's machine runs as and when it was made (its hours count from
/// then): the machine row's word, else the request's own.
fn rented(s: &hive_store::Store, req: &hive_tasks::ComputeRequest) -> (String, String) {
    let (m, mrow) = load_machine(s, &req.id);
    let mut ready_at = req.updated_at.clone();
    if !mrow.0.is_empty() && !mrow.str("created_at").is_empty() {
        ready_at = mrow.str("created_at");
    }
    let mut executor = req.executor.clone();
    if let Some(m) = &m
        && !m.executor.is_empty()
    {
        executor = m.executor.clone();
    }
    (executor, ready_at)
}
/// The task a job was planned for: `TASK-7` from a `task:TASK-7` or
/// `task:TASK-7:label` source; empty for any other job.
pub(crate) fn task_of_source(source: &str) -> &str {
    source
        .strip_prefix("task:")
        .and_then(|rest| rest.split(':').next())
        .unwrap_or("")
}

/// Says on the task of each job in `kept` that its work went with the
/// released machine `executor`: the commits were never fetched, so the
/// branch and patch they would have been kept as do not exist and the work
/// has to be done again. Before, the loss was named only on the compute card
/// and in a finding, which whoever reads the task never saw (review of PR 283
/// item 4). A cancelled job's work is wanted by nobody and is not said.
/// Returns the daemon's lines.
pub(crate) fn say_lost_work(
    s: &hive_store::Store,
    actor: &str,
    executor: &str,
    kept: &[crate::executor::KeptClone],
    why: &str,
) -> Vec<String> {
    let mut lines = Vec::new();
    let Ok(conn) = s.conn() else {
        return lines;
    };
    for k in kept {
        if k.job_status == "cancelled" {
            continue;
        }
        let job = crate::util::must_one(
            &conn,
            "SELECT j.project, j.source, j.title, COALESCE(r.agent, j.lease_agent, '') AS agent FROM jobs j LEFT JOIN runs r ON r.id = ? WHERE j.id = ?",
            &[&k.run, &k.job],
        );
        let source = job.str("source");
        let task = task_of_source(&source);
        if task.is_empty() {
            continue;
        }
        let branch = crate::land::failed_work_branch(&job.str("agent"), &k.job);
        let patch = std::path::Path::new(&s.home)
            .join("patches")
            .join(job.str("project"))
            .join(format!("{}.patch", branch.replace('/', "__")));
        let body = format!(
            "The work of job {job_id} ({title}) is lost. Its run {run} committed on the rented machine {executor}, but neither the fetch nor the bundle brought the commits back, so its clone was kept there (since {at}). {executor} is released ({why}) and the clone went with it, so the branch {branch} and the patch {patch} were never made. Nothing of that run's work can be fetched any more; it has to be done again.",
            job_id = k.job,
            title = job.str("title"),
            run = k.run,
            at = k.at,
            patch = patch.display(),
        );
        match hive_tasks::send_message(s, &job.str("project"), task, actor, &body) {
            Ok(_) => lines.push(format!("the loss of {}'s work is said on {task}", k.job)),
            Err(e) => lines.push(format!(
                "the loss of {}'s work is not said on {task}: {}",
                k.job,
                crate::run::err_message(&e)
            )),
        }
    }
    lines
}

/// The jobs of kept clones, for a line: `job-a, job-b`.
fn kept_jobs(kept: &[crate::executor::KeptClone]) -> String {
    kept.iter()
        .map(|k| k.job.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}
/// Whether the hours a ready request asked for are over, counted from when its machine was made
/// (`save_machine` keeps that stamp), as the day's cap counts them.
fn hours_up(req: &hive_tasks::ComputeRequest, ready_at: &str, now: &str) -> bool {
    !ready_at.is_empty()
        && req.hours > 0
        && crate::util::must_iso(now) - crate::util::must_iso(ready_at)
            >= chrono::Duration::hours(req.hours as i64)
}
/// The rented machines whose hours are up. They get no new work (`run::executor_slots`), so the
/// runs in flight on them are their last and `release_due` lets them go when those end.
/// Measured on one hub before this: a machine approved for 4 hours was released after 21.9 and
/// another after 12.1; 17 of the first one's 22 runs began after its hours were up, because the
/// queue leased to it again each time a run ended and a machine with a run in flight is not
/// released.
pub(crate) fn rentals_past_hours(
    s: &hive_store::Store,
    now: &str,
) -> std::collections::BTreeSet<String> {
    hive_tasks::list_compute(s, "", "ready")
        .unwrap_or_default()
        .iter()
        .filter_map(|req| {
            let (executor, ready_at) = rented(s, req);
            (!executor.is_empty() && hours_up(req, &ready_at, now)).then_some(executor)
        })
        .collect()
}
pub(crate) fn compute_finding(
    s: &hive_store::Store,
    actor: &str,
    req: hive_tasks::ComputeRequest,
    summary: &str,
    body: &str,
) -> Result<()> {
    crate::board::create_post(
        s,
        s.now(),
        crate::board::PostOpts {
            kind: "finding".into(),
            project: req.project,
            area: COMPUTE_AREA.into(),
            thread: COMPUTE_THREAD.into(),
            summary: crate::util::trunc(summary, 230),
            body: body.into(),
            agent: crate::board::actor_agent(s, actor.into()),
            on: req.proposal_post,
            ..Default::default()
        },
    )?;
    Ok(())
}
pub(crate) fn caps_allow(
    caps: hive_tasks::Caps,
    req: hive_tasks::ComputeRequest,
    _now: &str,
) -> String {
    if let Some(hit) = caps_hit(&caps, &req) {
        return hit.words();
    }
    if !caps.login_source {
        return hive_tasks::NO_LOGIN_SOURCE.into();
    }
    String::new()
}
/// The rental limit a request passes when the worker is about to provision it (TASK-165): none
/// for the owner's own call - asked or approved by the owner - which no limit refuses; for an
/// agent's or the daemon's, `compute.max_vms` and `compute.max_hours_per_day` with the request
/// already counted in both (`hive_tasks::compute::hours_today` holds every open request's
/// unrun hours, this one's included).
pub(crate) fn caps_hit(
    caps: &hive_tasks::Caps,
    req: &hive_tasks::ComputeRequest,
) -> Option<hive_tasks::LimitHit> {
    if hive_tasks::owners_call(req) {
        return None;
    }
    caps.limit_hit_counted(req.hours)
}
/// What the jobs waiting now would make of one machine (TASK-165 items 3 and 4): how many it
/// could run, how many wait in all, and per project the words a waiting job needs that the
/// machine lacks (`brain` -> `macos`). Counts queued improve and task jobs of active projects
/// that are not held on this machine, as the starved queue's rental does (`rentable_jobs`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RentalFit {
    pub fits: usize,
    pub waiting: usize,
    pub blocked: Vec<BlockedProject>,
}

/// A project whose waiting jobs a machine cannot run, and what they need that it lacks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BlockedProject {
    pub project: String,
    pub jobs: usize,
    pub missing: Vec<String>,
}

impl RentalFit {
    /// "brain needs macos; ios needs macos, xcode": what keeps the waiting jobs off the machine.
    pub fn needs_words(&self) -> String {
        self.blocked
            .iter()
            .map(|b| format!("{} needs {}", b.project, b.missing.join(", ")))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// The line the owner reads before renting: "2 waiting job(s) can run on it", or "No
    /// waiting job can run on this machine: brain needs macos", or "No job is waiting now".
    pub fn words(&self) -> String {
        if self.fits > 0 {
            return format!("{} waiting job(s) can run on it", self.fits);
        }
        if self.blocked.is_empty() {
            return "No job is waiting now".into();
        }
        format!(
            "No waiting job can run on this machine: {}",
            self.needs_words()
        )
    }

    /// Why a rented machine sat idle, for its release (item 4): "Released after 30 idle
    /// minutes: no waiting job could run there (brain needs macos)".
    pub fn idle_release_words(&self) -> String {
        let lead = format!("Released after {COMPUTE_IDLE_MINUTES} idle minutes");
        if self.fits > 0 {
            return format!(
                "{lead}: {} waiting job(s) could run there but none started",
                self.fits
            );
        }
        if self.blocked.is_empty() {
            return format!("{lead}: no job was waiting");
        }
        format!(
            "{lead}: no waiting job could run there ({})",
            self.needs_words()
        )
    }
}

/// [`RentalFit`] for a machine offering `caps` that serves `project` (every project for
/// `hive_tasks::ANY_PROJECT` or an empty one).
pub(crate) fn rental_fit(
    s: &hive_store::Store,
    project: &str,
    caps: &std::collections::BTreeSet<String>,
) -> RentalFit {
    let Ok(conn) = s.conn() else {
        return RentalFit::default();
    };
    let any = project.is_empty() || project == hive_tasks::ANY_PROJECT;
    let rows = crate::util::must_rows(
        &conn,
        &format!(
            "SELECT j.project, j.needs FROM jobs j JOIN projects p ON p.slug = j.project WHERE j.status = 'queued' AND p.status = 'active' AND j.kind IN ({}) AND (? OR j.project = ?) AND NOT EXISTS (SELECT 1 FROM harness_sessions h JOIN runs r ON r.id = h.run_id WHERE h.job_id = j.id AND h.status = 'held' AND r.executor = '{}') ORDER BY j.created_at",
            crate::jobs::work_kinds_sql(),
            crate::executor::LOCAL_EXECUTOR
        ),
        &[&any, &project],
    );
    let mut fit = RentalFit {
        waiting: rows.len(),
        ..Default::default()
    };
    for r in rows {
        let missing = hive_store::needs::missing(&r.str("needs"), caps);
        if missing.is_empty() {
            fit.fits += 1;
            continue;
        }
        let project = r.str("project");
        let i = match fit.blocked.iter().position(|b| b.project == project) {
            Some(i) => i,
            None => {
                fit.blocked.push(BlockedProject {
                    project,
                    ..Default::default()
                });
                fit.blocked.len() - 1
            }
        };
        let b = &mut fit.blocked[i];
        b.jobs += 1;
        for w in missing {
            if !b.missing.contains(&w) {
                b.missing.push(w);
            }
        }
    }
    fit
}

/// What a machine `executor` offers, from its row; the class the daemon rents when the row is
/// gone or says nothing.
pub(crate) fn offered_caps(
    s: &hive_store::Store,
    executor: &str,
) -> std::collections::BTreeSet<String> {
    let row = if executor.is_empty() {
        Row::default()
    } else {
        crate::executor::executor_row(s, executor)
    };
    if row.0.is_empty() {
        return crate::daemon::rented_caps(s);
    }
    crate::executor::caps(&row)
}

/// The owner's rental preview (TASK-165 items 1 and 3), before a machine is filed from the
/// console: what the waiting jobs make of a rented machine for `project`, and the limit the
/// request would pass if an agent asked for it - which the owner's own request does not obey,
/// but is told about with the choice to raise it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RentalPreview {
    pub fit: RentalFit,
    pub fit_words: String,
    pub limit: Option<hive_tasks::LimitHit>,
    pub limit_words: String,
    /// The OS words the project's own needs carry (`macos`) that a rented Linux machine lacks:
    /// what "Let project jobs run on Linux" would drop.
    pub os_needs: Vec<String>,
    pub linux_risk: String,
}

pub fn rental_preview(s: &hive_store::Store, project: &str, hours: i32) -> RentalPreview {
    let caps = crate::daemon::rented_caps(s);
    let fit = rental_fit(s, project, &caps);
    let limit = hive_tasks::get_caps(s)
        .ok()
        .and_then(|c| c.limit_hit(hours));
    let needs = if project.is_empty() || project == hive_tasks::ANY_PROJECT {
        String::new()
    } else {
        s.conn()
            .ok()
            .and_then(|c| {
                c.query_row(
                    "SELECT needs FROM projects WHERE slug = ?",
                    [project],
                    |r| r.get::<_, String>(0),
                )
                .ok()
            })
            .unwrap_or_default()
    };
    let os_needs: Vec<String> = hive_store::needs::stored(&needs)
        .into_iter()
        .filter(|w| hive_store::needs::OS_WORDS.contains(&w.as_str()) && !caps.contains(w))
        .collect();
    let linux_risk = if os_needs.is_empty() {
        String::new()
    } else {
        format!(
            "{project}'s jobs then run on Linux machines too: work that only builds or passes its check on {} fails there and lands nothing, and a project file that says `needs: {}` sets it back when it changes",
            os_needs.join(", "),
            os_needs.join(", ")
        )
    };
    RentalPreview {
        fit_words: fit.words(),
        fit,
        limit_words: limit.as_ref().map(|h| h.words()).unwrap_or_default(),
        limit,
        os_needs,
        linux_risk,
    }
}

/// "Let project jobs run on Linux": drops the OS words a rented Linux machine lacks from the
/// project's needs (its queued jobs follow, `hive_board::set_project_needs`), as the owner.
/// Returns the needs left.
pub fn let_project_run_on_linux(
    s: &hive_store::Store,
    project: &str,
    actor: &str,
) -> Result<String> {
    let preview = rental_preview(s, project, 1);
    if preview.os_needs.is_empty() {
        return Ok(String::new());
    }
    let conn = s.conn()?;
    let needs: String = conn.query_row(
        "SELECT needs FROM projects WHERE slug = ?",
        [project],
        |r| r.get(0),
    )?;
    let kept: Vec<String> = hive_store::needs::stored(&needs)
        .into_iter()
        .filter(|w| !preview.os_needs.contains(w))
        .collect();
    let set = if kept.is_empty() {
        "-".to_string()
    } else {
        kept.join(",")
    };
    hive_board::set_project_needs(s, project, &set, actor)
}

/// What the worker checks before it asks the provider for anything, in its order: the caps, that
/// the provider is built in, and that the binary the machine would be paired with can be
/// resolved. `Err` is the reason the request fails with, or `NO_LOGIN_SOURCE`, which holds it
/// instead.
pub(crate) fn checks_before_provider(
    s: &hive_store::Store,
    req: &hive_tasks::ComputeRequest,
    now: &str,
) -> std::result::Result<Arc<dyn Provider>, String> {
    let caps = hive_tasks::get_caps(s).map_err(|e| crate::run::err_message(&e))?;
    let why = caps_allow(caps.clone(), req.clone(), now);
    if !why.is_empty() {
        return Err(why);
    }
    let Some(p) = provider_named(&caps.provider) else {
        return Err(format!(
            "no compute provider {} is built in (have: {})",
            crate::board::py_repr(caps.provider),
            provider_names().join(", ")
        ));
    };
    // The binary the machine would be paired with must be resolvable before
    // anything is bought: CR-001/CR-002 paid for a VM that was then failed
    // for want of a linux/amd64 hive. The probe (env var, sibling file,
    // cross-build dry check) never builds; the real resolution runs again
    // in pair_machine once the machine exists.
    let size = if req.size.trim().is_empty() {
        s.setting(&hive_store::compute_size_setting(&p.name()), "")
    } else {
        req.size.clone()
    };
    let arch = hands_arch_for_size(&p.name(), &size);
    crate::pair::set_hands_source_root(brain_source_root(s));
    // A test may replace the resolver itself; that override is authoritative.
    let blocked = if crate::pair::hands_binary_overridden() {
        (crate::pair::HANDS_BINARY
            .read()
            .expect("hands_binary poisoned"))(&arch)
        .err()
        .map(|e| crate::run::err_message(&e))
    } else {
        match crate::pair::hands_binary_plan(&arch) {
            crate::pair::HandsBinaryPlan::Missing(_) => {
                Some(crate::pair::hands_binary_plan_line(&arch))
            }
            _ => None,
        }
    };
    if let Some(reason) = blocked {
        return Err(format!("{reason}; no machine was bought"));
    }
    Ok(p)
}
/// Why the worker would fail a request the daemon filed at `now` in those checks, which need no
/// request to be asked: no hands binary for the provider's size, a cap that allows nothing, a
/// provider that is not built in. `None` when the worker would go on to the provider, or would
/// hold the request for a login source rather than fail it.
pub(crate) fn would_fail_before_provider(
    s: &hive_store::Store,
    hours: i32,
    now: &str,
) -> Option<String> {
    let next = hive_tasks::ComputeRequest {
        hours,
        at: now.to_string(),
        ..Default::default()
    };
    checks_before_provider(s, &next, now)
        .err()
        .filter(|why| why != hive_tasks::NO_LOGIN_SOURCE)
        .map(|why| redact_secrets(&why))
}
/// The local ssh files a provisioning writes for one machine, removed: the `Host hands-cr-N`
/// block the pairing makes for the hive user, the `Host hive-cr-N-admin` block for the admin
/// account, and the machine's host key lines in known_hosts (a provider hands the same address
/// to a later machine, whose key differs; a line kept from the old one would refuse the new one
/// as a changed host). An empty `hub_home` is the daemon's own home. The lines say what went.
pub(crate) fn forget_machine_ssh(hub_home: &str, m: &Machine) -> Vec<String> {
    let hub_home = if hub_home.is_empty() {
        crate::util::home_dir()
    } else {
        hub_home.to_string()
    };
    let mut lines = Vec::new();
    let mut aliases = Vec::new();
    if !m.executor.is_empty() {
        aliases.push(m.executor.clone());
    }
    if !m.name.is_empty() {
        aliases.push(format!("{}-admin", m.name));
    }
    for alias in aliases {
        if crate::pair::pair_remove_alias(&hub_home, &alias) {
            lines.push(format!("removed the {alias} block from ~/.ssh/config"));
        }
    }
    let ip = m
        .detail
        .get("public_ip")
        .cloned()
        .filter(|ip| !ip.is_empty())
        .or_else(|| {
            m.address
                .rsplit('@')
                .next()
                .map(|h| h.split(':').next().unwrap_or("").to_string())
        })
        .unwrap_or_default();
    let n = crate::pair::pair_remove_known_hosts(&hub_home, &ip, 22);
    if n > 0 {
        lines.push(format!("removed {n} known_hosts line(s) for {ip}"));
    }
    lines
}
/// The event that says when the release of a request's machine began, written once per request
/// (before its first attempt). `released_at` is stamped after the provider's delete returns, up
/// to `COMPUTE_RELEASE_BUDGET` later; whether a machine ran past its hours is measured to this
/// (`daemon::ran_past_hours`), so a slow teardown is not an overrun.
pub(crate) const RELEASE_BEGAN_ACTION: &str = "compute.releasing";
fn release_began(
    s: &hive_store::Store,
    actor: &str,
    req: &hive_tasks::ComputeRequest,
    m: &Machine,
) {
    let conn = s.conn().unwrap();
    let seen = crate::util::must_one(
        &conn,
        "SELECT COUNT(*) AS n FROM events WHERE action = ? AND subject = ?",
        &[&RELEASE_BEGAN_ACTION, &req.id],
    )
    .int("n");
    if seen > 0 {
        return;
    }
    crate::util::event_at(
        &conn,
        &s.now(),
        actor,
        RELEASE_BEGAN_ACTION,
        &req.id,
        json!({"project": req.project, "provider": m.provider, "name": m.name}),
    );
}
pub(crate) fn release_compute(
    s: &hive_store::Store,
    actor: &str,
    hub_home: &str,
    req: hive_tasks::ComputeRequest,
    why: &str,
    say: impl Fn(&str),
) -> bool {
    let (loaded, _) = load_machine(s, &req.id);
    let m = loaded.unwrap_or_else(|| Machine {
        provider: s.setting("compute.provider", ""),
        executor: req.executor.clone(),
        ..Default::default()
    });
    if !m.name.is_empty()
        && let Some(p) = provider_named(&m.provider)
    {
        release_began(s, actor, &req, &m);
        let rctx = hive_bus::CancellationToken::new().with_timeout(COMPUTE_RELEASE_BUDGET);
        if let Err(e) = p.release(&rctx, &m) {
            let msg = crate::run::err_message(&e);
            let attempts = machine_release_failed(s, &req.id, &msg);
            say(&format!(
                "compute {}: release attempt {} failed: {}",
                req.id,
                attempts,
                redact_secrets(&msg)
            ));
            if attempts == 1 {
                let _ = compute_finding(
                    s,
                    actor,
                    req.clone(),
                    &format!(
                        "compute {}: releasing {} failed ({}); the daemon retries",
                        req.id, m.name, why
                    ),
                    &format!(
                        "`{}` on {} should be released because {}, but the provider refused: {}\n\nThe daemon retries every loop; a human may delete it by hand (its tag is hive-request={}).\n",
                        m.name,
                        m.provider,
                        why,
                        redact_secrets(&msg),
                        req.id
                    ),
                );
            }
            return false;
        }
    }
    // What the machine still holds goes with it: read before its executor
    // is removed, said on the jobs' tasks once the release is written down.
    let kept = if m.executor.is_empty() {
        Vec::new()
    } else {
        crate::executor::kept_work(s, &m.executor)
    };
    if !m.executor.is_empty()
        && !crate::executor::executor_row(s, &m.executor).0.is_empty()
        && let Err(e) = hive_board::remove_executor(s, &m.executor, actor, false)
    {
        say(&format!(
            "compute {}: executor {} not removed: {}",
            req.id,
            m.executor,
            crate::run::err_message(&e)
        ));
    }
    for line in forget_machine_ssh(hub_home, &m) {
        say(&format!("compute {}: {line}", req.id));
    }
    let now = s.now();
    machine_released(s, &req.id, &now);
    if let Err(e) = hive_tasks::set_compute_status(s, &req.id, "released", "", actor) {
        say(&format!(
            "compute {}: {}",
            req.id,
            crate::run::err_message(&e)
        ));
        return false;
    }
    let _ = hive_tasks::note_compute(s, &req.id, why);
    for line in say_lost_work(s, actor, &m.executor, &kept, why) {
        say(&format!("compute {}: {line}", req.id));
    }
    if let Err(e) = compute_finding(
        s,
        actor,
        req.clone(),
        &format!("compute {} released: {}", req.id, why),
        &format!(
            "The machine `{}` ({}) that served {}'s request is gone because {}; executor `{}` is removed. Runs it made stay in the log.\n",
            m.name, m.provider, req.requested_by, why, m.executor
        ),
    ) {
        say(&format!(
            "compute {}: finding not posted: {}",
            req.id,
            crate::run::err_message(&e)
        ));
    }
    say(&format!("compute {} released: {}", req.id, why));
    true
}
