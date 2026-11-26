use anyhow::Error as AnyError;
use hive_cli::flags::{FlagError, FlagSet, Output, parse_interleaved};
use hive_cli::{Command, Context};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};

// The build-info block `build.rs` stamps this binary with (`BUILD_INFO_TEXT`). It is generated
// here, in the leaf crate, so that a move of the brain's HEAD recompiles this crate and relinks
// instead of rebuilding the eighteen crates that depend on `hive-cli`; `run_streaming` hands
// it to `hive_cli::install_build_info`. Taking a `&'static str` to the constant keeps the block
// in rodata as one literal, which the pair-hands inspection greps out of a hand's binary.
include!(concat!(env!("OUT_DIR"), "/build_info_const.rs"));

// The version decision `build.rs` makes for that block. The build script compiles the file
// itself; here it is compiled for its tests only.
#[cfg(test)]
mod version_stamp;

/// One writer shared between the runner and the command's `Context`. Lines
/// stream through as they are written, which `hive daemon` and `hive hub`
/// need for their service logs.
///
/// It also remembers whether the reader went away (a write or flush failed
/// with `BrokenPipe`), so `hive ... | head -n 1` can end quietly once `head`
/// has read its line instead of reporting the closed pipe as a failure.
#[derive(Clone)]
struct Shared {
    inner: Arc<Mutex<Box<dyn Write + Send>>>,
    closed: Arc<AtomicBool>,
}

impl Shared {
    fn new(w: Box<dyn Write + Send>) -> Self {
        Shared {
            inner: Arc::new(Mutex::new(w)),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// A write or flush through this writer found its reader gone.
    fn reader_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    fn note<T>(&self, r: std::io::Result<T>) -> std::io::Result<T> {
        if let Err(e) = &r
            && e.kind() == std::io::ErrorKind::BrokenPipe
        {
            self.closed.store(true, Ordering::Relaxed);
        }
        r
    }
}

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let r = self
            .inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .write(buf);
        self.note(r)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let r = self.inner.lock().unwrap_or_else(|e| e.into_inner()).flush();
        self.note(r)
    }
}

/// The command failed only because its standard output was closed under it:
/// the reader (`head`, a pager quit early) took what it wanted and left. That
/// is how a pipeline ends, not a fault, so nothing is reported. A broken pipe
/// anywhere else (a child process's stdin, a socket) still is.
fn stopped_by_closed_stdout(out: &Shared, e: &AnyError) -> bool {
    out.reader_closed()
        && e.chain().any(|c| {
            c.downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::BrokenPipe)
        })
}

#[derive(Default)]
struct Globals {
    home: String,
    project: String,
    agent: String,
    session: String,
    now: String,
    json: bool,
    quiet: bool,
    no_color: bool,
}

impl Globals {
    fn bind(&self, fs: &mut FlagSet) {
        fs.string_var(
            "home",
            &std::env::var("HIVE_HOME").unwrap_or_default(),
            "Hive home (default $HIVE_HOME or ~/.hive)",
        );
        fs.string_var(
            "project",
            &std::env::var("HIVE_PROJECT").unwrap_or_default(),
            "project slug (default $HIVE_PROJECT, else the checkout you are in)",
        );
        fs.string_var(
            "agent",
            &std::env::var("HIVE_AGENT").unwrap_or_default(),
            "the agent you act as (default $HIVE_AGENT)",
        );
        fs.string_var(
            "session",
            &std::env::var("HIVE_SESSION").unwrap_or_default(),
            "the session you act in (default $HIVE_SESSION)",
        );
        fs.string_var(
            "now",
            &std::env::var("HIVE_NOW").unwrap_or_default(),
            "fixed UTC clock like 2026-09-06T10:00:00Z (tests)",
        );
        fs.bool_var("json", false, "machine-readable output");
        fs.bool_var("quiet", false, "print only what a script needs");
        fs.bool_var("no-color", false, "no colour even on a terminal");
    }

    fn apply(&mut self, fs: &FlagSet) {
        self.home = fs.get_string("home");
        self.project = fs.get_string("project");
        self.agent = fs.get_string("agent");
        self.session = fs.get_string("session");
        self.now = fs.get_string("now");
        self.json = fs.get_bool("json");
        self.quiet = fs.get_bool("quiet");
        self.no_color = fs.get_bool("no-color");
    }
}

static REGISTER: Once = Once::new();

/// Registers every crate's commands (and the model providers) once. Order
/// does not matter; the help output sorts by group and name.
pub fn register_all() {
    REGISTER.call_once(|| {
        // Store events feed the process bus from the start.
        hive_bus::install_store_observer();
        register_version();
        hive_ask::register();
        hive_board::register();
        hive_cred_command::register();
        hive_evolve::register();
        hive_goals_command::register();
        hive_harness::register();
        hive_hooks::register();
        hive_hub::register();
        hive_inspect::register();
        hive_mcp::register();
        hive_provider_copilot::register();
        hive_provider_openai::register();
        hive_provider_anthropic::register();
        hive_provider_gemini::register();
        hive_records::register();
        hive_runner::register();
        hive_tally::register();
        hive_tasks::register();
        hive_voice::register();
        hive_evolve::init();
    });
}

pub fn register_version() {
    hive_cli::register(Command {
        name: "version".into(),
        group: "inspect".into(),
        summary: "print the hive version".into(),
        usage: "hive version [--json]".into(),
        aliases: Vec::new(),
        no_store: true,
        read_only: false,
        raw: false,
        run: version_run,
        flags: None,
    });
}

fn version_run(c: &mut Context, _fs: &FlagSet, _args: &[String]) -> anyhow::Result<()> {
    let version = hive_cli::version();
    // What the daemon's daily update check last found (TASK-087), read from
    // the store without creating or migrating it; no store, no line.
    let update = version_update_state(&c.home);
    let line = update
        .as_ref()
        .and_then(|st| hive_cli::update::notice(&version, st));
    if c.json {
        let update = update.map(|st| {
            serde_json::json!({
                "latest": st.latest,
                "url": st.url,
                "checked_at": st.checked_at,
                "newer": st.newer_than(&version),
                "notice": line,
            })
        });
        let mut doc = serde_json::json!({
            "version": version,
            "revision": hive_cli::revision(),
            "update": update,
        });
        // Only where a package manager installed this program
        // (hive_cli::managed): who, and the command that upgrades it.
        if let Some(m) = hive_cli::managed::this_program() {
            doc["managed_by"] = serde_json::json!(m.owner());
            doc["upgrade_command"] = serde_json::json!(m.upgrade);
        }
        hive_cli::write_json(c.out.as_mut(), &doc)?;
    } else {
        writeln!(c.out, "{version}")?;
        if let Some(line) = line {
            writeln!(c.out, "{line}")?;
        }
    }
    Ok(())
}

/// The update check's state in the store under `home`, opened read-only so
/// `hive version` never creates, migrates or writes a store.
fn version_update_state(home: &str) -> Option<hive_cli::update::UpdateState> {
    if home.is_empty()
        || !std::path::Path::new(home)
            .join(hive_store::STORE_NAME)
            .exists()
    {
        return None;
    }
    let s = hive_store::Store::open(hive_store::Options {
        home: home.to_string(),
        create: false,
        read_only: true,
        skip_records: true,
        now: String::new(),
    })
    .ok()?;
    hive_cli::update::stored_state(&s)
}

/// What the caller knows about the terminal the command runs at. A
/// `dyn Write` cannot answer for itself, and a test has no terminal, so the
/// binary's entry says what the process has and a test says what it wants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Terminal {
    /// Standard input is a terminal.
    pub stdin: bool,
    /// Standard output is a terminal.
    pub stdout: bool,
}

impl Terminal {
    /// The terminal of this process's own standard input and output.
    pub fn of_process() -> Self {
        use std::io::IsTerminal;
        Terminal {
            stdin: std::io::stdin().is_terminal(),
            stdout: std::io::stdout().is_terminal(),
        }
    }

    /// A person types here and reads here: both ends are terminals.
    fn attended(self) -> bool {
        self.stdin && self.stdout
    }
}

