//! The one way an email thread is read for a model.
//!
//! A reply usually carries the whole conversation before it as quoted text.
//! Reading each message whole therefore re-reads every earlier message once
//! per reply: a 7-message thread costs ~28 message-reads, and a model that
//! extracts facts from each reply extracts the same facts again and again.
//! This module reads a thread once, as a unit:
//!
//! - each message is reduced to its **new content**: a quoted block or a
//!   signature goes only when an earlier message of the thread already
//!   contains it, and an own paragraph only when it repeats one
//!   (`thread_clean::new_text`) — a forward, or a quote of mail that was never
//!   synced, stays;
//! - one **total** character budget is shared across the messages
//!   ([`allocate_budget`]): short messages whole, long ones a fair share, the
//!   oldest dropped when a share would be too small to be useful, and an
//!   optional **focus** message (the one a draft answers) served first;
//! - [`render_thread`] is the format the chat shows a thread in.
//!
//! Every feature that shows a thread — or one message of a thread — to a
//! model goes through here: the chat's thread context and `get_thread`,
//! reply drafts, research, and the per-email extractors (memory, tasks,
//! lenses) via [`message_new_content`]. Pure core, thin loaders at the end.

use crate::db::Database;
use crate::models::Email;
use crate::services::thread_clean::{clean_email_body, new_text, normalize_body, History};

/// One message of a thread as stored: raw body, before any cleaning.
#[derive(Debug, Clone, PartialEq)]
pub struct ThreadMessage {
    pub id: String,
    pub sender: String,
    pub sender_email: String,
    pub timestamp: i64,
    pub subject: String,
    /// Raw body (HTML or text).
    pub body: String,
    /// List preview, used when the body is missing.
    pub snippet: String,
}

impl ThreadMessage {
    pub fn from_email(email: &Email, body: String) -> Self {
        Self {
            id: email.id.clone(),
            sender: email.sender.clone(),
            sender_email: email.sender_email.clone(),
            timestamp: email.timestamp,
            subject: email.subject.clone(),
            body,
            snippet: email.snippet.clone(),
        }
    }
}

/// One message as the model reads it: its new content, cut to its share.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadMessage {
    /// 1-based position in the full thread (omitted messages included).
    pub position: usize,
    pub id: String,
    pub sender: String,
    pub sender_email: String,
    pub timestamp: i64,
    pub subject: String,
    pub text: String,
}

/// A thread read for a model: the kept messages, oldest first, and how many
/// of the oldest were dropped to fit the budget.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ThreadRead {
    pub messages: Vec<ReadMessage>,
    pub omitted: usize,
}

/// How much of a thread to read.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadOptions {
    /// Characters of message text for the whole thread.
    pub budget_chars: usize,
    /// Below this share a message is dropped rather than cut to nothing.
    pub min_chars_per_message: usize,
    /// A message served first (up to `focus_max_chars`): the one a reply answers.
    pub focus_id: Option<String>,
    pub focus_max_chars: usize,
}

impl ReadOptions {
    /// A budget shared evenly, no focus.
    pub fn budget(budget_chars: usize) -> Self {
        Self {
            budget_chars,
            min_chars_per_message: MIN_CHARS_PER_MESSAGE,
            focus_id: None,
            focus_max_chars: 0,
        }
    }
}

/// Default share floor: a few sentences.
pub const MIN_CHARS_PER_MESSAGE: usize = 300;
/// The chat's thread budget (open email, conversation about a thread,
/// `get_thread`).
pub const CHAT_THREAD_BUDGET: usize = 16_000;
/// Each message's new content: what it adds to the thread. Pure.
///
/// Every message is compared with the ones before it ([`History`] holds their
/// whole bodies, quotes included): quoted text, signatures and paragraphs the
/// thread already has are dropped, anything else — a forward, a quote of mail
/// that was never synced, a first signature — stays. A message with no body
/// falls back to its list preview.
pub fn new_content(messages: &[ThreadMessage]) -> Vec<String> {
    let mut history = History::default();
    let mut out = Vec::with_capacity(messages.len());
    for m in messages {
        let normalized = normalize_body(&m.body);
        if normalized.trim().is_empty() {
            out.push(m.snippet.trim().to_string());
            continue;
        }
        out.push(new_text(&normalized, &history));
        history.add(&normalized);
    }
    out
}

