//! Reaching the console through the team's hosted front (ADR-070 rule 5,
//! `hub.allow_remote`).
//!
//! The hub never listens beyond 127.0.0.1. When the owner sets
//! `hub.allow_remote` to the front's origin and starts the hub with
//! `--front-port`, a second loopback listener takes the front relay's
//! traffic; every request on it is [`Request::via_front`]. On that listener:
//!
//! - nothing is served while `hub.allow_remote` is empty (403), so clearing
//!   the setting closes the console to the front at once;
//! - once it is set, a request is answered only when its Host names the
//!   front (the host of `hub.allow_remote`), `localhost`, a `.localhost`
//!   name or an IP address literal, and gets 403 otherwise: a web page whose
//!   name was re-pointed at 127.0.0.1 (DNS rebinding) reaches this port too,
//!   and its reads would pass like the front's;
//! - reads pass (the front checks the member's web session before it relays);
//! - every write carries a member's passkey assertion over a nonce this brain
//!   issued and the write's method, path and body
//!   (`hive_runner::webauthn`); the brain verifies it against the member's
//!   registered key, spends the nonce once and moves the passkey's counter
//!   forward before the write reaches its route, so the front - which only
//!   relays bytes - cannot forge or replay one;
//! - settings and member writes need a member whose role is `owner`;
//! - routes that are loopback-only (credentials, folders, held-out fixtures,
//!   job repair) refuse it, whatever Host header the relay sends.
//!
//! On the loopback listener nothing changes: the same-origin write guard and
//! each route's confirmation header stay as they were.
//!
//! Routes: `GET /api/v1/nonce` hands out a nonce (only while
//! `hub.allow_remote` is set); `GET /api/v1/members` lists the team's
//! passkeys; `POST /api/v1/members` registers one and
//! `DELETE /api/v1/members/<credential>` revokes it (`X-Hive-Members: yes`).

use crate::httpx::{Request, ResponseWriter};
use crate::server::{Handler, write_error, write_json};
use hive_runner::webauthn;
use serde_json::{Value, json};

/// The confirmation header a members write carries.
pub(crate) const MEMBERS_HEADER: &str = "X-Hive-Members";

/// The path and query a write's challenge binds, as the browser sent them.
pub(crate) fn signed_path(r: &Request) -> String {
    match r.url.query() {
        Some(q) => format!("{}?{q}", r.url.path()),
        None => r.url.path().to_string(),
    }
}

fn forbidden(w: &mut ResponseWriter, what: &str, why: &str, fix: &str) {
    write_error(w, 403, what.into(), why.into(), fix.into());
}

/// The headers of a passkey-signed write, decoded; None when any is missing
/// or not base64url.
fn assertion_of(r: &Request) -> Option<(String, webauthn::Assertion)> {
    let nonce = r.header(webauthn::NONCE_HEADER);
    let credential_id = r.header(webauthn::CREDENTIAL_HEADER);
    if nonce.is_empty() || credential_id.is_empty() {
        return None;
    }
    let authenticator_data = webauthn::from_b64url(&r.header(webauthn::AUTHENTICATOR_HEADER))?;
    let client_data_json = webauthn::from_b64url(&r.header(webauthn::CLIENT_DATA_HEADER))?;
    let signature = webauthn::from_b64url(&r.header(webauthn::SIGNATURE_HEADER))?;
    Some((
        nonce,
        webauthn::Assertion {
            credential_id,
            authenticator_data,
            client_data_json,
            signature,
        },
    ))
}

/// Whether a write route needs the team's owner rather than any member: settings and members,
/// and every write that rents past a rental limit or raises one (PR 314 review) - the rent
/// dialog's `compute/executors` with `"raise": true`, `compute/<id>/rent`, `compute/<id>/approve`
/// and `compute/refile-today`. A member's own request under the limits stays any member's.
fn owner_route(route: &str, body: &[u8]) -> bool {
    if route == "settings" || route == "members" || route.starts_with("members/") {
        return true;
    }
    let parts: Vec<&str> = route.trim_matches('/').split('/').collect();
    match parts.as_slice() {
        ["compute", "refile-today"] => true,
        ["compute", _, "rent" | "approve"] => true,
        ["compute", "executors"] => raises_a_limit(body),
        _ => false,
    }
}

/// Whether a rent dialog's body asks to raise a limit. A body that does not read as a JSON
/// object, or a `raise` the route would refuse, is taken as one that does: the owner's passkey
/// is asked for rather than a member's write let through on a doubt.
fn raises_a_limit(body: &[u8]) -> bool {
    match serde_json::from_slice::<crate::additions::JsonMap>(body) {
        Ok(payload) => {
            crate::machines::payload_bool(&payload, "raise", false).is_none_or(|raise| raise)
        }
        Err(_) => true,
    }
}

