//! Turn execution for chat-with-your-emails: prompt assembly, the tool-call
//! loop (including XML-tool-call salvage and direct-tool shortcuts),
//! thread-bound turns, and the top-level `run_chat_turn` orchestrator that
//! streams the answer and persists the assistant message.

use std::sync::Arc;
use std::time::Duration;

use chrono::{Datelike, Utc};
use tokio::time::timeout;

use crate::ai::provider::{AIProvider, AiMessage};
use crate::db::Database;
use crate::models::error::{AppError, Result};
use crate::models::{
    ChatMessage, ChatMessageSource, ChatPhase, ChatRenamedEvent, ChatSourcesEvent, ChatStreamEvent, ChatTrace,
    ChatTraceEvent, LlmCallTrace, RetrievalTrace, RouteDecision, RouteMode, ToolCallTrace,
};
use crate::services::ai::AiService;
use crate::util::html::strip_html_for_fts;

use super::conversations::{derive_title, title_is_default};
use super::retrieval::{
    mark_relevant_region, retrieve_context_with_trace, smart_body_slice, smart_body_slice_indexed, ScoredEmail,
    MAX_SOURCE_BODY_CHARS, TOP_K_SOURCES,
};
use super::routing::classify_route;
use super::tools;
use super::{
    count_invalid_citations, emit_log, emit_phase, format_date, phase_for_tool, strip_invalid_citations,
    strip_tool_call_markup, truncate_chars,
};

/// Max conversation turns (user+assistant combined) kept in the prompt.
const MAX_HISTORY_TURNS: usize = 6;

/// `Utc::now()` routed through the `Clock` seam so eval cases can pin "today"
/// to a specific date via `services::clock::install(FixedClock::new(...))`.
/// Production wiring leaves `SystemClock` installed, so this is identical to
/// `Utc::now()` for the live app. Only the date-shaping call sites that
/// influence what the model sees go through this helper; latency/telemetry
/// timers stay on bare `Utc::now()` / `Instant::now()` so traces remain
/// accurate.
fn now_utc() -> chrono::DateTime<Utc> {
    chrono::DateTime::<Utc>::from_timestamp(crate::services::clock::current().now_secs(), 0).unwrap_or_else(Utc::now)
}

/// Format a message list as readable text for the reasoning panel (and for
/// Phoenix tracing when enabled). Shows each message's role, content, and any
/// tool calls — including tool-result messages that carry search_emails
/// output back to the LLM. Gated to `debug_assertions` because every call
/// site is too (the reasoning panel only renders in dev builds); compiling
/// the body in release would trigger a `dead_code` warning.
#[cfg(debug_assertions)]
fn format_messages_for_trace(messages: &[crate::ai::provider::AiMessage]) -> String {
    // No truncation: the reasoning panel needs the FULL prompt so the
    // developer can copy-paste it verbatim into a llama.cpp / Ollama prompt
    // for debugging. A long system prompt + tool results can run to tens of
    // KB; that's still cheap to serialise into the ChatTrace JSON blob, and
    // capture is dev-only (cfg(debug_assertions)).
    messages
        .iter()
        .map(|m| {
            let tool_call_str = match &m.tool_calls {
                Some(tcs) if !tcs.is_empty() => {
                    let calls: Vec<String> = tcs
                        .iter()
                        .map(|tc| format!("{}({})", tc.function.name, tc.function.arguments))
                        .collect();
                    format!("\ntool_calls: {}", calls.join(", "))
                }
                _ => String::new(),
            };
            format!("[{}] {}{}", m.role, m.content, tool_call_str)
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

/// Format a tool-round response for the reasoning panel (and Phoenix tracing
/// when enabled). Dev-only — see `format_messages_for_trace`.
#[cfg(debug_assertions)]
fn format_response_for_trace(response: &crate::ai::provider::AiMessage) -> String {
    // No truncation — see format_messages_for_trace. Full response capture
    // makes the reasoning panel copy-paste reproducible.
    let mut parts: Vec<String> = Vec::new();
    if !response.content.is_empty() {
        parts.push(response.content.clone());
    }
    if let Some(tool_calls) = &response.tool_calls {
        for tc in tool_calls {
            parts.push(format!("tool_call: {}({})", tc.function.name, tc.function.arguments));
        }
    }
    if parts.is_empty() {
        "(empty)".to_string()
    } else {
        parts.join("\n")
    }
}

/// Render a `search_emails` tool result, grouped by Gmail category in the
/// priority order Primary → Updates → Other. Users reading a chat answer want
/// direct mail surfaced first; shipping / receipt "Updates" next; and newsletter
/// / social noise last. Callers (the LLM) read this text so we emit explicit
/// "## Primary / ## Updates / ## Other" headers — cheaper and more reliable
/// than asking the model to re-sort by itself.
/// Prefix string for the "tool result ←" log line that exposes the email
/// count for search-shaped results. `format_search_emails_output` emits one
/// `- id=…` line per email row, so we count those. Returns either
/// `"N emails, "` or `""` (empty for tools that don't return email lists).
fn tool_result_count_hint(tool_name: &str, result: &str) -> String {
    match tool_name {
        "search_emails" | "search_contacts" | "get_attachments" => {
            let n = result.matches("\n- ").count();
            if n > 0 {
                format!("{} rows, ", n)
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

/// Assemble `(role, content)` messages to send to the chat backend.
///
/// Layout:
///   system: instructions (per-turn-stable)
///   user:   <older turn>
///   assistant: <older turn>
///   ...
///   user:   sources block + the new question
///
/// `system_template` is the user-editable system-prompt template — typically
/// loaded via `prompts::get_template(db, "chat.system")`. We render it with
/// `{{today}}`, `{{tomorrow}}`, `{{language_instruction}}`. The per-turn
/// dynamic Sources block travels in the FINAL USER MESSAGE, not the system
/// message: the llama.cpp actor reuses the [system + history] KV prefix
/// across turns, and per-turn bytes in the system message would invalidate
/// the whole cache every turn. The block stays in code (not the template)
/// because its formatting (citation numbering, smart-snippet slicing,
/// relevant-region markers) is structurally tied to retrieval and unsafe to
/// expose to the user-editable template.
pub fn build_prompt(
    sources: &[ScoredEmail],
    history: &[ChatMessage],
    user_question: &str,
    language: &str,
    user_email: &str,
    system_template: &str,
    tools_section: &str,
) -> Vec<(String, String)> {
    let today = now_utc().format("%Y-%m-%d").to_string();
    let tomorrow = (now_utc() + chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
    let language_instruction = if language.is_empty() {
        "Reply in the language the user writes in.".to_string()
    } else {
        format!("Reply in {language}.")
    };

    // Self-reference resolution: the model has no innate notion of who "I"/"me"
    // is, so it cannot turn "emails I sent" into a from-filter on its own. Hand
    // it the active account address and tell it how to map first-person
    // sender/recipient references onto search_emails' `from`/`to` arguments.
    // Built here (not hard-coded in the template) so an empty address — no
    // account on the turn — degrades to a blank line instead of leaking a
    // placeholder. Mirrors how `language_instruction` is assembled.
    let user_identity = if user_email.trim().is_empty() {
        String::new()
    } else {
        format!(
            "YOUR USER'S IDENTITY: you are assisting {user_email}. \"I\", \"me\", and \"my\" in the \
question refer to THIS address. Map first-person mail references onto search_emails filters — issue \
the call, never answer from memory:\n  \
- The user is the AUTHOR (\"emails I sent\", \"sent by me\", \"my sent mail\", \"correos que envié\", \
or the equivalent in any language) → search_emails with from={user_email} (NEVER to).\n  \
- The user is the RECIPIENT (\"sent to me\", \"emails I received\", \"in my inbox\", or the \
equivalent) → search_emails with to={user_email} (NEVER from).\n  \
Do not swap from and to: \"I sent\" is always from, \"sent to me\" is always to."
        )
    };

    let mut tpl_vars = std::collections::HashMap::new();
    tpl_vars.insert("today", today);
    tpl_vars.insert("tomorrow", tomorrow);
    tpl_vars.insert("language_instruction", language_instruction);
    tpl_vars.insert("user_identity", user_identity);
    tpl_vars.insert("tools_section", tools_section.to_string());
    let system = crate::services::prompts::render(system_template, &tpl_vars);

    let mut tail = String::with_capacity(user_question.len() + sources.len() * 1200 + 128);
    if sources.is_empty() {
        tail.push_str(
            "Sources: (none pre-retrieved for this turn — you MUST call search_emails \
before answering any factual question about the user's mailbox.)\n",
        );
    } else {
        tail.push_str(&format!("Sources (valid citation range: [1]..[{}]):\n", sources.len()));
        for src in sources {
            let body_text = strip_html_for_fts(&src.body);
            let sliced = smart_body_slice_indexed(&body_text, user_question, MAX_SOURCE_BODY_CHARS);
            let marked = mark_relevant_region(&sliced);
            tail.push_str(&format!(
                "[{}] From: {} <{}>  Subject: {}  Date: {}\n    {}\n\n",
                src.citation_number,
                src.email.sender,
                src.email.sender_email,
                src.email.subject,
                format_date(src.email.timestamp),
                marked
            ));
        }
    }
    tail.push('\n');
    tail.push_str(user_question);

    let mut messages: Vec<(String, String)> = Vec::with_capacity(history.len() + 2);
    messages.push(("system".to_string(), system));

    // Trim history to the last MAX_HISTORY_TURNS turns, preserving order.
    let start = history.len().saturating_sub(MAX_HISTORY_TURNS);
    for msg in &history[start..] {
        if msg.role == "user" || msg.role == "assistant" {
            // User rows replay the exact bytes they were PROMPTED with
            // (memory header + sources + question) — not the raw question —
            // so this turn's prompt purely extends the previous one and the
            // llama.cpp KV prefix survives. See `ChatMessage::prompt_content`.
            let content = match (&msg.prompt_content, msg.role.as_str()) {
                (Some(p), "user") => p.clone(),
                _ => msg.content.clone(),
            };
            messages.push((msg.role.clone(), content));
        }
    }

    messages.push(("user".to_string(), tail));
    messages
}

/// Prepend a per-turn context block (e.g. the memory header) to the final
/// user message. Per-turn content must never land in the system message: the
/// llama.cpp actor reuses the [system + history] KV prefix across turns, and
/// any per-turn byte there invalidates it.
fn prepend_to_final_user_message(messages: &mut [(String, String)], block: &str) {
    if let Some((_, content)) = messages.last_mut() {
        *content = format!("{block}\n\n{content}");
    }
}

/// Persist the final user-message bytes (memory header + sources + question)
/// onto the user row so future turns replay them byte-identically — see
/// `ChatMessage::prompt_content`. Failure degrades to a debug log: a turn
/// must not fail because cache-warmth metadata could not be written.
fn persist_prompted_tail(db: &Database, user_message_id: &str, messages: &[(String, String)]) {
    let Some((role, content)) = messages.last() else {
        return;
    };
    if role != "user" {
        return;
    }
    if let Err(e) = db.update_chat_message_prompt_content(user_message_id, content) {
        emit_log("debug", &format!("prompt_content persist skipped: {e}"));
    }
}

// ── Tool calling ────────────────────────────────────────────────────────────

/// Maximum tool-call round-trips before we force the model to answer.
// Headroom for models that fetch incrementally (Qwen 3.6 35B-A3B emits one
// get_email_body per round and sometimes burns a round on a malformed
// search_emails({}) call). A multi-email tabulation needs ~1 search + N body
// reads + 1 synthesis round; too low a cap forces a tools-free synthesis pass
// that incremental fetchers tend to fill with another <tool_call> (gated to an
// empty reply). The plan_answer closing instruction is the safety net when the
// cap is still hit; this keeps the common case finishing in-loop.
const MAX_TOOL_ROUNDS: usize = 10;

/// Per-tool dispatch result: the text the LLM sees plus the structural
/// allowlists this tool contributed (email ids + draft ids). Callers fold
/// the refs into per-turn accumulators that end up on the assistant
/// `ChatMessage` as `referenced_email_ids` / `referenced_draft_ids`.
struct DispatchedTool {
    text: String,
    email_refs: Vec<String>,
    draft_refs: Vec<String>,
    /// Set when the mangled-address rescue re-ran the search with the
    /// question's verbatim address — callers record THESE args in the trace
    /// so it reflects the call that actually produced the result.
    corrected_args: Option<serde_json::Value>,
}

/// Marker prefix of every empty `search_emails` result — the trigger for the
/// mangled-address rescue in [`dispatch_tool`].
const NO_MATCHING_EMAILS: &str = "No matching emails found";

/// Dispatch one tool call through the registry: look up the tool (honouring
/// feature gating), execute it, emit any `ToolEffect`s as `chat-tool-effect`
/// Tauri events for the frontend to react to, and return the text the LLM
/// will see as the tool-result message along with the email-id allowlist
/// this tool produced.
///
/// `user_question` powers the mangled-address rescue: when a `search_emails`
/// call comes back empty and its from/to looks like a mis-transcription of
/// the ONE address written in the question (see
/// [`correct_mangled_address_args`]), the search is re-run once with the
/// verbatim address and the corrected result is returned, with
/// `corrected_args` set so callers trace what actually ran.
async fn dispatch_tool(
    registry: &tools::ToolRegistry,
    db: &Arc<Database>,
    account_id: &str,
    categories: &[String],
    user_question: &str,
    name: &str,
    args: serde_json::Value,
) -> DispatchedTool {
    match registry.get(name, db.as_ref()) {
        Some(tool) => {
            let ctx = tools::ToolCtx {
                db,
                account_id,
                categories,
            };
            match tool.execute(&ctx, args.clone()).await {
                Ok(mut out) => {
                    let mut corrected_args = None;
                    // Mangled-address rescue: an empty search whose from/to is
                    // a mis-transcription of the question's address gets ONE
                    // retry with the verbatim address. Result text names the
                    // correction so the model's prose uses the right address.
                    if name == "search_emails" && out.text.starts_with(NO_MATCHING_EMAILS) {
                        if let Some((fixed, right, wrong)) = correct_mangled_address_args(&args, user_question) {
                            if let Ok(second) = tool.execute(&ctx, fixed.clone()).await {
                                if !second.text.starts_with(NO_MATCHING_EMAILS) {
                                    emit_log(
                                        "info",
                                        &format!(
                                            "tool_loop: search_emails({wrong}) found nothing — retried with the \
question's verbatim address ({right})"
                                        ),
                                    );
                                    out = tools::ToolOutput {
                                        text: format!(
                                            "(No results for {wrong} — that address does not appear in the user's \
question. Retried with {right}, written verbatim in the question. Use {right} from now on.)\n{}",
                                            second.text
                                        ),
                                        ..second
                                    };
                                    corrected_args = Some(fixed);
                                }
                            }
                        }
                    }
                    // Effects are fire-and-forget through the event seam: a
                    // dropped effect never poisons the tool result — the LLM
                    // still gets its text.
                    for eff in &out.effects {
                        crate::services::events::emit("chat-tool-effect", eff);
                    }
                    DispatchedTool {
                        text: out.text,
                        email_refs: out.email_refs,
                        draft_refs: out.draft_refs,
                        corrected_args,
                    }
                }
                Err(e) => DispatchedTool {
                    text: format!("Tool '{name}' error: {e}"),
                    email_refs: Vec::new(),
                    draft_refs: Vec::new(),
                    corrected_args: None,
                },
            }
        }
        None => {
            // Distinguish unknown vs gated-off so the LLM and the user get a
            // useful hint instead of a flat "unknown tool".
            let text = if registry.lookup(name).is_some() {
                format!("Tool '{name}' is currently disabled in Settings.")
            } else {
                format!("Unknown tool: {name}")
            };
            DispatchedTool {
                text,
                email_refs: Vec::new(),
                draft_refs: Vec::new(),
                corrected_args: None,
            }
        }
    }
}

/// Sync test shim. Internally drives `dispatch_tool` through a current-thread
/// runtime so the existing tool tests (all `#[test]`-flavoured) don't need to
/// be rewritten to `#[tokio::test]`. Production code never calls this — the
/// real chat loop uses `dispatch_tool` directly so it can emit effects via
/// the real `AppHandle`.
#[cfg(test)]
pub(in crate::services::chat) fn execute_tool(
    db: &Arc<Database>,
    account_id: &str,
    categories: &[String],
    name: &str,
    arguments: &serde_json::Value,
) -> String {
    // The tests assume every tool is available, including gated ones, because
    // the old `execute_tool` had no gating. Enable each feature on a fresh
    // test DB so lookups succeed. Production code path goes through real
    // Settings flags via the registry.
    let _ = db.set_preference("memory_enabled", "true");
    let _ = db.set_preference("task_enabled", "true");
    let _ = db.set_preference("lenses_enabled", "true");
    let _ = db.set_preference("ai_drafts_enabled", "true");

    let registry = tools::default_registry();
    let args = arguments.clone();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    // Existing tests only assert on text — drop the refs after dispatch.
    rt.block_on(dispatch_tool(&registry, db, account_id, categories, "", name, args))
        .text
}

/// Salvage XML-style tool calls embedded in plain assistant text.
///
/// Some small instruction-tuned models (Qwen 3.5 4B notably) sometimes
/// emit tool calls as text using this Llama-3 style envelope instead of
/// returning them in the JSON `tool_calls` field the chat-completion API
/// expects:
///
/// ```text
/// <tool_call>
/// <function=get_email_body>
/// <parameter=email_id>
/// 19e6e27f48f95297
/// </parameter>
/// </function>
/// </tool_call>
/// ```
///
/// When that happens the tool loop sees an empty `tool_calls` array and
/// would otherwise treat the XML as the model's final answer. This parser
/// extracts every `<tool_call>` block and converts it into a proper
/// `AiToolCall` so the loop can dispatch it normally.
///
/// Integer-shaped parameter values are promoted to a JSON number so args
/// like `limit=25` still match the tool's schema; everything else stays a
/// string. Returns an empty `Vec` when no `<tool_call>` block is found.
/// Split `text` into the inner bodies of its `<tool_call>` blocks. When the
/// model omits `</tool_call>` (Qwen 3.6 batches calls as newline-separated
/// `<tool_call>{json}` lines with no closing tags), the next `<tool_call>`
/// open tag — or the end of the text — acts as an implicit close. The bool
/// records whether the block was EXPLICITLY closed: Hermes `<function=>`
/// bodies are only trusted when properly closed (a truncated one would
/// half-parse into an args-less call), while JSON bodies self-validate via
/// parsing and accept the implicit close.
fn tool_call_block_bodies(text: &str) -> Vec<(&str, bool)> {
    const OPEN: &str = "<tool_call>";
    const CLOSE: &str = "</tool_call>";
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel_open) = text[cursor..].find(OPEN) {
        let after_open = cursor + rel_open + OPEN.len();
        let rest = &text[after_open..];
        let (end, next_cursor, closed) = match (rest.find(CLOSE), rest.find(OPEN)) {
            (Some(c), Some(o)) if c < o => (after_open + c, after_open + c + CLOSE.len(), true),
            (Some(c), None) => (after_open + c, after_open + c + CLOSE.len(), true),
            (_, Some(o)) => (after_open + o, after_open + o, false),
            (None, None) => (text.len(), text.len(), false),
        };
        out.push((text[after_open..end].trim(), closed));
        cursor = next_cursor;
    }
    out
}

pub(crate) fn parse_xml_tool_calls(text: &str) -> Vec<crate::ai::provider::AiToolCall> {
    use crate::ai::provider::{AiToolCall, AiToolCallFunction};

    const FN_OPEN: &str = "<function=";
    const PARAM_OPEN: &str = "<parameter=";
    const PARAM_CLOSE: &str = "</parameter>";

    let mut out = Vec::new();
    for (block, explicitly_closed) in tool_call_block_bodies(text) {
        // A <tool_call> block carries one of two shapes:
        //   1. Hermes function-XML: <function=NAME><parameter=K>V</parameter>…
        //   2. JSON body: {"name":…,"arguments":…} — Qwen 3.6's shape.
        // With no <function=> sub-element, treat the block as JSON.
        let fn_start = match block.find(FN_OPEN) {
            // A Hermes body without its explicit `</tool_call>` is a truncated
            // stream — drop it rather than half-parse an args-less call.
            Some(_) if !explicitly_closed => continue,
            Some(s) => s + FN_OPEN.len(),
            None => {
                if let Some(call) = parse_json_tool_call_block(block) {
                    out.push(call);
                }
                continue;
            }
        };
        let fn_end = match block[fn_start..].find('>') {
            Some(e) => fn_start + e,
            None => continue,
        };
        let name = block[fn_start..fn_end].trim().to_string();
        if name.is_empty() {
            continue;
        }

        // Walk every <parameter=KEY>VALUE</parameter> inside the function
        // block and accumulate them into a JSON object.
        let mut args = serde_json::Map::new();
        let mut search_from = fn_end + 1;
        while let Some(rel_p) = block[search_from..].find(PARAM_OPEN) {
            let key_start = search_from + rel_p + PARAM_OPEN.len();
            let key_end = match block[key_start..].find('>') {
                Some(e) => key_start + e,
                None => break,
            };
            let key = block[key_start..key_end].trim().to_string();
            let val_start = key_end + 1;
            let val_end = match block[val_start..].find(PARAM_CLOSE) {
                Some(e) => val_start + e,
                None => break,
            };
            let raw_val = block[val_start..val_end].trim();
            let json_val: serde_json::Value = if let Ok(n) = raw_val.parse::<i64>() {
                serde_json::Value::Number(n.into())
            } else if let Ok(f) = raw_val.parse::<f64>() {
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::String(raw_val.to_string()))
            } else {
                serde_json::Value::String(raw_val.to_string())
            };
            if !key.is_empty() {
                args.insert(key, json_val);
            }
            search_from = val_end + PARAM_CLOSE.len();
        }

        out.push(AiToolCall {
            function: AiToolCallFunction {
                name,
                arguments: serde_json::Value::Object(args),
            },
        });
    }
    out
}

/// Parse the FIRST complete JSON value in `s`, tolerating trailing garbage —
/// Qwen 3.6 occasionally emits an extra closing brace after the object
/// (`{"name":…,"limit":25}}`), which a strict `from_str` rejects outright.
fn parse_first_json_value(s: &str) -> Option<serde_json::Value> {
    serde_json::Deserializer::from_str(s)
        .into_iter::<serde_json::Value>()
        .next()?
        .ok()
}

/// Parse the JSON body of a `<tool_call>{…}</tool_call>` block (Qwen 3.6's
/// shape, as opposed to the `<function=>` Hermes form). Mirrors the leniency
/// of the runtime's native Qwen parser: hoists a `name` nested inside
/// `arguments` when there is no top-level one, treats top-level keys beside
/// `name` as the arguments when the `arguments` wrapper was dropped, defaults
/// truly absent arguments to an empty object, and tolerates trailing garbage
/// after the object. Returns `None` when the block is not a well-formed
/// object or carries no resolvable tool name.
fn parse_json_tool_call_block(inner: &str) -> Option<crate::ai::provider::AiToolCall> {
    use crate::ai::provider::{AiToolCall, AiToolCallFunction};

    let value = parse_first_json_value(inner)?;
    let obj = value.as_object()?;
    let mut arguments = obj.get("arguments").cloned().unwrap_or_else(|| {
        // Flattened shape: the model dropped the `arguments` wrapper and put
        // the args at top level next to `name` — collect everything else.
        let flat: serde_json::Map<String, serde_json::Value> = obj
            .iter()
            .filter(|(k, _)| k.as_str() != "name")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        serde_json::Value::Object(flat)
    });
    let name = match obj.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => {
            // Salvage the flattened `{"arguments":{…,"name":"<tool>"}}` shape
            // by hoisting the nested name out of the args we dispatch.
            let args_obj = arguments.as_object_mut()?;
            match args_obj.remove("name") {
                Some(serde_json::Value::String(n)) => n,
                _ => return None,
            }
        }
    };
    Some(AiToolCall {
        function: AiToolCallFunction { name, arguments },
    })
}

/// (tool name, its parameter property keys, its required keys). The shape
/// needed to infer which tool a nameless tool-call block targets.
pub(crate) type ToolArgKeys = (&'static str, Vec<String>, Vec<String>);

/// Tiebreak when several tools share the most-specific schema for a nameless
/// call. The only real collision today is `get_email_body` vs `get_attachments`
/// — both take exactly `{email_id}`. An unnamed batched `email_id` call (Qwen
/// 3.6 reading emails to answer a question) is overwhelmingly a body read;
/// `get_attachments` is niche and the model names it explicitly when it means
/// it. Earlier entries win.
const INFER_TIEBREAK_PREFERENCE: &[&str] = &["get_email_body", "search_emails"];

/// Infer which tool a nameless tool-call block targets from its argument keys.
///
/// Qwen 3.6 under no-think batches body reads but drops the function name,
/// emitting `<tool_call>{"arguments":{"email_id":"…"}}</tool_call>`. We recover
/// the target by matching the call's argument keys against each tool's schema:
/// every key must be a valid property of the tool, AND all of the tool's
/// required params must be present. Returns the name only when EXACTLY one tool
/// matches — ambiguous (e.g. empty args) or unmatched blocks yield `None`, so we
/// never dispatch a guess.
fn infer_tool_from_arg_keys(arg_keys: &[String], tools: &[ToolArgKeys]) -> Option<&'static str> {
    use std::collections::HashSet;
    // No keys = no signal. Bare `<tool_call>{}` could be any zero-arg tool.
    if arg_keys.is_empty() {
        return None;
    }
    let keyset: HashSet<&str> = arg_keys.iter().map(String::as_str).collect();
    // A tool is a candidate when every supplied key is one of its properties
    // AND all of its required params are supplied.
    let mut candidates: Vec<(&'static str, usize)> = tools
        .iter()
        .filter(|(_, props, required)| {
            let props_set: HashSet<&str> = props.iter().map(String::as_str).collect();
            keyset.iter().all(|k| props_set.contains(k)) && required.iter().all(|r| keyset.contains(r.as_str()))
        })
        .map(|(name, props, _)| (*name, props.len()))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    // Disambiguate by specificity: a tool whose whole schema is just the keys
    // supplied (e.g. get_email_body = {email_id}) is a far better fit than one
    // that merely lists those keys among many optionals (generate_email_draft
    // also takes an optional `email_id`). Smallest property set wins.
    candidates.sort_by_key(|(_, n)| *n);
    let smallest = candidates[0].1;
    let tied: Vec<&'static str> = candidates
        .iter()
        .take_while(|(_, n)| *n == smallest)
        .map(|(name, _)| *name)
        .collect();
    if tied.len() == 1 {
        return Some(tied[0]);
    }
    // Several tools share the most-specific schema (get_email_body vs
    // get_attachments, both {email_id}). Break the tie by preference; if none of
    // the tied tools is preferred, the call is genuinely ambiguous — don't guess.
    INFER_TIEBREAK_PREFERENCE.iter().copied().find(|p| tied.contains(p))
}

/// Salvage NAMELESS `<tool_call>{json}</tool_call>` blocks by inferring each
/// block's tool from its argument keys (see [`infer_tool_from_arg_keys`]).
/// Companion to the named-call parsers — only runs when those find nothing, so
/// blocks that already carry a name never reach here.
fn parse_unnamed_tool_calls(text: &str, tools: &[ToolArgKeys]) -> Vec<crate::ai::provider::AiToolCall> {
    use crate::ai::provider::{AiToolCall, AiToolCallFunction};

    let mut out = Vec::new();
    // Implicit-close block splitting + lenient first-value parse — tolerates
    // the missing-close-tag and trailing-brace defects the named parsers also
    // accept; incomplete JSON simply fails the parse and is skipped.
    for (inner, _explicitly_closed) in tool_call_block_bodies(text) {
        let Some(value) = parse_first_json_value(inner) else {
            continue;
        };
        let Some(obj) = value.as_object() else { continue };
        // A resolvable top-level name means a named parser already handled it.
        if obj.get("name").and_then(|v| v.as_str()).is_some() {
            continue;
        }
        // Qwen wraps args under `arguments`; some emitters drop the wrapper and
        // put bare args at the top level. Support both.
        let mut arguments = match obj.get("arguments") {
            Some(a) => a.clone(),
            None => value.clone(),
        };
        let Some(args_obj) = arguments.as_object() else {
            continue;
        };
        let keys: Vec<String> = args_obj.keys().filter(|k| k.as_str() != "name").cloned().collect();
        if let Some(name) = infer_tool_from_arg_keys(&keys, tools) {
            // Drop any stray `name` key from the dispatched args.
            if let Some(o) = arguments.as_object_mut() {
                o.remove("name");
            }
            out.push(AiToolCall {
                function: AiToolCallFunction {
                    name: name.to_string(),
                    arguments,
                },
            });
        }
    }
    out
}

/// Full text-salvage chain for tool calls a model leaked as plain text instead
/// of the structured tool_calls channel: named `<tool_call>` XML/JSON blocks →
/// python-call literals → nameless blocks resolved by arg-key inference.
/// Returns the parsed calls plus the format label for logging. Shared by the
/// tool loop and the empty-synthesis recovery so both recognise the same
/// malformed shapes.
fn salvage_text_tool_calls(
    content: &str,
    registry: &tools::ToolRegistry,
    db: &Database,
) -> (Vec<crate::ai::provider::AiToolCall>, &'static str) {
    let mut parsed = parse_xml_tool_calls(content);
    let mut kind = "XML";
    if parsed.is_empty() {
        let known_tools = registry.names();
        parsed = parse_python_call_tool_calls(content, &known_tools);
        kind = "python-call";
    }
    if parsed.is_empty() {
        // Nameless `<tool_call>{"arguments":{…}}` blocks (Qwen 3.6 batched
        // no-think): infer each tool from its argument keys.
        let schemas = registry.arg_key_schemas(db);
        parsed = parse_unnamed_tool_calls(content, &schemas);
        kind = "inferred-name";
    }
    (parsed, kind)
}

/// Distinct email addresses written verbatim in `text`, first-occurrence
/// order, deduplicated case-insensitively (original casing preserved).
fn extract_email_addresses(text: &str) -> Vec<String> {
    use std::sync::OnceLock;
    static EMAIL_RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = EMAIL_RE.get_or_init(|| {
        // Hard-coded literal that cannot fail by construction.
        #[allow(clippy::unwrap_used)]
        regex::Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").unwrap()
    });
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for m in re.find_iter(text) {
        let addr = m.as_str().to_string();
        if seen.insert(addr.to_lowercase()) {
            out.push(addr);
        }
    }
    out
}

/// Decide whether an EMPTY `search_emails` result should be retried with the
/// verbatim address from the question.
///
/// Observed on Qwen 3.6 35B: the model TRANSLATES the address while copying
/// it out of the question ("cosasdefreelance@…" → "thingsdefreelance@…" /
/// "stufffreelance@…"), searches the wrong sender across several rounds, and
/// finally tells the user no emails exist. The mangled transcription is
/// recognisable: the searched from/to is an address that does NOT appear in
/// the question, while the question names exactly ONE address sharing its
/// domain or local part. Only ever consulted AFTER a search came back empty,
/// so a successful search on a legitimately different address (e.g. one
/// discovered via search_contacts) is never touched.
///
/// Returns `(corrected_args, right_address, wrong_address)`.
fn correct_mangled_address_args(
    args: &serde_json::Value,
    user_question: &str,
) -> Option<(serde_json::Value, String, String)> {
    let addrs = extract_email_addresses(user_question);
    let [qaddr] = addrs.as_slice() else {
        return None;
    };
    let qlower = qaddr.to_lowercase();
    let (qlocal, qdomain) = qlower.split_once('@')?;
    let question_lower = user_question.to_lowercase();
    for field in ["from", "to"] {
        let Some(v) = args.get(field).and_then(|v| v.as_str()).map(str::trim) else {
            continue;
        };
        let vlower = v.to_lowercase();
        // Only address-shaped values can be transcription slips; display-name
        // filters ("sharique") are left alone.
        let Some((vlocal, vdomain)) = vlower.split_once('@') else {
            continue;
        };
        // An address the user actually wrote is intentional, even if empty.
        if question_lower.contains(&vlower) {
            continue;
        }
        if vdomain == qdomain || vlocal == qlocal {
            let mut corrected = args.clone();
            if let Some(obj) = corrected.as_object_mut() {
                obj.insert(field.to_string(), serde_json::Value::String(qaddr.clone()));
            }
            return Some((corrected, qaddr.clone(), v.to_string()));
        }
    }
    None
}

/// Deterministically repair a filterless `search_emails` call using an email
/// address the user wrote verbatim in the question.
///
/// Weak/stubborn models (Qwen 3.6 35B on long analytical prompts) issue
/// `search_emails({})` — or worse, MANGLE the address when retrying (observed:
/// "cosasdefreelance@…" translated to "thingsdefreelance@…"). When the call
/// carries no selective filter and the question names exactly ONE address, we
/// inject it instead of bouncing a validation error off the model. The
/// preceding word decides direction ("a"/"to"/"para" → recipient, else
/// sender); ambiguity (zero or several addresses) leaves the call untouched.
/// `include_bodies` is set like the planner's preseeds so the synthesis has
/// content in one shot. Returns true when the args were modified.
fn repair_filterless_search_args(args: &mut serde_json::Value, user_question: &str) -> bool {
    const SELECTIVE: [&str; 6] = ["query", "from", "to", "subject", "since", "until"];
    let Some(obj) = args.as_object() else {
        return false;
    };
    let has_filter = SELECTIVE.iter().any(|k| {
        obj.get(*k)
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    });
    if has_filter {
        return false;
    }
    let addrs = extract_email_addresses(user_question);
    let [addr] = addrs.as_slice() else {
        return false;
    };
    // Direction from the word right before the first mention: "envié a X" /
    // "sent to X" / "para X" → recipient; anything else (incl. "de X") → sender.
    let field = match user_question
        .find(addr.as_str())
        .map(|pos| &user_question[..pos])
        .and_then(|before| before.split_whitespace().last().map(str::to_lowercase))
        .as_deref()
    {
        Some("a") | Some("to") | Some("para") | Some("hacia") => "to",
        _ => "from",
    };
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    obj.insert(field.to_string(), serde_json::Value::String(addr.clone()));
    obj.entry("limit").or_insert(serde_json::json!(25));
    obj.entry("include_bodies").or_insert(serde_json::json!(true));
    true
}

/// Deterministically repair an id-less `get_email_body` call: inject the next
/// email ref (from this turn's tool results, insertion order) that has not
/// been read yet.
///
/// Companion to [`repair_filterless_search_args`] for the other degenerate
/// shape Qwen 3.6 emits: it batches K body reads after a search but drops
/// every `email_id` (`get_email_body({})` × K), which used to bounce K
/// "missing email_id" errors and leave the analysis grounded in snippets
/// only. A batch of id-less reads right after a search unambiguously means
/// "read the results I just got" — walk them in order. Declines (returns
/// `None`) when the call already has an id or no unread ref remains, so
/// normal validation still applies.
fn repair_missing_email_id(
    args: &mut serde_json::Value,
    available_refs: &[String],
    consumed: &std::collections::HashSet<String>,
) -> Option<String> {
    let has_id = args
        .as_object()?
        .get("email_id")
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    if has_id {
        return None;
    }
    let next = available_refs.iter().find(|r| !consumed.contains(*r))?.clone();
    args.as_object_mut()?
        .insert("email_id".to_string(), serde_json::Value::String(next.clone()));
    Some(next)
}

/// Canonical, argument-order-independent key for a tool call (`name|args`), so
/// two calls that differ only in JSON key order are recognised as the same.
/// Used by the tool loop to spot a model re-issuing an identical call instead
/// of answering.
fn tool_call_key(tc: &crate::ai::provider::AiToolCall) -> String {
    format!("{}|{}", tc.function.name, canonical_json(&tc.function.arguments))
}

/// Stable string form of a JSON value with object keys sorted (recursively).
fn canonical_json(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let body = keys
                .into_iter()
                .map(|k| format!("{k}:{}", canonical_json(&map[k])))
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
        serde_json::Value::Array(arr) => {
            format!("[{}]", arr.iter().map(canonical_json).collect::<Vec<_>>().join(","))
        }
        other => other.to_string(),
    }
}

/// Salvage Python-call-style tool calls embedded in plain assistant text.
///
/// Companion to [`parse_xml_tool_calls`] for the *other* malformed shape
/// small models emit when they lose the tool-call channel — a Python
/// function-call literal either prefixed with `tool_call:` or appearing
/// alone on a line as a bare `name(args)`:
///
/// ```text
/// tool_call: get_email_body(email_id="19e73d15a43b67e8")
/// search_emails(from="lena.park@orbitfreight.co", limit=1)
/// ```
///
/// A line is considered a salvageable call when EITHER:
///   1. It begins with `tool_call:` (case-insensitive) — prefix-tagged form,
///      accepted regardless of the identifier; OR
///   2. It is structurally `name(args)` after trimming (no prose before the
///      identifier, no prose after the closing paren, modulo trailing
///      `.,;` punctuation) AND `name` matches one of `known_tools`.
///
/// The registry check on the bare form is the false-positive guard: prose
/// like "I used `search_emails(...)` to find it" still won't trigger a
/// phantom call because the line isn't structurally just the call. An
/// unknown identifier is treated as prose either way.
///
/// Arguments are parsed as `key=value` pairs:
///   - `"..."` and `'...'` → string
///   - integer / float literals → JSON number (so `limit=25` matches the
///     tool schema)
///   - `true` / `false` → JSON bool
///   - anything else → string (kept verbatim)
pub(crate) fn parse_python_call_tool_calls(text: &str, known_tools: &[&str]) -> Vec<crate::ai::provider::AiToolCall> {
    use crate::ai::provider::{AiToolCall, AiToolCallFunction};

    let is_valid_ident = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');

    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed_start = line.trim_start();
        // Branch 1: explicit `tool_call:` prefix — accepted regardless of
        // identifier (legacy salvage from when the registry wasn't threaded
        // through; still useful when the model invents a tool name).
        //
        // `.get(..len)` is boundary-safe: returns None when the byte boundary
        // falls inside a multibyte UTF-8 character (e.g. Spanish `á` at byte
        // 9..11 of "Aquí están…"). The old `[..len]` slice panicked there.
        // This parser now runs on EVERY assistant turn (not just salvage), so
        // a multibyte leading char anywhere in real output trips the boundary.
        let rest_owned;
        let prefix_matches = trimmed_start
            .get(.."tool_call:".len())
            .is_some_and(|p| p.eq_ignore_ascii_case("tool_call:"));
        let rest: &str = if prefix_matches {
            // The `"tool_call:"` prefix is ASCII, so byte 10 is on a char
            // boundary by construction whenever the prefix matched — slicing
            // forward from there is safe.
            trimmed_start["tool_call:".len()..].trim_start()
        } else {
            // Branch 2: bare `name(args)` line. The whole line, after
            // trimming whitespace and trailing punctuation, must structurally
            // be the call, AND `name` must be in the registry.
            let stripped = line.trim().trim_end_matches(['.', ',', ';']).trim_end();
            let paren_open = match stripped.find('(') {
                Some(i) => i,
                None => continue,
            };
            // Closing paren must terminate the (stripped) line.
            if !stripped.ends_with(')') {
                continue;
            }
            let name = stripped[..paren_open].trim();
            if !is_valid_ident(name) || !known_tools.contains(&name) {
                continue;
            }
            rest_owned = stripped.to_string();
            &rest_owned
        };

        // `rest` should now look like `name(args)`. Find the outer parens.
        let paren_open = match rest.find('(') {
            Some(i) => i,
            None => continue,
        };
        let paren_close = match rest.rfind(')') {
            Some(i) if i > paren_open => i,
            _ => continue,
        };
        let name = rest[..paren_open].trim();
        if !is_valid_ident(name) {
            continue;
        }
        let args_str = &rest[paren_open + 1..paren_close];
        let args = parse_python_call_kwargs(args_str);

        out.push(AiToolCall {
            function: AiToolCallFunction {
                name: name.to_string(),
                arguments: serde_json::Value::Object(args),
            },
        });
    }
    out
}

/// Parse a Python-call kwargs string (`a="x", b=1, c=true`) into a JSON
/// object. Tolerates whitespace; respects single- and double-quoted
/// strings so commas inside strings don't split a pair in two.
fn parse_python_call_kwargs(s: &str) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    for piece in split_top_level_commas(s) {
        let pair = piece.trim();
        if pair.is_empty() {
            continue;
        }
        let eq = match pair.find('=') {
            Some(i) => i,
            None => continue,
        };
        let key = pair[..eq].trim();
        let raw_val = pair[eq + 1..].trim();
        if key.is_empty() {
            continue;
        }
        out.insert(key.to_string(), parse_python_value(raw_val));
    }
    out
}

/// Split a comma-separated kwargs body at top level, respecting `"` and
/// `'` string delimiters so `a="x,y", b=2` produces two pieces, not three.
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0usize;
    let mut quote: Option<u8> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None => match b {
                b'"' | b'\'' => quote = Some(b),
                b',' => {
                    out.push(&s[start..i]);
                    start = i + 1;
                }
                _ => {}
            },
        }
    }
    if start <= s.len() {
        out.push(&s[start..]);
    }
    out
}

