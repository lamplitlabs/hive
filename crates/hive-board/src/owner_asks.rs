//! Which asks still wait for the owner (TASK-129). Nothing waits on the
//! owner by default: a project's lead decides the routine asks addressed to
//! the owner - proposals, questions, findings, handoffs and suggestions -
//! whatever `owner.presence` says. Only an ask at a hard stop stays the
//! owner's: money, a destructive or irreversible act, credentials,
//! publication, a Tier 2 decision (an ADR or an owner-only path), a command
//! a proposal would authorize (ADR-021) and the evolution loop's stop
//! question (ADR-043). The hub shows those few on the Sessions strip
//! (`decider: owner` on its question rows, with `hard_stop` naming why).

use std::sync::LazyLock;

/// Each hard stop with the words that put an ask there, lowercased.
/// Matched on word boundaries against the ask's summary and body once the
/// path-like tokens are gone. The list is the net, not the rule: an ask
/// names its hard stop on a `Hard stop: <class>` line ([`EXPLICIT`]), and
/// the briefs tell agents to write one.
static HARD_STOP_WORDS: LazyLock<Vec<(&'static str, regex::Regex)>> = LazyLock::new(|| {
    [
        (
            "money",
            r"\b(spend money|spends money|(raise|increase|lift|the|a|monthly|spending|cloud|azure) budget|billing|billed|invoice\w*|payment\w*|pay for|purchas\w*|subscription\w*|credit card|costs? money|compute requests?|paid (plan|tier|account|seat|version|service)s?|buy|buys|buying|bought|rent|rents|rented|renting|rental|usd|eur|gbp|dollars?|euros?|per month|a month|monthly (fee|cost|price|bill)s?)\b|[$€£]\s?\d",
        ),
        (
            "a destructive or irreversible act",
            r"\b(irreversibl\w*|destructive|force[- ]push\w*|push (-f|--force)|rm -rf|drop (the |a |an |this |that )?(\w+ )?(tables?|databases?|columns?|schemas?)|wipe\w*|purg(e|es|ed|ing)|deallocat\w*|(delete|deletes|deleting|destroy|destroys|destroying) (the |a |an |this |that |these |those |all |every |old |stale |remote |unused |our |its )*([\w.-]+ )?(repos?|repository|repositories|databases?|accounts?|projects?|buckets?|resource groups?|vms?|machines?|history|backups?|branch|branches|tags?|releases?|tables?|users?|volumes?|disks?|data)\b|push (origin )?--delete|data migrations?|rewrite (the )?history|cannot be undone|can't be undone)\b",
        ),
        (
            "credentials",
            r"\b(credentials?|secrets?|passwords?|api[- ]keys?|(access|api|deploy|repository|repo|personal) tokens?|(the|a|your) token (that|to|which|for)|ssh keys?|signing keys?|private keys?|(github |personal access )?pats?|(rotate|rotates|rotating|revoke|revokes|revoking|regenerate|regenerating) (the |a |an |our |its |your )?(\w+ )?(tokens?|keys?|pats?|certificates?|certs?)|\w*_(api_key|secret|secret_key|access_key|password|auth_token|access_token|api_token)|(github|gh|npm|pypi|cargo|openai|anthropic|azure|aws|hf|huggingface)_token)\b",
        ),
        (
            "publication",
            r"\b(publish\w*|publication|deploy\w*|go live|goes live|go public|(to|in|into|on) (production|prod)|ship (it |this |that )?to (users|customers)|public repo\w*|make ((it|\w+ [\w-]+|\w+ [\w-]+ [\w-]+) )?public|app store|play store|(cut|ship|tag|publish) (the |a |our )?(v?\d+(\.\d+)*|new|next|first) release|release (v?\d|to )|tag v?\d|(send|sends|sending|sent) (an? |the |our |this |that )?(\w+ )?(e-?mails?|newsletters?|announcements?)|e-?mail (the|all|every|our) (users|customers|subscribers|list)|mailing list|press release|announce\w*)\b",
        ),
        (
            "a Tier 2 decision",
            r"\b(tier[- ]?2|owner[- ]only|project('s)? goal|hive goal set|(change|changes|set|reset|rewrite|replace|amend|edit|update) (the |this |our |its )?goal|(amend|amends|amending|change|changes|edit|edits|update|updates|rewrite|rewrites)\w* (to )?(the |its |our )?constitution|constitution(al)? (amendment|change)s?|relicens\w*|(change|changes|changing|switch|switches|switching|move|moves|moving) (the |our |its )?licen[cs]e|licen[cs]e (change|switch)\w*)\b",
        ),
    ]
    .into_iter()
    .map(|(why, re)| (why, regex::Regex::new(re).expect("hard stop regex")))
    .collect()
});

