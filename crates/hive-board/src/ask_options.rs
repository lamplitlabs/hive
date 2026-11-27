//! The answers a question for the owner carries (TASK-129, owner's words of
//! 2026-11-26: "always give option no text.. and add recommended one as
//! well.. always").
//!
//! An ask names its answers in its body, one line each, and marks one of
//! them recommended with a line of why:
//!
//! ```text
//! Yes: Upgrade it - queues the major upgrade behind the project's checks
//! No: Leave it to me - nothing is queued; the alert stays open
//! Recommended: Yes - the advisory is real and the checks guard the upgrade
//! ```
//!
//! or, for more than two answers:
//!
//! ```text
//! Option: Stripe test mode - the fixtures call Stripe's sandbox
//! Option: A local fake - no network; the fixtures replay recorded answers
//! Recommended: A local fake - the checks stay offline and deterministic
//! ```
//!
//! The text after ` - ` (or ` — `) is what picking the answer does. The
//! owner answers with one pick; a question with no answers, or none
//! recommended, never reaches the owner: it goes back to the project's lead,
//! which writes them or decides it.

use serde::Serialize;
use std::sync::LazyLock;

/// The longest label an answer carries: it is a button.
pub const MAX_OPTION_LABEL: usize = 60;
/// The most answers one question offers; more is a form, not a choice.
pub const MAX_OPTIONS: usize = 6;

/// One answer the owner can pick.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AskOption {
    /// What the button says.
    pub label: String,
    /// What picking it does, in one line; empty when the post names none.
    pub effect: String,
    /// `yes` or `no` for the two answers of a yes-or-no question (`Yes:` /
    /// `No:` lines); empty for an `Option:` line, which records the owner's
    /// yes with the label as the answer.
    pub answer: String,
    /// The one the asker recommends: shown first and as the default.
    pub recommended: bool,
}

/// The answers of one ask, recommended first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AskOptions {
    pub options: Vec<AskOption>,
    /// Why the recommended answer is the one; empty when none is marked.
    pub why: String,
}

impl AskOptions {
    /// Whether the ask may reach the owner: two or more answers, exactly one
    /// of them recommended.
    pub fn ready(&self) -> bool {
        self.options.len() >= 2 && self.options.iter().filter(|o| o.recommended).count() == 1
    }

    /// The recommended answer, if one is marked.
    pub fn recommended(&self) -> Option<&AskOption> {
        self.options.iter().find(|o| o.recommended)
    }

    /// The answer whose label is `label` (case and surrounding space
    /// ignored), or the yes / no answer when `label` is `yes` or `no`.
    pub fn find(&self, label: &str) -> Option<&AskOption> {
        let want = label.trim();
        self.options
            .iter()
            .find(|o| o.label.eq_ignore_ascii_case(want))
            .or_else(|| {
                let lower = want.to_ascii_lowercase();
                self.options.iter().find(|o| o.answer == lower)
            })
    }

    /// What is missing before the ask may reach the owner, in a few words;
    /// empty when it is ready.
    pub fn missing(&self) -> String {
        if self.options.len() < 2 {
            return "no answers to pick (two or more `Option:` lines, or `Yes:` and `No:`)".into();
        }
        match self.options.iter().filter(|o| o.recommended).count() {
            0 => "no recommended answer (a `Recommended: <label> - <why>` line)".into(),
            1 => String::new(),
            _ => "more than one recommended answer".into(),
        }
    }
}

static OPTION_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"(?mi)^[ \t]*[-*]?[ \t]*\**(option|yes|no|recommended)\**[ \t]*:[ \t]*(.+?)[ \t]*$",
    )
    .expect("ask option regex")
});

/// Splits `label - effect` (also ` — ` and ` – `) at its first separator.
fn split_label(text: &str) -> (String, String) {
    let text = text.trim().trim_matches('*').trim();
    let cut = [" - ", " — ", " – "]
        .iter()
        .filter_map(|sep| text.find(sep).map(|i| (i, sep.len())))
        .min_by_key(|(i, _)| *i);
    let (label, effect) = match cut {
        Some((i, n)) => (&text[..i], &text[i + n..]),
        None => (text, ""),
    };
    let label = label
        .trim()
        .trim_matches(|c| c == '`' || c == '"')
        .trim_end_matches(['.', '!'])
        .trim();
    (label.to_string(), effect.trim().to_string())
}

/// Whether `line` opens or closes a fenced code block (` ``` ` or `~~~`,
/// up to three spaces in, as Markdown reads it).
fn is_fence(line: &str) -> bool {
    let t = line.trim_start_matches(' ');
    line.len() - t.len() <= 3 && (t.starts_with("```") || t.starts_with("~~~"))
}

