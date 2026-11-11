use super::*;
use hive_testutil::httptest::{self, Response};
use std::path::Path;
use std::sync::Arc;

/// Restores the endpoint and empties the cache when a test ends.
pub(crate) struct EndpointGuard(Option<Endpoint>);

impl Drop for EndpointGuard {
    fn drop(&mut self) {
        if let Some(e) = self.0.take() {
            set_endpoint(e);
        }
    }
}

pub(crate) fn point_at(api: &str, web: &str) -> EndpointGuard {
    EndpointGuard(Some(set_endpoint(Endpoint {
        api: api.trim_end_matches('/').to_string(),
        web: web.trim_end_matches('/').to_string(),
    })))
}

/// A fresh 2048-bit RSA key as GitHub hands one out, written 0600 into
/// `dir`, with its public half beside it; None without openssl.
pub(crate) fn app_key(dir: &Path) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let key = dir.join("app.pem");
    let public = dir.join("app.pub.pem");
    let ok = std::process::Command::new("openssl")
        .args(["genrsa", "-out"])
        .arg(&key)
        .arg("2048")
        .output()
        .ok()?
        .status
        .success()
        && std::process::Command::new("openssl")
            .args(["rsa", "-pubout", "-in"])
            .arg(&key)
            .arg("-out")
            .arg(&public)
            .output()
            .ok()?
            .status
            .success();
    if !ok {
        return None;
    }
    crate::test_helpers::run(".", &["chmod", "600", key.to_str().unwrap()]);
    Some((key, public))
}

/// Configures the App on `s` with the key at `key`.
pub(crate) fn configure(s: &hive_store::Store, key: &Path) {
    let h = crate::util::HUMAN_AGENT;
    s.set_setting(h, APP_ID, "4242").unwrap();
    s.set_setting(h, INSTALLATION_ID, "777").unwrap();
    s.set_setting(h, PRIVATE_KEY_REF, &format!("file://{}", key.display()))
        .unwrap();
}

/// What the fake GitHub API saw: each request's method, path, auth and body.
pub(crate) type Seen = Arc<Mutex<Vec<(String, String, String, String)>>>;

