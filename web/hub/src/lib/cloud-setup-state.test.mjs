import assert from "node:assert/strict";
import { test } from "node:test";
import { backupTo, changes, composeBackup, fieldProblem, initialDraft, missingFields, problems, readiness, readReference, signInCommand, visibleFields } from "./cloud-setup-state.ts";

// The shape GET /api/v1/clouds answers (crates/hive-hub/src/clouds.rs),
// cut down to Azure, Multipass and the three backup targets.
const f = (setting, label, required = false, kind = "identifier") => ({ setting, label, help: "", kind, required, placeholder: "" });
const payload = (over = {}) => ({
  available: true,
  can_edit: true,
  provider: "azure",
  clouds: [
    {
      id: "azure",
      label: "Microsoft Azure",
      blurb: "",
      auth_setting: "compute.azure.auth",
      auth: [
        { id: "login", label: "Your Azure account", blurb: "", sign_in: "az login", fields: [] },
        {
          id: "service-principal",
          label: "Service principal",
          blurb: "",
          sign_in: 'az login --service-principal --tenant {compute.azure.tenant} --username {compute.azure.client_id} --password "{compute.azure.client_secret}"',
          fields: [f("compute.azure.tenant", "Tenant", true), f("compute.azure.client_id", "Client id", true), f("compute.azure.client_secret", "Where the secret lives", true, "reference")],
        },
      ],
      fields: [f("compute.azure.subscription", "Subscription"), f("compute.azure.resource_group", "Resource group", true), f("compute.azure.location", "Region", true), f("compute.azure.vm_size", "Machine size")],
      backup_target: "azure-blob",
      auth_value: "login",
      values: { "compute.azure.auth": "login", "compute.azure.tenant": "", "compute.azure.client_id": "", "compute.azure.client_secret": "", "compute.azure.subscription": "", "compute.azure.resource_group": "", "compute.azure.location": "westeurope", "compute.azure.vm_size": "Standard_B2ms" },
      missing: ["compute.azure.resource_group"],
    },
    { id: "multipass", label: "Multipass (this computer)", blurb: "", auth_setting: "", auth: [{ id: "local", label: "No sign-in", blurb: "", sign_in: "", fields: [] }], fields: [f("compute.multipass.size", "Size")], backup_target: "", auth_value: "local", values: { "compute.multipass.size": "cpus=2,mem=4G,disk=32G" }, missing: [] },
  ],
  backup: {
    to: "",
    target: "",
    parts: {},
    values: { "backup.subscription": "" },
    targets: [
      { id: "folder", label: "A folder", blurb: "", cloud: "", parts: [f("path", "Folder", true)], template: "{path}", fields: [] },
      { id: "azure-blob", label: "Azure Blob Storage", blurb: "", cloud: "azure", parts: [f("account", "Storage account", true), f("container", "Container", true), f("prefix", "Folder in the container")], template: "az://{account}/{container}/{prefix}", fields: [f("backup.subscription", "Subscription")] },
      { id: "command", label: "A command", blurb: "", cloud: "", parts: [f("command", "Command", true)], template: "run: {command}", fields: [] },
    ],
  },
  ...over,
});

test("the form starts from what the store holds and shows the sign-in mode's fields first", () => {
  const p = payload();
  const d = initialDraft(p);
  assert.equal(d.provider, "azure");
  assert.equal(d.auth.azure, "login");
  assert.equal(d.values["compute.azure.location"], "westeurope");
  const az = p.clouds[0];
  assert.deepEqual(visibleFields(az, d).map((x) => x.setting), ["compute.azure.subscription", "compute.azure.resource_group", "compute.azure.location", "compute.azure.vm_size"]);
  d.auth.azure = "service-principal";
  assert.deepEqual(visibleFields(az, d).slice(0, 3).map((x) => x.setting), ["compute.azure.tenant", "compute.azure.client_id", "compute.azure.client_secret"]);
  assert.deepEqual(missingFields(az, d).map((x) => x.setting), ["compute.azure.tenant", "compute.azure.client_id", "compute.azure.client_secret", "compute.azure.resource_group"]);
  assert.match(readiness(p, d), /still needs tenant, client id, where the secret lives, resource group/);
  assert.deepEqual(changes(p, initialDraft(p)), [], "nothing changed, nothing saved");
});

