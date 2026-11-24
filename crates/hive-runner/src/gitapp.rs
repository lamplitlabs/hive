//! GitHub App installation tokens (ADR-070 rule 3, step 3): git is logged
//! in to once, on the web, by installing one GitHub App on the owner's
//! account, and every git command Hive runs inherits that login through a
//! short-lived token instead of a personal access token kept on a machine.
//!
//! The brain holds three settings: `git.github_app.app_id`,
//! `git.github_app.installation_id` and `git.github_app.private_key_ref`, a
//! hive-cred reference to the App's private key that only the brain
//! resolves (ADR-041 D4 refuses it inside a job). For one repository it
//!
//! 1. signs a ten-minute App JWT (RS256, with the `openssl` command - the
//!    key goes to it on stdin and is never written to disk),
//! 2. exchanges it at `POST /app/installations/{id}/access_tokens` for an
//!    installation token scoped to that one repository (`repositories`),
//! 3. keeps the token in memory until five minutes before it expires, so a
//!    one-hour token is minted again after 55 minutes, never sooner.
//!
//! The token then travels as a lease: to a hand on its push verb's stdin
//! ([`hive_hand_core::gitcred::token_lease`]), and to the brain's own git
//! commands through a [`hive_hand_core::gitcred::Helper`] that lives as long
//! as the command. `gh` gets it as `GH_TOKEN`. A [`Token`]'s `Debug` and
//! every error here name the repository, never the token or the key.

use hive_hand_core::gitcred;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex, RwLock};
use std::time::Duration;

/// Setting names (ADR-070 step 3).
pub const APP_ID: &str = "git.github_app.app_id";
pub const INSTALLATION_ID: &str = "git.github_app.installation_id";
pub const PRIVATE_KEY_REF: &str = "git.github_app.private_key_ref";

/// A token is minted again once it has less than this left (a one-hour
/// token is reused for 55 minutes).
pub const REFRESH_BEFORE_SECS: i64 = 5 * 60;
/// The JWT's lifetime: GitHub refuses one longer than ten minutes. Its
/// `iat` is set a minute in the past against clock drift, so it expires
/// nine minutes from now.
pub const JWT_SECS: i64 = 10 * 60;
const JWT_BACKDATE_SECS: i64 = 60;
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);

/// The App the settings name; None while any of the three is empty.
#[derive(Clone, PartialEq, Eq)]
pub struct App {
    pub app_id: String,
    pub installation_id: String,
    pub key_ref: String,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("app_id", &self.app_id)
            .field("installation_id", &self.installation_id)
            .field("key_ref", &self.key_ref)
            .finish()
    }
}

/// The App the store's settings configure, or None.
pub fn configured(s: &hive_store::Store) -> Option<App> {
    let app = App {
        app_id: s.setting(APP_ID, "").trim().to_string(),
        installation_id: s.setting(INSTALLATION_ID, "").trim().to_string(),
        key_ref: s.setting(PRIVATE_KEY_REF, "").trim().to_string(),
    };
    (!app.app_id.is_empty() && !app.installation_id.is_empty() && !app.key_ref.is_empty())
        .then_some(app)
}

/// Where the App API and the git host it covers live: github.com unless a
/// test points them at its own servers ([`set_endpoint`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// API base without a trailing slash: `https://api.github.com`.
    pub api: String,
    /// The web base the install link is built on and the git remotes the
    /// App's tokens are for live under: `https://github.com`.
    pub web: String,
}

impl Endpoint {
    /// `host[:port]` of the git remotes the App's tokens are for.
    pub fn git_host(&self) -> String {
        gitcred::http_host(&self.web).unwrap_or_default()
    }
}

impl Default for Endpoint {
    fn default() -> Self {
        Endpoint {
            api: "https://api.github.com".into(),
            web: "https://github.com".into(),
        }
    }
}

static ENDPOINT: LazyLock<RwLock<Endpoint>> = LazyLock::new(|| RwLock::new(Endpoint::default()));

