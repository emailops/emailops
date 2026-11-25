use std::collections::HashSet;

use rusqlite::params;

use crate::models::error::Result;
use crate::models::{Attachment, AttachmentRule, AttachmentRuleSuggestion, AttachmentRuleSuggestionStatus};
use crate::services::attachment_suggestions::{AttachmentObservation, ResolvedSuggestion, SuggestionCandidate};

use super::{AccountScope, Database};

/// The attachment columns `row_to_attachment` reads, plus the name of the rule
/// that saved each one. Callers append `WHERE …` on `a`.
const ATTACHMENT_SELECT: &str = "SELECT a.id, a.account_id, a.email_id, a.rule_id, a.gmail_attachment_id, a.filename, a.mime_type, a.file_size, a.file_path, a.tags_json, a.sender_email, a.subject, a.email_timestamp, a.created_at, r.name
     FROM attachments a LEFT JOIN attachment_rules r ON r.id = a.rule_id";

/// Newest email first; the id breaks ties so paging never repeats or skips a row.
const ATTACHMENT_ORDER: &str = "ORDER BY a.email_timestamp DESC, a.id DESC";

type SqlParams = Vec<Box<dyn rusqlite::types::ToSql>>;

/// `JOIN … WHERE …` for the attachments of `scope` carrying `tag`, and its
/// parameters. `AllEnabled` spans every enabled account, like the outbox.
fn scoped_filter(scope: AccountScope<'_>, tag: Option<&str>) -> Result<(String, SqlParams)> {
    let (mut sql, mut params): (String, SqlParams) = match scope {
        AccountScope::Account(id) => ("WHERE a.account_id = ?".to_string(), vec![Box::new(id.to_string())]),
        AccountScope::AllEnabled => (
            "JOIN accounts acc ON acc.id = a.account_id WHERE acc.enabled = 1".to_string(),
            vec![],
        ),
    };
    if let Some(tag) = tag {
        sql.push_str(" AND a.tags_json LIKE ? ESCAPE '\\'");
        params.push(Box::new(tag_like_pattern(tag)?));
    }
    Ok((sql, params))
}

/// `LIKE` pattern matching `tag` as one whole element of `tags_json`: the tag
/// encoded as a JSON string (as stored), with the `LIKE` wildcards escaped so
/// `%` and `_` in a tag are literal. `LIKE` ignores ASCII case.
fn tag_like_pattern(tag: &str) -> Result<String> {
    let encoded = serde_json::to_string(tag)?;
    let escaped = encoded.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    Ok(format!("%{escaped}%"))
}

fn param_refs(params: &SqlParams) -> Vec<&dyn rusqlite::types::ToSql> {
    params.iter().map(|p| p.as_ref()).collect()
}

impl Database {
    // --- Attachment Rules ---

