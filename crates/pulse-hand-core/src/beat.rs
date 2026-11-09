//! The heartbeat clock: how often a running job says it is alive, and the
//! ticker that says when. Tests swap the ticker for one they drive.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, RwLock};

/// How often a running job beats; the board's `HEARTBEAT_INTERVAL`.
pub const HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

pub type HeartbeatTicksFn = dyn Fn(
        std::time::Duration,
    ) -> (
        crossbeam_channel::Receiver<chrono::DateTime<chrono::Utc>>,
        Box<dyn Fn() + Send + Sync>,
    ) + Send
    + Sync;
pub static HEARTBEAT_TICKS: LazyLock<RwLock<Box<HeartbeatTicksFn>>> = LazyLock::new(|| {
    RwLock::new(Box::new(|period| {
        // time.NewTicker: a one-slot channel that drops ticks nobody is reading; Stop ends it.
        let (tx, rx) = crossbeam_channel::bounded(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stopped);
        std::thread::spawn(move || {
            let mut next = std::time::Instant::now() + period;
            loop {
                let now = std::time::Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                }
                if thread_stop.load(Ordering::SeqCst) {
                    break;
                }
                if let Err(crossbeam_channel::TrySendError::Disconnected(_)) =
                    tx.try_send(chrono::Utc::now())
                {
                    break;
                }
                next += period;
            }
        });
        (
            rx,
            Box::new(move || stopped.store(true, Ordering::SeqCst)) as Box<dyn Fn() + Send + Sync>,
        )
    }))
});
pub fn set_heartbeat_ticks(f: Box<HeartbeatTicksFn>) {
    *HEARTBEAT_TICKS.write().expect("heartbeat ticks lock") = f;
}

/// An agent name the board accepts: lowercase, 2-40 characters, starting
/// with a letter (the board's `valid_name`).
pub fn valid_agent_name(name: &str) -> bool {
    static RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^[a-z][a-z0-9-]{1,39}$").unwrap());
    RE.is_match(name)
}

/// Who a running job is: the agent, its run (= session), the job and its
/// lease generation, and the executor it runs on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionIdentity {
    pub agent: String,
    pub session: String,
    pub run_id: String,
    pub job_id: String,
    pub generation: i64,
    pub executor: String,
}
impl ExecutionIdentity {
    pub fn validate(&self) -> anyhow::Result<()> {
        if !valid_agent_name(&self.agent)
            || self.session.is_empty()
            || self.session != self.run_id
            || self.job_id.is_empty()
            || self.executor.is_empty()
            || self.generation < 1
            || self.run_id.len() > 160
            || self.job_id.len() > 160
            || self.executor.len() > 160
        {
            return Err(anyhow::anyhow!(
                "heartbeat needs a valid agent, run/session, job, generation and executor"
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionHeartbeat {
    #[serde(flatten)]
    pub identity: ExecutionIdentity,
    #[serde(default)]
    pub at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stopped_at: String,
}
impl std::ops::Deref for ExecutionHeartbeat {
    type Target = ExecutionIdentity;
    fn deref(&self) -> &Self::Target {
        &self.identity
    }
}
impl ExecutionHeartbeat {
    pub fn validate(&self) -> anyhow::Result<()> {
        self.identity.validate()?;
        let at = chrono::DateTime::parse_from_rfc3339(&self.at)
            .map_err(|_| anyhow::anyhow!("heartbeat has no valid observation time"))?;
        if self.stopped_at.is_empty() {
            return Ok(());
        }
        let stop = chrono::DateTime::parse_from_rfc3339(&self.stopped_at)?;
        if stop < at {
            return Err(anyhow::anyhow!("heartbeat has no valid stop time"));
        }
        Ok(())
    }
}
