//! The before/after report: the one page an agent leaves when its change
//! has a face. The agent takes pictures of each screen it is about to change
//! before it changes anything, and again when it is done, saves them in its
//! artifacts folder (`$PULSE_ARTIFACTS`, artifacts.rs) and writes
//! `report.json` beside them; `pulse report` turns that into `report.html`:
//! each change as a "Was" and a "Now" line over its before | after pair,
//! grouped by area, with the checks that ran and where the work stands. The
//! hub serves the page beside the job or the conversation, and the landing
//! of a UI change asks for it (`landing.ui_report`).
//!
//! The page is built here, never by hand, so every agent's report reads the
//! same; it needs no script and no font from elsewhere, and names its
//! pictures by their file names, which the hub serves from the same folder.
//!
//! PULSE builds the same page itself for every job and session once the work
//! is done ([`build_full`], pulse-runner's work_report.rs gathers the
//! [`Facts`]): what PULSE knows - where the work stands, who did it and how,
//! its commits and files, the checks - around the agent's own report.json
//! when there is one, and every other picture the agent saved, so each piece
//! of work has one page to read whether or not it changed a screen.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

/// The manifest the agent writes in its artifacts folder.
pub const MANIFEST: &str = "report.json";
/// The page `pulse report` builds beside it.
pub const PAGE: &str = "report.html";

/// What `report.json` holds: the agent's account of a change that has a
/// face. Every text is a sentence or two; `code` in backticks is set as code.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Manifest {
    /// The page's heading: what the work did, in a few words.
    pub title: String,
    /// What changed for the person looking at the screen, and why.
    pub summary: String,
    /// Each visible change, in the order the owner should read them.
    pub changes: Vec<Change>,
    /// The checks that ran on the change, one line each.
    pub checks: Vec<String>,
    /// Where the work stands: the branch, landed or not, what is left.
    pub status: String,
    /// Changes that are not pictured, one line each.
    pub notes: Vec<String>,
}

/// One visible change: how the screen was, how it is now, and the pictures
/// of both.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Change {
    /// The part of the product it is shown under ("Sidebar", "Settings").
    pub area: String,
    pub title: String,
    /// How the screen was before the change.
    pub was: String,
    /// How it is now.
    pub now: String,
    /// The picture before the change: a file name in the artifacts folder.
    pub before: String,
    /// The picture after it.
    pub after: String,
    /// More pairs of the same change (phone width, dark theme), each labelled.
    pub pairs: Vec<Pair>,
}

/// A before | after pair of pictures, by file name in the artifacts folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Pair {
    /// What the pair shows ("Phone", "Dark"); empty for the main one.
    pub label: String,
    pub before: String,
    pub after: String,
}

impl Change {
    /// Its pairs: the one named on the change itself first, then `pairs`.
    pub fn all_pairs(&self) -> Vec<Pair> {
        let mut out = Vec::new();
        if !self.before.trim().is_empty() || !self.after.trim().is_empty() {
            out.push(Pair {
                label: String::new(),
                before: self.before.clone(),
                after: self.after.clone(),
            });
        }
        out.extend(self.pairs.iter().cloned());
        out
    }
}

/// What a folder's report shows, and what is wrong with it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Check {
    /// `report.json` is in the folder.
    pub manifest: bool,
    /// `report.html` is in the folder.
    pub page: bool,
    pub title: String,
    pub changes: usize,
    /// Before | after pairs with both pictures in the folder.
    pub pairs: usize,
    /// What the agent should fix, one sentence each.
    pub problems: Vec<String>,
}

impl Check {
    /// At least one change is shown before and after: what the landing of a
    /// UI change asks for.
    pub fn shows_change(&self) -> bool {
        self.pairs > 0
    }

    /// Nothing left for the agent to fix.
    pub fn clean(&self) -> bool {
        self.shows_change() && self.problems.is_empty()
    }

    /// "3 changes, 4 before/after pairs" - what the posts say of it.
    pub fn counts(&self) -> String {
        format!(
            "{} {}, {} before/after {}",
            self.changes,
            if self.changes == 1 {
                "change"
            } else {
                "changes"
            },
            self.pairs,
            if self.pairs == 1 { "pair" } else { "pairs" }
        )
    }
}

/// What PULSE knows about a piece of work, for the page it builds when the
/// work is done; every text is a sentence or a few words.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// What the work is: "Task TASK-083", "Job", "Session".
    pub kind: String,
    /// The heading when report.json gives none: the task's or session's title.
    pub title: String,
    /// What it did when report.json says nothing: the worker's summary, the
    /// session's last word.
    pub summary: String,
    /// One word for where it stands: Merged, Landed, In review, Done,
    /// Working, Failed, Stopped.
    pub state: String,
    /// One sentence on where it stands, read before the agent's own.
    pub status: String,
    /// Who did it and how: ("Agent", "borealis-28"), ("Took", "1h 21m").
    pub about: Vec<(String, String)>,
    /// Where to read more: a label and a URL.
    pub links: Vec<(String, String)>,
    /// The work's commits, oldest first: a short sha and its subject.
    pub commits: Vec<(String, String)>,
    /// The files those commits changed.
    pub files: Vec<FileChange>,
    /// What PULSE checked on it (the verify step, the landing), one line each.
    pub checks: Vec<String>,
    /// More that belongs to the work: sub-sessions and tasks, one line each.
    pub notes: Vec<String>,
    /// When PULSE built the page, for its foot.
    pub built: String,
}

/// One file a piece of work changed, with its lines added and removed
/// (`None` for a binary file).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub added: Option<u64>,
    pub removed: Option<u64>,
}

/// The manifest in `dir`: `Ok(None)` when there is none, the parse error in
/// words when it does not read.
pub fn read_manifest(dir: &Path) -> Result<Option<Manifest>, String> {
    let path = dir.join(MANIFEST);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{MANIFEST} could not be read: {e}")),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("{MANIFEST} does not read: {e}"))
}