/// Share `budget` chars across message lengths (`lens`, oldest first). Pure.
///
/// The focus message, when there is one, is served first (up to
/// `focus_max_chars`). The others then split what is left: the newest ones
/// that still get a useful share (`min_chars_per_message`, or their whole
/// length when shorter) are kept, the older ones get 0, and the kept ones are
/// water-filled — short messages whole, long ones an equal share of the rest.
pub fn allocate_budget(lens: &[usize], opts: &ReadOptions, focus: Option<usize>) -> Vec<usize> {
    let mut alloc = vec![0; lens.len()];
    let mut remaining = opts.budget_chars;
    if let Some(f) = focus.filter(|f| *f < lens.len()) {
        alloc[f] = lens[f].min(opts.focus_max_chars).min(remaining);
        remaining -= alloc[f];
    }
    let others: Vec<usize> = (0..lens.len()).filter(|i| Some(*i) != focus).collect();
    // Keep the newest messages that still leave each a useful share.
    let mut first_kept = others.len();
    while first_kept > 0 {
        let n = others.len() - first_kept + 1;
        let useful = opts.min_chars_per_message.min(lens[others[first_kept - 1]]);
        if remaining / n < useful {
            break;
        }
        first_kept -= 1;
    }
    let mut kept: Vec<usize> = others[first_kept..].to_vec();
    kept.sort_by_key(|&i| lens[i]);
    let mut slots = kept.len();
    for i in kept {
        let take = lens[i].min(remaining / slots);
        alloc[i] = take;
        remaining -= take;
        slots -= 1;
    }
    alloc
}

/// Char-aware cut with a "…" marker so the model knows the text continues.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// Read a thread: new content per message, the budget applied. Pure.
///
/// A message whose new content is empty (a reply that only quoted) is kept
/// with empty text — who answered, and when, is still part of the thread.
pub fn read_thread(messages: &[ThreadMessage], opts: &ReadOptions) -> ThreadRead {
    let texts = new_content(messages);
    let lens: Vec<usize> = texts.iter().map(|t| t.chars().count()).collect();
    let focus = opts
        .focus_id
        .as_ref()
        .and_then(|id| messages.iter().position(|m| &m.id == id));
    let alloc = allocate_budget(&lens, opts, focus);
    let mut read = ThreadRead::default();
    for (i, ((m, text), share)) in messages.iter().zip(&texts).zip(&alloc).enumerate() {
        if *share == 0 && lens[i] > 0 {
            read.omitted += 1;
            continue;
        }
        read.messages.push(ReadMessage {
            position: i + 1,
            id: m.id.clone(),
            sender: m.sender.clone(),
            sender_email: m.sender_email.clone(),
            timestamp: m.timestamp,
            subject: m.subject.clone(),
            text: truncate_chars(text, *share),
        });
    }
    read
}