impl Handler {
    /// Whether a write came from the owner: on the loopback listener always; through the front
    /// only when the passkey that signed it - [`Handler::front_gate`] checked the signature
    /// already - is the team owner's.
    pub(crate) fn owners_write(&self, r: &Request) -> bool {
        if !r.via_front {
            return true;
        }
        let Some(store) = self.store.as_deref() else {
            return false;
        };
        let credential = r.header(webauthn::CREDENTIAL_HEADER);
        matches!(store.member(&credential), Ok(Some(m)) if m.active() && m.role == "owner")
    }

    /// The front's gate: true when the request may go on to its route.
    /// Requests on the loopback listener always may; the rest are checked as
    /// the module says and refused with 403 here.
    pub(crate) fn front_gate(&self, w: &mut ResponseWriter, r: &Request) -> bool {
        if !r.via_front {
            return true;
        }
        let Some(store) = self.store.as_deref() else {
            forbidden(
                w,
                "This console is not open to the front",
                "No Hive store is open, so hub.allow_remote cannot be read.",
                "Run hive init, then restart the hub.",
            );
            return false;
        };
        let Some(origin) = store.hub_allow_remote() else {
            forbidden(
                w,
                "This console is not open to the front",
                "hub.allow_remote is empty, or an agent set it; only the owner opens the console to the front.",
                "Set hub.allow_remote to the front's origin on this machine's console.",
            );
            return false;
        };
        // The relay's own Host rule, the console's (`Handler::host_gate`)
        // with the front's name in place of the owner's list: a web page
        // whose name was re-pointed at 127.0.0.1 reaches this port too, and
        // its same-origin reads carry no Origin header for the check below.
        let front_host = webauthn::origin_host(&origin).unwrap_or_default();
        let name = crate::server::host_name(&r.host());
        if !crate::server::local_name(&name) && (name.is_empty() || name != front_host) {
            let what = if name.is_empty() {
                "This relay needs a Host header".to_string()
            } else {
                format!("This relay does not answer to the name {name}")
            };
            forbidden(
                w,
                &what,
                "A web page can point a name of its own at 127.0.0.1 to reach the relay port on this machine (DNS rebinding), so the relay answers only to the front's own name, to localhost and to IP addresses.",
                &format!(
                    "Have the front send its own name ({front_host}) or 127.0.0.1 as the Host header."
                ),
            );
            return false;
        }
        let sent_origin = r.header("Origin");
        if !sent_origin.is_empty() && sent_origin.trim_end_matches('/') != origin {
            forbidden(
                w,
                "Cross-origin requests are not allowed",
                "The browser names another origin than the front's (hub.allow_remote).",
                "Open the console at the front's address.",
            );
            return false;
        }
        if r.method == "GET" || r.method == "HEAD" || r.method == "OPTIONS" {
            return true;
        }
        let Some((nonce, assertion)) = assertion_of(r) else {
            forbidden(
                w,
                "Unsigned write refused",
                "A write that reaches this brain through the front must carry a member's passkey signature.",
                "Sign the write with your passkey: GET /api/v1/nonce, then send the assertion headers.",
            );
            return false;
        };
        let member = match store.member(&assertion.credential_id) {
            Ok(Some(m)) if m.active() => m,
            Ok(_) => {
                forbidden(
                    w,
                    "Unknown passkey",
                    "No active member of this team holds that passkey.",
                    "Register the passkey with the team, or ask the owner to restore it.",
                );
                return false;
            }
            Err(err) => {
                crate::addition_failure(w, err);
                return false;
            }
        };
        let Some(public_key) = webauthn::from_b64url(&member.public_key) else {
            forbidden(
                w,
                "Unknown passkey",
                "The member's registered key is not base64url.",
                "Register the passkey again.",
            );
            return false;
        };
        let challenge = webauthn::write_challenge(&nonce, &r.method, &signed_path(r), &r.body);
        let expected = webauthn::Expected {
            challenge: &challenge,
            origin: &origin,
            public_key: &public_key,
            sign_count: u32::try_from(member.sign_count).unwrap_or(u32::MAX),
        };
        let count = match webauthn::verify_assertion(&assertion, &expected) {
            Ok(count) => count,
            Err(refusal) => {
                forbidden(
                    w,
                    "Passkey signature refused",
                    &format!("{refusal}."),
                    "Sign the write again with your registered passkey.",
                );
                return false;
            }
        };
        match store.nonce_take(&nonce) {
            Ok(true) => {}
            Ok(false) => {
                forbidden(
                    w,
                    "Nonce refused",
                    "The nonce was already spent, has expired, or was not issued by this brain; a signed write is accepted once.",
                    "Ask for a new nonce (GET /api/v1/nonce) and sign again.",
                );
                return false;
            }
            Err(err) => {
                crate::addition_failure(w, err);
                return false;
            }
        }
        match store.member_used(&member.credential_id, i64::from(count)) {
            Ok(true) => {}
            Ok(false) => {
                forbidden(
                    w,
                    "Passkey signature refused",
                    "The passkey's counter did not move forward; another write used it first.",
                    "Sign the write again.",
                );
                return false;
            }
            Err(err) => {
                crate::addition_failure(w, err);
                return false;
            }
        }
        let route = r.path().strip_prefix("/api/v1/").unwrap_or("");
        if owner_route(route, &r.body) && member.role != "owner" {
            forbidden(
                w,
                "Only the team's owner can do this",
                "Settings and members change, and a machine is rented past a limit, only with an owner's passkey.",
                "Ask the team's owner.",
            );
            return false;
        }
        true
    }

