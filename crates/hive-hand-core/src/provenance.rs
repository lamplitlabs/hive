//! Provenance (ADR-022): the trailers every commit of a run carries, added
//! by a `prepare-commit-msg` hook local to the clone.

/// The trailer keys, in the order the hook writes them.
pub static PROVENANCE_TRAILERS: &[&str] = &["Hive-Agent", "Hive-Job", "Hive-Executor"];

/// The `prepare-commit-msg` hook that adds this run's trailers.
pub fn provenance_hook(agent: &str, job_id: &str, executor: &str) -> String {
    let values = [agent, job_id, executor];
    let parts = PROVENANCE_TRAILERS
        .iter()
        .zip(values)
        .map(|(k, v)| {
            format!(
                "--trailer {}",
                crate::util::shell_quote(&format!("{k}: {v}"))
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "#!/bin/sh\n# hive: provenance trailers for every commit of this run (ADR-022). Local to this clone, never committed.\nexec git interpret-trailers --in-place --if-exists doNothing {parts} \"$1\"\n"
    )
}