/// The endpoint in use.
pub fn endpoint() -> Endpoint {
    ENDPOINT.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Test seam: points the App API and its git host at a test's servers and
/// returns the previous endpoint (restore it when the test ends). Clears
/// the token cache, which belongs to the old endpoint.
pub fn set_endpoint(e: Endpoint) -> Endpoint {
    clear_cache();
    std::mem::replace(&mut *ENDPOINT.write().unwrap_or_else(|e| e.into_inner()), e)
}

/// One installation token. `Debug` never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct Token {
    secret: String,
    /// Unix seconds GitHub said it expires at.
    pub expires_at: i64,
    /// What it was minted for: `OWNER/REPO`, or `*` for every repository
    /// the installation covers.
    pub scope: String,
    /// True when this call minted it, false for a copy from the cache.
    pub fresh: bool,
}

impl Token {
    /// The token itself, for a credential helper or `GH_TOKEN` only.
    pub fn secret(&self) -> &str {
        &self.secret
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Token")
            .field("scope", &self.scope)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Tokens by (installation, scope), kept until [`REFRESH_BEFORE_SECS`]
/// before they expire.
static CACHE: LazyLock<Mutex<BTreeMap<(String, String), Token>>> = LazyLock::new(Mutex::default);

/// Forgets every cached token (a settings change, a test).
pub fn clear_cache() {
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn b64url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// The JWT's two signed parts, `header.claims`, for `app_id` at `now`.
pub fn jwt_signing_input(app_id: &str, now: i64) -> String {
    let header = json!({"alg": "RS256", "typ": "JWT"});
    let iat = now - JWT_BACKDATE_SECS;
    let claims = json!({"iat": iat, "exp": iat + JWT_SECS, "iss": app_id});
    format!(
        "{}.{}",
        b64url(header.to_string().as_bytes()),
        b64url(claims.to_string().as_bytes())
    )
}

/// The App JWT for `app_id` at `now`, signed RS256 with `key` (a PEM RSA
/// private key, PKCS#1 as GitHub downloads it or PKCS#8). The key reaches
/// `openssl` on stdin; only the unsigned header and claims touch disk.
pub fn sign_jwt(app_id: &str, key: &[u8], now: i64, home: &str) -> Result<String, String> {
    if !hive_cred::is_private_key(key) {
        return Err("the GitHub App key is not a PEM private key".into());
    }
    let input = jwt_signing_input(app_id, now);
    let tmp = ScratchDir::new(home).map_err(|e| format!("could not write the JWT to sign: {e}"))?;
    let unsigned = tmp.0.join("unsigned");
    let signature = tmp.0.join("signature");
    std::fs::write(&unsigned, input.as_bytes())
        .map_err(|e| format!("could not write the JWT to sign: {e}"))?;
    let argv: Vec<String> = vec![
        "openssl".into(),
        "dgst".into(),
        "-sha256".into(),
        "-sign".into(),
        "/dev/stdin".into(),
        "-out".into(),
        signature.display().to_string(),
        unsigned.display().to_string(),
    ];
    let dir = if std::path::Path::new(home).is_dir() {
        home.to_string()
    } else {
        std::env::temp_dir().display().to_string()
    };
    let out = crate::proc::run_group(
        &hive_bus::CancellationToken::new(),
        &argv,
        &dir,
        &crate::proc::environ(),
        key,
        Duration::from_secs(20),
    )
    .map_err(|e| format!("openssl could not start to sign the GitHub App JWT: {e}"))?;
    let sig = std::fs::read(&signature).unwrap_or_default();
    if out.timeout || out.exit_code != 0 || sig.is_empty() {
        return Err(format!(
            "openssl could not sign the GitHub App JWT: {}",
            hive_cred::redact(&crate::run::squeeze(&out.stderr, 200), &[])
        ));
    }
    Ok(format!("{input}.{}", b64url(&sig)))
}

/// A 0700 directory for one signature, removed on drop.
struct ScratchDir(std::path::PathBuf);

impl ScratchDir {
    fn new(home: &str) -> std::io::Result<ScratchDir> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let parent = if std::path::Path::new(home).is_dir() {
            std::path::Path::new(home).join("tmp")
        } else {
            std::env::temp_dir()
        };
        std::fs::create_dir_all(&parent)?;
        let dir = parent.join(format!(
            "jwt-{}-{}-{:x}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut b = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b.create(&dir)?;
        Ok(ScratchDir(dir))
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Reads the App's private key from its reference, on the brain only.
fn read_key(home: &str, app: &App) -> Result<Vec<u8>, String> {
    let parsed = hive_cred::parse(&app.key_ref).map_err(|e| format!("{PRIVATE_KEY_REF}: {e}"))?;
    hive_cred::new(home)
        .resolve(parsed)
        .map(|secret| secret.key)
        .map_err(|e| {
            format!(
                "{PRIVATE_KEY_REF}: {}",
                hive_cred::redact(&e.to_string(), &[])
            )
        })
}

fn client() -> &'static reqwest::blocking::Client {
    static CLIENT: LazyLock<reqwest::blocking::Client> = LazyLock::new(|| {
        reqwest::blocking::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("github app http client")
    });
    &CLIENT
}

/// One App API call with `auth` (`Bearer JWT` or `token TOKEN`); the
/// error names the call and GitHub's message, never the credential.
fn api(method: &str, path: &str, auth: &str, body: Option<&Value>) -> Result<Value, String> {
    let url = format!("{}{path}", endpoint().api);
    let mut req = match method {
        "POST" => client().post(&url),
        _ => client().get(&url),
    }
    .header("Authorization", auth)
    .header("User-Agent", "hive")
    .header("Accept", "application/vnd.github+json")
    .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(b) = body {
        req = req
            .header("Content-Type", "application/json")
            .body(b.to_string());
    }
    let what = format!("{method} {path}");
    let resp = req.send().map_err(|e| {
        format!(
            "{what}: {}",
            crate::executor::redact_tokens(&e.without_url().to_string())
        )
    })?;
    let status = resp.status().as_u16();
    let text = resp.text().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        let msg = v
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("no message");
        return Err(format!(
            "{what} answered {status}: {}",
            crate::executor::redact_tokens(&crate::run::squeeze(msg, 200))
        ));
    }
    Ok(v)
}

fn parse_expiry(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|d| d.timestamp())
}

/// `owner/repo` -> `repo`, the name the `repositories` field takes.
fn repo_name(scope: &str) -> &str {
    scope.rsplit('/').next().unwrap_or(scope)
}

/// Exchanges a JWT for an installation token scoped to `scope`
/// (`OWNER/REPO`, or `*` for the whole installation).
fn exchange(app: &App, jwt: &str, scope: &str) -> Result<Token, String> {
    let body = if scope == "*" {
        json!({})
    } else {
        json!({"repositories": [repo_name(scope)]})
    };
    let v = api(
        "POST",
        &format!("/app/installations/{}/access_tokens", app.installation_id),
        &format!("Bearer {jwt}"),
        Some(&body),
    )?;
    let secret = v.get("token").and_then(Value::as_str).unwrap_or_default();
    if !gitcred::valid_token(secret) {
        return Err("GitHub answered the token exchange without a token".into());
    }
    let expires_at = v
        .get("expires_at")
        .and_then(Value::as_str)
        .and_then(parse_expiry)
        .ok_or("GitHub answered the token exchange without an expiry")?;
    Ok(Token {
        secret: secret.to_string(),
        expires_at,
        scope: scope.to_string(),
        fresh: true,
    })
}

/// The installation token for `scope`, from the cache while it has more
/// than [`REFRESH_BEFORE_SECS`] left at `now`, else minted afresh with a
/// JWT signed by the key `load_key` returns.
pub fn token_at(
    app: &App,
    scope: &str,
    now: i64,
    home: &str,
    load_key: &dyn Fn() -> Result<Vec<u8>, String>,
) -> Result<Token, String> {
    let key = (app.installation_id.clone(), scope.to_string());
    if let Some(t) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).get(&key)
        && t.expires_at - now > REFRESH_BEFORE_SECS
    {
        let mut t = t.clone();
        t.fresh = false;
        return Ok(t);
    }
    let pem = load_key()?;
    let jwt = sign_jwt(&app.app_id, &pem, now, home)?;
    drop(pem);
    let t = exchange(app, &jwt, scope)?;
    CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, t.clone());
    Ok(t)
}

/// The installation token for one repository (`OWNER/REPO`) now.
pub fn token_for(s: &hive_store::Store, app: &App, repo: &str) -> Result<Token, String> {
    let home = s.home.clone();
    token_at(app, repo, now_secs(), &home, &|| read_key(&home, app))
}

/// A remote the App's tokens cover: the App's git host and `OWNER/REPO`,
/// from an https, ssh or scp-style url; None for any other host or shape.
pub fn app_remote(url: &str) -> Option<(String, String)> {
    let (host, path) = if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        let (authority, path) = rest.split_once('/')?;
        (authority.rsplit('@').next().unwrap_or(authority), path)
    } else if let Some(rest) = url.strip_prefix("ssh://") {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next().unwrap_or(authority);
        (host.split(':').next().unwrap_or(host), path)
    } else if !url.contains("://") {
        let (userhost, path) = url.split_once(':')?;
        (userhost.rsplit('@').next().unwrap_or(userhost), path)
    } else {
        return None;
    };
    if host != endpoint().git_host() {
        return None;
    }
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut segs = path.split('/');
    let (owner, repo) = (segs.next()?, segs.next()?);
    let ok = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    (ok(owner) && ok(repo) && segs.next().is_none())
        .then(|| (host.to_string(), format!("{owner}/{repo}")))
}

