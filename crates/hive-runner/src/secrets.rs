//! Granting team vault secrets to one run (ADR-070 rule 4, step 4).
//!
//! A project's recipe declares the names its jobs may have (`secrets:`);
//! a job's brief may ask for some of them with a `Secrets: A, B` line. The
//! brain grants exactly the asked names the recipe declares, refuses the
//! rest by name, and records both (`secret.granted`, `secret.refused`).
//! On a brain that runs the job itself (LocalExec) the values are opened in
//! memory, handed to the worker process as env for that run only
//! (`secret.read` per name), and wiped when the run is done. They are never
//! written to the worktree, a log line, an event or an error: the run log
//! and the parsed result are redacted with [`SecretGrant::redact`].
//!
//! Remote hands open the team key with their own device key (step 2's
//! registry); until that lands a project whose recipe declares secrets runs
//! on the brain only ([`crate::recipe::RECIPE_UNFIT_SECRETS`]).

use crate::recipe::Recipe;
use hive_hand_core::vault::{Zeroize, Zeroizing};
use serde_json::json;
use std::collections::BTreeMap;

/// The brief line a lead writes to ask for vault secrets for one job.
pub(crate) const BRIEF_SECRETS_PREFIX: &str = "Secrets:";

/// The names a job's brief asks for: every `Secrets: A, B` line, split on
/// commas and spaces, in order, without repeats.
pub(crate) fn brief_secret_names(brief: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in brief.lines() {
        let Some(rest) = line.trim().strip_prefix(BRIEF_SECRETS_PREFIX) else {
            continue;
        };
        for name in rest.split([',', ' ', '\t']).map(str::trim) {
            let name = name.trim_matches('`');
            if !name.is_empty() && !out.iter().any(|n| n == name) {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// What the brain decided for one run: the names granted and the names
/// refused, each refusal with its reason. No values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct GrantDecision {
    pub granted: Vec<String>,
    pub refused: Vec<(String, String)>,
}

/// Decides the grant: every name the recipe declares, plus the brief's asks
/// that the recipe declares too; an ask the recipe does not declare is
/// refused with the reason.
pub(crate) fn decide(recipe: Option<&Recipe>, brief: &str) -> GrantDecision {
    let declared: Vec<String> = recipe.map(|r| r.secrets.clone()).unwrap_or_default();
    let mut d = GrantDecision {
        granted: declared.clone(),
        refused: Vec::new(),
    };
    for name in brief_secret_names(brief) {
        if declared.contains(&name) {
            continue;
        }
        let why = if let Some(problem) = hive_hand_core::vault::name_problem(&name) {
            problem
        } else {
            format!(
                "`{name}` is not declared in the project's recipe (`secrets:` in {}); a job gets only the names its project declares",
                crate::recipe::RECIPE_PATH
            )
        };
        d.refused.push((name, why));
    }
    d
}

/// The secrets one run was granted, opened in memory. Values are wiped
/// when the grant is dropped.
#[derive(Default)]
pub(crate) struct SecretGrant {
    values: Vec<(String, Zeroizing<String>)>,
}

impl SecretGrant {
    pub(crate) fn names(&self) -> Vec<String> {
        self.values.iter().map(|(n, _)| n.clone()).collect()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    /// The worker's env with the granted values added; the copy is wiped
    /// when it is dropped, right after the worker ends.
    pub(crate) fn worker_env(&self, base: &BTreeMap<String, String>) -> WorkerEnv {
        let mut map = base.clone();
        for (name, value) in &self.values {
            map.insert(name.clone(), value.to_string());
        }
        WorkerEnv(map)
    }
    /// `text` with every granted value replaced by `[secret NAME]`.
    pub(crate) fn redact(&self, text: &str) -> String {
        let mut out = text.to_string();
        // Longest first, so a value that contains another is replaced whole.
        let mut values: Vec<&(String, Zeroizing<String>)> = self.values.iter().collect();
        values.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
        for (name, value) in values {
            let v = value.trim();
            if v.len() >= 4 && out.contains(v) {
                out = out.replace(v, &format!("[secret {name}]"));
            }
        }
        out
    }
}

/// A worker's env that carries secret values: wiped on drop.
pub(crate) struct WorkerEnv(BTreeMap<String, String>);
impl std::ops::Deref for WorkerEnv {
    type Target = BTreeMap<String, String>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Drop for WorkerEnv {
    fn drop(&mut self) {
        for v in self.0.values_mut() {
            v.zeroize();
        }
    }
}

/// Who and where, for the audit events.
pub(crate) struct GrantFor<'a> {
    pub job: &'a str,
    pub run: &'a str,
    pub project: &'a str,
    pub machine: &'a str,
    pub agent: &'a str,
}

/// Grants and opens the run's secrets on the brain (LocalExec), recording
/// `secret.refused`, `secret.granted` and one `secret.read` per name. A
/// granted name the vault cannot open is refused by name, with the reason,
/// and the worker runs without it. Returns the grant and the log lines.
pub(crate) fn grant_local(
    s: &hive_store::Store,
    recipe: Option<&Recipe>,
    brief: &str,
    who: &GrantFor<'_>,
) -> (SecretGrant, Vec<String>) {
    let d = decide(recipe, brief);
    let mut lines = Vec::new();
    let mut refused = d.refused.clone();
    let mut grant = SecretGrant::default();
    for name in &d.granted {
        match s.vault_open(std::slice::from_ref(name)) {
            Ok(mut v) => grant.values.append(&mut v),
            Err(e) => refused.push((name.clone(), e.to_string())),
        }
    }
    for (name, why) in &refused {
        lines.push(format!("secret {name} refused: {why}"));
    }
    if !refused.is_empty() {
        record(
            s,
            who,
            "secret.refused",
            json!({
                "job": who.job, "run": who.run, "project": who.project, "machine": who.machine,
                "names": refused.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(),
                "reasons": refused.iter().map(|(n, w)| (n.clone(), w.clone())).collect::<BTreeMap<_, _>>(),
            }),
        );
    }
    if !grant.is_empty() {
        let names = grant.names();
        record(
            s,
            who,
            "secret.granted",
            json!({"job": who.job, "run": who.run, "project": who.project, "machine": who.machine, "names": names}),
        );
        for name in &names {
            record(
                s,
                who,
                "secret.read",
                json!({"job": who.job, "run": who.run, "machine": who.machine, "name": name}),
            );
        }
        lines.push(format!(
            "secrets granted to this run from the team vault: {} (env of the worker only)",
            names.join(", ")
        ));
    }
    (grant, lines)
}

/// Records that `machine` was refused a project's declared secrets because
/// it is a remote hand without a device key yet.
pub(crate) fn refuse_remote(s: &hive_store::Store, recipe: &Recipe, who: &GrantFor<'_>) -> String {
    let why = format!(
        "{} runs on remote hands; they open the team vault with a device key, which this brain does not hold for them yet (ADR-070 step 2), so jobs that declare secrets run on the brain",
        who.machine
    );
    record(
        s,
        who,
        "secret.refused",
        json!({
            "job": who.job, "run": who.run, "project": who.project, "machine": who.machine,
            "names": recipe.secrets, "reason": why,
        }),
    );
    why
}

fn record(s: &hive_store::Store, who: &GrantFor<'_>, action: &str, detail: serde_json::Value) {
    let actor = if who.agent.is_empty() {
        crate::util::HUMAN_AGENT
    } else {
        who.agent
    };
    if let Err(e) = s.event(actor, action, who.job, detail) {
        eprintln!("hive: {action} for {} was not recorded: {e}", who.job);
    }
}

/// `hive secret set NAME` reads the value from stdin (never an argument, so
/// it is in no shell history or process list); a typed value is refused.
pub(crate) fn secret_set(
    s: &hive_store::Store,
    actor: &str,
    name: &str,
    input: &mut dyn std::io::Read,
    is_tty: bool,
) -> anyhow::Result<String> {
    if std::env::var("HIVE_JOB").is_ok_and(|j| !j.is_empty()) {
        return Err(crate::util::fail(
            2,
            "secret set: a job never writes the team vault; the owner sets secrets",
        )
        .into());
    }
    if is_tty {
        return Err(crate::util::fail(
            2,
            format!("secret set: pipe the value in, e.g. `pbpaste | hive secret set {name}`; it is never typed or passed as an argument"),
        )
        .into());
    }
    let mut raw = Zeroizing::new(Vec::new());
    input.read_to_end(&mut raw)?;
    // One trailing newline is the shell's, not the secret's.
    let mut len = raw.len();
    if raw[..len].ends_with(b"\n") {
        len -= 1;
        if raw[..len].ends_with(b"\r") {
            len -= 1;
        }
    }
    let e = s.vault_set(actor, name, &raw[..len])?;
    Ok(format!(
        "secret {} set in the team vault ({} bytes sealed; the value is never shown again)",
        e.name, e.sealed_bytes
    ))
}

pub(crate) fn cmd_secret(
    c: &mut hive_cli::Context,
    _fs: &hive_cli::FlagSet,
    args: &[String],
) -> anyhow::Result<()> {
    let s = c
        .store
        .clone()
        .ok_or_else(|| anyhow::anyhow!("secret: store is needed"))?;
    let action = args.first().map(String::as_str).unwrap_or("list");
    let name = args.get(1).cloned().unwrap_or_default();
    let actor = crate::commands::actor_of(c);
    match action {
        "list" => {
            let entries = s.vault_entries()?;
            if c.json {
                return hive_cli::write_json(&mut c.out, &json!({"secrets": entries}));
            }
            if entries.is_empty() {
                c.println("the team vault holds no secrets; add one with `hive secret set NAME` (value on stdin)")?;
                return Ok(());
            }
            for e in entries {
                c.printf(format_args!(
                    "{}  set by {} at {}\n",
                    e.name, e.updated_by, e.updated_at
                ))?;
            }
            Ok(())
        }
        "set" | "rm" | "remove" if name.is_empty() => {
            Err(crate::util::fail(2, format!("hive secret {action} needs a NAME")).into())
        }
        "set" => {
            use std::io::IsTerminal;
            let line = secret_set(
                &s,
                &actor,
                &name,
                &mut std::io::stdin(),
                std::io::stdin().is_terminal(),
            )?;
            c.println(line)?;
            Ok(())
        }
        "rm" | "remove" => {
            if s.vault_remove(&actor, &name)? {
                c.println(format!("secret {name} removed from the team vault"))?;
                Ok(())
            } else {
                Err(crate::util::fail(1, format!("the team vault holds no {name}")).into())
            }
        }
        _ => Err(crate::util::fail(
            2,
            format!("hive secret {action}: use list, set NAME or rm NAME"),
        )
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe(secrets: &[&str]) -> Recipe {
        Recipe {
            secrets: secrets.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_brief_asks_with_secrets_lines() {
        let brief =
            "Do the thing.\nSecrets: GH_TOKEN, `NPM_TOKEN`\n  Secrets: GH_TOKEN SENTRY_DSN\n";
        assert_eq!(
            brief_secret_names(brief),
            ["GH_TOKEN", "NPM_TOKEN", "SENTRY_DSN"]
        );
        assert!(brief_secret_names("no secrets here").is_empty());
    }

    #[test]
    fn an_undeclared_name_is_refused_with_the_reason_and_declared_ones_are_granted() {
        let r = recipe(&["GH_TOKEN", "NPM_TOKEN"]);
        let d = decide(Some(&r), "Secrets: GH_TOKEN, AWS_KEY, bad-name");
        assert_eq!(d.granted, ["GH_TOKEN", "NPM_TOKEN"]);
        assert_eq!(d.refused.len(), 2);
        assert_eq!(d.refused[0].0, "AWS_KEY");
        assert!(d.refused[0].1.contains("not declared"), "{:?}", d.refused);
        assert!(
            d.refused[1].1.contains("not a secret name"),
            "{:?}",
            d.refused
        );
        let none = decide(None, "Secrets: GH_TOKEN");
        assert!(none.granted.is_empty());
        assert_eq!(none.refused[0].0, "GH_TOKEN");
    }

    #[test]
    fn redaction_replaces_every_granted_value_by_its_name() {
        let grant = SecretGrant {
            values: vec![
                ("A_TOKEN".into(), Zeroizing::new("tok-123456".into())),
                ("SHORT".into(), Zeroizing::new("ab".into())),
            ],
        };
        assert_eq!(
            grant.redact("echo tok-123456 and ab"),
            "echo [secret A_TOKEN] and ab"
        );
        let base = BTreeMap::from([("HIVE_JOB".to_string(), "j".to_string())]);
        let env = grant.worker_env(&base);
        assert_eq!(env.get("A_TOKEN").map(String::as_str), Some("tok-123456"));
        assert_eq!(env.get("HIVE_JOB").map(String::as_str), Some("j"));
        assert!(!base.contains_key("A_TOKEN"));
    }

    #[test]
    fn secret_set_reads_stdin_and_refuses_a_terminal_or_a_job() {
        let _serial = crate::test_helpers::serial();
        let s = crate::test_helpers::new_store();
        let mut input: &[u8] = b"s3cret-value\n";
        let line = secret_set(&s, "owner", "API_KEY", &mut input, false).unwrap();
        assert!(line.contains("API_KEY"), "{line}");
        assert!(!line.contains("s3cret"), "{line}");
        let opened = s.vault_open(&["API_KEY".to_string()]).unwrap();
        assert_eq!(opened[0].1.as_str(), "s3cret-value");
        let mut input: &[u8] = b"x";
        assert!(secret_set(&s, "owner", "API_KEY", &mut input, true).is_err());
    }
}