fn parse_python_value(raw: &str) -> serde_json::Value {
    let r = raw.trim();
    if r.len() >= 2 && ((r.starts_with('"') && r.ends_with('"')) || (r.starts_with('\'') && r.ends_with('\''))) {
        return serde_json::Value::String(r[1..r.len() - 1].to_string());
    }
    if let Ok(n) = r.parse::<i64>() {
        return serde_json::Value::Number(n.into());
    }
    if let Ok(f) = r.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return serde_json::Value::Number(n);
        }
    }
    match r {
        "true" | "True" => return serde_json::Value::Bool(true),
        "false" | "False" => return serde_json::Value::Bool(false),
        "null" | "None" => return serde_json::Value::Null,
        _ => {}
    }
    serde_json::Value::String(r.to_string())
}

// ── Direct-tool shortcut routing ────────────────────────────────────────────
//
// Some queries are so stereotyped ("summary of today's emails", "mis pendientes")
// that the cheapest way to answer them is to skip the LLM's tool-choice round
// entirely and drive the tool call from the backend. On a local 3B-class model
// that's a 2–6 s latency win: the first LLM round just decides which tool to
// call, and we can decide that deterministically from keywords for these cases.
//
// Returns `Some(Vec<AiToolCall>)` when the question matches a shortcut. The
// caller feeds these calls as the "virtual round 0" into `run_tool_loop`, so
// the first real LLM pass is the one that *summarises* the tool results —
// exactly the work the LLM is actually good at.
/// Whether the tools-first query planner runs. Defaults ON; set the
/// `chat.planner_enabled` preference to `"false"` to disable (falls back to the
/// model's own tool-choice round).
fn planner_enabled(db: &Arc<Database>) -> bool {
    db.get_preference("chat.planner_enabled")
        .ok()
        .flatten()
        .map(|v| v != "false")
        .unwrap_or(true)
}

/// `calendar_available`: whether the conversation's account has calendar
/// integration (Gmail/Outlook) — the weekly-report shortcut preseeds the
/// account's calendar events alongside the email summary when it does.
fn heuristic_direct_tools(
    user_question: &str,
    calendar_available: bool,
) -> Option<Vec<crate::ai::provider::AiToolCall>> {
    use crate::ai::provider::{AiToolCall, AiToolCallFunction};
    let q = user_question.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }

    // Helper: build a search_emails call with an ISO since/until window.
    let search_since_until = |since: chrono::NaiveDate, until: chrono::NaiveDate| -> Vec<AiToolCall> {
        vec![AiToolCall {
            function: AiToolCallFunction {
                name: "search_emails".to_string(),
                arguments: serde_json::json!({
                    "since": since.format("%Y-%m-%d").to_string(),
                    "until": until.format("%Y-%m-%d").to_string(),
                    "limit": 25,
                    // Internal arg (not in the tool's LLM-facing schema): pull
                    // full bodies so the summary pass has everything it needs in
                    // one shot — the model never has to call get_email_body and
                    // can't leak that tool-call markup into the summary.
                    "include_bodies": true,
                }),
            },
        }]
    };

    // Keyword co-occurrence rather than fixed substrings: the frontend quick
    // shortcuts send verbose prompts (e.g. "Hazme un resumen de los
    // emails que he recibido hoy. Formatéalo como una tabla markdown…") that
    // wouldn't match rigid phrases like "resumen de hoy". Co-occurrence with
    // mutual exclusion of competing time windows gets the right intent.
    let has_today = q.contains("hoy") || q.contains("today");
    let has_week = q.contains("semana") || q.contains("week");
    let has_month = q.contains("mes ") || q.contains(" mes.") || q.contains(" mes,") || q.contains("month");
    let has_summary = q.contains("resumen")
        || q.contains("resume ")
        || q.contains("resúmeme")
        || q.contains("resumeme")
        || q.contains("summary")
        || q.contains("summarize")
        || q.contains("summarise");

    // Summary of today's emails (EN + ES).
    if has_today && has_summary && !has_week && !has_month {
        let today = now_utc().date_naive();
        let tomorrow = today + chrono::Duration::days(1);
        return Some(search_since_until(today, tomorrow));
    }

    // Summary of this week's emails (EN + ES). When the account has calendar
    // integration, also preseed the week's calendar events so the report
    // covers meetings — the model summarises both in one pass.
    if has_week && has_summary && !has_month {
        let now = now_utc().date_naive();
        let days_since_monday = now.weekday().num_days_from_monday() as i64;
        let monday = now - chrono::Duration::days(days_since_monday);
        let next_monday = monday + chrono::Duration::days(7);
        let mut calls = search_since_until(monday, next_monday);
        if calendar_available {
            calls.push(AiToolCall {
                function: AiToolCallFunction {
                    name: "list_calendar_events".to_string(),
                    arguments: serde_json::json!({
                        "since": monday.format("%Y-%m-%d").to_string(),
                        "until": next_monday.format("%Y-%m-%d").to_string(),
                    }),
                },
            });
        }
        return Some(calls);
    }

    // Pending tasks: frontend sends "Identifica los emails que requieren mi
    // respuesta o acción…" — match on the action verbs rather than exact phrases.
    let pending_triggers = [
        "pendiente",
        "pendientes",
        "requieren mi respuesta",
        "requieren respuesta",
        "mi acción",
        "mi accion",
        "pending tasks",
        "my pending",
        "what are my pending",
        "open tasks",
        "what needs my",
        "awaiting my",
        "needs my reply",
    ];
    if pending_triggers.iter().any(|p| q.contains(p)) {
        return Some(vec![AiToolCall {
            function: AiToolCallFunction {
                name: "list_pending_tasks".to_string(),
                arguments: serde_json::json!({}),
            },
        }]);
    }

    // Open threads / waiting on
    let open_threads_patterns = [
        "open threads",
        "what am i waiting on",
        "waiting on me",
        "hilos abiertos",
        "qué tengo pendiente de responder",
        "que tengo pendiente de responder",
    ];
    if open_threads_patterns.iter().any(|p| q.contains(p)) {
        return Some(vec![AiToolCall {
            function: AiToolCallFunction {
                name: "list_open_threads".to_string(),
                arguments: serde_json::json!({}),
            },
        }]);
    }

    None
}

/// Outcome of the tool-call loop — returned instead of just the message list
/// so `run_chat_turn` can distinguish "model answered directly" from
/// "everything timed out before we got any assistant content" and react
/// accordingly (the latter must surface an error to the user instead of
/// trying to re-stream from the initial prompt, which will likely also hang).
struct ToolLoopOutcome {
    messages: Vec<AiMessage>,
    /// True when the loop exited without producing any assistant message
    /// (e.g. `chat_with_tools` errored on round 0 with no prior answer).
    failed_without_answer: bool,
    /// Human-readable reason the loop aborted, if any.
    error: Option<String>,
    /// Union (insertion-order, dedup'd) of every `ToolOutput.email_refs`
    /// produced by tool calls in this turn. The frontend uses it as an
    /// allowlist for `email://EMAIL_ID` links the LLM emits in its prose.
    aggregated_email_refs: Vec<String>,
    /// Same shape as `aggregated_email_refs` but for `ToolOutput.draft_refs`
    /// — the `draft://DRAFT_ID` allowlist for re-open-the-draft chips.
    aggregated_draft_refs: Vec<String>,
    /// True when the loop already streamed the final assistant answer to the
    /// client live (token-by-token via `chat_stream_with_tools`). When set, the
    /// caller's direct-answer path must NOT re-emit the answer as a single
    /// `chat-stream` token — doing so would duplicate the whole bubble.
    answer_streamed_live: bool,
}

/// How `run_chat_turn` should turn the tool loop's final messages into the
/// user-visible answer.
enum AnswerPlan {
    /// The loop already produced a complete assistant text answer (e.g. a
    /// normal tools-first turn where the model replied after a tool round).
    /// Emit it as a single stream token — no extra model round-trip.
    DirectText(String),
    /// The loop stopped before writing any answer text: a preseeded shortcut
    /// handed back tool results for synthesis, or the loop hit MAX_TOOL_ROUNDS.
    /// Stream a synthesis pass over these messages. `tool_calls` are kept so the
    /// assistant→tool linkage that binds each result to its call survives into
    /// the prompt (providers that render OpenAI-style tool roles need it;
    /// text-only streaming backends ignore the field).
    StreamSynthesis(Vec<AiMessage>),
}

/// Closing instruction appended to a synthesis pass (the loop ended on tool
/// results — a preseeded shortcut, or a run that hit `MAX_TOOL_ROUNDS` while
/// still fetching). Without it, models that fetch incrementally (Qwen 3.6
/// 35B-A3B emits one `get_email_body` per round) keep emitting another
/// `<tool_call>` in the tools-free synthesis stream; the stream gate strips
/// that markup, leaving the user an EMPTY reply. This forces a prose answer
/// from whatever was already gathered. Bilingual so it lands regardless of the
/// system prompt's reply language.
const SYNTHESIS_CLOSING_INSTRUCTION: &str =
    "Ya tienes toda la información necesaria en los resultados de las tools anteriores. \
NO llames a ninguna tool más ni escribas etiquetas <tool_call>. Responde AHORA a la pregunta del usuario \
usando solo esa información; si falta algún dato, responde con lo que tengas.\n\n\
You already have all the information you need in the tool results above. Do NOT call any more tools or emit \
<tool_call> markup. Answer the user's question NOW using only that information; if something is missing, \
answer with what you have.";

/// Corrective instruction for the ONE retry after the synthesis stream came
/// back empty — the model answered the tools-free synthesis pass with nothing
/// but tool-call markup that the gate/strip removed, so
/// [`SYNTHESIS_CLOSING_INSTRUCTION`] alone did not land. Harder wording than
/// the closing instruction: name the failure, forbid tool syntax, and demand
/// a prose answer even when the tool results carry nothing useful (e.g. every
/// call in the loop failed validation). Bilingual for the same reason as
/// [`SYNTHESIS_CLOSING_INSTRUCTION`].
const SYNTHESIS_RETRY_INSTRUCTION: &str =
    "Tu respuesta anterior quedó vacía: contenía solo una llamada a una tool. Las tools NO están disponibles \
en este paso. Escribe AHORA una respuesta en prosa para el usuario. Si no encontraste la información \
necesaria, dilo claramente y sugiere cómo reformular la pregunta.\n\n\
Your previous reply was empty: it contained only a tool call. Tools are NOT available in this step. Write a \
prose answer for the user NOW. If you could not find the information, say so plainly and suggest how the \
user could rephrase the question.";

/// Instruction appended after executing tool call(s) salvaged from an empty
/// synthesis stream: the model answered with a tool call instead of prose, we
/// ran it for real, and this closes the loop by demanding prose over the new
/// result. Bilingual for the same reason as [`SYNTHESIS_CLOSING_INSTRUCTION`].
const SYNTHESIS_SALVAGED_TOOL_INSTRUCTION: &str =
    "La tool que pediste ya se ha ejecutado; su resultado está arriba. No hay más tools disponibles. \
Responde AHORA en prosa a la pregunta del usuario usando esa información; si no es suficiente, dilo \
claramente y sugiere cómo reformular la pregunta.\n\n\
The tool you requested has been executed; its result is above. No more tools are available. Answer the \
user's question NOW in prose using that information; if it is not enough, say so plainly and suggest how \
the user could rephrase the question.";

/// Corrective instruction for the ONE retry after the model produced a
/// categorical "no results" answer even though the tool results above DO
/// contain emails (observed on Qwen 3.6 35B-A3B: `search_emails` returned
/// "## Primary (1) …" with a full body excerpt and the model still replied
/// "No se han encontrado correos electrónicos"). Bilingual for the same
/// reason as [`SYNTHESIS_CLOSING_INSTRUCTION`].
const CONTRADICTION_RETRY_INSTRUCTION: &str =
    "Tu respuesta anterior dice que no se encontraron correos, pero los resultados de las tools de arriba \
SÍ contienen correos. Vuelve a leer esos resultados y responde AHORA a la pregunta del usuario usándolos. \
No digas que no hay resultados.\n\n\
Your previous answer claims no emails were found, but the tool results above DO contain emails. Re-read \
those results and answer the user's question NOW using them. Do not claim there are no results.";

/// Max length (in chars) for an answer to count as a categorical "no results"
/// claim in [`answer_claims_no_results`]. Real summaries run much longer; the
/// observed failure shape is 1-3 short sentences, possibly with an empty
/// placeholder table. The cap keeps a genuine answer that merely MENTIONS an
/// absence ("…no emails from Alice, though") from triggering a retry.
const NO_RESULTS_CLAIM_MAX_CHARS: usize = 400;

/// True when `answer` is a short, categorical "no emails / no results" claim.
///
/// Pure half of the contradiction guard: a model can hand back tool results
/// that plainly contain emails and still answer "nothing found" (observed on
/// Qwen 3.6 35B-A3B with a single long newsletter body in the tool result).
/// Patterns cover the four UI languages; matching is substring-based over the
/// lowercased answer and deliberately conservative — a long answer never
/// matches, see [`NO_RESULTS_CLAIM_MAX_CHARS`].
fn answer_claims_no_results(answer: &str) -> bool {
    let trimmed = answer.trim();
    if trimmed.is_empty() || trimmed.chars().count() > NO_RESULTS_CLAIM_MAX_CHARS {
        return false;
    }
    let norm = trimmed.to_lowercase();
    const PATTERNS: &[&str] = &[
        // Spanish
        "no se han encontrado correo",
        "no se encontraron correo",
        "no se ha encontrado ning",
        "no se encontró ning",
        "no hay correos",
        "no hay ningún correo",
        "no hay ningun correo",
        "no he encontrado correo",
        "no he encontrado ning",
        "no encontré correo",
        "no encontré ning",
        "no tienes correos",
        // English
        "no emails were found",
        "no emails found",
        "no emails match",
        "no emails in your inbox",
        "there are no emails",
        "you have no emails",
        "couldn't find any email",
        "could not find any email",
        "found no email",
        "no messages were found",
        "no messages found",
        // French
        "aucun courriel",
        "aucun e-mail",
        "aucun email",
        "aucun message trouvé",
        "aucun résultat",
        // German
        "keine e-mails gefunden",
        "keine emails gefunden",
        "keine e-mails in",
        "es wurden keine",
        "keine nachrichten gefunden",
    ];
    PATTERNS.iter().any(|p| norm.contains(p))
}

/// User-facing fallback when the turn still has no answer text after the
/// synthesis retry (model returned empty twice). Shipping this instead of a
/// blank bubble tells the user what happened and how to move forward.
/// Localized via the same [`crate::services::i18n::Language`] that drives the
/// reply-language instruction, so the hint matches the conversation language.
fn empty_answer_hint(lang: crate::services::i18n::Language) -> &'static str {
    use crate::services::i18n::Language;
    match lang {
        Language::En => {
            "I couldn't generate an answer for this request (the model returned an empty response twice). \
Try rephrasing the question in a simpler way, or narrow it down — for example, name the sender or a date range."
        }
        Language::Es => {
            "No he podido generar una respuesta para esta petición (el modelo devolvió una respuesta vacía \
dos veces). Prueba a reformular la pregunta de forma más sencilla o acótala; por ejemplo, indica el \
remitente o un rango de fechas."
        }
        Language::Fr => {
            "Je n'ai pas pu générer de réponse pour cette demande (le modèle a renvoyé une réponse vide deux \
fois). Essayez de reformuler la question plus simplement ou de la préciser — par exemple, indiquez \
l'expéditeur ou une plage de dates."
        }
        Language::De => {
            "Ich konnte für diese Anfrage keine Antwort erzeugen (das Modell hat zweimal eine leere Antwort \
geliefert). Formuliere die Frage einfacher oder grenze sie ein – nenne zum Beispiel den Absender oder \
einen Zeitraum."
        }
    }
}

