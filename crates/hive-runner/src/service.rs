#![allow(dead_code)]

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::{LazyLock, RwLock};

pub(crate) const SERVICE_LABEL: &str = "hive.daemon";
pub(crate) const HUB_PORT: i32 = 8765;
pub(crate) const LAUNCHER_NAME: &str = "hive-daemon.sh";
pub(crate) static SERVICE_SLEEP: LazyLock<RwLock<fn(std::time::Duration)>> =
    LazyLock::new(|| RwLock::new(std::thread::sleep));
pub(crate) fn set_service_sleep(f: fn(std::time::Duration)) -> fn(std::time::Duration) {
    std::mem::replace(&mut *SERVICE_SLEEP.write().expect("service sleep lock"), f)
}
pub(crate) type RunServiceFn = dyn Fn(Vec<String>) -> (i32, String) + Send + Sync;
pub(crate) static RUN_SERVICE: LazyLock<RwLock<Box<RunServiceFn>>> = LazyLock::new(|| {
    RwLock::new(Box::new(|argv| {
        if argv.is_empty() {
            return (127, String::new());
        }
        match Command::new(&argv[0]).args(&argv[1..]).output() {
            Ok(out) => {
                let mut text = String::from_utf8_lossy(&out.stdout).to_string();
                text.push_str(&String::from_utf8_lossy(&out.stderr));
                (out.status.code().unwrap_or(-1), text)
            }
            Err(err) => (127, err.to_string()),
        }
    }))
});
pub(crate) fn set_run_service(f: Box<RunServiceFn>) -> Box<RunServiceFn> {
    std::mem::replace(&mut *RUN_SERVICE.write().expect("run service lock"), f)
}
pub(crate) static PROTECTED_FOLDERS: &[&str] = &["Desktop", "Documents", "Downloads"];

#[derive(Debug, Clone, Default)]
pub(crate) struct ServiceUnit {
    pub(crate) name: String,
    pub(crate) label: String,
    pub(crate) unit: String,
    pub(crate) launcher: String,
    pub(crate) log: String,
    pub(crate) desc: String,
    pub(crate) args: Vec<String>,
    pub(crate) guard: bool,
    pub(crate) text: String,
}

#[derive(Debug, Clone, Default)]
pub struct ServiceInstallOptions {
    pub interval: i32,
    pub hub_port: i32,
    pub no_hub: bool,
    pub dry_run: bool,
    /// DeferHub writes the hub's unit but leaves its load to the caller (load_hub in the result): a foreground
    /// hub on the same port must stop listening first, or the service's hub cannot bind.
    pub defer_hub: bool,
}

pub type LoadUnitFn = Arc<dyn Fn() -> anyhow::Result<()> + Send + Sync>;

/// The load step `prepare_unit` returns. It is handed the writer when it runs, so the load's
/// lines reach the same `out` as the rest of the install.
pub(crate) type LoadStep = Box<dyn Fn(&mut dyn std::io::Write) -> anyhow::Result<()> + Send + Sync>;

struct Tee<'a> {
    out: &'a mut dyn std::io::Write,
    buf: &'a mut Vec<u8>,
}

impl std::io::Write for Tee<'_> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(b);
        self.out.write_all(b)?;
        Ok(b.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }
}

#[derive(Clone, Default, Serialize)]
pub struct ServiceInstall {
    #[serde(rename = "kind")]
    pub kind: String,
    #[serde(rename = "daemon_path")]
    pub daemon_path: String,
    #[serde(rename = "hub_path")]
    pub hub_path: String,
    #[serde(rename = "output")]
    pub output: String,
    #[serde(skip)]
    pub load_hub: Option<LoadUnitFn>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UnitState {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "path")]
    pub path: String,
    #[serde(rename = "present")]
    pub present: bool,
    #[serde(rename = "loaded")]
    pub loaded: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServiceState {
    #[serde(rename = "kind")]
    pub kind: String,
    #[serde(rename = "supported")]
    pub supported: bool,
    #[serde(rename = "installed")]
    pub installed: bool,
    #[serde(rename = "units")]
    pub units: Vec<UnitState>,
    #[serde(rename = "binary")]
    pub binary: String,
    #[serde(rename = "logs")]
    pub logs: String,
}

pub(crate) fn daemon_unit(interval: i32) -> ServiceUnit {
    ServiceUnit {
        name: "daemon".into(),
        label: SERVICE_LABEL.into(),
        unit: "hive-daemon".into(),
        launcher: LAUNCHER_NAME.into(),
        log: "daemon.log".into(),
        desc: "Hive daemon - the agents' machine".into(),
        args: vec!["daemon".into(), "--interval".into(), interval.to_string()],
        guard: true,
        text: String::new(),
    }
}

pub(crate) fn hub_unit(port: i32) -> ServiceUnit {
    let port = if port <= 0 { HUB_PORT } else { port };
    ServiceUnit {
        name: "hub".into(),
        label: "hive.hub".into(),
        unit: "hive-hub".into(),
        launcher: "hive-hub.sh".into(),
        log: "hub.log".into(),
        desc: "Hive hub - the owner's console".into(),
        args: vec![
            "hub".into(),
            "--port".into(),
            port.to_string(),
            "--no-browser".into(),
            "--restart-on-change".into(),
        ],
        guard: false,
        text: String::new(),
    }
}

pub(crate) fn service_units(interval: i32, port: i32, no_hub: bool) -> Vec<ServiceUnit> {
    let mut units = vec![daemon_unit(interval)];
    if !no_hub {
        units.push(hub_unit(port));
    }
    units
}

pub(crate) fn service_paths(system: String, home: String) -> (String, String) {
    unit_paths(daemon_unit(0), system, home)
}

pub(crate) fn unit_paths(u: ServiceUnit, system: String, home: String) -> (String, String) {
    let system = if system.is_empty() {
        hive_gitenv::GOOS.to_string()
    } else {
        system
    };
    let home = if home.is_empty() {
        crate::util::home_dir()
    } else {
        home
    };
    if system == "macos" || system == "darwin" {
        (
            "launchd".into(),
            Path::new(&home)
                .join("Library")
                .join("LaunchAgents")
                .join(format!("{}.plist", u.label))
                .display()
                .to_string(),
        )
    } else {
        (
            "systemd".into(),
            Path::new(&home)
                .join(".config/systemd/user")
                .join(format!("{}.service", u.unit))
                .display()
                .to_string(),
        )
    }
}