/// An explicit `Hard stop: <class>` line (or a summary that opens with
/// one): always honoured, whatever the other words say.
static EXPLICIT: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?im)^[ \t>*-]*(?:question:\s*)?hard[ -]stop:[ \t]*([^\n|]*)")
        .expect("explicit hard stop regex")
});

/// What an agent writes on a `Hard stop:` line to say there is none.
const NO_HARD_STOP: &[&str] = &["none", "no", "n/a", "na", "nothing", "not one"];

/// Money written as an amount: `$26/month`, `40 USD`, `€5`. Tested on the
/// text before the path-like tokens go, since `$26/month` is one of them.
static AMOUNT: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)[$€£]\s?\d|\b\d+([.,]\d+)?\s?(usd|eur|gbp|dollars?|euros?)\b|\b(usd|eur|gbp)\s?\d",
    )
    .expect("amount regex")
});

/// The line the runner opens a worker's `QUESTION:` post with. It says
/// why any worker asks, so it is no evidence that this ask is at a hard
/// stop: the lead reads the question itself.
pub const WORKER_QUESTION_PREAMBLE: &str =
    "The worker stopped because this needs a real decision or crosses a hard stop.";

static PATH_TOKEN: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\S*/\S*").expect("path token regex"));

/// Why the ask is the owner's alone; empty when it is a routine ask the
/// project's lead decides. `kind` is the post's kind (a question, a
/// suggestion, a finding, a handoff); `summary` and `body` its text.
pub fn hard_stop_ask(kind: &str, summary: &str, body: &str) -> String {
    if kind == "question" && summary == hive_store::EVOLUTION_STOP_SUMMARY {
        return "the evolution loop stopped (ADR-043)".into();
    }
    if kind == "question" && summary == hive_store::EVOLUTION_FLAT_SUMMARY {
        return "the evolution loop paused (flat fixtures)".into();
    }
    // Every explicit line is read: the first that names a class decides.
    // A line that says there is none (`Hard stop: none`) is no evidence
    // either way: its label goes before the words are read ("hard stop" on
    // it would otherwise count as naming one) and the rest of the line
    // stays, so `Hard stop: none - delete the old release tags?` is still
    // read for what it asks.
    let mut said_none = false;
    for c in EXPLICIT.captures_iter(&format!("{summary}\n{body}")) {
        // The class is the line's first words, before any explanation.
        let class = c[1]
            .split([';', ',', '.', '(', '\u{2014}'])
            .next()
            .unwrap_or_default()
            .split(" - ")
            .next()
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        if NO_HARD_STOP.contains(&class.as_str()) {
            said_none = true;
            continue;
        }
        return if class.is_empty() {
            "names a hard stop".into()
        } else {
            format!("names a hard stop: {class}")
        };
    }
    let (summary, body) = if said_none {
        (
            EXPLICIT.replace_all(summary, "$1").into_owned(),
            EXPLICIT.replace_all(body, "$1").into_owned(),
        )
    } else {
        (summary.to_string(), body.to_string())
    };
    let (summary, body) = (summary.as_str(), body.as_str());
    let what = crate::jobs::owner_only_proposal(summary, body);
    if !what.is_empty() {
        return what;
    }
    if crate::jobs::authorizes_command(body) {
        return "authorizes a command (ADR-021)".into();
    }
    // A path names a file, not an act: `deploy/azure/main.bicep` or
    // `scripts/publish.sh` in a routine handoff is not a publication.
    let body = body.replace(WORKER_QUESTION_PREAMBLE, "");
    if AMOUNT.is_match(&format!("{summary}\n{body}")) {
        return "is about money".into();
    }
    let text = PATH_TOKEN
        .replace_all(&format!("{summary}\n{body}").to_lowercase(), " ")
        .into_owned();
    if text.contains("hard stop") {
        return "names a hard stop".into();
    }
    HARD_STOP_WORDS
        .iter()
        .find(|(_, re)| re.is_match(&text))
        .map(|(why, _)| format!("is about {why}"))
        .unwrap_or_default()
}

