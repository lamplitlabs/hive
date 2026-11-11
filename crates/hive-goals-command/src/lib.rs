//! Hive: the hive goal command.

mod command;

pub use command::register;
#[cfg(test)]
pub(crate) mod test_helpers {
    pub(crate) fn serial() -> impl Drop {
        hive_testutil::env_lock()
    }
}
