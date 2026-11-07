//! SQLite store foundation for PULSE.

pub mod archive;
pub mod chat;
pub mod chat_ddl;
pub mod clouds;
pub mod ctx;
pub mod daily_cap;
pub mod ddl;
pub mod evolve_ddl;
pub mod filepath;
pub mod forget;
pub mod gojson;
pub use erased_serde;
pub mod harness;
pub mod harness_ddl;
pub mod hooks_ddl;
pub mod kinds;
pub mod mcp_ddl;
pub mod mcp_servers;
pub mod needs;
pub mod posts;
pub mod projects;
pub mod pyjson;
pub mod records;
pub mod records_ddl;
pub mod runner_ddl;
pub mod settings;
pub mod skills;
pub mod store;
pub mod task_source;
pub mod tasks_ddl;
pub mod variants;

pub use archive::*;
pub use chat::*;
pub use chat_ddl::CHAT_DDL;
pub use ctx::{CancellationToken, ContextError};
pub use daily_cap::{DAILY_CAP_COUNT_SQL, DAILY_CAP_JOBS_SQL, daily_cap_of};
pub use ddl::{DDL, JOB_COLUMNS};
pub use evolve_ddl::EVOLVE_DDL;
pub use forget::*;
pub use harness::*;
pub use harness_ddl::{HARNESS_COLUMNS, HARNESS_DDL};
pub use hooks_ddl::HOOKS_DDL;
pub use kinds::*;
pub use mcp_ddl::MCP_DDL;
pub use mcp_servers::*;
pub use posts::*;
pub use projects::*;
pub use pyjson::*;
pub use records::*;
pub use records_ddl::{RECORDS_DDL, RECORDS_VERSION};
pub use runner_ddl::{RUNNER_COLUMNS, RUNNER_DDL};
pub use settings::*;
pub use skills::*;
pub use store::*;
pub use task_source::{TASK_SOURCE_PREFIX, task_source_clause, task_source_id, task_source_params};
pub use tasks_ddl::{COMPUTE_COLUMNS, MESSAGE_COLUMNS, SCHEDULE_COLUMNS, TASK_COLUMNS, TASKS_DDL};
pub use variants::*;

#[cfg(test)]
pub(crate) mod test_helpers {
    pub(crate) fn serial() -> impl Drop {
        pulse_testutil::env_lock()
    }
}
