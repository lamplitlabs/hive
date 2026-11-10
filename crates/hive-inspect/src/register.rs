use anyhow::Result;
use hive_cli::flags::FlagSet;
use hive_cli::{Command, Context};
use hive_store::{COMMONS_SLUG, HUMAN_AGENT, Project, Store};
use std::sync::Once;

static REGISTER: Once = Once::new();

pub fn register() {
    REGISTER.call_once(|| {
        for cmd in [
            Command {
                name: "status".into(),
                group: "inspect".into(),
                summary: "one screen: store, projects, agents, jobs, runner".into(),
                usage: "hive status [--json]".into(),
                aliases: vec![],
                no_store: false,
                read_only: true,
                raw: false,
                run: crate::status::run_status,
                flags: None,
            },
            Command {
                name: "projects".into(),
                group: "inspect".into(),
                summary: "registered projects, including inactive projects".into(),
                usage: "hive projects [--json]\n       (to deactivate one: hive remove project <name>)".into(),
                aliases: vec![],
                no_store: false,
                read_only: true,
                raw: false,
                run: crate::lists::run_projects,
                flags: None,
            },
            Command {
                name: "jobs".into(),
                group: "inspect".into(),
                summary: "the runner's queue".into(),
                usage: "hive jobs [--project P] [--all] [--json]".into(),
                aliases: vec![],
                no_store: false,
                read_only: true,
                raw: false,
                run: crate::lists::run_jobs,
                flags: Some(all_flag),
            },
            Command {
                name: "agents".into(),
                group: "inspect".into(),
                summary: "agent scorecards (--all includes pruned agents); list, lint, schema: the custom agent definitions".into(),
                usage: format!(
                    "hive agents [--all] [--json]\n       {}",
                    hive_harness::agents_command::USAGE
                ),
                aliases: vec![],
                no_store: true,
                read_only: true,
                raw: false,
                run: crate::lists::run_agents,
                flags: Some(agents_flags),
            },
            Command {
                name: "events".into(),
                group: "inspect".into(),
                summary: "the append-only event log, newest first".into(),
                usage: "hive events [--limit N] [--project P] [--since SEQ|TIME] [--json]".into(),
                aliases: vec![],
                no_store: false,
                read_only: true,
                raw: false,
                run: crate::lists::run_events,
                flags: Some(events_flags),
            },
            Command {
                name: "settings".into(),
                group: "human".into(),
                summary: "read or change a setting".into(),
                usage: crate::settings::settings_usage(),
                aliases: vec![],
                no_store: true,
                read_only: false,
                raw: false,
                run: crate::settings::run_settings,
                flags: None,
            },
            Command {
                name: "schema".into(),
                group: "inspect".into(),
                summary: "print the schema".into(),
                usage: "hive schema [--json]".into(),
                aliases: vec![],
                no_store: true,
                read_only: false,
                raw: false,
                run: run_schema,
                flags: None,
            },
            Command {
                name: "verify".into(),
                group: "inspect".into(),
                summary: "verify a commit's signature and trailers against its run".into(),
                usage: "hive verify <ref> [--project P] [--json]".into(),
                aliases: vec![],
                no_store: false,
                read_only: true,
                raw: false,
                run: crate::verify::run_verify,
                flags: None,
            },
            Command {
                name: "signers".into(),
                group: "inspect".into(),
                summary: "an allowed-signers file for every run's key".into(),
                usage: "hive signers [--project P] [--json]".into(),
                aliases: vec![],
                no_store: false,
                read_only: true,
                raw: false,
                run: crate::verify::run_signers,
                flags: None,
            },
            Command {
                name: "stop".into(),
                group: "human".into(),
                summary: "stop the runner using the store's kill switch".into(),
                usage: "hive stop".into(),
                aliases: vec![],
                no_store: false,
                read_only: false,
                raw: false,
                run: run_stop,
                flags: None,
            },
            Command {
                name: "start".into(),
                group: "human".into(),
                summary: "let the runner run again".into(),
                usage: "hive start".into(),
                aliases: vec![],
                no_store: false,
                read_only: false,
                raw: false,
                run: run_start,
                flags: None,
            },
        ] {
            hive_cli::register(cmd);
        }
    });
}

