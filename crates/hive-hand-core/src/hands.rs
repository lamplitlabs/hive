#![allow(unused_variables, dead_code, unused_imports)]
use anyhow::{Result, anyhow};
use base64::Engine;
use regex::Regex;
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, RwLock};
use std::time::Duration;
/// What a hand runner exits with when its budget runs out (`timeout`'s code).
pub const REMOTE_EXIT_TIMEOUT: i32 = 124;
pub const HANDS_DEFAULT_WORKDIR: &str = "/srv/hive";
static HANDS_JOB_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9._-]+$").unwrap());
static HANDS_BRANCH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9._/-]+$").unwrap());
pub static HANDS_ENV_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(HIVE_[A-Z_]+|GIT_TERMINAL_PROMPT)=.*$").unwrap());
/// Names the hands keep for themselves: a recipe's `env:` may set anything the
/// hub would, but not what decides which binaries load or whose account runs.
const HANDS_OWN_ENV: [&str; 6] = ["PATH", "HOME", "USER", "LOGNAME", "SHELL", "PWD"];
const HANDS_OWN_ENV_PREFIXES: [&str; 3] = ["LD_", "DYLD_", "SSH_"];
/// Whether `run` takes `K=V`: the hub's own keys, or a name a recipe's `env:`
/// may set (the hub refused secret names and secret-shaped values before it
/// sent them; the hands refuse the names again, cheaply).
pub fn hands_env_allowed(kv: &str) -> bool {
    if HANDS_ENV_RE.is_match(kv) {
        return true;
    }
    let Some((k, _)) = kv.split_once('=') else {
        return false;
    };
    crate::names::recipe_env_name_ok(k)
        && !HANDS_OWN_ENV.contains(&k)
        && !HANDS_OWN_ENV_PREFIXES.iter().any(|p| k.starts_with(p))
}
static HANDS_B64_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^b64 ([A-Za-z0-9+/=]+)$").unwrap());
static HANDS_UPLOAD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^git-upload-pack '([^'[:cntrl:]]+)'$").unwrap());
static HANDS_PINNED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^command="[^"]*hive-hands[^"]*",([^ ]*,)?restrict"#).unwrap());
#[derive(Debug, Clone)]
pub struct HandsError {
    pub msg: String,
}
impl std::fmt::Display for HandsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "hive-hands: {}", self.msg)
    }
}
impl std::error::Error for HandsError {}
pub fn hands_die(msg: impl Into<String>) -> anyhow::Error {
    anyhow!(HandsError { msg: msg.into() })
}
#[derive(Debug, Clone)]
pub struct HandsExit {
    pub code: i32,
}
impl std::fmt::Display for HandsExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exit {}", self.code)
    }
}
impl std::error::Error for HandsExit {}
/// What a hands verb's outcome means to the process that ran it: `Ok` for a
/// verb that finished and for a `run`, `verify`, or `tool` whose command
/// exited 0; otherwise a `crate::error::Error` carrying the command's own exit,
/// 2 after writing the refusal for a `HandsError`, 1 after writing any other
/// error. The installed binary and the tests' helper process both go through
/// here. The hub reads `verify … -- test -s run.key` as "the key is in place"
/// only on exit 0, and the first job sent to a freshly provisioned Azure hand
/// (hands-cr-062, 2026-11-05) failed that preflight with the key intact:
/// `cmd_hands` wrapped `HandsExit { code: 0 }` in `errorf(0)`, whose
/// `exit_code` reads 0 as 1, so `verify … -- true` replayed by hand came back
/// 1 while `exit 7` came back 7. The test double mapped the code itself and
/// never saw the difference, which is why there is one mapping now.
pub fn hands_outcome(res: Result<()>, stderr: &mut dyn Write) -> Result<()> {
    let Err(err) = res else {
        return Ok(());
    };
    if let Some(hx) = err.downcast_ref::<HandsExit>() {
        return if hx.code == 0 {
            Ok(())
        } else {
            Err(crate::error::errorf(hx.code, "", "", "").into())
        };
    }
    if let Some(he) = err.downcast_ref::<HandsError>() {
        writeln!(stderr, "{he}")?;
        return Err(crate::error::errorf(2, "", "", "").into());
    }
    writeln!(stderr, "{err:#}")?;
    Err(crate::error::errorf(1, "", "", "").into())
}
pub struct Hands {
    pub workdir: String,
    pub home: String,
    pub stdin: Box<dyn Read + Send>,
    pub stdout: Box<dyn Write + Send>,
    pub stderr: Box<dyn Write + Send>,
}
impl std::fmt::Debug for Hands {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hands")
            .field("workdir", &self.workdir)
            .field("home", &self.home)
            .finish_non_exhaustive()
    }
}
/// The worker `run` may start: `hive harness run …`, bare or under the
/// recipe's pins as `mise exec <tool@version>… -- hive harness run …`.
pub fn hands_allowed_worker(rest: &[String]) -> bool {
    let rest = match rest {
        [mise, exec, tail @ ..] if mise == "mise" && exec == "exec" => {
            let Some(sep) = tail.iter().position(|a| a == "--") else {
                return false;
            };
            if sep == 0
                || !tail[..sep]
                    .iter()
                    .all(|p| crate::names::RECIPE_TOOL_RE.is_match(p))
            {
                return false;
            }
            &tail[sep + 1..]
        }
        _ => rest,
    };
    match rest
        .first()
        .map(|s| {
            std::path::Path::new(s)
                .file_name()
                .and_then(|x| x.to_str())
                .unwrap_or(s)
                .to_string()
        })
        .as_deref()
    {
        Some("hive") => rest.len() >= 3 && rest[1] == "harness" && rest[2] == "run",
        _ => false,
    }
}
pub fn new_hands<'a>(
    stdin: impl Read + Send + 'a,
    stdout: impl Write + Send + 'a,
    stderr: impl Write + Send + 'a,
) -> Hands {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/nonexistent".into());
    let path = [
        std::path::Path::new(&home)
            .join(".local/bin")
            .display()
            .to_string(),
        std::path::Path::new(&home)
            .join(".cargo/bin")
            .display()
            .to_string(),
        std::path::Path::new(&home)
            .join(".bun/bin")
            .display()
            .to_string(),
        std::env::var("PATH").unwrap_or_default(),
    ]
    .join(":");
    unsafe {
        std::env::set_var("PATH", path);
    }
    Hands {
        workdir: std::env::var("HIVE_HANDS_WORKDIR")
            .unwrap_or_else(|_| HANDS_DEFAULT_WORKDIR.into()),
        home,
        stdin: erase_read_lifetime(Box::new(stdin)),
        stdout: erase_write_lifetime(Box::new(stdout)),
        stderr: erase_write_lifetime(Box::new(stderr)),
    }
}
fn erase_read_lifetime<'a>(r: Box<dyn Read + Send + 'a>) -> Box<dyn Read + Send> {
    // `Hands` is used synchronously by the command dispatcher; borrowed stdio writers outlive it.
    unsafe { std::mem::transmute::<Box<dyn Read + Send + 'a>, Box<dyn Read + Send>>(r) }
}
fn erase_write_lifetime<'a>(w: Box<dyn Write + Send + 'a>) -> Box<dyn Write + Send> {
    // `Hands` is used synchronously by the command dispatcher; borrowed stdio writers outlive it.
    unsafe { std::mem::transmute::<Box<dyn Write + Send + 'a>, Box<dyn Write + Send>>(w) }
}
impl Hands {
    pub fn job_dir(&self, job: &str) -> Result<String> {
        if !HANDS_JOB_RE.is_match(job) || job == "." || job == ".." {
            return Err(hands_die("bad job id"));
        }
        Ok(format!("{}/{}", self.workdir, job))
    }
    pub fn clone_dir(&self, job: &str) -> Result<String> {
        let clone = format!("{}/clone", self.job_dir(job)?);
        if !Path::new(&clone).join(".git").is_dir() {
            return Err(hands_die(format!("no clone for job {job}")));
        }
        Ok(clone)
    }
    pub fn with_timeout(
        &mut self,
        budget: Duration,
        dir: &str,
        argv: &[String],
        extra: &std::collections::BTreeMap<String, String>,
    ) -> i32 {
        self.with_timeout_observed(budget, dir, argv, extra, None)
    }
    pub fn with_timeout_observed(
        &mut self,
        budget: Duration,
        dir: &str,
        argv: &[String],
        extra: &std::collections::BTreeMap<String, String>,
        observer: Option<&crate::proc::ProcessObserver>,
    ) -> i32 {
        if argv.is_empty() {
            return 127;
        }
        let mut env = crate::proc::worker_env_with_cli_path(
            &crate::util::strip_hook_git_env(
                &std::env::vars()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>(),
            ),
            extra,
        );
        env.retain(|kv| {
            !kv.starts_with("SSH_ORIGINAL_COMMAND=")
                && !kv.starts_with("SSH_CONNECTION=")
                && !kv.starts_with("SSH_CLIENT=")
        });
        let cancel = crate::ctx::CancellationToken::new();
        let grace = *HANDS_KILL_AFTER.read().unwrap();
        let out = crate::proc::run_group_observed(
            &cancel,
            argv,
            dir,
            &env,
            &[],
            budget + grace,
            observer,
        );
        match out {
            Ok(o) if o.timeout => REMOTE_EXIT_TIMEOUT,
            Ok(o) => {
                let _ = self.stdout.write_all(o.stdout.as_bytes());
                let _ = self.stderr.write_all(o.stderr.as_bytes());
                o.exit_code
            }
            Err(e) => {
                let _ = writeln!(self.stderr, "{}: {e}", argv[0]);
                127
            }
        }
    }
    pub fn probe(&mut self) -> Result<()> {
        let gitv = Command::new("git")
            .arg("--version")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.split_whitespace().nth(2).map(ToOwned::to_owned))
            .unwrap_or_else(|| "missing".into());
        let token_env = std::env::var("COPILOT_GITHUB_TOKEN").unwrap_or_default()
            + &std::env::var("GH_TOKEN").unwrap_or_default()
            + &std::env::var("GITHUB_TOKEN").unwrap_or_default();
        let mut harness = if !token_env.is_empty()
            || Path::new(&self.home)
                .join(".hive/private/providers/copilot.json")
                .is_file()
        {
            "yes"
        } else {
            "no"
        }
        .to_string();
        let (mut hivev, mut hive_lookup) = match which::which("hive") {
            Ok(p) => (
                p.canonicalize().unwrap_or(p).display().to_string(),
                String::new(),
            ),
            Err(_) => (
                "missing".into(),
                std::env::var("PATH")
                    .unwrap_or_default()
                    .replace(' ', "%20"),
            ),
        };
        if hive_lookup.is_empty() && hivev == "missing" {
            hive_lookup = "-".into();
        }
        if std::env::var("HIVE_HANDS_ALLOW_ANY_WORKER").as_deref() == Ok("1") {
            if hivev == "missing" {
                hivev = "test".into();
            }
            harness = "yes".into();
        }
        let mut jobs = Vec::new();
        let mut keys = Vec::new();
        let mut remotes = 0;
        for e in std::fs::read_dir(&self.workdir)
            .into_iter()
            .flatten()
            .flatten()
        {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let name = e.file_name().to_string_lossy().to_string();
                jobs.push(name.clone());
                if e.path().join("run.key").exists() {
                    keys.push(name.clone());
                }
                let clone = e.path().join("clone");
                if clone.join(".git").is_dir() {
                    let out = Command::new(hive_gitenv::git_program())
                        .args(["-C", &clone.display().to_string(), "remote"])
                        .env_clear()
                        .envs(crate::util::git_env(&[]).into_iter().filter_map(|kv| {
                            kv.split_once('=')
                                .map(|(k, v)| (k.to_string(), v.to_string()))
                        }))
                        .output();
                    if out
                        .ok()
                        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
                        .unwrap_or(false)
                    {
                        remotes += 1;
                    }
                }
            }
        }
        let mut keys_ok = "no".to_string();
        if let Ok(data) =
            std::fs::read_to_string(Path::new(&self.home).join(".ssh/authorized_keys"))
        {
            let mut total = 0;
            let mut pinned = 0;
            for line in data
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
            {
                total += 1;
                if HANDS_PINNED_RE.is_match(line) {
                    pinned += 1;
                }
            }
            if total > 0 && total == pinned {
                keys_ok = "yes".into();
            }
        }
        match std::env::var("HIVE_HANDS_KEYS_OK") {
            Ok(v) if !v.is_empty() => {
                keys_ok = v;
            }
            _ => {}
        }
        let _ = std::fs::create_dir_all(&self.workdir);
        let free = Command::new("df")
            .args(["-Pm", &self.workdir])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| {
                s.lines()
                    .nth(1)
                    .and_then(|l| l.split_whitespace().nth(3).map(ToOwned::to_owned))
            })
            .unwrap_or_else(|| "0".into());
        // The passwd entry of the real uid, not $USER.
        let user = nix::unistd::User::from_uid(nix::unistd::getuid())
            .ok()
            .flatten()
            .map_or_else(|| "?".to_string(), |u| u.name);
        let lookup = if hive_lookup.is_empty() {
            String::new()
        } else {
            format!(" hive_lookup={hive_lookup}")
        };
        let hw = crate::machine::Hardware::probed();
        let dash = |v: Vec<String>| {
            if v.is_empty() {
                "-".into()
            } else {
                v.join(",")
            }
        };
        writeln!(
            self.stdout,
            "hands=ok user={user} workdir={} git={gitv} harness={harness} hive={hivev}{lookup} remotes={remotes} keys_ok={keys_ok} disk_free_mb={free} jobs={} keys={} heartbeat=1 steps=1 recipe=1 push=1 os={} arch={} found={} cores={} mem_mb={}",
            self.workdir,
            dash(jobs),
            dash(keys),
            crate::needs::this_os(),
            crate::needs::this_arch(),
            dash(crate::machine::found_labels().to_vec()),
            hw.cores,
            hw.mem_mb
        )?;
        Ok(())
    }
    pub fn prepare(&mut self, job: &str, branch: &str, agent: &str) -> Result<()> {
        if !HANDS_JOB_RE.is_match(agent) {
            return Err(hands_die("bad agent name"));
        }
        if !HANDS_BRANCH_RE.is_match(branch) {
            return Err(hands_die("bad branch"));
        }
        let dir = self.job_dir(job)?;
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| hands_die(e.to_string()))?;
        let bundle = format!("{dir}/source.bundle");
        let mut data = Vec::new();
        self.stdin.read_to_end(&mut data)?;
        std::fs::write(&bundle, data).map_err(|e| hands_die(e.to_string()))?;
        if !crate::util::git("", &["init", "-q", &format!("{dir}/verify")]).ok() {
            return Err(hands_die("cannot make the scratch repository"));
        }
        if !crate::util::git(&format!("{dir}/verify"), &["bundle", "verify", &bundle]).ok() {
            let _ = std::fs::remove_dir_all(format!("{dir}/verify"));
            return Err(hands_die("the bundle does not verify"));
        }
        let _ = std::fs::remove_dir_all(format!("{dir}/verify"));
        let clone = format!("{dir}/clone");
        let cl = if branch == "HEAD" {
            crate::util::git(
                "",
                &[
                    "-c",
                    "advice.detachedHead=false",
                    "clone",
                    "-q",
                    "--no-tags",
                    &bundle,
                    &clone,
                ],
            )
        } else {
            crate::util::git(
                "",
                &["clone", "-q", "--no-tags", "-b", branch, &bundle, &clone],
            )
        };
        if !cl.ok() {
            return Err(hands_die(cl.stderr.trim().to_string()));
        }
        let _ = std::fs::remove_file(bundle);
        let _ = crate::util::git(&clone, &["remote", "remove", "origin"]);
        let _ = crate::util::git(&clone, &["config", "user.name", agent]);
        let _ = crate::util::git(
            &clone,
            &["config", "user.email", &format!("{agent}@hive.local")],
        );
        let _ = crate::util::git(&clone, &["config", "commit.gpgsign", "false"]);
        let _ = crate::util::git(&clone, &["config", "tag.gpgsign", "false"]);
        write!(
            self.stdout,
            "{}",
            crate::util::git(&clone, &["rev-parse", "HEAD"]).stdout
        )?;
        Ok(())
    }
    pub fn key(&mut self, job: &str, executor: &str) -> Result<()> {
        if !HANDS_JOB_RE.is_match(executor) {
            return Err(hands_die("bad executor name"));
        }
        let dir = self.job_dir(job)?;
        let clone = self.clone_dir(job)?;
        let key = format!("{dir}/run.key");
        let mut data = Vec::new();
        self.stdin.read_to_end(&mut data)?;
        std::fs::write(&key, &data).map_err(|e| hands_die(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600));
        }
        if !String::from_utf8_lossy(&data).contains("-----BEGIN OPENSSH PRIVATE KEY-----") {
            let _ = std::fs::remove_file(&key);
            return Err(hands_die("not an openssh private key"));
        }
        if !crate::util::file_exists(&key) {
            return Err(hands_die(format!(
                "the run's signing key {key} does not exist; refusing to configure user.signingkey (ADR-022)"
            )));
        }
        let agent = crate::util::git(&clone, &["config", "user.name"])
            .stdout
            .trim()
            .to_string();
        if !HANDS_JOB_RE.is_match(&agent) {
            return Err(hands_die("the clone has no agent identity (prepare first)"));
        }
        let hooks = format!("{clone}/.git/hooks");
        for args in [
            vec!["config", "gpg.format", "ssh"],
            vec!["config", "gpg.ssh.program", "ssh-keygen"],
            vec!["config", "user.signingkey", &key],
            vec!["config", "commit.gpgsign", "true"],
            vec!["config", "tag.gpgsign", "false"],
            vec!["config", "core.hooksPath", &hooks],
        ] {
            let _ = crate::util::git(&clone, &args);
        }
        std::fs::create_dir_all(&hooks)?;
        let hook = format!("{hooks}/prepare-commit-msg");
        std::fs::write(
            &hook,
            crate::provenance::provenance_hook(&agent, job, executor),
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755));
        }
        writeln!(self.stdout, "keyed")?;
        Ok(())
    }
    pub fn hook(&mut self, job: &str) -> Result<()> {
        let clone = self.clone_dir(job)?;
        let mut data = Vec::new();
        self.stdin.read_to_end(&mut data)?;
        let hooks = format!("{clone}/.git/hooks");
        std::fs::create_dir_all(&hooks)?;
        let hook = format!("{hooks}/pre-commit");
        std::fs::write(&hook, data)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755));
        }
        writeln!(self.stdout, "hooked")?;
        Ok(())
    }
    pub fn brief(&mut self, job: &str) -> Result<()> {
        let dir = self.job_dir(job)?;
        if !Path::new(&dir).is_dir() {
            return Err(hands_die(format!("no job dir for {job} (prepare first)")));
        }
        let mut data = Vec::new();
        self.stdin.read_to_end(&mut data)?;
        std::fs::write(format!("{dir}/brief.md"), data)?;
        Ok(())
    }
    pub fn run(&mut self, job: &str, minutes: &str, rest: &[String]) -> Result<()> {
        let clone = self.clone_dir(job)?;
        let mut extra = BTreeMap::new();
        let mut i = 0;
        while i < rest.len() && rest[i] != "--" {
            if !hands_env_allowed(&rest[i]) {
                return Err(hands_die(format!(
                    "env key not allowed: {}",
                    rest[i].split_once('=').map(|x| x.0).unwrap_or(&rest[i])
                )));
            }
            let (k, v) = rest[i].split_once('=').unwrap_or((&rest[i], ""));
            extra.insert(k.to_string(), v.to_string());
            i += 1;
        }
        if i >= rest.len() || rest[i] != "--" {
            return Err(hands_die("run needs -- before the worker command"));
        }
        let cmd = &rest[i + 1..];
        if cmd.is_empty() {
            return Err(hands_die("run needs a worker command"));
        }
        if std::env::var("HIVE_HANDS_ALLOW_ANY_WORKER").as_deref() != Ok("1")
            && !hands_allowed_worker(cmd)
        {
            return Err(hands_die("only the hive harness worker may run here"));
        }
        let budget = hands_budget(minutes)?;
        Err(anyhow!(HandsExit {
            code: self.with_timeout_observed(budget, &clone, cmd, &extra, None)
        }))
    }
    pub fn result(&mut self, job: &str) -> Result<()> {
        let clone = self.clone_dir(job)?;
        let data = std::fs::read(format!("{clone}/HIVE_RESULT.md"))
            .map_err(|_| anyhow!(HandsExit { code: 3 }))?;
        self.stdout.write_all(&data)?;
        Ok(())
    }
    pub fn count(&mut self, job: &str, base: &str) -> Result<()> {
        let clone = self.clone_dir(job)?;
        if !Regex::new(r"^[0-9a-f]{40,64}$").unwrap().is_match(base) {
            return Err(hands_die("bad base"));
        }
        let r = crate::util::git(&clone, &["rev-list", "--count", &format!("{base}..HEAD")]);
        write!(self.stdout, "{}", r.stdout)?;
        if !r.ok() {
            write!(self.stderr, "{}", r.stderr)?;
            return Err(anyhow!(HandsExit { code: r.code }));
        }
        Ok(())
    }
    pub fn dirty(&mut self, job: &str) -> Result<()> {
        let clone = self.clone_dir(job)?;
        write!(
            self.stdout,
            "{}",
            crate::util::git(&clone, &["status", "--porcelain", "--untracked-files=no"]).stdout
        )?;
        Ok(())
    }
    /// Prints, base64, a git bundle of the job's clone `HEAD` past `base`
    /// (ADR-070 step 2): a hand that dials out has no sshd for the brain's
    /// `git fetch ssh://`, so its commits come back in the answer to this
    /// verb. `HEAD` at `base` (no commits) prints nothing.
    pub fn bundle(&mut self, job: &str, base: &str) -> Result<()> {
        let clone = self.clone_dir(job)?;
        if !Regex::new(r"^[0-9a-f]{40,64}$").unwrap().is_match(base) {
            return Err(hands_die("bad base"));
        }
        let head = crate::util::git(&clone, &["rev-parse", "HEAD"]);
        if !head.ok() {
            write!(self.stderr, "{}", head.stderr)?;
            return Err(anyhow!(HandsExit { code: head.code }));
        }
        if head.stdout.trim() == base {
            return Ok(());
        }
        let file = format!("{clone}/../result.bundle");
        let r = crate::util::git_timeout(
            &clone,
            Duration::from_secs(300),
            &["bundle", "create", "-q", &file, "HEAD", &format!("^{base}")],
        );
        if !r.ok() {
            let _ = std::fs::remove_file(&file);
            write!(self.stderr, "{}", r.stderr)?;
            return Err(anyhow!(HandsExit { code: r.code }));
        }
        let data = std::fs::read(&file);
        let _ = std::fs::remove_file(&file);
        let data = data.map_err(|e| hands_die(e.to_string()))?;
        write!(
            self.stdout,
            "{}",
            base64::engine::general_purpose::STANDARD.encode(data)
        )?;
        Ok(())
    }
    pub fn changed(&mut self, job: &str, base: &str) -> Result<()> {
        let clone = self.clone_dir(job)?;
        if !Regex::new(r"^[0-9a-f]{40,64}$").unwrap().is_match(base) {
            return Err(hands_die("bad base"));
        }
        let r = crate::util::git(&clone, &["diff", "--name-only", &format!("{base}..HEAD")]);
        write!(self.stdout, "{}", r.stdout)?;
        if !r.ok() {
            write!(self.stderr, "{}", r.stderr)?;
            return Err(anyhow!(HandsExit { code: r.code }));
        }
        Ok(())
    }
    /// Pushes the accepted commit `sha` of the job's clone to `refs/heads/HEAD`
    /// at `url` with the credential the brain leased for this push (ADR-070
    /// step 1b): an SSH private key on stdin, written beside the clone with
    /// mode 0600 for the seconds of the push and removed whatever the push
    /// did. An empty stdin pushes with what git on this machine already
    /// uses. Only a `hive/` branch, and only one the remote does not hold
    /// yet: a branch already there (an open pull request a reviewer may have
    /// pushed to) stays the brain's delivery to update, never this push's to
    /// overwrite.
    pub fn push(&mut self, job: &str, url: &str, sha: &str, head: &str) -> Result<()> {
        let dir = self.job_dir(job)?;
        let clone = self.clone_dir(job)?;
        if url.is_empty() || url.starts_with('-') || url.chars().any(char::is_control) {
            return Err(hands_die("bad push url"));
        }
        if !Regex::new(r"^[0-9a-f]{40,64}$").unwrap().is_match(sha) {
            return Err(hands_die("bad sha"));
        }
        if !HANDS_BRANCH_RE.is_match(head)
            || !head.starts_with("hive/")
            || head.contains("..")
            || head.ends_with('/')
        {
            return Err(hands_die("push: the branch must be a hive/ branch"));
        }
        if !crate::util::git(&clone, &["cat-file", "-e", &format!("{sha}^{{commit}}")]).ok() {
            return Err(hands_die(format!(
                "push: {sha} is not a commit of job {job}"
            )));
        }
        let mut key = Vec::new();
        self.stdin.read_to_end(&mut key)?;
        while key.last().is_some_and(u8::is_ascii_whitespace) {
            key.pop();
        }
        struct Remove(String);
        impl Drop for Remove {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let mut env = Vec::new();
        // A GitHub App installation token (ADR-070 step 3) goes into a
        // credential helper beside the clone, never into it, for this push
        // alone; the helper is removed when the push returns.
        let _helper = match crate::gitcred::token_from_lease(&key) {
            Some(token) => {
                let Some(host) = crate::gitcred::http_host(url) else {
                    return Err(hands_die("push: a leased token needs an https url"));
                };
                let helper = crate::gitcred::Helper::install(Path::new(&dir), &host, &token)
                    .map_err(|e| hands_die(e.to_string()))?;
                env.extend(helper.env(&[]));
                key.clear();
                Some(helper)
            }
            None if key.starts_with(crate::gitcred::TOKEN_LEASE_PREFIX.as_bytes()) => {
                return Err(hands_die("push: the leased token is not a token"));
            }
            None => None,
        };
        let _key_file = if key.is_empty() {
            None
        } else {
            let head_text = String::from_utf8_lossy(&key[..key.len().min(64)]).to_string();
            if !(head_text.starts_with("-----BEGIN ") && head_text.contains("PRIVATE KEY-----")) {
                return Err(hands_die(
                    "push: the leased credential is not an ssh private key",
                ));
            }
            let path = format!("{dir}/push.key");
            let guard = Remove(path.clone());
            let mut f = {
                let mut o = std::fs::OpenOptions::new();
                o.write(true).create(true).truncate(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    o.mode(0o600);
                }
                o.open(&path).map_err(|e| hands_die(e.to_string()))?
            };
            f.write_all(&key)?;
            f.write_all(b"\n")?;
            drop(f);
            // HIVE_HANDS_SSH names another ssh client (a test's stand-in,
            // or one the machine's owner prefers); it gets the same flags.
            let ssh = std::env::var("HIVE_HANDS_SSH")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "ssh".to_string());
            env.push(format!(
                "GIT_SSH_COMMAND={ssh} -oBatchMode=yes -oIdentitiesOnly=yes -oIdentityAgent=none -i '{}'",
                path.replace('\'', r#"'\''"#)
            ));
            Some(guard)
        };
        let lease = format!("--force-with-lease=refs/heads/{head}:");
        let r = crate::util::git_timeout_env(
            &clone,
            Duration::from_secs(300),
            &env,
            &[
                "push",
                "-q",
                &lease,
                url,
                &format!("{sha}:refs/heads/{head}"),
            ],
        );
        if !r.ok() {
            write!(
                self.stderr,
                "{}{}",
                r.stderr,
                r.err.as_deref().unwrap_or("")
            )?;
            return Err(anyhow!(HandsExit {
                code: if r.code == 0 { 1 } else { r.code }
            }));
        }
        writeln!(self.stdout, "pushed {sha} {head}")?;
        Ok(())
    }
    pub fn drop_result(&mut self, job: &str) -> Result<()> {
        let clone = self.clone_dir(job)?;
        if crate::util::git(&clone, &["ls-files", "--error-unmatch", "HIVE_RESULT.md"]).ok() {
            let _ = crate::util::git(&clone, &["rm", "-q", "--cached", "HIVE_RESULT.md"]);
            let _ = crate::util::git(
                &clone,
                &[
                    "commit",
                    "-q",
                    "-m",
                    "runner: drop HIVE_RESULT.md (never committed)",
                ],
            );
            writeln!(self.stdout, "dropped")?;
        } else {
            writeln!(self.stdout, "clean")?;
        }
        Ok(())
    }
    pub fn verify(&mut self, job: &str, minutes: &str, rest: &[String]) -> Result<()> {
        if rest.first().map(String::as_str) != Some("--") {
            return Err(hands_die("verify needs -- before the command"));
        }
        if rest.len() != 2 {
            return Err(hands_die("verify takes one shell command string"));
        }
        let clone = self.clone_dir(job)?;
        let budget = hands_budget(minutes)?;
        Err(anyhow!(HandsExit {
            code: self.with_timeout(
                budget,
                &clone,
                &["bash".into(), "-c".into(), rest[1].clone()],
                &BTreeMap::new()
            )
        }))
    }
    /// Answers the hub's question whether a tool a setup missed is usable
    /// now, so a re-probe does not clear an unfit mark the next setup would
    /// raise again. The check is the hub's line, run the way a setup line is.
    pub fn tool(&mut self, check: &str) -> Result<()> {
        let workdir = self.workdir.clone();
        let _ = std::fs::create_dir_all(&workdir);
        Err(anyhow!(HandsExit {
            code: self.with_timeout(
                Duration::from_secs(60),
                &workdir,
                &["bash".into(), "-c".into(), check.to_string()],
                &BTreeMap::new()
            )
        }))
    }
    pub fn clean(&mut self, job: &str) -> Result<()> {
        let dir = self.job_dir(job)?;
        // Processes the worker left running under the job directory go first;
        // the runner's log keeps the stderr line.
        let mut notes = Vec::new();
        crate::proc::sweep_job_dir(&dir, &mut |line| notes.push(line));
        for note in notes {
            writeln!(self.stderr, "{note}")?;
        }
        // What the agent left registered on the clone goes next and is
        // reported line by line; the runner writes it down against the agent.
        for left in crate::tidy::clear_leftovers(&format!("{dir}/clone")) {
            writeln!(self.stdout, "{}", crate::tidy::leftover_line(&left))?;
        }
        std::fs::remove_dir_all(dir)?;
        Ok(())
    }
    pub fn upload_pack(&mut self, path: &str) -> Result<()> {
        let path = path.trim_matches('\'');
        if !path.starts_with(&(self.workdir.clone() + "/"))
            || !path.ends_with("/clone")
            || path
                .trim_start_matches(&(self.workdir.clone() + "/"))
                .matches('/')
                .count()
                != 1
        {
            return Err(hands_die(format!(
                "upload-pack path must be a job clone under {}",
                self.workdir
            )));
        }
        if path.contains("..") {
            return Err(hands_die("bad path"));
        }
        if !Path::new(path).join(".git").is_dir() {
            return Err(hands_die("no such clone"));
        }
        let mut child = Command::new("git")
            .args(["upload-pack", path])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let status = child.wait()?;
        if !status.success() {
            return Err(anyhow!(HandsExit {
                code: status.code().unwrap_or(1)
            }));
        }
        Ok(())
    }
    pub fn hands_main(&mut self, args: &[String]) -> Result<()> {
        let mut argv = args.to_vec();
        if let Ok(cmd) = std::env::var("SSH_ORIGINAL_COMMAND") {
            if cmd.is_empty() {
                // No forced command override.
            } else {
                if let Some(c) = HANDS_UPLOAD_RE.captures(&cmd) {
                    argv = vec!["git-upload-pack".into(), c[1].to_string()];
                } else if let Some(c) = HANDS_B64_RE.captures(&cmd) {
                    let raw = base64::engine::general_purpose::STANDARD.decode(&c[1]).map_err(|_| hands_die("refused: commands arrive as 'b64 ...' from hive.py or as git-upload-pack"))?;
                    argv = String::from_utf8_lossy(&raw)
                        .split('\0')
                        .map(ToOwned::to_owned)
                        .collect();
                    while argv.last().map(|s| s.is_empty()).unwrap_or(false) {
                        argv.pop();
                    }
                    if argv.is_empty() {
                        return Err(hands_die("empty command"));
                    }
                } else {
                    return Err(hands_die(
                        "refused: commands arrive as 'b64 ...' from hive.py or as git-upload-pack",
                    ));
                }
            }
        }
        if argv.is_empty() {
            return Err(hands_die(
                "no command (this key only runs hive-hands verbs)",
            ));
        }
        let verb = argv[0].as_str();
        let rest = &argv[1..];
        let need = |n: usize, usage: &str| -> Result<()> {
            if rest.len() != n {
                Err(hands_die(usage))
            } else {
                Ok(())
            }
        };
        match verb {
            "probe" => self.probe(),
            "prepare" => {
                need(3, "prepare JOB BRANCH AGENT")?;
                self.prepare(&rest[0], &rest[1], &rest[2])
            }
            "key" => {
                need(2, "key JOB EXECUTOR")?;
                self.key(&rest[0], &rest[1])
            }
            "hook" => {
                need(1, "hook JOB")?;
                self.hook(&rest[0])
            }
            "brief" => {
                need(1, "brief JOB")?;
                self.brief(&rest[0])
            }
            "run" => {
                if rest.len() < 4 {
                    return Err(hands_die("run JOB MINUTES [K=V ...] -- CMD ..."));
                }
                self.run(&rest[0], &rest[1], &rest[2..])
            }
            "result" => {
                need(1, "result JOB")?;
                self.result(&rest[0])
            }
            "count" => {
                need(2, "count JOB BASE")?;
                self.count(&rest[0], &rest[1])
            }
            "dirty" => {
                need(1, "dirty JOB")?;
                self.dirty(&rest[0])
            }
            "changed" => {
                need(2, "changed JOB BASE")?;
                self.changed(&rest[0], &rest[1])
            }
            "push" => {
                need(4, "push JOB URL SHA BRANCH")?;
                self.push(&rest[0], &rest[1], &rest[2], &rest[3])
            }
            "drop-result" => {
                need(1, "drop-result JOB")?;
                self.drop_result(&rest[0])
            }
            "verify" => {
                if rest.len() < 4 {
                    return Err(hands_die("verify JOB MINUTES -- CMD"));
                }
                self.verify(&rest[0], &rest[1], &rest[2..])
            }
            "tool" => {
                need(1, "tool CHECK")?;
                self.tool(&rest[0])
            }
            "clean" => {
                need(1, "clean JOB")?;
                self.clean(&rest[0])
            }
            "bundle" => {
                need(2, "bundle JOB BASE")?;
                self.bundle(&rest[0], &rest[1])
            }
            "git-upload-pack" => {
                need(1, "git-upload-pack PATH")?;
                self.upload_pack(&rest[0])
            }
            "heartbeat" => {
                need(2, "heartbeat JOB SESSION")?;
                self.heartbeat(rest[0].clone(), rest[1].clone())
            }
            "steps" => {
                need(3, "steps JOB SESSION OFFSET")?;
                self.steps(rest[0].clone(), rest[1].clone(), rest[2].clone())
            }
            _ => Err(hands_die(format!("verb not allowed: {verb}"))),
        }
    }
}
pub fn hands_budget(budget: &str) -> Result<Duration> {
    let (n, secs) = if let Some(s) = budget.strip_suffix('s') {
        (
            s.parse::<u64>().map_err(|_| hands_die("bad minutes"))?,
            true,
        )
    } else {
        (
            budget
                .parse::<u64>()
                .map_err(|_| hands_die("bad minutes"))?,
            false,
        )
    };
    if n < 1 {
        return Err(hands_die("the budget must be at least one second"));
    }
    Ok(Duration::from_secs(if secs { n } else { n * 60 }))
}
pub static HANDS_KILL_AFTER: LazyLock<RwLock<Duration>> =
    LazyLock::new(|| RwLock::new(Duration::from_secs(30)));
