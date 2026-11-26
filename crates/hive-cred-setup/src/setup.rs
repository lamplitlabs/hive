//! Credential setup: resolve, probe, save, report, clear, generate and browse push credentials.

use anyhow::{Result as AnyResult, anyhow};
use rusqlite::OptionalExtension;
use serde::Serialize;
use std::fmt;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

/// Probe function seam used by tests.
pub type ProbeFn = Arc<dyn Fn(&str, &str, &str) -> AnyResult<()> + Send + Sync>;

/// Key generation seam used by tests.
pub type KeygenFn = Arc<dyn Fn(&str, &str) -> AnyResult<()> + Send + Sync>;

/// Deps are the seams tests replace; `None` fields mean the real thing.
#[derive(Clone, Default)]
pub struct Deps {
    pub probe: Option<ProbeFn>,
    pub keygen: Option<KeygenFn>,
}

impl Deps {
    fn probe(&self) -> ProbeFn {
        self.probe.clone().unwrap_or_else(|| Arc::new(probe_remote))
    }

    fn keygen(&self) -> KeygenFn {
        self.keygen.clone().unwrap_or_else(|| Arc::new(keygen))
    }
}

/// Refusal is why a step did not happen, in words safe to show.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Refusal {
    #[serde(skip)]
    pub code: i32,
    pub what: String,
    pub why: String,
    pub fix: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub fingerprint: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub public_key: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub probes: Vec<Probe>,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.what, self.why)
    }
}

impl std::error::Error for Refusal {}

/// Probe is one project's upstream asked whether the key may push.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Probe {
    pub project: String,
    pub branch: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub remote: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub skipped: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub why: String,
}

/// Result is a saved reference and what was learned on the way.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Result {
    pub reference: String,
    pub fingerprint: String,
    pub public_key: String,
    pub source: String,
    pub probed: bool,
    pub probes: Vec<Probe>,
    pub landing_push: String,
}

/// The refusal for a pasted key or token.
pub fn not_a_reference() -> Refusal {
    Refusal {
        code: 2,
        what: "That is the key, not a reference to it".into(),
        why: "hive stores where the key lives, never the key (ADR-041).".into(),
        fix: "hive cred set \"op://VAULT/ITEM/private key?ssh-format=openssh\"".into(),
        ..Refusal::default()
    }
}

/// Resolves text as a reference, checks the secret, probes upstreams and saves it.
pub fn apply(
    s: &hive_store::Store,
    r: &hive_cred::Resolver,
    actor: &str,
    text: &str,
    probe: bool,
    deps: Deps,
) -> AnyResult<Result> {
    let text = text.trim();
    if hive_cred::is_private_key(text.as_bytes()) || hive_store::secret_shaped(text) {
        return Err(not_a_reference().into());
    }
    let reference = hive_cred::parse(text).map_err(|err| Refusal {
        code: 2,
        what: "Not a push credential reference".into(),
        why: err.to_string(),
        fix: "hive help cred".into(),
        ..Refusal::default()
    })?;
    let secret = r.resolve(reference.clone()).map_err(|err| Refusal {
        code: 1,
        what: "The key could not be read".into(),
        why: hive_cred::redact(&err.to_string(), &[]),
        fix: "Fix the reference or the item it names, then try again. Nothing was saved.".into(),
        ..Refusal::default()
    })?;
    let material = r.materialize(&secret).map_err(|err| Refusal {
        code: 1,
        what: "Could not write the key for the check".into(),
        why: hive_cred::redact(&err.to_string(), &[]),
        fix: "Check HIVE_HOME is writable.".into(),
        ..Refusal::default()
    })?;
    let (pubkey, fingerprint) = match r.inspect(&material.key_path) {
        Ok(v) => v,
        Err(err) => {
            material.cleanup();
            return Err(Refusal {
                code: 1,
                what: "The secret is not a usable SSH private key".into(),
                why: hive_cred::redact(&err.to_string(), &[]),
                fix: "For a 1Password SSH Key item the reference ends in /private key?ssh-format=openssh. Nothing was saved.".into(),
                ..Refusal::default()
            }
            .into());
        }
    };

    let mut out = Result {
        reference: reference.to_string(),
        fingerprint: fingerprint.clone(),
        public_key: pubkey.clone(),
        source: secret.source.clone(),
        probed: probe,
        probes: Vec::new(),
        landing_push: String::new(),
    };
    if probe {
        for p in s.projects(true)? {
            if p.slug == hive_store::COMMONS_SLUG || p.path.is_empty() {
                continue;
            }
            let branch = default_branch(&p.default_branch);
            let mut pr = Probe {
                project: p.slug.clone(),
                branch: branch.clone(),
                remote: upstream_remote(&p.path, &branch),
                ..Probe::default()
            };
            if pr.remote.is_empty() {
                pr.skipped = format!(
                    "no upstream on {branch} (git branch --set-upstream-to=origin/{branch} {branch})"
                );
                out.probes.push(pr);
                continue;
            }
            if let Err(err) = deps.probe()(&p.path, &pr.remote, &material.ssh_command) {
                pr.why = hive_cred::redact(&err.to_string(), &[]);
                out.probes.push(pr.clone());
                material.cleanup();
                return Err(Refusal {
                    code: 1,
                    what: format!("The key does not open {}'s remote {}", p.slug, pr.remote),
                    why: pr.why,
                    fix: "Add the public key to the repository as a deploy key with write access (or to a machine user), then try again. Nothing was saved.".into(),
                    fingerprint,
                    public_key: pubkey,
                    probes: out.probes,
                }
                .into());
            }
            pr.ok = true;
            out.probes.push(pr);
        }
    }
    material.cleanup();

    s.set_setting(actor, "push.credential", &reference.to_string())?;
    s.event(
        actor,
        "cred.set",
        "push.credential",
        serde_json::json!({
            "reference": reference.to_string(),
            "fingerprint": fingerprint,
            "source": secret.source,
            "probed": probe,
        }),
    )?;
    out.landing_push = s.setting("landing.push", "off");
    Ok(out)
}

