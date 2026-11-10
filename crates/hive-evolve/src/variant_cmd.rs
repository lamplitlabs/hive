//! `hive evolve variant new|list|show|retire|promote` (ADR-049, PLAN-050
//! step 3): registration of agent-program variants and the by-hand moves of
//! the incumbent pointer.
//!
//! `new` and `promote` are refused unless `evolve.mode=variants`, so the
//! default fixtures mode has nothing to run. `list` and `show` only read;
//! `retire` is the safe direction and works in either mode. `promote` moves
//! a project's incumbent, which the promotion test (step 8) owns, so only the
//! owner runs it by hand.

use crate::commands::{calling_actor, current_project, parse_command_flags, with_store};
use crate::fixtures::input_error;
use crate::genome::{self, Genome};
use anyhow::Result;
use serde::Serialize;
use std::path::Path;

const VARIANT_SUBS: &str = "choose new, list, show, retire or promote";

#[derive(Serialize)]
struct Shown {
    #[serde(flatten)]
    variant: hive_store::Variant,
    projects: Vec<String>,
    kinds: Vec<String>,
    incumbent_of: Vec<String>,
    digest: String,
    genome: Option<Genome>,
    #[serde(skip_serializing_if = "String::is_empty")]
    genome_error: String,
}

fn base_row(s: &hive_store::Store) -> hive_store::Variant {
    hive_store::Variant {
        id: hive_store::BASE_VARIANT.to_string(),
        project_scope: "[]".into(),
        kinds_scope: "[]".into(),
        hypothesis:
            "the empty overlay: the agent program as the repository and the store define it".into(),
        state: "promoted".into(),
        evidence: "[]".into(),
        created_at: s.now(),
        ..Default::default()
    }
}

/// Every row, with `base` first even before its first use writes it.
fn archive(s: &hive_store::Store) -> Result<Vec<hive_store::Variant>> {
    let mut rows = s.variants()?;
    if !rows.iter().any(|v| v.id == hive_store::BASE_VARIANT) {
        rows.insert(0, base_row(s));
    }
    Ok(rows)
}

fn incumbent_of(s: &hive_store::Store, id: &str) -> Result<Vec<String>> {
    Ok(s.projects(true)?
        .into_iter()
        .filter(|p| p.slug != hive_store::COMMONS_SLUG && s.incumbent(&p.slug) == id)
        .map(|p| p.slug)
        .collect())
}

fn require_variants_mode(s: &hive_store::Store, what: &str) -> Result<()> {
    if s.variants_mode() {
        return Ok(());
    }
    Err(input_error(
        &format!("evolve variant {what} needs evolve.mode=variants"),
        "the agent-program loop is off (evolve.mode=fixtures); nothing would run a variant",
        "hive settings set evolve.mode variants",
    ))
}

fn split_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

fn show(s: &hive_store::Store, v: hive_store::Variant) -> Result<Shown> {
    let (genome, genome_error) = match genome::installed_genome(&v) {
        Ok(g) => (Some(g), String::new()),
        Err(e) => (None, e.to_string()),
    };
    Ok(Shown {
        projects: v.projects(),
        kinds: v.kinds(),
        incumbent_of: incumbent_of(s, &v.id)?,
        digest: genome.as_ref().map(Genome::digest).unwrap_or_default(),
        genome,
        genome_error,
        variant: v,
    })
}