/// Runs the CLI for callers that lend borrowed writers (tests). Output is
/// collected and copied out at the end. The command runs at no terminal.
pub fn run(argv: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    run_at(argv, Terminal::default(), out, err)
}

/// [`run`] at the terminal the caller names.
fn run_at(argv: &[String], terminal: Terminal, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let out_buf = hive_cli::Buffer::new();
    let err_buf = hive_cli::Buffer::new();
    let code = run_streaming(
        argv,
        Box::new(out_buf.clone()),
        Box::new(err_buf.clone()),
        terminal,
    );
    let _ = write!(out, "{}", out_buf.string());
    let _ = write!(err, "{}", err_buf.string());
    code
}

/// The binary's entry: writes stream straight through to `out`/`err`.
/// `terminal` says whether standard input and `out` are terminals.
pub fn run_streaming(
    argv: &[String],
    out: Box<dyn Write + Send>,
    err: Box<dyn Write + Send>,
    terminal: Terminal,
) -> i32 {
    let out_is_tty = terminal.stdout;
    // The stamp goes in before any command runs: `hive version`, the daemon's heartbeat, the
    // hub's running view and the self-update smoke test all read it through `hive_cli`.
    hive_cli::install_build_info(BUILD_INFO_TEXT);
    register_all();
    // Whether a package manager installed this program is read now and
    // remembered: its upgrade removes this version's files, the marker
    // among them, while a daemon or a console started from them still
    // runs (hive_cli::managed).
    let _ = hive_cli::managed::this();
    // launchd starts a service with 256 open files allowed; a hub holding a
    // provider and every MCP server per conversation runs out of them and
    // then cannot even spawn git. The ceiling goes as high as the system
    // allows before any command runs (hive_cli::limits).
    let _ = hive_cli::limits::raise_open_files();
    let mut out = Shared::new(out);
    let mut err = Shared::new(err);
    let out = &mut out;
    let err = &mut err;
    let mut gfs = FlagSet::new("hive");
    gfs.set_output(Output::Discard);
    Globals::default().bind(&mut gfs);
    if let Err(e) = gfs.parse(argv) {
        return match e {
            FlagError::Help => {
                let help_color = color_enabled(out_is_tty, false);
                let _ = hive_cli::help(out, help_color);
                0
            }
            FlagError::Message(m) => {
                let _ = writeln!(err, "hive: {m}\n  fix: hive help");
                2
            }
        };
    }
    let mut g = Globals::default();
    g.apply(&gfs);
    let project_given = gfs.is_set("project");
    let color = color_enabled(out_is_tty, g.no_color);
    let rest = gfs.args();
    if rest.is_empty() {
        let _ = hive_cli::help(out, color);
        return 0;
    }
    let name = &rest[0];
    let rest = &rest[1..];
    if name == "help" || name == "-h" || name == "--help" {
        return help(rest, out, err, color);
    }
    let Some(cmd) = hive_cli::lookup(name) else {
        return unknown(name, err);
    };
    let home = match hive_store::resolve_home(&g.home) {
        Ok(h) => h,
        Err(e) => return report(name, e, err),
    };

    let mut ctx = Context {
        store: None,
        home,
        out: Box::new(out.clone()),
        err: Box::new(err.clone()),
        json: g.json,
        quiet: g.quiet,
        color,
        agent: g.agent,
        project: g.project.clone(),
        session: g.session,
        now: g.now,
        args: Vec::new(),
    };

    let mut fs = FlagSet::new(name);
    fs.set_output(Output::Discard);
    if let Some(bind) = cmd.flags {
        bind(&mut fs);
    }
    let args = if cmd.raw {
        // raw commands parse their own flags; no late globals
        rest.to_vec()
    } else {
        // the global flags are accepted after the command word too, so
        // `hive post --project X ...` works as well as `hive --project X post ...`
        late_bind(&mut fs);
        let args = match parse_interleaved(&mut fs, rest) {
            Ok(a) => a,
            Err(FlagError::Help) => {
                let _ = hive_cli::help_command(out, color, &cmd);
                return 0;
            }
            Err(FlagError::Message(m)) => {
                let _ = writeln!(err, "hive {name}: {m}\n  fix: hive help {name}");
                return 2;
            }
        };
        // a --project written before the command word reaches the command as
        // a set flag too, so a command that tells an explicit project apart
        // from the ambient one ($HIVE_PROJECT) sees both spellings alike
        if project_given && !fs.is_set("project") {
            let _ = fs.set("project", &g.project);
        }
        apply_late(&fs, &mut ctx);
        args
    };
    ctx.args = args.clone();
    if name == "post" {
        for flag in ["summary", "body"] {
            if fs.is_set(flag)
                && let Some(what) = shell_mangled_flag(&fs.get_string(flag))
            {
                let _ = writeln!(err, "{}", shell_mangled_warning(flag, what));
            }
        }
    }

    if !cmd.no_store {
        let open = |create: bool| {
            hive_store::Store::open(hive_store::Options {
                home: g.home.clone(),
                create,
                read_only: cmd.read_only,
                skip_records: cmd.read_only,
                now: ctx.now.clone(),
            })
        };
        let mut opened = open(name == "init");
        // The first `hive hub` a person starts on a machine makes the store
        // itself, the way `hive init --store-only` does, and carries on.
        if cmd.name == "hub"
            && opened.as_ref().is_err_and(is_no_store)
            && hive_hub::makes_first_store(&fs, terminal.attended(), &ctx.home)
        {
            opened = open(true);
            if let Ok(s) = &opened {
                let _ = writeln!(out, "{}", first_store_line(&s.path));
            }
        }
        match opened {
            Ok(s) => ctx.store = Some(s),
            Err(e) => return report(name, e, err),
        }
        beat_as_agent(&ctx, cmd.read_only);
        release_prompt(&ctx, name, cmd.read_only, err);
    }
    let result = (cmd.run)(&mut ctx, &fs, &args);
    if !cmd.no_store {
        beat_as_agent(&ctx, cmd.read_only);
    }
    match result {
        Ok(()) => 0,
        // the command asked for its own help: print it, exit 0
        Err(e) if matches!(e.downcast_ref::<FlagError>(), Some(FlagError::Help)) => {
            let _ = hive_cli::help_command(out, color, &cmd);
            0
        }
        Err(e) if stopped_by_closed_stdout(out, &e) => 0,
        Err(e) => report(name, e, err),
    }
}

/// Names what makes a `--summary` or `--body` value look like the shell
/// mangled it before `hive post` saw it: a suspiciously short value (a
/// double-quoted `` `phrase` `` substituted away leaves a word or two), an
/// unmatched backtick (one half of the pair survived), or an unmatched `<`
/// (an unquoted redirection or a `<tag>` the shell swallowed). Fenced code
/// is skipped since a ```` ``` ```` fence is legitimately odd.
fn shell_mangled_flag(text: &str) -> Option<&'static str> {
    const SHORT: usize = 3;
    let trimmed = text.trim();
    if !trimmed.is_empty() && trimmed.chars().count() < SHORT {
        return Some("a suspiciously short value");
    }
    let mut in_fence = false;
    let mut ticks = 0usize;
    let mut angle = 0i64;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        ticks += line.matches('`').count();
        for ch in line.chars() {
            match ch {
                '<' => angle += 1,
                '>' if angle > 0 => angle -= 1,
                _ => {}
            }
        }
    }
    if ticks % 2 == 1 {
        return Some("an unmatched backtick");
    }
    if angle > 0 {
        return Some("an unmatched '<'");
    }
    None
}

/// The stderr line `hive post` prints when `--summary` or `--body` carries
/// `what` (see `shell_mangled_flag`); it points at the safe spellings.
fn shell_mangled_warning(flag: &str, what: &str) -> String {
    format!(
        "warning: hive post --{flag} has {what}; the shell may have mangled it before hive saw it\n  fix: pipe the body on stdin with a heredoc (hive post ... <<'EOF') or pass --body-file FILE"
    )
}