/// Decide how to produce the answer from the tool loop's final message list.
/// Pure so the routing is unit-testable without a provider or `AppHandle`.
///
/// A trailing assistant message with real text and no pending tool_calls is a
/// finished answer; anything else (a tool result, a still-open tool call, a
/// blank assistant turn, or text that is ONLY tool-call markup and would strip
/// to nothing) needs a streamed synthesis pass — to which we append an
/// explicit "stop calling tools, answer now" instruction.
fn plan_answer(mut final_messages: Vec<AiMessage>) -> AnswerPlan {
    let is_direct = final_messages
        .last()
        .map(|m| {
            m.role == "assistant"
                && m.tool_calls.as_ref().map(|v| v.is_empty()).unwrap_or(true)
                && !strip_tool_call_markup(&m.content).trim().is_empty()
        })
        .unwrap_or(false);
    if is_direct {
        // Guarded by `is_direct`, so `last()` is Some.
        let content = final_messages.last().map(|m| m.content.clone()).unwrap_or_default();
        AnswerPlan::DirectText(content)
    } else {
        // Force a prose answer from the gathered results instead of another
        // `<tool_call>` (which the synthesis gate would strip to empty).
        final_messages.push(AiMessage {
            role: "user".to_string(),
            content: SYNTHESIS_CLOSING_INSTRUCTION.to_string(),
            tool_calls: None,
        });
        AnswerPlan::StreamSynthesis(final_messages)
    }
}

/// Whether a tool-loop round may stream its assistant prose to the user live.
///
/// A round must NOT stream when a nudge is still possible: small models often
/// "announce" a tool call as plain text on the first force-tool round
/// ("Voy a buscar el contacto…") and then stop. We nudge once and discard that
/// announcement instead of accepting it as the answer — streaming it live would
/// leak the discarded text to the user. Once a tool has executed, nudges are
/// exhausted, or the turn does not force tool use (thread-bound Q&A), the
/// model's prose is a genuine answer and is safe to stream token-by-token.
///
/// This is the exact inverse of the nudge guard in the loop, kept as one pure
/// function so both the gate decision and the nudge branch share a single
/// source of truth.
fn round_may_stream_live(force_tool_use: bool, no_tool_executed_yet: bool, nudges_used: u32, max_nudges: u32) -> bool {
    !(force_tool_use && no_tool_executed_yet && nudges_used < max_nudges)
}

/// Run a tool-call loop: send the prompt with tool definitions, execute any
/// requested tool calls, feed results back, repeat until the model gives a
/// text-only response (or we hit MAX_TOOL_ROUNDS).
#[allow(clippy::too_many_arguments)]
/// Pure: assemble the trace entry for one tool-loop LLM round. `result` is
/// `None` when the call failed. Prompt/prefill stats are copied from the
/// provider result when the backend reports them; `input`/`output` snapshots
/// are attached by the caller (dev builds only).
fn build_tool_round_trace(
    round: i32,
    latency_ms: i64,
    result: Option<&crate::ai::provider::ToolStreamResult>,
) -> LlmCallTrace {
    LlmCallTrace {
        kind: "tool_round".to_string(),
        round,
        latency_ms,
        tool_calls_requested: result
            .and_then(|r| r.message.tool_calls.as_ref())
            .map(|v| v.len() as i32)
            .unwrap_or(0),
        failed: result.is_none(),
        prompt_tokens: result.and_then(|r| r.prompt_eval_count),
        prefill_ms: result.and_then(|r| r.prefill_ms),
        cached_prompt_tokens: result.and_then(|r| r.cached_prompt_tokens),
        prefix_plan: result.and_then(|r| r.prefix_plan).map(|s| s.to_string()),
        sys_cached_before: result.and_then(|r| r.sys_cached_before),
        sys_cached_after: result.and_then(|r| r.sys_cached_after),
        system_prefix_tokens: result.and_then(|r| r.system_prefix_tokens),
        stable_tokens: result.and_then(|r| r.stable_tokens),
        dropped_front_tokens: result.and_then(|r| r.dropped_front_tokens),
        input: None,
        output: None,
    }
}

/// Pure: assemble the trace entry for the final streaming synthesis call.
/// `result` is `None` when the stream failed or timed out.
fn build_final_stream_trace(latency_ms: i64, result: Option<&crate::ai::provider::ChatStreamResult>) -> LlmCallTrace {
    LlmCallTrace {
        kind: "final_stream".to_string(),
        round: -1,
        latency_ms,
        tool_calls_requested: 0,
        failed: result.is_none(),
        prompt_tokens: result.and_then(|r| r.prompt_eval_count),
        prefill_ms: result.and_then(|r| r.prefill_ms),
        cached_prompt_tokens: result.and_then(|r| r.cached_prompt_tokens),
        prefix_plan: result.and_then(|r| r.prefix_plan).map(|s| s.to_string()),
        sys_cached_before: result.and_then(|r| r.sys_cached_before),
        sys_cached_after: result.and_then(|r| r.sys_cached_after),
        system_prefix_tokens: result.and_then(|r| r.system_prefix_tokens),
        stable_tokens: result.and_then(|r| r.stable_tokens),
        dropped_front_tokens: result.and_then(|r| r.dropped_front_tokens),
        input: None,
        output: None,
    }
}

/// Trace entry for the pre-loop query planner (`plan_search`). Surfaced in the
/// flow timeline so the planner LLM call is visible (`kind: "planner"`, sorted
/// before round 0 via `round: -2`) instead of hidden behind a log line.
/// `outcome` is `"search"` or `"defer"`.
fn build_planner_trace(latency_ms: i64, outcome: &str) -> LlmCallTrace {
    LlmCallTrace {
        kind: "planner".to_string(),
        round: -2,
        latency_ms,
        tool_calls_requested: 0,
        failed: false,
        prompt_tokens: None,
        prefill_ms: None,
        cached_prompt_tokens: None,
        prefix_plan: None,
        sys_cached_before: None,
        sys_cached_after: None,
        system_prefix_tokens: None,
        stable_tokens: None,
        dropped_front_tokens: None,
        input: None,
        output: Some(format!("planner: {outcome}")),
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_tool_loop(
    db: &Arc<Database>,
    registry: &Arc<tools::ToolRegistry>,
    provider: &dyn AIProvider,
    conversation_id: &str,
    message_id: &str,
    account_id: &str,
    categories: &[String],
    user_question: &str,
    initial_messages: Vec<(String, String)>,
    preseeded_tool_calls: Option<Vec<crate::ai::provider::AiToolCall>>,
    force_tool_use: bool,
    tool_traces: &mut Vec<ToolCallTrace>,
    llm_calls: &mut Vec<LlmCallTrace>,
) -> ToolLoopOutcome {
    // Feature-flag–aware: tools whose `is_available(db)` returns false are
    // omitted from the array the LLM sees.
    let tools = registry.definitions(db.as_ref());

    // Convert (role, content) pairs into AiMessage structs.
    let mut messages: Vec<AiMessage> = initial_messages
        .into_iter()
        .map(|(role, content)| AiMessage {
            role,
            content,
            tool_calls: None,
        })
        .collect();

    let mut had_any_answer = false;
    let mut abort_error: Option<String> = None;
    // Set when the round that produced the final answer streamed its prose to
    // the client live (token-by-token). The caller reads this to decide whether
    // its direct-answer path should re-emit the answer as a single token or skip
    // (it was already shipped).
    let mut answer_streamed_live = false;
    // Small local models (e.g. qwen3.5-4b) sometimes "announce" they will call
    // a tool ("Voy a buscar el contacto…") and then stop without emitting an
    // actual tool_call — turning the announcement into the final answer. Allow
    // exactly one nudge round in that situation before accepting the bare text.
    let mut nudges_used = 0u32;
    const MAX_NUDGES: u32 = 1;
    // Insertion-ordered, dedup'd union of every email id each tool call
    // hands back. Persisted on the assistant message at end of turn and
    // shipped to the frontend as the `email://EMAIL_ID` allowlist. The
    // HashSet is the dedup key; the Vec preserves first-seen order so the
    // UI can render chips in the order tools produced them.
    let mut aggregated_email_refs: Vec<String> = Vec::new();
    let mut seen_email_refs: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Same shape for drafts (`draft://DRAFT_ID` chips).
    let mut aggregated_draft_refs: Vec<String> = Vec::new();
    let mut seen_draft_refs: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Canonical keys of tool calls already executed this turn. Some models
    // (qwen3.5-9b observed) re-issue the SAME search every round instead of
    // answering, burning the whole MAX_TOOL_ROUNDS budget on identical calls;
    // a round that only repeats executed calls makes no progress, so we break
    // to synthesis instead of re-running it.
    let mut executed_tool_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Email ids whose body has been fetched (or scheduled) this turn — the
    // consumed-set for `repair_missing_email_id`, so a batch of id-less
    // get_email_body calls walks distinct unread search results.
    let mut read_body_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    // ── Preseeded round 0: execute heuristic-detected tool calls directly ──
    // For shortcut queries like "summary of today's emails" we already know
    // which tool to call. Skip the LLM tool-choice round entirely: synthesise
    // an assistant message with the tool_calls, execute each tool, append the
    // tool results, and fall through to the normal loop so the LLM's first
    // call just synthesises the answer from the tool output.
    if let Some(tool_calls) = preseeded_tool_calls {
        if !tool_calls.is_empty() {
            emit_log(
                "info",
                &format!(
                    "shortcut: executing {} tool(s) directly (skipped LLM tool-choice round)",
                    tool_calls.len()
                ),
            );

            // Synthetic assistant message with the preseeded tool_calls.
            messages.push(AiMessage {
                role: "assistant".to_string(),
                content: String::new(),
                tool_calls: Some(tool_calls.clone()),
            });

            for tc in &tool_calls {
                let name = &tc.function.name;
                let args = &tc.function.arguments;

                let args_str = serde_json::to_string(args).unwrap_or_else(|_| args.to_string());
                emit_log(
                    "info",
                    &format!("tool call → {}({})", name, truncate_chars(&args_str, 400)),
                );

                // Tool-specific status ("Searching emails" / "Generating
                // draft" / …) so the UI reflects what each call is doing.
                emit_phase(conversation_id, message_id, phase_for_tool(name));
                let t_tool = std::time::Instant::now();
                let dispatched =
                    dispatch_tool(registry, db, account_id, categories, user_question, name, args.clone()).await;
                let elapsed_ms = t_tool.elapsed().as_millis() as i64;
                let traced_args = dispatched.corrected_args.unwrap_or_else(|| args.clone());
                let result = dispatched.text;
                for id in dispatched.email_refs {
                    if seen_email_refs.insert(id.clone()) {
                        aggregated_email_refs.push(id);
                    }
                }
                for id in dispatched.draft_refs {
                    if seen_draft_refs.insert(id.clone()) {
                        aggregated_draft_refs.push(id);
                    }
                }

                emit_log(
                    "info",
                    &format!(
                        "tool result ← {} ({}{} chars, {}ms)",
                        name,
                        tool_result_count_hint(name, &result),
                        result.len(),
                        elapsed_ms,
                    ),
                );

                tool_traces.push(ToolCallTrace {
                    name: name.clone(),
                    // Preseeded shortcut tools run before the LLM loop.
                    round: -1,
                    arguments: traced_args,
                    result_preview: truncate_chars(&result, 16000),
                    result_chars: result.len() as i32,
                    elapsed_ms,
                });

                messages.push(AiMessage {
                    role: "tool".to_string(),
                    content: result,
                    tool_calls: None,
                });
            }

            // The heuristic chose the tool(s) and they have run. Fall through
            // into the normal tool loop rather than returning here: the model's
            // first round sees the tool results and usually synthesises the
            // answer directly, but it may also decide it needs a FOLLOW-UP tool
            // (e.g. read a body with get_email_body before summarising). The
            // loop can dispatch that follow-up and salvage tool calls the model
            // emits as text; a tools-free synthesis pass cannot, and would strip
            // such a leaked call into an empty answer.
        }
    }

    for round in 0..MAX_TOOL_ROUNDS {
        // Snapshot the prompt sent to the model so the reasoning panel can
        // show exactly what each tool round received. Dev-only — release
        // builds skip the formatting to avoid the per-round allocation cost.
        #[cfg(debug_assertions)]
        let input_snapshot = format_messages_for_trace(&messages);

        // Surface every LLM round at info-level. On first turn this is where
        // the model GGUF loads (multi-second), so users would otherwise see
        // "thinking…" with no further progress until first token.
        emit_log(
            "info",
            &format!(
                "llm round {}: calling chat_stream_with_tools ({} msgs in context)",
                round,
                messages.len(),
            ),
        );
        // While the model thinks, reflect "Processing prompt…" rather than
        // leaving the UI on the previous tool's status (e.g. a preseeded
        // search_emails leaves "Searching emails…" up for the whole multi-second
        // LLM round, making the turn look stuck). RunningTools is the generic
        // "the model is working" phase.
        emit_phase(conversation_id, message_id, ChatPhase::RunningTools);
        let t_call = std::time::Instant::now();

        // Stream this round's prose to the client live UNLESS a nudge is still
        // possible — see `round_may_stream_live`. On llama.cpp the provider's
        // StreamGate suppresses tool-call syntax, so a tool-calling round emits
        // nothing here and only a genuine text answer reaches `on_token`. The
        // atomic records whether any token was actually shipped so the caller's
        // direct-answer path can skip re-emitting an already-streamed answer.
        let stream_live = round_may_stream_live(force_tool_use, tool_traces.is_empty(), nudges_used, MAX_NUDGES);
        let streamed_any = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let on_token: Box<dyn FnMut(String) -> bool + Send> = if stream_live {
            let conv_for_token = conversation_id.to_string();
            let msg_for_token = message_id.to_string();
            let streamed_flag = streamed_any.clone();
            Box::new(move |token: String| {
                if !token.is_empty() {
                    streamed_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    crate::services::events::emit(
                        "chat-stream",
                        ChatStreamEvent {
                            message_id: msg_for_token.clone(),
                            conversation_id: conv_for_token.clone(),
                            token,
                            done: false,
                            error: None,
                            token_count: None,
                            latency_ms: None,
                            replace: None,
                        },
                    );
                }
                true
            })
        } else {
            // Nudge still possible: buffer silently. Any prose this round is a
            // potential tool-call announcement we may discard, so never ship it.
            Box::new(|_token: String| true)
        };
        let call_result = provider
            .chat_stream_with_tools(messages.clone(), tools.clone(), on_token)
            .await;
        let last_round_streamed_live = streamed_any.load(std::sync::atomic::Ordering::Relaxed);
        let call_ms = t_call.elapsed().as_millis() as i64;
        emit_log(
            "info",
            &format!(
                "llm round {}: returned [{}ms] ({})",
                round,
                call_ms,
                match &call_result {
                    Ok(r) => format!(
                        "{} tool_calls",
                        r.message.tool_calls.as_ref().map(|v| v.len()).unwrap_or(0)
                    ),
                    Err(e) => format!("error: {}", e),
                }
            ),
        );

        let response = match call_result {
            Ok(r) => {
                #[allow(unused_mut)] // mutated only in debug builds (trace snapshots)
                let mut trace = build_tool_round_trace(round as i32, call_ms, Some(&r));
                #[cfg(debug_assertions)]
                {
                    trace.input = Some(input_snapshot);
                    trace.output = Some(format_response_for_trace(&r.message));
                }
                llm_calls.push(trace);
                r.message
            }
            Err(e) => {
                #[allow(unused_mut)] // mutated only in debug builds (trace snapshots)
                let mut trace = build_tool_round_trace(round as i32, call_ms, None);
                #[cfg(debug_assertions)]
                {
                    trace.input = Some(input_snapshot);
                }
                llm_calls.push(trace);
                let msg = format!("tool-call round {} failed: {}", round, e);
                emit_log("warn", &msg);
                abort_error = Some(e.to_string());
                break;
            }
        };

        // Salvage tool calls that some small models (notably Qwen 3.5 4B)
        // emit as plain text instead of through the JSON tool_calls
        // channel. Two formats are recognised: `<tool_call>…</tool_call>`
        // XML envelopes, and `tool_call: name(args)` Python-call literals.
        // Swap any parse hits into the response as if they came through
        // normally — keeps the rest of the loop blissfully unaware of the
        // model quirk.
        let response = if response.tool_calls.as_ref().map(|tc| tc.is_empty()).unwrap_or(true) {
            let (parsed, kind) = salvage_text_tool_calls(&response.content, registry, db);
            if !parsed.is_empty() {
                let names: Vec<&str> = parsed.iter().map(|c| c.function.name.as_str()).collect();
                emit_log(
                    "warn",
                    &format!(
                        "model emitted tool call as text instead of a tool_call message — \
                         salvaged {} {}-format call(s): [{}]. This usually means the \
                         current model has weak tool-calling support.",
                        parsed.len(),
                        kind,
                        names.join(", ")
                    ),
                );
                AiMessage {
                    role: response.role,
                    content: String::new(),
                    tool_calls: Some(parsed),
                }
            } else {
                response
            }
        } else {
            response
        };

        let tool_calls = match &response.tool_calls {
            Some(tc) if !tc.is_empty() => tc.clone(),
            _ => {
                // No tool calls. If no tool has actually executed yet, this is
                // very likely a small-model failure mode: the model wrote
                // something like "Voy a buscar el contacto…" instead of
                // emitting the tool_call. Nudge it once and continue the loop
                // before accepting the bare text as the final answer.
                //
                // Only when `force_tool_use` is set: in tool-first chat a turn
                // is expected to call a tool, so a bare-text reply is a failure
                // worth nudging. Thread-bound chat is the opposite — most turns
                // are plain Q&A over the seeded thread and should accept the
                // direct answer; nudging there would push the model to call the
                // lone draft tool when the user only asked a question.
                if !round_may_stream_live(force_tool_use, tool_traces.is_empty(), nudges_used, MAX_NUDGES) {
                    nudges_used += 1;
                    emit_log(
                        "info",
                        "tool_loop: model returned text with no tool call \
                         and no tool has executed yet — nudging once",
                    );
                    // Keep the model's announcement in history so its next
                    // turn can reference what it said it would do, then push
                    // a corrective user message asking for the actual call.
                    messages.push(response);
                    messages.push(AiMessage {
                        role: "user".to_string(),
                        content: "No describas lo que vas a hacer — llama \
                                  directamente a la tool apropiada AHORA \
                                  (search_contacts, search_emails, etc.). \
                                  Si realmente no puedes responder con las \
                                  tools disponibles, dilo explícitamente en \
                                  una sola frase.\n\nDo not describe what \
                                  you are going to do — call the appropriate \
                                  tool NOW. If you genuinely cannot answer \
                                  with the available tools, say so plainly \
                                  in a single sentence."
                            .to_string(),
                        tool_calls: None,
                    });
                    continue;
                }
                // No tool calls — model gave a direct answer. If this round
                // streamed its prose live, record that so the caller skips
                // re-emitting it as a single token. Push and break.
                had_any_answer = true;
                answer_streamed_live = last_round_streamed_live;
                messages.push(response);
                break;
            }
        };

        // Deterministic repair: a filterless search_emails call gets the email
        // address the user wrote verbatim injected before dispatch (when the
        // question names exactly one), instead of bouncing a validation error
        // off a model that tends to retry with the same empty — or a mangled —
        // filter. Runs before the no-progress guard so dedup keys see the
        // repaired args.
        let mut tool_calls = tool_calls;
        for tc in &mut tool_calls {
            if tc.function.name == "search_emails"
                && repair_filterless_search_args(&mut tc.function.arguments, user_question)
            {
                emit_log(
                    "info",
                    &format!(
                        "tool_loop: search_emails had no filters — injected address from the question ({})",
                        truncate_chars(&tc.function.arguments.to_string(), 200)
                    ),
                );
            }
            if tc.function.name == "get_email_body" {
                // Track explicit reads; repair id-less reads with the next
                // unread search result (Qwen 3.6 batches body reads but drops
                // every email_id).
                if let Some(id) = tc
                    .function
                    .arguments
                    .get("email_id")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    read_body_ids.insert(id.to_string());
                } else if let Some(injected) =
                    repair_missing_email_id(&mut tc.function.arguments, &aggregated_email_refs, &read_body_ids)
                {
                    read_body_ids.insert(injected.clone());
                    emit_log(
                        "info",
                        &format!(
                            "tool_loop: get_email_body had no email_id — injected next unread result ({injected})"
                        ),
                    );
                }
            }
        }

        // No-progress guard: if every call this round was already executed with
        // identical args, the model is spinning (re-searching instead of
        // answering). Break to synthesis from the results we already have rather
        // than waste the rest of the round budget on duplicates.
        if tool_calls
            .iter()
            .all(|tc| executed_tool_keys.contains(&tool_call_key(tc)))
        {
            emit_log(
                "info",
                "tool_loop: model repeated already-executed tool call(s) with no new args \
                 — synthesising from existing results instead of looping",
            );
            break;
        }

        // Push the assistant's tool-call message so the history stays coherent.
        had_any_answer = true;
        messages.push(response);

        for tc in &tool_calls {
            executed_tool_keys.insert(tool_call_key(tc));
            let name = &tc.function.name;
            let args = &tc.function.arguments;

            // Surfaced in the output panel so users can see exactly which tools
            // the model is calling and with what arguments. Info level (not debug)
            // because this is the main observability hook for chat retrieval.
            let args_str = serde_json::to_string(args).unwrap_or_else(|_| args.to_string());
            emit_log(
                "info",
                &format!("tool call → {}({})", name, truncate_chars(&args_str, 400)),
            );

            // Tool-specific status ("Searching emails" / "Generating draft" /
            // …) so the UI reflects what each call is doing instead of a flat
            // "Running tools" for the whole loop.
            emit_phase(conversation_id, message_id, phase_for_tool(name));
            let t_tool = std::time::Instant::now();
            let dispatched =
                dispatch_tool(registry, db, account_id, categories, user_question, name, args.clone()).await;
            let elapsed_ms = t_tool.elapsed().as_millis() as i64;
            let traced_args = dispatched.corrected_args.unwrap_or_else(|| args.clone());
            let result = dispatched.text;
            for id in dispatched.email_refs {
                if seen_email_refs.insert(id.clone()) {
                    aggregated_email_refs.push(id);
                }
            }
            for id in dispatched.draft_refs {
                if seen_draft_refs.insert(id.clone()) {
                    aggregated_draft_refs.push(id);
                }
            }

            emit_log(
                "info",
                &format!("tool result ← {} ({} chars, {}ms)", name, result.len(), elapsed_ms),
            );

            // Record the call for the reasoning trace. We keep a generous cap
            // (16 KiB) so the eval report's expandable panel can show the full
            // tool output for debugging — large enough for a typical thread or
            // 25-row search_emails dump, small enough to bound the JSON blob.
            tool_traces.push(ToolCallTrace {
                name: name.clone(),
                round: round as i32,
                arguments: traced_args,
                result_preview: truncate_chars(&result, 16000),
                result_chars: result.len() as i32,
                elapsed_ms,
            });

            messages.push(AiMessage {
                role: "tool".to_string(),
                content: result,
                tool_calls: None,
            });
        }
    }

    ToolLoopOutcome {
        messages,
        failed_without_answer: !had_any_answer,
        error: abort_error,
        aggregated_email_refs,
        aggregated_draft_refs,
        answer_streamed_live,
    }
}

// ── Orchestration ───────────────────────────────────────────────────────────

/// Which grounding strategy a single chat turn should use.
///
/// Produced by [`plan_turn_mode`] and consumed at the top of [`run_chat_turn`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ChatTurnMode {
    /// The conversation itself was seeded with a thread (via
    /// `create_conversation_with_thread`) — every turn is bound to it.
    ConversationThread,
    /// The user is looking at this thread in the main view right now; ground
    /// only THIS turn in it.
    AmbientThread {
        /// Thread to hydrate.
        thread_id: String,
        /// Account that owns `thread_id`, when the caller knows it.
        ///
        /// Required because the chat surface's account is not necessarily the
        /// thread's: in unified ("All accounts") mode the panel runs on the
        /// first enabled account while the user reads a thread belonging to
        /// any of them. Looking the thread up under the wrong account finds
        /// nothing and silently drops the context. `None` means "use the
        /// turn's own account", which is correct for single-account callers.
        account_id: Option<String>,
    },
    /// No context — the normal route/retrieval/tool-loop pipeline.
    Rag,
}

/// Decide how to ground a turn, given whether the conversation carries seeded
/// system messages and which thread (if any) the main view currently shows.
///
/// Pure so the precedence rules are unit-testable without a DB or provider.
/// A conversation-level binding always wins: a chat explicitly created *about*
/// thread A must not silently re-point at thread B because the user scrolled
/// somewhere else while it was open.
pub(super) fn plan_turn_mode(
    system_message_count: usize,
    ambient_thread_id: Option<&str>,
    ambient_account_id: Option<&str>,
) -> ChatTurnMode {
    if system_message_count > 0 {
        return ChatTurnMode::ConversationThread;
    }
    match ambient_thread_id.map(str::trim) {
        Some(id) if !id.is_empty() => ChatTurnMode::AmbientThread {
            thread_id: id.to_string(),
            // Blank is treated as absent, same as the thread id: an empty
            // string would become a lookup that matches no account.
            account_id: ambient_account_id
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .map(str::to_string),
        },
        _ => ChatTurnMode::Rag,
    }
}

/// Word stems that mean "produce an email for me" across the four shipped UI
/// languages, accent-folded and lower-cased. Matched as *prefixes of whole
/// words* so `redact` catches `redacta`/`redactar` without `borrador` firing
/// inside an unrelated word.
///
/// Deliberately verb-led: bare nouns like `respuesta` / `reply` are excluded
/// because they appear in ordinary questions about a thread ("¿qué respuesta
/// espera?"), which must be answered as text, not turned into a draft.
const DRAFT_INTENT_STEMS: &[&str] = &[
    // Spanish
    "borrador",
    "escrib",
    "redact",
    "respond",
    "contest",
    "responde",
    "contesta",
    // English
    "draft",
    "write",
    "compos",
    "reply",
    "replies",
    // German
    "entwurf",
    "schreib",
    "verfass",
    "antwort",
    // French
    "brouillon",
    "ecri",
    "redig",
];