fn format_date(unix_secs: i64) -> String {
    use chrono::TimeZone;
    chrono::Utc
        .timestamp_opt(unix_secs, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// The chat's thread format: a header, then each message with its id (so the
/// model can cite or reply to it), sender, date and new content. Pure.
pub fn render_thread(read: &ThreadRead) -> String {
    let Some(first) = read.messages.first() else {
        return String::new();
    };
    let total = read.messages.len() + read.omitted;
    let mut out = format!(
        "EMAIL THREAD CONTEXT\nSubject: {}\nMessages: {total}\n---\n\n",
        first.subject
    );
    if read.omitted > 0 {
        out.push_str(&format!("[{} earlier messages omitted]\n\n", read.omitted));
    }
    for m in &read.messages {
        let body = if m.text.is_empty() { "(no new content)" } else { &m.text };
        out.push_str(&format!(
            "[{n}] (id: {id}) From: {sender} <{addr}>\n    Date: {date}\n    Subject: {subj}\n\n{body}\n\n",
            n = m.position,
            id = m.id,
            sender = m.sender,
            addr = m.sender_email,
            date = format_date(m.timestamp),
            subj = m.subject,
        ));
    }
    out
}

// ── Loaders ─────────────────────────────────────────────────────────────────

/// A thread's messages, oldest first, with their raw bodies.
pub fn load_thread(
    db: &Database,
    account_id: &str,
    thread_id: &str,
) -> crate::models::error::Result<Vec<ThreadMessage>> {
    let emails = db.get_thread(account_id, thread_id)?;
    Ok(emails
        .iter()
        .map(|e| ThreadMessage::from_email(e, db.get_email_body(&e.id).unwrap_or_default()))
        .collect())
}

/// One message's new content, read in the context of its thread: what an
/// extractor should read instead of a reply's full body with its quoted
/// history. Falls back to the message's own cleaned body when the thread
/// cannot be loaded.
pub fn message_new_content(db: &Database, email: &Email, raw_body: &str) -> String {
    let thread = match db.get_thread(&email.account_id, &email.thread_id) {
        Ok(t) if t.len() > 1 => t,
        _ => return clean_email_body(raw_body, usize::MAX),
    };
    // Earlier messages (and this one) only: later replies are not history.
    let mut messages: Vec<ThreadMessage> = Vec::new();
    for e in &thread {
        if e.id == email.id {
            messages.push(ThreadMessage::from_email(email, raw_body.to_string()));
            break;
        }
        messages.push(ThreadMessage::from_email(
            e,
            db.get_email_body(&e.id).unwrap_or_default(),
        ));
    }
    if messages.last().map(|m| m.id != email.id).unwrap_or(true) {
        return clean_email_body(raw_body, usize::MAX);
    }
    new_content(&messages).pop().unwrap_or_default()
}

/// Test fixture shared by the extractors' tests: account `acct`, thread `t1`
/// with a request (`e1`) and a reply (`e2`) that quotes it.
#[cfg(test)]
pub(crate) mod fixtures {
    use crate::db::Database;

    pub const REQUEST: &str = "The first message asks for a budget for the customer portal before Friday.";
    pub const REPLY_NEW: &str = "We accept the budget of 4,000 EUR and start on Monday.";

    pub fn seed_quoting_thread(db: &Database) {
        use rusqlite::params;
        let conn = db.connection();
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
             VALUES ('acct', 'gmail', 'me@example.com', 'Me', 0)",
            [],
        )
        .unwrap();
        let reply = format!("{REPLY_NEW}\n\nOn Mon, 1 Jun 2017, Ana wrote:\n> {REQUEST}");
        for (id, ts, body) in [("e1", 100, REQUEST.to_string()), ("e2", 200, reply)] {
            conn.execute(
                "INSERT INTO emails
                 (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                  recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
                 VALUES (?1,'acct','t1','Portal budget','Ana','ana@example.com','example.com',
                         '[]','[]','snip',?2,0,'primary',0)",
                params![id, ts],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO email_bodies(email_id, body) VALUES (?1, ?2)",
                params![id, body],
            )
            .unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_messages_new_content_in_its_thread_leaves_out_what_it_quotes() {
        let db = Database::new_for_testing().unwrap();
        fixtures::seed_quoting_thread(&db);
        let email = db.get_email_by_id("e2").unwrap().expect("e2");
        let raw = db.get_email_body("e2").unwrap();
        assert_eq!(message_new_content(&db, &email, &raw), fixtures::REPLY_NEW);
    }

    fn msg(id: &str, ts: i64, body: &str) -> ThreadMessage {
        ThreadMessage {
            id: id.into(),
            sender: format!("Sender {id}"),
            sender_email: format!("{id}@example.com"),
            timestamp: ts,
            subject: "Budget for the portal".into(),
            body: body.into(),
            snippet: format!("snippet {id}"),
        }
    }

    const ASK: &str = "Could you send me a budget for the customer portal mock-up? We need it before Friday.";
    const ANSWER: &str = "Sure, the budget for the portal mock-up is 4,000 EUR, delivery in three weeks.";

    // ── new content ──

    #[test]
    fn a_reply_keeps_only_what_it_adds_when_the_quote_is_marked() {
        let thread = vec![
            msg("m1", 1, ASK),
            msg("m2", 2, &format!("{ANSWER}\n\nOn Mon, 1 Jun 2017, Ana wrote:\n> {ASK}")),
        ];
        let new = new_content(&thread);
        assert_eq!(new[0], ASK);
        assert_eq!(new[1], ANSWER);
    }

    #[test]
    fn a_repeated_paragraph_without_any_quote_marker_is_dropped() {
        // Clients that forward or paste the previous message without an
        // attribution line: the marker-based cut sees nothing to cut.
        let thread = vec![msg("m1", 1, ASK), msg("m2", 2, &format!("{ANSWER}\n\n{ASK}"))];
        assert_eq!(new_content(&thread)[1], ANSWER);
    }

    #[test]
    fn interleaved_quote_lines_go_only_when_the_thread_has_them() {
        // `ASK` is m1's text; "Anything else?" quotes something this thread
        // never had, so it is the only copy and stays.
        let body = format!("> {ASK}\nThe budget is 4,000 EUR.\n> Anything else?\nNo, that is all.");
        let new = new_content(&[msg("m1", 1, ASK), msg("m2", 2, &body)]);
        assert_eq!(new[1], "The budget is 4,000 EUR.\n> Anything else?\nNo, that is all.");
    }

    #[test]
    fn a_one_message_thread_keeps_a_forwarded_message() {
        let body = "FYI\n\n---------- Forwarded message ---------\nFrom: Ana <ana@example.com>\nSubject: Budget\n\nThe budget is 4,000 EUR.";
        assert!(new_content(&[msg("m1", 1, body)])[0].contains("The budget is 4,000 EUR."));
    }

    #[test]
    fn short_lines_that_repeat_are_kept() {
        let thread = vec![
            msg("m1", 1, &format!("{ASK}\n\nThanks!")),
            msg("m2", 2, &format!("{ANSWER}\n\nThanks!")),
        ];
        assert!(new_content(&thread)[1].ends_with("Thanks!"));
    }

    #[test]
    fn an_empty_body_falls_back_to_the_preview() {
        assert_eq!(new_content(&[msg("m1", 1, "")])[0], "snippet m1");
    }

    // ── budget ──

    #[test]
    fn short_messages_are_kept_whole_and_long_ones_share_the_rest() {
        assert_eq!(
            allocate_budget(&[100, 5000, 5000], &ReadOptions::budget(2100), None),
            vec![100, 1000, 1000]
        );
    }

    #[test]
    fn the_oldest_are_dropped_when_a_share_would_be_too_small() {
        // 6 long messages, 1000 chars: 166 each is below the 300 floor, so
        // only the newest three are read (333 each).
        let alloc = allocate_budget(&[5000; 6], &ReadOptions::budget(1000), None);
        assert_eq!(alloc, vec![0, 0, 0, 333, 333, 334]);
    }

    #[test]
    fn a_focus_message_is_served_first() {
        let opts = ReadOptions {
            focus_max_chars: 6000,
            ..ReadOptions::budget(7000)
        };
        let alloc = allocate_budget(&[5000, 5000, 8000], &opts, Some(2));
        assert_eq!(alloc[2], 6000);
        assert_eq!(alloc[0] + alloc[1], 1000);
    }

    #[test]
    fn nothing_to_share_nothing_allocated() {
        assert!(allocate_budget(&[], &ReadOptions::budget(1000), None).is_empty());
    }

    // ── read + render ──

    #[test]
    fn reading_a_thread_applies_new_content_and_the_budget() {
        let thread = vec![
            msg("m1", 1, ASK),
            msg("m2", 2, &format!("{ANSWER}\n\nOn Mon, 1 Jun 2017, Ana wrote:\n> {ASK}")),
        ];
        let read = read_thread(&thread, &ReadOptions::budget(10_000));
        assert_eq!(read.omitted, 0);
        assert_eq!(read.messages.len(), 2);
        assert_eq!(read.messages[1].text, ANSWER);
        let total: usize = read.messages.iter().map(|m| m.text.chars().count()).sum();
        assert!(total <= 10_000);
    }

    #[test]
    fn the_rendered_thread_names_each_message_by_id_and_notes_what_was_omitted() {
        let thread: Vec<ThreadMessage> = (1..=6)
            .map(|i| {
                msg(
                    &format!("m{i}"),
                    i,
                    &format!("message {i} says something new. ").repeat(160),
                )
            })
            .collect();
        let read = read_thread(&thread, &ReadOptions::budget(1000));
        assert_eq!(read.omitted, 3);
        let text = render_thread(&read);
        assert!(
            text.starts_with("EMAIL THREAD CONTEXT\nSubject: Budget for the portal\nMessages: 6\n"),
            "{text}"
        );
        assert!(text.contains("[3 earlier messages omitted]"), "{text}");
        assert!(text.contains("[4] (id: m4) From: Sender m4 <m4@example.com>"), "{text}");
        assert!(!text.contains("(id: m1)"), "{text}");
    }
}