pub(crate) fn rfc3339(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

/// A fake GitHub App API: mints `ghs_minted<N>` tokens that expire an hour
/// after `clock()` and answers the App, installation and repository
/// questions the Settings panel asks.
pub(crate) fn fake_github(clock: Arc<dyn Fn() -> i64 + Send + Sync>) -> (httptest::Server, Seen) {
    let seen: Seen = Arc::default();
    let log = seen.clone();
    let minted = Arc::new(Mutex::new(0u32));
    let srv = httptest::Server::new(move |req| {
        let auth = req.header("authorization").unwrap_or("").to_string();
        log.lock().unwrap().push((
            req.method.clone(),
            req.uri.clone(),
            auth.clone(),
            req.body_str(),
        ));
        match (req.method.as_str(), req.path()) {
            ("POST", "/app/installations/777/access_tokens") if auth.starts_with("Bearer ey") => {
                let mut n = minted.lock().unwrap();
                *n += 1;
                Response::json(
                    201,
                    &json!({
                        "token": format!("ghs_minted{}", *n),
                        "expires_at": rfc3339(clock() + 3600),
                        "permissions": {"contents": "write"},
                    }),
                )
            }
            ("GET", "/app") if auth.starts_with("Bearer ey") => Response::json(
                200,
                &json!({"slug": "hive-for-amistio", "name": "Hive for Amistio", "html_url": "https://github.com/apps/hive-for-amistio"}),
            ),
            ("GET", "/app/installations/777") if auth.starts_with("Bearer ey") => Response::json(
                200,
                &json!({"id": 777, "account": {"login": "amistio", "type": "Organization"}, "repository_selection": "selected"}),
            ),
            ("GET", "/installation/repositories") if auth.starts_with("token ghs_minted") => {
                Response::json(
                    200,
                    &json!({"total_count": 2, "repositories": [{"full_name": "amistio/hive"}, {"full_name": "amistio/site"}]}),
                )
            }
            _ => Response::json(401, &json!({"message": "Bad credentials"})),
        }
    });
    (srv, seen)
}

fn b64_json(part: &str) -> Value {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(part)
        .expect("base64url part");
    serde_json::from_slice(&bytes).expect("json part")
}

/// Whether `jwt` carries a valid RS256 signature by the key whose public
/// half is at `public`.
fn signature_verifies(jwt: &str, public: &Path, dir: &Path) -> bool {
    use base64::Engine;
    let (input, sig) = jwt.rsplit_once('.').unwrap();
    let data = dir.join("jwt.input");
    let sig_file = dir.join("jwt.sig");
    std::fs::write(&data, input).unwrap();
    std::fs::write(
        &sig_file,
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(sig)
            .unwrap(),
    )
    .unwrap();
    std::process::Command::new("openssl")
        .args(["dgst", "-sha256", "-verify"])
        .arg(public)
        .arg("-signature")
        .arg(&sig_file)
        .arg(&data)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn the_app_is_configured_only_when_all_three_settings_are_set() {
    let _serial = crate::test_helpers::serial();
    let s = crate::test_helpers::new_store();
    assert_eq!(configured(&s), None);
    let h = crate::util::HUMAN_AGENT;
    s.set_setting(h, APP_ID, "4242").unwrap();
    s.set_setting(h, INSTALLATION_ID, "777").unwrap();
    assert_eq!(configured(&s), None, "no key reference yet");
    s.set_setting(h, PRIVATE_KEY_REF, "file:///etc/hive/app.pem")
        .unwrap();
    let app = configured(&s).expect("configured");
    assert_eq!(app.app_id, "4242");
    assert_eq!(app.installation_id, "777");
    let st = status(&s);
    assert_eq!(st["configured"], true);
    assert!(
        st["error"]
            .as_str()
            .unwrap()
            .contains("git.github_app.private_key_ref"),
        "an unreadable key is named by its setting: {st}"
    );
}

#[test]
fn the_jwt_carries_the_app_claims_for_ten_minutes_and_verifies_with_the_key() {
    let _serial = crate::test_helpers::serial();
    let dir = tempfile::tempdir().unwrap();
    let Some((key, public)) = app_key(dir.path()) else {
        return;
    };
    let pem = std::fs::read(&key).unwrap();
    let now = 1_800_000_000;
    let jwt = sign_jwt("4242", &pem, now, &dir.path().display().to_string()).unwrap();
    let parts: Vec<&str> = jwt.split('.').collect();
    assert_eq!(parts.len(), 3, "{jwt}");
    assert_eq!(b64_json(parts[0]), json!({"alg": "RS256", "typ": "JWT"}));
    let claims = b64_json(parts[1]);
    assert_eq!(claims["iss"], "4242");
    assert_eq!(
        claims["iat"],
        now - 60,
        "backdated a minute for clock drift"
    );
    assert_eq!(claims["exp"], now - 60 + 600, "GitHub's ten-minute ceiling");
    assert!(
        signature_verifies(&jwt, &public, dir.path()),
        "RS256 by the key"
    );
    assert!(!jwt.contains("PRIVATE"));
    let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("tmp"))
        .map(|rd| rd.collect())
        .unwrap_or_default();
    assert!(leftovers.is_empty(), "scratch left: {leftovers:?}");

    let err = sign_jwt("4242", b"not a key", now, "/nonexistent").unwrap_err();
    assert!(err.contains("not a PEM private key"), "{err}");
}

#[test]
fn a_token_is_minted_for_one_repository_reused_for_55_minutes_then_minted_again() {
    let _serial = crate::test_helpers::serial();
    let _guard = hive_testutil::env_lock();
    let s = crate::test_helpers::new_store();
    let dir = tempfile::tempdir().unwrap();
    let Some((key, public)) = app_key(dir.path()) else {
        return;
    };
    configure(&s, &key);
    let clock = Arc::new(Mutex::new(1_800_000_000i64));
    let c = clock.clone();
    let (gh, seen) = fake_github(Arc::new(move || *c.lock().unwrap()));
    let _ep = point_at(&gh.url(), "https://github.com");
    let app = configured(&s).unwrap();
    let home = s.home.clone();
    let loads = Arc::new(Mutex::new(0));
    let l = loads.clone();
    let k = key.clone();
    let load = move || -> Result<Vec<u8>, String> {
        *l.lock().unwrap() += 1;
        Ok(std::fs::read(&k).unwrap())
    };
    let at = |secs: i64| {
        *clock.lock().unwrap() = secs;
        token_at(&app, "amistio/hive", secs, &home, &load).unwrap()
    };
    let t0 = 1_800_000_000;
    let first = at(t0);
    assert_eq!(first.secret(), "ghs_minted1");
    assert!(first.fresh);
    assert_eq!(first.expires_at, t0 + 3600);
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let (method, uri, auth, body) = &seen[0];
        assert_eq!(
            (method.as_str(), uri.as_str()),
            ("POST", "/app/installations/777/access_tokens")
        );
        assert_eq!(
            serde_json::from_str::<Value>(body).unwrap(),
            json!({"repositories": ["hive"]}),
            "scoped to the one repository the job needs"
        );
        let jwt = auth.strip_prefix("Bearer ").unwrap();
        assert!(signature_verifies(jwt, &public, dir.path()));
        assert_eq!(b64_json(jwt.split('.').nth(1).unwrap())["iss"], "4242");
    }
    let again = at(t0 + 54 * 60);
    assert_eq!(again.secret(), "ghs_minted1", "reused within 55 minutes");
    assert!(!again.fresh);
    assert_eq!(*loads.lock().unwrap(), 1, "the key is read only to mint");
    let other = at(t0 + 54 * 60);
    assert_eq!(other.secret(), "ghs_minted1");
    let refreshed = at(t0 + 55 * 60 + 1);
    assert_eq!(refreshed.secret(), "ghs_minted2", "minted again at 55 min");
    assert_eq!(refreshed.expires_at, t0 + 55 * 60 + 1 + 3600);
    assert_eq!(*loads.lock().unwrap(), 2);
    let site = token_at(&app, "amistio/site", t0 + 55 * 60 + 1, &home, &load).unwrap();
    assert_eq!(site.secret(), "ghs_minted3", "one token per repository");
    assert_eq!(seen.lock().unwrap().len(), 3);
    assert!(
        !format!("{first:?} {refreshed:?} {app:?}").contains("ghs_"),
        "Debug shows no token"
    );

    // A refusal names the call and GitHub's words, never a credential.
    let bad = App {
        installation_id: "1".into(),
        ..app.clone()
    };
    let err = token_at(&bad, "amistio/hive", t0, &home, &load).unwrap_err();
    assert!(
        err.contains("/app/installations/1/access_tokens answered 401: Bad credentials"),
        "{err}"
    );
    assert!(!err.contains("ey") || !err.contains("Bearer"), "{err}");
}

