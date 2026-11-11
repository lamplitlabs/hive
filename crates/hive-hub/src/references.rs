//! References: a new conversation or task names earlier sessions and
//! tasks, and their summaries go in front of its first message, so the
//! agent starts with that context instead of gathering it again.

use crate::httpx::ResponseWriter;
use crate::server::write_error;
use serde_json::Value;
use std::collections::BTreeMap;

/// How many references one create may carry.
pub(crate) const MAX_REFERENCES: usize = 10;
/// How much of one piece (a brief, a reply, a result) a summary keeps.
const PIECE_CHARS: usize = 1500;

/// Reads `references` from the payload: a list of conversation ids and
/// task ids (TASK-...). None after writing a 400.
pub(crate) fn parse(
    w: &mut ResponseWriter,
    payload: &BTreeMap<String, Value>,
) -> Option<Vec<String>> {
    let bad = |w: &mut ResponseWriter, why: &str| {
        write_error(
            w,
            400,
            "Invalid references".to_string(),
            why.to_string(),
            "Send references as a list of session and task ids.".to_string(),
        );
    };
    let items = match payload.get("references") {
        None | Some(Value::Null) => return Some(Vec::new()),
        Some(Value::Array(a)) => a,
        _ => {
            bad(w, "references must be a list.");
            return None;
        }
    };
    let mut out: Vec<String> = Vec::new();
    for v in items {
        let Some(s) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            bad(w, "every reference must be a non-empty id.");
            return None;
        };
        if !out.iter().any(|o| o == s) {
            out.push(s.to_string());
        }
    }
    if out.len() > MAX_REFERENCES {
        bad(w, &format!("at most {MAX_REFERENCES} references."));
        return None;
    }
    Some(out)
}

fn clip(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= PIECE_CHARS {
        return s.to_string();
    }
    let head: String = s.chars().take(PIECE_CHARS).collect();
    format!("{head}…")
}

fn is_task(id: &str) -> bool {
    id.to_ascii_uppercase().starts_with("TASK-")
}

fn task_summary(store: &hive_store::Store, id: &str) -> Result<String, String> {
    let (t, _) = hive_tasks::get(store, id).map_err(|_| format!("task {id} not found"))?;
    let mut s = format!(
        "### Task {} — {} ({})\nBrief:\n{}\n",
        t.id,
        t.title,
        t.status,
        clip(&t.brief)
    );
    let jobs = hive_tasks::jobs(store, &t.id).unwrap_or_default();
    if let Some(j) = jobs.iter().rev().find(|j| !j.result.trim().is_empty()) {
        s.push_str(&format!(
            "Latest result ({}, {}):\n{}\n",
            j.id,
            j.status,
            clip(&j.result)
        ));
    }
    Ok(s)
}

fn session_summary(store: &hive_store::Store, id: &str) -> Result<String, String> {
    let row = store
        .chat_get(id)
        .ok()
        .flatten()
        .ok_or_else(|| format!("session {id} not found"))?;
    let msgs = store.chat_messages(id, 0).unwrap_or_default();
    let mut s = format!("### Session {} — {}\n", row.id, row.title);
    if let Some(m) = msgs.iter().find(|m| m.role == "user") {
        s.push_str(&format!("Asked:\n{}\n", clip(&m.content)));
    }
    if let Some(m) = msgs
        .iter()
        .rev()
        .find(|m| m.role == "assistant" && !m.content.trim().is_empty())
    {
        s.push_str(&format!("Last answer:\n{}\n", clip(&m.content)));
    }
    Ok(s)
}

/// The context block for `ids`, or an error naming the first id that
/// matches nothing. Empty for no references.
pub(crate) fn context(store: &hive_store::Store, ids: &[String]) -> Result<String, String> {
    if ids.is_empty() {
        return Ok(String::new());
    }
    let mut parts = Vec::new();
    for id in ids {
        parts.push(if is_task(id) {
            task_summary(store, id)?
        } else {
            session_summary(store, id)?
        });
    }
    Ok(format!(
        "## Context from earlier work\nThe owner referenced these; build on them instead of gathering the context again. Read more with the board or session tools only if this is not enough.\n\n{}\n---\n\n",
        parts.join("\n")
    ))
}

/// `text` with the referenced context in front; None after writing a 400.
pub(crate) fn with_context(
    w: &mut ResponseWriter,
    store: Option<&hive_store::Store>,
    ids: &[String],
    text: &str,
) -> Option<String> {
    if ids.is_empty() {
        return Some(text.to_string());
    }
    let Some(store) = store else {
        return Some(text.to_string());
    };
    match context(store, ids) {
        Ok(c) => Some(format!("{c}{text}")),
        Err(e) => {
            write_error(
                w,
                400,
                "Unknown reference".to_string(),
                format!("{e}."),
                "Pick sessions and tasks that exist.".to_string(),
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers_http::{additions_project, additions_store};

    #[test]
    fn context_summarises_sessions_and_tasks_and_names_unknown_ids() {
        let _env = hive_testutil::env_lock();
        let ts = additions_store(false);
        let s = &ts.store;
        additions_project(s, "demo");
        let c = s
            .chat_create(&hive_store::Conversation {
                id: "chat-ref-1".into(),
                title: "Fix the lander".into(),
                status: "idle".into(),
                project: "demo".into(),
                ..Default::default()
            })
            .unwrap();
        s.chat_add_message(&c.id, "user", "why does land fail?", &Value::Null)
            .unwrap();
        s.chat_add_message(&c.id, "assistant", "the lock was stale", &Value::Null)
            .unwrap();
        let t =
            hive_tasks::new(s, "demo", "Speed up verify", "pool the tests", 0, "owner").unwrap();

        let got = context(s, &[c.id.clone(), t.id.clone()]).unwrap();
        for want in [
            "Fix the lander",
            "why does land fail?",
            "the lock was stale",
            "Speed up verify",
            "pool the tests",
        ] {
            assert!(got.contains(want), "{want} missing from {got}");
        }
        assert_eq!(context(s, &[]).unwrap(), "");
        assert!(
            context(s, &["chat-nope".into()])
                .unwrap_err()
                .contains("chat-nope")
        );
        assert!(
            context(s, &["TASK-999".into()])
                .unwrap_err()
                .contains("TASK-999")
        );
    }
}
