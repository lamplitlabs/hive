//! UI evidence: what a worker left for the owner to look at when its change
//! has a face. A job's artifacts folder (`PULSE_HOME/work/jobs/<job>/artifacts`,
//! `pulse_harness::artifacts`) reaches the worker as `$PULSE_ARTIFACTS`; the
//! brief's rule 10 asks for a screenshot or a report there whenever the
//! change touches a screen, and the run's result post and the landing post
//! name what was left - or say plainly that a UI change landed unshown. This
//! is a soft gate: nothing is held for missing evidence, the owner is told.
use std::path::PathBuf;

/// A path a person looks at the result of: a page, a view, a stylesheet, a
/// native screen. Test files and generated bundles are not screens.
pub(crate) fn is_ui_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let parts: Vec<&str> = lower.split('/').collect();
    let name = parts.last().copied().unwrap_or("");
    if name.contains(".test.")
        || name.contains(".spec.")
        || name.ends_with(".stories.tsx")
        || parts.iter().any(|p| {
            matches!(
                *p,
                "__tests__"
                    | "tests"
                    | "test"
                    | "node_modules"
                    | "out"
                    | "dist"
                    | "build"
                    | ".next"
            )
        })
    {
        return false;
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    match ext {
        "tsx" | "jsx" | "vue" | "svelte" | "astro" | "html" | "htm" | "css" | "scss" | "sass"
        | "less" | "styl" | "xib" | "storyboard" => true,
        "xml" => parts.len() >= 2 && parts[parts.len() - 2].starts_with("layout"),
        "dart" => {
            stem.ends_with("_screen")
                || stem.ends_with("_page")
                || stem.ends_with("_view")
                || stem.ends_with("_widget")
        }
        "kt" | "swift" | "java" => {
            stem.ends_with("screen")
                || stem.ends_with("view")
                || stem.ends_with("page")
                || stem.ends_with("activity")
                || stem.ends_with("fragment")
        }
        _ => false,
    }
}

/// The UI paths among `files`, in order.
pub(crate) fn ui_paths(files: &[String]) -> Vec<String> {
    files.iter().filter(|f| is_ui_path(f)).cloned().collect()
}

/// What a job's change shows of itself: the UI files it touched and the
/// files its worker left in the artifacts folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UiEvidence {
    pub ui_paths: Vec<String>,
    pub artifacts: Vec<String>,
    pub dir: PathBuf,
}

/// How many names a post line spells out before "and N more".
const NAMED: usize = 6;

impl UiEvidence {
    /// The evidence of `job_id`: `changed` are the branch's files, the
    /// artifacts are read from the job's folder under `home` (empty when the
    /// folder is missing).
    pub(crate) fn collect(home: &str, job_id: &str, changed: &[String]) -> UiEvidence {
        let dir = pulse_harness::artifacts::job_artifacts_dir(home, job_id);
        UiEvidence {
            ui_paths: ui_paths(changed),
            artifacts: pulse_harness::artifacts::names(&dir),
            dir,
        }
    }

    pub(crate) fn touches_ui(&self) -> bool {
        !self.ui_paths.is_empty()
    }

    pub(crate) fn shown(&self) -> bool {
        !self.artifacts.is_empty()
    }

    /// A UI change that left nothing to look at.
    pub(crate) fn unshown(&self) -> bool {
        self.touches_ui() && !self.shown()
    }

    /// The `- Evidence:` line of a run's result post; nothing when the
    /// change has no face and left no files.
    pub(crate) fn result_line(&self) -> Option<String> {
        if self.shown() {
            Some(format!(
                "- Evidence: {} in {}",
                listed(&self.artifacts),
                self.dir.display()
            ))
        } else if self.touches_ui() {
            Some(format!(
                "- Evidence: none - {} changed and nothing was left in $PULSE_ARTIFACTS (rule 10); the owner cannot see this change without building it",
                counted(&self.ui_paths, "UI file")
            ))
        } else {
            None
        }
    }

    /// The sentence the landing post ends with.
    pub(crate) fn landing_sentence(&self) -> String {
        if self.shown() && self.touches_ui() {
            format!(
                "Evidence for {}: {} in the job's artifacts folder (the hub shows them beside the job).",
                counted(&self.ui_paths, "UI file"),
                listed(&self.artifacts)
            )
        } else if self.shown() {
            format!(
                "Evidence left by the worker: {} (the hub shows them beside the job).",
                listed(&self.artifacts)
            )
        } else if self.touches_ui() {
            format!(
                "Landed unshown: {} changed ({}) and the worker left no screenshot or report in its artifacts folder; look at the change itself before trusting the screen.",
                counted(&self.ui_paths, "UI file"),
                listed(&self.ui_paths)
            )
        } else {
            String::new()
        }
    }

    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "ui_paths": self.ui_paths,
            "artifacts": self.artifacts,
            "shown": self.shown(),
            "unshown": self.unshown(),
        })
    }
}

