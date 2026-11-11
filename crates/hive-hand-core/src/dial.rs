//! The hand's side of the dial-out channel (ADR-070 step 2): `hive-hand
//! join CODE` enrols this machine with the brain once, `hive-hand dial`
//! then holds the outbound connection and runs the verbs the brain leases.
//! Nothing listens here: no sshd, no open port, no key from the brain.
//!
//! The device key is an ed25519 pair `ssh-keygen` makes in the hand's own
//! directory ([`hand_home`], mode 0700; the private key 0600); only its
//! public half leaves the machine. Each connect signs a fresh brain nonce
//! with it (`ssh-keygen -Y sign`, namespace [`SIGN_NAMESPACE`]).
//!
//! HTTP goes through the system `curl`, as git and ssh-keygen do: the hand
//! carries no HTTP stack of its own.

use crate::ctx::CancellationToken;
use anyhow::{Result, anyhow};
use base64::Engine;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The `ssh-keygen -Y` namespace of a connect signature; the brain verifies
/// in the same one.
pub const SIGN_NAMESPACE: &str = "hive-hand";
/// What the hand keeps about its enrolment, beside the key.
pub const DEVICE_FILE: &str = "device.json";
/// The device key's private half (the public half is `device_key.pub`).
pub const KEY_FILE: &str = "device_key";

/// The hand's own directory: `HIVE_HAND_HOME`, else `~/.hive-hand`.
pub fn hand_home() -> PathBuf {
    match std::env::var_os("HIVE_HAND_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".hive-hand"),
    }
}

/// An enrolled hand: which brain, which device, which executor it works as.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Device {
    pub brain: String,
    pub device: String,
    pub executor: String,
    pub workdir: String,
}

/// The device this hand enrolled as, if it did.
pub fn load_device(home: &Path) -> Result<Device> {
    let text = std::fs::read_to_string(home.join(DEVICE_FILE)).map_err(|_| {
        anyhow!(
            "this machine is not enrolled ({} has no {DEVICE_FILE}); run `hive-hand join <code>` with a code from the Machines page",
            home.display()
        )
    })?;
    Ok(serde_json::from_str(&text)?)
}

/// What one call to the brain answered.
#[derive(Debug, Clone)]
pub struct Answer {
    pub status: u16,
    pub body: Value,
}

impl Answer {
    fn ok(self, what: &str) -> Result<Value> {
        if self.status == 200 {
            return Ok(self.body);
        }
        let why = self.body["error"].as_str().unwrap_or("no reason given");
        let fix = self.body["fix"].as_str().unwrap_or("");
        Err(anyhow!(
            "{what} refused ({}): {why}{}",
            self.status,
            if fix.is_empty() {
                String::new()
            } else {
                format!(" - {fix}")
            }
        ))
    }
}

/// POSTs `body` as JSON to `{brain}/api/v1/hands/{route}` with curl.
pub fn post(
    cancel: &CancellationToken,
    brain: &str,
    route: &str,
    body: &Value,
    budget: Duration,
) -> Result<Answer> {
    let url = format!("{}/api/v1/hands/{route}", brain.trim_end_matches('/'));
    let argv: Vec<String> = [
        "curl",
        "-sS",
        "-X",
        "POST",
        "-H",
        "Content-Type: application/json",
        "-H",
        "Expect:",
        "--data-binary",
        "@-",
        "--max-time",
        &budget.as_secs().max(1).to_string(),
        "-w",
        "\n%{http_code}",
        &url,
    ]
    .iter()
    .map(|a| a.to_string())
    .collect();
    let out = crate::proc::run_group(
        cancel,
        &argv,
        "",
        &crate::proc::environ(),
        body.to_string().as_bytes(),
        budget + Duration::from_secs(5),
    )?;
    if out.timeout {
        return Err(anyhow!("{url}: no answer in {}s", budget.as_secs()));
    }
    if out.exit_code != 0 {
        return Err(anyhow!(
            "{url}: curl exit {}: {}",
            out.exit_code,
            out.stderr.trim()
        ));
    }
    let (text, code) = out.stdout.rsplit_once('\n').unwrap_or(("", &out.stdout));
    let status = code.trim().parse::<u16>().unwrap_or(0);
    let body = serde_json::from_str(text).unwrap_or(Value::Null);
    Ok(Answer { status, body })
}

