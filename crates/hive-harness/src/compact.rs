//! Context compaction: when a run's prompt grows past a share of the
//! model's context window, or the request it is about to send past the
//! bytes the provider will take, the older part of the conversation is
//! replaced by a summary the model writes, and the run goes on with the
//! summary plus the most recent messages. The system prompt and the task
//! always stay.

use hive_provider::{Message, Request, Role, Tool};

/// Percent of the context window a prompt may use before compaction
/// (the `harness.compact_at` setting; 0 turns compaction off). Half the
/// window: past it every call re-reads a long history for little gain,
/// and the providers that bill per token charge for all of it each time.
pub const DEFAULT_COMPACT_AT: i32 = 50;

/// MiB one request may run to before the conversation is compacted,
/// whatever the token count says (the `provider.max_request_mb` setting;
/// 0 turns the size trigger off). The Copilot gateway times out reading
/// a body of about 1.5 MiB and more (`Timed out reading request body. Try
/// again, or use a smaller request size.`, HTTP 408), far under the 70%
/// of a 1M-token window the token trigger alone would wait for.
pub const DEFAULT_MAX_REQUEST_MB: f64 = 1.0;

/// The most recent messages compaction never touches.
pub const COMPACT_KEEP_RECENT: usize = 6;

/// Model calls between two compactions (or after one that failed).
pub const COMPACT_MIN_GAP_CALLS: i32 = 3;

/// The first line of the message that stands in for the compacted part.
pub const COMPACT_MARKER: &str = "[Earlier conversation compacted]";

/// Characters of one message kept in the transcript the summarizer reads.
pub const COMPACT_MESSAGE_LIMIT: usize = 3000;

/// Characters of the whole transcript the summarizer reads: about 30,000
/// tokens, forty messages at their limit. The oldest part is cut first
/// and the earlier summary, which carries it, always travels whole, so a
/// cut loses little; the summary call stays cheap and quick instead of
/// re-reading up to 100,000 tokens of history each time.
pub const COMPACT_TRANSCRIPT_LIMIT: usize = 120_000;

/// Tokens the summary may run to.
pub const COMPACT_SUMMARY_TOKENS: i32 = 4000;

const SUMMARY_SYSTEM: &str = "You are compacting the context of a coding agent that is part-way through a task so it can go on with less history. Write a summary in plain text that lets the agent continue exactly where it left off. Cover, under these headings: Goal (the task and what done looks like); Done so far (files read or changed, commands run and what they showed, decisions and why); Facts learned (paths, names, versions, errors, numbers, anything the agent would otherwise have to look up again); Open problems; Next steps (what to do first). Keep exact file paths, identifiers, commands and error messages verbatim. Keep anything the user or the task asked to keep. When the transcript opens with an earlier summary, carry its facts forward unless later messages supersede them: it is all that remains of the conversation before it. Be concrete and complete; leave out pleasantries and reasoning that led nowhere.";

/// True when the prompt is large enough, relative to the window, that the
/// run should compact before the next call.
pub fn should_compact(compact_at: i32, context_window: i32, last_prompt_tokens: i32) -> bool {
    if compact_at <= 0 || context_window <= 0 || last_prompt_tokens <= 0 {
        return false;
    }
    let threshold = (f64::from(context_window) * f64::from(compact_at.min(100)) / 100.0) as i32;
    last_prompt_tokens > threshold
}

/// The byte budget of one request for a `provider.max_request_mb` value:
/// 0 (or less, or not a number) turns the size trigger off.
pub fn request_budget_bytes(max_request_mb: f64) -> usize {
    if !max_request_mb.is_finite() || max_request_mb <= 0.0 {
        return 0;
    }
    (max_request_mb * 1024.0 * 1024.0) as usize
}

/// About how many bytes a request of `msgs` and `tools` takes on the wire:
/// every message's text, tool-call arguments and images, the tool schemas,
/// and a little per field for the JSON around them. The vendors' shapes
/// differ from this by a few percent, not more; the estimate is there to
/// compact before a gateway refuses the body, not to bill by it.
pub fn estimate_request_bytes(msgs: &[Message], tools: &[Tool]) -> usize {
    const MESSAGE_OVERHEAD: usize = 48;
    const TOOL_CALL_OVERHEAD: usize = 80;
    const IMAGE_OVERHEAD: usize = 64;
    const TOOL_OVERHEAD: usize = 64;
    let messages: usize = msgs
        .iter()
        .map(|m| {
            MESSAGE_OVERHEAD
                + m.content.len()
                + m.name.len()
                + m.tool_call_id.len()
                + m.tool_calls
                    .iter()
                    .map(|c| TOOL_CALL_OVERHEAD + c.id.len() + c.name.len() + c.arguments.len())
                    .sum::<usize>()
                + m.images
                    .iter()
                    .map(|i| IMAGE_OVERHEAD + i.media_type.len() + i.data.len())
                    .sum::<usize>()
        })
        .sum();
    let tools: usize = tools
        .iter()
        .map(|t| {
            TOOL_OVERHEAD + t.name.len() + t.description.len() + t.parameters.to_string().len()
        })
        .sum();
    messages + tools
}