/// "A newer Hive vX is available - run: hive update" on stderr, once per
/// release (TASK-090): read from the store, no network. `hive update`
/// says it itself, and a read-only command cannot record that the line was
/// shown, so neither prints it. It is the owner's line: a command an agent
/// or a job runs neither prints nor uses it up. Never fails the command.
fn release_prompt(ctx: &Context, name: &str, read_only: bool, err: &mut dyn Write) {
    if read_only
        || name == "update"
        || !ctx.agent.is_empty()
        || std::env::var("HIVE_JOB").is_ok_and(|j| !j.is_empty())
    {
        return;
    }
    let Some(s) = ctx.store.as_ref() else {
        return;
    };
    if let Some(line) = hive_runner::release_update::release_prompt(s) {
        let _ = writeln!(err, "{line}");
    }
}

/// Every command run as an agent is a sign of life: the agent's heartbeat is
/// refreshed around the command, and `heartbeat.dead_after` seconds of silence
/// make the agent dead. It never fails the command, and a read-only command
/// opens a writable store only when the beat is actually due.
fn beat_as_agent(ctx: &Context, read_only: bool) {
    let Some(s) = ctx.store.as_ref() else {
        return;
    };
    if ctx.agent.is_empty() || !hive_board::beat_due(s, &ctx.agent) {
        return;
    }
    if read_only {
        if let Ok(w) = hive_store::Store::open(hive_store::Options {
            home: ctx.home.clone(),
            create: false,
            read_only: false,
            skip_records: true,
            now: ctx.now.clone(),
        }) {
            let _ = hive_board::beat_agent(&w, &ctx.agent, "cli");
        }
    } else {
        let _ = hive_board::beat_agent(s, &ctx.agent, "cli");
    }
}

fn color_enabled(out_is_tty: bool, no_color: bool) -> bool {
    if no_color || std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    out_is_tty
}

fn late_bind(fs: &mut FlagSet) {
    if fs.lookup("project").is_none() {
        fs.string_var("project", "", "project slug");
    }
    if fs.lookup("agent").is_none() {
        fs.string_var("agent", "", "the agent you act as");
    }
    if fs.lookup("session").is_none() {
        fs.string_var("session", "", "the session you act in");
    }
    if fs.lookup("now").is_none() {
        fs.string_var("now", "", "fixed UTC clock");
    }
    if fs.lookup("json").is_none() {
        fs.bool_var("json", false, "machine-readable output");
    }
    if fs.lookup("quiet").is_none() {
        fs.bool_var("quiet", false, "print only what a script needs");
    }
    if fs.lookup("no-color").is_none() {
        fs.bool_var("no-color", false, "no colour");
    }
}

fn apply_late(fs: &FlagSet, ctx: &mut Context) {
    if fs.is_set("project") && !fs.get_string("project").is_empty() {
        ctx.project = fs.get_string("project");
    }
    if fs.is_set("agent") && !fs.get_string("agent").is_empty() {
        ctx.agent = fs.get_string("agent");
    }
    if fs.is_set("session") && !fs.get_string("session").is_empty() {
        ctx.session = fs.get_string("session");
    }
    if fs.is_set("now") && !fs.get_string("now").is_empty() {
        ctx.now = fs.get_string("now");
    }
    if fs.get_bool("json") {
        ctx.json = true;
    }
    if fs.get_bool("quiet") {
        ctx.quiet = true;
    }
    if fs.get_bool("no-color") {
        ctx.color = false;
    }
}

fn help(args: &[String], out: &mut dyn Write, err: &mut dyn Write, color: bool) -> i32 {
    if args.is_empty() {
        let _ = hive_cli::help(out, color);
        return 0;
    }
    let Some(cmd) = hive_cli::lookup(&args[0]) else {
        return unknown(&args[0], err);
    };
    let _ = hive_cli::help_command(out, color, &cmd);
    0
}

fn unknown(name: &str, err: &mut dyn Write) -> i32 {
    let _ = writeln!(err, "hive: unknown command {name:?}");
    let s = hive_cli::suggest(name, 3);
    if !s.is_empty() {
        let _ = writeln!(err, "  did you mean: {}", s.join(", "));
    }
    let _ = writeln!(err, "  fix: hive help");
    2
}

fn is_no_store(e: &AnyError) -> bool {
    e.downcast_ref::<hive_store::NotFoundError>().is_some()
}

/// What `hive hub` prints, before its serving lines, when it made the store.
fn first_store_line(path: &str) -> String {
    format!("No store yet: made one at {path}")
}

fn report(name: &str, errv: AnyError, err: &mut dyn Write) -> i32 {
    if let Some(ce) = errv.downcast_ref::<hive_cli::Error>() {
        if !ce.what.is_empty() {
            let _ = writeln!(err, "{ce}");
        }
        return ce.exit_code();
    }
    if let Some(nf) = errv.downcast_ref::<hive_store::NotFoundError>() {
        // A worker (HIVE_JOB set) may never create a store, so telling it
        // to run `hive init` would send it in a circle.
        let in_worker = std::env::var("HIVE_JOB").is_ok_and(|v| !v.is_empty());
        let e = if in_worker {
            hive_cli::errorf(
                2,
                &format!("No store at {}", nf.path),
                "this command runs inside a Hive job (HIVE_JOB is set), and workers never create a store",
                "point HIVE_HOME at the owner's existing store, or run `hive init <project-path>` outside a job",
            )
        } else {
            hive_cli::errorf(
                2,
                &format!("No store at {}", nf.path),
                "this machine has no Hive store yet",
                hive_cli::NO_STORE_FIX,
            )
        };
        let _ = writeln!(err, "{e}");
        return 2;
    }
    if let Some(input) = errv.downcast_ref::<hive_store::InputError>() {
        let _ = writeln!(err, "hive {name}: {}", input.msg);
        return 2;
    }
    let _ = writeln!(err, "hive {name}: {errv}");
    1
}

#[cfg(test)]
pub(crate) mod test_helpers {
    use std::sync::{Mutex, MutexGuard};

    static SERIAL: Mutex<()> = Mutex::new(());

    /// The command registry, the process env and the captured `SEEN` context
    /// are shared between tests, so every test holds this lock first.
    pub(crate) fn serial() -> MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hive_testutil::setenv;

    #[test]
    fn shell_mangled_flag_names_the_pattern() {
        assert_eq!(shell_mangled_flag("ok"), Some("a suspiciously short value"));
        assert_eq!(
            shell_mangled_flag("fix the `run loop"),
            Some("an unmatched backtick")
        );
        assert_eq!(
            shell_mangled_flag("see <evidence-file"),
            Some("an unmatched '<'")
        );
        assert_eq!(shell_mangled_flag("fix the `run` loop <a> b"), None);
        assert_eq!(shell_mangled_flag("```\n` <\n```\nfine"), None);
        assert_eq!(shell_mangled_flag(""), None);
        let warning = shell_mangled_warning("body", "an unmatched backtick");
        assert!(warning.starts_with("warning: hive post --body has an unmatched backtick"));
        assert!(warning.contains("<<'EOF'") && warning.contains("--body-file"));
    }
    use std::path::Path;
    use std::sync::Mutex;
    use tempfile::tempdir;