/// Report is the state without secrets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    pub reference: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub scheme: String,
    pub landing_push: String,
    pub token: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub set_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub set_by: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub fingerprint: String,
}

/// Reads the state. The fingerprint comes from the cred.set event; nothing is resolved.
pub fn status(s: &hive_store::Store, r: &hive_cred::Resolver) -> Report {
    let mut rep = Report {
        reference: s.setting("push.credential", ""),
        landing_push: s.setting("landing.push", "off"),
        token: "none".into(),
        ..Report::default()
    };
    if !rep.reference.is_empty() {
        if let Ok(reference) = hive_cred::parse(&rep.reference) {
            rep.scheme = reference.scheme;
        }
        if let Ok(conn) = s.conn() {
            let _ = conn
                .query_row(
                    "SELECT updated_at, updated_by FROM settings WHERE name = 'push.credential'",
                    [],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .map(|(at, by)| {
                    rep.set_at = at;
                    rep.set_by = by;
                });
            let _ = conn
                .query_row(
                    "SELECT json_extract(detail, '$.fingerprint') FROM events WHERE action = 'cred.set' AND json_extract(detail, '$.reference') = ? ORDER BY seq DESC LIMIT 1",
                    [&rep.reference],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
                .map(|v| {
                    rep.fingerprint = v.flatten().unwrap_or_default();
                });
        }
    }
    if let Ok((_, source)) = r.token() {
        rep.token = source;
    }
    rep
}

/// Cleared says what Clear removed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Cleared {
    pub was: String,
    pub token_removed: Vec<String>,
}

/// Unsets push.credential and optionally removes the stored service-account token.
pub fn clear(
    s: &hive_store::Store,
    r: &hive_cred::Resolver,
    actor: &str,
    token: bool,
) -> AnyResult<Cleared> {
    let mut out = Cleared {
        was: s.setting("push.credential", ""),
        token_removed: Vec::new(),
    };
    if !out.was.is_empty() {
        s.set_setting(actor, "push.credential", "")?;
        s.event(
            actor,
            "cred.clear",
            "push.credential",
            serde_json::json!({ "was": out.was }),
        )?;
    }
    if token {
        out.token_removed = r.clear_token()?;
    }
    Ok(out)
}

/// GeneratedKeyName is the file under $HIVE_HOME/secrets the console mints.
pub const GENERATED_KEY_NAME: &str = "push_key";

/// Generated is a key of the daemon's own on this machine: never the private half.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Generated {
    pub path: String,
    pub reference: String,
    pub fingerprint: String,
    pub public_key: String,
    pub existed: bool,
}

