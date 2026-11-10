//! `pulse-hand`: the executor a machine installs once (ADR-070). It holds
//! the executor path alone - the `pulse-hands` verbs that prepare a job's
//! worktree, take the run's signing key and hooks, run the worker inside a
//! budget, report the result and clean up - and none of the brain, which
//! stays hosted and drives these verbs.
//!
//! The brain reaches a hand the way it reaches `pulse hands` on a paired
//! machine: one verb per call, the command line in `SSH_ORIGINAL_COMMAND`
//! (`b64 <argv joined by NUL>`, or `git-upload-pack <path>` for fetching the
//! result), the payload on stdin. On the same machine that is a pipe: the
//! brain spawns `pulse-hand` with the variable set. A verb may also be given
//! as arguments (`pulse-hand probe`) when an operator tries one by hand.

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(first) = args.first() {
        match first.as_str() {
            "--version" | "version" => {
                println!("pulse-hand {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--help" | "-h" | "help" => {
                print!("{}", usage());
                return ExitCode::SUCCESS;
            }
            "join" => return enrol(&args[1..]),
            "dial" => return dial(&args[1..]),
            _ => {}
        }
    }
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut h = pulse_hand_core::hands::new_hands(std::io::stdin(), stdout, stderr);
    let res = h.hands_main(&args);
    let res = pulse_hand_core::hands::hands_outcome(res, &mut h.stderr);
    let _ = h.stdout.flush();
    let _ = h.stderr.flush();
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let code = match err.downcast_ref::<pulse_hand_core::Error>() {
                Some(e) => {
                    if !e.what.is_empty() {
                        let _ = writeln!(std::io::stderr(), "{e}");
                    }
                    e.exit_code()
                }
                None => {
                    let _ = writeln!(std::io::stderr(), "pulse-hand: {err:#}");
                    1
                }
            };
            ExitCode::from(u8::try_from(code.clamp(0, 255)).unwrap_or(1))
        }
    }
}

fn usage() -> &'static str {
    "pulse-hand: runs the jobs the PULSE brain hands this machine (ADR-070).\n\
     \n\
     usage: pulse-hand join [--brain URL] [--name NAME] [--labels a,b] [--workers N] [--workdir DIR] CODE\n\
     \x20      pulse-hand dial   (hold the outbound channel to the brain)\n\
     \x20      pulse-hand VERB [ARGS...]   (or the verb in SSH_ORIGINAL_COMMAND)\n\
     \n\
     join enrols this machine once with a join code from the brain's Machines\n\
     page: it makes a device key in PULSE_HAND_HOME (default ~/.pulse-hand)\n\
     and dials in. PULSE_BRAIN is the brain's URL when --brain is not given.\n\
     \n\
     verbs: probe, prepare, key, hook, brief, run, result, count, dirty,\n\
     changed, bundle, drop-result, verify, tool, clean, steps, git-upload-pack\n\
     \n\
     PULSE_HANDS_WORKDIR names the directory jobs run under (default /srv/pulse).\n"
}

fn flag_value(args: &[String], i: &mut usize, name: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("{name} needs a value"))
}

fn parse_join(args: &[String]) -> Result<pulse_hand_core::dial::JoinOptions, String> {
    let mut o = pulse_hand_core::dial::JoinOptions {
        brain: std::env::var("PULSE_BRAIN").unwrap_or_default(),
        workers: 1,
        ..Default::default()
    };
    let mut words = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--brain" => o.brain = flag_value(args, &mut i, a)?,
            "--name" => o.name = flag_value(args, &mut i, a)?,
            "--labels" => o.labels = flag_value(args, &mut i, a)?,
            "--workdir" => o.workdir = flag_value(args, &mut i, a)?,
            "--workers" => {
                o.workers = flag_value(args, &mut i, a)?
                    .parse()
                    .map_err(|_| "--workers takes a whole number".to_string())?
            }
            _ if a.starts_with("--") => return Err(format!("unknown flag {a}")),
            _ => words.push(a.to_string()),
        }
        i += 1;
    }
    o.code = words.join(" ");
    if o.code.trim().is_empty() {
        return Err("join needs the code the Machines page shows".into());
    }
    Ok(o)
}

/// `pulse-hand join CODE`: enrols this machine, then dials in.
fn enrol(args: &[String]) -> ExitCode {
    let o = match parse_join(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("pulse-hand join: {e}\n\n{}", usage());
            return ExitCode::from(2);
        }
    };
    let home = pulse_hand_core::dial::hand_home();
    match pulse_hand_core::dial::join(&pulse_hand_core::CancellationToken::new(), &home, &o) {
        Ok(d) => {
            println!(
                "joined {} as {} (device {}); jobs run under {}",
                d.brain, d.executor, d.device, d.workdir
            );
            if std::env::var("PULSE_HAND_JOIN_ONLY").is_ok_and(|v| v == "1") {
                return ExitCode::SUCCESS;
            }
            dial(&[])
        }
        Err(e) => {
            eprintln!("pulse-hand join: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// `pulse-hand dial`: holds the outbound channel until a signal or the
/// brain revokes this device.
fn dial(_args: &[String]) -> ExitCode {
    let home = pulse_hand_core::dial::hand_home();
    let Ok(me) = std::env::current_exe() else {
        eprintln!("pulse-hand dial: cannot find this binary");
        return ExitCode::FAILURE;
    };
    let cancel = pulse_hand_core::CancellationToken::new();
    let log = |line: &str| eprintln!("pulse-hand: {line}");
    match pulse_hand_core::dial::dial(&cancel, &home, &me, &log) {
        Ok(pulse_hand_core::dial::DialEnd::Stopped) => ExitCode::SUCCESS,
        Ok(pulse_hand_core::dial::DialEnd::Refused(why)) => {
            eprintln!(
                "pulse-hand dial: the brain refused this machine: {why}\nAsk the owner for a new join code and run `pulse-hand join <code>`."
            );
            ExitCode::from(3)
        }
        Err(e) => {
            eprintln!("pulse-hand dial: {e:#}");
            ExitCode::FAILURE
        }
    }
}
