//! The dial-out hand channel (ADR-070 step 2): a machine enrols once with a
//! join code and then dials the brain; nothing reaches into it.
//!
//! 1. **Join code.** The owner asks the Machines page for one
//!    ([`issue_join_code`]): six words, valid [`JOIN_CODE_MINUTES`] minutes,
//!    good for one join. Only its SHA-256 is stored.
//! 2. **Join.** `hive-hand join <code>` makes an ed25519 device key on the
//!    machine and sends the public half with the code and the machine's facts
//!    ([`join`]). The brain registers the device (`hand_devices`) and the
//!    executor it works as, an `executors` row of kind [`DIAL_KIND`].
//! 3. **Connect.** The hand asks for a nonce ([`nonce`]), signs it with its
//!    device key (`ssh-keygen -Y sign`, namespace [`SIGN_NAMESPACE`]) and
//!    trades the signature for a session token ([`connect`]). A tampered
//!    signature, a used or lapsed nonce and a revoked device are refused.
//! 4. **Lease.** Every hands verb the brain would send over ssh is queued in
//!    the device's mailbox (`hand_calls`, [`dial_run`]); the hand's long poll
//!    takes it ([`take_call`]), runs it with its own `hive-hand` exactly as
//!    sshd's forced command would, and posts the outcome back
//!    ([`finish_call`]). The lease and report semantics are the `hive-hands`
//!    protocol's, so the executor code is the ssh path's.
//! 5. **Revoke.** [`revoke`] drops the device key and its sessions and
//!    removes its executor; the hand's next call or connect is refused.
//!
//! The hub serves [`hand_api`] under `/api/v1/hands/`; the hub and the
//! daemon are two processes on one store, so the mailbox is a table.

use crate::util::{Querier, Row, event_at, is_nil, must_one, must_rows, plus_minutes, slugify};
use anyhow::{Result, anyhow};
use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// The executor kind of a hand that dials in.
pub const DIAL_KIND: &str = "dial";
/// How long a join code is good for.
pub const JOIN_CODE_MINUTES: i64 = 10;
/// How long a connect nonce is good for.
pub const NONCE_MINUTES: i64 = 2;
/// How long a session token lives before the hand signs in again.
pub const SESSION_HOURS: i64 = 12;
/// The `ssh-keygen -Y` namespace a hand signs its connect nonce in, so a
/// signature made for anything else (a commit, a file) never connects.
pub const SIGN_NAMESPACE: &str = "hive-hand";
/// A hand that has not polled for this long is not connected: a verb for it
/// is refused at once as unreachable instead of waiting in the mailbox.
pub const LIVE_SECONDS: i64 = 90;
/// The longest one long poll is held open.
pub const POLL_MAX_SECONDS: u64 = 25;
/// The largest request the hand channel takes: a call's output (the
/// `bundle` verb's commits, a worker's log) comes back base64 in JSON.
pub const HAND_BODY_LIMIT: usize = 512 * 1024 * 1024;

/// Join-code words: short, distinct, easy to read aloud; six of them are
/// over 47 bits, and a code lives ten minutes and works once.
const WORDS: &[&str] = &[
    "acorn", "adobe", "agent", "alarm", "album", "alder", "alley", "alpha", "amber", "anchor",
    "angle", "ankle", "apple", "apron", "arbor", "arena", "arrow", "aspen", "atlas", "attic",
    "badge", "bagel", "baker", "bamboo", "banjo", "barley", "basil", "basin", "beach", "beacon",
    "berry", "birch", "bison", "blade", "blaze", "bloom", "board", "bonus", "boost", "brass",
    "bread", "brick", "brook", "brush", "bugle", "cabin", "cable", "cacao", "camel", "canal",
    "candle", "canoe", "canyon", "cargo", "carrot", "cedar", "chalk", "charm", "cherry", "chess",
    "chime", "cider", "cinder", "circle", "citrus", "clay", "cliff", "clock", "cloud", "clover",
    "cobalt", "comet", "coral", "cosmos", "cotton", "crane", "crayon", "creek", "crown", "cubic",
    "dahlia", "daisy", "delta", "denim", "desert", "diary", "dingo", "dolphin", "dome", "dragon",
    "drift", "drum", "dune", "eagle", "easel", "ember", "emerald", "engine", "epoch", "falcon",
    "fable", "fern", "ferry", "fiber", "field", "fig", "flame", "flint", "flora", "flute",
    "forest", "fossil", "fox", "frost", "fudge", "galaxy", "garden", "garnet", "gecko", "geyser",
    "ginger", "glacier", "globe", "goose", "granite", "grape", "gravel", "grove", "guava", "gull",
    "harbor", "hazel", "heron", "hickory", "honey", "horizon", "husky", "igloo", "indigo", "iris",
    "island", "ivory", "jade", "jasmine", "jelly", "jungle", "juniper", "kayak", "kelp", "kettle",
    "kiwi", "koala", "ladder", "lagoon", "lantern", "larch", "lemon", "lilac", "linen", "lizard",
    "lotus", "lunar", "magnet", "mango", "maple", "marble", "meadow", "melon", "mesa", "meteor",
    "mint", "mirror", "mocha", "monsoon", "moss", "mural", "nectar", "nickel", "noodle", "nutmeg",
    "oasis", "ocean", "olive", "onyx", "opal", "orbit", "orchid", "otter", "oyster", "paddle",
    "panda", "papaya", "parrot", "pebble", "pepper", "petal", "piano", "pine", "planet", "plum",
    "polar", "poppy", "prairie", "prism", "pumpkin", "quartz", "quill", "radar", "raven", "reef",
    "ribbon", "ridge", "river", "robin", "rocket", "saddle", "saffron", "salmon", "sapphire",
    "satin", "shell", "sierra", "silver", "spruce", "summit", "sunset", "swan", "tango", "thistle",
    "thunder", "tiger", "timber", "topaz", "tulip", "tundra", "turtle", "umber", "valley",
    "velvet", "violet", "walnut", "willow", "zebra", "zenith", "zephyr", "zinc",
];

/// A refusal the hand API answers with its HTTP status.
#[derive(Debug, Clone)]
pub struct DialError {
    pub status: u16,
    pub what: String,
    pub fix: String,
}
impl std::fmt::Display for DialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.what)
    }
}
impl std::error::Error for DialError {}

