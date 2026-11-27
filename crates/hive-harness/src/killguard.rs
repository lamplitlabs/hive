//! Stopping processes by pattern: refused. At 22:14Z on 26 November one
//! agent stopped its mock server with `pkill -f "mock/server.mjs" -U $(id
//! -u) -f`; macOS `pkill` read `-U`, the user id and `-f` as more patterns,
//! matched every process of the user and ended every agent's worker and
//! some of the owner's programs (TASK-151). An agent stops what it started
//! by that process's pid; `pkill`, `killall` and a `kill` whose pids come
//! from `pgrep`, `pidof`, `lsof` or `ps` (in `$(...)` or through `xargs`)
//! are refused before they run. `kill <pid>`, `kill $!`, `kill %1` and
//! `kill $(cat server.pid)` stay allowed.
//!
//! The shell's own words do not hide a command: `if`, `then`, `else`, `do`,
//! `while`, `!`, `{` and the like are skipped before the program is read, so
//! `if true; then pkill node; fi` is refused. A variable filled from a pid
//! lister - `pid=$(pgrep node)`, `for p in $(pgrep node)`, `pgrep node |
//! while read p` - makes a later `kill $pid` refused too (TASK-170).
use crate::readonly::{Item, basename, strip_assignments, strip_flags, tokenize};

/// The one line a refused command gets.
pub const KILL_REFUSAL: &str = "refused: stopping processes by name or pattern (pkill, killall, kill fed from pgrep/lsof/pidof/ps) also stops the owner's programs and every other agent's worker; stop only what you started, by its pid: `kill <pid>` (the pid `$!` gave when you started it, or the process tool's stop for a background command)";

/// The programs that print pids by name, port or pattern.
const PID_LISTERS: &[&str] = &["pgrep", "pidof", "lsof", "ps"];

/// The programs that signal processes by name or pattern.
const PATTERN_KILLERS: &[&str] = &["pkill", "killall", "killall5", "skill", "slay"];

/// Why `command` may not run, or None when it may: it stops processes by
/// name or pattern instead of by a pid the agent knows.
pub fn kill_refusal(command: &str) -> Option<String> {
    refused(command, 0).then(|| KILL_REFUSAL.to_string())
}

fn refused(command: &str, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    // What earlier segments of this command line listed: pids that may
    // reach a `kill` through `xargs` or a variable further down.
    let mut listed = Listed::default();
    let mut words: Vec<String> = Vec::new();
    let mut nested: Vec<String> = Vec::new();
    for item in tokenize(command) {
        match item {
            Item::Word(w) => words.push(w),
            Item::Redirect(_) => {}
            Item::Nested(inner) => {
                if refused(&inner, depth + 1) {
                    return true;
                }
                nested.push(inner);
            }
            Item::Sep | Item::Open | Item::Close => {
                let segment = std::mem::take(&mut words);
                let inner = std::mem::take(&mut nested);
                if segment_refused(segment, &inner, &mut listed, depth) {
                    return true;
                }
            }
        }
    }
    segment_refused(words, &nested, &mut listed, depth)
}

/// What the segments of one command line have listed so far.
#[derive(Default)]
struct Listed {
    /// An earlier segment ran a pid lister: its output may reach `xargs kill`.
    output: bool,
    /// Shell variables whose value came from a pid lister.
    vars: Vec<String>,
}

/// The shell's reserved words and grouping marks that may open a segment
/// before the command it runs.
const SHELL_KEYWORDS: &[&str] = &[
    "if", "then", "elif", "else", "fi", "do", "done", "while", "until", "!", "{", "}",
];

/// Notes the variables `words` assign from a `$(...)` that lists pids.
fn note_assignments(words: &[String], nested: &[String], listed: &mut Listed, depth: usize) {
    let from_lister = || nested.iter().any(|n| lists_pids(n, depth + 1));
    for w in words {
        let Some((name, value)) = w.split_once('=') else {
            break;
        };
        let is_name = name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !is_name {
            break;
        }
        if value.contains("$(...)") && from_lister() {
            listed.vars.push(name.to_string());
        }
    }
}

/// Whether `word` expands one of `vars` (`$p`, `${p}`, `"$pids"`).
fn uses_var(word: &str, vars: &[String]) -> bool {
    vars.iter().any(|v| {
        word.match_indices('$').any(|(i, _)| {
            let rest = &word[i + 1..];
            let rest = rest.strip_prefix('{').unwrap_or(rest);
            rest.strip_prefix(v.as_str()).is_some_and(|after| {
                !after
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            })
        })
    })
}

