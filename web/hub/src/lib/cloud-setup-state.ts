// The cloud setup form's state (ADR-070): which cloud machines come from,
// how this machine signs in to it, the fields that cloud reads, and where
// backups go - all rendered from the descriptors GET /api/v1/clouds sends
// (crates/pulse-store/src/clouds), so a new cloud is a new descriptor and
// no new form. The form saves through POST /api/v1/settings one changed
// setting at a time, under the store's own validation. Pure: no DOM, no
// fetch - the settings page and the tests drive it alike.
import type { BackupTarget, Cloud, CloudField, CloudsPayload } from "./api";

export interface CloudDraft {
  provider: string;
  // Sign-in mode per cloud id.
  auth: Record<string, string>;
  // Store setting -> value, every cloud's fields and the backup fields.
  values: Record<string, string>;
  backupTarget: string;
  // Part slot -> value for each backup target, so switching back keeps them.
  backupParts: Record<string, Record<string, string>>;
}

export function initialDraft(p: CloudsPayload): CloudDraft {
  const auth: Record<string, string> = {};
  const values: Record<string, string> = { ...p.backup.values };
  for (const c of p.clouds) {
    auth[c.id] = c.auth_value || c.auth[0]?.id || "";
    Object.assign(values, c.values);
  }
  const backupParts: Record<string, Record<string, string>> = {};
  for (const t of p.backup.targets) backupParts[t.id] = t.id === p.backup.target ? { ...p.backup.parts } : {};
  return { provider: p.provider || p.clouds[0]?.id || "", auth, values, backupTarget: p.backup.target, backupParts };
}

export function cloudOf(p: CloudsPayload, id: string): Cloud | undefined {
  return p.clouds.find((c) => c.id === id);
}

// The sign-in mode `id` names on `cloud`, its first (default) one otherwise.
export function authMode(cloud: Cloud, id: string) {
  return cloud.auth.find((a) => a.id === id) ?? cloud.auth[0];
}

// What the form shows for `cloud` under the drafted sign-in: the mode's own
// fields first, then what every machine from the cloud reads.
export function visibleFields(cloud: Cloud, draft: CloudDraft): CloudField[] {
  return [...(authMode(cloud, draft.auth[cloud.id] ?? "")?.fields ?? []), ...cloud.fields];
}

const REFERENCE = /^(op:\/\/[^/\s]+\/[^/\s]+\/[^\s]+|keychain:\/\/[^/\s]+\/[^/\s]+|file:\/\/\/\S+)$/;

// Mirrors the store's own refusals closely enough to say so while typing;
// the store has the last word when the form saves.
export function fieldProblem(f: CloudField, value: string): string {
  const v = value.trim();
  if (!v) return f.required ? f.label + " is needed." : "";
  if (f.kind === "reference") {
    return REFERENCE.test(v) ? "" : "A reference to where the secret lives - op://VAULT/ITEM/FIELD, keychain://SERVICE/ACCOUNT or file:///ABSOLUTE/PATH - never the secret itself.";
  }
  if (/\s/.test(v) && !f.setting.startsWith("compute.multipass")) return f.label + " is one word, without spaces.";
  if (/-----BEGIN|\b(ghp_|github_pat_|sk-)[A-Za-z0-9_-]{8,}|[A-Za-z0-9+/_=-]{40,}/.test(v)) return "That looks like a secret. PULSE keeps names and ids here; sign in on this machine instead.";
  return "";
}

// The required fields still empty, as the runner would refuse them.
export function missingFields(cloud: Cloud, draft: CloudDraft): CloudField[] {
  return visibleFields(cloud, draft).filter((f) => f.required && !(draft.values[f.setting] ?? "").trim());
}

// A shell read of a secret reference, so the sign-in command never shows
// the secret and the owner never pastes it.
export function readReference(ref: string): string {
  const v = ref.trim();
  if (v.startsWith("op://")) return "$(op read '" + v + "')";
  const kc = /^keychain:\/\/([^/]+)\/([^/]+)$/.exec(v);
  if (kc) return "$(security find-generic-password -s '" + kc[1] + "' -a '" + kc[2] + "' -w)";
  if (v.startsWith("file://")) return "$(cat '" + v.slice("file://".length) + "')";
  return "";
}