pub(crate) fn xml_esc(s: String) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(crate) fn service_file_text(
    kind: String,
    interval: i32,
    binary: String,
    log_path: String,
    path_env: String,
    hive_home: String,
) -> String {
    unit_file_text(
        daemon_unit(interval),
        kind,
        binary,
        log_path,
        path_env,
        hive_home,
    )
}

pub(crate) fn unit_file_text(
    u: ServiceUnit,
    kind: String,
    binary: String,
    log_path: String,
    path_env: String,
    hive_home: String,
) -> String {
    let args = vec![
        "/bin/sh".to_string(),
        unit_launcher_path(u.clone(), hive_home.clone()),
    ];
    if kind == "launchd" {
        let mut lines = vec![
            r#"<?xml version="1.0" encoding="UTF-8"?>"#.to_string(),
            r#"<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">"#.to_string(),
            r#"<plist version="1.0">"#.to_string(),
            "<dict>".to_string(),
            format!("  <key>Label</key><string>{}</string>", u.label),
            "  <key>ProgramArguments</key>".to_string(),
            "  <array>".to_string(),
        ];
        for a in args {
            lines.push(format!("    <string>{}</string>", xml_esc(a)));
        }
        lines.extend([
            "  </array>".to_string(),
            "  <key>RunAtLoad</key><true/>".to_string(),
            "  <key>KeepAlive</key><true/>".to_string(),
            "  <key>ThrottleInterval</key><integer>30</integer>".to_string(),
            format!(
                "  <key>WorkingDirectory</key><string>{}</string>",
                xml_esc(
                    Path::new(&binary)
                        .parent()
                        .unwrap_or(Path::new(""))
                        .display()
                        .to_string()
                )
            ),
            format!(
                "  <key>StandardOutPath</key><string>{}</string>",
                xml_esc(log_path.clone())
            ),
            format!(
                "  <key>StandardErrorPath</key><string>{}</string>",
                xml_esc(log_path)
            ),
            "  <key>EnvironmentVariables</key>".to_string(),
            "  <dict>".to_string(),
            format!("    <key>PATH</key><string>{}</string>", xml_esc(path_env)),
            format!(
                "    <key>HIVE_HOME</key><string>{}</string>",
                xml_esc(hive_home)
            ),
            "    <key>LANG</key><string>en_US.UTF-8</string>".to_string(),
            "  </dict>".to_string(),
            "</dict>".to_string(),
            "</plist>".to_string(),
            String::new(),
        ]);
        return lines.join("\n");
    }
    [
        "[Unit]".to_string(),
        format!("Description={}", u.desc),
        "After=network-online.target".to_string(),
        String::new(),
        "[Service]".to_string(),
        format!("ExecStart={}", crate::util::shell_join(&args)),
        format!(
            "WorkingDirectory={}",
            Path::new(&binary)
                .parent()
                .unwrap_or(Path::new(""))
                .display()
        ),
        "Restart=always".to_string(),
        "RestartSec=30".to_string(),
        systemd_env_line("PATH", &path_env),
        systemd_env_line("HIVE_HOME", &hive_home),
        format!("StandardOutput=append:{log_path}"),
        format!("StandardError=append:{log_path}"),
        String::new(),
        "[Install]".to_string(),
        "WantedBy=default.target".to_string(),
        String::new(),
    ]
    .join("\n")
}

/// One `Environment=` line of a systemd unit. systemd splits what follows
/// at spaces and reads quotes and backslashes as its own, so a value that
/// holds any of them goes inside double quotes with `"` and `\` escaped:
/// under WSL the PATH carries `/mnt/c/Program Files` and the like, and
/// unquoted it was cut at the first space. `%` starts a specifier in a unit
/// file, so it is doubled. A line end inside the value is written as an
/// escape: systemd ends a line at a bare carriage return too, and a `PATH`
/// set in a profile saved with Windows line ends carries one. A value
/// systemd reads whole as it stands is written as before.
pub(crate) fn systemd_env_line(name: &str, value: &str) -> String {
    let value = value.replace('%', "%%");
    let reads_whole = !value
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\'));
    if reads_whole {
        return format!("Environment={name}={value}");
    }
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("Environment=\"{name}={escaped}\"")
}

pub(crate) fn launcher_path(hive_home: String) -> String {
    unit_launcher_path(daemon_unit(0), hive_home)
}

pub(crate) fn unit_launcher_path(u: ServiceUnit, hive_home: String) -> String {
    Path::new(&hive_home)
        .join("bin")
        .join(u.launcher)
        .display()
        .to_string()
}

pub(crate) fn launcher_text(binary: String, interval: i32) -> String {
    unit_launcher_text(daemon_unit(interval), binary)
}

