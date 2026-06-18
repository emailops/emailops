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
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use serde_json::Value;

use crate::ai::ollama::ParsedSearchQuery;
use crate::models::error::{AppError, Result};
use crate::models::{Account, AppLogEvent, ChatMessageSource, ChatTrace, Draft, Email};
use crate::services::dashboard::AccountDashboard;
use crate::services::events::EventSink;
use crate::services::logger::Logger;
use crate::services::search::SearchMethod;

use super::{render, OutputMode, RenderStyle};

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

/// Event-sink backend for chat streaming. Behaviour is gated on [`RenderStyle`]
/// so the agent (`--json`) and piped (`Plain`) paths never pay for styling:
///   - **Json** / **Plain** — silent. The command owns stdout: Json prints one
///     envelope; Plain prints the final clean render once (see
///     [`render_final_answer`]).
///   - **Rich** — streams `chat-stream` tokens to stdout in a **dim** live
///     preview, accumulating the raw text. On `done` it closes the dim style and
///     clears the preview region (cursor up + clear-down), leaving the cursor at
///     the preview's start so the caller can print the aligned, colored render in
///     its place. If the preview is taller than the viewport (would have
///     scrolled), it drops to a fresh line instead of corrupting scrollback.
pub struct CliEventSink {
    style: RenderStyle,
    /// Rich-only: the streamed markdown accumulated this turn, used to size the
    /// preview-clear on `done`. `Mutex` because `emit` takes `&self` (the sink is
    /// installed behind an `Arc`).
    preview: Mutex<String>,
    /// Whether the dim SGR is currently open, so it is reset exactly once.
    dim_open: AtomicBool,
}

impl CliEventSink {
    pub fn new(style: RenderStyle) -> Self {
        Self {
            style,
            preview: Mutex::new(String::new()),
            dim_open: AtomicBool::new(false),
        }
    }

    /// Clear the dim live preview accumulated this turn, leaving the cursor where
    /// the preview began (Rich only). The caller then prints the clean render in
    /// its place. Falls back to a fresh line when the preview is taller than the
    /// viewport (cursor-up would be unreliable past a scroll).
    fn clear_preview(&self) {
        let preview = std::mem::take(&mut *self.preview.lock().unwrap_or_else(PoisonError::into_inner));
        let (width, height) = render::term_size();
        let rows = render::count_visual_rows(&preview, width);
        let mut out = std::io::stdout();
        if rows < height {
            // Move to the start of the preview block, then clear to end of screen.
            if rows > 1 {
                let _ = write!(out, "\x1b[{}F", rows - 1); // cursor up (rows-1), col 0
            } else {
                let _ = write!(out, "\r");
            }
            let _ = write!(out, "\x1b[0J"); // erase from cursor to end of screen
        } else {
            // Too tall to reposition safely — keep the preview, start fresh below.
            let _ = writeln!(out);
        }
        let _ = out.flush();
    }
}

impl EventSink for CliEventSink {
    fn emit(&self, name: &str, payload: Value) {
        // Only the Rich (interactive TTY) path streams a live preview. Json and
        // Plain stay silent so stdout is a clean envelope / final-render channel.
        if self.style != RenderStyle::Rich || name != "chat-stream" {
            return;
        }
        if let Some(token) = payload.get("token").and_then(Value::as_str) {
            // Open the dim style once; the whole preview is transient.
            if !self.dim_open.swap(true, Ordering::Relaxed) {
                print!("\x1b[2m");
            }
            print!("{token}");
            self.preview
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push_str(token);
            let _ = std::io::stdout().flush();
        }
        if payload.get("done").and_then(Value::as_bool) == Some(true) {
            if self.dim_open.swap(false, Ordering::Relaxed) {
                print!("\x1b[0m"); // reset the dim style before clearing
            }
            self.clear_preview();
        }
    }
}

