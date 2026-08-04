//! Conversation lifecycle: title derivation plus the thin CRUD service layer
//! the chat commands delegate to (list / create / rename / delete / messages,
//! and thread-seeded conversation creation).

use crate::db::Database;
use crate::models::error::Result;
use crate::models::ChatMessage;

// ── Title derivation ────────────────────────────────────────────────────────

/// Max characters to keep when auto-deriving a chat title from the first user
/// message. Long enough to stay readable, short enough to fit in the sidebar.
const TITLE_MAX_CHARS: usize = 60;

/// Title values treated as "unset" — the auto-title logic overwrites these.
/// Matches the default from `commands::chat::create_chat_conversation`.
pub(super) fn title_is_default(title: &str) -> bool {
    let t = title.trim();
    t.is_empty() || t.eq_ignore_ascii_case("new chat")
}

/// Derive a conversation title from the first user message.
///
/// Rules:
///   - Collapse consecutive whitespace / newlines to single spaces.
///   - Truncate to TITLE_MAX_CHARS at a word boundary (if possible).
///   - Append "…" when truncated.
///   - Capitalize the first character so the sidebar looks tidy.
pub fn derive_title(message: &str) -> String {
    // Collapse all whitespace into single spaces.
    let collapsed: String = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.is_empty() {
        return "New chat".to_string();
    }

    // char-aware truncation at a word boundary if possible.
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= TITLE_MAX_CHARS {
        return capitalize_first(trimmed);
    }
    // Try to cut at the last space within TITLE_MAX_CHARS.
    let slice: String = chars.iter().take(TITLE_MAX_CHARS).collect();
    let cut = slice.rfind(' ').unwrap_or(slice.len());
    let mut head = slice[..cut].trim_end().to_string();
    if head.is_empty() {
        // Single very long word — hard-cut at TITLE_MAX_CHARS.
        head = slice;
    }
    head.push('…');
    capitalize_first(&head)
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// ── Conversations CRUD ──────────────────────────────────────────────────────
//
// Thin service layer over the chat DB. Commands delegate here so the project's
// command/service/db layering convention is consistently followed (per
// CLAUDE.md). Trivial today; gives us a place to grow conversation lifecycle
// concerns (auto-titling, archival, retention) without churning the command
// surface.

pub fn list_conversations(db: &Database, account_id: &str) -> Result<Vec<crate::models::ChatConversation>> {
    db.list_chat_conversations(account_id)
}

pub fn create_conversation(
    db: &Database,
    account_id: &str,
    title: Option<String>,
) -> Result<crate::models::ChatConversation> {
    let title = title.unwrap_or_else(|| "New chat".to_string());
    db.create_chat_conversation(account_id, &title)
}

pub fn rename_conversation(db: &Database, id: &str, title: &str) -> Result<()> {
    db.rename_chat_conversation(id, title)
}

pub fn delete_conversation(db: &Database, id: &str) -> Result<()> {
    db.delete_chat_conversation(id)
}

pub fn get_messages(db: &Database, conversation_id: &str) -> Result<Vec<ChatMessage>> {
    db.get_chat_messages(conversation_id)
}

/// Create a new conversation seeded with the cleaned content of an email
/// thread. The thread is stored as a single role='system' message; on every
/// turn `run_chat_turn` detects its presence, skips RAG retrieval / tools, and
/// injects the cleaned thread into the system prompt. Snapshot semantics: the
/// chat sees the thread exactly as it was at conversation creation time.
pub fn create_conversation_with_thread(
    db: &Database,
    account_id: &str,
    thread_id: &str,
) -> Result<crate::models::ChatConversation> {
    let (context, subject) = build_thread_context(db, account_id, thread_id)?;
    let title = thread_title_from_subject(&subject);
    db.create_chat_conversation_with_system_message(account_id, &title, &context)
}

/// Hydrate a thread into the cleaned, quote-stripped context block the chat
/// system prompt embeds, returning it alongside the thread's subject.
///
/// Shared by two callers with different lifetimes for the result:
///   - [`create_conversation_with_thread`] persists it as a role='system'
///     message, binding the whole conversation to the thread (snapshot
///     semantics — the chat sees the thread as of creation time).
///   - the ambient-context path in `run_chat_turn` builds it fresh per turn,
///     for the thread the user currently has open in the main view.
pub fn build_thread_context(db: &Database, account_id: &str, thread_id: &str) -> Result<(String, String)> {
    let emails = db.get_thread(account_id, thread_id)?;
    if emails.is_empty() {
        return Err(crate::models::error::AppError::NotFound(format!(
            "thread {thread_id} for account {account_id}"
        )));
    }

    // Hydrate body for each email up front. Empty bodies (e.g. emails awaiting
    // re-download) are surfaced in the formatted context as "(empty)".
    let mut bodies: std::collections::HashMap<String, String> = std::collections::HashMap::with_capacity(emails.len());
    for e in &emails {
        // Bodies that fail to load for any reason are downgraded to empty so
        // a single broken row can't block the entire chat creation.
        let body = db.get_email_body(&e.id).unwrap_or_default();
        bodies.insert(e.id.clone(), body);
    }

    let context = crate::services::thread_clean::format_thread_context(
        &emails,
        |id| bodies.get(id).cloned(),
        crate::services::thread_clean::chars_per_email(emails.len()),
    );

    Ok((context, emails[0].subject.clone()))
}

/// Strip RE:/FWD: prefixes (and locale variants) from a subject and truncate
/// to a sidebar-friendly length. Falls back to "Email thread" when the subject
/// is empty after stripping.
fn thread_title_from_subject(subject: &str) -> String {
    let mut s = subject.trim().to_string();
    let prefixes = ["re:", "fw:", "fwd:", "rv:", "tr:", "wg:", "aw:"];
    loop {
        let lower = s.to_lowercase();
        let stripped = prefixes.iter().find_map(|p| lower.strip_prefix(p).map(|_| p.len()));
        match stripped {
            Some(n) => {
                s = s[n..].trim_start().to_string();
            }
            None => break,
        }
    }
    if s.is_empty() {
        return "Email thread".to_string();
    }
    // Reuse the existing title-truncation helper for consistency with auto-titles.
    derive_title(&s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn seed_thread_email(db: &Database, id: &str, account: &str, thread_id: &str, subject: &str, body: &str) {
        let conn = db.connection();
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
             VALUES (?1, 'gmail', ?1, 'Test', 0)",
            params![account],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails
             (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
              recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
             VALUES (?1,?2,?3,?4,'Dana Ito','dana@example.test','example.test','[]','[]','snip',100,0,'primary',0)",
            params![id, account, thread_id, subject],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO email_bodies(email_id, body) VALUES (?1,?2)",
            params![id, body],
        )
        .unwrap();
    }

    // ── Thread context (shared by seeded conversations + the chat panel's
    //    ambient per-turn grounding) ───────────────────────────────────────

    #[test]
    fn thread_context_includes_body_and_returns_subject() {
        let db = Database::new_for_testing().expect("test db");
        seed_thread_email(&db, "e1", "acct-1", "t-1", "Depot handover", "Keys are in the lockbox.");

        let (context, subject) = build_thread_context(&db, "acct-1", "t-1").expect("context");

        assert!(context.contains("Keys are in the lockbox."), "body missing: {context}");
        assert_eq!(subject, "Depot handover");
    }

    #[test]
    fn thread_context_errors_for_unknown_thread() {
        // The chat panel's ambient path relies on this being an error rather
        // than an empty-but-Ok context: an empty context would silently send a
        // "answer only from the thread above" prompt with no thread in it. The
        // caller catches this and falls back to normal retrieval instead.
        let db = Database::new_for_testing().expect("test db");
        let err = build_thread_context(&db, "acct-1", "t-missing").expect_err("should not resolve");
        assert!(
            matches!(err, crate::models::error::AppError::NotFound(_)),
            "expected NotFound, got {err:?}"
        );
    }

    #[test]
    fn thread_context_scoped_to_its_account() {
        // Cross-account read guard: a thread id from another account must not
        // resolve, or the panel could ground a turn in someone else's mail.
        let db = Database::new_for_testing().expect("test db");
        seed_thread_email(&db, "e1", "acct-1", "t-1", "Depot handover", "Keys are in the lockbox.");

        assert!(build_thread_context(&db, "acct-2", "t-1").is_err());
    }

    // ── Title derivation ────────────────────────────────────────────────

    #[test]
    fn title_default_values_detected() {
        assert!(title_is_default(""));
        assert!(title_is_default("  "));
        assert!(title_is_default("New chat"));
        assert!(title_is_default("new chat"));
        assert!(title_is_default("NEW CHAT"));
        assert!(!title_is_default("My actual chat"));
    }

    #[test]
    fn title_short_message_capitalized() {
        assert_eq!(derive_title("hola mundo"), "Hola mundo");
    }

    #[test]
    fn title_collapses_whitespace() {
        assert_eq!(
            derive_title("hazme   un\n resumen\tde los emails"),
            "Hazme un resumen de los emails"
        );
    }

    #[test]
    fn title_truncates_long_message_at_word_boundary() {
        let msg = "pasame todas las facturas enviadas durante el último trimestre del año \
                   pasado agrupadas por cliente";
        let t = derive_title(msg);
        assert!(t.ends_with('…'));
        // Should never exceed TITLE_MAX_CHARS + 1 (for the ellipsis).
        assert!(t.chars().count() <= TITLE_MAX_CHARS + 1);
        // Should cut at a word boundary (no trailing partial word before '…').
        let without_ellipsis = t.trim_end_matches('…');
        assert!(!without_ellipsis.ends_with(' '));
    }

    #[test]
    fn title_empty_message_fallback() {
        assert_eq!(derive_title("   \n\t "), "New chat");
    }
}