/// Mints an ed25519 key under $HIVE_HOME/secrets/push_key.
pub fn generate(r: &hive_cred::Resolver, replace: bool, deps: Deps) -> AnyResult<Generated> {
    if r.worker || std::env::var("HIVE_JOB").is_ok_and(|v| !v.is_empty()) {
        return Err(hive_cred::ErrWorker.into());
    }
    let dir = Path::new(&r.home).join("secrets");
    let path = dir.join(GENERATED_KEY_NAME);
    let path_s = path.display().to_string();
    let mut out = Generated {
        path: path_s.clone(),
        reference: format!("file://{path_s}"),
        ..Generated::default()
    };
    if path.symlink_metadata().is_ok() {
        if !replace {
            let (pubkey, fp) = r.inspect(&path_s).map_err(|err| Refusal {
                code: 1,
                what: format!("A file is already at {path_s} but is not a usable key"),
                why: hive_cred::redact(&err.to_string(), &[]),
                fix: "Generate again with replace to overwrite it.".into(),
                ..Refusal::default()
            })?;
            out.existed = true;
            out.public_key = pubkey;
            out.fingerprint = fp;
            return Ok(out);
        }
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("pub"));
    }
    fs::create_dir_all(&dir).map_err(|err| Refusal {
        code: 1,
        what: format!("Could not create {}", dir.display()),
        why: err.to_string(),
        fix: "Check HIVE_HOME is writable.".into(),
        ..Refusal::default()
    })?;
    set_mode(&dir, 0o700);
    let mut comment = "hive push key".to_string();
    if let Ok(host) = std::env::var("HOSTNAME")
        && !host.is_empty()
    {
        comment.push_str(&format!(" ({host})"));
    } else if let Ok(host) = std::env::var("COMPUTERNAME")
        && !host.is_empty()
    {
        comment.push_str(&format!(" ({host})"));
    }
    deps.keygen()(&path_s, &comment).map_err(|err| Refusal {
        code: 1,
        what: "ssh-keygen could not make the key".into(),
        why: hive_cred::redact(&err.to_string(), &[]),
        fix: "Install OpenSSH's ssh-keygen and try again.".into(),
        ..Refusal::default()
    })?;
    set_mode(&path, 0o600);
    let (pubkey, fp) = r.inspect(&path_s).map_err(|err| Refusal {
        code: 1,
        what: "The generated key could not be read back".into(),
        why: hive_cred::redact(&err.to_string(), &[]),
        fix: format!("Remove {path_s} and generate again."),
        ..Refusal::default()
    })?;
    out.public_key = pubkey;
    out.fingerprint = fp;
    Ok(out)
}

fn keygen(path: &str, comment: &str) -> AnyResult<()> {
    let mut cmd = Command::new("ssh-keygen");
    cmd.args(["-q", "-t", "ed25519", "-N", "", "-C", comment, "-f", path])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let out = child.wait_with_output()?;
            let text = output_text(&out.stdout, &out.stderr);
            if text.is_empty() {
                return Err(anyhow!("signal: killed"));
            }
            return Err(anyhow!(text));
        }
        thread::sleep(Duration::from_millis(50));
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let text = text.trim();
        if text.is_empty() {
            return Err(anyhow!("exit status {}", out.status));
        }
        return Err(anyhow!(text.to_string()));
    }
    Ok(())
}

/// Vault is one 1Password vault the service account may read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Vault {
    pub id: String,
    pub name: String,
}

/// Item is one SSH Key item in a vault.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub category: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub updated_at: String,
    pub reference: String,
}

/// Lists what the service account can see: `op vault list`.
pub fn vaults(r: &hive_cred::Resolver) -> AnyResult<Vec<Vault>> {
    let (out, _) = r.op(&["vault", "list", "--format", "json"])?;
    let vaults: Vec<Vault> = serde_json::from_str(&out)
        .map_err(|_| anyhow!("op vault list answered something that is not JSON"))?;
    Ok(vaults)
}

/// Lists the SSH Key items of one vault.
pub fn items(r: &hive_cred::Resolver, vault: &str) -> AnyResult<Vec<Item>> {
    let vault = vault.trim();
    if vault.is_empty() {
        return Err(Refusal {
            code: 2,
            what: "A vault is required".into(),
            why: "op item list needs --vault.".into(),
            fix: "Pick a vault first.".into(),
            ..Refusal::default()
        }
        .into());
    }
    let (out, _) = r.op(&[
        "item",
        "list",
        "--vault",
        vault,
        "--categories",
        "SSH Key",
        "--format",
        "json",
    ])?;
    #[derive(serde::Deserialize)]
    struct RawItem {
        id: String,
        title: String,
        category: String,
        #[serde(default)]
        updated_at: String,
        #[serde(default)]
        vault: Vault,
    }
    let raw: Vec<RawItem> = serde_json::from_str(&out)
        .map_err(|_| anyhow!("op item list answered something that is not JSON"))?;
    Ok(raw
        .into_iter()
        .map(|it| {
            let v = if it.vault.id.is_empty() && it.vault.name.is_empty() {
                Vault {
                    id: vault.into(),
                    name: vault.into(),
                }
            } else {
                it.vault
            };
            Item {
                reference: op_reference(&v, &it.title, &it.id),
                id: it.id,
                title: it.title,
                category: it.category,
                updated_at: it.updated_at,
            }
        })
        .collect())
}

/// Builds the op:// reference to an SSH Key item's private key.
pub fn op_reference(v: &Vault, title: &str, id: &str) -> String {
    let vault = if plain_name(&v.name) { &v.name } else { &v.id };
    let item = if plain_name(title) { title } else { id };
    format!("op://{vault}/{item}/private key?ssh-format=openssh")
}

fn plain_name(s: &str) -> bool {
    !s.is_empty() && s == s.trim() && !s.contains(['/', '?', '\n'])
}

/// Field is something the owner types for a provider that has no listing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Field {
    pub name: String,
    pub label: String,
    pub placeholder: String,
}

