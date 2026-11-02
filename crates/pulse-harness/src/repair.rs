//! Repairing a conversation that carries a tool call whose arguments are
//! not JSON. A reply the gateway cut mid-stream used to leave one behind
//! (`{"path": "AGENTS.md", "replace": "..."` with no closing brace): the
//! harness could not run it, and a vendor that validates its input refused
//! every later request carrying it (`Invalid JSON format in tool call
//! arguments`, HTTP 400), so the conversation was stuck for good. The
//! provider no longer hands such a reply over; this module heals the
//! histories that have one anyway - a reply the model's own token limit
//! cut, at dispatch; a conversation a vendor refuses, before the request
//! is sent again; a saved conversation, when it is carried over.

use pulse_provider::{EMPTY_ARGUMENTS, Message, Role, ToolCall, invalid_arguments};

/// One tool call the repair rewrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepairedCall {
    pub id: String,
    pub name: String,
    /// The arguments as they were.
    pub arguments: String,
    /// Why they could not stay: serde's reason when they did not parse;
    /// empty when they were empty (a tool called with none, which the
    /// harness ran as `{}` and which the vendors do not take back as `""`).
    pub reason: String,
}

impl RepairedCall {
    /// `edit toolu_01D9...`, for a note.
    pub fn label(&self) -> String {
        if self.id.is_empty() {
            self.name.clone()
        } else {
            format!("{} {}", self.name, self.id)
        }
    }
}

/// Rewrites `call`'s arguments to `{}` when they cannot go back to a
/// vendor as they are (not JSON, or empty) and says what they were; None
/// when they are JSON and stay.
pub fn repair_call(call: &mut ToolCall) -> Option<RepairedCall> {
    let reason = match invalid_arguments(&call.arguments) {
        Some(reason) => reason,
        None if call.arguments.trim().is_empty() => String::new(),
        None => return None,
    };
    let arguments = std::mem::replace(&mut call.arguments, EMPTY_ARGUMENTS.to_string());
    Some(RepairedCall {
        id: call.id.clone(),
        name: call.name.clone(),
        arguments,
        reason,
    })
}

/// The tool result for a call whose arguments did not parse: what went
/// wrong, what arrived (the model sees what it sent, cut where it was
/// cut), and what to do.
pub fn invalid_arguments_result(name: &str, arguments: &str, reason: &str) -> String {
    format!(
        "{name}: arguments are not JSON ({reason}). What arrived: {arguments}. Call the tool again with complete JSON arguments."
    )
}