    fn sv(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    fn run(args: &[&str], out: &mut Vec<u8>, err: &mut Vec<u8>) -> i32 {
        super::run(&sv(args), out, err)
    }

    fn s(b: &[u8]) -> String {
        String::from_utf8(b.to_vec()).unwrap()
    }

    /// A standard output whose reader has gone: every write is `BrokenPipe`.
    struct ClosedPipe;

    impl Write for ClosedPipe {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
    }

    #[test]
    fn closed_stdout_ends_the_command_quietly() {
        let _serial = crate::test_helpers::serial();
        // `hive version | head -c 0`: the reader is gone before the first line.
        let err = hive_cli::Buffer::new();
        let code = run_streaming(
            &sv(&["version"]),
            Box::new(ClosedPipe),
            Box::new(err.clone()),
            Terminal::default(),
        );
        assert_eq!(code, 0, "stderr: {:?}", err.string());
        assert_eq!(err.string(), "", "a closed pipe is not reported");
    }

    #[test]
    fn broken_pipe_elsewhere_is_still_reported() {
        // stdout is fine: a broken pipe to a child or a socket is a real fault.
        let out = Shared::new(Box::new(Vec::new()));
        let pipe: AnyError = std::io::Error::from(std::io::ErrorKind::BrokenPipe).into();
        assert!(!stopped_by_closed_stdout(&out, &pipe));
        // stdout closed, but the command failed for another reason.
        let mut closed = Shared::new(Box::new(ClosedPipe));
        assert!(writeln!(closed, "x").is_err());
        let other = anyhow::anyhow!("no such job");
        assert!(!stopped_by_closed_stdout(&closed, &other));
        assert!(stopped_by_closed_stdout(&closed, &pipe.context("print")));
    }

    #[test]
    fn help_and_version() {
        let _serial = crate::test_helpers::serial();
        let (mut out, mut errb) = (Vec::new(), Vec::new());
        let code = run(&["help"], &mut out, &mut errb);
        if code != 0 || !s(&out).starts_with(hive_cli::TITLE) {
            panic!("help: {code} {:?}", s(&out));
        }
        out.clear();
        let code = run(&[], &mut out, &mut errb);
        if code != 0 || !s(&out).contains("Runner - jobs, workers and landing") {
            panic!("bare: {code} {:?}", s(&out));
        }
        out.clear();
        let code = run(&["-h"], &mut out, &mut errb);
        if code != 0 || !s(&out).starts_with(hive_cli::TITLE) {
            panic!("-h: {code}");
        }
        out.clear();
        let code = run(&["help", "work"], &mut out, &mut errb);
        if code != 0 || !s(&out).contains("Usage: hive work") {
            panic!("help work: {code} {:?}", s(&out));
        }
        out.clear();
        let code = run(&["version"], &mut out, &mut errb);
        if code != 0 || s(&out).trim().is_empty() {
            panic!("version: {code} {:?}", s(&out));
        }
        // `run` installs the stamp build.rs generated before it dispatches, so the command
        // prints the stamped version, and in a git checkout the stamp names the revision.
        assert!(hive_cli::read_build_info().is_some(), "no stamp installed");
        assert_eq!(s(&out).lines().next().unwrap_or(""), hive_cli::version());
        if hive_testutil::manifest_dir(env!("CARGO_MANIFEST_DIR"))
            .join("../../.git")
            .exists()
        {
            let rev = hive_cli::revision();
            assert!(
                rev.len() == 40 && rev.chars().all(|c| c.is_ascii_hexdigit()),
                "revision {rev:?}"
            );
        }
        out.clear();
        let code = run(&["--json", "version"], &mut out, &mut errb);
        if code != 0 || !s(&out).contains("\"version\":") {
            panic!("version --json: {code} {:?}", s(&out));
        }
    }

    /// The stamp `build.rs` gave this very build. Whatever release tags the
    /// checkout holds, the build is not behind the release its `Cargo.toml`
    /// names, so `hive update` never offers a build the release it was built
    /// from (`version_stamp::source_version`).
    #[test]
    fn this_build_is_stamped_no_older_than_the_workspace_version() {
        let info = hive_cli::parse_build_info(BUILD_INFO_TEXT).expect("the stamp parses");
        let stamped = hive_cli::build_version(&info);
        let workspace = format!("v{}", env!("CARGO_PKG_VERSION"));
        // `dev`, the stamp of a build outside a git checkout, is behind nothing.
        if hive_cli::update::parse_semver(&stamped).is_some() {
            assert!(
                !hive_cli::update::is_newer(&workspace, &stamped),
                "stamped {stamped}, behind the workspace version {workspace}"
            );
        }
    }

    #[test]
    fn unknown_command() {
        let _serial = crate::test_helpers::serial();
        let (mut out, mut errb) = (Vec::new(), Vec::new());
        let home = tempdir().unwrap();
        let code = run(
            &["--home", home.path().to_str().unwrap(), "statuz"],
            &mut out,
            &mut errb,
        );
        if code != 2
            || !s(&errb).contains("unknown command \"statuz\"")
            || !s(&errb).contains("did you mean:")
        {
            panic!("code {code} stderr {:?}", s(&errb));
        }
    }

    // The `provider` setting offers exactly the providers the binary
    // registers, each with a console label, and every API-key provider
    // owns the endpoint setting its description promises.
    #[test]
    fn registered_providers_match_the_provider_setting_choices() {
        let _serial = crate::test_helpers::serial();
        register_all();
        let mut choices = hive_store::HARNESS_RULES["provider"].choices.clone();
        choices.sort();
        assert_eq!(hive_provider::names(), choices);
        assert!(choices.len() >= 17, "{choices:?}");
        for name in &choices {
            assert!(
                !hive_provider::label_of(name).is_empty(),
                "{name} has no label"
            );
            assert_eq!(
                hive_store::HARNESS_RULES.contains_key(&format!("provider.{name}.base_url")),
                name != "copilot",
                "{name} endpoint setting"
            );
        }
    }

    fn needs_store(_c: &mut Context, _fs: &FlagSet, _args: &[String]) -> anyhow::Result<()> {
        Ok(())
    }

    #[test]
    fn no_store_mapping() {
        let _serial = crate::test_helpers::serial();
        let _job = setenv("HIVE_JOB", "");
        let home = tempdir().unwrap();
        let home = home.path().to_str().unwrap();
        if hive_cli::lookup("zz-needs-store").is_none() {
            hive_cli::register(Command {
                name: "zz-needs-store".into(),
                group: "inspect".into(),
                summary: "test".into(),
                usage: String::new(),
                aliases: vec![],
                no_store: false,
                read_only: false,
                raw: false,
                run: needs_store,
                flags: None,
            });
        }
        let (mut out, mut errb) = (Vec::new(), Vec::new());
        let code = run(&["--home", home, "zz-needs-store"], &mut out, &mut errb);
        let want = hive_store::store_path(home).unwrap();
        if code != 2 || s(&errb) != no_store_text(&want) {
            panic!("code {code} stderr {:?}", s(&errb));
        }
        if Path::new(&want).exists() {
            panic!("store created by a non-init command");
        }
    }

    /// The whole "No store" error, as every command that needs a store prints it.
    fn no_store_text(store: &str) -> String {
        format!(
            "No store at {store}\n  why: this machine has no Hive store yet\n  fix: hive init --store-only (or hive init <project-path> to register a project as well)\n"
        )
    }

    const AT_A_TERMINAL: Terminal = Terminal {
        stdin: true,
        stdout: true,
    };

    fn run_at(terminal: Terminal, args: &[&str], out: &mut Vec<u8>, err: &mut Vec<u8>) -> i32 {
        super::run_at(&sv(args), terminal, out, err)
    }

    /// A port something else holds: a hub given it stops where it would
    /// start to serve, so a test that starts one never waits on it.
    fn taken_port() -> (std::net::TcpListener, String) {
        let holder = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = holder.local_addr().unwrap().port().to_string();
        (holder, port)
    }

    /// The first `hive hub` a person starts on a machine makes the store
    /// `hive init --store-only` makes, says so in one line and goes on to
    /// serve; the next one finds the store and says nothing about it.
    #[test]
    fn the_first_hub_at_a_terminal_makes_the_store_and_carries_on() {
        let _serial = crate::test_helpers::serial();
        let _job = setenv("HIVE_JOB", "");
        let _agent = setenv("HIVE_AGENT", "");
        let _no_color = setenv("NO_COLOR", "1");
        let dir = tempdir().unwrap();
        // The home is not there yet; the directory it goes in is.
        let home = dir.path().join("hive");
        let _home = setenv("HIVE_HOME", home.to_str().unwrap());
        let want = hive_store::store_path(home.to_str().unwrap()).unwrap();
        let (_holder, port) = taken_port();
        let hub = ["hub", "--no-browser", "--port", port.as_str()];

        let (mut out, mut errb) = (Vec::new(), Vec::new());
        let code = run_at(AT_A_TERMINAL, &hub, &mut out, &mut errb);
        assert_eq!(code, 1, "stdout {:?}, stderr {:?}", s(&out), s(&errb));
        assert!(
            s(&out).starts_with(&format!("No store yet: made one at {want}\n")),
            "{:?}",
            s(&out)
        );
        // It went on to serve: the port was the only thing in its way.
        assert!(
            s(&errb).starts_with("The hub could not listen\n") && !s(&errb).contains("No store"),
            "{:?}",
            s(&errb)
        );
        assert!(home.join(hive_store::STORE_NAME).is_file());
        // The store is the one `hive init --store-only` makes: no project in it.
        let (mut out, mut errb) = (Vec::new(), Vec::new());
        assert_eq!(
            run(&["--json", "projects"], &mut out, &mut errb),
            0,
            "{}",
            s(&errb)
        );
        let projects: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(projects.as_array().map(Vec::len), Some(0), "{projects}");

        let (mut out, mut errb) = (Vec::new(), Vec::new());
        let code = run_at(AT_A_TERMINAL, &hub, &mut out, &mut errb);
        assert_eq!(code, 1, "stdout {:?}, stderr {:?}", s(&out), s(&errb));
        assert!(!s(&out).contains("No store yet"), "{:?}", s(&out));
        assert!(
            s(&errb).starts_with("The hub could not listen\n"),
            "{:?}",
            s(&errb)
        );
    }

    /// Where a store may not be made, the hub that would make one says why,
    /// in the words `hive init --store-only` has for it.
    #[test]
    fn the_first_hub_says_why_it_cannot_make_a_store_inside_a_checkout() {
        let _serial = crate::test_helpers::serial();
        let _job = setenv("HIVE_JOB", "");
        let _agent = setenv("HIVE_AGENT", "");
        let _no_color = setenv("NO_COLOR", "1");
        let dir = tempdir().unwrap();
        // The least git takes for a repository.
        let checkout = dir.path().join("checkout");
        std::fs::create_dir_all(checkout.join(".git/objects")).unwrap();
        std::fs::create_dir_all(checkout.join(".git/refs")).unwrap();
        std::fs::write(checkout.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let home = checkout.join("home");
        let _home = setenv("HIVE_HOME", home.to_str().unwrap());
        let (_holder, port) = taken_port();
        let mut said = Vec::new();
        for (terminal, args) in [
            (
                AT_A_TERMINAL,
                vec!["hub", "--no-browser", "--port", port.as_str()],
            ),
            (Terminal::default(), vec!["init", "--store-only"]),
        ] {
            let (mut out, mut errb) = (Vec::new(), Vec::new());
            let code = run_at(terminal, &args, &mut out, &mut errb);
            assert_eq!(code, 2, "{args:?}: stdout {:?}", s(&out));
            let err = s(&errb);
            let (who, why) = err.split_once(": ").unwrap_or(("", err.as_str()));
            assert_eq!(who, format!("hive {}", args[0]), "{err:?}");
            assert!(
                why.starts_with("The store ") && why.contains(" must live outside the project "),
                "{err:?}"
            );
            assert!(out.is_empty() && !home.exists(), "{args:?}: {:?}", s(&out));
            said.push(why.to_string());
        }
        assert_eq!(said[0], said[1]);
    }

    /// Everywhere else a hub without a store ends as it always did: with the
    /// "No store" error and status 2, and nothing made on disk.
    #[test]
    fn a_hub_nobody_started_by_hand_makes_no_store() {
        let _serial = crate::test_helpers::serial();
        let _job = setenv("HIVE_JOB", "");
        let _agent = setenv("HIVE_AGENT", "");
        let _no_color = setenv("NO_COLOR", "1");
        let dir = tempdir().unwrap();
        let home = dir.path().join("hive");
        let _home = setenv("HIVE_HOME", home.to_str().unwrap());
        let want = hive_store::store_path(home.to_str().unwrap()).unwrap();
        let (_holder, port) = taken_port();
        let port = port.as_str();
        let no_terminal = Terminal::default();
        let only_output = Terminal {
            stdin: false,
            stdout: true,
        };
        let only_input = Terminal {
            stdin: true,
            stdout: false,
        };
        for (what, terminal, args) in [
            (
                "no terminal",
                no_terminal,
                vec!["hub", "--no-browser", "--port", port],
            ),
            (
                "input is not a terminal",
                only_output,
                vec!["hub", "--no-browser", "--port", port],
            ),
            (
                "output is not a terminal",
                only_input,
                vec!["hub", "--no-browser", "--port", port],
            ),
            (
                "the service's own hub",
                AT_A_TERMINAL,
                vec!["hub", "--no-browser", "--port", port, "--restart-on-change"],
            ),
            (
                "serve, which needs a project",
                AT_A_TERMINAL,
                vec!["serve", "--no-browser", "--port", port],
            ),
        ] {
            let (mut out, mut errb) = (Vec::new(), Vec::new());
            let code = run_at(terminal, &args, &mut out, &mut errb);
            assert_eq!(code, 2, "{what}: stdout {:?}", s(&out));
            assert_eq!(s(&errb), no_store_text(&want), "{what}");
            assert!(out.is_empty(), "{what}: {:?}", s(&out));
            assert!(!home.exists(), "{what}: the home was made");
        }

        // A home on a disk that is not mounted: neither it nor the directory
        // it goes in is there, and neither is made.
        let volume = dir.path().join("volume");
        let away = volume.join("hive");
        let _home = setenv("HIVE_HOME", away.to_str().unwrap());
        let want = hive_store::store_path(away.to_str().unwrap()).unwrap();
        let (mut out, mut errb) = (Vec::new(), Vec::new());
        let code = run_at(
            AT_A_TERMINAL,
            &["hub", "--no-browser", "--port", port],
            &mut out,
            &mut errb,
        );
        assert_eq!(code, 2, "stdout {:?}", s(&out));
        assert_eq!(s(&errb), no_store_text(&want));
        assert!(out.is_empty() && !volume.exists(), "{:?}", s(&out));

        // Inside a job nothing makes a store, a terminal or not.
        let _home = setenv("HIVE_HOME", home.to_str().unwrap());
        let _job = setenv("HIVE_JOB", "job-test");
        let (mut out, mut errb) = (Vec::new(), Vec::new());
        let code = run_at(
            AT_A_TERMINAL,
            &["hub", "--no-browser", "--port", port],
            &mut out,
            &mut errb,
        );
        assert_eq!(code, 2, "stdout {:?}", s(&out));
        assert!(
            s(&errb).contains("No store at") && s(&errb).contains("HIVE_JOB"),
            "{:?}",
            s(&errb)
        );
        assert!(out.is_empty() && !home.exists(), "{:?}", s(&out));
    }

    /// What the zz-post command saw: (Agent, Project, JSON, Now, Store.Now(), Home), args, kind.
    type Seen = Option<(
        (String, String, bool, String, Option<String>, String),
        Vec<String>,
        String,
    )>;
    static SEEN: Mutex<Seen> = Mutex::new(None);

    fn zz_post_flags(fs: &mut FlagSet) {
        fs.string_var("kind", "", "kind");
    }

    fn zz_post_run(c: &mut Context, fs: &FlagSet, args: &[String]) -> anyhow::Result<()> {
        *SEEN.lock().unwrap() = Some((
            (
                c.agent.clone(),
                c.project.clone(),
                c.json,
                c.now.clone(),
                c.store.as_ref().map(|st| st.now()),
                c.home.clone(),
            ),
            args.to_vec(),
            fs.get_string("kind"),
        ));
        if args.first().is_some_and(|a| a == "fail") {
            return Err(hive_cli::errorf(3, "boom", "because", "hive zz-post ok").into());
        }
        if args.first().is_some_and(|a| a == "input") {
            return c
                .store
                .as_ref()
                .expect("store")
                .set_setting("human", "quorum", "x");
        }
        Ok(())
    }

    #[test]
    fn interleaved_flags_and_errors() {
        let _serial = crate::test_helpers::serial();
        let _job = setenv("HIVE_JOB", "");
        let _no_color = setenv("NO_COLOR", "1");
        let home = tempdir().unwrap();
        let home = home.path().to_str().unwrap();
        hive_store::create(Path::new(home).join(hive_store::STORE_NAME)).unwrap();
        if hive_cli::lookup("zz-post").is_none() {
            hive_cli::register(Command {
                name: "zz-post".into(),
                group: "agents".into(),
                summary: "test".into(),
                usage: String::new(),
                aliases: vec![],
                no_store: false,
                read_only: false,
                raw: false,
                run: zz_post_run,
                flags: Some(zz_post_flags),
            });
        }

        let (mut out, mut errb) = (Vec::new(), Vec::new());
        let code = run(
            &[
                "--home",
                home,
                "--agent",
                "early",
                "zz-post",
                "one",
                "--kind",
                "finding",
                "two",
                "--project",
                "hive",
                "--json",
                "--now",
                "2026-09-06T10:00:00Z",
                "--",
                "-three",
            ],
            &mut out,
            &mut errb,
        );
        if code != 0 {
            panic!("code {code} stderr {:?}", s(&errb));
        }
        let (seen, seen_args, seen_kind) = SEEN.lock().unwrap().clone().unwrap();
        if seen_args.join(",") != "one,two,-three" || seen_kind != "finding" {
            panic!("args {seen_args:?} kind {seen_kind:?}");
        }
        let (agent, project, json, now, store_now, seen_home) = seen;
        if agent != "early"
            || project != "hive"
            || !json
            || now != "2026-09-06T10:00:00Z"
            || store_now.as_deref() != Some("2026-09-06T10:00:00Z")
        {
            panic!("context {agent:?} {project:?} {json} {now:?} {store_now:?}");
        }
        let base = Path::new(home).file_name().unwrap().to_str().unwrap();
        if !seen_home.ends_with(base) {
            panic!("home {seen_home:?}");
        }

        errb.clear();
        let code = run(&["--home", home, "zz-post", "fail"], &mut out, &mut errb);
        if code != 3 || s(&errb) != "boom\n  why: because\n  fix: hive zz-post ok\n" {
            panic!("cli.Error: {code} {:?}", s(&errb));
        }
        errb.clear();
        let code = run(&["--home", home, "zz-post", "input"], &mut out, &mut errb);
        if code != 2 || s(&errb) != "hive zz-post: quorum takes a whole number, not 'x'\n" {
            panic!("InputError: {code} {:?}", s(&errb));
        }
        errb.clear();
        let code = run(&["--home", home, "zz-post", "--bogus"], &mut out, &mut errb);
        if code != 2 || !s(&errb).contains("flag provided but not defined: -bogus") {
            panic!("bad flag: {code} {:?}", s(&errb));
        }
        out.clear();
        let code = run(&["--home", home, "zz-post", "-h"], &mut out, &mut errb);
        if code != 0 || !s(&out).contains("-kind string") {
            panic!("command -h: {code} {:?}", s(&out));
        }
    }

    /// `hive executor label NAME -word` through the whole command line: the
    /// word with a dash is removed, and the flags around it are still flags.
    #[test]
    fn executor_label_removes_a_word_written_with_a_leading_dash() {
        let _serial = crate::test_helpers::serial();
        let _job = setenv("HIVE_JOB", "");
        let _agent = setenv("HIVE_AGENT", "");
        let dir = tempdir().unwrap();
        let home = dir.path();
        let _home = setenv("HIVE_HOME", home.to_str().unwrap());
        hive_store::create(home.join(hive_store::STORE_NAME)).unwrap();
        let ok = |args: &[&str]| -> String {
            let (mut out, mut errb) = (Vec::new(), Vec::new());
            let code = run(args, &mut out, &mut errb);
            assert_eq!(code, 0, "{args:?}: {}", s(&errb));
            s(&out)
        };
        ok(&[
            "executor",
            "add",
            "gpu-box",
            "--ssh",
            "hive@gpu-box",
            "--workdir",
            "/srv/hive",
            "--labels",
            "gpu,cuda,android",
        ]);
        assert_eq!(
            ok(&["executor", "label", "gpu-box", "-cuda"]),
            "gpu-box offers linux android gpu\n"
        );
        // A global flag after the word is still a flag: nothing is printed.
        assert_eq!(ok(&["executor", "label", "gpu-box", "-gpu", "--quiet"]), "");
        assert_eq!(
            ok(&["executor", "label", "gpu-box", "--", "-android"]),
            "gpu-box offers linux\n"
        );
        assert_eq!(
            ok(&["executor", "label", "gpu-box", "+a,-b"]),
            "gpu-box offers linux a\n"
        );
        // Two dashes still ask for a flag, and there is none of that name.
        let (mut out, mut errb) = (Vec::new(), Vec::new());
        let code = run(
            &["executor", "label", "gpu-box", "--cuda"],
            &mut out,
            &mut errb,
        );
        assert_eq!(
            (code, s(&errb).as_str()),
            (
                2,
                "hive executor: flag provided but not defined: -cuda\n  fix: hive help executor\n"
            )
        );
    }

    /// `hive update` looks at Hive's own releases only for the owner. Under
    /// an agent's name (`--agent` or HIVE_AGENT) or inside a job, an empty
    /// `update.latest_url` is refused first. The store follows the source
    /// channel here, so no run of this test can reach the network: the owner
    /// is refused too, by the channel.
    #[test]
    fn update_with_no_latest_url_is_refused_for_an_agent_and_inside_a_job() {
        let _serial = crate::test_helpers::serial();
        let _job = setenv("HIVE_JOB", "");
        let _agent = setenv("HIVE_AGENT", "");
        let dir = tempdir().unwrap();
        let home = dir.path();
        let _home = setenv("HIVE_HOME", home.to_str().unwrap());
        hive_store::create(home.join(hive_store::STORE_NAME)).unwrap();
        let store = hive_store::Store::open(hive_store::Options {
            home: home.to_str().unwrap().to_string(),
            skip_records: true,
            ..Default::default()
        })
        .unwrap();
        store
            .set_setting("human", hive_cli::update::CHANNEL_SETTING, "source")
            .unwrap();
        let update = |args: &[&str]| -> (i32, String, String) {
            let (mut out, mut errb) = (Vec::new(), Vec::new());
            let code = run(args, &mut out, &mut errb);
            (code, s(&out), s(&errb))
        };
        let empty = "hive update: update.latest_url is empty, so Hive does not know where its releases are; set it first: hive settings set update.latest_url https://api.github.com/repos/OWNER/REPO/releases/latest\n";
        let refused = (1, String::new(), empty.to_string());

        assert_eq!(update(&["--agent", "aurora", "update"]), refused);
        {
            let _agent = setenv("HIVE_AGENT", "aurora");
            assert_eq!(update(&["update"]), refused);
        }
        {
            let _job = setenv("HIVE_JOB", "job-test");
            assert_eq!(update(&["update"]), refused);
        }
        // The owner is past that refusal, named or not.
        for args in [vec!["update"], vec!["--agent", "human", "update"]] {
            let (code, out, err) = update(&args);
            assert_eq!((code, out.as_str()), (1, ""), "{args:?}");
            assert!(
                err.starts_with("hive update: update.channel resolves to source here"),
                "{args:?}: {err:?}"
            );
        }
    }

    #[test]
    fn settings_help_without_store() {
        let _serial = crate::test_helpers::serial();
        let _job = setenv("HIVE_JOB", "");
        let dir = tempdir().unwrap();
        let home = dir.path().join("missing");
        let home_s = home.to_str().unwrap().to_string();
        let _home = setenv("HIVE_HOME", &home_s);
        let mut first = String::new();
        for args in [
            vec!["settings", "help"],
            vec!["settings", "--help"],
            vec!["settings", "-h"],
            vec!["help", "settings"],
        ] {
            let (mut out, mut errb) = (Vec::new(), Vec::new());
            let code = run(&args, &mut out, &mut errb);
            if code != 0 || !errb.is_empty() {
                panic!(
                    "{}: help: code {code}, stdout {:?}, stderr {:?}",
                    args.join(" "),
                    s(&out),
                    s(&errb)
                );
            }
            for want in [
                "Usage: hive settings [get NAME | set NAME VALUE | list | help]".to_string(),
                "minutes_per_job".to_string(),
                format!(
                    "default {} minutes",
                    hive_store::DEFAULT_SETTINGS["minutes_per_job"]
                ),
                "harness.max_steps".to_string(),
                format!(
                    "default {} model calls",
                    hive_store::HARNESS_SETTINGS["harness.max_steps"]
                ),
                "evolve.limits".to_string(),
                format!("default {}", hive_store::EVOLVE_SETTINGS["evolve.limits"]),
                "evolve_minutes and evolve_credits are not supported".to_string(),
                "not aliases".to_string(),
                "hive settings get NAME".to_string(),
                "hive settings list --json".to_string(),
            ] {
                if !s(&out).contains(&want) {
                    panic!("{}: help missing {want:?}:\n{}", args.join(" "), s(&out));
                }
            }
            if first.is_empty() {
                first = s(&out);
            } else if s(&out) != first {
                panic!(
                    "{}: help routes disagree:\n{}\nwant:\n{first}",
                    args.join(" "),
                    s(&out)
                );
            }
            if home.exists() {
                panic!("{}: help touched the store home", args.join(" "));
            }
        }
    }

    #[test]
    fn project_flag_reaches_the_command_but_the_ambient_project_does_not() {
        let _serial = crate::test_helpers::serial();
        let _job = setenv("HIVE_JOB", "");
        let _agent = setenv("HIVE_AGENT", "");
        let _ambient = setenv("HIVE_PROJECT", "demo");
        let dir = tempdir().unwrap();
        let home = dir.path().to_str().unwrap();
        hive_store::create(Path::new(home).join(hive_store::STORE_NAME)).unwrap();
        let store = hive_store::Store::open(hive_store::Options {
            home: home.to_string(),
            skip_records: true,
            ..hive_store::Options::default()
        })
        .unwrap();
        store
            .conn()
            .unwrap()
            .execute(
                &format!(
                    "INSERT INTO projects (slug, name, path, added_at) VALUES ('demo', 'Demo', '{}', '2026-09-06T00:00:00Z')",
                    dir.path().join("demo").display()
                ),
                [],
            )
            .unwrap();
        drop(store);
        let add = |extra_before: &[&str], name: &str, extra_after: &[&str]| -> String {
            let mut args = vec!["--home", home, "--json"];
            args.extend_from_slice(extra_before);
            args.extend_from_slice(&[
                "skill",
                "add",
                "--name",
                name,
                "--description",
                "d",
                "--body",
                "the body",
            ]);
            args.extend_from_slice(extra_after);
            let (mut out, mut errb) = (Vec::new(), Vec::new());
            let code = run(&args, &mut out, &mut errb);
            if code != 0 {
                panic!("{args:?}: code {code} stderr {:?}", s(&errb));
            }
            let v: serde_json::Value = serde_json::from_str(&s(&out)).unwrap();
            v["skill"]["project"].as_str().unwrap().to_string()
        };
        // HIVE_PROJECT alone - a worker's shell - never narrows what is added
        if !add(&[], "ambient", &[]).is_empty() {
            panic!("the ambient project scoped the skill");
        }
        // --project before or after the command word does, the same way
        if add(&["--project", "demo"], "before", &[]) != "demo" {
            panic!("--project before the command word was not seen");
        }
        if add(&[], "after", &["--project", "demo"]) != "demo" {
            panic!("--project after the command word was not seen");
        }
    }

    #[test]
    fn missing_store_inside_worker_does_not_suggest_init() {
        let _serial = crate::test_helpers::serial();
        let _job = setenv("HIVE_JOB", "job-test");
        let dir = tempdir().unwrap();
        let home = dir.path().join("missing");
        let _home = setenv("HIVE_HOME", home.to_str().unwrap());
        let (mut out, mut errb) = (Vec::new(), Vec::new());
        let code = run(&["init", dir.path().to_str().unwrap()], &mut out, &mut errb);
        let err = s(&errb);
        if code != 2
            || !out.is_empty()
            || !err.contains("No store at")
            || !err.contains("HIVE_JOB")
            || err.contains("fix: hive init")
        {
            panic!(
                "worker init: code {code}, stdout {:?}, stderr {err:?}",
                s(&out)
            );
        }
        if home.exists() {
            panic!("worker init created a store home");
        }
    }

    #[test]
    fn version_says_when_a_newer_release_is_known() {
        let _serial = crate::test_helpers::serial();
        let _job = setenv("HIVE_JOB", "");
        let dir = tempdir().unwrap();
        let home = dir.path().to_str().unwrap();
        let _home = setenv("HIVE_HOME", home);
        // No store: the version alone, and `update` is null in JSON.
        let (mut out, mut errb) = (Vec::new(), Vec::new());
        assert_eq!(run(&["version"], &mut out, &mut errb), 0, "{}", s(&errb));
        assert_eq!(s(&out).lines().count(), 1, "{:?}", s(&out));
        hive_store::Store::create(dir.path().join(hive_store::STORE_NAME)).unwrap();
        let store = hive_store::Store::open(hive_store::Options {
            home: home.to_string(),
            skip_records: true,
            ..Default::default()
        })
        .unwrap();
        let state = hive_cli::update::UpdateState {
            checked_at: "2026-11-09T02:00:00Z".into(),
            source: "https://releases.test/latest.json".into(),
            latest: "v999.0.0".into(),
            url: "https://releases.test/v999.0.0".into(),
            error: String::new(),
        };
        store
            .set_meta(
                hive_cli::update::META_KEY,
                &serde_json::to_string(&state).unwrap(),
            )
            .unwrap();
        // The check is off (no URL): an old result says nothing.
        out.clear();
        assert_eq!(run(&["version"], &mut out, &mut errb), 0);
        assert_eq!(s(&out).lines().count(), 1, "{:?}", s(&out));
        store
            .set_setting("human", hive_cli::update::LATEST_URL_SETTING, &state.source)
            .unwrap();
        out.clear();
        assert_eq!(run(&["version"], &mut out, &mut errb), 0);
        let text = s(&out);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], hive_cli::version());
        if hive_cli::update::parse_semver(&hive_cli::version()).is_some() {
            assert_eq!(
                lines.get(1).copied().unwrap_or(""),
                format!(
                    "A newer Hive is available: v999.0.0 (running {}) - https://releases.test/v999.0.0",
                    hive_cli::version()
                )
            );
        }
        out.clear();
        assert_eq!(run(&["--json", "version"], &mut out, &mut errb), 0);
        let doc: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(doc["update"]["latest"], "v999.0.0");
        assert_eq!(doc["update"]["url"], "https://releases.test/v999.0.0");
    }