/// Provider is one place a push key can live.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Provider {
    pub id: String,
    pub title: String,
    pub blurb: String,
    pub available: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub unavailable: String,
    pub needs_token: bool,
    pub browse: bool,
    pub generate: bool,
    pub fields: Vec<Field>,
    pub example: String,
}

/// Lists the providers on this machine, unavailable ones included with the reason.
pub fn providers(r: &hive_cred::Resolver) -> Vec<Provider> {
    let mut op = Provider {
        id: "op".into(),
        title: "1Password".into(),
        blurb: "An SSH Key item in a vault of its own, read with a service account that sees only that vault and never needs the app unlocked.".into(),
        available: r.op_available(),
        needs_token: true,
        browse: true,
        fields: Vec::new(),
        example: "op://Hive/hive push key/private key?ssh-format=openssh".into(),
        ..Provider::default()
    };
    if !op.available {
        op.unavailable = "op not found; install 1Password CLI (brew install 1password-cli)".into();
    }
    let mut keychain = Provider {
        id: "keychain".into(),
        title: "macOS Keychain".into(),
        blurb: "A generic password item in the login Keychain holding the private key.".into(),
        available: r.os() == "darwin",
        fields: vec![
            Field {
                name: "service".into(),
                label: "Service".into(),
                placeholder: "hive".into(),
            },
            Field {
                name: "account".into(),
                label: "Account".into(),
                placeholder: "push-key".into(),
            },
        ],
        example: "keychain://hive/push-key".into(),
        ..Provider::default()
    };
    if !keychain.available {
        keychain.unavailable = "the Keychain is macOS only; use 1Password or a file here".into();
    }
    let key_path = Path::new(&r.home).join("secrets").join(GENERATED_KEY_NAME);
    let file = Provider {
        id: "file".into(),
        title: "A file on this machine".into(),
        blurb: "A private key file only this user can read (mode 0600). The console can make one for the daemon.".into(),
        available: true,
        generate: true,
        fields: vec![Field {
            name: "path".into(),
            label: "Path".into(),
            placeholder: key_path.display().to_string(),
        }],
        example: format!("file://{}", key_path.display()),
        ..Provider::default()
    };
    vec![op, keychain, file]
}

/// "main" when a project names none.
pub fn default_branch(b: &str) -> String {
    if b.is_empty() {
        "main".into()
    } else {
        b.into()
    }
}

/// The remote the branch tracks, or "".
pub fn upstream_remote(root: &str, branch: &str) -> String {
    let out = Command::new(hive_gitenv::git_program())
        .args([
            "-C",
            root,
            "for-each-ref",
            "--format=%(upstream:remotename)",
            &format!("refs/heads/{}", default_branch(branch)),
        ])
        .env_clear()
        .envs(env_pairs(hive_gitenv::env()))
        .output();
    match out {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => String::new(),
    }
}

/// Checks that the key opens the remote for writing without changing it:
/// a dry-run push of a branch nobody has. Git offers the key over ssh only,
/// so on a remote whose url is GitHub's https one the dry run fails for want
/// of a credential however good the key is; when that is all git says, it
/// is made once more against the url's ssh form, the rule the daemon's own
/// pushes follow (`hive_gitenv::forge`), and that answer counts.
pub fn probe_remote(root: &str, remote: &str, ssh_command: &str) -> AnyResult<()> {
    let first = dry_run(root, remote, ssh_command)?;
    let Err(https_said) = first else {
        return Ok(());
    };
    let Some(ssh) = hive_gitenv::forge::ssh_retry_url(root, &push_url(root, remote), &https_said)
    else {
        return Err(anyhow!(https_said));
    };
    match dry_run(root, &ssh, ssh_command)? {
        Ok(()) => Ok(()),
        Err(ssh_said) => Err(anyhow!(
            "failed over https ({https_said}), then over ssh ({ssh}): {ssh_said}"
        )),
    }
}

/// The url a push to `remote` goes to: its push url, else its url, else
/// the name itself (git reads it the same way).
fn push_url(root: &str, remote: &str) -> String {
    for args in [
        ["remote", "get-url", "--push", remote].as_slice(),
        ["remote", "get-url", remote].as_slice(),
    ] {
        let out = Command::new(hive_gitenv::git_program())
            .args(["-C", root])
            .args(args)
            .env_clear()
            .envs(env_pairs(hive_gitenv::env()))
            .output();
        if let Ok(out) = out
            && out.status.success()
        {
            let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !url.is_empty() {
                return url;
            }
        }
    }
    remote.to_string()
}

