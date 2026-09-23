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
// "This answer is wrong" — the per-turn instruction that steers the retry.
pub(crate) mod correction;
// The short-circuit turn that fills an app form after a `form` planner verdict.
pub(crate) mod form_turn;
// `pub(crate)` for the query-planner eval harness, which scores it directly
// instead of inferring its quality from chat answers.
pub(crate) mod planner;
mod prewarm;
pub(crate) mod research;
pub(crate) mod retrieval;
mod routing;
// The turn's trace as one ordered step list — shared by the reasoning panel,
// the CLI and the eval report.
pub mod trace_steps;
mod turn;
// What the user has on screen, as one validated per-turn context line.
pub(crate) mod view_context;

// ── Re-exports for external callers (commands/, evals/) ──────────────────────
pub use conversations::{
    build_thread_context, create_conversation, create_conversation_with_thread, delete_conversation, get_messages,
    list_conversations, rename_conversation,
};
pub use prewarm::prewarm_chat;
pub use retrieval::{
    default_categories, normalize_categories, retrieve_context, retrieve_context_with_trace, ScoredEmail,
    DEFAULT_RAG_CATEGORIES,
};
// `smart_body_slice` / `MAX_SOURCE_BODY_CHARS` are consumed by the eval harness
// (`crate::services::chat::…`), which only compiles under the `eval` feature, so
// the re-export reads as unused on a default `--no-default-features` build.
#[allow(unused_imports)]
pub(crate) use retrieval::{smart_body_slice, MAX_SOURCE_BODY_CHARS};
pub use turn::{build_prompt, run_chat_turn, TurnContext};
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
    let invalid = bare_citation_numbers(answer)
        .into_iter()
        .filter(|&n| n == 0 || n > max_valid)
        .count();
    i32::try_from(invalid).unwrap_or(i32::MAX)
}

/// The numbers of the bare `[n]` citation markers in `answer`, in order. The
/// UI resolves each one to the n-th numbered source.
pub(crate) fn bare_citation_numbers(answer: &str) -> Vec<usize> {
    // Naive scan for [<digits>]. Regex would be overkill; this runs on every
    // turn and the answer is short.
    let bytes = answer.as_bytes();
    let mut i = 0;
    let mut numbers = Vec::new();
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
                        numbers.push(n);
                    }
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    numbers
}

/// Point each bare `[n]` at the email the answer itself says it means.
///
/// A model that found its evidence through a tool (whose results carry no
/// number) sometimes numbers those emails itself and defines each number with
/// a link: `… [1].\n\n[1](email://X)`. The UI resolves a bare `[1]` to the
/// first numbered Source (`source_ids[0]`), so the citation opened an
/// unrelated email. Where the answer defines `[n]` as an email other than
/// source n, rewrite each bare `[n]` into that link and drop the lines that
/// only held the definitions. Markers without a definition, or whose
/// definition agrees with the source, are left alone; so is a number defined
/// as two different emails.
pub(crate) fn relink_self_numbered_citations(answer: &str, source_ids: &[String]) -> String {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static DEFINITION_RE: OnceLock<regex::Regex> = OnceLock::new();
    static MARKER_RE: OnceLock<regex::Regex> = OnceLock::new();
    // Hard-coded literals that cannot fail by construction.
    #[allow(clippy::unwrap_used)]
    let definition_re = DEFINITION_RE.get_or_init(|| regex::Regex::new(r"\[(\d+)\]\(email://([^)\s]+)\)").unwrap());
    // The optional `(` tells a link label (`[n](…)`) from a bare marker; the
    // regex crate has no lookahead.
    #[allow(clippy::unwrap_used)]
    let marker_re = MARKER_RE.get_or_init(|| regex::Regex::new(r"\[(\d+)\](\()?").unwrap());

    let mut defined: HashMap<usize, Option<&str>> = HashMap::new();
    for cap in definition_re.captures_iter(answer) {
        let (Ok(n), Some(id)) = (cap[1].parse::<usize>(), cap.get(2).map(|m| m.as_str())) else {
            continue;
        };
        let entry = defined.entry(n).or_insert(Some(id));
        if *entry != Some(id) {
            *entry = None;
        }
    }
    let relink: HashMap<usize, &str> = defined
        .into_iter()
        .filter_map(|(n, id)| id.map(|id| (n, id)))
        .filter(|&(n, id)| n.checked_sub(1).and_then(|i| source_ids.get(i)).map(String::as_str) != Some(id))
        .collect();
    if relink.is_empty() {
        return answer.to_string();
    }

    let is_relinked_definition =
        |cap: &regex::Captures<'_>| cap[1].parse::<usize>().ok().and_then(|n| relink.get(&n)) == Some(&&cap[2]);
    let kept: Vec<&str> = answer
        .lines()
        .filter(|line| {
            let only_definitions = !line.trim().is_empty()
                && definition_re.captures_iter(line).all(|c| is_relinked_definition(&c))
                && definition_re.replace_all(line, "").trim().is_empty();
            !only_definitions
        })
        .collect();
    let body = kept.join("\n");
    let relinked = marker_re.replace_all(&body, |cap: &regex::Captures<'_>| {
        let whole = cap[0].to_string();
        if cap.get(2).is_some() {
            return whole;
        }
        match cap[1].parse::<usize>().ok().and_then(|n| relink.get(&n)) {
            Some(id) => format!("[{}](email://{id})", &cap[1]),
            None => whole,
        }
    });
    relinked.trim_end().to_string()
}