pub(crate) fn unit_launcher_text(u: ServiceUnit, binary: String) -> String {
    let head = vec![
        "#!/bin/sh".to_string(),
        "# Written by `hive service install`. The service runs hive through this script so that a".to_string(),
        "# restart is a loop here, not a launchd/systemd throttle, and the crash guard has a place to".to_string(),
        "# live. macOS holds hive itself responsible for reading your folders; the stable signing".to_string(),
        "# requirement hive gives every build is what keeps that answer across builds (ADR-035).".to_string(),
    ];
    let mut start_args = vec![binary.clone()];
    start_args.extend(u.args.clone());
    let start = format!("{} &", crate::util::shell_join(&start_args));
    if !u.guard {
        let mut lines = head;
        lines.extend([
            start,
            "pid=$!".into(),
            r#"trap 'kill -TERM "$pid" 2>/dev/null' TERM INT HUP"#.into(),
            r#"wait "$pid""#.into(),
            "code=$?".into(),
            r#"# A SIGKILL'd child becomes a zombie; kill -0 still succeeds, so wait again would loop forever."#.into(),
            r#"while [ "$code" -gt 128 ] && [ "$code" -ne 137 ] && kill -0 "$pid" 2>/dev/null; do"#.into(),
            r#"  wait "$pid""#.into(),
            "  code=$?".into(),
            "done".into(),
            r#"exit "$code""#.into(),
            String::new(),
        ]);
        return lines.join("\n");
    }
    let mut lines = head;
    lines.extend([
        "# Three exits other than 3 within a minute of starting put the previous build back (ADR-035).".into(),
        format!("bin={}", crate::util::shell_join(&[binary])),
        "early=0".into(),
        "stopping=".into(),
        "while :; do".into(),
        "  started=$(date +%s)".into(),
        format!("  {start}"),
        "  pid=$!".into(),
        r#"  trap 'stopping=1; kill -TERM "$pid" 2>/dev/null' TERM INT HUP"#.into(),
        r#"  wait "$pid""#.into(),
        "  code=$?".into(),
        r#"  while [ "$code" -gt 128 ] && [ "$code" -ne 137 ] && kill -0 "$pid" 2>/dev/null; do"#.into(),
        r#"    wait "$pid""#.into(),
        "    code=$?".into(),
        "  done".into(),
        r#"  if [ -n "$stopping" ] || [ "$code" -eq 3 ] || [ $(( $(date +%s) - started )) -ge 60 ]; then"#.into(),
        r#"    exit "$code""#.into(),
        "  fi".into(),
        "  early=$((early + 1))".into(),
        r#"  if [ "$early" -lt 3 ]; then"#.into(),
        "    sleep 2".into(),
        "    continue".into(),
        "  fi".into(),
        "  early=0".into(),
        r#"  if [ ! -f "$bin.prev" ]; then"#.into(),
        r#"    exit "$code""#.into(),
        "  fi".into(),
        r#"  mv -f "$bin" "$bin.failed" && mv -f "$bin.prev" "$bin" || exit "$code""#.into(),
        r#"  echo "restored the previous hive after 3 early exits (last exit $code)""#.into(),
        r#"  date -u +%Y-%m-%dT%H:%M:%SZ > "$bin.restored""#.into(),
        "  sleep 2".into(),
        "done".into(),
        String::new(),
    ]);
    lines.join("\n")
}

pub(crate) fn launcher_from_plist(text: String) -> String {
    for mut line in text.lines().map(str::trim) {
        let mut path = String::new();
        if line.starts_with("<string>") && line.ends_with(".sh</string>") {
            path = line
                .trim_start_matches("<string>")
                .trim_end_matches("</string>")
                .to_string();
        } else if line.starts_with("ExecStart=/bin/sh ") && line.ends_with(".sh") {
            path = line.trim_start_matches("ExecStart=/bin/sh ").to_string();
        }
        line = path.trim_matches(&['\'', '"'][..]);
        if !line.is_empty()
            && Path::new(line)
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|b| {
                    b.starts_with("hive-")
                        || b.starts_with(&format!("{}-", hive_store::LEGACY_NAME))
                })
        {
            return line.to_string();
        }
    }
    String::new()
}

pub(crate) fn service_commands(kind: String, path: String, action: String) -> Vec<Vec<String>> {
    unit_commands(daemon_unit(0), kind, path, action)
}

pub(crate) fn unit_commands(
    u: ServiceUnit,
    kind: String,
    path: String,
    action: String,
) -> Vec<Vec<String>> {
    if kind == "launchd" {
        #[cfg(unix)]
        let uid = unsafe { libc::getuid() };
        // launchd is macOS's, so off unix this arm is not asked for.
        #[cfg(not(unix))]
        let uid = 0;
        let domain = format!("gui/{uid}");
        return match action.as_str() {
            "install" => vec![
                vec![
                    "launchctl".into(),
                    "bootout".into(),
                    format!("{domain}/{}", u.label),
                ],
                vec!["launchctl".into(), "bootstrap".into(), domain, path],
            ],
            "uninstall" => vec![vec![
                "launchctl".into(),
                "bootout".into(),
                format!("{domain}/{}", u.label),
            ]],
            _ => vec![vec![
                "launchctl".into(),
                "print".into(),
                format!("{domain}/{}", u.label),
            ]],
        };
    }
    match action.as_str() {
        "install" => vec![
            vec!["systemctl".into(), "--user".into(), "daemon-reload".into()],
            vec![
                "systemctl".into(),
                "--user".into(),
                "enable".into(),
                "--now".into(),
                u.unit,
            ],
        ],
        "uninstall" => vec![vec![
            "systemctl".into(),
            "--user".into(),
            "disable".into(),
            "--now".into(),
            u.unit,
        ]],
        _ => vec![vec![
            "systemctl".into(),
            "--user".into(),
            "status".into(),
            u.unit,
            "--no-pager".into(),
        ]],
    }
}

/// The running program's real file, symbolic links resolved: the one an
/// update replaces. Named once, when the process first asks
/// (`hive_cli::managed::this`): once the file has been replaced or removed,
/// Linux gives the running one the name `hive (deleted)`.
pub(crate) fn hive_binary() -> String {
    hive_cli::managed::this_real_path()
        .map(|real| real.display().to_string())
        .unwrap_or_else(|| "hive".to_string())
}

/// The path a service starts hive by. A package manager keeps each version
/// in a place of its own and names the program by a path that stays
/// (`hive_cli::managed`, the marker's `path`): a service file that named
/// the real file would name nothing after the next upgrade. Everywhere else
/// it is [`hive_binary`].
pub(crate) fn launch_binary() -> String {
    hive_cli::managed::this_launch_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "hive".to_string())
}

/// The directories a person's tools usually live in, in the order they are
/// added to a service's PATH: the one hive itself is started from, the
/// user's own (`~/.local/bin` holds mise and uv, `~/.cargo/bin` cargo), then
/// Homebrew's and the system's.
pub(crate) fn usual_tool_dirs(user_home: &str, hive_bin: &str) -> Vec<String> {
    let mut dirs = Vec::new();
    if !hive_bin.is_empty() {
        dirs.push(hive_bin.to_string());
    }
    if !user_home.is_empty() {
        dirs.push(format!("{user_home}/.local/bin"));
        dirs.push(format!("{user_home}/.cargo/bin"));
    }
    dirs.extend(
        [
            "/opt/homebrew/bin",
            "/home/linuxbrew/.linuxbrew/bin",
            "/usr/local/bin",
            "/snap/bin",
        ]
        .map(String::from),
    );
    dirs
}

