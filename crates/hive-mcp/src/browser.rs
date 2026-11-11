//! The browser behind the `playwright` tool connection.
//!
//! `@playwright/mcp` wants Google Chrome, opens it with a window, keeps a
//! profile per working directory on disk, and writes its snapshots and
//! screenshots into `.playwright-mcp/` under the working directory. None of
//! that suits an agent: the machine may have Edge and no Chrome, a window
//! is a distraction on the owner's screen, a profile is shared between two
//! agents in the same clone, and a screenshot in the clone ends up in a
//! commit or nowhere the owner looks.
//!
//! So when hive starts a stdio server that is Playwright's, it sets the
//! environment the server reads its configuration from
//! (`PLAYWRIGHT_MCP_*`): headless, isolated, the output directory the
//! caller names (the conversation's artifacts folder, which the hub shows
//! beside the chat; a job's own temporary directory), absolute file paths
//! in the tool results, and the first browser this machine has - Chrome,
//! Edge, Chromium, Brave, or Playwright's own download. A variable the
//! registration sets itself (`--env PLAYWRIGHT_MCP_BROWSER=firefox`) wins
//! over every one of these.

use hive_store::McpServer;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The environment variable hive itself reads for a default output
/// folder: a conversation's artifacts folder (the harness sets it for the
/// agent's commands, so `hive mcp call` in a conversation lands its
/// screenshots there too).
pub const ARTIFACTS_ENV: &str = "HIVE_ARTIFACTS";

/// What the caller knows about where the server runs: nothing of it is
/// required, each part refines the defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Workspace {
    /// The working directory of the server process; Playwright treats it as
    /// the workspace root an explicitly named file is resolved against.
    pub cwd: Option<PathBuf>,
    /// Where automatically named output files (screenshots, snapshots,
    /// traces) go.
    pub output_dir: Option<PathBuf>,
}

impl Workspace {
    /// The workspace this process's own environment describes: see
    /// [`Workspace::from_vars`].
    pub fn from_env() -> Self {
        Self::from_vars(|k| std::env::var(k).ok())
    }

    /// The workspace an environment describes, `var` answering each
    /// variable: one folder that is both where a stdio server runs and
    /// where a browser's output goes. The artifacts folder it names
    /// (`HIVE_ARTIFACTS`: a conversation's, shown in the hub); for a job
    /// (`HIVE_JOB` set) without one, the run's own `TMPDIR`, which sits
    /// beside the clone and goes with it. Never the checkout: screenshots
    /// would pile up there as `.playwright-mcp/`, and the harness ends
    /// whatever a command leaves running under a job's clone, which took
    /// the servers themselves (the "closed its stdin: Broken pipe" of
    /// every later tool call). Anywhere else nothing is set: the server
    /// inherits the caller's directory and Playwright's defaults stand.
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Self {
        let non_empty = |k: &str| var(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let dir = non_empty(ARTIFACTS_ENV).or_else(|| {
            var("HIVE_JOB")
                .filter(|v| !v.is_empty())
                .and_then(|_| non_empty("TMPDIR"))
        });
        Self {
            cwd: dir.clone(),
            output_dir: dir,
        }
    }
}

/// A browser this machine has, as Playwright names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Browser {
    /// What `--browser` takes: a channel (`chrome`, `msedge`, `chromium`)
    /// or a browser name.
    pub channel: &'static str,
    /// The binary found, or empty for Playwright's own download.
    pub path: PathBuf,
}

impl Browser {
    pub fn describe(&self) -> String {
        if self.path.as_os_str().is_empty() {
            format!("{} (Playwright's own download)", self.channel)
        } else {
            format!("{} at {}", self.channel, self.path.display())
        }
    }
}

/// Is this stdio server Microsoft's Playwright MCP (`npx @playwright/mcp`,
/// a global `playwright-mcp`, or `playwright` itself in MCP mode)?
pub fn is_playwright(server: &McpServer) -> bool {
    if server.transport != "stdio" {
        return false;
    }
    std::iter::once(server.command.as_str())
        .chain(server.args.iter().map(String::as_str))
        .any(|piece| {
            let p = piece.trim();
            p.starts_with("@playwright/mcp")
                || p.contains("/@playwright/mcp/")
                || p == "playwright-mcp"
                || p.ends_with("/playwright-mcp")
        })
}