    #[test]
    fn every_command_names_a_newer_release_once() {
        let _serial = crate::test_helpers::serial();
        let _job = setenv("HIVE_JOB", "");
        let _agent = setenv("HIVE_AGENT", "");
        let dir = tempdir().unwrap();
        let home = dir.path().to_str().unwrap();
        let _home = setenv("HIVE_HOME", home);
        hive_store::Store::create(dir.path().join(hive_store::STORE_NAME)).unwrap();
        let store = hive_store::Store::open(hive_store::Options {
            home: home.to_string(),
            skip_records: true,
            ..Default::default()
        })
        .unwrap();
        let state = hive_cli::update::UpdateState {
            checked_at: "2026-11-09T02:00:00Z".into(),
            source: "https://releases.test/latest.json".into(),
            latest: "v999.0.0".into(),
            url: "https://releases.test/v999.0.0".into(),
            error: String::new(),
        };
        store
            .set_meta(
                hive_cli::update::META_KEY,
                &serde_json::to_string(&state).unwrap(),
            )
            .unwrap();
        store
            .set_setting("human", hive_cli::update::LATEST_URL_SETTING, &state.source)
            .unwrap();
        store
            .set_setting("human", hive_cli::update::CHANNEL_SETTING, "release")
            .unwrap();
        let line = "A newer Hive v999.0.0 is available - run: hive update";
        // `run` installs the build-info stamp, so `version()` is whatever the stamp says from
        // the first command on (another test may have run one already): decide on the stamped
        // version, not on the `dev` a bare lib test starts with.
        hive_cli::install_build_info(BUILD_INFO_TEXT);
        let semver = hive_cli::update::parse_semver(&hive_cli::version()).is_some();
        // `settings` runs without a store (no_store) and so can never print the line;
        // `stop` opens a writable store the way an ordinary owner command does.
        let (mut out, mut errb) = (Vec::new(), Vec::new());
        assert_eq!(run(&["stop"], &mut out, &mut errb), 0, "{}", s(&errb));
        assert_eq!(s(&errb).contains(line), semver, "{:?}", s(&errb));
        // Shown once: the next command is quiet.
        errb.clear();
        assert_eq!(run(&["stop"], &mut out, &mut errb), 0, "{}", s(&errb));
        assert!(!s(&errb).contains("A newer Hive"), "{:?}", s(&errb));
        if semver {
            assert_eq!(
                store.meta(hive_cli::update::PROMPTED_KEY).unwrap().0,
                "v999.0.0"
            );
        }
        // An agent's command never prints it.
        store.set_meta(hive_cli::update::PROMPTED_KEY, "").unwrap();
        errb.clear();
        assert_eq!(
            run(&["--agent", "a1", "stop"], &mut out, &mut errb),
            0,
            "{}",
            s(&errb)
        );
        assert!(!s(&errb).contains("A newer Hive"), "{:?}", s(&errb));
    }

