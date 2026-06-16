//! CLI output backends + renderers.
//!
//! Two seam backends:
//!   - [`CliLogger`] routes `app-log` events to **stderr** so stdout stays a
//!     clean data channel (pipeable / JSON-parseable).
//!   - [`CliEventSink`] turns the chat service's `chat-stream` token events into
//!     live **stdout** output in pretty mode (and stays silent in JSON mode, so
//!     the command can print one well-formed JSON document at the end).
//!
//! Plus the per-command pretty/JSON renderers.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use crate::ai::ollama::ParsedSearchQuery;
use crate::models::error::{AppError, Result};
use crate::models::{Account, AppLogEvent, ChatMessageSource, ChatTrace, Draft, Email};
use crate::services::events::EventSink;
use crate::services::logger::Logger;
use crate::services::search::SearchMethod;

use super::OutputMode;

/// Logger backend for the CLI: prints `HH:MM:SS.mmm [level] source: message` to
/// stderr. The wall-clock prefix makes it easy to read timings off a chat trace
/// (e.g. how long model load or a tool call took). When quiet only `error`-level
/// lines survive.
///
/// Verbosity is held in a shared [`AtomicBool`] (not a plain field) so the chat
/// paths can flip it per-turn: a chat without `--trace` suppresses the diagnostic
/// app-log stream (route / retrieval / kv / stage lines) so stdout stays the
/// clean answer channel, and re-enables it for `--trace`.
pub struct CliLogger {
    quiet: Arc<AtomicBool>,
}

impl CliLogger {
    pub fn new(quiet: Arc<AtomicBool>) -> Self {
        Self { quiet }
    }
}

/// Whether a log line at `level` should be emitted under the current quiet
/// setting. Errors always pass; everything else is suppressed when quiet. Pure
/// so the gating is unit-testable without capturing stderr.
fn should_emit(quiet: bool, level: &str) -> bool {
    !quiet || level == "error"
}

/// Format one log line with a local-time prefix. Pure so the formatting is
/// unit-testable without capturing stderr or the system clock.
fn format_log_line(now: chrono::DateTime<chrono::Local>, level: &str, source: &str, message: &str) -> String {
    format!("{} [{}] {}: {}", now.format("%H:%M:%S%.3f"), level, source, message)
}

impl Logger for CliLogger {
    fn log(&self, event: AppLogEvent) {
        if !should_emit(self.quiet.load(Ordering::Relaxed), &event.level) {
            return;
        }
        eprintln!(
            "{}",
            format_log_line(chrono::Local::now(), &event.level, &event.source, &event.message)
        );
    }
}

/// Event-sink backend for one-shot CLI runs. In pretty mode it streams
/// `chat-stream` tokens to stdout as they arrive; in JSON mode it stays silent
/// so the command owns stdout and emits a single JSON document.
pub struct CliEventSink {
    mode: OutputMode,
}

impl CliEventSink {
    pub fn new(mode: OutputMode) -> Self {
        Self { mode }
    }
}

impl EventSink for CliEventSink {
    fn emit(&self, name: &str, payload: Value) {
        if self.mode == OutputMode::Json {
            return;
        }
        if name != "chat-stream" {
            return;
        }
        if let Some(token) = payload.get("token").and_then(Value::as_str) {
            print!("{token}");
            let _ = std::io::stdout().flush();
        }
        if payload.get("done").and_then(Value::as_bool) == Some(true) {
            println!();
        }
    }
}

/// The stable success envelope: `{ ok: true, data, error: null }`. Agents parse
/// `ok` first, then read `data`; a missing/false `ok` means read `error`.
pub fn ok_envelope<T: Serialize>(data: T) -> Value {
    serde_json::json!({ "ok": true, "data": data, "error": Value::Null })
}