    /// `/api/v1/nonce` and `/api/v1/members[/<credential>]`; false when the
    /// route is another.
    pub(crate) fn remote_api(&self, w: &mut ResponseWriter, r: &Request, route: &str) -> bool {
        let parts: Vec<&str> = route.trim_matches('/').split('/').collect();
        match parts.as_slice() {
            ["nonce"] => self.nonce_route(w, r),
            ["members"] => self.members_route(w, r, None),
            ["members", credential] => self.members_route(w, r, Some(credential)),
            _ => return false,
        }
        true
    }

    fn nonce_route(&self, w: &mut ResponseWriter, r: &Request) {
        if !crate::addition_method(w, r, vec!["GET".into()]) {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            crate::addition_failure(w, anyhow::anyhow!("store is not available"));
            return;
        };
        let Some(origin) = store.hub_allow_remote() else {
            forbidden(
                w,
                "Nonces are only issued while the console is open to the front",
                "hub.allow_remote is empty; writes on this machine need no passkey.",
                "Set hub.allow_remote to the front's origin first.",
            );
            return;
        };
        let nonce = webauthn::new_nonce();
        match store.nonce_issue(&nonce, webauthn::NONCE_SECONDS) {
            Ok(Some(expires_at)) => {
                w.header_mut().insert(
                    hyper::header::CACHE_CONTROL,
                    hyper::header::HeaderValue::from_static("no-store"),
                );
                write_json(
                    w,
                    200,
                    json!({
                        "nonce": nonce,
                        "expires_at": expires_at,
                        "origin": origin,
                        "rp_id": webauthn::origin_host(&origin).unwrap_or_default(),
                        "challenge": format!(
                            "base64url(SHA-256(\"{}\\n\" + nonce + \"\\n\" + METHOD + \" \" + path?query + \"\\n\" + body))",
                            webauthn::CHALLENGE_DOMAIN
                        ),
                    }),
                );
            }
            Ok(None) => write_error(
                w,
                429,
                "Too many unspent nonces".into(),
                "The brain keeps a bounded number of nonces at once.".into(),
                "Wait a few minutes for unspent ones to expire.".into(),
            ),
            Err(err) => crate::addition_failure(w, err),
        }
    }