    #[test]
    fn settings_help_rejects_arguments() {
        let _serial = crate::test_helpers::serial();
        let _job = setenv("HIVE_JOB", "");
        let dir = tempdir().unwrap();
        let home = dir.path().join("missing");
        let _home = setenv("HIVE_HOME", home.to_str().unwrap());
        let (mut out, mut errb) = (Vec::new(), Vec::new());
        let code = run(&["settings", "help", "extra"], &mut out, &mut errb);
        if code != 2 || !out.is_empty() || !s(&errb).contains("unrecognized arguments: extra") {
            panic!(
                "help arguments: code {code}, stdout {:?}, stderr {:?}",
                s(&out),
                s(&errb)
            );
        }
        if home.exists() {
            panic!("invalid help touched the store home");
        }
    }

    #[test]
    fn settings_actions_need_store() {
        let _serial = crate::test_helpers::serial();
        let _job = setenv("HIVE_JOB", "");
        let dir = tempdir().unwrap();
        let home = dir.path().join("missing");
        let _home = setenv("HIVE_HOME", home.to_str().unwrap());
        for args in [
            vec!["settings"],
            vec!["settings", "list"],
            vec!["settings", "get", "minutes_per_job"],
            vec!["settings", "set", "minutes_per_job", "11"],
        ] {
            let (mut out, mut errb) = (Vec::new(), Vec::new());
            let code = run(&args, &mut out, &mut errb);
            let want = format!(
                "No store at {}",
                home.join(hive_store::STORE_NAME).to_str().unwrap()
            );
            if code != 2 || !out.is_empty() || !s(&errb).contains(&want) {
                panic!(
                    "{args:?}: code {code}, stdout {:?}, stderr {:?}",
                    s(&out),
                    s(&errb)
                );
            }
            if home.exists() {
                panic!("{args:?} created a store home");
            }
        }
    }

