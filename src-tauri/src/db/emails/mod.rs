use crate::db::Database;
use crate::models::error::Result;
use crate::models::{
    CompanyContactsGroup, Contact, ContactDetail, ContactsPage, ContactsQuery, Email, FilterSuggestion,
    FilteredEmailsResult, QuickFilterStats,
};
use rusqlite::params;
use std::collections::HashMap;

pub mod attachments;
pub mod autocomplete;
pub mod batch;
pub mod contacts;
pub mod crud;
pub mod failed;
pub mod search;

#[cfg(test)]
pub(super) mod test_helpers;

// Common helpers live in crate::util — re-exported here so submodules can keep
// using bare names (`classify_kind(addr)` etc.) without long absolute paths.
pub(super) use crate::util::email_addr::{classify_kind, parse_addr_list, split_name_addr};
pub(super) use crate::util::html::strip_html_for_fts;

// Body lives in email_bodies — all queries on the emails table use this column list.
pub(super) const EMAIL_COLUMNS: &str = "id, account_id, thread_id, message_id, subject, sender, sender_email, \
     recipients_json, cc_json, snippet, timestamp, is_read, triage_status, category, mailbox";

pub(super) fn row_to_email(row: &rusqlite::Row) -> rusqlite::Result<Email> {
    let recipients_json: String = row.get(7)?;
    let recipients: Vec<String> = serde_json::from_str(&recipients_json).unwrap_or_default();
    let cc_json: String = row.get::<_, String>(8).unwrap_or_else(|_| "[]".to_string());
    let cc: Vec<String> = serde_json::from_str(&cc_json).unwrap_or_default();

    Ok(Email {
        id: row.get(0)?,
        account_id: row.get(1)?,
        thread_id: row.get(2)?,
        message_id: row.get(3)?,
        subject: row.get(4)?,
        sender: row.get(5)?,
        sender_email: row.get(6)?,
        recipients,
        cc,
        body: String::new(),
        snippet: row.get(9)?,
        timestamp: row.get(10)?,
        is_read: row.get::<_, i32>(11)? != 0,
        triage_status: row.get(12)?,
        category: row.get::<_, String>(13).unwrap_or_else(|_| "primary".to_string()),
        mailbox: row.get::<_, String>(14).unwrap_or_else(|_| "inbox".to_string()),
    })
}

/// Compute the exclusive upper bound for a prefix range scan.
/// e.g. "ashley" → Some("ashlez"), "z" → Some("{"), "\u{ffff}" → None.
pub(super) fn prefix_upper_bound(prefix: &str) -> Option<String> {
    let mut bytes = prefix.as_bytes().to_vec();
    // Increment the last byte; carry if it overflows.
    while let Some(last) = bytes.pop() {
        if last < 0xFF {
            bytes.push(last + 1);
            return Some(String::from_utf8_lossy(&bytes).into_owned());
        }
        // 0xFF → carry to previous byte
    }
    None // every byte was 0xFF — no upper bound
}

/// Domain extracted for the `sender_domain` index column. Empty default when
/// no `@` is present so the column is never NULL.
pub(super) fn extract_sender_domain(sender_email: &str) -> String {
    crate::util::email_addr::extract_domain(sender_email).unwrap_or_default()
}

// ── Contact aggregation helpers ─────────────────────────────────────────────

#[derive(Default)]
pub(super) struct ContactAccum {
    pub(super) received: i32,
    pub(super) sent: i32,
    pub(super) last_ts: Option<i64>,
    pub(super) first_ts: Option<i64>,
    pub(super) name: String,
}

impl ContactAccum {
    pub(super) fn bump_ts(&mut self, ts: i64) {
        self.last_ts = Some(self.last_ts.map_or(ts, |cur| cur.max(ts)));
        self.first_ts = Some(self.first_ts.map_or(ts, |cur| cur.min(ts)));
    }
}

/// Relationship score in [0, 100]. Blends three signals: log-frequency,
/// recency (30-day half-life), and bidirectionality (ratio of the smaller
/// side to the larger). Tuned so that a frequent two-way correspondent
/// crosses 60+ while a one-shot newsletter sits below 20.
pub(super) fn relationship_score(a: &ContactAccum, now_secs: i64) -> f64 {
    // No reciprocity, no relationship. If the user has never sent/replied to
    // this contact, the score is 0 regardless of how many emails they sent us
    // or how recently — an inbox full of newsletters is not a relationship.
    if a.sent == 0 {
        return 0.0;
    }

    let total = (a.received + a.sent).max(1) as f64;
    // ln(1100) ≈ 7, so total≈1100 saturates the frequency component.
    let frequency = (total.ln() / 7.0).clamp(0.0, 1.0);

    let last = a.last_ts.unwrap_or(0).max(0);
    let days = ((now_secs - last) as f64 / 86_400.0).max(0.0);
    let recency = 1.0 / (1.0 + days / 30.0);

    // `sent == 0` already returned 0 above, so here we only need to guard the
    // received-only direction (we wrote to them but never heard back).
    let bidi = if a.received == 0 {
        0.0
    } else {
        let r = a.received as f64;
        let s = a.sent as f64;
        r.min(s) / r.max(s)
    };

    let raw = 0.45 * frequency + 0.35 * recency + 0.20 * bidi;
    (raw * 100.0).round()
}