fn keygen(home: &Path) -> Result<String> {
    std::fs::create_dir_all(home)?;
    set_mode(home, 0o700);
    let key = home.join(KEY_FILE);
    let public = home.join(format!("{KEY_FILE}.pub"));
    if !key.exists() || !public.exists() {
        let _ = std::fs::remove_file(&key);
        let _ = std::fs::remove_file(&public);
        let made = std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", "hive-hand", "-f"])
            .arg(&key)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| anyhow!("ssh-keygen: {e} (OpenSSH makes the device key)"))?;
        if !made.status.success() {
            return Err(anyhow!(
                "ssh-keygen failed: {}",
                String::from_utf8_lossy(&made.stderr).trim()
            ));
        }
    }
    set_mode(&key, 0o600);
    Ok(std::fs::read_to_string(public)?.trim().to_string())
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

/// What `hive-hand join` says about this machine.
#[derive(Debug, Clone, Default)]
pub struct JoinOptions {
    pub brain: String,
    pub code: String,
    pub name: String,
    pub labels: String,
    pub workers: i64,
    pub workdir: String,
}

/// Enrols this machine: makes the device key (once), sends its public half
/// with the code and the machine's facts, keeps the answer in
/// [`DEVICE_FILE`]. A code is good once; joining again needs a new one and
/// replaces the enrolment.
pub fn join(cancel: &CancellationToken, home: &Path, o: &JoinOptions) -> Result<Device> {
    if o.brain.is_empty() {
        return Err(anyhow!(
            "no brain to join: pass --brain URL (the Machines page's line has it) or set HIVE_BRAIN"
        ));
    }
    let public_key = keygen(home)?;
    let name = if o.name.is_empty() {
        hostname()
    } else {
        o.name.clone()
    };
    let workdir = if o.workdir.is_empty() {
        home.join("work").display().to_string()
    } else {
        o.workdir.clone()
    };
    std::fs::create_dir_all(&workdir)?;
    let answer = post(
        cancel,
        &o.brain,
        "join",
        &json!({
            "code": o.code,
            "public_key": public_key,
            "name": name,
            "os": crate::needs::this_os(),
            "arch": crate::needs::this_arch(),
            "labels": o.labels,
            "workers": o.workers.max(1),
            "workdir": workdir,
        }),
        Duration::from_secs(60),
    )?
    .ok("join")?;
    let device = Device {
        brain: o.brain.trim_end_matches('/').to_string(),
        device: answer["device"].as_str().unwrap_or_default().to_string(),
        executor: answer["executor"].as_str().unwrap_or_default().to_string(),
        workdir,
    };
    if device.device.is_empty() {
        return Err(anyhow!("the brain answered the join without a device id"));
    }
    let file = home.join(DEVICE_FILE);
    std::fs::write(&file, serde_json::to_string_pretty(&device)? + "\n")?;
    set_mode(&file, 0o600);
    Ok(device)
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "hand".to_string())
}

/// The bytes a connect signs; the brain builds the same.
pub fn connect_message(device: &str, nonce: &str) -> String {
    format!("hive-hand connect {device} {nonce}\n")
}

