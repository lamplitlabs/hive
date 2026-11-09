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
     usage: pulse-hand VERB [ARGS...]   (or the verb in SSH_ORIGINAL_COMMAND)\n\
     \n\
     verbs: probe, prepare, key, hook, brief, run, result, count, dirty,\n\
     changed, drop-result, verify, tool, clean, steps, git-upload-pack\n\
     \n\
     PULSE_HANDS_WORKDIR names the directory jobs run under (default /srv/pulse).\n"
}
