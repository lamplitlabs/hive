#![allow(dead_code)]

use anyhow::anyhow;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::{LazyLock, RwLock};
use std::time::Duration;

pub(crate) const ADMIN_USER: &str = "hiveadmin";
pub(crate) const HANDS_WORKDIR: &str = "/srv/hive";
pub(crate) const HANDS_DISK_GB: &str = "64";
pub(crate) const HOSTKEY_MARKER: &str = "hive-hostkey ";
pub(crate) const HOSTKEY_BLOCK_TOP: &str = "-----BEGIN SSH HOST KEY KEYS-----";
pub(crate) const HOSTKEY_BLOCK_END: &str = "-----END SSH HOST KEY KEYS-----";
pub(crate) const TAG_HANDS: &str = "hive";
pub(crate) const TAG_REQUEST: &str = "hive-request";
pub(crate) const TAG_PROJECT: &str = "hive-project";
pub(crate) static PROVISION_WAIT: LazyLock<RwLock<Duration>> =
    LazyLock::new(|| RwLock::new(Duration::from_secs(20)));
pub(crate) fn set_provision_wait(value: Duration) -> Duration {
    std::mem::replace(
        &mut *PROVISION_WAIT.write().expect("provision_wait poisoned"),
        value,
    )
}
pub(crate) type CliRunner = dyn Fn(&hive_bus::CancellationToken, Vec<String>, Duration) -> anyhow::Result<(i32, String, String)>
    + Send
    + Sync;
pub(crate) type PairHandsFn = dyn Fn(
        &hive_store::Store,
        &str,
        &str,
        &str,
        crate::pair::PairOpts,
    ) -> anyhow::Result<Option<Box<crate::pair::PairReport>>>
    + Send
    + Sync;
pub(crate) type ProviderLogFn = dyn Fn(&str, &[serde_json::Value]) + Send + Sync;

pub(crate) use crate::compute::provider_named;
pub use crate::compute::{
    LogSink, Machine, Provider, ProvisionRequest, provider_names, register_provider,
};

pub(crate) static PAIR_HANDS: LazyLock<RwLock<Box<PairHandsFn>>> = LazyLock::new(|| {
    RwLock::new(Box::new(|s, admin, name, now, o| {
        crate::pair::pair_executor(s, admin.to_string(), name.to_string(), now.to_string(), o)
    }))
});

pub(crate) fn set_pair_hands(f: Box<PairHandsFn>) -> Box<PairHandsFn> {
    std::mem::replace(&mut *PAIR_HANDS.write().expect("pair_hands poisoned"), f)
}

pub(crate) fn cli_command(program: String, missing: String, env: &[String]) -> Box<CliRunner> {
    let env = env.to_vec();
    Box::new(move |cancel, args, timeout| {
        if cancel.is_cancelled() {
            return Err(anyhow!(
                "{program} {} timed out after {:?}",
                args.iter().take(2).cloned().collect::<Vec<_>>().join(" "),
                timeout
            ));
        }
        let environ = crate::proc::environ();
        let mut cmd = Command::new(hive_cli::procs::resolve_program(&program, &environ));
        cmd.args(&args);
        cmd.env_clear();
        for kv in environ {
            if let Some((k, v)) = kv.split_once('=') {
                cmd.env(k, v);
            }
        }
        for kv in &env {
            if let Some((k, v)) = kv.split_once('=') {
                cmd.env(k, v);
            }
        }
        let out = cmd.output();
        match out {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                Ok((o.status.code().unwrap_or(-1), stdout, stderr))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(anyhow!(missing.clone())),
            Err(e) => Err(anyhow!(e)),
        }
    })
}

pub(crate) fn cli_err(
    what: String,
    rc: i32,
    stdout: String,
    stderr: String,
    err: anyhow::Error,
) -> anyhow::Result<()> {
    if !err.to_string().is_empty() {
        return Err(anyhow!(
            "{}: {}",
            what,
            crate::compute::redact_secrets(&crate::run::err_message(&err))
        ));
    }
    let mut text = stderr.trim().to_string();
    if text.is_empty() {
        text = stdout.trim().to_string();
    }
    if text.is_empty() {
        text = format!("exit {rc}");
    }
    Err(anyhow!(
        "{}: {}",
        what,
        crate::compute::redact_secrets(&crate::run::squeeze(&text, 400))
    ))
}

pub(crate) fn not_found_answer(stdout: String, stderr: String) -> bool {
    let low = format!("{stderr}{stdout}").to_lowercase();
    [
        "notfound",
        "not found",
        "could not be found",
        "does not exist",
        "was not found",
        "no such",
        "instance does not exist",
    ]
    .iter()
    .any(|w| low.contains(w))
}

pub(crate) fn cloud_init_config(admin_key: String) -> String {
    let mut b = String::from(
        "#cloud-config\n# Hive compute machine (agents' hands) - Ubuntu 24.04 (ADR-020, D-D). Prepared for `hive pair`, which\n# adds the hive user, its forced-command key, the linux hive binary and the sshd drop-in over the admin session.\npackage_update: true\npackage_upgrade: true\npackages:\n  - git\n  - python3\n  - ca-certificates\n  - curl\n  - gnupg\n  - build-essential\nusers:\n  - default\n",
    );
    let key = admin_key.trim();
    if !key.is_empty() {
        b.push_str(&format!(
            "  - name: {ADMIN_USER}\n    shell: /bin/bash\n    sudo: ALL=(ALL) NOPASSWD:ALL\n    lock_passwd: true\n    ssh_authorized_keys:\n      - {key}\n"
        ));
    }
    b.push_str(&format!(
        "runcmd:\n  # The host key on the serial console: the channel the provider vouches for (aws, gcp read it back).\n  - sh -c 'echo \"{HOSTKEY_MARKER}$(cat /etc/ssh/ssh_host_ed25519_key.pub)\" > /dev/console'\n"
    ));
    b.push_str(&format!(
        "  # The work directory the forced command roots every path in; pairing sets the owner.\n  - mkdir -p {HANDS_WORKDIR}\nfinal_message: \"hive compute machine ready after $UPTIME seconds\"\n"
    ));
    b
}

pub(crate) fn write_cloud_init(admin_key: String) -> anyhow::Result<String> {
    let mut path = std::env::temp_dir();
    path.push(format!("hive-cloud-init-{}.yaml", uuid::Uuid::new_v4()));
    std::fs::write(&path, cloud_init_config(admin_key))?;
    Ok(path.to_string_lossy().to_string())
}

pub(crate) fn host_key_lines(text: String) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let add = |line: &str, out: &mut Vec<String>, seen: &mut std::collections::BTreeSet<String>| {
        let f = line.split_whitespace().collect::<Vec<_>>();
        if f.len() >= 2 && f[0] == "ssh-ed25519" {
            let key = format!("{} {}", f[0], f[1]);
            if seen.insert(key.clone()) {
                out.push(key);
            }
        }
    };
    let mut in_block = false;
    for raw in text.replace('\r', "").lines() {
        let line = raw.trim();
        if let Some(i) = line.find(HOSTKEY_MARKER) {
            add(&line[i + HOSTKEY_MARKER.len()..], &mut out, &mut seen);
        } else if line == HOSTKEY_BLOCK_TOP {
            in_block = true;
        } else if line == HOSTKEY_BLOCK_END {
            in_block = false;
        } else if in_block || line.starts_with("ssh-ed25519 ") {
            add(line, &mut out, &mut seen);
        }
    }
    out
}

pub(crate) fn provision_start(
    provider: String,
    req: crate::provider::ProvisionRequest,
) -> anyhow::Result<(hive_store::Store, String, Box<ProviderLogFn>)> {
    let Some(s) = req.store.clone() else {
        return Err(anyhow!("{provider}: a store is needed"));
    };
    if req.executor.is_empty() || !crate::pair::valid_name(req.executor.clone()) {
        return Err(anyhow!(
            "{}: executor name {} is not lowercase words with dashes",
            provider,
            crate::board::py_repr(req.executor)
        ));
    }
    // The caps hold a request while there is no login to give a new machine; a provider is never
    // asked for one without it, since a machine without a login is paid for and does nothing.
    if !crate::token::has_login_source(&s) {
        return Err(anyhow!(
            "{}: a machine would have no Copilot login (place one with `hive executor token --with-token`); no machine was bought",
            hive_tasks::NO_LOGIN_SOURCE
        ));
    }
    let hub_home = if req.hub_home.is_empty() {
        crate::util::home_dir()
    } else {
        req.hub_home
    };
    Ok((s, hub_home, provider_log(req.log)))
}

/// The provider's log line sink: each line, formatted and with secrets redacted, goes to the
/// request's `log` (the compute worker's daemon-log line and the request's note); a request
/// without one logs nowhere. Templates are the Go `%s`/`%d`/`%v` kind the provider code carries.
pub(crate) fn provider_log(sink: Option<crate::compute::LogSink>) -> Box<ProviderLogFn> {
    Box::new(move |template, args| {
        if let Some(sink) = &sink {
            sink(&crate::compute::redact_secrets(&format_log(template, args)));
        }
    })
}

/// Fills a `%s`/`%d`/`%v` template from its arguments in order; an argument beyond the verbs is
/// appended, a verb beyond the arguments reads `%?`.
pub(crate) fn format_log(template: &str, args: &[serde_json::Value]) -> String {
    let word = |v: &serde_json::Value| match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let mut out = String::with_capacity(template.len());
    let mut args = args.iter();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.peek() {
                Some('s' | 'd' | 'v') => {
                    chars.next();
                    match args.next() {
                        Some(v) => out.push_str(&word(v)),
                        None => out.push_str("%?"),
                    }
                    continue;
                }
                Some('%') => {
                    chars.next();
                    out.push('%');
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
    }
    for v in args {
        out.push(' ');
        out.push_str(&word(v));
    }
    out
}

pub(crate) fn machine_tags(req: crate::provider::ProvisionRequest) -> BTreeMap<String, String> {
    BTreeMap::from([
        (TAG_HANDS.to_string(), "hands".to_string()),
        (TAG_REQUEST.to_string(), req.id),
        (TAG_PROJECT.to_string(), req.project),
    ])
}

pub(crate) fn machine_name(request_id: String) -> String {
    crate::util::slugify(&format!("hive-{}", request_id.to_lowercase()), 40)
}

/// Remembers a machine the moment the provider made it, before the address, host key and pairing
/// steps that can still fail: whatever happens after this, the compute worker finds the row and
/// releases the machine instead of leaving it running (CR-002 ran for a day because the Rust port
/// dropped the release the Go daemon did on error).
pub(crate) fn machine_created(req: &crate::provider::ProvisionRequest, m: &Machine) {
    if let Some(s) = &req.store {
        crate::compute::save_machine(s, &req.id, m, &s.now());
    }
}

pub(crate) fn write_admin_alias(
    hub_home: String,
    alias: String,
    host: String,
) -> anyhow::Result<()> {
    let config = Path::new(&hub_home).join(".ssh").join("config");
    let text = std::fs::read_to_string(&config).unwrap_or_default();
    if let Some(b) = crate::pair::pair_alias_state(text.clone(), alias.clone()) {
        if b.hostname != host {
            return Err(anyhow!(
                "ssh alias {} already points at {}, not {}",
                alias,
                b.hostname,
                host
            ));
        }
        return Ok(());
    }
    std::fs::create_dir_all(config.parent().unwrap())?;
    let mut prefix = String::new();
    if !text.is_empty() {
        prefix.push('\n');
        if !text.ends_with('\n') {
            prefix.push('\n');
        }
    }
    let block = [
        format!("Host {alias}"),
        format!("    HostName {host}"),
        "    Port 22".to_string(),
        format!("    User {ADMIN_USER}"),
        "    IdentityFile ~/.ssh/hive_hands_ed25519".to_string(),
        "    IdentitiesOnly yes".to_string(),
        String::new(),
    ]
    .join("\n");
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config)?;
    f.write_all(format!("{prefix}{block}").as_bytes())?;
    Ok(())
}