/// The browsers Playwright can drive by channel, in the order hive picks
/// them, each with where macOS, Linux and Windows keep it.
const CANDIDATES: &[(&str, &[&str])] = &[
    (
        "chrome",
        &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/opt/google/chrome/chrome",
            "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
            "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
        ],
    ),
    (
        "msedge",
        &[
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/usr/bin/microsoft-edge",
            "/usr/bin/microsoft-edge-stable",
            "/opt/microsoft/msedge/msedge",
            "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
            "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
        ],
    ),
    (
        "chrome-beta",
        &[
            "/Applications/Google Chrome Beta.app/Contents/MacOS/Google Chrome Beta",
            "/usr/bin/google-chrome-beta",
        ],
    ),
    (
        "chrome-canary",
        &["/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary"],
    ),
    (
        "msedge-beta",
        &[
            "/Applications/Microsoft Edge Beta.app/Contents/MacOS/Microsoft Edge Beta",
            "/usr/bin/microsoft-edge-beta",
        ],
    ),
    (
        "msedge-dev",
        &[
            "/Applications/Microsoft Edge Dev.app/Contents/MacOS/Microsoft Edge Dev",
            "/usr/bin/microsoft-edge-dev",
        ],
    ),
];

/// Chromium builds Playwright has no channel for: driven by executable path.
const BY_PATH: &[&str] = &[
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/snap/bin/chromium",
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    "/usr/bin/brave-browser",
    "/Applications/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
];

/// Where Playwright keeps the browsers it downloads itself.
fn playwright_cache_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = std::env::var_os("PLAYWRIGHT_BROWSERS_PATH").filter(|v| !v.is_empty()) {
        out.push(PathBuf::from(p));
    }
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        let home = PathBuf::from(home);
        out.push(home.join("Library/Caches/ms-playwright"));
        out.push(home.join(".cache/ms-playwright"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()) {
        out.push(PathBuf::from(local).join("ms-playwright"));
    }
    out
}

/// Does a Playwright cache hold a chromium build?
fn playwright_has_chromium(exists: &dyn Fn(&Path) -> bool) -> bool {
    playwright_cache_dirs().iter().any(|dir| {
        std::fs::read_dir(dir)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                (name.starts_with("chromium-") || name.starts_with("chromium_headless_shell-"))
                    && exists(&e.path())
            })
    })
}

/// The browser hive would hand Playwright on this machine, if any: the
/// first installed of the channels above, then a chromium by path, then
/// Playwright's own download.
pub fn detect() -> Option<Browser> {
    detect_with(&|p: &Path| p.exists())
}

pub(crate) fn detect_with(exists: &dyn Fn(&Path) -> bool) -> Option<Browser> {
    for (channel, paths) in CANDIDATES {
        if let Some(p) = paths.iter().map(Path::new).find(|p| exists(p)) {
            return Some(Browser {
                channel,
                path: p.to_path_buf(),
            });
        }
    }
    if let Some(p) = BY_PATH.iter().map(Path::new).find(|p| exists(p)) {
        return Some(Browser {
            channel: "chromium",
            path: p.to_path_buf(),
        });
    }
    if playwright_has_chromium(exists) {
        return Some(Browser {
            channel: "chromium",
            path: PathBuf::new(),
        });
    }
    None
}

/// The configuration hive sets for a Playwright server that the
/// registration does not: headless, isolated, the output folder, absolute
/// paths, the browser. Each as `(name, value)` for the child's environment.
/// Empty for any other server.
pub fn launch_env(server: &McpServer, ws: &Workspace) -> BTreeMap<String, String> {
    launch_env_with(server, ws, detect().as_ref())
}

pub(crate) fn launch_env_with(
    server: &McpServer,
    ws: &Workspace,
    browser: Option<&Browser>,
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    if !is_playwright(server) {
        return env;
    }
    let set = |env: &mut BTreeMap<String, String>, k: &str, v: String| {
        if !server.env.contains_key(k) && !server.args.iter().any(|a| flag_for(k) == a) {
            env.insert(k.to_string(), v);
        }
    };
    set(&mut env, "PLAYWRIGHT_MCP_HEADLESS", "1".into());
    set(&mut env, "PLAYWRIGHT_MCP_ISOLATED", "1".into());
    set(&mut env, "PLAYWRIGHT_MCP_FILE_PATHS", "absolute".into());
    if let Some(dir) = &ws.output_dir {
        set(
            &mut env,
            "PLAYWRIGHT_MCP_OUTPUT_DIR",
            dir.display().to_string(),
        );
    }
    let picks_browser = [
        "--browser",
        "--executable-path",
        "--cdp-endpoint",
        "--extension",
    ]
    .iter()
    .any(|f| {
        server
            .args
            .iter()
            .any(|a| a == f || a.starts_with(&format!("{f}=")))
    }) || [
        "PLAYWRIGHT_MCP_BROWSER",
        "PLAYWRIGHT_MCP_EXECUTABLE_PATH",
        "PLAYWRIGHT_MCP_CDP_ENDPOINT",
    ]
    .iter()
    .any(|k| server.env.contains_key(*k));
    if !picks_browser && let Some(b) = browser {
        if b.channel == "chromium" && !b.path.as_os_str().is_empty() {
            env.insert(
                "PLAYWRIGHT_MCP_EXECUTABLE_PATH".into(),
                b.path.display().to_string(),
            );
        } else {
            env.insert("PLAYWRIGHT_MCP_BROWSER".into(), b.channel.to_string());
        }
    }
    env
}

