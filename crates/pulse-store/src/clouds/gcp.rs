//! Google Cloud: Compute Engine instances through gcloud.

use super::{Cloud, field, mode};

pub fn descriptor() -> Cloud {
    Cloud {
        id: "gcp".into(),
        label: "Google Cloud".into(),
        blurb: "Compute Engine instances made and removed with the Google Cloud CLI (gcloud) on this machine.".into(),
        auth_setting: String::new(),
        auth: vec![mode(
            "login",
            "Your Google account",
            "Sign in once in a browser; gcloud keeps the sign-in on this machine.",
            "gcloud auth login",
            vec![],
        )],
        fields: vec![
            field("compute.gcp.project", "Project", "The project the instances are made in. Empty: gcloud's current project.", false, "my-project"),
            field("compute.gcp.zone", "Zone", "The Compute Engine zone, e.g. europe-west4-a.", true, "europe-west4-a"),
            field("compute.gcp.machine_type", "Machine type", "The machine type each instance gets.", false, "e2-medium"),
            field("compute.gcp.image", "Image", "An Ubuntu image family or a full image path.", false, "ubuntu-2404-lts-amd64"),
            field("compute.gcp.network", "Network", "The VPC network. Empty: default.", false, "default"),
        ],
        backup_target: String::new(),
    }
}