/// The PATH a service is given, and the directories that were added to the
/// caller's to make it. A service keeps the PATH it was installed with, and
/// every tool the daemon and its runs start is found on it or not at all:
/// gh for pull requests, a credential helper, the tools a project's recipe
/// pins. So the caller's PATH is kept as it is and in its order, and each of
/// the `usual` directories that exists and is not on it goes after it.
/// `hive service install` run from a script, over ssh or from an app, where
/// PATH is `/usr/bin:/bin:/usr/sbin:/sbin`, once left a daemon that could
/// start none of them (2026-11-20: two days of refused pull requests,
/// skipped fetches and failed backups before anyone saw why). A directory
/// added here comes last, so it shadows nothing the caller's PATH names.
pub(crate) fn service_path(
    caller: &str,
    usual: &[String],
    exists: &dyn Fn(&str) -> bool,
) -> (String, Vec<String>) {
    let same = |a: &str, b: &str| a.trim_end_matches('/') == b.trim_end_matches('/');
    let mut have: Vec<&str> = caller.split(':').filter(|d| !d.is_empty()).collect();
    let mut added = Vec::new();
    for dir in usual {
        if have.iter().any(|d| same(d, dir)) || !exists(dir) {
            continue;
        }
        have.push(dir);
        added.push(dir.clone());
    }
    let path = if added.is_empty() {
        caller.to_string()
    } else if caller.is_empty() {
        added.join(":")
    } else {
        format!("{caller}:{}", added.join(":"))
    };
    (path, added)
}

/// [`service_path`] for this process: its own PATH and the usual
/// directories of this user and this hive.
fn path_for_services() -> (String, Vec<String>) {
    let caller = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into());
    let binary = launch_binary();
    let hive_bin = Path::new(&binary)
        .parent()
        .map(|dir| dir.display().to_string())
        .unwrap_or_default();
    service_path(
        &caller,
        &usual_tool_dirs(&crate::util::home_dir(), &hive_bin),
        &|dir| Path::new(dir).is_dir(),
    )
}

pub(crate) fn install_service_inner(
    out: &mut dyn std::io::Write,
    interval: i32,
    hive_home: String,
    system: String,
    user_home: String,
    dry_run: bool,
) -> anyhow::Result<String> {
    install_unit(
        out,
        daemon_unit(interval),
        hive_home,
        system,
        user_home,
        dry_run,
    )
}

pub(crate) fn install_unit(
    out: &mut dyn std::io::Write,
    u: ServiceUnit,
    hive_home: String,
    system: String,
    user_home: String,
    dry_run: bool,
) -> anyhow::Result<String> {
    let (path, load) = prepare_unit(out, u, hive_home, system, user_home, dry_run)?;
    if let Some(load) = load {
        load(out)?;
    }
    Ok(path)
}

pub(crate) fn prepare_unit(
    out: &mut dyn std::io::Write,
    u: ServiceUnit,
    hive_home: String,
    system: String,
    user_home: String,
    dry_run: bool,
) -> anyhow::Result<(String, Option<LoadStep>)> {
    let (kind, path) = unit_paths(u.clone(), system, user_home);
    let log_dir = Path::new(&hive_home).join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let (path_env, _) = path_for_services();
    let binary = launch_binary();
    let text = unit_file_text(
        u.clone(),
        kind.clone(),
        binary.clone(),
        log_dir.join(&u.log).display().to_string(),
        path_env,
        hive_home.clone(),
    );
    let launcher = unit_launcher_text(u.clone(), binary);
    let launcher_file = unit_launcher_path(u.clone(), hive_home);
    writeln!(out, "{} launcher: {}", u.name, launcher_file)?;
    writeln!(out, "{} {} service file: {}", u.name, kind, path)?;
    if dry_run {
        writeln!(out, "{launcher}")?;
        writeln!(out, "{text}")?;
        return Ok((path, None));
    }
    if same_file(path.clone(), text.clone()) && same_file(launcher_file.clone(), launcher.clone()) {
        let status = unit_commands(u.clone(), kind.clone(), path.clone(), "status".into());
        if (RUN_SERVICE.read().expect("run service lock"))(status[0].clone()).0 == 0 {
            writeln!(out, "{}: unchanged and loaded; left running", u.name)?;
            return Ok((path, None));
        }
    }
    if let Some(parent) = Path::new(&launcher_file).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&launcher_file, launcher)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&launcher_file, std::fs::Permissions::from_mode(0o755))?;
    }
    if let Some(parent) = Path::new(&path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, text)?;
    let u2 = u.clone();
    let kind2 = kind.clone();
    let path2 = path.clone();
    Ok((
        path,
        Some(Box::new(move |out: &mut dyn std::io::Write| {
            load_unit(out, u2.clone(), kind2.clone(), path2.clone())
        })),
    ))
}

pub(crate) fn load_unit(
    out: &mut dyn std::io::Write,
    u: ServiceUnit,
    kind: String,
    path: String,
) -> anyhow::Result<()> {
    for (i, cmd) in unit_commands(u.clone(), kind.clone(), path.clone(), "install".into())
        .into_iter()
        .enumerate()
    {
        let (mut rc, mut text) = (RUN_SERVICE.read().expect("run service lock"))(cmd.clone());
        if kind == "launchd" && i == 1 && rc == 5 {
            (SERVICE_SLEEP.read().expect("service sleep lock"))(std::time::Duration::from_secs(3));
            (rc, text) = (RUN_SERVICE.read().expect("run service lock"))(cmd.clone());
        }
        let tolerated = kind == "launchd" && i == 0;
        let mut suffix = String::new();
        if rc != 0 {
            if tolerated {
                suffix = " (not loaded before)".into();
            } else {
                suffix = format!(" -> exit {}: {}", rc, text.trim());
            }
        }
        writeln!(out, "{}{}", cmd.join(" "), suffix)?;
        if rc != 0 && !tolerated {
            return Err(anyhow!(
                "could not load the {} service; the file is written, load it by hand: {}",
                u.name,
                cmd.join(" ")
            ));
        }
    }
    Ok(())
}

