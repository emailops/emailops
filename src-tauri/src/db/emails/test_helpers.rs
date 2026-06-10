use super::strip_html_for_fts;
use crate::db::Database;

/// Ensures the account FK target exists without disturbing existing rows.
/// Tests share the production schema (which enforces FKs), so any helper
/// that inserts a row referencing accounts(id) must call this first.
pub(super) fn ensure_account(conn: &rusqlite::Connection, account_id: &str) {
    conn.execute(
        "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
             VALUES (?1, 'gmail', ?1, 'Test', 0)",
        rusqlite::params![account_id],
    )
    .unwrap();
}

/// Insert a minimal email row for testing (no FTS-searchable fields needed).
pub(super) fn insert_email(db: &Database, id: &str, account_id: &str, thread_id: &str, timestamp: i64) {
    let conn = db.connection();
    ensure_account(&conn, account_id);
    conn.execute(
        "INSERT INTO emails
                 (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                  recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
                 VALUES (?1,?2,?3,'subj','sender','s@s.com','s.com','[]','[]','snip',?4,0,'primary',0)",
        rusqlite::params![id, account_id, thread_id, timestamp],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO email_bodies (email_id, body) VALUES (?1, 'body')",
        rusqlite::params![id],
    )
    .unwrap();
}

/// Insert a minimal email row with a specific Gmail category.
pub(super) fn insert_email_with_category(
    db: &Database,
    id: &str,
    account_id: &str,
    thread_id: &str,
    timestamp: i64,
    category: &str,
) {
    let conn = db.connection();
    ensure_account(&conn, account_id);
    conn.execute(
        "INSERT INTO emails
                 (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                  recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox, created_at)
                 VALUES (?1,?2,?3,'subj','sender','s@s.com','s.com','[]','[]','snip',?4,0,?5,'inbox',0)",
        rusqlite::params![id, account_id, thread_id, timestamp, category],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO email_bodies (email_id, body) VALUES (?1, 'body')",
        rusqlite::params![id],
    )
    .unwrap();
}

/// Insert an email with full sender/subject/body fields for search tests.
/// Manually inserts into FTS with HTML-stripped body (triggers removed).
pub(super) fn insert_search_email(
    db: &Database,
    id: &str,
    account_id: &str,
    thread_id: &str,
    sender: &str,
    sender_email: &str,
    subject: &str,
    body: &str,
    timestamp: i64,
) {
    let sender_domain = sender_email
        .rsplit_once('@')
        .map(|(_, d)| d.to_lowercase())
        .unwrap_or_default();
    let body_text = strip_html_for_fts(body);
    let conn = db.connection();
    ensure_account(&conn, account_id);
    conn.execute(
        "INSERT INTO emails
                 (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                  recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,'[]','[]','snip',?8,0,'primary',0)",
        rusqlite::params![
            id,
            account_id,
            thread_id,
            subject,
            sender,
            sender_email,
            sender_domain,
            timestamp,
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO email_bodies (email_id, body) VALUES (?1, ?2)",
        rusqlite::params![id, body],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO emails_fts(email_id, subject, sender, body) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![id, subject, sender, body_text],
    )
    .unwrap();
}

pub(super) fn tag_email(db: &Database, email_id: &str, tag_type: &str, tag_value: &str) {
    db.connection()
        .execute(
            "INSERT OR REPLACE INTO email_tags (email_id, tag_type, tag_value, confidence, created_at)
                 VALUES (?1, ?2, ?3, NULL, 0)",
            rusqlite::params![email_id, tag_type, tag_value],
        )
        .unwrap();
}

/// Insert an email with bespoke sender + recipients_json + mailbox so we
/// can exercise the bidirectional aggregation.
pub(super) fn insert_contact_email(
    db: &Database,
    id: &str,
    account_id: &str,
    thread_id: &str,
    sender: &str,
    sender_email: &str,
    recipients_json: &str,
    mailbox: &str,
    timestamp: i64,
) {
    let sender_domain = sender_email
        .rsplit_once('@')
        .map(|(_, d)| d.to_lowercase())
        .unwrap_or_default();
    let conn = db.connection();
    ensure_account(&conn, account_id);
    conn.execute(
        "INSERT INTO emails
                 (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                  recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox, created_at)
                 VALUES (?1,?2,?3,'subj',?4,?5,?6,?7,'[]','snip',?8,0,'primary',?9,0)",
        rusqlite::params![
            id,
            account_id,
            thread_id,
            sender,
            sender_email,
            sender_domain,
            recipients_json,
            timestamp,
            mailbox,
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO email_bodies (email_id, body) VALUES (?1, 'body')",
        rusqlite::params![id],
    )
    .unwrap();
}

pub(super) fn insert_account(db: &Database, id: &str, email: &str) {
    db.connection()
        .execute(
            "INSERT OR REPLACE INTO accounts (id, provider, email, name, created_at)
                 VALUES (?1, 'gmail', ?2, 'Test', 0)",
            rusqlite::params![id, email],
        )
        .unwrap();
}
