#![allow(dead_code)]

use anyhow::{Result, anyhow};
use hive_cli::{Command, Context, FlagSet, register as cli_register};
use serde_json::json;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::LazyLock;
use std::sync::Once;
use std::sync::{Arc, Mutex};

static REGISTER: Once = Once::new();

pub fn register() {
    REGISTER.call_once(|| {
        // The crates this one builds on register first; the runner's own
        // side effects (compute providers, the command table) follow.
        hive_bus::install_store_observer();
        hive_board::register();
        hive_tasks::register();
        hive_evolve::register();
        hive_evolve::init();
        hive_tally::register();
        hive_records::register();
        hive_hooks::register();
        hive_provider_copilot::register();
        hive_provider_openai::register();
        hive_provider_anthropic::register();
        hive_provider_gemini::register();
        crate::compute::init();
        crate::aws::init();
        crate::azure::init();
        crate::gcp::init();
        crate::multipass::init();
        hive_board::set_uninstall_service(Some(|out| {
            crate::service::uninstall_service(out, "", "");
            String::new()
        }));
        *hive_tasks::COMPUTE_PROVIDERS
            .write()
            .expect("compute providers hook lock") = Some(cmd_compute_providers);

        cli_register(Command { name:"plan".into(), group:"runner".into(), summary:"the job planner: turn the tally into queued jobs".into(), usage:"hive plan [--project SLUG] [--now T]".into(), aliases:vec![], no_store:false, read_only:false, raw:false, run:cmd_plan, flags:None });
        cli_register(Command { name:"work".into(), group:"runner".into(), summary:"lease the next job and run one worker here".into(), usage:"hive work [--project SLUG] [--job ID] [--dry-run] [--worker-cmd CMD] [--executor NAME] [--agent A] [--now T]".into(), aliases:vec![], no_store:false, read_only:false, raw:false, run:cmd_work, flags:Some(work_flags) });
        cli_register(Command { name:"land".into(), group:"runner".into(), summary:"push finished branches to the upstream as pull requests and settle the open ones (a project without an upstream lands them locally); --branch proposes one branch, --local merges it here instead".into(), usage:"hive land [--project SLUG] [--branch B] [--all] [--limit N] [--no-verify] [--dry-run] [--propose] [--local]".into(), aliases:vec![], no_store:false, read_only:false, raw:false, run:cmd_land, flags:Some(land_flags) });
        cli_register(Command { name:"pipeline".into(), group:"runner".into(), summary:"the GitHub Actions runs of the pushes hive made and of each project's default branch head, whoever pushed it: what is still watched, the last verdicts (pipeline.watch); --check reads GitHub now instead of waiting for the daemon's pass".into(), usage:"hive pipeline [--project SLUG] [--all] [--check] [--limit N] [--json]".into(), aliases:vec![], no_store:false, read_only:false, raw:false, run:cmd_pipeline, flags:Some(pipeline_flags) });
        cli_register(Command { name:"review".into(), group:"runner".into(), summary:"review the pull requests Hive opened: list and read them, check one out in the job's clone, then merge it, ask for changes, or hand it to the owner".into(), usage:"hive review list [--project SLUG] | show N | diff N | checkout N | merge N [--by owner] | changes N --note TEXT | owner N --note TEXT".into(), aliases:vec![], no_store:false, read_only:false, raw:false, run:crate::review_cli::cmd_review, flags:Some(crate::review_cli::review_flags) });
        cli_register(Command { name:"daemon".into(), group:"runner".into(), summary:"the runner loop: land, plan, dispatch workers, sweep".into(), usage:DAEMON_USAGE.into(), aliases:vec![], no_store:false, read_only:false, raw:false, run:cmd_daemon, flags:Some(daemon_flags) });
        cli_register(Command { name:"sweep".into(), group:"runner".into(), summary:"remove what runs and landings left on this machine and nothing holds any more (the daemon does this every ten minutes); --reclaim runs the disk reclaim ladder the daemon climbs under daemon.min_free_mb".into(), usage:"hive sweep [--dry-run] [--held] [--json] [--reclaim]".into(), aliases:vec![], no_store:false, read_only:false, raw:false, run:cmd_sweep, flags:Some(sweep_flags) });
        cli_register(Command { name:"backup".into(), group:"runner".into(), summary:"copy the store off this machine: a checked, gzipped copy of hive.sqlite to backup.to - a directory, az://ACCOUNT/CONTAINER[/PREFIX] (Azure Blob Storage), or `run: COMMAND {file} {name}` (the daemon does this every backup.every_hours); --status shows the last one and when the next is due".into(), usage:"hive backup [--to DEST] [--keep N] [--status] [--json]".into(), aliases:vec![], no_store:false, read_only:false, raw:false, run:cmd_backup, flags:Some(backup_flags) });
        cli_register(Command { name:"store".into(), group:"runner".into(), summary:"store upkeep: `archive` moves ledger rows older than daemon.archive_days that nothing reads any more (cold events, finished runs, ended transcripts) to hive-archive.sqlite next to hive.sqlite (the daemon does this once a month); --dry-run only counts them".into(), usage:"hive store archive [--dry-run] [--json]".into(), aliases:vec![], no_store:false, read_only:false, raw:false, run:cmd_store, flags:Some(store_flags) });
        cli_register(Command { name:"service".into(), group:"runner".into(), summary:"install, uninstall or show the user services that keep the daemon and the hub alive (launchd / systemd --user)".into(), usage:"hive service install|uninstall|status|show [--interval S] [--hub-port N] [--no-hub] [--dry-run]".into(), aliases:vec![], no_store:false, read_only:false, raw:false, run:cmd_service, flags:Some(service_flags) });
        cli_register(Command { name:"bulk".into(), group:"runner".into(), summary:"keep the regenerable bulk (work, cache, worktrees, build dirs) on another disk; the daemon waits while that disk is away".into(), usage:"hive bulk status | attach VOLUME [--any-folder] [--dry-run] | detach [--lost] [--dry-run]".into(), aliases:vec![], no_store:false, read_only:false, raw:false, run:cmd_bulk, flags:Some(bulk_flags) });
        cli_register(Command { name:"lead".into(), group:"agents".into(), summary:"the role leads of a project: seat one per concern (rsi, docs, security, monitor, or a word of your own), list, pause, resume, end, run, and ask which roles the project is missing".into(), usage:"hive lead add --project SLUG --role R [--title T] [--brief B | --brief-file F] [--every 1d]\nhive lead list [--project SLUG] [--status S] [--json]\nhive lead show LEAD-NNN [--json]\nhive lead pause|resume|end|run LEAD-NNN [--note N]\nhive lead suggest --project SLUG [--json]\nhive lead roles --project SLUG [--json]".into(), aliases:vec![], no_store:false, read_only:false, raw:false, run:crate::lead_cli::cmd_lead, flags:Some(crate::lead_cli::lead_flags) });
        cli_register(Command { name:"toolkit".into(), group:"agents".into(), summary:"the tools (MCP servers) and skills a project's shape asks for and does not have: show them, add one, print a drafted skill".into(), usage:"hive toolkit [show] [--project SLUG] [--all] [--json]\nhive toolkit add tool|skill NAME --project SLUG\nhive toolkit draft NAME --project SLUG      (prints the skill body hive would add)\n\nA suggestion comes from what the checkout shows: its remote, manifests, lockfiles and folders, and what is already registered. Without --project every active project is read. The daemon asks the owner about one suggestion per project at a time (toolkit.suggest_days); a yes registers it, a declined thread rests it.".into(), aliases:vec![], no_store:false, read_only:false, raw:false, run:crate::toolkit::cmd_toolkit, flags:Some(crate::toolkit::toolkit_flags) });
        cli_register(Command { name:"executor".into(), group:"runner".into(), summary:"list, add, probe, drain, activate, remove or log in the executors (remote hands)".into(), usage:"hive executor list | add NAME --ssh user@host[:port] --workdir /abs [--workers N] [--projects a,b|*] [--labels a,b] | probe NAME [--login] | drain NAME | activate NAME | remove NAME | label NAME WORDS|+a,-b|-b | reserve NAME [--off] | login NAME --admin admin@host [--with-token | --remove] | token [--with-token | --remove]".into(), aliases:vec![], no_store:false, read_only:false, raw:false, run:cmd_executor, flags:Some(executor_flags) });
        cli_register(Command { name:"env".into(), group:"runner".into(), summary:"the environment recipe per project (a CTX-environment record): show, set, import, check".into(), usage:"hive env show|set|import|check [--project SLUG] [--executor NAME] [--file F] [--json]".into(), aliases:vec![], no_store:false, read_only:false, raw:false, run:cmd_env, flags:Some(env_flags) });
        cli_register(Command { name:"secret".into(), group:"runner".into(), summary:"the team secret vault (ADR-070 rule 4): set a value from stdin, list names (never values), remove; a project's recipe `secrets:` names what its jobs are granted".into(), usage:"hive secret list [--json] | set NAME  (value on stdin) | rm NAME".into(), aliases:vec!["secrets".into()], no_store:false, read_only:false, raw:false, run:crate::secrets::cmd_secret, flags:None });
        cli_register(Command { name:"hands".into(), group:"runner".into(), summary:"the forced command on a paired machine (probe, prepare, key, run, ...)".into(), usage:"hive hands VERB [ARGS]  (or via SSH_ORIGINAL_COMMAND)".into(), aliases:vec![], no_store:true, read_only:false, raw:true, run:cmd_hands, flags:None });
        cli_register(Command { name:"pair".into(), group:"runner".into(), summary:"pair a Linux machine as hands over its admin ssh session: inspect it, apply as root (sudo -n), register and probe the executor; --dry-run only prints the scripts".into(), usage:"hive pair ADMIN@HOST[:PORT] --name N [--workdir D] [--workers N] [--projects P] [--dry-run | --inspect]".into(), aliases:vec![], no_store:false, read_only:false, raw:false, run:cmd_pair, flags:Some(pair_flags) });
        cli_register(Command { name:"update".into(), group:"runner".into(), summary:"install the newer Hive release update.latest_url names, or Hive's own latest release while that setting is empty (checksum-verified); --check only says whether one exists. A hive that a package manager such as Homebrew installed is left to it: the command names that manager's upgrade command".into(), usage:"hive update [--check] [--json]".into(), aliases:vec![], no_store:false, read_only:false, raw:false, run:crate::release_update::cmd_update, flags:Some(crate::release_update::update_flags) });
        cli_register(Command { name:"onboard".into(), group:"runner".into(), summary:"register a project (init) and queue its onboarding job; prints what stays human".into(), usage:"hive onboard [PATH] [--name N] [--remote URL] [--verify CMD] [--brain] [--now T]".into(), aliases:vec![], no_store:true, read_only:false, raw:false, run:cmd_onboard, flags:Some(onboard_flags) });
    });
}

fn work_flags(fs: &mut FlagSet) {
    fs.string_var("job", "", "one job id");
    fs.bool_var("dry-run", false, "lease nothing; show what would run");
    fs.string_var(
        "worker-cmd",
        "",
        "harness command (the brief text goes in for {brief}; default: hive harness run)",
    );
    fs.string_var("executor", "", "run on this executor");
}

fn land_flags(fs: &mut FlagSet) {
    fs.string_var(
        "branch",
        "",
        "push this branch and open its pull request (with --local: merge it into the default branch here)",
    );
    fs.bool_var("all", false, "every active project");
    fs.int_var(
        "limit",
        5,
        "accepted for older scripts; a pass opens at most ten pull requests per project",
    );
    fs.bool_var("no-verify", false, "skip the verify command (--local only)");
    fs.bool_var(
        "dry-run",
        false,
        "list what a pass would deliver: tasks, jobs, session branches, fixes",
    );
    fs.bool_var(
        "propose",
        false,
        "with --branch: what --branch does by itself now; kept for scripts",
    );
    fs.bool_var(
        "local",
        false,
        "with --branch: merge the branch into the local default branch instead of opening a pull request; the local branch then diverges from the upstream",
    );
}