/// The https url of `OWNER/REPO` on the App's git host, the form a token
/// authenticates (an ssh remote is pushed over https instead).
pub fn https_url(repo: &str) -> String {
    format!("{}/{repo}.git", endpoint().web.trim_end_matches('/'))
}

/// The App token for the remote at `url`, when an App is configured and
/// the url is on its host: Some(Ok((repo, token))), Some(Err(why)) when
/// minting failed, None when the App does not apply.
pub fn token_for_url(s: &hive_store::Store, url: &str) -> Option<Result<(String, Token), String>> {
    let app = configured(s)?;
    let (_, repo) = app_remote(url)?;
    Some(token_for(s, &app, &repo).map(|t| (repo, t)))
}

/// A credential helper for a brain-side git command on `url`, with the
/// `insteadOf` rewrites that send an ssh remote of the App's host over
/// https, so a fetch or push names the remote as it is configured.
pub fn helper_env(
    s: &hive_store::Store,
    token: &Token,
    repo: &str,
) -> Result<(gitcred::Helper, Vec<String>), String> {
    let host = endpoint().git_host();
    let parent = std::path::Path::new(&s.home).join("tmp").join("gitcred");
    let helper = gitcred::Helper::install(&parent, &host, token.secret())
        .map_err(|e| format!("credential helper for {repo}: {e}"))?;
    let https = https_url(repo);
    let bare = host.split(':').next().unwrap_or(&host).to_string();
    let rewrites = vec![
        (
            format!("url.{https}.insteadOf"),
            format!("git@{bare}:{repo}.git"),
        ),
        (
            format!("url.{https}.insteadOf"),
            format!("git@{bare}:{repo}"),
        ),
        (
            format!("url.{https}.insteadOf"),
            format!("ssh://git@{bare}/{repo}.git"),
        ),
        (
            format!("url.{https}.insteadOf"),
            format!("ssh://git@{bare}/{repo}"),
        ),
    ];
    let env = helper.env(&rewrites);
    Ok((helper, env))
}

