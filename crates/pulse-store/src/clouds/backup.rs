//! Where backups of the store can go, in the registry's shape: each target
//! names the fields the form asks for and how they make `backup.to`, plus
//! the cloud whose sign-in it uses (an `az://` destination signs in the way
//! `compute.azure.auth` says, like the machines). A new destination is one
//! more entry in [`targets`] and its delivery in pulse-runner backup.rs.

use super::{Field, field};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The setting every target writes.
pub const TO: &str = "backup.to";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupTarget {
    pub id: String,
    pub label: String,
    pub blurb: String,
    /// The cloud (see [`super::REGISTRY`]) whose sign-in the target uses;
    /// empty when it needs none of PULSE's.
    pub cloud: String,
    /// The parts of `backup.to`; each `setting` is the part's name in
    /// `template`, not a store setting.
    pub parts: Vec<Field>,
    /// `backup.to` with `{part}` placeholders. A trailing `/` left by an
    /// empty last part is dropped.
    pub template: String,
    /// Store settings the target also reads.
    pub fields: Vec<Field>,
}

impl BackupTarget {
    /// `backup.to` made from the parts.
    pub fn compose(&self, parts: &BTreeMap<String, String>) -> String {
        let mut out = self.template.clone();
        for p in &self.parts {
            let slot = format!("{{{}}}", p.setting);
            let mut v = parts.get(&p.setting).map(|v| v.trim()).unwrap_or("");
            // A part between slashes is one path segment: its own slashes
            // at either end would double the template's.
            if out.contains(&format!("/{slot}")) || out.contains(&format!("{slot}/")) {
                v = v.trim_matches('/');
            }
            out = out.replace(&slot, v);
        }
        let template_ends_open = self.template.ends_with('}');
        while template_ends_open && out.ends_with('/') && out.len() > 1 {
            out.pop();
        }
        out
    }
}

pub fn targets() -> Vec<BackupTarget> {
    vec![
        BackupTarget {
            id: "folder".into(),
            label: "A folder".into(),
            blurb: "Another drive, a network share or a synced folder (iCloud, Dropbox). It must exist already; a drive that is not mounted is refused.".into(),
            cloud: String::new(),
            parts: vec![field("path", "Folder", "An absolute path, or one starting with ~/.", true, "/Volumes/Backup/pulse")],
            template: "{path}".into(),
            fields: vec![],
        },
        BackupTarget {
            id: "azure-blob".into(),
            label: "Azure Blob Storage".into(),
            blurb: "A container in one of your storage accounts, reached with the same Azure sign-in as the machines.".into(),
            cloud: "azure".into(),
            parts: vec![
                field("account", "Storage account", "The storage account name.", true, "mystorageaccount"),
                field("container", "Container", "The blob container; it must exist.", true, "pulse-backups"),
                field("prefix", "Folder in the container", "Optional path in front of each file name.", false, "store"),
            ],
            template: "az://{account}/{container}/{prefix}".into(),
            fields: vec![field("backup.subscription", "Subscription", "Only when the storage account is outside the az CLI's default subscription.", false, "")],
        },
        BackupTarget {
            id: "command".into(),
            label: "A command".into(),
            blurb: "Anything else - rclone, aws s3 cp, scp - run with {file} (the backup's path) and {name} (its file name) filled in.".into(),
            cloud: String::new(),
            parts: vec![field("command", "Command", "Must name {file}.", true, "rclone copy {file} remote:pulse")],
            template: "run: {command}".into(),
            fields: vec![],
        },
    ]
}

/// The target `backup.to` names and its parts; `None` while it is empty.
pub fn split(to: &str) -> Option<(String, BTreeMap<String, String>)> {
    let to = to.trim();
    if to.is_empty() {
        return None;
    }
    let mut parts = BTreeMap::new();
    if let Some(rest) = to.strip_prefix("az://") {
        let mut it = rest.trim_matches('/').splitn(3, '/');
        for key in ["account", "container", "prefix"] {
            parts.insert(
                key.to_string(),
                it.next().unwrap_or("").trim_matches('/').to_string(),
            );
        }
        return Some(("azure-blob".into(), parts));
    }
    if let Some(rest) = to.strip_prefix("run:") {
        parts.insert("command".into(), rest.trim().to_string());
        return Some(("command".into(), parts));
    }
    parts.insert("path".into(), to.to_string());
    Some(("folder".into(), parts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clouds::cloud;

    #[test]
    fn targets_round_trip_backup_to() {
        for to in [
            "/Volumes/Kingston/pulse",
            "az://acct/pulse-backups",
            "az://acct/pulse-backups/store/nightly",
            "run: rclone copy {file} remote:pulse",
        ] {
            let (id, parts) = split(to).unwrap();
            let t = targets().into_iter().find(|t| t.id == id).unwrap();
            assert_eq!(t.compose(&parts), to, "{id}");
        }
        assert_eq!(split("  "), None);
    }

    #[test]
    fn a_cloud_target_signs_in_like_the_cloud_and_reads_known_settings() {
        for t in targets() {
            if !t.cloud.is_empty() {
                let c = cloud(&t.cloud).expect("the target's cloud is registered");
                assert_eq!(c.backup_target, t.id);
            }
            for f in &t.fields {
                assert!(
                    crate::settings::validate_setting(&f.setting, "").is_ok(),
                    "{}",
                    f.setting
                );
            }
            for p in &t.parts {
                assert!(
                    t.template.contains(&format!("{{{}}}", p.setting)),
                    "{}: {}",
                    t.id,
                    p.setting
                );
            }
        }
    }
}