fn daemon_flags(fs: &mut FlagSet) {
    fs.int_var(
        "interval",
        crate::daemon::DEFAULT_DAEMON_INTERVAL_SECONDS,
        "idle wait and runner-error retry delay in seconds",
    );
    fs.bool_var("once", false, "one loop, then exit");
    fs.int_var("max-loops", 0, "stop after N loops (0 = forever)");
    fs.bool_var("dry-run", false, "lease and plan nothing");
    fs.string_var("worker-cmd", "", "harness command for every run");
    fs.bool_var(
        "no-restart",
        false,
        "do not exit for a restart when the hive binary changes",
    );
    fs.bool_var(
        "force",
        false,
        "start even when the heartbeat names another live daemon",
    );
    fs.int_var(
        "max-jobs",
        0,
        "local runs at once (0 = the capacity setting, then what the machine's cores and memory allow)",
    );
}

fn sweep_flags(fs: &mut FlagSet) {
    fs.bool_var(
        "dry-run",
        false,
        "list what would go and what is held; remove nothing",
    );
    fs.bool_var("held", false, "also list what the sweep keeps and why");
    fs.bool_var(
        "reclaim",
        false,
        "climb the disk reclaim ladder now, floor or not: stale temp dirs of ours, then the sweep with its windows shortened; names the largest entries left",
    );
}

fn backup_flags(fs: &mut FlagSet) {
    fs.string_var(
        "to",
        "",
        "where this copy goes, instead of backup.to: a directory, az://ACCOUNT/CONTAINER[/PREFIX], or `run: COMMAND {file} {name}`",
    );
    fs.int_var(
        "keep",
        -1,
        "copies of this machine kept at the destination after this one, instead of backup.keep (0 keeps every copy)",
    );
    fs.bool_var(
        "status",
        false,
        "show the last backup, the settings in force and when the daemon takes the next; copy nothing",
    );
}

fn store_flags(fs: &mut FlagSet) {
    fs.bool_var(
        "dry-run",
        false,
        "count what the archive would move; move nothing",
    );
}

fn service_flags(fs: &mut FlagSet) {
    fs.int_var(
        "interval",
        crate::daemon::DEFAULT_DAEMON_INTERVAL_SECONDS,
        "daemon idle wait and runner-error retry delay in seconds",
    );
    fs.int_var(
        "hub-port",
        crate::service::HUB_PORT,
        "port the hub service listens on (127.0.0.1)",
    );
    fs.bool_var(
        "no-hub",
        false,
        "install the daemon only; run the console by hand with `hive hub`",
    );
    fs.bool_var(
        "dry-run",
        false,
        "print the units or plists instead of installing",
    );
}

fn bulk_flags(fs: &mut FlagSet) {
    fs.bool_var(
        "any-folder",
        false,
        "attach: accept a folder that is not a mount point",
    );
    fs.bool_var(
        "lost",
        false,
        "detach: the volume is gone for good; drop the links and start with empty dirs",
    );
    fs.bool_var("dry-run", false, "print what would move; change nothing");
}

fn executor_flags(fs: &mut FlagSet) {
    fs.string_var("ssh", "", "user@host[:port] of the executor");
    fs.string_var("workdir", "", "absolute work directory on the executor");
    fs.int_var("workers", 1, "parallel workers there");
    fs.string_var("projects", "*", "comma-separated project slugs or *");
    fs.string_var(
        "labels",
        "",
        "what the machine offers beyond its OS and architecture, comma-separated words a project can need (add)",
    );
    fs.bool_var("off", false, "reserve: let the machine take any job again");
    fs.string_var("admin", "", "admin ssh address for login");
    fs.bool_var(
        "with-token",
        false,
        "read a GitHub token from stdin instead of reusing the brain's login source",
    );
    fs.bool_var("remove", false, "remove the worker login");
    fs.bool_var("login", false, "probe: also run the login check");
    fs.dash_words(label_word);
}

/// `hive executor label NAME -word` removes a word: after `label NAME`, an
/// argument that begins with one dash and then a letter or a digit is a
/// change to the labels. The flags of `hive executor` stay flags there.
fn label_word(positional: &[String], arg: &str) -> bool {
    positional.len() >= 2
        && positional[0] == "label"
        && arg
            .strip_prefix('-')
            .is_some_and(|word| word.starts_with(|c: char| c.is_ascii_alphanumeric()))
}

fn env_flags(fs: &mut FlagSet) {
    fs.string_var("executor", "", "one executor; default: every executor");
    fs.string_var("file", "", "recipe text for `env set` (default: stdin)");
}

fn pair_flags(fs: &mut FlagSet) {
    fs.string_var("name", "", "executor name (lowercase words with dashes)");
    fs.string_var("workdir", "/srv/hive", "work directory on the machine");
    fs.int_var("workers", 1, "parallel workers there");
    fs.string_var("projects", "*", "comma-separated project slugs or *");
    fs.bool_var(
        "dry-run",
        false,
        "inspect and print the apply, undo and commit scripts; change nothing here or there",
    );
    fs.bool_var("inspect", false, "alias for --dry-run");
}

fn onboard_flags(fs: &mut FlagSet) {
    fs.string_var("name", "", "project name");
    fs.string_var("remote", "", "git remote URL");
    fs.string_var("verify", "", "the project's verify command");
    fs.bool_var("brain", false, "this checkout is the brain");
}

pub(crate) fn now_of(c: &hive_cli::Context) -> String {
    if !c.now.is_empty() {
        c.now.clone()
    } else if let Some(s) = &c.store {
        s.now()
    } else {
        hive_store::now()
    }
}

pub(crate) fn actor_of(c: &hive_cli::Context) -> String {
    if c.agent.is_empty() {
        crate::util::HUMAN_AGENT.into()
    } else {
        c.agent.clone()
    }
}

pub(crate) fn str_flag(fs: &FlagSet, name: &str) -> String {
    fs.lookup(name)
        .map(|_| fs.get_string(name))
        .unwrap_or_default()
}

pub(crate) fn bool_flag(fs: &FlagSet, name: &str) -> bool {
    fs.lookup(name).is_some_and(|_| fs.get_bool(name))
}

pub(crate) fn int_flag(fs: &FlagSet, name: &str) -> i32 {
    fs.lookup(name)
        .map(|_| fs.get_int(name))
        .unwrap_or_default()
}

pub(crate) fn cmd_plan(c: &mut Context, _fs: &FlagSet, _args: &[String]) -> Result<()> {
    let s = c
        .store
        .clone()
        .ok_or_else(|| anyhow!("plan: store is needed"))?;
    let now = now_of(c);
    let slugs = if !c.project.is_empty() {
        vec![crate::env::current_project(&s, c.project.clone())?.str("slug")]
    } else {
        let conn = s.conn()?;
        crate::board::project_rows(&conn, false)
            .into_iter()
            .map(|p| p.str("slug"))
            .collect()
    };
    for slug in slugs {
        let jobs = crate::plan::plan_jobs(&s, slug, now.clone(), "plan".into())?;
        for j in jobs {
            c.printf(format_args!(
                "planned {} {}: {}\n",
                j.str("kind"),
                j.str("id"),
                j.str("title")
            ))?;
        }
    }
    Ok(())
}

pub(crate) fn cmd_work(c: &mut Context, fs: &FlagSet, _args: &[String]) -> Result<()> {
    let s = c
        .store
        .clone()
        .ok_or_else(|| anyhow!("work: store is needed"))?;
    let now = now_of(c);
    let worker = crate::run::pick_worker(&s, &now, &c.agent, &[])?;
    let (run, reason) = crate::run::work_once(
        &s,
        &worker,
        &c.project,
        &str_flag(fs, "job"),
        &now,
        crate::run::RunOpts {
            dry_run: bool_flag(fs, "dry-run"),
            worker_cmd: str_flag(fs, "worker-cmd"),
            executor: str_flag(fs, "executor"),
            ..Default::default()
        },
    )?;
    if run.is_empty() {
        c.printf(format_args!("idle: {reason}\n"))?;
        return Err(hive_cli::errorf(3, "", "", "").into());
    }
    c.printf(format_args!(
        "{} {} by {} on {}: {}\n",
        run.str("status"),
        run.str("job_id"),
        run.str("agent"),
        run.str("executor"),
        run.str("summary")
    ))?;
    if !run.str("branch").is_empty() {
        c.printf(format_args!(
            "branch {} (local; `hive land` or the daemon merges it once the checkout is clean)\n",
            run.str("branch")
        ))?;
    }
    let st = run.str("status");
    if st != "done" && st != "dry-run" {
        return Err(hive_cli::errorf(1, "", "", "").into());
    }
    Ok(())
}

fn pipeline_flags(fs: &mut FlagSet) {
    fs.bool_var("all", false, "every active project");
    fs.bool_var(
        "check",
        false,
        "read the runs from GitHub now, record the verdicts and plan what pipeline.watch allows",
    );
    fs.int_var("limit", 5, "verdicts shown per project");
}

