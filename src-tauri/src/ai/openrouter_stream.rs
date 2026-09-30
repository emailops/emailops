//! Pure parsing for OpenRouter's streamed chat completions: the SSE line
//! splitter, the per-line event parser, the accumulator that turns deltas
//! into one assistant message, and the wire form of a conversation. No I/O;
//! `openrouter.rs` owns the HTTP side.

use serde::{Deserialize, Serialize};

use crate::ai::openrouter::UsageInfo;
use crate::ai::provider::{AiMessage, AiToolCall, AiToolCallFunction};
use crate::models::error::{AppError, Result};

// ── Request side ─────────────────────────────────────────────────────────────

/// One message of a chat-completions request.
#[derive(Debug, Serialize, PartialEq)]
pub(super) struct WireMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<WireToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl WireMessage {
    pub fn text(role: &str, content: &str) -> Self {
        Self {
            role: role.to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_call_id: None,
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
pub(super) struct WireToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: WireFunction,
}

#[derive(Debug, Serialize, PartialEq)]
pub(super) struct WireFunction {
    pub name: String,
    /// JSON-encoded arguments: the chat-completions format carries them as a
    /// string, not an object.
    pub arguments: String,
}

/// The conversation in chat-completions form.
///
/// `AiMessage` carries no tool-call ids, and the format requires each `tool`
/// result to name the call it answers. Ids are therefore assigned here, by
/// position (`call_<message>_<call>`), and each `tool` message takes the
/// oldest unanswered one. A `tool` message with no call left to answer is
/// sent as a `user` message, which every model accepts.
pub(super) fn wire_messages(messages: &[AiMessage]) -> Vec<WireMessage> {
    // Ids of the latest assistant turn's calls that no result has answered.
    let mut unanswered: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    messages
        .iter()
        .enumerate()
        .map(|(position, message)| {
            if message.role == "tool" {
                return match unanswered.pop_front() {
                    Some(id) => WireMessage {
                        tool_call_id: Some(id),
                        ..WireMessage::text("tool", &message.content)
                    },
                    None => WireMessage::text("user", &message.content),
                };
            }
            if message.role == "assistant" {
                unanswered.clear();
            }
            let calls: Vec<WireToolCall> = message
                .tool_calls
                .iter()
                .flatten()
                .enumerate()
                .map(|(n, call)| WireToolCall {
                    id: format!("call_{position}_{n}"),
                    kind: "function",
                    function: WireFunction {
                        name: call.function.name.clone(),
                        arguments: call.function.arguments.to_string(),
                    },
                })
                .collect();
            unanswered.extend(calls.iter().map(|call| call.id.clone()));
            WireMessage {
                tool_calls: (!calls.is_empty()).then_some(calls),
                ..WireMessage::text(&message.role, &message.content)
            }
        })
        .collect()
}

// ── Response side ────────────────────────────────────────────────────────────

/// Splits an SSE byte stream into lines. Network chunks can end mid-line (and
/// mid-character), so bytes are only decoded once a whole line has arrived.
#[derive(Debug, Default)]
pub(super) struct SseLines {
    buf: Vec<u8>,
}

impl SseLines {
    /// Add `bytes`; return every line they complete, without its line ending.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(bytes);
        let mut lines = Vec::new();
        while let Some(idx) = self.buf.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=idx).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            lines.push(line);
        }
        lines
    }

    /// The unterminated last line, if the stream ended in one.
    pub fn finish(&mut self) -> Option<Vec<u8>> {
        let rest = std::mem::take(&mut self.buf);
        (!rest.iter().all(u8::is_ascii_whitespace)).then_some(rest)
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<UsageInfo>,
    #[serde(default)]
    error: Option<StreamError>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: Delta,
    #[serde(default)]
    finish_reason: Option<String>,
}

/// The part of a delta EmailOps reads. Reasoning (`reasoning`,
/// `reasoning_details`) is deliberately not a field: it is never shown and
/// never added to the answer.
#[derive(Debug, Default, Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct ToolCallDelta {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    function: Option<FunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct FunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamError {
    #[serde(default)]
    message: String,
    /// A number or a string, depending on the upstream provider.
    #[serde(default)]
    code: serde_json::Value,
}

#[derive(Debug)]
pub(super) enum SseEvent {
    Chunk(StreamChunk),
    /// `data: [DONE]`: the reply is complete.
    Done,
}

