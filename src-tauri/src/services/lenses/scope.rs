//! Evaluate a `LensScope` to the list of matching `email_id`s.
//!
//! The evaluator is deliberately conservative for v1: it composes SQL from
//! verified-safe primitives (no user-supplied identifiers reach the SQL —
//! only values, always via parameter binding) and never uses leading wildcards
//! on indexed columns. FTS5 queries are pre-sanitised with `escape_fts_query`
//! to neutralise user-supplied operator syntax.

use rusqlite::params_from_iter;

use crate::db::Database;
use crate::models::error::Result;
use crate::models::lens::{Direction, LensScope};

/// Maximum email_ids a single scope evaluation will return. v1 caps at 5 000
/// so a runaway scope can't materialise the entire mailbox into memory. PRD
/// §7.4 calls out 32 766 as SQLite's parameter limit; we leave headroom.
pub const MAX_SCOPE_RESULTS: i64 = 5_000;

/// Return `email_id`s that match the scope, ordered by `timestamp DESC`.
pub fn evaluate(db: &Database, scope: &LensScope) -> Result<Vec<String>> {
    evaluate_with_limit(db, scope, MAX_SCOPE_RESULTS)
}

/// Same as [`evaluate`] but with an explicit cap (useful for previews).
pub fn evaluate_with_limit(db: &Database, scope: &LensScope, limit: i64) -> Result<Vec<String>> {
    let conn = db.reader();

    // SAFETY: every fragment below either uses a literal SQL constant or a
    // parameter binding. We never interpolate user-supplied strings into SQL.
    let mut where_parts: Vec<String> = vec!["e.is_deleted = 0".to_string()];
    let mut binds: Vec<rusqlite::types::Value> = Vec::new();

    // Accounts.
    if let Some(account_ids) = scope.account_ids.as_ref() {
        if account_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (1..=account_ids.len())
            .map(|i| format!("?{}", binds.len() + i))
            .collect::<Vec<_>>()
            .join(",");
        where_parts.push(format!("e.account_id IN ({placeholders})"));
        for id in account_ids {
            binds.push(id.clone().into());
        }
    }

    // Mailboxes.
    if let Some(boxes) = scope.mailboxes.as_ref() {
        if boxes.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (1..=boxes.len())
            .map(|i| format!("?{}", binds.len() + i))
            .collect::<Vec<_>>()
            .join(",");
        where_parts.push(format!("e.mailbox IN ({placeholders})"));
        for b in boxes {
            binds.push(b.clone().into());
        }
    }

    // Categories. The UI presents capitalized Gmail names ("Primary", "Updates")
    // but sync stores `emails.category` lowercased, so we normalize both sides.
    if let Some(cats) = scope.categories.as_ref() {
        if cats.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (1..=cats.len())
            .map(|i| format!("?{}", binds.len() + i))
            .collect::<Vec<_>>()
            .join(",");
        where_parts.push(format!("LOWER(e.category) IN ({placeholders})"));
        for c in cats {
            binds.push(c.to_lowercase().into());
        }
    }

    // Sender domains.
    if let Some(domains) = scope.sender_domains.as_ref() {
        if domains.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (1..=domains.len())
            .map(|i| format!("?{}", binds.len() + i))
            .collect::<Vec<_>>()
            .join(",");
        where_parts.push(format!("e.sender_domain IN ({placeholders})"));
        for d in domains {
            binds.push(d.to_lowercase().into());
        }
    }

    // Sender emails.
    if let Some(emails) = scope.sender_emails.as_ref() {
        if emails.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (1..=emails.len())
            .map(|i| format!("?{}", binds.len() + i))
            .collect::<Vec<_>>()
            .join(",");
        where_parts.push(format!("LOWER(e.sender_email) IN ({placeholders})"));
        for em in emails {
            binds.push(em.to_lowercase().into());
        }
    }

    // Direction. Inbound = mailbox != 'sent'; Outbound = mailbox = 'sent'.
    if let Some(dir) = scope.direction {
        match dir {
            Direction::Inbound => where_parts.push("e.mailbox != 'sent'".to_string()),
            Direction::Outbound => where_parts.push("e.mailbox = 'sent'".to_string()),
            Direction::Either => {}
        }
    }

    // Date range.
    if let Some(range) = scope.date_range.as_ref() {
        if let Some(days) = range.last_days {
            let cutoff = now_secs().saturating_sub(days.saturating_mul(86_400));
            where_parts.push(format!("e.timestamp >= ?{}", binds.len() + 1));
            binds.push(cutoff.into());
        } else {
            if let Some(from) = range.from {
                where_parts.push(format!("e.timestamp >= ?{}", binds.len() + 1));
                binds.push(from.into());
            }
            if let Some(to) = range.to {
                where_parts.push(format!("e.timestamp <= ?{}", binds.len() + 1));
                binds.push(to.into());
            }
        }
    }

    // FTS5 keyword query.
    // `query_search_body` defaults to true (search subject + sender + body).
    // When false, restrict the MATCH to the subject column only.
    let fts_join = if let Some(q) = scope.query.as_ref().filter(|q| !q.trim().is_empty()) {
        let safe = escape_fts_query(q);
        let fts_target = if scope.query_search_body {
            "emails_fts MATCH"
        } else {
            "subject MATCH"
        };
        where_parts.push(format!(
            "e.id IN (SELECT email_id FROM emails_fts WHERE {fts_target} ?{})",
            binds.len() + 1
        ));
        binds.push(safe.into());
        true
    } else {
        false
    };

    // Tag filters (one AND'd EXISTS clause per tag).
    if let Some(tags) = scope.tags.as_ref() {
        for tag in tags {
            where_parts.push(format!(
                "EXISTS (SELECT 1 FROM email_tags et WHERE et.email_id = e.id \
                 AND et.tag_type = ?{} AND et.tag_value = ?{})",
                binds.len() + 1,
                binds.len() + 2
            ));
            binds.push(tag.tag_type.clone().into());
            binds.push(tag.tag_value.clone().into());
        }
    }

    let where_sql = where_parts.join(" AND ");
    where_parts.clear(); // not used after this point
                         // _fts_join not actually used — the IN-subquery handles it cleanly.
    let _ = fts_join;

    let limit_bind = binds.len() + 1;
    let sql = format!(
        "SELECT e.id FROM emails e \
         WHERE {where_sql} \
         ORDER BY e.timestamp DESC, e.id DESC \
         LIMIT ?{limit_bind}"
    );
    binds.push(limit.into());

    let mut stmt = conn.prepare(&sql)?;
    let ids = stmt
        .query_map(params_from_iter(binds.iter()), |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(ids)
}

/// FTS5 has its own micro-syntax (`AND`, `OR`, `NEAR`, quoted phrases, etc.).
/// To keep user-supplied queries safe and predictable in v1 we strip
/// double-quotes and dangerous punctuation, leaving an AND-joined bag of
/// terms. Phase 2 can expose advanced operators behind a power-user toggle.
fn escape_fts_query(q: &str) -> String {
    let mut out = String::with_capacity(q.len());
    let mut prev_space = true;
    for c in q.chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' || c == '@' || c == '.' {
            out.push(c);
            prev_space = false;
        } else if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            // Drop quotes, parens, asterisks — anything that could become an operator.
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        }
    }
    out.trim().to_string()
}

