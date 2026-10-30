use super::*;

impl Database {
    /// Record (or refresh) a failed email download so it can be retried on the next sync.
    /// If the email_id is already present, the record is left unchanged (use
    /// `increment_failed_email_retry` to bump the counter on subsequent failures).
    pub fn add_failed_email(&self, account_id: &str, email_id: &str, error: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let conn = self.connection();
        conn.execute(
            "INSERT INTO sync_failed_emails (email_id, account_id, failed_at, retry_count, last_error)
             VALUES (?1, ?2, ?3, 0, ?4)
             ON CONFLICT(email_id, account_id) DO NOTHING",
            params![email_id, account_id, now, error],
        )?;
        Ok(())
    }

    /// Return all failed email IDs for an account, ordered by when they first failed.
    /// Returns (email_id, retry_count, failed_at) tuples.
    pub fn get_failed_emails(&self, account_id: &str) -> Result<Vec<(String, i32, i64)>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT email_id, retry_count, failed_at FROM sync_failed_emails
             WHERE account_id = ?1
             ORDER BY failed_at ASC",
        )?;
        let rows = stmt.query_map(params![account_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?, row.get::<_, i64>(2)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Remove a failed email record (called after a successful retry).
    pub fn remove_failed_email(&self, account_id: &str, email_id: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM sync_failed_emails WHERE email_id = ?1 AND account_id = ?2",
            params![email_id, account_id],
        )?;
        Ok(())
    }

    /// Bump the retry counter and update the error message for a failed email.
    /// `failed_at` keeps the first failure: the give-up window runs from it.
    pub fn increment_failed_email_retry(&self, account_id: &str, email_id: &str, error: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE sync_failed_emails
             SET retry_count = retry_count + 1, last_error = ?3
             WHERE email_id = ?1 AND account_id = ?2",
            params![email_id, account_id, error],
        )?;
        Ok(())
    }

    /// Remove multiple failed-email records in a single transaction (called after successful retry or insert).
    pub fn remove_failed_emails_batch(&self, account_id: &str, email_ids: &[String]) -> Result<()> {
        if email_ids.is_empty() {
            return Ok(());
        }
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        for email_id in email_ids {
            tx.execute(
                "DELETE FROM sync_failed_emails WHERE email_id = ?1 AND account_id = ?2",
                params![email_id, account_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Database;

    fn database_with_account() -> Database {
        let db = Database::new_for_testing().unwrap();
        db.connection()
            .execute(
                "INSERT INTO accounts (id, provider, email, name, created_at)
                 VALUES ('acc-1', 'imap', 'a@example.com', 'A', 0)",
                [],
            )
            .unwrap();
        db
    }

    #[test]
    fn a_retry_keeps_the_time_of_the_first_failure() {
        let db = database_with_account();
        db.add_failed_email("acc-1", "e1", "boom").unwrap();
        db.connection()
            .execute("UPDATE sync_failed_emails SET failed_at = 1000", [])
            .unwrap();

        db.increment_failed_email_retry("acc-1", "e1", "boom again").unwrap();

        assert_eq!(
            db.get_failed_emails("acc-1").unwrap(),
            vec![("e1".to_string(), 1, 1000)]
        );
    }
}