/// Signs `message` with the device key: an armored SSH signature.
pub fn sign(home: &Path, message: &str) -> Result<String> {
    let key = home.join(KEY_FILE);
    let out = crate::proc::run_group(
        &CancellationToken::new(),
        &[
            "ssh-keygen".to_string(),
            "-q".to_string(),
            "-Y".to_string(),
            "sign".to_string(),
            "-f".to_string(),
            key.display().to_string(),
            "-n".to_string(),
            SIGN_NAMESPACE.to_string(),
        ],
        "",
        &crate::proc::environ(),
        message.as_bytes(),
        Duration::from_secs(30),
    )?;
    if out.timeout || out.exit_code != 0 || !out.stdout.contains("BEGIN SSH SIGNATURE") {
        return Err(anyhow!("signing the nonce failed: {}", out.stderr.trim()));
    }
    Ok(out.stdout)
}

/// Signs in: asks for a nonce, signs it, trades the signature for a
/// session token.
pub fn connect(cancel: &CancellationToken, home: &Path, d: &Device) -> Result<String> {
    let nonce = post(
        cancel,
        &d.brain,
        "nonce",
        &json!({"device": d.device}),
        Duration::from_secs(30),
    )?
    .ok("nonce")?;
    let nonce = nonce["nonce"].as_str().unwrap_or_default().to_string();
    let signature = sign(home, &connect_message(&d.device, &nonce))?;
    let answer = post(
        cancel,
        &d.brain,
        "connect",
        &json!({"device": d.device, "nonce": nonce, "signature": signature}),
        Duration::from_secs(30),
    )?
    .ok("connect")?;
    answer["token"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("the brain answered the connect without a token"))
}

/// Runs one leased verb with `hand` (this `hive-hand`), exactly as sshd's
/// forced command would: the `b64` line in `SSH_ORIGINAL_COMMAND`, the
/// payload on stdin.
pub fn run_verb(
    cancel: &CancellationToken,
    hand: &Path,
    workdir: &str,
    args: &[String],
    stdin: &[u8],
    timeout: Duration,
) -> (i32, String, String) {
    let runner = crate::transport::local_hand_runner(
        hand.to_path_buf(),
        vec![
            ("HIVE_HANDS_WORKDIR".to_string(), workdir.to_string()),
            ("HIVE_HANDS_KEYS_OK".to_string(), "yes".to_string()),
        ],
    );
    let argv = vec![
        hand.display().to_string(),
        crate::transport::hands_command_line(args),
    ];
    match runner(cancel, &argv, stdin, timeout) {
        Ok(v) => v,
        Err(e) => (
            crate::hands::REMOTE_EXIT_TIMEOUT,
            String::new(),
            format!("hive-hand: {e}"),
        ),
    }
}

/// How a dial session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialEnd {
    /// The cancel token was cancelled (a signal, a test).
    Stopped,
    /// The brain refused the device: it was revoked or never enrolled.
    Refused(String),
}