/// `hive pipeline`: the pushes the daemon still watches and the verdicts
/// it recorded, per project; `--check` runs the watcher's pass by hand.
pub(crate) fn cmd_pipeline(c: &mut Context, fs: &FlagSet, _args: &[String]) -> Result<()> {
    let s = c
        .store
        .clone()
        .ok_or_else(|| anyhow!("pipeline: store is needed"))?;
    let now = now_of(c);
    let actor = actor_of(c);
    let targets = if bool_flag(fs, "all") || (c.project.is_empty() && !c.json) {
        crate::board::project_rows(&s.conn()?, false)
    } else {
        vec![crate::env::current_project(&s, c.project.clone())?]
    };
    let limit = int_flag(fs, "limit").max(1) as i64;
    let mode = s.setting(crate::pipeline::WATCH, "fix");
    let conn = s.conn()?;
    let mut out = Vec::new();
    for proj in &targets {
        let slug = proj.str("slug");
        let lines = if bool_flag(fs, "check") {
            crate::pipeline::tend_pipeline(&s, &slug, &now, &actor)
        } else {
            Vec::new()
        };
        let watch = crate::pipeline::load_watch(&s, &slug);
        let pending = crate::pipeline::pending(&conn, &slug, &now);
        let verdicts: Vec<serde_json::Value> = crate::pipeline::verdicts(&conn, &slug, limit)
            .iter()
            .map(|r| {
                let mut d: serde_json::Value =
                    serde_json::from_str(&r.str("detail")).unwrap_or(json!({}));
                if let Some(o) = d.as_object_mut() {
                    o.insert(
                        "verdict".into(),
                        json!(r.str("action").trim_start_matches("pipeline.")),
                    );
                    o.insert("at".into(), json!(r.str("at")));
                }
                d
            })
            .collect();
        out.push(json!({
            "project": slug,
            "repo": crate::security::github_repo(&proj.str("remote")),
            "watching": pending.iter().map(|p| json!({
                "sha": p.sha, "at": p.at, "remote": p.remote, "branch": p.branch,
                "number": p.number, "url": p.url,
                "last_read": watch.polled.get(&p.sha).cloned().unwrap_or_default(),
            })).collect::<Vec<_>>(),
            "head": if watch.head_sha.is_empty() { serde_json::Value::Null } else { json!({
                "sha": watch.head_sha, "by": watch.head_by, "read": watch.head_read, "next": watch.head_next,
                "waiting_since": watch.head_waiting_since,
            }) },
            "verdicts": verdicts,
            "lines": lines,
        }));
    }
    if c.json {
        return hive_cli::write_json(&mut c.out, &json!({"watch": mode, "projects": out}));
    }
    c.printf(format_args!("pipeline.watch is {mode}\n"))?;
    for p in &out {
        let slug = p["project"].as_str().unwrap_or_default();
        for l in p["lines"].as_array().into_iter().flatten() {
            c.printf(format_args!("{}\n", l.as_str().unwrap_or_default()))?;
        }
        let watching = p["watching"].as_array().cloned().unwrap_or_default();
        let verdicts = p["verdicts"].as_array().cloned().unwrap_or_default();
        if p["repo"].is_null() {
            if !watching.is_empty() {
                c.printf(format_args!(
                    "{slug}: {} pushed, not on GitHub, so not watched\n",
                    pushes_text(watching.len())
                ))?;
            }
            continue;
        }
        if watching.is_empty() && verdicts.is_empty() && p["head"].is_null() {
            if bool_flag(fs, "all") || !c.project.is_empty() {
                c.printf(format_args!(
                    "{slug}: no push of hive's to watch; the branch head is read on the daemon's next pass\n"
                ))?;
            }
            continue;
        }
        if let Some(head) = p["head"].as_object() {
            let sha = head["sha"].as_str().unwrap_or_default();
            let by = head["by"].as_str().unwrap_or_default();
            let waiting = head["waiting_since"].as_str().unwrap_or_default();
            c.printf(format_args!(
                "{slug}: branch head {} (pushed by {}), read {}{}\n",
                crate::recipe::short(sha, 12),
                if by.is_empty() { "hive" } else { by },
                head["read"].as_str().unwrap_or_default(),
                if waiting.is_empty() {
                    String::new()
                } else {
                    format!("; red, waiting since {waiting} for the checkout to hold it")
                }
            ))?;
        }
        for w in &watching {
            let at = w["at"].as_str().unwrap_or_default();
            let age = crate::util::format_minutes(
                (chrono::DateTime::parse_from_rfc3339(&now)
                    .ok()
                    .zip(chrono::DateTime::parse_from_rfc3339(at).ok()))
                .map(|(n, a)| (n - a).num_minutes())
                .unwrap_or(0),
            );
            c.printf(format_args!(
                "{slug}: watching {} on {} (pushed {age} ago)\n",
                crate::recipe::short(w["sha"].as_str().unwrap_or_default(), 12),
                target_text(w)
            ))?;
        }
        for v in &verdicts {
            let failed: Vec<&str> = v["failed"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
                .unwrap_or_default();
            let runs: Vec<&str> = v["runs"]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x["workflow"].as_str()).collect())
                .unwrap_or_default();
            let verdict = v["verdict"].as_str().unwrap_or_default();
            let mut line = format!(
                "{slug}: {} at {} on {} {}",
                verdict,
                crate::recipe::short(v["sha"].as_str().unwrap_or_default(), 12),
                target_text(v),
                v["at"].as_str().unwrap_or_default()
            );
            match verdict {
                "failed" => line.push_str(&format!(" - {} failed", failed.join(", "))),
                "passed" => line.push_str(&format!(" - {}", runs.join(", "))),
                "rerun" => line.push_str(
                    " - GitHub no longer had the failed log; the failed jobs were rerun for a fresh one",
                ),
                _ => {}
            }
            if let Some(by) = v["by"].as_str() {
                line.push_str(&format!(" (pushed by {by})"));
            }
            if let Some(job) = v["job"].as_str() {
                line.push_str(&format!("; fix job {job}{}", fix_job_state(&conn, job)));
            }
            if v["superseded"].as_bool() == Some(true) {
                line.push_str(" (superseded)");
            }
            c.printf(format_args!("{line}\n"))?;
        }
    }
    Ok(())
}

fn pushes_text(n: usize) -> String {
    format!("{n} {}", crate::util::plural(n as i32, "push"))
}

/// ` (done)`, ` (queued)`, ` (failed: blocked: ...)`: how the fix job a
/// verdict names ended, so a red pipeline nobody fixed is not read as
/// looked after. Empty when the job is not in the store.
fn fix_job_state(conn: &rusqlite::Connection, id: &str) -> String {
    let rows = crate::util::must_rows(conn, "SELECT status, result FROM jobs WHERE id = ?", &[&id]);
    let Some(job) = rows.first() else {
        return String::new();
    };
    let status = job.str("status");
    let result = crate::util::trunc(
        &job.str("result")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        160,
    );
    match status.as_str() {
        "failed" | "cancelled" if !result.is_empty() => format!(" ({status}: {result})"),
        _ => format!(" ({status})"),
    }
}

/// `origin/main` or `pull request #7`, from a watch or verdict record.
fn target_text(v: &serde_json::Value) -> String {
    match v["number"].as_i64() {
        Some(n) if n > 0 => format!("pull request #{n}"),
        _ => format!(
            "{}/{}",
            v["remote"].as_str().unwrap_or_default(),
            v["branch"].as_str().unwrap_or_default()
        ),
    }
}

pub(crate) fn cmd_land(c: &mut Context, fs: &FlagSet, _args: &[String]) -> Result<()> {
    let s = c
        .store
        .clone()
        .ok_or_else(|| anyhow!("land: store is needed"))?;
    let now = now_of(c);
    let actor = actor_of(c);
    let targets = if bool_flag(fs, "all") {
        crate::board::project_rows(&s.conn()?, false)
    } else {
        vec![crate::env::current_project(&s, c.project.clone())?]
    };
    let mut worst = 0;
    let rank = BTreeMap::from([
        ("landed", 0),
        ("merged", 0),
        ("conflict", 1),
        ("failed-verify", 1),
        ("missing", 1),
    ]);
    let branch = str_flag(fs, "branch");
    let verify = !bool_flag(fs, "no-verify");
    let local = bool_flag(fs, "local");
    if local && branch.is_empty() {
        return Err(hive_cli::errorf(
            2,
            "land --local needs --branch",
            "--local merges one named branch into the local default branch by hand",
            "hive land --project SLUG --branch B --local",
        )
        .into());
    }
    if bool_flag(fs, "propose") && branch.is_empty() {
        return Err(hive_cli::errorf(
            2,
            "land --propose needs --branch",
            "--propose names the branch to open a pull request for",
            "hive land --project SLUG --branch B",
        )
        .into());
    }
    for proj in targets {
        let slug = proj.str("slug");
        if bool_flag(fs, "dry-run") {
            for line in crate::delivery::dry_run(&s, &proj, &now) {
                c.printf(format_args!("{line}\n"))?;
            }
            continue;
        }
        if branch.is_empty() {
            // One delivery pass for the project: the pull requests owed,
            // the open ones settled, the default branch brought up to its
            // upstream (delivery.rs) - what the daemon does every pass.
            let lines = crate::delivery::tend(&s, std::slice::from_ref(&proj), &now);
            if lines.is_empty() {
                c.printf(format_args!("{slug}: nothing waiting\n"))?;
            }
            for line in lines {
                c.printf(format_args!("{line}\n"))?;
            }
            continue;
        }
        let def = crate::baseline::default_branch(proj.clone());
        let has_upstream = crate::forge::upstream_of(&proj.str("path"), &def).is_some();
        if !local && has_upstream {
            // A session's agent proposing its own change that has a face
            // brings the before/after report first; the owner proposing it
            // by hand has looked at it.
            if actor != crate::util::HUMAN_AGENT
                && let Some(e) = crate::ui_evidence::chat_branch_evidence(&s, &proj, &branch)
                    .filter(|e| e.wants_report())
            {
                c.printf(format_args!(
                    "{slug} not proposed: {}\n",
                    crate::ui_evidence::session_report_message(&e, &branch, &def)
                ))?;
                worst = worst.max(1);
                continue;
            }
            match crate::delivery::propose_branch(&s, &proj, &branch, &now) {
                Ok(d) => c.printf(format_args!("{}\n", d.line))?,
                Err(why) => {
                    c.printf(format_args!("{slug} not proposed: {why}\n"))?;
                    worst = worst.max(1);
                }
            }
            continue;
        }
        if local && has_upstream {
            c.printf(format_args!(
                "{slug}: merging {branch} into {def} here; the local {def} will be ahead of its upstream until that is pushed or reset, and the daemon only ever fast-forwards it\n"
            ))?;
        } else {
            c.printf(format_args!("{slug}: no upstream, landing locally\n"))?;
        }
        // The hold a failed setup or a red base left says this command retries
        // now.
        crate::land::clear_landing_retry(&s, &slug);
        let job = query_one_row(
            &*s.conn()?,
            "SELECT * FROM jobs WHERE project = ? AND branch = ? ORDER BY finished_at DESC",
            &[&slug, &branch],
        );
        // The job's last run names the commit its clone was made from: a branch
        // forked before main was squashed to a new root replays from there
        // instead of conflicting on every file (land.rs replay_from).
        let run_id = if job.str("id").is_empty() {
            String::new()
        } else {
            crate::experiment::last_run_id(&s, job.str("id"))
        };
        let (state, note) = crate::land::land_branch(
            &s,
            proj.clone(),
            branch.clone(),
            actor.clone(),
            now.clone(),
            crate::land::LandOpts {
                job,
                run_id,
                verify,
                by_hand: actor == crate::util::HUMAN_AGENT,
                ..Default::default()
            },
        )?;
        c.printf(format_args!("{slug} {state}: {note}\n"))?;
        let w = rank.get(state.as_str()).copied().unwrap_or(3);
        if w > worst {
            worst = w;
        }
    }
    if worst != 0 {
        return Err(hive_cli::errorf(worst, "", "", "").into());
    }
    Ok(())
}

pub(crate) fn query_one_row(
    q: &dyn crate::util::Querier,
    sql: &str,
    args: &[&dyn rusqlite::ToSql],
) -> crate::util::Row {
    crate::util::query_one(q, sql, args)
        .unwrap_or_default()
        .unwrap_or_default()
}

const DAEMON_USAGE: &str = "hive daemon [--interval S] [--once] [--max-loops N] [--max-jobs N] [--dry-run] [--worker-cmd CMD] [--no-restart] [--force] [--agent A]";

/// The pid of another daemon that still beats on this store: the heartbeat
/// row names a pid other than ours that is alive, its note is not the final
/// `exited`/`restarting` beat, and the beat is fresh (the same window
/// `hive status` calls alive), so a pid reused after a reboot does not
/// block the service from starting.
pub(crate) fn live_daemon_pid(s: &hive_store::Store) -> Option<(i32, String)> {
    let (raw, exists) = s.meta("daemon").ok()?;
    if !exists {
        return None;
    }
    let beat: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let pid = i32::try_from(beat.get("pid")?.as_i64()?).ok()?;
    if pid <= 1 || pid as u32 == std::process::id() {
        return None;
    }
    let note = beat.get("note").and_then(|v| v.as_str()).unwrap_or("");
    if note == "exited" || note == "restarting" {
        return None;
    }
    let at = beat.get("at").and_then(|v| v.as_str()).unwrap_or("");
    let at_time = crate::util::parse_iso(at).ok()?;
    let now = crate::util::parse_iso(&s.now()).ok()?;
    let interval = beat
        .get("interval")
        .and_then(|v| v.as_i64())
        .filter(|n| *n > 0)
        .unwrap_or(crate::daemon::DEFAULT_DAEMON_INTERVAL_SECONDS as i64);
    if (now - at_time).num_seconds() > 2 * interval + 60 {
        return None;
    }
    // ESRCH is the only answer that means gone; EPERM is a live process of
    // another user.
    #[cfg(unix)]
    let gone = matches!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
        Err(nix::errno::Errno::ESRCH)
    );
    // Off unix a process cannot be asked whether it lives yet
    // (docs/windows-port.md), so a fresh heartbeat stands for a live daemon.
    #[cfg(not(unix))]
    let gone = false;
    if gone {
        return None;
    }
    Some((pid, at.to_string()))
}

/// Why `hive daemon` will not start: stray positional arguments (exit 2)
/// or another live daemon on the same store (exit 3) unless `--force`.
fn daemon_start_refusal(
    s: &hive_store::Store,
    fs: &FlagSet,
    args: &[String],
) -> Option<hive_cli::Error> {
    if !args.is_empty() {
        return Some(crate::util::fail(
            2,
            format!(
                "hive daemon takes no arguments (got {}); usage: {DAEMON_USAGE}",
                args.join(" ")
            ),
        ));
    }
    if bool_flag(fs, "force") {
        return None;
    }
    let (pid, at) = live_daemon_pid(s)?;
    Some(crate::util::fail(
        3,
        format!(
            "a hive daemon is already running on this store (pid {pid}, last heartbeat {at}); stop it first or pass --force"
        ),
    ))
}

