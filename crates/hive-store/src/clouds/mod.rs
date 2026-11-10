//! The cloud registry: what each place Hive can get machines from, or send
//! backups to, asks of the owner - how this machine signs in to it and the
//! settings it reads (ADR-070 Part A). The hub renders its setup form from
//! these descriptors (`GET /api/v1/clouds`), the store's settings rules hold
//! the values, and the runner's providers read them. A new cloud is one new
//! module here, listed in [`REGISTRY`], plus its provider in hive-runner.
//!
//! Nothing here holds a secret. A sign-in is the cloud CLI's own login on
//! this machine; where a cloud needs a secret (an Azure service principal's
//! password) the descriptor names a `reference` field - op://, keychain://
//! or file:// - that the console fills into the sign-in command the owner
//! runs. Hive never reads it.

pub mod aws;
pub mod azure;
pub mod backup;
pub mod gcp;
pub mod multipass;

use serde::{Deserialize, Serialize};

/// One value a cloud reads: a store setting, shown as a labelled field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// The store setting that holds it (`compute.azure.resource_group`).
    pub setting: String,
    pub label: String,
    pub help: String,
    /// `identifier` (a name or id, never a secret) or `reference` (where a
    /// secret lives: op://, keychain://, file://).
    pub kind: String,
    /// The cloud cannot provision without it.
    pub required: bool,
    pub placeholder: String,
}

/// One way this machine signs in to a cloud.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthMode {
    pub id: String,
    pub label: String,
    pub blurb: String,
    /// The command the owner runs on this machine, as the daemon's user.
    /// `{setting}` placeholders are filled from the fields; a `reference`
    /// field becomes a shell read of the reference, never the secret.
    pub sign_in: String,
    /// What only this mode asks for.
    pub fields: Vec<Field>,
}

/// A cloud as the setup form shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cloud {
    /// The `compute.provider` value.
    pub id: String,
    pub label: String,
    pub blurb: String,
    /// The choice setting that names the sign-in mode; empty when the
    /// cloud has one way in.
    pub auth_setting: String,
    /// The first is the default.
    pub auth: Vec<AuthMode>,
    /// What machines from this cloud read, whatever the sign-in.
    pub fields: Vec<Field>,
    /// The backup target (see [`backup`]) that stores copies in this cloud
    /// with the same sign-in; empty when there is none.
    pub backup_target: String,
}

impl Cloud {
    /// The sign-in mode `value` names, the default for an unknown one.
    pub fn auth_mode(&self, value: &str) -> &AuthMode {
        let v = value.trim();
        self.auth
            .iter()
            .find(|a| a.id == v)
            .unwrap_or(&self.auth[0])
    }

    /// Every field the cloud reads under sign-in mode `auth`, the mode's own first.
    pub fn fields_for(&self, auth: &str) -> Vec<&Field> {
        let mut out: Vec<&Field> = self.auth_mode(auth).fields.iter().collect();
        out.extend(self.fields.iter());
        out
    }

    /// Every setting the cloud names, in any mode.
    pub fn settings(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.auth_setting.is_empty() {
            out.push(self.auth_setting.clone());
        }
        for a in &self.auth {
            out.extend(a.fields.iter().map(|f| f.setting.clone()));
        }
        out.extend(self.fields.iter().map(|f| f.setting.clone()));
        out
    }

    /// The required settings still empty under the sign-in mode the store
    /// names, as `get` reads them.
    pub fn missing(&self, get: &dyn Fn(&str) -> String) -> Vec<String> {
        let auth = if self.auth_setting.is_empty() {
            String::new()
        } else {
            get(&self.auth_setting)
        };
        self.fields_for(&auth)
            .into_iter()
            .filter(|f| f.required && get(&f.setting).trim().is_empty())
            .map(|f| f.setting.clone())
            .collect()
    }
}

/// One module per cloud, in the order the form lists them.
pub const REGISTRY: &[fn() -> Cloud] = &[
    azure::descriptor,
    aws::descriptor,
    gcp::descriptor,
    multipass::descriptor,
];

/// Every cloud, in the order the form lists them.
pub fn clouds() -> Vec<Cloud> {
    REGISTRY.iter().map(|f| f()).collect()
}

/// The cloud with `compute.provider` value `id`.
pub fn cloud(id: &str) -> Option<Cloud> {
    clouds().into_iter().find(|c| c.id == id.trim())
}

pub(crate) fn field(
    setting: &str,
    label: &str,
    help: &str,
    required: bool,
    placeholder: &str,
) -> Field {
    Field {
        setting: setting.into(),
        label: label.into(),
        help: help.into(),
        kind: "identifier".into(),
        required,
        placeholder: placeholder.into(),
    }
}