pub fn all_flag(fs: &mut FlagSet) {
    fs.bool_var("all", false, "include finished jobs or pruned agents");
}

fn agents_flags(fs: &mut FlagSet) {
    all_flag(fs);
    hive_harness::agents_command::flags(fs);
}

fn events_flags(fs: &mut FlagSet) {
    fs.int_var("limit", 40, "maximum events; a negative value means all");
    fs.string_var(
        "since",
        "",
        "events after this sequence number or UTC timestamp",
    );
}

pub(crate) fn bool_flag(fs: &FlagSet, name: &str) -> bool {
    fs.get_bool(name)
}

pub(crate) fn string_flag(fs: &FlagSet, name: &str) -> String {
    fs.get_string(name)
}

pub(crate) fn no_args(args: &[String]) -> Result<()> {
    if !args.is_empty() {
        return Err(hive_cli::errorf(
            2,
            &format!("hive: unrecognized arguments: {}", args.join(" ")),
            "",
            "",
        )
        .into());
    }
    Ok(())
}

pub(crate) fn store(c: &Context) -> Result<&Store> {
    c.store
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("store is not open"))
}

pub(crate) fn actor(c: &Context) -> String {
    if !c.agent.is_empty() {
        return c.agent.clone();
    }
    if let Ok(a) = std::env::var("HIVE_AGENT")
        && !a.is_empty()
    {
        return a;
    }
    HUMAN_AGENT.to_string()
}

pub(crate) fn input_error(err: anyhow::Error) -> anyhow::Error {
    if let Some(input) = err.downcast_ref::<hive_store::InputError>() {
        hive_cli::errorf(2, &format!("hive: {input}"), "", "").into()
    } else {
        err
    }
}

pub(crate) fn json_output<T: serde::Serialize>(c: &mut Context, value: &T) -> Result<()> {
    if c.quiet {
        return Ok(());
    }
    hive_cli::write_json(&mut c.out, value)
}

fn run_schema(c: &mut Context, _fs: &FlagSet, args: &[String]) -> Result<()> {
    no_args(args)?;
    if c.json {
        #[derive(serde::Serialize)]
        struct Schema<'a> {
            schema: &'a str,
        }
        return json_output(
            c,
            &Schema {
                schema: hive_store::DDL,
            },
        );
    }
    c.printf(format_args!("{}", hive_store::DDL))?;
    Ok(())
}

fn run_stop(c: &mut Context, _fs: &FlagSet, args: &[String]) -> Result<()> {
    set_stopped(c, args, true)
}

fn run_start(c: &mut Context, _fs: &FlagSet, args: &[String]) -> Result<()> {
    set_stopped(c, args, false)
}

fn set_stopped(c: &mut Context, args: &[String], stopped: bool) -> Result<()> {
    no_args(args)?;
    let (value, state) = if stopped {
        ("1", "stopped")
    } else {
        ("0", "allowed")
    };
    store(c)?
        .set_setting(&actor(c), "stop", value)
        .map_err(input_error)?;
    if c.json {
        #[derive(serde::Serialize)]
        struct Stopped {
            stopped: bool,
        }
        json_output(c, &Stopped { stopped })
    } else {
        c.printf(format_args!("runner {state}\n"))?;
        Ok(())
    }
}

pub(crate) fn trunc(text: &str, n: usize) -> String {
    let mut chars = text.chars();
    let taken: String = chars.by_ref().take(n).collect();
    if chars.next().is_some() {
        taken.chars().take(n.saturating_sub(1)).collect::<String>() + "~"
    } else {
        text.to_string()
    }
}

pub(crate) fn or_dash(text: &str) -> &str {
    if text.is_empty() { "-" } else { text }
}

