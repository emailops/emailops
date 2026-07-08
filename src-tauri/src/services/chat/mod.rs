// Chat-with-your-emails service.
//
// Responsibilities:
//   - Retrieve relevant emails for a user question (hybrid vector + FTS with RRF).
//   - Assemble a grounded prompt (system + sources + trimmed history + user turn).
//   - Drive the streaming Ollama chat and emit `chat-stream` / `chat-sources` events.
//   - Persist the assistant message and its citations.
//
// The Tauri command layer is responsible for submitting a run to `ai_queue`; this
// module exposes one high-level entry point, `run_chat_turn`.
//
// This module is the parent of the chat service. The pure pieces are split into
// submodules — `retrieval` (hybrid search + slicing), `routing` (RAG vs tools),
// `conversations` (lifecycle CRUD + titling), and `turn` (prompt assembly + tool
// loop + `run_chat_turn`). A handful of small pure helpers stay here because the
// `chat/tools/*` submodules and external callers reference them at
// `crate::services::chat::<name>`.

pub mod tools;

mod conversations;
mod planner;
mod prewarm;
mod retrieval;
mod routing;
mod turn;

// ── Re-exports for external callers (commands/, evals/) ──────────────────────
pub use conversations::{
    create_conversation, create_conversation_with_thread, delete_conversation, get_messages, list_conversations,
    rename_conversation,
};
pub use prewarm::prewarm_chat;
pub use retrieval::{retrieve_context, retrieve_context_with_trace, ScoredEmail, DEFAULT_RAG_CATEGORIES};
// `smart_body_slice` / `MAX_SOURCE_BODY_CHARS` are consumed by the eval harness
// (`crate::services::chat::…`), which only compiles under the `eval` feature, so
// the re-export reads as unused on a default `--no-default-features` build.
#[allow(unused_imports)]
pub(crate) use retrieval::{smart_body_slice, MAX_SOURCE_BODY_CHARS};
pub use turn::{build_prompt, run_chat_turn};
// Tool-call salvage parsers — used as the secondary/tertiary fallback by the
// embedded llama.cpp tool-call parsing chain (`ai/llama_cpp/runtime.rs`) after
// `parse_qwen_tool_calls` (the primary). Only the llamacpp feature consumes
// them, so the re-export is gated to keep the no-feature build quiet.
#[cfg(feature = "llamacpp")]
pub(crate) use turn::{parse_python_call_tool_calls, parse_xml_tool_calls};

use std::sync::Arc;

use chrono::{TimeZone, Utc};

use crate::db::Database;
use crate::models::{ChatPhase, ChatPhaseEvent, Email};

// ── Logging helper ──────────────────────────────────────────────────────────

pub(super) fn emit_log(level: &str, message: &str) {
    crate::services::logger::log(level, "chat", message);
}

/// Notify the UI which coarse stage the in-flight turn just entered, so the
/// chat bubble can show an LM Studio-style "Processing…" status before any
/// answer tokens stream. Fire-and-forget: a dropped phase event only costs a
/// less-specific status, never correctness, so we swallow emit errors like the
/// other one-shot chat events.
pub(super) fn emit_phase(conversation_id: &str, message_id: &str, phase: ChatPhase) {
    crate::services::events::emit(
        "chat-phase",
        ChatPhaseEvent {
            message_id: message_id.to_string(),
            conversation_id: conversation_id.to_string(),
            phase,
        },
    );
}

/// Map a tool name to the specific processing phase shown while it runs, so the
/// status reads "Searching contacts" / "Searching emails" / "Retrieving email"
/// / "Generating draft" instead of the generic "Running tools". Tools without a
/// dedicated phase fall back to [`ChatPhase::RunningTools`]. Pure so the mapping
/// is unit-testable without an `AppHandle`.
pub(super) fn phase_for_tool(tool_name: &str) -> ChatPhase {
    match tool_name {
        "search_contacts" => ChatPhase::SearchingContacts,
        "search_emails" => ChatPhase::SearchingEmails,
        "get_email_body" | "get_thread" => ChatPhase::RetrievingEmail,
        "generate_email_draft" => ChatPhase::GeneratingDraft,
        _ => ChatPhase::RunningTools,
    }
}

