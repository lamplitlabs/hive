//! Conversation titles that follow the conversation. A conversation is
//! born with the first line of its first message as its title; as the
//! talk moves on that line stops describing it. After each reply the
//! agent asks the model for a short name over the recent exchange and
//! writes it, unless the owner named the conversation by hand (then it
//! is theirs and stays).
//!
//! The call is small (one line out), has no tools and never affects the
//! turn: a failed or empty answer leaves the title as it was.

use pulse_provider::{Message, Request, Role, TITLE_INITIATOR};

/// The title stays within this many characters, like the one a new
/// conversation gets from its first message.
pub const TITLE_CHARS: usize = 64;

/// The model may answer with up to this many tokens; a title needs far
/// fewer, the margin covers a model that explains itself first.
pub const TITLE_TOKENS: i32 = 60;

/// How much of each message goes into the request, from its start.
const MESSAGE_CHARS: usize = 600;

/// How many of the most recent messages the request shows.
const RECENT_MESSAGES: usize = 12;

const TITLE_SYSTEM: &str = "You name conversations between a person and a coding agent. Read the recent exchange and answer with one short title, 3 to 8 words, that says what the conversation is about now, in the words the person used. Plain text: no quotes, no trailing period, no markdown, no preamble, nothing but the title.";

/// One message of the exchange as the request shows it.
pub struct Line<'a> {
    /// Who wrote it: `owner` or `agent`.
    pub who: &'a str,
    pub text: &'a str,
}

/// The request that asks `model` for a title of the exchange `lines`
/// (oldest first; only the last [`RECENT_MESSAGES`] are shown).
/// `current` is the title the conversation has now, so the model keeps it
/// when nothing changed.
pub fn title_request(model: &str, current: &str, lines: &[Line<'_>]) -> Request {
    let skip = lines.len().saturating_sub(RECENT_MESSAGES);
    let mut exchange = String::new();
    for line in &lines[skip..] {
        let text = line.text.trim();
        if text.is_empty() {
            continue;
        }
        exchange.push_str(line.who);
        exchange.push_str(": ");
        exchange.push_str(&head(text, MESSAGE_CHARS));
        exchange.push_str("\n\n");
    }
    let user = format!(
        "The conversation's current title: {}\n\nThe recent exchange, oldest first:\n\n{}Answer with the title only.",
        current.trim(),
        exchange
    );
    Request {
        model: model.to_string(),
        messages: vec![
            Message {
                role: Role::System,
                content: TITLE_SYSTEM.to_string(),
                ..Message::new(Role::System)
            },
            Message {
                role: Role::User,
                content: user,
                ..Message::new(Role::User)
            },
        ],
        tools: Vec::new(),
        max_tokens: TITLE_TOKENS,
        temperature: None,
        initiator: TITLE_INITIATOR.to_string(),
        reasoning: String::new(),
    }
}

/// The title in a model's answer: its first non-empty line (or the one
/// after a `Here is the title:` preamble), stripped of quotes, list marks,
/// a `Title:` label and a trailing period, clipped to [`TITLE_CHARS`].
/// None when nothing usable is left.
pub fn title_from_answer(answer: &str) -> Option<String> {
    let mut lines = answer.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next()?;
    let line = match lines.next() {
        Some(next) if first.ends_with(':') => next,
        _ => first,
    };
    let line = line.trim_start_matches(['#', '-', '*', '>']).trim();
    let line = match line.split_once(':') {
        Some((label, rest)) if label.trim().eq_ignore_ascii_case("title") => rest,
        _ => line,
    };
    let line = line
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | '“' | '”' | '‘' | '’'))
        .trim()
        .trim_end_matches('.')
        .trim();
    if line.is_empty() || line.chars().count() > TITLE_CHARS {
        return None;
    }
    Some(line.to_string())
}

/// The start of `s`, at most `limit` characters, with a mark when cut.
fn head(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    let cut: String = s.chars().take(limit).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_shows_the_recent_exchange_and_the_current_title() {
        let many: Vec<String> = (0..20).map(|i| format!("message {i}")).collect();
        let lines: Vec<Line<'_>> = many
            .iter()
            .enumerate()
            .map(|(i, text)| Line {
                who: if i % 2 == 0 { "owner" } else { "agent" },
                text,
            })
            .collect();
        let req = title_request("fake-1", "Old name", &lines);
        assert_eq!(req.model, "fake-1");
        assert!(req.tools.is_empty(), "a title call has no tools");
        assert_eq!(req.max_tokens, TITLE_TOKENS);
        assert_eq!(req.initiator, TITLE_INITIATOR);
        assert_eq!(req.messages.len(), 2);
        let user = &req.messages[1].content;
        assert!(user.contains("current title: Old name"), "{user}");
        assert!(user.contains("owner: message 8\n"), "{user}");
        assert!(user.contains("agent: message 19\n"), "{user}");
        assert!(
            !user.contains("message 7\n"),
            "older messages are left out: {user}"
        );
    }

    #[test]
    fn long_messages_are_cut_and_blank_ones_skipped() {
        let long = "x".repeat(2_000);
        let lines = [
            Line {
                who: "owner",
                text: "   ",
            },
            Line {
                who: "agent",
                text: &long,
            },
        ];
        let req = title_request("m", "t", &lines);
        let user = &req.messages[1].content;
        assert!(!user.contains("owner:"), "{user}");
        assert!(user.contains(&format!("agent: {}…", "x".repeat(MESSAGE_CHARS))));
        assert!(user.len() < 1_000, "{}", user.len());
    }

    #[test]
    fn the_answer_is_cleaned_to_one_title() {
        assert_eq!(
            title_from_answer("\n\"Fix the login redirect.\"\n").as_deref(),
            Some("Fix the login redirect")
        );
        assert_eq!(
            title_from_answer("Title: Hub retitles conversations").as_deref(),
            Some("Hub retitles conversations")
        );
        assert_eq!(
            title_from_answer("# **Bold** name\nsecond line").as_deref(),
            Some("**Bold** name")
        );
        assert_eq!(
            title_from_answer("Sure! Here is a title:\nRetry the flaky test").as_deref(),
            Some("Retry the flaky test")
        );
        assert_eq!(
            title_from_answer("Only a label:").as_deref(),
            Some("Only a label:")
        );
        assert_eq!(title_from_answer("   \n\n"), None);
        assert_eq!(title_from_answer("\"\""), None);
        assert_eq!(title_from_answer(&"w".repeat(TITLE_CHARS + 1)), None);
        assert_eq!(
            title_from_answer(&"w".repeat(TITLE_CHARS)).as_deref(),
            Some("w".repeat(TITLE_CHARS).as_str())
        );
    }
}