/// Why an ask is the owner's alone, as the hub marks every row of the
/// owner's (`hard_stop`) and the lead's `decide` refuses it: a proposal a
/// compute request names is money whatever it says; any other ask is
/// [`hard_stop_ask`]. Empty when the project's lead decides it.
pub fn owner_only_ask(
    conn: &rusqlite::Connection,
    post_id: &str,
    kind: &str,
    summary: &str,
    body: &str,
) -> String {
    use rusqlite::OptionalExtension;
    let compute = conn
        .query_row(
            "SELECT 1 FROM compute_requests WHERE proposal_post = ? LIMIT 1",
            [post_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .ok()
        .flatten()
        .is_some();
    if compute {
        return "is about money: a compute request".into();
    }
    hard_stop_ask(kind, summary, body)
}

/// Whether `post_id` is the security watcher's finding about a major
/// dependency upgrade: the one test the hub and the runner share, read from
/// the `security.major` event the watcher writes with the post it told,
/// never from the finding's words (PR #278 review).
pub fn is_security_major_finding(conn: &rusqlite::Connection, post_id: &str) -> bool {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT 1 FROM events WHERE action = 'security.major' AND json_extract(detail, '$.post') = ? LIMIT 1",
        [post_id],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .ok()
    .flatten()
    .is_some()
}

/// The posts that are the watcher's major-upgrade findings, of `project`
/// or of every project ([`is_security_major_finding`] for every post at
/// once).
pub fn security_major_findings(
    conn: &rusqlite::Connection,
    project: Option<&str>,
) -> rusqlite::Result<std::collections::BTreeSet<String>> {
    let mut stmt = conn.prepare(
        "SELECT json_extract(detail, '$.post') FROM events WHERE action = 'security.major' AND (?1 IS NULL OR subject = ?1)",
    )?;
    let rows = stmt.query_map([project], |r| r.get::<_, Option<String>>(0))?;
    let mut out = std::collections::BTreeSet::new();
    for r in rows {
        if let Some(id) = r? {
            out.insert(id);
        }
    }
    Ok(out)
}

/// The answers the security watcher writes on a major-upgrade finding, for
/// one posted before it wrote them (an older finding has no `Recommended:`
/// line): with `dependencies.major_updates = ask` such a finding is the
/// owner's, and the owner answers with one pick.
pub const SECURITY_MAJOR_ANSWERS: &str = "Yes: Upgrade it\nNo: Leave it to me\nRecommended: Yes - the job lands only when the project's checks pass on the new major\n";

/// `body` with the watcher's answers to a major upgrade appended when it
/// names none ready to pick ([`crate::ask_options`]).
pub fn with_security_major_answers(body: &str) -> String {
    if crate::ask_options::ask_options(body).ready() {
        return body.to_string();
    }
    format!("{}\n\n{SECURITY_MAJOR_ANSWERS}", body.trim_end())
}

/// The answers of the improvement loop's flat-fixture pause
/// (hive-evolve flat_gate): the owner's call, like the loop's stop.
pub const FLAT_PAUSE_ANSWERS: &str = "Yes: Run more cycles - the loop plans again with the fixtures as they are and asks again after as many tied cycles\nNo: Keep it paused - the loop waits until a fixture whose score moves is enabled\nRecommended: No - more tied cycles cost worker runs and teach nothing until a fixture tells variants apart\n";

/// The answers a question of the daemon offers when its body names none
/// (TASK-129: the owner answers every question with one pick). The daemon
/// asks about this machine - a login, a hand, a held drain - which a person
/// fixes on the machine; a lead seat or a toolkit entry carries a marker
/// line, and a yes acts on it.
pub fn daemon_default_answers(body: &str) -> &'static str {
    static MARKER: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?m)^[ \t]*(lead-role|toolkit):[ \t]*\S").expect("daemon marker regex")
    });
    match MARKER.captures(body).map(|c| c[1].to_string()).as_deref() {
        Some("lead-role") => {
            "Yes: Seat the lead - the daemon seats the lead role the marker line names; it fixes what it can and never decides for you\n\
No: Not now - no lead is seated; the daemon may suggest it again later\n\
Recommended: Yes - nobody owns this concern now, and a lead can be removed again with one command\n"
        }
        Some("toolkit") => {
            "Yes: Add it - the daemon adds the tool or skill the marker line names (a server that needs a secret waits until you put it in place)\n\
No: Not now - nothing is added; the daemon may suggest it again later\n\
Recommended: Yes - the project shows the signal it answers, and it can be removed again with one command\n"
        }
        _ => {
            "Yes: Hand it to an agent - a job looks into what it names and reports back\n\
No: I handle it - the question closes; the daemon asks again if it still holds\n\
Recommended: No - it is about this machine; do what the post says on it\n"
        }
    }
}