fn print_shown(c: &mut hive_cli::Context, sh: &Shown) -> Result<()> {
    let v = &sh.variant;
    let or_all = |x: &[String]| {
        if x.is_empty() {
            "all".to_string()
        } else {
            x.join(", ")
        }
    };
    c.println(format!("Variant {}  ({})", v.id, v.state))?;
    c.println(format!("  parent      {}", or_value(&v.parent)))?;
    c.println(format!("  hypothesis  {}", v.hypothesis))?;
    c.println(format!("  islands     {}", or_all(&sh.projects)))?;
    c.println(format!("  kinds       {}", or_all(&sh.kinds)))?;
    c.println(format!(
        "  incumbent   {}",
        or_value(&sh.incumbent_of.join(", "))
    ))?;
    c.println(format!("  created     {}", v.created_at))?;
    if !v.promoted_at.is_empty() {
        c.println(format!("  promoted    {}", v.promoted_at))?;
    }
    if !v.retired_at.is_empty() {
        c.println(format!("  retired     {}", v.retired_at))?;
    }
    c.println(format!("  dir         {}", or_value(&v.dir)))?;
    if !sh.genome_error.is_empty() {
        c.println(format!("  genome      unreadable: {}", sh.genome_error))?;
        return Ok(());
    }
    let Some(g) = &sh.genome else {
        return Ok(());
    };
    c.println(format!("  digest      {}", sh.digest))?;
    if g.settings.is_empty()
        && g.agents.is_empty()
        && g.skills.is_empty()
        && g.instructions.is_empty()
    {
        c.println("  overlay     (empty)")?;
        return Ok(());
    }
    for (k, val) in &g.settings {
        c.println(format!("  setting     {k} = {val}"))?;
    }
    for f in g.agents.keys().chain(g.skills.keys()) {
        c.println(format!("  file        {f}"))?;
    }
    if !g.instructions.trim().is_empty() {
        c.println("  instructions.md:")?;
        for line in g.instructions.lines() {
            c.println(format!("    {line}"))?;
        }
    }
    Ok(())
}

fn or_value(s: &str) -> &str {
    if s.trim().is_empty() { "-" } else { s }
}