pub fn install_service(
    out: &mut dyn std::io::Write,
    hive_home: &str,
    o: ServiceInstallOptions,
) -> anyhow::Result<ServiceInstall> {
    let mut o = o;
    let hive_home = if hive_home.is_empty() {
        crate::util::home_dir()
    } else {
        hive_home.to_string()
    };
    if o.interval <= 0 {
        o.interval = crate::daemon::DEFAULT_DAEMON_INTERVAL_SECONDS;
    }
    if o.hub_port <= 0 {
        o.hub_port = HUB_PORT;
    }
    let mut buf = Vec::new();
    let mut w = Tee { out, buf: &mut buf };
    let (kind, daemon_path) = service_paths(String::new(), String::new());
    let mut result = ServiceInstall {
        kind,
        daemon_path,
        ..Default::default()
    };
    // Said once, for both units: what was added to this shell's PATH.
    let (_, added) = path_for_services();
    if !added.is_empty() {
        use std::io::Write as _;
        writeln!(
            w,
            "PATH for the services: this shell's PATH, then {}, which exist and are not on it",
            added.join(", ")
        )?;
    }
    if !o.dry_run {
        // Sign before the units load, so the first daemon to run is the one whose folder answer sticks.
        crate::sign::ensure_signed(&mut w, &hive_binary(), hive_cli::managed::this_program());
        // Units from before the rename (pulse.daemon, pulse.hub) would keep a
        // second daemon leasing jobs beside this one: unload and remove them.
        retire_legacy_units(&mut w, String::new(), String::new());
    }
    for u in service_units(o.interval, o.hub_port, o.no_hub) {
        let (path, load) = prepare_unit(
            &mut w,
            u.clone(),
            hive_home.clone(),
            String::new(),
            String::new(),
            o.dry_run,
        )?;
        if u.name == "hub" {
            result.hub_path = path;
        }
        let Some(load) = load else {
            continue;
        };
        if u.name == "hub" && o.defer_hub {
            // The deferred load runs after `output` was taken; its only caller (the console's
            // service route) has no writer for it, so the lines land in a buffer nobody reads.
            result.load_hub = Some(Arc::new(move || {
                let mut sink = Vec::new();
                load(&mut sink)
            }));
            continue;
        }
        load(&mut w)?;
    }
    result.output = String::from_utf8_lossy(&buf).to_string();
    Ok(result)
}

pub fn service_status(hive_home: String) -> ServiceState {
    let hive_home = if hive_home.is_empty() {
        crate::util::home_dir()
    } else {
        hive_home
    };
    let (kind, _) = service_paths(String::new(), String::new());
    let mut state = ServiceState {
        kind: kind.clone(),
        supported: matches!(hive_gitenv::GOOS, "darwin" | "macos" | "linux"),
        binary: launch_binary(),
        logs: Path::new(&hive_home).join("logs").display().to_string(),
        ..Default::default()
    };
    for u in service_units(0, 0, false) {
        let (_, path) = unit_paths(u.clone(), String::new(), String::new());
        let mut unit = UnitState {
            name: u.name.clone(),
            path: path.clone(),
            present: crate::util::file_exists(&path),
            loaded: false,
        };
        if unit.present {
            let status = unit_commands(u.clone(), kind.clone(), path, "status".into());
            unit.loaded = (RUN_SERVICE.read().expect("run service lock"))(status[0].clone()).0 == 0;
        }
        if u.name == "daemon" {
            state.installed = unit.present && unit.loaded;
        }
        state.units.push(unit);
    }
    state
}

pub(crate) fn same_file(path: String, text: String) -> bool {
    std::fs::read_to_string(path).is_ok_and(|data| data == text)
}

pub fn uninstall_service(out: &mut dyn std::io::Write, system: &str, user_home: &str) {
    let _ = uninstall_service_inner(out, system.to_string(), user_home.to_string());
}

/// The service units an install from before the rename left behind, under
/// their old labels (`pulse.daemon`, `pulse.hub`): the same two units with
/// `pulse` for `hive` in label, unit file and launcher name.
pub(crate) fn legacy_units() -> Vec<ServiceUnit> {
    service_units(0, 0, false)
        .into_iter()
        .map(|mut u| {
            u.label = u.label.replace("hive", hive_store::LEGACY_NAME);
            u.unit = u.unit.replace("hive", hive_store::LEGACY_NAME);
            u.launcher = u.launcher.replace("hive", hive_store::LEGACY_NAME);
            u
        })
        .collect()
}

/// Unloads and removes the legacy units (`legacy_units`) that are present,
/// with their launcher scripts, and names each on `out`. Returns how many
/// unit files it removed. Nothing to do is silent.
pub(crate) fn retire_legacy_units(
    out: &mut dyn std::io::Write,
    system: String,
    user_home: String,
) -> usize {
    let mut removed = 0;
    for u in legacy_units() {
        let (kind, path) = unit_paths(u.clone(), system.clone(), user_home.clone());
        if !crate::util::file_exists(&path) {
            continue;
        }
        let _ = writeln!(
            out,
            "retiring the {} service from before the rename ({})",
            u.label, path
        );
        for cmd in unit_commands(u, kind, path.clone(), "uninstall".into()) {
            let (rc, text) = (RUN_SERVICE.read().expect("run service lock"))(cmd.clone());
            let suffix = if rc != 0 {
                format!(" -> exit {}: {}", rc, text.trim())
            } else {
                String::new()
            };
            let _ = writeln!(out, "{}{}", cmd.join(" "), suffix);
        }
        if let Ok(data) = std::fs::read_to_string(&path) {
            let launcher = launcher_from_plist(data);
            if !launcher.is_empty() && crate::util::file_exists(&launcher) {
                let _ = std::fs::remove_file(&launcher);
                let _ = writeln!(out, "removed {launcher}");
            }
        }
        let _ = std::fs::remove_file(&path);
        let _ = writeln!(out, "removed {path}");
        removed += 1;
    }
    removed
}

