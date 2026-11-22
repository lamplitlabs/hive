//! Machines that dial in (ADR-070 step 2).
//!
//! The owner's side, guarded like every console write:
//!
//! - `POST /api/v1/machines/join-codes` issues a six-word join code, good
//!   for ten minutes and one join, and the `hive-hand join` line to run on
//!   the machine.
//! - `POST /api/v1/machines/devices/{device-or-executor}/revoke` drops a
//!   device's key and sessions and removes its executor; the hand's next
//!   call is refused.
//!
//! The hand's side, `POST /api/v1/hands/{join,nonce,connect,poll,result}`,
//! is [`hive_runner::dial::hand_api`]: a hand is not a browser, so it
//! proves itself with the join code once and its device key after that.
//! The console's Host rule (`Handler::host_gate`) leaves these routes out
//! for that reason: a hand may reach the hub through a proxy, under a name
//! of the owner's.

use crate::additions::{ADDITION_WRITES, addition_failure, addition_method, addition_not_found};
use crate::httpx::{Request, ResponseWriter};
use crate::server::{Handler, write_error, write_json};
use serde_json::json;

impl Handler {
    pub(crate) fn addition_machines(
        &self,
        w: &mut ResponseWriter,
        r: &Request,
        parts: Vec<String>,
    ) {
        let join_codes = parts.len() == 2 && parts[1] == "join-codes";
        let revoke = parts.len() == 4 && parts[1] == "devices" && parts[3] == "revoke";
        if !join_codes && !revoke {
            addition_not_found(w);
            return;
        }
        if !addition_method(w, r, vec!["POST".into()])
            || !self.addition_available(w, r, vec!["hand_devices".into(), "hand_join_codes".into()])
            || !crate::server::same_origin_write(w, r)
        {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            return;
        };
        let _lock = ADDITION_WRITES.lock().unwrap_or_else(|e| e.into_inner());
        let now = store.now();
        if join_codes {
            match hive_runner::dial::issue_join_code(store, &now, "human") {
                Ok(code) => {
                    let host = r.host();
                    let brain = if host.is_empty() {
                        String::new()
                    } else {
                        let scheme = match r.header("X-Forwarded-Proto").as_str() {
                            "https" => "https",
                            _ => "http",
                        };
                        format!("{scheme}://{host}")
                    };
                    let command = if brain.is_empty() {
                        format!("hive-hand join {}", code.code)
                    } else {
                        format!("hive-hand join --brain {brain} {}", code.code)
                    };
                    write_json(
                        w,
                        200,
                        json!({
                            "code": code.code,
                            "expires_at": code.expires_at,
                            "minutes": hive_runner::dial::JOIN_CODE_MINUTES,
                            "brain": brain,
                            "command": command,
                        }),
                    );
                }
                Err(e) => addition_failure(w, e.to_string()),
            }
            return;
        }
        match hive_runner::dial::revoke(store, &now, &parts[2], "human") {
            Ok(device) => write_json(w, 200, json!({"device": device})),
            Err(e) => match e.downcast_ref::<hive_runner::dial::DialError>() {
                Some(d) => write_error(
                    w,
                    d.status as i64,
                    d.what.clone(),
                    d.what.clone(),
                    d.fix.clone(),
                ),
                None => addition_failure(w, e.to_string()),
            },
        }
    }

    pub(crate) fn addition_hands(&self, w: &mut ResponseWriter, r: &Request, parts: Vec<String>) {
        if parts.len() != 2 {
            addition_not_found(w);
            return;
        }
        if !addition_method(w, r, vec!["POST".into()])
            || !self.addition_available(w, r, vec!["hand_devices".into(), "hand_calls".into()])
        {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            return;
        };
        let (status, body) = hive_runner::dial::hand_api(store, &parts[1], &r.body, &r.ctx);
        write_json(w, i64::from(status), body);
    }
}

#[cfg(test)]
mod tests {
    use crate::test_helpers_http::{additions_store, handler, request};
    use serde_json::Value;

    fn post(
        h: crate::server::Handler,
        path: &str,
        body: &str,
    ) -> (u16, serde_json::Map<String, Value>) {
        let (status, map, _) = request(h, "POST", path, body, &[]);
        (status, map.into_iter().collect())
    }

    #[test]
    fn join_codes_are_issued_to_the_console_and_refused_cross_site() {
        let s = additions_store(false);
        let (status, body) = post(handler(&s), "/api/v1/machines/join-codes", "{}");
        assert_eq!(status, 200, "{body:?}");
        let code = body["code"].as_str().unwrap();
        assert_eq!(code.split('-').count(), 6, "six words: {code}");
        assert_eq!(body["expires_at"], "2026-09-06T12:10:00Z");
        let command = body["command"].as_str().unwrap();
        assert!(
            command.starts_with("hive-hand join --brain http://127.0.0.1:")
                && command.ends_with(code),
            "{command}"
        );
        let (status, _, _) = request(
            handler(&s),
            "POST",
            "/api/v1/machines/join-codes",
            "{}",
            &[("Origin", "https://evil.example")],
        );
        assert_eq!(status, 403, "a cross-site page cannot mint join codes");
        let (status, _, _) = request(handler(&s), "GET", "/api/v1/machines/join-codes", "", &[]);
        assert_eq!(status, 405);
    }

    #[test]
    fn a_hand_joins_through_the_hub_and_the_owner_revokes_it() {
        let s = additions_store(false);
        let (_, body) = post(handler(&s), "/api/v1/machines/join-codes", "{}");
        let code = body["code"].as_str().unwrap().replace('-', " ");
        let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFnK0ljcL0vNfwh0SJNWr3tkC2ANeKkLb9FRGmBwJrLO hand@box";
        let join = serde_json::json!({
            "code": code, "public_key": key, "name": "Studio Mac",
            "os": "macos", "arch": "arm64", "workers": 2, "workdir": "/Users/hive/hands",
        })
        .to_string();
        let (status, joined) = post(handler(&s), "/api/v1/hands/join", &join);
        assert_eq!(status, 200, "{joined:?}");
        assert_eq!(joined["executor"], "studio-mac");
        let device = joined["device"].as_str().unwrap().to_string();
        let (status, again) = post(handler(&s), "/api/v1/hands/join", &join);
        assert_eq!(status, 403, "a code works once: {again:?}");
        assert!(again["error"].as_str().unwrap().contains("already used"));

        let (status, _, compute) = request(handler(&s), "GET", "/api/v1/compute", "", &[]);
        assert_eq!(status, 200, "{compute}");
        let compute: Value = serde_json::from_str(&compute).unwrap();
        assert_eq!(compute["devices"][0]["id"], device.as_str());
        assert_eq!(compute["executors"][0]["kind"], "dial");

        let (status, revoked) = post(
            handler(&s),
            "/api/v1/machines/devices/studio-mac/revoke",
            "{}",
        );
        assert_eq!(status, 200, "{revoked:?}");
        assert_eq!(revoked["device"]["revoked_by"], "human");
        let (status, refused) = post(
            handler(&s),
            "/api/v1/hands/nonce",
            &serde_json::json!({"device": device}).to_string(),
        );
        assert_eq!(status, 403, "a revoked device cannot connect: {refused:?}");
        let (status, _) = post(
            handler(&s),
            "/api/v1/machines/devices/studio-mac/revoke",
            "{}",
        );
        assert_eq!(status, 404, "nothing left to revoke");
    }
}
