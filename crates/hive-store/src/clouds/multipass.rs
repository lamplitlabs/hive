//! Multipass: virtual machines on this computer, no account at all.

use super::{Cloud, field, mode};

pub fn descriptor() -> Cloud {
    Cloud {
        id: "multipass".into(),
        label: "Multipass (this computer)".into(),
        blurb: "Virtual machines on this computer, made with Multipass. Nothing to sign in to."
            .into(),
        auth_setting: String::new(),
        auth: vec![mode(
            "local",
            "No sign-in",
            "Multipass runs here; it needs no account.",
            "",
            vec![],
        )],
        fields: vec![
            field(
                "compute.multipass.size",
                "Size",
                "cpus=N,mem=SIZE,disk=SIZE for each machine.",
                false,
                "cpus=2,mem=4G,disk=32G",
            ),
            field(
                "compute.multipass.image",
                "Image",
                "The Multipass image, e.g. 24.04.",
                false,
                "24.04",
            ),
        ],
        backup_target: String::new(),
    }
}