pub(crate) fn wait_cloud_init(
    ctx: &hive_bus::CancellationToken,
    admin: String,
    log: &(dyn Fn(&str, &[serde_json::Value]) + Send + Sync),
) -> anyhow::Result<()> {
    let probe = "cloud-init status --wait >/dev/null 2>&1; command -v git >/dev/null 2>&1 && echo hive-ready";
    let mut try_no = 1;
    loop {
        let res =
            crate::executor::admin_ssh_run(&admin, probe, &[], Duration::from_secs(15 * 60), "");
        if let Ok((0, ref out, _)) = res
            && out.contains("hive-ready")
        {
            return Ok(());
        }
        if let Err(e) = res {
            log(
                "ssh attempt %d: %s",
                &[json!(try_no), json!(crate::run::err_message(&e))],
            );
        }
        if !sleep_ctx(
            ctx,
            *PROVISION_WAIT.read().expect("provision_wait poisoned"),
        ) {
            return Err(anyhow!("gave up waiting for {}: context canceled", admin));
        }
        try_no += 1;
    }
}

/// The steps after the provider has a machine with an address and a host key: the hub's ssh
/// files, cloud-init, the pairing, the harness login, then the probe that makes the executor
/// active. In that order because a machine Hive bought has no login until the daemon copies one
/// over: a pairing whose probe lacks only the login is paired and waiting
/// (`PairOpts::login_follows`), the login is placed, and the probe after it decides. Any
/// failure returns the reason the worker stores (the probe's note, the step that failed) and the
/// worker releases what was created.
pub(crate) fn provision_finish(
    ctx: &hive_bus::CancellationToken,
    req: crate::provider::ProvisionRequest,
    m: &mut crate::provider::Machine,
    hub_home: String,
    ip: String,
    hostkeys: Vec<String>,
    log: Box<ProviderLogFn>,
) -> anyhow::Result<()> {
    let Some(s) = req.store.clone() else {
        return Err(anyhow!("{}: a store is needed", m.provider));
    };
    if ip.is_empty() {
        return Err(anyhow!("{} has no public address", m.name));
    }
    if hostkeys.is_empty() {
        return Err(anyhow!("{} gave no ssh host key", m.name));
    }
    m.address = format!("{ADMIN_USER}@{ip}");
    m.detail.insert("public_ip".into(), ip.clone());
    crate::pair::pair_write_known_hosts(hub_home.clone(), ip.clone(), 22, hostkeys[..1].to_vec());
    let alias = format!("{}-admin", m.name);
    write_admin_alias(hub_home.clone(), alias.clone(), ip.clone())?;
    let admin = format!("{ADMIN_USER}@{alias}");
    log(
        "%s is up at %s; waiting for cloud-init",
        &[json!(m.name), json!(ip)],
    );
    wait_cloud_init(ctx, admin.clone(), &*log)?;
    log(
        "cloud-init done on %s; pairing %s",
        &[json!(m.name), json!(req.executor)],
    );
    let paired = (PAIR_HANDS.read().expect("pair_hands poisoned"))(
        &s,
        &admin,
        &req.executor,
        &s.now(),
        crate::pair::PairOpts {
            workdir: HANDS_WORKDIR.into(),
            workers: 1,
            projects: req.project.clone(),
            actor: crate::util::HUMAN_AGENT.into(),
            hub_home: hub_home.clone(),
            login_follows: true,
            ..Default::default()
        },
    );
    let rep = match paired {
        Ok(Some(rep)) => rep,
        Ok(None) => return Err(anyhow!("pairing {} did not complete", req.executor)),
        // A refusal, a failed apply or an undone pairing: the report is the error's text.
        Err(e) => {
            let lines = crate::run::err_message(&e)
                .lines()
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            for l in &lines {
                log("pair: %s", &[json!(l)]);
            }
            return Err(anyhow!(
                "pairing {} failed: {}",
                req.executor,
                crate::compute::redact_secrets(&crate::pair::pair_failure_reason(&lines))
            ));
        }
    };
    for l in &rep.lines {
        log("pair: %s", &[json!(l)]);
    }
    if !rep.ok {
        return Err(anyhow!(
            "pairing {} did not complete: {}",
            req.executor,
            crate::compute::redact_secrets(&crate::pair::pair_failure_reason(&rep.lines))
        ));
    }
    // The admin address the hub keeps, and the login step uses, is the alias the pairing just
    // spoke through: its `Host` block names the hands key, which is the only key the machine
    // knows (`write_admin_alias`). CR-060 (2026-11-05) paired through the alias and then placed
    // the login at the bare `hiveadmin@<ip>`, which ssh answered with "Permission denied
    // (publickey)"; the machine was released over a key it had.
    crate::executor::set_executor_admin(&s, &req.executor, &admin);
    // The login. The caps hold a request without a source before anything is bought, so one is
    // expected here; a source that went away meanwhile leaves a machine that cannot work.
    let (tok, origin) = crate::token::login_source(&s)?;
    if tok.is_empty() {
        return Err(anyhow!(
            "{}: {} is paired but has no Copilot login to work with; the source went away while the machine was being provisioned (place one with `hive executor token --with-token`)",
            hive_tasks::NO_LOGIN_SOURCE,
            req.executor
        ));
    }
    log(
        "placing the %s on %s",
        &[json!(origin), json!(req.executor)],
    );
    let mut rdr = crate::token::token_reader(tok);
    let res = (*crate::executor::PLACE_LOGIN
        .read()
        .expect("place_login poisoned"))(
        &s,
        &s.now(),
        &req.executor,
        &admin,
        &mut rdr,
        false,
        false,
        &req.actor,
    )?;
    for l in &res.lines {
        log("login: %s", &[json!(crate::compute::redact_secrets(l))]);
    }
    if !res.ok {
        return Err(anyhow!(
            "the {} was refused on {}: {}",
            origin,
            req.executor,
            crate::compute::redact_secrets(&login_failure_reason(&res.lines))
        ));
    }
    // The probe decides: with the login on the machine the hands must answer fit, and the
    // executor is active when they do. The login step probes itself; this is for a login placed
    // another way (a test's, an older hands build) that left the row as the pairing made it.
    let row = crate::executor::executor_row(&s, &req.executor);
    if row.str("status") != "active" {
        let now = s.now();
        let (ok, note) = crate::executor::probe_executor(&s, &req.executor, &req.actor, &now)?;
        log("probe: %s", &[json!(note)]);
        if !ok {
            return Err(anyhow!(
                "{} is paired and logged in but its probe fails: {}",
                req.executor,
                crate::compute::redact_secrets(&note)
            ));
        }
        if crate::executor::executor_row(&s, &req.executor).str("status") != "active" {
            crate::executor::set_executor_status(
                &s,
                &now,
                &req.executor,
                "active",
                &req.actor,
                Some(&note),
            )?;
        }
    }
    log(
        "%s is active: executor %s",
        &[json!(m.name), json!(req.executor)],
    );
    Ok(())
}