// The command the owner runs on this machine for the drafted sign-in, every
// {setting} slot filled; an empty slot reads <Label> so the gap shows.
export function signInCommand(cloud: Cloud, draft: CloudDraft): string {
  const mode = authMode(cloud, draft.auth[cloud.id] ?? "");
  if (!mode?.sign_in) return "";
  const byName = new Map(visibleFields(cloud, draft).map((f) => [f.setting, f]));
  return mode.sign_in.replace(/"?\{([^}]+)\}"?/g, (whole, name: string) => {
    const f = byName.get(name);
    const v = (draft.values[name] ?? "").trim();
    if (!v) return "<" + (f?.label ?? name) + ">";
    if (f?.kind === "reference") return '"' + (readReference(v) || "<" + f.label + ">") + '"';
    return whole.startsWith('"') ? '"' + v + '"' : v;
  });
}

// backup.to from a target's parts; mirrors BackupTarget::compose in Rust.
export function composeBackup(t: BackupTarget, parts: Record<string, string>): string {
  let out = t.template;
  for (const p of t.parts) {
    const slot = "{" + p.setting + "}";
    let v = (parts[p.setting] ?? "").trim();
    if (out.includes("/" + slot) || out.includes(slot + "/")) v = v.replace(/^\/+|\/+$/g, "");
    out = out.split(slot).join(v);
  }
  if (t.template.endsWith("}")) while (out.length > 1 && out.endsWith("/")) out = out.slice(0, -1);
  return out;
}

export function backupTo(p: CloudsPayload, draft: CloudDraft): string {
  const t = p.backup.targets.find((x) => x.id === draft.backupTarget);
  return t ? composeBackup(t, draft.backupParts[t.id] ?? {}) : "";
}

// The settings the drafted form changes, in the order to save them:
// provider, sign-in mode, the chosen cloud's fields, then the backup.
// Only the chosen cloud's fields are saved; a target's parts all go in one
// backup.to, and only when every required part is filled.
export function changes(p: CloudsPayload, draft: CloudDraft): [string, string][] {
  const out: [string, string][] = [];
  const was = initialDraft(p);
  const put = (name: string, value: string, before: string) => {
    if (value.trim() !== before.trim()) out.push([name, value.trim()]);
  };
  put("compute.provider", draft.provider, was.provider);
  const cloud = cloudOf(p, draft.provider);
  if (cloud) {
    if (cloud.auth_setting) put(cloud.auth_setting, draft.auth[cloud.id] ?? "", was.auth[cloud.id] ?? "");
    for (const f of visibleFields(cloud, draft)) put(f.setting, draft.values[f.setting] ?? "", was.values[f.setting] ?? "");
  }
  const t = p.backup.targets.find((x) => x.id === draft.backupTarget);
  if (t) {
    const parts = draft.backupParts[t.id] ?? {};
    if (t.parts.every((x) => !x.required || (parts[x.setting] ?? "").trim())) put("backup.to", composeBackup(t, parts), p.backup.to);
    for (const f of t.fields) put(f.setting, draft.values[f.setting] ?? "", was.values[f.setting] ?? "");
  } else if (!draft.backupTarget && p.backup.to) {
    out.push(["backup.to", ""]);
  }
  return out;
}

// Every problem that stops a save, field by field (setting -> sentence).
export function problems(p: CloudsPayload, draft: CloudDraft): Record<string, string> {
  const out: Record<string, string> = {};
  const cloud = cloudOf(p, draft.provider);
  if (cloud)
    for (const f of visibleFields(cloud, draft)) {
      const why = fieldProblem(f, draft.values[f.setting] ?? "");
      // A required field may stay empty while the owner fills the form in
      // steps; the summary line names it. Only a wrong value stops a save.
      if (why && (draft.values[f.setting] ?? "").trim()) out[f.setting] = why;
    }
  const t = p.backup.targets.find((x) => x.id === draft.backupTarget);
  if (t) {
    for (const f of t.fields) {
      const why = fieldProblem(f, draft.values[f.setting] ?? "");
      if (why && (draft.values[f.setting] ?? "").trim()) out[f.setting] = why;
    }
    const parts = draft.backupParts[t.id] ?? {};
    if (t.id === "command" && (parts.command ?? "").trim() && !(parts.command ?? "").includes("{file}")) out["backup:command"] = "The command must name {file}, the backup to send.";
    if (t.id === "folder" && (parts.path ?? "").trim() && !/^(\/|~)/.test((parts.path ?? "").trim())) out["backup:path"] = "An absolute path, or one starting with ~/.";
  }
  return out;
}

// One line under the form saying what is still needed, or that it is set.
export function readiness(p: CloudsPayload, draft: CloudDraft): string {
  const cloud = cloudOf(p, draft.provider);
  if (!cloud) return "Pick where extra machines come from.";
  const missing = missingFields(cloud, draft);
  if (missing.length) return cloud.label + " still needs " + missing.map((f) => f.label.toLowerCase()).join(", ") + ".";
  return cloud.label + " is set up. Machines are made only when an agent asks and you approve, within the limits below.";
}