    /// `hive init --store-only` creates the store and registers nothing:
    /// a hosted brain's image (deploy/entrypoint.sh) starts from it with no
    /// checkout, then sets `brain.code none`; a project flag beside it is
    /// refused.
    #[test]
    fn init_store_only_creates_the_store_and_registers_no_project() {
        let _serial = crate::test_helpers::serial();
        let _job = setenv("HIVE_JOB", "");
        let _agent = setenv("HIVE_AGENT", "");
        let dir = tempdir().unwrap();
        let home = dir.path().join("hive");
        let _home = setenv("HIVE_HOME", home.to_str().unwrap());
        let (mut out, mut errb) = (Vec::new(), Vec::new());
        let code = run(&["init", "--store-only"], &mut out, &mut errb);
        assert_eq!(code, 0, "stdout {:?}, stderr {:?}", s(&out), s(&errb));
        assert!(home.join(hive_store::STORE_NAME).is_file());
        assert!(s(&out).starts_with("store: "), "{}", s(&out));
        for args in [
            vec!["settings", "set", "brain.code", "none"],
            vec!["settings", "set", "daemon.self_update", "release"],
        ] {
            let (mut out, mut errb) = (Vec::new(), Vec::new());
            let code = run(&args, &mut out, &mut errb);
            assert_eq!(code, 0, "{args:?}: {:?} {:?}", s(&out), s(&errb));
        }
        let (mut out, mut errb) = (Vec::new(), Vec::new());
        assert_eq!(run(&["projects"], &mut out, &mut errb), 0, "{}", s(&errb));
        assert!(!s(&out).contains("brain"), "no project: {}", s(&out));
        let (mut out, mut errb) = (Vec::new(), Vec::new());
        let code = run(&["init", "--store-only", "--brain"], &mut out, &mut errb);
        assert_eq!(code, 2, "stdout {:?}, stderr {:?}", s(&out), s(&errb));
        assert!(s(&errb).contains("registers no project"), "{}", s(&errb));
    }