fn now_secs() -> i64 {
    crate::services::clock::now_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::lens::{DateRange, Direction, LensScope, TagFilter};

    fn insert_email(db: &Database, id: &str, account: &str, mailbox: &str, ts: i64) {
        insert_email_with_category(db, id, account, mailbox, "primary", ts);
    }

    /// Insert an email with explicit sender_email and sender_domain, for
    /// tests that exercise the sender-based scope filters directly.
    #[allow(clippy::too_many_arguments)]
    fn insert_email_with_sender(
        db: &Database,
        id: &str,
        account: &str,
        sender_email: &str,
        sender_domain: &str,
        mailbox: &str,
        ts: i64,
    ) {
        let conn = db.connection();
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at) \
             VALUES (?1, 'gmail', ?1, 'Test', 0)",
            rusqlite::params![account],
        )
        .expect("seed account");
        conn.execute(
            "INSERT INTO emails (id, account_id, thread_id, subject, sender, sender_email, \
                                 sender_domain, recipients_json, cc_json, snippet, timestamp, \
                                 mailbox, category, created_at) \
             VALUES (?1, ?2, ?3, 'Subject', 'Sender', ?4, ?5, '[]', '[]', '', ?6, ?7, 'primary', ?6)",
            rusqlite::params![id, account, format!("t-{id}"), sender_email, sender_domain, ts, mailbox],
        )
        .expect("insert email");
        conn.execute(
            "INSERT INTO email_bodies (email_id, body) VALUES (?1, '')",
            rusqlite::params![id],
        )
        .expect("insert email body");
    }

    fn insert_email_with_category(db: &Database, id: &str, account: &str, mailbox: &str, category: &str, ts: i64) {
        let conn = db.connection();
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at) \
             VALUES (?1, 'gmail', ?1, 'Test', 0)",
            rusqlite::params![account],
        )
        .expect("seed account");
        conn.execute(
            "INSERT INTO emails (id, account_id, thread_id, subject, sender, sender_email, \
                                 sender_domain, recipients_json, cc_json, snippet, timestamp, \
                                 mailbox, category, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '[]', '[]', '', ?8, ?9, ?10, ?8)",
            rusqlite::params![
                id,
                account,
                format!("t-{id}"),
                "Subject",
                "Sender",
                format!("sender@{account}.example"),
                format!("{account}.example"),
                ts,
                mailbox,
                category,
            ],
        )
        .expect("insert email");
        conn.execute(
            "INSERT INTO email_bodies (email_id, body) VALUES (?1, '')",
            rusqlite::params![id],
        )
        .expect("insert email body");
    }

    #[test]
    fn empty_scope_returns_all_recent_emails() {
        let db = Database::new_for_testing().expect("db");
        insert_email(&db, "a", "acct1", "inbox", 100);
        insert_email(&db, "b", "acct1", "sent", 200);
        insert_email(&db, "c", "acct2", "inbox", 300);

        let ids = evaluate(&db, &LensScope::default()).unwrap();
        assert_eq!(ids, vec!["c".to_string(), "b".to_string(), "a".to_string()]);
    }

    #[test]
    fn account_filter_narrows_result() {
        let db = Database::new_for_testing().expect("db");
        insert_email(&db, "a", "acct1", "inbox", 100);
        insert_email(&db, "b", "acct2", "inbox", 200);

        let scope = LensScope {
            account_ids: Some(vec!["acct2".into()]),
            ..Default::default()
        };
        let ids = evaluate(&db, &scope).unwrap();
        assert_eq!(ids, vec!["b".to_string()]);
    }

    #[test]
    fn direction_outbound_filters_to_sent_mailbox() {
        let db = Database::new_for_testing().expect("db");
        insert_email(&db, "in1", "acct1", "inbox", 100);
        insert_email(&db, "out1", "acct1", "sent", 200);

        let scope = LensScope {
            direction: Some(Direction::Outbound),
            ..Default::default()
        };
        let ids = evaluate(&db, &scope).unwrap();
        assert_eq!(ids, vec!["out1".to_string()]);
    }

    #[test]
    fn empty_account_list_yields_no_results() {
        let db = Database::new_for_testing().expect("db");
        insert_email(&db, "a", "acct1", "inbox", 100);

        let scope = LensScope {
            account_ids: Some(vec![]),
            ..Default::default()
        };
        let ids = evaluate(&db, &scope).unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn date_range_last_days_filters_old_emails() {
        let db = Database::new_for_testing().expect("db");
        let now = now_secs();
        insert_email(&db, "recent", "acct1", "inbox", now - 60); // 1 minute ago
        insert_email(&db, "old", "acct1", "inbox", now - 86_400 * 90); // 90 days ago

        let scope = LensScope {
            date_range: Some(DateRange {
                last_days: Some(30),
                from: None,
                to: None,
            }),
            ..Default::default()
        };
        let ids = evaluate(&db, &scope).unwrap();
        assert_eq!(ids, vec!["recent".to_string()]);
    }

    #[test]
    fn tag_filter_requires_matching_tag() {
        let db = Database::new_for_testing().expect("db");
        insert_email(&db, "tagged", "acct1", "inbox", 200);
        insert_email(&db, "untagged", "acct1", "inbox", 100);
        {
            let conn = db.connection();
            conn.execute(
                "INSERT INTO email_tags (email_id, tag_type, tag_value, created_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params!["tagged", "topic", "invoice", 0i64],
            )
            .unwrap();
        }
        let scope = LensScope {
            tags: Some(vec![TagFilter {
                tag_type: "topic".into(),
                tag_value: "invoice".into(),
            }]),
            ..Default::default()
        };
        let ids = evaluate(&db, &scope).unwrap();
        assert_eq!(ids, vec!["tagged".to_string()]);
    }

    /// Integration test for the default scope shipped to new Lenses from the
    /// custom-creation form:
    ///
    ///   - mailbox: inbox
    ///   - direction: inbound only
    ///   - categories: Primary, Updates
    ///   - last 60 days
    ///
    /// Hits the real SQL evaluator end-to-end against an in-memory DB.
    #[test]
    fn default_lens_scope_selects_only_matching_emails() {
        let db = Database::new_for_testing().expect("db");
        let now = now_secs();
        let day = 86_400i64;

        // Should match — inbox, Primary, recent.
        insert_email_with_category(&db, "ok_primary", "acct1", "inbox", "Primary", now - 5 * day);
        // Should match — inbox, Updates, recent.
        insert_email_with_category(&db, "ok_updates", "acct1", "inbox", "Updates", now - 30 * day);
        // Should match — at the 60-day boundary (cutoff is inclusive >=).
        insert_email_with_category(
            &db,
            "ok_boundary",
            "acct1",
            "inbox",
            "Primary",
            now - 60 * day + 60, // 60 days minus 1 minute → still inside window
        );

        // Filtered out — Promotions category.
        insert_email_with_category(&db, "no_promo", "acct1", "inbox", "Promotions", now - day);
        // Filtered out — Social category.
        insert_email_with_category(&db, "no_social", "acct1", "inbox", "Social", now - day);
        // Filtered out — outbound (mailbox = sent → direction Inbound rejects).
        insert_email_with_category(&db, "no_sent", "acct1", "sent", "Primary", now - 2 * day);
        // Filtered out — archive mailbox.
        insert_email_with_category(&db, "no_archive", "acct1", "archive", "Primary", now - 2 * day);
        // Filtered out — older than 60 days.
        insert_email_with_category(&db, "no_old", "acct1", "inbox", "Primary", now - 120 * day);

        let scope = LensScope {
            mailboxes: Some(vec!["inbox".into()]),
            categories: Some(vec!["Primary".into(), "Updates".into()]),
            direction: Some(Direction::Inbound),
            date_range: Some(DateRange {
                last_days: Some(60),
                from: None,
                to: None,
            }),
            ..Default::default()
        };

        let mut ids = evaluate(&db, &scope).unwrap();
        ids.sort();

        let mut expected = vec![
            "ok_primary".to_string(),
            "ok_updates".to_string(),
            "ok_boundary".to_string(),
        ];
        expected.sort();
        assert_eq!(
            ids, expected,
            "default scope should return exactly the inbox/Primary|Updates/inbound/<=60d emails"
        );
    }

    /// Regression test: sync writes `emails.category` lowercased (`primary`,
    /// `updates`, …) but the Lens scope editor sends the Gmail-style
    /// capitalized names from the UI (`"Primary"`, `"Updates"`). Without
    /// case-insensitive matching, a Lens scoped to Primary+Updates would
    /// match zero rows on real data — pin the normalization here.
    #[test]
    fn category_filter_matches_case_insensitively() {
        let db = Database::new_for_testing().expect("db");
        let now = now_secs();
        let day = 86_400i64;

        // DB stores categories lowercase (this matches what real sync writes).
        insert_email_with_category(&db, "ok_primary", "acct1", "inbox", "primary", now - day);
        insert_email_with_category(&db, "ok_updates", "acct1", "inbox", "updates", now - day);
        insert_email_with_category(&db, "no_social", "acct1", "inbox", "social", now - day);

        // Scope uses the capitalized form the UI emits.
        let scope = LensScope {
            mailboxes: Some(vec!["inbox".into()]),
            categories: Some(vec!["Primary".into(), "Updates".into()]),
            direction: Some(Direction::Inbound),
            ..Default::default()
        };

        let mut ids = evaluate(&db, &scope).unwrap();
        ids.sort();
        assert_eq!(ids, vec!["ok_primary".to_string(), "ok_updates".to_string()]);
    }

    #[test]
    fn fts_special_chars_are_neutralised() {
        // Quoted phrase + operator chars must not break the query.
        let cleaned = escape_fts_query(r#"hello "world" OR foo*"#);
        assert!(!cleaned.contains('"'));
        assert!(!cleaned.contains('*'));
        // Whitespace-separated terms are preserved.
        assert!(cleaned.contains("hello"));
        assert!(cleaned.contains("world"));
    }

    /// Scope with only sender_domains set — email whose domain matches must be returned.
    #[test]
    fn sender_domain_filter_alone_matches() {
        let db = Database::new_for_testing().expect("db");
        let now = now_secs();
        insert_email_with_sender(
            &db,
            "hub",
            "acc1",
            "invoice@impacthub.net",
            "impacthub.net",
            "inbox",
            now,
        );
        insert_email_with_sender(
            &db,
            "stripe",
            "acc1",
            "billing@stripe.com",
            "stripe.com",
            "inbox",
            now - 1,
        );

        let scope = LensScope {
            sender_domains: Some(vec!["impacthub.net".into()]),
            ..Default::default()
        };
        let ids = evaluate(&db, &scope).unwrap();
        assert!(
            ids.contains(&"hub".to_string()),
            "domain-matched email must be returned"
        );
        assert!(
            !ids.contains(&"stripe".to_string()),
            "stripe.com email must not match impacthub.net domain filter"
        );
    }

    /// Scope with only sender_emails set — exact-address email must be returned.
    #[test]
    fn sender_email_filter_alone_matches() {
        let db = Database::new_for_testing().expect("db");
        let now = now_secs();
        insert_email_with_sender(
            &db,
            "hub",
            "acc1",
            "invoice@impacthub.net",
            "impacthub.net",
            "inbox",
            now,
        );
        insert_email_with_sender(
            &db,
            "stripe",
            "acc1",
            "billing@stripe.com",
            "stripe.com",
            "inbox",
            now - 1,
        );

        let scope = LensScope {
            sender_emails: Some(vec!["billing@stripe.com".into()]),
            ..Default::default()
        };
        let ids = evaluate(&db, &scope).unwrap();
        assert!(
            ids.contains(&"stripe".to_string()),
            "exact-email-matched email must be returned"
        );
        assert!(
            !ids.contains(&"hub".to_string()),
            "impacthub.net email must not match billing@stripe.com filter"
        );
    }
}