/// The command-line flag that sets the same thing as an environment
/// variable, so a registration that passes the flag is left alone.
fn flag_for(var: &str) -> &'static str {
    match var {
        "PLAYWRIGHT_MCP_HEADLESS" => "--headless",
        "PLAYWRIGHT_MCP_ISOLATED" => "--isolated",
        "PLAYWRIGHT_MCP_FILE_PATHS" => "--file-paths",
        "PLAYWRIGHT_MCP_OUTPUT_DIR" => "--output-dir",
        _ => "",
    }
}

/// One line for `hive mcp show` and the run's start note: what hive adds
/// when it starts this server, or nothing for a server it leaves alone.
pub fn describe(server: &McpServer, ws: &Workspace) -> Option<String> {
    if !is_playwright(server) {
        return None;
    }
    let browser = detect();
    let env = launch_env_with(server, ws, browser.as_ref());
    let mut parts = Vec::new();
    if env.contains_key("PLAYWRIGHT_MCP_HEADLESS") {
        parts.push("headless".to_string());
    }
    if env.contains_key("PLAYWRIGHT_MCP_ISOLATED") {
        parts.push("a fresh profile each start".to_string());
    }
    match &browser {
        Some(b) if env.contains_key("PLAYWRIGHT_MCP_BROWSER")
            || env.contains_key("PLAYWRIGHT_MCP_EXECUTABLE_PATH") =>
        {
            parts.push(format!("browser {}", b.describe()));
        }
        Some(_) => {}
        None => parts.push(
            "no browser found on this machine (install Chrome or Edge, or run `npx playwright install chromium`)"
                .to_string(),
        ),
    }
    if let Some(dir) = env.get("PLAYWRIGHT_MCP_OUTPUT_DIR") {
        parts.push(format!("screenshots and snapshots in {dir}"));
    } else if ws.output_dir.is_none() {
        parts.push(
            "screenshots and snapshots in .playwright-mcp/ under the working directory (a conversation's go to its artifacts folder, a job's to its own TMPDIR)"
                .to_string(),
        );
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!("hive starts it {}", parts.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playwright() -> McpServer {
        McpServer {
            name: "playwright".into(),
            transport: "stdio".into(),
            command: "npx".into(),
            args: vec!["-y".into(), "@playwright/mcp@latest".into()],
            ..Default::default()
        }
    }

    fn edge() -> Browser {
        Browser {
            channel: "msedge",
            path: PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
        }
    }

    #[test]
    fn playwright_is_recognised_by_its_package_or_binary_only() {
        assert!(is_playwright(&playwright()));
        assert!(is_playwright(&McpServer {
            command: "playwright-mcp".into(),
            args: vec![],
            ..playwright()
        }));
        assert!(is_playwright(&McpServer {
            command: "/usr/local/bin/playwright-mcp".into(),
            args: vec!["--caps".into(), "vision".into()],
            ..playwright()
        }));
        assert!(is_playwright(&McpServer {
            command: "node".into(),
            args: vec!["/opt/node_modules/@playwright/mcp/cli.js".into()],
            ..playwright()
        }));
        assert!(!is_playwright(&McpServer {
            args: vec!["-y".into(), "@azure/mcp@latest".into()],
            ..playwright()
        }));
        assert!(!is_playwright(&McpServer {
            transport: "http".into(),
            url: "https://playwright.example/mcp".into(),
            ..playwright()
        }));
        // The chrome-devtools server is a browser too, but not this one.
        assert!(!is_playwright(&McpServer {
            args: vec!["-y".into(), "chrome-devtools-mcp@latest".into()],
            ..playwright()
        }));
    }

    #[test]
    fn a_playwright_server_gets_headless_isolated_and_the_output_dir() {
        let ws = Workspace {
            cwd: Some(PathBuf::from("/work/clone")),
            output_dir: Some(PathBuf::from("/home/u/.hive/work/chat/c1/artifacts")),
        };
        let env = launch_env_with(&playwright(), &ws, Some(&edge()));
        assert_eq!(env["PLAYWRIGHT_MCP_HEADLESS"], "1");
        assert_eq!(env["PLAYWRIGHT_MCP_ISOLATED"], "1");
        assert_eq!(env["PLAYWRIGHT_MCP_FILE_PATHS"], "absolute");
        assert_eq!(
            env["PLAYWRIGHT_MCP_OUTPUT_DIR"],
            "/home/u/.hive/work/chat/c1/artifacts"
        );
        assert_eq!(env["PLAYWRIGHT_MCP_BROWSER"], "msedge");
        assert!(!env.contains_key("PLAYWRIGHT_MCP_EXECUTABLE_PATH"));

        // No output folder known: Playwright's own default stays.
        let env = launch_env_with(&playwright(), &Workspace::default(), Some(&edge()));
        assert!(!env.contains_key("PLAYWRIGHT_MCP_OUTPUT_DIR"));
        assert_eq!(env["PLAYWRIGHT_MCP_HEADLESS"], "1");

        // No browser found: nothing is forced, Playwright says what is missing.
        let env = launch_env_with(&playwright(), &ws, None);
        assert!(!env.contains_key("PLAYWRIGHT_MCP_BROWSER"));
        assert!(!env.contains_key("PLAYWRIGHT_MCP_EXECUTABLE_PATH"));
        assert_eq!(env["PLAYWRIGHT_MCP_HEADLESS"], "1");
    }

    #[test]
    fn a_chromium_without_a_channel_goes_by_executable_path() {
        let brave = Browser {
            channel: "chromium",
            path: PathBuf::from("/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"),
        };
        let env = launch_env_with(&playwright(), &Workspace::default(), Some(&brave));
        assert_eq!(
            env["PLAYWRIGHT_MCP_EXECUTABLE_PATH"],
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser"
        );
        assert!(!env.contains_key("PLAYWRIGHT_MCP_BROWSER"));
        // Playwright's own download: the chromium channel, no path.
        let own = Browser {
            channel: "chromium",
            path: PathBuf::new(),
        };
        let env = launch_env_with(&playwright(), &Workspace::default(), Some(&own));
        assert_eq!(env["PLAYWRIGHT_MCP_BROWSER"], "chromium");
        assert_eq!(own.describe(), "chromium (Playwright's own download)");
        assert!(
            brave
                .describe()
                .starts_with("chromium at /Applications/Brave")
        );
    }

    #[test]
    fn the_registration_wins_over_every_default() {
        let mut s = playwright();
        s.env.insert("PLAYWRIGHT_MCP_HEADLESS".into(), "0".into());
        s.env
            .insert("PLAYWRIGHT_MCP_BROWSER".into(), "firefox".into());
        s.args.push("--output-dir".into());
        s.args.push("/srv/shots".into());
        let ws = Workspace {
            cwd: None,
            output_dir: Some(PathBuf::from("/art")),
        };
        let env = launch_env_with(&s, &ws, Some(&edge()));
        assert!(!env.contains_key("PLAYWRIGHT_MCP_HEADLESS"), "{env:?}");
        assert!(!env.contains_key("PLAYWRIGHT_MCP_BROWSER"), "{env:?}");
        assert!(!env.contains_key("PLAYWRIGHT_MCP_OUTPUT_DIR"), "{env:?}");
        assert_eq!(env["PLAYWRIGHT_MCP_ISOLATED"], "1");

        // --browser as a flag, --headless as a flag, --executable-path=... too.
        let flagged = McpServer {
            args: vec![
                "-y".into(),
                "@playwright/mcp@latest".into(),
                "--headless".into(),
                "--browser".into(),
                "webkit".into(),
            ],
            ..playwright()
        };
        let env = launch_env_with(&flagged, &ws, Some(&edge()));
        assert!(!env.contains_key("PLAYWRIGHT_MCP_HEADLESS"), "{env:?}");
        assert!(!env.contains_key("PLAYWRIGHT_MCP_BROWSER"), "{env:?}");
        assert_eq!(env["PLAYWRIGHT_MCP_OUTPUT_DIR"], "/art");
        let by_path = McpServer {
            args: vec![
                "--executable-path=/x/chrome".into(),
                "@playwright/mcp".into(),
            ],
            ..playwright()
        };
        let env = launch_env_with(&by_path, &ws, Some(&edge()));
        assert!(!env.contains_key("PLAYWRIGHT_MCP_BROWSER"), "{env:?}");
        assert!(
            !env.contains_key("PLAYWRIGHT_MCP_EXECUTABLE_PATH"),
            "{env:?}"
        );
    }

    #[test]
    fn other_servers_are_left_alone() {
        let azure = McpServer {
            name: "azure-mcp".into(),
            args: vec![
                "-y".into(),
                "@azure/mcp@latest".into(),
                "server".into(),
                "start".into(),
            ],
            ..playwright()
        };
        let ws = Workspace {
            cwd: None,
            output_dir: Some(PathBuf::from("/art")),
        };
        assert!(launch_env_with(&azure, &ws, Some(&edge())).is_empty());
        assert_eq!(describe(&azure, &ws), None);
    }

    #[test]
    fn detection_prefers_chrome_then_edge_then_a_chromium_by_path() {
        let only = |want: &'static str| move |p: &Path| p.to_string_lossy().contains(want);
        let b = detect_with(&only("Microsoft Edge.app")).unwrap();
        assert_eq!(b.channel, "msedge");
        assert!(b.path.ends_with("Microsoft Edge"));
        let b = detect_with(&|p: &Path| {
            let s = p.to_string_lossy();
            s.contains("Microsoft Edge.app") || s.contains("Google Chrome.app")
        })
        .unwrap();
        assert_eq!(b.channel, "chrome");
        let b = detect_with(&only("Brave Browser")).unwrap();
        assert_eq!(b.channel, "chromium");
        assert!(b.path.ends_with("Brave Browser"));
        assert_eq!(detect_with(&|_: &Path| false), None);
    }

    #[test]
    fn describe_names_what_hive_adds() {
        let ws = Workspace {
            cwd: None,
            output_dir: Some(PathBuf::from("/art")),
        };
        let text = describe(&playwright(), &ws).unwrap();
        assert!(
            text.starts_with("hive starts it headless, a fresh profile each start"),
            "{text}"
        );
        assert!(
            text.ends_with("screenshots and snapshots in /art"),
            "{text}"
        );
        let text = describe(&playwright(), &Workspace::default()).unwrap();
        assert!(
            text.contains(".playwright-mcp/ under the working directory"),
            "{text}"
        );
    }

    #[test]
    fn the_workspace_reads_the_artifacts_folder_from_the_environment() {
        let _guard = hive_testutil::env_lock();
        let before = std::env::var_os(ARTIFACTS_ENV);
        unsafe { std::env::set_var(ARTIFACTS_ENV, "/home/u/.hive/work/chat/c1/artifacts") };
        let ws = Workspace::from_env();
        assert_eq!(
            ws.output_dir,
            Some(PathBuf::from("/home/u/.hive/work/chat/c1/artifacts"))
        );
        unsafe { std::env::set_var(ARTIFACTS_ENV, "") };
        let ws = Workspace::from_env();
        // Under a job this test runs in the job's TMPDIR; anywhere else nothing.
        assert!(ws.output_dir.is_none() || ws.cwd == ws.output_dir, "{ws:?}");
        match before {
            Some(v) => unsafe { std::env::set_var(ARTIFACTS_ENV, v) },
            None => unsafe { std::env::remove_var(ARTIFACTS_ENV) },
        }
    }

    #[test]
    fn a_job_without_artifacts_uses_its_own_tmpdir_and_a_shell_the_default() {
        let vars = |pairs: &'static [(&str, &str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        // A conversation: the artifacts folder, whatever else is set.
        let ws = Workspace::from_vars(vars(&[
            ("HIVE_ARTIFACTS", "/art"),
            ("HIVE_JOB", "job-1"),
            ("TMPDIR", "/w/job-1.tmp"),
        ]));
        assert_eq!(ws.output_dir, Some(PathBuf::from("/art")));
        assert_eq!(ws.cwd, Some(PathBuf::from("/art")));
        // A job: the run's own TMPDIR beside the clone, never the clone.
        let ws = Workspace::from_vars(vars(&[("HIVE_JOB", "job-1"), ("TMPDIR", "/w/job-1.tmp")]));
        assert_eq!(ws.output_dir, Some(PathBuf::from("/w/job-1.tmp")));
        assert_eq!(ws.cwd, Some(PathBuf::from("/w/job-1.tmp")));
        // The owner's shell has a TMPDIR too, but no job: Playwright's default.
        let ws = Workspace::from_vars(vars(&[("TMPDIR", "/var/folders/xy")]));
        assert_eq!(ws, Workspace::default());
        let ws = Workspace::from_vars(vars(&[("HIVE_JOB", "job-1"), ("HIVE_ARTIFACTS", "")]));
        assert_eq!(ws, Workspace::default());
    }
}