/// What the report in `dir` shows; nothing is written.
pub fn check(dir: &Path) -> Check {
    let mut c = Check {
        page: dir.join(PAGE).is_file(),
        ..Check::default()
    };
    match read_manifest(dir) {
        Ok(Some(m)) => {
            c.manifest = true;
            inspect(dir, &m, &mut c);
        }
        Ok(None) => c.problems.push(format!(
            "there is no {MANIFEST} in {}; write one (`pulse report --example` prints its shape)",
            dir.display()
        )),
        Err(e) => {
            c.manifest = true;
            c.problems.push(e);
        }
    }
    c
}

/// Builds `report.html` in `dir` from its manifest and says what it shows.
/// The page is written whenever the manifest reads, problems and all, so a
/// half-done report is still there to look at; a missing or unreadable
/// manifest writes nothing.
pub fn build(dir: &Path) -> Check {
    let mut c = check(dir);
    let Ok(Some(m)) = read_manifest(dir) else {
        return c;
    };
    let html = render(dir, &m, &built_at(dir));
    let tmp = dir.join(format!(".{PAGE}.tmp"));
    match std::fs::write(&tmp, html).and_then(|_| std::fs::rename(&tmp, dir.join(PAGE))) {
        Ok(()) => c.page = true,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            c.problems.push(format!(
                "{PAGE} could not be written in {}: {e}",
                dir.display()
            ));
        }
    }
    c
}

/// The report in `dir`, with its page built when the manifest is newer than
/// the page or the page is missing: an agent that wrote the manifest and
/// forgot `pulse report` still shows its work.
pub fn ensure_page(dir: &Path) -> Check {
    let modified = |name: &str| {
        std::fs::metadata(dir.join(name))
            .and_then(|m| m.modified())
            .ok()
    };
    match (modified(MANIFEST), modified(PAGE)) {
        (Some(m), Some(p)) if p >= m => check(dir),
        (Some(_), _) => build(dir),
        _ => check(dir),
    }
}

/// Builds `report.html` in `dir` from what PULSE knows of the work and the
/// agent's report.json when there is one (an unreadable one is left out and
/// named among the problems). The page is written either way: every finished
/// job and session gets one.
pub fn build_full(dir: &Path, facts: &Facts) -> Check {
    if let Err(e) = std::fs::create_dir_all(dir) {
        return Check {
            problems: vec![format!("{} could not be made: {e}", dir.display())],
            ..Check::default()
        };
    }
    let mut c = check(dir);
    let manifest = read_manifest(dir).ok().flatten();
    let html = render_page(dir, manifest.as_ref(), Some(facts), &facts.built);
    // A name of its own per writer: two requests for the page at once each
    // write a whole page and the last rename wins.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(".{PAGE}.{}.{nanos}.tmp", std::process::id()));
    match std::fs::write(&tmp, html).and_then(|_| std::fs::rename(&tmp, dir.join(PAGE))) {
        Ok(()) => c.page = true,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            c.problems
                .push(format!("{PAGE} could not be written in {}: {e}", dir.display()));
        }
    }
    c
}

/// The pictures in `dir` that no before/after pair of `m` names, by name:
/// the screenshots an agent saved outside a pair still show.
pub fn loose_pictures(dir: &Path, m: Option<&Manifest>) -> Vec<String> {
    let used: BTreeSet<String> = m
        .map(|m| {
            m.changes
                .iter()
                .flat_map(Change::all_pairs)
                .flat_map(|p| [p.before.trim().to_string(), p.after.trim().to_string()])
                .collect()
        })
        .unwrap_or_default();
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| !name.starts_with('.') && !used.contains(name))
        .filter(|name| picture(dir, name).is_ok())
        .collect();
    out.sort();
    out
}

/// When the manifest was written, for the page's foot.
fn built_at(dir: &Path) -> String {
    std::fs::metadata(dir.join(MANIFEST))
        .and_then(|m| m.modified())
        .map(|t| {
            let t: chrono::DateTime<chrono::Utc> = t.into();
            t.format("%Y-%m-%d %H:%M UTC").to_string()
        })
        .unwrap_or_default()
}

/// The pair's two pictures as found in the folder.
struct PairState {
    before: Result<(), String>,
    after: Result<(), String>,
    /// Both are there and byte for byte the same picture.
    same: bool,
}

impl PairState {
    fn of(dir: &Path, p: &Pair) -> PairState {
        let before = picture(dir, &p.before);
        let after = picture(dir, &p.after);
        let same = before.is_ok() && after.is_ok() && same_bytes(dir, &p.before, &p.after);
        PairState {
            before,
            after,
            same,
        }
    }

    fn complete(&self) -> bool {
        self.before.is_ok() && self.after.is_ok()
    }
}

fn inspect(dir: &Path, m: &Manifest, c: &mut Check) {
    c.title = m.title.trim().to_string();
    c.changes = m.changes.len();
    if m.title.trim().is_empty() {
        c.problems.push("the report has no title".into());
    }
    if m.changes.is_empty() {
        c.problems.push(
            "the report lists no changes; each visible change needs its before and after pictures"
                .into(),
        );
    }
    for (i, ch) in m.changes.iter().enumerate() {
        let name = change_name(i, ch);
        if ch.title.trim().is_empty() {
            c.problems.push(format!("{name} has no title"));
        }
        if ch.was.trim().is_empty() || ch.now.trim().is_empty() {
            c.problems.push(format!(
                "{name} does not say what it was and what it is now (`was`, `now`)"
            ));
        }
        let pairs = ch.all_pairs();
        if pairs.is_empty() {
            c.problems.push(format!(
                "{name} names no before and after pictures; a change you cannot picture goes in `notes`"
            ));
        }
        for p in &pairs {
            let st = PairState::of(dir, p);
            let which = if p.label.trim().is_empty() {
                name.clone()
            } else {
                format!("{name} ({})", p.label.trim())
            };
            if let Err(e) = &st.before {
                c.problems.push(format!("{which}, before: {e}"));
            }
            if let Err(e) = &st.after {
                c.problems.push(format!("{which}, after: {e}"));
            }
            if st.complete() {
                c.pairs += 1;
            }
        }
    }
}

