//! Streaming replies for chat-completion vendors: reading a
//! `text/event-stream` body line by line, and folding its chunks back into
//! one [`Response`] while the text pieces go to the caller as they arrive.
//!
//! The chunk shape is OpenAI's (`choices[i].delta` with `content`,
//! `tool_calls[j]` keyed by `index`, a `finish_reason` on the last chunk of
//! a choice, `usage` on the final chunk when `stream_options.include_usage`
//! was asked for), which the OpenAI-compatible vendors and the Copilot API
//! share.

use crate::http::{decode_error, network_error};
use crate::{
    CancelToken, Choice, EMPTY_ARGUMENTS, Message, ProviderError, Response, Role, ToolCall, Usage,
};
use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};

/// The event stream's end marker.
const DONE: &str = "[DONE]";

/// Reads an event stream from `body`, handing the `data:` payload of each
/// event to `on_data` (multi-line data joined with newlines) until the
/// stream ends, `[DONE]` arrives or `on_data` fails. Comment lines and the
/// other fields (`event:`, `id:`, `retry:`) are skipped. More than
/// `max_bytes` fails with `body_too_large`; a transport error mid-stream
/// is a (retryable) network error; cancellation is checked between lines.
/// Ok(true) when `[DONE]` ended the stream, Ok(false) when the body ended
/// first (the caller decides whether what arrived is whole).
pub fn read_event_stream(
    provider: &str,
    cancel: &CancelToken,
    body: impl Read,
    max_bytes: usize,
    on_data: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<bool> {
    let mut reader = BufReader::new(body);
    let mut line = String::new();
    let mut data = String::new();
    let mut total = 0usize;
    loop {
        cancel.check()?;
        line.clear();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| network_error(provider, e))?;
        total += n;
        if total > max_bytes {
            return Err(ProviderError::new(
                provider,
                0,
                "body_too_large",
                format!(
                    "streamed response exceeded {} MiB",
                    max_bytes / (1024 * 1024)
                ),
                false,
            )
            .into());
        }
        let eof = n == 0;
        let text = line.trim_end_matches(['\n', '\r']);
        if text.is_empty() {
            if !data.is_empty() {
                let payload = std::mem::take(&mut data);
                if payload.trim() == DONE {
                    return Ok(true);
                }
                on_data(&payload)?;
            }
            if eof {
                return Ok(false);
            }
            continue;
        }
        if let Some(rest) = text.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
        // `event:`, `id:`, `retry:` and `:` comments carry nothing we use.
    }
}

/// One choice being assembled from its deltas.
#[derive(Debug, Default)]
struct ChunkChoice {
    role: Option<Role>,
    content: String,
    tool_calls: BTreeMap<u32, ChunkToolCall>,
    finish_reason: String,
}

#[derive(Debug, Default)]
struct ChunkToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// Folds chat-completion chunks into one [`Response`].
#[derive(Debug, Default)]
pub struct ChatChunks {
    id: String,
    model: String,
    choices: BTreeMap<u32, ChunkChoice>,
    usage: Usage,
    /// Chunks seen, so an empty stream can be told from a reply with
    /// nothing in it.
    pub chunks: usize,
}