/// True when a request of `bytes` is past `budget` ([`request_budget_bytes`];
/// 0 is off): the run should compact before sending it.
pub fn over_request_budget(bytes: usize, budget: usize) -> bool {
    budget > 0 && bytes > budget
}

/// Where the kept tail starts: the latest assistant turn that leaves at
/// least `keep` messages after it and at least one full turn before it.
/// Cutting at an assistant message keeps every tool result next to the
/// call that asked for it. None when the conversation is too short.
pub fn compaction_cut(msgs: &[Message], keep: usize) -> Option<usize> {
    let keep = keep.max(1);
    let last = msgs.len().checked_sub(keep)?;
    (4..=last).rev().find(|&i| msgs[i].role == Role::Assistant)
}

/// The compacted part of the conversation as text for the summarizer:
/// `msgs[2..cut]`, one block per message, long ones cut in the middle.
pub fn render_transcript(msgs: &[Message], cut: usize) -> String {
    let mut blocks: Vec<String> = Vec::new();
    for (i, m) in msgs.iter().enumerate().take(cut).skip(2) {
        let mut text = String::new();
        match m.role {
            Role::Assistant => {
                text.push_str("## assistant\n");
                if !m.content.trim().is_empty() {
                    text.push_str(&middle_cut(m.content.trim(), COMPACT_MESSAGE_LIMIT));
                    text.push('\n');
                }
                for tc in &m.tool_calls {
                    text.push_str(&format!(
                        "[tool call {}: {}]\n",
                        tc.name,
                        middle_cut(tc.arguments.trim(), 600)
                    ));
                }
            }
            Role::Tool => {
                let name = if m.name.is_empty() {
                    "result".to_string()
                } else {
                    m.name.clone()
                };
                text.push_str(&format!(
                    "## tool result ({name})\n{}\n",
                    middle_cut(m.content.trim(), COMPACT_MESSAGE_LIMIT)
                ));
            }
            Role::User => {
                // The earlier summary is bounded by COMPACT_SUMMARY_TOKENS
                // and is all that remains of what came before it, so it
                // travels whole.
                if m.content.starts_with(COMPACT_MARKER) {
                    text.push_str(&format!("## earlier summary\n{}\n", m.content.trim()));
                } else {
                    let head =
                        if i >= 2 && m.content.starts_with(super::harness::RETURN_CODE_PREFIX) {
                            "## observation"
                        } else {
                            "## user"
                        };
                    text.push_str(&format!(
                        "{head}\n{}\n",
                        middle_cut(m.content.trim(), COMPACT_MESSAGE_LIMIT)
                    ));
                }
            }
            Role::System => {
                text.push_str(&format!(
                    "## system\n{}\n",
                    middle_cut(m.content.trim(), COMPACT_MESSAGE_LIMIT)
                ));
            }
        }
        blocks.push(text);
    }
    let mut out = blocks.join("\n");
    if out.len() > COMPACT_TRANSCRIPT_LIMIT {
        // A cut drops the oldest part, which after one compaction is the
        // earlier summary; that block stays and the cut falls on the rest.
        let (head, rest) = match blocks.first() {
            Some(first) if first.starts_with("## earlier summary\n") => {
                (format!("{first}\n"), blocks[1..].join("\n"))
            }
            _ => (String::new(), out),
        };
        let budget = COMPACT_TRANSCRIPT_LIMIT.saturating_sub(head.len());
        let start = rest.ceil_char_boundary(rest.len().saturating_sub(budget));
        out = format!(
            "{head}[earliest part of the transcript omitted]\n\n{}",
            &rest[start..]
        );
    }
    out
}