fn change_name(i: usize, ch: &Change) -> String {
    let title = ch.title.trim();
    if title.is_empty() {
        format!("change {}", i + 1)
    } else {
        format!("change {} \"{title}\"", i + 1)
    }
}

/// `name` is a picture in `dir`: a plain file name of a PNG, JPEG, WebP,
/// GIF or AVIF file there, read by its first bytes rather than its name.
fn picture(dir: &Path, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("no picture is named".into());
    }
    if !crate::artifacts::plain_file_name(name) || name.starts_with('.') {
        return Err(format!(
            "`{name}` is not a file name in the folder; save the picture in the artifacts folder itself and name the file"
        ));
    }
    let Ok(mut f) = std::fs::File::open(dir.join(name)) else {
        return Err(format!("`{name}` is not in the folder"));
    };
    let mut head = [0u8; 16];
    let n = f.read(&mut head).unwrap_or(0);
    if is_picture(&head[..n]) {
        Ok(())
    } else {
        Err(format!(
            "`{name}` is not a picture (PNG, JPEG, WebP, GIF or AVIF)"
        ))
    }
}

fn is_picture(head: &[u8]) -> bool {
    head.starts_with(b"\x89PNG\r\n\x1a\n")
        || head.starts_with(&[0xFF, 0xD8, 0xFF])
        || head.starts_with(b"GIF87a")
        || head.starts_with(b"GIF89a")
        || (head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP")
        || (head.len() >= 12 && &head[4..8] == b"ftyp" && matches!(&head[8..12], b"avif" | b"avis"))
}

fn same_bytes(dir: &Path, a: &str, b: &str) -> bool {
    let (a, b) = (dir.join(a.trim()), dir.join(b.trim()));
    if a == b {
        return true;
    }
    let len = |p: &Path| std::fs::metadata(p).map(|m| m.len()).ok();
    if len(&a) != len(&b) {
        return false;
    }
    matches!((std::fs::read(&a), std::fs::read(&b)), (Ok(x), Ok(y)) if x == y)
}

/// The example `pulse report --example` prints: the shape of the manifest
/// with every field in use.
pub fn example() -> String {
    let m = Manifest {
        title: "Sidebar fits the owner's name".into(),
        summary: "The sidebar's foot cut the owner's name short and the presence switch crowded it; the row now gives the name the room it needs.".into(),
        changes: vec![
            Change {
                area: "Sidebar".into(),
                title: "Owner row".into(),
                was: "The name was cut to \"Mani...\" next to the presence switch.".into(),
                now: "The full name shows; the switch sits after it with a smaller gap.".into(),
                before: "before-sidebar.png".into(),
                after: "after-sidebar.png".into(),
                pairs: vec![Pair {
                    label: "Phone".into(),
                    before: "before-sidebar-phone.png".into(),
                    after: "after-sidebar-phone.png".into(),
                }],
            },
        ],
        checks: vec![
            "`npm test`: 516 passed".into(),
            "`npx tsc --noEmit`: clean".into(),
        ],
        status: "Committed on the job's branch; it lands at the next landing pass.".into(),
        notes: vec!["The rail mode's tooltip now names the owner too.".into()],
    };
    serde_json::to_string_pretty(&m).unwrap_or_default()
}

/// The page for `m`, its pictures named relative to the page.
pub fn render(dir: &Path, m: &Manifest, built: &str) -> String {
    render_page(dir, Some(m), None, built)
}

/// The page: the agent's manifest when there is one, PULSE's facts when it
/// builds the page itself, and the pictures no pair names.
fn render_page(dir: &Path, m: Option<&Manifest>, f: Option<&Facts>, built: &str) -> String {
    let empty = Manifest::default();
    let mm = m.unwrap_or(&empty);
    let title = [mm.title.trim(), f.map_or("", |f| f.title.trim())]
        .into_iter()
        .find(|t| !t.is_empty())
        .unwrap_or("What changed")
        .to_string();
    let mut pairs = 0;
    let mut sections = String::new();
    let groups = grouped(&mm.changes);
    let named_groups = groups.iter().filter(|(area, _)| !area.is_empty()).count();
    let mut toc = Vec::new();
    for (gi, (area, items)) in groups.iter().enumerate() {
        let id = format!("area-{}", gi + 1);
        sections.push_str(&format!("  <section id=\"{id}\">\n"));
        if !area.is_empty() || named_groups > 0 {
            let heading = if area.is_empty() { "Other" } else { area };
            sections.push_str(&format!("    <h2>{}</h2>\n", esc(heading)));
            toc.push(format!("<a href=\"#{id}\">{}</a>", esc(heading)));
        }
        for (i, ch) in items {
            sections.push_str(&change_html(dir, *i, ch, &mut pairs));
        }
        sections.push_str("  </section>\n");
    }
    // PULSE's page shows every picture; the agent's own lists its pairs only.
    let loose = if f.is_some() {
        loose_pictures(dir, m)
    } else {
        Vec::new()
    };
    if !loose.is_empty() {
        let heading = if mm.changes.is_empty() {
            "Screenshots"
        } else {
            "More screenshots"
        };
        sections.push_str(&format!(
            "  <section id=\"pictures\">\n    <h2>{heading}</h2>\n    <div class=\"gallery\">\n{}    </div>\n  </section>\n",
            loose.iter().map(|name| shot_html(name)).collect::<String>()
        ));
        if !toc.is_empty() {
            toc.push(format!("<a href=\"#pictures\">{heading}</a>"));
        }
    }
    if let Some(f) = f
        && (!f.commits.is_empty() || !f.files.is_empty())
    {
        sections.push_str(&changed_html(f));
        if !toc.is_empty() {
            toc.push("<a href=\"#changed\">What changed</a>".to_string());
        }
    }
    let mut facts = Vec::new();
    if !mm.changes.is_empty() {
        facts.push(format!(
            "<li><b>{}</b> {}</li>",
            mm.changes.len(),
            if mm.changes.len() == 1 {
                "change"
            } else {
                "changes"
            }
        ));
        facts.push(format!(
            "<li><b>{pairs}</b> before/after {}</li>",
            if pairs == 1 { "pair" } else { "pairs" }
        ));
    } else if f.is_none() {
        facts.push("<li><b>0</b> changes</li>".to_string());
        facts.push(format!(
            "<li><b>{pairs}</b> before/after {}</li>",
            if pairs == 1 { "pair" } else { "pairs" }
        ));
    }
    if !loose.is_empty() {
        facts.push(format!(
            "<li><b>{}</b> {}</li>",
            loose.len(),
            if loose.len() == 1 {
                "screenshot"
            } else {
                "screenshots"
            }
        ));
    }
    let checks_all: Vec<&String> = mm
        .checks
        .iter()
        .chain(f.map(|f| f.checks.iter()).into_iter().flatten())
        .collect();
    if !checks_all.is_empty() {
        facts.push(format!(
            "<li><b>{}</b> {} run</li>",
            checks_all.len(),
            if checks_all.len() == 1 {
                "check"
            } else {
                "checks"
            }
        ));
    }
    if let Some(f) = f {
        if !f.commits.is_empty() {
            facts.push(format!(
                "<li><b>{}</b> {}</li>",
                f.commits.len(),
                if f.commits.len() == 1 {
                    "commit"
                } else {
                    "commits"
                }
            ));
        }
        if !f.files.is_empty() {
            facts.push(format!(
                "<li><b>{}</b> {}</li>",
                f.files.len(),
                if f.files.len() == 1 { "file" } else { "files" }
            ));
        }
        for (label, value) in &f.about {
            if !value.trim().is_empty() {
                facts.push(format!(
                    "<li>{} <b>{}</b></li>",
                    esc(label.trim()),
                    esc(value.trim())
                ));
            }
        }
    }
    let kicker = match f {
        Some(f) if !f.state.trim().is_empty() || !f.kind.trim().is_empty() => format!(
            "    <p class=\"kicker\">{}{}</p>\n",
            if f.state.trim().is_empty() {
                String::new()
            } else {
                format!(
                    "<span class=\"state {}\">{}</span>",
                    state_class(&f.state),
                    esc(f.state.trim())
                )
            },
            esc(f.kind.trim())
        ),
        _ => String::new(),
    };
    let summary_text = [mm.summary.trim(), f.map_or("", |f| f.summary.trim())]
        .into_iter()
        .find(|t| !t.is_empty())
        .unwrap_or("");
    let summary = if summary_text.is_empty() {
        String::new()
    } else {
        format!("    <p class=\"lede\">{}</p>\n", inline(summary_text))
    };
    let status = [f.map_or("", |f| f.status.trim()), mm.status.trim()]
        .into_iter()
        .filter(|t| !t.is_empty())
        .map(|t| format!("    <p class=\"note\">{}</p>\n", inline(t)))
        .collect::<String>();
    let checks = if checks_all.is_empty() {
        String::new()
    } else {
        format!(
            "    <ul class=\"checks\" aria-label=\"Checks\">\n{}    </ul>\n",
            checks_all
                .iter()
                .map(|c| format!("      <li>{}</li>\n", inline(c)))
                .collect::<String>()
        )
    };
    let links = match f {
        Some(f) if !f.links.is_empty() => format!(
            "    <p class=\"links\">{}</p>\n",
            f.links
                .iter()
                .map(|(label, url)| format!(
                    "<a href=\"{}\" rel=\"noreferrer\">{}</a>",
                    esc(url.trim()),
                    esc(label.trim())
                ))
                .collect::<Vec<_>>()
                .join(" · ")
        ),
        _ => String::new(),
    };
    let hint = if pairs > 0 || !loose.is_empty() {
        "    <p class=\"hint\">Open a picture to see it full size; go back to return here.</p>\n"
    } else {
        ""
    };
    let nav = if toc.len() > 1 {
        format!(
            "  <nav class=\"toc\" aria-label=\"Sections\">{}</nav>\n",
            toc.join("")
        )
    } else {
        String::new()
    };
    let mut notes = if mm.notes.is_empty() {
        String::new()
    } else {
        format!(
            "    <p>Also changed, not pictured:</p>\n    <ul>\n{}    </ul>\n",
            mm.notes
                .iter()
                .map(|n| format!("      <li>{}</li>\n", inline(n)))
                .collect::<String>()
        )
    };
    if let Some(f) = f
        && !f.notes.is_empty()
    {
        notes.push_str(&format!(
            "    <p>Also part of this work:</p>\n    <ul>\n{}    </ul>\n",
            f.notes
                .iter()
                .map(|n| format!("      <li>{}</li>\n", inline(n)))
                .collect::<String>()
        ));
    }
    let built = match (f, m.is_some()) {
        (Some(_), true) => format!(
            "Built by PULSE from the work's record and the agent's <code>report.json</code>{}.",
            when(built)
        ),
        (Some(_), false) => format!("Built by PULSE from the work's record{}.", when(built)),
        (None, _) => format!(
            "Built by <code>pulse report</code> from <code>report.json</code>{}.",
            if built.is_empty() {
                String::new()
            } else {
                format!(", written {}", esc(built))
            }
        ),
    };
    let generator = if f.is_some() { "pulse" } else { "pulse report" };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<meta name=\"generator\" content=\"{generator}\">\n<title>{title}</title>\n<style>\n{STYLE}</style>\n</head>\n<body>\n<main class=\"wrap\">\n  <header>\n{kicker}    <h1>{h1}</h1>\n{summary}    <ul class=\"facts\">{facts}</ul>\n{status}{checks}{links}{hint}  </header>\n{nav}{sections}  <footer>\n{notes}    <p>{built}</p>\n  </footer>\n</main>\n</body>\n</html>\n",
        title = esc(&title),
        h1 = esc(&title),
        facts = facts.join(""),
    )
}