impl ChatChunks {
    /// Feeds one chunk and returns the text it added, for the caller to
    /// show as it arrives. A chunk that is an error envelope
    /// (`{"error": {...}}`) is returned as the vendor's error.
    pub fn push(&mut self, provider: &str, chunk: &Value) -> Result<String> {
        if let Some(err) = chunk.get("error").filter(|e| !e.is_null()) {
            let (code, msg) = crate::http::parse_error_body(chunk.to_string().as_bytes());
            let msg = if msg.is_empty() { err.to_string() } else { msg };
            return Err(ProviderError::new(provider, 0, &code, &msg, false).into());
        }
        self.chunks += 1;
        if self.id.is_empty()
            && let Some(id) = chunk.get("id").and_then(Value::as_str)
        {
            self.id = id.to_string();
        }
        if self.model.is_empty()
            && let Some(model) = chunk.get("model").and_then(Value::as_str)
        {
            self.model = model.to_string();
        }
        if let Some(usage) = chunk.get("usage").filter(|u| u.is_object()) {
            let prompt = usage
                .get("prompt_tokens")
                .and_then(Value::as_i64)
                .unwrap_or_default() as i32;
            let completion = usage
                .get("completion_tokens")
                .and_then(Value::as_i64)
                .unwrap_or_default() as i32;
            // Vendors that send usage on every chunk send running totals.
            if prompt > 0 {
                self.usage.prompt_tokens = prompt;
            }
            if completion > 0 {
                self.usage.completion_tokens = completion;
            }
        }
        let mut text = String::new();
        let empty = Vec::new();
        for (i, choice) in chunk
            .get("choices")
            .and_then(Value::as_array)
            .unwrap_or(&empty)
            .iter()
            .enumerate()
        {
            let index = choice
                .get("index")
                .and_then(Value::as_u64)
                .map_or(i as u32, |v| v as u32);
            let slot = self.choices.entry(index).or_default();
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str)
                && !reason.is_empty()
            {
                slot.finish_reason = reason.to_string();
            }
            // A few servers put a whole `message` in a chunk instead of a delta.
            let delta = choice
                .get("delta")
                .or_else(|| choice.get("message"))
                .filter(|d| d.is_object());
            let Some(delta) = delta else {
                continue;
            };
            if slot.role.is_none()
                && let Some(role) = delta.get("role").and_then(Value::as_str)
            {
                slot.role = serde_json::from_value(Value::String(role.to_string())).ok();
            }
            if let Some(piece) = delta.get("content").and_then(Value::as_str)
                && !piece.is_empty()
            {
                slot.content.push_str(piece);
                text.push_str(piece);
            }
            for (j, tc) in delta
                .get("tool_calls")
                .and_then(Value::as_array)
                .unwrap_or(&empty)
                .iter()
                .enumerate()
            {
                let index = tc
                    .get("index")
                    .and_then(Value::as_u64)
                    .map_or(j as u32, |v| v as u32);
                let call = slot.tool_calls.entry(index).or_default();
                if let Some(id) = tc.get("id").and_then(Value::as_str)
                    && !id.is_empty()
                {
                    call.id = id.to_string();
                }
                if let Some(f) = tc.get("function") {
                    if let Some(name) = f.get("name").and_then(Value::as_str)
                        && !name.is_empty()
                    {
                        call.name.push_str(name);
                    }
                    if let Some(args) = f.get("arguments").and_then(Value::as_str) {
                        call.arguments.push_str(args);
                    }
                }
            }
        }
        Ok(text)
    }

    /// What is missing when the stream ended before the reply was whole,
    /// None when it is complete. The body ending with neither a
    /// `finish_reason` on any choice nor `[DONE]` is a connection the
    /// gateway closed between chunks; a tool call whose accumulated
    /// `arguments` do not parse as JSON lost its last chunks (the harness
    /// could neither run it nor send it back, and a vendor that validates
    /// its input refuses every later request carrying it). A choice the
    /// model's token limit cut (`finish_reason: length`) is whole as far as
    /// the stream goes: the harness shows the model what arrived. Empty
    /// arguments are a tool called with none, not a cut.
    pub fn incomplete(&self, done: bool) -> Option<String> {
        if !done && self.choices.values().all(|c| c.finish_reason.is_empty()) {
            return Some("no finish reason arrived".to_string());
        }
        for choice in self.choices.values() {
            if choice.finish_reason == "length" {
                continue;
            }
            for tc in choice.tool_calls.values() {
                if let Some(e) = crate::invalid_arguments(&tc.arguments) {
                    return Some(format!(
                        "the arguments of tool call {} ({}) are not JSON: {e}",
                        tc.name, tc.id
                    ));
                }
            }
        }
        None
    }

    /// The whole reply; `request_id` is the stream's `id` for the caller
    /// to replace with the response header when there is one. A tool call
    /// that came with no arguments carries `{}`, which every vendor takes
    /// back; some send nothing at all for a tool without parameters.
    pub fn into_response(self) -> Response {
        let choices: Vec<Choice> = self
            .choices
            .into_values()
            .map(|c| {
                let mut msg = Message::new(c.role.unwrap_or(Role::Assistant));
                msg.content = c.content;
                msg.tool_calls = c
                    .tool_calls
                    .into_values()
                    .map(|tc| ToolCall {
                        id: tc.id,
                        name: tc.name,
                        arguments: if tc.arguments.trim().is_empty() {
                            EMPTY_ARGUMENTS.to_string()
                        } else {
                            tc.arguments
                        },
                    })
                    .collect();
                Choice {
                    finish_reason: c.finish_reason,
                    message: msg,
                }
            })
            .collect();
        let n_choices = choices.len() as u32;
        let (message, finish_reason) = crate::merge_choices(choices);
        Response {
            message: Some(message),
            finish_reason,
            choices: n_choices,
            usage: self.usage,
            model: self.model,
            request_id: self.id,
        }
    }
}