pub(crate) fn run_variant(c: &mut hive_cli::Context, args: &[String]) -> Result<()> {
    let Some(sub) = args.first().cloned() else {
        return Err(input_error(
            "Variant command required",
            VARIANT_SUBS,
            "hive help evolve",
        ));
    };
    let rest = &args[1..];
    let mut fs = hive_cli::FlagSet::new(format!("evolve variant {sub}"));
    crate::commands::add_common_flags(&mut fs, c);
    match sub.as_str() {
        "new" => {
            fs.string_var("id", "", "variant id (default: variant.yaml's id)");
            fs.string_var(
                "parent",
                "",
                "parent variant (default: the project's incumbent)",
            );
            fs.string_var(
                "hypothesis",
                "",
                "one line: the metric it should move and why",
            );
            fs.string_var("dir", "", "the genome directory to register");
            fs.string_var("kinds", "", "job kinds it may run, comma-separated (default: variant.yaml's, else the trial kinds)");
            fs.string_var("evidence", "", "evidence post ids, comma-separated");
            fs.string_var("proposer-job", "", "the job that proposed it");
        }
        "retire" | "promote" => {
            fs.string_var("reason", "", "why, kept on the event");
        }
        "list" | "show" => {}
        _ => {
            return Err(input_error(
                "Unknown variant command",
                VARIANT_SUBS,
                "hive help evolve",
            ));
        }
    }
    parse_command_flags(c, &mut fs, rest)?;
    crate::commands::apply_common_flags(c, &fs);
    let pos = fs.args().to_vec();
    let want = usize::from(matches!(sub.as_str(), "show" | "retire" | "promote"));
    if pos.len() != want {
        return Err(input_error(
            "Wrong variant arguments",
            "show, retire and promote take one variant id; new and list take flags",
            "hive help evolve",
        ));
    }
    let read_only = matches!(sub.as_str(), "list" | "show");
    let mut sc = crate::commands::store_context(c);
    with_store(&mut sc, read_only, |s| match sub.as_str() {
        "list" => {
            let rows = archive(s)?
                .into_iter()
                .map(|v| show(s, v))
                .collect::<Result<Vec<_>>>()?;
            let rows: Vec<Shown> = if c.project.is_empty() {
                rows
            } else {
                rows.into_iter()
                    .filter(|r| r.variant.on_island(&c.project))
                    .collect()
            };
            if c.json {
                let rows: Vec<_> = rows
                    .into_iter()
                    .map(|mut r| {
                        r.genome = None;
                        r
                    })
                    .collect();
                return hive_cli::write_json(
                    &mut *c.out,
                    &hive_store::gomap! {"mode": s.setting(hive_store::EVOLVE_MODE, hive_store::MODE_FIXTURES), "variants": rows},
                );
            }
            if !c.quiet {
                let table: Vec<Vec<String>> = rows
                    .iter()
                    .map(|r| {
                        vec![
                            r.variant.id.clone(),
                            r.variant.state.clone(),
                            or_value(&r.variant.parent).to_string(),
                            if r.projects.is_empty() {
                                "all".into()
                            } else {
                                r.projects.join(",")
                            },
                            or_value(&r.incumbent_of.join(",")).to_string(),
                            r.variant.hypothesis.clone(),
                        ]
                    })
                    .collect();
                hive_cli::table(
                    &mut *c.out,
                    c.color,
                    Some(&[
                        "ID",
                        "STATE",
                        "PARENT",
                        "ISLANDS",
                        "INCUMBENT",
                        "HYPOTHESIS",
                    ]),
                    &table,
                )?;
            }
            Ok(())
        }
        "show" => {
            let id = &pos[0];
            let v = if id == hive_store::BASE_VARIANT {
                s.variant(id)?.unwrap_or_else(|| base_row(s))
            } else {
                s.variant(id)?.ok_or_else(|| {
                    input_error(
                        &format!("No variant {id}"),
                        "no registered variant has that id",
                        "hive evolve variant list",
                    )
                })?
            };
            let sh = show(s, v)?;
            if c.json {
                return hive_cli::write_json(&mut *c.out, &sh);
            }
            print_shown(c, &sh)
        }
        "retire" => {
            let actor = calling_actor(s, c)?;
            let moved = s.retire_variant(&actor, &pos[0], &fs.get_string("reason"))?;
            if c.json {
                let moved: Vec<_> = moved
                    .iter()
                    .map(|(p, to)| serde_json::json!({"project": p, "to": to}))
                    .collect();
                return hive_cli::write_json(
                    &mut *c.out,
                    &hive_store::gomap! {"id": pos[0], "state": "retired", "pointers": moved},
                );
            }
            c.println(format!("Variant {} retired", pos[0]))?;
            for (p, to) in moved {
                c.println(format!("  {p}: incumbent back to {to}"))?;
            }
            Ok(())
        }
        "promote" => {
            require_variants_mode(s, "promote")?;
            let actor = calling_actor(s, c)?;
            if !hive_store::human_actor(&actor) {
                return Err(input_error(
                    "Only the owner promotes a variant by hand",
                    "the promotion test moves the incumbent pointer; an agent may register or retire a variant, never promote one",
                    "hive evolve variant show ID",
                ));
            }
            s.ensure_base_variant()?;
            let p = current_project(s, c)?;
            let was = s.promote_variant(&actor, &pos[0], &p.slug, &fs.get_string("reason"))?;
            if c.json {
                return hive_cli::write_json(
                    &mut *c.out,
                    &hive_store::gomap! {"id": pos[0], "project": p.slug, "was": was},
                );
            }
            c.println(format!(
                "Variant {} is the incumbent of {} (was {was})",
                pos[0], p.slug
            ))?;
            Ok(())
        }
        _ => register(s, c, &fs),
    })
}