// ── Citation validator ─────────────────────────────────────────────────────

/// Count how many `[n]` markers in `answer` reference a source number that
/// isn't in the valid range (1..=max_valid). Used by `run_chat_turn` to
/// surface hallucinated citations in the reasoning trace.
pub(crate) fn count_invalid_citations(answer: &str, max_valid: usize) -> i32 {
    // Naive scan for [<digits>]. Regex would be overkill; this runs on every
    // turn and the answer is short.
    let bytes = answer.as_bytes();
    let mut i = 0;
    let mut invalid = 0i32;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            // Collect digits until ']'.
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 && j < bytes.len() && bytes[j] == b']' {
                // `[n](…)` is a Markdown link whose label is a number, not a
                // citation marker — skip it (matches the frontend's `(?!\()`).
                let is_link_label = j + 1 < bytes.len() && bytes[j + 1] == b'(';
                if !is_link_label {
                    if let Ok(n) = std::str::from_utf8(&bytes[i + 1..j]).unwrap_or("0").parse::<usize>() {
                        if n == 0 || n > max_valid {
                            invalid += 1;
                        }
                    }
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    invalid
}

/// Remove `[n]` markers from `answer` where n is outside 1..=max_valid.
/// Qwen 4B in tool-results mode often invents `[1]..[9]` despite the system
/// prompt's CITATION CONTRACT — those markers are user-visible noise pointing
/// at nothing, so strip them rather than render them. Trims any whitespace
/// left adjacent (e.g. " [3]." → ".") so the answer reads naturally.
pub(crate) fn strip_invalid_citations(answer: &str, max_valid: usize) -> String {
    // Build a Vec<u8> by copying bytes — '[' and ']' are single-byte ASCII so
    // we can scan/skip byte-wise without splitting multibyte UTF-8 sequences.
    let bytes = answer.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 && j < bytes.len() && bytes[j] == b']' {
                // `[n](…)` is a Markdown link whose label is a number, not a
                // citation marker — leave it intact (matches the frontend's
                // `[n](…)` negative-lookahead). Stripping the label would
                // orphan the `(email://ID)` destination into raw, unparseable
                // text.
                let is_link_label = j + 1 < bytes.len() && bytes[j + 1] == b'(';
                let n = std::str::from_utf8(&bytes[i + 1..j])
                    .unwrap_or("0")
                    .parse::<usize>()
                    .unwrap_or(0);
                if !is_link_label && (n == 0 || n > max_valid) {
                    // Drop the marker. Also collapse a single leading space so
                    // "word [9]." becomes "word." instead of "word .".
                    if out.last() == Some(&b' ') {
                        out.pop();
                    }
                    i = j + 1;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    // Safe: only ASCII '[', ']' and digit bytes were ever skipped; multibyte
    // sequences were copied verbatim.
    String::from_utf8(out).unwrap_or_else(|_| answer.to_string())
}

/// Unambiguous markers that begin a tool-call payload some small models
/// (notably Qwen 3.5 4B) emit as plain text instead of through the structured
/// tool_calls channel. Only the *tag* markers live here — bare-JSON openers
/// like `{"name"` are deliberately excluded because they appear in legitimate
/// prose ("the JSON is {\"name\": …}"), so truncating on them would eat real
/// answer text. These mirror the tag subset of `StreamGate`'s `TOOL_OPENERS`.
pub(crate) const TOOL_CALL_TAG_MARKERS: &[&str] = &["<tool_call>", "<|python_tag|>", "[TOOL_CALLS]", "<function"];

/// Deterministic safety net for the live-stream gate: truncate `content` from
/// the earliest tool-call TAG marker to the end, returning the trimmed prose
/// that precedes it. When a small model leaks `…answer.<tool_call>{…}` into the
/// final answer text, this drops the markup so the persisted/rendered bubble
/// shows only the prose. No marker → returned unchanged (cheap no-op).
pub(crate) fn strip_tool_call_markup(content: &str) -> String {
    let cut = TOOL_CALL_TAG_MARKERS
        .iter()
        .filter_map(|m| content.find(m))
        .min()
        .unwrap_or(content.len());
    content[..cut].trim_end().to_string()
}

// ── Prompt assembly ─────────────────────────────────────────────────────────

pub(crate) fn format_date(ts: i64) -> String {
    Utc.timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| ts.to_string())
}

/// Parse an ISO-8601 date ('YYYY-MM-DD') to a unix timestamp in **seconds**
/// (midnight UTC on that date). Used by the `search_emails` tool to accept
/// human-friendly date bounds from the model.
pub(crate) fn parse_iso_date_secs(s: &str) -> std::result::Result<i64, String> {
    let date = chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|_| format!("expected 'YYYY-MM-DD', got '{}'", s))?;
    let dt = date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| format!("invalid date: {}", s))?
        .and_utc();
    Ok(dt.timestamp())
}

pub(crate) fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

/// Empty-result OR-fallback for the `search_emails` tool.
///
/// Background: `db.search_emails` runs the user `query` through FTS5 with
/// implicit AND semantics — every token must appear in a single email. Small
/// LLMs frequently emit synonym blobs (`"bug error incidencia fallo"`) and
/// expect OR semantics; the AND query then matches nothing and the model
/// burns rounds retrying single words.
///
/// This helper splits `query` into tokens (≥3 chars, capped at 8 to bound
/// the work), runs one `search_emails` per token, dedupes by email id, and
/// returns the top `limit` results by timestamp. Returns `None` when the
/// query has <2 substantive tokens (no point broadening a single-word query)
/// or when the merged set is empty.
pub(crate) fn or_fallback_search(
    db: &Arc<Database>,
    account_id: &str,
    query: &str,
    cat_filter: Option<&[String]>,
    from_filter: Option<&str>,
    to_filter: Option<&str>,
    subject_filter: Option<&str>,
    limit: i32,
) -> Option<Vec<Email>> {
    let tokens: Vec<&str> = query.split_whitespace().filter(|t| t.len() >= 3).take(8).collect();
    if tokens.len() < 2 {
        return None;
    }
    let mut by_id: std::collections::HashMap<String, Email> = std::collections::HashMap::new();
    for tok in &tokens {
        if let Ok(rs) = crate::services::emails::search_emails_filtered(
            db,
            account_id,
            tok,
            cat_filter,
            from_filter,
            to_filter,
            subject_filter,
            None,
            None,
            None,
            limit,
            false,
        ) {
            for e in rs {
                by_id.entry(e.id.clone()).or_insert(e);
            }
        }
    }
    if by_id.is_empty() {
        return None;
    }
    let mut combined: Vec<Email> = by_id.into_values().collect();
    combined.sort_by_key(|e| std::cmp::Reverse(e.timestamp));
    combined.truncate(limit as usize);
    Some(combined)
}

pub(crate) fn format_search_emails_output(emails: &[Email]) -> String {
    let mut primary: Vec<&Email> = Vec::new();
    let mut updates: Vec<&Email> = Vec::new();
    let mut other: Vec<&Email> = Vec::new();
    for e in emails {
        match e.category.as_str() {
            "primary" => primary.push(e),
            "updates" => updates.push(e),
            _ => other.push(e),
        }
    }

    let mut out = String::new();
    // Token-efficiency: the `## Primary`/`## Updates` section headers already
    // convey the category, so emitting `category=` on every row is redundant
    // context bloat. Only the "Other" bucket needs the per-row field because
    // it lumps social / promotions / forums / etc. together. Snippet is also
    // clipped at 100 chars (down from 120). `thread_id` is kept so the LLM
    // can follow up with `get_thread(thread_id=...)`.
    let mut render = |header: &str, group: &[&Email], show_category: bool| {
        if group.is_empty() {
            return;
        }
        out.push_str(&format!("## {} ({})\n", header, group.len()));
        for email in group {
            let head = format!(
                "- id={} thread_id={} from=\"{} <{}>\" subject=\"{}\" date={}",
                email.id,
                email.thread_id,
                email.sender,
                email.sender_email,
                email.subject,
                format_date(email.timestamp),
            );
            out.push_str(&head);
            if show_category {
                out.push_str(&format!(" category={}", email.category));
            }
            out.push_str(&format!(" snippet=\"{}\"\n", truncate_chars(&email.snippet, 100)));
        }
    };
    render("Primary", &primary, false);
    render("Updates", &updates, false);
    render("Other", &other, true);
    out
}

/// Like [`format_search_emails_output`] but inlines each email's full cleaned
/// body (looked up in `bodies` by id) under its row instead of just a snippet.
/// Used by the summary shortcuts, which preseed `include_bodies` so a weak
/// local model can summarise complete emails in a single pass — without it the
/// model tends to chain a `get_email_body` call per result and leak the
/// tool-call markup into its answer. Emails missing from `bodies` (or with an
/// empty body) fall back to the snippet line.
pub(crate) fn format_search_emails_output_with_bodies(
    emails: &[Email],
    bodies: &std::collections::HashMap<String, String>,
) -> String {
    let mut primary: Vec<&Email> = Vec::new();
    let mut updates: Vec<&Email> = Vec::new();
    let mut other: Vec<&Email> = Vec::new();
    for e in emails {
        match e.category.as_str() {
            "primary" => primary.push(e),
            "updates" => updates.push(e),
            _ => other.push(e),
        }
    }

    let mut out = String::new();
    let mut render = |header: &str, group: &[&Email], show_category: bool| {
        if group.is_empty() {
            return;
        }
        out.push_str(&format!("## {} ({})\n", header, group.len()));
        for email in group {
            let head = format!(
                "- id={} thread_id={} from=\"{} <{}>\" subject=\"{}\" date={}",
                email.id,
                email.thread_id,
                email.sender,
                email.sender_email,
                email.subject,
                format_date(email.timestamp),
            );
            out.push_str(&head);
            if show_category {
                out.push_str(&format!(" category={}", email.category));
            }
            out.push('\n');
            match bodies.get(&email.id) {
                Some(body) if !body.trim().is_empty() => {
                    out.push_str("  body:\n");
                    out.push_str(body.trim_end());
                    out.push('\n');
                }
                // No body available — keep the snippet so the row still carries
                // some content for the model.
                _ => {
                    out.push_str(&format!("  snippet=\"{}\"\n", truncate_chars(&email.snippet, 100)));
                }
            }
        }
    };
    render("Primary", &primary, false);
    render("Updates", &updates, false);
    render("Other", &other, true);
    out
}

/// Parse `YYYY-MM-DD` or RFC-3339 into a unix timestamp. Returns None on
/// anything we don't recognise — callers treat that as "no filter".
pub(crate) fn parse_iso_date_to_ts(raw: &str) -> Option<i64> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t) {
        return Some(dt.timestamp());
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        return d.and_hms_opt(0, 0, 0).map(|ndt| ndt.and_utc().timestamp());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_for_tool_maps_known_tools_to_specific_phases() {
        // The status label is keyed off these — search/contact/retrieve/draft
        // each get a tool-specific phase instead of the generic "Running tools".
        assert_eq!(
            serde_json::to_value(phase_for_tool("search_contacts")).unwrap(),
            "searchingContacts"
        );
        assert_eq!(
            serde_json::to_value(phase_for_tool("search_emails")).unwrap(),
            "searchingEmails"
        );
        assert_eq!(
            serde_json::to_value(phase_for_tool("get_email_body")).unwrap(),
            "retrievingEmail"
        );
        assert_eq!(
            serde_json::to_value(phase_for_tool("get_thread")).unwrap(),
            "retrievingEmail"
        );
        assert_eq!(
            serde_json::to_value(phase_for_tool("generate_email_draft")).unwrap(),
            "generatingDraft"
        );
    }

    #[test]
    fn phase_for_tool_falls_back_to_running_tools_for_others() {
        // Tools without a dedicated label (memory_search, create_task,
        // get_attachments, …) keep the generic "Running tools" status.
        assert_eq!(
            serde_json::to_value(phase_for_tool("memory_search")).unwrap(),
            "runningTools"
        );
        assert_eq!(
            serde_json::to_value(phase_for_tool("get_attachments")).unwrap(),
            "runningTools"
        );
    }

    #[test]
    fn count_invalid_citations_flags_out_of_range() {
        // Sources 1..=3 exist. Answer cites [1], [3], [5], [9] — 2 invalid.
        let ans = "El kickoff fue el 3 de marzo [1]. La propuesta bajó el precio [3], \
                   y luego firmaron [5]. Ver también [9].";
        assert_eq!(count_invalid_citations(ans, 3), 2);
    }

    #[test]
    fn count_invalid_citations_all_valid_returns_zero() {
        let ans = "Alice confirmó [1]. La factura llegó el viernes [2].";
        assert_eq!(count_invalid_citations(ans, 3), 0);
    }

    #[test]
    fn count_invalid_citations_ignores_non_citation_brackets() {
        // Bracketed text with non-numeric content should not be counted.
        let ans = "See [notes] and [TODO]. Source: [1].";
        assert_eq!(count_invalid_citations(ans, 1), 0);
    }

    #[test]
    fn count_invalid_citations_ignores_email_link_labels() {
        // `[n](email://ID)` is a Markdown link whose label happens to be a
        // number — NOT a citation marker. With no numbered Sources block
        // (max_valid=0) these must not count as hallucinated citations,
        // mirroring the frontend's `[n](...)` negative-lookahead.
        let ans = "[1](email://demo_a) [2](email://demo_b) [3](email://demo_c)";
        assert_eq!(count_invalid_citations(ans, 0), 0);
    }

    #[test]
    fn strip_invalid_citations_removes_out_of_range_markers() {
        // Baseline: bare `[n]` past the valid range is stripped, and the
        // single leading space is collapsed so the prose reads naturally.
        let ans = "Firmaron el contrato [1]. Ver también [9].";
        assert_eq!(
            strip_invalid_citations(ans, 3),
            "Firmaron el contrato [1]. Ver también."
        );
    }

    #[test]
    fn strip_invalid_citations_preserves_email_link_labels() {
        // Regression: the LLM emits `[n](email://ID)` links (numbered labels)
        // when answering from tool results, where there is no Sources block
        // (max_valid=0). The label `[n]` must survive verbatim so the link
        // still parses as a clickable chip — stripping it leaves an orphaned
        // `(email://ID)` that renders as raw text.
        let ans = "[1](email://demo_f65eb4007a7d405a) [2](email://demo_5aff66a371c64252)";
        assert_eq!(strip_invalid_citations(ans, 0), ans);
    }

    #[test]
    fn strip_invalid_citations_preserves_links_but_strips_bare_markers() {
        // Mixed: a real out-of-range bare citation is stripped while a
        // numbered link label is preserved.
        let ans = "See [the renewal](email://demo_x) [9].";
        assert_eq!(strip_invalid_citations(ans, 0), "See [the renewal](email://demo_x).");
    }

    #[test]
    fn strip_tool_call_markup_removes_xml_envelope_keeps_prose() {
        let s =
            "Here is your summary.\n<tool_call><function=get_email_body>{\"email_id\":\"e1\"}</function></tool_call>";
        assert_eq!(strip_tool_call_markup(s), "Here is your summary.");
    }

    #[test]
    fn strip_tool_call_markup_no_marker_is_identity() {
        let s = "A normal answer with a [1] citation and a {\"name\": value} mention.";
        assert_eq!(strip_tool_call_markup(s), s);
    }

    #[test]
    fn strip_tool_call_markup_handles_function_marker() {
        let s = "Summary done. <function=search_emails>{}";
        assert_eq!(strip_tool_call_markup(s), "Summary done.");
    }

    #[test]
    fn strip_tool_call_markup_handles_python_tag_and_toolcalls() {
        assert_eq!(strip_tool_call_markup("Done <|python_tag|>foo()"), "Done");
        assert_eq!(strip_tool_call_markup("Done [TOOL_CALLS][{}]"), "Done");
    }

    #[test]
    fn strip_tool_call_markup_picks_earliest_marker() {
        // Two markers present — truncate at the earlier one.
        let s = "Prose <function=a>{} and later <tool_call>{}";
        assert_eq!(strip_tool_call_markup(s), "Prose");
    }

    #[test]
    fn strip_tool_call_markup_marker_only_yields_empty() {
        assert_eq!(strip_tool_call_markup("<tool_call>{}"), "");
    }
}
