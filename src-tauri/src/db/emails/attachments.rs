use super::*;

impl Database {
    /// Returns file extension statistics for attachments in the account.
    /// Returns top 20 extensions ordered by distinct email count.
    pub fn get_attachment_ext_stats(&self, account_id: &str) -> Result<Vec<FilterSuggestion>> {
        let conn = self.reader();
        let mut stmt =
            conn.prepare("SELECT DISTINCT email_id, filename FROM email_attachment_meta WHERE account_id = ?1")?;
        let rows: Vec<(String, String)> = stmt
            .query_map(params![account_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Count distinct email_ids per extension
        let mut ext_emails: std::collections::HashMap<String, std::collections::HashSet<String>> =
            std::collections::HashMap::new();
        for (email_id, filename) in rows {
            if let Some(ext) = std::path::Path::new(&filename).extension() {
                let ext_str = ext.to_string_lossy().to_lowercase();
                if !ext_str.is_empty() && ext_str.len() <= 10 {
                    ext_emails.entry(ext_str).or_default().insert(email_id);
                }
            }
        }

        let mut stats: Vec<FilterSuggestion> = ext_emails
            .into_iter()
            .map(|(value, emails)| FilterSuggestion {
                value,
                count: emails.len() as i32,
            })
            .collect();
        stats.sort_by_key(|s| std::cmp::Reverse(s.count));
        stats.truncate(20);
        Ok(stats)
    }

    /// Insert multiple attachment meta records in a single transaction.
    /// Each tuple is (email_id, account_id, provider_attachment_id, filename, mime_type, file_size, inline_data).
    /// `inline_data` is Some(base64) for IMAP attachments whose bytes are embedded inline;
    /// it is None for Gmail attachments that must be re-fetched by provider_attachment_id.
    #[allow(clippy::type_complexity)]
    pub fn insert_email_attachment_metas_batch(
        &self,
        metas: &[(String, String, String, String, String, i64, Option<String>)],
    ) -> Result<()> {
        if metas.is_empty() {
            return Ok(());
        }
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        for (email_id, account_id, provider_attachment_id, filename, mime_type, file_size, inline_data) in metas {
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO email_attachment_meta
                 (id, email_id, account_id, provider_attachment_id, filename, mime_type, file_size, inline_data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(email_id, filename) DO NOTHING",
                params![
                    id,
                    email_id,
                    account_id,
                    provider_attachment_id,
                    filename,
                    mime_type,
                    file_size,
                    inline_data
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Fetch the inline_data (base64) stored for an IMAP attachment.
    /// Returns None if the attachment was not found or has no inline data.
    pub fn get_attachment_inline_data(&self, email_id: &str, filename: &str) -> Result<Option<String>> {
        let conn = self.reader();
        let result = conn.query_row(
            "SELECT inline_data FROM email_attachment_meta WHERE email_id = ?1 AND filename = ?2",
            params![email_id, filename],
            |row| row.get(0),
        );
        match result {
            Ok(data) => Ok(data),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Insert attachment metadata discovered during sync.
    /// ON CONFLICT DO NOTHING — safe to call repeatedly for the same email.
    pub fn insert_email_attachment_meta(
        &self,
        email_id: &str,
        account_id: &str,
        provider_attachment_id: &str,
        filename: &str,
        mime_type: &str,
        file_size: i64,
    ) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let conn = self.connection();
        conn.execute(
            "INSERT INTO email_attachment_meta
             (id, email_id, account_id, provider_attachment_id, filename, mime_type, file_size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(email_id, filename) DO NOTHING",
            params![
                id,
                email_id,
                account_id,
                provider_attachment_id,
                filename,
                mime_type,
                file_size
            ],
        )?;
        Ok(())
    }

    /// Update the local file_path once an attachment has been downloaded to disk.
    pub fn set_email_attachment_file_path(&self, email_id: &str, filename: &str, file_path: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE email_attachment_meta SET file_path = ?3 WHERE email_id = ?1 AND filename = ?2",
            params![email_id, filename, file_path],
        )?;
        Ok(())
    }

    pub fn get_email_attachment_metas_by_id(&self, id: &str) -> Result<Option<crate::models::EmailAttachmentMeta>> {
        let conn = self.reader();
        let result = conn.query_row(
            "SELECT id, email_id, account_id, provider_attachment_id, filename, mime_type, file_size, file_path
             FROM email_attachment_meta WHERE id = ?1",
            params![id],
            |row| {
                Ok(crate::models::EmailAttachmentMeta {
                    id: row.get(0)?,
                    email_id: row.get(1)?,
                    account_id: row.get(2)?,
                    provider_attachment_id: row.get(3)?,
                    filename: row.get(4)?,
                    mime_type: row.get(5)?,
                    file_size: row.get(6)?,
                    file_path: row.get(7)?,
                })
            },
        );
        match result {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_email_attachment_metas(&self, email_id: &str) -> Result<Vec<crate::models::EmailAttachmentMeta>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, email_id, account_id, provider_attachment_id, filename, mime_type, file_size, file_path
             FROM email_attachment_meta WHERE email_id = ?1 ORDER BY filename",
        )?;
        let rows = stmt.query_map(params![email_id], |row| {
            Ok(crate::models::EmailAttachmentMeta {
                id: row.get(0)?,
                email_id: row.get(1)?,
                account_id: row.get(2)?,
                provider_attachment_id: row.get(3)?,
                filename: row.get(4)?,
                mime_type: row.get(5)?,
                file_size: row.get(6)?,
                file_path: row.get(7)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }
}