fn refuse(status: u16, what: impl Into<String>, fix: impl Into<String>) -> anyhow::Error {
    DialError {
        status,
        what: what.into(),
        fix: fix.into(),
    }
    .into()
}

fn sha256_hex(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    for b in buf.iter_mut() {
        *b = rand::random();
    }
    hex::encode(buf)
}

/// A join code as typed: any case, words split by anything that is not a
/// letter (spaces, dashes, a pasted link's slashes) - joined by dashes.
pub fn normalize_code(code: &str) -> String {
    code.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_lowercase())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// A join code the owner hands a machine.
#[derive(Debug, Clone)]
pub struct JoinCode {
    pub code: String,
    pub expires_at: String,
}

/// Issues a six-word join code valid [`JOIN_CODE_MINUTES`] from `now`.
pub fn issue_join_code(s: &hive_store::Store, now: &str, actor: &str) -> Result<JoinCode> {
    let code = (0..6)
        .map(|_| WORDS[rand::random::<u32>() as usize % WORDS.len()])
        .collect::<Vec<_>>()
        .join("-");
    let expires_at = plus_minutes(now, JOIN_CODE_MINUTES as f64);
    let conn = s.conn()?;
    // Spent and lapsed codes are of no use to anyone: keep the table small.
    conn.execute(
        "DELETE FROM hand_join_codes WHERE expires_at < ? AND used_at = ''",
        [plus_minutes(now, -24.0 * 60.0)],
    )?;
    conn.execute(
        "INSERT INTO hand_join_codes (code_hash, created_at, expires_at, created_by) VALUES (?, ?, ?, ?)",
        rusqlite::params![sha256_hex(&normalize_code(&code)), now, expires_at, actor],
    )?;
    event_at(
        &crate::db::ConnQ(conn),
        now,
        actor,
        "hand.join_code",
        "",
        json!({"expires_at": expires_at}),
    );
    Ok(JoinCode { code, expires_at })
}

/// What a joining hand says about its machine.
#[derive(Debug, Clone, Default)]
pub struct JoinFacts {
    pub name: String,
    pub os: String,
    pub arch: String,
    pub labels: String,
    pub workers: i32,
    pub workdir: String,
}

/// A hand that joined: its device id and the executor it works as.
#[derive(Debug, Clone)]
pub struct Joined {
    pub device: String,
    pub executor: String,
}

/// Whether `key` is one OpenSSH ed25519 public key line.
pub fn valid_public_key(key: &str) -> bool {
    let mut parts = key.split_whitespace();
    let (Some(kind), Some(body)) = (parts.next(), parts.next()) else {
        return false;
    };
    kind == "ssh-ed25519"
        && !key.contains('\n')
        && base64::engine::general_purpose::STANDARD
            .decode(body)
            .is_ok_and(|raw| raw.len() == 51)
}

/// Enrols a hand: the code must be one this brain issued, unused and not
/// past its ten minutes (403 otherwise); the key must be an ed25519 public
/// key (400). Registers the device and its `dial` executor, drained until
/// its first connect.
pub fn join(
    s: &hive_store::Store,
    now: &str,
    code: &str,
    public_key: &str,
    facts: &JoinFacts,
) -> Result<Joined> {
    let public_key = public_key.trim();
    if !valid_public_key(public_key) {
        return Err(refuse(
            400,
            "public_key must be one ssh-ed25519 public key line",
            "Send the .pub half of the device key hive-hand made.",
        ));
    }
    let workdir = facts.workdir.trim().trim_end_matches('/').to_string();
    if !workdir.starts_with('/') || workdir.split('/').any(|p| p == "..") {
        return Err(refuse(
            400,
            "workdir must be an absolute path on the hand",
            "hive-hand sends the folder its jobs run under.",
        ));
    }
    let workers = if facts.workers < 1 {
        1
    } else {
        facts.workers.min(64)
    };
    let hash = sha256_hex(&normalize_code(code));
    let mut joined = None;
    crate::db::immediate(s, |q| {
        let row = must_one(
            q,
            "SELECT * FROM hand_join_codes WHERE code_hash = ?",
            &[&hash],
        );
        if is_nil(&row) {
            return Err(refuse(
                403,
                "That join code is not one this brain issued",
                "Check the six words, or ask the Machines page for a new code.",
            ));
        }
        if !row.str("used_at").is_empty() {
            return Err(refuse(
                403,
                "That join code was already used",
                "A code enrols one machine; ask the Machines page for a new one.",
            ));
        }
        if row.str("expires_at").as_str() <= now {
            return Err(refuse(
                403,
                "That join code has expired",
                "Codes last ten minutes; ask the Machines page for a new one.",
            ));
        }
        let device = format!("dev-{}", random_hex(6));
        let executor = free_executor_name(q, &facts.name);
        let labels = hive_store::needs::join(&hive_store::needs::stored(&facts.labels));
        let removed = !is_nil(&must_one(
            q,
            "SELECT name FROM executors WHERE name = ? AND status = 'removed'",
            &[&executor],
        ));
        if removed {
            q.conn().execute(
                "UPDATE executors SET kind = ?, address = '', workdir = ?, workers = ?, projects = '*', status = 'drained', created_at = ?, created_by = ?, last_probe_at = '', probe_note = '', next_probe_at = '', errors = 0, backoff_minutes = 0, os = ?, arch = ?, labels = ? WHERE name = ?",
                rusqlite::params![DIAL_KIND, workdir, workers, now, device, facts.os, facts.arch, labels, executor],
            )?;
        } else {
            q.conn().execute(
                "INSERT INTO executors (name, kind, address, workdir, workers, projects, status, created_at, created_by, os, arch, labels) VALUES (?, ?, '', ?, ?, '*', 'drained', ?, ?, ?, ?, ?)",
                rusqlite::params![executor, DIAL_KIND, workdir, workers, now, device, facts.os, facts.arch, labels],
            )?;
        }
        q.conn().execute(
            "INSERT INTO hand_devices (id, executor, public_key, created_at, created_by) VALUES (?, ?, ?, ?, ?)",
            rusqlite::params![device, executor, public_key, now, row.str("created_by")],
        )?;
        q.conn().execute(
            "UPDATE hand_join_codes SET used_at = ?, device = ? WHERE code_hash = ?",
            rusqlite::params![now, device, hash],
        )?;
        event_at(
            q,
            now,
            &device,
            "hand.join",
            &executor,
            json!({"device": device, "os": facts.os, "arch": facts.arch, "labels": labels, "workers": workers}),
        );
        joined = Some(Joined { device, executor });
        Ok(())
    })?;
    joined.ok_or_else(|| anyhow!("join recorded nothing"))
}