/// ", 2026-11-07 09:56 UTC" for the foot, or nothing.
fn when(built: &str) -> String {
    if built.trim().is_empty() {
        String::new()
    } else {
        format!(", {}", esc(built.trim()))
    }
}

/// The colour a state word reads in.
fn state_class(state: &str) -> &'static str {
    match state.trim().to_ascii_lowercase().as_str() {
        "merged" | "landed" | "done" | "finished" => "ok",
        "failed" | "cancelled" | "refused" => "bad",
        "working" | "in review" | "queued" => "run",
        _ => "wait",
    }
}

/// One loose picture: the shot, captioned with its file name.
fn shot_html(name: &str) -> String {
    let href = encode_name(name);
    format!(
        "      <figure class=\"shot\"><a href=\"{href}\"><img src=\"{href}\" alt=\"{alt}\" loading=\"lazy\"></a><figcaption>{alt}</figcaption></figure>\n",
        alt = esc(name)
    )
}

/// How many file rows the page lists before it says how many more.
const FILES_SHOWN: usize = 80;

/// The work's commits and the files they changed.
fn changed_html(f: &Facts) -> String {
    let mut s = String::from("  <section id=\"changed\">\n    <h2>What changed</h2>\n    <div class=\"change\">\n");
    if !f.commits.is_empty() {
        s.push_str(&format!(
            "      <h3>{}</h3>\n      <ol class=\"commits\">\n{}      </ol>\n",
            if f.commits.len() == 1 {
                "1 commit".to_string()
            } else {
                format!("{} commits", f.commits.len())
            },
            f.commits
                .iter()
                .map(|(sha, subject)| format!(
                    "        <li><code>{}</code> {}</li>\n",
                    esc(sha),
                    esc(subject)
                ))
                .collect::<String>()
        ));
    }
    if !f.files.is_empty() {
        let (added, removed) = f.files.iter().fold((0u64, 0u64), |(a, r), x| {
            (a + x.added.unwrap_or(0), r + x.removed.unwrap_or(0))
        });
        s.push_str(&format!(
            "      <h3>{} {} <span class=\"sum\"><span class=\"add\">+{added}</span> <span class=\"del\">&minus;{removed}</span></span></h3>\n      <div class=\"files-wrap\"><table class=\"files\">\n        <thead><tr><th scope=\"col\">File</th><th scope=\"col\">Added</th><th scope=\"col\">Removed</th></tr></thead>\n        <tbody>\n",
            f.files.len(),
            if f.files.len() == 1 { "file" } else { "files" }
        ));
        for x in f.files.iter().take(FILES_SHOWN) {
            let (a, r) = match (x.added, x.removed) {
                (Some(a), Some(r)) => (format!("+{a}"), format!("&minus;{r}")),
                _ => ("binary".to_string(), String::new()),
            };
            s.push_str(&format!(
                "          <tr><td><code>{}</code></td><td class=\"add\">{a}</td><td class=\"del\">{r}</td></tr>\n",
                esc(&x.path)
            ));
        }
        s.push_str("        </tbody>\n      </table></div>\n");
        if f.files.len() > FILES_SHOWN {
            s.push_str(&format!(
                "      <p class=\"hint\">And {} more files.</p>\n",
                f.files.len() - FILES_SHOWN
            ));
        }
    }
    s.push_str("    </div>\n  </section>\n");
    s
}

