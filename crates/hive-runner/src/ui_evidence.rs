//! UI evidence: what a worker left for the owner to look at when its change
//! has a face. A job's artifacts folder (`HIVE_HOME/work/jobs/<job>/artifacts`,
//! `hive_harness::artifacts`) reaches the worker as `$HIVE_ARTIFACTS`; the
//! brief's rule 10 asks for a before/after report there whenever the change
//! touches a screen - screenshots of each changed screen before and after,
//! `report.json` beside them and the page `hive report` builds from it
//! (`hive_harness::report`) - and the run's result post and the landing post
//! link the page, or say plainly that a UI change has none.
//!
//! `landing.ui_report` (per project: `landing.ui_report.<slug>`) decides what
//! a missing report costs. `require`: a worker that finished without one gets
//! one more turn to make it (run.rs), a local landing holds the branch until
//! it is there (land.rs), and a pull request without one is the owner's to
//! merge (review.rs); `warn`: the change lands and the posts say it has no
//! report; `off`: the brief asks only for a screenshot, as before.
use std::path::PathBuf;

/// What a missing before/after report costs a project's UI change
/// (`landing.ui_report`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    /// The change waits for its report.
    #[default]
    Require,
    /// The change lands without one, and the landing says so.
    Warn,
    /// No report is asked for.
    Off,
}

impl Mode {
    /// The mode of project `slug`: its own setting, else the machine's.
    pub(crate) fn of(s: &hive_store::Store, slug: &str) -> Mode {
        Mode::parse(&s.ui_report_mode(slug))
    }

    pub(crate) fn parse(v: &str) -> Mode {
        match v.trim() {
            "warn" => Mode::Warn,
            "off" => Mode::Off,
            _ => Mode::Require,
        }
    }

    pub(crate) fn asks(self) -> bool {
        self != Mode::Off
    }
}

/// The hub's address of a job's report page: the artifacts route serves the
/// page and, by their plain names, the pictures beside it.
pub(crate) fn job_report_href(job_id: &str) -> String {
    format!(
        "/api/v1/jobs/{job_id}/artifacts/{}",
        hive_harness::report::PAGE
    )
}

/// The hub's address of a session's report page.
pub(crate) fn chat_report_href(chat_id: &str) -> String {
    format!(
        "/api/v1/chat/{chat_id}/artifacts/{}",
        hive_harness::report::PAGE
    )
}

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

/// What a job's change shows of itself: the UI files it touched, the files
/// its worker left in the artifacts folder and what its report shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UiEvidence {
    pub ui_paths: Vec<String>,
    pub artifacts: Vec<String>,
    pub dir: PathBuf,
    /// What `report.json` there shows (`hive report`), its page built if
    /// the worker wrote the manifest and forgot the command.
    pub report: hive_harness::report::Check,
    /// The hub's address of the report page.
    pub href: String,
    pub mode: Mode,
}

/// How many names a post line spells out before "and N more".
const NAMED: usize = 6;

impl UiEvidence {
    /// The evidence of `job_id`: `changed` are the branch's files, the
    /// artifacts are read from the job's folder under `home` (empty when the
    /// folder is missing).
    pub(crate) fn collect(home: &str, job_id: &str, changed: &[String], mode: Mode) -> UiEvidence {
        let dir = hive_harness::artifacts::job_artifacts_dir(home, job_id);
        UiEvidence::of_dir(dir, job_report_href(job_id), changed, mode)
    }

    /// The evidence in `dir` (a job's or a session's folder) of a change to
    /// `changed`, its report page served at `href`.
    pub(crate) fn of_dir(dir: PathBuf, href: String, changed: &[String], mode: Mode) -> UiEvidence {
        let report = if dir.is_dir() {
            hive_harness::report::ensure_page(&dir)
        } else {
            hive_harness::report::Check::default()
        };
        UiEvidence {
            ui_paths: ui_paths(changed),
            artifacts: hive_harness::artifacts::names(&dir),
            dir,
            report,
            href,
            mode,
        }
    }

    /// The report shows at least one change before and after.
    pub(crate) fn reported(&self) -> bool {
        self.report.shows_change()
    }

