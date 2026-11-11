import assert from "node:assert/strict";
import { test } from "node:test";
import { backupTo, changes, composeBackup, fieldProblem, initialDraft, missingFields, problems, readiness, readReference, signInCommand, visibleFields } from "./cloud-setup-state.ts";

// The shape GET /api/v1/clouds answers (crates/pulse-hub/src/clouds.rs),
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
  assert.equal(fieldProblem(ref, "op://Infra/pulse-sp/credential"), "");
  assert.equal(fieldProblem(ref, "keychain://pulse/azure-sp"), "");
  assert.equal(fieldProblem(ref, "file:///Users/me/.pulse/sp"), "");
  assert.match(fieldProblem(f("compute.azure.resource_group", "Resource group"), "ghp_abcdefghijklmnop"), /looks like a secret/);
  assert.equal(readReference("keychain://pulse/azure-sp"), "$(security find-generic-password -s 'pulse' -a 'azure-sp' -w)");
  assert.equal(readReference("file:///Users/me/sp"), "$(cat '/Users/me/sp')");

  const p = payload();
  const d = initialDraft(p);
  assert.equal(signInCommand(p.clouds[0], d), "az login");
  d.auth.azure = "service-principal";
  assert.equal(signInCommand(p.clouds[0], d), 'az login --service-principal --tenant <Tenant> --username <Client id> --password <Where the secret lives>');
  Object.assign(d.values, { "compute.azure.tenant": "t-1", "compute.azure.client_id": "app-1", "compute.azure.client_secret": "op://Infra/pulse-sp/credential" });
  assert.equal(signInCommand(p.clouds[0], d), "az login --service-principal --tenant t-1 --username app-1 --password \"$(op read 'op://Infra/pulse-sp/credential')\"");
});

