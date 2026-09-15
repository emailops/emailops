use crate::db::Database;
use crate::models::error::Result;
use crate::models::{ClassificationRule, EmailTag};
use rusqlite::params;

/// Days after which a single message's contribution to the engagement shares
/// halves. Longer than the block-level recency half-life: this re-balances a
/// tag's own history, it doesn't decide whether the tag is still live.
pub const INTERACTION_HALF_LIFE_DAYS: f64 = 180.0;

/// One (account, tag value) row of the tag board's stats query, with the two
/// engagement signals aggregated over the same scan as the thread count.
#[derive(Debug, Clone)]
pub struct TagBoardRow {
    pub account_id: String,
    pub tag_value: String,
    pub count: i32,
    pub sent_share: f64,
    pub read_share: f64,
    /// Newest message carrying this tag — drives the recency decay.
    pub last_activity_at: Option<i64>,
}

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

    /// Remove one tag type from an email.
    ///
    /// Needed by junk scoring: when a re-score clears a message the derived
    /// `junk` chip has to go with it, or a stale badge outlives the verdict
    /// behind it.
    pub fn delete_email_tag(&self, email_id: &str, tag_type: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM email_tags WHERE email_id = ?1 AND tag_type = ?2",
            params![email_id, tag_type],
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

    /// Every distinct value ever assigned for `tag_type`, alphabetical — the
    /// vocabulary a filter can actually match, whatever Settings says today
    /// (rules and older defaults leave tags the current list may not name).
    /// One range scan of `idx_email_tags_type_value`.
    pub fn distinct_tag_values(&self, tag_type: &str) -> Result<Vec<String>> {
        let conn = self.reader();
        let mut stmt =
            conn.prepare("SELECT DISTINCT tag_value FROM email_tags WHERE tag_type = ?1 ORDER BY tag_value")?;
        let values = stmt
            .query_map(params![tag_type], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(values)
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
            // pending_sync = 0: optimistic sent copies awaiting reconciliation
            // are deleted when the provider's real copy arrives — classifying
            // them is wasted compute on a doomed row.
            "SELECT e.id FROM emails e
             WHERE e.account_id = ?1
               AND NOT EXISTS (
                   SELECT 1 FROM email_tags t WHERE t.email_id = e.id AND t.tag_type = 'intent'
               )
               AND LENGTH(e.snippet) > 20
               AND e.pending_sync = 0{cat_filter}{ts_filter}
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
               AND LENGTH(e.snippet) > 20
               AND e.pending_sync = 0",
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

    /// Per-`(account, tag_value)` thread counts for the tag board, narrowed by
    /// an [`EmailWindow`]. Unlike [`Database::get_tag_stats`] — which
    /// aggregates a whole scope into one row per tag value — this keeps the
    /// account, because the board renders one block per account+tag pair.
    ///
    /// Counts distinct `(account_id, thread_id)` pairs, matching what
    /// `get_filtered_emails` will actually list for the same block.
    pub fn get_tag_board_stats(
        &self,
        scope: crate::db::AccountScope<'_>,
        tag_type: &str,
        window: &crate::models::EmailWindow,
        now_ts: i64,
        limit: i32,
    ) -> Result<Vec<TagBoardRow>> {
        let (sql, binds) = Self::tag_board_stats_query(scope, tag_type, window, now_ts, limit);
        let conn = self.reader();
        let mut stmt = conn.prepare(&sql)?;
        let refs: Vec<&dyn rusqlite::ToSql> = binds.iter().map(|b| b.as_ref()).collect();
        let mut rows = stmt.query(refs.as_slice())?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(TagBoardRow {
                account_id: row.get(0)?,
                tag_value: row.get(1)?,
                count: row.get(2)?,
                sent_share: row.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
                read_share: row.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
                last_activity_at: row.get::<_, Option<i64>>(5)?,
            });
        }
        Ok(out)
    }

    /// `EXPLAIN QUERY PLAN` rows for the query above, so a test can assert the
    /// planner still drives from the tag index. See the test for why that is
    /// worth pinning.
    #[cfg(test)]
    pub(crate) fn explain_tag_board_stats(
        &self,
        scope: crate::db::AccountScope<'_>,
        tag_type: &str,
        window: &crate::models::EmailWindow,
        now_ts: i64,
        limit: i32,
    ) -> Vec<String> {
        let (sql, binds) = Self::tag_board_stats_query(scope, tag_type, window, now_ts, limit);
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let refs: Vec<&dyn rusqlite::ToSql> = binds.iter().map(|b| b.as_ref()).collect();
        let mut rows = stmt.query(refs.as_slice()).unwrap();
        let mut plan = Vec::new();
        while let Some(row) = rows.next().unwrap() {
            plan.push(row.get::<_, String>(3).unwrap());
        }
        plan
    }

    /// SQL + bind values for the tag board's stats query. Shared so the plan
    /// test explains exactly what production runs.
    fn tag_board_stats_query(
        scope: crate::db::AccountScope<'_>,
        tag_type: &str,
        window: &crate::models::EmailWindow,
        now_ts: i64,
        limit: i32,
    ) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
        // ?1 tag_type, ?2 limit, then the scope's account id (if any), then the
        // window binds — so the fixed indices stay stable.
        let mut next_index = 3usize;
        let (scope_cond, account_param): (String, Option<&str>) = match scope {
            crate::db::AccountScope::Account(id) => {
                let cond = format!("e.account_id = ?{next_index}");
                next_index += 1;
                (cond, Some(id))
            }
            crate::db::AccountScope::AllEnabled => (
                "e.account_id IN (SELECT id FROM accounts WHERE enabled = 1)".to_string(),
                None,
            ),
        };
        // `\` escapes LIKE's own wildcards so a user searching for "%" finds a
        // percent sign rather than everything.
        let search_term = window
            .search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| format!("%{}%", s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")));
        let search_sql = if search_term.is_some() {
            let frag = format!("AND t.tag_value LIKE ?{next_index} ESCAPE '\\'");
            next_index += 1;
            frag
        } else {
            String::new()
        };
        let (window_sql, window_binds) = window.sql("e", &mut next_index);

        // Each message contributes in proportion to how recent it is, so a tag
        // answered diligently three years ago and skimmed ever since reads as
        // cooling rather than as a live correspondence. The shares are
        // normalised by the same weights, so a uniformly-aged history is
        // unaffected — this shifts the *balance* within a history, while the
        // block-level decay in `tag_recency_factor` handles a tag that has gone
        // quiet altogether.
        // Last placeholder allocated for this query, so no further increment.
        let junk_sql = crate::db::exclude_junk_sql("e", window.hide_graymail);
        let latest_sql = crate::db::latest_tagged_in_thread_sql("e", 1, window.latest_tag_only);
        let now_idx = next_index;
        let w =
            format!("(1.0 / (1.0 + (MAX(0, ?{now_idx} - e.timestamp) / 86400.0) / {INTERACTION_HALF_LIFE_DAYS:.1}))");

        // Engagement is aggregated in the SAME pass as the thread count. The
        // obvious formulation — join back to every message of every matched
        // thread to ask "was this thread replied to?" — measured 202 SECONDS
        // for `company` on a 6 GB mailbox. Reading the signals off the tagged
        // messages themselves is one scan of rows already being touched: 219 ms
        // for the same query. Do not reintroduce the thread-level join.
        let sql = format!(
            "SELECT e.account_id AS aid,
                    t.tag_value AS tag_value,
                    COUNT(DISTINCT e.thread_id) AS cnt,
                    SUM({w} * CASE WHEN e.is_sent = 1 THEN 1.0 ELSE 0.0 END) / SUM({w}) AS sent_share,
                    SUM({w} * CASE WHEN e.is_read = 1 THEN 1.0 ELSE 0.0 END) / SUM({w}) AS read_share,
                    MAX(e.timestamp) AS last_ts
             FROM email_tags t INDEXED BY idx_email_tags_type_value
             -- CROSS JOIN pins the join order; SQLite never reorders one.
             -- `INDEXED BY` only fixes WHICH index is used for `t`, not that
             -- `t` drives the loop. Adding a timestamp filter was enough to
             -- flip the planner into scanning `emails` by date and probing the
             -- tag index per row: 190ms became 11s with a category+range
             -- window, and 86 SECONDS with a range alone, on a 6 GB mailbox.
             CROSS JOIN emails e ON e.id = t.email_id
             WHERE {scope_cond}
               AND t.tag_type = ?1
               AND e.is_deleted = 0
               AND e.mailbox IN ('inbox', 'sent')
               {junk_sql}
               {latest_sql}
               {search_sql}
               {window_sql}
             GROUP BY aid, tag_value
             ORDER BY cnt DESC, aid, tag_value
             LIMIT ?2"
        );

        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(tag_type.to_string()), Box::new(limit)];
        if let Some(id) = account_param {
            binds.push(Box::new(id.to_string()));
        }
        if let Some(term) = search_term {
            binds.push(Box::new(term));
        }
        binds.extend(window_binds);
        binds.push(Box::new(now_ts));

        (sql, binds)
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

    #[test]
    fn distinct_tag_values_lists_each_value_once_per_type() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc1");
        insert_email(&db, "e1", "acc1", "t1", 1_000);
        insert_email(&db, "e2", "acc1", "t2", 1_000);
        insert_email(&db, "e3", "acc1", "t3", 1_000);
        tag_email(&db, "e1", "intent", "newsletter");
        tag_email(&db, "e2", "intent", "newsletter");
        tag_email(&db, "e3", "intent", "complaint");
        tag_email(&db, "e3", "topic", "billing");
        assert_eq!(db.distinct_tag_values("intent").unwrap(), ["complaint", "newsletter"]);
        assert_eq!(db.distinct_tag_values("topic").unwrap(), ["billing"]);
        assert!(db.distinct_tag_values("company").unwrap().is_empty());
    }

    /// The tag board's stats query must always drive from `email_tags`, never
    /// scan `emails` and probe the tag index per row.
    ///
    /// This is a *plan* test on purpose. The inverted plan returns identical
    /// results, so no assertion about rows can catch it — but it took the
    /// unified board from 190ms to 11s with a date range applied, and to 86
    /// seconds with a range and no category. The planner flipped on its own
    /// once a timestamp filter appeared; only `CROSS JOIN` holds the order.
    #[test]
    fn tag_board_stats_drives_from_the_tag_index() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc1");
        insert_email(&db, "e1", "acc1", "t1", 1_000);
        tag_email(&db, "e1", "company", "globex");

        // A window is what used to flip the planner, so explain the worst case.
        let window = crate::models::EmailWindow {
            categories: vec!["primary".into()],
            since: Some(0),
            until: Some(9_999_999),
            search: None,
            hide_graymail: false,
            latest_tag_only: true,
        };
        let plan = db.explain_tag_board_stats(crate::db::AccountScope::AllEnabled, "company", &window, 0, 24);

        let first = plan.first().expect("query plan should not be empty");
        assert!(
            first.contains("email_tags"),
            "the outer loop must be email_tags, got: {first}\nfull plan: {plan:#?}"
        );
        assert!(!first.contains("SCAN emails"), "must never scan emails first: {first}");
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

    fn insert_classifiable_email(db: &Database, id: &str, account_id: &str, pending_sync: i32) {
        ensure_account(db, account_id);
        db.connection()
            .execute(
                "INSERT INTO emails
                     (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                      recipients_json, cc_json, snippet, timestamp, is_read, category, pending_sync, created_at)
                     VALUES (?1,?2,?1,'subj','sender','s@s.com','s.com','[]','[]',
                             'a snippet definitely longer than twenty characters',100,0,'primary',?3,0)",
                params![id, account_id, pending_sync],
            )
            .unwrap();
    }

    // Optimistic sent copies awaiting reconciliation must not enter the
    // classification backlog — they are deleted when the provider's real
    // copy arrives, so classifying them is wasted compute on a doomed row.
    #[test]
    fn unclassified_backlog_excludes_pending_sent_rows() {
        let db = Database::new_for_testing().unwrap();
        insert_classifiable_email(&db, "e-normal", "acc1", 0);
        insert_classifiable_email(&db, "e-pending", "acc1", 1);

        let ids = db.get_unclassified_email_ids("acc1", 10, &[], None).unwrap();
        assert!(ids.contains(&"e-normal".to_string()), "got {:?}", ids);
        assert!(
            !ids.contains(&"e-pending".to_string()),
            "pending rows must be excluded, got {:?}",
            ids
        );
        assert_eq!(db.count_unclassified_emails("acc1").unwrap(), 1);
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
                &crate::models::EmailWindow::default(),
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
