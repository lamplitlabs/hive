//! Microsoft Azure: VMs in a resource group, signed in with the owner's own
//! `az login` or as a service principal whose secret stays in the owner's
//! secret store.

use super::{Cloud, field, mode, reference};

pub fn descriptor() -> Cloud {
    Cloud {
        id: "azure".into(),
        label: "Microsoft Azure".into(),
        blurb: "Virtual machines in one of your resource groups, made and removed with the Azure CLI (az) on this machine.".into(),
        auth_setting: "compute.azure.auth".into(),
        auth: vec![
            mode(
                "login",
                "Your Azure account",
                "Sign in once in a browser with your own account (OAuth). The az CLI keeps the sign-in on this machine; PULSE uses it as it is.",
                "az login",
                vec![],
            ),
            mode(
                "service-principal",
                "Service principal",
                "An app registration with a role on the resource group only. PULSE keeps its tenant and client id and where its secret lives - never the secret - and refuses to work while az is signed in as anyone else.",
                "az login --service-principal --tenant {compute.azure.tenant} --username {compute.azure.client_id} --password \"{compute.azure.client_secret}\"",
                vec![
                    field("compute.azure.tenant", "Tenant", "The Microsoft Entra tenant id (or domain) the app belongs to.", true, "00000000-0000-0000-0000-000000000000"),
                    field("compute.azure.client_id", "Client id", "The application (client) id of the app registration.", true, "00000000-0000-0000-0000-000000000000"),
                    reference("compute.azure.client_secret", "Where the secret lives", "A reference to the client secret in 1Password (op://), the macOS Keychain (keychain://) or a 0600 file (file://). PULSE stores the reference and never reads the secret.", true),
                ],
            ),
        ],
        fields: vec![
            field("compute.azure.subscription", "Subscription", "The subscription id or name to create machines in. Empty: the az CLI's default one.", false, "my-subscription"),
            field("compute.azure.resource_group", "Resource group", "The resource group that holds the machines; created in the region below when missing.", true, "pulse-hands"),
            field("compute.azure.location", "Region", "The Azure region, e.g. westeurope.", true, "westeurope"),
            field("compute.azure.vm_size", "Machine size", "The VM size each machine gets.", false, "Standard_B2ms"),
            field("compute.azure.image", "Image", "The VM image alias or URN.", false, "Ubuntu2404"),
        ],
        backup_target: "azure-blob".into(),
    }
}
