//! The executor path of PULSE (ADR-070): what a hand needs to take one job
//! and run it on a machine - the `pulse-hands` protocol, process groups and
//! budgets, the worktree it prepares, the environment it hands a worker, the
//! machine it reports in its probe. `pulse-runner` (the brain) drives these;
//! `pulse-hand` (the binary a customer installs) is built from them alone.
//!
//! Nothing here plans, leads, scores or briefs: no lead, plan, evolve, tally,
//! research or idle code and no prompt text may ever live in this crate.
//! `scripts/check-hand-deps.sh` holds that line on the dependency graph.

pub mod beat;
pub mod ctx;
pub mod dial;
pub mod error;
pub mod gitcred;
pub mod hands;
pub mod local;
pub mod machine;
pub mod names;
pub mod needs;
pub mod pair;
pub mod proc;
pub mod procs;
pub mod provenance;
pub mod tidy;
pub mod tmpspace;
pub mod transport;
pub mod trash;
pub mod util;
pub mod worktree;

pub use ctx::{CancellationToken, ContextError};
pub use error::{Error, errorf};

#[cfg(test)]
pub(crate) mod test_helpers;
