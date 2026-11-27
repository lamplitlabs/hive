//! Hive: daemon, planner, workers, landing, executors, leads, compute providers, experiments, recovery, hygiene sweep.
//!
//! Every module re-exports its public items here so callers write `hive_runner::foo`.

pub(crate) mod agent_specs;
pub mod amend;
pub mod auto_adopt;
pub mod awake;
pub mod aws;
pub mod azure;
pub mod backup;
pub mod baseline;
pub mod board;
pub mod brief;
pub mod buildcache;
pub mod bulk;
pub mod capacity;
pub(crate) mod card_proof;
pub mod checkout;
pub mod commands;
pub mod compute;
pub mod daemon;
pub(crate) mod daemon_chores;
pub(crate) mod daemon_load;
pub mod daemon_maintenance;
pub mod daemon_watchdog;
pub(crate) mod daily_cap;
pub mod db;
pub mod decide;
pub(crate) mod delivery;
pub mod diskroom;
pub mod env;
pub mod evolve_plan;
pub mod exec;
pub mod executor;
pub mod experiment;
pub mod failure;
pub(crate) mod forge;
pub mod gcp;
pub mod gh_auth;
pub mod gitapp;
pub(crate) mod goal_draft;
pub(crate) mod hand_watch;
/// The `hive-hands` protocol lives with the executor path (hive-hand-core,
/// ADR-070), which the open `hive-hand` binary is built from.
pub use hive_hand_core::hands;
pub mod dial;
pub mod heartbeat;
pub mod heldout;
pub mod hygiene;
pub mod idle;
pub(crate) mod idle_hold;
pub mod idle_log;
pub mod inbox;
pub mod jobs;
pub(crate) mod kept_work;
pub mod keys;
pub mod land;
pub mod land_lock;
pub mod landing_context;
pub mod landing_evidence;
pub mod landing_outcome;
pub(crate) mod landing_pr;
pub mod lead;
pub mod lead_cli;
pub mod lead_roles;
pub mod lessons;
pub mod liveness;
pub mod login_gate;
pub mod machines;
pub mod manifest;
pub mod multipass;
pub mod onboard;
pub mod owner_wait;
pub mod p256;
pub mod pair;
pub mod pair_inspect;
pub mod pipeline;
pub mod plan;
pub(crate) mod precondition;
pub mod preflight;
pub use hive_hand_core::proc;
pub mod provider;
pub mod recipe;
pub mod reclaim;
pub(crate) mod records_sync;
pub mod recover;
pub mod recovery_api;
pub mod release_age;
pub mod release_update;
pub mod research;
pub(crate) mod restart;
pub mod retention;
pub mod retire;
pub(crate) mod revert_scan;
pub(crate) mod review;
pub(crate) mod review_check;
pub(crate) mod review_cli;
pub mod run;
pub mod runners;
pub mod schedule;
pub mod schedule_script;
pub(crate) mod secrets;
pub mod security;
pub mod selfupdate;
pub mod service;
pub mod setup_repair;
pub mod sign;
pub(crate) mod signal_stop;
pub(crate) mod stage_budget;
pub mod steps_mirror;
pub mod suggestions;
pub mod sweep;
pub mod tally;
pub(crate) mod unchecked;
pub use hive_hand_core::tidy;
pub mod tmpspace;
pub mod token;
pub mod toolkit;
pub(crate) mod ui_evidence;
pub(crate) mod update_check;
pub mod util;
pub mod verify_lock;
pub mod verify_memo;
pub mod webauthn;
pub mod work_report;
pub(crate) mod yield_lane;

pub(crate) use board::*;
pub use commands::*;
pub use compute::*;
pub use daemon::*;
pub use executor::*;
pub use keys::*;
pub use recipe::*;
pub use recovery_api::*;
pub use service::*;

#[cfg(test)]
pub(crate) mod test_helpers;
#[cfg(test)]
pub(crate) mod test_helpers_cloud;
#[cfg(test)]
pub(crate) mod test_helpers_completion;
#[cfg(test)]
pub(crate) mod test_helpers_daemon;
#[cfg(test)]
pub(crate) mod test_helpers_environment;
#[cfg(test)]
pub(crate) mod test_helpers_hands;
#[cfg(test)]
pub(crate) mod test_helpers_heartbeat;
#[cfg(test)]
pub(crate) mod test_helpers_landing;
#[cfg(test)]
pub(crate) mod test_helpers_login_reuse;
#[cfg(test)]
pub(crate) mod test_helpers_provider;
#[cfg(test)]
pub(crate) mod test_helpers_recover;
#[cfg(test)]
pub(crate) mod test_helpers_selfupdate;
#[cfg(test)]
pub(crate) mod test_helpers_staged_hive;
#[cfg(test)]
pub(crate) mod test_helpers_sweep;
#[cfg(test)]
pub(crate) mod test_helpers_token;
