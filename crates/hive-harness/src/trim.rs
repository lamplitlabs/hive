//! Request trimming: before each model call, tool results older than the
//! most recent few and longer than a threshold travel as their first and
//! last characters around a one-line marker. Only the request changes;
//! the conversation the run keeps, the trajectory and the session's steps
//! stay whole, so the compaction summariser and the owner's console still
//! read the full output. Measured over the 36 owner task sessions of three
//! days (2,513 model calls, 79,000 prompt tokens a call on average): tool
//! results made up about 124,000 characters of an average request, 84% of
//! them in results older than the eight most recent, and the prompt grew
//! by about a thousand tokens a call until the request-size compaction at
//! 164,000. Sending results over 4,000 characters trimmed to 750 at each
//! end once eight newer ones exist cuts the result characters of an average
//! request by 45%: about 14,000 tokens a call at four characters a token,
//! 18,000 at the three the measured results tokenise to.

use crate::harness::is_observation_message;
use hive_provider::Message;

/// Tool results that always travel whole, counting back from the latest
/// (the `harness.trim_keep_recent` setting; 0 turns trimming off). Eight:
/// a reply often carries several calls, so eight results reach back three
/// to six turns, which is what the model is still working from, and the
/// measured saving past eight is small (keeping six saves 2% more, ten 2%
/// less).
pub const DEFAULT_TRIM_KEEP_RECENT: usize = 8;

/// Characters an older tool result may hold before it is trimmed (the
/// `harness.trim_over` setting; 0 turns trimming off). 4,000 is about the
/// 80th percentile of the measured results (median 1,300, p90 5,900) and
/// the results above it hold two thirds of all the result characters, so
/// the threshold takes most of the bulk while leaving the short results
/// the model refers back to (a file listing, a test summary) as they were.
pub const DEFAULT_TRIM_OVER: usize = 4000;

/// Characters kept at each end of a trimmed result: the head holds the
/// return code and the start of the output, the tail its last lines, which
/// for a check or a build is where the verdict is.
pub const TRIM_EDGE: usize = 750;

/// The first words of the line that stands in for the trimmed part.
pub const TRIM_MARKER: &str = "[... trimmed ";

/// The conversation as the next request sends it: every tool result
/// older than the `keep_recent` most recent ones and longer than `over`
/// characters is replaced by its head and tail around a marker. The
/// system prompt, the task, user and assistant messages, the compaction
/// summary and the latest results are untouched, and nothing moves, so
/// every result still follows the call it answers. Either setting at 0
/// returns the messages as they are.
pub fn trim_for_request(msgs: &[Message], keep_recent: usize, over: usize) -> Vec<Message> {
    let mut out = msgs.to_vec();
    if keep_recent == 0 || over == 0 {
        return out;
    }
    let results: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(i, m)| *i >= 2 && is_observation_message(*i, m))
        .map(|(i, _)| i)
        .collect();
    let Some(older) = results.len().checked_sub(keep_recent) else {
        return out;
    };
    for &i in &results[..older] {
        if let Some(trimmed) = trim_result(&out[i].content, over) {
            out[i].content = trimmed;
        }
    }
    out
}

/// One result trimmed to its first and last [`TRIM_EDGE`] characters
/// around a marker naming how much was left out, or None when it is
/// within `over` characters or already trimmed.
pub fn trim_result(content: &str, over: usize) -> Option<String> {
    if over == 0 || content.len() <= over || content.contains(TRIM_MARKER) {
        return None;
    }
    let head = content.floor_char_boundary(TRIM_EDGE);
    let tail = content.ceil_char_boundary(content.len() - TRIM_EDGE);
    if tail <= head {
        return None;
    }
    Some(format!(
        "{}\n{}\n{}",
        &content[..head],
        trim_marker(tail - head, content.len()),
        &content[tail..]
    ))
}

/// The line that stands in for the trimmed part of a result.
pub fn trim_marker(trimmed: usize, total: usize) -> String {
    format!(
        "{TRIM_MARKER}{} of {} characters of this older result; run the command again if you need them ...]",
        with_commas(trimmed),
        with_commas(total)
    )
}

/// Characters of tool results a request carries, and how many of them
/// trimming took out of it: the note the run writes the first time it
/// trims, and the numbers a replay reports.
pub fn result_chars(msgs: &[Message]) -> usize {
    msgs.iter()
        .enumerate()
        .filter(|(i, m)| *i >= 2 && is_observation_message(*i, m))
        .map(|(_, m)| m.content.len())
        .sum()
}

