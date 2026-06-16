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
/// output back to the LLM. Always compiled so dev builds (without the
/// `tracing` feature) can still capture LLM I/O for the in-app debug panel.
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
/// when enabled). Always compiled — see `format_messages_for_trace`.
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

    let mut tpl_vars = std::collections::HashMap::new();
    tpl_vars.insert("today", today);
    tpl_vars.insert("tomorrow", tomorrow);
    tpl_vars.insert("language_instruction", language_instruction);
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
const MAX_TOOL_ROUNDS: usize = 5;

/// Per-tool dispatch result: the text the LLM sees plus the structural
/// allowlists this tool contributed (email ids + draft ids). Callers fold
/// the refs into per-turn accumulators that end up on the assistant
/// `ChatMessage` as `referenced_email_ids` / `referenced_draft_ids`.
struct DispatchedTool {
    text: String,
    email_refs: Vec<String>,
    draft_refs: Vec<String>,
}

/// Dispatch one tool call through the registry: look up the tool (honouring
/// feature gating), execute it, emit any `ToolEffect`s as `chat-tool-effect`
/// Tauri events for the frontend to react to, and return the text the LLM
/// will see as the tool-result message along with the email-id allowlist
/// this tool produced.
async fn dispatch_tool(
    registry: &tools::ToolRegistry,
    db: &Arc<Database>,
    account_id: &str,
    categories: &[String],
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
            match tool.execute(&ctx, args).await {
                Ok(out) => {
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
                    }
                }
                Err(e) => DispatchedTool {
                    text: format!("Tool '{name}' error: {e}"),
                    email_refs: Vec::new(),
                    draft_refs: Vec::new(),
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
    rt.block_on(dispatch_tool(&registry, db, account_id, categories, name, args))
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
pub(crate) fn parse_xml_tool_calls(text: &str) -> Vec<crate::ai::provider::AiToolCall> {
    use crate::ai::provider::{AiToolCall, AiToolCallFunction};

    const OPEN: &str = "<tool_call>";
    const CLOSE: &str = "</tool_call>";
    const FN_OPEN: &str = "<function=";
    const PARAM_OPEN: &str = "<parameter=";
    const PARAM_CLOSE: &str = "</parameter>";

    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel_open) = text[cursor..].find(OPEN) {
        let after_open = cursor + rel_open + OPEN.len();
        let close = match text[after_open..].find(CLOSE) {
            Some(c) => after_open + c,
            None => break,
        };
        let block = &text[after_open..close];
        cursor = close + CLOSE.len();

        // Function name: <function=NAME>...
        let fn_start = match block.find(FN_OPEN) {
            Some(s) => s + FN_OPEN.len(),
            None => continue,
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
fn heuristic_direct_tools(user_question: &str) -> Option<Vec<crate::ai::provider::AiToolCall>> {
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

    // Summary of this week's emails (EN + ES).
    if has_week && has_summary && !has_month {
        let now = now_utc().date_naive();
        let days_since_monday = now.weekday().num_days_from_monday() as i64;
        let monday = now - chrono::Duration::days(days_since_monday);
        let next_monday = monday + chrono::Duration::days(7);
        return Some(search_since_until(monday, next_monday));
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

/// Decide how to produce the answer from the tool loop's final message list.
/// Pure so the routing is unit-testable without a provider or `AppHandle`.
///
/// A trailing assistant message with real text and no pending tool_calls is a
/// finished answer; anything else (a tool result, a still-open tool call, a
/// blank assistant turn) needs a streamed synthesis pass.
fn plan_answer(final_messages: Vec<AiMessage>) -> AnswerPlan {
    let is_direct = final_messages
        .last()
        .map(|m| {
            m.role == "assistant"
                && m.tool_calls.as_ref().map(|v| v.is_empty()).unwrap_or(true)
                && !m.content.trim().is_empty()
        })
        .unwrap_or(false);
    if is_direct {
        // Guarded by `is_direct`, so `last()` is Some.
        let content = final_messages.last().map(|m| m.content.clone()).unwrap_or_default();
        AnswerPlan::DirectText(content)
    } else {
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

async fn run_tool_loop(
    db: &Arc<Database>,
    registry: &Arc<tools::ToolRegistry>,
    provider: &dyn AIProvider,
    conversation_id: &str,
    message_id: &str,
    account_id: &str,
    categories: &[String],
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
                let dispatched = dispatch_tool(registry, db, account_id, categories, name, args.clone()).await;
                let elapsed_ms = t_tool.elapsed().as_millis() as i64;
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
                    arguments: args.clone(),
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
            let mut parsed = parse_xml_tool_calls(&response.content);
            let mut kind = "XML";
            if parsed.is_empty() {
                let known_tools = registry.names();
                parsed = parse_python_call_tool_calls(&response.content, &known_tools);
                kind = "python-call";
            }
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

        // Push the assistant's tool-call message so the history stays coherent.
        had_any_answer = true;
        messages.push(response);

        for tc in &tool_calls {
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
            let dispatched = dispatch_tool(registry, db, account_id, categories, name, args.clone()).await;
            let elapsed_ms = t_tool.elapsed().as_millis() as i64;
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
                arguments: args.clone(),
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
using ONLY that thread as your source. Do not call any tools, do not search \
other emails. If the thread does not contain enough information to answer, say \
so plainly.",
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
    let mut tpl_vars = std::collections::HashMap::new();
    tpl_vars.insert("today", today);
    tpl_vars.insert("tomorrow", tomorrow);
    tpl_vars.insert("language_instruction", language_instruction);
    let system_template = crate::services::prompts::get_template(&db, "chat.system")?;
    let base_system = crate::services::prompts::render(&system_template, &tpl_vars);

    // Drafts are gated behind a Settings toggle (defaults ON). When enabled we
    // expose exactly one tool — `generate_email_draft` — so the user can ask
    // for a reply grounded in this thread; otherwise the model is told to use
    // no tools at all and just answer from the thread.
    let drafts_available = db.is_ai_drafts_enabled().unwrap_or(true);
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

    // Draft-only registry: the single tool the thread-bound model may call.
    // `run_tool_loop` further gates it on `is_available(db)`, so a disabled
    // drafts feature yields an empty tool menu and a pure-text answer.
    let draft_tool: Arc<dyn tools::Tool> = Arc::new(tools::generate_email_draft::GenerateEmailDraftTool);
    let registry: Arc<tools::ToolRegistry> = Arc::new(tools::ToolRegistry::with_tools(vec![draft_tool]));

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
) -> Result<()> {
    let turn_start = std::time::Instant::now();

    // Build the configured AI provider from DB preferences. Falls back to
    // Ollama if the provider preference is missing or unrecognised.
    let provider = AiService::load_provider(&db)?;

    // ── Thread-bound short-circuit ─────────────────────────────────────
    // If the conversation was seeded with an email thread (system-role
    // message inserted by `create_conversation_with_thread`), skip the
    // entire route/retrieval/tool-loop pipeline and answer using just the
    // cleaned thread as context.
    let system_messages = db.get_chat_system_messages(&conversation_id).unwrap_or_default();
    if !system_messages.is_empty() {
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
    let route = classify_route(&db, &user_question);
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
    let preseeded_tool_calls = heuristic_direct_tools(&user_question);
    if preseeded_tool_calls.is_some() {
        emit_log("info", "shortcut: matched direct-tool pattern");
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
    let mut initial_messages = build_prompt(
        &sources,
        &history,
        &user_question,
        ai_language.english_name(),
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
        aggregated_email_refs,
        aggregated_draft_refs,
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
    let stream_result: Result<crate::ai::provider::ChatStreamResult> = if loop_failed_without_answer {
        let detail = loop_error
            .clone()
            .unwrap_or_else(|| "tool-call loop failed before producing any answer".to_string());
        Err(AppError::AiError(detail))
    } else {
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
            // No direct text answer: the loop handed back tool results (a
            // preseeded shortcut, or a run that hit MAX_TOOL_ROUNDS). Synthesise
            // the final answer by STREAMING from those tool results so the user
            // sees tokens within a second or two. Bounded by STREAM_TIMEOUT so a
            // stuck provider can't leave the UI thinking forever.
            AnswerPlan::StreamSynthesis(synthesis_messages) => {
                const STREAM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);
                streaming_happened = true;

                let conv_id_for_stream = conversation_id.clone();
                let msg_id_for_stream = assistant_message_id.clone();

                // Snapshot the final prompt in dev builds so the reasoning panel
                // can show exactly what was sent to chat_stream. Done before the
                // move so we don't have to clone `synthesis_messages`.
                #[cfg(debug_assertions)]
                {
                    let snapshot = format_messages_for_trace(&synthesis_messages);
                    final_stream_input = Some(snapshot);
                }

                // Gate the live token stream: backends that parse tool calls
                // out of the raw stream (llama.cpp) can emit tool-call syntax
                // here as plain text. The gate forwards genuine prose and
                // suppresses any tool-call markup — prose-then-tag included —
                // so it never reaches the bubble. `chat_stream`'s returned
                // content is cleaned separately with `strip_tool_call_markup`
                // before persistence.
                let gate = Arc::new(std::sync::Mutex::new(crate::ai::stream_gate::StreamGate::new()));
                let gate_for_token = gate.clone();
                let conv_for_token = conv_id_for_stream.clone();
                let msg_for_token = msg_id_for_stream.clone();
                let stream_fut = provider.chat_stream(
                    synthesis_messages,
                    Box::new(move |token| {
                        // On the (unreachable) lock-poison case, forward the raw
                        // token rather than drop content — persistence still
                        // strips markup as the final safety net.
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
                // Flush any prose the gate held back while waiting to see if a
                // trailing chunk completed a tool-call tag.
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
                            },
                        );
                    }
                }
                res
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
                assert_eq!(out.len(), 4, "all messages carried through to the stream");
                let carrier = &out[2];
                assert_eq!(carrier.role, "assistant");
                assert_eq!(
                    carrier.tool_calls.as_ref().map(|v| v.len()).unwrap_or(0),
                    1,
                    "tool_calls preserved so the tool result is not orphaned"
                );
                assert_eq!(out[3].role, "tool");
            }
            AnswerPlan::DirectText(_) => panic!("a trailing tool result is not a direct answer"),
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
        let msgs = build_prompt(&sources, &[], "when do we ship?", "en", tpl(), "");
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
            tpl(),
            "TOOLS",
        );
        let turn3_no_sources = build_prompt(&[], &history, "anything else?", "en", tpl(), "TOOLS");
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

        let msgs = build_prompt(&[], &history, "and the invoice?", "en", tpl(), "");
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
        let msgs = build_prompt(&[], &history, "next?", "en", tpl(), "");
        assert_eq!(msgs[1], ("user".to_string(), "plain question".to_string()));
    }

    #[test]
    fn prompt_trims_body_length() {
        let long_body = "x".repeat(MAX_SOURCE_BODY_CHARS * 4);
        let sources = vec![make_scored(1, "long", &long_body)];
        let msgs = build_prompt(&sources, &[], "?", "en", tpl(), "");
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
        let msgs = build_prompt(&sources, &[], "?", "en", tpl(), "");
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
        let msgs = build_prompt(&[], &history, "new question", "en", tpl(), "");
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
        let msgs = build_prompt(&[], &history, "next", "en", tpl(), "");
        assert!(msgs.iter().all(|(_, c)| c != "do not surface me"));
    }

    #[test]
    fn prompt_empty_sources_advises_model_in_final_user_message() {
        let msgs = build_prompt(&[], &[], "anything?", "en", tpl(), "");
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
        let mut msgs = build_prompt(&[], &[], "anything?", "en", tpl(), "");
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
    fn prompt_advertises_citation_contract_and_few_shots() {
        // The new prompt rewrite must surface (a) the strict citation rule,
        // (b) the valid citation range, and (c) at least one few-shot example.
        let sources = vec![
            make_scored(1, "Kickoff", "reunión el martes 3 de marzo"),
            make_scored(2, "Proposal", "monthly fee drop to $1.5k"),
        ];
        let msgs = build_prompt(&sources, &[], "¿cuándo fue el kickoff?", "es", tpl(), "");
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
        let msgs = build_prompt(&[], &[], "hola", "es", tpl(), &tools_section);
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
        let msgs = build_prompt(&[], &[], "draft a reply", "en", tpl(), &tools_section);
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
        // With drafts off the model must be told to use no tools at all.
        assert!(
            sys.contains("Do not call any tools"),
            "missing no-tools instruction: {sys}"
        );
        assert!(
            !sys.contains("generate_email_draft"),
            "draft tool leaked when disabled: {sys}"
        );
    }

    #[test]
    fn prompt_hides_lens_tools_when_lenses_disabled() {
        use crate::services::chat::tools::default_registry;
        let db = Database::new_for_testing().expect("test db");
        // Lenses default OFF — confirm the section omits them entirely so a
        // user who never enabled the feature doesn't get tool calls for it.
        let tools_section = default_registry().render_system_prompt_section(&db);
        let msgs = build_prompt(&[], &[], "show me invoices lens", "en", tpl(), &tools_section);
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
        let calls = heuristic_direct_tools(prompt).expect("today shortcut must match the button prompt");
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
        let calls = heuristic_direct_tools(prompt).expect("week shortcut must match the button prompt");
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
    fn direct_shortcut_matches_pending_button_prompt() {
        let prompt = "Identifica los emails que requieren mi respuesta o acción. \
Preséntalos en una tabla markdown …";
        let calls = heuristic_direct_tools(prompt).expect("pending shortcut must match the button prompt");
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

        let msgs = build_prompt(&[], &[], "q", "en", tpl(), "");
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

        let calls = heuristic_direct_tools("summarize today's client emails")
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
        let out = heuristic_direct_tools("resumen de hoy y de la semana");
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