    #[test]
    fn settings_store_actions() {
        let _serial = crate::test_helpers::serial();
        let _job = setenv("HIVE_JOB", "");
        let _agent = setenv("HIVE_AGENT", "");
        let dir = tempdir().unwrap();
        let home = dir.path();
        let _home = setenv("HIVE_HOME", home.to_str().unwrap());
        hive_store::create(home.join(hive_store::STORE_NAME)).unwrap();
        for (args, want) in [
            (
                vec!["get", "minutes_per_job"],
                format!("{}\n", hive_store::DEFAULT_SETTINGS["minutes_per_job"]),
            ),
            (
                vec!["set", "minutes_per_job", "11"],
                "minutes_per_job=11\n".to_string(),
            ),
            (vec!["get", "minutes_per_job"], "11\n".to_string()),
            (
                vec!["get", "evolve.limits"],
                format!("{}\n", hive_store::EVOLVE_SETTINGS["evolve.limits"]),
            ),
        ] {
            let (mut out, mut errb) = (Vec::new(), Vec::new());
            let mut full = vec!["settings"];
            full.extend(args.iter().copied());
            let code = run(&full, &mut out, &mut errb);
            if code != 0 || s(&out) != want || !errb.is_empty() {
                panic!(
                    "{args:?}: code {code}, stdout {:?}, stderr {:?}",
                    s(&out),
                    s(&errb)
                );
            }
        }
        for name in ["evolve_minutes", "evolve_credits"] {
            let (mut out, mut errb) = (Vec::new(), Vec::new());
            let code = run(&["settings", "set", name, "60"], &mut out, &mut errb);
            if code != 2
                || !out.is_empty()
                || !s(&errb).contains(&format!("Unknown setting '{name}'"))
            {
                panic!(
                    "unsupported {name}: code {code}, stdout {:?}, stderr {:?}",
                    s(&out),
                    s(&errb)
                );
            }
        }
        let (mut out, mut errb) = (Vec::new(), Vec::new());
        let code = run(&["settings", "list", "--json"], &mut out, &mut errb);
        if code != 0 || !errb.is_empty() {
            panic!(
                "list: code {code}, stdout {:?}, stderr {:?}",
                s(&out),
                s(&errb)
            );
        }
        #[derive(serde::Deserialize, Debug)]
        struct Row {
            name: String,
            value: String,
            default: String,
        }
        let rows: Vec<Row> = serde_json::from_slice(&out).unwrap();
        let mut found = false;
        for row in &rows {
            if row.name == "evolve_minutes" || row.name == "evolve_credits" {
                panic!("unsupported setting advertised as configurable: {row:?}");
            }
            if row.name == "minutes_per_job" {
                found = true;
                if row.value != "11" || row.default != hive_store::DEFAULT_SETTINGS[&row.name] {
                    panic!("effective value and default lost: {row:?}");
                }
            }
        }
        if !found {
            panic!("settings list omitted minutes_per_job");
        }
    }
}