/// Reads a streamed chat-completion body to the end: every text piece goes
/// to `on_text` as it arrives and the folded reply comes back. A stream
/// that ends before the reply is whole ([`ChatChunks::incomplete`]) is
/// not a reply: before any text was shown it is `stream_incomplete`,
/// retryable, so the provider's retry loop asks again and the caller never
/// sees the partial message; after text was shown it is
/// `stream_interrupted`, not retryable (the caller would show the text
/// twice), the same as a transport failure at that point.
pub fn stream_chat_completion(
    provider: &str,
    cancel: &CancelToken,
    body: impl Read,
    max_bytes: usize,
    on_text: &mut dyn FnMut(&str),
) -> Result<Response> {
    let mut chunks = ChatChunks::default();
    let mut shown = false;
    let read = read_event_stream(provider, cancel, body, max_bytes, &mut |data| {
        let chunk: Value = serde_json::from_str(data).map_err(|e| decode_error(provider, e))?;
        let text = chunks.push(provider, &chunk)?;
        if !text.is_empty() {
            shown = true;
            on_text(&text);
        }
        Ok(())
    });
    let done = match read {
        Ok(done) => done,
        Err(err) if shown && crate::is_retryable(&err) => {
            return Err(ProviderError::new(
                provider,
                0,
                "stream_interrupted",
                format!("the reply stopped part way: {err}"),
                false,
            )
            .into());
        }
        Err(err) => return Err(err),
    };
    if let Some(missing) = chunks.incomplete(done) {
        return Err(if shown {
            ProviderError::new(
                provider,
                0,
                "stream_interrupted",
                format!("the reply stopped part way: {missing}"),
                false,
            )
        } else {
            ProviderError::new(
                provider,
                0,
                "stream_incomplete",
                format!("the reply ended before it was whole: {missing}"),
                true,
            )
        }
        .into());
    }
    if chunks.chunks == 0 {
        return Err(decode_error(provider, "the event stream carried no chunks").into());
    }
    Ok(chunks.into_response())
}

/// Whether a vendor error says it does not stream (or does not know
/// `stream_options`): a client-side status whose message names the field.
pub fn is_stream_unsupported(err: &anyhow::Error) -> bool {
    crate::provider_error(err).is_some_and(|e| {
        matches!(e.status, 400 | 404 | 405 | 415 | 422)
            && (e.message.to_ascii_lowercase().contains("stream")
                || e.code.to_ascii_lowercase().contains("stream"))
    })
}

/// How a provider streams: with the usage chunk asked for, plainly, or
/// not at all. A provider starts at [`StreamMode::WithUsage`] and steps
/// down as the endpoint complains ([`stream_mode_after`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StreamMode {
    #[default]
    WithUsage,
    Plain,
    Off,
}