pub(crate) fn current_project(c: &Context) -> Result<Project> {
    let mut explicit = c.project.clone();
    if explicit.is_empty() {
        explicit = std::env::var("HIVE_PROJECT").unwrap_or_default();
    }
    if explicit == COMMONS_SLUG {
        return Ok(Project {
            slug: explicit,
            name: "the commons".into(),
            ..Default::default()
        });
    }
    let s = store(c)?;
    if !explicit.is_empty() {
        if let Some(p) = s.project(&explicit)? {
            return Ok(p);
        }
        return Err(hive_cli::errorf(
            2,
            &format!(
                "hive: No project {} (hive projects lists them)",
                py_repr(&explicit)
            ),
            "",
            "",
        )
        .into());
    }
    let dir = std::env::current_dir()?.display().to_string();
    if let Some(p) = s.project_for_dir(&dir)? {
        return Ok(p);
    }
    Err(hive_cli::errorf(
        2,
        "hive: The current directory is not inside a registered project; pass --project SLUG, set HIVE_PROJECT, or run `hive init` here",
        "",
        "",
    )
    .into())
}

pub(crate) fn py_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::new();
    out.push(quote);
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

#[cfg(test)]
mod tests {
    use crate::test_helpers::*;

    #[test]
    fn schema_without_store() {
        let got = invoke(None, false, "", "schema", &[]);
        assert_eq!(got.code, 0);
        assert_eq!(got.out, hive_store::DDL);
    }

    #[test]
    fn registration_and_json() {
        let (_dir, s) = temporary_store();
        crate::register();
        for name in [
            "status", "projects", "jobs", "agents", "events", "schema", "signers",
        ] {
            let cmd = hive_cli::lookup(name).unwrap();
            assert_eq!(cmd.group, "inspect");
            assert!(cmd.read_only || cmd.no_store);
            let got = invoke(Some(s.clone()), true, "", name, &[]);
            assert_eq!(got.code, 0, "{name}: {got:?}");
            serde_json::from_str::<serde_json::Value>(&got.out).unwrap();
        }
        for name in ["settings", "start", "stop"] {
            let cmd = hive_cli::lookup(name).unwrap();
            assert_eq!(cmd.group, "human");
        }
    }

    #[test]
    fn projects_help_names_the_removal_path() {
        crate::register();
        let cmd = hive_cli::lookup("projects").unwrap();
        assert!(
            cmd.usage.contains("hive remove project <name>"),
            "hive projects --help must point at the removal command: {}",
            cmd.usage
        );
    }

    #[test]
    fn stop_start_round_trip() {
        let (_dir, s) = temporary_store();
        exec_sql(
            &s,
            "INSERT INTO meta(key,value) VALUES ('daemon','{\"pid\":123,\"note\":\"exited\",\"at\":\"2026-09-06T18:00:00Z\"}')",
        );
        let (daemon_before, _) = s.meta("daemon").unwrap();
        for (name, value, output) in [
            ("stop", "1", "runner stopped\n"),
            ("start", "0", "runner allowed\n"),
        ] {
            let before = event_count(&s);
            let got = invoke(Some(s.clone()), false, "", name, &[]);
            assert_eq!(got.code, 0);
            assert_eq!(got.out, output);
            assert_eq!(s.setting("stop", ""), value);
            check_setting_event(&s, "stop", value, "human");
            assert_eq!(event_count(&s), before + 1);
            let reason = crate::status::runner_stopped(&s).unwrap();
            assert_eq!(!reason.is_empty(), value == "1");
        }
        let (daemon_after, _) = s.meta("daemon").unwrap();
        assert_eq!(daemon_after, daemon_before);
        let got = invoke_agent(Some(s.clone()), "inspect-agent", false, "", "stop", &[]);
        assert_eq!(got.code, 0, "{got:?}");
        check_setting_event(&s, "stop", "1", "inspect-agent");
    }

    #[test]
    fn status_uses_actual_store_path() {
        let (_dir, s) = temporary_store();
        let got = invoke(Some(s.clone()), false, "", "status", &[]);
        assert_eq!(got.code, 0, "{got:?}");
        assert!(
            got.out
                .starts_with(&format!("store      {} (schema 3)\n", s.path))
        );
    }

    #[test]
    fn trunc_unicode() {
        let text = "界".repeat(10);
        assert_eq!(super::trunc(&text, 5), format!("{}~", "界".repeat(4)));
    }
}