pub(crate) fn cmd_daemon(c: &mut Context, fs: &FlagSet, args: &[String]) -> Result<()> {
    let s = c
        .store
        .clone()
        .ok_or_else(|| anyhow!("daemon: store is needed"))?;
    if let Some(refusal) = daemon_start_refusal(&s, fs, args) {
        return Err(refusal.into());
    }
    // The first SIGTERM/SIGINT cancels the loop; `_stop` (the guard) unregisters the
    // handler on return so a second signal terminates the process.
    let cancel = hive_bus::CancellationToken::new();
    let _stop = SignalStop::notify(&cancel)?;
    let loops = if bool_flag(fs, "once") {
        1
    } else {
        int_flag(fs, "max-loops")
    };
    // Lend the writer to the daemon for the run and take it back afterwards.
    let lent = Arc::new(Mutex::new(LentWriter(Some(std::mem::replace(
        &mut c.out,
        Box::new(std::io::sink()),
    )))));
    let out: crate::daemon::DaemonOut = lent.clone();
    let result = crate::daemon::daemon(
        &cancel,
        &s,
        crate::daemon::DaemonOpts {
            agent: c.agent.clone(),
            interval: int_flag(fs, "interval"),
            dry_run: bool_flag(fs, "dry-run"),
            worker_cmd: str_flag(fs, "worker-cmd"),
            max_loops: loops,
            restart_on_change: !bool_flag(fs, "no-restart"),
            max_jobs: int_flag(fs, "max-jobs"),
            out: Some(out.clone()),
            hub_home: crate::util::home_dir(),
            ..Default::default()
        },
    );
    drop(_stop);
    if let Some(w) = lent.lock().unwrap_or_else(|e| e.into_inner()).0.take() {
        c.out = w;
    }
    if result == "done" || result == "interrupted" {
        Ok(())
    } else {
        if result == "restart" {
            writeln!(
                c.err,
                "hive binary changed on disk; exiting 3 so the service restarts the daemon"
            )?;
        }
        Err(hive_cli::errorf(3, "", "", "").into())
    }
}

/// c.Out on loan to the daemon: writes after the command took it back go nowhere.
struct LentWriter(Option<Box<dyn Write + Send>>);

impl Write for LentWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match &mut self.0 {
            Some(w) => w.write(buf),
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match &mut self.0 {
            Some(w) => w.flush(),
            None => Ok(()),
        }
    }
}

/// SIGTERM/SIGINT cancel the token while the guard lives; dropping it unregisters the
/// handlers, restoring the default (terminating) disposition. The cancel runs on an
/// ordinary thread fed by the signal, never inside the handler itself.
#[cfg(unix)]
struct SignalStop {
    handle: signal_hook::iterator::Handle,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(unix)]
impl SignalStop {
    fn notify(cancel: &hive_bus::CancellationToken) -> Result<Self> {
        let mut signals = signal_hook::iterator::Signals::new([
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGINT,
        ])?;
        let handle = signals.handle();
        let cancel = cancel.clone();
        let thread = std::thread::Builder::new()
            .name("daemon-signals".into())
            .spawn(move || {
                if signals.forever().next().is_some() {
                    cancel.cancel();
                }
            })?;
        Ok(Self {
            handle,
            thread: Some(thread),
        })
    }
}