pub(crate) fn uninstall_service_inner(
    out: &mut dyn std::io::Write,
    system: String,
    user_home: String,
) -> String {
    let mut daemon_path = String::new();
    for u in service_units(0, 0, false) {
        let (kind, path) = unit_paths(u.clone(), system.clone(), user_home.clone());
        if u.name == "daemon" {
            daemon_path = path.clone();
        }
        if !crate::util::file_exists(&path) {
            continue;
        }
        for cmd in unit_commands(u, kind, path.clone(), "uninstall".into()) {
            let (rc, text) = (RUN_SERVICE.read().expect("run service lock"))(cmd.clone());
            let suffix = if rc != 0 {
                format!(" -> exit {}: {}", rc, text.trim())
            } else {
                String::new()
            };
            let _ = writeln!(out, "{}{}", cmd.join(" "), suffix);
        }
        if let Ok(data) = std::fs::read_to_string(&path) {
            let launcher = launcher_from_plist(data);
            if !launcher.is_empty() && crate::util::file_exists(&launcher) {
                let _ = std::fs::remove_file(&launcher);
                let _ = writeln!(out, "removed {launcher}");
            }
        }
        let _ = std::fs::remove_file(&path);
        let _ = writeln!(out, "removed {path}");
    }
    daemon_path
}

pub(crate) fn service_status_inner(
    out: &mut dyn std::io::Write,
    s: Option<&hive_store::Store>,
    system: &str,
    user_home: &str,
) {
    for u in service_units(0, 0, false) {
        let (kind, path) = unit_paths(u.clone(), system.to_string(), user_home.to_string());
        let state = if crate::util::file_exists(&path) {
            "present"
        } else {
            "absent"
        };
        let _ = writeln!(out, "{} service file {}: {}", u.name, path, state);
        if state == "absent" {
            continue;
        }
        for cmd in unit_commands(u, kind, path, "status".into()) {
            let (_, text) = (RUN_SERVICE.read().expect("run service lock"))(cmd);
            let mut lines = Vec::new();
            for l in text.lines() {
                if [
                    "state",
                    "pid",
                    "Active",
                    "Main PID",
                    "could not",
                    "not find",
                ]
                .iter()
                .any(|k| l.contains(k))
                {
                    lines.push(l.to_string());
                }
            }
            if lines.len() > 6 {
                lines.truncate(6);
            }
            if lines.is_empty() {
                let _ = writeln!(out, "{}", crate::util::trunc(text.trim(), 400));
            } else {
                let _ = writeln!(out, "{}", lines.join("\n"));
            }
        }
    }
    if let Some(s) = s {
        let _ = writeln!(
            out,
            "daemon     {}",
            crate::onboard::daemon_status(s, s.now())
        );
    }
}