    fn members_route(&self, w: &mut ResponseWriter, r: &Request, credential: Option<&str>) {
        let methods = if credential.is_some() {
            vec!["DELETE".to_string()]
        } else {
            vec!["GET".to_string(), "POST".to_string()]
        };
        if !crate::addition_method(w, r, methods) {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            crate::addition_failure(w, anyhow::anyhow!("store is not available"));
            return;
        };
        if r.method == "GET" {
            match store.members() {
                Ok(members) => write_json(w, 200, json!({ "members": members })),
                Err(err) => crate::addition_failure(w, err),
            }
            return;
        }
        if !crate::server::same_origin_write(w, r) {
            return;
        }
        if r.header(MEMBERS_HEADER) != "yes" {
            forbidden(
                w,
                &format!("missing {MEMBERS_HEADER}: yes"),
                "An explicit confirmation header is required.",
                "Confirm the change in the hub.",
            );
            return;
        }
        let actor = "human:dashboard";
        if let Some(credential) = credential {
            match store.member_revoke(actor, credential) {
                Ok(true) => write_json(w, 200, json!({ "revoked": credential })),
                Ok(false) => crate::addition_not_found(w),
                Err(err) => crate::addition_failure(w, err),
            }
            return;
        }
        let (payload, ok) = crate::server::read_payload(w, r);
        if !ok {
            return;
        }
        let input: hive_store::MemberInput =
            match serde_json::from_value(Value::Object(payload.into_iter().collect())) {
                Ok(input) => input,
                Err(err) => {
                    write_error(
                        w,
                        400,
                        format!("not a member: {err}"),
                        "A member is credential_id, member, name, role and public_key.".into(),
                        "Send the passkey's id and its SubjectPublicKeyInfo key, base64url.".into(),
                    );
                    return;
                }
            };
        if let Some(key) = webauthn::from_b64url(input.public_key.trim())
            && hive_runner::p256::PublicKey::parse(&key).is_none()
        {
            write_error(
                w,
                400,
                "the public key is not an ES256 (P-256) key".into(),
                "Passkey-signed writes verify ES256 signatures only.".into(),
                "Register the passkey with algorithm -7 (ES256).".into(),
            );
            return;
        }
        match store.member_put(actor, &input) {
            Ok(member) => write_json(w, 200, json!({ "member": member })),
            Err(err) if err.downcast_ref::<hive_store::InputError>().is_some() => write_error(
                w,
                400,
                err.to_string(),
                "The member does not satisfy the members table's rule.".into(),
                "Send credential_id, member, role (owner or member) and public_key.".into(),
            ),
            Err(err) => crate::addition_failure(w, err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::httpx::HandlerFn;
    use crate::test_helpers_http::{additions_store, handler};
    use hive_runner::{p256, webauthn::b64url};
    use std::sync::Arc;

    const FRONT: &str = "https://team.example.com";
    const ALICE: [u8; 32] = [7; 32];
    const MALLORY: [u8; 32] = [9; 32];

    fn spki(secret: &[u8; 32]) -> String {
        b64url(&p256::public_key_for_tests(secret).unwrap().to_spki())
    }

    /// A hub whose front relay listener is open for `FRONT`, with Alice's
    /// passkey registered as `role`.
    fn open_hub(
        role: &str,
    ) -> (
        crate::test_helpers_http::TestStore,
        crate::test_helpers::TestServer,
    ) {
        let s = additions_store(false);
        s.set_setting("human", "hub.allow_remote", FRONT).unwrap();
        s.member_put(
            "human",
            &hive_store::MemberInput {
                credential_id: "alice-key".into(),
                member: "user_alice".into(),
                name: "Alice".into(),
                role: role.into(),
                public_key: spki(&ALICE),
            },
        )
        .unwrap();
        let h = Arc::new(handler(&s));
        let f: HandlerFn = Arc::new(move |w, r| h.serve_http(w, r));
        let srv = crate::test_helpers::TestServer::start_on(f, true);
        (s, srv)
    }

    fn nonce(srv: &crate::test_helpers::TestServer) -> String {
        let resp = crate::test_helpers::client()
            .get(format!("{}/api/v1/nonce", srv.url))
            .header("Host", "team.example.com")
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        let body: Value = resp.json().unwrap();
        assert_eq!(body["origin"], FRONT);
        assert_eq!(body["rp_id"], "team.example.com");
        body["nonce"].as_str().unwrap().to_string()
    }

    /// Sends a write through the front, signed by `secret` as `credential`
    /// over `nonce` (unsigned when `secret` is None).
    fn write(
        srv: &crate::test_helpers::TestServer,
        path: &str,
        body: &str,
        signed: Option<(&[u8; 32], &str, &str, u32)>,
    ) -> (u16, Value) {
        let mut req = crate::test_helpers::client()
            .post(format!("{}{path}", srv.url))
            .header("Host", "team.example.com")
            .header("Origin", FRONT)
            .header("Content-Type", "application/json")
            .header("X-Hive-Settings", "yes")
            .body(body.to_string());
        if let Some((secret, credential, nonce, count)) = signed {
            let challenge = webauthn::write_challenge(nonce, "POST", path, body.as_bytes());
            let a = webauthn::assertion_for_tests(secret, credential, FRONT, &challenge, count);
            req = req
                .header(webauthn::NONCE_HEADER, nonce)
                .header(webauthn::CREDENTIAL_HEADER, credential)
                .header(
                    webauthn::AUTHENTICATOR_HEADER,
                    b64url(&a.authenticator_data),
                )
                .header(webauthn::CLIENT_DATA_HEADER, b64url(&a.client_data_json))
                .header(webauthn::SIGNATURE_HEADER, b64url(&a.signature));
        }
        let resp = req.send().unwrap();
        let status = resp.status().as_u16();
        (status, resp.json().unwrap_or(Value::Null))
    }

    const SETTING: &str = r#"{"name":"daemon.pass_seconds","value":"90"}"#;

    #[test]
    fn a_passkey_signed_write_through_the_front_is_accepted_once() {
        let _serial = crate::test_helpers::serial();
        let (s, srv) = open_hub("owner");
        let n = nonce(&srv);
        let (status, body) = write(
            &srv,
            "/api/v1/settings",
            SETTING,
            Some((&ALICE, "alice-key", &n, 1)),
        );
        assert_eq!(status, 200, "{body}");
        assert_eq!(s.settings().unwrap()["daemon.pass_seconds"], "90");
        let alice = s.member("alice-key").unwrap().unwrap();
        assert_eq!(alice.sign_count, 1);
        assert!(!alice.last_used_at.is_empty());

        // The same signed write again: the nonce is spent.
        let (status, body) = write(
            &srv,
            "/api/v1/settings",
            SETTING,
            Some((&ALICE, "alice-key", &n, 2)),
        );
        assert_eq!(status, 403, "{body}");
        assert_eq!(body["error"], "Nonce refused");
    }

    /// web/hub/src/lib/passkey-wire.json: one signed write the console's
    /// passkey-state.ts made (its npm test checks the same file). The brain
    /// must compute the same challenge and accept the write as sent.
    fn wire_fixture() -> Value {
        let path = hive_testutil::manifest_dir(env!("CARGO_MANIFEST_DIR"))
            .join("../../web/hub/src/lib/passkey-wire.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        serde_json::from_str(&text).expect("passkey-wire.json is JSON")
    }

    #[test]
    fn the_consoles_signed_write_fixture_is_accepted_by_the_brain() {
        let _serial = crate::test_helpers::serial();
        let f = wire_fixture();
        let str_of = |v: &Value| v.as_str().unwrap().to_string();
        let (nonce, method, path, body) = (
            str_of(&f["write"]["nonce"]),
            str_of(&f["write"]["method"]),
            str_of(&f["write"]["path"]),
            str_of(&f["write"]["body"]),
        );
        assert_eq!(f["nonce_route"], "GET /api/v1/nonce");
        assert_eq!(f["origin"], FRONT);

        // The challenge: the same bytes, the same hash.
        let challenge = webauthn::write_challenge(&nonce, &method, &path, body.as_bytes());
        assert_eq!(b64url(&challenge), f["challenge"], "the challenge differs");
        let mut bytes =
            format!("{}\n{nonce}\n{method} {path}\n", webauthn::CHALLENGE_DOMAIN).into_bytes();
        bytes.extend_from_slice(body.as_bytes());
        assert_eq!(b64url(&bytes), f["signed_bytes"], "the signed bytes differ");

        // The registration: the members route takes the console's body as is.
        let s = additions_store(false);
        s.set_setting("human", "hub.allow_remote", FRONT).unwrap();
        let member = f["member"]["body"].to_string();
        let confirm: Vec<(&str, &str)> = f["member"]["headers"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str().unwrap()))
            .collect();
        assert_eq!(confirm, [(MEMBERS_HEADER, "yes")]);
        let (status, reply, _) = crate::test_helpers_http::request(
            handler(&s),
            "POST",
            "/api/v1/members",
            &member,
            &confirm,
        );
        assert_eq!(status, 200, "{reply:?}");
        let credential = str_of(&f["member"]["body"]["credential_id"]);
        assert_eq!(s.member(&credential).unwrap().unwrap().role, "owner");

        // The write through the front, with the five headers as the console
        // sent them, over a nonce this brain issued.
        assert!(
            s.nonce_issue(&nonce, webauthn::NONCE_SECONDS)
                .unwrap()
                .is_some()
        );
        let headers = f["headers"].as_object().unwrap();
        let mut names: Vec<&str> = headers.keys().map(String::as_str).collect();
        names.sort_unstable();
        let mut want = [
            webauthn::NONCE_HEADER,
            webauthn::CREDENTIAL_HEADER,
            webauthn::AUTHENTICATOR_HEADER,
            webauthn::CLIENT_DATA_HEADER,
            webauthn::SIGNATURE_HEADER,
        ];
        want.sort_unstable();
        assert_eq!(names, want, "the header names differ");
        let h = Arc::new(handler(&s));
        let hf: HandlerFn = Arc::new(move |w, r| h.serve_http(w, r));
        let srv = crate::test_helpers::TestServer::start_on(hf, true);
        let send = || {
            let mut req = crate::test_helpers::client()
                .request(
                    reqwest::Method::from_bytes(method.as_bytes()).unwrap(),
                    format!("{}{path}", srv.url),
                )
                .header("Origin", FRONT)
                .body(body.clone());
            for (k, v) in f["write"]["headers"]
                .as_object()
                .unwrap()
                .iter()
                .chain(headers)
            {
                req = req.header(k.as_str(), v.as_str().unwrap());
            }
            let resp = req.send().unwrap();
            (
                resp.status().as_u16(),
                resp.json::<Value>().unwrap_or(Value::Null),
            )
        };
        let (status, reply) = send();
        assert_eq!(
            status, 200,
            "the brain refused the console's write: {reply}"
        );
        assert_eq!(s.settings().unwrap()["daemon.pass_seconds"], "90");
        assert_eq!(
            s.member(&credential).unwrap().unwrap().sign_count,
            f["sign_count"].as_i64().unwrap()
        );
        // Sent again, it is a replay.
        let (status, reply) = send();
        assert_eq!(status, 403, "{reply}");
    }

    #[test]
    fn a_replayed_assertion_is_refused() {
        let _serial = crate::test_helpers::serial();
        let (s, srv) = open_hub("owner");
        let n = nonce(&srv);
        let signed = Some((&ALICE, "alice-key", n.as_str(), 1));
        assert_eq!(write(&srv, "/api/v1/settings", SETTING, signed).0, 200);
        // The exact bytes again (same nonce, same counter): refused.
        let (status, body) = write(&srv, "/api/v1/settings", SETTING, signed);
        assert_eq!(status, 403, "{body}");
        assert!(body["why"].as_str().unwrap().contains("counter"), "{body}");
        // A fresh nonce with a counter that did not move is refused too.
        let fresh = nonce(&srv);
        let (status, body) = write(
            &srv,
            "/api/v1/settings",
            SETTING,
            Some((&ALICE, "alice-key", &fresh, 1)),
        );
        assert_eq!(status, 403, "{body}");
        assert_eq!(s.member("alice-key").unwrap().unwrap().sign_count, 1);
        // A nonce this brain never issued.
        let (status, body) = write(
            &srv,
            "/api/v1/settings",
            SETTING,
            Some((&ALICE, "alice-key", "made-up", 5)),
        );
        assert_eq!(status, 403, "{body}");
        assert_eq!(body["error"], "Nonce refused");
    }

    #[test]
    fn a_write_signed_with_another_key_is_refused() {
        let _serial = crate::test_helpers::serial();
        let (s, srv) = open_hub("owner");
        let n = nonce(&srv);
        let (status, body) = write(
            &srv,
            "/api/v1/settings",
            SETTING,
            Some((&MALLORY, "alice-key", &n, 1)),
        );
        assert_eq!(status, 403, "{body}");
        assert_eq!(body["error"], "Passkey signature refused");
        assert!(
            body["why"].as_str().unwrap().contains("does not verify"),
            "{body}"
        );
        assert_ne!(
            s.settings()
                .unwrap()
                .get("daemon.pass_seconds")
                .map(String::as_str),
            Some("90")
        );
        // The refused write did not spend the nonce or move the counter.
        assert_eq!(s.member("alice-key").unwrap().unwrap().sign_count, 0);
        // A signature over another body does not carry over to this one.
        let other = r#"{"name":"daemon.pass_seconds","value":"5"}"#;
        let challenge = webauthn::write_challenge(&n, "POST", "/api/v1/settings", other.as_bytes());
        let a = webauthn::assertion_for_tests(&ALICE, "alice-key", FRONT, &challenge, 1);
        let resp = crate::test_helpers::client()
            .post(format!("{}/api/v1/settings", srv.url))
            .header("Content-Type", "application/json")
            .header("X-Hive-Settings", "yes")
            .header(webauthn::NONCE_HEADER, &n)
            .header(webauthn::CREDENTIAL_HEADER, "alice-key")
            .header(
                webauthn::AUTHENTICATOR_HEADER,
                b64url(&a.authenticator_data),
            )
            .header(webauthn::CLIENT_DATA_HEADER, b64url(&a.client_data_json))
            .header(webauthn::SIGNATURE_HEADER, b64url(&a.signature))
            .body(SETTING)
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 403);
        // A passkey nobody registered.
        let (status, body) = write(
            &srv,
            "/api/v1/settings",
            SETTING,
            Some((&MALLORY, "mallory-key", &n, 1)),
        );
        assert_eq!(status, 403, "{body}");
        assert_eq!(body["error"], "Unknown passkey");
        // A revoked member's passkey stops working.
        assert!(s.member_revoke("human", "alice-key").unwrap());
        let (status, body) = write(
            &srv,
            "/api/v1/settings",
            SETTING,
            Some((&ALICE, "alice-key", &n, 1)),
        );
        assert_eq!(status, 403, "{body}");
        assert_eq!(body["error"], "Unknown passkey");
    }

    #[test]
    fn an_unsigned_write_to_a_remote_allowed_hub_gets_403() {
        let _serial = crate::test_helpers::serial();
        let (s, srv) = open_hub("owner");
        let (status, body) = write(&srv, "/api/v1/settings", SETTING, None);
        assert_eq!(status, 403, "{body}");
        assert_eq!(body["error"], "Unsigned write refused");
        assert_ne!(
            s.settings()
                .unwrap()
                .get("daemon.pass_seconds")
                .map(String::as_str),
            Some("90")
        );
        // Reads pass through the front.
        let resp = crate::test_helpers::client()
            .get(format!("{}/api/v1/settings", srv.url))
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        // Loopback-only routes refuse the front whatever Host it sends.
        let resp = crate::test_helpers::client()
            .get(format!("{}/api/v1/folders", srv.url))
            .header("Host", "127.0.0.1:8765")
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 403);
        // A browser on another origin is refused outright.
        let resp = crate::test_helpers::client()
            .get(format!("{}/api/v1/settings", srv.url))
            .header("Origin", "https://evil.example.net")
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 403);
    }

