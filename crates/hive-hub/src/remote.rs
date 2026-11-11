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

/// Whether a write route needs the team's owner rather than any member.
fn owner_route(route: &str) -> bool {
    route == "settings" || route == "members" || route.starts_with("members/")
}

impl Handler {
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
        let sent_origin = r.header("Origin");
        if !sent_origin.is_empty() && sent_origin.trim_end_matches('/') != origin {
            forbidden(
                w,
                "Cross-origin requests are not allowed",
                "The browser names another origin than the team's front.",
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
        if owner_route(route) && member.role != "owner" {
            forbidden(
                w,
                "Only the team's owner can do this",
                "Settings and members change only with an owner's passkey.",
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
        // The loopback listener issues no nonce while the console is closed.
        s.set_setting("human", "hub.allow_remote", "").unwrap();
        let (status, body, _) =
            crate::test_helpers_http::request(handler(&s), "GET", "/api/v1/nonce", "", &[]);
        assert_eq!(status, 403, "{body:?}");
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