/// A free executor name for a hand called `wanted`: its slug, or `hand`,
/// with `-2`, `-3`... when a live executor already has it.
fn free_executor_name(q: &crate::db::ConnQ, wanted: &str) -> String {
    let mut base = slugify(wanted, 36);
    if base.is_empty() || base == crate::executor::LOCAL_EXECUTOR || base == "x" {
        base = "hand".to_string();
    }
    let mut name = base.clone();
    let mut n = 1;
    loop {
        let taken = must_one(
            q,
            "SELECT name FROM executors WHERE name = ? AND status != 'removed'",
            &[&name],
        );
        if is_nil(&taken) {
            return name;
        }
        n += 1;
        name = format!("{base}-{n}");
    }
}

fn live_device(q: &(impl crate::util::Querier + ?Sized), device: &str) -> Result<Row> {
    let row = must_one(q, "SELECT * FROM hand_devices WHERE id = ?", &[&device]);
    if is_nil(&row) {
        return Err(refuse(
            403,
            format!("No device {device} is enrolled here"),
            "Enrol the machine again with `hive-hand join <code>`.",
        ));
    }
    if !row.str("revoked_at").is_empty() || row.str("public_key").is_empty() {
        return Err(refuse(
            403,
            format!("Device {device} was revoked"),
            "Ask the owner for a new join code and run `hive-hand join <code>` again.",
        ));
    }
    Ok(row)
}

/// A fresh nonce for `device` to sign, good for [`NONCE_MINUTES`].
pub fn nonce(s: &hive_store::Store, now: &str, device: &str) -> Result<String> {
    let conn = crate::db::ConnQ(s.conn()?);
    live_device(&conn, device)?;
    let nonce = random_hex(24);
    conn.execute("DELETE FROM hand_nonces WHERE expires_at < ?", &[&now])?;
    conn.execute(
        "INSERT INTO hand_nonces (nonce, device, expires_at) VALUES (?, ?, ?)",
        &[&nonce, &device, &plus_minutes(now, NONCE_MINUTES as f64)],
    )?;
    Ok(nonce)
}

/// The bytes a hand signs to connect: its device id and the nonce.
pub fn connect_message(device: &str, nonce: &str) -> String {
    format!("hive-hand connect {device} {nonce}\n")
}

/// Whether `signature` (an armored `ssh-keygen -Y sign` signature) is
/// `public_key`'s over `message` in [`SIGN_NAMESPACE`].
pub fn signature_ok(
    scratch: &std::path::Path,
    public_key: &str,
    message: &str,
    signature: &str,
) -> bool {
    let dir = scratch.join(format!("dial-verify-{}", random_hex(8)));
    let ok = verify_in(&dir, public_key, message, signature);
    let _ = std::fs::remove_dir_all(&dir);
    ok
}

fn verify_in(dir: &std::path::Path, public_key: &str, message: &str, signature: &str) -> bool {
    if public_key.contains('\n') || std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let signers = dir.join("allowed_signers");
    let sig = dir.join("connect.sig");
    if std::fs::write(&signers, format!("hand {public_key}\n")).is_err()
        || std::fs::write(&sig, signature).is_err()
    {
        return false;
    }
    let argv: Vec<String> = [
        "ssh-keygen",
        "-Y",
        "verify",
        "-f",
        &signers.display().to_string(),
        "-I",
        "hand",
        "-n",
        SIGN_NAMESPACE,
        "-s",
        &sig.display().to_string(),
    ]
    .iter()
    .map(|a| a.to_string())
    .collect();
    crate::proc::run_group(
        &hive_bus::CancellationToken::new(),
        &argv,
        "",
        &crate::proc::environ(),
        message.as_bytes(),
        Duration::from_secs(30),
    )
    .is_ok_and(|o| !o.timeout && o.exit_code == 0)
}

/// Trades a signed nonce for a session token: the nonce must be one handed
/// to `device`, unused and current, and `signature` its device key's.
/// Activates the device's executor on its first connect.
pub fn connect(
    s: &hive_store::Store,
    now: &str,
    device: &str,
    nonce: &str,
    signature: &str,
) -> Result<String> {
    let conn = crate::db::ConnQ(s.conn()?);
    let dev = live_device(&conn, device)?;
    let taken = conn.execute(
        "DELETE FROM hand_nonces WHERE nonce = ? AND device = ? AND expires_at >= ?",
        &[&nonce, &device, &now],
    )?;
    if taken != 1 {
        return Err(refuse(
            403,
            "That nonce is unknown, used or lapsed",
            "Ask for a new nonce and sign it within two minutes.",
        ));
    }
    if !signature_ok(
        std::path::Path::new(&s.home),
        &dev.str("public_key"),
        &connect_message(device, nonce),
        signature,
    ) {
        event_at(
            &conn,
            now,
            device,
            "hand.connect_refused",
            &dev.str("executor"),
            json!({"why": "bad signature"}),
        );
        return Err(refuse(
            403,
            "The signature does not verify with this device's key",
            "Sign the nonce with the device key `hive-hand join` made.",
        ));
    }
    let token = random_hex(32);
    conn.execute("DELETE FROM hand_sessions WHERE expires_at < ?", &[&now])?;
    conn.execute(
        "INSERT INTO hand_sessions (token_hash, device, created_at, expires_at) VALUES (?, ?, ?, ?)",
        &[
            &sha256_hex(&token),
            &device,
            &now,
            &crate::util::plus_hours(now, SESSION_HOURS as f64),
        ],
    )?;
    conn.execute(
        "UPDATE hand_devices SET last_seen_at = ? WHERE id = ?",
        &[&now, &device],
    )?;
    // A hand that dials in is reachable: its executor joins the rotation
    // on the first connect and comes back from unreachable on a reconnect
    // (a drain by the owner stays drained).
    conn.execute(
        &format!(
            "UPDATE executors SET status = 'active', errors = 0, backoff_minutes = 0, next_probe_at = '' WHERE name = ? AND kind = '{DIAL_KIND}' AND (status = 'unreachable' OR {})",
            crate::executor::NEVER_ACTIVE_SQL
        ),
        &[&dev.str("executor")],
    )?;
    event_at(
        &conn,
        now,
        device,
        "hand.connect",
        &dev.str("executor"),
        json!({"device": device}),
    );
    Ok(token)
}