/// Parse one SSE line. `Ok(None)` for anything that carries no event: a blank
/// line, a `:` comment (OpenRouter's keep-alive), a non-`data` field, or a
/// malformed payload (logged, the stream goes on). `Err` for an `error` object,
/// which OpenRouter sends mid-stream with HTTP 200 already committed.
pub(super) fn parse_sse_line(line: &[u8]) -> Result<Option<SseEvent>> {
    let text = String::from_utf8_lossy(line);
    let Some(payload) = text.strip_prefix("data:").map(str::trim) else {
        return Ok(None);
    };
    if payload.is_empty() {
        return Ok(None);
    }
    if payload == "[DONE]" {
        return Ok(Some(SseEvent::Done));
    }
    let chunk: StreamChunk = match serde_json::from_str(payload) {
        Ok(chunk) => chunk,
        Err(e) => {
            // Unusual but not necessarily fatal: a stream that loses its end
            // this way is still caught as cut off.
            crate::services::logger::log(
                "debug",
                "ai",
                format!(
                    "openrouter stream: skipping malformed chunk ({} bytes, err: {e})",
                    payload.len()
                ),
            );
            return Ok(None);
        }
    };
    if let Some(error) = &chunk.error {
        return Err(stream_error(error));
    }
    Ok(Some(SseEvent::Chunk(chunk)))
}

fn stream_error(error: &StreamError) -> AppError {
    let code = match &error.code {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(code) => format!(" ({code})"),
        other => format!(" ({other})"),
    };
    AppError::AiError(format!("OpenRouter stream error{code}: {}", error.message))
}

/// A tool call being assembled from its fragments.
#[derive(Debug, Default)]
struct PartialToolCall {
    name: String,
    arguments: String,
}

/// What a finished stream amounts to.
#[derive(Debug)]
pub(super) struct StreamOutcome {
    pub content: String,
    pub tool_calls: Vec<AiToolCall>,
    pub usage: Option<UsageInfo>,
}

/// Builds one assistant message out of a stream's chunks.
#[derive(Debug, Default)]
pub(super) struct StreamAccumulator {
    content: String,
    calls: Vec<PartialToolCall>,
    usage: Option<UsageInfo>,
    finish_reason: Option<String>,
}

impl StreamAccumulator {
    /// Take in one chunk; return the prose it added, if any, for the caller to
    /// forward to the user.
    pub fn apply(&mut self, chunk: StreamChunk) -> Option<String> {
        if chunk.usage.is_some() {
            self.usage = chunk.usage;
        }
        let mut prose = String::new();
        for choice in chunk.choices {
            if choice.finish_reason.is_some() {
                self.finish_reason = choice.finish_reason;
            }
            if let Some(content) = choice.delta.content {
                prose.push_str(&content);
            }
            for fragment in choice.delta.tool_calls.into_iter().flatten() {
                self.apply_tool_fragment(fragment);
            }
        }
        self.content.push_str(&prose);
        (!prose.is_empty()).then_some(prose)
    }

    fn apply_tool_fragment(&mut self, fragment: ToolCallDelta) {
        let function = fragment.function.unwrap_or(FunctionDelta {
            name: None,
            arguments: None,
        });
        let name = function.name.filter(|name| !name.is_empty());
        // Without an index, a fragment that names a function starts a call and
        // any other continues the latest one.
        let slot = fragment.index.unwrap_or_else(|| {
            let starts_a_call = name.is_some() || self.calls.is_empty();
            if starts_a_call {
                self.calls.len()
            } else {
                self.calls.len() - 1
            }
        });
        if self.calls.len() <= slot {
            self.calls.resize_with(slot + 1, PartialToolCall::default);
        }
        let call = &mut self.calls[slot];
        if let Some(name) = name {
            call.name = name;
        }
        if let Some(arguments) = function.arguments {
            call.arguments.push_str(&arguments);
        }
    }

    /// Whether the model said it had finished (a `finish_reason` arrived).
    pub fn finished(&self) -> bool {
        self.finish_reason.is_some()
    }

    /// The prose so far and nothing else: what a cancelled reply keeps.
    pub fn into_partial(self) -> StreamOutcome {
        StreamOutcome {
            content: self.content,
            tool_calls: Vec::new(),
            usage: self.usage,
        }
    }

