use crate::db::Database;
use crate::models::error::Result;
use crate::models::{ClassificationRule, EmailTag};
use rusqlite::params;

impl Database {
    /// Upsert the same `tag_type` on many emails in one transaction.
    /// Used by the company-tag backfill and sync hook; generic enough that
    /// any future single-type tagger (e.g. language detection) can reuse it.
    pub fn upsert_email_tags_batch(
        &self,
        tag_type: &str,
        pairs: &[(String, String)], // (email_id, tag_value)
    ) -> Result<()> {
        if pairs.is_empty() {
            return Ok(());
        }
        let mut conn = self.connection();
        let now = chrono::Utc::now().timestamp();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO email_tags (email_id, tag_type, tag_value, confidence, created_at)
                 VALUES (?1, ?2, ?3, NULL, ?4)",
            )?;
            for (email_id, tag_value) in pairs {
                stmt.execute(params![email_id, tag_type, tag_value, now])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Upsert a single tag for an email (one value per type).
    pub fn upsert_email_tag(
        &self,
        email_id: &str,
        tag_type: &str,
        tag_value: &str,
        confidence: Option<f64>,
    ) -> Result<()> {
        let conn = self.connection();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR REPLACE INTO email_tags (email_id, tag_type, tag_value, confidence, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![email_id, tag_type, tag_value, confidence, now],
        )?;
        Ok(())
    }

    /// Set all classification tags for an email at once.
    pub fn set_email_classification(
        &self,
        email_id: &str,
        priority: &str,
        intent: &str,
        topic: &str,
        confidence: Option<f64>,
    ) -> Result<()> {
        let mut conn = self.connection();
        let now = chrono::Utc::now().timestamp();
        let tx = conn.transaction()?;
        for (tag_type, tag_value) in [("priority", priority), ("intent", intent), ("topic", topic)] {
            tx.execute(
                "INSERT OR REPLACE INTO email_tags (email_id, tag_type, tag_value, confidence, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![email_id, tag_type, tag_value, confidence, now],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Batch-set classification tags for multiple emails in a single transaction.
    pub fn set_email_classifications_batch(
        &self,
        classifications: &[(String, String, String, String, Option<f64>)], // (email_id, priority, intent, topic, confidence)
    ) -> Result<()> {
        if classifications.is_empty() {
            return Ok(());
        }
        let mut conn = self.connection();
        let now = chrono::Utc::now().timestamp();
        let tx = conn.transaction()?;
        let mut stmt = tx.prepare(
            "INSERT OR REPLACE INTO email_tags (email_id, tag_type, tag_value, confidence, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (email_id, priority, intent, topic, confidence) in classifications {
            for (tag_type, tag_value) in [
                ("priority", priority.as_str()),
                ("intent", intent.as_str()),
                ("topic", topic.as_str()),
            ] {
                stmt.execute(params![email_id, tag_type, tag_value, confidence, now])?;
            }
        }
        drop(stmt);
        tx.commit()?;
        Ok(())
    }

    /// Get all tags for a single email.
    pub fn get_email_tags(&self, email_id: &str) -> Result<Vec<EmailTag>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT email_id, tag_type, tag_value, confidence, created_at
             FROM email_tags WHERE email_id = ?1",
        )?;
        let tags = stmt
            .query_map(params![email_id], |row| {
                Ok(EmailTag {
                    email_id: row.get(0)?,
                    tag_type: row.get(1)?,
                    tag_value: row.get(2)?,
                    confidence: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tags)
    }

    /// Get tags for multiple emails as a flat list (batch load for email lists).
    pub fn get_email_tags_batch(&self, email_ids: &[String]) -> Result<Vec<EmailTag>> {
        if email_ids.is_empty() {
            return Ok(vec![]);
        }
        let conn = self.reader();
        let placeholders: Vec<String> = (1..=email_ids.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT email_id, tag_type, tag_value, confidence, created_at
             FROM email_tags WHERE email_id IN ({})",
            placeholders.join(",")
        );
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> = email_ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
        let tags = stmt
            .query_map(params.as_slice(), |row| {
                Ok(EmailTag {
                    email_id: row.get(0)?,
                    tag_type: row.get(1)?,
                    tag_value: row.get(2)?,
                    confidence: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tags)
    }

    /// Get email IDs that don't have classification tags yet.
    /// When `categories` is non-empty, only emails whose `category` is in the list are returned.
    /// When `min_timestamp` is `Some`, emails older than the cutoff are excluded
    /// (typically from `Database::ai_processing_min_timestamp`).
    pub fn get_unclassified_email_ids(
        &self,
        account_id: &str,
        limit: i32,
        categories: &[String],
        min_timestamp: Option<i64>,
    ) -> Result<Vec<String>> {
        use rusqlite::types::ToSql;
        let mut bound: Vec<Box<dyn ToSql>> = vec![Box::new(account_id.to_string()), Box::new(limit)];
        let cat_filter = if categories.is_empty() {
            String::new()
        } else {
            let start = bound.len() + 1;
            let phs: Vec<String> = (start..start + categories.len()).map(|i| format!("?{i}")).collect();
            for cat in categories {
                bound.push(Box::new(cat.clone()));
            }
            format!(" AND e.category IN ({})", phs.join(", "))
        };
        let ts_filter = if let Some(ts) = min_timestamp {
            bound.push(Box::new(ts));
            format!(" AND e.timestamp >= ?{}", bound.len())
        } else {
            String::new()
        };
        let sql = format!(
            "SELECT e.id FROM emails e
             WHERE e.account_id = ?1
               AND NOT EXISTS (
                   SELECT 1 FROM email_tags t WHERE t.email_id = e.id AND t.tag_type = 'intent'
               )
               AND LENGTH(e.snippet) > 20{cat_filter}{ts_filter}
             ORDER BY e.timestamp DESC
             LIMIT ?2"
        );
        let conn = self.reader();
        let mut stmt = conn.prepare(&sql)?;
        let refs: Vec<&dyn ToSql> = bound.iter().map(|p| p.as_ref()).collect();
        let ids = stmt
            .query_map(refs.as_slice(), |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    }

    /// Count unclassified emails for an account.
    pub fn count_unclassified_emails(&self, account_id: &str) -> Result<i32> {
        let conn = self.reader();
        conn.query_row(
            "SELECT COUNT(*) FROM emails e
             WHERE e.account_id = ?1
               AND NOT EXISTS (
                   SELECT 1 FROM email_tags t WHERE t.email_id = e.id AND t.tag_type = 'intent'
               )
               AND LENGTH(e.snippet) > 20",
            params![account_id],
            |row| row.get(0),
        )
        .map_err(Into::into)
    }

    /// Get tag value distribution for smart filter stats.
    ///
    /// The count must match what the filtered list view will actually show.
    /// `db::emails::search::get_filtered_emails` (tag branch) shows a thread when
    /// ANY email in the thread carries the tag, displaying the thread's latest
    /// email as the representative. We count the same set: distinct threads with
    /// at least one tagged, non-deleted, inbox/sent email.
    ///
    /// Under `AllEnabled`, threads dedup per `(account_id, thread_id)` — thread
    /// ids are not globally unique across accounts.
    pub fn get_tag_stats(
        &self,
        scope: crate::db::AccountScope<'_>,
        tag_type: &str,
        limit: i32,
    ) -> Result<Vec<(String, i32)>> {
        let conn = self.reader();
        let (scope_cond, account_param): (&str, Option<&str>) = match scope {
            crate::db::AccountScope::Account(id) => ("e.account_id = ?3", Some(id)),
            crate::db::AccountScope::AllEnabled => {
                ("e.account_id IN (SELECT id FROM accounts WHERE enabled = 1)", None)
            }
        };
        let sql = format!(
            "SELECT tag_value, COUNT(*) AS cnt FROM (
                 SELECT DISTINCT t.tag_value AS tag_value, e.account_id, e.thread_id
                 FROM email_tags t
                 INNER JOIN emails e ON e.id = t.email_id
                 WHERE {scope_cond}
                   AND t.tag_type = ?1
                   AND e.is_deleted = 0
                   AND e.mailbox IN ('inbox', 'sent')
             )
             GROUP BY tag_value
             ORDER BY cnt DESC
             LIMIT ?2"
        );
        let mut stmt = conn.prepare(&sql)?;
        let map_row = |row: &rusqlite::Row| Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?));
        let stats = match account_param {
            Some(id) => stmt
                .query_map(params![tag_type, limit, id], map_row)?
                .filter_map(|r| r.ok())
                .collect(),
            None => stmt
                .query_map(params![tag_type, limit], map_row)?
                .filter_map(|r| r.ok())
                .collect(),
        };
        Ok(stats)
    }

    // -- Classification rules CRUD --

    pub fn insert_classification_rule(&self, rule: &ClassificationRule) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO classification_rules (id, account_id, name, sender_pattern, subject_pattern, priority, intent, topic, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                rule.id, rule.account_id, rule.name,
                rule.sender_pattern, rule.subject_pattern,
                rule.priority, rule.intent, rule.topic,
                rule.enabled as i32, rule.created_at, rule.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_classification_rule(&self, rule: &ClassificationRule) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE classification_rules SET name=?1, sender_pattern=?2, subject_pattern=?3, priority=?4, intent=?5, topic=?6, enabled=?7, updated_at=?8
             WHERE id=?9 AND account_id=?10",
            params![
                rule.name, rule.sender_pattern, rule.subject_pattern,
                rule.priority, rule.intent, rule.topic,
                rule.enabled as i32, rule.updated_at,
                rule.id, rule.account_id,
            ],
        )?;
        Ok(())
    }

    pub fn delete_classification_rule(&self, rule_id: &str, account_id: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM classification_rules WHERE id=?1 AND account_id=?2",
            params![rule_id, account_id],
        )?;
        Ok(())
    }

    pub fn get_classification_rules(&self, account_id: &str) -> Result<Vec<ClassificationRule>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, name, sender_pattern, subject_pattern, priority, intent, topic, enabled, created_at, updated_at
             FROM classification_rules WHERE account_id=?1 ORDER BY created_at ASC",
        )?;
        let rules = stmt
            .query_map(params![account_id], row_to_classification_rule)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rules)
    }

    pub fn get_enabled_classification_rules(&self, account_id: &str) -> Result<Vec<ClassificationRule>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, name, sender_pattern, subject_pattern, priority, intent, topic, enabled, created_at, updated_at
             FROM classification_rules WHERE account_id=?1 AND enabled=1 ORDER BY created_at ASC",
        )?;
        let rules = stmt
            .query_map(params![account_id], row_to_classification_rule)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rules)
    }

    pub fn count_classification_rules(&self, account_id: &str) -> Result<i32> {
        let conn = self.reader();
        conn.query_row(
            "SELECT COUNT(*) FROM classification_rules WHERE account_id=?1",
            params![account_id],
            |row| row.get(0),
        )
        .map_err(Into::into)
    }
}

