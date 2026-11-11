//! `/api/v1/secrets`: the team secret vault (ADR-070 rule 4) on the
//! Settings page. A list answers names, who set each and when - never a
//! value, not even masked bytes of it. A write takes the value once, seals
//! it on the brain in memory (hive-store vault.rs) and answers the entry
//! as the list shows it; the value is never sent back, logged or put in an
//! event. Removing a secret takes its name.

use crate::additions::{addition_failure, addition_method, addition_not_found, addition_string};
use crate::httpx::{Request, ResponseWriter};
use crate::server::{Handler, read_payload, write_error, write_json};
use serde_json::json;

impl Handler {
    pub(crate) fn addition_secrets(&self, w: &mut ResponseWriter, r: &Request, parts: Vec<String>) {
        match parts.as_slice() {
            [_] => {
                if !addition_method(w, r, vec!["GET".to_string(), "POST".to_string()]) {
                    return;
                }
                if r.method == "POST" {
                    self.secret_write(w, r);
                } else {
                    self.secret_list(w);
                }
            }
            [_, name, action] if action == "remove" => {
                if addition_method(w, r, vec!["POST".to_string()]) {
                    self.secret_remove(w, name);
                }
            }
            _ => addition_not_found(w),
        }
    }

    fn secret_list(&self, w: &mut ResponseWriter) {
        let Some(store) = self.store.as_deref() else {
            store_missing(w);
            return;
        };
        match store.vault_entries() {
            Ok(entries) => write_json(
                w,
                200,
                json!({"secrets": entries, "max_bytes": hive_store::VAULT_MAX_VALUE}),
            ),
            Err(e) => addition_failure(w, e),
        }
    }

    fn secret_write(&self, w: &mut ResponseWriter, r: &Request) {
        let (payload, ok) = read_payload(w, r);
        if !ok {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            store_missing(w);
            return;
        };
        let (name, ok) = addition_string(w, &payload, "name", true);
        if !ok {
            return;
        }
        let Some(value) = payload.get("value").and_then(|v| v.as_str()) else {
            write_error(
                w,
                400,
                "Invalid value".to_string(),
                "A secret needs its value as a string.".to_string(),
                "Paste the value and save again.".to_string(),
            );
            return;
        };
        let value = hive_store::Zeroizing::new(value.to_string());
        let name = name.trim();
        match store.vault_set(hive_store::HUMAN_AGENT, name, value.as_bytes()) {
            Ok(entry) => write_json(w, 201, json!({"secret": entry})),
            Err(e) => vault_failure(w, e),
        }
    }

    fn secret_remove(&self, w: &mut ResponseWriter, name: &str) {
        let Some(store) = self.store.as_deref() else {
            store_missing(w);
            return;
        };
        match store.vault_remove(hive_store::HUMAN_AGENT, name) {
            Ok(true) => write_json(w, 200, json!({"removed": name})),
            Ok(false) => write_error(
                w,
                404,
                format!("No secret {name}"),
                "The team vault holds no secret by that name.".to_string(),
                "Pick a name from the list.".to_string(),
            ),
            Err(e) => vault_failure(w, e),
        }
    }
}

/// A refused write as what / why / fix: a bad name or an empty or oversized
/// value is the owner's to fix (400); anything else is the store's.
fn vault_failure(w: &mut ResponseWriter, e: anyhow::Error) {
    let text = e.to_string();
    let first = text.lines().next().unwrap_or_default().to_string();
    if first.contains("secret name")
        || first.contains("Hive sets")
        || first.contains("is empty")
        || first.contains("KiB")
    {
        write_error(
            w,
            400,
            first,
            "The vault takes an upper-case name (letters, digits, `_`) and a value of 1 byte to 64 KiB.".to_string(),
            "Correct the name or value and save again.".to_string(),
        );
    } else {
        addition_failure(w, e);
    }
}

fn store_missing(w: &mut ResponseWriter) {
    write_error(
        w,
        503,
        "The store is unavailable".to_string(),
        "The hub has no open Hive store.".to_string(),
        "Run hive init, then restart hive hub.".to_string(),
    );
}

#[cfg(test)]
mod tests {
    use crate::test_helpers_http as http;
    use serde_json::Value;

    fn call(
        s: &hive_store::Store,
        method: &str,
        path: &str,
        body: &str,
    ) -> (u16, serde_json::Map<String, Value>) {
        http::additions_request(http::handler(s), method, path, body)
    }

    #[test]
    fn the_secrets_route_lists_names_only_and_never_answers_a_value() {
        let s = http::additions_store(false);
        let (code, v) = call(&s, "GET", "/api/v1/secrets", "");
        assert_eq!(code, 200, "{v:?}");
        assert_eq!(v["secrets"].as_array().unwrap().len(), 0);

        let (code, v) = call(
            &s,
            "POST",
            "/api/v1/secrets",
            r#"{"name":"NPM_TOKEN","value":"npm_hub-value-0123"}"#,
        );
        assert_eq!(code, 201, "{v:?}");
        assert_eq!(v["secret"]["name"], "NPM_TOKEN");
        let (code, v) = call(&s, "GET", "/api/v1/secrets", "");
        assert_eq!(code, 200);
        let text = serde_json::to_string(&v).unwrap();
        assert!(text.contains("NPM_TOKEN"), "{text}");
        assert!(!text.contains("npm_hub-value-0123"), "{text}");
        assert_eq!(v["secrets"][0]["updated_by"], hive_store::HUMAN_AGENT);
        let opened = s.vault_open(&["NPM_TOKEN".to_string()]).unwrap();
        assert_eq!(opened[0].1.as_str(), "npm_hub-value-0123");

        let (code, v) = call(
            &s,
            "POST",
            "/api/v1/secrets",
            r#"{"name":"npm-token","value":"x"}"#,
        );
        assert_eq!(code, 400, "{v:?}");
        assert!(
            v["error"].as_str().unwrap().contains("not a secret name"),
            "{v:?}"
        );
        let (code, v) = call(
            &s,
            "POST",
            "/api/v1/secrets",
            r#"{"name":"EMPTY","value":""}"#,
        );
        assert_eq!(code, 400, "{v:?}");

        let (code, v) = call(&s, "POST", "/api/v1/secrets/NPM_TOKEN/remove", "{}");
        assert_eq!(code, 200, "{v:?}");
        let (code, _) = call(&s, "POST", "/api/v1/secrets/NPM_TOKEN/remove", "{}");
        assert_eq!(code, 404);
        let (code, _) = call(&s, "DELETE", "/api/v1/secrets", "");
        assert_eq!(code, 405);
    }
}
