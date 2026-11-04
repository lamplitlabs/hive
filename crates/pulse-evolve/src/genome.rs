//! The genome of an agent-program variant (ADR-049 "The genome", PLAN-050
//! step 3): the directory `~/.pulse/variants/<id>/` with `variant.yaml`,
//! `agents/`, `skills/`, `instructions.md` and `settings.toml`.
//!
//! [`read_genome`] parses every file of a proposed overlay and refuses, naming
//! the offending key or file, anything a variant may not carry: a setting off
//! the allowlist or in the immutable set (`owner.*`, `stop`, `landing*`,
//! `push.*`, `compute.*`, `harness.deny_tools`, `harness.allow_tools`), a
//! value `validate_setting` rejects, the constitution, the evaluators or
//! `STOP`, an agent or skill that widens what a run may do, and any file off
//! the layout. [`near_duplicate`] compares the normalised overlay with the
//! archive members on the same island. A variant changes how an agent thinks
//! and which model it uses, never what it is allowed to do.
//!
//! Nothing here runs a variant: the resolver (step 4) reads the installed
//! directory, and only under `evolve.mode=variants`.

use anyhow::{Result, anyhow};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The files and folders a genome may hold; anything else is refused.
pub const GENOME_LAYOUT: &[&str] = &[
    "variant.yaml",
    "agents",
    "skills",
    "instructions.md",
    "settings.toml",
];

/// Overlays at least this similar (Jaccard over their normalised lines and
/// settings) to an archive member on the same island are near-duplicates.
pub const NEAR_DUPLICATE_THRESHOLD: f64 = 0.9;

/// The most bytes one genome file may hold, and the most files a genome may
/// hold: an overlay is a few specs and settings, not a tree.
const MAX_FILE_BYTES: u64 = 256 * 1024;
const MAX_FILES: usize = 64;

/// Agent and skill front-matter keys that change what a run may do (tools,
/// permissions, hooks, MCP servers); a variant may not carry them.
const CAPABILITY_KEYS: &[&str] = &[
    "tools",
    "allowed-tools",
    "allowedTools",
    "allowed_tools",
    "permissions",
    "permission",
    "permissionMode",
    "permission-mode",
    "permission_mode",
    "hooks",
    "mcpServers",
    "mcp-servers",
    "mcp_servers",
];

/// The variant.yaml keys registration reads; the row carries the rest.
const VARIANT_YAML_KEYS: &[&str] = &[
    "id",
    "parent",
    "hypothesis",
    "projects",
    "kinds",
    "evidence",
    "proposer_job",
    "state",
];

/// What `variant.yaml` said, before the CLI's flags override it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct VariantMeta {
    pub id: String,
    pub parent: String,
    pub hypothesis: String,
    pub projects: Vec<String>,
    pub kinds: Vec<String>,
    pub evidence: Vec<String>,
}

/// One parsed genome.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Genome {
    pub meta: VariantMeta,
    /// `settings.toml`, each value as `validate_setting` normalised it.
    pub settings: BTreeMap<String, String>,
    /// `agents/<file>` -> its text.
    pub agents: BTreeMap<String, String>,
    /// `skills/<folder>/<file>` -> its text.
    pub skills: BTreeMap<String, String>,
    pub instructions: String,
}

fn refuse(what: impl std::fmt::Display) -> anyhow::Error {
    anyhow!("{what}")
}

/// Whether a file name is one of the owner's evolution records.
fn immutable_file(name: &str) -> bool {
    let stem = name.rsplit('/').next().unwrap_or(name);
    let stem = stem.split('.').next().unwrap_or(stem).to_ascii_lowercase();
    matches!(stem.as_str(), "constitution" | "evaluators" | "stop")
}

fn read_text(path: &Path, rel: &str) -> Result<String> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Err(refuse(format!(
            "{rel} is a symlink; a genome holds plain files only"
        )));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(refuse(format!(
            "{rel} is {} bytes; a genome file holds at most {MAX_FILE_BYTES}",
            meta.len()
        )));
    }
    std::fs::read_to_string(path).map_err(|e| refuse(format!("{rel} is not readable text: {e}")))
}

