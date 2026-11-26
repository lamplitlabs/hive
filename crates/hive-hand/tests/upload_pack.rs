//! The brain fetches a run's commits with `git fetch
//! ssh://hive@<machine><workdir>/<job>/clone`; sshd hands that to the hand
//! as the forced command (`SSH_ORIGINAL_COMMAND=git-upload-pack '<path>'`).
//! This test plays sshd: git's ssh is a script that runs the `hive-hand`
//! binary with that variable set, its stdin and stdout the session, so the
//! fetch goes through the same code a machine runs. On 2026-11-26 the verb
//! piped git's streams and never pumped them, and every such fetch answered
//! "Could not read from remote repository" (TASK-133).

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[cfg(unix)]
#[test]
fn git_fetches_a_job_clone_through_the_forced_command() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    // The hand checks the path as text against its workdir, so both are
    // spelled from the same canonical root.
    let root_path = std::fs::canonicalize(root.path()).unwrap();
    let workdir = root_path.join("srv");
    let clone = workdir.join("job-20261126T000000Z-abcdef").join("clone");
    std::fs::create_dir_all(&clone).unwrap();
    git(&clone, &["init", "-q", "-b", "main"]);
    git(&clone, &["config", "user.name", "w"]);
    git(&clone, &["config", "user.email", "w@example.invalid"]);
    std::fs::write(clone.join("note.txt"), "work done on the machine\n").unwrap();
    git(&clone, &["add", "note.txt"]);
    git(&clone, &["commit", "-q", "-m", "work"]);
    let head = git(&clone, &["rev-parse", "HEAD"]);

    // sshd's part: the last word git passes is the command line; the hand
    // reads it from SSH_ORIGINAL_COMMAND, the session on stdin and stdout.
    let fake_ssh = root_path.join("fake-ssh");
    std::fs::write(
        &fake_ssh,
        format!(
            "#!/bin/sh\nfor last; do :; done\nSSH_ORIGINAL_COMMAND=\"$last\" HIVE_HANDS_WORKDIR='{}' exec '{}'\n",
            workdir.display(),
            env!("CARGO_BIN_EXE_hive-hand")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake_ssh, std::fs::Permissions::from_mode(0o755)).unwrap();

    let brain = root_path.join("brain");
    std::fs::create_dir_all(&brain).unwrap();
    git(&brain, &["init", "-q", "-b", "main"]);
    let url = format!("ssh://hive@machine.invalid{}", clone.display());
    let (tx, rx) = std::sync::mpsc::channel();
    let brain_dir = brain.clone();
    let ssh = fake_ssh.display().to_string();
    std::thread::spawn(move || {
        let out = Command::new("git")
            .current_dir(&brain_dir)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_SSH_COMMAND", &ssh)
            .env("GIT_SSH_VARIANT", "simple")
            .env("GIT_TERMINAL_PROMPT", "0")
            .args([
                "fetch",
                "-q",
                "--no-tags",
                &url,
                "+HEAD:refs/hive/landing/job",
            ])
            .output();
        let _ = tx.send(out);
    });
    // A stalled session (nothing pumped) hangs rather than fails; give it
    // a minute, not forever.
    let out = rx
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("git fetch through the forced command hung")
        .expect("git fetch runs");
    assert!(
        out.status.success(),
        "git fetch through the forced command failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(git(&brain, &["rev-parse", "refs/hive/landing/job"]), head);
    assert_eq!(
        git(&brain, &["show", "refs/hive/landing/job:note.txt"]),
        "work done on the machine"
    );
}
