//! Artifacts: the files an agent leaves for the owner's eyes - screenshots,
//! reports, HTML pages, CSVs - as opposed to the commits on its branch.
//!
//! A conversation keeps them under `HIVE_HOME/work/chat/<id>/artifacts`
//! (conversation.rs); a fleet job keeps them under
//! `HIVE_HOME/work/jobs/<job-id>/artifacts`, a folder that outlives the
//! run's throwaway clone so the hub can show the evidence beside the job
//! long after the clone is swept. Both reach the agent as `$HIVE_ARTIFACTS`
//! and both are served by the hub over `/api/v1/{chat,jobs}/{id}/artifacts`.
//!
//! The job folder sits under `work/` so a bulk disk, when one is attached,
//! holds it like the clones; its own sweep window is
//! `daemon.artifact_keep_days` (runner sweep), not the clone's grace.
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The environment variable that names the artifacts folder inside the
/// agent's commands (`hive_mcp` reads it for Playwright's output too).
pub const ARTIFACTS_ENV: &str = "HIVE_ARTIFACTS";

/// One file in an artifacts folder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Artifact {
    pub name: String,
    pub size: u64,
    pub modified: String,
}

/// `HIVE_HOME/work/jobs/<job-id>`: a job's durable folder.
pub fn job_dir(home: &str, job_id: &str) -> PathBuf {
    Path::new(home).join("work").join("jobs").join(job_id)
}

/// `HIVE_HOME/work/jobs/<job-id>/artifacts`: where a job's worker leaves
/// files for the owner.
pub fn job_artifacts_dir(home: &str, job_id: &str) -> PathBuf {
    job_dir(home, job_id).join("artifacts")
}

/// The files in `dir`, newest first; dotfiles and folders are not listed.
/// A missing folder lists nothing.
pub fn list(dir: &Path) -> Vec<Artifact> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(std::time::SystemTime, Artifact)> = entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            let modified = meta.modified().ok()?;
            let when: chrono::DateTime<chrono::Utc> = modified.into();
            Some((
                modified,
                Artifact {
                    name,
                    size: meta.len(),
                    modified: when.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                },
            ))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    out.into_iter().map(|(_, a)| a).collect()
}

/// The artifact names of `dir`, newest first (what a result post lists).
pub fn names(dir: &Path) -> Vec<String> {
    list(dir).into_iter().map(|a| a.name).collect()
}

/// The file `name` names in `dir` when it is a plain file name of a
/// regular file there; a path part, a control character or a dotfile
/// answers nothing, so a request never reaches outside the folder.
pub fn file(dir: &Path, name: &str) -> Option<PathBuf> {
    if !plain_file_name(name) || name.starts_with('.') {
        return None;
    }
    let path = dir.join(name);
    path.is_file().then_some(path)
}

/// `name` is one path segment with no control characters.
pub fn plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.chars().any(|c| c.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_folders_sit_under_work_jobs() {
        assert_eq!(
            job_artifacts_dir("/h", "job-1"),
            PathBuf::from("/h/work/jobs/job-1/artifacts")
        );
        assert_eq!(job_dir("/h", "job-1"), PathBuf::from("/h/work/jobs/job-1"));
    }

    #[test]
    fn list_skips_dotfiles_and_folders_and_orders_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("old.png"), b"png").unwrap();
        std::fs::write(dir.join(".hidden"), b"x").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("new.html"), b"<h1>hi</h1>").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::open(dir.join("new.html"))
            .unwrap()
            .set_modified(later)
            .unwrap();
        let got = list(dir);
        assert_eq!(
            got.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            vec!["new.html", "old.png"]
        );
        assert_eq!(got[0].size, 11);
        assert!(got[0].modified.ends_with('Z'));
        assert_eq!(names(dir), vec!["new.html", "old.png"]);
        assert!(list(&dir.join("missing")).is_empty());
    }

    #[test]
    fn file_refuses_paths_outside_the_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("shot.png"), b"png").unwrap();
        std::fs::write(dir.join(".env"), b"secret").unwrap();
        assert_eq!(file(dir, "shot.png"), Some(dir.join("shot.png")));
        for bad in [
            "",
            ".",
            "..",
            "../shot.png",
            "a/b",
            "a\\b",
            "x\u{1}",
            ".env",
            "missing",
        ] {
            assert_eq!(file(dir, bad), None, "{bad:?}");
        }
        assert!(!plain_file_name("a/b"));
        assert!(plain_file_name("a b.png"));
    }
}