/// The device a session token belongs to; refreshes its last-seen time.
pub fn session_device(s: &hive_store::Store, now: &str, token: &str) -> Result<String> {
    let conn = crate::db::ConnQ(s.conn()?);
    let row = must_one(
        &conn,
        "SELECT device FROM hand_sessions WHERE token_hash = ? AND expires_at >= ?",
        &[&sha256_hex(token), &now],
    );
    if is_nil(&row) {
        return Err(refuse(
            401,
            "The session is unknown or lapsed",
            "Connect again: ask for a nonce and sign it.",
        ));
    }
    let device = row.str("device");
    live_device(&conn, &device)?;
    conn.execute(
        "UPDATE hand_devices SET last_seen_at = ? WHERE id = ?",
        &[&now, &device],
    )?;
    Ok(device)
}

/// Revokes a device (by its id or its executor's name): its key and
/// sessions go, its executor is removed and its queued calls fail. The
/// hand's next call or connect is refused.
pub fn revoke(s: &hive_store::Store, now: &str, which: &str, actor: &str) -> Result<Value> {
    let mut out = Row::new();
    crate::db::immediate(s, |q| {
        let dev = must_one(
            q,
            "SELECT * FROM hand_devices WHERE (id = ? OR executor = ?) AND revoked_at = '' ORDER BY created_at DESC LIMIT 1",
            &[&which, &which],
        );
        if is_nil(&dev) {
            return Err(refuse(
                404,
                format!("No enrolled device {which}"),
                "Pick a machine that dials in from the Machines page.",
            ));
        }
        let id = dev.str("id");
        let executor = dev.str("executor");
        q.conn().execute(
            "UPDATE hand_devices SET revoked_at = ?, revoked_by = ?, public_key = '' WHERE id = ?",
            rusqlite::params![now, actor, id],
        )?;
        q.conn()
            .execute("DELETE FROM hand_sessions WHERE device = ?", [&id])?;
        q.conn()
            .execute("DELETE FROM hand_nonces WHERE device = ?", [&id])?;
        q.conn().execute(
            "UPDATE hand_calls SET status = 'cancelled', done_at = ? WHERE device = ? AND status IN ('queued', 'taken')",
            rusqlite::params![now, id],
        )?;
        q.conn().execute(
            "UPDATE executors SET status = 'removed' WHERE name = ? AND kind = ?",
            rusqlite::params![executor, DIAL_KIND],
        )?;
        event_at(
            q,
            now,
            actor,
            "hand.revoke",
            &executor,
            json!({"device": id}),
        );
        out = must_one(
            q,
            "SELECT id, executor, created_at, created_by, last_seen_at, revoked_at, revoked_by FROM hand_devices WHERE id = ?",
            &[&id],
        );
        Ok(())
    })?;
    Ok(serde_json::to_value(out)?)
}

/// Every enrolled device, newest first, for the Machines page.
pub fn devices(s: &hive_store::Store) -> Result<Vec<Value>> {
    let conn = s.conn()?;
    let rows = must_rows(
        &crate::db::ConnQ(conn),
        "SELECT id, executor, created_at, created_by, last_seen_at, revoked_at, revoked_by FROM hand_devices ORDER BY created_at DESC, id",
        &[],
    );
    Ok(rows
        .into_iter()
        .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
        .collect())
}

/// One hands verb waiting for (or taken by) a hand.
#[derive(Debug, Clone)]
pub struct Call {
    pub id: i64,
    pub args: Vec<String>,
    pub stdin: Vec<u8>,
    pub timeout_secs: i64,
}

