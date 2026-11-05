//! `GET /api/v1/clouds`: the cloud registry (pulse_store::clouds) with the
//! values the store holds, so the console renders the machine and backup
//! setup forms from the descriptors instead of a list of raw keys. Writes
//! go through `POST /api/v1/settings`, one field at a time, under the same
//! validation as everywhere else: a cloud field is an identifier, and a
//! secret is only ever a reference to where it lives (ADR-041, ADR-070).

use crate::httpx::{Request, ResponseWriter};
use crate::server::Handler;
use pulse_store::clouds::{self, backup};
use serde_json::{Value, json};

impl Handler {
    pub(crate) fn addition_clouds(&self, w: &mut ResponseWriter, r: &Request, parts: Vec<String>) {
        if parts.len() != 1 {
            crate::addition_not_found(w);
            return;
        }
        if !crate::addition_method(w, r, vec!["GET".to_string()])
            || !self.addition_available(w, r, vec![])
        {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            crate::write_json(w, 200, json!({"available": false, "can_edit": false}));
            return;
        };
        crate::write_json(w, 200, clouds_payload(store));
    }
}

/// A setting's value as the form may show it: never anything secret-shaped,
/// which the store refuses to save anyway.
fn shown(store: &pulse_store::Store, name: &str) -> String {
    let def = pulse_store::COMPUTE_SETTINGS
        .get(name)
        .cloned()
        .unwrap_or_default();
    let v = store.setting(name, &def);
    if pulse_store::secret_shaped(&v) {
        String::new()
    } else {
        v.trim().to_string()
    }
}

pub(crate) fn clouds_payload(store: &pulse_store::Store) -> Value {
    let get = |name: &str| shown(store, name);
    let rows: Vec<Value> = clouds::clouds()
        .into_iter()
        .map(|c| {
            let values: serde_json::Map<String, Value> = c
                .settings()
                .into_iter()
                .map(|n| {
                    let v = get(&n);
                    (n, Value::String(v))
                })
                .collect();
            let auth = if c.auth_setting.is_empty() {
                c.auth[0].id.clone()
            } else {
                c.auth_mode(&get(&c.auth_setting)).id.clone()
            };
            let missing = c.missing(&get);
            let mut row = serde_json::to_value(&c).unwrap_or_default();
            if let Some(o) = row.as_object_mut() {
                o.insert("auth_value".into(), json!(auth));
                o.insert("values".into(), Value::Object(values));
                o.insert("missing".into(), json!(missing));
            }
            row
        })
        .collect();
    let to = store.setting(backup::TO, "");
    let (target, parts) = backup::split(&to).unwrap_or_default();
    let targets = backup::targets();
    let mut backup_values = serde_json::Map::new();
    for f in targets.iter().flat_map(|t| t.fields.iter()) {
        let v = store.setting(&f.setting, "");
        backup_values.insert(
            f.setting.clone(),
            json!(if pulse_store::secret_shaped(&v) {
                ""
            } else {
                v.trim()
            }),
        );
    }
    json!({
        "available": true,
        "can_edit": true,
        "provider": store.setting("compute.provider", "azure").trim(),
        "clouds": rows,
        "backup": {
            "to": to.trim(),
            "target": target,
            "parts": parts,
            "values": backup_values,
            "targets": targets,
        },
    })
}

#[cfg(test)]
mod tests {
    use crate::test_helpers_http as http;
    use serde_json::Value;
    use std::sync::Arc;

    fn save(h: &crate::server::Handler, name: &str, value: &str) -> (u16, Value) {
        let h = Arc::new(h.clone_for_test());
        let server =
            crate::test_helpers::TestServer::start(Arc::new(move |w, r| h.serve_http(w, r)));
        let response = crate::test_helpers::client()
            .post(format!("{}/api/v1/settings", server.url))
            .header("Content-Type", "application/json")
            .header("X-Pulse-Settings", "yes")
            .body(serde_json::json!({"name": name, "value": value}).to_string())
            .send()
            .expect("save setting");
        let status = response.status().as_u16();
        (status, response.json().unwrap_or_default())
    }

    #[test]
    fn clouds_list_the_registry_with_the_stored_values() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(true);
        let h = http::handler(&s);
        let (status, data) = http::additions_request(h.clone(), "GET", "/api/v1/clouds", "");
        assert_eq!(status, 200, "{data:?}");
        assert_eq!(data["provider"], "azure");
        let clouds = data["clouds"].as_array().unwrap();
        let ids: Vec<&str> = clouds.iter().map(|c| c["id"].as_str().unwrap()).collect();
        assert_eq!(ids, pulse_store::COMPUTE_PROVIDERS);
        let az = &clouds[0];
        assert_eq!(az["auth_value"], "login");
        assert_eq!(az["values"]["compute.azure.vm_size"], "Standard_B2ms");
        assert_eq!(
            az["missing"],
            serde_json::json!(["compute.azure.resource_group", "compute.azure.location"])
        );
        assert_eq!(data["backup"]["target"], "");

        // The form saves through POST /settings, field by field.
        for (name, value) in [
            ("compute.azure.auth", "service-principal"),
            (
                "compute.azure.tenant",
                "11111111-2222-3333-4444-555555555555",
            ),
            ("compute.azure.client_secret", "keychain://pulse/azure-sp"),
            ("compute.azure.resource_group", "pulse-hands"),
            ("compute.azure.location", "westeurope"),
            ("backup.to", "az://acct/pulse-backups/store"),
        ] {
            let (code, result) = save(&h, name, value);
            assert_eq!(code, 200, "{name}: {result:?}");
        }
        let (code, result) = save(&h, "compute.azure.client_secret", "not-a-reference");
        assert_eq!(code, 400, "a raw secret is refused: {result:?}");

        let (_, data) = http::additions_request(h, "GET", "/api/v1/clouds", "");
        let az = &data["clouds"][0];
        assert_eq!(az["auth_value"], "service-principal");
        assert_eq!(
            az["values"]["compute.azure.client_secret"],
            "keychain://pulse/azure-sp"
        );
        assert_eq!(
            az["missing"],
            serde_json::json!(["compute.azure.client_id"])
        );
        let sp = az["auth"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == "service-principal")
            .unwrap();
        assert_eq!(sp["fields"][2]["kind"], "reference");
        assert_eq!(data["backup"]["target"], "azure-blob");
        assert_eq!(data["backup"]["parts"]["prefix"], "store");
        let targets = data["backup"]["targets"].as_array().unwrap();
        assert!(
            targets
                .iter()
                .any(|t| t["id"] == "azure-blob" && t["cloud"] == "azure")
        );
    }
}