    pub fn insert_attachment_rule(&self, rule: &AttachmentRule) -> Result<()> {
        let conn = self.connection();
        let tags_json = serde_json::to_string(&rule.tags)?;
        conn.execute(
            "INSERT INTO attachment_rules (id, account_id, name, sender_email_pattern, subject_pattern, filename_pattern, tags_json, enabled, created_at, updated_at, keep_latest_per_thread)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                rule.id,
                rule.account_id,
                rule.name,
                rule.sender_email_pattern,
                rule.subject_pattern,
                rule.filename_pattern,
                tags_json,
                rule.enabled as i32,
                rule.created_at,
                rule.updated_at,
                rule.keep_latest_per_thread,
            ],
        )?;
        Ok(())
    }

    pub fn update_attachment_rule(&self, rule: &AttachmentRule) -> Result<()> {
        let conn = self.connection();
        let tags_json = serde_json::to_string(&rule.tags)?;
        conn.execute(
            "UPDATE attachment_rules SET name = ?1, sender_email_pattern = ?2, subject_pattern = ?3, filename_pattern = ?4, tags_json = ?5, enabled = ?6, updated_at = ?7, keep_latest_per_thread = ?10
             WHERE id = ?8 AND account_id = ?9",
            params![
                rule.name,
                rule.sender_email_pattern,
                rule.subject_pattern,
                rule.filename_pattern,
                tags_json,
                rule.enabled as i32,
                rule.updated_at,
                rule.id,
                rule.account_id,
                rule.keep_latest_per_thread,
            ],
        )?;
        Ok(())
    }

    /// Update tags on all attachments belonging to a rule.
    pub fn update_attachments_tags_for_rule(&self, rule_id: &str, tags: &[String]) -> Result<()> {
        let conn = self.connection();
        let tags_json = serde_json::to_string(tags)?;
        conn.execute(
            "UPDATE attachments SET tags_json = ?1 WHERE rule_id = ?2",
            params![tags_json, rule_id],
        )?;
        Ok(())
    }

    pub fn get_attachments_for_email(&self, email_id: &str) -> Result<Vec<Attachment>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!("{ATTACHMENT_SELECT} WHERE a.email_id = ?1"))?;
        let attachments = stmt
            .query_map(params![email_id], row_to_attachment)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(attachments)
    }

    pub fn get_attachments_for_rule(&self, rule_id: &str) -> Result<Vec<Attachment>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!("{ATTACHMENT_SELECT} WHERE a.rule_id = ?1"))?;
        let attachments = stmt
            .query_map(params![rule_id], row_to_attachment)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(attachments)
    }

    pub fn delete_attachment_by_id(&self, attachment_id: &str) -> Result<Option<String>> {
        let conn = self.connection();
        let path: Option<String> = conn
            .query_row(
                "SELECT file_path FROM attachments WHERE id = ?1",
                params![attachment_id],
                |row| row.get(0),
            )
            .ok();
        conn.execute("DELETE FROM attachments WHERE id = ?1", params![attachment_id])?;
        Ok(path)
    }

    pub fn delete_attachment_rule(&self, rule_id: &str, account_id: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM attachment_rules WHERE id = ?1 AND account_id = ?2",
            params![rule_id, account_id],
        )?;
        Ok(())
    }

    /// Delete a rule and every attachment it collected in one transaction;
    /// returns the stored file paths for the caller to remove from disk.
    pub fn delete_attachment_rule_with_attachments(&self, rule_id: &str, account_id: &str) -> Result<Vec<String>> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        let paths = {
            let mut stmt = tx.prepare("SELECT file_path FROM attachments WHERE rule_id = ?1")?;
            let paths = stmt
                .query_map(params![rule_id], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            paths
        };
        tx.execute("DELETE FROM attachments WHERE rule_id = ?1", params![rule_id])?;
        tx.execute(
            "DELETE FROM attachment_rules WHERE id = ?1 AND account_id = ?2",
            params![rule_id, account_id],
        )?;
        tx.commit()?;
        Ok(paths)
    }

    pub fn get_attachment_rules(&self, account_id: &str) -> Result<Vec<AttachmentRule>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, name, sender_email_pattern, subject_pattern, filename_pattern, tags_json, enabled, created_at, updated_at, keep_latest_per_thread
             FROM attachment_rules WHERE account_id = ?1 AND enabled = 1
             ORDER BY created_at DESC",
        )?;
        let rules = stmt
            .query_map(params![account_id], row_to_attachment_rule)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rules)
    }

    pub fn get_all_attachment_rules(&self, account_id: &str) -> Result<Vec<AttachmentRule>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, name, sender_email_pattern, subject_pattern, filename_pattern, tags_json, enabled, created_at, updated_at, keep_latest_per_thread
             FROM attachment_rules WHERE account_id = ?1
             ORDER BY created_at DESC",
        )?;
        let rules = stmt
            .query_map(params![account_id], row_to_attachment_rule)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rules)
    }

    pub fn get_attachment_rule(&self, rule_id: &str) -> Result<Option<AttachmentRule>> {
        let conn = self.reader();
        let result = conn.query_row(
            "SELECT id, account_id, name, sender_email_pattern, subject_pattern, filename_pattern, tags_json, enabled, created_at, updated_at, keep_latest_per_thread
             FROM attachment_rules WHERE id = ?1",
            params![rule_id],
            row_to_attachment_rule,
        );
        match result {
            Ok(rule) => Ok(Some(rule)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    // --- Attachments ---

    pub fn insert_attachment(&self, attachment: &Attachment) -> Result<()> {
        let conn = self.connection();
        let tags_json = serde_json::to_string(&attachment.tags)?;
        conn.execute(
            "INSERT OR IGNORE INTO attachments (id, account_id, email_id, rule_id, gmail_attachment_id, filename, mime_type, file_size, file_path, tags_json, sender_email, subject, email_timestamp, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                attachment.id,
                attachment.account_id,
                attachment.email_id,
                attachment.rule_id,
                attachment.gmail_attachment_id,
                attachment.filename,
                attachment.mime_type,
                attachment.file_size,
                attachment.file_path,
                tags_json,
                attachment.sender_email,
                attachment.subject,
                attachment.email_timestamp,
                attachment.created_at,
            ],
        )?;
        Ok(())
    }

    /// Insert an attachment a rule collected, unless the rule was edited
    /// (its `updated_at` is no longer `rule_updated_at`) or deleted, the email
    /// is gone, or the same file is already stored for the rule. Returns
    /// whether the row was inserted. One statement, so there is no window
    /// between the check and the insert.
    pub fn insert_attachment_for_rule_version(&self, attachment: &Attachment, rule_updated_at: i64) -> Result<bool> {
        let conn = self.connection();
        let tags_json = serde_json::to_string(&attachment.tags)?;
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO attachments (id, account_id, email_id, rule_id, gmail_attachment_id, filename, mime_type, file_size, file_path, tags_json, sender_email, subject, email_timestamp, created_at)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14
             WHERE EXISTS (SELECT 1 FROM attachment_rules WHERE id = ?4 AND updated_at = ?15)
               AND EXISTS (SELECT 1 FROM emails WHERE id = ?3)",
            params![
                attachment.id,
                attachment.account_id,
                attachment.email_id,
                attachment.rule_id,
                attachment.gmail_attachment_id,
                attachment.filename,
                attachment.mime_type,
                attachment.file_size,
                attachment.file_path,
                tags_json,
                attachment.sender_email,
                attachment.subject,
                attachment.email_timestamp,
                attachment.created_at,
                rule_updated_at,
            ],
        )?;
        Ok(inserted > 0)
    }

    /// Whether `rule_id` already holds `filename` from a message of `email_id`'s
    /// thread sent at the same time or later: the copy a keep-latest rule
    /// keeps instead of this one (the same message filed twice included).
    pub fn newer_copy_held(&self, rule_id: &str, email_id: &str, filename: &str) -> Result<bool> {
        let conn = self.reader();
        Ok(conn.query_row(
            "SELECT EXISTS (
               SELECT 1 FROM emails e
               JOIN emails other ON other.account_id = e.account_id AND other.thread_id = e.thread_id
               JOIN attachments a ON a.email_id = other.id
               WHERE e.id = ?2 AND e.thread_id != ''
                 AND a.rule_id = ?1 AND a.filename = ?3 AND a.email_id != ?2
                 AND a.email_timestamp >= e.timestamp)",
            params![rule_id, email_id, filename],
            |row| row.get(0),
        )?)
    }

    /// Delete the attachments of `rule_id` that a newer copy of the same file
    /// name in the same thread supersedes (of copies sent at the same time,
    /// one stays), only those named `filename` when given. Returns the stored
    /// files of the deleted rows, for the caller to remove.
    pub fn delete_superseded_attachments(&self, rule_id: &str, filename: Option<&str>) -> Result<Vec<String>> {
        let conn = self.connection();
        let tx = conn.unchecked_transaction()?;
        let superseded: Vec<(String, String)> = {
            let mut stmt = tx.prepare(
                "SELECT a.id, a.file_path FROM attachments a
                 JOIN emails e ON e.id = a.email_id
                 WHERE a.rule_id = ?1 AND (?2 IS NULL OR a.filename = ?2) AND e.thread_id != ''
                   AND EXISTS (
                     SELECT 1 FROM attachments b
                     JOIN emails eb ON eb.id = b.email_id
                     WHERE b.rule_id = a.rule_id AND b.filename = a.filename AND b.id != a.id
                       AND eb.account_id = e.account_id AND eb.thread_id = e.thread_id
                       AND (b.email_timestamp > a.email_timestamp
                            OR (b.email_timestamp = a.email_timestamp AND b.id > a.id)))",
            )?;
            let rows = stmt
                .query_map(params![rule_id, filename], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        for (id, _) in &superseded {
            tx.execute("DELETE FROM attachments WHERE id = ?1", params![id])?;
        }
        tx.commit()?;
        Ok(superseded.into_iter().map(|(_, path)| path).collect())
    }

    pub fn attachment_exists(&self, email_id: &str, filename: &str, rule_id: &str) -> Result<bool> {
        let conn = self.reader();
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM attachments WHERE email_id = ?1 AND filename = ?2 AND rule_id = ?3",
            params![email_id, filename, rule_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// One page of the attachments of `scope` carrying `tag`, newest first.
    pub fn get_attachments(
        &self,
        scope: AccountScope<'_>,
        tag: Option<&str>,
        limit: i32,
        offset: i32,
    ) -> Result<Vec<Attachment>> {
        let (filter, mut params) = scoped_filter(scope, tag)?;
        params.push(Box::new(limit));
        params.push(Box::new(offset));
        let sql = format!("{ATTACHMENT_SELECT} {filter} {ATTACHMENT_ORDER} LIMIT ? OFFSET ?");
        let conn = self.reader();
        let mut stmt = conn.prepare(&sql)?;
        let attachments = stmt
            .query_map(param_refs(&params).as_slice(), row_to_attachment)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(attachments)
    }

    pub fn count_attachments(&self, scope: AccountScope<'_>, tag: Option<&str>) -> Result<i32> {
        let (filter, params) = scoped_filter(scope, tag)?;
        let sql = format!("SELECT COUNT(*) FROM attachments a {filter}");
        let conn = self.reader();
        let count: i32 = conn.query_row(&sql, param_refs(&params).as_slice(), |row| row.get(0))?;
        Ok(count)
    }

    /// Every id `get_attachments` would list for `scope` and `tag`, in the same
    /// order. Backs "select all" without loading every row.
    pub fn list_attachment_ids(&self, scope: AccountScope<'_>, tag: Option<&str>) -> Result<Vec<String>> {
        let (filter, params) = scoped_filter(scope, tag)?;
        let sql = format!("SELECT a.id FROM attachments a {filter} {ATTACHMENT_ORDER}");
        let conn = self.reader();
        let mut stmt = conn.prepare(&sql)?;
        let ids = stmt
            .query_map(param_refs(&params).as_slice(), |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    }

    pub fn get_attachment(&self, attachment_id: &str) -> Result<Option<Attachment>> {
        let conn = self.reader();
        let result = conn.query_row(
            &format!("{ATTACHMENT_SELECT} WHERE a.id = ?1"),
            params![attachment_id],
            row_to_attachment,
        );
        match result {
            Ok(att) => Ok(Some(att)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Count attachments saved by a given rule. Used by the delete-rule
    /// confirmation dialog so the user knows how many files will be removed.
    pub fn count_attachments_for_rule(&self, rule_id: &str) -> Result<i32> {
        let conn = self.reader();
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM attachments WHERE rule_id = ?1",
            params![rule_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Delete all attachments for a rule. Returns the file_paths for disk cleanup.
    pub fn delete_attachments_for_rule(&self, rule_id: &str) -> Result<Vec<String>> {
        let conn = self.connection();
        let mut stmt = conn.prepare("SELECT file_path FROM attachments WHERE rule_id = ?1")?;
        let paths: Vec<String> = stmt
            .query_map(params![rule_id], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        conn.execute("DELETE FROM attachments WHERE rule_id = ?1", params![rule_id])?;
        Ok(paths)
    }

    /// The distinct tags of the attachments of `scope`, one per spelling that
    /// differs only in ASCII case (the first in byte order, "Invoice" before
    /// "invoice"), sorted. The same folding as the `LIKE` tag filter, so each
    /// chip lists exactly the rows it stands for.
    pub fn get_all_tags(&self, scope: AccountScope<'_>) -> Result<Vec<String>> {
        let (filter, params) = scoped_filter(scope, None)?;
        let sql = format!("SELECT DISTINCT a.tags_json FROM attachments a {filter}");
        let conn = self.reader();
        let mut stmt = conn.prepare(&sql)?;
        let tag_jsons = stmt
            .query_map(param_refs(&params).as_slice(), |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        one_spelling_per_tag(&tag_jsons)
    }

    /// Every tag a rule or a collected attachment carries, in any account,
    /// enabled or not: what the rule form offers, so one tag serves every
    /// account. One spelling per tag, as [`Self::get_all_tags`].
    pub fn get_tag_vocabulary(&self) -> Result<Vec<String>> {
        let conn = self.reader();
        let mut stmt =
            conn.prepare("SELECT tags_json FROM attachments UNION SELECT tags_json FROM attachment_rules")?;
        let tag_jsons = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        one_spelling_per_tag(&tag_jsons)
    }

    // --- Attachment rule suggestions ---

    /// Every attachment on the account's received mail (inbox and filed
    /// folders, not deleted) since `since` — the input of the suggestion
    /// miner. Sent, spam and trash mail never make a rule candidate.
    pub fn get_attachment_observations(&self, account_id: &str, since: i64) -> Result<Vec<AttachmentObservation>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT m.email_id, e.sender_email, e.sender, e.subject, e.timestamp, m.filename, m.mime_type
             FROM email_attachment_meta m
             JOIN emails e ON e.id = m.email_id
             WHERE m.account_id = ?1
               AND e.timestamp >= ?2
               AND e.is_deleted = 0
               AND e.mailbox NOT IN ('sent', 'spam', 'trash')
               AND e.is_sent = 0",
        )?;
        let rows = stmt
            .query_map(params![account_id, since], |row| {
                Ok(AttachmentObservation {
                    email_id: row.get(0)?,
                    sender_email: row.get(1)?,
                    sender_name: row.get(2)?,
                    subject: row.get(3)?,
                    timestamp: row.get(4)?,
                    filename: row.get(5)?,
                    mime_type: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Patterns of the suggestions the user already accepted or dismissed.
    pub fn get_resolved_attachment_rule_suggestions(&self, account_id: &str) -> Result<Vec<ResolvedSuggestion>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT sender_email_pattern, filename_pattern FROM attachment_rule_suggestions
             WHERE account_id = ?1 AND status <> 'pending'",
        )?;
        let rows = stmt
            .query_map(params![account_id], |row| {
                Ok(ResolvedSuggestion {
                    sender_email_pattern: row.get(0)?,
                    filename_pattern: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Replace the account's pending suggestions with `candidates` in one
    /// transaction. A candidate whose key is already pending keeps its row
    /// id (so a UI holding it stays valid) and gets fresh counts; pending
    /// rows no longer proposed are removed; resolved rows are never touched.
    pub fn replace_pending_attachment_rule_suggestions(
        &self,
        account_id: &str,
        candidates: &[SuggestionCandidate],
        now: i64,
    ) -> Result<()> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;

        let pending_keys: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT suggestion_key FROM attachment_rule_suggestions
                 WHERE account_id = ?1 AND status = 'pending'",
            )?;
            let keys = stmt
                .query_map(params![account_id], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            keys
        };
        let proposed: HashSet<&str> = candidates.iter().map(|c| c.key.as_str()).collect();
        for key in pending_keys.iter().filter(|k| !proposed.contains(k.as_str())) {
            tx.execute(
                "DELETE FROM attachment_rule_suggestions
                 WHERE account_id = ?1 AND suggestion_key = ?2 AND status = 'pending'",
                params![account_id, key],
            )?;
        }

        for c in candidates {
            tx.execute(
                "INSERT INTO attachment_rule_suggestions
                 (id, account_id, suggestion_key, name, sender_email_pattern, filename_pattern, tags_json,
                  email_count, first_seen, last_seen, sample_filenames_json, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'pending', ?12, ?12)
                 ON CONFLICT (account_id, suggestion_key) DO UPDATE SET
                    name = excluded.name,
                    sender_email_pattern = excluded.sender_email_pattern,
                    filename_pattern = excluded.filename_pattern,
                    tags_json = excluded.tags_json,
                    email_count = excluded.email_count,
                    first_seen = excluded.first_seen,
                    last_seen = excluded.last_seen,
                    sample_filenames_json = excluded.sample_filenames_json,
                    updated_at = excluded.updated_at
                 WHERE status = 'pending'",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    account_id,
                    c.key,
                    c.name,
                    c.sender_email_pattern,
                    c.filename_pattern,
                    serde_json::to_string(&c.tags)?,
                    c.email_count,
                    c.first_seen,
                    c.last_seen,
                    serde_json::to_string(&c.sample_filenames)?,
                    now,
                ],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Pending suggestions, most recurring first.
    pub fn get_pending_attachment_rule_suggestions(&self, account_id: &str) -> Result<Vec<AttachmentRuleSuggestion>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, name, sender_email_pattern, filename_pattern, tags_json, email_count,
                    first_seen, last_seen, sample_filenames_json, status, created_at, updated_at
             FROM attachment_rule_suggestions
             WHERE account_id = ?1 AND status = 'pending'
             ORDER BY email_count DESC, last_seen DESC, id",
        )?;
        let rows = stmt
            .query_map(params![account_id], row_to_attachment_rule_suggestion)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Dismissed suggestions, most recently dismissed first.
    pub fn get_dismissed_attachment_rule_suggestions(&self, account_id: &str) -> Result<Vec<AttachmentRuleSuggestion>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, name, sender_email_pattern, filename_pattern, tags_json, email_count,
                    first_seen, last_seen, sample_filenames_json, status, created_at, updated_at
             FROM attachment_rule_suggestions
             WHERE account_id = ?1 AND status = 'dismissed'
             ORDER BY updated_at DESC, id",
        )?;
        let rows = stmt
            .query_map(params![account_id], row_to_attachment_rule_suggestion)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Put a dismissed suggestion back to pending. Returns `false` when the
    /// account has no dismissed suggestion with that id.
    pub fn restore_dismissed_attachment_rule_suggestion(
        &self,
        account_id: &str,
        suggestion_id: &str,
        now: i64,
    ) -> Result<bool> {
        let conn = self.connection();
        let changed = conn.execute(
            "UPDATE attachment_rule_suggestions SET status = 'pending', updated_at = ?1
             WHERE id = ?2 AND account_id = ?3 AND status = 'dismissed'",
            params![now, suggestion_id, account_id],
        )?;
        Ok(changed > 0)
    }

    /// Returns `false` when no suggestion with that id belongs to the account.
    pub fn set_attachment_rule_suggestion_status(
        &self,
        account_id: &str,
        suggestion_id: &str,
        status: AttachmentRuleSuggestionStatus,
        now: i64,
    ) -> Result<bool> {
        let conn = self.connection();
        let changed = conn.execute(
            "UPDATE attachment_rule_suggestions SET status = ?1, updated_at = ?2
             WHERE id = ?3 AND account_id = ?4",
            params![status.as_str(), now, suggestion_id, account_id],
        )?;
        Ok(changed > 0)
    }

    /// The account's emails a rule can reach (see
    /// `services::attachments::rules_apply_to_mailbox`): everything but Spam,
    /// Trash and locally deleted mail. The rule's patterns are matched by the
    /// caller.
    pub fn get_emails_matching_rule(&self, account_id: &str) -> Result<Vec<(String, String, String)>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, sender_email, subject FROM emails
             WHERE account_id = ?1 AND is_deleted = 0 AND mailbox NOT IN ('spam', 'trash')",
        )?;
        let rows = stmt
            .query_map(params![account_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

/// The tags in `tag_jsons` (JSON arrays), sorted, one per ASCII-case-folded
/// tag under its smallest spelling, so "invoice" and "Invoice" make one chip.
/// An unreadable array is an error, not a silently missing tag.
fn one_spelling_per_tag(tag_jsons: &[String]) -> Result<Vec<String>> {
    let mut by_folded: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for json_str in tag_jsons {
        let tags: Vec<String> = serde_json::from_str(json_str)?;
        for tag in tags {
            by_folded
                .entry(tag.to_ascii_lowercase())
                .and_modify(|kept| {
                    if tag < *kept {
                        kept.clone_from(&tag);
                    }
                })
                .or_insert(tag);
        }
    }
    let mut tags: Vec<String> = by_folded.into_values().collect();
    tags.sort();
    Ok(tags)
}

fn row_to_attachment_rule(row: &rusqlite::Row) -> rusqlite::Result<AttachmentRule> {
    let tags_json: String = row.get(6)?;
    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
    let enabled: i32 = row.get(7)?;
    Ok(AttachmentRule {
        id: row.get(0)?,
        account_id: row.get(1)?,
        name: row.get(2)?,
        sender_email_pattern: row.get(3)?,
        subject_pattern: row.get(4)?,
        filename_pattern: row.get(5)?,
        tags,
        enabled: enabled != 0,
        keep_latest_per_thread: row.get(10)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn row_to_attachment_rule_suggestion(row: &rusqlite::Row) -> rusqlite::Result<AttachmentRuleSuggestion> {
    // The columns are only ever written by serde and a CHECK-constrained
    // status, so anything unreadable is corruption: fail the read instead of
    // showing a suggestion with silently emptied fields.
    fn json_column<T: serde::de::DeserializeOwned>(row: &rusqlite::Row, idx: usize) -> rusqlite::Result<T> {
        let raw: String = row.get(idx)?;
        serde_json::from_str(&raw)
            .map_err(|e| rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Text, Box::new(e)))
    }
    let status: String = row.get(10)?;
    let status = AttachmentRuleSuggestionStatus::parse(&status).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            10,
            rusqlite::types::Type::Text,
            format!("unknown attachment rule suggestion status {status:?}").into(),
        )
    })?;
    Ok(AttachmentRuleSuggestion {
        id: row.get(0)?,
        account_id: row.get(1)?,
        name: row.get(2)?,
        sender_email_pattern: row.get(3)?,
        filename_pattern: row.get(4)?,
        tags: json_column(row, 5)?,
        email_count: row.get(6)?,
        first_seen: row.get(7)?,
        last_seen: row.get(8)?,
        sample_filenames: json_column(row, 9)?,
        status,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

/// Reads an `ATTACHMENT_SELECT` row. `tags_json` is only ever written by
/// serde, so an unreadable value is corruption: the read fails instead of
/// showing the attachment without its tags.
fn row_to_attachment(row: &rusqlite::Row) -> rusqlite::Result<Attachment> {
    let tags_json: String = row.get(9)?;
    let tags: Vec<String> = serde_json::from_str(&tags_json)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(e)))?;
    Ok(Attachment {
        id: row.get(0)?,
        account_id: row.get(1)?,
        email_id: row.get(2)?,
        rule_id: row.get(3)?,
        rule_name: row.get(14)?,
        gmail_attachment_id: row.get(4)?,
        filename: row.get(5)?,
        mime_type: row.get(6)?,
        file_size: row.get(7)?,
        file_path: row.get(8)?,
        tags,
        sender_email: row.get(10)?,
        subject: row.get(11)?,
        email_timestamp: row.get(12)?,
        created_at: row.get(13)?,
    })
}