/// Print a finished chat answer for the human (pretty) paths, replacing the dim
/// live preview the [`CliEventSink`] just cleared (Rich) or printing it for the
/// first time (Plain). Markdown is re-rendered through `termimad`: aligned
/// tables, styled headers/lists, internal `email://` links collapsed to labels.
/// No-op in Json mode (the command emits its own envelope).
pub fn render_final_answer(answer: &str, style: RenderStyle) {
    if style == RenderStyle::Json || answer.is_empty() {
        return;
    }
    let (width, _) = render::term_size();
    print!("{}", render::render_answer(answer, style.color(), width));
    let _ = std::io::stdout().flush();
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

pub fn render_accounts(accounts: &[Account], style: RenderStyle) -> Result<()> {
    if style == RenderStyle::Json {
        return emit_ok(accounts);
    }
    if accounts.is_empty() {
        println!("(no accounts configured)");
        return Ok(());
    }
    let color = style.color();
    for a in accounts {
        // Disabled accounts get a red ✗ marker; the provider + id are secondary.
        let flag = if a.enabled {
            " ".to_string()
        } else {
            paint("✗", "31", color)
        };
        println!(
            "{} {:<24} {} {}",
            flag,
            a.email,
            paint(&format!("{:<8}", a.provider), "2", color),
            dim(&a.id, color)
        );
    }
    Ok(())
}

/// Format a Unix-seconds timestamp as `YYYY-MM-DD HH:mm` in local time for the
/// emails list's leading date column. For the threaded `emails` view this is the
/// timestamp of the latest message shown for the thread (the thread's most
/// recent activity). Returns a fixed-width placeholder if the timestamp is out
/// of range — never expected for real rows, but keeps columns aligned.
fn format_thread_date(ts: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(ts, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        None => "????-??-?? ??:??".to_string(),
    }
}

pub fn render_emails(emails: &[Email], style: RenderStyle) -> Result<()> {
    if style == RenderStyle::Json {
        return emit_ok(emails);
    }
    if emails.is_empty() {
        println!("(no emails)");
        return Ok(());
    }
    let color = style.color();
    for e in emails {
        // Unread rows get a cyan • and a bold subject; the date and id are dim so
        // the sender/subject stay the focus.
        let read = if e.is_read {
            " ".to_string()
        } else {
            paint("•", "36", color)
        };
        let subject = truncate(&e.subject, 60);
        let subject = if e.is_read {
            format!("{subject:<60}")
        } else {
            paint(&format!("{subject:<60}"), "1", color)
        };
        println!(
            "{} {} {:<28} {} {}",
            dim(&format_thread_date(e.timestamp), color),
            read,
            truncate(&e.sender, 28),
            subject,
            dim(&e.id, color)
        );
    }
    Ok(())
}

pub fn render_email(email: &Email, body: &str, style: RenderStyle) -> Result<()> {
    if style == RenderStyle::Json {
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
    let color = style.color();
    // Header labels are dim so the values (subject, sender, …) read first.
    println!("{} {}", dim("Subject:", color), paint(&email.subject, "1", color));
    println!("{}    {} <{}>", dim("From:", color), email.sender, email.sender_email);
    if !email.recipients.is_empty() {
        println!("{}      {}", dim("To:", color), email.recipients.join(", "));
    }
    if !email.cc.is_empty() {
        println!("{}      {}", dim("Cc:", color), email.cc.join(", "));
    }
    println!(
        "{} {}   {} {}",
        dim("Mailbox:", color),
        email.mailbox,
        dim("Category:", color),
        email.category
    );
    println!("{}      {}", dim("Id:", color), dim(&email.id, color));
    println!();
    println!("{}", body_for_display(body));
    Ok(())
}

/// Indent every non-empty line of `text` by `prefix`, leaving blank lines
/// blank. Used by the thread view so each message body sits visually under its
/// header. Pure so the indentation behaviour is unit-testable.
fn indent_lines(text: &str, prefix: &str) -> String {
    text.lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render an email thread for the pretty `show` view: a heading plus one block
/// per message in chronological order, each body **indented** so the
/// conversation reads as a thread. `focus_id` is the message the user asked for
/// (marked ▶); `account_email` (when known) tags the user's own messages with
/// "(you)". JSON `show` keeps its single-email contract, so this is pretty-only.
pub fn render_thread(
    msgs: &[(Email, String)],
    focus_id: &str,
    account_email: Option<&str>,
    style: RenderStyle,
) -> Result<()> {
    let color = style.color();
    if msgs.is_empty() {
        println!("(no messages)");
        return Ok(());
    }
    let n = msgs.len();
    let count = if n > 1 {
        dim(&format!("   ({n} messages)"), color)
    } else {
        String::new()
    };
    println!(
        "{} {}{}",
        dim("Thread:", color),
        paint(&msgs[0].0.subject, "1", color),
        count
    );

    for (i, (e, body)) in msgs.iter().enumerate() {
        println!();
        let is_you = matches!(account_email, Some(acc) if acc.eq_ignore_ascii_case(&e.sender_email));
        let marker = if e.id == focus_id {
            paint("▶", "36", color)
        } else {
            dim("·", color)
        };
        let who = if is_you {
            format!("{} {}", e.sender, dim("(you)", color))
        } else {
            e.sender.clone()
        };
        println!(
            "{} {} {}  {}",
            marker,
            dim(&format!("[{}/{}]", i + 1, n), color),
            dim(&format_thread_date(e.timestamp), color),
            who,
        );
        println!("{}", indent_lines(&body_for_display(body), "    "));
    }
    Ok(())
}

/// Render dashboard-style stats — the same numbers as the app's dashboard cards
/// — one block per account: local/sent/server totals, pipeline coverage
/// (classified / embeddings / memory / tasks), and per-category counts.
pub fn render_stats(dashboards: &[AccountDashboard], style: RenderStyle) -> Result<()> {
    if style == RenderStyle::Json {
        return emit_ok(dashboards);
    }
    if dashboards.is_empty() {
        println!("(no accounts configured)");
        return Ok(());
    }
    let color = style.color();
    for (i, d) in dashboards.iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!("{} {}", dim("Account:", color), paint(&d.account.email, "1", color));
        let mut totals = format!(
            "  {} {}   {} {}",
            dim("emails:", color),
            d.synced_count,
            dim("sent:", color),
            d.sent_count
        );
        if let Some(server_total) = d.server_total {
            totals.push_str(&format!("   {} {}", dim("server total:", color), server_total));
        }
        println!("{totals}");
        println!(
            "  {} {}/{}   {} {}/{}",
            dim("classified:", color),
            d.classified_count,
            d.classified_eligible,
            dim("embeddings:", color),
            d.embedded_count,
            d.embedded_eligible
        );
        println!(
            "  {} {}/{}   {} {}/{}",
            dim("memory:", color),
            d.memory_analyzed_count,
            d.memory_eligible,
            dim("tasks:", color),
            d.task_analyzed_count,
            d.task_eligible
        );
        if !d.category_counts.is_empty() {
            println!("  {}", dim("by category:", color));
            for c in &d.category_counts {
                println!("    {:<12} {}", c.category, c.count);
            }
        }
    }
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

/// Collapse intra-line whitespace and drop blank lines entirely, so HTML that
/// wraps every line in its own block element (common in marketing email) renders
/// compact (single-spaced) instead of double-spaced. Plain-text bodies bypass
/// this path (see [`body_for_display`]), so their intentional blank lines stay.
fn normalize_lines(s: &str) -> String {
    s.split('\n')
        .map(|raw| raw.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Collapse a list of citation numbers into a compact, comma-separated string
/// with consecutive runs folded into `a-b` ranges, e.g. `[1,2,7,8,9]` →
/// `"1,2,7-9"`. Input is sorted and de-duplicated first. Pure / unit-testable.
fn collapse_citation_ranges(nums: &[i32]) -> String {
    let mut sorted: Vec<i32> = nums.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let start = sorted[i];
        let mut end = start;
        while i + 1 < sorted.len() && sorted[i + 1] == end + 1 {
            end = sorted[i + 1];
            i += 1;
        }
        // Only fold runs of 3+ into an `a-b` range; a bare pair stays "a,b"
        // (no shorter, and clearer) — so [1,2,7,8,9] → "1,2,7-9".
        if start == end {
            parts.push(start.to_string());
        } else if end == start + 1 {
            parts.push(start.to_string());
            parts.push(end.to_string());
        } else {
            parts.push(format!("{start}-{end}"));
        }
        i += 1;
    }
    parts.join(",")
}

/// Build the `sources:` lines of a chat trace, grouping sources that share the
/// same (subject, sender) onto one line prefixed with their collapsed citation
/// numbers, e.g. `[1,2,7-9] EmailOps weekly stats — EmailOps Labs`. A real
/// answer often cites several emails from the same thread/sender, so the raw
/// per-citation list repeats the same subject many times; grouping keeps the
/// block scannable. First-seen order is preserved. Pure / unit-testable.
fn format_trace_sources(sources: &[ChatMessageSource]) -> Vec<String> {
    let mut order: Vec<(String, String)> = Vec::new();
    let mut nums: std::collections::HashMap<(String, String), Vec<i32>> = std::collections::HashMap::new();
    for s in sources {
        let key = (s.subject.clone(), s.sender.clone());
        if !nums.contains_key(&key) {
            order.push(key.clone());
        }
        nums.entry(key).or_default().push(s.citation_number);
    }
    order
        .into_iter()
        .map(|(subject, sender)| {
            let cites = collapse_citation_ranges(nums.get(&(subject.clone(), sender.clone())).map_or(&[][..], |v| v));
            format!("  [{}] {} — {}", cites, truncate(&subject, 50), sender)
        })
        .collect()
}

/// Wrap `s` in an ANSI SGR `code` (e.g. `"2"` dim, `"36"` cyan, `"31"` red) when
/// `color` is set; otherwise return it unchanged so Plain/piped output stays
/// free of escape codes. The single chokepoint for all CLI coloring.
fn paint(s: &str, code: &str, color: bool) -> String {
    if color {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// Shorthand for the dim style — the trace block and secondary metadata.
fn dim(s: &str, color: bool) -> String {
    paint(s, "2", color)
}

/// Pretty-print the chat trace as a dim block on stderr (keeping stdout the
/// clean answer channel). Renders route, retrieval stats, tool calls, model
/// timings, and the (de-duplicated) retrieval sources. Called only when
/// `chat --trace` is set in pretty mode. `color` enables ANSI dim styling (Rich
/// terminals); Plain/piped callers pass `false` for escape-code-free output.
pub fn render_chat_trace(trace: Option<&ChatTrace>, sources: &[ChatMessageSource], color: bool) {
    let mut lines: Vec<String> = Vec::new();
    match trace {
        Some(t) => {
            lines.push(format!(
                "route:     {:?} ({}, {})",
                t.route.mode, t.route.classifier, t.route.reason
            ));
            if let Some(r) = &t.retrieval {
                lines.push(format!(
                    "retrieval: {} vec + {} fts → top {} ({} ms){}",
                    r.vector_hits,
                    r.fts_hits,
                    r.fused_top_k,
                    r.elapsed_ms,
                    if r.vector_fallback { ", vector fallback" } else { "" }
                ));
            }
            for tc in &t.tool_calls {
                lines.push(format!(
                    "tool:      {} ({} ms, {} chars)",
                    tc.name, tc.elapsed_ms, tc.result_chars
                ));
            }
            lines.push(format!("model:     {} ({} ms total)", t.model, t.total_elapsed_ms));
        }
        None => lines.push("(no trace recorded)".to_string()),
    }
    if !sources.is_empty() {
        lines.push("sources:".to_string());
        lines.extend(format_trace_sources(sources));
    }

    eprintln!("\n{}", dim("── trace ──────────────────────────────", color));
    for line in &lines {
        eprintln!("{}", dim(line, color));
    }
    eprintln!("{}", dim("───────────────────────────────────────", color));
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
    color: bool,
) {
    eprintln!("\n{}", dim("── search trace ───────────────────────", color));
    for line in format_search_trace_lines(method, ai_available, parsed, shown, total) {
        eprintln!("{}", dim(&line, color));
    }
    eprintln!("{}", dim("───────────────────────────────────────", color));
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
/// `show` view. Called after the streamed answer in pretty/REPL mode. `color`
/// dims the box borders on a Rich terminal; Plain/piped callers pass `false`.
pub fn render_draft(draft: &Draft, color: bool) {
    println!("\n{}", dim("── draft ──────────────────────────────", color));
    for line in format_draft_header_lines(draft) {
        println!("{line}");
    }
    println!("{}", dim("───────────────────────────────────────", color));
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
    fn collapse_citation_ranges_folds_consecutive_runs() {
        assert_eq!(collapse_citation_ranges(&[1, 2, 7, 8, 9]), "1,2,7-9");
        assert_eq!(collapse_citation_ranges(&[3]), "3");
        // Unsorted / duplicated input is normalized first.
        assert_eq!(collapse_citation_ranges(&[9, 7, 8, 2, 1, 2]), "1,2,7-9");
        assert_eq!(collapse_citation_ranges(&[]), "");
    }

    fn source(citation: i32, subject: &str, sender: &str) -> ChatMessageSource {
        ChatMessageSource {
            citation_number: citation,
            email_id: format!("eml-{citation}"),
            relevance_score: None,
            subject: subject.to_string(),
            sender: sender.to_string(),
            sender_email: String::new(),
            timestamp: 0,
            body_excerpt: None,
        }
    }

    #[test]
    fn format_trace_sources_groups_repeated_subject_sender() {
        // Five citations of the same weekly-stats thread plus two distinct ones,
        // interleaved — grouped onto one line each, in first-seen order.
        let sources = vec![
            source(1, "Weekly stats", "Metrics Bot"),
            source(2, "Weekly stats", "Metrics Bot"),
            source(3, "Onboarding question", "Nadia"),
            source(7, "Weekly stats", "Metrics Bot"),
            source(8, "Weekly stats", "Metrics Bot"),
            source(9, "Weekly stats", "Metrics Bot"),
        ];
        let lines = format_trace_sources(&sources);
        assert_eq!(lines.len(), 2, "two distinct (subject,sender) groups");
        assert_eq!(lines[0], "  [1,2,7-9] Weekly stats — Metrics Bot");
        assert_eq!(lines[1], "  [3] Onboarding question — Nadia");
    }

    #[test]
    fn dim_emits_ansi_only_when_color() {
        assert_eq!(dim("x", false), "x");
        assert_eq!(dim("x", true), "\x1b[2mx\x1b[0m");
    }

    #[test]
    fn paint_is_identity_without_color_and_wraps_with_color() {
        // The whole no-ANSI-in-Plain guarantee rests on this: every colored span
        // routes through `paint`, which is the identity when color is off.
        assert_eq!(paint("hello", "36", false), "hello");
        assert_eq!(paint("hello", "36", true), "\x1b[36mhello\x1b[0m");
    }

    #[test]
    fn indent_lines_indents_nonblank_lines_only() {
        let body = "Hi Ulises,\n\nLooking now — rolling back.\n— Marisol";
        assert_eq!(
            indent_lines(body, "    "),
            "    Hi Ulises,\n\n    Looking now — rolling back.\n    — Marisol"
        );
        // Empty input stays empty (no stray prefix).
        assert_eq!(indent_lines("", "    "), "");
    }

    #[test]
    fn format_thread_date_renders_local_yyyy_mm_dd_hh_mm() {
        use chrono::TimeZone;
        // Build the instant via Local and format via Local → tz-independent.
        let dt = chrono::Local
            .with_ymd_and_hms(2026, 6, 9, 14, 5, 6)
            .single()
            .expect("valid local time");
        assert_eq!(format_thread_date(dt.timestamp()), "2026-06-09 14:05");
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

    #[test]
    fn body_for_display_single_spaces_block_per_line_html() {
        // Marketing emails wrap each line in its own block (<div>/<p>/table
        // cells), which previously left a blank line between every line. The
        // HTML→text view should render compact: line breaks, no blank lines.
        let html = "<div>96</div><div>Hi Gero,</div><p>Net</p><p>Profit p/mo: $78</p>";
        let out = body_for_display(html);
        assert!(!out.contains("\n\n"), "no blank lines between lines: {out:?}");
        assert_eq!(out, "96\nHi Gero,\nNet\nProfit p/mo: $78");
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