/// Every plain file under `dir`, as paths relative to `root`, sorted.
fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let ft = e.file_type()?;
        if ft.is_symlink() {
            return Err(refuse(format!(
                "{rel} is a symlink; a genome holds plain files only"
            )));
        }
        if ft.is_dir() {
            walk(root, &path, out)?;
        } else {
            out.push(rel);
        }
        if out.len() > MAX_FILES {
            return Err(refuse(format!(
                "the genome holds more than {MAX_FILES} files"
            )));
        }
    }
    Ok(())
}

/// Parses and checks the genome in `dir`. Every refusal names the file and,
/// for a setting, the key.
pub fn read_genome(dir: &Path) -> Result<Genome> {
    if !dir.is_dir() {
        return Err(refuse(format!("{} is not a directory", dir.display())));
    }
    let mut files = Vec::new();
    walk(dir, dir, &mut files)?;
    let mut g = Genome::default();
    for rel in &files {
        if immutable_file(rel) {
            return Err(refuse(format!(
                "{rel}: the constitution, the evaluators and STOP are in the immutable set; a variant never carries them"
            )));
        }
        let top = rel.split('/').next().unwrap_or(rel);
        let nested = rel.contains('/');
        let known = match top {
            "agents" | "skills" => nested,
            "variant.yaml" | "instructions.md" | "settings.toml" => !nested,
            _ => false,
        };
        if !known {
            return Err(refuse(format!(
                "{rel} is off the genome layout ({})",
                GENOME_LAYOUT.join(", ")
            )));
        }
        let text = read_text(&dir.join(rel), rel)?;
        match top {
            "variant.yaml" => g.meta = parse_variant_yaml(&text)?,
            "settings.toml" => g.settings = parse_settings_toml(&text)?,
            "instructions.md" => g.instructions = text,
            "agents" => {
                check_agent(rel, &text)?;
                g.agents.insert(rel.to_string(), text);
            }
            _ => {
                check_skill_file(rel, &text)?;
                g.skills.insert(rel.to_string(), text);
            }
        }
    }
    let folders: BTreeSet<String> = g
        .skills
        .keys()
        .filter_map(|k| k.split('/').nth(1).map(str::to_string))
        .collect();
    for f in folders {
        if !g.skills.contains_key(&format!("skills/{f}/SKILL.md")) {
            return Err(refuse(format!(
                "skills/{f}/ has no SKILL.md; a skill folder needs one"
            )));
        }
    }
    Ok(g)
}

fn front_matter(rel: &str, text: &str) -> Result<pulse_records::records::Frontmatter> {
    let (front, _) = pulse_records::frontmatter::parse_frontmatter(text)
        .map_err(|e| refuse(format!("{rel}: front matter does not parse: {e}")))?;
    Ok(front)
}

fn check_capabilities(rel: &str, front: &pulse_records::records::Frontmatter) -> Result<()> {
    if let Some(k) = front.keys().find(|k| CAPABILITY_KEYS.contains(&k.as_str())) {
        return Err(refuse(format!(
            "{rel}: {k} changes what a run may do; a variant changes how an agent thinks and which model it uses, never what it is allowed to do"
        )));
    }
    Ok(())
}

fn check_agent(rel: &str, text: &str) -> Result<()> {
    if rel.matches('/').count() != 1 || !rel.ends_with(".md") {
        return Err(refuse(format!(
            "{rel}: agents/ holds agent spec files agents/<name>.md"
        )));
    }
    let front = front_matter(rel, text)?;
    check_capabilities(rel, &front)
}

fn check_skill_file(rel: &str, text: &str) -> Result<()> {
    if rel.matches('/').count() < 2 {
        return Err(refuse(format!(
            "{rel}: skills/ holds skill folders skills/<name>/SKILL.md"
        )));
    }
    if rel.ends_with("/SKILL.md") && rel.matches('/').count() == 2 {
        let front = front_matter(rel, text)?;
        check_capabilities(rel, &front)?;
    }
    Ok(())
}

fn string_list(key: &str, v: &serde_json::Value) -> Result<Vec<String>> {
    match v {
        serde_json::Value::String(s) => Ok(s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect()),
        serde_json::Value::Array(items) => items
            .iter()
            .map(|i| {
                i.as_str()
                    .map(|s| s.trim().to_string())
                    .ok_or_else(|| refuse(format!("variant.yaml: {key} must list strings")))
            })
            .collect(),
        serde_json::Value::Null => Ok(Vec::new()),
        _ => Err(refuse(format!("variant.yaml: {key} must be a list"))),
    }
}