#[test]
fn the_key_is_loaded_from_its_reference_on_the_brain_and_never_inside_a_job() {
    let _serial = crate::test_helpers::serial();
    let _guard = hive_testutil::env_lock();
    let s = crate::test_helpers::new_store();
    let dir = tempfile::tempdir().unwrap();
    let Some((key, _)) = app_key(dir.path()) else {
        return;
    };
    configure(&s, &key);
    let clock: Arc<dyn Fn() -> i64 + Send + Sync> = Arc::new(now_secs);
    let (gh, _) = fake_github(clock);
    let _ep = point_at(&gh.url(), "https://github.com");
    let app = configured(&s).unwrap();
    let t = token_for(&s, &app, "amistio/hive").unwrap();
    assert_eq!(t.secret(), "ghs_minted1");
    assert_eq!(
        gh_env(&s, "amistio/hive")
            .unwrap()
            .unwrap()
            .iter()
            .filter(|kv| kv.as_str() == "GH_TOKEN=ghs_minted1")
            .count(),
        1
    );
    assert!(
        gh_env(&s, "ghe.corp/amistio/hive").is_none(),
        "another host"
    );
    clear_cache();
    {
        let _job = hive_testutil::setenv("HIVE_JOB", "job-20261110T000000Z-inside");
        let err = token_for(&s, &app, "amistio/hive").unwrap_err();
        assert!(err.contains("ADR-041 D4"), "{err}");
    }
    // A key file others can read is refused.
    crate::test_helpers::run(".", &["chmod", "644", key.to_str().unwrap()]);
    let err = token_for(&s, &app, "amistio/hive").unwrap_err();
    assert!(err.contains(PRIVATE_KEY_REF), "{err}");
}