fn register(
    s: &hive_store::Store,
    c: &mut hive_cli::Context,
    fs: &hive_cli::FlagSet,
) -> Result<()> {
    require_variants_mode(s, "new")?;
    let p = current_project(s, c)?;
    let actor = calling_actor(s, c)?;
    let src = fs.get_string("dir");
    if src.trim().is_empty() {
        return Err(input_error(
            "A variant needs its genome",
            "--dir names the directory with variant.yaml, agents/, skills/, instructions.md and settings.toml",
            "hive help evolve",
        ));
    }
    let src = Path::new(&src);
    let g = genome::read_genome(src)
        .map_err(|e| input_error("Variant refused", &e.to_string(), "hive help evolve"))?;
    s.ensure_base_variant()?;
    let pick = |flag: &str, yaml: &str| {
        let v = fs.get_string(flag);
        if v.trim().is_empty() {
            yaml.trim().to_string()
        } else {
            v.trim().to_string()
        }
    };
    let id = pick("id", &g.meta.id);
    if !g.meta.id.is_empty() && g.meta.id != id {
        return Err(input_error(
            "Variant refused",
            &format!("--id {id} and variant.yaml's id {} disagree", g.meta.id),
            "hive help evolve",
        ));
    }
    let parent = {
        let v = pick("parent", &g.meta.parent);
        if v.is_empty() {
            s.incumbent(&p.slug)
        } else {
            v
        }
    };
    let mut projects = g.meta.projects.clone();
    if !projects.contains(&p.slug) {
        projects.insert(0, p.slug.clone());
    }
    for slug in &projects {
        if s.project(slug).is_err() {
            return Err(input_error(
                "Variant refused",
                &format!("island {slug} is not a registered project"),
                "hive projects",
            ));
        }
    }
    let kinds = {
        let k = split_list(&fs.get_string("kinds"));
        if k.is_empty() {
            g.meta.kinds.clone()
        } else {
            k
        }
    };
    let evidence = {
        let e = split_list(&fs.get_string("evidence"));
        if e.is_empty() {
            g.meta.evidence.clone()
        } else {
            e
        }
    };
    let archive = archive(s)?;
    for slug in &projects {
        if let Some((member, sim)) = genome::near_duplicate(&archive, slug, &g) {
            return Err(input_error(
                "Variant refused",
                &format!(
                    "near-duplicate of archive member {member} on island {slug} (similarity {:.2} >= {:.2}); change the overlay or build on {member}",
                    sim,
                    genome::NEAR_DUPLICATE_THRESHOLD
                ),
                &format!("hive evolve variant show {member}"),
            ));
        }
    }
    let dest = genome::variant_dir(&s.home, &id);
    let nv = hive_store::NewVariant {
        id: id.clone(),
        projects,
        kinds,
        parent,
        hypothesis: pick("hypothesis", &g.meta.hypothesis),
        proposer_job: fs.get_string("proposer-job"),
        evidence,
        dir: dest.display().to_string(),
    };
    if nv.id.is_empty()
        || !hive_store::valid_variant_id(&nv.id)
        || nv.id == hive_store::BASE_VARIANT
    {
        return Err(input_error(
            "Variant refused",
            &format!(
                "{:?} is not a variant id (letters, digits, . _ -; not base)",
                nv.id
            ),
            "hive help evolve",
        ));
    }
    if s.variant(&nv.id)?.is_some() {
        return Err(input_error(
            "Variant refused",
            &format!("variant {} is already registered", nv.id),
            &format!("hive evolve variant show {}", nv.id),
        ));
    }
    if nv.hypothesis.is_empty() {
        return Err(input_error(
            "Variant refused",
            "a variant needs a one-line hypothesis naming the metric it should move",
            "hive help evolve",
        ));
    }
    if dest.exists() {
        return Err(input_error(
            "Variant refused",
            &format!(
                "{} already exists; an archive directory is never reused",
                dest.display()
            ),
            "hive evolve variant list",
        ));
    }
    genome::install_genome(src, &dest, &g, &nv)?;
    let v = match s.insert_variant(&actor, &nv) {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&dest);
            return Err(e);
        }
    };
    let sh = show(s, v)?;
    if c.json {
        return hive_cli::write_json(&mut *c.out, &sh);
    }
    c.println(format!(
        "Variant {} registered (proposed, parent {}, digest {})",
        sh.variant.id, sh.variant.parent, sh.digest
    ))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::test_helpers::{
        join_test_agent, require_json, run_test_command, serial, test_project, test_store,
    };

    fn genome(files: &[(&str, &str)]) -> String {
        let d = crate::test_helpers::temp_path();
        for (rel, text) in files {
            let p = d.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        d.display().to_string()
    }

    fn variants_on(s: &hive_store::Store) {
        s.set_setting(
            hive_store::HUMAN_AGENT,
            hive_store::EVOLVE_MODE,
            hive_store::MODE_VARIANTS,
        )
        .unwrap();
    }

    #[test]
    fn new_is_refused_in_fixtures_mode_and_list_shows_base() {
        let _g = serial();
        crate::register::register();
        let s = test_store();
        test_project(&s, "demo");
        join_test_agent(&s, "tester");
        let dir = genome(&[("settings.toml", "light_reasoning = \"low\"\n")]);
        let (_, _, r) = run_test_command(
            &s,
            "evolve",
            &[
                "variant",
                "new",
                "--dir",
                &dir,
                "--id",
                "v1",
                "--hypothesis",
                "h",
            ],
        );
        let err = format!("{:#}", r.unwrap_err());
        assert!(err.contains("evolve.mode=variants"), "{err}");
        assert!(
            s.variants().unwrap().is_empty(),
            "fixtures mode wrote a row"
        );
        let (out, _, r) = run_test_command(&s, "evolve", &["variant", "list", "--json"]);
        r.unwrap();
        require_json(&out);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["mode"], "fixtures");
        assert_eq!(v["variants"][0]["id"], "base");
        assert_eq!(v["variants"][0]["incumbent_of"][0], "demo");
    }

    #[test]
    fn new_refuses_the_immutable_set_naming_the_key_and_the_owners_records() {
        let _g = serial();
        crate::register::register();
        let s = test_store();
        test_project(&s, "demo");
        join_test_agent(&s, "tester");
        variants_on(&s);
        for (files, needle) in [
            (
                vec![("settings.toml", "owner.name = \"x\"\n")],
                "owner.name",
            ),
            (vec![("settings.toml", "stop = \"1\"\n")], "stop"),
            (
                vec![("settings.toml", "landing.verify = \"off\"\n")],
                "landing.verify",
            ),
            (
                vec![("settings.toml", "push.remote = \"x\"\n")],
                "push.remote",
            ),
            (
                vec![("settings.toml", "compute.provider = \"azure\"\n")],
                "compute.provider",
            ),
            (
                vec![("settings.toml", "harness.deny_tools = \"\"\n")],
                "harness.deny_tools",
            ),
            (
                vec![("settings.toml", "harness.allow_tools = \"bash\"\n")],
                "harness.allow_tools",
            ),
            (vec![("constitution.md", "x\n")], "constitution"),
            (vec![("STOP", "\n")], "STOP"),
        ] {
            let dir = genome(&files);
            let (_, _, r) = run_test_command(
                &s,
                "evolve",
                &[
                    "variant",
                    "new",
                    "--dir",
                    &dir,
                    "--id",
                    "bad",
                    "--hypothesis",
                    "h",
                ],
            );
            let err = format!("{:#}", r.unwrap_err());
            assert!(err.contains(needle), "{files:?}: {err}");
        }
        assert!(s.variants().unwrap().iter().all(|v| v.id == "base"));
        assert!(!std::path::Path::new(&s.home).join("variants/bad").exists());
    }

    #[test]
    fn new_registers_show_prints_it_and_a_near_duplicate_names_the_member() {
        let _g = serial();
        crate::register::register();
        let s = test_store();
        test_project(&s, "demo");
        join_test_agent(&s, "tester");
        variants_on(&s);
        let dir = genome(&[
            (
                "variant.yaml",
                "id: terse\nhypothesis: terse briefs raise kept landings per token\n",
            ),
            (
                "agents/terse.md",
                "---\nname: terse\ndescription: d\n---\nBe terse.\n",
            ),
            ("instructions.md", "Prefer one small commit.\n"),
            ("settings.toml", "light_reasoning = \"low\"\n"),
        ]);
        let (out, _, r) =
            run_test_command(&s, "evolve", &["variant", "new", "--dir", &dir, "--json"]);
        r.unwrap();
        require_json(&out);
        let v = s.variant("terse").unwrap().expect("registered");
        assert_eq!(v.state, "proposed");
        assert_eq!(v.parent, "base");
        assert_eq!(v.projects(), vec!["demo"]);
        let installed = std::path::Path::new(&s.home).join("variants/terse");
        assert_eq!(v.dir, installed.display().to_string());
        assert!(installed.join("agents/terse.md").is_file());
        assert!(installed.join("variant.yaml").is_file());
        let (out, _, r) = run_test_command(&s, "evolve", &["variant", "show", "terse"]);
        r.unwrap();
        assert!(out.contains("light_reasoning = low"), "{out}");
        assert!(out.contains("agents/terse.md"), "{out}");
        assert!(out.contains("Prefer one small commit."), "{out}");

        let copy = genome(&[
            (
                "agents/renamed.md",
                "---\nname:  terse\ndescription: d\n---\nBe   terse.\n",
            ),
            ("instructions.md", "prefer one small commit.\n"),
            ("settings.toml", "light_reasoning = \"low\"\n"),
        ]);
        let (_, _, r) = run_test_command(
            &s,
            "evolve",
            &[
                "variant",
                "new",
                "--dir",
                &copy,
                "--id",
                "terse-2",
                "--hypothesis",
                "h",
            ],
        );
        let err = format!("{:#}", r.unwrap_err());
        assert!(
            err.contains("near-duplicate of archive member terse"),
            "{err}"
        );
        let empty = genome(&[]);
        let (_, _, r) = run_test_command(
            &s,
            "evolve",
            &[
                "variant",
                "new",
                "--dir",
                &empty,
                "--id",
                "empty",
                "--hypothesis",
                "h",
            ],
        );
        let err = format!("{:#}", r.unwrap_err());
        assert!(
            err.contains("near-duplicate of archive member base"),
            "{err}"
        );
        let registered = s
            .variants()
            .unwrap()
            .iter()
            .filter(|v| v.id != "base")
            .count();
        assert_eq!(registered, 1);
    }

    #[test]
    fn only_the_owner_promotes_and_retire_moves_the_pointer_back() {
        let _g = serial();
        crate::register::register();
        let s = test_store();
        test_project(&s, "demo");
        join_test_agent(&s, "tester");
        variants_on(&s);
        let dir = genome(&[("settings.toml", "worker_reasoning = \"high\"\n")]);
        let (_, _, r) = run_test_command(
            &s,
            "evolve",
            &[
                "variant",
                "new",
                "--dir",
                &dir,
                "--id",
                "deep",
                "--hypothesis",
                "more reasoning, fewer failed jobs",
            ],
        );
        r.unwrap();
        let (_, _, r) = run_test_command(&s, "evolve", &["variant", "promote", "deep"]);
        let err = format!("{:#}", r.unwrap_err());
        assert!(err.contains("Only the owner"), "{err}");
        assert_eq!(s.incumbent("demo"), "base");
        let (_, _, r) = run_test_command(
            &s,
            "evolve",
            &[
                "variant", "promote", "deep", "--agent", "", "--reason", "by hand",
            ],
        );
        r.unwrap();
        assert_eq!(s.incumbent("demo"), "deep");
        let (out, _, r) = run_test_command(
            &s,
            "evolve",
            &["variant", "retire", "deep", "--reason", "regressed"],
        );
        r.unwrap();
        assert!(out.contains("demo: incumbent back to base"), "{out}");
        assert_eq!(s.incumbent("demo"), "base");
        assert_eq!(s.variant("deep").unwrap().unwrap().state, "retired");
        let (_, _, r) = run_test_command(&s, "evolve", &["variant", "retire", "base"]);
        assert!(r.is_err());
    }
}