/// Takes the oldest queued call for `device`, waiting up to `wait` for one.
pub fn take_call(
    s: &hive_store::Store,
    device: &str,
    wait: Duration,
    cancel: &hive_bus::CancellationToken,
) -> Result<Option<Call>> {
    let deadline = Instant::now() + wait;
    loop {
        let mut call = None;
        crate::db::immediate(s, |q| {
            let row = q.conn().query_row(
                "SELECT id, args, stdin, timeout_secs FROM hand_calls WHERE device = ? AND status = 'queued' ORDER BY id LIMIT 1",
                [device],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Vec<u8>>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                },
            );
            match row {
                Ok((id, args, stdin, timeout_secs)) => {
                    q.conn().execute(
                        "UPDATE hand_calls SET status = 'taken', taken_at = ? WHERE id = ?",
                        rusqlite::params![hive_store::now(), id],
                    )?;
                    call = Some(Call {
                        id,
                        args: serde_json::from_str(&args).unwrap_or_default(),
                        stdin,
                        timeout_secs,
                    });
                    Ok(())
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(()),
                Err(e) => Err(e.into()),
            }
        })?;
        if call.is_some() || Instant::now() >= deadline || cancel.is_cancelled() {
            return Ok(call);
        }
        // Revoked while it waits: the poll ends now, so the hand learns it
        // at its next call instead of a long poll later.
        live_device(&crate::db::ConnQ(s.conn()?), device)?;
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Records the outcome of call `id`, which `device` took.
pub fn finish_call(
    s: &hive_store::Store,
    device: &str,
    id: i64,
    rc: i32,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<()> {
    let conn = s.conn()?;
    let n = conn.execute(
        "UPDATE hand_calls SET status = 'done', done_at = ?, rc = ?, stdout = ?, stderr = ? WHERE id = ? AND device = ? AND status = 'taken'",
        rusqlite::params![hive_store::now(), rc, stdout, stderr, id, device],
    )?;
    if n != 1 {
        return Err(refuse(
            409,
            format!("Call {id} is not one this device has open"),
            "Post a result only for a call the poll handed you, once.",
        ));
    }
    Ok(())
}

/// The stores the dial transport reaches by Hive home: a row loaded
/// through `executor_row` carries its store's home ([`DIAL_HOME_FIELD`]),
/// and [`dial_run`] queues its call in that store.
static STORES: LazyLock<Mutex<BTreeMap<String, hive_store::Store>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

/// The row field naming the Hive home of a `dial` row's store.
pub const DIAL_HOME_FIELD: &str = "dial_home";

/// Marks a `dial` row with its store so [`dial_run`] can reach the mailbox.
pub(crate) fn mark_row(s: &hive_store::Store, r: &mut Row) {
    if r.str("kind") != DIAL_KIND {
        return;
    }
    STORES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(s.home.clone())
        .or_insert_with(|| s.clone());
    r.insert(DIAL_HOME_FIELD, json!(s.home));
}

fn store_for(r: &Row) -> Result<hive_store::Store> {
    let home = r.str(DIAL_HOME_FIELD);
    if let Some(s) = STORES.lock().unwrap_or_else(|e| e.into_inner()).get(&home) {
        return Ok(s.clone());
    }
    let home = if home.is_empty() {
        hive_store::resolve_home("")?
    } else {
        home
    };
    hive_store::Store::open(hive_store::Options {
        home,
        skip_records: true,
        ..Default::default()
    })
}

/// One hands verb on a `dial` row: queued in its device's mailbox and
/// answered when the hand posts the outcome. A hand that is not dialled in
/// (no poll for [`LIVE_SECONDS`]) is unreachable at once; one that takes
/// the verb and goes silent is unreachable when it stops polling; a verb
/// over `timeout` is a deadline the caller sees as for ssh.
pub(crate) fn dial_run(
    cancel: &hive_bus::CancellationToken,
    r: &Row,
    args: &[String],
    stdin: &[u8],
    timeout: Duration,
) -> Result<(i32, String, String)> {
    let s = store_for(r)?;
    let executor = r.str("name");
    let conn = s.conn()?;
    let dev = must_one(
        &crate::db::ConnQ(s.conn()?),
        "SELECT * FROM hand_devices WHERE executor = ? AND revoked_at = '' ORDER BY created_at DESC LIMIT 1",
        &[&executor],
    );
    if is_nil(&dev) {
        return Err(crate::executor::ExecutorUnreachable {
            msg: format!("no enrolled device works as {executor} (revoked?)"),
        }
        .into());
    }
    let device = dev.str("id");
    let live = |conn: &rusqlite::Connection| -> bool {
        conn.query_row(
            "SELECT last_seen_at FROM hand_devices WHERE id = ?",
            [&device],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .and_then(|at| crate::util::parse_iso(&at).ok())
        .zip(crate::util::parse_iso(&s.now()).ok())
        .is_some_and(|(at, now)| (now - at).num_seconds() <= LIVE_SECONDS)
    };
    if !live(&conn) {
        return Err(crate::executor::ExecutorUnreachable {
            msg: format!(
                "hand {executor} has not dialled in for {LIVE_SECONDS}s (is `hive-hand dial` running there?)"
            ),
        }
        .into());
    }
    conn.execute(
        "INSERT INTO hand_calls (device, args, stdin, timeout_secs, created_at) VALUES (?, ?, ?, ?, ?)",
        rusqlite::params![
            device,
            serde_json::to_string(args)?,
            stdin,
            timeout.as_secs() as i64,
            hive_store::now()
        ],
    )?;
    let id = conn.last_insert_rowid();
    let deadline = Instant::now() + timeout;
    let mut checked = Instant::now();
    let give_up = |conn: &rusqlite::Connection| {
        let _ = conn.execute(
            "UPDATE hand_calls SET status = 'cancelled', done_at = ? WHERE id = ? AND status != 'done'",
            rusqlite::params![hive_store::now(), id],
        );
    };
    loop {
        let row = conn.query_row(
            "SELECT status, rc, stdout, stderr FROM hand_calls WHERE id = ?",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i32>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                ))
            },
        )?;
        match row.0.as_str() {
            "done" => {
                let _ = conn.execute("DELETE FROM hand_calls WHERE id = ?", [id]);
                return Ok((
                    row.1,
                    String::from_utf8_lossy(&row.2).into_owned(),
                    String::from_utf8_lossy(&row.3).into_owned(),
                ));
            }
            "cancelled" => {
                return Err(crate::executor::ExecutorUnreachable {
                    msg: format!("hand {executor} was revoked"),
                }
                .into());
            }
            _ => {}
        }
        if cancel.is_cancelled() {
            give_up(&conn);
            return Err(hive_bus::ContextError::Canceled.into());
        }
        if Instant::now() >= deadline {
            give_up(&conn);
            let verb = args.first().map(String::as_str).unwrap_or("");
            return Err(crate::executor::ExecutorUnreachable {
                msg: format!("dial {verb} timed out after {}s", timeout.as_secs()),
            }
            .into());
        }
        if checked.elapsed() >= Duration::from_secs(5) {
            checked = Instant::now();
            if !live(&conn) {
                give_up(&conn);
                return Err(crate::executor::ExecutorUnreachable {
                    msg: format!("hand {executor} stopped dialling in"),
                }
                .into());
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn body_str(body: &Value, key: &str) -> String {
    body.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn unb64(body: &Value, key: &str) -> Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(body_str(body, key))
        .map_err(|_| {
            refuse(
                400,
                format!("{key} must be base64"),
                "Encode bytes as standard base64.",
            )
        })
}

/// The hand side of the hub: `route` is the path under `/api/v1/hands/`
/// (`join`, `nonce`, `connect`, `poll`, `result`), `body` the JSON a hand
/// posted. Answers an HTTP status and a JSON body (`error`/`why`/`fix` on a
/// refusal, as every hub error).
pub fn hand_api(
    s: &hive_store::Store,
    route: &str,
    body: &[u8],
    cancel: &hive_bus::CancellationToken,
) -> (u16, Value) {
    let now = s.now();
    let answer = (|| -> Result<Value> {
        if body.len() > HAND_BODY_LIMIT {
            return Err(refuse(
                413,
                "The request is too large",
                "Keep a call's output under 512 MiB.",
            ));
        }
        let body: Value = serde_json::from_slice(body)
            .map_err(|_| refuse(400, "The body must be a JSON object", "Send JSON."))?;
        if !body.is_object() {
            return Err(refuse(400, "The body must be a JSON object", "Send JSON."));
        }
        match route {
            "join" => {
                let facts = JoinFacts {
                    name: body_str(&body, "name"),
                    os: body_str(&body, "os"),
                    arch: body_str(&body, "arch"),
                    labels: body_str(&body, "labels"),
                    workers: body.get("workers").and_then(Value::as_i64).unwrap_or(1) as i32,
                    workdir: body_str(&body, "workdir"),
                };
                let j = join(
                    s,
                    &now,
                    &body_str(&body, "code"),
                    &body_str(&body, "public_key"),
                    &facts,
                )?;
                Ok(json!({"device": j.device, "executor": j.executor}))
            }
            "nonce" => Ok(json!({"nonce": nonce(s, &now, &body_str(&body, "device"))?})),
            "connect" => {
                let token = connect(
                    s,
                    &now,
                    &body_str(&body, "device"),
                    &body_str(&body, "nonce"),
                    &body_str(&body, "signature"),
                )?;
                Ok(json!({"token": token, "session_hours": SESSION_HOURS}))
            }
            "poll" => {
                let device = session_device(s, &now, &body_str(&body, "token"))?;
                let wait = body
                    .get("wait_secs")
                    .and_then(Value::as_u64)
                    .unwrap_or(POLL_MAX_SECONDS)
                    .min(POLL_MAX_SECONDS);
                let call = take_call(s, &device, Duration::from_secs(wait), cancel)?;
                Ok(match call {
                    Some(c) => json!({"call": {
                        "id": c.id,
                        "args": c.args,
                        "stdin": b64(&c.stdin),
                        "timeout_secs": c.timeout_secs,
                    }}),
                    None => json!({"call": Value::Null}),
                })
            }
            "result" => {
                let device = session_device(s, &now, &body_str(&body, "token"))?;
                let id = body.get("id").and_then(Value::as_i64).unwrap_or(0);
                let rc = body.get("rc").and_then(Value::as_i64).unwrap_or(1) as i32;
                finish_call(
                    s,
                    &device,
                    id,
                    rc,
                    &unb64(&body, "stdout")?,
                    &unb64(&body, "stderr")?,
                )?;
                Ok(json!({"ok": true}))
            }
            _ => Err(refuse(
                404,
                format!("No hand route {route:?}"),
                "The hand routes are join, nonce, connect, poll and result.",
            )),
        }
    })();
    match answer {
        Ok(v) => (200, v),
        Err(e) => match e.downcast_ref::<DialError>() {
            Some(d) => (
                d.status,
                json!({"error": d.what, "why": d.what, "fix": d.fix}),
            ),
            None => (
                500,
                json!({"error": "The hand channel failed", "why": e.to_string(), "fix": "Try again; the hub log has the detail."}),
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{TEST_NOW, new_store, serial};

    fn facts() -> JoinFacts {
        JoinFacts {
            name: "Studio Mac".into(),
            os: "macos".into(),
            arch: "arm64".into(),
            labels: "gpu".into(),
            workers: 2,
            workdir: "/Users/hive/hands".into(),
        }
    }

    /// A device key pair made by ssh-keygen in `dir`: (private path, public line).
    fn device_key(dir: &std::path::Path) -> (std::path::PathBuf, String) {
        let key = dir.join("device_key");
        let made = std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&key)
            .output()
            .unwrap();
        assert!(made.status.success());
        let public = std::fs::read_to_string(dir.join("device_key.pub")).unwrap();
        (key, public.trim().to_string())
    }

    fn sign(key: &std::path::Path, message: &str) -> String {
        use std::io::Write;
        let mut child = std::process::Command::new("ssh-keygen")
            .args(["-q", "-Y", "sign", "-f"])
            .arg(key)
            .args(["-n", SIGN_NAMESPACE])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(message.as_bytes())
            .unwrap();
        String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap()
    }

    fn status_of(e: &anyhow::Error) -> u16 {
        e.downcast_ref::<DialError>().map(|d| d.status).unwrap_or(0)
    }

    const KEY: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFnK0ljcL0vNfwh0SJNWr3tkC2ANeKkLb9FRGmBwJrLO hand@box";

    #[test]
    fn a_join_code_works_once_within_ten_minutes_and_a_wrong_one_is_refused() {
        let _serial = serial();
        let s = new_store();
        let code = issue_join_code(&s, TEST_NOW, "human").unwrap();
        assert_eq!(code.code.split('-').count(), 6, "{}", code.code);
        assert_eq!(code.expires_at, "2026-09-06T10:10:00Z");
        let stored: String = s
            .conn()
            .unwrap()
            .query_row("SELECT code_hash FROM hand_join_codes", [], |r| r.get(0))
            .unwrap();
        assert!(!stored.contains(&code.code), "only the code's hash is kept");

        let wrong = join(
            &s,
            TEST_NOW,
            "acorn-acorn-acorn-acorn-acorn-acorn",
            KEY,
            &facts(),
        )
        .unwrap_err();
        assert_eq!(status_of(&wrong), 403, "{wrong}");
        assert!(wrong.to_string().contains("not one this brain issued"));

        // Ten minutes on the dot is too late.
        let late = join(&s, &code.expires_at, &code.code, KEY, &facts()).unwrap_err();
        assert_eq!(status_of(&late), 403);
        assert!(late.to_string().contains("expired"), "{late}");

        let bad_key = join(&s, TEST_NOW, &code.code, "ssh-rsa AAAA", &facts()).unwrap_err();
        assert_eq!(
            status_of(&bad_key),
            400,
            "a refused key does not spend the code"
        );

        // Typed with spaces and capitals, nine minutes in: still the code.
        let typed = code.code.replace('-', " ").to_uppercase();
        let joined = join(&s, "2026-09-06T10:09:00Z", &typed, KEY, &facts()).unwrap();
        assert_eq!(joined.executor, "studio-mac");
        let ex = crate::executor::executor_row(&s, "studio-mac");
        assert_eq!(
            (
                ex.str("kind"),
                ex.str("status"),
                ex.int("workers"),
                ex.str("os"),
                ex.str("labels")
            ),
            (
                "dial".into(),
                "drained".into(),
                2,
                "macos".into(),
                "gpu".into()
            ),
            "a dial executor waits drained for its first connect: {ex:?}"
        );
        let again = join(&s, "2026-09-06T10:09:30Z", &code.code, KEY, &facts()).unwrap_err();
        assert_eq!(status_of(&again), 403);
        assert!(again.to_string().contains("already used"), "{again}");

        // A second machine of the same name gets the next free name.
        let second = issue_join_code(&s, TEST_NOW, "human").unwrap();
        let other = join(&s, TEST_NOW, &second.code, KEY, &facts()).unwrap();
        assert_eq!(other.executor, "studio-mac-2");
    }

    #[test]
    fn a_device_connects_with_its_key_and_is_refused_once_tampered_or_revoked() {
        let _serial = serial();
        if which::which("ssh-keygen").is_err() {
            return;
        }
        let s = new_store();
        let keys = tempfile::tempdir().unwrap();
        let (key, public) = device_key(keys.path());
        let code = issue_join_code(&s, TEST_NOW, "human").unwrap();
        let joined = join(&s, TEST_NOW, &code.code, &public, &facts()).unwrap();
        let dev = joined.device.as_str();

        // A tampered signature: signed over another nonce.
        let n = nonce(&s, TEST_NOW, dev).unwrap();
        let forged = sign(&key, &connect_message(dev, "not-the-nonce"));
        let err = connect(&s, TEST_NOW, dev, &n, &forged).unwrap_err();
        assert_eq!(status_of(&err), 403, "{err}");
        assert!(err.to_string().contains("does not verify"), "{err}");
        // That nonce is spent; a good signature over it no longer connects.
        let late = connect(
            &s,
            TEST_NOW,
            dev,
            &n,
            &sign(&key, &connect_message(dev, &n)),
        )
        .unwrap_err();
        assert_eq!(status_of(&late), 403, "a nonce answers once");
        // Another key's signature over a fresh nonce is refused too.
        let other = tempfile::tempdir().unwrap();
        let (other_key, _) = device_key(other.path());
        let n = nonce(&s, TEST_NOW, dev).unwrap();
        let err = connect(
            &s,
            TEST_NOW,
            dev,
            &n,
            &sign(&other_key, &connect_message(dev, &n)),
        )
        .unwrap_err();
        assert_eq!(status_of(&err), 403);

        // The device's own key over the nonce connects and activates it.
        let n = nonce(&s, TEST_NOW, dev).unwrap();
        let token = connect(
            &s,
            TEST_NOW,
            dev,
            &n,
            &sign(&key, &connect_message(dev, &n)),
        )
        .unwrap();
        assert_eq!(session_device(&s, TEST_NOW, &token).unwrap(), dev);
        assert_eq!(
            crate::executor::executor_row(&s, &joined.executor).str("status"),
            "active"
        );
        let listed = devices(&s).unwrap();
        assert_eq!(listed[0]["last_seen_at"], TEST_NOW);
        assert!(
            listed[0].get("public_key").is_none(),
            "the console never sees key material"
        );

        // Revoked: the session and any new connect are refused, the executor is gone.
        let gone = revoke(&s, TEST_NOW, &joined.executor, "human").unwrap();
        assert_eq!(gone["id"], dev);
        let err = session_device(&s, TEST_NOW, &token).unwrap_err();
        assert_eq!(status_of(&err), 401, "{err}");
        let err = nonce(&s, TEST_NOW, dev).unwrap_err();
        assert_eq!(status_of(&err), 403);
        assert!(err.to_string().contains("revoked"), "{err}");
        assert!(crate::util::is_nil(&crate::executor::executor_row(
            &s,
            &joined.executor
        )));
        let err = revoke(&s, TEST_NOW, dev, "human").unwrap_err();
        assert_eq!(status_of(&err), 404);
    }

    #[test]
    fn a_verb_for_a_dial_hand_goes_through_its_mailbox() {
        let _serial = serial();
        let _now = hive_testutil::setenv("HIVE_NOW", "");
        let s = new_store();
        let code = issue_join_code(&s, TEST_NOW, "human").unwrap();
        let joined = join(&s, TEST_NOW, &code.code, KEY, &facts()).unwrap();
        let row = crate::executor::executor_row(&s, &joined.executor);
        assert_eq!(row.str(DIAL_HOME_FIELD), s.home);
        let cancel = hive_bus::CancellationToken::new();

        // Never dialled in: unreachable at once, nothing queued.
        let err = dial_run(
            &cancel,
            &row,
            &["probe".into()],
            &[],
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(
            err.downcast_ref::<crate::executor::ExecutorUnreachable>()
                .is_some(),
            "{err}"
        );

        s.conn()
            .unwrap()
            .execute(
                "UPDATE hand_devices SET last_seen_at = ? WHERE id = ?",
                [s.now(), joined.device.clone()],
            )
            .unwrap();
        let hand = {
            let s = (*s).clone();
            let device = joined.device.clone();
            std::thread::spawn(move || {
                let cancel = hive_bus::CancellationToken::new();
                let call = take_call(&s, &device, Duration::from_secs(10), &cancel)
                    .unwrap()
                    .expect("a call");
                assert_eq!(call.args, vec!["count".to_string(), "job-1".into()]);
                assert_eq!(call.stdin, b"payload");
                assert!(
                    finish_call(&s, "dev-other", call.id, 0, b"", b"").is_err(),
                    "only the taker answers"
                );
                finish_call(&s, &device, call.id, 0, b"3\n", b"").unwrap();
            })
        };
        let (rc, out, _) = dial_run(
            &cancel,
            &row,
            &["count".into(), "job-1".into()],
            b"payload",
            Duration::from_secs(20),
        )
        .unwrap();
        hand.join().unwrap();
        assert_eq!((rc, out.as_str()), (0, "3\n"));
        let left: i64 = s
            .conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM hand_calls", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0, "an answered call leaves the mailbox");

        // Nobody takes it: the deadline is the caller's, as over ssh.
        let err = dial_run(
            &cancel,
            &row,
            &["probe".into()],
            &[],
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(err.to_string().contains("timed out"), "{err}");
    }
}

#[cfg(test)]
mod end_to_end {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};

    /// The hub's `/api/v1/hands/*` routes over plain HTTP/1.1, enough for
    /// curl: one request per connection, answered by [`hand_api`].
    fn serve(s: hive_store::Store) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let s = s.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() {
                        return;
                    }
                    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let mut length = 0usize;
                    loop {
                        let mut h = String::new();
                        if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                            break;
                        }
                        if let Some((k, v)) = h.split_once(':')
                            && k.eq_ignore_ascii_case("content-length")
                        {
                            length = v.trim().parse().unwrap_or(0);
                        }
                    }
                    let mut body = vec![0u8; length];
                    let _ = reader.read_exact(&mut body);
                    let route = path.trim_start_matches("/api/v1/hands/");
                    let (status, answer) =
                        hand_api(&s, route, &body, &hive_bus::CancellationToken::new());
                    let text = answer.to_string();
                    let mut stream = stream;
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                        text.len()
                    );
                });
            }
        });
        format!("http://{addr}")
    }

    /// ADR-070 step 2 end to end: a real `hive-hand` joins a test brain
    /// with a join code, dials out, and runs one job over the dial channel
    /// (prepare, the worker's commit, the bundle back, the clean) with no
    /// ssh anywhere; revoked, its next call is refused and `dial` ends.
    #[test]
    fn a_hive_hand_joins_a_brain_and_completes_a_job_over_the_dial_channel() {
        let _serial = crate::test_helpers::serial();
        let _guard = hive_testutil::env_lock();
        if which::which("ssh-keygen").is_err() || which::which("curl").is_err() {
            return;
        }
        let bin = crate::test_helpers::build_hive();
        let _stage = crate::test_helpers_staged_hive::stage_local_hive(&bin);
        let s = crate::test_helpers::new_store();
        let _now = hive_testutil::setenv("HIVE_NOW", "");
        let (_proj, root) = crate::test_helpers::new_bare_repo(&s);
        crate::test_helpers::join(&s, "aurora");
        s.set_setting(crate::util::HUMAN_AGENT, "minutes_per_job", "1")
            .unwrap();
        let _ssh = crate::test_helpers_hands::install_run_ssh(|_, argv, _, _| {
            panic!("a dial hand is never reached over ssh: {argv:?}")
        });
        let brain = serve((*s).clone());
        let hand = crate::test_helpers_hands::hive_hand_binary();
        let hand_home = tempfile::tempdir().unwrap();
        let workdir = hand_home.path().join("work");
        let fake_path = crate::test_helpers::fake_hands_path(&[]);
        let hand_cmd = |args: &[&str]| {
            let mut cmd = std::process::Command::new(&hand);
            cmd.args(args)
                .env("HIVE_HAND_HOME", hand_home.path())
                .env("HOME", hand_home.path())
                .env("HIVE_HANDS_ALLOW_ANY_WORKER", "1")
                .env("PATH", fake_path.trim_start_matches("PATH="))
                .env_remove("HIVE_JOB")
                .env_remove("HIVE_AGENT")
                .env_remove("HIVE_HOME")
                .env_remove("HIVE_NOW");
            cmd
        };

        let code = issue_join_code(&s, &s.now(), "human").unwrap();
        let joined = hand_cmd(&["join", "--brain", &brain, "--name", "far-box", "--workdir"])
            .arg(&workdir)
            .arg(&code.code)
            .env("HIVE_HAND_JOIN_ONLY", "1")
            .output()
            .unwrap();
        assert!(
            joined.status.success(),
            "join: {}",
            String::from_utf8_lossy(&joined.stderr)
        );
        assert!(String::from_utf8_lossy(&joined.stdout).contains("as far-box"));
        let key_mode = {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(hand_home.path().join("device_key"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(key_mode, 0o600);
        let again = hand_cmd(&["join", "--brain", &brain, &code.code])
            .env("HIVE_HAND_JOIN_ONLY", "1")
            .output()
            .unwrap();
        assert!(!again.status.success(), "a join code is good once");
        assert!(String::from_utf8_lossy(&again.stderr).contains("already used"));

        struct Kill(std::process::Child);
        impl Drop for Kill {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut dialer = Kill(
            hand_cmd(&["dial"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(30);
        while crate::executor::executor_row(&s, "far-box").str("status") != "active" {
            assert!(Instant::now() < deadline, "the hand never connected");
            std::thread::sleep(Duration::from_millis(100));
        }

        let job = crate::jobs::add_job(
            &s,
            "proj".into(),
            "task".into(),
            "dial".into(),
            "commit a note over the dial channel".into(),
            1,
            0,
            "test".into(),
            crate::util::HUMAN_AGENT.into(),
            false,
        )
        .unwrap()
        .0;
        let (run, reason) = crate::run::work_once(
            &s,
            "aurora",
            "",
            "",
            &s.now(),
            crate::run::RunOpts {
                executor: "far-box".into(),
                worker_cmd: "sh -c 'echo hi > dialled.txt && git add dialled.txt && git commit -q -m dialled && echo SUMMARY: done over the dial channel'".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(reason.is_empty(), "{reason}");
        let log = std::fs::read_to_string(run.str("log")).unwrap_or_default();
        assert_eq!(run.str("status"), "done", "run {run:?}\n{log}");
        assert_eq!(run.str("executor"), "far-box");
        let job = crate::jobs::job_row(&s.conn().unwrap(), job.str("id"));
        let branch = job.str("branch");
        assert!(!branch.is_empty(), "no branch:\n{log}");
        let note = crate::util::git(
            &root,
            &["show", &format!("refs/heads/{branch}:dialled.txt")],
        );
        assert_eq!(
            note.stdout.trim(),
            "hi",
            "the hand's commit came back:\n{log}"
        );
        let entries = std::fs::read_dir(&workdir)
            .map(|rd| rd.collect::<Vec<_>>())
            .unwrap_or_default();
        assert!(entries.is_empty(), "hand job dir not cleaned: {entries:?}");

        revoke(&s, &s.now(), "far-box", "human").unwrap();
        let deadline = Instant::now() + Duration::from_secs(40);
        let status = loop {
            if let Some(status) = dialer.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "dial kept going after the revoke"
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        let mut err = String::new();
        let _ = dialer.0.stderr.take().unwrap().read_to_string(&mut err);
        assert_eq!(status.code(), Some(3), "{err}");
        assert!(err.contains("refused"), "{err}");
    }
}