/// The request that asks `model` for the summary; `asks` is what the
/// owner's `pre_compact` hooks want kept, if anything.
pub fn summary_request(model: &str, task: &str, transcript: &str, asks: &str) -> Request {
    let asks = if asks.trim().is_empty() {
        String::new()
    } else {
        format!(
            "\n\nThe owner's pre_compact hook asks this of the summary; honour it:\n{}",
            asks.trim()
        )
    };
    let user = format!(
        "The task the agent is working on:\n\n{}\n\nThe part of the conversation to compact, oldest first:\n\n{transcript}{asks}\n\nWrite the summary now.",
        middle_cut(task.trim(), 12_000)
    );
    Request {
        model: model.to_string(),
        messages: vec![
            Message {
                role: Role::System,
                content: SUMMARY_SYSTEM.to_string(),
                ..Message::new(Role::System)
            },
            Message {
                role: Role::User,
                content: user,
                ..Message::new(Role::User)
            },
        ],
        tools: Vec::new(),
        max_tokens: COMPACT_SUMMARY_TOKENS,
        temperature: None,
        initiator: "agent".to_string(),
        reasoning: String::new(),
    }
}

/// The message that replaces the compacted part.
pub fn compacted_message(summary: &str, replaced: usize) -> Message {
    Message {
        role: Role::User,
        content: format!(
            "{COMPACT_MARKER}\nThe {replaced} messages before this point were replaced by this summary, which you wrote. Continue the task from here; the messages after this one are the most recent ones, unchanged. Do not repeat work the summary records as done.\n\n{}",
            summary.trim()
        ),
        ..Message::new(Role::User)
    }
}

/// The conversation after compaction: system prompt, task, the summary
/// message, then `msgs[cut..]`.
pub fn apply_compaction(msgs: &[Message], cut: usize, summary: &str) -> Vec<Message> {
    let cut = cut.clamp(2, msgs.len());
    let mut out: Vec<Message> = Vec::with_capacity(msgs.len() - cut + 3);
    out.extend(msgs.iter().take(2).cloned());
    out.push(compacted_message(summary, cut - 2));
    out.extend(msgs[cut..].iter().cloned());
    out
}