#[cfg(unix)]
impl Drop for SignalStop {
    fn drop(&mut self) {
        self.handle.close();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// signal-hook has no signal iterator off unix. There the first Ctrl+C sets
/// a flag (signal_hook::flag) and a watcher thread turns it into the cancel;
/// dropping the guard unregisters the handler and stops the watcher. The
/// same shape as hive-evolve's `SignalStop`.
#[cfg(not(unix))]
struct SignalStop {
    id: Option<signal_hook::SigId>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(not(unix))]
impl SignalStop {
    fn notify(cancel: &hive_bus::CancellationToken) -> Result<Self> {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let hit = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (cancel, watch_hit, watch_stop) = (cancel.clone(), hit.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("daemon-signals".into())
            .spawn(move || {
                while !watch_stop.load(Ordering::Relaxed) {
                    if watch_hit.load(Ordering::Relaxed) {
                        cancel.cancel();
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            })?;
        // Built before the handler so a failed registration stops the watcher.
        let mut guard = Self {
            id: None,
            stop,
            thread: Some(thread),
        };
        guard.id = Some(signal_hook::flag::register(
            signal_hook::consts::SIGINT,
            hit,
        )?);
        Ok(guard)
    }
}

#[cfg(not(unix))]
impl Drop for SignalStop {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            signal_hook::low_level::unregister(id);
        }
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub(crate) fn cmd_sweep(c: &mut Context, fs: &FlagSet, args: &[String]) -> Result<()> {
    let s = c
        .store
        .clone()
        .ok_or_else(|| anyhow!("sweep: store is needed"))?;
    let now = now_of(c);
    if !args.is_empty() {
        return Err(crate::util::fail(
            2,
            "hive sweep takes no arguments (hive sweep [--dry-run] [--held] [--json] [--reclaim])",
        )
        .into());
    }
    let dry_run = bool_flag(fs, "dry-run");
    let mut probes = crate::checkout::CheckoutProbes::default();
    let (projects, held, _) = probes.ready(
        crate::board::project_rows(&s.conn()?, false),
        crate::checkout::CHECKOUT_PROBE_TIMEOUT,
    );
    if bool_flag(fs, "reclaim") {
        return cmd_sweep_reclaim(c, &s, projects, held, now, dry_run);
    }
    let mut rep = crate::sweep::sweep(
        &s,
        projects,
        now.clone(),
        crate::sweep::SweepOpts {
            dry_run,
            actor: actor_of(c),
            user_home: crate::util::home_dir(),
            ..Default::default()
        },
    )
    .unwrap_or_default();
    if !dry_run {
        crate::sweep::record_sweep(&s, Some(rep.clone()), now, actor_of(c));
    }
    if c.json {
        hive_cli::write_json(
            &mut c.out,
            &json!({"at": rep.at, "dry_run": rep.dry_run, "removed": rep.removed, "held": rep.held, "cargo_target": rep.cargo_target, "errors": rep.errors, "bytes": rep.bytes(), "unreadable": held, "runners": rep.runners.clone().or_else(|| crate::runners::RunnersNote::load(&s))}),
        )?;
        return Ok(());
    }
    for (slug, why) in held {
        c.printf(format_args!("skipped {slug}: {why}\n"))?;
    }
    for l in crate::sweep::sweep_lines(Some(rep.clone())) {
        c.printf(format_args!("{l}\n"))?;
    }
    if bool_flag(fs, "held") {
        for x in &rep.held {
            let where_ = if !x.project.is_empty() && (x.kind == "branch" || x.kind == "ref") {
                format!("{} {}", x.project, x.path)
            } else {
                x.path.clone()
            };
            c.printf(format_args!("held {} {} ({})\n", x.kind, where_, x.why))?;
        }
    }
    let verb = if dry_run { "would remove" } else { "removed" };
    c.printf(format_args!(
        "{} {} item(s), {}; {} held\n",
        verb,
        rep.removed.len(),
        crate::sweep::size_text(rep.bytes()),
        rep.held.len()
    ))?;
    if !rep.errors.is_empty() {
        return Err(hive_cli::errorf(
            1,
            &format!(
                "{} could not be swept",
                crate::util::plural(rep.errors.len() as i32, "item")
            ),
            &rep.errors[0],
            "fix the cause above and run hive sweep again; the daemon tries again every ten minutes",
        )
        .into());
    }
    Ok(())
}

/// `hive sweep --reclaim`: the ladder the daemon climbs under
/// `daemon.min_free_mb`, run by hand with every rung, floor or not.
pub(crate) fn cmd_backup(c: &mut Context, fs: &FlagSet, args: &[String]) -> Result<()> {
    let s = c
        .store
        .clone()
        .ok_or_else(|| anyhow!("backup: store is needed"))?;
    if !args.is_empty() {
        return Err(crate::util::fail(
            2,
            "hive backup takes no arguments (hive backup [--to DEST] [--keep N] [--status] [--json])",
        )
        .into());
    }
    if bool_flag(fs, "status") {
        let st = crate::backup::status(&s);
        if c.json {
            hive_cli::write_json(&mut c.out, &st)?;
            return Ok(());
        }
        let to = st["to"].as_str().unwrap_or_default();
        if to.is_empty() {
            c.printf(format_args!(
                "no backups: set {} to a directory, az://ACCOUNT/CONTAINER[/PREFIX] or `run: COMMAND {{file}} {{name}}` (hive settings set {} ...)
",
                crate::backup::TO,
                crate::backup::TO
            ))?;
        } else {
            c.printf(format_args!(
                "to: {to}
every: {} h (0: by hand only), keep: {} (0: all)
",
                st["every_hours"], st["keep"]
            ))?;
        }
        match crate::backup::last(&s) {
            Some(last) => c.printf(format_args!(
                "last: {} {}
",
                last.at,
                crate::backup::line(&last)
            ))?,
            None => c.printf(format_args!(
                "last: none yet
"
            ))?,
        }
        match st["next_due"].as_str() {
            Some("") => c.printf(format_args!(
                "next: on the daemon's next pass
"
            ))?,
            Some(at) => c.printf(format_args!(
                "next: {at}
"
            ))?,
            None => c.printf(format_args!(
                "next: none (the daemon takes none)
"
            ))?,
        }
        return Ok(());
    }
    let keep = int_flag(fs, "keep");
    let out = crate::backup::run_backup(
        &s,
        crate::backup::Opts {
            to: str_flag(fs, "to"),
            keep: if keep < 0 { None } else { Some(keep as i64) },
            actor: actor_of(c),
            now: now_of(c),
        },
    );
    if c.json {
        hive_cli::write_json(&mut c.out, &out)?;
    } else {
        c.printf(format_args!(
            "{}
",
            crate::backup::line(&out)
        ))?;
        if out.ok {
            c.printf(format_args!(
                "sha256 {}
",
                out.sha256
            ))?;
        }
    }
    if out.ok {
        Ok(())
    } else {
        Err(crate::util::fail(1, format!("backup failed: {}", out.error)).into())
    }
}

fn cmd_sweep_reclaim(
    c: &mut Context,
    s: &hive_store::Store,
    projects: Vec<crate::util::Row>,
    unreadable: BTreeMap<String, String>,
    now: String,
    dry_run: bool,
) -> Result<()> {
    let out = crate::reclaim::reclaim(
        s,
        projects,
        now,
        actor_of(c),
        BTreeMap::new(),
        dry_run,
        true,
        crate::util::home_dir(),
    );
    if !dry_run {
        crate::reclaim::record_reclaim(s, &out, &actor_of(c));
    }
    if c.json {
        hive_cli::write_json(
            &mut c.out,
            &json!({
                "at": out.at, "dry_run": out.dry_run, "rungs": out.rungs,
                "bytes": out.freed, "cleared": out.cleared, "free_mb": out.free_mb,
                "floor_mb": out.floor_mb, "largest": out.largest, "errors": out.errors,
                "unreadable": unreadable,
            }),
        )?;
        return Ok(());
    }
    for (slug, why) in unreadable {
        c.printf(format_args!("skipped {slug}: {why}\n"))?;
    }
    for l in &out.lines {
        c.printf(format_args!("{l}\n"))?;
    }
    let verb = if dry_run { "would free" } else { "freed" };
    c.printf(format_args!(
        "{verb} {} over {} rung(s); {} MB free{}\n",
        crate::sweep::size_text(out.freed as i64),
        out.rungs.len(),
        out.free_mb,
        if out.floor_mb > 0 {
            format!(" (daemon.min_free_mb {})", out.floor_mb)
        } else {
            String::new()
        }
    ))?;
    if !out.errors.is_empty() {
        return Err(hive_cli::errorf(
            1,
            &format!(
                "{} could not be reclaimed",
                crate::util::plural(out.errors.len() as i32, "item")
            ),
            &out.errors[0],
            "fix the cause above and run hive sweep --reclaim again; under the floor the daemon climbs the ladder every thirty minutes",
        )
        .into());
    }
    Ok(())
}

pub(crate) fn cmd_service(c: &mut Context, fs: &FlagSet, args: &[String]) -> Result<()> {
    let action = args.first().map(String::as_str).unwrap_or("status");
    match action {
        "install" => {
            let _ = crate::service::install_service(
                &mut c.out,
                &c.home,
                crate::service::ServiceInstallOptions {
                    interval: int_flag(fs, "interval"),
                    hub_port: int_flag(fs, "hub-port"),
                    no_hub: bool_flag(fs, "no-hub"),
                    dry_run: bool_flag(fs, "dry-run"),
                    defer_hub: false,
                },
            )?;
        }
        "uninstall" => {
            crate::service::uninstall_service(&mut c.out, "", "");
        }
        "show" => {
            for u in crate::service::service_units(
                int_flag(fs, "interval"),
                int_flag(fs, "hub-port"),
                bool_flag(fs, "no-hub"),
            ) {
                crate::service::install_unit(
                    &mut c.out,
                    u,
                    c.home.clone(),
                    String::new(),
                    String::new(),
                    true,
                )?;
            }
        }
        "status" => {
            let s = c
                .store
                .as_ref()
                .ok_or_else(|| anyhow!("service status: store is needed"))?;
            crate::service::service_status_inner(&mut c.out, Some(s), "", "");
        }
        _ => {
            let (_, path) = crate::service::service_paths("".into(), "".into());
            return Err(crate::util::fail(2, format!("hive service: unknown action {action:?} (install|uninstall|status|show); file would be {path}")).into());
        }
    }
    Ok(())
}

pub(crate) fn cmd_bulk(c: &mut Context, fs: &FlagSet, args: &[String]) -> Result<()> {
    let action = args.first().map(String::as_str).unwrap_or("status");
    let s = c
        .store
        .as_ref()
        .ok_or_else(|| anyhow!("bulk: store is needed"))?;
    match action {
        "status" => {
            writeln!(
                c.out,
                "{}",
                crate::bulk::status_line(std::path::Path::new(&c.home))
            )?;
        }
        "attach" => {
            let volume = args.get(1).filter(|v| !v.is_empty()).ok_or_else(|| {
                crate::util::fail(
                    2,
                    "hive bulk attach: name the volume, e.g. `hive bulk attach /Volumes/Kingston`",
                )
            })?;
            crate::bulk::attach(
                s,
                std::path::Path::new(volume),
                crate::bulk::AttachOpts {
                    any_folder: bool_flag(fs, "any-folder"),
                    dry_run: bool_flag(fs, "dry-run"),
                },
                &mut c.out,
            )
            .map_err(|e| crate::util::fail(2, format!("hive bulk attach: {e}")))?;
        }
        "detach" => {
            crate::bulk::detach(
                s,
                crate::bulk::DetachOpts {
                    lost: bool_flag(fs, "lost"),
                    dry_run: bool_flag(fs, "dry-run"),
                },
                &mut c.out,
            )
            .map_err(|e| crate::util::fail(2, format!("hive bulk detach: {e}")))?;
        }
        _ => {
            return Err(crate::util::fail(
                2,
                format!("hive bulk: unknown action {action:?} (status|attach VOLUME|detach)"),
            )
            .into());
        }
    }
    Ok(())
}

pub(crate) fn executors_text(s: &hive_store::Store, now: &str) -> String {
    let mut out = Vec::new();
    for r in crate::executor::executor_rows(s, true) {
        let name = r.str("name");
        let busy = crate::executor::executor_running(s, &name, now);
        let addr = if r.str("address").is_empty() {
            "this machine".into()
        } else {
            r.str("address")
        };
        out.push(format!(
            "{:<14} {:<11} {:<30} workers {} busy {} projects {}",
            name,
            r.str("status"),
            addr,
            r.int("workers"),
            busy,
            r.str("projects")
        ));
        out.push(format!(
            "{:14} offers {}",
            "",
            crate::executor::caps_text(&r)
        ));
        if name != crate::executor::LOCAL_EXECUTOR {
            let next = if r.str("next_probe_at").is_empty() {
                String::new()
            } else {
                format!("; next probe {}", r.str("next_probe_at"))
            };
            out.push(format!(
                "{:14} workdir {}; runs done {} failed {}; errors {}{}",
                "",
                r.str("workdir"),
                r.int("runs_done"),
                r.int("runs_failed"),
                r.int("errors"),
                next
            ));
            let last = if r.str("last_probe_at").is_empty() {
                "never".into()
            } else {
                r.str("last_probe_at")
            };
            let note = if r.str("probe_note").is_empty() {
                "-".into()
            } else {
                r.str("probe_note")
            };
            out.push(format!("{:14} probe {}: {}", "", last, note));
            let check = crate::executor::last_login_check(s, &name);
            let expires = crate::executor::login_expiry(s, &name);
            let warn =
                crate::executor::expiry_warning(&expires, now, crate::executor::LOGIN_EXPIRY_WARN);
            if !check.is_empty() || !expires.is_empty() {
                let text = if check.is_empty() {
                    "never checked".to_string()
                } else {
                    let state = if check
                        .get("ok")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                    {
                        "ok"
                    } else {
                        "FAILED"
                    };
                    format!(
                        "{} at {}: {}",
                        state,
                        check
                            .get("at")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or(""),
                        check
                            .get("note")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                    )
                };
                let exp = if expires.is_empty() {
                    String::new()
                } else {
                    format!("; token expires {expires}")
                };
                let w = if warn.is_empty() {
                    String::new()
                } else {
                    format!("; WARNING {warn}")
                };
                out.push(format!("{:14} login {}{}{}", "", text, exp, w));
            }
        }
    }
    format!("{}\n", out.join("\n"))
}

/// What `hive executor label NAME` says when no word follows.
const LABEL_USAGE: &str = "executor label NAME WORDS (a comma-separated list replaces the labels; +a,-b adds a and removes b; -b alone removes b, or -- -b when b is also a flag such as off)";

pub(crate) fn cmd_executor(c: &mut Context, fs: &FlagSet, args: &[String]) -> Result<()> {
    let s = c
        .store
        .clone()
        .ok_or_else(|| anyhow!("executor: store is needed"))?;
    let now = now_of(c);
    let Some(action) = args.first().map(String::as_str) else {
        return Err(crate::util::fail(
            2,
            "hive executor needs an action: list|add|probe|drain|activate|remove|label|reserve|login|token",
        )
        .into());
    };
    match action {
        "list" => {
            write!(c.out, "{}", executors_text(&s, &now))?;
            Ok(())
        }
        "token" => {
            let with_token = bool_flag(fs, "with-token");
            let remove = bool_flag(fs, "remove");
            let lines = match (with_token, remove) {
                (true, true) => {
                    return Err(
                        crate::util::fail(2, "executor token takes one of --with-token or --remove")
                            .into(),
                    );
                }
                (true, false) => {
                    use std::io::IsTerminal;
                    crate::token::executor_token_set(
                        &s,
                        &now,
                        &mut std::io::stdin(),
                        std::io::stdin().is_terminal(),
                        crate::util::HUMAN_AGENT,
                    )?
                }
                (false, true) => {
                    crate::token::executor_token_remove(&s, &now, crate::util::HUMAN_AGENT)?
                }
                (false, false) => crate::token::executor_token_report(&s, &now),
            };
            c.println(lines.join("\n"))?;
            Ok(())
        }
        _ if args.len() < 2 => Err(crate::util::fail(
            2,
            format!("executor {action} needs a NAME"),
        )
        .into()),
        "add" => {
            let name = &args[1];
            if str_flag(fs, "ssh").is_empty() || str_flag(fs, "workdir").is_empty() {
                return Err(crate::util::fail(
                    2,
                    "executor add needs --ssh user@host[:port] and --workdir /abs/path",
                )
                .into());
            }
            let labels = str_flag(fs, "labels");
            if !labels.is_empty() {
                hive_store::needs::parse(&labels).map_err(|e| crate::util::fail(2, e))?;
            }
            let mut r = crate::executor::add_executor(
                &s,
                &now,
                name,
                &str_flag(fs, "ssh"),
                &str_flag(fs, "workdir"),
                int_flag(fs, "workers"),
                &str_flag(fs, "projects"),
                &actor_of(c),
            )?;
            if !labels.is_empty() {
                crate::executor::set_labels(&s, &now, &r.str("name"), &labels, &actor_of(c))?;
                r = crate::executor::executor_row(&s, &r.str("name"));
            }
            c.printf(format_args!(
                "added {} ({}, {}; offers {}); it is drained until `hive executor probe {}` passes\n",
                r.str("name"),
                r.str("address"),
                r.str("workdir"),
                crate::executor::caps_text(&r),
                r.str("name")
            ))?;
            Ok(())
        }
        "label" => {
            let name = &args[1];
            if args.len() < 3 {
                return Err(crate::util::fail(2, LABEL_USAGE).into());
            }
            // Every word after NAME is part of the change: `+a -b` reads
            // as `+a,-b`.
            let change = args[2..].join(",");
            crate::executor::set_labels(&s, &now, name, &change, &actor_of(c))?;
            let r = crate::executor::executor_row(&s, name);
            c.printf(format_args!(
                "{name} offers {}\n",
                crate::executor::caps_text(&r)
            ))?;
            Ok(())
        }
        "reserve" => {
            let name = &args[1];
            let on = !bool_flag(fs, "off");
            crate::executor::set_reserved(&s, &now, name, on, &actor_of(c))?;
            if on {
                c.printf(format_args!(
                    "{name} reserved: it takes only the jobs that need something it offers\n"
                ))?;
            } else {
                c.printf(format_args!("{name} takes any job it can run again\n"))?;
            }
            Ok(())
        }
        "probe" => {
            let name = &args[1];
            let r = crate::executor::executor_row(&s, name);
            let deep =
                bool_flag(fs, "login") || (!crate::util::is_nil(&r) && crate::executor::login_drained(&r));
            let (mut ok, mut note) = crate::executor::probe_executor(&s, name, &actor_of(c), &now)?;
            let r = crate::executor::executor_row(&s, name);
            let status = if crate::util::is_nil(&r) {
                String::new()
            } else {
                r.str("status")
            };
            if ok {
                c.printf(format_args!("{name} ok, {status}: {note}\n"))?;
            } else {
                c.printf(format_args!("{name} FAILED ({status}): {note}\n"))?;
            }
            if ok && deep {
                (ok, note) =
                    crate::executor::login_check(&s, name, &actor_of(c), &now, true)?;
                let r = crate::executor::executor_row(&s, name);
                let state = if ok { "ok" } else { "FAILED" };
                c.printf(format_args!(
                    "{name} login check {state} ({}): {note}\n",
                    r.str("status")
                ))?;
            }
            if !ok {
                return Err(hive_cli::errorf(1, "", "", "").into());
            }
            Ok(())
        }
        "drain" => {
            let name = &args[1];
            crate::executor::set_executor_status(
                &s,
                &now,
                name,
                "drained",
                &actor_of(c),
                Some("drained by the human"),
            )?;
            c.printf(format_args!(
                "{name} drained: running jobs finish, no new job starts there\n"
            ))?;
            Ok(())
        }
        "activate" => {
            let name = &args[1];
            let r = crate::executor::executor_row(&s, name);
            if crate::util::is_nil(&r)
                && !crate::util::is_nil(&crate::util::must_one(
                    &s.conn()?,
                    "SELECT 1 FROM executors WHERE name = ? AND status = 'removed'",
                    &[name],
                ))
            {
                crate::util::exec1(
                    &s.conn()?,
                    "UPDATE executors SET status = 'drained', probe_note = '', errors = 0, backoff_minutes = 0, next_probe_at = '', healed_at = '' WHERE name = ?",
                    &[name],
                );
                crate::util::event_at(
                    &s.conn()?,
                    &now,
                    &actor_of(c),
                    "executor.revived",
                    name,
                    serde_json::Value::Null,
                );
                let (ok, note) = crate::executor::probe_executor(&s, name, &actor_of(c), &now)?;
                if !ok {
                    return Err(crate::util::fail(
                        1,
                        format!(
                            "{name} is back but its probe failed ({note}); fix that, then `hive executor probe {name}` and activate again"
                        ),
                    )
                    .into());
                }
            }
            let r = crate::executor::executor_row(&s, name);
            if crate::util::is_nil(&r) {
                return Err(crate::util::fail(2, format!("No executor {name}")).into());
            }
            if r.str("last_probe_at").is_empty() || !r.str("probe_note").starts_with("ok") {
                let mut note = r.str("probe_note");
                if note.is_empty() {
                    note = "never probed".into();
                }
                return Err(crate::util::fail(
                    2,
                    format!("Probe {name} first; its last probe did not pass: {note}"),
                )
                .into());
            }
            crate::executor::set_executor_status(&s, &now, name, "active", &actor_of(c), None)?;
            c.printf(format_args!("{name} active\n"))?;
            Ok(())
        }
        "remove" => {
            let name = &args[1];
            let _ = hive_board::remove_executor(&s, name, &actor_of(c), false)?;
            c.printf(format_args!(
                "{name} removed; the machine, its key, and its login are yours to retire\n"
            ))?;
            Ok(())
        }
        "login" => {
            let name = &args[1];
            let with_token = bool_flag(fs, "with-token");
            let remove = bool_flag(fs, "remove");
            if str_flag(fs, "admin").is_empty() {
                return Err(
                    crate::util::fail(2, "executor login needs --admin admin@host[:port]").into(),
                );
            }
            if with_token && remove {
                return Err(crate::util::fail(
                    2,
                    "executor login needs exactly one of --with-token or --remove",
                )
                .into());
            }
            let mut empty = std::io::empty();
            let mut stdin = std::io::stdin();
            let token: &mut dyn Read = if with_token { &mut stdin } else { &mut empty };
            let rep = crate::executor::executor_login(
                &s,
                &now,
                name,
                &str_flag(fs, "admin"),
                token,
                false,
                remove,
                crate::util::HUMAN_AGENT,
            )?;
            c.println(rep.lines.join("\n"))?;
            if !rep.ok {
                return Err(hive_cli::errorf(1, "", "", "").into());
            }
            Ok(())
        }
        _ => Err(crate::util::fail(
            2,
            format!(
                "hive executor: unknown action {action:?} (list|add|probe|drain|activate|remove|login|token)"
            ),
        )
        .into()),
    }
}

/// `hive store archive [--dry-run]`: the monthly archive of
/// hive-store archive.rs, by hand.
pub(crate) fn cmd_store(c: &mut Context, fs: &FlagSet, args: &[String]) -> Result<()> {
    let s = c
        .store
        .clone()
        .ok_or_else(|| anyhow!("store: store is needed"))?;
    if args.len() != 1 || args[0] != "archive" {
        return Err(crate::util::fail(2, "usage: hive store archive [--dry-run] [--json]").into());
    }
    let rep = s.archive_old_rows(bool_flag(fs, "dry-run"), &actor_of(c))?;
    if c.json {
        hive_cli::write_json(&mut c.out, &rep)?;
    } else {
        writeln!(c.out, "{}", rep.line())?;
        if rep.skipped.is_empty() && !rep.steps_cutoff.is_empty() && rep.steps_cutoff != rep.cutoff
        {
            writeln!(
                c.out,
                "harness steps of sessions ended before {}",
                rep.steps_cutoff
            )?;
        }
    }
    Ok(())
}

pub(crate) fn cmd_env(c: &mut Context, fs: &FlagSet, args: &[String]) -> Result<()> {
    let s = c
        .store
        .clone()
        .ok_or_else(|| anyhow!("env: store is needed"))?;
    let action = args.first().map(String::as_str).unwrap_or("check");
    match action {
        "check" => {
            let rep =
                crate::env::env_check(&s, c.project.clone(), str_flag(fs, "executor"), now_of(c))?;
            let Some(rep) = rep else {
                return Ok(());
            };
            if c.json {
                hive_cli::write_json(&mut c.out, &rep)?;
            } else {
                write!(c.out, "{}", crate::env::env_check_text(&rep))?;
            }
            // The same line the compute flow refuses with: where a linux
            // hands binary comes from on this host, or what to install.
            writeln!(c.out, "{}", crate::pair::hands_binary_plan_line("amd64"))?;
            if !rep.unfit.is_empty() {
                return Err(hive_cli::errorf(1, "", "", "").into());
            }
            Ok(())
        }
        "show" => {
            let project = crate::env::current_project(&s, c.project.clone())?;
            let (text, source, ok) = crate::env::env_show(&s, project.clone());
            if !ok {
                return Err(crate::util::fail(
                    3,
                    format!(
                        "{} has no environment recipe (`hive env set --file F` or `hive env import` stores one)",
                        project.str("slug")
                    ),
                )
                .into());
            }
            if c.json {
                let (recipe, problems) = crate::recipe::parse_recipe(&text);
                hive_cli::write_json(
                    &mut c.out,
                    &json!({"project": project.str("slug"), "source": source, "text": text, "recipe": recipe, "problems": problems}),
                )?;
                return Ok(());
            }
            writeln!(c.err, "# {}: {}", project.str("slug"), source)?;
            write!(c.out, "{text}")?;
            if !text.ends_with('\n') {
                writeln!(c.out)?;
            }
            Ok(())
        }
        "set" => {
            let project = crate::env::current_project(&s, c.project.clone())?;
            let file = str_flag(fs, "file");
            let text = if file.is_empty() || file == "-" {
                let mut text = String::new();
                std::io::stdin().read_to_string(&mut text)?;
                text
            } else {
                std::fs::read_to_string(&file)
                    .map_err(|e| crate::util::fail(2, format!("Cannot read the recipe: {e}")))?
            };
            if text.trim().is_empty() {
                return Err(crate::util::fail(
                    2,
                    "The recipe is empty; pass --file F or the text on stdin",
                )
                .into());
            }
            let (_id, rev, recipe) = crate::env::env_set(
                &s,
                project.clone(),
                text,
                actor_of(c),
                "store".into(),
                "hive env set".into(),
            )?;
            let summary = recipe
                .as_ref()
                .map(|r| crate::recipe::recipe_summary(r))
                .unwrap_or_default();
            c.printf(format_args!(
                "CTX-environment rev {rev} for {}: {summary}\n",
                project.str("slug")
            ))?;
            Ok(())
        }
        "import" => {
            let project = crate::env::current_project(&s, c.project.clone())?;
            let (id, rev, recipe) = crate::env::env_import(&s, project.clone(), actor_of(c))?;
            let summary = recipe
                .as_ref()
                .map(|r| crate::recipe::recipe_summary(r))
                .unwrap_or_default();
            c.printf(format_args!(
                "{id} rev {rev} for {} imported from {}: {summary}\n",
                project.str("slug"),
                crate::recipe::RECIPE_PATH
            ))?;
            Ok(())
        }
        _ => Err(crate::util::fail(
            2,
            format!("hive env: unknown action {action:?} (show|set|import|check)"),
        )
        .into()),
    }
}

pub(crate) fn cmd_hands(c: &mut Context, _fs: &FlagSet, args: &[String]) -> Result<()> {
    let out = std::mem::replace(&mut c.out, Box::new(std::io::sink()));
    let err = std::mem::replace(&mut c.err, Box::new(std::io::sink()));
    let mut h = crate::hands::new_hands(Box::new(std::io::stdin()), out, err);
    let res = h.hands_main(args);
    crate::hands::hands_outcome(res, &mut h.stderr)
}

pub(crate) fn cmd_pair(c: &mut Context, fs: &FlagSet, args: &[String]) -> Result<()> {
    if args.len() != 1 {
        return Err(crate::util::fail(
            2,
            "hive pair needs the admin ssh address (admin@host[:port]) and --name",
        )
        .into());
    }
    if str_flag(fs, "name").is_empty() {
        return Err(crate::util::fail(2, "hive pair needs --name (the executor's name)").into());
    }
    let s = c
        .store
        .clone()
        .ok_or_else(|| anyhow!("pair: store is needed"))?;
    let rep = crate::pair::pair_executor(
        &s,
        args[0].clone(),
        str_flag(fs, "name"),
        now_of(c),
        crate::pair::PairOpts {
            workdir: str_flag(fs, "workdir"),
            workers: int_flag(fs, "workers"),
            projects: str_flag(fs, "projects"),
            dry_run: bool_flag(fs, "dry-run") || bool_flag(fs, "inspect"),
            actor: actor_of(c),
            ..Default::default()
        },
    )?
    .ok_or_else(|| anyhow!("pair did not return a report"))?;
    c.println(rep.lines.join("\n"))?;
    if !rep.ok {
        return Err(hive_cli::errorf(1, "", "", "").into());
    }
    Ok(())
}

pub(crate) fn cmd_onboard(c: &mut Context, fs: &FlagSet, args: &[String]) -> Result<()> {
    let path = args.first().cloned().unwrap_or_else(|| ".".into());
    let s = hive_store::Store::open(hive_store::Options {
        home: c.home.clone(),
        create: true,
        now: c.now.clone(),
        ..Default::default()
    })?;
    let lines = crate::onboard::onboard_project(
        &s,
        path,
        now_of(c),
        crate::onboard::OnboardOpts {
            name: str_flag(fs, "name"),
            remote: str_flag(fs, "remote"),
            verify_cmd: str_flag(fs, "verify"),
            brain: bool_flag(fs, "brain"),
            actor: actor_of(c),
        },
    )?;
    c.println(lines.join("\n"))?;
    Ok(())
}

pub(crate) fn as_hands_error(err: &anyhow::Error, target: &mut serde_json::Value) -> bool {
    if let Some(e) = err.downcast_ref::<crate::hands::HandsError>() {
        *target = json!({"msg": e.msg});
        true
    } else {
        false
    }
}

pub(crate) fn as_hands_exit(err: &anyhow::Error, target: &mut serde_json::Value) -> bool {
    if let Some(e) = err.downcast_ref::<crate::hands::HandsExit>() {
        *target = json!({"code": e.code});
        true
    } else {
        false
    }
}

pub(crate) fn cmd_compute_providers(c: &mut Context) -> Result<()> {
    let names = crate::provider::provider_names();
    let chosen = c
        .store
        .as_ref()
        .map(|s| s.setting("compute.provider", ""))
        .unwrap_or_default();
    if c.json {
        let detail = names.iter().map(|n| {
            let p = crate::provider::provider_named(n);
            json!({"name": n, "kind": p.as_ref().map(|p| p.kind()).unwrap_or_default(), "auth": PROVIDER_AUTH.get(n.as_str()).copied().unwrap_or(""), "size_setting": hive_store::compute_size_setting(n), "chosen": *n == chosen})
        }).collect::<Vec<_>>();
        hive_cli::write_json(
            &mut c.out,
            &json!({"providers": names, "detail": detail, "setting": chosen}),
        )?;
        return Ok(());
    }
    if names.is_empty() {
        c.printf(format_args!("no compute provider is built in\n"))?;
        return Ok(());
    }
    for n in names {
        let mark = if n == chosen {
            "  (setting compute.provider)"
        } else {
            ""
        };
        let kind = crate::compute::provider_named(&n)
            .map(|p| p.kind())
            .unwrap_or_default();
        c.printf(format_args!(
            "{:<10} {}; auth: {}; size: {}{}\n",
            n,
            kind,
            PROVIDER_AUTH.get(n.as_str()).copied().unwrap_or(""),
            hive_store::compute_size_setting(&n),
            mark
        ))?;
    }
    Ok(())
}

pub(crate) static PROVIDER_AUTH: LazyLock<BTreeMap<&'static str, &'static str>> =
    LazyLock::new(|| {
        BTreeMap::from([
            ("azure", "az login"),
            ("aws", "aws configure or aws sso login"),
            ("gcp", "gcloud auth login"),
            ("multipass", "multipassd running locally"),
        ])
    });

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{join, new_bare_repo, new_store, recipe_with, run};
    use crate::test_helpers_provider::invoke;

    #[test]
    fn commands_registered_in_runner_group() {
        let _serial = crate::test_helpers::serial();
        register();
        for n in [
            "plan", "work", "land", "daemon", "service", "bulk", "executor", "env", "hands",
            "pair", "onboard", "pipeline",
        ] {
            let cmd = hive_cli::lookup(n).unwrap_or_else(|| panic!("{n} not registered"));
            assert_eq!(cmd.group, "runner", "{n} group");
        }
        let lead = hive_cli::lookup("lead").expect("lead registered");
        assert_eq!(
            lead.group, "agents",
            "lead sits with task, msg and schedule"
        );
        let h = hive_cli::lookup("hands").expect("hands registered");
        assert!(
            h.no_store && h.raw,
            "hands must run without a store and with raw args"
        );
        let o = hive_cli::lookup("onboard").expect("onboard registered");
        assert!(o.no_store, "onboard opens its own store with Create");
    }

    /// The summary of `hive update` says what it reads while the setting
    /// is empty, as the command does for the owner, and what it does where
    /// a package manager installed hive (release_update.rs).
    #[test]
    fn update_summary_says_what_an_empty_setting_and_a_package_manager_mean() {
        let _serial = crate::test_helpers::serial();
        register();
        let update = hive_cli::lookup("update").expect("update registered");
        assert_eq!(
            update.summary,
            "install the newer Hive release update.latest_url names, or Hive's own latest release while that setting is empty (checksum-verified); --check only says whether one exists. A hive that a package manager such as Homebrew installed is left to it: the command names that manager's upgrade command"
        );
    }

    #[test]
    fn work_plan_land_commands() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let (_row, dir) = new_bare_repo(&s);
        join(&s, "aurora");
        let (out, err_text, code) = invoke(&s, "work", "proj", &[]);
        assert!(
            code == 3 && out.starts_with("idle: "),
            "idle work: {code} {out:?} {err_text:?}"
        );
        let (out, _, code) = invoke(&s, "plan", "proj", &[]);
        assert_eq!(code, 0, "plan: {code} {out:?}");
        crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "t".into(),
            "do".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .expect("add job");
        let (out, _, code) = invoke(&s, "work", "proj", &["--dry-run"]);
        assert!(
            code == 0 && out.starts_with("dry-run ") && out.contains(" by aurora on local: "),
            "dry-run work: {code} {out:?}"
        );
        let (out, _, code) = invoke(
            &s,
            "work",
            "proj",
            &["--worker-cmd", "sh -c 'echo SUMMARY: fine'"],
        );
        assert!(
            code == 0 && out.starts_with("done "),
            "work: {code} {out:?}"
        );
        let (out, _, code) = invoke(&s, "land", "proj", &[]);
        assert!(
            code == 0 && out.trim() == "proj: nothing waiting",
            "land: {code} {out:?}"
        );
        let (out, _, code) = invoke(&s, "land", "proj", &["--dry-run"]);
        assert!(
            code == 0
                && out
                    == "proj: no upstream, finished branches land locally on main\n  nothing waiting for proj\n",
            "land --dry-run: {code} {out:?}"
        );
        assert!(
            !std::path::Path::new(&dir).join("HIVE_RESULT.md").exists(),
            "result file in checkout"
        );
    }

    #[test]
    fn executor_and_env_commands() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        new_bare_repo(&s);
        let (out, _, code) = invoke(&s, "executor", "", &["list"]);
        assert!(
            code == 0
                && out.starts_with(crate::executor::LOCAL_EXECUTOR)
                && out.contains("this machine"),
            "list: {code} {out:?}"
        );
        let (_, err_text, code) = invoke(&s, "executor", "", &["add", "hands"]);
        assert!(
            code == 2 && err_text.contains("--ssh"),
            "add without flags: {code} {err_text:?}"
        );
        let (out, _, code) = invoke(
            &s,
            "executor",
            "",
            &[
                "add",
                "hands",
                "--ssh",
                "hive@hands",
                "--workdir",
                "/srv/hive",
                "--workers",
                "2",
            ],
        );
        assert!(
            code == 0
                && out.contains("added hands (hive@hands, /srv/hive; offers linux); it is drained until `hive executor probe hands` passes"),
            "add: {code} {out:?}"
        );
        let (_, err_text, code) = invoke(&s, "executor", "", &["activate", "hands"]);
        assert!(
            code == 2 && err_text.contains("Probe hands first"),
            "activate unprobed: {code} {err_text:?}"
        );
        let (out, _, code) = invoke(&s, "executor", "", &["drain", "hands"]);
        assert!(
            code == 0 && out.contains("hands drained"),
            "drain: {code} {out:?}"
        );
        let (out, _, _) = invoke(&s, "executor", "", &["list"]);
        assert!(
            out.contains("hands")
                && out.contains("workdir /srv/hive")
                && out.contains("probe never: "),
            "list: {out:?}"
        );
        let (out, _, code) = invoke(&s, "env", "proj", &["check"]);
        assert!(
            code == 0 && out.contains("no environment recipe"),
            "env check: {code} {out:?}"
        );
        let (_, _err_text, code) = invoke(&s, "env", "proj", &["show"]);
        assert_eq!(code, 3, "env show without a recipe");
        let env_dir = tempfile::tempdir().expect("env temp");
        let f = env_dir.path().join("env.md");
        std::fs::write(
            &f,
            recipe_with(Some(&["node@22.1.0"]), None, None, None, "").as_bytes(),
        )
        .expect("write recipe");
        let (out, err_text, code) = invoke(
            &s,
            "env",
            "proj",
            &["set", "--file", &f.display().to_string()],
        );
        assert!(
            code == 0 && out.starts_with("CTX-environment rev 1 for proj: tools node@22.1.0"),
            "env set: {code} {out:?} {err_text:?}"
        );
        let (out, _, code) = invoke(&s, "env", "proj", &["show"]);
        assert!(
            code == 0 && out.contains("node@22.1.0"),
            "env show: {code} {out:?}"
        );
        let (out, _, code) = invoke(&s, "env", "proj", &["check"]);
        assert!(
            out.contains("recipe ok (CTX-environment); tools node@22.1.0"),
            "env check: {code} {out:?}"
        );
        // The doctor line the suggestion asks for: the same sentence the
        // compute request refusal carries, so a missing toolchain shows here
        // instead of inside a failed CR.
        assert!(
            out.contains("hands binary linux/amd64: built with cargo"),
            "env check hands line: {out:?}"
        );
        let (out, _, code) = invoke(&s, "executor", "", &["remove", "hands"]);
        assert!(
            code == 0
                && out.contains(
                    "hands removed; the machine, its key, and its login are yours to retire"
                ),
            "remove: {code} {out:?}"
        );
        assert!(
            crate::executor::executor_row(&s, "hands").is_empty(),
            "still there"
        );
    }

    /// `hive executor ARGS` with the arguments read the way the binary reads
    /// them: flags and words may mix (`hive_cli::flags::parse_interleaved`).
    fn executor(s: &hive_store::Store, args: &[&str]) -> (String, String, i32) {
        register();
        let cmd = hive_cli::lookup("executor").expect("executor registered");
        let mut fs = FlagSet::new("executor");
        fs.set_output(hive_cli::flags::Output::Discard);
        (cmd.flags.expect("executor flags"))(&mut fs);
        let argv: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        let positional = match hive_cli::flags::parse_interleaved(&mut fs, &argv) {
            Ok(positional) => positional,
            Err(e) => return (String::new(), format!("hive executor: {e}"), 2),
        };
        let out = hive_cli::Buffer::default();
        let err = hive_cli::Buffer::default();
        let mut c = Context {
            store: Some(s.clone()),
            home: s.home.clone(),
            out: Box::new(out.clone()),
            err: Box::new(err.clone()),
            json: false,
            quiet: false,
            color: false,
            agent: String::new(),
            project: String::new(),
            session: String::new(),
            now: crate::test_helpers::TEST_NOW.to_string(),
            args: positional.clone(),
        };
        let code = match (cmd.run)(&mut c, &fs, &positional) {
            Ok(()) => 0,
            Err(e) => match e.downcast_ref::<hive_cli::Error>() {
                Some(ce) => {
                    let _ = write!(c.err, "{ce}");
                    ce.exit_code()
                }
                None => {
                    let _ = write!(c.err, "{e}");
                    1
                }
            },
        };
        (out.string(), err.string(), code)
    }

    #[test]
    fn executor_label_reads_a_leading_dash_as_a_word_to_remove() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let labels = || crate::executor::executor_row(&s, "gpu-box").str("labels");
        let (out, err_text, code) = executor(
            &s,
            &[
                "add",
                "gpu-box",
                "-ssh",
                "hive@gpu-box",
                "-workdir",
                "/srv/hive",
                "-labels",
                "gpu,cuda,android",
            ],
        );
        assert_eq!(code, 0, "add: {out:?} {err_text:?}");
        assert_eq!(labels(), "gpu,cuda,android");

        // Each form, and what the machine's own words are after it.
        for (args, want, offers) in [
            // a word with a dash in front is removed
            (vec!["-cuda"], "gpu,android", "linux android gpu"),
            // as it is after `--`
            (vec!["--", "-android"], "gpu", "linux gpu"),
            // a signed list adds and removes
            (vec!["+cuda,-gpu"], "cuda", "linux cuda"),
            // one that begins with a dash, too
            (vec!["-cuda,+a"], "a", "linux a"),
            // signed words as arguments of their own
            (vec!["+b", "-a"], "b", "linux b"),
            // a plain list replaces the words
            (vec!["x,y"], "x,y", "linux x y"),
            (vec!["z", "y"], "z,y", "linux y z"),
            // removing a word that is not there changes nothing
            (vec!["-cuda"], "z,y", "linux y z"),
        ] {
            let mut full = vec!["label", "gpu-box"];
            full.extend(args.iter().copied());
            let (out, err_text, code) = executor(&s, &full);
            assert_eq!(code, 0, "{args:?}: {err_text:?}");
            assert_eq!(out, format!("gpu-box offers {offers}\n"), "{args:?}");
            assert_eq!(labels(), want, "{args:?}");
        }

        // The flags of hive executor are still flags, with one dash or two,
        // in label's place and in their own.
        let usage = "executor label NAME WORDS (a comma-separated list replaces the labels; +a,-b adds a and removes b; -b alone removes b, or -- -b when b is also a flag such as off)";
        for args in [vec!["label", "gpu-box"], vec!["label", "gpu-box", "-off"]] {
            let (out, err_text, code) = executor(&s, &args);
            assert_eq!((code, out.as_str(), err_text.as_str()), (2, "", usage));
        }
        let (out, _, code) = executor(&s, &["label", "gpu-box", "--off", "-y"]);
        assert_eq!((code, out.as_str()), (0, "gpu-box offers linux z\n"));
        let (out, _, code) = executor(&s, &["reserve", "gpu-box"]);
        assert!(code == 0 && out.starts_with("gpu-box reserved"), "{out:?}");
        let (out, _, code) = executor(&s, &["reserve", "gpu-box", "-off"]);
        assert_eq!(
            (code, out.as_str()),
            (0, "gpu-box takes any job it can run again\n")
        );

        // What is no word stays what it was: an undefined flag, or a word
        // the labels do not take.
        for args in [
            vec!["label", "gpu-box", "--cuda"],
            vec!["label", "-cuda"],
            vec!["list", "-cuda"],
            vec!["drain", "gpu-box", "-cuda"],
        ] {
            let (out, err_text, code) = executor(&s, &args);
            assert_eq!(
                (code, out.as_str(), err_text.as_str()),
                (2, "", "hive executor: flag provided but not defined: -cuda"),
                "{args:?}"
            );
        }
        let (_, err_text, code) = executor(&s, &["label", "gpu-box", "gpu", "-z"]);
        assert!(
            code == 2 && err_text.starts_with("`-z` is not a need"),
            "{code} {err_text:?}"
        );
        assert_eq!(labels(), "z");
    }

    /// The heartbeat row a running daemon leaves, naming `pid`.
    fn beat_for(s: &hive_store::Store, pid: u32, note: &str, at: &str) {
        crate::daemon::meta_upsert(
            s,
            "daemon",
            &json!({"pid": pid, "at": at, "loops": 3, "passes": 1, "interval": 60, "note": note}),
        );
    }

    fn daemon_flagset(args: &[&str]) -> FlagSet {
        let mut fs = FlagSet::new("daemon");
        daemon_flags(&mut fs);
        fs.parse(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
            .expect("daemon flags");
        fs
    }

    #[test]
    fn daemon_refuses_stray_arguments_with_its_usage() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let (out, err_text, code) = invoke(&s, "daemon", "", &["start", "--once"]);
        assert!(
            code == 2
                && err_text.contains("takes no arguments (got start)")
                && err_text.contains(DAEMON_USAGE),
            "stray argument: {code} {out:?} {err_text:?}"
        );
        let (raw, exists) = s.meta("daemon").unwrap();
        assert!(!exists, "a refused daemon wrote a heartbeat: {raw}");
    }

    #[test]
    fn daemon_refuses_a_second_live_instance_unless_forced() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let mut other = std::process::Command::new("/bin/sleep")
            .arg("60")
            .spawn()
            .expect("spawn sleep");
        let pid = other.id();
        beat_for(&s, pid, "idle", &s.now());
        let (out, err_text, code) = invoke(&s, "daemon", "", &["--once", "--dry-run"]);
        assert!(
            code == 3
                && err_text.contains(&format!("already running on this store (pid {pid}"))
                && err_text.contains("--force"),
            "second daemon: {code} {out:?} {err_text:?}"
        );
        let (raw, _) = s.meta("daemon").unwrap();
        assert!(
            raw.contains(&format!("\"pid\":{pid}")),
            "the refused daemon overwrote the live one's heartbeat: {raw}"
        );
        assert!(
            daemon_start_refusal(&s, &daemon_flagset(&["--force"]), &[]).is_none(),
            "--force still refused"
        );
        // The final beats and a stale beat do not hold the store.
        for note in ["exited", "restarting"] {
            beat_for(&s, pid, note, &s.now());
            assert_eq!(live_daemon_pid(&s), None, "note {note}");
        }
        beat_for(&s, pid, "idle", "2000-01-01T00:00:00Z");
        assert_eq!(live_daemon_pid(&s), None, "a stale beat held the store");
        // Our own pid is never another daemon.
        beat_for(&s, std::process::id(), "idle", &s.now());
        assert_eq!(live_daemon_pid(&s), None, "own pid");
        // A dead pid frees the store.
        beat_for(&s, pid, "idle", &s.now());
        assert_eq!(live_daemon_pid(&s).map(|(p, _)| p), Some(pid as i32));
        other.kill().expect("kill sleep");
        other.wait().expect("reap sleep");
        assert_eq!(live_daemon_pid(&s), None, "a dead pid held the store");
        assert!(daemon_start_refusal(&s, &daemon_flagset(&[]), &[]).is_none());
    }

