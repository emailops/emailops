//! Dashboard aggregation: per-account stats for the developer-facing dashboard view.
//!
//! Builds a single `AccountDashboard` per account in one Tauri call so the
//! frontend doesn't have to fan out into N separate commands. All counts are
//! computed against the local DB; the optional `server_total` is fetched
//! separately by `refresh_server_total` and cached in `user_preferences`.

use std::sync::Arc;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::db::Database;
use crate::models::error::Result;
use crate::models::{Account, SyncStatus};
use crate::services::classification;
use crate::services::embeddings;
use crate::services::memory::config as memory_config;
use crate::services::tasks::config as task_config;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryCount {
    pub category: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerTotalCache {
    pub count: i64,
    /// Unix seconds when the count was fetched from the provider.
    pub fetched_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDashboard {
    pub account: Account,
    pub sync: SyncStatus,
    /// MIN(timestamp) of locally synced emails — i.e. how far back we have data.
    /// Falls back to `account.sync_from_timestamp` when no emails are stored yet.
    pub synced_since: Option<i64>,
    /// COUNT(*) of locally stored, non-deleted emails for this account.
    pub synced_count: i64,
    /// Cached server-side total (from Gmail/IMAP/Outlook). `None` until the
    /// user clicks "Refresh total" at least once.
    pub server_total: Option<i64>,
    pub server_total_fetched_at: Option<i64>,
    pub category_counts: Vec<CategoryCount>,
    /// COUNT(*) of locally stored, non-deleted emails in mailbox='sent' for
    /// this account. Surfaced as a stat next to the local/server total so the
    /// user can see how much of their outbound history is indexed locally.
    pub sent_count: i64,
    /// Number of distinct emails with at least one `intent` tag.
    pub classified_count: i64,
    /// How many emails are *eligible* for classification given the user's
    /// configured categories (`ClassificationConfig.categories`). Used as the
    /// denominator for the "Classified" progress bar so % stays meaningful
    /// when the pipeline is restricted to e.g. "primary" only.
    pub classified_eligible: i64,
    /// Emails where the memory fact extractor has run.
    pub memory_analyzed_count: i64,
    /// Eligible-for-memory denominator: applies `MemoryConfig.categories` and
    /// (when `extract_from_self_only`) restricts to mailbox='sent'.
    pub memory_eligible: i64,
    /// Emails where task extraction has run.
    pub task_analyzed_count: i64,
    /// Eligible-for-task denominator: applies task categories/window and the
    /// task self-only rule.
    pub task_eligible: i64,
    /// Distinct emails for this account that have at least one row in
    /// `embedding_chunks`. Drives the "Embeddings" progress bar in the UI.
    pub embedded_count: i64,
    /// Eligible-for-embedding denominator: applies `EmbeddingsConfig.categories`.
    pub embedded_eligible: i64,
}

/// Build the cache key used to persist a per-account server total in
/// `user_preferences`. Co-located with the cache reader so callers don't
/// hard-code the format.
pub fn server_total_pref_key(account_id: &str) -> String {
    format!("dashboard.server_total.{}", account_id)
}

pub fn read_server_total_cache(db: &Arc<Database>, account_id: &str) -> Option<ServerTotalCache> {
    let raw = db.get_preference(&server_total_pref_key(account_id)).ok()??;
    serde_json::from_str::<ServerTotalCache>(&raw).ok()
}

pub fn write_server_total_cache(db: &Arc<Database>, account_id: &str, cache: &ServerTotalCache) -> Result<()> {
    let json = serde_json::to_string(cache)?;
    db.set_preference(&server_total_pref_key(account_id), &json)
}

/// Compute per-account stats for every account in the DB. One read connection
/// is held for the duration; with a pool of 4 readers the dashboard never
/// starves the inbox query.
pub fn collect_dashboards(db: &Arc<Database>) -> Result<Vec<AccountDashboard>> {
    let accounts = db.list_accounts()?;
    let mut out = Vec::with_capacity(accounts.len());
    for account in accounts {
        out.push(collect_one(db, account)?);
    }
    Ok(out)
}

/// Build a `COUNT(*)` query restricted to `account_id`, non-deleted emails,
/// optionally a category list, and optionally "self-authored" (matches the
/// memory extractor's `extract_from_self_only` rule: `sender_email == account
/// email`). Empty categories means no category filter.
///
/// `account_email` is only consulted when `self_only` is true; pass an empty
/// string otherwise.
fn count_eligible(
    conn: &rusqlite::Connection,
    account_id: &str,
    account_email: &str,
    categories: &[String],
    self_only: bool,
) -> Result<i64> {
    count_eligible_since(conn, account_id, account_email, categories, self_only, None)
}

fn count_eligible_since(
    conn: &rusqlite::Connection,
    account_id: &str,
    account_email: &str,
    categories: &[String],
    self_only: bool,
    min_timestamp: Option<i64>,
) -> Result<i64> {
    let mut sql = String::from("SELECT COUNT(*) FROM emails WHERE account_id = ?1 AND is_deleted = 0");
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(account_id.to_string())];
    if !categories.is_empty() {
        let placeholders: Vec<String> = (0..categories.len())
            .map(|i| format!("?{}", params_vec.len() + i + 1))
            .collect();
        sql.push_str(&format!(" AND category IN ({})", placeholders.join(",")));
        for c in categories {
            params_vec.push(Box::new(c.clone()));
        }
    }
    if self_only {
        // Mirror dashboard's sent_count detection: trust either the mailbox
        // column (IMAP/Outlook sent-folder pass) or sender match (Gmail, where
        // sent rows live under mailbox='inbox' due to the combined query).
        let placeholder = format!("?{}", params_vec.len() + 1);
        sql.push_str(&format!(
            " AND (mailbox = 'sent' OR LOWER(sender_email) = LOWER({}))",
            placeholder
        ));
        params_vec.push(Box::new(account_email.to_string()));
    }
    if let Some(ts) = min_timestamp {
        let placeholder = format!("?{}", params_vec.len() + 1);
        sql.push_str(&format!(" AND timestamp >= {}", placeholder));
        params_vec.push(Box::new(ts));
    }
    let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let count: i64 = conn.query_row(&sql, params_refs.as_slice(), |row| row.get(0))?;
    Ok(count)
}

fn collect_one(db: &Arc<Database>, account: Account) -> Result<AccountDashboard> {
    let sync = db.get_sync_status(&account.id)?;

    // Read the three pipeline configs once so the eligibility denominators
    // match what the corresponding pipelines actually process. Failures are
    // propagated — we'd rather show "loading" than mismatched numbers.
    let cls_cfg = classification::get_config(db)?;
    let mem_cfg = memory_config::get_config(db)?;
    let task_cfg = task_config::get_config(db)?;
    let emb_cfg = embeddings::get_embeddings_config(db, &account.id)?;

    let synced_count: i64;
    let synced_since: Option<i64>;
    let category_counts: Vec<CategoryCount>;
    let sent_count: i64;
    let classified_count: i64;
    let classified_eligible: i64;
    let memory_analyzed_count: i64;
    let memory_eligible: i64;
    let task_analyzed_count: i64;
    let task_eligible: i64;
    let embedded_count: i64;
    let embedded_eligible: i64;

    {
        let conn = db.reader();

        synced_count = conn.query_row(
            "SELECT COUNT(*) FROM emails WHERE account_id = ?1 AND is_deleted = 0",
            params![account.id],
            |row| row.get(0),
        )?;

        // Sent detection has to look at *who sent the email*, not the `mailbox`
        // column. The Gmail inbox sync pulls `category:primary OR in:sent` into
        // a single pass and stores everything with `mailbox='inbox'`; the
        // separate sent-folder pass then skips those rows as duplicates. So
        // `mailbox='sent'` is only reliably set for IMAP / Outlook accounts.
        // Matching on `sender_email = account.email` is provider-agnostic and
        // catches mail the user actually authored regardless of folder.
        sent_count = conn.query_row(
            "SELECT COUNT(*) FROM emails
             WHERE account_id = ?1
               AND is_deleted = 0
               AND (mailbox = 'sent' OR LOWER(sender_email) = LOWER(?2))",
            params![account.id, account.email],
            |row| row.get(0),
        )?;

        synced_since = match conn.query_row(
            "SELECT MIN(timestamp) FROM emails WHERE account_id = ?1 AND is_deleted = 0",
            params![account.id],
            |row| row.get::<_, Option<i64>>(0),
        ) {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(e.into()),
        };

        let mut stmt = conn.prepare(
            "SELECT category, COUNT(*) FROM emails
             WHERE account_id = ?1 AND is_deleted = 0
             GROUP BY category
             ORDER BY category",
        )?;
        let rows = stmt.query_map(params![account.id], |row| {
            Ok(CategoryCount {
                category: row.get(0)?,
                count: row.get(1)?,
            })
        })?;
        let mut cats = Vec::new();
        for r in rows {
            cats.push(r?);
        }
        category_counts = cats;

        // Distinct emails with an intent tag, scoped to this account. The
        // emails join is required because email_tags is account-agnostic;
        // the `idx_email_tags_email` index keeps it cheap.
        classified_count = conn.query_row(
            "SELECT COUNT(DISTINCT et.email_id)
             FROM email_tags et
             JOIN emails e ON e.id = et.email_id
             WHERE et.tag_type = 'intent'
               AND e.account_id = ?1
               AND e.is_deleted = 0",
            params![account.id],
            |row| row.get(0),
        )?;

        memory_analyzed_count = conn.query_row(
            "SELECT COUNT(*) FROM email_extraction_status s
             JOIN emails e ON e.id = s.email_id
             WHERE e.account_id = ?1
               AND e.is_deleted = 0
               AND s.pipeline = 'memory_facts'",
            params![account.id],
            |row| row.get(0),
        )?;

        task_analyzed_count = conn.query_row(
            "SELECT COUNT(*) FROM email_extraction_status s
             JOIN emails e ON e.id = s.email_id
             WHERE e.account_id = ?1
               AND e.is_deleted = 0
               AND s.pipeline = 'tasks'",
            params![account.id],
            |row| row.get(0),
        )?;

        // Distinct emails with ≥1 chunk in `embedding_chunks`. Joining via
        // `email_id` is cheap because the unique index on (email_id, chunk_index)
        // covers the lookup; the WHERE on `account_id` is satisfied by
        // `idx_emails_account_active`.
        embedded_count = conn.query_row(
            "SELECT COUNT(DISTINCT ec.email_id)
             FROM embedding_chunks ec
             JOIN emails e ON e.id = ec.email_id
             WHERE e.account_id = ?1
               AND e.is_deleted = 0",
            params![account.id],
            |row| row.get(0),
        )?;

        // Eligible denominators — what each pipeline would actually consider
        // given the current user config. Matching the same WHERE clauses the
        // pipelines use ensures the percentages converge to 100 %.
        classified_eligible = count_eligible(&conn, &account.id, &account.email, &cls_cfg.categories, false)?;
        memory_eligible = count_eligible(
            &conn,
            &account.id,
            &account.email,
            &mem_cfg.categories,
            mem_cfg.extract_from_self_only,
        )?;
        task_eligible = count_eligible_since(
            &conn,
            &account.id,
            &account.email,
            &task_cfg.categories,
            task_cfg.extract_from_self_only,
            task_cfg.backfill_min_timestamp(chrono::Utc::now().timestamp()),
        )?;
        embedded_eligible = count_eligible(&conn, &account.id, &account.email, &emb_cfg.categories, false)?;
    }

    let synced_since = synced_since.or(account.sync_from_timestamp);

    let cache = read_server_total_cache(db, &account.id);
    let (server_total, server_total_fetched_at) = match cache {
        Some(c) => (Some(c.count), Some(c.fetched_at)),
        None => (None, None),
    };

    Ok(AccountDashboard {
        account,
        sync,
        synced_since,
        synced_count,
        server_total,
        server_total_fetched_at,
        category_counts,
        sent_count,
        classified_count,
        classified_eligible,
        memory_analyzed_count,
        memory_eligible,
        task_analyzed_count,
        task_eligible,
        embedded_count,
        embedded_eligible,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Account;

    fn seed_account(db: &Database, id: &str, provider: &str) -> Account {
        let acc = Account {
            id: id.to_string(),
            provider: provider.to_string(),
            email: format!("{id}@example.com"),
            name: id.to_string(),
            created_at: 1_700_000_000,
            sort_order: 0,
            enabled: true,
            sync_from_timestamp: Some(1_700_000_000),
        };
        db.insert_account(&acc).unwrap();
        acc
    }

    fn insert_email(
        conn: &rusqlite::Connection,
        id: &str,
        account_id: &str,
        category: &str,
        timestamp: i64,
        extracted_at: Option<i64>,
    ) {
        insert_email_full(
            conn,
            id,
            account_id,
            category,
            timestamp,
            extracted_at,
            "inbox",
            "sender@example.com",
        );
    }

    fn insert_email_full(
        conn: &rusqlite::Connection,
        id: &str,
        account_id: &str,
        category: &str,
        timestamp: i64,
        extracted_at: Option<i64>,
        mailbox: &str,
        sender_email: &str,
    ) {
        conn.execute(
            "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
                 VALUES (?1, 'gmail', ?1, 'Test', 0)",
            rusqlite::params![account_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO emails (id, account_id, thread_id, subject, sender, sender_email,
                recipients_json, snippet, timestamp, is_deleted, category, mailbox,
                created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, '[]', '', ?7, 0, ?8, ?9, ?7)",
            rusqlite::params![
                id,
                account_id,
                format!("thr-{id}"),
                "subject",
                "Sender",
                sender_email,
                timestamp,
                category,
                mailbox,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO email_bodies (email_id, body) VALUES (?1, '')",
            rusqlite::params![id],
        )
        .unwrap();
        if let Some(at) = extracted_at {
            conn.execute(
                "INSERT INTO email_extraction_status (email_id, pipeline, extracted_at)
                 VALUES (?1, 'memory_facts', ?2), (?1, 'tasks', ?2)",
                rusqlite::params![id, at],
            )
            .unwrap();
        }
    }

    fn tag_intent(conn: &rusqlite::Connection, email_id: &str) {
        conn.execute(
            "INSERT INTO email_tags (email_id, tag_type, tag_value, created_at)
             VALUES (?1, 'intent', 'reply_needed', 1)",
            rusqlite::params![email_id],
        )
        .unwrap();
    }

    fn add_embedding_chunk(conn: &rusqlite::Connection, email_id: &str, chunk_index: i32) {
        conn.execute(
            "INSERT INTO embedding_chunks
                (email_id, chunk_index, embedding_model, content_hash, created_at)
             VALUES (?1, ?2, 'test-model', 'h', 1)",
            rusqlite::params![email_id, chunk_index],
        )
        .unwrap();
    }

    #[test]
    fn collects_counts_for_account() {
        let db = Database::new_for_testing().unwrap();
        let _acc = seed_account(&db, "acc1", "gmail");

        {
            let conn = db.connection();
            insert_email(&conn, "e1", "acc1", "primary", 1_700_001_000, Some(1_700_001_500));
            insert_email(&conn, "e2", "acc1", "primary", 1_700_002_000, None);
            insert_email(&conn, "e3", "acc1", "social", 1_700_003_000, Some(1_700_003_500));
            insert_email(&conn, "e4", "acc1", "updates", 1_700_004_000, None);
            tag_intent(&conn, "e1");
            tag_intent(&conn, "e2");
            // e3 / e4 are not classified

            // Embeddings: e1 has two chunks, e3 has one. e2 / e4 have none.
            // Distinct count should therefore be 2 (e1, e3).
            add_embedding_chunk(&conn, "e1", 0);
            add_embedding_chunk(&conn, "e1", 1);
            add_embedding_chunk(&conn, "e3", 0);
        }

        let db_arc = Arc::new(db);
        let dashboards = collect_dashboards(&db_arc).unwrap();
        assert_eq!(dashboards.len(), 1);
        let d = &dashboards[0];

        assert_eq!(d.account.id, "acc1");
        assert_eq!(d.synced_count, 4);
        assert_eq!(d.sent_count, 0, "no test emails are in sent mailbox");
        assert_eq!(d.synced_since, Some(1_700_001_000));
        assert_eq!(d.classified_count, 2);
        // Default classification config = ["primary"] → eligible = e1 + e2.
        assert_eq!(d.classified_eligible, 2);
        assert_eq!(d.memory_analyzed_count, 2);
        // Default memory config = ["primary"] AND extract_from_self_only=true →
        // eligible requires mailbox='sent'. None of these are.
        assert_eq!(d.memory_eligible, 0);
        assert_eq!(d.task_analyzed_count, 2);
        assert_eq!(d.task_eligible, 0);
        assert_eq!(d.embedded_count, 2, "e1 + e3 have chunks; chunk count != email count");
        // Default embeddings config = ["primary"] → eligible = e1 + e2.
        assert_eq!(d.embedded_eligible, 2);

        let cats: std::collections::HashMap<_, _> = d
            .category_counts
            .iter()
            .map(|c| (c.category.as_str(), c.count))
            .collect();
        assert_eq!(cats.get("primary"), Some(&2));
        assert_eq!(cats.get("social"), Some(&1));
        assert_eq!(cats.get("updates"), Some(&1));

        assert_eq!(d.server_total, None);
        assert_eq!(d.server_total_fetched_at, None);
    }

    #[test]
    fn uses_sync_from_timestamp_when_no_emails() {
        let db = Database::new_for_testing().unwrap();
        let _acc = seed_account(&db, "empty", "imap");
        let db_arc = Arc::new(db);
        let dashboards = collect_dashboards(&db_arc).unwrap();
        assert_eq!(dashboards.len(), 1);
        let d = &dashboards[0];
        assert_eq!(d.synced_count, 0);
        assert_eq!(d.synced_since, Some(1_700_000_000));
        assert!(d.category_counts.is_empty());
    }

    #[test]
    fn server_total_cache_roundtrip() {
        let db = Database::new_for_testing().unwrap();
        let _acc = seed_account(&db, "acc1", "gmail");
        let db_arc = Arc::new(db);
        let cache = ServerTotalCache {
            count: 12_345,
            fetched_at: 1_700_010_000,
        };
        write_server_total_cache(&db_arc, "acc1", &cache).unwrap();

        let dashboards = collect_dashboards(&db_arc).unwrap();
        let d = &dashboards[0];
        assert_eq!(d.server_total, Some(12_345));
        assert_eq!(d.server_total_fetched_at, Some(1_700_010_000));
    }

    #[test]
    fn sent_count_and_memory_eligibility_with_self_only() {
        let db = Database::new_for_testing().unwrap();
        let _acc = seed_account(&db, "acc1", "gmail");

        {
            let conn = db.connection();
            // The seeded account's email is "acc1@example.com" (per seed_account).
            // s1 is mailbox='sent' but external sender — counts via mailbox match.
            // s2 is mailbox='inbox' but sender == account email (Gmail-style row).
            // s2 verifies the sender_email fallback path.
            insert_email_full(
                &conn,
                "s1",
                "acc1",
                "primary",
                1_700_001_000,
                None,
                "sent",
                "external@other.com",
            );
            insert_email_full(
                &conn,
                "s2",
                "acc1",
                "primary",
                1_700_002_000,
                None,
                "inbox",
                "acc1@example.com",
            );
            insert_email_full(
                &conn,
                "i1",
                "acc1",
                "primary",
                1_700_003_000,
                None,
                "inbox",
                "external@other.com",
            );
            insert_email_full(
                &conn,
                "i2",
                "acc1",
                "social",
                1_700_004_000,
                None,
                "inbox",
                "external@other.com",
            );
        }
        let db_arc = Arc::new(db);
        let d = &collect_dashboards(&db_arc).unwrap()[0];

        // Both mailbox='sent' (s1) and sender_email match (s2) count as sent.
        assert_eq!(d.sent_count, 2);
        // classification = primary (default) → s1, s2, i1.
        assert_eq!(d.classified_eligible, 3);
        // memory = primary + extract_from_self_only → s1 (mailbox='sent') + s2
        // (sender match) — both are primary.
        assert_eq!(d.memory_eligible, 2);
        // embeddings = primary (default) → s1, s2, i1.
        assert_eq!(d.embedded_eligible, 3);
    }

    #[test]
    fn excludes_deleted_emails() {
        let db = Database::new_for_testing().unwrap();
        let _acc = seed_account(&db, "acc1", "gmail");

        {
            let conn = db.connection();
            insert_email(&conn, "k1", "acc1", "primary", 1_700_001_000, None);
            // Soft-deleted email should not be counted
            conn.execute(
                "INSERT INTO emails (id, account_id, thread_id, subject, sender, sender_email,
                    recipients_json, snippet, timestamp, is_deleted, category, mailbox,
                    created_at)
                 VALUES ('d1', 'acc1', 'thr-d1', 's', 'S', 's@e.com', '[]', '', 1, 1,
                    'primary', 'inbox', 1)",
                [],
            )
            .unwrap();
            conn.execute("INSERT INTO email_bodies (email_id, body) VALUES ('d1', '')", [])
                .unwrap();
        }
        let db_arc = Arc::new(db);
        let d = &collect_dashboards(&db_arc).unwrap()[0];
        assert_eq!(d.synced_count, 1);
    }
}