/// What an answer's sources are, and therefore what a bare `[n]` may mean.
///
/// Answers cite by `email://` link: numbered Sources invited Qwen 3.6 35B to
/// number the bullets of its own answer `[1]`, `[2]`… and the UI opened
/// unrelated emails (measured with the contract spelled out). The emails an
/// answer links are what it rests on, so they become its sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AnswerGrounding {
    /// Nothing linked and no tool handed back an email: the pre-retrieved
    /// Sources stand, and so do any bare `[n]` markers.
    Sources,
    /// These emails replace the pre-retrieved Sources, in this order, and
    /// every bare `[n]` is dropped.
    Emails(Vec<String>),
}

impl AnswerGrounding {
    /// How many bare `[n]` values are valid: the Source count, or none.
    pub(crate) fn citation_range(&self, source_count: usize) -> usize {
        match self {
            Self::Sources => source_count,
            Self::Emails(_) => 0,
        }
    }
}

/// Decide an answer's grounding. `source_ids` are the pre-retrieved Sources,
/// `tool_email_ids` every email the turn's tools returned in the order they
/// produced them. The emails the answer links — Sources or tool results, in
/// link order, each once — are its sources; a link outside both sets is
/// ignored. An answer that links nothing falls back to the tool emails, and
/// with none of those either the Sources stand.
pub(crate) fn plan_answer_grounding(source_ids: &[String], tool_email_ids: &[String], answer: &str) -> AnswerGrounding {
    let mut linked: Vec<String> = Vec::new();
    for id in linked_email_ids(answer) {
        if (tool_email_ids.contains(&id) || source_ids.contains(&id)) && !linked.contains(&id) {
            linked.push(id);
        }
    }
    if !linked.is_empty() {
        return AnswerGrounding::Emails(linked);
    }
    if !tool_email_ids.is_empty() {
        return AnswerGrounding::Emails(tool_email_ids.to_vec());
    }
    AnswerGrounding::Sources
}

/// The ids of the `[label](email://ID)` links in `answer`, in order, repeats
/// included.
fn linked_email_ids(answer: &str) -> Vec<String> {
    use std::sync::OnceLock;
    static LINK_RE: OnceLock<regex::Regex> = OnceLock::new();
    // Hard-coded literal that cannot fail by construction.
    #[allow(clippy::unwrap_used)]
    let link_re = LINK_RE.get_or_init(|| regex::Regex::new(r"\]\(email://([^)\s]+)\)").unwrap());
    link_re.captures_iter(answer).map(|c| c[1].to_string()).collect()
}

/// Remove `[n]` markers from `answer` where n is outside 1..=max_valid.
/// Qwen 4B in tool-results mode often invents `[1]..[9]` despite the system
/// prompt's CITATION CONTRACT — those markers are user-visible noise pointing
/// at nothing, so strip them rather than render them. Trims any whitespace
/// left adjacent (e.g. " [3]." → ".") so the answer reads naturally.
pub(crate) fn strip_invalid_citations(answer: &str, max_valid: usize) -> String {
    // A line that *begins* with an out-of-range marker is a footnote
    // definition, not prose that happens to be cited — "[1] list_calendar_events".
    // Removing only its marker strands the body as a bare line, which is how an
    // internal tool name ended up appended to an otherwise correct answer. The
    // definition of a citation that points at nothing is noise in full, so drop
    // the whole line before the marker pass runs.
    let mut dropped_footnote = false;
    let filtered: Vec<&str> = answer
        .lines()
        .filter(|line| {
            let is_footnote = is_invalid_footnote_line(line, max_valid);
            dropped_footnote |= is_footnote;
            !is_footnote
        })
        .collect();
    if dropped_footnote {
        let rejoined = filtered.join("\n");
        // Only trim when a line was actually removed, so answers that
        // legitimately end in whitespace are untouched on the common path.
        return strip_invalid_citation_markers(&rejoined, max_valid)
            .trim_end()
            .to_string();
    }
    strip_invalid_citation_markers(answer, max_valid)
}