#[test]
fn app_remotes_are_the_app_hosts_owner_repo_urls() {
    let _serial = crate::test_helpers::serial();
    for (url, want) in [
        ("git@github.com:amistio/hive.git", Some("amistio/hive")),
        ("https://github.com/amistio/hive", Some("amistio/hive")),
        (
            "https://me@github.com/amistio/hive.git/",
            Some("amistio/hive"),
        ),
        (
            "ssh://git@github.com/amistio/hive.git",
            Some("amistio/hive"),
        ),
        ("https://gitlab.com/amistio/hive.git", None),
        ("/tmp/origin.git", None),
        ("file:///tmp/origin.git", None),
        ("https://github.com/amistio", None),
        ("https://github.com/a/b/c", None),
        ("https://github.com/a/b;rm", None),
    ] {
        assert_eq!(app_remote(url).map(|(_, r)| r).as_deref(), want, "{url}");
    }
    assert_eq!(
        https_url("amistio/hive"),
        "https://github.com/amistio/hive.git"
    );
}

/// The brain's own fetch and push of an App remote (ADR-070 step 3, the
/// delivery pass's path): git authenticates with the installation token
/// through a helper outside the checkout, the helper is gone afterwards,
/// and nothing of it lands in the checkout's config or history.
#[test]
fn the_brain_fetches_and_pushes_an_app_remote_through_a_helper_it_removes() {
    let _serial = crate::test_helpers::serial();
    let _guard = hive_testutil::env_lock();
    if !hive_testutil::githttp::available() {
        return;
    }
    let s = crate::test_helpers::new_store();
    let dir = tempfile::tempdir().unwrap();
    let Some((key, _)) = app_key(dir.path()) else {
        return;
    };
    configure(&s, &key);
    let clock: Arc<dyn Fn() -> i64 + Send + Sync> = Arc::new(now_secs);
    let (gh, _) = fake_github(clock);
    let (proj, repo, bare) = crate::test_helpers_daemon::push_fixture(&s);
    let served = tempfile::tempdir().unwrap();
    let origin = served.path().join("amistio").join("hive.git");
    std::fs::create_dir_all(origin.parent().unwrap()).unwrap();
    crate::test_helpers::run(
        ".",
        &[
            "git",
            "clone",
            "-q",
            "--bare",
            &bare,
            origin.to_str().unwrap(),
        ],
    );
    use base64::Engine;
    let basic = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("x-access-token:ghs_minted1")
    );
    let git = hive_testutil::githttp::Server::new(served.path(), &basic);
    let _ep = point_at(&gh.url(), &git.url());
    let url = format!("{}/amistio/hive.git", git.url());
    crate::test_helpers::run(&repo, &["git", "remote", "set-url", "origin", &url]);

    // Upstream moves on; the brain's sync fetches it with the App token.
    let other = tempfile::tempdir().unwrap();
    crate::test_helpers::run(
        ".",
        &[
            "git",
            "clone",
            "-q",
            origin.to_str().unwrap(),
            other.path().to_str().unwrap(),
        ],
    );
    std::fs::write(other.path().join("up.txt"), "upstream\n").unwrap();
    crate::test_helpers::run(other.path(), &["git", "add", "-A"]);
    crate::test_helpers::run(other.path(), &["git", "commit", "-q", "-m", "upstream"]);
    crate::test_helpers::run(other.path(), &["git", "push", "-q", "origin", "main"]);
    let line = crate::forge::sync_default_from_upstream(&s, &proj, "daemon", &s.now(), true);
    let up_sha = crate::test_helpers::run(other.path(), &["git", "rev-parse", "HEAD"]);
    let have = crate::test_helpers::run(&repo, &["git", "rev-parse", "main"]);
    assert_eq!(
        have.trim(),
        up_sha.trim(),
        "fetched through the App: {line:?}"
    );

    // A branch pushed with the same environment the delivery pass uses.
    crate::test_helpers::run(&repo, &["git", "branch", "hive/a/job", "main"]);
    let (env, cleanup, why) = crate::forge::remote_credential_env(&s, repo.clone(), &url);
    assert!(why.is_empty(), "{why}");
    let helper_dirs = || -> Vec<_> {
        std::fs::read_dir(Path::new(&s.home).join("tmp").join("gitcred"))
            .map(|rd| rd.flatten().map(|e| e.path()).collect())
            .unwrap_or_default()
    };
    assert_eq!(helper_dirs().len(), 1, "a helper while the command runs");
    let pushed = crate::util::git_timeout_env(
        &repo,
        Duration::from_secs(60),
        &env,
        &["push", &url, "hive/a/job:refs/heads/hive/a/job"],
    );
    cleanup("", &[]);
    assert!(pushed.ok(), "{}", pushed.stderr);
    assert!(
        helper_dirs().is_empty(),
        "the helper is removed after the push"
    );
    let on_origin =
        crate::test_helpers::run(&origin, &["git", "rev-parse", "refs/heads/hive/a/job"]);
    assert_eq!(on_origin.trim(), up_sha.trim());
    assert!(git.seen().iter().any(|a| a == &basic), "{:?}", git.seen());
    let config = std::fs::read_to_string(Path::new(&repo).join(".git").join("config")).unwrap();
    assert!(
        !config.contains("ghs_") && !config.contains("credential"),
        "{config}"
    );
    let tracked = std::process::Command::new("git")
        .args(["grep", "-l", "ghs_minted", "hive/a/job"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert_eq!(
        tracked.status.code(),
        Some(1),
        "token committed: {tracked:?}"
    );
    assert!(!pushed.stderr.contains("ghs_"), "{}", pushed.stderr);
}

/// Without an App, or for a remote on another host, the path is today's:
/// `push.credential` and the machine's logins, no helper, no event.
#[test]
fn other_remotes_keep_the_push_credential_path() {
    let _serial = crate::test_helpers::serial();
    let _guard = hive_testutil::env_lock();
    let s = crate::test_helpers::new_store();
    let (env, cleanup, why) =
        crate::forge::remote_credential_env(&s, ".".into(), "git@github.com:amistio/hive.git");
    cleanup("", &[]);
    assert!(why.is_empty());
    assert!(
        !env.iter().any(|kv| kv.starts_with("GIT_CONFIG_COUNT")),
        "{env:?}"
    );
    s.set_setting(crate::util::HUMAN_AGENT, APP_ID, "1")
        .unwrap();
    s.set_setting(crate::util::HUMAN_AGENT, INSTALLATION_ID, "2")
        .unwrap();
    s.set_setting(
        crate::util::HUMAN_AGENT,
        PRIVATE_KEY_REF,
        "file:///nonexistent/app.pem",
    )
    .unwrap();
    let (env, cleanup, why) =
        crate::forge::remote_credential_env(&s, ".".into(), "https://gitlab.com/a/b.git");
    cleanup("", &[]);
    assert!(why.is_empty());
    assert!(!env.iter().any(|kv| kv.starts_with("GIT_CONFIG_COUNT")));
    // An App remote whose key cannot be read falls back, written down.
    let (env, cleanup, why) =
        crate::forge::remote_credential_env(&s, ".".into(), "https://github.com/a/b.git");
    cleanup("", &[]);
    assert!(why.is_empty(), "{why}");
    assert!(!env.iter().any(|kv| kv.starts_with("GIT_CONFIG_COUNT")));
    assert_eq!(
        crate::test_helpers::count(
            &s,
            "SELECT COUNT(*) AS n FROM events WHERE action = 'gitapp.fallback' AND subject = 'a/b'",
            &[]
        ),
        1
    );
}

#[test]
fn the_settings_panel_status_names_the_installation_and_its_repositories() {
    let _serial = crate::test_helpers::serial();
    let _guard = hive_testutil::env_lock();
    let s = crate::test_helpers::new_store();
    let st = status(&s);
    assert_eq!(st["configured"], false);
    assert_eq!(st["install_url"], "");
    let dir = tempfile::tempdir().unwrap();
    let Some((key, _)) = app_key(dir.path()) else {
        return;
    };
    configure(&s, &key);
    let clock: Arc<dyn Fn() -> i64 + Send + Sync> = Arc::new(now_secs);
    let (gh, _) = fake_github(clock);
    let _ep = point_at(&gh.url(), "https://github.com");
    let st = status(&s);
    assert_eq!(st["error"], "", "{st}");
    assert_eq!(st["configured"], true);
    assert_eq!(st["app"]["slug"], "hive-for-amistio");
    assert_eq!(
        st["install_url"],
        "https://github.com/apps/hive-for-amistio/installations/new"
    );
    assert_eq!(st["account"]["login"], "amistio");
    assert_eq!(st["repository_selection"], "selected");
    assert_eq!(st["repositories"], json!(["amistio/hive", "amistio/site"]));
    let text = st.to_string();
    assert!(
        !text.contains("ghs_") && !text.contains("PRIVATE"),
        "{text}"
    );
}