fn with_commas(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compact::{COMPACT_MARKER, apply_compaction};
    use hive_provider::{Role, ToolCall};

    fn m(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
            ..Message::new(Role::User)
        }
    }

    fn observation(size: usize, fill: char) -> String {
        format!(
            "<returncode>0</returncode>\n<output>\n{}\n</output>",
            fill.to_string().repeat(size)
        )
    }

    /// System, task, then `turns` tool rounds: an assistant call and its
    /// result of `size` characters each.
    fn conversation(turns: usize, size: usize) -> Vec<Message> {
        let mut msgs = vec![m(Role::System, "system"), m(Role::User, "the task")];
        for n in 0..turns {
            let mut call = m(Role::Assistant, "");
            call.tool_calls = vec![ToolCall {
                id: format!("c{n}"),
                name: "bash".into(),
                arguments: format!("{{\"command\":\"cmd {n}\"}}"),
            }];
            let mut result = m(
                Role::Tool,
                &observation(size, char::from(b'a' + (n % 26) as u8)),
            );
            result.tool_call_id = format!("c{n}");
            result.name = "bash".into();
            msgs.push(call);
            msgs.push(result);
        }
        msgs
    }

    #[test]
    fn the_latest_results_travel_whole_and_older_long_ones_trimmed() {
        let msgs = conversation(12, 6000);
        let out = trim_for_request(&msgs, 8, 4000);
        assert_eq!(out.len(), msgs.len(), "nothing is added or removed");
        let results: Vec<usize> = (0..out.len())
            .filter(|&i| out[i].role == Role::Tool)
            .collect();
        assert_eq!(results.len(), 12);
        for &i in &results[..4] {
            let text = &out[i].content;
            assert!(text.contains(TRIM_MARKER), "{i}: {text}");
            assert!(text.len() < 2 * TRIM_EDGE + 120, "{i}: {}", text.len());
            assert!(text.starts_with("<returncode>0</returncode>\n<output>\n"));
            assert!(text.ends_with("\n</output>"));
            assert_eq!(out[i].tool_call_id, msgs[i].tool_call_id, "pairing kept");
        }
        for &i in &results[4..] {
            assert_eq!(out[i], msgs[i], "the latest eight are as they were");
        }
        assert_eq!(out[0], msgs[0]);
        assert_eq!(out[1], msgs[1]);
        for i in (2..out.len()).filter(|&i| out[i].role == Role::Assistant) {
            assert_eq!(out[i], msgs[i], "assistant turns are never trimmed");
        }
        assert!(
            out[3]
                .content
                .contains("[... trimmed 4,546 of 6,046 characters of this older result; run the command again if you need them ...]"),
            "{}",
            out[3].content
        );
    }

    #[test]
    fn short_results_and_the_original_stay_as_they_are() {
        let msgs = conversation(12, 3000);
        let out = trim_for_request(&msgs, 8, 4000);
        assert_eq!(out, msgs, "3,000-character results are under the threshold");
        let mut mixed = conversation(12, 6000);
        mixed[3].content = observation(100, 'x');
        let out = trim_for_request(&mixed, 8, 4000);
        assert_eq!(out[3], mixed[3], "a short old result is left alone");
        assert!(out[5].content.contains(TRIM_MARKER));
        assert!(
            mixed[5].content.len() > 6000,
            "the conversation itself keeps the whole result"
        );
    }

    #[test]
    fn zero_turns_trimming_off() {
        let msgs = conversation(12, 6000);
        assert_eq!(trim_for_request(&msgs, 0, 4000), msgs);
        assert_eq!(trim_for_request(&msgs, 8, 0), msgs);
        assert_eq!(trim_result("x".repeat(10_000).as_str(), 0), None);
    }

    #[test]
    fn fenced_mode_observations_count_as_results_and_user_turns_do_not() {
        let mut msgs = vec![m(Role::System, "system"), m(Role::User, "the task")];
        for n in 0..10 {
            msgs.push(m(Role::Assistant, &format!("```bash\ncmd {n}\n```")));
            msgs.push(m(Role::User, &observation(5000, 'o')));
        }
        let long_user = format!("Please also {}", "look. ".repeat(1500));
        msgs.insert(4, m(Role::User, &long_user));
        let out = trim_for_request(&msgs, 2, 4000);
        assert_eq!(out[4].content, long_user, "a user message is never trimmed");
        assert!(
            out[3].content.contains(TRIM_MARKER),
            "the oldest observation is"
        );
        let last = out.len() - 1;
        assert_eq!(out[last], msgs[last]);
        assert_eq!(out[last - 2], msgs[last - 2]);
        assert!(out[last - 4].content.contains(TRIM_MARKER));
    }

    #[test]
    fn trimming_applies_after_a_compaction_summary_and_leaves_it_whole() {
        let msgs = conversation(14, 6000);
        let summary = format!("Goal: finish.\nFacts: {}", "f".repeat(9000));
        let compacted = apply_compaction(&msgs, 8, &summary);
        assert!(compacted[2].content.starts_with(COMPACT_MARKER));
        let out = trim_for_request(&compacted, 4, 4000);
        assert_eq!(out[2], compacted[2], "the summary travels whole");
        let results: Vec<usize> = (0..out.len())
            .filter(|&i| out[i].role == Role::Tool)
            .collect();
        assert_eq!(results.len(), 11);
        for &i in &results[..7] {
            assert!(out[i].content.contains(TRIM_MARKER), "{i}");
        }
        for &i in &results[7..] {
            assert_eq!(out[i], compacted[i]);
        }
    }

    #[test]
    fn a_trimmed_result_is_not_trimmed_again_and_multibyte_text_is_safe() {
        let once = trim_result(&observation(9000, 'q'), 4000).unwrap();
        assert_eq!(trim_result(&once, 4000), None);
        let uni = trim_result(&observation(5000, 'é'), 4000).unwrap();
        assert!(uni.contains(TRIM_MARKER));
        assert!(uni.ends_with("\n</output>"));
        assert_eq!(trim_result(&observation(4000, 'z'), 4046), None);
    }

    #[test]
    fn result_chars_counts_results_only() {
        let msgs = conversation(3, 1000);
        let each = msgs[3].content.len();
        assert_eq!(result_chars(&msgs), 3 * each);
        assert_eq!(result_chars(&msgs[..2]), 0);
        assert_eq!(with_commas(0), "0");
        assert_eq!(with_commas(999), "999");
        assert_eq!(with_commas(41_200), "41,200");
        assert_eq!(with_commas(1_234_567), "1,234,567");
    }
}