/// Fold the accents that separate a Spanish/French imperative from its stem
/// (`respóndele` → `respondele`, `écris` → `ecris`) and lower-case.
fn fold_for_intent_match(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

/// Does this prompt explicitly ask for an email to be written?
///
/// Gates whether `generate_email_draft` is offered at all on a thread-bound
/// turn. Thread-bound mode exposes a single tool, and a model handed one tool
/// plus an imperative will use it: "traduceme este email al español" saved a
/// reply draft instead of translating the thread. Read-only intents
/// (translate, summarise, explain, "what does this say") match nothing here,
/// so the tool is never on the menu and the model can only answer in text.
///
/// Allow-list rather than block-list: an unrecognised phrasing degrades to a
/// text answer the user can follow up on, whereas an unrecognised *read-only*
/// phrasing under a block-list would silently save a junk draft — the failure
/// being fixed. Pure, so the multilingual matrix is unit-testable.
pub(super) fn wants_email_draft(prompt: &str) -> bool {
    let folded = fold_for_intent_match(prompt);
    folded
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .any(|word| DRAFT_INTENT_STEMS.iter().any(|stem| word.starts_with(stem)))
}

/// Assemble the system prompt for a thread-bound turn: the rendered
/// `chat.system` base, the seeded thread context message(s), and a tail
/// instruction that grounds the model in the thread.
///
/// Pure so the grounding/draft contract is unit-testable without a provider
/// or `AppHandle`. When `drafts_available` is true the model is told it may
/// call `generate_email_draft` (the only tool exposed in this mode) for
/// explicit draft/reply requests; otherwise it is told to use no tools at all.
fn build_thread_bound_system(base_system: &str, system_messages: &[ChatMessage], drafts_available: bool) -> String {
    let mut system = base_system.to_string();

    // Append each seeded system message (in practice exactly one — the cleaned
    // thread — but robust to future multi-context chats).
    for msg in system_messages {
        system.push_str("\n\n");
        system.push_str(&msg.content);
    }

    if drafts_available {
        system.push_str(
            "\n\nIMPORTANT: You are chatting about the email thread above. Ground \
every answer ONLY in that thread — do not invent facts or claim details the \
thread does not contain. If the thread lacks the information to answer, say so \
plainly.\n\nThe ONLY action you may take is calling `generate_email_draft`, and \
ONLY when the user explicitly asks you to draft, write, or reply to a message. \
For a reply, pass `email_id` set to the exact value shown as `(id: ...)` next \
to the message you are replying to in the thread above. Never invent an id. For \
a plain question, do NOT call any tool — just answer from the thread.",
        );
    } else {
        system.push_str(
            "\n\nIMPORTANT: You are chatting about the email thread above. Answer \
using ONLY that thread as your source. Do not search other emails. If the \
thread does not contain enough information to answer, say so plainly.\n\nYou \
have NO tools on this turn. Answer the request directly in your reply — \
translate, summarise, explain or quote the thread inline as asked. Never \
mention tools, tool names, or your own limitations: do not write phrases like \
\"the tool is not available\" or \"I cannot save drafts\". If the user wants an \
email written, simply invite them to ask you to draft a reply.",
        );
    }

    system
}

/// Run one chat turn for a "thread-bound" conversation — one that was seeded
/// with the cleaned content of an email thread (see
/// [`create_conversation_with_thread`]). Skips RAG retrieval because the thread
/// is already the entire context the user wants the model to consider, and
/// exposes a single tool — `generate_email_draft` — so the user can ask it to
/// draft a reply grounded in that thread.
///
/// Used as a short-circuit at the top of [`run_chat_turn`].
#[allow(clippy::too_many_arguments)]
async fn run_thread_bound_turn(
    db: Arc<Database>,
    provider: Arc<dyn AIProvider>,
    conversation_id: String,
    assistant_message_id: String,
    account_id: String,
    user_question: String,
    history: Vec<ChatMessage>,
    system_messages: Vec<ChatMessage>,
    turn_start: std::time::Instant,
) -> Result<()> {
    /// Bounded so a stuck local model can't leave the UI thinking forever.
    /// Matches the existing final-stream timeout in `run_chat_turn`.
    const STREAM_TIMEOUT: Duration = Duration::from_secs(180);
    /// Cap how many recent user/assistant turns we replay before the new
    /// question. The seeded thread already eats a chunk of context — we don't
    /// want a long back-and-forth to push the thread itself out.
    const THREAD_HISTORY_TURNS: usize = 12;

    emit_log(
        "info",
        &format!(
            "thinking about thread… (model={}, msgs={})",
            provider.model_name(),
            system_messages.len()
        ),
    );

    // Render the base chat.system template (today / tomorrow / language
    // instruction), then layer the thread context + grounding/draft contract
    // on top via the pure `build_thread_bound_system` helper.
    let language = crate::services::i18n::resolve_ai_language(&db)?;
    let language_instruction = format!("Reply in {}.", language.english_name());
    let today = now_utc().format("%Y-%m-%d").to_string();
    let tomorrow = (now_utc() + chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
    // Drafts are gated behind a Settings toggle (defaults ON) AND behind the
    // user actually asking for one this turn. Both must hold: with a single
    // tool on the menu the model treats any imperative as licence to use it,
    // so a read-only request ("traduceme este email al español") came back as
    // a saved reply draft instead of a translation. `wants_email_draft` keeps
    // the tool off the menu entirely unless the turn asks for an email.
    let drafts_available = db.is_ai_drafts_enabled().unwrap_or(true) && wants_email_draft(&user_question);

    // Registry first: the rendered tool section feeds the base template below.
    // An empty registry yields an empty section and a pure-text answer.
    let registry: Arc<tools::ToolRegistry> = Arc::new(if drafts_available {
        let draft_tool: Arc<dyn tools::Tool> = Arc::new(tools::generate_email_draft::GenerateEmailDraftTool);
        tools::ToolRegistry::with_tools(vec![draft_tool])
    } else {
        tools::ToolRegistry::with_tools(vec![])
    });

    // `chat.system` carries a `{{ tools_section }}` placeholder. Unknown
    // variables are left INTACT by `prompts::render` (prompts/mod.rs:197), so
    // omitting it here shipped a literal "{{ tools_section }}" to the model —
    // a placeholder where the tool menu belongs, which invited invented tool
    // calls even on turns with no tools at all. Always bind it.
    let mut tpl_vars = std::collections::HashMap::new();
    tpl_vars.insert("today", today);
    tpl_vars.insert("tomorrow", tomorrow);
    tpl_vars.insert("language_instruction", language_instruction);
    tpl_vars.insert("tools_section", registry.render_system_prompt_section(db.as_ref()));
    // Empty rather than omitted, for the same reason: the identity block only
    // explains how to map "I"/"me" onto `search_emails` filters, and this path
    // never searches. Binding it blank drops the section; omitting it would
    // ship a literal "{{ user_identity }}" to the model.
    tpl_vars.insert("user_identity", String::new());
    let system_template = crate::services::prompts::get_template(&db, "chat.system")?;
    let base_system = crate::services::prompts::render(&system_template, &tpl_vars);
    debug_assert!(
        !base_system.contains("{{"),
        "thread-bound system prompt has an unbound placeholder: {base_system}"
    );
    let system = build_thread_bound_system(&base_system, &system_messages, drafts_available);

    // Build the message list as (role, content) pairs for the tool loop:
    // system + last N user/assistant turns + the current question.
    let mut initial_messages: Vec<(String, String)> = Vec::with_capacity(history.len() + 2);
    initial_messages.push(("system".to_string(), system));
    let start = history.len().saturating_sub(THREAD_HISTORY_TURNS);
    for msg in &history[start..] {
        if msg.role == "user" || msg.role == "assistant" {
            initial_messages.push((msg.role.clone(), msg.content.clone()));
        }
    }
    initial_messages.push(("user".to_string(), user_question.clone()));

    let mut tool_traces: Vec<ToolCallTrace> = Vec::new();
    let mut llm_calls: Vec<LlmCallTrace> = Vec::new();

    emit_log("info", "stage: tool_loop (thread-bound)");
    emit_phase(&conversation_id, &assistant_message_id, ChatPhase::RunningTools);
    let t_tool_loop = std::time::Instant::now();
    // Don't force a tool call: thread-bound chat is mostly Q&A about the
    // thread, and only an explicit "draft a reply" should produce a draft.
    let outcome = run_tool_loop(
        &db,
        &registry,
        provider.as_ref(),
        &conversation_id,
        &assistant_message_id,
        &account_id,
        &[],
        &user_question,
        initial_messages,
        None,
        false,
        &mut tool_traces,
        &mut llm_calls,
    )
    .await;
    let tool_loop_ms = t_tool_loop.elapsed().as_millis() as i64;
    let aggregated_email_refs = outcome.aggregated_email_refs.clone();
    let aggregated_draft_refs = outcome.aggregated_draft_refs.clone();

    // Whether the loop already shipped the answer live (skip the single-token
    // re-emit below if so). Captured before `outcome` is partially moved.
    let answer_streamed_live = outcome.answer_streamed_live;

    // If the loop ended on a direct assistant text answer, reuse it as-is;
    // otherwise stream a synthesis pass over whatever messages it produced.
    let direct_answer: Option<String> = outcome.messages.last().and_then(|m| {
        let no_calls = m.tool_calls.as_ref().map(|v| v.is_empty()).unwrap_or(true);
        if m.role == "assistant" && no_calls && !m.content.trim().is_empty() {
            Some(m.content.clone())
        } else {
            None
        }
    });

    emit_phase(&conversation_id, &assistant_message_id, ChatPhase::Generating);
    let stream_result: Result<crate::ai::provider::ChatStreamResult> = if outcome.failed_without_answer {
        let detail = outcome
            .error
            .clone()
            .unwrap_or_else(|| "tool-call loop failed before producing any answer".to_string());
        Err(AppError::AiError(detail))
    } else if let Some(answer) = direct_answer {
        // Strip any leaked tool-call markup before it reaches the bubble.
        let answer = strip_tool_call_markup(&answer);
        // Emit the whole answer as one stream token, then synthesise a
        // successful result (no extra round-trip to the model). Skip the emit
        // when the loop already streamed the answer live, or the bubble doubles.
        if !answer_streamed_live {
            crate::services::events::emit(
                "chat-stream",
                ChatStreamEvent {
                    message_id: assistant_message_id.clone(),
                    conversation_id: conversation_id.clone(),
                    token: answer.clone(),
                    done: false,
                    error: None,
                    token_count: None,
                    latency_ms: None,
                    replace: None,
                },
            );
        }
        Ok(crate::ai::provider::ChatStreamResult {
            content: answer,
            eval_count: None,
            prompt_eval_count: None,
            prefill_ms: None,
            cached_prompt_tokens: None,
            prefix_plan: None,
            sys_cached_before: None,
            sys_cached_after: None,
            system_prefix_tokens: None,
            stable_tokens: None,
            dropped_front_tokens: None,
        })
    } else {
        // Loop produced a non-text final message (e.g. hit MAX_TOOL_ROUNDS).
        // Stream a synthesis pass, bounded by STREAM_TIMEOUT.
        let conv_id_for_stream = conversation_id.clone();
        let msg_id_for_stream = assistant_message_id.clone();
        let ai_messages: Vec<AiMessage> = outcome
            .messages
            .into_iter()
            .map(|m| AiMessage {
                role: m.role,
                content: m.content,
                tool_calls: None,
            })
            .collect();
        // Gate the live stream so any tool-call markup the backend emits as raw
        // text is suppressed before it reaches the bubble (see the tools-first
        // synthesis path for the rationale).
        let gate = Arc::new(std::sync::Mutex::new(crate::ai::stream_gate::StreamGate::new()));
        let gate_for_token = gate.clone();
        let conv_for_token = conv_id_for_stream.clone();
        let msg_for_token = msg_id_for_stream.clone();
        let stream_fut = provider.chat_stream(
            ai_messages,
            Box::new(move |token| {
                let forward = gate_for_token.lock().map(|mut g| g.push(&token)).unwrap_or(token);
                if !forward.is_empty() {
                    crate::services::events::emit(
                        "chat-stream",
                        ChatStreamEvent {
                            message_id: msg_for_token.clone(),
                            conversation_id: conv_for_token.clone(),
                            token: forward,
                            done: false,
                            error: None,
                            token_count: None,
                            latency_ms: None,
                            replace: None,
                        },
                    );
                }
                true
            }),
        );
        let res = match timeout(STREAM_TIMEOUT, stream_fut).await {
            Ok(res) => res,
            Err(_) => Err(AppError::AiError(format!(
                "Streaming answer exceeded {}s — model may be stuck. Try a smaller model.",
                STREAM_TIMEOUT.as_secs()
            ))),
        };
        if let Ok(mut g) = gate.lock() {
            let tail = g.finish();
            if !tail.is_empty() {
                crate::services::events::emit(
                    "chat-stream",
                    ChatStreamEvent {
                        message_id: msg_id_for_stream.clone(),
                        conversation_id: conv_id_for_stream.clone(),
                        token: tail,
                        done: false,
                        error: None,
                        token_count: None,
                        latency_ms: None,
                        replace: None,
                    },
                );
            }
        }
        res
    };

    let latency_ms = turn_start.elapsed().as_millis() as i64;

    match stream_result {
        Ok(mut result) => {
            // Safety net: strip any tool-call markup that leaked into the final
            // text before persisting, so a reload shows the cleaned-up answer.
            result.content = strip_tool_call_markup(&result.content);
            let token_count = result.eval_count.map(|c| c as i32);
            if let Err(e) =
                db.update_chat_message_completion(&assistant_message_id, &result.content, token_count, Some(latency_ms))
            {
                emit_log("error", &format!("failed to persist assistant message: {e}"));
            }
            // Persist + ship the email/draft allowlists so the frontend chip
            // validator accepts the `draft://DRAFT_ID` the model emitted for
            // the reply instead of dropping it as hallucinated.
            if let Err(e) = db.update_chat_message_referenced_emails(&assistant_message_id, &aggregated_email_refs) {
                emit_log("error", &format!("failed to persist email refs: {e}"));
            }
            if let Err(e) = db.update_chat_message_referenced_drafts(&assistant_message_id, &aggregated_draft_refs) {
                emit_log("error", &format!("failed to persist draft refs: {e}"));
            }

            // Emit a minimal reasoning trace so the frontend receives the
            // referenced-draft allowlist live (the `chat-trace` event is the
            // delivery hook for it). The route is synthetic: a thread-bound
            // turn is a forced tools-first short-circuit.
            let trace = ChatTrace {
                route: RouteDecision {
                    mode: RouteMode::ToolsFirst,
                    reason: "thread-bound".to_string(),
                    matched_keywords: vec![],
                    classifier: "forced".to_string(),
                },
                retrieval: None,
                tool_calls: tool_traces.clone(),
                model: provider.model_name().to_string(),
                total_elapsed_ms: latency_ms,
                tool_loop_ms,
                llm_streaming_ms: None,
                llm_calls: llm_calls.clone(),
            };
            if let Err(e) = db.update_chat_message_trace(&assistant_message_id, &trace) {
                emit_log("error", &format!("failed to persist reasoning trace: {e}"));
            }
            crate::services::events::emit(
                "chat-trace",
                ChatTraceEvent {
                    message_id: assistant_message_id.clone(),
                    conversation_id: conversation_id.clone(),
                    trace,
                    referenced_email_ids: aggregated_email_refs.clone(),
                    referenced_draft_ids: aggregated_draft_refs.clone(),
                },
            );

            crate::services::events::emit(
                "chat-stream",
                ChatStreamEvent {
                    message_id: assistant_message_id.clone(),
                    conversation_id: conversation_id.clone(),
                    token: String::new(),
                    done: true,
                    error: None,
                    token_count,
                    latency_ms: Some(latency_ms),
                    replace: None,
                },
            );
            let tokens_str = token_count
                .map(|c| format!("{c} tokens"))
                .unwrap_or_else(|| "? tokens".to_string());
            emit_log(
                "success",
                &format!(
                    "thread reply complete ({}, {:.1}s)",
                    tokens_str,
                    latency_ms as f64 / 1000.0
                ),
            );
            Ok(())
        }
        Err(e) => {
            let err_text = format!("Chat failed: {e}");
            let _ = db.update_chat_message_completion(&assistant_message_id, &err_text, None, Some(latency_ms));
            crate::services::events::emit(
                "chat-stream",
                ChatStreamEvent {
                    message_id: assistant_message_id.clone(),
                    conversation_id: conversation_id.clone(),
                    token: String::new(),
                    done: true,
                    error: Some(err_text.clone()),
                    token_count: None,
                    latency_ms: Some(latency_ms),
                    replace: None,
                },
            );
            emit_log("error", &err_text);
            Err(e)
        }
    }
}

/// Run one chat turn: retrieve, persist sources, stream tokens, persist final content.
///
/// `assistant_message_id` must already exist in the DB (created empty by the
/// command layer) so the frontend can subscribe to events keyed off it before
/// any tokens arrive. `user_message_id` is this turn's user row: once the
/// final prompt tail is assembled it is persisted there as `prompt_content`
/// so later turns replay it byte-identically (KV prefix reuse).
#[allow(clippy::too_many_arguments)]
/// Run one tools-free synthesis stream, forwarding gated prose tokens to the
/// `chat-stream` event channel and flushing the gate's held-back tail at the
/// end. Extracted from the `StreamSynthesis` branch of [`run_chat_turn`] so
/// the empty-answer retry reuses the exact same gating/emit path as the first
/// attempt.
///
/// The gate matters because backends that parse tool calls out of the raw
/// stream (llama.cpp) can emit tool-call syntax here as plain text: the gate
/// forwards genuine prose and suppresses any tool-call markup — prose-then-tag
/// included — so it never reaches the bubble. The returned content is cleaned
/// separately with `strip_tool_call_markup` before persistence.
async fn run_gated_synthesis_stream(
    provider: &dyn AIProvider,
    messages: Vec<AiMessage>,
    conversation_id: &str,
    assistant_message_id: &str,
    stream_timeout: std::time::Duration,
) -> Result<crate::ai::provider::ChatStreamResult> {
    let gate = Arc::new(std::sync::Mutex::new(crate::ai::stream_gate::StreamGate::new()));
    let gate_for_token = gate.clone();
    let conv_for_token = conversation_id.to_string();
    let msg_for_token = assistant_message_id.to_string();
    let stream_fut = provider.chat_stream(
        messages,
        Box::new(move |token| {
            // On the (unreachable) lock-poison case, forward the raw token
            // rather than drop content — persistence still strips markup as
            // the final safety net.
            let forward = gate_for_token.lock().map(|mut g| g.push(&token)).unwrap_or(token);
            if !forward.is_empty() {
                crate::services::events::emit(
                    "chat-stream",
                    ChatStreamEvent {
                        message_id: msg_for_token.clone(),
                        conversation_id: conv_for_token.clone(),
                        token: forward,
                        done: false,
                        error: None,
                        token_count: None,
                        latency_ms: None,
                        replace: None,
                    },
                );
            }
            true
        }),
    );
    let res = match timeout(stream_timeout, stream_fut).await {
        Ok(res) => res,
        Err(_) => Err(AppError::AiError(format!(
            "Streaming answer exceeded {}s — model may be stuck. Try a smaller model.",
            stream_timeout.as_secs()
        ))),
    };
    // Flush any prose the gate held back while waiting to see if a trailing
    // chunk completed a tool-call tag.
    if let Ok(mut g) = gate.lock() {
        let tail = g.finish();
        if !tail.is_empty() {
            crate::services::events::emit(
                "chat-stream",
                ChatStreamEvent {
                    message_id: assistant_message_id.to_string(),
                    conversation_id: conversation_id.to_string(),
                    token: tail,
                    done: false,
                    error: None,
                    token_count: None,
                    latency_ms: None,
                    replace: None,
                },
            );
        }
    }
    res
}

/// Cap on tool calls salvaged from one empty synthesis attempt (a model that
/// batches several calls into one leaked block).
const MAX_SALVAGED_SYNTHESIS_CALLS: usize = 3;

/// Cap on salvage-and-execute rounds in the recovery ladder. Observed on
/// Qwen 3.6 35B: the first leaked call can be degenerate (`search_emails({})`
/// → validation error) with only the SECOND attempt carrying usable args, so
/// one round is not always enough. Two rounds + one corrective retry bound the
/// ladder at 4 model calls total.
const MAX_SYNTHESIS_RECOVERY_ROUNDS: usize = 2;

/// Outcome of [`synthesize_with_recovery`]: the (possibly retried) stream
/// result plus any email/draft refs contributed by salvaged tool calls, which
/// the caller must fold into the turn's citation allowlists.
struct SynthesisRecovery {
    result: Result<crate::ai::provider::ChatStreamResult>,
    email_refs: Vec<String>,
    draft_refs: Vec<String>,
}

/// Run the final tools-free synthesis stream with an empty-answer recovery
/// ladder. A weak or stubborn model can answer the synthesis with nothing but
/// tool-call markup — the gate suppresses it and the user would get a silently
/// empty bubble. On each empty attempt, in order:
///
/// 1. If the empty answer contains salvageable tool call(s) — named blocks,
///    python-call literals, or nameless blocks resolved by arg-key inference
///    (the model is telling us exactly what it needs) — execute them and
///    re-synthesise from their results. At most
///    [`MAX_SYNTHESIS_RECOVERY_ROUNDS`] such rounds.
/// 2. Otherwise retry ONCE with a corrective "answer in prose NOW"
///    instruction.
/// 3. Budget exhausted: hand the empty result back — the caller ships a
///    localized hint (see `empty_answer_hint`) instead of a blank bubble.
#[allow(clippy::too_many_arguments)]
async fn synthesize_with_recovery(
    provider: &dyn AIProvider,
    registry: &tools::ToolRegistry,
    db: &Arc<Database>,
    account_id: &str,
    categories: &[String],
    user_question: &str,
    synthesis_messages: Vec<AiMessage>,
    conversation_id: &str,
    assistant_message_id: &str,
    stream_timeout: std::time::Duration,
    llm_calls: &mut Vec<LlmCallTrace>,
    tool_traces: &mut Vec<ToolCallTrace>,
) -> SynthesisRecovery {
    let mut prompt_messages = synthesis_messages;
    let mut email_refs: Vec<String> = Vec::new();
    let mut draft_refs: Vec<String> = Vec::new();
    let mut salvage_rounds = 0usize;
    let mut corrective_done = false;

    loop {
        // Keep a copy so the next ladder step can re-prompt from this context.
        let retry_base = prompt_messages.clone();
        let t_attempt = std::time::Instant::now();
        let attempt = run_gated_synthesis_stream(
            provider,
            prompt_messages,
            conversation_id,
            assistant_message_id,
            stream_timeout,
        )
        .await;

        let empty_result = match attempt {
            Ok(r) if strip_tool_call_markup(&r.content).trim().is_empty() => r,
            // Healthy prose or a hard stream error — done either way.
            other => {
                return SynthesisRecovery {
                    result: other,
                    email_refs,
                    draft_refs,
                }
            }
        };

        // Record the empty attempt as its own trace entry so the reasoning
        // panel shows each call instead of one call with accumulated latency.
        #[allow(unused_mut)] // mutated only in debug builds (output snapshot)
        let mut empty_trace = build_final_stream_trace(t_attempt.elapsed().as_millis() as i64, Some(&empty_result));
        empty_trace.kind = "final_stream_empty".to_string();
        #[cfg(debug_assertions)]
        {
            empty_trace.output = Some(empty_result.content.clone());
        }
        llm_calls.push(empty_trace);

        let (salvaged_all, salvage_kind) = salvage_text_tool_calls(&empty_result.content, registry, db);
        let mut salvaged: Vec<crate::ai::provider::AiToolCall> =
            salvaged_all.into_iter().take(MAX_SALVAGED_SYNTHESIS_CALLS).collect();
        // Same deterministic repair as the tool loop: a filterless (or
        // empty-args) salvaged search gets the question's verbatim address.
        for tc in &mut salvaged {
            if tc.function.name == "search_emails" {
                repair_filterless_search_args(&mut tc.function.arguments, user_question);
            }
        }

        prompt_messages = retry_base;
        if !salvaged.is_empty() && salvage_rounds < MAX_SYNTHESIS_RECOVERY_ROUNDS {
            salvage_rounds += 1;
            emit_log(
                "info",
                &format!(
                    "final synthesis leaked {} {salvage_kind}-format tool call(s) instead of prose — \
executing and re-synthesising (round {salvage_rounds}/{MAX_SYNTHESIS_RECOVERY_ROUNDS})",
                    salvaged.len()
                ),
            );
            prompt_messages.push(AiMessage {
                role: "assistant".to_string(),
                content: String::new(),
                tool_calls: Some(salvaged.clone()),
            });
            for tc in &salvaged {
                let t_tool = std::time::Instant::now();
                let dispatched = dispatch_tool(
                    registry,
                    db,
                    account_id,
                    categories,
                    user_question,
                    &tc.function.name,
                    tc.function.arguments.clone(),
                )
                .await;
                tool_traces.push(ToolCallTrace {
                    name: tc.function.name.clone(),
                    // Salvaged from the final synthesis stream, after the loop.
                    round: -3,
                    arguments: dispatched
                        .corrected_args
                        .clone()
                        .unwrap_or_else(|| tc.function.arguments.clone()),
                    result_preview: truncate_chars(&dispatched.text, 16000),
                    result_chars: dispatched.text.len() as i32,
                    elapsed_ms: t_tool.elapsed().as_millis() as i64,
                });
                email_refs.extend(dispatched.email_refs);
                draft_refs.extend(dispatched.draft_refs);
                prompt_messages.push(AiMessage {
                    role: "tool".to_string(),
                    content: dispatched.text,
                    tool_calls: None,
                });
            }
            prompt_messages.push(AiMessage {
                role: "user".to_string(),
                content: SYNTHESIS_SALVAGED_TOOL_INSTRUCTION.to_string(),
                tool_calls: None,
            });
        } else if !corrective_done {
            corrective_done = true;
            emit_log(
                "info",
                "final synthesis came back empty — retrying once with a corrective instruction",
            );
            prompt_messages.push(AiMessage {
                role: "user".to_string(),
                content: SYNTHESIS_RETRY_INSTRUCTION.to_string(),
                tool_calls: None,
            });
        } else {
            // Ladder exhausted: return the empty result so the caller ships
            // the localized hint instead of a blank bubble.
            return SynthesisRecovery {
                result: Ok(empty_result),
                email_refs,
                draft_refs,
            };
        }
    }
}

pub async fn run_chat_turn(
    db: Arc<Database>,
    registry: Arc<tools::ToolRegistry>,
    conversation_id: String,
    user_message_id: String,
    assistant_message_id: String,
    account_id: String,
    user_question: String,
    model: String,
    history: Vec<ChatMessage>,
    categories: Vec<String>,
    ambient_thread_id: Option<String>,
    // Account owning `ambient_thread_id`, when the caller knows it. See
    // `ChatTurnMode::AmbientThread` for why this cannot be assumed to equal
    // `account_id`.
    ambient_account_id: Option<String>,
) -> Result<()> {
    let turn_start = std::time::Instant::now();

    // Build the configured AI provider from DB preferences, but let the
    // per-turn `model` argument (CLI `--model`, REPL `/model`, eval case model)
    // override the stored `ai_model` so the requested model actually drives the
    // runtime. Falls back to the preference when `model` is empty, and to Ollama
    // if the provider preference is missing or unrecognised.
    let provider = AiService::load_provider_with_model(&db, Some(&model))?;
    // Resolved once per turn rather than per retrieval: on a backend that
    // cannot embed this constructs the local embedder, and doing that inside
    // the retrieval leg would pay the model load on the clock the vector
    // timeout is measuring.
    let embedder = AiService::embedder_for(&db, &provider);

    // ── Thread-bound short-circuit ─────────────────────────────────────
    // Two ways a turn can be grounded in a single thread instead of running
    // the full route/retrieval/tool-loop pipeline:
    //   1. The conversation was seeded with one (`create_conversation_with_thread`).
    //   2. The chat panel passed the thread the user currently has open in the
    //      main view as ambient context for this turn only.
    // `plan_turn_mode` owns the precedence between them.
    let system_messages = db.get_chat_system_messages(&conversation_id).unwrap_or_default();
    let thread_context: Option<Vec<ChatMessage>> = match plan_turn_mode(
        system_messages.len(),
        ambient_thread_id.as_deref(),
        ambient_account_id.as_deref(),
    ) {
        ChatTurnMode::ConversationThread => Some(system_messages),
        ChatTurnMode::AmbientThread {
            thread_id,
            account_id: ambient_account,
        } => {
            // Look the thread up under ITS OWN account when the caller
            // supplied one. In unified mode the turn's `account_id` is
            // just the first enabled account and usually does not own the
            // open thread, which found nothing and dropped the context.
            let lookup_account = ambient_account.as_deref().unwrap_or(&account_id);
            // Build the context fresh for this turn. A failure here (thread
            // deleted mid-session, unreadable rows) must not kill the turn —
            // fall back to normal retrieval so the user still gets an answer.
            match super::conversations::build_thread_context(&db, lookup_account, &thread_id) {
                Ok((context, _subject)) => Some(vec![ChatMessage::ephemeral_system(&conversation_id, &context)]),
                Err(e) => {
                    // Surface this, don't just log it. The answer that
                    // follows is ungrounded but sounds confident ("which
                    // email do you mean?"), so a silent downgrade reads as
                    // the model being broken rather than the context being
                    // dropped.
                    emit_log(
                        "error",
                        "The open email couldn't be used as context for this answer — answering from search instead.",
                    );
                    emit_log(
                        "warn",
                        &format!("ambient thread context unavailable ({e}) — falling back to retrieval"),
                    );
                    None
                }
            }
        }
        ChatTurnMode::Rag => None,
    };

    if let Some(system_messages) = thread_context {
        return run_thread_bound_turn(
            db,
            provider,
            conversation_id,
            assistant_message_id,
            account_id,
            user_question,
            history,
            system_messages,
            turn_start,
        )
        .await;
    }

    emit_log(
        "info",
        &format!("thinking… (model={}, account={})", provider.model_name(), account_id),
    );

    // ── 0. Auto-derive conversation title from the first user turn ──────
    // If the conversation title is still the default placeholder and this turn
    // has no prior history, use the user's message as the title and notify the
    // UI so the sidebar updates without waiting for a reload.
    if history.is_empty() {
        match db.get_chat_conversation(&conversation_id) {
            Ok(Some(conv)) if title_is_default(&conv.title) => {
                let new_title = derive_title(&user_question);
                match db.rename_chat_conversation(&conversation_id, &new_title) {
                    Ok(_) => {
                        crate::services::events::emit(
                            "chat-renamed",
                            ChatRenamedEvent {
                                conversation_id: conversation_id.clone(),
                                title: new_title.clone(),
                            },
                        );
                    }
                    Err(e) => emit_log("error", &format!("auto-title rename failed: {}", e)),
                }
            }
            Ok(_) => {}
            Err(e) => emit_log("error", &format!("auto-title lookup failed: {}", e)),
        }
    }

    // ── 1. Classify route: RAG-first vs tools-first ─────────────────────
    // Stage markers are info-level on purpose: the chat panel filters by level
    // and "I can't tell where it's stuck" is the most common chat support
    // question. Per-stage timing follows on completion.
    let t_route = std::time::Instant::now();
    emit_log("info", "stage: route");
    emit_phase(&conversation_id, &assistant_message_id, ChatPhase::Routing);
    let route = classify_route(&db, &user_question, &history);
    emit_log(
        "info",
        &format!(
            "route: {:?} ({}) [{}ms]",
            route.mode,
            route.reason,
            t_route.elapsed().as_millis()
        ),
    );

    // Heuristic shortcut: recognise common phrasings and pre-seed the tool
    // call so we can skip the LLM's tool-choice round entirely. Returns None
    // for anything that doesn't match, in which case the normal loop runs.
    let calendar_available = db
        .get_account(&account_id)
        .ok()
        .flatten()
        .map(|a| {
            crate::sync::calendar_provider::provider_supports_calendar(&a.provider)
                && db.calendar_enabled(&a.id).unwrap_or(false)
        })
        .unwrap_or(false);
    let mut preseeded_tool_calls = heuristic_direct_tools(&user_question, calendar_available);
    if preseeded_tool_calls.is_some() {
        emit_log("info", "shortcut: matched direct-tool pattern");
    }

    // The active account address resolves first-person references ("emails I
    // sent") in both the query planner below and the system prompt's identity
    // line. A lookup failure degrades gracefully (no from/to resolution, no
    // identity line) rather than failing the turn.
    let user_email = db
        .get_account(&account_id)
        .ok()
        .flatten()
        .map(|a| a.email)
        .unwrap_or_default();

    // Query planner (tools-first fast path): when no heuristic matched and the
    // route is tools-first, ask the model — in ONE small completion on the
    // already-loaded chat provider, so no model swap — to turn the question into
    // a single search_emails filter. A concrete filter is pre-seeded as round-0
    // (the chat model then goes straight to synthesis, skipping the slow
    // tool-choice round); anything else (a write/draft/multi-step ask, an
    // unparseable reply, a provider error) defers to the normal loop. Gated by
    // the `chat.planner_enabled` preference (default on).
    // Trace entry for the planner LLM call, prepended to `llm_calls` below so it
    // shows in the flow timeline ahead of the tool rounds.
    let mut planner_trace: Option<LlmCallTrace> = None;
    if preseeded_tool_calls.is_none() && route.mode == RouteMode::ToolsFirst && planner_enabled(&db) {
        let template = crate::services::prompts::get_template(&db, "chat.query_plan")?;
        let today = now_utc().format("%Y-%m-%d").to_string();
        let t_plan = std::time::Instant::now();
        let plan = super::planner::plan_search(provider.as_ref(), &template, &user_email, &today, &user_question).await;
        let plan_ms = t_plan.elapsed().as_millis() as i64;
        match plan {
            super::planner::Plan::Search(plan) => {
                emit_log("info", &format!("planner: pre-seeded search_emails [{plan_ms}ms]"));
                planner_trace = Some(build_planner_trace(plan_ms, "search"));
                preseeded_tool_calls = Some(vec![plan.into_tool_call()]);
            }
            super::planner::Plan::Defer => {
                emit_log("debug", &format!("planner: deferred to model loop [{plan_ms}ms]"));
                planner_trace = Some(build_planner_trace(plan_ms, "defer"));
            }
        }
    }

    // ── 2. Retrieve sources (skipped entirely when route == ToolsFirst) ─
    // Match by reference so we can still read `route` later when assembling the
    // final ChatTrace.
    let t_retrieve = std::time::Instant::now();
    emit_log("info", "stage: retrieve");
    let (sources, retrieval_trace): (Vec<ScoredEmail>, Option<RetrievalTrace>) = match &route.mode {
        RouteMode::ToolsFirst => {
            emit_log("info", "retrieve: skipped (ToolsFirst route)");
            (Vec::new(), None)
        }
        RouteMode::RagFirst => {
            emit_phase(&conversation_id, &assistant_message_id, ChatPhase::Retrieving);
            match retrieve_context_with_trace(
                &db,
                provider.as_ref(),
                embedder.as_deref(),
                &account_id,
                &user_question,
                &categories,
                TOP_K_SOURCES,
            )
            .await
            {
                Ok((srcs, trace)) => {
                    emit_log(
                        "info",
                        &format!(
                            "retrieve: {} sources (vec={} fts={} fused→{}) [{}ms]",
                            srcs.len(),
                            trace.vector_hits,
                            trace.fts_hits,
                            trace.fused_top_k,
                            t_retrieve.elapsed().as_millis()
                        ),
                    );
                    (srcs, Some(trace))
                }
                Err(e) => {
                    emit_log("error", &format!("retrieval error: {}", e));
                    (Vec::new(), None)
                }
            }
        }
    };

    // ── 3. Persist citations + notify frontend ───────────────────────────
    // Include denormalized email metadata so the UI can render source details
    // (subject, sender, date) without extra API calls.
    // Keep the excerpt a bit shorter than what we hand to the LLM so the
    // "sources used" UI shows a readable preview, not a 4KB wall of text.
    const SOURCES_EXCERPT_CHARS: usize = 600;
    let source_rows: Vec<ChatMessageSource> = sources
        .iter()
        .map(|s| {
            let body_text = strip_html_for_fts(&s.body);
            let excerpt = smart_body_slice(&body_text, &user_question, SOURCES_EXCERPT_CHARS);
            ChatMessageSource {
                citation_number: s.citation_number,
                email_id: s.email.id.clone(),
                relevance_score: Some(s.score),
                subject: s.email.subject.clone(),
                sender: s.email.sender.clone(),
                sender_email: s.email.sender_email.clone(),
                timestamp: s.email.timestamp,
                body_excerpt: if excerpt.is_empty() { None } else { Some(excerpt) },
            }
        })
        .collect();

    if let Err(e) = db.insert_chat_message_sources(&assistant_message_id, &source_rows) {
        emit_log("error", &format!("failed to persist citations: {}", e));
    }

    crate::services::events::emit(
        "chat-sources",
        ChatSourcesEvent {
            message_id: assistant_message_id.clone(),
            conversation_id: conversation_id.clone(),
            sources: source_rows.clone(),
        },
    );

    // ── 4. Tool-call loop + streaming reply ─────────────────────────────
    let ai_language = crate::services::i18n::resolve_ai_language(&db)?;
    let system_template = crate::services::prompts::get_template(&db, "chat.system")?;
    // Tools section is rendered from the registry so it stays in lockstep
    // with what `definitions(&db)` advertises to the LLM via the
    // function-calling menu. Disabling a feature in Settings instantly
    // removes its tools from BOTH places — no template edit needed.
    let tools_section = registry.render_system_prompt_section(db.as_ref());
    // `user_email` was resolved earlier (before the query planner) and feeds the
    // system prompt's first-person identity line here.
    let mut initial_messages = build_prompt(
        &sources,
        &history,
        &user_question,
        ai_language.english_name(),
        &user_email,
        &system_template,
        &tools_section,
    );

    // Inject the memory header into the final user message — but only when
    // the user has the Memory feature enabled. Disabling it in Settings
    // should remove `<memory>...</memory>` from the prompt entirely (the
    // user can verify this in the reasoning panel). It rides with the
    // per-turn tail (not the system message) because it is derived from the
    // current question and would otherwise break the cross-turn KV prefix.
    // Pure SQLite reads → negligible latency. Errors degrade to no-header
    // rather than failing the turn.
    let memory_enabled = db
        .get_preference("memory_enabled")
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(false);
    if memory_enabled {
        match crate::services::memory::header::build_header(&db, &account_id, &user_question) {
            Ok(Some(header)) => prepend_to_final_user_message(&mut initial_messages, &header),
            Ok(None) => {}
            Err(e) => emit_log("debug", &format!("memory header skipped: {e}")),
        }
    }

    // The final user-message bytes are now fixed — persist them so the next
    // turn's history replay extends this prompt instead of diverging from it.
    persist_prompted_tail(&db, &user_message_id, &initial_messages);

    // Collected by run_tool_loop; fed into the final ChatTrace below.
    let mut tool_traces: Vec<ToolCallTrace> = Vec::new();
    let mut llm_calls: Vec<LlmCallTrace> = Vec::new();
    // The planner ran before the loop; surface it first in the timeline.
    if let Some(pt) = planner_trace.take() {
        llm_calls.push(pt);
    }

    // Run the tool loop only on the ToolsFirst path. RagFirst is strictly
    // sources-only: no tool definitions exposed, no tool loop, single LLM
    // stream over the prompt with retrieved sources. This is Pattern B —
    // exactly one retrieval mechanism per turn, so the model never has to
    // choose between stale RAG sources and a fresh tool call.
    let (
        final_messages,
        tool_loop_ms,
        loop_failed_without_answer,
        loop_error,
        mut aggregated_email_refs,
        mut aggregated_draft_refs,
        loop_answer_streamed_live,
    ) = match &route.mode {
        RouteMode::ToolsFirst => {
            emit_log("info", "stage: tool_loop");
            emit_phase(&conversation_id, &assistant_message_id, ChatPhase::RunningTools);
            let t_tool_loop = std::time::Instant::now();
            let outcome = run_tool_loop(
                &db,
                &registry,
                provider.as_ref(),
                &conversation_id,
                &assistant_message_id,
                &account_id,
                &categories,
                &user_question,
                initial_messages,
                preseeded_tool_calls,
                true,
                &mut tool_traces,
                &mut llm_calls,
            )
            .await;
            let elapsed = t_tool_loop.elapsed().as_millis() as i64;
            emit_log(
                "info",
                &format!(
                    "tool_loop: done ({} tool calls, {} llm rounds) [{}ms]",
                    tool_traces.len(),
                    llm_calls.len(),
                    elapsed
                ),
            );
            (
                outcome.messages,
                elapsed,
                outcome.failed_without_answer,
                outcome.error,
                outcome.aggregated_email_refs,
                outcome.aggregated_draft_refs,
                outcome.answer_streamed_live,
            )
        }
        RouteMode::RagFirst => {
            // No tool loop, no preseeded tools. Convert the prompt directly
            // into AiMessages so the stream branch below can run a single
            // LLM call over sources + history. tool_traces/llm_calls stay
            // empty — they get populated only by the tool loop. The RAG
            // path also has no email-ref or draft-ref allowlist: every
            // source the model can cite is already in `sources` as a
            // numbered citation, and those drive `[n](citation://n)`
            // chips — `email://` / `draft://` are reserved for tool-mode
            // mentions.
            emit_log("info", "stage: tool_loop skipped (RagFirst route)");
            let msgs: Vec<AiMessage> = initial_messages
                .into_iter()
                .map(|(role, content)| AiMessage {
                    role,
                    content,
                    tool_calls: None,
                })
                .collect();
            // RagFirst never runs the tool loop, so nothing was streamed there;
            // the synthesis stream below ships the answer.
            (msgs, 0i64, false, None, Vec::new(), Vec::new(), false)
        }
    };

    emit_log("info", "stage: stream");
    emit_phase(&conversation_id, &assistant_message_id, ChatPhase::Generating);
    let t_stream = std::time::Instant::now();
    let mut streaming_happened = false;
    // Captures the exact prompt sent to the final LLM call (dev builds only).
    // Set in either branch below — direct-answer reuses the tool-loop output,
    // streaming branch snapshots the full message list before move.
    #[cfg(debug_assertions)]
    #[allow(unused_assignments)]
    let mut final_stream_input: Option<String> = None;
    // If the tool loop gave up without any assistant answer at all (round 0
    // timeout, provider error, etc.), don't try to re-stream from the initial
    // prompt — the same model just failed us, the stream has no overall
    // timeout in `chat_stream`, and a silent hang here is exactly what
    // freezes the "thinking…" indicator on the client. Fail fast instead.
    const STREAM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);
    let stream_result: Result<crate::ai::provider::ChatStreamResult> = if loop_failed_without_answer {
        let detail = loop_error
            .clone()
            .unwrap_or_else(|| "tool-call loop failed before producing any answer".to_string());
        Err(AppError::AiError(detail))
    } else {
        // Contradiction guard needs the tool-loop transcript to run a
        // corrective retry; only worth keeping when tools actually handed
        // back emails this turn (`aggregated_email_refs` is the
        // format-independent "the tools found something" signal).
        let contradiction_retry_messages: Option<Vec<AiMessage>> = if aggregated_email_refs.is_empty() {
            None
        } else {
            Some(final_messages.clone())
        };
        match plan_answer(final_messages) {
            // The tool loop ended with a direct assistant text answer — reuse it
            // as-is and SKIP the re-stream. Otherwise we'd be appending the answer
            // as context and asking the model to continue, which produces empty
            // tokens because the model already said everything it wanted to.
            AnswerPlan::DirectText(answer) => {
                // Defense-in-depth: a weak local model can leak tool-call markup
                // (`<tool_call>…`) into its direct answer. Truncate from the
                // first tag marker so the markup never reaches the bubble.
                let answer = strip_tool_call_markup(&answer);
                // Qwen 4B in tool-results mode often invents `[1]..[9]` citation
                // markers despite the CITATION CONTRACT — strip any that fall
                // outside the retrieved source range BEFORE emitting so the user
                // never sees them. count_invalid_citations later then reports 0.
                let answer = strip_invalid_citations(&answer, sources.len());
                // Contradiction guard: the model answered "no emails found"
                // even though the tool results above DO contain emails
                // (observed on Qwen 3.6 35B-A3B). Run ONE corrective retry
                // over the same transcript; keep the original answer if the
                // retry fails, so the guard can never make things worse.
                let retry_messages = contradiction_retry_messages.filter(|_| answer_claims_no_results(&answer));
                if let Some(mut retry_messages) = retry_messages {
                    emit_log(
                        "info",
                        "contradiction guard: answer claims no results but tools returned emails — one corrective retry",
                    );
                    // The wrong answer may already sit in the live bubble
                    // (streamed by the tool loop) — clear it before the retry
                    // streams the corrected tokens.
                    if loop_answer_streamed_live {
                        crate::services::events::emit(
                            "chat-stream",
                            ChatStreamEvent {
                                message_id: assistant_message_id.clone(),
                                conversation_id: conversation_id.clone(),
                                token: String::new(),
                                done: false,
                                error: None,
                                token_count: None,
                                latency_ms: None,
                                replace: Some(true),
                            },
                        );
                    }
                    // The transcript already ends with the contradictory
                    // assistant answer (that's what made plan_answer pick
                    // DirectText) — append only the corrective instruction.
                    retry_messages.push(AiMessage {
                        role: "user".to_string(),
                        content: CONTRADICTION_RETRY_INSTRUCTION.to_string(),
                        tool_calls: None,
                    });
                    streaming_happened = true;
                    #[cfg(debug_assertions)]
                    {
                        final_stream_input = Some(format_messages_for_trace(&retry_messages));
                    }
                    let recovery = synthesize_with_recovery(
                        provider.as_ref(),
                        &registry,
                        &db,
                        &account_id,
                        &categories,
                        &user_question,
                        retry_messages,
                        &conversation_id,
                        &assistant_message_id,
                        STREAM_TIMEOUT,
                        &mut llm_calls,
                        &mut tool_traces,
                    )
                    .await;
                    for id in recovery.email_refs {
                        if !aggregated_email_refs.contains(&id) {
                            aggregated_email_refs.push(id);
                        }
                    }
                    for id in recovery.draft_refs {
                        if !aggregated_draft_refs.contains(&id) {
                            aggregated_draft_refs.push(id);
                        }
                    }
                    match recovery.result {
                        Ok(result) if !strip_tool_call_markup(&result.content).trim().is_empty() => Ok(result),
                        _ => {
                            // Retry failed or came back empty — restore the
                            // original answer (a contradictory answer beats an
                            // error or a blank bubble). `replace` also wipes
                            // any partial retry tokens that reached the UI.
                            emit_log(
                                "error",
                                "contradiction retry failed/empty — keeping the original answer",
                            );
                            crate::services::events::emit(
                                "chat-stream",
                                ChatStreamEvent {
                                    message_id: assistant_message_id.clone(),
                                    conversation_id: conversation_id.clone(),
                                    token: answer.clone(),
                                    done: false,
                                    error: None,
                                    token_count: None,
                                    latency_ms: None,
                                    replace: Some(true),
                                },
                            );
                            Ok(crate::ai::provider::ChatStreamResult {
                                content: answer,
                                eval_count: None,
                                prompt_eval_count: None,
                                prefill_ms: None,
                                cached_prompt_tokens: None,
                                prefix_plan: None,
                                sys_cached_before: None,
                                sys_cached_after: None,
                                system_prefix_tokens: None,
                                stable_tokens: None,
                                dropped_front_tokens: None,
                            })
                        }
                    }
                } else {
                    // If the tool loop already streamed this answer token-by-token,
                    // re-emitting it as one token would duplicate the whole bubble —
                    // skip the emit and just synthesise the result for persistence.
                    // Otherwise emit the full answer as a single stream token so the
                    // UI sees it (no extra model round-trip).
                    if !loop_answer_streamed_live {
                        crate::services::events::emit(
                            "chat-stream",
                            ChatStreamEvent {
                                message_id: assistant_message_id.clone(),
                                conversation_id: conversation_id.clone(),
                                token: answer.clone(),
                                done: false,
                                error: None,
                                token_count: None,
                                latency_ms: None,
                                replace: None,
                            },
                        );
                    }
                    Ok(crate::ai::provider::ChatStreamResult {
                        content: answer,
                        eval_count: None,
                        prompt_eval_count: None,
                        prefill_ms: None,
                        cached_prompt_tokens: None,
                        prefix_plan: None,
                        sys_cached_before: None,
                        sys_cached_after: None,
                        system_prefix_tokens: None,
                        stable_tokens: None,
                        dropped_front_tokens: None,
                    })
                }
            }
            // No direct text answer: the loop handed back tool results (a
            // preseeded shortcut, or a run that hit MAX_TOOL_ROUNDS). Synthesise
            // the final answer by STREAMING from those tool results so the user
            // sees tokens within a second or two. Bounded by STREAM_TIMEOUT so a
            // stuck provider can't leave the UI thinking forever.
            AnswerPlan::StreamSynthesis(synthesis_messages) => {
                streaming_happened = true;

                // Snapshot the final prompt in dev builds so the reasoning panel
                // can show exactly what was sent to chat_stream. Done before the
                // move so we don't have to clone `synthesis_messages`.
                #[cfg(debug_assertions)]
                {
                    let snapshot = format_messages_for_trace(&synthesis_messages);
                    final_stream_input = Some(snapshot);
                }

                // Empty-answer recovery lives in `synthesize_with_recovery`:
                // an empty stream (the model leaked a tool call instead of
                // prose) triggers either a salvage-and-execute pass or a
                // corrective retry. The persistence path below ships a
                // localized hint if even the recovery comes back empty.
                let recovery = synthesize_with_recovery(
                    provider.as_ref(),
                    &registry,
                    &db,
                    &account_id,
                    &categories,
                    &user_question,
                    synthesis_messages,
                    &conversation_id,
                    &assistant_message_id,
                    STREAM_TIMEOUT,
                    &mut llm_calls,
                    &mut tool_traces,
                )
                .await;
                // Salvaged tool calls can contribute email/draft refs the
                // final answer may cite — fold them into the allowlists.
                for id in recovery.email_refs {
                    if !aggregated_email_refs.contains(&id) {
                        aggregated_email_refs.push(id);
                    }
                }
                for id in recovery.draft_refs {
                    if !aggregated_draft_refs.contains(&id) {
                        aggregated_draft_refs.push(id);
                    }
                }
                recovery.result
            }
        }
    };

    let streaming_ms = t_stream.elapsed().as_millis() as i64;
    let latency_ms = turn_start.elapsed().as_millis() as i64;

    // Record the final-stream LLM call latency (captured regardless of success
    // so the reasoning panel can show "slow / timed out" breakdowns).
    if streaming_happened {
        #[allow(unused_mut)] // mutated only in debug builds (trace snapshots)
        let mut trace = build_final_stream_trace(streaming_ms, stream_result.as_ref().ok());
        // Dev-only: surface the exact prompt + streamed answer in the
        // reasoning panel so the user can copy-paste them while debugging.
        #[cfg(debug_assertions)]
        {
            trace.input = final_stream_input.take();
            trace.output = Some(match &stream_result {
                Ok(r) => r.content.clone(),
                Err(e) => format!("(error) {}", e),
            });
        }
        llm_calls.push(trace);
    }

    match stream_result {
        Ok(mut result) => {
            // Strip any leaked tool-call markup, then hallucinated citations,
            // before persisting so a re-render after reload shows the cleaned-up
            // text. The direct-answer path already stripped earlier; this catches
            // the live-streaming path (and is a cheap no-op when nothing leaked).
            result.content = strip_tool_call_markup(&result.content);
            result.content = strip_invalid_citations(&result.content, sources.len());
            // Robustness net: still no answer text after the synthesis retry
            // (or a direct answer that stripped to nothing). Ship a localized
            // rephrase hint instead of a silently blank bubble, and emit it as
            // a stream token so the live UI shows it too.
            if result.content.trim().is_empty() {
                emit_log(
                    "error",
                    "assistant answer empty after synthesis retry — shipping rephrase hint instead of a blank bubble",
                );
                result.content = empty_answer_hint(ai_language).to_string();
                crate::services::events::emit(
                    "chat-stream",
                    ChatStreamEvent {
                        message_id: assistant_message_id.clone(),
                        conversation_id: conversation_id.clone(),
                        token: result.content.clone(),
                        done: false,
                        error: None,
                        token_count: None,
                        latency_ms: None,
                        replace: None,
                    },
                );
            }
            let token_count = result.eval_count.map(|c| c as i32);
            if let Err(e) =
                db.update_chat_message_completion(&assistant_message_id, &result.content, token_count, Some(latency_ms))
            {
                emit_log("error", &format!("failed to persist assistant message: {}", e));
            }
            // Persist the structural email-ref allowlist alongside the
            // final content. Frontend validates `email://EMAIL_ID` markdown
            // links against this list when rendering the bubble — anything
            // outside it gets dropped + warned. Direct-answer and re-stream
            // paths converge here so both write the refs once.
            if let Err(e) = db.update_chat_message_referenced_emails(&assistant_message_id, &aggregated_email_refs) {
                emit_log("error", &format!("failed to persist email refs: {}", e));
            }
            // Same shape for draft refs (`draft://DRAFT_ID` chips).
            if let Err(e) = db.update_chat_message_referenced_drafts(&assistant_message_id, &aggregated_draft_refs) {
                emit_log("error", &format!("failed to persist draft refs: {}", e));
            }
            #[cfg(feature = "tracing")]
            crate::ai::tracing::driver().record_chat_turn(crate::ai::tracing::ChatTurnTrace {
                model: provider.model_name().to_string(),
                user_question: user_question.clone(),
                final_answer: result.content.clone(),
                prompt_tokens: result.prompt_eval_count.unwrap_or(0),
                completion_tokens: result.eval_count.unwrap_or(0),
                total_ms: latency_ms as u64,
                route_mode: format!("{:?}", route.mode),
                retrieval: retrieval_trace.as_ref().map(|r| crate::ai::tracing::RetrievalInfo {
                    vector_hits: r.vector_hits,
                    fts_hits: r.fts_hits,
                    elapsed_ms: r.elapsed_ms,
                    embedding_ms: r.embedding_ms,
                    vec_search_ms: r.vec_search_ms,
                    fts_search_ms: r.fts_search_ms,
                    rerank_ms: r.rerank_ms,
                    query_rewrite_ms: r.query_rewrite_ms,
                    expanded_query: r.expanded_query.clone(),
                    vector_fallback: r.vector_fallback,
                    invalid_citations: r.invalid_citations,
                    documents: source_rows
                        .iter()
                        .map(|s| crate::ai::tracing::RetrievedDocument {
                            id: s.email_id.clone(),
                            score: s.relevance_score.unwrap_or(0.0),
                            content: s.body_excerpt.clone().unwrap_or_default(),
                            metadata_json: serde_json::json!({
                                "citation_number": s.citation_number,
                                "subject": s.subject,
                                "sender": s.sender,
                                "sender_email": s.sender_email,
                                "timestamp": s.timestamp,
                            })
                            .to_string(),
                        })
                        .collect(),
                }),
                tool_calls: tool_traces
                    .iter()
                    .map(|t| crate::ai::tracing::ToolCallInfo {
                        name: t.name.clone(),
                        arguments_json: serde_json::to_string(&t.arguments).unwrap_or_default(),
                        result_preview: t.result_preview.clone(),
                        elapsed_ms: t.elapsed_ms,
                    })
                    .collect(),
                llm_calls: llm_calls
                    .iter()
                    .map(|l| {
                        let (input, output) = if l.kind == "final_stream" {
                            // Final stream: user question → final answer
                            (Some(user_question.clone()), Some(result.content.clone()))
                        } else {
                            // Tool rounds: captured per-round in run_tool_loop
                            (l.input.clone(), l.output.clone())
                        };
                        crate::ai::tracing::LlmCallInfo {
                            kind: l.kind.clone(),
                            round: l.round,
                            latency_ms: l.latency_ms,
                            tool_calls_requested: l.tool_calls_requested,
                            failed: l.failed,
                            input,
                            output,
                        }
                    })
                    .collect(),
                error: None,
            });

            // ── 4b. Citation validation ───────────────────────────────────
            // Count [n] markers in the answer that reference a source number
            // outside the retrieved set. 0 = all valid, n>0 = hallucinated
            // citations (prompt contract violation). Logged to the trace so
            // the eval report can track the rate over time.
            let invalid_citations = if result.content.trim().is_empty() {
                -1
            } else {
                count_invalid_citations(&result.content, sources.len())
            };
            if invalid_citations > 0 {
                emit_log(
                    "warn",
                    &format!(
                        "answer contains {} citation(s) outside the source range 1..={}",
                        invalid_citations,
                        sources.len()
                    ),
                );
            }
            let retrieval_trace = retrieval_trace.clone().map(|mut t| {
                t.invalid_citations = invalid_citations;
                t
            });

            // ── 5. Assemble, persist, and emit the reasoning trace ─────────
            // Done after stream success so the trace reflects the full flow
            // (routing + retrieval + any tool calls + total wall-clock time).
            let trace = ChatTrace {
                route: route.clone(),
                retrieval: retrieval_trace.clone(),
                tool_calls: tool_traces.clone(),
                model: model.clone(),
                total_elapsed_ms: latency_ms,
                tool_loop_ms,
                llm_streaming_ms: if streaming_happened { Some(streaming_ms) } else { None },
                llm_calls: llm_calls.clone(),
            };
            if let Err(e) = db.update_chat_message_trace(&assistant_message_id, &trace) {
                emit_log("error", &format!("failed to persist reasoning trace: {}", e));
            }
            crate::services::events::emit(
                "chat-trace",
                ChatTraceEvent {
                    message_id: assistant_message_id.clone(),
                    conversation_id: conversation_id.clone(),
                    trace,
                    referenced_email_ids: aggregated_email_refs.clone(),
                    referenced_draft_ids: aggregated_draft_refs.clone(),
                },
            );

            crate::services::events::emit(
                "chat-stream",
                ChatStreamEvent {
                    message_id: assistant_message_id.clone(),
                    conversation_id: conversation_id.clone(),
                    token: String::new(),
                    done: true,
                    error: None,
                    token_count,
                    latency_ms: Some(latency_ms),
                    replace: None,
                },
            );
            let tokens_str = token_count
                .map(|c| format!("{} tokens", c))
                .unwrap_or_else(|| "? tokens".to_string());
            emit_log(
                "success",
                &format!(
                    "reply complete ({}, {:.1}s, {} sources)",
                    tokens_str,
                    latency_ms as f64 / 1000.0,
                    sources.len()
                ),
            );
            Ok(())
        }
        Err(e) => {
            let err_text = format!("Chat failed: {}", e);
            let _ = db.update_chat_message_completion(&assistant_message_id, &err_text, None, Some(latency_ms));
            crate::services::events::emit(
                "chat-stream",
                ChatStreamEvent {
                    message_id: assistant_message_id.clone(),
                    conversation_id,
                    token: String::new(),
                    done: true,
                    error: Some(err_text.clone()),
                    token_count: None,
                    latency_ms: Some(latency_ms),
                    replace: None,
                },
            );
            emit_log("error", &err_text);
            Err(AppError::AiError(err_text))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Email;

    /// Default chat-system template for tests — keeps the build_prompt
    /// signature tidy at each callsite without dragging the full default
    /// string in at every call.
    fn tpl() -> &'static str {
        crate::services::prompts::defaults::CHAT_SYSTEM
    }

    fn make_scored(citation_number: i32, subject: &str, body: &str) -> ScoredEmail {
        let email = Email {
            id: format!("e{}", citation_number),
            account_id: "acc1".into(),
            thread_id: format!("t{}", citation_number),
            message_id: None,
            subject: subject.into(),
            sender: "Alice".into(),
            sender_email: "alice@example.com".into(),
            recipients: vec![],
            cc: vec![],
            body: body.into(),
            snippet: String::new(),
            timestamp: 1_700_000_000,
            is_read: true,
            triage_status: None,
            category: "primary".into(),
            mailbox: "inbox".into(),
            is_sent: false,
            headers: None,
        };
        ScoredEmail {
            email,
            body: body.into(),
            score: 1.0 / citation_number as f32,
            citation_number,
        }
    }

    use crate::ai::provider::{AiToolCall, AiToolCallFunction};

    fn ai_msg(role: &str, content: &str) -> AiMessage {
        AiMessage {
            role: role.into(),
            content: content.into(),
            tool_calls: None,
        }
    }

    fn ai_tool_call(name: &str) -> AiToolCall {
        AiToolCall {
            function: AiToolCallFunction {
                name: name.into(),
                arguments: serde_json::json!({}),
            },
        }
    }

    #[test]
    fn tool_round_trace_carries_prefill_stats_from_the_provider() {
        let result = crate::ai::provider::ToolStreamResult {
            message: AiMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: Some(vec![ai_tool_call("search_emails")]),
            },
            eval_count: Some(42),
            prompt_eval_count: Some(812),
            prefill_ms: Some(3950),
            cached_prompt_tokens: Some(0),
            prefix_plan: Some("ColdPrefill"),
            sys_cached_before: Some(2344),
            sys_cached_after: Some(5151),
            system_prefix_tokens: Some(5151),
            stable_tokens: Some(805),
            dropped_front_tokens: Some(0),
        };
        let trace = build_tool_round_trace(2, 4280, Some(&result));
        assert_eq!(trace.kind, "tool_round");
        assert_eq!(trace.round, 2);
        assert_eq!(trace.latency_ms, 4280);
        assert_eq!(trace.tool_calls_requested, 1);
        assert!(!trace.failed);
        assert_eq!(trace.prompt_tokens, Some(812));
        assert_eq!(trace.prefill_ms, Some(3950));
        assert_eq!(trace.cached_prompt_tokens, Some(0));
    }

    #[test]
    fn tool_round_trace_marks_failure_and_carries_no_stats() {
        let trace = build_tool_round_trace(0, 9000, None);
        assert!(trace.failed);
        assert_eq!(trace.tool_calls_requested, 0);
        assert_eq!(trace.prompt_tokens, None);
        assert_eq!(trace.prefill_ms, None);
        assert_eq!(trace.cached_prompt_tokens, None);
    }

    #[test]
    fn final_stream_trace_carries_prefill_stats_from_the_provider() {
        let result = crate::ai::provider::ChatStreamResult {
            content: "We ship in March [1].".into(),
            eval_count: Some(17),
            prompt_eval_count: Some(1200),
            prefill_ms: Some(2100),
            cached_prompt_tokens: Some(0),
            prefix_plan: Some("Extend"),
            sys_cached_before: Some(2344),
            sys_cached_after: Some(2344),
            system_prefix_tokens: Some(2344),
            stable_tokens: Some(1193),
            dropped_front_tokens: Some(0),
        };
        let trace = build_final_stream_trace(2900, Some(&result));
        assert_eq!(trace.kind, "final_stream");
        assert_eq!(trace.round, -1);
        assert_eq!(trace.latency_ms, 2900);
        assert!(!trace.failed);
        assert_eq!(trace.prompt_tokens, Some(1200));
        assert_eq!(trace.prefill_ms, Some(2100));
        assert_eq!(trace.cached_prompt_tokens, Some(0));
    }

    #[test]
    fn final_stream_trace_marks_failure_when_stream_errored() {
        let trace = build_final_stream_trace(180_000, None);
        assert!(trace.failed);
        assert_eq!(trace.prompt_tokens, None);
    }

    #[test]
    fn llm_call_trace_serializes_prefill_fields_in_camel_case_and_skips_none() {
        let mut trace = build_tool_round_trace(0, 100, None);
        let json = serde_json::to_value(&trace).expect("serialize");
        assert!(json.get("promptTokens").is_none(), "None fields must be skipped");
        assert!(json.get("prefillMs").is_none());
        assert!(json.get("cachedPromptTokens").is_none());

        trace.prompt_tokens = Some(812);
        trace.prefill_ms = Some(3950);
        trace.cached_prompt_tokens = Some(0);
        let json = serde_json::to_value(&trace).expect("serialize");
        assert_eq!(json["promptTokens"], 812);
        assert_eq!(json["prefillMs"], 3950);
        assert_eq!(json["cachedPromptTokens"], 0);
    }

    #[test]
    fn no_results_claims_match_all_ui_languages() {
        // The two observed Qwen 3.6 failure answers (app + CLI runs of the
        // today-summary shortcut), plus equivalents in the other UI languages.
        let positives = [
            "No se han encontrado correos electrónicos que coincidan con los criterios de búsqueda.",
            "No hay correos electrónicos en tu bandeja de entrada para hoy.\n\
             | Remitente | Asunto | Urgencia | Resumen |\n| (ninguno) | (ninguno) | — | — |\n\
             No hay nada urgente ni pendiente por revisar hoy.",
            "No emails were found matching your search criteria.",
            "There are no emails in your inbox for today.",
            "I couldn't find any emails for today.",
            "Aucun courriel ne correspond à votre recherche.",
            "Es wurden keine E-Mails gefunden.",
        ];
        for answer in positives {
            assert!(
                answer_claims_no_results(answer),
                "must detect a categorical no-results claim: {answer:?}"
            );
        }
    }

    #[test]
    fn real_answers_do_not_count_as_no_results_claims() {
        let negatives = [
            // Real summaries — even ones that mention an absence — must never
            // trigger the retry.
            "You received 12 emails today. The most important one is from Bob about the Q3 budget. \
             There were no emails from Alice, though, so the contract review is still pending. \
             Everything else is routine: three newsletters, two GitHub notifications, and a receipt \
             from AWS. Nothing needs an urgent reply before tomorrow morning, but the budget thread \
             deserves a look today because the deadline is Friday and finance is waiting on it.",
            // Short answers without a no-results phrase.
            "You got one email today: the freelance newsletter [1].",
            "Hoy has recibido un correo: la newsletter de freelance.",
            "",
            "   ",
        ];
        for answer in negatives {
            assert!(
                !answer_claims_no_results(answer),
                "must NOT flag a real answer: {answer:?}"
            );
        }
    }

    #[test]
    fn long_answers_never_count_as_no_results_claims() {
        // Length cap: a pattern inside a long, real answer must not trigger.
        let long = format!(
            "No emails found from Alice. {}",
            "But here is the full summary. ".repeat(20)
        );
        assert!(long.chars().count() > NO_RESULTS_CLAIM_MAX_CHARS);
        assert!(!answer_claims_no_results(&long));
    }

    #[test]
    fn plan_answer_reuses_a_direct_assistant_text_answer() {
        // Normal tools-first turn: the model answered in plain text after a
        // tool round. That text is complete — emit it as-is, no re-stream.
        let messages = vec![
            ai_msg("system", "…"),
            ai_msg("user", "when do we ship?"),
            ai_msg("assistant", "We ship in March [1]."),
        ];
        match plan_answer(messages) {
            AnswerPlan::DirectText(text) => assert_eq!(text, "We ship in March [1]."),
            AnswerPlan::StreamSynthesis(_) => panic!("expected DirectText for a complete assistant answer"),
        }
    }

    #[test]
    fn plan_answer_streams_synthesis_when_loop_ended_on_tool_results() {
        // Preseeded shortcut shape: the synthetic assistant tool-call message
        // plus the tool result, with no assistant text yet. The synthesis must
        // be streamed, and the assistant→tool linkage (tool_calls) must survive
        // into the streamed messages so the provider can bind the result to its
        // call — dropping it (the old strip) orphans the tool message.
        let mut assistant = ai_msg("assistant", "");
        assistant.tool_calls = Some(vec![ai_tool_call("search_emails")]);
        let messages = vec![
            ai_msg("system", "…"),
            ai_msg("user", "resume mis correos de hoy"),
            assistant,
            ai_msg("tool", "## Primary (3)\n- id=e1 …"),
        ];
        match plan_answer(messages) {
            AnswerPlan::StreamSynthesis(out) => {
                assert_eq!(
                    out.len(),
                    5,
                    "all messages carried through, plus the appended closing instruction"
                );
                let carrier = &out[2];
                assert_eq!(carrier.role, "assistant");
                assert_eq!(
                    carrier.tool_calls.as_ref().map(|v| v.len()).unwrap_or(0),
                    1,
                    "tool_calls preserved so the tool result is not orphaned"
                );
                assert_eq!(out[3].role, "tool");
                // The closing instruction is appended last so the synthesis
                // writes prose instead of another <tool_call>.
                assert_eq!(out[4].role, "user");
                assert!(
                    out[4].content.to_lowercase().contains("tool"),
                    "closing instruction must tell the model to stop calling tools"
                );
                assert!(out[4].tool_calls.is_none());
            }
            AnswerPlan::DirectText(_) => panic!("a trailing tool result is not a direct answer"),
        }
    }

    #[test]
    fn plan_answer_appends_closing_instruction_when_loop_hit_max_rounds() {
        // The exact Qwen 3.6 failure: the loop exhausted MAX_TOOL_ROUNDS still
        // fetching bodies (trailing message is a tool result, no assistant
        // text). The synthesis must carry a closing "stop calling tools, answer
        // now" instruction or the model emits another <tool_call> that the gate
        // strips, leaving an empty reply.
        let mut assistant = ai_msg("assistant", "");
        assistant.tool_calls = Some(vec![ai_tool_call("get_email_body")]);
        let messages = vec![
            ai_msg("user", "lista todas las rondas de los últimos 5 correos de itnig"),
            assistant,
            ai_msg("tool", "## body of email 5 …"),
        ];
        match plan_answer(messages) {
            AnswerPlan::StreamSynthesis(out) => {
                let last = out.last().expect("synthesis carries at least the closing instruction");
                assert_eq!(last.role, "user");
                assert!(last.content.to_lowercase().contains("tool"));
                assert!(last.tool_calls.is_none());
            }
            AnswerPlan::DirectText(_) => panic!("a run that ended on a tool result needs synthesis"),
        }
    }

    #[test]
    fn plan_answer_streams_when_final_assistant_text_is_blank() {
        // An assistant message with only whitespace is not a real answer —
        // route it through synthesis rather than emitting an empty bubble.
        let messages = vec![ai_msg("user", "hola"), ai_msg("assistant", "   ")];
        assert!(matches!(plan_answer(messages), AnswerPlan::StreamSynthesis(_)));
    }

    #[test]
    fn repair_fills_from_when_question_names_one_address() {
        // The production failure: the model issues search_emails({}) even
        // though the question names the sender verbatim. Repair injects it.
        let mut args = serde_json::json!({});
        let repaired = repair_filterless_search_args(
            &mut args,
            "hazme un análisis de los emails de cosasdefreelance@substack.com",
        );
        assert!(repaired);
        assert_eq!(args["from"], "cosasdefreelance@substack.com");
        assert_eq!(args["limit"], 25);
        assert_eq!(args["include_bodies"], true);
    }

    #[test]
    fn repair_fills_to_when_address_follows_a_recipient_preposition() {
        for q in [
            "cuántos emails envié a maria@acme.com",
            "how many emails did I send to maria@acme.com",
            "correos para maria@acme.com",
        ] {
            let mut args = serde_json::json!({});
            assert!(repair_filterless_search_args(&mut args, q), "{q}");
            assert_eq!(args["to"], "maria@acme.com", "{q}");
            assert!(args.get("from").is_none(), "{q}");
        }
    }

    #[test]
    fn repair_leaves_calls_that_already_have_a_selective_filter() {
        let mut args = serde_json::json!({"from": "x@y.com"});
        assert!(!repair_filterless_search_args(&mut args, "emails de a@b.com"));
        assert_eq!(args["from"], "x@y.com");
    }

    #[test]
    fn repair_treats_limit_only_and_blank_filters_as_filterless() {
        // limit / include_bodies are not selective; blank strings don't count.
        let mut args = serde_json::json!({"limit": 5, "query": "  "});
        assert!(repair_filterless_search_args(&mut args, "emails de a@b.com"));
        assert_eq!(args["from"], "a@b.com");
        assert_eq!(args["limit"], 5, "existing limit preserved");
    }

    #[test]
    fn repair_declines_on_zero_or_multiple_addresses() {
        // No address, or an ambiguous pair — never guess.
        let mut args = serde_json::json!({});
        assert!(!repair_filterless_search_args(&mut args, "emails de juan"));
        assert!(!repair_filterless_search_args(
            &mut args,
            "emails de a@b.com y de c@d.com"
        ));
        assert_eq!(args, serde_json::json!({}));
    }

    #[test]
    fn repair_dedups_repeated_mentions_of_the_same_address() {
        let mut args = serde_json::json!({});
        assert!(repair_filterless_search_args(
            &mut args,
            "emails de Ana@Acme.com — sí, los de ana@acme.com"
        ));
        assert_eq!(args["from"], "Ana@Acme.com", "first verbatim occurrence wins");
    }

    #[test]
    fn mangled_address_is_corrected_from_the_question() {
        // The model TRANSLATES the address while copying it (observed:
        // "cosasdefreelance@…" → "thingsdefreelance@…"), searches the wrong
        // sender, finds nothing, and reports "no emails found". Same domain +
        // absent from the question = mangled transcription → correct it.
        let args = serde_json::json!({"from": "thingsdefreelance@substack.com", "limit": 25, "order": "newest"});
        let q = "analiza los emails de cosasdefreelance@substack.com";
        let (corrected, right, wrong) = correct_mangled_address_args(&args, q).expect("rescue fires");
        assert_eq!(corrected["from"], "cosasdefreelance@substack.com");
        assert_eq!(corrected["limit"], 25, "other args preserved");
        assert_eq!(right, "cosasdefreelance@substack.com");
        assert_eq!(wrong, "thingsdefreelance@substack.com");
    }

    #[test]
    fn mangled_address_rescue_covers_the_to_field() {
        let args = serde_json::json!({"to": "stuff@acme.com"});
        let q = "correos que envié a cosas@acme.com";
        let (corrected, ..) = correct_mangled_address_args(&args, q).expect("rescue fires");
        assert_eq!(corrected["to"], "cosas@acme.com");
    }

    #[test]
    fn address_present_in_question_is_never_corrected() {
        // The model searched exactly what the user wrote — an empty result is
        // a REAL empty result.
        let q = "emails de alice@acme.com";
        let args = serde_json::json!({"from": "alice@acme.com"});
        assert!(correct_mangled_address_args(&args, q).is_none());
    }

    #[test]
    fn unrelated_or_ambiguous_addresses_are_not_corrected() {
        let q_one = "emails de alice@acme.com";
        // Different domain AND different local part → not a transcription slip.
        let args = serde_json::json!({"from": "bob@other.org"});
        assert!(correct_mangled_address_args(&args, q_one).is_none());
        // Display-name filter (no '@') → nothing to correct.
        let args = serde_json::json!({"from": "sharique"});
        assert!(correct_mangled_address_args(&args, q_one).is_none());
        // Several addresses in the question → ambiguous, never guess.
        let args = serde_json::json!({"from": "alicia@acme.com"});
        assert!(correct_mangled_address_args(&args, "compara alice@acme.com y bob@acme.com").is_none());
    }

    #[test]
    fn repair_missing_email_id_injects_next_unread_ref() {
        // Qwen 3.6 batches body reads but drops every email_id — five
        // get_email_body({}) in one round. Each empty call gets the next
        // search-result id that hasn't been read yet.
        let refs = vec!["e1".to_string(), "e2".to_string(), "e3".to_string()];
        let mut consumed: std::collections::HashSet<String> = ["e1".to_string()].into_iter().collect();

        let mut args = serde_json::json!({});
        let injected = repair_missing_email_id(&mut args, &refs, &consumed);
        assert_eq!(injected.as_deref(), Some("e2"));
        assert_eq!(args["email_id"], "e2");

        consumed.insert("e2".to_string());
        let mut args2 = serde_json::json!({});
        assert_eq!(
            repair_missing_email_id(&mut args2, &refs, &consumed).as_deref(),
            Some("e3"),
            "successive empty calls walk the unread refs in order"
        );
    }

    #[test]
    fn repair_missing_email_id_respects_an_explicit_id() {
        let refs = vec!["e1".to_string()];
        let consumed = std::collections::HashSet::new();
        let mut args = serde_json::json!({"email_id": "custom"});
        assert!(repair_missing_email_id(&mut args, &refs, &consumed).is_none());
        assert_eq!(args["email_id"], "custom");
    }

    #[test]
    fn repair_missing_email_id_declines_without_unread_refs() {
        let consumed: std::collections::HashSet<String> = ["e1".to_string()].into_iter().collect();
        let mut args = serde_json::json!({});
        // All refs consumed…
        assert!(repair_missing_email_id(&mut args, &["e1".to_string()], &consumed).is_none());
        // …or no refs at all: leave the call for normal validation.
        assert!(repair_missing_email_id(&mut args, &[], &consumed).is_none());
        assert_eq!(args, serde_json::json!({}));
    }

    #[tokio::test]
    async fn loop_repairs_idless_body_reads_with_search_result_refs() {
        // End-to-end through run_tool_loop: a preseeded search returns email
        // refs, then the model batches TWO get_email_body({}) calls with no
        // ids. The loop must inject e1 and e2 (distinct) instead of bouncing
        // two "missing email_id" errors.
        struct SearchWithRefs;
        #[async_trait::async_trait]
        impl tools::Tool for SearchWithRefs {
            fn name(&self) -> &'static str {
                "search_emails"
            }
            fn description(&self) -> &'static str {
                "scripted search"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({ "type": "object", "properties": {} })
            }
            async fn execute(
                &self,
                _ctx: &tools::ToolCtx<'_>,
                _args: serde_json::Value,
            ) -> std::result::Result<tools::ToolOutput, tools::ToolError> {
                Ok(tools::ToolOutput::text_with_email_refs(
                    "- id=e1 …\n- id=e2 …".to_string(),
                    vec!["e1".to_string(), "e2".to_string()],
                ))
            }
        }
        struct EchoBody;
        #[async_trait::async_trait]
        impl tools::Tool for EchoBody {
            fn name(&self) -> &'static str {
                "get_email_body"
            }
            fn description(&self) -> &'static str {
                "scripted body read"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({ "type": "object", "properties": {"email_id": {"type":"string"}}, "required": ["email_id"] })
            }
            async fn execute(
                &self,
                _ctx: &tools::ToolCtx<'_>,
                args: serde_json::Value,
            ) -> std::result::Result<tools::ToolOutput, tools::ToolError> {
                let id = args.get("email_id").and_then(|v| v.as_str()).unwrap_or("<missing>");
                Ok(tools::ToolOutput::text(format!("body of {id}")))
            }
        }

        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let registry = Arc::new(tools::ToolRegistry::with_tools(vec![
            Arc::new(SearchWithRefs) as Arc<dyn tools::Tool>,
            Arc::new(EchoBody) as Arc<dyn tools::Tool>,
        ]));

        let provider = crate::ai::provider::FakeAiProvider::new();
        // Round 0: batch of two id-less body reads (the observed failure).
        provider.push_chat_message(AiMessage {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: Some(vec![ai_tool_call("get_email_body"), ai_tool_call("get_email_body")]),
        });
        // Round 1: prose.
        provider.push_chat_message(AiMessage {
            role: "assistant".to_string(),
            content: "Perfil construido.".to_string(),
            tool_calls: None,
        });

        let mut tool_traces: Vec<ToolCallTrace> = Vec::new();
        let mut llm_calls: Vec<LlmCallTrace> = Vec::new();
        let outcome = run_tool_loop(
            &db,
            &registry,
            &provider,
            "conv-1",
            "msg-1",
            "acct-1",
            &[],
            "analiza los correos de esa newsletter",
            vec![
                ("system".to_string(), "SYS".to_string()),
                ("user".to_string(), "analiza los correos de esa newsletter".to_string()),
            ],
            Some(vec![ai_tool_call("search_emails")]),
            true,
            &mut tool_traces,
            &mut llm_calls,
        )
        .await;

        let body_ids: Vec<String> = tool_traces
            .iter()
            .filter(|t| t.name == "get_email_body")
            .filter_map(|t| t.arguments.get("email_id").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        assert_eq!(
            body_ids,
            vec!["e1".to_string(), "e2".to_string()],
            "both id-less reads must be repaired with distinct search-result ids; traces: {:?}",
            tool_traces.iter().map(|t| (&t.name, &t.arguments)).collect::<Vec<_>>()
        );
        assert!(
            outcome
                .messages
                .iter()
                .any(|m| m.role == "tool" && m.content == "body of e1"),
            "repaired call actually executed"
        );
    }

    #[test]
    fn plan_answer_streams_when_final_assistant_text_is_only_tool_markup() {
        // The exact empty-reply failure: the final assistant message carried no
        // structured tool_calls but its text was ONLY a degenerate <tool_call>
        // block the parsers could not salvage (args-only, no name). Stripping
        // the markup leaves nothing to show — route through synthesis instead
        // of shipping an empty bubble as a "direct" answer.
        let messages = vec![
            ai_msg("user", "analiza los correos de x@substack.com"),
            ai_msg(
                "assistant",
                "<tool_call>{\"arguments\": {\"from\": \"x@substack.com\", \"limit\": 25}}\n</tool_call>",
            ),
        ];
        assert!(matches!(plan_answer(messages), AnswerPlan::StreamSynthesis(_)));
    }

    #[test]
    fn empty_answer_hint_is_localized_and_actionable() {
        use crate::services::i18n::Language;
        for lang in Language::ALL {
            let hint = empty_answer_hint(lang);
            assert!(!hint.trim().is_empty(), "hint must exist for {lang:?}");
        }
        // The hint must steer the user toward a rephrase, in their language.
        assert!(empty_answer_hint(Language::Es).contains("reformular"));
        assert!(empty_answer_hint(Language::En).contains("rephras"));
    }

    #[test]
    fn round_may_stream_live_suppresses_first_force_tool_round() {
        // force_tool_use turn, nothing executed yet, nudge still available:
        // the model might be announcing a tool call as text — do NOT stream it.
        assert!(!round_may_stream_live(true, true, 0, 1));
    }

    #[test]
    fn round_may_stream_live_allows_after_nudge_exhausted() {
        // Same force-tool turn, but the one nudge is spent — accept and stream
        // the bare-text answer instead of looping forever.
        assert!(round_may_stream_live(true, true, 1, 1));
    }

    #[test]
    fn round_may_stream_live_allows_once_a_tool_has_executed() {
        // A tool already ran; the model's prose is now a genuine synthesis
        // answer, so stream it live even on a force-tool turn.
        assert!(round_may_stream_live(true, false, 0, 1));
    }

    #[test]
    fn round_may_stream_live_allows_thread_bound_qna() {
        // Thread-bound chat never forces a tool call: every round is a real
        // answer and may stream live from the first token.
        assert!(round_may_stream_live(false, true, 0, 1));
    }

    #[test]
    fn plan_answer_streams_when_final_assistant_still_holds_a_tool_call() {
        // Defensive: a trailing assistant message that still carries a tool_call
        // is not a finished answer, even if it has some content.
        let mut assistant = ai_msg("assistant", "let me check");
        assistant.tool_calls = Some(vec![ai_tool_call("search_emails")]);
        let messages = vec![ai_msg("user", "?"), assistant];
        assert!(matches!(plan_answer(messages), AnswerPlan::StreamSynthesis(_)));
    }

    #[tokio::test]
    async fn empty_synthesis_salvages_leaked_tool_call_and_resynthesises() {
        // The exact production failure (Qwen 3.6 35B): the tools-free synthesis
        // stream answered with NOTHING but a complete, valid tool call. The
        // stream gate suppresses it, so the user used to get a silently empty
        // bubble. The recovery must execute the call the model asked for and
        // re-synthesise prose from its result.
        struct ScriptedTool {
            name: &'static str,
            output: String,
        }
        #[async_trait::async_trait]
        impl tools::Tool for ScriptedTool {
            fn name(&self) -> &'static str {
                self.name
            }
            fn description(&self) -> &'static str {
                "scripted test tool"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({ "type": "object", "properties": {} })
            }
            async fn execute(
                &self,
                _ctx: &tools::ToolCtx<'_>,
                _args: serde_json::Value,
            ) -> std::result::Result<tools::ToolOutput, tools::ToolError> {
                Ok(tools::ToolOutput::text(self.output.clone()))
            }
        }

        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let registry = tools::ToolRegistry::with_tools(vec![Arc::new(ScriptedTool {
            name: "search_emails",
            output: "No matching emails found.".to_string(),
        }) as Arc<dyn tools::Tool>]);

        let provider = crate::ai::provider::FakeAiProvider::new();
        // Attempt 1: markup-only "answer" — strips to empty.
        provider.push_chat_response(
            "<tool_call>{\"name\":\"search_emails\",\"arguments\":{\"from\":\"x@substack.com\",\"limit\":25}}</tool_call>",
        );
        // Attempt 2 (after the salvaged tool ran): real prose.
        provider.push_chat_response("No encontré correos de x@substack.com.");

        let mut llm_calls: Vec<LlmCallTrace> = Vec::new();
        let mut tool_traces: Vec<ToolCallTrace> = Vec::new();
        let recovery = synthesize_with_recovery(
            &provider,
            &registry,
            &db,
            "acct-1",
            &[],
            "analiza los correos de x@substack.com",
            vec![ai_msg("user", "analiza los correos de x@substack.com")],
            "conv-1",
            "msg-1",
            std::time::Duration::from_secs(5),
            &mut llm_calls,
            &mut tool_traces,
        )
        .await;

        let result = recovery.result.expect("retry stream succeeded");
        assert_eq!(result.content, "No encontré correos de x@substack.com.");
        // The salvaged call executed and is visible in the trace.
        assert!(
            tool_traces.iter().any(|t| t.name == "search_emails" && t.round == -3),
            "salvaged tool call must be dispatched and traced; got {:?}",
            tool_traces.iter().map(|t| (&t.name, t.round)).collect::<Vec<_>>()
        );
        // The empty first attempt is traced separately so latency isn't doubled.
        assert!(llm_calls.iter().any(|c| c.kind == "final_stream_empty"));
        // The retry prompt carried the tool result back to the model.
        let calls = provider.chat_calls();
        assert_eq!(calls.len(), 2, "one initial synthesis + one recovery synthesis");
        assert!(
            calls[1]
                .iter()
                .any(|m| m.role == "tool" && m.content.contains("No matching emails found.")),
            "recovery prompt must include the executed tool's result"
        );
    }

    #[tokio::test]
    async fn empty_synthesis_salvages_nameless_tool_call_via_arg_key_inference() {
        // Variant of the production failure: the leaked block is args-only —
        // `{"arguments":{"from":…,"limit":…}}` with NO name. The named parser
        // can't place it, but arg-key inference can (from/limit uniquely match
        // search_emails' schema), same as the in-loop salvage chain.
        struct SearchLikeTool;
        #[async_trait::async_trait]
        impl tools::Tool for SearchLikeTool {
            fn name(&self) -> &'static str {
                "search_emails"
            }
            fn description(&self) -> &'static str {
                "scripted search"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string"},
                        "from": {"type": "string"},
                        "limit": {"type": "integer"}
                    },
                    "required": []
                })
            }
            async fn execute(
                &self,
                _ctx: &tools::ToolCtx<'_>,
                _args: serde_json::Value,
            ) -> std::result::Result<tools::ToolOutput, tools::ToolError> {
                Ok(tools::ToolOutput::text("1 email from x@substack.com: hola".to_string()))
            }
        }

        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let registry = tools::ToolRegistry::with_tools(vec![Arc::new(SearchLikeTool) as Arc<dyn tools::Tool>]);

        let provider = crate::ai::provider::FakeAiProvider::new();
        provider.push_chat_response(
            "<tool_call>{\"arguments\": {\"from\": \"x@substack.com\", \"limit\": 25}}\n</tool_call>",
        );
        provider.push_chat_response("Encontré 1 correo de x@substack.com.");

        let mut llm_calls: Vec<LlmCallTrace> = Vec::new();
        let mut tool_traces: Vec<ToolCallTrace> = Vec::new();
        let recovery = synthesize_with_recovery(
            &provider,
            &registry,
            &db,
            "acct-1",
            &[],
            "analiza los correos de x@substack.com",
            vec![ai_msg("user", "analiza los correos de x@substack.com")],
            "conv-1",
            "msg-1",
            std::time::Duration::from_secs(5),
            &mut llm_calls,
            &mut tool_traces,
        )
        .await;

        let result = recovery.result.expect("retry stream succeeded");
        assert_eq!(result.content, "Encontré 1 correo de x@substack.com.");
        assert!(
            tool_traces.iter().any(|t| t.name == "search_emails" && t.round == -3),
            "nameless call must be inferred and dispatched; got {:?}",
            tool_traces.iter().map(|t| (&t.name, t.round)).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn empty_synthesis_ladder_allows_a_second_salvage_round_then_prose() {
        // Observed on Qwen 3.6 35B: the first leaked call is degenerate
        // (`search_emails({})` → validation error), and only the SECOND leaked
        // call carries usable args. The recovery ladder must execute both
        // (bounded) before the model finally writes prose.
        struct ScriptedTool;
        #[async_trait::async_trait]
        impl tools::Tool for ScriptedTool {
            fn name(&self) -> &'static str {
                "search_emails"
            }
            fn description(&self) -> &'static str {
                "scripted search"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({ "type": "object", "properties": {"from": {"type":"string"}}, "required": [] })
            }
            async fn execute(
                &self,
                _ctx: &tools::ToolCtx<'_>,
                args: serde_json::Value,
            ) -> std::result::Result<tools::ToolOutput, tools::ToolError> {
                let text = if args.get("from").is_some() {
                    "1 email found."
                } else {
                    "Error: needs a filter."
                };
                Ok(tools::ToolOutput::text(text.to_string()))
            }
        }

        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let registry = tools::ToolRegistry::with_tools(vec![Arc::new(ScriptedTool) as Arc<dyn tools::Tool>]);

        let provider = crate::ai::provider::FakeAiProvider::new();
        // Attempt 1: degenerate empty-args call.
        provider.push_chat_response("<tool_call>{\"name\":\"search_emails\",\"arguments\":{}}</tool_call>");
        // Attempt 2: still a tool call, but a usable one this time.
        provider.push_chat_response(
            "<tool_call>{\"name\":\"search_emails\",\"arguments\":{\"from\":\"x@substack.com\"}}</tool_call>",
        );
        // Attempt 3: prose at last.
        provider.push_chat_response("Encontré 1 correo.");

        let mut llm_calls: Vec<LlmCallTrace> = Vec::new();
        let mut tool_traces: Vec<ToolCallTrace> = Vec::new();
        let recovery = synthesize_with_recovery(
            &provider,
            &registry,
            &db,
            "acct-1",
            &[],
            "analiza los correos de x@substack.com",
            vec![ai_msg("user", "analiza los correos de x@substack.com")],
            "conv-1",
            "msg-1",
            std::time::Duration::from_secs(5),
            &mut llm_calls,
            &mut tool_traces,
        )
        .await;

        let result = recovery.result.expect("final stream succeeded");
        assert_eq!(result.content, "Encontré 1 correo.");
        assert_eq!(
            tool_traces.iter().filter(|t| t.round == -3).count(),
            2,
            "both salvaged calls dispatched; got {:?}",
            tool_traces.iter().map(|t| (&t.name, t.round)).collect::<Vec<_>>()
        );
        assert_eq!(provider.chat_calls().len(), 3);
    }

    #[tokio::test]
    async fn empty_synthesis_ladder_gives_up_after_budget_and_returns_empty() {
        // A model that NEVER stops emitting tool calls must not loop forever:
        // 2 salvage rounds + 1 corrective retry, then hand the empty result
        // back so the caller ships the localized hint.
        struct ScriptedTool;
        #[async_trait::async_trait]
        impl tools::Tool for ScriptedTool {
            fn name(&self) -> &'static str {
                "search_emails"
            }
            fn description(&self) -> &'static str {
                "scripted search"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({ "type": "object", "properties": {} })
            }
            async fn execute(
                &self,
                _ctx: &tools::ToolCtx<'_>,
                _args: serde_json::Value,
            ) -> std::result::Result<tools::ToolOutput, tools::ToolError> {
                Ok(tools::ToolOutput::text("result".to_string()))
            }
        }

        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let registry = tools::ToolRegistry::with_tools(vec![Arc::new(ScriptedTool) as Arc<dyn tools::Tool>]);

        let provider = crate::ai::provider::FakeAiProvider::new();
        for _ in 0..10 {
            provider.push_chat_response("<tool_call>{\"name\":\"search_emails\",\"arguments\":{}}</tool_call>");
        }

        let mut llm_calls: Vec<LlmCallTrace> = Vec::new();
        let mut tool_traces: Vec<ToolCallTrace> = Vec::new();
        let recovery = synthesize_with_recovery(
            &provider,
            &registry,
            &db,
            "acct-1",
            &[],
            "hola",
            vec![ai_msg("user", "hola")],
            "conv-1",
            "msg-1",
            std::time::Duration::from_secs(5),
            &mut llm_calls,
            &mut tool_traces,
        )
        .await;

        let result = recovery.result.expect("stream itself never errored");
        assert!(
            strip_tool_call_markup(&result.content).trim().is_empty(),
            "still empty after budget — the caller ships the hint"
        );
        // 1 initial + 2 salvage rounds + 1 corrective = 4 model calls max.
        assert_eq!(provider.chat_calls().len(), 4, "recovery budget must be bounded");
    }

    #[tokio::test]
    async fn empty_synthesis_without_salvageable_call_retries_with_corrective_instruction() {
        // Nothing to salvage (the model produced a blank answer, not a leaked
        // tool call): retry once with the corrective instruction appended.
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let registry = tools::ToolRegistry::with_tools(Vec::new());

        let provider = crate::ai::provider::FakeAiProvider::new();
        provider.push_chat_response(""); // attempt 1: empty
        provider.push_chat_response("Aquí tienes el análisis."); // attempt 2: prose

        let mut llm_calls: Vec<LlmCallTrace> = Vec::new();
        let mut tool_traces: Vec<ToolCallTrace> = Vec::new();
        let recovery = synthesize_with_recovery(
            &provider,
            &registry,
            &db,
            "acct-1",
            &[],
            "hola",
            vec![ai_msg("user", "hola")],
            "conv-1",
            "msg-1",
            std::time::Duration::from_secs(5),
            &mut llm_calls,
            &mut tool_traces,
        )
        .await;

        let result = recovery.result.expect("retry stream succeeded");
        assert_eq!(result.content, "Aquí tienes el análisis.");
        assert!(tool_traces.is_empty(), "no tool should run when nothing was salvaged");
        let calls = provider.chat_calls();
        assert_eq!(calls.len(), 2);
        let last = calls[1].last().expect("retry prompt has messages");
        assert_eq!(last.role, "user");
        assert!(
            last.content.contains("prosa") && last.content.contains("prose"),
            "corrective instruction must be appended (bilingual): {}",
            last.content
        );
    }

    #[tokio::test]
    async fn non_empty_synthesis_returns_without_retry() {
        // A healthy synthesis (prose on the first attempt) must not spend a
        // second model call.
        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let registry = tools::ToolRegistry::with_tools(Vec::new());

        let provider = crate::ai::provider::FakeAiProvider::new();
        provider.push_chat_response("Respuesta normal.");

        let mut llm_calls: Vec<LlmCallTrace> = Vec::new();
        let mut tool_traces: Vec<ToolCallTrace> = Vec::new();
        let recovery = synthesize_with_recovery(
            &provider,
            &registry,
            &db,
            "acct-1",
            &[],
            "hola",
            vec![ai_msg("user", "hola")],
            "conv-1",
            "msg-1",
            std::time::Duration::from_secs(5),
            &mut llm_calls,
            &mut tool_traces,
        )
        .await;

        let result = recovery.result.expect("stream succeeded");
        assert_eq!(result.content, "Respuesta normal.");
        assert_eq!(provider.chat_calls().len(), 1, "no retry for a healthy answer");
        assert!(llm_calls.is_empty(), "no extra trace entries for a healthy answer");
    }

    #[tokio::test]
    async fn shortcut_continues_loop_so_model_can_call_a_follow_up_tool() {
        // Regression: a preseeded shortcut (e.g. "summarise today's emails")
        // used to run its tool and then return immediately, handing the tool
        // results to a tools-free streaming synthesis pass. When the model
        // answered that pass by emitting ANOTHER tool call (get_email_body) —
        // as small models do for "summarise" queries — the synthesis path
        // stripped the tool-call markup and shipped an EMPTY body. The fix lets
        // the shortcut fall through into the tool loop, which can actually
        // dispatch the follow-up tool and then synthesise prose.
        //
        // No event-sink install here, so the default NoopEventSink absorbs the
        // loop's phase/log emits — no `seam_test_lock` needed.

        // A scripted tool that returns fixed text regardless of its arguments.
        struct ScriptedTool {
            name: &'static str,
            output: String,
        }
        #[async_trait::async_trait]
        impl tools::Tool for ScriptedTool {
            fn name(&self) -> &'static str {
                self.name
            }
            fn description(&self) -> &'static str {
                "scripted test tool"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({ "type": "object", "properties": {} })
            }
            async fn execute(
                &self,
                _ctx: &tools::ToolCtx<'_>,
                _args: serde_json::Value,
            ) -> std::result::Result<tools::ToolOutput, tools::ToolError> {
                Ok(tools::ToolOutput::text(self.output.clone()))
            }
        }

        let db = Arc::new(Database::new_for_testing().expect("test db"));
        let registry = Arc::new(tools::ToolRegistry::with_tools(vec![
            Arc::new(ScriptedTool {
                name: "search_emails",
                output: "## Today\n- id=e1 RE: Weekly Jorge".to_string(),
            }) as Arc<dyn tools::Tool>,
            Arc::new(ScriptedTool {
                name: "get_email_body",
                output: "Full body: we ship on March 3rd.".to_string(),
            }) as Arc<dyn tools::Tool>,
        ]));

        // Round 0: the model reacts to the preseeded search results by asking
        // for a body (the exact shape that produced the empty answer). Round 1:
        // with the body in hand it writes the final prose.
        let provider = crate::ai::provider::FakeAiProvider::new();
        provider.push_chat_message(AiMessage {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: Some(vec![ai_tool_call("get_email_body")]),
        });
        provider.push_chat_message(AiMessage {
            role: "assistant".to_string(),
            content: "We ship on March 3rd.".to_string(),
            tool_calls: None,
        });

        let mut tool_traces: Vec<ToolCallTrace> = Vec::new();
        let mut llm_calls: Vec<LlmCallTrace> = Vec::new();

        let outcome = run_tool_loop(
            &db,
            &registry,
            &provider,
            "conv-1",
            "msg-1",
            "acct-1",
            &[],
            "summarise today's emails",
            vec![
                ("system".to_string(), "SYS".to_string()),
                ("user".to_string(), "summarise today's emails".to_string()),
            ],
            Some(vec![ai_tool_call("search_emails")]),
            true,
            &mut tool_traces,
            &mut llm_calls,
        )
        .await;

        // The loop continued past the shortcut: the follow-up get_email_body ran.
        assert!(
            tool_traces.iter().any(|t| t.name == "get_email_body"),
            "expected the loop to dispatch the model's follow-up get_email_body call; traces: {:?}",
            tool_traces.iter().map(|t| &t.name).collect::<Vec<_>>()
        );

        // The final assistant message is real prose, not an empty body.
        let last = outcome.messages.last().expect("at least one message");
        assert_eq!(last.role, "assistant");
        assert_eq!(last.content, "We ship on March 3rd.");

        // …and that complete answer routes as DirectText — no extra synthesis
        // pass that would re-strip a leaked tool call back into emptiness.
        match plan_answer(outcome.messages) {
            AnswerPlan::DirectText(text) => assert_eq!(text, "We ship on March 3rd."),
            AnswerPlan::StreamSynthesis(_) => panic!("expected DirectText once the loop synthesised the answer"),
        }
    }

    fn make_message(role: &str, content: &str) -> ChatMessage {
        ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: "c1".into(),
            role: role.into(),
            content: content.into(),
            model: None,
            token_count: None,
            latency_ms: None,
            created_at: 0,
            sources: Vec::new(),
            trace: None,
            referenced_email_ids: Vec::new(),
            referenced_draft_ids: Vec::new(),
            prompt_content: None,
        }
    }

    #[test]
    fn prompt_includes_numbered_sources_in_final_user_message() {
        let sources = vec![
            make_scored(1, "Q1 plan", "we will ship by march"),
            make_scored(2, "Invoice", "please pay by friday"),
        ];
        let msgs = build_prompt(&sources, &[], "when do we ship?", "en", "", tpl(), "");
        assert_eq!(msgs[0].0, "system");
        // The per-turn sources block must NOT live in the system message —
        // it would invalidate the cross-turn KV prefix every turn.
        let sys = &msgs[0].1;
        assert!(!sys.contains("[1] From: Alice"), "sources leaked into system: {sys}");
        assert!(
            !sys.contains("valid citation range"),
            "sources header leaked into system"
        );
        let (last_role, last) = msgs.last().unwrap();
        assert_eq!(last_role, "user");
        assert!(last.contains("[1] From: Alice"));
        assert!(last.contains("Subject: Q1 plan"));
        assert!(last.contains("[2] From: Alice"));
        assert!(last.contains("Subject: Invoice"));
        assert!(last.contains("valid citation range: [1]..[2]"));
        // The question comes AFTER the sources block, at the very end.
        let q_pos = last.rfind("when do we ship?").expect("question missing");
        let src_pos = last.find("[2] From: Alice").expect("sources missing");
        assert!(q_pos > src_pos, "question must follow the sources block");
        assert!(last.trim_end().ends_with("when do we ship?"));
    }

    #[test]
    fn system_message_is_byte_stable_across_turns() {
        // Different sources, question, and history must all render the SAME
        // system message — that stability is what lets the llama.cpp actor
        // reuse the [system + older history] KV prefix across turns.
        let turn1 = build_prompt(
            &[make_scored(1, "Q1 plan", "we will ship by march")],
            &[],
            "when do we ship?",
            "en",
            "",
            tpl(),
            "TOOLS",
        );
        let history = vec![
            make_message("user", "when do we ship?"),
            make_message("assistant", "March [1]."),
        ];
        let turn2 = build_prompt(
            &[make_scored(1, "Invoice", "please pay by friday")],
            &history,
            "and the invoice?",
            "en",
            "",
            tpl(),
            "TOOLS",
        );
        let turn3_no_sources = build_prompt(&[], &history, "anything else?", "en", "", tpl(), "TOOLS");
        assert_eq!(turn1[0], turn2[0], "system message changed between turns");
        assert_eq!(
            turn1[0], turn3_no_sources[0],
            "system message depends on sources presence"
        );
    }

    #[test]
    fn history_replays_prompt_content_bytes_for_user_rows() {
        // A past user turn was PROMPTED as "memory header + sources + question"
        // but its `content` stores only the raw question. Replaying the raw
        // question would diverge from the bytes the KV cache holds, so replay
        // must prefer `prompt_content` when present.
        let mut past_user = make_message("user", "when do we ship?");
        past_user.prompt_content = Some(
            "## What I remember\n- prefers metric\n\nSources (valid citation range: [1]..[1]):\n[1] …\n\nwhen do we ship?"
                .to_string(),
        );
        let mut past_assistant = make_message("assistant", "March [1].");
        // Assistant rows never carry prompt_content; replay keeps content even
        // if some bug were to populate it.
        past_assistant.prompt_content = Some("SHOULD NOT BE USED".to_string());
        let history = vec![past_user.clone(), past_assistant];

        let msgs = build_prompt(&[], &history, "and the invoice?", "en", "", tpl(), "");
        let replayed_user = &msgs[1];
        assert_eq!(replayed_user.0, "user");
        assert_eq!(
            replayed_user.1,
            past_user.prompt_content.clone().unwrap(),
            "user history row must replay the as-prompted bytes"
        );
        let replayed_assistant = &msgs[2];
        assert_eq!(replayed_assistant.0, "assistant");
        assert_eq!(replayed_assistant.1, "March [1].");
    }

    #[test]
    fn persist_prompted_tail_stores_final_user_bytes() {
        let db = Database::new_for_testing().expect("test db");
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at, sort_order, enabled) \
                 VALUES ('a1', 'gmail', 'a@b.c', 'a', 0, 0, 1)",
                [],
            )
            .expect("seed account");
        let conv = db.create_chat_conversation("a1", "t").expect("conv");
        let user = db
            .insert_chat_message(&conv.id, "user", "raw question", None)
            .expect("user msg");

        let messages = vec![
            ("system".to_string(), "SYS".to_string()),
            ("user".to_string(), "HEADER\n\nSources …\n\nraw question".to_string()),
        ];
        persist_prompted_tail(&db, &user.id, &messages);

        let msgs = db.get_chat_messages(&conv.id).expect("msgs");
        assert_eq!(msgs[0].content, "raw question", "display content untouched");
        assert_eq!(
            msgs[0].prompt_content.as_deref(),
            Some("HEADER\n\nSources …\n\nraw question")
        );
    }

    #[test]
    fn persist_prompted_tail_ignores_non_user_tail() {
        let db = Database::new_for_testing().expect("test db");
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at, sort_order, enabled) \
                 VALUES ('a1', 'gmail', 'a@b.c', 'a', 0, 0, 1)",
                [],
            )
            .expect("seed account");
        let conv = db.create_chat_conversation("a1", "t").expect("conv");
        let user = db
            .insert_chat_message(&conv.id, "user", "raw question", None)
            .expect("user msg");

        let messages = vec![("assistant".to_string(), "answer".to_string())];
        persist_prompted_tail(&db, &user.id, &messages);

        let msgs = db.get_chat_messages(&conv.id).expect("msgs");
        assert_eq!(msgs[0].prompt_content, None);
    }

    #[test]
    fn history_without_prompt_content_falls_back_to_content() {
        let history = vec![
            make_message("user", "plain question"),
            make_message("assistant", "answer"),
        ];
        let msgs = build_prompt(&[], &history, "next?", "en", "", tpl(), "");
        assert_eq!(msgs[1], ("user".to_string(), "plain question".to_string()));
    }

    #[test]
    fn prompt_trims_body_length() {
        let long_body = "x".repeat(MAX_SOURCE_BODY_CHARS * 4);
        let sources = vec![make_scored(1, "long", &long_body)];
        let msgs = build_prompt(&sources, &[], "?", "en", "", tpl(), "");
        let last = &msgs.last().unwrap().1;
        // Source body should be truncated (ellipsis marker present).
        assert!(last.contains("…"));
        // The sources-bearing message must be meaningfully shorter than the
        // raw body — otherwise the truncation is not taking effect.
        assert!(
            last.len() < long_body.len(),
            "final user message ({}) should be shorter than untruncated body ({})",
            last.len(),
            long_body.len(),
        );
    }

    #[test]
    fn prompt_strips_html_from_bodies() {
        let sources = vec![make_scored(1, "html email", "<p>hello <b>world</b></p>")];
        let msgs = build_prompt(&sources, &[], "?", "en", "", tpl(), "");
        let last = &msgs.last().unwrap().1;
        assert!(last.contains("hello"));
        assert!(last.contains("world"));
        assert!(!last.contains("<b>"));
        assert!(!last.contains("<p>"));
    }

    #[test]
    fn prompt_keeps_only_last_six_turns() {
        let mut history: Vec<ChatMessage> = Vec::new();
        for i in 0..10 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            history.push(make_message(role, &format!("turn-{}", i)));
        }
        let msgs = build_prompt(&[], &history, "new question", "en", "", tpl(), "");
        // system + 6 history turns + user question = 8
        assert_eq!(msgs.len(), 8);
        // The oldest included turn should be turn-4 (indices 4..10 = 6 turns).
        assert_eq!(msgs[1].1, "turn-4");
        assert_eq!(msgs[6].1, "turn-9");
        assert!(msgs[7].1.trim_end().ends_with("new question"));
    }

    #[test]
    fn prompt_skips_system_role_in_history() {
        let history = vec![
            make_message("system", "do not surface me"),
            make_message("user", "hi"),
            make_message("assistant", "hello"),
        ];
        let msgs = build_prompt(&[], &history, "next", "en", "", tpl(), "");
        assert!(msgs.iter().all(|(_, c)| c != "do not surface me"));
    }

    #[test]
    fn prompt_empty_sources_advises_model_in_final_user_message() {
        let msgs = build_prompt(&[], &[], "anything?", "en", "", tpl(), "");
        // When no sources were pre-retrieved, the prompt must push the model
        // toward calling search_emails rather than refusing or guessing —
        // but from the per-turn user message, never the (stable) system one.
        let sys = &msgs[0].1;
        assert!(!sys.contains("none pre-retrieved"), "advisory leaked into system");
        let last = &msgs.last().unwrap().1;
        assert!(last.contains("none pre-retrieved"));
        assert!(last.contains("search_emails"));
        assert!(last.trim_end().ends_with("anything?"));
    }

    #[test]
    fn memory_header_prepends_to_final_user_message_not_system() {
        let mut msgs = build_prompt(&[], &[], "anything?", "en", "", tpl(), "");
        prepend_to_final_user_message(&mut msgs, "<memory>user likes tables</memory>");
        assert!(
            !msgs[0].1.contains("<memory>"),
            "memory header leaked into system: would break the cross-turn KV prefix"
        );
        let last = &msgs.last().unwrap().1;
        assert!(last.starts_with("<memory>user likes tables</memory>"));
        assert!(last.trim_end().ends_with("anything?"));
    }

    #[test]
    fn prompt_injects_account_email_for_self_reference() {
        // When an account address is supplied, the system prompt must hand it
        // to the model and tell it to map first-person sender/recipient
        // references onto search_emails' from/to filters — otherwise the model
        // cannot resolve "emails I sent" into a filter at all.
        let msgs = build_prompt(&[], &[], "emails I sent", "en", "me@acme.com", tpl(), "");
        let sys = &msgs[0].1;
        assert!(
            sys.contains("me@acme.com"),
            "account email missing from system prompt: {sys}"
        );
        assert!(
            sys.contains("from=me@acme.com"),
            "missing from-filter guidance for self-reference: {sys}"
        );
        assert!(
            sys.contains("to=me@acme.com"),
            "missing to-filter guidance for self-reference: {sys}"
        );
    }

    #[test]
    fn prompt_omits_identity_line_without_account_email() {
        // No account on the turn → no leaked placeholder, no dangling sentence.
        let msgs = build_prompt(&[], &[], "hello", "en", "", tpl(), "");
        let sys = &msgs[0].1;
        assert!(!sys.contains("{{user_identity}}"), "placeholder leaked: {sys}");
        assert!(
            !sys.contains("YOUR USER'S IDENTITY"),
            "identity guidance present despite no account email: {sys}"
        );
    }

    #[test]
    fn prompt_advertises_citation_contract_and_few_shots() {
        // The new prompt rewrite must surface (a) the strict citation rule,
        // (b) the valid citation range, and (c) at least one few-shot example.
        let sources = vec![
            make_scored(1, "Kickoff", "reunión el martes 3 de marzo"),
            make_scored(2, "Proposal", "monthly fee drop to $1.5k"),
        ];
        let msgs = build_prompt(&sources, &[], "¿cuándo fue el kickoff?", "es", "", tpl(), "");
        let sys = &msgs[0].1;
        assert!(sys.contains("CITATION CONTRACT"), "missing citation contract section");
        assert!(sys.contains("Example 1"), "missing few-shot examples");
        // The per-turn valid range travels with the sources block in the
        // final user message.
        let last = &msgs.last().unwrap().1;
        assert!(last.contains("valid citation range: [1]..[2]"), "missing valid range");
    }

    #[test]
    fn prompt_includes_app_context_and_tools() {
        use crate::services::chat::tools::default_registry;
        let db = Database::new_for_testing().expect("test db");
        // The dynamic `Tools:` section is now rendered from the registry —
        // build it the same way `run_chat_turn` does and feed it in.
        let tools_section = default_registry().render_system_prompt_section(&db);
        let msgs = build_prompt(&[], &[], "hola", "es", "", tpl(), &tools_section);
        let sys = &msgs[0].1;
        // App identity so the model never claims it lacks mailbox access.
        assert!(sys.contains("EmailOps"));
        // Every always-on tool must be documented so the model knows what it
        // can call. Memory/task/lens/draft tools are gated behind Settings
        // and excluded from a default test DB; the always-on five must show.
        for tool in [
            "search_contacts",
            "search_emails",
            "get_email_body",
            "get_thread",
            "get_attachments",
        ] {
            assert!(sys.contains(tool), "system prompt missing tool: {}", tool);
        }
    }

    #[test]
    fn prompt_advertises_draft_tool_when_drafts_enabled() {
        use crate::services::chat::tools::default_registry;
        let db = Database::new_for_testing().expect("test db");
        // Drafts default to ON; confirm the LLM sees `generate_email_draft`
        // so it actually calls the tool instead of inventing a draft inline.
        let tools_section = default_registry().render_system_prompt_section(&db);
        let msgs = build_prompt(&[], &[], "draft a reply", "en", "", tpl(), &tools_section);
        let sys = &msgs[0].1;
        assert!(
            sys.contains("generate_email_draft"),
            "draft tool missing from prompt: {sys}"
        );
    }

    #[test]
    fn thread_bound_system_offers_draft_tool_when_drafts_enabled() {
        let thread = make_message("system", "[1] (id: e1) From: Alice\n    Subject: Hi\n\nthread body");
        let sys = build_thread_bound_system("BASE", std::slice::from_ref(&thread), true);
        // Base template and the seeded thread context are both present.
        assert!(sys.contains("BASE"));
        assert!(sys.contains("thread body"));
        // Draft tool is offered; the no-tools instruction must be absent so the
        // model actually calls `generate_email_draft` on an explicit request.
        assert!(sys.contains("generate_email_draft"), "draft tool not offered: {sys}");
        assert!(
            !sys.contains("Do not call any tools"),
            "no-tools instruction leaked: {sys}"
        );
    }

    #[test]
    fn thread_bound_system_forbids_tools_when_drafts_disabled() {
        let thread = make_message("system", "[1] (id: e1) From: Alice\n    Subject: Hi\n\nthread body");
        let sys = build_thread_bound_system("BASE", std::slice::from_ref(&thread), false);
        assert!(sys.contains("BASE"));
        assert!(sys.contains("thread body"));
        // With drafts off the model must be told it has no tools at all…
        assert!(
            sys.contains("You have NO tools on this turn"),
            "missing no-tools instruction: {sys}"
        );
        assert!(
            !sys.contains("generate_email_draft"),
            "draft tool leaked when disabled: {sys}"
        );
        // …and must be told to answer the request inline rather than explain
        // that it can't. The "tool is not available" apology leaked the
        // internal tool name into a user-facing reply.
        assert!(
            sys.contains("Never mention tools, tool names, or your own limitations"),
            "missing no-tool-talk instruction: {sys}"
        );
    }

    #[test]
    fn thread_bound_binds_every_chat_system_placeholder() {
        // `prompts::render` leaves unknown placeholders INTACT (prompts/mod.rs),
        // so any variable the thread-bound path forgets to bind is shipped to
        // the model verbatim. A literal "{{ tools_section }}" sitting where the
        // tool menu belongs is what made a translate request emit an invented
        // `generate_email_draft` call. This pins the exact variable set that
        // path binds, so adding one to `chat.system` fails here rather than in
        // production.
        let db = Database::new_for_testing().expect("test db");
        let template = crate::services::prompts::get_template(&db, "chat.system").expect("template");

        let mut vars = std::collections::HashMap::new();
        vars.insert("today", "2026-01-01".to_string());
        vars.insert("tomorrow", "2026-01-02".to_string());
        vars.insert("language_instruction", "Reply in Spanish.".to_string());
        vars.insert("tools_section", String::new());
        vars.insert("user_identity", String::new());

        let rendered = crate::services::prompts::render(&template, &vars);
        assert!(
            !rendered.contains("{{"),
            "unbound placeholder in thread-bound system prompt: {rendered}"
        );
    }

    // ── Draft-intent gating in thread-bound mode ────────────────────────
    //
    // Thread-bound turns expose exactly one tool. Without a gate the model
    // reaches for it on any imperative — "traduceme este email" saved a reply
    // draft instead of translating. Only an explicit draft/write/reply
    // request may put the tool on the menu.

    #[test]
    fn draft_intent_read_only_requests_expose_no_tool() {
        for prompt in [
            "traduceme este email al español",
            "traduce este correo",
            "resume este hilo",
            "resúmeme la conversación",
            "explica qué me está pidiendo",
            "¿de qué va este correo?",
            "translate this email to Spanish",
            "summarise this thread",
            "what is she asking for?",
            "fasse diesen Thread zusammen",
            "übersetze diese E-Mail",
            "résume ce fil",
            "traduis ce courriel",
        ] {
            assert!(!wants_email_draft(prompt), "should NOT offer draft tool: {prompt}");
        }
    }

    #[test]
    fn draft_intent_explicit_requests_expose_the_tool() {
        for prompt in [
            "escribe una respuesta",
            "escríbele una respuesta a Nadia",
            "redacta un borrador",
            "respóndele que sí",
            "contesta este correo",
            "draft a reply",
            "write a reply to Nadia",
            "reply to this email",
            "compose a response",
            "schreibe eine Antwort",
            "antworte ihr",
            "écris une réponse",
            "rédige un brouillon",
        ] {
            assert!(wants_email_draft(prompt), "should offer draft tool: {prompt}");
        }
    }

    #[test]
    fn draft_intent_folds_accents_and_case() {
        // Accented imperatives are the common Spanish form; the matcher must
        // not depend on the user typing them unaccented.
        assert!(wants_email_draft("RESPÓNDELE"));
        assert!(wants_email_draft("Redacta un Borrador"));
        assert!(!wants_email_draft("TRADÚCEME ESTE EMAIL"));
    }

    #[test]
    fn draft_intent_ignores_substring_collisions() {
        // "borrador"/"reply" must match as words, not inside unrelated ones.
        assert!(!wants_email_draft("el correo es irreplicable"));
        assert!(!wants_email_draft(""));
    }

    // ── Turn-mode planning (ambient view context) ───────────────────────

    #[test]
    fn turn_mode_is_rag_without_any_context() {
        assert_eq!(plan_turn_mode(0, None, None), ChatTurnMode::Rag);
    }

    #[test]
    fn turn_mode_is_conversation_bound_when_seeded_with_thread() {
        // "Chat about this thread" seeds a role='system' message at creation;
        // that binding owns the whole conversation.
        assert_eq!(plan_turn_mode(1, None, None), ChatTurnMode::ConversationThread);
    }

    #[test]
    fn turn_mode_uses_ambient_thread_when_view_has_one_open() {
        // Right-hand chat panel: the thread the user is looking at grounds
        // this turn only.
        assert_eq!(
            plan_turn_mode(0, Some("t-42"), None),
            ChatTurnMode::AmbientThread {
                thread_id: "t-42".to_string(),
                account_id: None
            }
        );
    }

    #[test]
    fn ambient_thread_carries_its_own_account() {
        // Regression: in unified ("All accounts") mode the panel is handed the
        // *first enabled* account, which usually does not own the thread the
        // user is reading. `get_thread(account, thread)` then returns nothing,
        // `build_thread_context` errors NotFound, and the turn silently falls
        // back to retrieval — the user asks "resume el correo" with an email
        // open and is told the model doesn't know which email they mean.
        // The thread's own account must ride along with the thread id.
        assert_eq!(
            plan_turn_mode(0, Some("t-42"), Some("acct-owning-t-42")),
            ChatTurnMode::AmbientThread {
                thread_id: "t-42".to_string(),
                account_id: Some("acct-owning-t-42".to_string()),
            }
        );
    }

    #[test]
    fn ambient_thread_without_account_falls_back_to_the_turn_account() {
        // Single-account installs (and any older caller) send no account; the
        // turn's own account is correct there, so `None` must stay valid
        // rather than disabling the grounding.
        assert_eq!(
            plan_turn_mode(0, Some("t-42"), None),
            ChatTurnMode::AmbientThread {
                thread_id: "t-42".to_string(),
                account_id: None,
            }
        );
    }

    #[test]
    fn blank_ambient_account_id_is_ignored() {
        // Same defensive treatment the thread id already gets: an empty string
        // must not become an account lookup that matches nothing.
        assert_eq!(
            plan_turn_mode(0, Some("t-42"), Some("   ")),
            ChatTurnMode::AmbientThread {
                thread_id: "t-42".to_string(),
                account_id: None,
            }
        );
    }

    #[test]
    fn conversation_binding_wins_over_ambient_thread() {
        // A conversation explicitly created about thread A must not silently
        // re-point at thread B just because the user scrolled to it.
        assert_eq!(plan_turn_mode(1, Some("t-99"), None), ChatTurnMode::ConversationThread);
    }

    #[test]
    fn blank_ambient_thread_id_is_ignored() {
        // Defensive: an empty string from the frontend must not be treated as
        // a real thread and send the turn down the grounded path with no
        // context at all.
        assert_eq!(plan_turn_mode(0, Some(""), None), ChatTurnMode::Rag);
        assert_eq!(plan_turn_mode(0, Some("   "), None), ChatTurnMode::Rag);
    }

    #[test]
    fn prompt_hides_lens_tools_when_lenses_disabled() {
        use crate::services::chat::tools::default_registry;
        let db = Database::new_for_testing().expect("test db");
        // Lenses default OFF — confirm the section omits them entirely so a
        // user who never enabled the feature doesn't get tool calls for it.
        let tools_section = default_registry().render_system_prompt_section(&db);
        let msgs = build_prompt(&[], &[], "show me invoices lens", "en", "", tpl(), &tools_section);
        let sys = &msgs[0].1;
        assert!(!sys.contains("get_lens_data"), "lens tool leaked when feature off");
        assert!(!sys.contains("list_lenses"), "lens tool leaked when feature off");
    }

    // ── Direct-tool shortcuts ───────────────────────────────────────────
    //
    // These prompts are the exact strings the "Resumen de hoy", "Esta
    // semana" and "Pendientes" quick-access buttons in ChatView.tsx send.
    // Regression test: the heuristic must recognise them so the chat skips
    // the LLM's tool-choice round entirely (saves ~2–6 s on local models).

    #[test]
    fn direct_shortcut_matches_today_summary_button_prompt() {
        let prompt = "Hazme un resumen de los emails que he recibido hoy. \
Formatéalo como una tabla markdown con las columnas | Remitente | Asunto | Hora | Urgencia | Resumen |, \
ordenados por urgencia. Cita cada email con su número de referencia. \
Termina con un párrafo breve destacando lo más importante del día.";
        let calls = heuristic_direct_tools(prompt, false).expect("today shortcut must match the button prompt");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "search_emails");
        let args = &calls[0].function.arguments;
        assert!(args.get("since").is_some(), "must pass a since=today bound");
        assert!(args.get("until").is_some(), "must pass an until=tomorrow bound");
        assert_eq!(
            args.get("include_bodies").and_then(|v| v.as_bool()),
            Some(true),
            "summary shortcut must preseed full bodies so the model need not call get_email_body"
        );
    }

    #[test]
    fn direct_shortcut_matches_this_week_button_prompt() {
        let prompt = "Resume los emails más importantes de esta semana. \
Usa una tabla markdown con columnas | Día | Remitente | Asunto | Tema | Acción sugerida |, \
ordenados cronológicamente. Cita cada entrada. \
Termina con dos o tres frases sobre los temas dominantes de la semana.";
        let calls = heuristic_direct_tools(prompt, false).expect("week shortcut must match the button prompt");
        assert_eq!(calls[0].function.name, "search_emails");
        assert_eq!(
            calls[0]
                .function
                .arguments
                .get("include_bodies")
                .and_then(|v| v.as_bool()),
            Some(true),
            "week summary shortcut must preseed full bodies too"
        );
    }

    #[test]
    fn week_summary_preseeds_calendar_events_when_account_has_calendar() {
        let calls = heuristic_direct_tools("summary of this week", true).expect("week shortcut must match");
        assert_eq!(calls.len(), 2, "email search + calendar events");
        assert_eq!(calls[0].function.name, "search_emails");
        assert_eq!(calls[1].function.name, "list_calendar_events");
        // Both cover the same Monday..next-Monday window.
        assert_eq!(
            calls[0].function.arguments.get("since"),
            calls[1].function.arguments.get("since")
        );
        assert_eq!(
            calls[0].function.arguments.get("until"),
            calls[1].function.arguments.get("until")
        );
    }

    #[test]
    fn week_summary_skips_calendar_without_integration() {
        let calls = heuristic_direct_tools("resumen de la semana", false).expect("week shortcut must match");
        assert_eq!(calls.len(), 1, "IMAP-only accounts get the email summary only");
        assert_eq!(calls[0].function.name, "search_emails");
    }

    #[test]
    fn today_summary_is_unaffected_by_calendar_availability() {
        let calls = heuristic_direct_tools("summary of today", true).expect("today shortcut must match");
        assert_eq!(calls.len(), 1, "the daily summary stays email-only");
        assert_eq!(calls[0].function.name, "search_emails");
    }

    #[test]
    fn direct_shortcut_matches_pending_button_prompt() {
        let prompt = "Identifica los emails que requieren mi respuesta o acción. \
Preséntalos en una tabla markdown …";
        let calls = heuristic_direct_tools(prompt, false).expect("pending shortcut must match the button prompt");
        assert_eq!(calls[0].function.name, "list_pending_tasks");
    }

    /// Serialise tests that mutate the global `services::clock` registry —
    /// otherwise a parallel test that reads `now_secs()` may observe the
    /// pinned date and assert the wrong thing.
    fn clock_lock() -> std::sync::MutexGuard<'static, ()> {
        static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
        M.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn build_prompt_reads_today_from_installed_clock() {
        // Pinning the clock to 2024-01-15 must surface "2024-01-15" / "2024-01-16"
        // in the rendered system message — proves the prompt's idea of "today"
        // comes from the seam, not bare `Utc::now()`. This is the load-bearing
        // guarantee for eval cases that pin `as_of:` against a static fixture.
        let _g = clock_lock();
        let pinned = chrono::NaiveDate::from_ymd_opt(2024, 1, 15)
            .expect("valid date")
            .and_hms_opt(0, 0, 0)
            .expect("valid time");
        let pinned_secs = pinned.and_utc().timestamp();
        let clock = crate::services::clock::install_for_testing(pinned_secs);

        let msgs = build_prompt(&[], &[], "q", "en", "", tpl(), "");
        let system = &msgs[0].1;
        assert!(
            system.contains("2024-01-15"),
            "system msg must reflect pinned today, got: {system}"
        );
        assert!(
            system.contains("2024-01-16"),
            "system msg must reflect pinned tomorrow, got: {system}"
        );

        // Reset so unrelated tests see the real clock again.
        let _ = clock;
        crate::services::clock::install(std::sync::Arc::new(crate::services::clock::SystemClock));
    }

    #[test]
    fn direct_today_shortcut_uses_installed_clock_for_search_window() {
        // The today-summary heuristic computes its since/until window via
        // `now_utc().date_naive()`. Pinning the clock must produce a window
        // anchored to the pinned day — the bug the demo_daily_summary case
        // hit was that this window came from wall-clock and missed every
        // email in the frozen demo dataset.
        let _g = clock_lock();
        let pinned = chrono::NaiveDate::from_ymd_opt(2024, 1, 15)
            .expect("valid date")
            .and_hms_opt(0, 0, 0)
            .expect("valid time");
        crate::services::clock::install_for_testing(pinned.and_utc().timestamp());

        let calls = heuristic_direct_tools("summarize today's client emails", false)
            .expect("today shortcut must match the demo prompt");
        let args = &calls[0].function.arguments;
        assert_eq!(args.get("since").and_then(|v| v.as_str()), Some("2024-01-15"));
        assert_eq!(args.get("until").and_then(|v| v.as_str()), Some("2024-01-16"));

        crate::services::clock::install(std::sync::Arc::new(crate::services::clock::SystemClock));
    }

    #[test]
    fn direct_shortcut_today_ignored_when_week_also_mentioned() {
        // "resumen" + "hoy" + "semana" — ambiguous; bail out of the today
        // shortcut rather than answering the wrong window.
        let out = heuristic_direct_tools("resumen de hoy y de la semana", false);
        // Week shortcut wins because "has_month" is false and the today
        // branch is skipped when has_week is true.
        let calls = out.expect("should fall through to week shortcut");
        let args = &calls[0].function.arguments;
        let since = args.get("since").and_then(|v| v.as_str()).unwrap_or("");
        // The week bound must start on a Monday — not on "today".
        let monday = chrono::NaiveDate::parse_from_str(since, "%Y-%m-%d").unwrap();
        assert_eq!(monday.weekday(), chrono::Weekday::Mon);
    }
    // ── XML tool-call rescue ────────────────────────────────────────────
    //
    // Some small models emit tool calls as plain `<tool_call>` text
    // instead of through the JSON tool_calls channel. `parse_xml_tool_calls`
    // salvages those so the loop can still dispatch them.

    #[test]
    fn parse_xml_tool_calls_returns_empty_for_plain_text() {
        assert!(parse_xml_tool_calls("just a regular answer with no XML").is_empty());
    }

    #[test]
    fn parse_xml_tool_calls_extracts_single_call_with_one_param() {
        let text = "<tool_call>\n<function=get_email_body>\n<parameter=email_id>\n19e6e27f48f95297\n</parameter>\n</function>\n</tool_call>";
        let calls = parse_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "get_email_body");
        assert_eq!(calls[0].function.arguments["email_id"], "19e6e27f48f95297");
    }

    #[test]
    fn parse_xml_tool_calls_extracts_multiple_calls() {
        // The actual failure mode that triggered this rescue: Qwen 3.5 4B
        // dumped four `get_email_body` calls back-to-back as text after a
        // `search_emails` returned four hits.
        let text = "<tool_call>\n<function=get_email_body>\n<parameter=email_id>\na\n</parameter>\n</function>\n</tool_call>\n\
                    <tool_call>\n<function=get_email_body>\n<parameter=email_id>\nb\n</parameter>\n</function>\n</tool_call>";
        let calls = parse_xml_tool_calls(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.arguments["email_id"], "a");
        assert_eq!(calls[1].function.arguments["email_id"], "b");
    }

    #[test]
    fn parse_xml_tool_calls_promotes_integer_params_to_numbers() {
        // `limit=25` must be a JSON number so it matches the tool schema's
        // integer type — leaving it as a string would silently break the
        // search_emails call.
        let text = "<tool_call><function=search_emails><parameter=limit>25</parameter><parameter=query>foo</parameter></function></tool_call>";
        let calls = parse_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert!(calls[0].function.arguments["limit"].is_number());
        assert_eq!(calls[0].function.arguments["limit"].as_i64(), Some(25));
        assert_eq!(calls[0].function.arguments["query"], "foo");
    }

    #[test]
    fn parse_xml_tool_calls_skips_blocks_with_missing_close_tag() {
        // Truncated stream — keep the well-formed calls, drop the
        // malformed one. Don't panic.
        let text = "<tool_call><function=ok><parameter=k>v</parameter></function></tool_call>\
                    <tool_call><function=broken><parameter=k>v";
        let calls = parse_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "ok");
    }

    // Qwen 3.6 (35B-A3B) emits the OTHER `<tool_call>` shape: a JSON body
    // `{"name":…,"arguments":…}` inside the tags, with no `<function=>`
    // sub-element. When this leaks as text (e.g. a tools-free synthesis pass),
    // the salvage must still recover it — otherwise the markup is stripped and
    // the user gets an empty reply.

    #[test]
    fn parse_xml_tool_calls_handles_json_body_inside_tool_call() {
        let text = r#"<tool_call>{"name":"get_email_body","arguments":{"email_id":"19e9151c42eba1e7"}}</tool_call>"#;
        let calls = parse_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "get_email_body");
        assert_eq!(calls[0].function.arguments["email_id"], "19e9151c42eba1e7");
    }

    #[test]
    fn parse_xml_tool_calls_handles_json_body_with_nested_name() {
        // Qwen 3.6's exact final-round output: `name` nested inside
        // `arguments`, no top-level `name`. Hoist it so the call dispatches
        // and strip it from the args the tool receives.
        let text = r#"<tool_call>{"arguments":{"email_id":"19e9151c42eba1e7","name":"get_email_body"}}</tool_call>"#;
        let calls = parse_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "get_email_body");
        assert_eq!(calls[0].function.arguments["email_id"], "19e9151c42eba1e7");
        assert!(
            calls[0].function.arguments.get("name").is_none(),
            "nested name must be stripped from dispatched args"
        );
    }

    #[test]
    fn parse_xml_tool_calls_json_body_defaults_missing_arguments_to_empty() {
        let text = r#"<tool_call>{"name":"list_drafts"}</tool_call>"#;
        let calls = parse_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "list_drafts");
        assert!(calls[0]
            .function
            .arguments
            .as_object()
            .map(|o| o.is_empty())
            .unwrap_or(false));
    }

    #[test]
    fn parse_xml_tool_calls_parses_newline_batched_calls_without_close_tags() {
        // Verbatim production emission: batched calls as newline-separated
        // `<tool_call>{json}` lines with NO closing tags. The next open tag
        // (or end of text) is an implicit close.
        let text = "<tool_call>{\"arguments\":{\"email_id\":\"e1\"},\"name\":\"get_email_body\"}\n\
<tool_call>{\"arguments\":{\"email_id\":\"e2\"},\"name\":\"get_email_body\"}\n";
        let calls = parse_xml_tool_calls(text);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.arguments["email_id"], "e1");
        assert_eq!(calls[1].function.arguments["email_id"], "e2");
    }

    #[test]
    fn parse_unnamed_tool_calls_handles_missing_close_tags() {
        // Nameless variant of the batched shape — arg-key inference still
        // resolves each line once the implicit close splits them.
        let text =
            "<tool_call>{\"arguments\":{\"email_id\":\"e1\"}}\n<tool_call>{\"arguments\":{\"email_id\":\"e2\"}}\n";
        let calls = parse_unnamed_tool_calls(text, &tool_schemas_fixture());
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.name, "get_email_body");
        assert_eq!(calls[1].function.arguments["email_id"], "e2");
    }

    #[test]
    fn parse_xml_tool_calls_salvages_flattened_args_with_trailing_brace() {
        // Verbatim production emission (Qwen 3.6 35B): args flattened next to
        // `name` (no `arguments` wrapper) AND a trailing extra brace. Strict
        // JSON parsing rejects the block, so no tool ever ran.
        let text = "<tool_call>{\"name\":\"search_emails\",\"from\":\"sharique\",\"limit\":25}}\n</tool_call>";
        let calls = parse_xml_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "search_emails");
        assert_eq!(
            calls[0].function.arguments,
            serde_json::json!({"from":"sharique","limit":25})
        );
    }

    // ── Repeated-tool-call detection (no-progress loop guard) ───────────

    fn tc(name: &str, args: serde_json::Value) -> crate::ai::provider::AiToolCall {
        use crate::ai::provider::{AiToolCall, AiToolCallFunction};
        AiToolCall {
            function: AiToolCallFunction {
                name: name.to_string(),
                arguments: args,
            },
        }
    }

    #[test]
    fn tool_call_key_is_argument_order_independent() {
        // qwen3.5-9b re-issued the same search every round; the key must match
        // regardless of the order the model serialised the args in.
        let a = tc(
            "search_emails",
            serde_json::json!({"since":"2026-06-30","limit":25,"include_bodies":true}),
        );
        let b = tc(
            "search_emails",
            serde_json::json!({"limit":25,"include_bodies":true,"since":"2026-06-30"}),
        );
        assert_eq!(tool_call_key(&a), tool_call_key(&b));
    }

    #[test]
    fn tool_call_key_differs_on_name_or_args() {
        let base = tc("search_emails", serde_json::json!({"limit":25}));
        assert_ne!(
            tool_call_key(&base),
            tool_call_key(&tc("search_emails", serde_json::json!({"limit":5}))),
            "different args → different key"
        );
        assert_ne!(
            tool_call_key(&base),
            tool_call_key(&tc("get_email_body", serde_json::json!({"limit":25}))),
            "different tool → different key"
        );
    }

    // ── Nameless tool-call inference (Qwen 3.6 batched no-think) ─────────
    //
    // Under no-think Qwen 3.6 batches body reads but drops the function name:
    //   <tool_call>{"arguments":{"email_id":"…"}}</tool_call>
    // We recover the target by matching argument keys to a unique tool schema.

    fn tool_schemas_fixture() -> Vec<ToolArgKeys> {
        vec![
            ("get_email_body", vec!["email_id".into()], vec!["email_id".into()]),
            (
                "search_emails",
                vec!["query".into(), "from".into(), "to".into(), "limit".into()],
                vec![],
            ),
            // generate_email_draft ALSO takes an optional `email_id` (no required
            // params) → a naive subset match makes {email_id} ambiguous;
            // specificity (smallest schema) drops it.
            (
                "generate_email_draft",
                vec!["email_id".into(), "to".into(), "subject".into(), "instructions".into()],
                vec![],
            ),
            // get_attachments has the SAME single-key schema as get_email_body —
            // an irreducible tie that only the preference order can break.
            ("get_attachments", vec!["email_id".into()], vec!["email_id".into()]),
            ("list_drafts", vec!["limit".into()], vec![]),
        ]
    }

    #[test]
    fn infer_tool_from_unique_arg_keys() {
        let tools = tool_schemas_fixture();
        // {email_id} fits get_email_body, generate_email_draft AND get_attachments.
        // generate_email_draft loses on specificity; get_email_body beats the
        // equally-specific get_attachments via the preference tiebreak.
        assert_eq!(
            infer_tool_from_arg_keys(&["email_id".into()], &tools),
            Some("get_email_body")
        );
        // {query} fits only search_emails.
        assert_eq!(
            infer_tool_from_arg_keys(&["query".into()], &tools),
            Some("search_emails")
        );
    }

    #[test]
    fn infer_tool_refuses_ambiguous_or_unknown() {
        let tools = tool_schemas_fixture();
        // Empty args match every no-required tool → ambiguous → None.
        assert_eq!(infer_tool_from_arg_keys(&[], &tools), None);
        // A key no tool declares → None (don't dispatch a guess).
        assert_eq!(infer_tool_from_arg_keys(&["nonsense".into()], &tools), None);
        // Missing a required key → no match.
        assert_eq!(
            infer_tool_from_arg_keys(&["from".into()], &tools),
            Some("search_emails"),
            "from is a valid search_emails property with no unmet required"
        );
    }

    #[test]
    fn parse_unnamed_tool_calls_recovers_qwen36_batched_bodies() {
        // The exact failing output: five nameless get_email_body blocks.
        let text = "<tool_call>{\"arguments\":{\"email_id\":\"19efda3692fed705\"}}\n</tool_call>\
                    <tool_call>{\"arguments\":{\"email_id\":\"19ed97b980f720e0\"}}\n</tool_call>\
                    <tool_call>{\"arguments\":{\"email_id\":\"19eb56f57e174e7e\"}}\n</tool_call>";
        let calls = parse_unnamed_tool_calls(text, &tool_schemas_fixture());
        assert_eq!(calls.len(), 3);
        assert!(calls.iter().all(|c| c.function.name == "get_email_body"));
        assert_eq!(calls[0].function.arguments["email_id"], "19efda3692fed705");
        assert_eq!(calls[2].function.arguments["email_id"], "19eb56f57e174e7e");
    }

    #[test]
    fn parse_unnamed_tool_calls_skips_blocks_it_cannot_resolve() {
        // Named blocks are left for the named parsers; unresolvable args are dropped.
        let text = "<tool_call>{\"name\":\"get_email_body\",\"arguments\":{\"email_id\":\"a\"}}</tool_call>\
                    <tool_call>{\"arguments\":{\"mystery\":\"x\"}}</tool_call>";
        let calls = parse_unnamed_tool_calls(text, &tool_schemas_fixture());
        assert!(calls.is_empty(), "named block skipped, unknown-arg block refused");
    }

    // ── Python-call tool-call rescue ────────────────────────────────────
    //
    // Same idea as the XML rescue, but for the *other* malformed shape
    // Qwen 3.5 4B emits when it fumbles the tool-call channel:
    //
    //   tool_call: get_email_body(email_id="19e73d15a43b67e8")
    //
    // The exact user-reported failure was this line preceded by
    // "Voy a obtener el contenido del email..." narration. The salvage
    // must keep only the `tool_call:` lines so a model that confidently
    // writes a python-shaped sentence in prose ("I will use `foo(x=1)`
    // here") doesn't trigger a phantom tool execution.

    #[test]
    fn parse_python_call_tool_calls_returns_empty_for_plain_text() {
        assert!(parse_python_call_tool_calls("just a normal answer, no calls here", &[]).is_empty());
    }

    #[test]
    fn parse_python_call_tool_calls_extracts_single_call_with_string_arg() {
        let text = "tool_call: get_email_body(email_id=\"19e73d15a43b67e8\")";
        let calls = parse_python_call_tool_calls(text, &[]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "get_email_body");
        assert_eq!(calls[0].function.arguments["email_id"], "19e73d15a43b67e8");
    }

    #[test]
    fn parse_python_call_tool_calls_tolerates_narration_prefix() {
        // The exact failure mode from the bug report — the model
        // narrates a plan first ("Voy a obtener..."), then emits the
        // tool_call line. The salvage must find the call regardless.
        let text = "Voy a obtener el contenido del email que mencioné en la tabla anterior.\n\
                    tool_call: get_email_body(email_id=\"19e73d15a43b67e8\")";
        let calls = parse_python_call_tool_calls(text, &[]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "get_email_body");
        assert_eq!(calls[0].function.arguments["email_id"], "19e73d15a43b67e8");
    }

    #[test]
    fn parse_python_call_tool_calls_promotes_integer_kwargs_to_numbers() {
        // Same reason as the XML rescue — `limit=25` must be a JSON
        // number to match the tool schema's integer type.
        let text = "tool_call: search_emails(query=\"foo\", limit=25)";
        let calls = parse_python_call_tool_calls(text, &[]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.arguments["query"], "foo");
        assert!(calls[0].function.arguments["limit"].is_number());
        assert_eq!(calls[0].function.arguments["limit"].as_i64(), Some(25));
    }

    #[test]
    fn parse_python_call_tool_calls_handles_multiple_calls_on_separate_lines() {
        let text = "tool_call: get_email_body(email_id=\"a\")\n\
                    tool_call: get_email_body(email_id=\"b\")";
        let calls = parse_python_call_tool_calls(text, &[]);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function.arguments["email_id"], "a");
        assert_eq!(calls[1].function.arguments["email_id"], "b");
    }

    #[test]
    fn parse_python_call_tool_calls_ignores_function_shaped_prose_without_prefix() {
        // Critical false-positive guard: assistant prose that *describes*
        // a tool call (without the `tool_call:` prefix) must NOT trigger
        // a salvage. Otherwise an answer like "I used search_emails(...)
        // to find it" would re-invoke the search every render.
        let text = "I called search_emails(query=\"foo\", limit=25) to find it.";
        assert!(parse_python_call_tool_calls(text, &[]).is_empty());
    }

    #[test]
    fn parse_python_call_tool_calls_does_not_panic_on_multibyte_leading_chars() {
        // Real-world panic captured after the 0.1.147 migration when the
        // assistant produced Spanish prose. The old `[..10]` byte slice
        // cut through `á` (bytes 9..11 of `están`) and panicked with
        // "byte index 10 is not a char boundary". This parser now runs on
        // EVERY assistant turn (not just salvage), so any multibyte leading
        // character in real prose would trigger the panic.
        let cases = [
            "Aquí están los últimos resultados:", // the exact byte boundary that panicked
            "Sí — encontré 5 emails.",
            "Café con leche",                   // single non-ASCII byte (under 10 chars total)
            "❤️",                               // emoji (very multi-byte)
            "α β γ tool_call: search_emails()", // Greek letters before the prefix; must NOT match
        ];
        for text in cases {
            // The point of the test is "doesn't panic"; the result is
            // expected to be empty for all of these because none are a
            // structural `tool_call:` prefix at byte 0 of a line.
            let calls = parse_python_call_tool_calls(text, &[]);
            assert!(calls.is_empty(), "unexpected match for {text:?}: {calls:?}");
        }
    }

    #[test]
    fn parse_python_call_tool_calls_accepts_single_quoted_strings() {
        // Models alternate between " and ' depending on locale / mood.
        let text = "tool_call: get_email_body(email_id='abc-123')";
        let calls = parse_python_call_tool_calls(text, &[]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.arguments["email_id"], "abc-123");
    }

    #[test]
    fn parse_python_call_tool_calls_salvages_bare_registered_tool_line_after_narration() {
        // Exact failure mode: model narrates first, then emits a bare
        // `name(args)` line with no `tool_call:` prefix. As long as
        // `name` matches a registered tool, the line is structurally
        // a function call → salvage it.
        let text = "I can find Lena Park's most recent email about the renewal terms to draft the reply for you.\n\
                    search_emails(from=\"lena.park@orbitfreight.co\", limit=1, since=\"2026-05-26\")";
        let calls = parse_python_call_tool_calls(text, &["search_emails", "get_email_body"]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "search_emails");
        assert_eq!(calls[0].function.arguments["from"], "lena.park@orbitfreight.co");
        assert_eq!(calls[0].function.arguments["limit"].as_i64(), Some(1));
        assert_eq!(calls[0].function.arguments["since"], "2026-05-26");
    }

    #[test]
    fn parse_python_call_tool_calls_tolerates_trailing_punctuation_on_bare_line() {
        // Some models terminate the call with a period or comma.
        let text = "search_emails(from=\"a@b.com\", limit=1).";
        let calls = parse_python_call_tool_calls(text, &["search_emails"]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "search_emails");
    }

    #[test]
    fn parse_python_call_tool_calls_ignores_bare_unregistered_tool_line() {
        // Salvage is registry-bounded — an unknown identifier is treated
        // as prose, not as a phantom call.
        let text = "fabricate_facts(x=1)";
        assert!(parse_python_call_tool_calls(text, &["search_emails"]).is_empty());
    }

    #[test]
    fn parse_python_call_tool_calls_ignores_registered_tool_name_inside_prose() {
        // Even with `search_emails` in the registry, prose surrounding
        // the call must NOT trigger a salvage. The line must be
        // structurally a function call after trim.
        let text = "I called search_emails(query=\"foo\") to find it and it worked.";
        assert!(parse_python_call_tool_calls(text, &["search_emails"]).is_empty());
    }
}