/// Whether `line` is a footnote definition whose citation number is out of
/// range — i.e. it starts (ignoring indentation) with `[n]` where n is 0 or
/// greater than `max_valid`. `[n](…)` is excluded: that is a Markdown link
/// label, not a footnote (see [`strip_invalid_citation_markers`]).
fn is_invalid_footnote_line(line: &str, max_valid: usize) -> bool {
    let t = line.trim_start();
    let Some(rest) = t.strip_prefix('[') else {
        return false;
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return false;
    }
    let after = &rest[digits.len()..];
    let Some(after) = after.strip_prefix(']') else {
        return false;
    };
    if after.starts_with('(') {
        return false;
    }
    let n = digits.parse::<usize>().unwrap_or(0);
    n == 0 || n > max_valid
}

fn strip_invalid_citation_markers(answer: &str, max_valid: usize) -> String {
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

/// The calendar day a timestamp falls on in a zone `offset_secs` ahead of UTC.
pub(crate) fn local_date(ts: i64, offset_secs: i32) -> chrono::NaiveDate {
    Utc.timestamp_opt(ts + offset_secs as i64, 0)
        .single()
        .map(|dt| dt.date_naive())
        .unwrap_or_default()
}

/// Unix seconds of local midnight starting `date` in a zone `offset_secs`
/// ahead of UTC — the inclusive start of that local day.
pub(crate) fn local_day_start(date: chrono::NaiveDate, offset_secs: i32) -> i64 {
    date.and_time(chrono::NaiveTime::MIN).and_utc().timestamp() - offset_secs as i64
}

/// A message's date as the user sees it (their zone, not UTC).
pub(crate) fn format_date(ts: i64) -> String {
    local_date(ts, crate::services::clock::utc_offset_secs())
        .format("%Y-%m-%d")
        .to_string()
}

/// Parse an ISO-8601 date ('YYYY-MM-DD') to a unix timestamp in **seconds**
/// (local midnight on that date). Used by the `search_emails` tool to accept
/// human-friendly date bounds from the model.
pub(crate) fn parse_iso_date_secs(s: &str) -> std::result::Result<i64, String> {
    let date = chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|_| format!("expected 'YYYY-MM-DD', got '{}'", s))?;
    // Midnight in the user's zone: a `since=today` bound must not start
    // yesterday evening (or tonight) just because the machine is not on UTC.
    Ok(local_day_start(date, crate::services::clock::utc_offset_secs()))
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
    tag_filters: Option<&[crate::db::emails::search::TagQuery]>,
    limit: i32,
    unread_only: bool,
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
            tag_filters,
            limit,
            false,
            unread_only,
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

/// Leads a search result in which some row stands for a longer thread. Only a
/// hint: listing or counting questions don't need the rest of the thread.
const THREAD_SIZE_HINT: &str = "(messages=N: the row is the latest match in a thread of N messages — call get_thread(thread_id) when the answer needs the whole conversation, e.g. to summarise an exchange)\n";

pub(crate) fn format_search_emails_output(
    emails: &[Email],
    thread_sizes: &std::collections::HashMap<(String, String), i64>,
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
    // A row is one representative per thread; say how long that thread is
    // so the model doesn't summarise an exchange from its latest message.
    let size_of = |e: &Email| {
        thread_sizes
            .get(&(e.account_id.clone(), e.thread_id.clone()))
            .copied()
            .filter(|n| *n > 1)
    };
    if emails.iter().any(|e| size_of(e).is_some()) {
        out.push_str(THREAD_SIZE_HINT);
    }
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
            if let Some(n) = size_of(email) {
                out.push_str(&format!(" messages={n}"));
            }
            // Read state is data the model may be asked about; it must never
            // guess it from an email's age or from reply state.
            if !email.is_read {
                out.push_str(" unread");
            }
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
    thread_sizes: &std::collections::HashMap<(String, String), i64>,
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
    // A row is one representative per thread; say how long that thread is
    // so the model doesn't summarise an exchange from its latest message.
    let size_of = |e: &Email| {
        thread_sizes
            .get(&(e.account_id.clone(), e.thread_id.clone()))
            .copied()
            .filter(|n| *n > 1)
    };
    if emails.iter().any(|e| size_of(e).is_some()) {
        out.push_str(THREAD_SIZE_HINT);
    }
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
            if let Some(n) = size_of(email) {
                out.push_str(&format!(" messages={n}"));
            }
            // Read state is data the model may be asked about; it must never
            // guess it from an email's age or from reply state.
            if !email.is_read {
                out.push_str(" unread");
            }
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
        return Some(local_day_start(d, crate::services::clock::utc_offset_secs()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Local day boundaries ────────────────────────────────────────────
    //
    // Dates shown to and parsed from the model were UTC: at 01:30 Madrid
    // time a "today" search started yesterday, and a message received at
    // 00:30 was dated the day before. All day math goes through the clock's
    // UTC offset now.

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        chrono::NaiveDate::from_ymd_opt(y, m, d)
            .and_then(|date| date.and_hms_opt(h, min, 0))
            .map(|ndt| ndt.and_utc().timestamp())
            .expect("valid test date")
    }

    #[test]
    fn local_date_shifts_by_the_offset() {
        let late_evening = utc(2026, 4, 16, 23, 0);
        assert_eq!(local_date(late_evening, 0).to_string(), "2026-04-16");
        assert_eq!(local_date(late_evening, 7_200).to_string(), "2026-04-17");
        assert_eq!(local_date(utc(2026, 4, 17, 1, 0), -7_200).to_string(), "2026-04-16");
    }

    #[test]
    fn local_day_start_is_midnight_in_the_offset() {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 4, 17).expect("date");
        assert_eq!(local_day_start(day, 0), utc(2026, 4, 17, 0, 0));
        assert_eq!(local_day_start(day, 7_200), utc(2026, 4, 16, 22, 0));
    }

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
    fn strip_invalid_citations_drops_orphaned_footnote_lines() {
        // Regression: on a tool-results turn (max_valid = 0) the model answers
        // with an inline marker AND a footnote line defining it. Stripping only
        // the markers left the definition's body stranded as a bare line — the
        // user saw the internal tool name appended to an otherwise correct
        // answer:
        //   "…de 07:00 a 09:30.\n\n list_calendar_events"
        // A footnote defining a citation that points at nothing is noise in
        // full, so the whole line goes, not just its marker.
        let ans = "La siguiente reunión es:\n\n- Evento a las 07:00 [1].\n\n[1] list_calendar_events";
        assert_eq!(
            strip_invalid_citations(ans, 0),
            "La siguiente reunión es:\n\n- Evento a las 07:00."
        );
    }

    #[test]
    fn strip_invalid_citations_keeps_footnote_lines_for_valid_citations() {
        // The mirror case: a footnote whose marker IS in range refers to a real
        // source, so both marker and definition must survive untouched.
        let ans = "Ver el contrato [1].\n\n[1] Contrato de servicios";
        assert_eq!(strip_invalid_citations(ans, 3), ans);
    }

    // ── relink_self_numbered_citations ──────────────────────────────────
    //
    // A model that found its evidence through a tool numbers those emails
    // itself and says which email each number meant by defining it:
    // `[1](email://eml-claim)`. The UI still resolves the bare `[1]` to the
    // first pre-retrieved Source — an unrelated shipping notice.

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn relink_points_a_self_numbered_marker_at_the_email_the_answer_defines() {
        let ans = "Write to help@vendor.example [1] or care@vendor.example [2].\n\n\
                   [1](email://eml-claim)\n[2](email://eml-care)";
        assert_eq!(
            relink_self_numbered_citations(ans, &ids(&["eml-ship", "eml-review", "eml-claim"])),
            "Write to help@vendor.example [1](email://eml-claim) or care@vendor.example [2](email://eml-care)."
        );
    }

    #[test]
    fn relink_keeps_a_marker_whose_definition_matches_its_source() {
        let ans = "The kickoff was on March 3rd [1].\n\n[1](email://eml-k)";
        assert_eq!(relink_self_numbered_citations(ans, &ids(&["eml-k"])), ans);
    }

    #[test]
    fn relink_leaves_markers_the_answer_does_not_define() {
        let ans = "The kickoff was on March 3rd [1], see [the email](email://eml-k).";
        assert_eq!(relink_self_numbered_citations(ans, &ids(&["eml-ship"])), ans);
    }

    #[test]
    fn relink_covers_a_self_numbered_marker_past_the_source_range() {
        // Relinked before `strip_invalid_citations` runs, so the marker becomes
        // a link instead of being stripped as a hallucination.
        let ans = "Write to help@vendor.example [4].\n\n[4](email://eml-claim)";
        assert_eq!(
            relink_self_numbered_citations(ans, &ids(&["eml-ship"])),
            "Write to help@vendor.example [4](email://eml-claim)."
        );
    }

    #[test]
    fn relink_ignores_a_number_defined_as_two_different_emails() {
        let ans = "Write to help@vendor.example [1].\n\n[1](email://eml-a) [1](email://eml-b)";
        assert_eq!(relink_self_numbered_citations(ans, &ids(&["eml-ship"])), ans);
    }

    // ── plan_answer_grounding ───────────────────────────────────────────
    //
    // Answers cite by `email://` link. The emails an answer links are its
    // sources; with no link, the emails the tools returned stand in, and with
    // neither the pre-retrieved Sources stay as they are.

    #[test]
    fn grounding_keeps_the_sources_when_nothing_is_linked_and_no_tool_returned_an_email() {
        let plan = plan_answer_grounding(&ids(&["s1", "s2"]), &[], "March 3rd [2].");
        assert_eq!(plan, AnswerGrounding::Sources);
    }

    #[test]
    fn grounding_lists_only_the_linked_emails_in_link_order() {
        let plan = plan_answer_grounding(
            &ids(&["s1"]),
            &ids(&["t1", "t2", "t3"]),
            "See [the claim](email://t3) and [the order](email://t1).",
        );
        assert_eq!(plan, AnswerGrounding::Emails(ids(&["t3", "t1"])));
    }

    #[test]
    fn grounding_narrows_a_rag_answer_to_the_sources_it_links() {
        let plan = plan_answer_grounding(&ids(&["s1", "s2", "s3"]), &[], "See [the ticket](email://s2).");
        assert_eq!(plan, AnswerGrounding::Emails(ids(&["s2"])));
    }

    #[test]
    fn grounding_mixes_linked_sources_and_tool_emails() {
        let plan = plan_answer_grounding(
            &ids(&["s1", "s2"]),
            &ids(&["t1", "t2"]),
            "[the ticket](email://s2) and [the reply](email://t1)",
        );
        assert_eq!(plan, AnswerGrounding::Emails(ids(&["s2", "t1"])));
    }

    #[test]
    fn grounding_falls_back_to_the_tool_emails_when_nothing_is_linked() {
        let plan = plan_answer_grounding(&ids(&["s1"]), &ids(&["t1", "t2"]), "Write to help@vendor.example.");
        assert_eq!(plan, AnswerGrounding::Emails(ids(&["t1", "t2"])));
    }

    #[test]
    fn grounding_ignores_a_link_outside_the_sources_and_tool_emails() {
        let with_tools = plan_answer_grounding(&ids(&["s1"]), &ids(&["t1"]), "See [it](email://bogus).");
        assert_eq!(with_tools, AnswerGrounding::Emails(ids(&["t1"])));
        let rag_only = plan_answer_grounding(&ids(&["s1"]), &[], "See [it](email://bogus).");
        assert_eq!(rag_only, AnswerGrounding::Sources);
    }

    #[test]
    fn grounding_lists_a_repeatedly_linked_email_once() {
        let plan = plan_answer_grounding(&[], &ids(&["t1", "t2"]), "[a](email://t2) and [b](email://t2).");
        assert_eq!(plan, AnswerGrounding::Emails(ids(&["t2"])));
    }

    #[test]
    fn grounding_citation_range_is_empty_once_the_sources_are_replaced() {
        assert_eq!(AnswerGrounding::Sources.citation_range(3), 3);
        assert_eq!(AnswerGrounding::Emails(ids(&["t1"])).citation_range(3), 0);
    }

    #[test]
    fn strip_invalid_citations_line_start_rule_does_not_eat_prose() {
        // Only a marker at the START of a line is a footnote definition. An
        // invalid marker mid-sentence must still strip just the marker and
        // leave the surrounding prose intact.
        let ans = "El pago [9] se confirmó ayer.";
        assert_eq!(strip_invalid_citations(ans, 0), "El pago se confirmó ayer.");
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