test("saving writes only what changed, the chosen cloud first, then the backup", () => {
  const p = payload();
  const d = initialDraft(p);
  d.auth.azure = "service-principal";
  Object.assign(d.values, { "compute.azure.tenant": "t-1", "compute.azure.client_id": "app-1", "compute.azure.client_secret": "keychain://pulse/azure-sp", "compute.azure.resource_group": "pulse-hands", "compute.multipass.size": "cpus=4" });
  d.backupTarget = "azure-blob";
  d.backupParts["azure-blob"] = { account: "acct", container: "/pulse-backups/", prefix: "" };
  assert.deepEqual(changes(p, d), [
    ["compute.azure.auth", "service-principal"],
    ["compute.azure.tenant", "t-1"],
    ["compute.azure.client_id", "app-1"],
    ["compute.azure.client_secret", "keychain://pulse/azure-sp"],
    ["compute.azure.resource_group", "pulse-hands"],
    ["backup.to", "az://acct/pulse-backups"],
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
  assert.equal(composeBackup(folder, { path: "/Volumes/Kingston/pulse" }), "/Volumes/Kingston/pulse");
  assert.equal(composeBackup(blob, { account: "acct", container: "c", prefix: "store/nightly" }), "az://acct/c/store/nightly");
  assert.equal(composeBackup(blob, { account: "acct", container: "c" }), "az://acct/c");
  assert.equal(composeBackup(cmd, { command: "rclone copy {file} r:pulse" }), "run: rclone copy {file} r:pulse");

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

test("a cloud with no sign-in modes, or a payload with no clouds, still gives the form a starting point", () => {
  const bare = { id: "bare", label: "Bare cloud", blurb: "", auth_setting: "", auth: [], fields: [f("compute.bare.zone", "Zone", true)], backup_target: "", auth_value: "", values: {}, missing: [] };
  const twoModes = { ...payload().clouds[0], id: "azure", auth_value: "" };
  const p = payload({ provider: "", clouds: [bare, twoModes] });
  const d = initialDraft(p);
  assert.equal(d.provider, "bare", "no stored provider: the first cloud is picked");
  assert.equal(d.auth.bare, "", "no sign-in modes: no mode is drafted");
  assert.equal(d.auth.azure, "login", "no stored mode: the cloud's first (default) mode");
  assert.deepEqual(visibleFields(bare, d).map((x) => x.setting), ["compute.bare.zone"], "no mode: only the cloud's own fields");
  assert.equal(signInCommand(bare, d), "", "no mode: no sign-in command to show");
  assert.deepEqual(missingFields(bare, d).map((x) => x.setting), ["compute.bare.zone"], "a field the store never sent counts as empty");
  assert.equal(readiness(p, d), "Bare cloud still needs zone.");
  assert.deepEqual(problems(p, d), {}, "an empty required field is named in the summary, not a stopping problem");
  assert.deepEqual(changes(p, d), [], "the defaulted provider is what the store would pick too; no auth setting to save");
  d.values["compute.bare.zone"] = "north-1";
  assert.equal(readiness(p, d), "Bare cloud is set up. Machines are made only when an agent asks and you approve, within the limits below.");
  assert.deepEqual(changes(p, d), [["compute.bare.zone", "north-1"]], "a field the store never held is saved once filled");

  const empty = payload({ provider: "", clouds: [] });
  const e = initialDraft(empty);
  assert.equal(e.provider, "", "no clouds at all: nothing is picked");
  assert.equal(readiness(empty, e), "Pick where extra machines come from.");
  assert.deepEqual(problems(empty, e), {});
  assert.deepEqual(changes(empty, e), []);
});

test("an unknown or undrafted sign-in mode falls back to the cloud's default one", () => {
  const p = payload();
  const az = p.clouds[0];
  const d = initialDraft(p);
  d.auth.azure = "no-such-mode";
  assert.equal(signInCommand(az, d), "az login", "an unknown mode reads as the default");
  assert.equal(visibleFields(az, d)[0].setting, "compute.azure.subscription");
  delete d.auth.azure;
  assert.equal(signInCommand(az, d), "az login", "no drafted mode reads as the default");
  assert.equal(visibleFields(az, d).length, 4);
  assert.deepEqual(changes(p, d), [["compute.azure.auth", ""]], "a cleared mode is saved as empty");
  const fresh = { ...payload(), clouds: payload().clouds.map((c) => ({ ...c, values: {} })) };
  const n = initialDraft(fresh);
  n.values = {};
  assert.deepEqual(changes(fresh, n), [], "neither draft nor store holds a value: nothing changes");
});

test("a field value is checked the way the store would refuse it", () => {
  assert.equal(fieldProblem(f("compute.azure.location", "Region", true), "   "), "Region is needed.", "a blank required field");
  assert.equal(fieldProblem(f("compute.azure.vm_size", "Machine size"), ""), "", "a blank optional field is fine");
  assert.equal(fieldProblem(f("compute.azure.resource_group", "Resource group"), "pulse hands"), "Resource group is one word, without spaces.");
  assert.equal(fieldProblem(f("compute.multipass.size", "Size"), "cpus=2 mem=4G"), "", "Multipass sizes may hold spaces");
  assert.match(fieldProblem(f("compute.multipass.size", "Size"), "-----BEGIN RSA"), /looks like a secret/);
  assert.equal(fieldProblem(f("compute.azure.location", "Region"), "westeurope"), "");

  const p = payload();
  const d = initialDraft(p);
  d.values["compute.azure.resource_group"] = "pulse hands";
  assert.deepEqual(problems(p, d), { "compute.azure.resource_group": "Resource group is one word, without spaces." });
  delete d.values["compute.azure.location"];
  assert.deepEqual(missingFields(p.clouds[0], d).map((x) => x.setting), ["compute.azure.location"], "a value missing from the draft is empty");
  assert.equal(readiness(p, d), "Microsoft Azure still needs region.");
  assert.deepEqual(problems(p, d), { "compute.azure.resource_group": "Resource group is one word, without spaces." });
});

test("a secret reference is read in the sign-in command only when it names a scheme", () => {
  assert.equal(readReference("  op://Infra/sp/credential  "), "$(op read 'op://Infra/sp/credential')");
  assert.equal(readReference("hunter2"), "", "no scheme: nothing to read");
  assert.equal(readReference("keychain://pulse/azure/extra"), "", "a keychain reference with a third part is not a keychain read");
  assert.equal(readReference(""), "");

  const p = payload();
  const az = p.clouds[0];
  const d = initialDraft(p);
  d.auth.azure = "service-principal";
  Object.assign(d.values, { "compute.azure.tenant": "t-1", "compute.azure.client_id": "app-1", "compute.azure.client_secret": "hunter2" });
  assert.equal(signInCommand(az, d), 'az login --service-principal --tenant t-1 --username app-1 --password "<Where the secret lives>"', "an unreadable reference shows its label, never the value");
  delete d.values["compute.azure.tenant"];
  assert.match(signInCommand(az, d), /--tenant <Tenant> /, "a value missing from the draft shows its label");

  const custom = {
    ...az,
    auth: [{ id: "cli", label: "CLI", blurb: "", sign_in: 'cloud login --zone "{compute.azure.location}" --org {compute.other.org}', fields: [] }],
  };
  const c = initialDraft(p);
  c.auth.azure = "cli";
  assert.equal(signInCommand(custom, c), 'cloud login --zone "westeurope" --org <compute.other.org>', "a quoted slot keeps its quotes; a slot naming no field shows the setting name");
});

test("the backup target string and its saves follow the chosen target", () => {
  const p = payload();
  const d = initialDraft(p);
  assert.equal(backupTo(p, d), "", "no target: no backup.to");
  d.backupTarget = "gone";
  assert.equal(backupTo(p, d), "", "an unknown target: no backup.to");
  assert.deepEqual(changes(p, d), [], "an unknown target saves nothing, nor turns backups off when they are off");
  d.backupTarget = "folder";
  delete d.backupParts.folder;
  assert.equal(backupTo(p, d), "", "a target without drafted parts composes empty");
  assert.deepEqual(changes(p, d), [], "a required part still empty: backup.to is not saved");
  assert.deepEqual(problems(p, d), {}, "no parts drafted: nothing to refuse yet");
  d.backupParts.folder = { path: "Volumes/pulse" };
  assert.deepEqual(problems(p, d), { "backup:path": "An absolute path, or one starting with ~/." });
  d.backupParts.folder = { path: "~/pulse-backups" };
  assert.deepEqual(problems(p, d), {});
  assert.deepEqual(changes(p, d), [["backup.to", "~/pulse-backups"]]);
  d.backupParts.folder = { path: "/Volumes/pulse" };
  assert.deepEqual(problems(p, d), {});

  d.backupTarget = "command";
  delete d.backupParts.command;
  assert.deepEqual(problems(p, d), {}, "no command drafted: nothing to refuse yet");
  d.backupParts.command = { command: "rclone copy {file} r:pulse" };
  assert.deepEqual(problems(p, d), {});
  assert.deepEqual(changes(p, d), [["backup.to", "run: rclone copy {file} r:pulse"]]);

  d.backupTarget = "azure-blob";
  d.backupParts["azure-blob"] = { account: "acct", container: "c" };
  d.values["backup.subscription"] = "sub one";
  assert.deepEqual(problems(p, d), { "backup.subscription": "Subscription is one word, without spaces." }, "a backup field is checked like a cloud field");
  d.values["backup.subscription"] = "sub-1";
  assert.deepEqual(changes(p, d), [["backup.to", "az://acct/c"], ["backup.subscription", "sub-1"]]);
  delete d.values["backup.subscription"];
  assert.deepEqual(problems(p, d), {}, "a backup field missing from the draft is empty, not wrong");
  const bare = { ...p, backup: { ...p.backup, values: {} } };
  const b = initialDraft(bare);
  b.backupTarget = "azure-blob";
  delete b.values["backup.subscription"];
  assert.deepEqual(changes(bare, b), [], "neither draft nor store holds the backup field: nothing changes");

  const needs = payload({ backup: { ...p.backup, targets: [{ id: "s3", label: "S3", blurb: "", cloud: "", parts: [f("bucket", "Bucket", true)], template: "s3://{bucket}", fields: [f("backup.region", "Backup region", true)] }] } });
  const n = initialDraft(needs);
  n.backupTarget = "s3";
  assert.deepEqual(problems(needs, n), {}, "a required backup field missing from the draft waits for the owner, it does not stop a save");
});