/// A heartbeat file's temporary name is unique per write.
static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn hands_session_path(
    dir: &str,
    session: &str,
    prefix: &str,
    suffix: &str,
    what: &str,
) -> Result<String> {
    if !HANDS_JOB_RE.is_match(session) || session == "." || session == ".." || session.len() > 160 {
        return Err(hands_die(format!("bad {what} session")));
    }
    Ok(Path::new(dir)
        .join(format!("{prefix}{session}{suffix}"))
        .display()
        .to_string())
}

pub fn hands_heartbeat_path(dir: String, session: String) -> anyhow::Result<String> {
    hands_session_path(&dir, &session, "presence-", ".json", "heartbeat")
}

pub fn hands_steps_path(dir: String, session: String) -> anyhow::Result<String> {
    hands_session_path(&dir, &session, "harness-steps-", ".jsonl", "steps")
}

pub fn write_hands_heartbeat(
    path: String,
    h: crate::beat::ExecutionHeartbeat,
) -> anyhow::Result<()> {
    let data = serde_json::to_vec(&h)?;
    let dir = Path::new(&path)
        .parent()
        .ok_or_else(|| anyhow::anyhow!("heartbeat path has no parent"))?;
    let tmp = dir.join(format!(
        ".presence-{}-{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let cleanup = Cleanup(tmp.clone());
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(&data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path)?;
    std::mem::forget(cleanup);
    Ok(())
}

impl Hands {
    fn owned_job_dir(&self, job: &str) -> Result<String> {
        if !HANDS_JOB_RE.is_match(job) || job == "." || job == ".." {
            return Err(hands_die("bad job"));
        }
        Ok(Path::new(&self.workdir).join(job).display().to_string())
    }

    pub fn heartbeat_observer(
        &mut self,
        job: String,
        env: BTreeMap<String, String>,
    ) -> anyhow::Result<Option<crate::proc::ProcessObserver>> {
        let generation = env
            .get("HIVE_GENERATION")
            .ok_or_else(|| hands_die("bad heartbeat generation"))?
            .parse::<i64>()
            .map_err(|_| hands_die("bad heartbeat generation"))?;
        let id = crate::beat::ExecutionIdentity {
            agent: env.get("HIVE_AGENT").cloned().unwrap_or_default(),
            session: env.get("HIVE_SESSION").cloned().unwrap_or_default(),
            run_id: env.get("HIVE_SESSION").cloned().unwrap_or_default(),
            job_id: job.clone(),
            generation,
            executor: env.get("HIVE_EXECUTOR").cloned().unwrap_or_default(),
        };
        id.validate()?;
        let dir = self.owned_job_dir(&job)?;
        let path = hands_heartbeat_path(dir, id.session.clone())?;
        let beat = Arc::new(std::sync::Mutex::new(crate::beat::ExecutionHeartbeat {
            identity: id,
            at: String::new(),
            stopped_at: String::new(),
        }));
        let reported = Arc::new(AtomicBool::new(false));
        let beat_for_beat = Arc::clone(&beat);
        let reported_for_beat = Arc::clone(&reported);
        let path_for_beat = path.clone();
        let beat_cb = move || {
            let mut h = beat_for_beat.lock().expect("heartbeat lock");
            h.at = crate::util::rfc3339_nano(chrono::Utc::now());
            if let Err(err) = write_hands_heartbeat(path_for_beat.clone(), h.clone())
                && !reported_for_beat.swap(true, Ordering::SeqCst)
            {
                let _ = err;
            }
        };
        let beat_for_stop = Arc::clone(&beat);
        let reported_for_stop = Arc::clone(&reported);
        let stop_cb = move || {
            let mut h = beat_for_stop.lock().expect("heartbeat lock");
            if !h.at.is_empty() {
                h.stopped_at = crate::util::rfc3339_nano(chrono::Utc::now());
                if let Err(err) = write_hands_heartbeat(path.clone(), h.clone())
                    && !reported_for_stop.swap(true, Ordering::SeqCst)
                {
                    let _ = err;
                }
            }
        };
        Ok(Some(crate::proc::ProcessObserver {
            beat: Some(Arc::new(beat_cb)),
            stop: Some(Arc::new(stop_cb)),
        }))
    }
    pub fn heartbeat(&mut self, job: String, session: String) -> anyhow::Result<()> {
        let dir = self.owned_job_dir(&job)?;
        let path = hands_heartbeat_path(dir, session.clone())?;
        let mut f = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(anyhow::anyhow!(HandsExit { code: 3 }));
            }
            Err(e) => return Err(e.into()),
        };
        let mut data = Vec::new();
        let mut limited = std::io::Read::take(&mut f, 4097);
        limited.read_to_end(&mut data)?;
        let beat: crate::beat::ExecutionHeartbeat =
            serde_json::from_slice(&data).map_err(|_| hands_die("invalid heartbeat evidence"))?;
        if data.len() > 4096
            || beat.validate().is_err()
            || beat.job_id != job
            || beat.session != session
        {
            return Err(hands_die("invalid heartbeat evidence"));
        }
        self.stdout.write_all(&data)?;
        Ok(())
    }
    pub fn steps(
        &mut self,
        job: String,
        session: String,
        offset_text: String,
    ) -> anyhow::Result<()> {
        let dir = self.owned_job_dir(&job)?;
        let path = hands_steps_path(dir, session)?;
        let offset = offset_text
            .parse::<u64>()
            .map_err(|_| hands_die("bad steps offset"))?;
        let mut f = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(anyhow::anyhow!(HandsExit { code: 3 }));
            }
            Err(e) => return Err(e.into()),
        };
        f.seek(SeekFrom::Start(offset))?;
        let mut data = Vec::new();
        let mut limited = std::io::Read::take(&mut f, 1024 * 1024);
        limited.read_to_end(&mut data)?;
        if let Some(i) = data.iter().rposition(|b| *b == b'\n') {
            data.truncate(i + 1);
        } else {
            data.clear();
        }
        self.stdout.write_all(&data)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hands(workdir: &str, input: &str) -> Hands {
        Hands {
            workdir: workdir.into(),
            home: tempfile::tempdir().unwrap().path().display().to_string(),
            stdin: Box::new(std::io::Cursor::new(input.as_bytes().to_vec())),
            stdout: Box::new(Vec::<u8>::new()),
            stderr: Box::new(Vec::<u8>::new()),
        }
    }

    #[test]
    fn hands_tool_answers_with_the_checks_exit_code() {
        let _serial = crate::test_helpers::serial();
        let workdir = tempfile::tempdir().unwrap();
        let mut h = make_hands(&workdir.path().display().to_string(), "");
        for (check, code) in [
            ("exit 0", 0),
            ("exit 3", 3),
            ("command -v no-such-tool-hive >/dev/null", 1),
        ] {
            let err = h.hands_main(&["tool".into(), check.into()]).unwrap_err();
            assert_eq!(crate::test_helpers::hands_exit_code(err), code, "{check}");
        }
        let err = h.hands_main(&["tool".into()]).unwrap_err().to_string();
        assert!(err.contains("tool CHECK"), "{err}");
    }

    /// `probe` says `heartbeat=1 steps=1`, and the brain then calls these two
    /// verbs (heartbeat.rs, steps_mirror.rs) through the same dispatch every
    /// other verb takes; a refusal there leaves a remote job with no presence
    /// and no live steps.
    #[test]
    fn hands_main_answers_the_heartbeat_and_steps_verbs_it_advertises() {
        let _serial = crate::test_helpers::serial();
        let workdir = tempfile::tempdir().unwrap();
        let dir = workdir.path().join("job-live");
        std::fs::create_dir_all(&dir).unwrap();
        let wd = workdir.path().display().to_string();
        #[derive(Clone, Default)]
        struct Buf(Arc<std::sync::Mutex<Vec<u8>>>);
        impl Write for Buf {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let with_out = |buf: &Buf| -> Hands {
            let mut h = make_hands(&wd, "");
            h.stdout = Box::new(buf.clone());
            h
        };
        let text = |buf: &Buf| String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        let code_of = |args: &[&str]| -> i32 {
            let mut h = make_hands(&wd, "");
            let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            match h.hands_main(&args) {
                Ok(()) => 0,
                Err(e) => crate::test_helpers::hands_exit_code(e),
            }
        };
        // Nothing written yet: exit 3, the "not yet" the brain waits through.
        assert_eq!(code_of(&["heartbeat", "job-live", "run-live"]), 3);
        assert_eq!(code_of(&["steps", "job-live", "run-live", "0"]), 3);
        // A wrong argument count is a usage error, not a refusal.
        for (args, usage) in [
            (&["heartbeat", "job-live"][..], "heartbeat JOB SESSION"),
            (
                &["steps", "job-live", "run-live"][..],
                "steps JOB SESSION OFFSET",
            ),
        ] {
            let mut h = make_hands(&wd, "");
            let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            let err = h.hands_main(&args).unwrap_err().to_string();
            assert!(err.contains(usage), "{err}");
        }

        let id = crate::beat::ExecutionIdentity {
            agent: "live".into(),
            session: "run-live".into(),
            run_id: "run-live".into(),
            job_id: "job-live".into(),
            generation: 2,
            executor: "hands".into(),
        };
        let beat = hands_heartbeat_path(dir.display().to_string(), id.session.clone()).unwrap();
        write_hands_heartbeat(
            beat,
            crate::beat::ExecutionHeartbeat {
                identity: id.clone(),
                at: "2026-11-10T03:00:00Z".into(),
                stopped_at: String::new(),
            },
        )
        .unwrap();
        let buf = Buf::default();
        with_out(&buf)
            .hands_main(&["heartbeat".into(), "job-live".into(), "run-live".into()])
            .unwrap();
        let got: crate::beat::ExecutionHeartbeat = serde_json::from_str(&text(&buf)).unwrap();
        assert_eq!(got.identity, id);

        let steps = hands_steps_path(dir.display().to_string(), "run-live".into()).unwrap();
        std::fs::write(&steps, "{\"event\":\"begin\"}\npart").unwrap();
        let buf = Buf::default();
        with_out(&buf)
            .hands_main(&[
                "steps".into(),
                "job-live".into(),
                "run-live".into(),
                "0".into(),
            ])
            .unwrap();
        assert_eq!(text(&buf), "{\"event\":\"begin\"}\n");
    }

    #[test]
    fn hands_outcome_exits_with_the_commands_own_code_zero_included() {
        let exit_of = |res: Result<()>, stderr: &mut Vec<u8>| match hands_outcome(res, stderr) {
            Ok(()) => 0,
            Err(e) => e
                .downcast_ref::<crate::error::Error>()
                .map(|ce| ce.exit_code())
                .unwrap_or(-1),
        };
        let mut stderr = Vec::new();
        assert_eq!(exit_of(Ok(()), &mut stderr), 0);
        assert_eq!(exit_of(Err(anyhow!(HandsExit { code: 0 })), &mut stderr), 0);
        assert_eq!(exit_of(Err(anyhow!(HandsExit { code: 7 })), &mut stderr), 7);
        assert!(stderr.is_empty(), "{}", String::from_utf8_lossy(&stderr));
        assert_eq!(exit_of(Err(hands_die("bad base")), &mut stderr), 2);
        assert_eq!(
            String::from_utf8_lossy(&stderr).trim(),
            "hive-hands: bad base"
        );
        stderr.clear();
        assert_eq!(exit_of(Err(anyhow!("disk gone")), &mut stderr), 1);
        assert!(String::from_utf8_lossy(&stderr).contains("disk gone"));
    }

    #[test]
    fn hands_take_a_recipes_env_and_pins_but_keep_their_own_names() {
        for ok in [
            "HIVE_AGENT=aurora",
            "GIT_TERMINAL_PROMPT=0",
            "CARGO_TARGET_DIR=/srv/hive/target",
            "CI=1",
            "MISE_IGNORED_CONFIG_PATHS=/srv/hive/j/clone",
            "MISE_YES=1",
            "_X=",
        ] {
            assert!(hands_env_allowed(ok), "{ok} refused");
        }
        for bad in [
            "PATH=/evil",
            "HOME=/elsewhere",
            "LD_PRELOAD=/evil.so",
            "DYLD_INSERT_LIBRARIES=/evil.dylib",
            "SSH_AUTH_SOCK=/x",
            "SECRET=x",
            "NPM_TOKEN=x",
            "lower=1",
            "NOEQUALS",
        ] {
            assert!(!hands_env_allowed(bad), "{bad} accepted");
        }
        let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(hands_allowed_worker(&argv(&["hive", "harness", "run"])));
        assert!(hands_allowed_worker(&argv(&[
            "mise",
            "exec",
            "node@22.1.0",
            "pnpm@9.1.0",
            "--",
            "hive",
            "harness",
            "run",
            "-p",
            "x"
        ])));
        for bad in [
            vec!["mise", "exec", "--", "hive", "harness", "run"],
            vec!["mise", "exec", "node", "--", "hive", "harness", "run"],
            vec!["mise", "exec", "node@22.1.0", "hive", "harness", "run"],
            vec!["mise", "exec", "node@22.1.0", "--", "sh", "-c", "true"],
            vec!["mise", "exec", "node@22.1.0", "--"],
            vec!["mise", "run", "--", "hive", "harness", "run"],
        ] {
            assert!(!hands_allowed_worker(&argv(&bad)), "{bad:?} allowed");
        }
    }

    #[test]
    fn hands_push_uses_the_leased_key_and_removes_it() {
        let _serial = crate::test_helpers::serial();
        let job = "job-20261109T000000Z-push01";
        let workdir = tempfile::tempdir().unwrap();
        let clone = workdir.path().join(job).join("clone");
        std::fs::create_dir_all(&clone).unwrap();
        let run = |args: &[&str]| crate::test_helpers::run(&clone, args);
        run(&["git", "init", "-q", "-b", "main"]);
        run(&["git", "config", "user.name", "t"]);
        run(&["git", "config", "user.email", "t@x"]);
        run(&["git", "commit", "-q", "--allow-empty", "-m", "work"]);
        let sha = String::from_utf8(
            std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&clone)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();
        let origin = workdir.path().join("origin.git");
        crate::test_helpers::run(
            workdir.path(),
            &["git", "init", "-q", "--bare", "origin.git"],
        );
        // A stand-in ssh: it records the key it was given, then runs the
        // remote command locally.
        let seen = workdir.path().join("seen.key");
        let fake = workdir.path().join("fake-ssh");
        std::fs::write(
            &fake,
            format!(
                "#!/bin/sh\nwhile [ $# -gt 1 ]; do case \"$1\" in -i) shift; cat \"$1\" > '{}';; esac; shift; done\nexec sh -c \"$1\"\n",
                seen.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        unsafe { std::env::set_var("HIVE_HANDS_SSH", &fake) };
        let url = format!("ssh://example.invalid{}", origin.display());
        let key =
            "-----BEGIN OPENSSH PRIVATE KEY-----\nleased\n-----END OPENSSH PRIVATE KEY-----\n";
        let wd = workdir.path().display().to_string();

        let mut h = make_hands(&wd, "junk");
        let err = h
            .push(job, &url, &sha, "hive/a/job")
            .unwrap_err()
            .to_string();
        assert!(err.contains("not an ssh private key"), "{err}");
        let mut h = make_hands(&wd, key);
        let err = h.push(job, &url, &sha, "main").unwrap_err().to_string();
        assert!(err.contains("hive/ branch"), "{err}");

        let mut h = make_hands(&wd, key);
        h.push(job, &url, &sha, "hive/a/job").unwrap();
        unsafe { std::env::remove_var("HIVE_HANDS_SSH") };
        assert_eq!(std::fs::read_to_string(&seen).unwrap(), key);
        assert!(!workdir.path().join(job).join("push.key").exists());
        let got = std::process::Command::new("git")
            .args(["rev-parse", "refs/heads/hive/a/job"])
            .current_dir(&origin)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&got.stdout).trim(), sha);

        // A branch the remote already holds is not this push's to move.
        run(&["git", "commit", "-q", "--allow-empty", "-m", "more"]);
        let mut h = make_hands(&wd, "");
        let again = String::from_utf8(
            std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&clone)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap();
        let url = format!("file://{}", origin.display());
        assert!(h.push(job, &url, again.trim(), "hive/a/job").is_err());
    }

    /// ADR-070 step 3: a lease that carries a GitHub App installation
    /// token pushes over https through a credential helper the hand writes
    /// beside the clone for that one push and removes after it; the token
    /// is in no output and no file of the clone.
    #[test]
    fn hands_push_with_a_leased_app_token_uses_a_helper_and_removes_it() {
        let _serial = crate::test_helpers::serial();
        if !hive_testutil::githttp::available() {
            return;
        }
        let job = "job-20261110T000000Z-push02";
        let token = "ghs_handLeaseToken42";
        let workdir = tempfile::tempdir().unwrap();
        let clone = workdir.path().join(job).join("clone");
        std::fs::create_dir_all(&clone).unwrap();
        let run = |args: &[&str]| crate::test_helpers::run(&clone, args);
        run(&["git", "init", "-q", "-b", "main"]);
        run(&["git", "commit", "-q", "--allow-empty", "-m", "work"]);
        let sha = run(&["git", "rev-parse", "HEAD"]).trim().to_string();
        let served = tempfile::tempdir().unwrap();
        let bare = served.path().join("o").join("r.git");
        std::fs::create_dir_all(&bare).unwrap();
        crate::test_helpers::run(&bare, &["git", "init", "-q", "--bare"]);
        use base64::Engine;
        let auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"))
        );
        let srv = hive_testutil::githttp::Server::new(served.path(), &auth);
        let url = format!("{}/o/r.git", srv.url());
        let wd = workdir.path().display().to_string();
        let lease = String::from_utf8(crate::gitcred::token_lease(token)).unwrap();

        // A token is for an https url only.
        let mut h = make_hands(&wd, &lease);
        let err = h
            .push(job, "ssh://example.invalid/o/r.git", &sha, "hive/a/job")
            .unwrap_err()
            .to_string();
        assert!(err.contains("https url"), "{err}");
        let mut h = make_hands(&wd, "github-app-token bad token");
        let err = h
            .push(job, &url, &sha, "hive/a/job")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("not a token") && !err.contains("bad token"),
            "{err}"
        );

        let mut h = make_hands(&wd, &lease);
        h.push(job, &url, &sha, "hive/a/job").unwrap();
        let pushed = crate::util::git(
            bare.to_str().unwrap(),
            &["rev-parse", "refs/heads/hive/a/job"],
        );
        assert_eq!(pushed.stdout.trim(), sha, "{}", pushed.stderr);
        assert!(
            srv.seen().contains(&auth),
            "the token never reached the server"
        );
        let left: Vec<_> = std::fs::read_dir(workdir.path().join(job))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            left,
            vec!["clone".to_string()],
            "the helper outlived the push"
        );
        let config = std::fs::read_to_string(clone.join(".git").join("config")).unwrap();
        assert!(
            !config.contains(token) && !config.contains("credential"),
            "{config}"
        );
    }

    #[test]
    fn hands_key_refuses_signing_key_without_key_file() {
        let _serial = crate::test_helpers::serial();
        if unsafe { libc::getuid() } == 0 {
            return;
        }
        let job = "job-20260918T000000Z-abcdef";
        for (name, stdin, readonly, want_err, want_key) in [
            (
                "write fails",
                "-----BEGIN OPENSSH PRIVATE KEY-----\nx\n",
                true,
                "permission denied",
                false,
            ),
            (
                "not a key",
                "junk",
                false,
                "not an openssh private key",
                false,
            ),
            (
                "ok",
                "-----BEGIN OPENSSH PRIVATE KEY-----\nx\n",
                false,
                "",
                true,
            ),
        ] {
            let workdir = tempfile::tempdir().unwrap();
            let dir = workdir.path().join(job);
            let clone = dir.join("clone");
            std::fs::create_dir_all(&clone).unwrap();
            crate::test_helpers::run(&clone, &["git", "init", "-q"]);
            crate::test_helpers::run(&clone, &["git", "config", "user.name", "aurora"]);
            if readonly {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
                }
            }
            let mut h = make_hands(&workdir.path().display().to_string(), stdin);
            let err = h
                .key(job, "exec")
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            if !want_err.is_empty() {
                assert!(err.to_lowercase().contains(want_err), "{name}: {err}");
            } else {
                assert!(err.is_empty(), "{name}: {err}");
            }
            if readonly {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755));
                }
            }
            let out = std::process::Command::new(hive_gitenv::git_program())
                .args(["config", "--local", "--get", "user.signingkey"])
                .current_dir(&clone)
                .env_clear()
                .envs(crate::util::git_env(&[]).into_iter().filter_map(|kv| {
                    kv.split_once('=')
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                }))
                .output()
                .unwrap();
            let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if want_key {
                assert_eq!(got, dir.join("run.key").display().to_string());
            } else {
                assert!(got.is_empty(), "{name}: user.signingkey={got}");
                assert!(!dir.join("run.key").exists(), "{name}: run.key left behind");
            }
        }
    }
}