fn counted(items: &[String], noun: &str) -> String {
    format!(
        "{} {}",
        items.len(),
        crate::util::plural(items.len() as i32, noun)
    )
}

fn listed(items: &[String]) -> String {
    let mut s = items
        .iter()
        .take(NAMED)
        .map(|n| format!("`{n}`"))
        .collect::<Vec<_>>()
        .join(", ");
    if items.len() > NAMED {
        s.push_str(&format!(" and {} more", items.len() - NAMED));
    }
    s
}

/// The folder the hub serves for a job, made for the worker before its run;
/// `None` when it cannot be made (the run goes on, the brief says nothing).
pub(crate) fn prepare_dir(home: &str, job_id: &str) -> Option<PathBuf> {
    let dir = pulse_harness::artifacts::job_artifacts_dir(home, job_id);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// After a run: a folder the worker left nothing in goes with the clone
/// (the sweep's own rule for an empty one, applied at once), so a job with
/// no evidence leaves no trace under `work/jobs`; one with files stays for
/// the hub. `remove_dir` refuses a folder with anything in it, which is the
/// whole check.
pub(crate) fn drop_empty_dir(home: &str, job_id: &str) {
    let dir = pulse_harness::artifacts::job_artifacts_dir(home, job_id);
    if std::fs::remove_dir(&dir).is_ok() {
        let _ = std::fs::remove_dir(pulse_harness::artifacts::job_dir(home, job_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn ui_paths_are_screens_not_tests_or_bundles() {
        for yes in [
            "web/hub/src/components/chat/task-view.tsx",
            "web/hub/src/app/globals.css",
            "site/index.html",
            "app/src/main/res/layout/activity_main.xml",
            "ios/App/Base.lproj/Main.storyboard",
            "lib/screens/home_screen.dart",
            "app/src/main/java/com/x/LoginActivity.kt",
            "Sources/App/SettingsView.swift",
        ] {
            assert!(is_ui_path(yes), "{yes}");
        }
        for no in [
            "web/hub/src/lib/api.ts",
            "web/hub/src/components/chat/task-view.test.tsx",
            "web/hub/src/components/__tests__/x.tsx",
            "web/hub/out/_next/static/chunks/app/page-1234.js",
            "crates/pulse-hub/src/chat.rs",
            "README.md",
            "app/src/main/res/values/strings.xml",
            "lib/models/user.dart",
            "src/Repository.kt",
        ] {
            assert!(!is_ui_path(no), "{no}");
        }
        assert_eq!(
            ui_paths(&v(&["a.rs", "b.tsx", "c.css"])),
            v(&["b.tsx", "c.css"])
        );
    }

    #[test]
    fn evidence_lines_name_files_or_say_unshown() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_str().unwrap();
        let changed = v(&["web/hub/src/x.tsx", "crates/a/src/lib.rs"]);
        let e = UiEvidence::collect(home, "job-1", &changed);
        assert!(e.touches_ui() && !e.shown() && e.unshown());
        let line = e.result_line().unwrap();
        assert!(
            line.starts_with("- Evidence: none - 1 UI file changed"),
            "{line}"
        );
        assert!(
            e.landing_sentence()
                .starts_with("Landed unshown: 1 UI file changed (`web/hub/src/x.tsx`)")
        );
        assert_eq!(e.json()["unshown"], true);

        let dir = prepare_dir(home, "job-1").unwrap();
        std::fs::write(dir.join("after.png"), b"png").unwrap();
        std::fs::write(dir.join("report.html"), b"<h1>ok</h1>").unwrap();
        let e = UiEvidence::collect(home, "job-1", &changed);
        assert!(e.shown() && !e.unshown());
        let line = e.result_line().unwrap();
        assert!(line.starts_with("- Evidence: `"), "{line}");
        assert!(line.contains("`after.png`") && line.contains("`report.html`"));
        assert!(line.ends_with(&dir.display().to_string()));
        assert!(
            e.landing_sentence()
                .starts_with("Evidence for 1 UI file: `")
        );
        assert_eq!(e.json()["artifacts"].as_array().unwrap().len(), 2);

        // No UI, no files: nothing to say.
        let quiet = UiEvidence::collect(home, "job-2", &v(&["src/lib.rs"]));
        assert_eq!(quiet.result_line(), None);
        assert_eq!(quiet.landing_sentence(), "");
        // Files without a UI change are still named.
        let dir = prepare_dir(home, "job-3").unwrap();
        std::fs::write(dir.join("bench.csv"), b"a,b\n").unwrap();
        let e = UiEvidence::collect(home, "job-3", &v(&["src/lib.rs"]));
        assert!(
            e.landing_sentence()
                .starts_with("Evidence left by the worker: `bench.csv`")
        );
    }

    #[test]
    fn long_lists_are_cut() {
        let many: Vec<String> = (0..9).map(|i| format!("f{i}.png")).collect();
        let s = listed(&many);
        assert!(s.ends_with("and 3 more"), "{s}");
        assert_eq!(s.matches('`').count(), 12);
    }
}