fn scalar(key: &str, v: &serde_json::Value) -> Result<String> {
    match v {
        serde_json::Value::String(s) => Ok(s.trim().to_string()),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::Null => Ok(String::new()),
        _ => Err(refuse(format!("variant.yaml: {key} must be a string"))),
    }
}

/// Parses `variant.yaml` (the YAML subset the records use).
pub fn parse_variant_yaml(text: &str) -> Result<VariantMeta> {
    let wrapped = format!("---\n{}\n---\n", text.trim_end());
    let front = front_matter("variant.yaml", &wrapped)?;
    let mut m = VariantMeta::default();
    for (k, v) in &front {
        match k.as_str() {
            "id" => m.id = scalar(k, v)?,
            "parent" => m.parent = scalar(k, v)?,
            "hypothesis" => m.hypothesis = scalar(k, v)?,
            "projects" => m.projects = string_list(k, v)?,
            "kinds" => m.kinds = string_list(k, v)?,
            "evidence" => m.evidence = string_list(k, v)?,
            "proposer_job" | "state" => {}
            _ => {
                return Err(refuse(format!(
                    "variant.yaml: {k} is not a variant field ({})",
                    VARIANT_YAML_KEYS.join(", ")
                )));
            }
        }
    }
    Ok(m)
}

/// Strips a `#` comment that is not inside a quoted string.
fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, c) {
            (Some('"'), '\\') => escaped = true,
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, '#') => return &line[..i],
            _ => {}
        }
    }
    line
}

fn toml_value(raw: &str, at: &str) -> Result<String> {
    let raw = raw.trim();
    if let Some(body) = raw.strip_prefix('"') {
        let Some(body) = body.strip_suffix('"') else {
            return Err(refuse(format!("{at}: unterminated string")));
        };
        let mut out = String::new();
        let mut chars = body.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                other => {
                    return Err(refuse(format!(
                        "{at}: unsupported escape \\{}",
                        other.map(String::from).unwrap_or_default()
                    )));
                }
            }
        }
        return Ok(out);
    }
    if let Some(body) = raw.strip_prefix('\'') {
        return body
            .strip_suffix('\'')
            .map(str::to_string)
            .ok_or_else(|| refuse(format!("{at}: unterminated string")));
    }
    if raw.is_empty() || raw.starts_with('[') || raw.starts_with('{') {
        return Err(refuse(format!(
            "{at}: a setting value is a string, a number or a boolean"
        )));
    }
    Ok(raw.to_string())
}

/// Parses `settings.toml` (flat keys, optional `[table]` prefixes, string,
/// number and boolean values) and checks each key: the immutable set and the
/// allowlist first, then the setting's own rule through `validate_setting`.
/// The first offending key, in file order, is named.
pub fn parse_settings_toml(text: &str) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let mut table = String::new();
    for (n, line) in text.lines().enumerate() {
        let at = format!("settings.toml:{}", n + 1);
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let Some(name) = rest.strip_suffix(']') else {
                return Err(refuse(format!("{at}: unterminated table header")));
            };
            let name = name.trim().trim_matches('"');
            if name.is_empty() || name.starts_with('[') {
                return Err(refuse(format!("{at}: unsupported table header")));
            }
            table = format!("{name}.");
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            return Err(refuse(format!("{at}: expected key = value")));
        };
        let key = k.trim().trim_matches('"').trim_matches('\'');
        if key.is_empty() {
            return Err(refuse(format!("{at}: empty key")));
        }
        let name = format!("{table}{key}");
        if let Some(why) = pulse_store::variant_setting_refusal(&name) {
            return Err(refuse(format!("settings.toml: {why}")));
        }
        let value = toml_value(v, &at)?;
        let value = pulse_store::validate_setting(&name, &value)
            .map_err(|e| refuse(format!("settings.toml: {name}: {e}")))?;
        if out.insert(name.clone(), value).is_some() {
            return Err(refuse(format!("{at}: {name} is set twice")));
        }
    }
    Ok(out)
}