    #[test]
    fn service_command_show() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let (out, _, code) = invoke(&s, "service", "", &["show", "--interval", "45"]);
        assert!(
            code == 0
                && out.contains("daemon")
                && (out.contains("<plist") || out.contains("[Service]")),
            "service show: {code} {out:?}"
        );
        let (_, path) = crate::service::service_paths(String::new(), String::new());
        assert!(
            !(std::path::Path::new(&path).exists() && out.contains("wrote")),
            "show installed something"
        );
    }

    #[test]
    fn pair_command_refusals() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let (_, err_text, code) = invoke(&s, "pair", "", &["admin@host"]);
        assert!(
            code == 2 && err_text.contains("--name"),
            "pair without name: {code} {err_text:?}"
        );
        let (_, err_text, code) = invoke(&s, "pair", "", &["admin@host", "--name", "local"]);
        assert!(
            code == 2 && err_text.contains("Executor name"),
            "pair local: {code} {err_text:?}"
        );
    }

    #[test]
    fn worker_commit_lands_locally_when_the_project_has_no_upstream() {
        let _serial = crate::test_helpers::serial();
        if !std::path::Path::new("/usr/bin/ssh-keygen").exists()
            && !std::path::Path::new("/usr/local/bin/ssh-keygen").exists()
        {
            eprintln!("slow worker test skipped: no ssh-keygen");
            return;
        }
        let s = new_store();
        let (_row, dir) = new_bare_repo(&s);
        join(&s, "aurora");
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "add a file".into(),
            "add f.txt".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .expect("add job");
        let (out, err_text, code) = invoke(
            &s,
            "work",
            "proj",
            &[
                "--worker-cmd",
                "sh -c 'echo hi > f.txt && git add f.txt && git commit -q -m \"add f\" && echo SUMMARY: added f'",
            ],
        );
        assert!(
            code == 0
                && out.starts_with("done ")
                && out.contains(&format!("branch hive/aurora/{}", job.str("id"))),
            "work: {code} {out:?} {err_text:?}"
        );
        let branches = run(&dir, &["git", "branch", "--list", "hive/*"]);
        assert!(
            branches.contains(&format!("hive/aurora/{}", job.str("id"))),
            "branch not pushed to the checkout: {branches:?}"
        );
        // The project has no upstream to open a pull request on: plain
        // `hive land` lands the branch here, as before.
        let (out, err_text, code) = invoke(&s, "land", "proj", &[]);
        assert!(
            code == 0
                && out.starts_with("proj: no upstream, landing locally\n")
                && out.contains("proj: landed "),
            "land: {code} {out:?} {err_text:?}"
        );
        assert!(
            std::path::Path::new(&dir).join("f.txt").exists(),
            "f.txt not on main after landing"
        );
        let (out, _, code) = invoke(&s, "land", "proj", &[]);
        assert!(
            code == 0 && out.trim() == "proj: nothing waiting",
            "land again: {code} {out:?}"
        );
    }

    #[test]
    fn land_branch_by_hand_replays_a_job_branch_forked_before_main_was_squashed() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let (_row, repo) = new_bare_repo(&s);
        let fork = run(&repo, &["git", "rev-parse", "main"]).trim().to_string();
        let branch = "hive/aurora/job-squashed";
        run(&repo, &["git", "checkout", "-q", "-b", branch, "main"]);
        std::fs::write(
            std::path::Path::new(&repo).join("README.md"),
            "# job change\n",
        )
        .unwrap();
        run(&repo, &["git", "commit", "-q", "-am", "job change"]);
        run(&repo, &["git", "checkout", "-q", "main"]);
        // The owner squashes main to a new root commit after the job's run forked.
        std::fs::write(std::path::Path::new(&repo).join("squash.txt"), "squash\n").unwrap();
        run(&repo, &["git", "add", "squash.txt"]);
        let tree = run(&repo, &["git", "write-tree"]).trim().to_string();
        let squashed = run(&repo, &["git", "commit-tree", &tree, "-m", "Squash"])
            .trim()
            .to_string();
        run(&repo, &["git", "reset", "-q", "--hard", &squashed]);
        let (job, _) = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "change the readme".into(),
            "change the readme".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .expect("add job");
        let conn = s.conn().unwrap();
        conn.execute(
            "UPDATE jobs SET status = 'done', branch = ?, finished_at = ? WHERE id = ?",
            rusqlite::params![branch, s.now(), job.str("id")],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO runs (id, job_id, agent, project, started_at, finished_at, status, base) VALUES ('run-squashed', ?, 'aurora', 'proj', ?, ?, 'done', ?)",
            rusqlite::params![job.str("id"), s.now(), s.now(), fork],
        )
        .unwrap();
        drop(conn);

        let (out, err_text, code) =
            invoke(&s, "land", "proj", &["--branch", branch, "--no-verify"]);
        assert!(
            code == 0
                && out.contains("proj landed: ")
                && out.contains("replayed from its run base"),
            "land --branch: {code} {out:?} {err_text:?}"
        );
        assert_eq!(
            run(&repo, &["git", "show", "main:README.md"]),
            "# job change\n"
        );
        assert_eq!(run(&repo, &["git", "show", "main:squash.txt"]), "squash\n");
        let reachable = std::process::Command::new("git")
            .args(["-C", &repo, "merge-base", "--is-ancestor", &fork, "main"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .unwrap()
            .success();
        assert!(!reachable, "the squashed-away history came back into main");
    }

    #[test]
    fn land_branch_proposes_a_pull_request_and_local_is_the_way_round_it() {
        let _serial = crate::test_helpers::serial();
        let _env = hive_testutil::env_lock();
        let gh = crate::test_helpers_daemon::fake_gh();
        let s = new_store();
        let (_proj, repo, bare) = crate::test_helpers_daemon::push_fixture(&s);
        run(&repo, &["git", "checkout", "-q", "-b", "feat/one", "main"]);
        std::fs::write(std::path::Path::new(&repo).join("one.txt"), "one\n").unwrap();
        run(&repo, &["git", "add", "-A"]);
        run(&repo, &["git", "commit", "-q", "-m", "one"]);
        run(&repo, &["git", "checkout", "-q", "main"]);
        let tip = run(&repo, &["git", "rev-parse", "feat/one"])
            .trim()
            .to_string();

        let (out, err_text, code) = invoke(&s, "land", "proj", &["--branch", "feat/one"]);
        assert!(
            code == 0
                && out.trim()
                    == "proposed pr proj feat/one: #7 opened https://github.com/o/r/pull/7",
            "land --branch: {code} {out:?} {err_text:?}"
        );
        assert_eq!(
            crate::test_helpers_daemon::bare_ref(&bare, "refs/heads/feat/one"),
            tip
        );
        assert_eq!(gh.head_of(7), "feat/one");
        assert!(
            !std::path::Path::new(&repo).join("one.txt").exists(),
            "nothing merged locally"
        );
        // `--propose` is what `--branch` does now; it still parses.
        let (out, _, code) = invoke(&s, "land", "proj", &["--branch", "feat/one", "--propose"]);
        assert!(
            code == 0 && out.contains("is delivered at"),
            "land --propose: {code} {out:?}"
        );
        let (out, _, code) = invoke(&s, "land", "proj", &["--dry-run"]);
        assert!(
            code == 0 && out.starts_with("proj: pull requests go to origin/main ("),
            "land --dry-run: {code} {out:?}"
        );

        // `--local` merges here and says what that costs.
        run(
            &repo,
            &["git", "checkout", "-q", "-b", "hive/hand/two", "main"],
        );
        std::fs::write(std::path::Path::new(&repo).join("two.txt"), "two\n").unwrap();
        run(&repo, &["git", "add", "-A"]);
        run(&repo, &["git", "commit", "-q", "-m", "two"]);
        run(&repo, &["git", "checkout", "-q", "main"]);
        let (out, err_text, code) = invoke(
            &s,
            "land",
            "proj",
            &["--branch", "hive/hand/two", "--local", "--no-verify"],
        );
        assert!(
            code == 0
                && out.starts_with("proj: merging hive/hand/two into main here; the local main will be ahead of its upstream")
                && (out.contains("proj landed: ") || out.contains("proj merged: ")),
            "land --local: {code} {out:?} {err_text:?}"
        );
        assert!(std::path::Path::new(&repo).join("two.txt").exists());
        let (_, err_text, code) = invoke(&s, "land", "proj", &["--local"]);
        assert!(
            code == 2 && err_text.contains("land --local needs --branch"),
            "{code} {err_text:?}"
        );
    }
}