/// One dry-run push to `to` (a remote's name or a url) with the key's ssh
/// command: Ok(Ok) when git would push, Ok(Err) with what git said when it
/// would not, Err when git could not be run.
fn dry_run(root: &str, to: &str, ssh_command: &str) -> AnyResult<std::result::Result<(), String>> {
    let mut cmd = Command::new(hive_gitenv::git_program());
    cmd.args([
        "-C",
        root,
        "push",
        "--dry-run",
        "--porcelain",
        to,
        "HEAD:refs/heads/hive-cred-probe-does-not-exist",
    ])
    .env_clear()
    .envs(env_pairs(hive_gitenv::env()))
    .env("GIT_SSH_COMMAND", ssh_command)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            let out = child.wait_with_output()?;
            if out.status.success() {
                return Ok(Ok(()));
            }
            let text = output_text(&out.stdout, &out.stderr);
            return Ok(Err(if text.is_empty() {
                format!("exit status {}", out.status)
            } else {
                text
            }));
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let out = child.wait_with_output()?;
            let text = output_text(&out.stdout, &out.stderr);
            return Err(anyhow!(format!(
                "timed out after 60s: {}",
                if text.is_empty() {
                    "signal: killed".to_string()
                } else {
                    text
                }
            )));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn output_text(stdout: &[u8], stderr: &[u8]) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    )
    .trim()
    .to_string()
}