/// `body` with the answers the owner picks from when the asker wrote
/// none ready to pick: the watcher's for a major upgrade (`major`), the
/// loop's for its flat-fixture pause, the daemon's for a question the
/// daemon asked (`agent`), also one posted before it wrote answers. Any
/// other body comes back as it is.
pub fn with_default_owner_answers(
    kind: &str,
    agent: &str,
    summary: &str,
    body: &str,
    major: bool,
) -> String {
    if major {
        return with_security_major_answers(body);
    }
    if crate::ask_options::ask_options(body).ready() {
        return body.to_string();
    }
    // The loop's stop keeps the console's own pair (keep paused, carry
    // on). The daemon asks it, and the daemon's "Hand it to an agent" would
    // record a yes: the loop would run on past its stop with no job started.
    if kind == "question" && summary == hive_store::EVOLUTION_STOP_SUMMARY {
        return body.to_string();
    }
    if kind == "question" && summary == hive_store::EVOLUTION_FLAT_SUMMARY {
        return format!("{}\n\n{FLAT_PAUSE_ANSWERS}", body.trim_end());
    }
    if kind == "question" && agent == "daemon" {
        return format!("{}\n\n{}", body.trim_end(), daemon_default_answers(body));
    }
    body.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_major_upgrade_is_named_by_the_watchers_event_and_gets_its_answers() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE events (seq INTEGER PRIMARY KEY, action TEXT, subject TEXT, detail TEXT);
             INSERT INTO events (action, subject, detail) VALUES ('security.major', 'one', '{\"post\":\"P-major\"}');
             INSERT INTO events (action, subject, detail) VALUES ('security.unfixable', 'one', '{\"post\":\"P-unfixable\"}');",
        )
        .unwrap();
        assert!(is_security_major_finding(&conn, "P-major"));
        assert!(!is_security_major_finding(&conn, "P-unfixable"));
        assert!(!is_security_major_finding(&conn, "P-words-only"));
        let all = security_major_findings(&conn, None).unwrap();
        assert_eq!(all.into_iter().collect::<Vec<_>>(), vec!["P-major"]);
        assert!(
            security_major_findings(&conn, Some("two"))
                .unwrap()
                .is_empty()
        );
        // An older finding without answers gets the watcher's; one with
        // answers keeps its own.
        let o = crate::ask_options::ask_options(&with_security_major_answers("Dependabot #7."));
        assert!(o.ready());
        assert_eq!(o.options[0].label, "Upgrade it");
        let own =
            "Dependabot #20.\n\nYes: Upgrade it\nNo: Leave it to me\nRecommended: No - not yet";
        assert_eq!(with_security_major_answers(own), own);
    }

    /// The classes the PR #278 review found the word list missed, one line
    /// each, and the explicit `Hard stop:` line that is always honoured.
    #[test]
    fn the_hard_stop_net_catches_the_classes_and_the_explicit_line_wins() {
        for (text, want) in [
            ("Move to the $26/month plan?", "is about money"),
            ("It costs 40 USD a month.", "is about money"),
            ("Buy a domain for the site?", "is about money"),
            ("Rent a GPU box for the evals?", "is about money"),
            ("Switch to the paid tier of the API?", "is about money"),
            ("Rotate the GitHub PAT?", "is about credentials"),
            ("Put OPENAI_API_KEY in the recipe?", "is about credentials"),
            ("Ship to prod tonight?", "is about publication"),
            ("Make this repo public?", "is about publication"),
            ("Cut the 1.0 release?", "is about publication"),
            ("Send an e-mail to the beta users?", "is about publication"),
            (
                "Delete the old branches?",
                "is about a destructive or irreversible act",
            ),
            (
                "Run git push --force on main?",
                "is about a destructive or irreversible act",
            ),
            (
                "Drop the events table?",
                "is about a destructive or irreversible act",
            ),
            (
                "Delete the v0 tags and releases?",
                "is about a destructive or irreversible act",
            ),
            ("Change the project goal?", "is about a Tier 2 decision"),
            (
                "Amend the constitution for the new loop?",
                "is about a Tier 2 decision",
            ),
            (
                "Tier 2: move the store to Postgres?",
                "is about a Tier 2 decision",
            ),
            (
                "Owner-only: rename the product?",
                "is about a Tier 2 decision",
            ),
            (
                "Switch the licence to Apache-2.0?",
                "is about a Tier 2 decision",
            ),
        ] {
            assert_eq!(hard_stop_ask("question", text, ""), want, "{text}");
        }
        // Routine words stay the lead's.
        for text in [
            "Delete the unused helper?",
            "Fix the production build warning?",
            "Raise the test timeout to 30 s?",
            "Bump the patch release notes wording?",
        ] {
            assert_eq!(hard_stop_ask("question", text, ""), "", "{text}");
        }
        // The explicit line is the rule; the words are only the net.
        assert_eq!(
            hard_stop_ask(
                "question",
                "Go ahead with it?",
                "Why: see the thread.\nHard stop: money\n"
            ),
            "names a hard stop: money"
        );
        assert_eq!(
            hard_stop_ask("question", "Hard stop: credentials - the deploy key", ""),
            "names a hard stop: credentials"
        );
        // `Hard stop: none` says there is none: the words decide, as without
        // the line (second review of pull request 278).
        assert_eq!(
            hard_stop_ask(
                "handoff",
                "Tidy the helper",
                "Hard stop: none\nRemove the unused helper function.\n"
            ),
            ""
        );
        assert_eq!(
            hard_stop_ask(
                "question",
                "Clean up?",
                "Hard stop: n/a\nThis force-pushes main.\n"
            ),
            "is about a destructive or irreversible act"
        );
        // A none line is still read for what follows on it: the words, and
        // the paths and names only the owner decides on.
        assert_eq!(
            hard_stop_ask(
                "question",
                "Go on?",
                "Hard stop: none - Delete the old release tags?\n"
            ),
            "is about a destructive or irreversible act"
        );
        assert_eq!(
            hard_stop_ask(
                "question",
                "Go on?",
                "Hard stop: none, but it deletes the release tags\n"
            ),
            "is about a destructive or irreversible act"
        );
        assert!(
            !hard_stop_ask(
                "suggestion",
                "Tidy the workflow",
                "Hard stop: none - edits .github/workflows/ci.yml\n"
            )
            .is_empty()
        );
        assert!(!hard_stop_ask("suggestion", "Hard stop: none - accept ADR-081", "").is_empty());
        // A line that says none does not hide a line that names one.
        assert_eq!(
            hard_stop_ask(
                "question",
                "Go on?",
                "Hard stop: none\nLater in the thread:\nHard stop: money\n"
            ),
            "names a hard stop: money"
        );
        // Two phrases the net caught before it was widened, and lost.
        assert_eq!(
            hard_stop_ask("question", "Remove them?", "Delete the repos we left."),
            "is about a destructive or irreversible act"
        );
        assert_eq!(
            hard_stop_ask("question", "Open it up?", "Make public the roadmap."),
            "is about publication"
        );
        // The loop's flat-fixture pause is the owner's like its stop.
        assert_eq!(
            hard_stop_ask("question", hive_store::EVOLUTION_FLAT_SUMMARY, ""),
            "the evolution loop paused (flat fixtures)"
        );
        let answered = with_default_owner_answers(
            "question",
            "evolve",
            hive_store::EVOLUTION_FLAT_SUMMARY,
            "paused",
            false,
        );
        assert!(
            crate::ask_options::ask_options(&answered).ready(),
            "{answered}"
        );
    }

    /// A daemon question without answers, also one from before it wrote
    /// them, is shown with the daemon's own; a lead seat and a toolkit
    /// entry say what a yes does there and recommend it.
    #[test]
    fn a_daemon_question_gets_the_daemons_answers() {
        let login =
            with_default_owner_answers("question", "daemon", "Login wanted", "Sign in.", false);
        let o = crate::ask_options::ask_options(&login);
        assert!(o.ready(), "{login}");
        assert!(login.contains("Hand it to an agent"), "{login}");
        let seat = with_default_owner_answers(
            "question",
            "daemon",
            "Seat a security lead?",
            "Nobody owns it.\n\nlead-role: proj security\n",
            false,
        );
        assert!(seat.contains("Yes: Seat the lead"), "{seat}");
        assert!(seat.contains("Recommended: Yes"), "{seat}");
        let tool = with_default_owner_answers(
            "question",
            "daemon",
            "Connect GitHub?",
            "toolkit: proj tool github\n",
            false,
        );
        assert!(tool.contains("Yes: Add it"), "{tool}");
        // An agent's bare question is left as it is: it goes to its lead.
        assert_eq!(
            with_default_owner_answers("question", "proj-lead", "Go on?", "Bare.", false),
            "Bare."
        );
        // The loop's stop is the daemon's question too, and keeps the
        // console's own pair: "Hand it to an agent" would record a yes and
        // the loop would run on (second review of pull request 278).
        let stop = with_default_owner_answers(
            "question",
            "daemon",
            hive_store::EVOLUTION_STOP_SUMMARY,
            "The loop stopped: a constraint was violated.",
            false,
        );
        assert_eq!(stop, "The loop stopped: a constraint was violated.");
        assert!(!crate::ask_options::ask_options(&stop).ready());
    }

    #[test]
    fn routine_asks_are_the_leads_and_hard_stops_the_owners() {
        for (kind, summary, body, want) in [
            // Routine: the lead decides these whatever owner.presence says.
            (
                "question",
                "Retry the flaky landing?",
                "The worker stopped because this needs a real decision or crosses a hard stop.\n\nRetry the flaky landing?\n",
                "",
            ),
            (
                "question",
                "Ship the rename?",
                "Yes: Rename it\nNo: Keep it",
                "",
            ),
            (
                "handoff",
                "Run the queued cleanup?",
                "Tier 1 work in crates/hive-runner.",
                "",
            ),
            (
                "suggestion",
                "Retire the obsolete stress-test suggestion",
                "It no longer applies at HEAD.",
                "",
            ),
            (
                "finding",
                "Dependabot #30 (low): no patched version of hono exists yet",
                "Yes: Look for a way around it\nNo: Keep watching",
                "",
            ),
            // Hard stops: these wait for the owner on the Sessions strip.
            (
                "question",
                hive_store::EVOLUTION_STOP_SUMMARY,
                "",
                "the evolution loop stopped (ADR-043)",
            ),
            (
                "question",
                "Raise the cap?",
                "hive settings set compute.max_vms 3",
                "changes compute.max_vms",
            ),
            (
                "question",
                "Merge pull request #224?",
                "Why it waits for you:\n- touches .github/workflows/checks.yml",
                "points at .github/",
            ),
            ("suggestion", "Accept ADR-071?", "", "names ADR-071"),
            (
                "suggestion",
                "Pair the VM",
                "pair: vm1\n",
                "authorizes a command (ADR-021)",
            ),
            (
                "question",
                "Keep the VM size or move to a 4-core one?",
                "A bigger VM costs money every hour.",
                "is about money",
            ),
            (
                "finding",
                "For the owner: the Homebrew tap is live",
                "The token that lets a release write its formulae is yours to place.",
                "is about credentials",
            ),
            (
                "question",
                "Put the new wording on hivefleet.dev now?",
                "It is a publication, so it is the owner's.",
                "is about publication",
            ),
            (
                "question",
                "Clean up the old runs?",
                "This force-pushes main and cannot be undone.",
                "is about a destructive or irreversible act",
            ),
            (
                "question",
                "Go on?",
                "This crosses a hard stop.",
                "names a hard stop",
            ),
            // A path is not an act, and a time budget is not money.
            (
                "handoff",
                "Finish the bicep lint",
                "Edit deploy/azure/main.bicep and scripts/publish.sh; the token budget of the brief is fine.",
                "",
            ),
        ] {
            assert_eq!(hard_stop_ask(kind, summary, body), want, "{summary}");
        }
    }
}