fn row_to_classification_rule(row: &rusqlite::Row) -> rusqlite::Result<ClassificationRule> {
    Ok(ClassificationRule {
        id: row.get(0)?,
        account_id: row.get(1)?,
        name: row.get(2)?,
        sender_pattern: row.get(3)?,
        subject_pattern: row.get(4)?,
        priority: row.get(5)?,
        intent: row.get(6)?,
        topic: row.get(7)?,
        enabled: row.get::<_, i32>(8)? != 0,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    // Minimal local helpers. The richer helpers in db::emails::test_helpers are
    // pub(super) to the emails module, so we replicate the small surface needed
    // here. Keep in sync with the emails table schema.
    fn ensure_account(db: &Database, account_id: &str) {
        db.connection()
            .execute(
                "INSERT OR IGNORE INTO accounts (id, provider, email, name, created_at)
                 VALUES (?1, 'gmail', ?1, 'Test', 0)",
                params![account_id],
            )
            .unwrap();
    }

    fn insert_email(db: &Database, id: &str, account_id: &str, thread_id: &str, timestamp: i64) {
        ensure_account(db, account_id);
        db.connection()
            .execute(
                "INSERT INTO emails
                     (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                      recipients_json, cc_json, snippet, timestamp, is_read, category, created_at)
                     VALUES (?1,?2,?3,'subj','sender','s@s.com','s.com','[]','[]','snip',?4,0,'primary',0)",
                params![id, account_id, thread_id, timestamp],
            )
            .unwrap();
    }

    fn insert_email_with_mailbox(
        db: &Database,
        id: &str,
        account_id: &str,
        thread_id: &str,
        timestamp: i64,
        mailbox: &str,
        is_deleted: i32,
    ) {
        ensure_account(db, account_id);
        db.connection()
            .execute(
                "INSERT INTO emails
                     (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                      recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox, is_deleted, created_at)
                     VALUES (?1,?2,?3,'subj','sender','s@s.com','s.com','[]','[]','snip',?4,0,'primary',?5,?6,0)",
                params![id, account_id, thread_id, timestamp, mailbox, is_deleted],
            )
            .unwrap();
    }

    fn tag_email(db: &Database, email_id: &str, tag_type: &str, tag_value: &str) {
        db.connection()
            .execute(
                "INSERT OR REPLACE INTO email_tags (email_id, tag_type, tag_value, confidence, created_at)
                 VALUES (?1, ?2, ?3, NULL, 0)",
                params![email_id, tag_type, tag_value],
            )
            .unwrap();
    }

    fn stat_for(stats: &[(String, i32)], value: &str) -> Option<i32> {
        stats.iter().find(|(v, _)| v == value).map(|(_, c)| *c)
    }

    // Regression: sidebar count must match the list query semantics.
    // A thread counts once if ANY email in it has the tag — even when the user
    // replied and the thread representative is no longer the tagged inbound.
    #[test]
    fn get_tag_stats_counts_threads_not_email_rows() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        // Thread A: 3 emails from Globex (tagged) + 1 user reply (not tagged).
        // Old buggy behavior: COUNT(*) on email_tags rows → 3.
        // Correct behavior: 1 thread.
        insert_email(&db, "a1", account, "thread-a", 100);
        insert_email(&db, "a2", account, "thread-a", 200);
        insert_email(&db, "a3", account, "thread-a", 300);
        insert_email(&db, "a4-reply", account, "thread-a", 400); // user reply, latest
        tag_email(&db, "a1", "company", "globex");
        tag_email(&db, "a2", "company", "globex");
        tag_email(&db, "a3", "company", "globex");

        // Thread B: single tagged email.
        insert_email(&db, "b1", account, "thread-b", 500);
        tag_email(&db, "b1", "company", "globex");

        // Different company in another thread to make sure GROUP BY works.
        insert_email(&db, "c1", account, "thread-c", 600);
        tag_email(&db, "c1", "company", "acme");

        let stats = db
            .get_tag_stats(crate::db::AccountScope::Account(account), "company", 15)
            .unwrap();

        assert_eq!(
            stat_for(&stats, "globex"),
            Some(2),
            "globex should count 2 distinct threads, got {:?}",
            stats
        );
        assert_eq!(
            stat_for(&stats, "acme"),
            Some(1),
            "acme should count 1 thread, got {:?}",
            stats
        );
    }

    #[test]
    fn get_tag_stats_excludes_soft_deleted() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_email_with_mailbox(&db, "live", account, "thread-live", 100, "inbox", 0);
        insert_email_with_mailbox(&db, "deleted", account, "thread-deleted", 200, "inbox", 1);
        tag_email(&db, "live", "company", "globex");
        tag_email(&db, "deleted", "company", "globex");

        let stats = db
            .get_tag_stats(crate::db::AccountScope::Account(account), "company", 15)
            .unwrap();
        assert_eq!(
            stat_for(&stats, "globex"),
            Some(1),
            "soft-deleted tagged email must not be counted, got {:?}",
            stats
        );
    }

    #[test]
    fn get_tag_stats_excludes_spam_and_trash() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_email_with_mailbox(&db, "ok", account, "thread-ok", 100, "inbox", 0);
        insert_email_with_mailbox(&db, "spam", account, "thread-spam", 200, "spam", 0);
        insert_email_with_mailbox(&db, "trash", account, "thread-trash", 300, "trash", 0);
        insert_email_with_mailbox(&db, "sent", account, "thread-sent", 400, "sent", 0);
        tag_email(&db, "ok", "company", "globex");
        tag_email(&db, "spam", "company", "globex");
        tag_email(&db, "trash", "company", "globex");
        tag_email(&db, "sent", "company", "globex");

        let stats = db
            .get_tag_stats(crate::db::AccountScope::Account(account), "company", 15)
            .unwrap();
        assert_eq!(
            stat_for(&stats, "globex"),
            Some(2),
            "only inbox + sent should count, got {:?}",
            stats
        );
    }

    // Sidebar count must equal the list query result count for the same tag.
    // If these ever drift apart, users see a number in the sidebar that doesn't
    // match what shows up — exactly the original Globex bug.
    #[test]
    fn get_tag_stats_matches_get_filtered_emails_count() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        // Scenarios mixed: replied threads, unreplied, single-email, deleted, spam.
        insert_email(&db, "t1-a", account, "thread-1", 100);
        insert_email(&db, "t1-reply", account, "thread-1", 200);
        tag_email(&db, "t1-a", "intent", "request");

        insert_email(&db, "t2-only", account, "thread-2", 300);
        tag_email(&db, "t2-only", "intent", "request");

        insert_email(&db, "t3-old", account, "thread-3", 400);
        insert_email(&db, "t3-new", account, "thread-3", 500);
        tag_email(&db, "t3-new", "intent", "request"); // latest is the tagged one

        insert_email_with_mailbox(&db, "t4-spam", account, "thread-4", 600, "spam", 0);
        tag_email(&db, "t4-spam", "intent", "request");

        let stats = db
            .get_tag_stats(crate::db::AccountScope::Account(account), "intent", 15)
            .unwrap();
        let sidebar_count = stat_for(&stats, "request").unwrap_or(0);

        let list = db
            .get_filtered_emails(
                crate::db::AccountScope::Account(account),
                None,
                None,
                Some("intent"),
                Some("request"),
                None,
                100,
                0,
            )
            .unwrap();

        assert_eq!(
            sidebar_count as usize,
            list.emails.len(),
            "sidebar count ({}) must match list length ({}) for the same tag",
            sidebar_count,
            list.emails.len()
        );
        assert_eq!(
            sidebar_count, 3,
            "expected 3 matching threads (1,2,3 — not the spam one), got {}",
            sidebar_count
        );
    }
}