/// The stable failure envelope: `{ ok: false, data: null, error: {code, params, message} }`.
/// `error` is the same shape `AppError` serializes to at the Tauri boundary, so
/// agents see one error schema across CLI and app.
pub fn error_envelope(err: &AppError) -> Value {
    serde_json::json!({ "ok": false, "data": Value::Null, "error": err })
}

/// Print the success envelope wrapping `data` as pretty JSON on stdout.
pub fn emit_ok<T: Serialize>(data: T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&ok_envelope(data))?);
    Ok(())
}

/// Render an error at the process boundary. In JSON mode the failure envelope
/// goes to **stdout** (so an agent reading stdout always gets one envelope,
/// success or failure); in pretty mode a terse line goes to **stderr**.
pub fn emit_error(err: &AppError, mode: OutputMode) {
    match mode {
        OutputMode::Json => match serde_json::to_string_pretty(&error_envelope(err)) {
            Ok(s) => println!("{s}"),
            Err(_) => eprintln!("error: {err}"),
        },
        OutputMode::Pretty => eprintln!("error: {err}"),
    }
}

/// Map an `AppError` to a process exit code so shell/agent callers can branch on
/// failure *class* without parsing text. `0` is success (handled by the caller).
/// Codes are grouped by remediation: input (2), missing data (3), auth (4),
/// network/sync (5), AI (6), user-cancelled (130, the SIGINT convention),
/// everything else (1).
pub fn exit_code(err: &AppError) -> u8 {
    match err {
        AppError::InvalidInput(_) => 2,
        AppError::NotFound(_) => 3,
        AppError::AuthError(_) | AppError::OAuthError(_) | AppError::KeyringError(_) | AppError::NeedsReauth { .. } => {
            4
        }
        AppError::HttpError(_) | AppError::SyncError(_) => 5,
        AppError::AiError(_) | AppError::AiDisabled | AppError::BudgetExceeded(_) => 6,
        AppError::Cancelled => 130,
        AppError::DbError(_) | AppError::JsonError(_) | AppError::IoError(_) => 1,
    }
}

pub fn render_accounts(accounts: &[Account], mode: OutputMode) -> Result<()> {
    if mode == OutputMode::Json {
        return emit_ok(accounts);
    }
    if accounts.is_empty() {
        println!("(no accounts configured)");
        return Ok(());
    }
    for a in accounts {
        let flag = if a.enabled { " " } else { "✗" };
        println!("{} {:<24} {:<8} {}", flag, a.email, a.provider, a.id);
    }
    Ok(())
}

pub fn render_emails(emails: &[Email], mode: OutputMode) -> Result<()> {
    if mode == OutputMode::Json {
        return emit_ok(emails);
    }
    if emails.is_empty() {
        println!("(no emails)");
        return Ok(());
    }
    for e in emails {
        let read = if e.is_read { " " } else { "•" };
        let subject = truncate(&e.subject, 60);
        println!("{} {:<28} {:<60} {}", read, truncate(&e.sender, 28), subject, e.id);
    }
    Ok(())
}

pub fn render_email(email: &Email, body: &str, mode: OutputMode) -> Result<()> {
    if mode == OutputMode::Json {
        let doc = serde_json::json!({
            "id": email.id,
            "threadId": email.thread_id,
            "subject": email.subject,
            "sender": email.sender,
            "senderEmail": email.sender_email,
            "recipients": email.recipients,
            "cc": email.cc,
            "timestamp": email.timestamp,
            "isRead": email.is_read,
            "mailbox": email.mailbox,
            "category": email.category,
            "body": body,
        });
        return emit_ok(doc);
    }
    println!("Subject: {}", email.subject);
    println!("From:    {} <{}>", email.sender, email.sender_email);
    if !email.recipients.is_empty() {
        println!("To:      {}", email.recipients.join(", "));
    }
    if !email.cc.is_empty() {
        println!("Cc:      {}", email.cc.join(", "));
    }
    println!("Mailbox: {}   Category: {}", email.mailbox, email.category);
    println!("Id:      {}", email.id);
    println!();
    println!("{}", body_for_display(body));
    Ok(())
}