    /// A UI change whose report the project requires and that has none:
    /// what holds a landing and keeps a pull request for the owner.
    pub(crate) fn wants_report(&self) -> bool {
        self.mode == Mode::Require && self.touches_ui() && !self.reported()
    }

    /// The markdown link to the report page with what it shows.
    fn report_link(&self) -> String {
        format!(
            "[the before/after report]({}) ({})",
            self.href,
            self.report.counts()
        )
    }

    /// What is wrong with the report, briefly, for a post or the worker.
    pub(crate) fn report_trouble(&self) -> String {
        if !self.report.manifest {
            return format!(
                "there is no {} in $HIVE_ARTIFACTS",
                hive_harness::report::MANIFEST
            );
        }
        let shown: Vec<String> = self.report.problems.iter().take(3).cloned().collect();
        if shown.is_empty() {
            "it shows no change before and after".into()
        } else {
            shown.join("; ")
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

    /// The `- Report:` and `- Evidence:` lines of a run's result post;
    /// nothing when the change has no face and left no files.
    pub(crate) fn result_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.reported() {
            lines.push(format!("- Report: {}", self.report_link()));
        } else if self.touches_ui() && self.mode.asks() {
            lines.push(format!(
                "- Report: none - {} changed and {} (rule 10); {}",
                counted(&self.ui_paths, "UI file"),
                self.report_trouble(),
                if self.mode == Mode::Require {
                    "the landing waits for one (landing.ui_report)"
                } else {
                    "it lands without one (landing.ui_report is warn)"
                }
            ));
        }
        if self.shown() {
            lines.push(format!(
                "- Evidence: {} in {}",
                listed(&self.artifacts),
                self.dir.display()
            ));
        } else if self.touches_ui() && !self.mode.asks() {
            lines.push(format!(
                "- Evidence: none - {} changed and nothing was left in $HIVE_ARTIFACTS (rule 10); the owner cannot see this change without building it",
                counted(&self.ui_paths, "UI file")
            ));
        }
        lines
    }

    /// The sentence the landing post ends with.
    pub(crate) fn landing_sentence(&self) -> String {
        if self.reported() {
            return format!(
                "See {} for {}.",
                self.report_link(),
                if self.touches_ui() {
                    counted(&self.ui_paths, "UI file")
                } else {
                    "this change".to_string()
                }
            );
        }
        if self.touches_ui() && self.mode.asks() {
            return format!(
                "Landed without a before/after report: {} changed ({}) and {}; look at the change itself before trusting the screen.",
                counted(&self.ui_paths, "UI file"),
                listed(&self.ui_paths),
                self.report_trouble()
            );
        }
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
        let mut v = serde_json::json!({
            "ui_paths": self.ui_paths,
            "artifacts": self.artifacts,
            "shown": self.shown(),
            "unshown": self.unshown(),
            "reported": self.reported(),
        });
        if self.report.manifest {
            v["report"] = serde_json::json!({
                "href": self.href,
                "changes": self.report.changes,
                "pairs": self.report.pairs,
                "problems": self.report.problems,
            });
        }
        v
    }
}

/// What the worker is told in its report turn: its commits change a screen
/// and its artifacts folder shows no change before and after yet. The turn
/// carries its session on; the branch is already taken from the verified
/// clone, so nothing it commits now lands.
pub(crate) fn report_turn_message(e: &UiEvidence, base: &str) -> String {
    let dir = e.dir.display();
    let base_short = crate::recipe::short(base, 12);
    format!(
        "Before this run ends: your commits change {} ({}) and {}. A change that has a face reaches the default branch only with its before/after report (landing.ui_report), so make it now. This turn is for the report alone - the extra turn this run gets: change no code and make no commit.\n\n\
1. Before: show each changed screen as it was at the base commit {base_short}. Check the base out beside your clone (`git worktree add \"$TMPDIR/before\" {base}`), run the project's dev server or test harness from there and save each screen as a PNG in $HIVE_ARTIFACTS ({dir}): before-<screen>.png at 1440x900, plus 390x844 for a screen people open on a phone, plus the dark theme when the change touches colour.\n\
2. After: the same screens, the same sizes, from your clone, saved as after-<screen>.png.\n\
3. Write {dir}/{manifest} (`hive report --example` prints its shape): a title, a summary of what changed for the person looking, one entry per visible change with `was`, `now`, `before` and `after`, the checks you ran and where the work stands. Then run `hive report`: it builds {page} and names anything missing; run it until it names nothing.\n\
4. Remove the worktree (`git worktree remove --force \"$TMPDIR/before\"`) and stop any server you started.\n\n\
End with your SUMMARY line as before, naming {page}.",
        counted(&e.ui_paths, "UI file"),
        listed(&e.ui_paths),
        e.report_trouble(),
        manifest = hive_harness::report::MANIFEST,
        page = hive_harness::report::PAGE,
    )
}

/// The evidence a pull request's work left, found from its head: the job's
/// folder for `hive/<agent>/<job>`, the best folder of the task's jobs for
/// `hive/task-<id>` (one whose report shows a change, else the first) and
/// the session's for `hive/chat-<id>`. `None` for a head Hive keeps no
/// folder for: an experiment's `evolve/...`, a person's branch, a job or a
/// session the store does not know.
pub(crate) fn pr_evidence(
    s: &hive_store::Store,
    head: &str,
    files: &[String],
) -> Option<UiEvidence> {
    let home = s.home.clone();
    if let Some(id) = head.strip_prefix(hive_harness::conversation::BRANCH_PREFIX) {
        let chat = s.chat_get(id).ok().flatten()?;
        let mode = Mode::of(s, &chat.project);
        return Some(UiEvidence::of_dir(
            hive_harness::conversation::artifacts_dir(&home, id),
            chat_report_href(id),
            files,
            mode,
        ));
    }
    let conn = s.conn().ok()?;
    if let Some(task) = head.strip_prefix("hive/task-") {
        let pr = crate::util::must_one(
            &conn,
            "SELECT detail FROM events WHERE action = 'land.pr' AND json_extract(detail, '$.task') = ? ORDER BY seq DESC LIMIT 1",
            &[&task],
        );
        let detail: serde_json::Value = serde_json::from_str(&pr.str("detail")).unwrap_or_default();
        let slug = detail["project"].as_str().unwrap_or_default().to_string();
        let mode = Mode::of(s, &slug);
        let jobs: Vec<String> = detail["jobs"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|j| j.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let each: Vec<UiEvidence> = jobs
            .iter()
            .map(|j| UiEvidence::collect(&home, j, files, mode))
            .collect();
        let best = each.iter().position(|e| e.reported()).unwrap_or(0);
        return each.into_iter().nth(best);
    }
    let rest = head.strip_prefix("hive/")?;
    let (_, job_id) = rest.split_once('/')?;
    let job = crate::jobs::job_row(&*conn, job_id.to_string());
    if job.0.is_empty() {
        return None;
    }
    let mode = Mode::of(s, &job.str("project"));
    Some(UiEvidence::collect(&home, job_id, files, mode))
}

/// Why a pull request is the owner's to merge for want of its report;
/// `None` when it shows its change, has no face, or the project does not
/// require a report.
pub(crate) fn pr_report_reason(
    s: &hive_store::Store,
    head: &str,
    files: &[String],
) -> Option<String> {
    let e = pr_evidence(s, head, files)?;
    e.wants_report().then(|| {
        format!(
            "changes {} with no before/after report ({}; landing.ui_report)",
            counted(&e.ui_paths, "UI file"),
            e.report_trouble()
        )
    })
}

/// How long a session's pull request waits for the report it was asked for
/// before the daemon opens it anyway, for the owner to merge.
pub(crate) const REPORT_WAIT_MINUTES: i64 = 60;
/// The event of a session asked for its report; the wait counts from it.
pub(crate) const REPORT_ASKED: &str = "land.report-asked";

/// The evidence of a session's branch (`hive/chat-<id>`): its files
/// against the project's default branch, and the session's artifacts folder.
pub(crate) fn chat_branch_evidence(
    s: &hive_store::Store,
    project: &crate::util::Row,
    branch: &str,
) -> Option<UiEvidence> {
    let id = branch
        .strip_prefix(hive_harness::conversation::BRANCH_PREFIX)
        .filter(|id| !id.is_empty())?;
    let root = project.str("path");
    let def = crate::baseline::default_branch(project.clone());
    let base = crate::util::git_timeout(
        &root,
        std::time::Duration::from_secs(30),
        &["merge-base", &def, branch],
    )
    .stdout
    .trim()
    .to_string();
    if base.is_empty() {
        return None;
    }
    let files = crate::land::branch_files(root, base, branch.to_string());
    Some(UiEvidence::of_dir(
        hive_harness::conversation::artifacts_dir(&s.home, id),
        chat_report_href(id),
        &files,
        Mode::of(s, &project.str("slug")),
    ))
}

/// What a session is told when its branch changes a screen and its folder
/// shows no change before and after.
pub(crate) fn session_report_message(e: &UiEvidence, branch: &str, def: &str) -> String {
    format!(
        "Your branch {branch} changes {} ({}) and {}. Its pull request opens once the before/after report shows the change (landing.ui_report): save a screenshot of each changed screen as it was on {def} (a worktree of {def} beside yours, with the project's dev server) and as it is now in $HIVE_ARTIFACTS ({}), write report.json there (`hive report --example` prints its shape: a was and a now line per change, its before and after pictures, the checks you ran) and run `hive report` until it names nothing. The daemon waits {REPORT_WAIT_MINUTES} minutes for it, then opens the pull request for the owner to merge.",
        counted(&e.ui_paths, "UI file"),
        listed(&e.ui_paths),
        e.report_trouble(),
        e.dir.display()
    )
}

/// Whether the daemon's pull request for a session's branch waits for its
/// before/after report. The first pass that finds the change without one
/// asks the session (`chat.wake`, the session runs a turn on it) and waits;
/// the pull request opens once the report shows the change, or
/// REPORT_WAIT_MINUTES after the ask, when the review gives it to the owner.
/// A session once asked is not asked again for the branch, and an archived
/// one cannot be asked, so it does not wait. Returns the pass's log line
/// while it waits.
pub(crate) fn session_waits_for_report(
    s: &hive_store::Store,
    project: &crate::util::Row,
    branch: &str,
    now: &str,
) -> Option<String> {
    let e = chat_branch_evidence(s, project, branch)?;
    if !e.wants_report() {
        return None;
    }
    let id = branch.strip_prefix(hive_harness::conversation::BRANCH_PREFIX)?;
    let chat = s.chat_get(id).ok().flatten()?;
    if chat.status == "archived" {
        return None;
    }
    let slug = project.str("slug");
    let conn = s.conn().ok()?;
    let asked = crate::util::must_one(
        &conn,
        "SELECT at FROM events WHERE action = ? AND subject = ? AND json_extract(detail, '$.head') = ? ORDER BY seq DESC LIMIT 1",
        &[&REPORT_ASKED, &slug, &branch],
    )
    .str("at");
    if asked.is_empty() {
        let def = crate::baseline::default_branch(project.clone());
        crate::util::event_at(
            &conn,
            now,
            crate::delivery::ACTOR,
            REPORT_ASKED,
            &slug,
            serde_json::json!({
                "project": slug,
                "head": branch,
                "conversation": id,
                "ui_paths": e.ui_paths,
                "note": e.report_trouble(),
            }),
        );
        crate::util::event_at(
            &conn,
            now,
            crate::delivery::ACTOR,
            hive_harness::conversation::WAKE_ACTION,
            id,
            serde_json::json!({"text": session_report_message(&e, branch, &def)}),
        );
        return Some(format!(
            "proposed {slug} {branch}: waits for its before/after report; session {id} asked"
        ));
    }
    let until = crate::util::plus_minutes(&asked, REPORT_WAIT_MINUTES as f64);
    (now < until.as_str()).then(|| {
        format!("proposed {slug} {branch}: waits for its before/after report until {until}")
    })
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
    let dir = hive_harness::artifacts::job_artifacts_dir(home, job_id);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// After a run: a folder the worker left nothing in goes with the clone
/// (the sweep's own rule for an empty one, applied at once), so a job with
/// no evidence leaves no trace under `work/jobs`; one with files stays for
/// the hub. `remove_dir` refuses a folder with anything in it, which is the
/// whole check.
pub(crate) fn drop_empty_dir(home: &str, job_id: &str) {
    let dir = hive_harness::artifacts::job_artifacts_dir(home, job_id);
    if std::fs::remove_dir(&dir).is_ok() {
        let _ = std::fs::remove_dir(hive_harness::artifacts::job_dir(home, job_id));
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
            "crates/hive-hub/src/chat.rs",
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

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
    const PNG2: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR-2";

    /// A report of one change, before and after, in `dir`.
    fn write_report(dir: &std::path::Path) {
        std::fs::write(dir.join("before-x.png"), PNG).unwrap();
        std::fs::write(dir.join("after-x.png"), PNG2).unwrap();
        std::fs::write(
            dir.join(hive_harness::report::MANIFEST),
            r#"{"title":"X","changes":[{"title":"X","was":"w","now":"n","before":"before-x.png","after":"after-x.png"}]}"#,
        )
        .unwrap();
    }

    #[test]
    fn a_ui_change_without_a_report_says_what_it_costs() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_str().unwrap();
        let changed = v(&["web/hub/src/x.tsx", "crates/a/src/lib.rs"]);
        let e = UiEvidence::collect(home, "job-1", &changed, Mode::Require);
        assert!(e.touches_ui() && !e.shown() && e.unshown() && e.wants_report());
        let lines = e.result_lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("- Report: none - 1 UI file changed and there is no report.json in $HIVE_ARTIFACTS (rule 10); the landing waits for one"),
            "{lines:?}"
        );
        assert!(e.landing_sentence().starts_with(
            "Landed without a before/after report: 1 UI file changed (`web/hub/src/x.tsx`)"
        ));
        assert_eq!(e.json()["unshown"], true);
        assert_eq!(e.json()["reported"], false);

        // Warn: the same lines, and the change lands.
        let e = UiEvidence::collect(home, "job-1", &changed, Mode::Warn);
        assert!(!e.wants_report());
        assert!(e.result_lines()[0].ends_with("it lands without one (landing.ui_report is warn)"));

        // Off: the evidence line of before, nothing about a report.
        let e = UiEvidence::collect(home, "job-1", &changed, Mode::Off);
        assert!(!e.wants_report());
        let lines = e.result_lines();
        assert!(
            lines[0].starts_with("- Evidence: none - 1 UI file changed"),
            "{lines:?}"
        );
        assert!(
            e.landing_sentence()
                .starts_with("Landed unshown: 1 UI file changed")
        );

        // Pictures without a report.json do not show the change.
        let dir = prepare_dir(home, "job-1").unwrap();
        std::fs::write(dir.join("after.png"), PNG).unwrap();
        let e = UiEvidence::collect(home, "job-1", &changed, Mode::Require);
        assert!(e.shown() && !e.unshown() && e.wants_report());
        let lines = e.result_lines();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[1].starts_with("- Evidence: `after.png` in "),
            "{lines:?}"
        );
    }

    #[test]
    fn a_report_is_built_linked_and_lets_the_change_through() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_str().unwrap();
        let changed = v(&["web/hub/src/x.tsx"]);
        let dir = prepare_dir(home, "job-1").unwrap();
        write_report(&dir);
        // The worker wrote the manifest and forgot `hive report`: the page is built.
        let e = UiEvidence::collect(home, "job-1", &changed, Mode::Require);
        assert!(e.reported() && !e.wants_report());
        assert!(dir.join(hive_harness::report::PAGE).is_file());
        let link = "[the before/after report](/api/v1/jobs/job-1/artifacts/report.html) (1 change, 1 before/after pair)";
        assert_eq!(e.result_lines()[0], format!("- Report: {link}"));
        assert_eq!(e.landing_sentence(), format!("See {link} for 1 UI file."));
        assert_eq!(e.json()["report"]["pairs"], 1);
        assert_eq!(
            e.json()["report"]["href"],
            "/api/v1/jobs/job-1/artifacts/report.html"
        );

        // No UI, no files: nothing to say.
        let quiet = UiEvidence::collect(home, "job-2", &v(&["src/lib.rs"]), Mode::Require);
        assert!(quiet.result_lines().is_empty() && !quiet.wants_report());
        assert_eq!(quiet.landing_sentence(), "");
        // Files without a UI change are still named.
        let dir = prepare_dir(home, "job-3").unwrap();
        std::fs::write(dir.join("bench.csv"), b"a,b\n").unwrap();
        let e = UiEvidence::collect(home, "job-3", &v(&["src/lib.rs"]), Mode::Require);
        assert!(
            e.landing_sentence()
                .starts_with("Evidence left by the worker: `bench.csv`")
        );
    }