/// `GH_TOKEN` for `gh` on `repo` (`OWNER/REPO` as `gh --repo` takes it),
/// when an App is configured: the daemon's environment with the App's
/// token for that repository. None when no App is set or the repository
/// is on another host; Some(Err) says why minting failed.
pub fn gh_env(s: &hive_store::Store, repo: &str) -> Option<Result<Vec<String>, String>> {
    let app = configured(s)?;
    let e = endpoint();
    let host = e.git_host();
    let repo = if host == "github.com" {
        repo.to_string()
    } else {
        repo.strip_prefix(&format!("{host}/"))?.to_string()
    };
    if repo.split('/').count() != 2 {
        return None;
    }
    Some(token_for(s, &app, &repo).map(|t| {
        let mut extra = BTreeMap::new();
        extra.insert("GH_TOKEN".to_string(), t.secret().to_string());
        crate::proc::env_with(&crate::proc::environ(), &extra)
    }))
}

/// What the hub's Settings -> Git panel shows: whether an App is set, the
/// installation's account and the repositories it covers (asked of the App
/// API), and the link that installs the App. Never a token or the key.
pub fn status(s: &hive_store::Store) -> Value {
    let app_id = s.setting(APP_ID, "").trim().to_string();
    let installation_id = s.setting(INSTALLATION_ID, "").trim().to_string();
    let key_ref = s.setting(PRIVATE_KEY_REF, "").trim().to_string();
    let mut out = json!({
        "configured": false,
        "app_id": app_id,
        "installation_id": installation_id,
        "private_key_ref": key_ref,
        "app": Value::Null,
        "install_url": "",
        "account": Value::Null,
        "repository_selection": "",
        "repositories": [],
        "error": "",
    });
    let Some(app) = configured(s) else {
        return out;
    };
    out["configured"] = json!(true);
    let home = s.home.clone();
    let e = endpoint();
    let result = (|| -> Result<(), String> {
        let pem = read_key(&home, &app)?;
        let jwt = sign_jwt(&app.app_id, &pem, now_secs(), &home)?;
        drop(pem);
        let me = api("GET", "/app", &format!("Bearer {jwt}"), None)?;
        let slug = me.get("slug").and_then(Value::as_str).unwrap_or_default();
        out["app"] = json!({
            "slug": slug,
            "name": me.get("name").and_then(Value::as_str).unwrap_or_default(),
            "html_url": me.get("html_url").and_then(Value::as_str).unwrap_or_default(),
        });
        if !slug.is_empty() {
            out["install_url"] = json!(format!("{}/apps/{slug}/installations/new", e.web));
        }
        let inst = api(
            "GET",
            &format!("/app/installations/{}", app.installation_id),
            &format!("Bearer {jwt}"),
            None,
        )?;
        let account = inst.get("account").cloned().unwrap_or(Value::Null);
        out["account"] = json!({
            "login": account.get("login").and_then(Value::as_str).unwrap_or_default(),
            "type": account.get("type").and_then(Value::as_str).unwrap_or_default(),
        });
        out["repository_selection"] = json!(
            inst.get("repository_selection")
                .and_then(Value::as_str)
                .unwrap_or_default()
        );
        let t = token_at(&app, "*", now_secs(), &home, &|| read_key(&home, &app))?;
        let repos = api(
            "GET",
            "/installation/repositories?per_page=100",
            &format!("token {}", t.secret()),
            None,
        )?;
        let names: Vec<Value> = repos
            .get("repositories")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|r| r.get("full_name").and_then(Value::as_str))
                    .map(|n| json!(n))
                    .collect()
            })
            .unwrap_or_default();
        out["repositories"] = json!(names);
        Ok(())
    })();
    if let Err(why) = result {
        out["error"] = json!(why);
    }
    out
}

#[cfg(test)]
#[path = "gitapp_tests.rs"]
pub(crate) mod tests;
