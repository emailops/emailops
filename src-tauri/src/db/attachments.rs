use rusqlite::params;

use crate::models::error::Result;
use crate::models::{Attachment, AttachmentRule};

use super::Database;

impl Database {
    // --- Attachment Rules ---

    pub fn insert_attachment_rule(&self, rule: &AttachmentRule) -> Result<()> {
        let conn = self.connection();
        let tags_json = serde_json::to_string(&rule.tags)?;
        conn.execute(
            "INSERT INTO attachment_rules (id, account_id, name, sender_email_pattern, subject_pattern, filename_pattern, tags_json, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
            ],
        )?;
        Ok(())
    }

    pub fn update_attachment_rule(&self, rule: &AttachmentRule) -> Result<()> {
        let conn = self.connection();
        let tags_json = serde_json::to_string(&rule.tags)?;
        conn.execute(
            "UPDATE attachment_rules SET name = ?1, sender_email_pattern = ?2, subject_pattern = ?3, filename_pattern = ?4, tags_json = ?5, enabled = ?6, updated_at = ?7
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
        let mut stmt = conn.prepare(
            "SELECT id, account_id, email_id, rule_id, gmail_attachment_id, filename, mime_type, file_size, file_path, tags_json, sender_email, subject, email_timestamp, created_at
             FROM attachments WHERE email_id = ?1",
        )?;
        let attachments = stmt
            .query_map(params![email_id], row_to_attachment)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(attachments)
    }

    pub fn get_attachments_for_rule(&self, rule_id: &str) -> Result<Vec<Attachment>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, email_id, rule_id, gmail_attachment_id, filename, mime_type, file_size, file_path, tags_json, sender_email, subject, email_timestamp, created_at
             FROM attachments WHERE rule_id = ?1",
        )?;
        let attachments = stmt
            .query_map(params![rule_id], row_to_attachment)?
            .filter_map(|r| r.ok())
            .collect();
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

    pub fn get_attachment_rules(&self, account_id: &str) -> Result<Vec<AttachmentRule>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, account_id, name, sender_email_pattern, subject_pattern, filename_pattern, tags_json, enabled, created_at, updated_at
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
            "SELECT id, account_id, name, sender_email_pattern, subject_pattern, filename_pattern, tags_json, enabled, created_at, updated_at
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
            "SELECT id, account_id, name, sender_email_pattern, subject_pattern, filename_pattern, tags_json, enabled, created_at, updated_at
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

    pub fn attachment_exists(&self, email_id: &str, filename: &str, rule_id: &str) -> Result<bool> {
        let conn = self.reader();
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM attachments WHERE email_id = ?1 AND filename = ?2 AND rule_id = ?3",
            params![email_id, filename, rule_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn get_attachments(
        &self,
        account_id: &str,
        tag: Option<&str>,
        limit: i32,
        offset: i32,
    ) -> Result<Vec<Attachment>> {
        let conn = self.reader();
        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match tag {
            Some(t) => (
                "SELECT id, account_id, email_id, rule_id, gmail_attachment_id, filename, mime_type, file_size, file_path, tags_json, sender_email, subject, email_timestamp, created_at
                 FROM attachments WHERE account_id = ?1 AND tags_json LIKE ?2
                 ORDER BY email_timestamp DESC LIMIT ?3 OFFSET ?4".to_string(),
                vec![
                    Box::new(account_id.to_string()),
                    Box::new(format!("%\"{}\"%" , t)),
                    Box::new(limit),
                    Box::new(offset),
                ],
            ),
            None => (
                "SELECT id, account_id, email_id, rule_id, gmail_attachment_id, filename, mime_type, file_size, file_path, tags_json, sender_email, subject, email_timestamp, created_at
                 FROM attachments WHERE account_id = ?1
                 ORDER BY email_timestamp DESC LIMIT ?2 OFFSET ?3".to_string(),
                vec![
                    Box::new(account_id.to_string()),
                    Box::new(limit),
                    Box::new(offset),
                ],
            ),
        };
        let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let attachments = stmt
            .query_map(params_refs.as_slice(), row_to_attachment)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(attachments)
    }

    pub fn count_attachments(&self, account_id: &str, tag: Option<&str>) -> Result<i32> {
        let conn = self.reader();
        let count: i32 = match tag {
            Some(t) => conn.query_row(
                "SELECT COUNT(*) FROM attachments WHERE account_id = ?1 AND tags_json LIKE ?2",
                params![account_id, format!("%\"{}\"%", t)],
                |row| row.get(0),
            )?,
            None => conn.query_row(
                "SELECT COUNT(*) FROM attachments WHERE account_id = ?1",
                params![account_id],
                |row| row.get(0),
            )?,
        };
        Ok(count)
    }

    pub fn get_attachment(&self, attachment_id: &str) -> Result<Option<Attachment>> {
        let conn = self.reader();
        let result = conn.query_row(
            "SELECT id, account_id, email_id, rule_id, gmail_attachment_id, filename, mime_type, file_size, file_path, tags_json, sender_email, subject, email_timestamp, created_at
             FROM attachments WHERE id = ?1",
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

    /// Get all distinct tags across attachments for an account.
    pub fn get_all_tags(&self, account_id: &str) -> Result<Vec<String>> {
        let conn = self.reader();
        let mut stmt = conn.prepare("SELECT DISTINCT tags_json FROM attachments WHERE account_id = ?1")?;
        let tag_jsons: Vec<String> = stmt
            .query_map(params![account_id], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        let mut all_tags = std::collections::BTreeSet::new();
        for json_str in &tag_jsons {
            if let Ok(tags) = serde_json::from_str::<Vec<String>>(json_str) {
                for tag in tags {
                    all_tags.insert(tag);
                }
            }
        }
        Ok(all_tags.into_iter().collect())
    }

    /// Get emails matching a rule's criteria for retroactive application.
    pub fn get_emails_matching_rule(&self, account_id: &str) -> Result<Vec<(String, String, String)>> {
        let conn = self.reader();
        let mut stmt = conn.prepare("SELECT id, sender_email, subject FROM emails WHERE account_id = ?1")?;
        let rows = stmt
            .query_map(params![account_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }
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
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn row_to_attachment(row: &rusqlite::Row) -> rusqlite::Result<Attachment> {
    let tags_json: String = row.get(9)?;
    let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
    Ok(Attachment {
        id: row.get(0)?,
        account_id: row.get(1)?,
        email_id: row.get(2)?,
        rule_id: row.get(3)?,
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