    #[test]
    fn the_report_turn_names_the_files_the_base_and_the_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_str().unwrap();
        let e = UiEvidence::collect(
            home,
            "job-1",
            &v(&["web/a.tsx", "web/b.css"]),
            Mode::Require,
        );
        let m = report_turn_message(&e, "0123456789abcdef0123");
        assert!(m.starts_with("Before this run ends: your commits change 2 UI files (`web/a.tsx`, `web/b.css`) and there is no report.json"), "{m}");
        assert!(
            m.contains("git worktree add \"$TMPDIR/before\" 0123456789abcdef0123"),
            "{m}"
        );
        assert!(m.contains("base commit 0123456789ab."), "{m}");
        assert!(
            m.contains(&format!("{}/report.json", e.dir.display())),
            "{m}"
        );
        assert!(m.contains("change no code and make no commit"), "{m}");
    }

    #[test]
    fn a_pull_request_finds_its_job_task_or_session_folder() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        let home = s.home.clone();
        let files = v(&["web/hub/src/x.tsx"]);
        let conn = s.conn().unwrap();
        conn.execute(
            "INSERT INTO jobs (id, project, title, kind, status, created_at) VALUES ('job-9', 'web', 't', 'work', 'done', '2026-11-07T09:00:00Z')",
            [],
        )
        .unwrap();
        crate::util::event_at(
            &conn,
            "2026-11-07T10:00:00Z",
            "hive",
            "land.pr",
            "web",
            serde_json::json!({"project": "web", "task": "TASK-7", "jobs": ["job-9"], "number": 3}),
        );
        drop(conn);
        // A job's own head and its task's head both find the job's folder.
        let why = pr_report_reason(&s, "hive/aurora/job-9", &files).unwrap();
        assert!(
            why.starts_with(
                "changes 1 UI file with no before/after report (there is no report.json"
            ),
            "{why}"
        );
        assert!(pr_report_reason(&s, "hive/task-TASK-7", &files).is_some());
        // An experiment, a person's branch or a job nobody knows: no folder, no reason.
        for head in ["evolve/B-1", "feature/x", "hive/aurora/job-404"] {
            assert!(pr_evidence(&s, head, &files).is_none(), "{head}");
        }
        // No face, or the project lets it through: no reason.
        assert!(pr_report_reason(&s, "hive/aurora/job-9", &v(&["src/lib.rs"])).is_none());
        s.set_setting("human", &hive_store::ui_report_setting("web"), "warn")
            .unwrap();
        assert!(pr_report_reason(&s, "hive/aurora/job-9", &files).is_none());
        s.set_setting("human", &hive_store::ui_report_setting("web"), "require")
            .unwrap();
        // The report is there: the pull request goes the usual way.
        let dir = prepare_dir(&home, "job-9").unwrap();
        write_report(&dir);
        assert!(pr_report_reason(&s, "hive/aurora/job-9", &files).is_none());
        assert!(pr_report_reason(&s, "hive/task-TASK-7", &files).is_none());
    }

    #[test]
    fn long_lists_are_cut() {
        let many: Vec<String> = (0..9).map(|i| format!("f{i}.png")).collect();
        let s = listed(&many);
        assert!(s.ends_with("and 3 more"), "{s}");
        assert_eq!(s.matches('`').count(), 12);
    }
}
