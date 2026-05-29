use super::*;

impl Database {
    /// Insert multiple emails in a single transaction. Uses INSERT OR REPLACE so
    /// duplicate IDs are overwritten. Each insert triggers the FTS update triggers
    /// within the same transaction, which is faster than N separate transactions.
    pub fn insert_emails_batch(&self, emails: &[Email]) -> Result<()> {
        if emails.is_empty() {
            return Ok(());
        }
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        let now = chrono::Utc::now().timestamp();
        for email in emails {
            let recipients_json = serde_json::to_string(&email.recipients)?;
            let cc_json = serde_json::to_string(&email.cc)?;
            let sender_domain = extract_sender_domain(&email.sender_email);
            let mailbox = normalize_mailbox(&email.mailbox);
            // Remove stale FTS entry before REPLACE (DELETE trigger may not fire
            // during INSERT OR REPLACE without recursive_triggers enabled).
            tx.execute("DELETE FROM emails_fts WHERE email_id = ?1", params![email.id])?;
            tx.execute(
                r#"INSERT OR REPLACE INTO emails
                   (id, account_id, thread_id, message_id, subject, sender, sender_email,
                    sender_domain, recipients_json, cc_json, snippet, timestamp, is_read, triage_status, category, mailbox, created_at)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)"#,
                params![
                    email.id,
                    email.account_id,
                    email.thread_id,
                    email.message_id,
                    email.subject,
                    email.sender,
                    email.sender_email,
                    sender_domain,
                    recipients_json,
                    cc_json,
                    email.snippet,
                    email.timestamp,
                    email.is_read as i32,
                    email.triage_status,
                    email.category,
                    mailbox,
                    now,
                ],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO email_bodies (email_id, body) VALUES (?1, ?2)",
                params![email.id, email.body],
            )?;
            // Manual FTS insert with stripped HTML (triggers removed)
            let body_text = strip_html_for_fts(&email.body);
            tx.execute(
                "INSERT INTO emails_fts(email_id, subject, sender, body) VALUES (?1, ?2, ?3, ?4)",
                params![email.id, email.subject, email.sender, body_text],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