/// Checks one simple command; `nested` holds the `$(...)` commands its
/// words take their text from.
fn segment_refused(
    mut words: Vec<String>,
    nested: &[String],
    listed: &mut Listed,
    depth: usize,
) -> bool {
    note_assignments(&words, nested, listed, depth);
    strip_assignments(&mut words);
    let mut from_stdin = false;
    loop {
        let Some(first) = words.first() else {
            return false;
        };
        let prog = basename(first).to_string();
        match prog.as_str() {
            k if SHELL_KEYWORDS.contains(&k) => {
                words.remove(0);
                note_assignments(&words, nested, listed, depth);
                strip_assignments(&mut words);
            }
            "for" | "select" => {
                // `for p in $(pgrep node)`: the loop variable holds pids.
                if let Some(var) = words.get(1)
                    && words.get(2).is_some_and(|w| w == "in")
                    && nested.iter().any(|n| lists_pids(n, depth + 1))
                {
                    listed.vars.push(var.clone());
                }
                return false;
            }
            "export" | "local" | "declare" | "typeset" | "readonly" => {
                words.remove(0);
                strip_flags(&mut words, &[]);
                note_assignments(&words, nested, listed, depth);
                return false;
            }
            "read" => {
                // `pgrep node | while read p`: each line read is a pid.
                if listed.output {
                    words.remove(0);
                    strip_flags(&mut words, &["-d", "-n", "-N", "-p", "-t", "-u", "-a"]);
                    listed.vars.extend(words.iter().cloned());
                }
                return false;
            }
            "sudo" | "doas" => {
                words.remove(0);
                strip_flags(
                    &mut words,
                    &["-u", "-g", "-C", "-h", "-p", "-r", "-t", "-U"],
                );
            }
            "env" => {
                words.remove(0);
                strip_flags(&mut words, &["-u", "-S", "--unset", "-C", "--chdir"]);
                strip_assignments(&mut words);
            }
            "nice" => {
                words.remove(0);
                strip_flags(&mut words, &["-n", "--adjustment"]);
            }
            "nohup" | "time" | "command" | "builtin" | "exec" | "unbuffer" | "noglob" => {
                words.remove(0);
                strip_flags(&mut words, &[]);
            }
            "timeout" | "gtimeout" => {
                words.remove(0);
                strip_flags(&mut words, &["-k", "-s", "--kill-after", "--signal"]);
                if !words.is_empty() {
                    words.remove(0); // the duration
                }
            }
            "xargs" => {
                words.remove(0);
                strip_flags(
                    &mut words,
                    &[
                        "-n", "-I", "-i", "-P", "-d", "-s", "-L", "-E", "-a", "-e", "-R", "-S",
                    ],
                );
                from_stdin = true;
            }
            "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish" => {
                let flags: Vec<String> = words[1..]
                    .iter()
                    .take_while(|w| w.starts_with('-') && w.len() > 1)
                    .cloned()
                    .collect();
                let runs_string = flags
                    .iter()
                    .any(|f| !f.starts_with("--") && f.contains('c'));
                return runs_string
                    && words
                        .get(1 + flags.len())
                        .is_some_and(|script| refused(script, depth + 1));
            }
            "eval" => return refused(&words[1..].join(" "), depth + 1),
            p if PATTERN_KILLERS.contains(&p) => return true,
            p if PID_LISTERS.contains(&p) => {
                listed.output = true;
                return false;
            }
            "kill" => {
                let pids = kill_targets(&words[1..]);
                // `kill -1` as a pid is every process the user may signal.
                if pids.iter().any(|p| p == "-1") {
                    return true;
                }
                if from_stdin && listed.output {
                    return true;
                }
                if pids.iter().any(|p| uses_var(p, &listed.vars)) {
                    return true;
                }
                let fed = pids.iter().any(|p| p.contains("$(...)"));
                return fed && nested.iter().any(|n| lists_pids(n, depth + 1));
            }
            _ => return false,
        }
    }
}

/// The pid arguments of `kill`: what follows its one signal option
/// (`-9`, `-TERM`, `-s TERM`, `-n 9`) and an optional `--`.
fn kill_targets(args: &[String]) -> Vec<String> {
    let mut i = 0;
    if let Some(first) = args.first() {
        match first.as_str() {
            "-s" | "-n" => i = 2,
            "--" => {}
            "-l" | "-L" => return Vec::new(),
            a if a.starts_with('-') && a.len() > 1 => i = 1,
            _ => {}
        }
    }
    if args.get(i).is_some_and(|a| a == "--") {
        i += 1;
    }
    args.get(i..).map(<[String]>::to_vec).unwrap_or_default()
}