pub(crate) fn protected_folders_note(
    s: &hive_store::Store,
    system: String,
    user_home: String,
) -> String {
    let system = if system.is_empty() {
        hive_gitenv::GOOS.to_string()
    } else {
        system
    };
    if system != "darwin" && system != "macos" {
        return String::new();
    }
    let mut user_home = if user_home.is_empty() {
        crate::util::home_dir()
    } else {
        user_home
    };
    if let Ok(rp) = std::fs::canonicalize(&user_home) {
        user_home = rp.display().to_string();
    }
    let mut hits = Vec::new();
    let conn = match s.conn() {
        Ok(conn) => conn,
        Err(_) => return String::new(),
    };
    for p in crate::board::project_rows(&conn, false) {
        let mut path = p.str("path");
        if let Ok(rp) = std::fs::canonicalize(&path) {
            path = rp.display().to_string();
        }
        for folder in PROTECTED_FOLDERS {
            let guarded = Path::new(&user_home).join(folder);
            if path == guarded.display().to_string()
                || path.starts_with(&(guarded.join("").display().to_string()))
                || path.starts_with(&format!("{}/", guarded.display()))
            {
                hits.push(format!("{} ({})", p.str("slug"), path));
                break;
            }
        }
    }
    if hits.is_empty() {
        return String::new();
    }
    format!(
        "macOS: {} live under a folder macOS guards ({}).\n  The first time the daemon touches them a dialog asks whether hive may access that folder: answer Allow once, or the\n  loop waits on it (hive status then says the daemon is silent). macOS ties the answer to the binary's signing\n  requirement, so hive signs every build as `hive` and the answer holds across rebuilds; a build that skipped the\n  signing step asks again. Checkouts outside those folders never ask.",
        hits.join(", "),
        PROTECTED_FOLDERS.join("/")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{LazyLock, Mutex};

    static SERVICE_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    struct SigningSystemGuard;

    impl Drop for SigningSystemGuard {
        fn drop(&mut self) {
            *crate::sign::SIGNING_SYSTEM.write().unwrap() = String::new();
        }
    }

    #[test]
    fn a_systemd_unit_keeps_an_environment_value_with_a_space_whole() {
        // Under WSL the Windows PATH follows the Linux one, spaces and all.
        let unit = service_file_text(
            "systemd".into(),
            60,
            "/opt/hive".into(),
            "/h/.hive/logs/daemon.log".into(),
            "/usr/bin:/mnt/c/Program Files/Git/cmd:/mnt/c/100%/bin".into(),
            "/h/.hive".into(),
        );
        assert!(
            unit.contains(
                "\nEnvironment=\"PATH=/usr/bin:/mnt/c/Program Files/Git/cmd:/mnt/c/100%%/bin\"\n"
            ),
            "{unit}"
        );
        // A value systemd reads whole as it stands is written as before.
        assert!(
            unit.contains("\nEnvironment=HIVE_HOME=/h/.hive\n"),
            "{unit}"
        );
        assert_eq!(
            systemd_env_line("K", r#"a "b" \c"#),
            r#"Environment="K=a \"b\" \\c""#
        );
        assert_eq!(systemd_env_line("K", "it's"), r#"Environment="K=it's""#);
        // A line end would cut the line short, so it is written as an escape.
        assert_eq!(
            systemd_env_line("K", "/usr/bin\r"),
            r#"Environment="K=/usr/bin\r""#
        );
        assert_eq!(systemd_env_line("K", "a\nb"), r#"Environment="K=a\nb""#);
    }

    #[test]
    fn install_service_and_service_status() {
        let _serial = crate::test_helpers::serial();
        let _guard = SERVICE_TEST_LOCK.lock().unwrap();
        let user_home = tempfile::tempdir().unwrap();
        let _env_1 = hive_testutil::setenv("HOME", user_home.path());
        let hive_home = tempfile::tempdir().unwrap();
        let hive_home_s = hive_home.path().display().to_string();
        *crate::sign::SIGNING_SYSTEM.write().unwrap() = "linux".into();
        let _signing = SigningSystemGuard;
        let old = set_run_service(Box::new(|_| (0, String::new())));
        let calls = std::sync::Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let loaded =
            std::sync::Arc::new(Mutex::new(std::collections::BTreeMap::<String, bool>::new()));
        let which = |argv: &[String]| -> String {
            if argv.join(" ").contains("hub") {
                return "hub".into();
            }
            "daemon".into()
        };
        // The verb of a service command, wherever it stands: `bootstrap` in
        // `launchctl bootstrap ...`, `enable` in `systemctl --user enable
        // --now ...`.
        fn verb(argv: &[String]) -> &str {
            argv.iter()
                .skip(1)
                .find(|a| !a.starts_with('-'))
                .map(String::as_str)
                .unwrap_or("")
        }
        let _ = set_run_service(Box::new({
            let calls = calls.clone();
            let loaded = loaded.clone();
            move |argv| {
                calls.lock().unwrap().push(argv.clone());
                if argv.len() < 2 {
                    return (0, String::new());
                }
                match verb(&argv) {
                    // launchctl print / systemctl status: exit 0 once the unit's load ran
                    "print" | "status" => {
                        if loaded
                            .lock()
                            .unwrap()
                            .get(&which(&argv))
                            .copied()
                            .unwrap_or(false)
                        {
                            return (0, String::new());
                        }
                        return (113, "not loaded".into());
                    }
                    "bootstrap" | "enable" => {
                        loaded.lock().unwrap().insert(which(&argv), true);
                    }
                    _ => {}
                }
                (0, String::new())
            }
        }));
        let (kind, daemon_path) = service_paths(String::new(), String::new());
        let (_, hub_path) = unit_paths(hub_unit(0), String::new(), String::new());

        let before = service_status(hive_home_s.clone());
        if before.kind != kind
            || !before.supported
            || before.installed
            || before.units.len() != 2
            || before.units[0].name != "daemon"
            || before.units[0].path != daemon_path
            || before.units[0].present
            || before.units[1].name != "hub"
            || before.units[1].path != hub_path
            || before.logs != hive_home.path().join("logs").display().to_string()
            || !calls.lock().unwrap().is_empty()
        {
            panic!(
                "status before install: {before:?} (calls {:?})",
                calls.lock().unwrap()
            );
        }

        let mut out = Vec::new();
        let dry = install_service(
            &mut out,
            &hive_home_s,
            ServiceInstallOptions {
                dry_run: true,
                ..Default::default()
            },
        );
        let dry = match dry {
            Ok(dry)
                if dry.kind == kind
                    && dry.daemon_path == daemon_path
                    && dry.hub_path == hub_path
                    && dry.load_hub.is_none() =>
            {
                dry
            }
            Ok(dry) => panic!("dry run: {:?} {:?}", dry.output, dry.hub_path),
            Err(err) => panic!("dry run: {err}"),
        };
        if crate::util::file_exists(&daemon_path)
            || crate::util::file_exists(&hub_path)
            || !calls.lock().unwrap().is_empty()
            || dry.output != String::from_utf8_lossy(&out)
            || !dry.output.contains("--port 8765")
            || !dry.output.contains(&format!(
                "--interval {} &",
                crate::daemon::DEFAULT_DAEMON_INTERVAL_SECONDS
            ))
        {
            panic!(
                "dry run wrote or loaded something: {:?}\n{}",
                calls.lock().unwrap(),
                dry.output
            );
        }

        // A foreground hub installs with the hub's load deferred: the daemon loads, the hub unit is written, its load waits.
        out.clear();
        let deferred = install_service(
            &mut std::io::sink(),
            &hive_home_s,
            ServiceInstallOptions {
                interval: 45,
                hub_port: 8765,
                defer_hub: true,
                ..Default::default()
            },
        );
        let deferred = match deferred {
            Ok(deferred)
                if deferred.load_hub.is_some()
                    && crate::util::file_exists(&daemon_path)
                    && crate::util::file_exists(&hub_path) =>
            {
                deferred
            }
            Ok(deferred) => panic!("deferred install: {:?}", deferred.output),
            Err(err) => panic!("deferred install: {err}"),
        };
        if !loaded
            .lock()
            .unwrap()
            .get("daemon")
            .copied()
            .unwrap_or(false)
            || loaded.lock().unwrap().get("hub").copied().unwrap_or(false)
        {
            panic!(
                "deferred install loaded the wrong units: {:?}",
                loaded.lock().unwrap()
            );
        }
        let launcher = std::fs::read_to_string(launcher_path(hive_home_s.clone()));
        if launcher.is_err() || !launcher.as_ref().unwrap().contains("--interval 45 &") {
            panic!("daemon launcher: {launcher:?}");
        }
        if !out.is_empty() || !deferred.output.contains("hub launcher: ") {
            panic!("deferred output:\n{}", deferred.output);
        }
        let mid = service_status(hive_home_s.clone());
        if !mid.installed || !mid.units[0].loaded || !mid.units[1].present || mid.units[1].loaded {
            panic!("status between: {mid:?}");
        }
        let load_hub = deferred.load_hub.as_ref().unwrap();
        if let Err(err) = load_hub() {
            panic!("LoadHub: {err} {:?}", loaded.lock().unwrap());
        }
        if !loaded.lock().unwrap().get("hub").copied().unwrap_or(false) {
            panic!("LoadHub: {:?}", loaded.lock().unwrap());
        }
        let after = service_status(hive_home_s.clone());
        if !after.installed || !after.units[1].loaded {
            panic!("status after: {after:?}");
        }

        // Installing again with the same files is a no-op that leaves both running.
        calls.lock().unwrap().clear();
        let again = install_service(
            &mut std::io::sink(),
            &hive_home_s,
            ServiceInstallOptions {
                interval: 45,
                ..Default::default()
            },
        );
        let again = match again {
            Ok(again)
                if again.load_hub.is_none() && again.output.contains("unchanged and loaded") =>
            {
                again
            }
            Ok(again) => panic!("second install: {:?}", again.output),
            Err(err) => panic!("second install: {err}"),
        };
        let _ = again;
        for argv in calls.lock().unwrap().iter() {
            if matches!(verb(argv), "bootstrap" | "bootout" | "enable" | "restart") {
                panic!(
                    "an unchanged install reloaded a unit: {:?}",
                    calls.lock().unwrap()
                );
            }
        }
        let data = std::fs::read_to_string(&hub_path);
        if data.is_err() || !data.as_ref().unwrap().contains("hive-hub.sh") {
            panic!("hub unit: {data:?}");
        }
        let _ = set_run_service(old);
    }

    /// A service keeps the PATH it was installed with. The caller's PATH
    /// stays whole and first; the usual tool directories that exist and are
    /// missing from it follow, each once.
    #[test]
    fn a_service_path_keeps_the_callers_and_adds_the_usual_directories_it_lacks() {
        let usual = usual_tool_dirs("/Users/me", "/Users/me/.hive/bin");
        assert_eq!(
            usual,
            [
                "/Users/me/.hive/bin",
                "/Users/me/.local/bin",
                "/Users/me/.cargo/bin",
                "/opt/homebrew/bin",
                "/home/linuxbrew/.linuxbrew/bin",
                "/usr/local/bin",
                "/snap/bin",
            ]
        );
        // What this Mac has: no linuxbrew and no snap.
        let on_disk = |dir: &str| {
            [
                "/Users/me/.hive/bin",
                "/Users/me/.local/bin",
                "/Users/me/.cargo/bin",
                "/opt/homebrew/bin",
                "/usr/local/bin",
            ]
            .contains(&dir)
        };

        // The PATH a script, an ssh session or an app has: the one that
        // left a daemon without gh, op, az, mise and cargo.
        let bare = "/Users/me/.hive/bin:/usr/bin:/bin:/usr/sbin:/sbin";
        let (path, added) = service_path(bare, &usual, &on_disk);
        assert_eq!(
            path,
            "/Users/me/.hive/bin:/usr/bin:/bin:/usr/sbin:/sbin:/Users/me/.local/bin:/Users/me/.cargo/bin:/opt/homebrew/bin:/usr/local/bin"
        );
        assert_eq!(
            added,
            [
                "/Users/me/.local/bin",
                "/Users/me/.cargo/bin",
                "/opt/homebrew/bin",
                "/usr/local/bin"
            ]
        );

        // A shell that names them all, in an order of its own and one with
        // a slash at its end: the PATH is the caller's, to the character.
        let full = "/opt/homebrew/bin/:/Users/me/.cargo/bin:/usr/local/bin:/usr/bin:/bin:/Users/me/.local/bin:/Users/me/.hive/bin";
        assert_eq!(
            service_path(full, &usual, &on_disk),
            (full.to_string(), vec![])
        );

        // Run again on its own result, nothing more is added.
        assert_eq!(
            service_path(&path, &usual, &on_disk),
            (path.clone(), vec![])
        );

        // No PATH at all, and a machine with none of the directories.
        assert_eq!(
            service_path("", &usual, &on_disk).0,
            "/Users/me/.hive/bin:/Users/me/.local/bin:/Users/me/.cargo/bin:/opt/homebrew/bin:/usr/local/bin"
        );
        assert_eq!(
            service_path(bare, &usual, &|_| false),
            (bare.to_string(), vec![])
        );
        // No home and no binary to name: only the system-wide ones are tried.
        assert_eq!(
            usual_tool_dirs("", ""),
            [
                "/opt/homebrew/bin",
                "/home/linuxbrew/.linuxbrew/bin",
                "/usr/local/bin",
                "/snap/bin",
            ]
        );
    }

    #[test]
    fn legacy_units_are_retired_by_name() {
        let _serial = crate::test_helpers::serial();
        let _guard = SERVICE_TEST_LOCK.lock().unwrap();
        let user_home = tempfile::tempdir().unwrap();
        let home = user_home.path().display().to_string();
        let legacy = legacy_units();
        assert_eq!(legacy.len(), 2);
        assert_eq!(legacy[0].label, "pulse.daemon");
        assert_eq!(legacy[0].unit, "pulse-daemon");
        assert_eq!(legacy[0].launcher, "pulse-daemon.sh");
        assert_eq!(legacy[1].label, "pulse.hub");

        // Nothing present: silent, nothing removed.
        let mut out = Vec::new();
        assert_eq!(
            retire_legacy_units(&mut out, "linux".into(), home.clone()),
            0
        );
        assert!(out.is_empty(), "{:?}", String::from_utf8_lossy(&out));

        // A legacy daemon unit with its launcher: both go, the unit is unloaded.
        let (_, path) = unit_paths(legacy[0].clone(), "linux".into(), home.clone());
        std::fs::create_dir_all(Path::new(&path).parent().unwrap()).unwrap();
        let launcher = user_home.path().join("pulse-daemon.sh");
        std::fs::write(&launcher, "#!/bin/sh\n").unwrap();
        std::fs::write(
            &path,
            format!("[Service]\nExecStart=/bin/sh {}\n", launcher.display()),
        )
        .unwrap();
        let calls = std::sync::Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let old = set_run_service(Box::new({
            let calls = calls.clone();
            move |argv| {
                calls.lock().unwrap().push(argv);
                (0, String::new())
            }
        }));
        let mut out = Vec::new();
        let removed = retire_legacy_units(&mut out, "linux".into(), home.clone());
        let _ = set_run_service(old);
        let text = String::from_utf8_lossy(&out);
        assert_eq!(removed, 1, "{text}");
        assert!(!Path::new(&path).exists(), "{text}");
        assert!(!launcher.exists(), "{text}");
        assert!(text.contains("retiring the pulse.daemon service"), "{text}");
        let calls = calls.lock().unwrap();
        assert!(
            calls
                .iter()
                .any(|c| c.iter().any(|a| a.contains("pulse-daemon"))),
            "{calls:?}"
        );
        // The current units are untouched.
        let (_, current) = unit_paths(daemon_unit(0), "linux".into(), home);
        assert!(!Path::new(&current).exists());
    }
}