test("a secret is held by reference and the sign-in command reads it without showing it", () => {
  const ref = f("compute.azure.client_secret", "Where the secret lives", true, "reference");
  assert.match(fieldProblem(ref, "s3cr3t-value~"), /never the secret itself/);
  assert.equal(fieldProblem(ref, "op://Infra/hive-sp/credential"), "");
  assert.equal(fieldProblem(ref, "keychain://hive/azure-sp"), "");
  assert.equal(fieldProblem(ref, "file:///Users/me/.hive/sp"), "");
  assert.match(fieldProblem(f("compute.azure.resource_group", "Resource group"), "ghp_abcdefghijklmnop"), /looks like a secret/);
  assert.equal(readReference("keychain://hive/azure-sp"), "$(security find-generic-password -s 'hive' -a 'azure-sp' -w)");
  assert.equal(readReference("file:///Users/me/sp"), "$(cat '/Users/me/sp')");

  const p = payload();
  const d = initialDraft(p);
  assert.equal(signInCommand(p.clouds[0], d), "az login");
  d.auth.azure = "service-principal";
  assert.equal(signInCommand(p.clouds[0], d), 'az login --service-principal --tenant <Tenant> --username <Client id> --password <Where the secret lives>');
  Object.assign(d.values, { "compute.azure.tenant": "t-1", "compute.azure.client_id": "app-1", "compute.azure.client_secret": "op://Infra/hive-sp/credential" });
  assert.equal(signInCommand(p.clouds[0], d), "az login --service-principal --tenant t-1 --username app-1 --password \"$(op read 'op://Infra/hive-sp/credential')\"");
});

test("saving writes only what changed, the chosen cloud first, then the backup", () => {
  const p = payload();
  const d = initialDraft(p);
  d.auth.azure = "service-principal";
  Object.assign(d.values, { "compute.azure.tenant": "t-1", "compute.azure.client_id": "app-1", "compute.azure.client_secret": "keychain://hive/azure-sp", "compute.azure.resource_group": "hive-hands", "compute.multipass.size": "cpus=4" });
  d.backupTarget = "azure-blob";
  d.backupParts["azure-blob"] = { account: "acct", container: "/hive-backups/", prefix: "" };
  assert.deepEqual(changes(p, d), [
    ["compute.azure.auth", "service-principal"],
    ["compute.azure.tenant", "t-1"],
    ["compute.azure.client_id", "app-1"],
    ["compute.azure.client_secret", "keychain://hive/azure-sp"],
    ["compute.azure.resource_group", "hive-hands"],
    ["backup.to", "az://acct/hive-backups"],
  ]);
  assert.match(readiness(p, d), /Microsoft Azure is set up/);
  assert.deepEqual(problems(p, d), {});
  d.values["compute.azure.client_secret"] = "hunter2hunter2";
  assert.ok(problems(p, d)["compute.azure.client_secret"]);

  d.provider = "multipass";
  assert.deepEqual(changes(p, d).slice(0, 2), [["compute.provider", "multipass"], ["compute.multipass.size", "cpus=4"]]);
});

test("backup targets compose backup.to like the store splits it", () => {
  const [folder, blob, cmd] = payload().backup.targets;
  assert.equal(composeBackup(folder, { path: "/Volumes/Kingston/hive" }), "/Volumes/Kingston/hive");
  assert.equal(composeBackup(blob, { account: "acct", container: "c", prefix: "store/nightly" }), "az://acct/c/store/nightly");
  assert.equal(composeBackup(blob, { account: "acct", container: "c" }), "az://acct/c");
  assert.equal(composeBackup(cmd, { command: "rclone copy {file} r:hive" }), "run: rclone copy {file} r:hive");

  const p = payload({ backup: { ...payload().backup, to: "az://acct/c/store", target: "azure-blob", parts: { account: "acct", container: "c", prefix: "store" } } });
  const d = initialDraft(p);
  assert.equal(backupTo(p, d), "az://acct/c/store");
  assert.deepEqual(changes(p, d), []);
  d.backupTarget = "command";
  d.backupParts.command = { command: "scp somewhere" };
  assert.equal(problems(p, d)["backup:command"], "The command must name {file}, the backup to send.");
  d.backupTarget = "";
  assert.deepEqual(changes(p, d), [["backup.to", ""]], "no target turns backups off");
});

test("the hub's real /api/v1/clouds answer (captured in mock/api/clouds.json) drives the form", async () => {
  const { readFile } = await import("node:fs/promises");
  const p = JSON.parse(await readFile(new URL("../../mock/api/clouds.json", import.meta.url), "utf8"));
  assert.deepEqual(p.clouds.map((c) => c.id), ["azure", "aws", "gcp", "multipass"]);
  const d = initialDraft(p);
  assert.equal(d.provider, "azure");
  assert.deepEqual(changes(p, d), [], "the stored values round-trip unchanged");
  const az = p.clouds.find((c) => c.id === "azure");
  assert.deepEqual(az.auth.map((m) => m.id), ["login", "service-principal"]);
  assert.ok(visibleFields(az, d).some((x) => x.setting === "compute.azure.resource_group" && x.required));
  d.auth.azure = "service-principal";
  const secret = visibleFields(az, d).find((x) => x.setting === "compute.azure.client_secret");
  assert.equal(secret.kind, "reference", "the secret field holds a reference, never the secret");
  assert.ok(missingFields(az, d).some((x) => x.setting === "compute.azure.tenant"));
  assert.ok(p.backup.targets.some((t) => t.id === "azure-blob" && t.cloud === "azure"));
});