/// The line of a login report that says why it did not take: the failed check or probe, the
/// refused login, the unreachable machine; else its last line.
pub(crate) fn login_failure_reason(lines: &[String]) -> String {
    lines
        .iter()
        .find(|l| {
            l.contains("FAILED") || l.starts_with("login failed") || l.contains("unreachable:")
        })
        .or_else(|| lines.iter().rev().find(|l| !l.trim().is_empty()))
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn sleep_ctx(ctx: &hive_bus::CancellationToken, d: std::time::Duration) -> bool {
    let step = Duration::from_millis(10);
    let start = std::time::Instant::now();
    while start.elapsed() < d {
        if ctx.is_cancelled() {
            return false;
        }
        std::thread::sleep(step.min(d.saturating_sub(start.elapsed())));
    }
    !ctx.is_cancelled()
}

pub(crate) fn poll_until<F>(
    ctx: &hive_bus::CancellationToken,
    what: String,
    mut step: F,
) -> anyhow::Result<()>
where
    F: FnMut() -> anyhow::Result<bool>,
{
    let mut try_no = 0;
    loop {
        if try_no > 0
            && !sleep_ctx(
                ctx,
                *PROVISION_WAIT.read().expect("provision_wait poisoned"),
            )
        {
            return Err(anyhow!("gave up waiting for {}: context canceled", what));
        }
        if step()? {
            return Ok(());
        }
        try_no += 1;
    }
}

pub(crate) fn cli_list(out: String) -> Vec<String> {
    out.split_whitespace().map(ToOwned::to_owned).collect()
}

pub(crate) fn machine_from_detail(
    provider: String,
    name: String,
    detail_json: String,
    resource_group: String,
) -> Option<crate::provider::Machine> {
    #[derive(Deserialize)]
    struct Detail {
        #[serde(default)]
        tags: BTreeMap<String, String>,
        #[serde(default)]
        detail: BTreeMap<String, String>,
    }
    let mut m = crate::provider::Machine {
        provider,
        name,
        tags: BTreeMap::new(),
        detail: BTreeMap::new(),
        ..Default::default()
    };
    if let Ok(d) = serde_json::from_str::<Detail>(&detail_json) {
        m.tags = d.tags;
        m.detail = d.detail;
    }
    if m.detail.get("resource_group").is_none_or(String::is_empty) && !resource_group.is_empty() {
        m.detail.insert("resource_group".into(), resource_group);
    }
    Some(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{TEST_NOW, new_bare_repo, new_store};
    use crate::test_helpers_cloud::{TEST_DAEMON, install_ssh_hostname};
    use crate::test_helpers_daemon::{exec1, must_one, set_setting};
    use crate::test_helpers_environment::drained_by_login;
    use crate::test_helpers_provider::*;

    /// The compute harness with the machine sweep off: the daemon pass's landing-maintenance
    /// sweep (`tend_sweep`) otherwise walks the real `$HOME` runner folders and waits on each
    /// installed runner's `--version`, and `DaemonMaintenance::finish` holds the test for it -
    /// seconds a pass, minutes across these tests under `cargo test --workspace`. The compute
    /// sweep these tests exercise is separate (`compute::sweep_due`) and stays on.
    fn compute_harness(s: &hive_store::Store, settings: &[(&str, &str)]) -> ComputeHarness {
        let h = new_compute_harness(s, settings);
        set_setting(s, "daemon.sweep", "off");
        h
    }

    const TEST_HOST_KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBLOCKBLOCKBLOCKBLOCKBLOCKBLOCKBLOCKBLOCKBLOCKB root@hive";

    #[test]
    fn providers_registered() {
        let _serial = crate::test_helpers::serial();
        crate::azure::init();
        crate::aws::init();
        crate::gcp::init();
        crate::multipass::init();
        let names = provider_names();
        let mut want = hive_store::COMPUTE_PROVIDERS
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        want.sort();
        assert_eq!(names, want);
        let mut kinds = std::collections::BTreeSet::new();
        for n in names {
            let p = provider_named(&n).unwrap_or_else(|| panic!("provider {n} missing"));
            assert_eq!(p.name(), n);
            assert!(!p.kind().is_empty(), "provider {n}: empty kind");
            assert!(kinds.insert(p.kind()), "provider {n}: duplicate kind");
            assert!(
                crate::commands::PROVIDER_AUTH
                    .get(n.as_str())
                    .is_some_and(|v| !v.is_empty()),
                "provider {n}: no auth line"
            );
            let size = hive_store::compute_size_setting(&n);
            assert!(!size.is_empty(), "provider {n}: no size setting");
            assert!(
                hive_store::COMPUTE_RULES.contains_key(size.as_str()),
                "provider {n}: no size setting default"
            );
        }
    }

    #[test]
    fn host_key_lines() {
        let _serial = crate::test_helpers::serial();
        let console = "[   12.345] cloud-init[812]: -----BEGIN SSH HOST KEY KEYS-----\n\
ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBFAKE root@hive\n\
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBLOCKBLOCKBLOCKBLOCKBLOCKBLOCKBLOCKBLOCKBLOCKB root@hive\n\
-----END SSH HOST KEY KEYS-----\n\
some other noise\n\
hive-hostkey ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBLOCKBLOCKBLOCKBLOCKBLOCKBLOCKBLOCKBLOCKBLOCKB root@hive\n";
        let keys = super::host_key_lines(console.to_string());
        assert_eq!(keys.len(), 1, "keys: {keys:?}");
        assert!(
            keys[0].starts_with("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBLOCK"),
            "keys: {keys:?}"
        );
        let keys = super::host_key_lines(format!(
            "Enable succeeded: \n[stdout]\n{TEST_HOST_KEY}\n[stderr]\n"
        ));
        assert_eq!(
            keys,
            vec![
                TEST_HOST_KEY
                    .split_whitespace()
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(" ")
            ],
            "run-command shape"
        );
        assert!(super::host_key_lines("nothing yet\n".to_string()).is_empty());
    }

    #[test]
    fn cloud_init_config_admin_key() {
        let _serial = crate::test_helpers::serial();
        let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHUBHUBHUBHUBHUBHUBHUBHUBHUBHUBHUBHUBHUBHUBHUB hive-hub";
        let with = cloud_init_config(key.to_string());
        for want in [
            "#cloud-config",
            &format!("name: {ADMIN_USER}"),
            &format!("- {key}"),
            "ALL=(ALL) NOPASSWD:ALL",
            HOSTKEY_MARKER,
            "/dev/console",
            &format!("mkdir -p {HANDS_WORKDIR}"),
        ] {
            assert!(with.contains(want), "cloud-init lacks {want:?}:\n{with}");
        }
        for absent in [
            "deb.nodesource.com",
            "apt-get install -y nodejs",
            "npm install -g @github/copilot",
        ] {
            assert!(
                !with.contains(absent),
                "cloud-init has a legacy install line {absent:?}:\n{with}"
            );
        }
        let without = cloud_init_config(String::new());
        assert!(
            !without.contains(&format!("name: {ADMIN_USER}"))
                && !without.contains("ssh_authorized_keys"),
            "cloud-init without a key still adds a user:\n{without}"
        );
        assert!(
            without.contains(HOSTKEY_MARKER),
            "host key line missing without a key"
        );
        let path = write_cloud_init(key.to_string()).expect("write cloud-init");
        let data = std::fs::read_to_string(&path).expect("read cloud-init");
        let _ = std::fs::remove_file(path);
        assert_eq!(data, with);
    }

    #[test]
    fn daemon_once_compute_aws() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        let s = new_store();
        let h = compute_harness(
            &s,
            &[
                ("compute.provider", "aws"),
                ("compute.aws.region", "eu-west-1"),
                ("compute.aws.profile", "hive"),
            ],
        );
        let aws = new_fake_aws();
        let _fake = install_fake(crate::aws::set_run_aws, {
            let aws = aws.clone();
            Box::new(move |ctx, args, timeout| aws.run(ctx, args, timeout))
        });
        h.provision_and_release(&s, "aws", "203.0.113.9");

        let run = aws.first("ec2 run-instances");
        for want in [
            &format!("--image-id resolve:ssm:{}", crate::aws::AWS_IMAGE),
            "--instance-type t3.medium",
            "--subnet-id subnet-1",
            "--security-group-ids sg-hive",
            "--associate-public-ip-address",
            "--user-data file://",
            &format!("VolumeSize={}", HANDS_DISK_GB),
            "HttpTokens=required",
            "Tags=[{Key=Name,Value=hive-cr-001},{Key=hive,Value=hands},{Key=hive-request,Value=CR-001},{Key=hive-project,Value=proj}]",
            "--profile hive --region eu-west-1 --output json",
        ] {
            assert!(run.contains(want), "run-instances lacks {want:?}: {run}");
        }
        assert!(
            aws.has("ec2 create-security-group --group-name hive-hands")
                && aws.has("ec2 authorize-security-group-ingress --group-id sg-hive --protocol tcp --port 22 --cidr 0.0.0.0/0"),
            "security group: {:?}",
            aws.joined()
        );
        assert_eq!(
            aws.0.lock().unwrap().consoles,
            2,
            "console reads: {:?}",
            aws.joined()
        );
        assert!(
            aws.has("ec2 get-console-output --instance-id i-0abc --latest --query Output --profile hive --region eu-west-1 --output text"),
            "console reads: {:?}",
            aws.joined()
        );
        assert_eq!(aws.0.lock().unwrap().terminate, vec!["i-0abc"]);
        let (m, _) = crate::compute::load_machine(&s, "CR-001");
        let m = m.expect("machine");
        assert_eq!(
            m.detail.get("instance_id").map(String::as_str),
            Some("i-0abc")
        );
        assert_eq!(
            m.detail.get("region").map(String::as_str),
            Some("eu-west-1")
        );
        assert_eq!(
            m.detail.get("security_group").map(String::as_str),
            Some("sg-hive")
        );
        assert_eq!(m.detail.get("subnet").map(String::as_str), Some("subnet-1"));
    }

    #[test]
    fn daemon_once_compute_gcp() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        let s = new_store();
        let h = compute_harness(
            &s,
            &[
                ("compute.provider", "gcp"),
                ("compute.gcp.zone", "europe-west1-b"),
            ],
        );
        let gcp = new_fake_gcloud();
        let _fake = install_fake(crate::gcp::set_run_gcloud, {
            let gcp = gcp.clone();
            Box::new(move |ctx, args, timeout| gcp.run(ctx, args, timeout))
        });
        h.provision_and_release(&s, "gcp", "203.0.113.11");

        let create = gcp.first("compute instances create");
        for want in [
            "compute instances create hive-cr-001 --zone europe-west1-b --machine-type e2-medium --image-family ubuntu-2404-lts-amd64 --image-project ubuntu-os-cloud",
            &format!("--boot-disk-size {}GB", HANDS_DISK_GB),
            "--boot-disk-auto-delete",
            "--network default",
            "--labels hive=hands,hive-request=cr-001,hive-project=proj",
            "--tags hive-hands",
            "--metadata-from-file user-data=",
            "--project hive-proj --format json",
        ] {
            assert!(create.contains(want), "create lacks {want:?}: {create}");
        }
        assert!(
            gcp.has("compute firewall-rules create hive-hands-ssh --network default --direction INGRESS --allow tcp:22 --source-ranges 0.0.0.0/0 --target-tags hive-hands"),
            "firewall: {:?}",
            gcp.joined()
        );
        assert_eq!(
            gcp.0.lock().unwrap().serials,
            2,
            "serial reads: {:?}",
            gcp.joined()
        );
        assert!(
            gcp.has("compute instances get-serial-port-output hive-cr-001 --zone europe-west1-b --project hive-proj"),
            "serial reads: {:?}",
            gcp.joined()
        );
        assert_eq!(gcp.0.lock().unwrap().deleted, vec!["hive-cr-001"]);
        assert!(
            gcp.has("compute instances delete hive-cr-001 --zone europe-west1-b --quiet --project hive-proj"),
            "deleted: {:?} {:?}",
            gcp.0.lock().unwrap().deleted,
            gcp.joined()
        );
        let (m, _) = crate::compute::load_machine(&s, "CR-001");
        let m = m.expect("machine");
        assert_eq!(
            m.detail.get("project").map(String::as_str),
            Some("hive-proj")
        );
        assert_eq!(
            m.detail.get("zone").map(String::as_str),
            Some("europe-west1-b")
        );
        assert_eq!(m.detail.get("type").map(String::as_str), Some("e2-medium"));
    }

    #[test]
    fn daemon_once_compute_multipass() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        let s = new_store();
        let h = compute_harness(&s, &[("compute.provider", "multipass")]);
        let mp = new_fake_multipass();
        let _fake = install_fake(crate::multipass::set_run_multipass, {
            let mp = mp.clone();
            Box::new(move |ctx, args, timeout| mp.run(ctx, args, timeout))
        });
        h.provision_and_release(&s, "multipass", "192.168.64.7");

        let launch = mp.first("launch ");
        assert!(
            launch.starts_with("launch 24.04 --name hive-cr-001 --cloud-init ")
                && launch.ends_with("--timeout 1200 --cpus 2 --memory 4G --disk 32G"),
            "launch: {launch}"
        );
        assert_eq!(
            mp.0.lock().unwrap().execs,
            2,
            "exec reads: {:?}",
            mp.joined()
        );
        assert!(
            mp.has("exec hive-cr-001 -- cat /etc/ssh/ssh_host_ed25519_key.pub"),
            "exec reads: {:?}",
            mp.joined()
        );
        assert_eq!(mp.0.lock().unwrap().deleted, vec!["hive-cr-001"]);
        let (m, _) = crate::compute::load_machine(&s, "CR-001");
        let m = m.expect("machine");
        assert_eq!(m.detail.get("image").map(String::as_str), Some("24.04"));
        assert_eq!(m.detail.get("cpus").map(String::as_str), Some("2"));
        assert_eq!(m.detail.get("memory").map(String::as_str), Some("4G"));
        assert_eq!(m.detail.get("disk").map(String::as_str), Some("32G"));
    }

    #[test]
    fn compute_fails_when_the_provider_cli_is_missing() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        let s = new_store();
        let h = compute_harness(
            &s,
            &[
                ("compute.provider", "gcp"),
                ("compute.gcp.zone", "europe-west1-b"),
            ],
        );
        let _fake = install_fake(
            crate::gcp::set_run_gcloud,
            Box::new(|_, _, _| {
                Err(anyhow::anyhow!(
                    "the Google Cloud CLI (gcloud) is not installed on this machine"
                ))
            }),
        );
        let req = hive_tasks::request_compute(
            &s,
            "proj",
            "aurora",
            "",
            2,
            "the fixtures take an hour here",
        )
        .unwrap_or_else(|err| panic!("{err}"));
        hive_tasks::approve(&s, &req.id, "go ahead", crate::util::HUMAN_AGENT)
            .unwrap_or_else(|err| panic!("{err}"));
        let log = crate::test_helpers::once_daemon(
            &s,
            crate::daemon::DaemonOpts {
                worker_cmd: "sh -c 'echo SUMMARY: nothing'".into(),
                hub_home: h.hub_home.clone(),
                ..Default::default()
            },
        );
        let got = hive_tasks::get_compute(&s, &req.id).unwrap_or_default();
        if got.status != "failed" || !log.contains("gcloud) is not installed") {
            panic!("request: {got:?}\n{log}");
        }
        if !crate::executor::executor_row(&s, "hands-cr-001")
            .0
            .is_empty()
        {
            panic!("an executor was registered without a machine");
        }
    }

    #[test]
    fn daemon_once_compute_slow_provision() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        let s = new_store();
        let h = compute_harness(&s, &[("compute.provider", "multipass")]);
        let _old_wait = set_provision_wait(std::time::Duration::from_millis(1500));
        let mp = new_fake_multipass();
        let _fake = install_fake(crate::multipass::set_run_multipass, {
            let mp = mp.clone();
            Box::new(move |ctx, args, timeout| mp.run(ctx, args, timeout))
        });
        let req = hive_tasks::request_compute(
            &s,
            "proj",
            "aurora",
            "",
            2,
            "the fixtures take an hour here",
        )
        .expect("request compute");
        hive_tasks::approve(&s, &req.id, "go ahead", hive_store::HUMAN_AGENT).expect("approve");
        let log = crate::test_helpers::once_daemon(
            &s,
            crate::daemon::DaemonOpts {
                hub_home: h.hub_home.clone(),
                ..Default::default()
            },
        );
        let got = hive_tasks::get_compute(&s, &req.id).expect("get compute");
        assert_eq!(
            (got.status.as_str(), got.executor.as_str()),
            ("ready", "hands-cr-001"),
            "slow provision after once-daemon: {got:?}\n{log}"
        );
    }

    const AZURE_SETTINGS: &[(&str, &str)] = &[
        ("compute.provider", "azure"),
        ("compute.azure.resource_group", "hive-hands"),
        ("compute.azure.location", "westeurope"),
        ("compute.azure.vm_size", "Standard_B2ms"),
    ];

    fn approved_request(s: &hive_store::Store) -> hive_tasks::ComputeRequest {
        let req =
            hive_tasks::request_compute(s, "proj", "aurora", "", 2, "the fixtures take an hour")
                .unwrap_or_else(|err| panic!("{err}"));
        hive_tasks::approve(s, &req.id, "go ahead", crate::util::HUMAN_AGENT)
            .unwrap_or_else(|err| panic!("{err}"))
    }

    fn once(s: &hive_store::Store, h: &ComputeHarness) -> String {
        crate::test_helpers::once_daemon(
            s,
            crate::daemon::DaemonOpts {
                worker_cmd: "sh -c 'echo SUMMARY: nothing'".into(),
                hub_home: h.hub_home.clone(),
                ..Default::default()
            },
        )
    }

    fn findings_like(s: &hive_store::Store, pattern: &str) -> Vec<String> {
        let conn = s.conn().unwrap();
        crate::util::must_rows(
            &conn,
            "SELECT summary FROM posts WHERE kind = 'finding' AND summary LIKE ? ORDER BY id",
            &[&pattern],
        )
        .iter()
        .map(|r| r.str("summary"))
        .collect()
    }

    /// CR-002 on the live hub: the VM was bought, then the request failed for want of a
    /// linux/amd64 hive binary. The binary is now checked first, so nothing is bought.
    #[test]
    fn compute_buys_nothing_without_a_hands_binary() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        let s = new_store();
        let h = compute_harness(&s, AZURE_SETTINGS);
        let az = new_fake_az();
        let _fake = install_fake(crate::azure::set_run_az, {
            let az = az.clone();
            Box::new(move |ctx, args, timeout| az.run(ctx, args, timeout))
        });
        let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let harness_binary = crate::pair::set_hands_binary(Box::new({
            let asked = asked.clone();
            move |arch| {
                asked.lock().unwrap().push(arch.to_string());
                Err(anyhow::anyhow!(
                    "no linux/{arch} hive binary to ship: set HIVE_HANDS_BINARY"
                ))
            }
        }));
        let req = approved_request(&s);
        let log = once(&s, &h);
        let _ = crate::pair::set_hands_binary(harness_binary);
        let got = hive_tasks::get_compute(&s, &req.id).unwrap_or_default();
        assert_eq!(got.status, "failed", "request: {got:?}\n{log}");
        assert_eq!(
            *asked.lock().unwrap(),
            vec!["amd64"],
            "Standard_B2ms is amd64"
        );
        assert!(
            !az.has("vm create") && !az.has("group create") && az.deleted().is_empty(),
            "az was asked to make something: {:?}",
            az.calls()
        );
        let found = findings_like(&s, "compute CR-001 failed: no linux/amd64 hive binary%");
        assert!(
            found.len() == 1 && found[0].ends_with("; no machine was bought"),
            "findings: {found:?}\n{log}"
        );
        assert!(
            crate::executor::executor_row(&s, "hands-cr-001")
                .0
                .is_empty(),
            "an executor was registered without a machine"
        );
    }

    /// A machine approved for hours goes when they are up. Measured on one hub: a machine asked
    /// for four hours was released after 21.9, and 17 of its 22 runs began after the hours were
    /// up, since the queue leased to it again each time a run ended and a machine with a run in
    /// flight is not released.
    #[test]
    fn a_rental_past_its_hours_takes_no_new_work_and_goes_when_its_run_ends() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        let s = new_store();
        let h = compute_harness(&s, AZURE_SETTINGS);
        // A run may take two hours here, so one that began inside the machine's hours is still
        // in flight when they are up.
        set_setting(&s, "minutes_per_job", "120");
        let az = new_fake_az();
        let _fake = install_fake(crate::azure::set_run_az, {
            let az = az.clone();
            Box::new(move |ctx, args, timeout| az.run(ctx, args, timeout))
        });
        let req = approved_request(&s);
        let log = once(&s, &h);
        let got = hive_tasks::get_compute(&s, &req.id).unwrap_or_default();
        assert_eq!(
            (got.status.as_str(), got.executor.as_str(), got.hours),
            ("ready", "hands-cr-001", 2),
            "request: {got:?}\n{log}"
        );
        // Two slots on the machine, one of them busy with a run that began half an hour in.
        let after = |minutes: f64| crate::util::plus_minutes(TEST_NOW, minutes);
        exec1(
            &s,
            "UPDATE executors SET workers = 2 WHERE name = 'hands-cr-001'",
            &[],
        );
        exec1(
            &s,
            "INSERT INTO runs (id, job_id, agent, project, started_at, status, executor) VALUES ('run-1', 'job-1', 'aurora', 'proj', ?, 'running', 'hands-cr-001')",
            &[&after(30.0)],
        );
        let slots = |now: &str| crate::run::executor_slots(&s, now, &BTreeMap::new(), true);
        assert_eq!(
            slots(&after(60.0)),
            ["hands-cr-001"],
            "inside its hours the free slot is offered"
        );
        // Its two hours are up: no slot is offered, so nothing new is leased to it.
        assert_eq!(
            slots(&after(120.0)),
            Vec::<String>::new(),
            "a rental past its hours still offers a slot"
        );
        // It is not released under the run in flight; the log and the request's card say what
        // it waits for, once.
        let cancel = hive_bus::CancellationToken::new();
        let worker = crate::compute::new_compute_worker(
            &cancel,
            &s,
            crate::compute::ComputeOpts {
                actor: TEST_DAEMON.into(),
                hub_home: h.hub_home.clone(),
            },
        );
        let tick = |now: &str| {
            let mut lines = worker.tick(&cancel, now);
            worker.wait(Duration::from_secs(60));
            lines.extend(worker.drain());
            lines
        };
        const LAST_RUN: &str = "its 2 hour(s) are up: hands-cr-001 takes no new work and is released when its last run ends";
        let lines = tick(&after(120.0));
        let got = hive_tasks::get_compute(&s, &req.id).unwrap_or_default();
        assert_eq!(
            (got.status.as_str(), got.note.as_str()),
            ("ready", LAST_RUN),
            "{lines:?}"
        );
        assert_eq!(lines, [format!("compute {}: {LAST_RUN}", req.id)]);
        assert_eq!(tick(&after(120.25)), Vec::<String>::new());
        // No second machine is rented while this one finishes its last run, though work
        // waits and the owner pre-authorised one: it still counts as a machine against
        // compute.max_vms and as the one open request.
        set_setting(&s, "compute.auto_approve", "1");
        crate::test_helpers_daemon::queued_job(&s, "j-waits", "proj", "task", "owner");
        let rent = |now: &str| {
            crate::daemon::auto_compute(
                &s,
                "1 job(s) waited 120 min with every slot on this machine busy or held",
                crate::daemon::HANDS_WANTED_MINUTES,
                now,
                TEST_DAEMON,
            )
        };
        set_setting(&s, "compute.max_vms", "1");
        assert_eq!(
            rent(&after(120.5)),
            Err(crate::daemon::NoAutoCompute::Why(
                "compute.max_vms 1 reached (1 in use)".into()
            ))
        );
        set_setting(&s, "compute.max_vms", "2");
        assert_eq!(
            rent(&after(120.5)),
            Err(crate::daemon::NoAutoCompute::Open {
                id: req.id.clone(),
                status: "ready".into()
            })
        );
        // The run ends: the machine goes on the next tick.
        exec1(
            &s,
            "UPDATE runs SET status = 'done', finished_at = ? WHERE id = 'run-1'",
            &[&after(125.0)],
        );
        let lines = tick(&after(125.25));
        let got = hive_tasks::get_compute(&s, &req.id).unwrap_or_default();
        assert_eq!(
            (got.status.as_str(), got.note.as_str()),
            ("released", "its 2 hour(s) are up"),
            "{lines:?}"
        );
        assert_eq!(az.deleted(), vec!["hive-cr-001"], "{lines:?}");
        // Nothing waits after the release: the queue still waits, so the daemon asks for a
        // fresh machine on its usual terms.
        let next = rent(&after(125.5)).expect("a fresh machine is asked for");
        assert_eq!(
            (next.id.as_str(), next.status.as_str(), next.hours),
            ("CR-002", "approved", crate::daemon::AUTO_COMPUTE_HOURS),
            "{next:?}"
        );
        cancel.cancel();
    }

    /// TASK-138 item 1: a rented machine that holds a clone whose commits could not be fetched
    /// is not released when it sits idle for `COMPUTE_IDLE_MINUTES` - the clone is the run's
    /// only copy while its summary says how to fetch it. The card says why the machine stays;
    /// once the clone is fetched (or its approved hours are up) it goes as before. Before this,
    /// `release_due` did not look at kept clones and the work was lost after thirty minutes.
    #[test]
    fn an_idle_rental_holding_unfetched_work_is_not_released() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        let s = new_store();
        let h = compute_harness(&s, AZURE_SETTINGS);
        let az = new_fake_az();
        let _fake = install_fake(crate::azure::set_run_az, {
            let az = az.clone();
            Box::new(move |ctx, args, timeout| az.run(ctx, args, timeout))
        });
        let req = approved_request(&s);
        let log = once(&s, &h);
        let got = hive_tasks::get_compute(&s, &req.id).unwrap_or_default();
        assert_eq!(got.status, "ready", "request: {got:?}\n{log}");
        let after = |minutes: f64| crate::util::plus_minutes(TEST_NOW, minutes);
        // A run that committed ended ten minutes in; its commits could not be fetched.
        crate::test_helpers_daemon::queued_job(&s, "job-kept", "proj", "task", "owner");
        exec1(
            &s,
            "UPDATE jobs SET status = 'failed' WHERE id = 'job-kept'",
            &[],
        );
        exec1(
            &s,
            "INSERT INTO runs (id, job_id, agent, project, started_at, finished_at, status, commits, executor) VALUES ('run-kept', 'job-kept', 'aurora', 'proj', ?, ?, 'failed', 1, 'hands-cr-001')",
            &[&after(5.0), &after(10.0)],
        );
        crate::util::event_at(
            &s.conn().unwrap(),
            &after(10.0),
            "runner",
            crate::executor::FETCH_FAILED_EVENT,
            "hands-cr-001",
            serde_json::json!({"job": "job-kept", "run": "run-kept", "clone": "/srv/hive/job-kept/clone"}),
        );
        let cancel = hive_bus::CancellationToken::new();
        let worker = crate::compute::new_compute_worker(
            &cancel,
            &s,
            crate::compute::ComputeOpts {
                actor: TEST_DAEMON.into(),
                hub_home: h.hub_home.clone(),
            },
        );
        assert_eq!(
            worker.release_due(got.clone(), &after(60.0)),
            "",
            "an idle machine holding unfetched work was due for release"
        );
        let tick = |now: &str| {
            let mut lines = worker.tick(&cancel, now);
            worker.wait(Duration::from_secs(60));
            lines.extend(worker.drain());
            lines
        };
        let lines = tick(&after(60.0));
        let got = hive_tasks::get_compute(&s, &req.id).unwrap_or_default();
        assert_eq!(got.status, "ready", "{lines:?}");
        assert!(
            got.note.contains("holds the clone(s) of job-kept")
                && got.note.contains("not released while idle"),
            "the card does not say why the machine stays: {:?}",
            got.note
        );
        assert!(az.deleted().is_empty(), "{lines:?}");
        // Inside its hours an idle machine with kept work stays; when they are up it goes,
        // and the reason names what went with it.
        let why = worker.release_due(got.clone(), &after(121.0));
        assert!(
            why.starts_with("its 2 hour(s) are up") && why.contains("job-kept"),
            "{why}"
        );
        // Once the clone is fetched the idle rule applies again.
        crate::util::event_at(
            &s.conn().unwrap(),
            &after(70.0),
            "daemon",
            crate::executor::KEPT_GONE_EVENT,
            "hands-cr-001",
            serde_json::json!({"job": "job-kept", "run": "run-kept", "why": "fetched"}),
        );
        assert_eq!(
            worker.release_due(got, &after(71.0)),
            format!(
                "the executor sat idle for {} minutes",
                crate::compute::COMPUTE_IDLE_MINUTES
            )
        );
        cancel.cancel();
    }

    /// A failure after the machine exists releases it (VM and its siblings) before the request
    /// is failed, as the Go runner did; the machine row keeps the released time.
    #[test]
    fn compute_releases_what_a_failed_provisioning_created() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        let s = new_store();
        let h = compute_harness(&s, AZURE_SETTINGS);
        let az = new_fake_az();
        let _fake = install_fake(crate::azure::set_run_az, {
            let az = az.clone();
            Box::new(move |ctx, args, timeout| {
                if args.join(" ").starts_with("vm run-command invoke") {
                    return Ok((1, String::new(), "RunCommand is disabled here".into()));
                }
                az.run(ctx, args, timeout)
            })
        });
        let req = approved_request(&s);
        let log = once(&s, &h);
        let got = hive_tasks::get_compute(&s, &req.id).unwrap_or_default();
        assert_eq!(got.status, "failed", "request: {got:?}\n{log}");
        assert_eq!(az.deleted(), vec!["hive-cr-001"], "{log}");
        let tags = az.0.lock().unwrap().tags.get("hive-cr-001").cloned();
        assert_eq!(
            tags.as_ref().and_then(|t| t.get(TAG_REQUEST).cloned()),
            Some("CR-001".to_string()),
            "the VM carries its request tag: {tags:?}"
        );
        let (m, row) = crate::compute::load_machine(&s, &req.id);
        assert!(
            m.is_some_and(|m| m.name == "hive-cr-001") && !row.str("released_at").is_empty(),
            "machine row after the failure: {row:?}"
        );
        let found = findings_like(&s, "compute CR-001 failed:%");
        assert!(
            found.len() == 1
                && found[0].contains("RunCommand is disabled here")
                && found[0].ends_with("; what was created is released"),
            "findings: {found:?}\n{log}"
        );
        assert!(
            crate::executor::executor_row(&s, "hands-cr-001")
                .0
                .is_empty(),
            "an executor was registered for a failed machine"
        );
    }

    const LOGIN_MISSING_NOTE: &str = "harness login missing (run `hive executor login hands-cr-001 --admin <admin@host> --with-token`)";
    const HANDS_PROBE_OK: &str = "hands=ok user=hive workdir=/srv/hive git=2.43 harness=yes hive=/usr/local/bin/hive remotes=0 keys_ok=yes disk_free_mb=200000 jobs=- keys=- heartbeat=1\n";

    /// A pairing as `pair_apply_live` leaves one on a machine that has no login yet
    /// (`login_follows`): the row registered and drained with the probe's note, the `Host NAME`
    /// block written under `hub_home`, the report ok. Returns the pair hook it replaced.
    fn install_login_pending_pair(hub_home: &str) -> Box<PairHandsFn> {
        let hub_home = hub_home.to_string();
        set_pair_hands(Box::new(move |st, admin, name, now, o| {
            assert!(o.login_follows, "the compute path pairs with login_follows");
            crate::executor::add_executor(
                st,
                now,
                name,
                &format!("hive@{name}"),
                &o.workdir,
                o.workers,
                &o.projects,
                hive_store::HUMAN_AGENT,
            )?;
            st.conn()?.execute(
                "UPDATE executors SET probe_note = ?, last_probe_at = ? WHERE name = ?",
                rusqlite::params![LOGIN_MISSING_NOTE, now, name],
            )?;
            let config = std::path::Path::new(&hub_home).join(".ssh").join("config");
            let mut text = std::fs::read_to_string(&config).unwrap_or_default();
            text.push('\n');
            text.push_str(&crate::pair::pair_host_block(
                name.to_string(),
                "203.0.113.5".into(),
                22,
            ));
            std::fs::write(&config, text).expect("ssh config");
            Ok(Some(Box::new(crate::pair::PairReport {
                ok: true,
                lines: vec![
                    format!(
                        "registered executor {name} (hive@{name}, {}, 1 worker(s), projects {})",
                        o.workdir, o.projects
                    ),
                    format!("paired {name}: drained (the harness login comes next)"),
                    format!("  host      203.0.113.5:22 (admin {admin})"),
                    format!(
                        "  probe     {LOGIN_MISSING_NOTE}; the caller places the login and probes again"
                    ),
                    format!("  undo      hive remove executor {name}"),
                ],
                ..Default::default()
            })))
        }))
    }

    /// An ssh fake that answers the hands' probe with `reply` and everything else (cloud-init,
    /// the login) with `hive-ready`. Like the machine Hive buys, it knows the hands key
    /// alone, which only the `<name>-admin` alias block names: a bare `user@ip` address gets
    /// ssh's "Permission denied (publickey)" (exit 255), the answer CR-060 got.
    fn install_probe_ssh(reply: &'static str) -> crate::executor::SshRunner {
        crate::executor::set_run_ssh(move |_, argv: &[String], _, _| {
            if argv.last().is_some_and(|a| a == "b64 cHJvYmUA") {
                return Ok((0, reply.to_string(), String::new()));
            }
            if let Some(target) = argv.iter().find(|a| a.contains('@'))
                && target
                    .split('@')
                    .nth(1)
                    .is_some_and(|h| h.chars().all(|c| c.is_ascii_digit() || c == '.'))
            {
                return Ok((
                    255,
                    String::new(),
                    format!("{target}: Permission denied (publickey)."),
                ));
            }
            Ok((0, "hive-ready\n".into(), String::new()))
        })
    }

    fn ssh_files(h: &ComputeHarness) -> (String, String) {
        (
            std::fs::read_to_string(format!("{}/.ssh/config", h.hub_home)).unwrap_or_default(),
            std::fs::read_to_string(format!("{}/.ssh/known_hosts", h.hub_home)).unwrap_or_default(),
        )
    }

    /// CR-001..CR-059 on the live hub: every machine came up, was paired, and failed on the
    /// probe's `harness login missing` - the login was the step after the one that failed. The
    /// login now follows the pairing, the probe after it makes the hand active, and the machine
    /// is kept; every step reaches the daemon log under the request's id.
    #[test]
    fn compute_places_the_login_after_pairing_and_the_probe_makes_the_hand_active() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        let s = new_store();
        let h = compute_harness(&s, AZURE_SETTINGS);
        let az = new_fake_az();
        let _fake = install_fake(crate::azure::set_run_az, {
            let az = az.clone();
            Box::new(move |ctx, args, timeout| az.run(ctx, args, timeout))
        });
        let _pair = install_login_pending_pair(&h.hub_home);
        let _ssh = install_probe_ssh(HANDS_PROBE_OK);
        let req = approved_request(&s);
        let log = once(&s, &h);
        let got = hive_tasks::get_compute(&s, &req.id).unwrap_or_default();
        assert_eq!(
            (
                got.status.as_str(),
                got.executor.as_str(),
                got.note.as_str()
            ),
            ("ready", "hands-cr-001", ""),
            "request: {got:?}\n{log}"
        );
        let ex = crate::executor::executor_row(&s, "hands-cr-001");
        assert!(
            ex.str("status") == "active"
                && ex.str("probe_note").starts_with("ok:")
                && ex.str("admin") == "hiveadmin@hive-cr-001-admin",
            "executor after the login: {ex:?}\n{log}"
        );
        let placed = place_login_calls();
        assert!(
            placed.len() == 1 && placed[0].admin == "hiveadmin@hive-cr-001-admin",
            "the login goes through the alias the pairing used: {placed:?}"
        );
        assert!(az.deleted().is_empty(), "the machine was released: {log}");
        let (_, mrow) = crate::compute::load_machine(&s, &req.id);
        assert_eq!(mrow.str("released_at"), "", "machine row: {mrow:?}");
        for want in [
            "compute CR-001: hive-cr-001 is up at 203.0.113.5; waiting for cloud-init",
            "compute CR-001: cloud-init done on hive-cr-001; pairing hands-cr-001",
            "compute CR-001: pair: paired hands-cr-001: drained (the harness login comes next)",
            "compute CR-001: placing the executor token on hands-cr-001",
            "compute CR-001: login: placed login",
            "compute CR-001: probe: ok: ",
            "compute CR-001: hive-cr-001 is active: executor hands-cr-001",
            "compute CR-001 ready: executor hands-cr-001",
        ] {
            assert!(log.contains(want), "daemon log lacks {want:?}:\n{log}");
        }
        assert!(
            !log.contains("did not complete"),
            "the old failure is still reported:\n{log}"
        );
    }

    /// The probe after the login still decides: a hand that is unfit for another reason fails
    /// the request with that reason - on the request's note and the finding, not the report's
    /// `undo` line - and the attempt leaves nothing behind: the machine is released, the
    /// executor row and the ssh files written for the machine are gone.
    #[test]
    fn compute_fails_with_the_probe_note_and_leaves_no_executor_behind() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        let s = new_store();
        let h = compute_harness(&s, AZURE_SETTINGS);
        set_setting(&s, "daemon.min_free_mb", "10240");
        let az = new_fake_az();
        let _fake = install_fake(crate::azure::set_run_az, {
            let az = az.clone();
            Box::new(move |ctx, args, timeout| az.run(ctx, args, timeout))
        });
        let _pair = install_login_pending_pair(&h.hub_home);
        let _ssh = install_probe_ssh(
            "hands=ok user=hive workdir=/srv/hive git=2.43 harness=yes hive=/usr/local/bin/hive remotes=0 keys_ok=yes disk_free_mb=100 jobs=- keys=- heartbeat=1\n",
        );
        let req = approved_request(&s);
        let log = once(&s, &h);
        let got = hive_tasks::get_compute(&s, &req.id).unwrap_or_default();
        let want_reason = "hands-cr-001 is paired and logged in but its probe fails: under 10 GiB free on /srv/hive; what was created is released";
        assert_eq!(
            (got.status.as_str(), got.note.as_str()),
            ("failed", want_reason),
            "request: {got:?}\n{log}"
        );
        let found = findings_like(&s, "compute CR-001 failed:%");
        assert_eq!(
            found,
            vec![format!("compute CR-001 failed: {want_reason}")],
            "{log}"
        );
        assert_eq!(az.deleted(), vec!["hive-cr-001"], "{log}");
        assert!(
            crate::executor::executor_row(&s, "hands-cr-001")
                .0
                .is_empty(),
            "the executor row survived the failed provisioning\n{log}"
        );
        let (config, known) = ssh_files(&h);
        assert!(
            crate::pair::pair_alias_state(config.clone(), "hands-cr-001".into()).is_none()
                && crate::pair::pair_alias_state(config.clone(), "hive-cr-001-admin".into())
                    .is_none(),
            "ssh config after the failure: {config:?}\n{log}"
        );
        assert!(
            !known.lines().any(|l| l.starts_with("203.0.113.5 ")),
            "known_hosts after the failure: {known:?}\n{log}"
        );
        for want in [
            "compute CR-001: probe: under 10 GiB free on /srv/hive",
            "compute CR-001: removed executor hands-cr-001",
            "compute CR-001: removed the hands-cr-001 block from ~/.ssh/config",
            "compute CR-001: removed the hive-cr-001-admin block from ~/.ssh/config",
            "compute CR-001: removed 1 known_hosts line(s) for 203.0.113.5",
            &format!("compute CR-001 failed: {want_reason}"),
        ] {
            assert!(log.contains(want), "daemon log lacks {want:?}:\n{log}");
        }
    }

    /// A pairing that stops short for a reason of its own (here: not ok, git missing) fails the
    /// request with the probe's verdict, where the undo one-liner used to be.
    #[test]
    fn compute_fails_with_the_pair_verdict_not_its_undo_line() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        let s = new_store();
        let h = compute_harness(&s, AZURE_SETTINGS);
        let az = new_fake_az();
        let _fake = install_fake(crate::azure::set_run_az, {
            let az = az.clone();
            Box::new(move |ctx, args, timeout| az.run(ctx, args, timeout))
        });
        let _pair = set_pair_hands(Box::new(|st, _admin, name, now, o| {
            crate::executor::add_executor(
                st,
                now,
                name,
                &format!("hive@{name}"),
                &o.workdir,
                o.workers,
                &o.projects,
                hive_store::HUMAN_AGENT,
            )?;
            Ok(Some(Box::new(crate::pair::PairReport {
                ok: false,
                lines: vec![
                    format!("paired {name}: drained (the probe did not pass)"),
                    "  probe     FAILED: git missing; harness login missing (run `hive executor login hands-cr-001 --admin <admin@host> --with-token`)".into(),
                    format!("  next      fix that on the machine, then `hive executor probe {name}`"),
                    format!("  undo      hive remove executor {name}"),
                ],
                ..Default::default()
            })))
        }));
        let req = approved_request(&s);
        let log = once(&s, &h);
        let got = hive_tasks::get_compute(&s, &req.id).unwrap_or_default();
        assert_eq!(got.status, "failed", "request: {got:?}\n{log}");
        assert!(
            got.note
                .starts_with("pairing hands-cr-001 did not complete: probe FAILED: git missing")
                && got.note.ends_with("; what was created is released"),
            "note: {:?}\n{log}",
            got.note
        );
        assert!(
            place_login_calls().is_empty(),
            "a login was placed on hands that did not pair"
        );
        assert!(
            crate::executor::executor_row(&s, "hands-cr-001")
                .0
                .is_empty(),
            "the executor row survived\n{log}"
        );
        assert_eq!(az.deleted(), vec!["hive-cr-001"], "{log}");
    }

    /// The provider boundary refuses to buy without a login source, whatever reached it; the
    /// worker's caps hold such a request earlier still.
    #[test]
    fn provision_start_buys_nothing_without_a_login_source() {
        let _serial = crate::test_helpers::serial();
        let s = new_store();
        let req = ProvisionRequest {
            id: "CR-001".into(),
            project: "proj".into(),
            executor: "hands-cr-001".into(),
            store: Some((*s).clone()),
            ..Default::default()
        };
        let err = match provision_start("azure".into(), req.clone()) {
            Err(err) => err,
            Ok(_) => panic!("a machine was bought without a login source"),
        };
        assert!(
            err.to_string().starts_with(hive_tasks::NO_LOGIN_SOURCE)
                && err.to_string().ends_with("no machine was bought"),
            "error: {err}"
        );
        crate::test_helpers_hands::write_executor_token(&s);
        let (_, _, log) = match provision_start("azure".into(), req) {
            Ok(v) => v,
            Err(err) => panic!("with a source: {err}"),
        };
        log("nothing listens: %s", &[json!("fine")]);
    }

    #[test]
    fn format_log_fills_go_verbs_in_order() {
        assert_eq!(
            format_log(
                "%s is up at %s; attempt %d",
                &[json!("vm"), json!("1.2.3.4"), json!(3)]
            ),
            "vm is up at 1.2.3.4; attempt 3"
        );
        assert_eq!(format_log("100%% done %s", &[json!("x")]), "100% done x");
        assert_eq!(format_log("short %s %s", &[json!("one")]), "short one %?");
        assert_eq!(
            format_log("extra", &[json!("one"), json!(2)]),
            "extra one 2"
        );
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let sink = seen.clone();
        let log = provider_log(Some(std::sync::Arc::new(move |l: &str| {
            sink.lock().unwrap().push(l.to_string())
        })));
        log(
            "login: %s",
            &[json!(
                "token github_pat_11ATESTTOKEN0000000000_TESTTESTTESTTESTTESTTESTTESTTESTTESTTESTTESTTESTTESTTESTTEST placed"
            )],
        );
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["login: token [redacted] placed".to_string()],
            "the sink redacts"
        );
    }

    /// The rows older daemons left behind - 54 drained `hands-cr-N` executors on the live hub,
    /// each probed every cycle against an address that is gone - go with the sweep, with the ssh
    /// files written for their machines; a hand whose machine is live, and a drained machine
    /// Hive did not buy, stay.
    #[test]
    fn compute_sweep_forgets_the_hands_of_released_machines() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        let s = new_store();
        let h = compute_harness(&s, AZURE_SETTINGS);
        let az = new_fake_az();
        let _fake = install_fake(crate::azure::set_run_az, {
            let az = az.clone();
            Box::new(move |ctx, args, timeout| az.run(ctx, args, timeout))
        });
        let gone = approved_request(&s);
        let live = approved_request(&s);
        for (r, ip) in [(&gone, "203.0.113.5"), (&live, "203.0.113.6")] {
            let name = format!("hive-{}", r.id.to_lowercase());
            let executor = crate::compute::compute_executor_name(&r.id);
            crate::compute::save_machine(
                &s,
                &r.id,
                &Machine {
                    provider: "azure".into(),
                    name: name.clone(),
                    executor: executor.clone(),
                    address: format!("hiveadmin@{ip}"),
                    detail: BTreeMap::from([("public_ip".to_string(), ip.to_string())]),
                    ..Default::default()
                },
                TEST_NOW,
            );
            crate::executor::add_executor(
                &s,
                TEST_NOW,
                &executor,
                &format!("hive@{executor}"),
                HANDS_WORKDIR,
                1,
                "proj",
                hive_store::HUMAN_AGENT,
            )
            .unwrap_or_else(|err| panic!("{err}"));
            write_admin_alias(h.hub_home.clone(), format!("{name}-admin"), ip.into())
                .unwrap_or_else(|err| panic!("{err}"));
            let config = format!("{}/.ssh/config", h.hub_home);
            let mut text = std::fs::read_to_string(&config).unwrap_or_default();
            text.push('\n');
            text.push_str(&crate::pair::pair_host_block(executor, ip.into(), 22));
            std::fs::write(&config, text).expect("ssh config");
            crate::pair::pair_write_known_hosts(
                h.hub_home.clone(),
                ip.into(),
                22,
                vec![TEST_HOST_KEY.into()],
            );
        }
        hive_tasks::set_compute_status(&s, &gone.id, "failed", "", TEST_DAEMON)
            .unwrap_or_else(|err| panic!("{err}"));
        crate::compute::machine_released(&s, &gone.id, TEST_NOW);
        hive_tasks::set_compute_status(&s, &live.id, "ready", "hands-cr-002", TEST_DAEMON)
            .unwrap_or_else(|err| panic!("{err}"));
        crate::executor::add_executor(
            &s,
            TEST_NOW,
            "vm-seven",
            "hive@vm-seven",
            HANDS_WORKDIR,
            1,
            "*",
            hive_store::HUMAN_AGENT,
        )
        .unwrap_or_else(|err| panic!("{err}"));
        let log = once(&s, &h);
        assert!(
            crate::executor::executor_row(&s, "hands-cr-001")
                .0
                .is_empty(),
            "the stale hand stayed registered\n{log}"
        );
        assert_eq!(
            must_one(
                &s,
                "SELECT status FROM executors WHERE name = 'hands-cr-001'",
                &[]
            )
            .str("status"),
            "removed",
            "the row is kept as removed"
        );
        for kept in ["hands-cr-002", "vm-seven"] {
            assert!(
                !crate::executor::executor_row(&s, kept).0.is_empty(),
                "{kept} was removed by the sweep\n{log}"
            );
        }
        let (config, known) = ssh_files(&h);
        for gone in ["hands-cr-001", "hive-cr-001-admin"] {
            assert!(
                crate::pair::pair_alias_state(config.clone(), gone.into()).is_none(),
                "{gone} block kept: {config:?}"
            );
        }
        for kept in ["hands-cr-002", "hive-cr-002-admin"] {
            assert!(
                crate::pair::pair_alias_state(config.clone(), kept.into()).is_some(),
                "{kept} block removed: {config:?}"
            );
        }
        assert!(
            !known.lines().any(|l| l.starts_with("203.0.113.5 "))
                && known.lines().any(|l| l.starts_with("203.0.113.6 ")),
            "known_hosts: {known:?}"
        );
        assert!(
            log.contains("compute CR-001: removed executor hands-cr-001; its machine was released")
                && log
                    .contains("compute CR-001: removed the hands-cr-001 block from ~/.ssh/config")
                && log.contains("compute CR-001: removed 1 known_hosts line(s) for 203.0.113.5"),
            "{log}"
        );
        // Nothing left to forget: the next sweep says nothing about it.
        let later = hive_store::Store::open(hive_store::Options {
            home: s.home.clone(),
            now: crate::util::plus_hours(TEST_NOW, 1.0),
            ..Default::default()
        })
        .expect("later store");
        let cancel = hive_bus::CancellationToken::new();
        let w = crate::compute::new_compute_worker(
            &cancel,
            &later,
            crate::compute::ComputeOpts {
                hub_home: h.hub_home.clone(),
                ..Default::default()
            },
        );
        assert!(
            w.forget_stale_hands().is_empty(),
            "a second sweep found more"
        );
    }

    /// Two ways a daemon restart mid-create leaves a request `provisioning`: the machine was
    /// recorded (released at once) or not yet (the next sweep finds it by its tag).
    #[test]
    fn compute_stale_provisioning_is_released_now_or_by_the_sweep() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        let s = new_store();
        let h = compute_harness(&s, AZURE_SETTINGS);
        let az = new_fake_az();
        let _fake = install_fake(crate::azure::set_run_az, {
            let az = az.clone();
            Box::new(move |ctx, args, timeout| az.run(ctx, args, timeout))
        });
        let recorded = approved_request(&s);
        let forgotten = approved_request(&s);
        for r in [&recorded, &forgotten] {
            hive_tasks::set_compute_status(&s, &r.id, "provisioning", "", TEST_DAEMON)
                .unwrap_or_else(|err| panic!("{err}"));
            az.leak(&format!("hive-{}", r.id.to_lowercase()), &r.id, "proj");
        }
        crate::compute::save_machine(
            &s,
            &recorded.id,
            &Machine {
                provider: "azure".into(),
                name: "hive-cr-001".into(),
                executor: "hands-cr-001".into(),
                detail: BTreeMap::from([("resource_group".to_string(), "hive-hands".to_string())]),
                ..Default::default()
            },
            TEST_NOW,
        );
        let log = once(&s, &h);
        for r in [&recorded, &forgotten] {
            let got = hive_tasks::get_compute(&s, &r.id).unwrap_or_default();
            assert_eq!(got.status, "failed", "{}: {got:?}\n{log}", r.id);
        }
        assert_eq!(az.deleted(), vec!["hive-cr-001"], "first daemon run\n{log}");
        let found = findings_like(&s, "compute CR-00_ failed: the daemon stopped%");
        assert!(
            found.len() == 2
                && found.iter().any(|f| f.starts_with("compute CR-001") && f.ends_with("; what was created is released"))
                && found.iter().any(|f| f.starts_with("compute CR-002") && f.contains("released by the sweep")),
            "findings: {found:?}\n{log}"
        );
        // the sweep of that run saw CR-002 still provisioning; the next one releases it
        let log = once(&s, &h);
        assert_eq!(
            az.deleted(),
            vec!["hive-cr-001", "hive-cr-002"],
            "second run\n{log}"
        );
        let conn = s.conn().unwrap();
        let swept = crate::util::must_rows(
            &conn,
            "SELECT subject, detail FROM events WHERE action = 'compute.leftover-released' ORDER BY seq",
            &[],
        );
        assert!(
            swept.len() == 1
                && swept[0].str("subject") == "CR-002"
                && swept[0].str("detail").contains("\"name\": \"hive-cr-002\""),
            "leftover events: {swept:?}\n{log}"
        );
    }

    /// The sweep never calls the provider on a hub that has not bought a machine, releases only
    /// machines tagged with a request of this store that is over, and leaves the rest alone.
    #[test]
    fn compute_sweep_releases_only_what_is_over() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        let s = new_store();
        let h = compute_harness(&s, AZURE_SETTINGS);
        let az = new_fake_az();
        let _fake = install_fake(crate::azure::set_run_az, {
            let az = az.clone();
            Box::new(move |ctx, args, timeout| az.run(ctx, args, timeout))
        });
        az.leak("hive-cr-001", "CR-001", "proj");
        az.leak("hive-cr-002", "CR-002", "proj");
        az.leak("hive-cr-009", "CR-009", "other-store");
        let over = hive_tasks::request_compute(&s, "proj", "aurora", "", 2, "done long ago")
            .unwrap_or_else(|err| panic!("{err}"));
        let live = hive_tasks::request_compute(&s, "proj", "aurora", "", 2, "in use")
            .unwrap_or_else(|err| panic!("{err}"));
        let log = once(&s, &h);
        assert!(
            !az.has("vm list") && az.deleted().is_empty(),
            "nothing was ever bought here, yet az was asked: {:?}\n{log}",
            az.calls()
        );
        hive_tasks::set_compute_status(&s, &over.id, "failed", "", TEST_DAEMON)
            .unwrap_or_else(|err| panic!("{err}"));
        hive_tasks::set_compute_status(&s, &live.id, "ready", "hands-cr-002", TEST_DAEMON)
            .unwrap_or_else(|err| panic!("{err}"));
        crate::compute::save_machine(
            &s,
            &live.id,
            &Machine {
                provider: "azure".into(),
                name: "hive-cr-002".into(),
                executor: "hands-cr-002".into(),
                ..Default::default()
            },
            &s.now(),
        );
        let log = once(&s, &h);
        assert!(
            az.has("vm list -d --query"),
            "no sweep: {:?}\n{log}",
            az.calls()
        );
        assert_eq!(az.deleted(), vec!["hive-cr-001"], "{log}");
        let found = findings_like(&s, "compute CR-001: released hive-cr-001 (azure)%");
        assert_eq!(found.len(), 1, "findings: {found:?}\n{log}");
        let alive: Vec<String> = az.0.lock().unwrap().created.keys().cloned().collect();
        assert_eq!(alive, vec!["hive-cr-002", "hive-cr-009"], "left alone");
    }

    #[test]
    fn hands_arch_follows_the_size_family() {
        for (provider, size, want) in [
            ("azure", "Standard_B2ms", "amd64"),
            ("azure", "Standard_D2s_v5", "amd64"),
            ("azure", "Standard_D2ps_v5", "arm64"),
            ("azure", "Standard_B2pts_v2", "arm64"),
            ("azure", "Standard_E4pds_v5", "arm64"),
            ("azure", "Standard_NP10s", "amd64"),
            ("azure", "", "amd64"),
            ("aws", "t3.medium", "amd64"),
            ("aws", "t4g.medium", "arm64"),
            ("aws", "c6gn.large", "arm64"),
            ("aws", "a1.large", "arm64"),
            ("aws", "g4dn.xlarge", "amd64"),
            ("gcp", "e2-medium", "amd64"),
            ("gcp", "t2a-standard-2", "arm64"),
            ("gcp", "c4a-standard-4", "arm64"),
            ("nowhere", "anything", "amd64"),
        ] {
            assert_eq!(
                crate::compute::hands_arch_for_size(provider, size),
                want,
                "{provider} {size}"
            );
        }
    }

    #[test]
    fn tend_retires_and_deletes_our_ec2_instance() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        let s = new_store();
        new_bare_repo(&s);
        set_setting(&s, "compute.provider", "aws");
        set_setting(&s, "compute.aws.region", "eu-west-1");
        drained_by_login(&s, "vm-seven", "hive@vm-seven");
        let aws = new_fake_aws();
        aws.0.lock().unwrap().instances.insert(
            "i-0777".into(),
            AwsFake {
                id: "i-0777".into(),
                name: "hive-cr-007".into(),
                request: "CR-007".into(),
                project: "proj".into(),
                state: "running".into(),
                ip: "203.0.113.7".into(),
            },
        );
        aws.0.lock().unwrap().describes = 5;
        let _fake = install_fake(crate::aws::set_run_aws, {
            let aws = aws.clone();
            Box::new(move |ctx, args, timeout| aws.run(ctx, args, timeout))
        });
        let _host = install_ssh_hostname(|_host| "203.0.113.7".into());

        let lines = crate::retire::tend_executors(&s, TEST_NOW.into(), TEST_DAEMON.into());
        let calls = aws.joined();
        if calls.len() != 2
            || calls[0]
                != "ec2 describe-instances --filters Name=tag:hive,Values=hands Name=instance-state-name,Values=pending,running,stopping,stopped --region eu-west-1 --output json"
            || calls[1]
                != "ec2 stop-instances --instance-ids i-0777 --region eu-west-1 --output json"
        {
            panic!("aws calls: {calls:?}");
        }
        let r = must_one(
            &s,
            "SELECT * FROM retired_vms WHERE executor = 'vm-seven'",
            &[],
        );
        if r.0.is_empty()
            || r.str("provider") != "aws"
            || r.str("name") != "hive-cr-007"
            || r.str("delete_after") != crate::util::plus_hours(TEST_NOW, 24.0)
            || !r.str("detail").contains(r#""instance_id":"i-0777""#)
        {
            panic!("retired_vms: {r:?}");
        }
        let ev = must_one(
            &s,
            "SELECT * FROM events WHERE action = 'executor.retired' AND subject = 'vm-seven'",
            &[],
        );
        let d = crate::util::json_map(&ev.str("detail"));
        if d["provider"] != "aws" || d["instance_id"] != "i-0777" || d["place"] != "eu-west-1" {
            panic!("event: {ev:?}");
        }
        let p = must_one(
            &s,
            "SELECT * FROM posts WHERE kind = 'finding' AND thread = 'compute'",
            &[],
        );
        if p.str("summary")
            != format!(
                "retired vm-seven: login refused; EC2 instance hive-cr-007 (eu-west-1) stopped now, deleted at {} unless kept",
                crate::util::plus_hours(TEST_NOW, 24.0)
            )
        {
            panic!("summary: {:?}", p.str("summary"));
        }
        if !p.str("body").contains("Undo: `aws ec2 start-instances --instance-ids i-0777 --region eu-west-1` then `hive executor activate vm-seven`") {
            panic!("body:\n{}", p.str("body"));
        }
        if !lines
            .join("\n")
            .contains("retired vm-seven: stopped hive-cr-007 (eu-west-1)")
        {
            panic!("lines: {lines:?}");
        }

        // At the grace the instance is terminated through the row's provider, whatever compute.provider says now.
        set_setting(&s, "compute.provider", "azure");
        aws.reset();
        let later = crate::util::plus_hours(TEST_NOW, 24.0);
        let lines = crate::retire::tend_executors(&s, later.clone(), TEST_DAEMON.into());
        let calls = aws.joined();
        if calls.len() != 2
            || !calls[0].starts_with("ec2 describe-instances --filters Name=instance-id,Values=i-0777 --region eu-west-1")
            || calls[1] != "ec2 terminate-instances --instance-ids i-0777 --region eu-west-1 --output json"
        {
            panic!("delete calls: {calls:?}");
        }
        let r = must_one(
            &s,
            "SELECT * FROM retired_vms WHERE executor = 'vm-seven'",
            &[],
        );
        if r.str("deleted_at") != later || !r.str("note").is_empty() {
            panic!("row: {r:?}");
        }
        if must_one(
            &s,
            "SELECT * FROM posts WHERE kind = 'finding' AND thread = 'compute' AND summary = 'deleted hive-cr-007 (eu-west-1)'",
            &[],
        )
        .0
        .is_empty()
        {
            panic!(
                "delete finding missing: {:?}",
                crate::test_helpers_daemon::must_rows(&s, "SELECT summary FROM posts", &[])
            );
        }
        if !lines
            .join("\n")
            .contains("deleted hive-cr-007 (eu-west-1): instance i-0777 with its root volume")
        {
            panic!("lines: {lines:?}");
        }
        // Already gone: closed without a terminate.
        aws.reset();
        exec1(
            &s,
            "INSERT INTO retired_vms (executor, provider, resource_group, name, deallocated_at, delete_after, detail) VALUES ('vm-gone', 'aws', '', 'hive-cr-gone', ?, ?, ?)",
            &[
                &TEST_NOW,
                &TEST_NOW,
                &r#"{"detail":{"instance_id":"i-gone","region":"eu-west-1"}}"#,
            ],
        );
        let lines = crate::retire::tend_executors(&s, later.clone(), TEST_DAEMON.into());
        let calls = aws.joined();
        if calls.len() != 1
            || !calls[0]
                .starts_with("ec2 describe-instances --filters Name=instance-id,Values=i-gone")
        {
            panic!("missing instance calls: {calls:?}");
        }
        let r = must_one(
            &s,
            "SELECT * FROM retired_vms WHERE executor = 'vm-gone'",
            &[],
        );
        if r.str("note") != "vm not found at delete time" {
            panic!("row: {r:?}");
        }
        let _ = lines;
    }

    #[test]
    fn tend_retires_and_deletes_our_gce_instance() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        crate::commands::register();
        let s = new_store();
        new_bare_repo(&s);
        s.set_setting(crate::util::HUMAN_AGENT, "compute.provider", "gcp")
            .unwrap();
        s.set_setting(crate::util::HUMAN_AGENT, "compute.gcp.project", "hive-proj")
            .unwrap();
        drained_by_login(&s, "vm-seven", "hive@vm-seven");
        let gcp = new_fake_gcloud();
        gcp.0.lock().unwrap().instances.insert(
            "hive-cr-007".into(),
            GcpFake {
                name: "hive-cr-007".into(),
                status: "RUNNING".into(),
                ip: "203.0.113.7".into(),
                request: "cr-007".into(),
                project: "proj".into(),
            },
        );
        let _fake = install_fake(crate::gcp::set_run_gcloud, {
            let gcp = gcp.clone();
            Box::new(move |ctx, args, timeout| gcp.run(ctx, args, timeout))
        });
        let _host = install_ssh_hostname(|_| "203.0.113.7".into());
        let lines = crate::retire::tend_executors(&s, TEST_NOW.into(), "daemon".into());
        let calls = gcp.joined();
        assert_eq!(
            calls,
            vec![
                "compute instances list --filter=labels.hive=hands --project hive-proj --format json",
                "compute instances stop hive-cr-007 --zone europe-west1-b --quiet --project hive-proj",
            ],
            "gcloud calls"
        );
        assert!(
            lines
                .join("\n")
                .contains("retired vm-seven: stopped hive-cr-007 (europe-west1-b)"),
            "lines: {lines:?}"
        );
        gcp.reset();
        let later = crate::util::plus_hours(TEST_NOW, 24.0);
        let lines = crate::retire::tend_executors(&s, later, "daemon".into());
        let calls = gcp.joined();
        assert_eq!(
            calls,
            vec![
                "compute instances describe hive-cr-007 --zone europe-west1-b --project hive-proj --format json",
                "compute instances delete hive-cr-007 --zone europe-west1-b --quiet --project hive-proj",
            ],
            "delete calls"
        );
        assert!(
            lines.join("\n").contains(
                "deleted hive-cr-007 (europe-west1-b): instance hive-cr-007 with its boot disk"
            ),
            "lines: {lines:?}"
        );
    }

    #[test]
    fn tend_retires_and_deletes_our_multipass_instance() {
        let _serial = crate::test_helpers::serial();
        let _lock = provider_test_lock();
        crate::commands::register();
        let s = new_store();
        new_bare_repo(&s);
        s.set_setting(crate::util::HUMAN_AGENT, "compute.provider", "multipass")
            .unwrap();
        drained_by_login(&s, "vm-seven", "hive@vm-seven");
        drained_by_login(&s, "vm-mine", "hive@192.168.64.3");
        let mp = new_fake_multipass();
        mp.0.lock().unwrap().instances.insert(
            "hive-cr-007".into(),
            MpFake {
                name: "hive-cr-007".into(),
                state: "Running".into(),
                ip: "192.168.64.7".into(),
            },
        );
        mp.0.lock().unwrap().instances.insert(
            "primary".into(),
            MpFake {
                name: "primary".into(),
                state: "Running".into(),
                ip: "192.168.64.3".into(),
            },
        );
        let _fake = install_fake(crate::multipass::set_run_multipass, {
            let mp = mp.clone();
            Box::new(move |ctx, args, timeout| mp.run(ctx, args, timeout))
        });
        let _host = install_ssh_hostname(|host| {
            if host == "vm-seven" {
                "192.168.64.7".into()
            } else {
                host.into()
            }
        });
        let lines = crate::retire::tend_executors(&s, TEST_NOW.into(), "daemon".into());
        let calls = mp.joined();
        assert_eq!(
            calls,
            vec![
                "list --format json",
                "list --format json",
                "stop hive-cr-007"
            ],
            "multipass calls"
        );
        let text = lines.join("\n");
        assert!(
            text.contains("retired vm-seven: stopped hive-cr-007 (this machine)")
                && text.contains("left running because Hive did not create it (no Multipass instance tagged hive=hands is named vm-mine or answers at 192.168.64.3)"),
            "lines: {lines:?}"
        );
        mp.reset();
        let later = crate::util::plus_hours(TEST_NOW, 24.0);
        let lines = crate::retire::tend_executors(&s, later, "daemon".into());
        assert_eq!(
            mp.joined(),
            vec![
                "info hive-cr-007 --format json",
                "delete --purge hive-cr-007"
            ],
            "delete calls"
        );
        assert!(
            lines
                .join("\n")
                .contains("deleted hive-cr-007 (this machine): instance hive-cr-007 with its disk"),
            "lines: {lines:?}"
        );
        assert!(
            mp.0.lock().unwrap().instances.contains_key("primary"),
            "the owner's own VM was deleted"
        );
    }
}