/// The changes by area, areas in the order they first come.
fn grouped(changes: &[Change]) -> Vec<(String, Vec<(usize, &Change)>)> {
    let mut out: Vec<(String, Vec<(usize, &Change)>)> = Vec::new();
    for (i, ch) in changes.iter().enumerate() {
        let area = ch.area.trim().to_string();
        match out.iter_mut().find(|(a, _)| *a == area) {
            Some((_, items)) => items.push((i, ch)),
            None => out.push((area, vec![(i, ch)])),
        }
    }
    // Unnamed changes read last when other changes have an area.
    out.sort_by_key(|(a, _)| a.is_empty());
    out
}

fn change_html(dir: &Path, i: usize, ch: &Change, pairs: &mut usize) -> String {
    let title = if ch.title.trim().is_empty() {
        format!("Change {}", i + 1)
    } else {
        ch.title.trim().to_string()
    };
    let mut s = format!(
        "    <article class=\"change\" id=\"change-{}\">\n      <h3>{}</h3>\n",
        i + 1,
        esc(&title)
    );
    if !ch.was.trim().is_empty() || !ch.now.trim().is_empty() {
        s.push_str("      <dl class=\"delta\">\n");
        if !ch.was.trim().is_empty() {
            s.push_str(&format!(
                "        <div class=\"was\"><dt>Was</dt><dd>{}</dd></div>\n",
                inline(&ch.was)
            ));
        }
        if !ch.now.trim().is_empty() {
            s.push_str(&format!(
                "        <div class=\"now\"><dt>Now</dt><dd>{}</dd></div>\n",
                inline(&ch.now)
            ));
        }
        s.push_str("      </dl>\n");
    }
    for p in ch.all_pairs() {
        let st = PairState::of(dir, &p);
        if st.complete() {
            *pairs += 1;
        }
        if !p.label.trim().is_empty() {
            s.push_str(&format!("      <h4>{}</h4>\n", esc(p.label.trim())));
        }
        s.push_str("      <div class=\"pair\">\n");
        s.push_str(&figure("Before", "before", &title, &p.before, &st.before));
        s.push_str(&figure("After", "after", &title, &p.after, &st.after));
        s.push_str("      </div>\n");
        if st.same {
            s.push_str("      <p class=\"same\">The two pictures are the same: this screen looks as it did.</p>\n");
        }
    }
    s.push_str("    </article>\n");
    s
}

fn figure(
    caption: &str,
    class: &str,
    title: &str,
    name: &str,
    found: &Result<(), String>,
) -> String {
    let body = match found {
        Ok(()) => {
            let href = encode_name(name.trim());
            format!(
                "<a href=\"{href}\"><img src=\"{href}\" alt=\"{caption}: {}\" loading=\"lazy\"></a>",
                esc(title)
            )
        }
        Err(why) => format!("<div class=\"missing\">{}</div>", inline(why)),
    };
    format!("        <figure class=\"{class}\"><figcaption>{caption}</figcaption>{body}</figure>\n")
}