/// Render an email body for terminal display. HTML bodies are converted to
/// readable plain text (style/script dropped, block elements become line
/// breaks, entities decoded); plain-text bodies pass through untouched. JSON
/// output keeps the raw body — this is only for the pretty `show` view.
fn body_for_display(body: &str) -> String {
    if looks_like_html(body) {
        html_to_text(body)
    } else {
        body.to_string()
    }
}

fn looks_like_html(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    ["</", "<br", "<p>", "<p ", "<div", "<html", "<body", "<table", "<span"]
        .iter()
        .any(|needle| lower.contains(needle))
}

/// Block-level / line-break tags that should produce a newline in the rendered
/// text. Everything else (inline tags) collapses to a single space.
const BLOCK_TAGS: &[&str] = &[
    "p",
    "br",
    "div",
    "tr",
    "li",
    "ul",
    "ol",
    "table",
    "blockquote",
    "hr",
    "body",
    "head",
    "section",
    "article",
    "header",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
];

fn tag_is_block(tag_body: &str) -> bool {
    let name: String = tag_body
        .trim_start_matches('/')
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    BLOCK_TAGS.contains(&name.as_str())
}

fn starts_with_ci(bytes: &[u8], i: usize, pat: &[u8]) -> bool {
    i + pat.len() <= bytes.len() && bytes[i..i + pat.len()].eq_ignore_ascii_case(pat)
}

