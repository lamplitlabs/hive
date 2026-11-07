//! `pulse report`: builds the before/after page (report.rs) in an agent's
//! artifacts folder from the `report.json` it wrote there, and says what is
//! still missing. A worker or a session's agent runs it when its change has
//! a face; it needs no store, so it runs on a remote hand too.
use crate::artifacts::ARTIFACTS_ENV;
use crate::report::{self, MANIFEST, PAGE};
use anyhow::Result;
use pulse_cli::{Command, Context, FlagSet, errorf, register as cli_register, write_json};
use serde_json::json;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Once;

static REGISTER: Once = Once::new();

const USAGE: &str = "pulse report [--dir DIR] [--check] [--json]\n       pulse report --example";

pub fn register() {
    REGISTER.call_once(|| {
        cli_register(Command {
            name: "report".into(),
            group: "runner".into(),
            summary: "build the before/after page (report.html) from report.json and the screenshots in the artifacts folder".into(),
            usage: USAGE.into(),
            aliases: vec![],
            no_store: true,
            read_only: false,
            raw: false,
            run,
            flags: Some(flags),
        });
    });
}

fn flags(fs: &mut FlagSet) {
    fs.string_var(
        "dir",
        "",
        &format!("the folder that holds {MANIFEST} and the pictures (default: ${ARTIFACTS_ENV})"),
    );
    fs.bool_var(
        "check",
        false,
        &format!("say what the report shows without writing {PAGE}"),
    );
    fs.bool_var("example", false, &format!("print an example {MANIFEST}"));
}

fn run(c: &mut Context, fs: &FlagSet, args: &[String]) -> Result<()> {
    if !args.is_empty() {
        return Err(errorf(
            2,
            &format!("pulse report: unexpected arguments: {}", args.join(" ")),
            "",
            USAGE,
        )
        .into());
    }
    if fs.get_bool("example") {
        writeln!(c.out, "{}", report::example())?;
        return Ok(());
    }
    let mut dir = fs.get_string("dir");
    if dir.is_empty() {
        dir = std::env::var(ARTIFACTS_ENV).unwrap_or_default();
    }
    if dir.trim().is_empty() {
        return Err(errorf(
            2,
            "pulse report: which folder?",
            &format!(
                "${ARTIFACTS_ENV} is not set here: this machine has no folder the hub serves."
            ),
            "pulse report --dir <folder with report.json>",
        )
        .into());
    }
    let dir = PathBuf::from(dir.trim());
    if !dir.is_dir() {
        return Err(errorf(
            2,
            &format!("pulse report: {} is not a folder", dir.display()),
            "",
            "pulse report --dir <folder with report.json>",
        )
        .into());
    }
    let checked = if fs.get_bool("check") {
        report::check(&dir)
    } else {
        report::build(&dir)
    };
    let page = dir.join(PAGE);
    if c.json {
        write_json(
            c.out.as_mut(),
            &json!({
                "dir": dir.display().to_string(),
                "page": if checked.page { page.display().to_string() } else { String::new() },
                "title": checked.title,
                "changes": checked.changes,
                "pairs": checked.pairs,
                "shows_change": checked.shows_change(),
                "problems": checked.problems,
            }),
        )?;
    } else {
        if checked.manifest {
            let verb = if fs.get_bool("check") {
                "shows"
            } else if checked.page {
                "written:"
            } else {
                "not written:"
            };
            writeln!(c.out, "{PAGE} {verb} {}", checked.counts())?;
            if checked.page && !fs.get_bool("check") {
                writeln!(c.out, "  {}", page.display())?;
            }
        }
        if !checked.problems.is_empty() {
            writeln!(c.out, "to fix:")?;
            for p in &checked.problems {
                writeln!(c.out, "  - {p}")?;
            }
        }
    }
    if checked.clean() {
        return Ok(());
    }
    let what = if checked.shows_change() {
        format!(
            "pulse report: {} to fix",
            if checked.problems.len() == 1 {
                "1 problem".to_string()
            } else {
                format!("{} problems", checked.problems.len())
            }
        )
    } else {
        "pulse report: no change is shown before and after yet".to_string()
    };
    Err(errorf(
        1,
        &what,
        "A report shows each visible change as a picture before it and one after it, both saved in the artifacts folder.",
        &format!("edit {MANIFEST} and run pulse report again (pulse report --example prints its shape)"),
    )
    .into())
}