/// HTML-escaped text with `code` spans set as code.
fn inline(text: &str) -> String {
    let escaped = esc(text.trim());
    let parts: Vec<&str> = escaped.split('`').collect();
    if parts.len() < 3 {
        return escaped;
    }
    let mut out = String::new();
    let last_open = parts.len().is_multiple_of(2); // an odd backtick count leaves one unpaired
    for (i, part) in parts.iter().enumerate() {
        let in_code = i % 2 == 1;
        if in_code && !(last_open && i == parts.len() - 1) {
            out.push_str("<code>");
            out.push_str(part);
            out.push_str("</code>");
        } else {
            if in_code {
                out.push('`');
            }
            out.push_str(part);
        }
    }
    out
}

fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// A file name as a relative URL: everything but the unreserved characters
/// percent-encoded, so a space or a `#` in a name still finds the file.
fn encode_name(name: &str) -> String {
    let mut out = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The page's look: the console's neutral canvas, ink text and teal for
/// links, with the "was" and "now" colours of a before/after review; light
/// or dark with the viewer's system, unless the page that frames it sets
/// `data-theme` on the root.
const STYLE: &str = r#":root {
  --bg: oklch(0.985 0.002 255);
  --card: #fff;
  --ink: oklch(0.26 0.04 262);
  --muted: oklch(0.48 0.02 262);
  --line: oklch(0.92 0.004 255);
  --teal: oklch(0.52 0.095 186);
  --was: oklch(0.55 0.16 30);
  --now: oklch(0.5 0.12 155);
  --shade: oklch(0.96 0.004 255);
  --warn: oklch(0.58 0.13 70);
  --font: ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif;
  --mono: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
  color-scheme: light;
}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {
    --bg: oklch(0.17 0.01 260);
    --card: oklch(0.21 0.01 260);
    --ink: oklch(0.94 0.005 260);
    --muted: oklch(0.72 0.01 260);
    --line: oklch(0.32 0.01 260);
    --teal: oklch(0.78 0.1 186);
    --was: oklch(0.75 0.13 30);
    --now: oklch(0.78 0.12 155);
    --shade: oklch(0.25 0.01 260);
    --warn: oklch(0.8 0.12 75);
    color-scheme: dark;
  }
}
:root[data-theme="dark"] {
  --bg: oklch(0.17 0.01 260);
  --card: oklch(0.21 0.01 260);
  --ink: oklch(0.94 0.005 260);
  --muted: oklch(0.72 0.01 260);
  --line: oklch(0.32 0.01 260);
  --teal: oklch(0.78 0.1 186);
  --was: oklch(0.75 0.13 30);
  --now: oklch(0.78 0.12 155);
  --shade: oklch(0.25 0.01 260);
  --warn: oklch(0.8 0.12 75);
  color-scheme: dark;
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--ink); font: 15px/1.55 var(--font); padding: 40px 16px 64px; -webkit-font-smoothing: antialiased; }
.wrap { max-width: 1240px; margin: 0 auto; display: grid; gap: 40px; }
header { display: grid; gap: 10px; max-width: 76ch; }
h1 { font-size: 30px; line-height: 1.15; font-weight: 600; letter-spacing: -0.02em; margin: 0; text-wrap: balance; }
.lede { margin: 0; color: var(--muted); font-size: 16px; }
.facts { display: flex; flex-wrap: wrap; gap: 8px; margin: 6px 0 0; padding: 0; list-style: none; }
.facts li { font-size: 13px; color: var(--muted); background: var(--card); border: 1px solid var(--line); border-radius: 6px; padding: 3px 9px; }
.facts b { color: var(--ink); font-weight: 600; font-variant-numeric: tabular-nums; }
.checks { margin: 4px 0 0; padding-left: 18px; font-size: 14px; color: var(--muted); }
.note { font-size: 14px; color: var(--muted); border-left: 2px solid var(--teal); padding: 2px 0 2px 12px; margin: 4px 0 0; }
.hint { font-size: 13px; color: var(--muted); margin: 0; }
code { font-family: var(--mono); font-size: 0.88em; background: var(--shade); border-radius: 4px; padding: 1px 5px; overflow-wrap: anywhere; }
nav.toc { display: flex; flex-wrap: wrap; gap: 6px 14px; font-size: 14px; }
a { color: var(--teal); }
nav.toc a { text-decoration: none; }
nav.toc a:hover { text-decoration: underline; }
section { display: grid; gap: 20px; scroll-margin-top: 16px; }
h2 { font-size: 13px; font-weight: 600; letter-spacing: 0.06em; text-transform: uppercase; color: var(--muted); margin: 0; padding-bottom: 8px; border-bottom: 1px solid var(--line); }
.change { background: var(--card); border: 1px solid var(--line); border-radius: 14px; padding: 20px; display: grid; gap: 14px; min-width: 0; }
h3 { font-size: 18px; font-weight: 600; letter-spacing: -0.01em; margin: 0; text-wrap: balance; }
h4 { font-size: 12px; font-weight: 600; letter-spacing: 0.05em; text-transform: uppercase; color: var(--muted); margin: 4px 0 -6px; }
.delta { margin: 0; display: grid; gap: 6px; max-width: 80ch; }
.delta div { display: grid; grid-template-columns: 44px minmax(0, 1fr); gap: 10px; align-items: baseline; }
.delta dt { font-size: 11px; font-weight: 600; letter-spacing: 0.06em; text-transform: uppercase; }
.delta .was dt { color: var(--was); }
.delta .now dt { color: var(--now); }
.delta dd { margin: 0; min-width: 0; }
.pair { display: grid; grid-template-columns: minmax(0, 1fr); gap: 12px; align-items: start; }
@media (min-width: 900px) { .pair { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
figure { margin: 0; min-width: 0; display: grid; gap: 6px; justify-items: start; }
figcaption { font-size: 12px; font-weight: 600; letter-spacing: 0.05em; text-transform: uppercase; color: var(--was); }
.after figcaption { color: var(--now); }
figure a { display: block; max-width: 100%; }
figure img { display: block; max-width: 100%; height: auto; border: 1px solid var(--line); border-radius: 10px; background: var(--shade); }
.missing { width: 100%; min-height: 120px; display: grid; place-items: center; text-align: center; padding: 16px; font-size: 13px; color: var(--was); border: 1px dashed var(--was); border-radius: 10px; background: var(--shade); }
.same { font-size: 13px; color: var(--muted); margin: 0; }
footer { font-size: 13px; color: var(--muted); display: grid; gap: 6px; max-width: 80ch; }
footer p { margin: 0; }
footer ul { margin: 0; padding-left: 18px; }
a:focus-visible { outline: 2px solid var(--teal); outline-offset: 2px; border-radius: 3px; }
.kicker { margin: 0; display: flex; flex-wrap: wrap; align-items: center; gap: 8px; font-size: 13px; color: var(--muted); }
.state { font-weight: 600; font-size: 12px; letter-spacing: 0.04em; text-transform: uppercase; border: 1px solid currentColor; border-radius: 999px; padding: 1px 9px; }
.state.ok { color: var(--now); }
.state.bad { color: var(--was); }
.state.run { color: var(--teal); }
.state.wait { color: var(--warn); }
.links { margin: 2px 0 0; font-size: 14px; overflow-wrap: anywhere; }
.gallery { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(100%, 260px), 1fr)); gap: 16px; }
.shot figcaption { font-size: 12px; font-weight: 500; letter-spacing: 0; text-transform: none; color: var(--muted); overflow-wrap: anywhere; }
.commits { margin: 0; padding-left: 20px; display: grid; gap: 4px; font-size: 14px; }
.commits code { margin-right: 4px; }
.sum { font-size: 13px; font-weight: 500; margin-left: 6px; }
.add { color: var(--now); }
.del { color: var(--was); }
.files-wrap { overflow-x: auto; }
.files { width: 100%; border-collapse: collapse; font-size: 13px; }
.files th { text-align: left; font-weight: 600; color: var(--muted); padding: 4px 8px; border-bottom: 1px solid var(--line); }
.files td { padding: 4px 8px; border-bottom: 1px solid var(--line); vertical-align: top; }
.files td:first-child { width: 100%; }
.files td code { background: none; padding: 0; word-break: break-all; }
.files td.add, .files td.del, .files th:not(:first-child) { text-align: right; white-space: nowrap; font-variant-numeric: tabular-nums; }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
    const PNG2: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR-other";

    fn manifest(dir: &Path, json: &str) {
        std::fs::write(dir.join(MANIFEST), json).unwrap();
    }

    #[test]
    fn a_report_shows_a_change_when_one_pair_has_both_pictures() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // No manifest: nothing shown, and the agent is told what to write.
        let c = check(dir);
        assert!(!c.manifest && !c.shows_change());
        assert!(c.problems[0].contains("pulse report --example"), "{c:?}");

        std::fs::write(dir.join("before.png"), PNG).unwrap();
        std::fs::write(dir.join("after.png"), PNG2).unwrap();
        std::fs::write(dir.join("fake.png"), b"not a picture").unwrap();
        manifest(
            dir,
            r#"{"title":"Owner row","changes":[
                {"area":"Sidebar","title":"Name","was":"cut","now":"whole","before":"before.png","after":"after.png",
                 "pairs":[{"label":"Phone","before":"before.png","after":"missing.png"}]},
                {"title":"Switch","was":"crowded","now":"spaced","before":"shots/a.png","after":"fake.png"}
            ]}"#,
        );
        let c = check(dir);
        assert!(c.manifest && !c.page);
        assert_eq!((c.changes, c.pairs), (2, 1));
        assert!(c.shows_change() && !c.clean());
        assert_eq!(c.counts(), "2 changes, 1 before/after pair");
        let all = c.problems.join("\n");
        assert!(
            all.contains("change 1 \"Name\" (Phone), after: `missing.png` is not in the folder"),
            "{all}"
        );
        assert!(
            all.contains(
                "change 2 \"Switch\", before: `shots/a.png` is not a file name in the folder"
            ),
            "{all}"
        );
        assert!(all.contains("`fake.png` is not a picture"), "{all}");
    }

    #[test]
    fn a_manifest_with_a_stray_field_says_which() {
        let tmp = tempfile::tempdir().unwrap();
        manifest(tmp.path(), r#"{"title":"x","screens":[]}"#);
        let c = check(tmp.path());
        assert!(c.manifest && !c.shows_change());
        assert!(c.problems[0].contains("unknown field `screens`"), "{c:?}");
        // Nothing is built from a manifest that does not read.
        let c = build(tmp.path());
        assert!(!c.page && !tmp.path().join(PAGE).exists());
    }

    #[test]
    fn the_page_groups_by_area_escapes_text_and_names_pictures_relatively() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("a b.png"), PNG).unwrap();
        std::fs::write(dir.join("same.png"), PNG).unwrap();
        std::fs::write(dir.join("after.webp"), b"RIFF\0\0\0\0WEBPVP8 ").unwrap();
        manifest(
            dir,
            r#"{"title":"Fix <b>","summary":"Uses `useNow` & more","checks":["`npm test`: 3 passed"],
                "status":"On the job's branch","notes":["tooltip"],
                "changes":[
                  {"title":"Unnamed","was":"w","now":"n","before":"same.png","after":"same.png"},
                  {"area":"Sidebar","title":"Row","was":"w","now":"n","before":"a b.png","after":"after.webp"}
                ]}"#,
        );
        let c = build(dir);
        assert!(c.page && c.clean(), "{c:?}");
        let html = std::fs::read_to_string(dir.join(PAGE)).unwrap();
        assert!(html.contains("<title>Fix &lt;b&gt;</title>"), "{html}");
        assert!(html.contains("Uses <code>useNow</code> &amp; more"));
        assert!(html.contains("<code>npm test</code>: 3 passed"));
        assert!(html.contains("src=\"a%20b.png\""));
        assert!(html.contains("<b>2</b> before/after pairs"));
        // The named area comes first; the unnamed change reads under Other.
        let sidebar = html.find("<h2>Sidebar</h2>").unwrap();
        let other = html.find("<h2>Other</h2>").unwrap();
        assert!(sidebar < other);
        assert!(html.contains("The two pictures are the same"));
        assert!(html.contains("Also changed, not pictured"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn ensure_page_builds_a_missing_or_stale_page_only() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("b.png"), PNG).unwrap();
        std::fs::write(dir.join("a.png"), PNG2).unwrap();
        manifest(
            dir,
            r#"{"title":"t","changes":[{"title":"c","was":"w","now":"n","before":"b.png","after":"a.png"}]}"#,
        );
        let c = ensure_page(dir);
        assert!(c.page && c.shows_change());
        // A page newer than its manifest is the agent's to keep.
        std::fs::write(dir.join(PAGE), "kept").unwrap();
        ensure_page(dir);
        assert_eq!(std::fs::read_to_string(dir.join(PAGE)).unwrap(), "kept");
        // A manifest written after the page builds it again.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .append(true)
            .open(dir.join(MANIFEST))
            .unwrap()
            .set_modified(later)
            .unwrap();
        ensure_page(dir);
        assert!(
            std::fs::read_to_string(dir.join(PAGE))
                .unwrap()
                .starts_with("<!doctype html>")
        );
    }

    fn facts() -> Facts {
        Facts {
            kind: "Task TASK-083".into(),
            title: "Runs record token cost".into(),
            summary: "Runs record `tokens` and cost.".into(),
            state: "Merged".into(),
            status: "Merged into main as pull request #125.".into(),
            about: vec![("Agent".into(), "borealis-28".into()), ("Cost".into(), String::new())],
            links: vec![("Pull request #125".into(), "https://github.com/o/r/pull/125".into())],
            commits: vec![("a4c067b".into(), "runner: runs record <cost>".into())],
            files: vec![
                FileChange { path: "crates/run.rs".into(), added: Some(12), removed: Some(3) },
                FileChange { path: "web/shot.png".into(), added: None, removed: None },
            ],
            checks: vec!["Verified: `cargo test`".into()],
            notes: vec!["RSI B: stopped by a restart".into()],
            built: "2026-11-07 10:00 UTC".into(),
        }
    }

    #[test]
    fn pulse_builds_a_page_for_work_without_a_report_of_its_own() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("artifacts");
        // No folder yet: the page makes it.
        let c = build_full(&dir, &facts());
        assert!(c.page, "{c:?}");
        let html = std::fs::read_to_string(dir.join(PAGE)).unwrap();
        assert!(html.contains("<title>Runs record token cost</title>"), "{html}");
        assert!(html.contains("<span class=\"state ok\">Merged</span>Task TASK-083"));
        assert!(html.contains("Runs record <code>tokens</code> and cost."));
        assert!(html.contains("<li>Agent <b>borealis-28</b></li>"));
        // An empty fact is left out rather than shown blank.
        assert!(!html.contains("Cost <b>"));
        assert!(html.contains("<li><b>1</b> commit</li><li><b>2</b> files</li>"));
        assert!(html.contains("<code>a4c067b</code> runner: runs record &lt;cost&gt;"));
        assert!(html.contains("<span class=\"add\">+12</span> <span class=\"del\">&minus;3</span>"));
        assert!(html.contains("<td class=\"add\">binary</td>"));
        assert!(html.contains("href=\"https://github.com/o/r/pull/125\""));
        assert!(html.contains("Also part of this work:"));
        assert!(html.contains("Built by PULSE from the work's record, 2026-11-07 10:00 UTC."));
        // Nothing before/after to count, and no picture hint without pictures.
        assert!(!html.contains("before/after pair"));
        assert!(!html.contains("Open a picture"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn pulse_s_page_keeps_the_agent_s_pairs_and_shows_every_other_picture() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("before.png"), PNG).unwrap();
        std::fs::write(dir.join("after.png"), PNG2).unwrap();
        std::fs::write(dir.join("extra shot.png"), PNG).unwrap();
        std::fs::write(dir.join("notes.txt"), b"not a picture").unwrap();
        manifest(
            dir,
            r#"{"title":"Owner row","summary":"The row fits.","status":"On the branch.","checks":["`npm test`: 3 passed"],
                "changes":[{"title":"Name","was":"cut","now":"whole","before":"before.png","after":"after.png"}]}"#,
        );
        assert_eq!(loose_pictures(dir, read_manifest(dir).unwrap().as_ref()), vec!["extra shot.png"]);
        let c = build_full(dir, &facts());
        assert!(c.page && c.shows_change(), "{c:?}");
        let html = std::fs::read_to_string(dir.join(PAGE)).unwrap();
        // The agent's title and summary lead; PULSE's status reads first.
        assert!(html.contains("<title>Owner row</title>"));
        assert!(html.contains("The row fits."));
        let pulse = html.find("Merged into main as pull request #125.").unwrap();
        let agent = html.find("On the branch.").unwrap();
        assert!(pulse < agent);
        assert!(html.contains("<li><b>1</b> before/after pair</li><li><b>1</b> screenshot</li><li><b>2</b> checks run</li>"));
        assert!(html.contains("<h2>More screenshots</h2>"));
        assert!(html.contains("src=\"extra%20shot.png\""));
        assert!(!html.contains("notes.txt"));
        assert!(html.contains("and the agent's <code>report.json</code>"));
        assert!(html.contains("<a href=\"#pictures\">More screenshots</a>") || !html.contains("nav class"));
    }

    #[test]
    fn the_example_reads_back_and_inline_code_pairs_backticks() {
        let m: Manifest = serde_json::from_str(&example()).unwrap();
        assert_eq!(m.changes[0].all_pairs().len(), 2);
        assert_eq!(inline("a `b` c"), "a <code>b</code> c");
        assert_eq!(inline("a `b` c `d"), "a <code>b</code> c `d");
        assert_eq!(inline("no code"), "no code");
        assert_eq!(encode_name("x#1 é.png"), "x%231%20%C3%A9.png");
    }
}