fn env_pairs(env: Vec<String>) -> Vec<(String, String)> {
    env.into_iter()
        .filter_map(|kv| {
            kv.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect()
}

#[cfg(unix)]
fn set_mode(path: impl AsRef<Path>, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_mode(_path: impl AsRef<Path>, _mode: u32) {}

#[cfg(test)]
mod tests {
    use super::*;
    use hive_cred::{RunError, Runner, TOKEN_ENV};
    use hive_store::{Options, Store};
    use std::collections::BTreeMap;
    use std::ops::Deref;
    use std::sync::{Arc, Mutex};
    use tempfile::{TempDir, tempdir};

    const TEST_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----\n";
    const TEST_TOKEN: &str = "ops_abcdefghijklmnopqrstuvwxyz0123456789";

    struct TestStore {
        store: Store,
        _home: TempDir,
        _guard: hive_testutil::EnvGuard,
    }

    impl Deref for TestStore {
        type Target = Store;

        fn deref(&self) -> &Self::Target {
            &self.store
        }
    }

    fn setup_store() -> TestStore {
        let guard = hive_testutil::env_lock();
        for key in [
            "HIVE_JOB",
            "HIVE_AGENT",
            "HIVE_PROJECT",
            "HIVE_SESSION",
            "OP_SERVICE_ACCOUNT_TOKEN",
        ] {
            unsafe { std::env::set_var(key, "") };
        }
        let home = tempdir().unwrap();
        unsafe { std::env::set_var("HIVE_HOME", home.path().display().to_string()) };
        let store = Store::open(Options {
            home: home.path().display().to_string(),
            create: true,
            now: "2026-09-15T06:00:00Z".into(),
            ..Options::default()
        })
        .unwrap();
        TestStore {
            store,
            _home: home,
            _guard: guard,
        }
    }

    #[derive(Clone, Default)]
    struct FakeOp {
        calls: Arc<Mutex<Vec<String>>>,
        env: Arc<Mutex<Vec<Vec<String>>>>,
        out: Arc<Mutex<BTreeMap<String, String>>>,
        fail: Arc<Mutex<String>>,
    }

    impl FakeOp {
        fn with_out(out: BTreeMap<String, String>) -> Self {
            Self {
                out: Arc::new(Mutex::new(out)),
                ..Self::default()
            }
        }

        fn runner(&self) -> Runner {
            let calls = Arc::clone(&self.calls);
            let envs = Arc::clone(&self.env);
            let out = Arc::clone(&self.out);
            let fail = Arc::clone(&self.fail);
            Arc::new(move |name, args, _stdin, env| {
                calls
                    .lock()
                    .unwrap()
                    .push(format!("{name} {}", args.join(" ")));
                if let Some(env) = env {
                    envs.lock().unwrap().push(env.to_vec());
                }
                match name {
                    "op" => {
                        let verb = if args.first().is_some_and(|a| a == "read") {
                            "read".to_string()
                        } else {
                            args.iter().take(2).cloned().collect::<Vec<_>>().join(" ")
                        };
                        if *fail.lock().unwrap() == verb {
                            return Err(RunError::new(format!(
                                "[ERROR] 2026/09/15 06:00:00 unauthorized: token {TEST_TOKEN} rejected"
                            )));
                        }
                        Ok((
                            out.lock().unwrap().get(&verb).cloned().unwrap_or_default(),
                            String::new(),
                        ))
                    }
                    "ssh-keygen" => {
                        if args.first().is_some_and(|a| a == "-y") {
                            Ok((
                                "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFakePublicKey hive-push\n"
                                    .into(),
                                String::new(),
                            ))
                        } else {
                            Ok((
                                "256 SHA256:FakeFingerprint0123456789 hive-push (ED25519)\n".into(),
                                String::new(),
                            ))
                        }
                    }
                    _ => Err(RunError::new("not faked")),
                }
            })
        }
    }

    fn resolver(home: &str, f: &FakeOp, goos: &str) -> hive_cred::Resolver {
        hive_cred::Resolver {
            home: home.into(),
            run: Some(f.runner()),
            goos: goos.into(),
            getenv: Some(Arc::new(|k| {
                if k == TOKEN_ENV {
                    TEST_TOKEN.into()
                } else {
                    String::new()
                }
            })),
            ..hive_cred::Resolver::default()
        }
    }

    #[test]
    fn op_reference_uses_names_while_they_survive() {
        let _serial = crate::test_helpers::serial();
        let v = Vault {
            id: "vid1".into(),
            name: "Hive".into(),
        };
        assert_eq!(
            op_reference(&v, "hive push key", "iid1"),
            "op://Hive/hive push key/private key?ssh-format=openssh"
        );
        assert_eq!(
            op_reference(
                &Vault {
                    id: "vid1".into(),
                    name: "ops/prod".into()
                },
                "key?",
                "iid1"
            ),
            "op://vid1/iid1/private key?ssh-format=openssh"
        );
        assert_eq!(
            op_reference(
                &Vault {
                    id: "vid1".into(),
                    name: " padded ".into()
                },
                "",
                "iid1"
            ),
            "op://vid1/iid1/private key?ssh-format=openssh"
        );
        for reference in [
            op_reference(&v, "hive push key", "iid1"),
            op_reference(
                &Vault {
                    id: "vid1".into(),
                    name: String::new(),
                },
                "a/b",
                "iid1",
            ),
        ] {
            hive_cred::parse(&reference).unwrap();
        }
    }

    #[test]
    fn vaults_and_items_list_through_op() {
        let _serial = crate::test_helpers::serial();
        let f = FakeOp::with_out(BTreeMap::from([
            (
                "vault list".into(),
                r#"[{"id":"v1","name":"Hive"},{"id":"v2","name":"ops/shared"}]"#.into(),
            ),
            (
                "item list".into(),
                r#"[{"id":"i1","title":"hive push key","category":"SSH_KEY","updated_at":"2026-09-14T10:00:00Z","vault":{"id":"v1","name":"Hive"}}]"#.into(),
            ),
        ]));
        let home = tempdir().unwrap();
        let r = resolver(&home.path().display().to_string(), &f, "linux");
        let got_vaults = vaults(&r).unwrap();
        assert_eq!(got_vaults.len(), 2);
        assert_eq!(got_vaults[0].name, "Hive");
        let got_items = items(&r, "v1").unwrap();
        assert_eq!(got_items.len(), 1);
        assert_eq!(
            got_items[0].reference,
            "op://Hive/hive push key/private key?ssh-format=openssh"
        );
        assert_eq!(got_items[0].title, "hive push key");
        assert!(
            f.calls.lock().unwrap()[1]
                .contains("item list --vault v1 --categories SSH Key --format json")
        );
        for env in f.env.lock().unwrap().iter() {
            assert!(
                env.iter()
                    .any(|kv| kv == &format!("{TOKEN_ENV}={TEST_TOKEN}"))
            );
            assert!(
                !env.iter().any(|kv| kv.starts_with("OP_CONNECT_")),
                "connect variables must be stripped"
            );
        }
        assert!(items(&r, " ").is_err());
        f.out
            .lock()
            .unwrap()
            .insert("vault list".into(), "not json".into());
        let err = vaults(&r).unwrap_err().to_string();
        assert!(err.contains("op vault list answered something that is not JSON"));
        assert!(!err.contains("not json"));
        *f.fail.lock().unwrap() = "vault list".into();
        let err = vaults(&r).unwrap_err().to_string();
        assert!(!err.contains(TEST_TOKEN));
        let empty = FakeOp::with_out(BTreeMap::from([("vault list".into(), "[]".into())]));
        let home = tempdir().unwrap();
        let got = vaults(&resolver(
            &home.path().display().to_string(),
            &empty,
            "linux",
        ))
        .unwrap();
        assert_eq!(got.len(), 0);
    }

    #[test]
    fn providers_follow_the_machine() {
        let _serial = crate::test_helpers::serial();
        let f = FakeOp::default();
        let mac = providers(&resolver("/h", &f, "darwin"));
        assert_eq!(mac.len(), 3);
        assert_eq!(mac[0].id, "op");
        assert!(mac[0].available);
        assert!(mac[0].browse);
        assert!(mac[0].needs_token);
        assert!(mac[1].available);
        assert_eq!(mac[1].id, "keychain");
        assert_eq!(mac[1].fields.len(), 2);
        assert!(mac[2].generate);
        assert_eq!(mac[2].example, "file:///h/secrets/push_key");
        let lin = providers(&resolver("/h", &f, "linux"));
        assert!(!lin[1].available);
        assert!(!lin[1].unavailable.is_empty());
    }

    #[test]
    fn generate_then_apply_a_file_key() {
        let _serial = crate::test_helpers::serial();
        if Command::new("ssh-keygen").arg("-h").output().is_err() {
            return;
        }
        let s = setup_store();
        let mut r = hive_cred::new(&s.home);
        r.goos = "linux".into();
        let g = generate(&r, false, Deps::default()).unwrap();
        assert!(g.public_key.starts_with("ssh-ed25519 "));
        assert!(g.fingerprint.starts_with("SHA256:"));
        assert!(!g.existed);
        assert_eq!(
            g.reference,
            format!(
                "file://{}",
                Path::new(&s.home).join("secrets/push_key").display()
            )
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&g.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let again = generate(&r, false, Deps::default()).unwrap();
        assert!(again.existed);
        assert_eq!(again.fingerprint, g.fingerprint);
        let replaced = generate(&r, true, Deps::default()).unwrap();
        assert!(!replaced.existed);
        assert_ne!(replaced.fingerprint, g.fingerprint);

        let probes = Arc::new(Mutex::new(Vec::new()));
        let probes_sink = Arc::clone(&probes);
        let ok = Deps {
            probe: Some(Arc::new(move |_root, remote, ssh| {
                probes_sink.lock().unwrap().push(format!("{remote} {ssh}"));
                Ok(())
            })),
            keygen: None,
        };
        let root = push_project(&s, "alpha");
        let res = apply(&s, &r, "human", &replaced.reference, true, ok).unwrap();
        assert_eq!(res.fingerprint, replaced.fingerprint);
        assert_eq!(res.source, "file");
        assert_eq!(res.probes.len(), 1);
        assert!(res.probes[0].ok);
        assert_eq!(res.probes[0].project, "alpha");
        assert_eq!(s.setting("push.credential", ""), replaced.reference);
        assert_eq!(probes.lock().unwrap().len(), 1);
        assert!(probes.lock().unwrap()[0].contains("-oIdentityAgent=none -i '"));
        assert!(fs::metadata(root).is_ok());
        let rep = status(&s, &r);
        assert_eq!(rep.reference, replaced.reference);
        assert_eq!(rep.fingerprint, replaced.fingerprint);
        assert_eq!(rep.scheme, "file");
        assert_eq!(rep.set_by, "human");

        let cleared = clear(&s, &r, "human", false).unwrap();
        assert_eq!(cleared.was, replaced.reference);
        let refuse = Deps {
            probe: Some(Arc::new(|_, _, _| {
                Err(anyhow!(
                    "ERROR: The key you are authenticating with has been marked as read only."
                ))
            })),
            keygen: None,
        };
        let err = apply(&s, &r, "human", &replaced.reference, true, refuse).unwrap_err();
        let refu = err.downcast_ref::<Refusal>().unwrap();
        assert_eq!(refu.public_key, replaced.public_key);
        assert_eq!(refu.code, 1);
        assert!(refu.why.contains("read only"));
        assert_eq!(refu.probes.len(), 1);
        assert!(!refu.probes[0].ok);
        assert!(!refu.probes[0].why.is_empty());
        assert_eq!(s.setting("push.credential", ""), "");
        let key_text = fs::read_to_string(&replaced.path).unwrap();
        let err = apply(&s, &r, "human", &key_text, false, Deps::default()).unwrap_err();
        let refu = err.downcast_ref::<Refusal>().unwrap();
        assert_eq!(refu.code, 2);
        assert!(!refu.to_string().contains("PRIVATE KEY-----\n"));
        let res = apply(
            &s,
            &r,
            "human",
            &replaced.reference,
            false,
            Deps {
                probe: Some(Arc::new(|_, _, _| Err(anyhow!("no")))),
                keygen: None,
            },
        )
        .unwrap();
        assert!(!res.probed);
        assert!(res.probes.is_empty());
        let _guard = hive_testutil::env_lock();
        unsafe { std::env::set_var("HIVE_JOB", "job-1") };
        assert!(
            generate(&r, true, Deps::default())
                .unwrap_err()
                .is::<hive_cred::ErrWorker>()
        );
        unsafe { std::env::set_var("HIVE_JOB", "") };
    }

    #[test]
    fn apply_through_op_names_the_item_not_the_key() {
        let _serial = crate::test_helpers::serial();
        let s = setup_store();
        let f = FakeOp::with_out(BTreeMap::from([("read".into(), TEST_KEY.into())]));
        let r = resolver(&s.home, &f, "linux");
        let res = apply(
            &s,
            &r,
            "human:dashboard",
            "op://Hive/hive push key/private key?ssh-format=openssh",
            false,
            Deps::default(),
        )
        .unwrap();
        assert_eq!(res.source, "1Password (token from the environment)");
        assert_eq!(res.fingerprint, "SHA256:FakeFingerprint0123456789");
        let tmp = Path::new(&s.home).join("tmp");
        if tmp.exists() {
            assert_eq!(fs::read_dir(tmp).unwrap().count(), 0);
        }
        *f.fail.lock().unwrap() = "read".into();
        let err = apply(
            &s,
            &r,
            "human:dashboard",
            "op://Hive/missing/private key?ssh-format=openssh",
            false,
            Deps::default(),
        )
        .unwrap_err();
        let refu = err.downcast_ref::<Refusal>().unwrap();
        assert!(!refu.why.contains(TEST_TOKEN));
        assert_eq!(refu.what, "The key could not be read");
    }

    /// `hive cred set` probes the key the way the daemon will push with
    /// it: on a remote whose url is GitHub's https one, git offers the key
    /// over ssh only, so when GitHub asking for a credential is all git
    /// says, the dry run is made once more over the url's ssh form and
    /// that answer counts. On another host nothing goes over ssh.
    #[test]
    fn the_probe_reaches_an_https_github_remote_over_ssh_with_the_key() {
        let s = setup_store();
        if !hive_testutil::githttp::available() {
            return;
        }
        let root = push_project(&s, "https-origin");
        let dir = tempdir().unwrap();
        let served = dir.path().join("owner").join("repo.git");
        fs::create_dir_all(served.parent().unwrap()).unwrap();
        let bare = Path::new(&s.home).join("https-origin.git");
        let out = Command::new(hive_gitenv::git_program())
            .args(["clone", "-q", "--bare"])
            .arg(&bare)
            .arg(&served)
            .env_clear()
            .envs(env_pairs(hive_testutil::git_env()))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let srv = hive_testutil::githttp::Server::new(dir.path(), "Basic nothing-has-this");
        let url = format!("{}/owner/repo.git", srv.url());
        let out = Command::new(hive_gitenv::git_program())
            .args(["-C", &root, "remote", "set-url", "origin", &url])
            .output()
            .unwrap();
        assert!(out.status.success());
        // GitHub's ssh as seen from here: records its arguments, then runs
        // git's command where the repository is served, or refuses.
        let record = dir.path().join("ssh-record");
        let ssh = dir.path().join("fake-ssh");
        fs::write(
            &ssh,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nif [ -n \"$HIVE_TEST_SSH_REFUSE\" ]; then echo \"$HIVE_TEST_SSH_REFUSE\" >&2; exit 255; fi\nfor a in \"$@\"; do last=\"$a\"; done\ncd '{}' && exec sh -c \"$last\"\n",
                record.display(),
                dir.path().display()
            ),
        )
        .unwrap();
        set_mode(&ssh, 0o755);
        let ssh_command = format!("'{}' -i /dev/null", ssh.display());

        // On another forge's address the https url has no ssh form here.
        let _unset = hive_testutil::unsetenv("HIVE_FORGE_WEB");
        let err = probe_remote(&root, "origin", &ssh_command).unwrap_err();
        assert!(err.to_string().contains("could not read Username"), "{err}");
        assert!(
            !record.exists(),
            "ssh was tried for a url that is not GitHub's"
        );

        let _web = hive_testutil::setenv("HIVE_FORGE_WEB", srv.url());
        probe_remote(&root, "origin", &ssh_command).unwrap();
        let args = fs::read_to_string(&record).unwrap();
        assert!(
            args.contains("git@127.0.0.1\n") && args.contains("-i\n"),
            "{args}"
        );

        let _refuse = hive_testutil::setenv(
            "HIVE_TEST_SSH_REFUSE",
            "git@github.com: Permission denied (publickey).",
        );
        let err = probe_remote(&root, "origin", &ssh_command)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("failed over https") && err.contains("Permission denied (publickey)"),
            "{err}"
        );
        let remote = Command::new(hive_gitenv::git_program())
            .args(["-C", &root, "remote", "get-url", "origin"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&remote.stdout).trim(), url);
    }

    fn push_project(s: &Store, slug: &str) -> String {
        let root = Path::new(&s.home).join(format!("project-{slug}"));
        let bare = Path::new(&s.home).join(format!("{slug}.git"));
        let root_s = root.display().to_string();
        let bare_s = bare.display().to_string();
        let root_arg = root_s.as_str();
        let bare_arg = bare_s.as_str();
        for args in [
            vec!["init", "-q", "--bare", bare_arg],
            vec!["init", "-q", "-b", "main", root_arg],
            vec![
                "-C",
                root_arg,
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "init",
            ],
            vec!["-C", root_arg, "remote", "add", "origin", bare_arg],
            vec!["-C", root_arg, "push", "-q", "-u", "origin", "main"],
        ] {
            let mut cmd = Command::new(hive_gitenv::git_program());
            cmd.args(args)
                .env_clear()
                .envs(env_pairs(hive_testutil::git_env()));
            let out = cmd.output().unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO projects (slug,name,path,default_branch,added_at) VALUES (?,?,?,?,?)",
                rusqlite::params![slug, slug, root_s, "main", s.now()],
            )
            .unwrap();
        root.display().to_string()
    }
}