/// Holds the dial channel until `cancel` or a refusal: connects, long-polls
/// for leased verbs and runs each on its own thread (the brain may probe
/// while a worker runs), posting every outcome back. A lapsed session
/// connects again; a network failure waits and tries again.
pub fn dial(
    cancel: &CancellationToken,
    home: &Path,
    hand: &Path,
    log: &(dyn Fn(&str) + Sync),
) -> Result<DialEnd> {
    let d = load_device(home)?;
    let mut token = String::new();
    let mut pause = Duration::from_secs(1);
    while !cancel.is_cancelled() {
        if token.is_empty() {
            match connect(cancel, home, &d) {
                Ok(t) => {
                    token = t;
                    pause = Duration::from_secs(1);
                    log(&format!(
                        "connected to {} as {} ({})",
                        d.brain, d.executor, d.device
                    ));
                }
                Err(e) if e.to_string().contains("refused (403)") => {
                    return Ok(DialEnd::Refused(e.to_string()));
                }
                Err(e) => {
                    log(&format!(
                        "connect failed: {e}; trying again in {}s",
                        pause.as_secs()
                    ));
                    let _ = cancel.done().recv_timeout(pause);
                    pause = (pause * 2).min(Duration::from_secs(60));
                    continue;
                }
            }
        }
        let answer = match post(
            cancel,
            &d.brain,
            "poll",
            &json!({"token": token, "wait_secs": 20}),
            Duration::from_secs(40),
        ) {
            Ok(a) => a,
            Err(e) => {
                if cancel.is_cancelled() {
                    break;
                }
                log(&format!(
                    "poll failed: {e}; trying again in {}s",
                    pause.as_secs()
                ));
                let _ = cancel.done().recv_timeout(pause);
                pause = (pause * 2).min(Duration::from_secs(60));
                continue;
            }
        };
        match answer.status {
            200 => {}
            401 => {
                token.clear();
                continue;
            }
            403 => {
                let why = answer.body["error"]
                    .as_str()
                    .unwrap_or("refused")
                    .to_string();
                return Ok(DialEnd::Refused(why));
            }
            other => {
                log(&format!(
                    "poll answered {other}; trying again in {}s",
                    pause.as_secs()
                ));
                let _ = cancel.done().recv_timeout(pause);
                pause = (pause * 2).min(Duration::from_secs(60));
                continue;
            }
        }
        pause = Duration::from_secs(1);
        let call = &answer.body["call"];
        if call.is_null() {
            continue;
        }
        let id = call["id"].as_i64().unwrap_or(0);
        let args: Vec<String> = call["args"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|v| v.as_str().unwrap_or_default().to_string())
                    .collect()
            })
            .unwrap_or_default();
        let stdin = base64::engine::general_purpose::STANDARD
            .decode(call["stdin"].as_str().unwrap_or_default())
            .unwrap_or_default();
        let timeout = Duration::from_secs(call["timeout_secs"].as_u64().unwrap_or(600).max(1));
        log(&format!(
            "call {id}: {}",
            args.first().map(String::as_str).unwrap_or("?")
        ));
        let cancel = cancel.clone();
        let hand = hand.to_path_buf();
        let d = d.clone();
        let token = token.clone();
        std::thread::spawn(move || {
            let (rc, out, err) = run_verb(&cancel, &hand, &d.workdir, &args, &stdin, timeout);
            let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s.as_bytes());
            let _ = post(
                &CancellationToken::new(),
                &d.brain,
                "result",
                &json!({"token": token, "id": id, "rc": rc, "stdout": b64(&out), "stderr": b64(&err)}),
                Duration::from_secs(120),
            );
        });
    }
    Ok(DialEnd::Stopped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_made_with_the_device_key_verifies_in_the_hand_namespace() {
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let home = std::env::temp_dir().join(format!("hive-hand-dial-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let public = keygen(&home).unwrap();
        assert!(public.starts_with("ssh-ed25519 "), "{public}");
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(home.join(KEY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the private key is the hand's alone");
        assert_eq!(keygen(&home).unwrap(), public, "the key is made once");
        let message = connect_message("dev-1", "abc");
        let sig = sign(&home, &message).unwrap();
        std::fs::write(home.join("allowed"), format!("hand {public}\n")).unwrap();
        std::fs::write(home.join("sig"), &sig).unwrap();
        let verify = |msg: &str, ns: &str| {
            use std::io::Write;
            let mut child = std::process::Command::new("ssh-keygen")
                .args(["-Y", "verify", "-f"])
                .arg(home.join("allowed"))
                .args(["-I", "hand", "-n", ns, "-s"])
                .arg(home.join("sig"))
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(msg.as_bytes())
                .unwrap();
            child.wait().unwrap().success()
        };
        assert!(verify(&message, SIGN_NAMESPACE));
        assert!(!verify(&connect_message("dev-1", "abd"), SIGN_NAMESPACE));
        assert!(
            !verify(&message, "git"),
            "a signature for another namespace never connects"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn an_unenrolled_hand_says_how_to_join() {
        let err = load_device(Path::new("/nonexistent/hive-hand")).unwrap_err();
        assert!(err.to_string().contains("hive-hand join <code>"), "{err}");
    }
}