/// The mode to try next after `err` came back for a streamed call in
/// `current`: a complaint naming `stream_options` drops only that; any
/// other complaint about streaming turns it off. None when the error is
/// not about streaming (or streaming is already off), so the caller
/// reports it.
pub fn stream_mode_after(err: &anyhow::Error, current: StreamMode) -> Option<StreamMode> {
    if current == StreamMode::Off || !is_stream_unsupported(err) {
        return None;
    }
    let about_options = crate::provider_error(err).is_some_and(|e| {
        e.message.to_ascii_lowercase().contains("stream_options")
            || e.code.to_ascii_lowercase().contains("stream_options")
    });
    Some(match (current, about_options) {
        (StreamMode::WithUsage, true) => StreamMode::Plain,
        _ => StreamMode::Off,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn events(lines: &[&str]) -> std::io::Cursor<String> {
        std::io::Cursor::new(lines.join("\n") + "\n")
    }

    #[test]
    fn event_stream_joins_data_lines_skips_comments_and_stops_at_done() {
        let body = events(&[
            ": keep-alive",
            "event: message",
            "data: {\"a\":1}",
            "",
            "data: line one",
            "data:line two",
            "",
            "data: [DONE]",
            "",
            "data: after done",
            "",
        ]);
        let mut seen = Vec::new();
        read_event_stream("v", &CancelToken::new(), body, 1 << 20, &mut |d| {
            seen.push(d.to_string());
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, vec!["{\"a\":1}", "line one\nline two"]);
    }

    #[test]
    fn event_stream_flushes_the_last_event_without_a_blank_line() {
        let body = std::io::Cursor::new("data: tail\r\n".to_string());
        let mut seen = Vec::new();
        read_event_stream("v", &CancelToken::new(), body, 1 << 20, &mut |d| {
            seen.push(d.to_string());
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, vec!["tail"]);
    }

    #[test]
    fn event_stream_honours_the_cap_and_cancellation() {
        let body = events(&["data: 0123456789", "", "data: 0123456789", ""]);
        let err =
            read_event_stream("v", &CancelToken::new(), body, 20, &mut |_| Ok(())).unwrap_err();
        assert_eq!(crate::provider_error(&err).unwrap().code, "body_too_large");
        let cancel = CancelToken::new();
        cancel.cancel();
        let body = events(&["data: x", ""]);
        assert!(read_event_stream("v", &cancel, body, 1 << 20, &mut |_| Ok(())).is_err());
    }

    #[test]
    fn chunks_fold_text_tool_calls_usage_and_finish_reason() {
        let mut shown = String::new();
        let body = events(&[
            &format!(
                "data: {}",
                json!({"id":"chatcmpl-1","model":"m","choices":[{"index":0,"delta":{"role":"assistant","content":"Hel"},"finish_reason":null}]})
            ),
            "",
            &format!(
                "data: {}",
                json!({"choices":[{"index":0,"delta":{"content":"lo"},"finish_reason":null}]})
            ),
            "",
            &format!(
                "data: {}",
                json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":"lookup","arguments":"{\"id\""}}]},"finish_reason":null}]})
            ),
            "",
            &format!(
                "data: {}",
                json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":":1}"}}]},"finish_reason":"tool_calls"}]})
            ),
            "",
            &format!(
                "data: {}",
                json!({"choices":[],"usage":{"prompt_tokens":7,"completion_tokens":4}})
            ),
            "",
            "data: [DONE]",
            "",
        ]);
        let resp = stream_chat_completion("v", &CancelToken::new(), body, 1 << 20, &mut |t| {
            shown.push_str(t);
        })
        .unwrap();
        assert_eq!(shown, "Hello");
        let msg = resp.message.unwrap();
        assert_eq!(msg.content, "Hello");
        assert_eq!(msg.tool_calls.len(), 1);
        assert_eq!(msg.tool_calls[0].id, "call-1");
        assert_eq!(msg.tool_calls[0].name, "lookup");
        assert_eq!(msg.tool_calls[0].arguments, "{\"id\":1}");
        assert_eq!(resp.finish_reason, "tool_calls");
        assert_eq!(resp.usage.prompt_tokens, 7);
        assert_eq!(resp.usage.completion_tokens, 4);
        assert_eq!(resp.model, "m");
        assert_eq!(resp.request_id, "chatcmpl-1");
        assert_eq!(resp.choices, 1);
    }

    #[test]
    fn chunks_split_across_choices_merge_like_a_whole_reply() {
        let body = events(&[
            &format!(
                "data: {}",
                json!({"choices":[{"index":0,"delta":{"content":"Lint passes."},"finish_reason":"stop"}]})
            ),
            "",
            &format!(
                "data: {}",
                json!({"choices":[{"index":1,"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"bash","arguments":"{}"}}]},"finish_reason":"tool_calls"}]})
            ),
            "",
        ]);
        let resp =
            stream_chat_completion("v", &CancelToken::new(), body, 1 << 20, &mut |_| {}).unwrap();
        assert_eq!(resp.choices, 2);
        assert_eq!(resp.finish_reason, "tool_calls");
        let msg = resp.message.unwrap();
        assert_eq!(msg.content, "Lint passes.");
        assert_eq!(msg.tool_calls[0].name, "bash");
    }

    #[test]
    fn an_error_event_is_the_vendors_error_and_an_empty_stream_is_a_decode_error() {
        let body = events(&[
            &format!(
                "data: {}",
                json!({"error":{"message":"model overloaded","type":"server_error"}})
            ),
            "",
        ]);
        let err = stream_chat_completion("v", &CancelToken::new(), body, 1 << 20, &mut |_| {})
            .unwrap_err();
        let perr = crate::provider_error(&err).unwrap();
        assert_eq!(perr.code, "server_error");
        assert!(perr.message.contains("model overloaded"), "{perr}");
        let body = events(&["data: [DONE]", ""]);
        let err = stream_chat_completion("v", &CancelToken::new(), body, 1 << 20, &mut |_| {})
            .unwrap_err();
        assert_eq!(crate::provider_error(&err).unwrap().code, "decode");
    }

    #[test]
    fn a_transport_failure_after_text_was_shown_is_not_retried() {
        struct Breaks(std::io::Cursor<String>);
        impl Read for Breaks {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = self.0.read(buf)?;
                if n == 0 {
                    return Err(std::io::Error::other("connection reset"));
                }
                Ok(n)
            }
        }
        let text = format!(
            "data: {}\n\n",
            json!({"choices":[{"index":0,"delta":{"content":"half"}}]})
        );
        let mut shown = String::new();
        let err = stream_chat_completion(
            "v",
            &CancelToken::new(),
            Breaks(std::io::Cursor::new(text)),
            1 << 20,
            &mut |t| shown.push_str(t),
        )
        .unwrap_err();
        assert_eq!(shown, "half");
        let perr = crate::provider_error(&err).unwrap();
        assert_eq!(perr.code, "stream_interrupted");
        assert!(!perr.retryable);
        // Before any text, the same failure is an ordinary network error.
        let err = stream_chat_completion(
            "v",
            &CancelToken::new(),
            Breaks(std::io::Cursor::new(String::new())),
            1 << 20,
            &mut |_| {},
        )
        .unwrap_err();
        assert!(crate::is_retryable(&err));
    }

    /// The user's case: the Copilot gateway closed the body between two
    /// chunks of a tool call, before any `finish_reason` and `[DONE]`.
    const CUT_ARGUMENTS: &str = r#"{"path": "AGENTS.md", "replace": "  with the GitHub noreply address.\n\n## Rollback Planning""#;

    /// The second case the same day, in another conversation: a `bash`
    /// call cut one byte short, the closing brace alone missing. A scan of
    /// the 847 trajectories since Oct 29 found exactly these two: a rare
    /// gateway cut, not a parser quirk.
    const CUT_SHORT_ARGUMENTS: &str = r#"{"command": "pulse manifest sync --project leben-in-deutschland 2>&1 | tail -6; echo ---; pulse skill list --project leben-in-deutschland 2>&1 | grep -i \"scraper\"""#;

    fn cut_tool_call_chunks() -> Vec<String> {
        vec![
            json!({"id":"chatcmpl-cut","model":"m","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"toolu_01D9XWUQtuVBTWixzvum3zYd","type":"function","function":{"name":"edit","arguments":""}}]},"finish_reason":null}]}).to_string(),
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":CUT_ARGUMENTS}}]},"finish_reason":null}]}).to_string(),
        ]
    }

    fn stream_of(chunks: &[String], done: bool) -> std::io::Cursor<String> {
        let mut text = String::new();
        for c in chunks {
            text.push_str(&format!("data: {c}\n\n"));
        }
        if done {
            text.push_str("data: [DONE]\n\n");
        }
        std::io::Cursor::new(text)
    }

    #[test]
    fn a_stream_that_ends_before_its_finish_reason_is_asked_for_again() {
        // Nothing was shown (a tool-call-only reply), so the partial
        // message is not a reply at all: a retryable error, named, that
        // the provider's retry loop turns into a second request.
        let mut shown = String::new();
        let err = stream_chat_completion(
            "copilot",
            &CancelToken::new(),
            stream_of(&cut_tool_call_chunks(), false),
            1 << 20,
            &mut |t| shown.push_str(t),
        )
        .unwrap_err();
        assert!(shown.is_empty());
        let perr = crate::provider_error(&err).unwrap();
        assert_eq!(perr.code, "stream_incomplete");
        assert!(perr.retryable);
        assert!(crate::is_retryable(&err));
        assert_eq!(
            perr.message,
            "the reply ended before it was whole: no finish reason arrived"
        );
        // An empty body is the same failure, one chunk earlier.
        let err = stream_chat_completion(
            "copilot",
            &CancelToken::new(),
            stream_of(&[], false),
            1 << 20,
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(
            crate::provider_error(&err).unwrap().code,
            "stream_incomplete"
        );
        // The other conversation's cut: the whole call but its last byte,
        // in one chunk, then the body ends.
        let short = [
            json!({"id":"chatcmpl-short","model":"m","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"toolu_01PToC5gLgbC4XyNrtnHnH13","type":"function","function":{"name":"bash","arguments":CUT_SHORT_ARGUMENTS}}]},"finish_reason":null}]}).to_string(),
        ];
        let err = stream_chat_completion(
            "copilot",
            &CancelToken::new(),
            stream_of(&short, false),
            1 << 20,
            &mut |_| {},
        )
        .unwrap_err();
        let perr = crate::provider_error(&err).unwrap();
        assert_eq!(perr.code, "stream_incomplete");
        assert!(perr.retryable);
        assert_eq!(CUT_SHORT_ARGUMENTS.len(), 165);
        assert_eq!(
            crate::invalid_arguments(CUT_SHORT_ARGUMENTS).as_deref(),
            Some("EOF while parsing an object at line 1 column 165")
        );

        // After text was shown the same cut is `stream_interrupted`, not
        // retryable here: the caller decides, as for a transport failure.
        let mut chunks = vec![
            json!({"choices":[{"index":0,"delta":{"role":"assistant","content":"Editing AGENTS.md"}}]}).to_string(),
        ];
        chunks.extend(cut_tool_call_chunks());
        let mut shown = String::new();
        let err = stream_chat_completion(
            "copilot",
            &CancelToken::new(),
            stream_of(&chunks, false),
            1 << 20,
            &mut |t| shown.push_str(t),
        )
        .unwrap_err();
        assert_eq!(shown, "Editing AGENTS.md");
        let perr = crate::provider_error(&err).unwrap();
        assert_eq!(perr.code, "stream_interrupted");
        assert!(!perr.retryable);
        assert!(perr.message.contains("no finish reason arrived"), "{perr}");
    }

    #[test]
    fn a_tool_call_whose_arguments_are_not_json_is_an_incomplete_reply() {
        // A finish reason and [DONE] arrived, yet the arguments do not
        // parse: the chunks that held their end never came. The error
        // names the call and serde's reason.
        let mut chunks = cut_tool_call_chunks();
        chunks.push(
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}).to_string(),
        );
        let err = stream_chat_completion(
            "copilot",
            &CancelToken::new(),
            stream_of(&chunks, true),
            1 << 20,
            &mut |_| {},
        )
        .unwrap_err();
        let perr = crate::provider_error(&err).unwrap();
        assert_eq!(perr.code, "stream_incomplete");
        assert!(perr.retryable);
        assert_eq!(
            perr.message,
            "the reply ended before it was whole: the arguments of tool call edit (toolu_01D9XWUQtuVBTWixzvum3zYd) are not JSON: EOF while parsing an object at line 1 column 93"
        );

        // The model's own token limit cutting the arguments is not a
        // broken stream: the reply goes through as it is and the harness
        // tells the model what arrived.
        let mut chunks = cut_tool_call_chunks();
        chunks
            .push(json!({"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}).to_string());
        let resp = stream_chat_completion(
            "copilot",
            &CancelToken::new(),
            stream_of(&chunks, true),
            1 << 20,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(resp.finish_reason, "length");
        assert_eq!(resp.message.unwrap().tool_calls[0].arguments, CUT_ARGUMENTS);

        // A finish reason without [DONE] is a whole reply (as the
        // split-choices case above), and so is [DONE] without a finish
        // reason when the arguments parse.
        let chunks = [
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"bash","arguments":"{\"command\":\"true\"}"}}]},"finish_reason":null}]}).to_string(),
        ];
        let resp = stream_chat_completion(
            "copilot",
            &CancelToken::new(),
            stream_of(&chunks, true),
            1 << 20,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(
            resp.message.unwrap().tool_calls[0].arguments,
            "{\"command\":\"true\"}"
        );
    }

    #[test]
    fn a_tool_call_with_no_arguments_folds_to_an_empty_object() {
        let chunks = [
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"list"}}]},"finish_reason":null}]}).to_string(),
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":""}}]},"finish_reason":"tool_calls"}]}).to_string(),
        ];
        let resp = stream_chat_completion(
            "v",
            &CancelToken::new(),
            stream_of(&chunks, true),
            1 << 20,
            &mut |_| {},
        )
        .unwrap();
        let msg = resp.message.unwrap();
        assert_eq!(msg.tool_calls[0].name, "list");
        assert_eq!(msg.tool_calls[0].arguments, EMPTY_ARGUMENTS);
    }

    #[test]
    fn event_stream_says_whether_done_ended_it() {
        let body = events(&["data: {}", "", "data: [DONE]", ""]);
        assert!(
            read_event_stream("v", &CancelToken::new(), body, 1 << 20, &mut |_| Ok(())).unwrap()
        );
        let body = events(&["data: {}", ""]);
        assert!(
            !read_event_stream("v", &CancelToken::new(), body, 1 << 20, &mut |_| Ok(())).unwrap()
        );
    }

    #[test]
    fn stream_mode_steps_down_as_the_endpoint_complains() {
        let options: anyhow::Error = ProviderError::new(
            "v",
            400,
            "invalid_request_error",
            "Unrecognized request argument: stream_options",
            false,
        )
        .into();
        let streaming: anyhow::Error = ProviderError::new(
            "v",
            400,
            "invalid_request_error",
            "streaming is not supported for this model",
            false,
        )
        .into();
        let other: anyhow::Error =
            ProviderError::new("v", 400, "bad", "model not found", false).into();
        assert_eq!(
            stream_mode_after(&options, StreamMode::WithUsage),
            Some(StreamMode::Plain)
        );
        assert_eq!(
            stream_mode_after(&options, StreamMode::Plain),
            Some(StreamMode::Off)
        );
        assert_eq!(
            stream_mode_after(&streaming, StreamMode::WithUsage),
            Some(StreamMode::Off)
        );
        assert_eq!(stream_mode_after(&other, StreamMode::WithUsage), None);
        assert_eq!(stream_mode_after(&streaming, StreamMode::Off), None);
    }

    #[test]
    fn stream_unsupported_reads_the_vendors_complaint() {
        let err: anyhow::Error = ProviderError::new(
            "v",
            400,
            "invalid_request_error",
            "stream_options is not supported",
            false,
        )
        .into();
        assert!(is_stream_unsupported(&err));
        let err: anyhow::Error =
            ProviderError::new("v", 400, "bad", "model not found", false).into();
        assert!(!is_stream_unsupported(&err));
        let err: anyhow::Error = ProviderError::new("v", 500, "", "stream broke", true).into();
        assert!(!is_stream_unsupported(&err));
    }
}