fn normal_line(line: &str) -> String {
    line.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

impl Genome {
    /// The normalised overlay: one feature per setting and per non-empty
    /// line of each text, whitespace collapsed and lowercased, with the
    /// file's role but not its name (a renamed copy is still a copy).
    pub fn features(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for (k, v) in &self.settings {
            out.insert(format!("setting:{k}={}", normal_line(v)));
        }
        let texts = self
            .agents
            .values()
            .map(|t| ("agent", t))
            .chain(self.skills.values().map(|t| ("skill", t)))
            .chain(std::iter::once(("instructions", &self.instructions)));
        for (role, text) in texts {
            for line in text.lines() {
                let l = normal_line(line);
                if !l.is_empty() && l != "---" {
                    out.insert(format!("{role}:{l}"));
                }
            }
        }
        out
    }

    /// A short digest of the normalised overlay.
    pub fn digest(&self) -> String {
        let mut h = Sha256::new();
        for f in self.features() {
            h.update(f.as_bytes());
            h.update(b"\n");
        }
        h.finalize()
            .iter()
            .take(6)
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

/// Jaccard similarity of two feature sets; two empty overlays are equal.
pub fn similarity(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        return 1.0;
    }
    a.intersection(b).count() as f64 / union as f64
}

/// Where variant `id`'s genome lives under PULSE home `home`.
pub fn variant_dir(home: &str, id: &str) -> PathBuf {
    Path::new(home).join("variants").join(id)
}

/// The installed genome of `v`; `base` (and a row without a directory) is the
/// empty overlay.
pub fn installed_genome(v: &pulse_store::Variant) -> Result<Genome> {
    if v.dir.trim().is_empty() {
        return Ok(Genome::default());
    }
    read_genome(Path::new(&v.dir))
}

/// The archive member on island `slug` whose overlay is within the
/// threshold of `g`, with the similarity; the closest one when several are.
/// Retired members count: the archive is never pruned. A member whose
/// directory no longer reads is skipped.
pub fn near_duplicate(
    archive: &[pulse_store::Variant],
    slug: &str,
    g: &Genome,
) -> Option<(String, f64)> {
    let mine = g.features();
    archive
        .iter()
        .filter(|v| v.on_island(slug))
        .filter_map(|v| {
            let theirs = installed_genome(v).ok()?.features();
            Some((v.id.clone(), similarity(&mine, &theirs)))
        })
        .filter(|(_, sim)| *sim >= NEAR_DUPLICATE_THRESHOLD)
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

/// Copies the checked genome from `src` to `dest` (which must not exist) and
/// writes its `variant.yaml` from the registered row.
pub fn install_genome(
    src: &Path,
    dest: &Path,
    g: &Genome,
    v: &pulse_store::NewVariant,
) -> Result<()> {
    if dest.exists() {
        return Err(refuse(format!("{} already exists", dest.display())));
    }
    std::fs::create_dir_all(dest)?;
    let copy = || -> Result<()> {
        let files = g
            .agents
            .keys()
            .chain(g.skills.keys())
            .map(String::as_str)
            .chain(
                ["instructions.md", "settings.toml"]
                    .into_iter()
                    .filter(|f| src.join(f).is_file()),
            );
        for rel in files {
            let to = dest.join(rel);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(src.join(rel), &to)?;
        }
        std::fs::write(dest.join("variant.yaml"), variant_yaml(v))?;
        Ok(())
    };
    copy().inspect_err(|_| {
        let _ = std::fs::remove_dir_all(dest);
    })
}

fn yaml_quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

fn yaml_list(items: &[String]) -> String {
    format!(
        "[{}]",
        items
            .iter()
            .map(|i| yaml_quote(i))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// The `variant.yaml` the archive keeps beside the overlay.
pub fn variant_yaml(v: &pulse_store::NewVariant) -> String {
    format!(
        "id: {}\nparent: {}\nhypothesis: {}\nprojects: {}\nkinds: {}\nevidence: {}\nproposer_job: {}\n",
        yaml_quote(&v.id),
        yaml_quote(&v.parent),
        yaml_quote(v.hypothesis.trim()),
        yaml_list(&v.projects),
        yaml_list(&v.kinds),
        yaml_list(&v.evidence),
        yaml_quote(&v.proposer_job),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn genome_dir(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (rel, text) in files {
            let p = d.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        d
    }

    #[test]
    fn settings_toml_takes_the_allowlist_and_names_an_offending_key() {
        let ok = parse_settings_toml(
            "# model class\nworker_model = \"gpt-5\"\n[harness]\nmax_steps = 40 # cap\n",
        )
        .unwrap();
        assert_eq!(ok.get("worker_model").map(String::as_str), Some("gpt-5"));
        assert_eq!(ok.get("harness.max_steps").map(String::as_str), Some("40"));
        for (text, key) in [
            ("owner.name = \"x\"", "owner.name"),
            ("stop = \"1\"", "stop"),
            ("landing.verify = \"off\"", "landing.verify"),
            ("[push]\nremote = \"origin\"", "push.remote"),
            ("[compute]\nprovider = \"azure\"", "compute.provider"),
            ("harness.deny_tools = \"\"", "harness.deny_tools"),
            ("harness.allow_tools = \"bash\"", "harness.allow_tools"),
            ("daemon.pass_seconds = 5", "daemon.pass_seconds"),
        ] {
            let err = parse_settings_toml(text).unwrap_err().to_string();
            assert!(err.contains(key), "{text}: {err}");
        }
        let err = parse_settings_toml("harness.max_steps = \"many\"").unwrap_err();
        assert!(err.to_string().contains("harness.max_steps"), "{err}");
        let err = parse_settings_toml("worker_model = \"a\"\nworker_model = \"b\"").unwrap_err();
        assert!(err.to_string().contains("twice"), "{err}");
    }

    #[test]
    fn a_genome_refuses_the_owners_records_capabilities_and_stray_files() {
        for (files, needle) in [
            (vec![("constitution.md", "x")], "constitution.md"),
            (vec![("agents/STOP", "x")], "agents/STOP"),
            (vec![("evaluators.md", "x")], "evaluators.md"),
            (vec![("briefs/lead.md", "x")], "briefs/lead.md"),
            (
                vec![("agents/a.md", "---\nname: a\ntools: [bash]\n---\nbody\n")],
                "tools",
            ),
            (
                vec![(
                    "skills/s/SKILL.md",
                    "---\nname: s\nallowed-tools: bash\n---\n",
                )],
                "allowed-tools",
            ),
            (vec![("skills/s/notes.md", "x")], "SKILL.md"),
            (vec![("variant.yaml", "owner: me\n")], "owner"),
        ] {
            let d = genome_dir(&files);
            let err = read_genome(d.path()).unwrap_err().to_string();
            assert!(err.contains(needle), "{files:?}: {err}");
        }
    }

    #[test]
    fn a_genome_parses_every_file_and_a_renamed_copy_is_a_near_duplicate() {
        let d = genome_dir(&[
            (
                "variant.yaml",
                "id: terse\nhypothesis: shorter briefs land more per token\nkinds: [task, improve]\n",
            ),
            (
                "agents/terse.md",
                "---\nname: terse\nmodel: m\n---\nBe terse.\n",
            ),
            (
                "skills/s/SKILL.md",
                "---\nname: s\ndescription: d\n---\nSteps.\n",
            ),
            ("instructions.md", "Prefer small commits.\n"),
            ("settings.toml", "light_reasoning = \"low\"\n"),
        ]);
        let g = read_genome(d.path()).unwrap();
        assert_eq!(g.meta.id, "terse");
        assert_eq!(g.meta.kinds, vec!["task", "improve"]);
        assert_eq!(g.agents.len(), 1);
        assert_eq!(g.skills.len(), 1);
        assert!(g.instructions.contains("small commits"));
        let mut renamed = g.clone();
        let text = renamed.agents.remove("agents/terse.md").unwrap();
        renamed.agents.insert(
            "agents/other.md".into(),
            text.replace("Be  terse.", "be terse."),
        );
        assert_eq!(similarity(&g.features(), &renamed.features()), 1.0);
        assert_eq!(g.digest(), renamed.digest());
        assert!(similarity(&g.features(), &Genome::default().features()) < 0.5);
        assert_eq!(
            similarity(&Genome::default().features(), &Genome::default().features()),
            1.0
        );
    }
}
