//! Hive: owner tasks, messages, compute requests and caps.

pub mod activity;
pub mod commands;
pub mod compute;
pub mod delivery;
pub mod hands;
pub mod leads;
pub mod lifecycle;
pub mod manifest;
pub mod messages;
pub mod owner_wait;
pub mod reports;
pub mod schedules;
pub mod suggestions;
pub mod tasks;
pub mod work;

pub use activity::*;
pub use commands::*;
pub use compute::*;
pub use delivery::*;
pub use lifecycle::{
    LifecycleJob, Phase, PhaseStep, StepState, TaskLifecycle, WaitsOn, by_phase, derive_lifecycle,
    lifecycle_for, lifecycles_for, owner_turn,
};
pub use messages::*;
pub use owner_wait::*;
pub use reports::*;
pub use tasks::*;
pub use work::*;

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) mod test_helpers {
    pub(crate) fn serial() -> impl Drop {
        hive_testutil::env_lock()
    }
}
