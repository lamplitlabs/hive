//! A git smart-HTTP test server: `git http-backend` behind [`crate::httptest`],
//! answering only requests that carry one exact `Authorization` header, so a
//! test proves that git authenticated with the credential it was handed (a
//! GitHub App installation token sent as `x-access-token:<token>` Basic auth).
//!
//! ```ignore
//! let srv = githttp::Server::new(&parent_dir, "Basic eC1hY2Nlc3MtdG9rZW46dG9r");
//! let url = format!("{}/origin.git", srv.url());
//! ```
//!
//! An unauthenticated request gets `401` with `WWW-Authenticate: Basic`, which
//! is what makes git ask its credential helper. `seen()` lists every
//! `Authorization` header the server was sent, so a test can also assert what
//! git did not send.

use crate::httptest::{self, Response};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

pub struct Server {
    inner: httptest::Server,
    seen: Arc<Mutex<Vec<String>>>,
}

/// Whether `git http-backend` is installed beside git (a test skips without it).
pub fn available() -> bool {
    Command::new(hive_gitenv::git_program())
        .arg("--exec-path")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .is_some_and(|dir| std::path::Path::new(&dir).join("git-http-backend").exists())
}

impl Server {
    /// Serves the repositories under `root` (`<url>/<name>.git`), receive-pack
    /// included, to requests whose `Authorization` header is exactly `auth`.
    pub fn new(root: &std::path::Path, auth: &str) -> Server {
        let root = root.to_path_buf();
        let auth = auth.to_string();
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let log = seen.clone();
        let inner = httptest::Server::new(move |req| {
            let got = req.header("authorization").unwrap_or("").to_string();
            log.lock().unwrap().push(got.clone());
            if got != auth {
                return Response::text(401, "auth needed\n")
                    .header("WWW-Authenticate", "Basic realm=\"hive-test\"");
            }
            backend(&root, req)
        });
        Server { inner, seen }
    }

    /// Base URL without a trailing slash.
    pub fn url(&self) -> String {
        self.inner.url()
    }

    /// Every `Authorization` header sent so far ("" for none), in order.
    pub fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

/// One request through `git http-backend` as CGI.
fn backend(root: &std::path::Path, req: &httptest::Request) -> Response {
    let mut cmd = Command::new(hive_gitenv::git_program());
    cmd.arg("http-backend")
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("GIT_PROJECT_ROOT", root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        // An authenticated user may push (http-backend refuses receive-pack
        // to an anonymous one unless http.receivepack is set).
        .env("REMOTE_USER", "x-access-token")
        .env("REMOTE_ADDR", "127.0.0.1")
        .env("REQUEST_METHOD", &req.method)
        .env("PATH_INFO", req.path())
        .env("QUERY_STRING", req.query())
        .env(
            "CONTENT_TYPE",
            req.header("content-type").unwrap_or_default(),
        )
        .env("CONTENT_LENGTH", req.body.len().to_string())
        .env(
            "GIT_PROTOCOL",
            req.header("git-protocol").unwrap_or_default(),
        )
        .env(
            "HTTP_CONTENT_ENCODING",
            req.header("content-encoding").unwrap_or_default(),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Response::text(500, format!("http-backend: {e}\n")),
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&req.body);
    }
    let out = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => return Response::text(500, format!("http-backend: {e}\n")),
    };
    // CGI output: headers, a blank line, the body.
    let raw = out.stdout;
    let (head, body) = match find(&raw, b"\r\n\r\n") {
        Some(i) => (&raw[..i], &raw[i + 4..]),
        None => match find(&raw, b"\n\n") {
            Some(i) => (&raw[..i], &raw[i + 2..]),
            None => {
                return Response::text(
                    500,
                    format!("http-backend: {}\n", String::from_utf8_lossy(&out.stderr)),
                );
            }
        },
    };
    let mut status = 200;
    let mut resp_headers = Vec::new();
    for line in String::from_utf8_lossy(head).lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        if k.eq_ignore_ascii_case("status") {
            status = v
                .split_whitespace()
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(500);
        } else if !k.eq_ignore_ascii_case("content-length") {
            resp_headers.push((k.to_string(), v.to_string()));
        }
    }
    let mut resp = Response::new(status).body(body.to_vec());
    for (k, v) in resp_headers {
        resp = resp.header(&k, &v);
    }
    resp
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}