pub(crate) fn reference(setting: &str, label: &str, help: &str, required: bool) -> Field {
    Field {
        kind: "reference".into(),
        placeholder: "op://VAULT/ITEM/FIELD".into(),
        ..field(setting, label, help, required, "")
    }
}

pub(crate) fn mode(
    id: &str,
    label: &str,
    blurb: &str,
    sign_in: &str,
    fields: Vec<Field>,
) -> AuthMode {
    AuthMode {
        id: id.into(),
        label: label.into(),
        blurb: blurb.into(),
        sign_in: sign_in.into(),
        fields,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        COMPUTE_PROVIDERS, COMPUTE_SETTINGS, compute_size_setting, validate_setting,
    };

    #[test]
    fn the_registry_is_the_provider_list() {
        let ids: Vec<String> = clouds().into_iter().map(|c| c.id).collect();
        assert_eq!(
            ids,
            COMPUTE_PROVIDERS
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );
        for c in clouds() {
            assert!(!c.auth.is_empty(), "{}: a cloud has a way in", c.id);
            assert!(
                c.fields
                    .iter()
                    .any(|f| f.setting == compute_size_setting(&c.id)),
                "{}: the form shows the size setting the runner reads",
                c.id
            );
        }
    }

    #[test]
    fn every_field_is_a_setting_the_store_takes() {
        for c in clouds() {
            for name in c.settings() {
                assert!(
                    COMPUTE_SETTINGS.contains_key(&name),
                    "{}: {name} has no default",
                    c.id
                );
                let def = &COMPUTE_SETTINGS[&name];
                assert!(
                    validate_setting(&name, def).is_ok(),
                    "{}: {name} default {def:?}",
                    c.id
                );
            }
            if !c.auth_setting.is_empty() {
                for a in &c.auth {
                    assert_eq!(validate_setting(&c.auth_setting, &a.id).unwrap(), a.id);
                }
                assert_eq!(
                    COMPUTE_SETTINGS[&c.auth_setting], c.auth[0].id,
                    "{}: the first mode is the default",
                    c.id
                );
            } else {
                assert_eq!(
                    c.auth.len(),
                    1,
                    "{}: several modes need a setting to pick one",
                    c.id
                );
            }
            for f in c
                .auth
                .iter()
                .flat_map(|a| a.fields.iter())
                .chain(c.fields.iter())
            {
                assert!(
                    matches!(f.kind.as_str(), "identifier" | "reference"),
                    "{}: {}",
                    c.id,
                    f.setting
                );
            }
        }
    }

    #[test]
    fn azure_signs_in_with_a_login_or_a_service_principal_and_needs_a_resource_group() {
        let az = cloud("azure").unwrap();
        assert_eq!(az.auth_setting, "compute.azure.auth");
        assert_eq!(
            az.auth.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            ["login", "service-principal"]
        );
        let sp = az.auth_mode("service-principal");
        let secret = sp
            .fields
            .iter()
            .find(|f| f.setting == "compute.azure.client_secret")
            .unwrap();
        assert_eq!(
            secret.kind, "reference",
            "the secret is held by reference only"
        );
        assert!(sp.sign_in.contains("{compute.azure.client_secret}"));
        assert!(
            validate_setting("compute.azure.client_secret", "hunter2-not-a-reference").is_err()
        );
        assert_eq!(
            validate_setting(
                "compute.azure.client_secret",
                "op://Infra/hive-sp/credential"
            )
            .unwrap(),
            "op://Infra/hive-sp/credential"
        );
        assert_eq!(az.backup_target, "azure-blob");

        let settings = std::collections::BTreeMap::from([("compute.azure.location", "westeurope")]);
        let get = |k: &str| settings.get(k).map(|v| v.to_string()).unwrap_or_default();
        assert_eq!(az.missing(&get), ["compute.azure.resource_group"]);
        let settings = std::collections::BTreeMap::from([
            ("compute.azure.auth", "service-principal"),
            ("compute.azure.resource_group", "hive-hands"),
            ("compute.azure.location", "westeurope"),
        ]);
        let get = |k: &str| settings.get(k).map(|v| v.to_string()).unwrap_or_default();
        assert_eq!(
            az.missing(&get),
            [
                "compute.azure.tenant",
                "compute.azure.client_id",
                "compute.azure.client_secret"
            ]
        );
        assert_eq!(
            az.auth_mode("moon").id,
            "login",
            "an unknown mode is the default"
        );
    }
}
