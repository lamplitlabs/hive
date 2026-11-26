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
/// Matched on word boundaries against the ask's summary and body.
static HARD_STOP_WORDS: LazyLock<Vec<(&'static str, regex::Regex)>> = LazyLock::new(|| {
    [
        (
            "money",
            r"\b(spend money|spends money|(raise|increase|lift|the|a|monthly|spending|cloud|azure) budget|billing|billed|invoice\w*|payment\w*|pay for|purchas\w*|subscription\w*|credit card|costs? money|compute requests?|paid plan)\b|[$€£]\s?\d",
        ),
        (
            "a destructive or irreversible act",
            r"\b(irreversibl\w*|destructive|force[- ]push\w*|rm -rf|drop (table|database)|wipe\w*|purg(e|es|ed|ing)|deallocat\w*|delete (the |a |this |that )?(repo|repository|database|account|project|bucket|resource group|vm|machine|history|backup)s?|data migrations?|rewrite (the )?history|cannot be undone|can't be undone)\b",
        ),
        (
            "credentials",
            r"\b(credentials?|secrets?|passwords?|api[- ]keys?|(access|api|deploy|repository|repo|personal) tokens?|(the|a|your) token (that|to|which|for)|ssh keys?|signing keys?|private keys?)\b",
        ),
        (
            "publication",
            r"\b(publish\w*|publication|deploy\w*|go live|goes live|(to|in|into) production|public repo\w*|make (it |the \w+ )?public|app store|play store|release (v?\d|to )|tag v?\d)\b",
        ),
    ]
    .into_iter()
    .map(|(why, re)| (why, regex::Regex::new(re).expect("hard stop regex")))
    .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

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