/// The answer lines of `body` as `(word, text)` pairs, from its last block
/// of such lines. Lines inside fenced code blocks are never answers (a diff
/// line `-no: cache` is not a No button), and a daemon-built ask ends with
/// its answers after free text, so a `Recommended:` or `Yes:` line earlier
/// in a quoted note does not hide or replace them (TASK-149 item 12). A
/// block is a run of answer lines that only blank lines may separate; any
/// other line ends it.
fn answer_lines(body: &str) -> Vec<(String, String)> {
    let mut blocks: Vec<Vec<(String, String)>> = Vec::new();
    let mut current: Vec<(String, String)> = Vec::new();
    let mut fence = false;
    for line in body.lines() {
        if is_fence(line) {
            fence = !fence;
            if !current.is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
            continue;
        }
        if fence {
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        match OPTION_RE.captures(line) {
            Some(c) => current.push((c[1].to_ascii_lowercase(), c[2].to_string())),
            None => {
                if !current.is_empty() {
                    blocks.push(std::mem::take(&mut current));
                }
            }
        }
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    blocks.pop().unwrap_or_default()
}

/// Reads the answers an ask names in its body (see the module docs), from
/// its last block of answer lines outside code fences ([`answer_lines`]). A
/// `Yes:` or `No:` line whose label is a sentence (longer than a button) is
/// the explanation under it, not an answer, as the hub has always read it.
pub fn ask_options(body: &str) -> AskOptions {
    let mut out = AskOptions::default();
    let mut recommended: Option<(String, String)> = None;
    for (word, text) in answer_lines(body) {
        let (label, effect) = split_label(&text);
        if label.is_empty() {
            continue;
        }
        if word == "recommended" {
            if recommended.is_none() {
                recommended = Some((label, effect));
            }
            continue;
        }
        if label.chars().count() > MAX_OPTION_LABEL {
            continue;
        }
        let answer = if word == "option" {
            String::new()
        } else {
            word
        };
        if !answer.is_empty() && out.options.iter().any(|o| o.answer == answer) {
            continue;
        }
        if out
            .options
            .iter()
            .any(|o| o.label.eq_ignore_ascii_case(&label))
        {
            continue;
        }
        if out.options.len() >= MAX_OPTIONS {
            continue;
        }
        out.options.push(AskOption {
            label,
            effect,
            answer,
            recommended: false,
        });
    }
    if let Some((label, why)) = recommended {
        let pick = out.find(&label).map(|o| o.label.clone());
        if let Some(pick) = pick {
            for o in &mut out.options {
                o.recommended = o.label == pick;
            }
            out.why = why;
            // The recommended answer comes first.
            out.options.sort_by_key(|o| !o.recommended);
        }
    }
    out
}

/// The lines a daemon-built ask ends with: its two answers and the
/// recommended one, in the shape [`ask_options`] reads.
pub fn yes_no_lines(yes: (&str, &str), no: (&str, &str), recommend_yes: bool, why: &str) -> String {
    format!(
        "Yes: {} - {}\nNo: {} - {}\nRecommended: {} - {}\n",
        yes.0,
        yes.1,
        no.0,
        no.1,
        if recommend_yes { "Yes" } else { "No" },
        why
    )
}

/// `Option:` lines for the given answers, the one at `recommended` marked
/// with `why` - the shape [`ask_options`] reads back.
pub fn option_lines(answers: &[&str], recommended: usize, why: &str) -> String {
    let mut out = String::new();
    for a in answers {
        out.push_str(&format!("Option: {}\n", a.trim()));
    }
    if let Some(r) = answers.get(recommended) {
        let (label, _) = split_label(r);
        out.push_str(&format!("Recommended: {label} - {}\n", why.trim()));
    }
    out
}

/// Splits a worker's `QUESTION:` report line into the question and the
/// answers it offers: `QUESTION: <question> | <recommended answer - what it
/// does> | <alternative - what it does>`. The first answer is the worker's
/// recommendation. The second value holds the body lines the hub reads
/// ([`option_lines`]); it is empty when the line names fewer than two
/// answers, and such an ask goes to the lead, not the owner.
pub fn split_question_line(line: &str) -> (String, String) {
    let mut parts = line.split(" | ").map(str::trim).filter(|p| !p.is_empty());
    let question = parts.next().unwrap_or_default().to_string();
    let answers: Vec<&str> = parts.collect();
    if answers.len() < 2 {
        return (line.trim().to_string(), String::new());
    }
    (
        question,
        option_lines(&answers, 0, "the worker's recommendation"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_question_line_names_its_answers() {
        let (q, lines) = split_question_line(
            "Rotate the key now? | Rotate it - a job rotates it tonight | Keep it - nothing changes",
        );
        assert_eq!(q, "Rotate the key now?");
        let o = ask_options(&lines);
        assert!(o.ready(), "{lines}");
        assert_eq!(o.options[0].label, "Rotate it");
        assert!(o.options[0].recommended);
        assert_eq!(o.options[1].effect, "nothing changes");
        let (q, lines) = split_question_line("Rotate the key now?");
        assert_eq!((q.as_str(), lines.as_str()), ("Rotate the key now?", ""));
    }

    #[test]
    fn yes_no_with_recommendation_is_ready_and_recommended_first() {
        let o = ask_options(
            "Dependabot #20.\n\nYes: Upgrade it - queues the upgrade\nNo: Leave it to me - nothing is queued\nRecommended: No - the major breaks the API\n",
        );
        assert!(o.ready(), "{o:?}");
        assert_eq!(o.options[0].label, "Leave it to me");
        assert_eq!(o.options[0].answer, "no");
        assert!(o.options[0].recommended);
        assert_eq!(o.options[1].effect, "queues the upgrade");
        assert_eq!(o.why, "the major breaks the API");
        assert_eq!(o.find("yes").unwrap().label, "Upgrade it");
        assert_eq!(o.find("leave it to me").unwrap().answer, "no");
    }

    #[test]
    fn options_name_the_recommended_one_by_label() {
        let o = ask_options(
            "Which sandbox?\n\nOption: Stripe test mode — the fixtures call Stripe\nOption: A local fake — no network\n- **Option:** Skip it - nothing changes\nRecommended: a local fake - offline and deterministic\n",
        );
        assert!(o.ready(), "{o:?}");
        assert_eq!(o.options.len(), 3);
        assert_eq!(o.options[0].label, "A local fake");
        assert_eq!(o.recommended().unwrap().effect, "no network");
        assert_eq!(o.options[2].label, "Skip it");
        assert!(o.options.iter().all(|x| x.answer.is_empty()));
    }

    #[test]
    fn a_question_without_answers_or_recommendation_is_not_ready() {
        let none = ask_options("Which payment sandbox should the fixtures hit?");
        assert!(!none.ready());
        assert!(none.missing().contains("no answers"));
        let one = ask_options("Option: Only one - x\nRecommended: Only one - y");
        assert!(!one.ready());
        let unmarked = ask_options("Yes: Upgrade it\nNo: Leave it to me\n");
        assert!(!unmarked.ready());
        assert!(unmarked.missing().contains("no recommended"));
        // A recommendation naming no answer marks nothing.
        let stray = ask_options("Option: A\nOption: B\nRecommended: C - why");
        assert!(!stray.ready());
        assert_eq!(stray.why, "");
    }

    #[test]
    fn a_sentence_after_yes_is_an_explanation_not_a_button() {
        let o = ask_options(
            "Yes: this queues a job that looks for a replacement within the project's checks and reports\nNo: Keep watching\n",
        );
        assert_eq!(o.options.len(), 1);
        assert_eq!(o.options[0].label, "Keep watching");
    }

    #[test]
    fn answers_come_from_the_last_block_outside_code_fences() {
        // A review note quoted above the answers carries its own
        // `Recommended:` line, and the diff carries `-no:` and `+yes:` lines.
        let body = format!(
            "Pull request #7\n\nThe review:\n\nRecommended: squash the fixups first - they read as noise\nYes: it builds\n\n```diff\n-no: cache\n+yes: cache\n Recommended: No - stale\n```\n\n~~~\nOption: Inside a tilde fence\n~~~\n\n{}",
            yes_no_lines(
                ("Merge it", "it lands"),
                ("Not yet", "it stays open"),
                false,
                "read the reasons first"
            ),
        );
        let o = ask_options(&body);
        assert!(o.ready(), "{o:?}");
        assert_eq!(o.options.len(), 2, "{o:?}");
        assert_eq!(o.options[0].label, "Not yet");
        assert!(o.options[0].recommended);
        assert_eq!(o.options[1].label, "Merge it");
        assert_eq!(o.why, "read the reasons first");
        // The answers may sit apart by blank lines and still be one block.
        let spaced = ask_options("Why?\n\nOption: A - x\n\nOption: B - y\n\nRecommended: B - z\n");
        assert!(spaced.ready(), "{spaced:?}");
        assert_eq!(spaced.options[0].label, "B");
        // Answers only inside a fence are no answers at all.
        let fenced = ask_options("```\nYes: Go - a\nNo: Stop - b\nRecommended: Yes - c\n```\n");
        assert!(fenced.options.is_empty(), "{fenced:?}");
        // An unclosed fence hides everything after it.
        assert!(
            ask_options("```\nYes: Go\nNo: Stop\nRecommended: Yes - c")
                .options
                .is_empty()
        );
    }

    #[test]
    fn daemon_lines_round_trip() {
        let text = yes_no_lines(
            ("Yes, carry on", "the loop resumes"),
            ("No, keep it paused", "the loop waits"),
            false,
            "review the gate first",
        );
        let o = ask_options(&text);
        assert!(o.ready());
        assert_eq!(o.options[0].label, "No, keep it paused");
        assert_eq!(o.options[0].answer, "no");
    }
}