    #[test]
    fn the_front_is_closed_until_the_owner_sets_allow_remote() {
        let _serial = crate::test_helpers::serial();
        let (s, srv) = open_hub("owner");
        // An agent's value does not count.
        s.set_setting("cinder-47", "hub.allow_remote", FRONT)
            .unwrap();
        for path in ["/api/v1/settings", "/api/v1/nonce"] {
            let resp = crate::test_helpers::client()
                .get(format!("{}{path}", srv.url))
                .send()
                .unwrap();
            assert_eq!(resp.status().as_u16(), 403, "{path}");
        }
        // Closed, the relay says so whatever name the request came under.
        let resp = crate::test_helpers::client()
            .get(format!("{}/api/v1/settings", srv.url))
            .header("Host", "evil.example:8766")
            .send()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 403);
        let body: Value = resp.json().unwrap();
        assert_eq!(body["error"], "This console is not open to the front");
        // The loopback listener issues no nonce while the console is closed.
        s.set_setting("human", "hub.allow_remote", "").unwrap();
        let (status, body, _) =
            crate::test_helpers_http::request(handler(&s), "GET", "/api/v1/nonce", "", &[]);
        assert_eq!(status, 403, "{body:?}");
    }

    /// DNS rebinding reaches the relay port as it reached the console's: a
    /// web page whose name was re-pointed at 127.0.0.1 sends same-origin
    /// reads there, with no Origin header and its own name as Host.
    #[test]
    fn the_relay_answers_only_to_the_fronts_name_localhost_and_ip_addresses() {
        use std::io::{Read, Write};
        let _serial = crate::test_helpers::serial();
        let (_s, srv) = open_hub("owner");
        let get = |path: &str, host: &str| {
            let resp = crate::test_helpers::client()
                .get(format!("{}{path}", srv.url))
                .header("Host", host)
                .send()
                .unwrap();
            (
                resp.status().as_u16(),
                resp.json::<Value>().unwrap_or(Value::Null),
            )
        };
        let why = "A web page can point a name of its own at 127.0.0.1 to reach the relay port on this machine (DNS rebinding), so the relay answers only to the front's own name, to localhost and to IP addresses.";
        let fix =
            "Have the front send its own name (team.example.com) or 127.0.0.1 as the Host header.";
        for path in ["/", "/api/v1/settings"] {
            let (status, body) = get(path, "evil.example:8766");
            assert_eq!(status, 403, "{path}");
            assert_eq!(
                body,
                json!({
                    "error": "This relay does not answer to the name evil.example",
                    "why": why,
                    "fix": fix,
                }),
                "{path}"
            );
        }
        // The front's own name, whatever its case and port, and what is
        // never a rebound name pass as before.
        for host in [
            "team.example.com",
            "TEAM.Example.com:443",
            "localhost:8766",
            "relay.localhost",
            "127.0.0.1:8766",
            "[::1]:8766",
            "10.0.0.7",
        ] {
            for path in ["/", "/api/v1/settings"] {
                let resp = crate::test_helpers::client()
                    .get(format!("{}{path}", srv.url))
                    .header("Host", host)
                    .send()
                    .unwrap();
                assert_eq!(resp.status().as_u16(), 200, "{host} {path}");
            }
        }
        // The front's name as it is, nothing wider.
        for host in [
            "sub.team.example.com",
            "team.example.com.evil.example",
            "example.com",
            "[team.example.com]",
        ] {
            assert_eq!(get("/api/v1/settings", host).0, 403, "{host}");
        }
        // No Host at all.
        let mut stream =
            std::net::TcpStream::connect(srv.url.trim_start_matches("http://")).unwrap();
        stream
            .write_all(b"GET /api/v1/settings HTTP/1.1\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        assert!(answer.starts_with("HTTP/1.1 403 "), "{answer}");
        let body: Value = answer
            .find('{')
            .zip(answer.rfind('}'))
            .and_then(|(start, end)| serde_json::from_str(&answer[start..=end]).ok())
            .unwrap_or_else(|| panic!("{answer}"));
        assert_eq!(
            body,
            json!({
                "error": "This relay needs a Host header",
                "why": why,
                "fix": fix,
            })
        );
    }

    #[test]
    fn settings_and_members_through_the_front_need_the_owner() {
        let _serial = crate::test_helpers::serial();
        let (s, srv) = open_hub("member");
        let n = nonce(&srv);
        let (status, body) = write(
            &srv,
            "/api/v1/settings",
            SETTING,
            Some((&ALICE, "alice-key", &n, 1)),
        );
        assert_eq!(status, 403, "{body}");
        assert_eq!(body["error"], "Only the team's owner can do this");
        assert_ne!(
            s.settings()
                .unwrap()
                .get("daemon.pass_seconds")
                .map(String::as_str),
            Some("90")
        );
    }

    /// PR 314 review item 2: through the front, renting past a limit or raising one takes the
    /// team owner's passkey. A member who is not the owner gets 403 on the rent dialog's
    /// `"raise": true`, on `compute/<id>/rent`, `compute/<id>/approve` and
    /// `compute/refile-today`, and the limit stays as it was; the member's own request without a
    /// raise passes the gate but, over a limit, waits for the owner. Before, any member's
    /// signed write was taken as the owner's and raised the limit.
    #[test]
    fn a_member_who_is_not_the_owner_cannot_rent_past_a_limit_through_the_front() {
        let _serial = crate::test_helpers::serial();
        let (s, srv) = open_hub("member");
        crate::test_helpers_http::additions_project(&s, "one");
        for (k, v) in [
            ("compute.auto_approve", "1"),
            ("compute.max_vms", "0"),
            ("compute.max_hours_per_day", "0"),
        ] {
            s.set_setting("human", k, v).unwrap();
        }
        let r = hive_tasks::request_compute(&s, "one", "one-lead", "small", 2, "agent's").unwrap();
        let hit = hive_tasks::get_caps(&s).unwrap().limit_hit(2).unwrap();
        hive_tasks::not_rented(&s, &r.id, &hit, "hive").unwrap();
        let other = hive_tasks::request_compute(&s, "one", "one-lead", "small", 2, "more").unwrap();
        let rent = format!("/api/v1/compute/{}/rent", r.id);
        let approve = format!("/api/v1/compute/{}/approve", other.id);
        let writes: [(&str, &str); 4] = [
            (
                "/api/v1/compute/executors",
                r#"{"mode":"request","project":"one","size":"small","hours":3,"reason":"mine","raise":true}"#,
            ),
            (&rent, r#"{"raise":true}"#),
            (&approve, r#"{"words":"go"}"#),
            ("/api/v1/compute/refile-today", "{}"),
        ];
        for (count, (path, body)) in (1u32..).zip(writes) {
            let n = nonce(&srv);
            let (status, reply) = write(&srv, path, body, Some((&ALICE, "alice-key", &n, count)));
            assert_eq!(status, 403, "{path}: {reply}");
            assert_eq!(
                reply["error"], "Only the team's owner can do this",
                "{path}"
            );
        }
        assert_eq!(s.setting("compute.max_hours_per_day", ""), "0");
        assert_eq!(s.setting("compute.max_vms", ""), "0");
        let n = |id: &str| hive_tasks::get_compute(&s, id).unwrap();
        assert_eq!(n(&other.id).status, "pending");
        assert!(hive_tasks::open_limit_hit(&s, &r.id).unwrap().is_some());
        // The member's own request with no raise passes the gate, and over the limits it waits.
        let nn = nonce(&srv);
        let (status, reply) = write(
            &srv,
            "/api/v1/compute/executors",
            r#"{"mode":"request","project":"one","size":"small","hours":3,"reason":"mine"}"#,
            Some((&ALICE, "alice-key", &nn, 5)),
        );
        assert_eq!(status, 201, "{reply}");
        assert_eq!(reply["request"]["status"], "pending", "{reply}");
        assert_eq!(s.setting("compute.max_hours_per_day", ""), "0");
    }

    /// The owner's passkey through the front still rents past a limit and raises it.
    #[test]
    fn the_owner_rents_past_a_limit_through_the_front() {
        let _serial = crate::test_helpers::serial();
        let (s, srv) = open_hub("owner");
        crate::test_helpers_http::additions_project(&s, "one");
        s.set_setting("human", "compute.max_hours_per_day", "2")
            .unwrap();
        s.set_setting("human", "compute.max_vms", "none").unwrap();
        let n = nonce(&srv);
        let (status, reply) = write(
            &srv,
            "/api/v1/compute/executors",
            r#"{"mode":"request","project":"one","size":"small","hours":3,"reason":"mine","raise":true}"#,
            Some((&ALICE, "alice-key", &n, 1)),
        );
        assert_eq!(status, 201, "{reply}");
        assert_eq!(reply["request"]["status"], "approved", "{reply}");
        assert_eq!(s.setting("compute.max_hours_per_day", ""), "3");
    }

    #[test]
    fn the_owner_registers_and_revokes_members_on_this_machine() {
        let _serial = crate::test_helpers::serial();
        let s = additions_store(false);
        let h = || handler(&s);
        let member = json!({
            "credential_id": "bob-key", "member": "user_bob", "name": "Bob",
            "role": "member", "public_key": spki(&MALLORY),
        })
        .to_string();
        let (status, _, _) =
            crate::test_helpers_http::request(h(), "POST", "/api/v1/members", &member, &[]);
        assert_eq!(status, 403, "the confirmation header is required");
        let confirm = [(MEMBERS_HEADER, "yes")];
        let (status, body, _) =
            crate::test_helpers_http::request(h(), "POST", "/api/v1/members", &member, &confirm);
        assert_eq!(status, 200, "{body:?}");
        assert_eq!(body["member"]["role"], "member");
        let bad = json!({"credential_id": "x", "member": "u", "role": "member", "public_key": b64url(b"not a key")}).to_string();
        let (status, _, _) =
            crate::test_helpers_http::request(h(), "POST", "/api/v1/members", &bad, &confirm);
        assert_eq!(status, 400);
        let (status, body, _) =
            crate::test_helpers_http::request(h(), "GET", "/api/v1/members", "", &[]);
        assert_eq!(status, 200);
        assert_eq!(body["members"].as_array().unwrap().len(), 1);
        let (status, _, _) = crate::test_helpers_http::request(
            h(),
            "DELETE",
            "/api/v1/members/bob-key",
            "",
            &confirm,
        );
        assert_eq!(status, 200);
        assert!(!s.member("bob-key").unwrap().unwrap().active());
    }
}