/// Whether `command` runs a program that prints pids by name or pattern.
fn lists_pids(command: &str, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    let mut words: Vec<String> = Vec::new();
    let mut found = false;
    let mut check = |words: &mut Vec<String>| {
        let mut w = std::mem::take(words);
        strip_assignments(&mut w);
        while let Some(first) = w.first() {
            let prog = basename(first).to_string();
            match prog.as_str() {
                "sudo" | "doas" | "env" | "nice" | "nohup" | "command" | "exec" => {
                    w.remove(0);
                    strip_flags(&mut w, &["-u", "-g", "-n"]);
                    strip_assignments(&mut w);
                }
                p => {
                    found |= PID_LISTERS.contains(&p);
                    break;
                }
            }
        }
    };
    for item in tokenize(command) {
        match item {
            Item::Word(w) => words.push(w),
            Item::Nested(inner) => {
                if lists_pids(&inner, depth + 1) {
                    return true;
                }
            }
            Item::Redirect(_) => {}
            Item::Sep | Item::Open | Item::Close => check(&mut words),
        }
    }
    check(&mut words);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_kills_are_refused() {
        for cmd in [
            // The command that stopped every worker on 26 November.
            r#"pkill -f "mock/server.mjs" -U $(id -u) -f"#,
            "pkill -f next-server",
            "/usr/bin/pkill node",
            "killall node",
            "sudo killall -9 hive",
            "cd web && npm run build; pkill -f vite || true",
            "kill $(pgrep -f mock/server.mjs)",
            "kill -9 `pgrep node`",
            "kill -TERM $(lsof -ti :3000)",
            "kill $(pidof node)",
            "kill -9 $(ps aux | grep '[s]erver.mjs' | awk '{print $2}')",
            "pgrep -f server.mjs | xargs kill",
            "lsof -ti tcp:8766 | xargs kill -9",
            "ps -ef | grep vite | awk '{print $2}' | xargs -n1 kill",
            "sh -c 'pkill -f vite'",
            "bash -lc \"killall node\"",
            "timeout 5 pkill node",
            "env FOO=1 pkill node",
            "kill -9 -1",
            "kill -- -1",
            "echo $(pkill node)",
            // The shell's own words before the command (TASK-170).
            "if true; then pkill node; fi",
            "if pgrep -q node; then killall node; fi",
            "[ -f x ] && { pkill -f vite; }",
            "while true; do killall -9 node; sleep 1; done",
            "until false; do pkill node; done",
            "! pkill node",
            "if [ -n \"$CI\" ]; then :; else FOO=1 pkill node; fi",
            "for i in 1 2; do sudo pkill -f server; done",
            "(cd web && pkill -f next)",
            // Pids that reach `kill` through a variable.
            "pid=$(pgrep -f server.mjs); kill $pid",
            "PIDS=$(lsof -ti :3000) && kill -9 $PIDS",
            "export p=$(pidof node); kill \"${p}\"",
            "for p in $(pgrep node); do kill $p; done",
            "for p in `ps -o pid= -U me`; do kill -9 \"$p\"; done",
            "pgrep node | while read p; do kill $p; done",
            "if true; then pid=$(pgrep node); kill $pid; fi",
        ] {
            assert_eq!(
                kill_refusal(cmd).as_deref(),
                Some(KILL_REFUSAL),
                "should refuse: {cmd}"
            );
        }
    }

    #[test]
    fn kill_by_a_known_pid_runs() {
        for cmd in [
            "kill 12345",
            "kill -9 12345",
            "kill -TERM 12345 12346",
            "kill -s TERM 4242",
            "kill $!",
            "kill %1",
            "kill \"$PID\"",
            "kill $(cat /tmp/server.pid)",
            "node server.mjs & echo $! > /tmp/server.pid",
            "kill -l",
            "kill -0 4242 && echo alive",
            "pgrep -f server.mjs",
            "ps -o pid,command -p 4242",
            "lsof -i :3000",
            "grep -rn pkill crates/",
            "git log --grep killall",
            "echo 'never pkill'",
            "if [ -f /tmp/s.pid ]; then kill $(cat /tmp/s.pid); fi",
            "while kill -0 4242; do sleep 1; done",
            "for f in *.rs; do echo $f; done",
            "pid=$(cat /tmp/server.pid); kill $pid",
            "p=$(pgrep node); echo $p; kill $pids",
            "for p in $(pgrep node); do echo $p; done",
            "node server.mjs & pid=$!; sleep 2; kill $pid",
            "if true; then echo pkill; fi",
        ] {
            assert_eq!(kill_refusal(cmd), None, "should run: {cmd}");
        }
    }

    #[test]
    fn the_refusal_is_one_line_that_names_the_pid_way() {
        assert!(!KILL_REFUSAL.contains('\n'));
        assert!(KILL_REFUSAL.contains("kill <pid>"));
        assert!(KILL_REFUSAL.contains("pkill"));
    }
}