/// Rewrites, in place, every tool call of `msgs` whose arguments are not
/// JSON (or are empty) to `{}`, the object every vendor takes back -
/// OpenAI's `arguments` string, Anthropic's `input`, Gemini's `args` - and
/// gives a call that has no tool result at all one, right after its
/// message, saying the arguments were not JSON. Tool results that exist
/// stay: they already told the model. Returns what was rewritten, in
/// order; nothing for a sound history, the common case.
pub fn repair_tool_arguments(msgs: &mut Vec<Message>) -> Vec<RepairedCall> {
    let mut repaired = Vec::new();
    let mut i = 0;
    while i < msgs.len() {
        let (head, tail) = msgs.split_at_mut(i + 1);
        let msg = &mut head[i];
        let mut results: Vec<Message> = Vec::new();
        if matches!(msg.role, Role::Assistant) {
            for call in &mut msg.tool_calls {
                let answered = |id: &str| {
                    tail.iter()
                        .any(|m| matches!(m.role, Role::Tool) && m.tool_call_id == id)
                };
                let Some(fixed) = repair_call(call) else {
                    continue;
                };
                if !answered(&call.id) {
                    let content = if fixed.reason.is_empty() {
                        format!(
                            "{}: the call carried no arguments and no result was recorded. Call the tool again if it is still needed.",
                            fixed.name
                        )
                    } else {
                        invalid_arguments_result(&fixed.name, &fixed.arguments, &fixed.reason)
                    };
                    results.push(Message {
                        role: Role::Tool,
                        content,
                        tool_call_id: call.id.clone(),
                        name: call.name.clone(),
                        ..Message::new(Role::Tool)
                    });
                }
                repaired.push(fixed);
            }
        }
        let added = results.len();
        for (k, result) in results.into_iter().enumerate() {
            msgs.insert(i + 1 + k, result);
        }
        i += 1 + added;
    }
    repaired
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The last message of the user's conversation: an `edit` whose
    /// arguments the Copilot gateway cut at column 93, with the tool
    /// result the harness gave it.
    const CUT_ARGUMENTS: &str = r#"{"path": "AGENTS.md", "replace": "  with the GitHub noreply address.\n\n## Rollback Planning""#;
    const CUT_ID: &str = "toolu_01D9XWUQtuVBTWixzvum3zYd";

    /// The second conversation stuck the same day: a `bash` call cut one
    /// byte short, the closing brace alone missing.
    const CUT_SHORT_ARGUMENTS: &str = r#"{"command": "pulse manifest sync --project leben-in-deutschland 2>&1 | tail -6; echo ---; pulse skill list --project leben-in-deutschland 2>&1 | grep -i \"scraper\"""#;
    const CUT_SHORT_ID: &str = "toolu_01PToC5gLgbC4XyNrtnHnH13";

    fn assistant(calls: Vec<(&str, &str, &str)>) -> Message {
        Message {
            tool_calls: calls
                .into_iter()
                .map(|(id, name, arguments)| ToolCall {
                    id: id.to_string(),
                    name: name.to_string(),
                    arguments: arguments.to_string(),
                })
                .collect(),
            ..Message::new(Role::Assistant)
        }
    }

    fn tool(id: &str, name: &str, content: &str) -> Message {
        Message {
            content: content.to_string(),
            tool_call_id: id.to_string(),
            name: name.to_string(),
            ..Message::new(Role::Tool)
        }
    }

    fn user(content: &str) -> Message {
        Message {
            content: content.to_string(),
            ..Message::new(Role::User)
        }
    }

    #[test]
    fn the_users_cut_edit_call_is_rewritten_and_its_result_kept() {
        let result =
            "edit: arguments are not JSON: EOF while parsing an object at line 1 column 93";
        let mut msgs = vec![
            user("Please update AGENTS.md"),
            assistant(vec![("call-1", "read", r#"{"path":"AGENTS.md"}"#)]),
            tool("call-1", "read", "## Rollback Planning"),
            assistant(vec![(CUT_ID, "edit", CUT_ARGUMENTS)]),
            tool(CUT_ID, "edit", result),
            user("Please land this branch now"),
        ];
        let before = msgs.clone();
        let repaired = repair_tool_arguments(&mut msgs);
        assert_eq!(
            repaired,
            vec![RepairedCall {
                id: CUT_ID.to_string(),
                name: "edit".to_string(),
                arguments: CUT_ARGUMENTS.to_string(),
                reason: "EOF while parsing an object at line 1 column 93".to_string(),
            }]
        );
        assert_eq!(repaired[0].label(), format!("edit {CUT_ID}"));
        assert_eq!(msgs.len(), before.len(), "the result that exists stays");
        assert_eq!(msgs[3].tool_calls[0].arguments, EMPTY_ARGUMENTS);
        assert_eq!(msgs[4], before[4]);
        // Everything else is untouched, and the repaired history parses.
        for (i, m) in msgs.iter().enumerate() {
            if i != 3 {
                assert_eq!(*m, before[i], "message {i}");
            }
            for call in &m.tool_calls {
                assert!(invalid_arguments(&call.arguments).is_none());
            }
        }
        // A second pass finds nothing.
        assert!(repair_tool_arguments(&mut msgs).is_empty());
    }

    #[test]
    fn the_other_conversations_cut_bash_call_is_rewritten_too() {
        // One byte short, with the result the old harness gave it (a
        // bash call whose arguments did not parse reads as having no
        // command).
        let mut msgs = vec![
            user("sync the manifest"),
            assistant(vec![(CUT_SHORT_ID, "bash", CUT_SHORT_ARGUMENTS)]),
            tool(
                CUT_SHORT_ID,
                "bash",
                "tool arguments must be a JSON object with a string command",
            ),
            user("try again"),
        ];
        let repaired = repair_tool_arguments(&mut msgs);
        assert_eq!(repaired.len(), 1);
        assert_eq!(repaired[0].id, CUT_SHORT_ID);
        assert_eq!(repaired[0].arguments, CUT_SHORT_ARGUMENTS);
        assert_eq!(
            repaired[0].reason,
            "EOF while parsing an object at line 1 column 165"
        );
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[1].tool_calls[0].arguments, EMPTY_ARGUMENTS);
        assert_eq!(
            msgs[2].content,
            "tool arguments must be a JSON object with a string command"
        );
    }

    #[test]
    fn a_call_without_a_result_gets_one_that_says_what_arrived() {
        let mut msgs = vec![
            user("go"),
            assistant(vec![
                ("c-1", "bash", r#"{"command":"true"}"#),
                ("c-2", "edit", r#"{"path": "a.md", "replace": "x"#),
                ("c-3", "list", ""),
            ]),
            tool("c-1", "bash", "<returncode>0</returncode>"),
            tool("c-3", "list", "nothing"),
            user("and then"),
        ];
        let repaired = repair_tool_arguments(&mut msgs);
        assert_eq!(repaired.len(), 2);
        assert_eq!(repaired[0].id, "c-2");
        assert!(repaired[0].reason.starts_with("EOF while parsing"));
        assert_eq!(repaired[1].id, "c-3");
        assert_eq!(repaired[1].reason, "");
        assert_eq!(repaired[1].arguments, "");
        assert_eq!(msgs.len(), 6);
        assert_eq!(msgs[1].tool_calls[0].arguments, r#"{"command":"true"}"#);
        assert_eq!(msgs[1].tool_calls[1].arguments, EMPTY_ARGUMENTS);
        assert_eq!(msgs[1].tool_calls[2].arguments, EMPTY_ARGUMENTS);
        let added = &msgs[2];
        assert_eq!(added.role, Role::Tool);
        assert_eq!(added.tool_call_id, "c-2");
        assert_eq!(added.name, "edit");
        assert_eq!(
            added.content,
            r#"edit: arguments are not JSON (EOF while parsing a string at line 1 column 30). What arrived: {"path": "a.md", "replace": "x. Call the tool again with complete JSON arguments."#
        );
        assert_eq!(msgs[3].tool_call_id, "c-1", "the existing results follow");
        assert_eq!(msgs[4].tool_call_id, "c-3");
        assert_eq!(msgs[5].content, "and then");
    }

    #[test]
    fn a_sound_history_is_left_alone() {
        let mut msgs = vec![
            Message {
                content: "system".to_string(),
                ..Message::new(Role::System)
            },
            user("go"),
            assistant(vec![("c-1", "bash", r#"{"command":"true"}"#)]),
            tool("c-1", "bash", "ok"),
            // Arguments the tool rejects as the wrong shape are still JSON:
            // the tool's result already told the model, nothing to repair.
            assistant(vec![("c-2", "read", "[1, 2]")]),
            tool("c-2", "read", "read: arguments must be a JSON object"),
            Message {
                content: "done".to_string(),
                ..Message::new(Role::Assistant)
            },
        ];
        let before = msgs.clone();
        assert!(repair_tool_arguments(&mut msgs).is_empty());
        assert_eq!(msgs, before);
        let mut empty: Vec<Message> = Vec::new();
        assert!(repair_tool_arguments(&mut empty).is_empty());
    }

    #[test]
    fn repair_call_rewrites_only_what_cannot_go_back() {
        let mut call = ToolCall {
            id: "c".to_string(),
            name: "bash".to_string(),
            arguments: r#"{"command":"true"}"#.to_string(),
        };
        assert_eq!(repair_call(&mut call), None);
        assert_eq!(call.arguments, r#"{"command":"true"}"#);
        call.arguments = "  ".to_string();
        let fixed = repair_call(&mut call).unwrap();
        assert_eq!(fixed.reason, "");
        assert_eq!(fixed.arguments, "  ");
        assert_eq!(call.arguments, EMPTY_ARGUMENTS);
        call.arguments = CUT_ARGUMENTS.to_string();
        let fixed = repair_call(&mut call).unwrap();
        assert_eq!(
            fixed.reason,
            "EOF while parsing an object at line 1 column 93"
        );
        assert_eq!(call.arguments, EMPTY_ARGUMENTS);
        assert_eq!(
            invalid_arguments_result("edit", &fixed.arguments, &fixed.reason),
            format!(
                "edit: arguments are not JSON (EOF while parsing an object at line 1 column 93). What arrived: {CUT_ARGUMENTS}. Call the tool again with complete JSON arguments."
            )
        );
        let unnamed = RepairedCall {
            id: String::new(),
            name: "edit".to_string(),
            arguments: String::new(),
            reason: String::new(),
        };
        assert_eq!(unnamed.label(), "edit");
    }
}