/// Clamp a mailbox string to the four supported values. Unknown values fall back
/// to 'inbox' so a stray provider value can never poison the DB.
pub(crate) fn normalize_mailbox(raw: &str) -> &'static str {
    match raw {
        "sent" => "sent",
        "spam" => "spam",
        "trash" => "trash",
        _ => "inbox",
    }
}

// Used by test code to benchmark the scalar subquery approach against
// the inbox-scoped variant. Production paths use `latest_inbox_email_predicate`.
#[cfg(test)]
pub(super) fn latest_thread_email_predicate(alias: &str) -> String {
    // Scalar subquery: check that this email IS the latest in its thread.
    // Uses idx_emails_thread_latest (account_id, thread_id, timestamp DESC, id DESC)
    // for a single index seek per row — much faster than NOT EXISTS with OR.
    format!(
        "{alias}.id = (
            SELECT sub.id FROM emails sub
            WHERE sub.account_id = {alias}.account_id
              AND sub.thread_id = {alias}.thread_id
              AND sub.is_deleted = 0
            ORDER BY sub.timestamp DESC, sub.id DESC
            LIMIT 1
        )"
    )
}

/// Inbox-scoped variant of [`latest_thread_email_predicate`].
///
/// Picks the latest email in the thread that is *itself* in the Inbox.
/// Without this, when `sync_extra_mailboxes` ingests a Sent reply with a
/// newer timestamp than the original message, the cross-mailbox predicate
/// selects the Sent row — but the inbox view's `mailbox = 'inbox'` clause
/// then filters that row out, AND the older inbox row fails the "is latest"
/// test, so the entire thread disappears from the inbox. Every thread the
/// user replied to became invisible.
pub(super) fn latest_inbox_email_predicate(alias: &str) -> String {
    format!(
        "{alias}.id = (
            SELECT sub.id FROM emails sub
            WHERE sub.account_id = {alias}.account_id
              AND sub.thread_id = {alias}.thread_id
              AND sub.is_deleted = 0
              AND sub.mailbox = 'inbox'
            ORDER BY sub.timestamp DESC, sub.id DESC
            LIMIT 1
        )"
    )
}

pub(super) fn thread_order_clause(alias: &str, ascending: bool) -> String {
    let dir = if ascending { "ASC" } else { "DESC" };
    format!("{alias}.timestamp {dir}, {alias}.id {dir}")
}

/// Sanitize a user query for FTS5 MATCH.
/// Escapes special characters and converts to a prefix query so partial words match.
pub(super) fn sanitize_fts_query(query: &str) -> String {
    let words: Vec<String> = query
        .split_whitespace()
        .filter(|w| !w.is_empty())
        .map(|w| {
            // Strip FTS5 operators and special chars
            let cleaned: String = w
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '@' || *c == '.')
                .collect();
            if cleaned.is_empty() {
                String::new()
            } else {
                // Use prefix matching (word*) so partial words match
                format!("\"{}\"*", cleaned)
            }
        })
        .filter(|w| !w.is_empty())
        .collect();

    if words.is_empty() {
        String::new()
    } else {
        words.join(" ")
    }
}

#[cfg(test)]
mod relationship_score_tests {
    use super::{relationship_score, ContactAccum};

    const NOW: i64 = 1_700_000_000;

    #[test]
    fn never_sent_scores_zero_even_when_frequent_and_recent() {
        // A contact the user has never replied to / sent to is not a real
        // relationship, no matter how many emails they sent us or how
        // recently. Relationship strength requires reciprocity.
        let inbound_only = ContactAccum {
            received: 500,
            sent: 0,
            last_ts: Some(NOW),
            first_ts: Some(NOW - 86_400),
            name: "Newsletter".into(),
        };
        assert_eq!(
            relationship_score(&inbound_only, NOW),
            0.0,
            "a contact we never sent to must score 0"
        );
    }

    #[test]
    fn bidirectional_contact_scores_above_zero() {
        let two_way = ContactAccum {
            received: 20,
            sent: 15,
            last_ts: Some(NOW),
            first_ts: Some(NOW - 30 * 86_400),
            name: "Alice".into(),
        };
        assert!(
            relationship_score(&two_way, NOW) > 0.0,
            "a two-way correspondent must score above 0"
        );
    }
}