    /// The complete reply. `Err` when a tool call's arguments are not valid
    /// JSON (typically a reply cut off at the output limit).
    pub fn finish(self) -> Result<StreamOutcome> {
        let tool_calls = self
            .calls
            .into_iter()
            .map(|call| {
                if call.name.is_empty() {
                    return Err(AppError::AiError(
                        "OpenRouter returned a tool call without a function name".to_string(),
                    ));
                }
                let arguments = if call.arguments.trim().is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_str(&call.arguments).map_err(|e| {
                        AppError::AiError(format!(
                            "OpenRouter returned malformed arguments for the {} tool call: {e}",
                            call.name
                        ))
                    })?
                };
                Ok(AiToolCall {
                    function: AiToolCallFunction {
                        name: call.name,
                        arguments,
                    },
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(StreamOutcome {
            content: self.content,
            tool_calls,
            usage: self.usage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lines_of(chunks: &[&[u8]]) -> Vec<String> {
        let mut lines = SseLines::default();
        let mut out = Vec::new();
        for chunk in chunks {
            out.extend(lines.push(chunk));
        }
        out.extend(lines.finish());
        out.into_iter().map(|l| String::from_utf8(l).unwrap()).collect()
    }

    fn chunk(json: &str) -> StreamChunk {
        match parse_sse_line(format!("data: {json}").as_bytes()) {
            Ok(Some(SseEvent::Chunk(c))) => c,
            other => panic!("expected a chunk, got {other:?}"),
        }
    }

    /// Feed `chunks` to a fresh accumulator; return it with the prose deltas.
    fn accumulate(chunks: &[&str]) -> (StreamAccumulator, Vec<String>) {
        let mut acc = StreamAccumulator::default();
        let deltas = chunks.iter().filter_map(|c| acc.apply(chunk(c))).collect();
        (acc, deltas)
    }

    // ── SseLines ────────────────────────────────────────────────────────────

    #[test]
    fn a_line_split_across_chunks_is_joined() {
        assert_eq!(
            lines_of(&[b"data: {\"a\"", b":1}\n\ndata: [DONE]\n\n"]),
            vec!["data: {\"a\":1}", "", "data: [DONE]", ""]
        );
    }

    #[test]
    fn a_multibyte_character_split_across_chunks_survives() {
        let line = "data: {\"t\":\"año €\"}\n".as_bytes();
        // Cut inside the two-byte `ñ` and inside the three-byte `€`.
        let cut_a = line.iter().position(|b| *b == 0xC3).unwrap() + 1;
        let cut_b = line.iter().position(|b| *b == 0xE2).unwrap() + 2;
        assert_eq!(
            lines_of(&[&line[..cut_a], &line[cut_a..cut_b], &line[cut_b..]]),
            vec!["data: {\"t\":\"año €\"}"]
        );
    }

    #[test]
    fn crlf_line_endings_are_stripped() {
        assert_eq!(
            lines_of(&[b"data: 1\r\n\r\ndata: 2\r\n"]),
            vec!["data: 1", "", "data: 2"]
        );
    }

    #[test]
    fn a_final_line_without_a_newline_is_still_a_line() {
        assert_eq!(lines_of(&[b"data: 1\ndata: [DONE]"]), vec!["data: 1", "data: [DONE]"]);
    }

    // ── parse_sse_line ──────────────────────────────────────────────────────

    #[test]
    fn lines_without_an_event_are_skipped() {
        for line in [
            "",
            "   ",
            ": OPENROUTER PROCESSING",
            ":",
            "event: message",
            "id: 7",
            "retry: 3000",
            "data: {not json",
            "data:",
        ] {
            assert!(
                matches!(parse_sse_line(line.as_bytes()), Ok(None)),
                "{line:?} must be skipped"
            );
        }
    }

    #[test]
    fn done_is_recognised_with_or_without_the_space() {
        assert!(matches!(parse_sse_line(b"data: [DONE]"), Ok(Some(SseEvent::Done))));
        assert!(matches!(parse_sse_line(b"data:[DONE]"), Ok(Some(SseEvent::Done))));
    }

    #[test]
    fn a_mid_stream_error_object_is_an_error() {
        let line = br#"data: {"id":"gen-1","error":{"code":"server_error","message":"Provider disconnected unexpectedly"},"choices":[{"index":0,"delta":{"content":""},"finish_reason":"error"}]}"#;
        let err = parse_sse_line(line).unwrap_err();
        assert!(
            matches!(&err, AppError::AiError(m) if m.contains("Provider disconnected unexpectedly") && m.contains("server_error")),
            "{err}"
        );
    }

    #[test]
    fn a_numeric_error_code_is_reported_too() {
        let err = parse_sse_line(br#"data: {"error":{"code":429,"message":"Rate limited"}}"#).unwrap_err();
        assert!(
            matches!(&err, AppError::AiError(m) if m.contains("Rate limited") && m.contains("429")),
            "{err}"
        );
    }

    // ── StreamAccumulator ───────────────────────────────────────────────────

    #[test]
    fn content_deltas_are_forwarded_and_joined() {
        let (acc, deltas) = accumulate(&[
            r#"{"choices":[{"delta":{"role":"assistant","content":"Hel"}}]}"#,
            r#"{"choices":[{"delta":{"content":"lo"}}]}"#,
            r#"{"choices":[{"delta":{"content":""},"finish_reason":"stop"}]}"#,
        ]);
        assert_eq!(deltas, vec!["Hel", "lo"]);
        assert!(acc.finished());
        let outcome = acc.finish().unwrap();
        assert_eq!(outcome.content, "Hello");
        assert!(outcome.tool_calls.is_empty());
    }

    #[test]
    fn reasoning_never_reaches_the_answer() {
        let (acc, deltas) = accumulate(&[
            r#"{"choices":[{"delta":{"content":null,"reasoning":"Let me think about the invoice"}}]}"#,
            r#"{"choices":[{"delta":{"reasoning_details":[{"type":"reasoning.text","text":"step by step"}]}}]}"#,
            r#"{"choices":[{"delta":{"reasoning_content":"more thinking"}}]}"#,
            r#"{"choices":[{"delta":{"content":"Paid."},"finish_reason":"stop"}]}"#,
        ]);
        assert_eq!(deltas, vec!["Paid."]);
        assert_eq!(acc.finish().unwrap().content, "Paid.");
    }

    #[test]
    fn a_tool_call_is_assembled_from_its_fragments() {
        let (acc, deltas) = accumulate(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"search_emails","arguments":""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"query\":"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"invoice\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ]);
        assert!(deltas.is_empty(), "a tool-call turn streams no prose");
        let outcome = acc.finish().unwrap();
        assert_eq!(outcome.tool_calls.len(), 1);
        assert_eq!(outcome.tool_calls[0].function.name, "search_emails");
        assert_eq!(outcome.tool_calls[0].function.arguments, json!({"query": "invoice"}));
    }

    #[test]
    fn interleaved_tool_calls_are_kept_apart_by_index() {
        let (acc, _) = accumulate(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"search_emails","arguments":"{\"query\""}},{"index":1,"id":"b","function":{"name":"get_email_body","arguments":"{\"email_id\""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":":\"e7\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":\"q3\"}"}}]}}]}"#,
        ]);
        let calls = acc.finish().unwrap().tool_calls;
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.name, "search_emails");
        assert_eq!(calls[0].function.arguments, json!({"query": "q3"}));
        assert_eq!(calls[1].function.name, "get_email_body");
        assert_eq!(calls[1].function.arguments, json!({"email_id": "e7"}));
    }

    /// Some upstream providers omit `index` and send each call whole.
    #[test]
    fn a_fragment_without_an_index_starts_a_call_when_it_names_one() {
        let (acc, _) = accumulate(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"name":"search_emails","arguments":"{\"query\":"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"arguments":"\"a\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"function":{"name":"list_contacts","arguments":"{}"}}]}}]}"#,
        ]);
        let calls = acc.finish().unwrap().tool_calls;
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.arguments, json!({"query": "a"}));
        assert_eq!(calls[1].function.name, "list_contacts");
    }

    #[test]
    fn a_call_without_arguments_gets_an_empty_object() {
        let (acc, _) = accumulate(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"list_accounts"}}]}}]}"#,
        ]);
        assert_eq!(acc.finish().unwrap().tool_calls[0].function.arguments, json!({}));
    }

    #[test]
    fn malformed_tool_arguments_are_an_error_naming_the_tool() {
        let (acc, _) = accumulate(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"search_emails","arguments":"{\"query\":\"inv"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"length"}]}"#,
        ]);
        let err = acc.finish().unwrap_err();
        assert!(
            matches!(&err, AppError::AiError(m) if m.contains("search_emails")),
            "{err}"
        );
    }

    #[test]
    fn a_call_that_never_got_a_name_is_an_error() {
        let (acc, _) =
            accumulate(&[r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{}"}}]}}]}"#]);
        assert!(acc.finish().is_err());
    }

    #[test]
    fn usage_is_taken_from_the_chunk_that_carries_it() {
        let (acc, _) = accumulate(&[
            r#"{"choices":[{"delta":{"content":"Hi"},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":194,"completion_tokens":2,"total_tokens":196,"cost":0.0125}}"#,
        ]);
        let usage = acc.finish().unwrap().usage.expect("usage");
        assert_eq!(usage.prompt_tokens, Some(194));
        assert_eq!(usage.completion_tokens, Some(2));
        assert_eq!(usage.cost, Some(0.0125));
    }

    #[test]
    fn a_cancelled_reply_keeps_its_prose_and_no_half_built_call() {
        let (acc, _) = accumulate(&[
            r#"{"choices":[{"delta":{"content":"Looking"}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"search_emails","arguments":"{\"qu"}}]}}]}"#,
        ]);
        assert!(!acc.finished());
        let partial = acc.into_partial();
        assert_eq!(partial.content, "Looking");
        assert!(partial.tool_calls.is_empty());
    }

    // ── wire_messages ───────────────────────────────────────────────────────

    fn msg(role: &str, content: &str) -> AiMessage {
        AiMessage {
            role: role.to_string(),
            content: content.to_string(),
            tool_calls: None,
        }
    }

    fn calling(calls: &[(&str, serde_json::Value)]) -> AiMessage {
        AiMessage {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: Some(
                calls
                    .iter()
                    .map(|(name, arguments)| AiToolCall {
                        function: AiToolCallFunction {
                            name: name.to_string(),
                            arguments: arguments.clone(),
                        },
                    })
                    .collect(),
            ),
        }
    }

    #[test]
    fn plain_messages_go_out_as_role_and_content() {
        let wire = serde_json::to_value(wire_messages(&[msg("system", "Be brief."), msg("user", "Hi")])).unwrap();
        assert_eq!(
            wire,
            json!([{"role": "system", "content": "Be brief."}, {"role": "user", "content": "Hi"}])
        );
    }

    #[test]
    fn tool_results_answer_the_calls_in_order() {
        let wire = serde_json::to_value(wire_messages(&[
            msg("user", "Find the invoice"),
            calling(&[
                ("search_emails", json!({"query": "invoice"})),
                ("list_contacts", json!({})),
            ]),
            msg("tool", "1 email"),
            msg("tool", "2 contacts"),
        ]))
        .unwrap();
        assert_eq!(
            wire,
            json!([
                {"role": "user", "content": "Find the invoice"},
                {"role": "assistant", "content": "", "tool_calls": [
                    {"id": "call_1_0", "type": "function",
                     "function": {"name": "search_emails", "arguments": "{\"query\":\"invoice\"}"}},
                    {"id": "call_1_1", "type": "function",
                     "function": {"name": "list_contacts", "arguments": "{}"}},
                ]},
                {"role": "tool", "content": "1 email", "tool_call_id": "call_1_0"},
                {"role": "tool", "content": "2 contacts", "tool_call_id": "call_1_1"},
            ])
        );
    }

    #[test]
    fn ids_stay_unique_across_rounds() {
        let wire = wire_messages(&[
            calling(&[("search_emails", json!({"query": "a"}))]),
            msg("tool", "r1"),
            calling(&[("search_emails", json!({"query": "b"}))]),
            msg("tool", "r2"),
        ]);
        assert_eq!(wire[1].tool_call_id.as_deref(), Some("call_0_0"));
        assert_eq!(wire[3].tool_call_id.as_deref(), Some("call_2_0"));
    }

    /// The format rejects a `tool` message that answers no call.
    #[test]
    fn a_tool_result_with_no_call_to_answer_is_sent_as_a_user_message() {
        let wire = wire_messages(&[msg("user", "Hi"), msg("tool", "stray result")]);
        assert_eq!(wire[1], WireMessage::text("user", "stray result"));
    }

    /// A later assistant turn closes the round: its results cannot answer an
    /// earlier round's leftover call.
    #[test]
    fn an_unanswered_call_is_not_answered_by_a_later_round() {
        let wire = wire_messages(&[
            calling(&[("search_emails", json!({})), ("list_contacts", json!({}))]),
            msg("tool", "only one result"),
            msg("assistant", "Done."),
            msg("tool", "stray"),
        ]);
        assert_eq!(wire[1].tool_call_id.as_deref(), Some("call_0_0"));
        assert_eq!(wire[3].role, "user");
    }

    /// An assistant message whose tool-call list is empty is a plain message.
    #[test]
    fn an_empty_tool_call_list_is_omitted() {
        let wire = wire_messages(&[AiMessage {
            role: "assistant".to_string(),
            content: "Hello".to_string(),
            tool_calls: Some(Vec::new()),
        }]);
        assert_eq!(wire[0], WireMessage::text("assistant", "Hello"));
    }
}