/// Keeps the start and the end of a long text.
fn middle_cut(s: &str, limit: usize) -> String {
    if s.len() <= limit || limit < 40 {
        return s.to_string();
    }
    let head = s.floor_char_boundary(limit / 2);
    let tail = s.ceil_char_boundary(s.len() - limit / 2);
    format!(
        "{}\n[... {} characters cut ...]\n{}",
        &s[..head],
        tail - head,
        &s[tail..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hive_provider::ToolCall;

    fn m(role: Role, content: &str) -> Message {
        Message {
            role,
            content: content.to_string(),
            ..Message::new(Role::User)
        }
    }

    fn conversation() -> Vec<Message> {
        // S, U(task), then three turns: A(tc) T, A(tc) T T, A(text) U(obs), A(tc) T
        let mut a1 = m(Role::Assistant, "");
        a1.tool_calls = vec![ToolCall {
            id: "c1".into(),
            name: "read".into(),
            arguments: r#"{"path":"a.rs"}"#.into(),
        }];
        let mut t1 = m(
            Role::Tool,
            "<returncode>0</returncode>\n<output>\nfn a() {}\n</output>",
        );
        t1.tool_call_id = "c1".into();
        t1.name = "read".into();
        let mut a2 = m(Role::Assistant, "Two more.");
        a2.tool_calls = vec![
            ToolCall {
                id: "c2".into(),
                name: "grep".into(),
                arguments: "{}".into(),
            },
            ToolCall {
                id: "c3".into(),
                name: "glob".into(),
                arguments: "{}".into(),
            },
        ];
        let mut t2 = m(Role::Tool, "g");
        t2.tool_call_id = "c2".into();
        let mut t3 = m(Role::Tool, "h");
        t3.tool_call_id = "c3".into();
        let a3 = m(Role::Assistant, "```bash\nls\n```");
        let o3 = m(
            Role::User,
            "<returncode>0</returncode>\n<output>\na.rs\n</output>",
        );
        let mut a4 = m(Role::Assistant, "");
        a4.tool_calls = vec![ToolCall {
            id: "c4".into(),
            name: "edit".into(),
            arguments: "{}".into(),
        }];
        let mut t4 = m(Role::Tool, "ok");
        t4.tool_call_id = "c4".into();
        vec![
            m(Role::System, "system"),
            m(Role::User, "the task"),
            a1,
            t1,
            a2,
            t2,
            t3,
            a3,
            o3,
            a4,
            t4,
        ]
    }

    #[test]
    fn threshold_is_a_share_of_the_window() {
        assert!(should_compact(70, 1000, 701));
        assert!(!should_compact(70, 1000, 700));
        assert!(!should_compact(0, 1000, 999), "0 is off");
        assert!(!should_compact(70, 0, 999), "no window known");
        assert!(!should_compact(70, 1000, 0), "no measurement yet");
        assert!(should_compact(150, 1000, 1001), "capped at 100 percent");
        assert_eq!(DEFAULT_COMPACT_AT, 50);
    }

    #[test]
    fn the_request_budget_is_mebibytes_and_zero_is_off() {
        assert_eq!(request_budget_bytes(1.0), 1024 * 1024);
        assert_eq!(request_budget_bytes(0.5), 512 * 1024);
        assert_eq!(request_budget_bytes(DEFAULT_MAX_REQUEST_MB), 1024 * 1024);
        assert_eq!(request_budget_bytes(0.0), 0);
        assert_eq!(request_budget_bytes(-2.0), 0);
        assert_eq!(request_budget_bytes(f64::NAN), 0);
        assert!(over_request_budget(1024 * 1024 + 1, 1024 * 1024));
        assert!(!over_request_budget(1024 * 1024, 1024 * 1024));
        assert!(!over_request_budget(usize::MAX, 0), "0 is off");
    }

    #[test]
    fn the_size_estimate_counts_text_calls_images_and_tools() {
        let msgs = conversation();
        let text: usize = msgs
            .iter()
            .map(|m| {
                m.content.len()
                    + m.tool_calls
                        .iter()
                        .map(|c| c.arguments.len())
                        .sum::<usize>()
            })
            .sum();
        let plain = estimate_request_bytes(&msgs, &[]);
        assert!(plain > text, "{plain} covers the JSON around {text} chars");
        assert!(plain < text + 200 * msgs.len(), "{plain} is not far over");
        // A big observation moves the estimate by about its size.
        let mut big = msgs.clone();
        big[3].content.push_str(&"x".repeat(100_000));
        let grown = estimate_request_bytes(&big, &[]);
        assert!(
            (100_000..100_010).contains(&(grown - plain)),
            "{}",
            grown - plain
        );
        // So does an image: its base64 travels in the body.
        let mut shot = msgs.clone();
        shot[1].images.push(hive_provider::ImageContent {
            media_type: "image/png".into(),
            data: "A".repeat(50_000),
        });
        let with_image = estimate_request_bytes(&shot, &[]);
        assert!(with_image - plain >= 50_000, "{}", with_image - plain);
        // And the tool schemas, which ride every request.
        let tool = Tool {
            name: "bash".into(),
            description: "Runs a command.".into(),
            parameters: serde_json::json!({"type":"object","properties":{"command":{"type":"string","description":"the command"}}}),
        };
        let with_tool = estimate_request_bytes(&msgs, std::slice::from_ref(&tool));
        let schema = tool.parameters.to_string().len() + tool.name.len() + tool.description.len();
        assert!(with_tool - plain >= schema, "{}", with_tool - plain);
        assert_eq!(estimate_request_bytes(&[], &[]), 0);
    }

    #[test]
    fn the_cut_lands_on_an_assistant_turn_leaving_the_tail() {
        let msgs = conversation();
        assert_eq!(msgs.len(), 11);
        // keep 6 -> last index 5; the latest assistant at or before 5 is a2 (index 4)
        assert_eq!(compaction_cut(&msgs, 6), Some(4));
        // keep 2 -> last index 9 = a4
        assert_eq!(compaction_cut(&msgs, 2), Some(9));
        // keep 3 -> last index 8 (observation) -> back to a3 at 7
        assert_eq!(compaction_cut(&msgs, 3), Some(7));
        // nothing before index 4 counts as a turn worth compacting
        assert_eq!(compaction_cut(&msgs[..6], 3), None);
        assert_eq!(compaction_cut(&msgs[..2], 1), None);
        assert_eq!(compaction_cut(&[], 1), None);
    }

    #[test]
    fn transcript_names_every_part_and_cuts_long_ones() {
        let mut msgs = conversation();
        msgs[3].content = format!(
            "<returncode>0</returncode>\n<output>\n{}\n</output>",
            "x".repeat(10_000)
        );
        let text = render_transcript(&msgs, 9);
        assert!(
            text.starts_with("## assistant\n[tool call read: {\"path\":\"a.rs\"}]\n"),
            "{text}"
        );
        assert!(
            text.contains("## tool result (read)\n<returncode>0</returncode>"),
            "{text}"
        );
        assert!(text.contains("characters cut ...]"), "{text}");
        assert!(
            text.contains("## assistant\nTwo more.\n[tool call grep: {}]\n[tool call glob: {}]\n"),
            "{text}"
        );
        assert!(text.contains("## tool result (result)\ng\n"), "{text}");
        assert!(text.contains("## assistant\n```bash\nls\n```\n"), "{text}");
        assert!(
            text.contains("## observation\n<returncode>0</returncode>"),
            "{text}"
        );
        assert!(
            !text.contains("[tool call edit"),
            "index 9 is where the tail starts"
        );
        assert!(
            !text.contains("the task"),
            "the task is not part of the transcript"
        );
        assert!(text.len() < 4000, "{}", text.len());
    }

    #[test]
    fn compaction_replaces_the_middle_and_keeps_the_ends() {
        let msgs = conversation();
        let out = apply_compaction(&msgs, 7, "Goal: finish.\nDone so far: read a.rs.");
        assert_eq!(out.len(), 2 + 1 + 4);
        assert_eq!(out[0].content, "system");
        assert_eq!(out[1].content, "the task");
        assert_eq!(out[2].role, Role::User);
        assert!(
            out[2].content.starts_with(COMPACT_MARKER),
            "{}",
            out[2].content
        );
        assert!(
            out[2].content.contains("The 5 messages before this point"),
            "{}",
            out[2].content
        );
        assert!(
            out[2]
                .content
                .ends_with("Goal: finish.\nDone so far: read a.rs.")
        );
        assert_eq!(out[3].content, "```bash\nls\n```");
        assert_eq!(out[6].tool_call_id, "c4");
        // a second compaction folds the earlier summary into the transcript
        let again = render_transcript(&out, 5);
        assert!(
            again.starts_with("## earlier summary\n[Earlier conversation compacted]"),
            "{again}"
        );
    }

    #[test]
    fn a_cut_transcript_keeps_the_earlier_summary_whole() {
        let mut msgs = conversation();
        let summary = format!("Goal: finish.\nFacts: {}", "f".repeat(8000));
        msgs = apply_compaction(&msgs, 4, &summary);
        // Enough long observations after the summary to pass the limit.
        for _ in 0..200 {
            msgs.push(m(Role::Assistant, "```bash\ncat big\n```"));
            msgs.push(m(
                Role::User,
                format!(
                    "<returncode>0</returncode>\n<output>\n{}\n</output>",
                    "y".repeat(4000)
                )
                .as_str(),
            ));
        }
        let cut = msgs.len() - 2;
        let text = render_transcript(&msgs, cut);
        assert!(
            text.starts_with("## earlier summary\n[Earlier conversation compacted]"),
            "{}",
            &text[..120]
        );
        assert!(
            text.contains(&"f".repeat(8000)),
            "the summary is not middle-cut"
        );
        assert!(text.contains("[earliest part of the transcript omitted]"));
        assert!(
            text.len() <= COMPACT_TRANSCRIPT_LIMIT + 20_000,
            "{}",
            text.len()
        );
        let omitted = text.find("[earliest part").unwrap();
        assert!(
            omitted > 8000,
            "the cut falls after the summary, not before it"
        );
    }

    #[test]
    fn the_summary_request_has_no_tools_and_carries_the_task() {
        let req = summary_request("m-1", "Fix the bug", "## assistant\nhi\n", "");
        assert_eq!(req.model, "m-1");
        assert!(req.tools.is_empty());
        assert_eq!(req.initiator, "agent");
        assert_eq!(req.max_tokens, COMPACT_SUMMARY_TOKENS);
        assert_eq!(req.messages.len(), 2);
        assert_eq!(req.messages[0].role, Role::System);
        assert!(req.messages[0].content.contains("Next steps"));
        assert!(req.messages[1].content.contains("Fix the bug"));
        assert!(req.messages[1].content.contains("## assistant\nhi"));
    }

    #[test]
    fn middle_cut_keeps_both_ends() {
        assert_eq!(middle_cut("short", 100), "short");
        let long = format!("{}{}", "a".repeat(100), "b".repeat(100));
        let cut = middle_cut(&long, 60);
        assert!(cut.starts_with(&"a".repeat(30)));
        assert!(cut.ends_with(&"b".repeat(30)));
        assert!(cut.contains("[... 140 characters cut ...]"));
        let uni = "é".repeat(100);
        let cut = middle_cut(&uni, 61);
        assert!(cut.contains("characters cut"));
    }
}
