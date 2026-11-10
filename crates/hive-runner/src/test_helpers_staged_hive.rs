use parking_lot::Mutex;
use std::sync::LazyLock;

#[allow(dead_code, reason = "shared test helper; not every one has a caller")]
static STAGED_LOCAL_HIVE: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new(String::new()));

#[allow(dead_code, reason = "shared test helper; not every one has a caller")]
fn staged_local_hive() -> String {
    STAGED_LOCAL_HIVE.lock().clone()
}

#[allow(dead_code, reason = "shared test helper; not every one has a caller")]
pub(crate) struct StageLocalHiveGuard {
    prev: String,
    prev_hook: Option<fn() -> String>,
}

impl Drop for StageLocalHiveGuard {
    fn drop(&mut self) {
        *STAGED_LOCAL_HIVE.lock() = self.prev.clone();
        *crate::run::STAGED_HIVE_PATH
            .write()
            .expect("staged hive path") = self.prev_hook;
    }
}

#[allow(dead_code, reason = "shared test helper; not every one has a caller")]
pub(crate) fn stage_local_hive(bin: &str) -> StageLocalHiveGuard {
    let prev_hook = *crate::run::STAGED_HIVE_PATH
        .read()
        .expect("staged hive path");
    let prev = {
        let mut slot = STAGED_LOCAL_HIVE.lock();
        let prev = slot.clone();
        *slot = bin.to_string();
        prev
    };
    *crate::run::STAGED_HIVE_PATH
        .write()
        .expect("staged hive path") = Some(staged_local_hive);
    StageLocalHiveGuard { prev, prev_hook }
}