fn html_to_text(html: &str) -> String {
    let bytes = html.as_bytes();
    let mut out = String::with_capacity(html.len() / 2);
    let mut i = 0;
    let mut skip_until: Option<&'static [u8]> = None;

    while i < bytes.len() {
        if let Some(close) = skip_until {
            if starts_with_ci(bytes, i, close) {
                i += close.len();
                skip_until = None;
            } else {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'<' {
            if starts_with_ci(bytes, i, b"<style") {
                skip_until = Some(b"</style>");
                i += 1;
                continue;
            }
            if starts_with_ci(bytes, i, b"<script") {
                skip_until = Some(b"</script>");
                i += 1;
                continue;
            }
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'>' {
                j += 1;
            }
            let tag_body = &html[i + 1..j.min(bytes.len())];
            out.push(if tag_is_block(tag_body) { '\n' } else { ' ' });
            i = (j + 1).min(bytes.len());
            continue;
        }
        // Copy one UTF-8 char (we are at a char boundary on valid &str input).
        let b = bytes[i];
        let ch_len = if b < 0x80 {
            1
        } else if b < 0xE0 {
            2
        } else if b < 0xF0 {
            3
        } else {
            4
        };
        let end = (i + ch_len).min(bytes.len());
        out.push_str(&html[i..end]);
        i = end;
    }

    normalize_lines(&crate::util::html::decode_html_entities(&out))
}

/// Collapse intra-line whitespace, drop runs of blank lines down to one, and
/// trim leading/trailing blank lines.
fn normalize_lines(s: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut prev_blank = true; // skip leading blanks
    for raw in s.split('\n') {
        let line = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        let blank = line.is_empty();
        if blank && prev_blank {
            continue;
        }
        out.push(line);
        prev_blank = blank;
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// Pretty-print the chat trace as a dim block on stderr (keeping stdout the
/// clean answer channel). Renders route, retrieval stats, tool calls, model
/// timings, and the retrieval sources. Called only when `chat --trace` is set
/// in pretty mode.
pub fn render_chat_trace(trace: Option<&ChatTrace>, sources: &[ChatMessageSource]) {
    eprintln!("\n── trace ──────────────────────────────");
    match trace {
        Some(t) => {
            eprintln!(
                "route:     {:?} ({}, {})",
                t.route.mode, t.route.classifier, t.route.reason
            );
            if let Some(r) = &t.retrieval {
                eprintln!(
                    "retrieval: {} vec + {} fts → top {} ({} ms){}",
                    r.vector_hits,
                    r.fts_hits,
                    r.fused_top_k,
                    r.elapsed_ms,
                    if r.vector_fallback { ", vector fallback" } else { "" }
                );
            }
            for tc in &t.tool_calls {
                eprintln!(
                    "tool:      {} ({} ms, {} chars)",
                    tc.name, tc.elapsed_ms, tc.result_chars
                );
            }
            eprintln!("model:     {} ({} ms total)", t.model, t.total_elapsed_ms);
        }
        None => eprintln!("(no trace recorded)"),
    }
    if !sources.is_empty() {
        eprintln!("sources:");
        for s in sources {
            eprintln!("  [{}] {} — {}", s.citation_number, truncate(&s.subject, 50), s.sender);
        }
    }
    eprintln!("───────────────────────────────────────");
}

/// Build the body lines of a `search --trace` block. Pure (no I/O) so the line
/// formatting is unit-testable; [`render_search_trace`] wraps these with the
/// header/footer and prints them to stderr.
fn format_search_trace_lines(
    method: &SearchMethod,
    ai_available: bool,
    parsed: Option<&ParsedSearchQuery>,
    shown: usize,
    total: usize,
) -> Vec<String> {
    let mut lines = vec![
        format!("method:    {:?}", method),
        format!("ai:        {}", if ai_available { "available" } else { "unavailable" }),
    ];

    let mut filters: Vec<String> = Vec::new();
    if let Some(p) = parsed {
        if let Some(f) = &p.from_filter {
            filters.push(format!("from={f}"));
        }
        if let Some(f) = &p.to_filter {
            filters.push(format!("to={f}"));
        }
        if let Some(f) = &p.subject_filter {
            filters.push(format!("subject={f}"));
        }
        if !p.keywords.is_empty() {
            filters.push(format!("keywords=[{}]", p.keywords.join(", ")));
        }
        if p.is_unread == Some(true) {
            filters.push("unread".to_string());
        }
        if p.has_attachment == Some(true) {
            filters.push("has_attachment".to_string());
        }
        if let Some(t) = p.after_timestamp {
            filters.push(format!("after={t}"));
        }
        if let Some(t) = p.before_timestamp {
            filters.push(format!("before={t}"));
        }
        if !p.tag_filters.is_empty() {
            filters.push(format!("tags=[{}]", p.tag_filters.join(", ")));
        }
    }
    lines.push(format!(
        "filters:   {}",
        if filters.is_empty() {
            "(none)".to_string()
        } else {
            filters.join(" ")
        }
    ));
    lines.push(format!("results:   {shown} shown / {total} hits"));
    lines
}

/// Print a `search --trace` block to stderr (so stdout stays a clean data
/// channel), styled to match [`render_chat_trace`].
pub fn render_search_trace(
    method: &SearchMethod,
    ai_available: bool,
    parsed: Option<&ParsedSearchQuery>,
    shown: usize,
    total: usize,
) {
    eprintln!("\n── search trace ───────────────────────");
    for line in format_search_trace_lines(method, ai_available, parsed, shown, total) {
        eprintln!("{line}");
    }
    eprintln!("───────────────────────────────────────");
}

/// Build the header lines (To / Subject / Id) of a draft block. Pure (no I/O) so
/// the formatting is unit-testable; [`render_draft`] wraps these with the body.
fn format_draft_header_lines(draft: &Draft) -> Vec<String> {
    let to = if draft.to_addresses.is_empty() {
        "(none)".to_string()
    } else {
        draft.to_addresses.join(", ")
    };
    vec![
        format!("To:      {to}"),
        format!("Subject: {}", draft.subject),
        format!("Id:      {}", draft.id),
    ]
}

/// Print a draft the chat assistant just created as a block on **stdout** (it's
/// answer content, not a diagnostic). The body is HTML→text rendered like the
/// `show` view. Called after the streamed answer in pretty/REPL mode.
pub fn render_draft(draft: &Draft) {
    println!("\n── draft ──────────────────────────────");
    for line in format_draft_header_lines(draft) {
        println!("{line}");
    }
    println!("───────────────────────────────────────");
    println!("{}", body_for_display(&draft.body));
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_leaves_short_strings_untouched() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn should_emit_passes_everything_when_not_quiet() {
        assert!(should_emit(false, "info"));
        assert!(should_emit(false, "debug"));
        assert!(should_emit(false, "error"));
    }

    #[test]
    fn should_emit_suppresses_non_errors_when_quiet() {
        assert!(!should_emit(true, "info"));
        assert!(!should_emit(true, "debug"));
        assert!(!should_emit(true, "success"));
        // Errors always survive so failures never go silent.
        assert!(should_emit(true, "error"));
    }

    #[test]
    fn format_log_line_prefixes_local_timestamp() {
        use chrono::TimeZone;
        let ts = chrono::Local
            .with_ymd_and_hms(2026, 6, 9, 14, 5, 6)
            .single()
            .expect("valid local time");
        let line = format_log_line(ts, "info", "chat", "stage: route");
        assert_eq!(line, "14:05:06.000 [info] chat: stage: route");
    }

    #[test]
    fn body_for_display_passes_plain_text_through() {
        let text = "Hi Gero,\n\nSee you tomorrow.\n— Ana";
        assert_eq!(body_for_display(text), text);
    }

    #[test]
    fn body_for_display_strips_html_to_readable_text() {
        let html = "<html><body><p>Buenas tardes</p><p>Gracias</p></body></html>";
        let out = body_for_display(html);
        assert!(!out.contains('<'), "no tags remain: {out:?}");
        assert!(out.contains("Buenas tardes"));
        assert!(out.contains("Gracias"));
    }

    fn parsed_with_keywords(keywords: &[&str]) -> ParsedSearchQuery {
        ParsedSearchQuery {
            keywords: keywords.iter().map(|s| s.to_string()).collect(),
            from_filter: None,
            to_filter: None,
            subject_filter: None,
            has_attachment: None,
            is_unread: None,
            after_timestamp: None,
            before_timestamp: None,
            tag_filters: Vec::new(),
        }
    }

    #[test]
    fn search_trace_reports_method_ai_and_counts() {
        let lines = format_search_trace_lines(&SearchMethod::KeywordSearch, false, None, 5, 12);
        assert_eq!(lines[0], "method:    KeywordSearch");
        assert_eq!(lines[1], "ai:        unavailable");
        assert_eq!(lines[2], "filters:   (none)");
        assert_eq!(lines[3], "results:   5 shown / 12 hits");
    }

    #[test]
    fn search_trace_renders_parsed_filters() {
        let mut parsed = parsed_with_keywords(&["invoice", "march"]);
        parsed.from_filter = Some("ana@acme.com".to_string());
        parsed.is_unread = Some(true);
        let lines = format_search_trace_lines(&SearchMethod::PatternParsed, true, Some(&parsed), 3, 3);
        assert_eq!(lines[1], "ai:        available");
        assert_eq!(
            lines[2],
            "filters:   from=ana@acme.com keywords=[invoice, march] unread"
        );
    }

    #[test]
    fn body_for_display_drops_style_and_script_blocks() {
        let html = "<style>p{color:red}</style><script>var x=1;</script><p>Hola</p>";
        let out = body_for_display(html);
        assert!(out.contains("Hola"));
        assert!(!out.contains("color"), "style block leaked: {out:?}");
        assert!(!out.contains("var x"), "script block leaked: {out:?}");
    }

    #[test]
    fn body_for_display_decodes_entities_and_keeps_paragraph_breaks() {
        let html = "<p>a &amp; b</p><p>c</p>";
        let out = body_for_display(html);
        assert!(out.contains("a & b"), "entities decoded: {out:?}");
        assert!(out.contains("c"));
        assert!(!out.starts_with('\n') && !out.ends_with('\n'), "trimmed: {out:?}");
    }

    fn sample_draft(to: Vec<&str>) -> Draft {
        Draft {
            id: "a8e6d45a".to_string(),
            email_id: None,
            account_id: "acct-1".to_string(),
            to_addresses: to.into_iter().map(String::from).collect(),
            subject: "Confirmar reunión Alina".to_string(),
            body: "Hola Alina".to_string(),
            ai_generated: true,
            status: "draft".to_string(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn draft_header_reports_to_subject_and_id() {
        let lines = format_draft_header_lines(&sample_draft(vec!["alina@example.com"]));
        assert_eq!(lines[0], "To:      alina@example.com");
        assert_eq!(lines[1], "Subject: Confirmar reunión Alina");
        assert_eq!(lines[2], "Id:      a8e6d45a");
    }

    #[test]
    fn draft_header_shows_none_when_no_recipients() {
        let lines = format_draft_header_lines(&sample_draft(vec![]));
        assert_eq!(lines[0], "To:      (none)");
    }

    #[test]
    fn truncate_adds_ellipsis_when_too_long() {
        let out = truncate("abcdefghij", 5);
        assert_eq!(out.chars().count(), 5);
        assert!(out.ends_with('…'));
    }

    #[test]
    fn ok_envelope_wraps_data_with_null_error() {
        let v = ok_envelope(serde_json::json!({ "n": 3 }));
        assert_eq!(v["ok"], true);
        assert_eq!(v["data"]["n"], 3);
        assert!(v["error"].is_null());
    }

    #[test]
    fn error_envelope_carries_appror_shape() {
        let v = error_envelope(&AppError::NotFound("email x".into()));
        assert_eq!(v["ok"], false);
        assert!(v["data"].is_null());
        assert_eq!(v["error"]["code"], "not_found");
        assert!(v["error"]["message"].as_str().unwrap().contains("email x"));
    }

    #[test]
    fn exit_code_groups_errors_by_remediation() {
        // input
        assert_eq!(exit_code(&AppError::InvalidInput("x".into())), 2);
        // not found
        assert_eq!(exit_code(&AppError::NotFound("x".into())), 3);
        // auth
        assert_eq!(exit_code(&AppError::AuthError("x".into())), 4);
        assert_eq!(exit_code(&AppError::OAuthError("x".into())), 4);
        assert_eq!(exit_code(&AppError::KeyringError("x".into())), 4);
        assert_eq!(exit_code(&AppError::NeedsReauth { account_id: "a".into() }), 4);
        // network / sync
        assert_eq!(exit_code(&AppError::SyncError("x".into())), 5);
        // AI
        assert_eq!(exit_code(&AppError::AiError("x".into())), 6);
        assert_eq!(exit_code(&AppError::AiDisabled), 6);
        assert_eq!(exit_code(&AppError::BudgetExceeded("x".into())), 6);
        // cancelled
        assert_eq!(exit_code(&AppError::Cancelled), 130);
        // catch-all
        assert_eq!(exit_code(&AppError::IoError("x".into())), 1);
    }

    #[test]
    fn ok_envelope_preserves_nested_structure() {
        let v = ok_envelope(serde_json::json!({ "items": [1, 2], "meta": { "total": 2 } }));
        assert_eq!(v["data"]["items"][1], 2);
        assert_eq!(v["data"]["meta"]["total"], 2);
        assert!(v["error"].is_null());
    }

    #[test]
    fn error_envelope_always_nulls_data() {
        let v = error_envelope(&AppError::AiDisabled);
        assert!(v["data"].is_null());
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"]["code"], "ai_disabled");
        // parameterless variant still carries an (empty) params object
        assert!(v["error"]["params"].is_object());
    }
}
