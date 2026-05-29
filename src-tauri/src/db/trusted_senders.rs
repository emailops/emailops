use crate::db::Database;
use crate::models::error::Result;
use rusqlite::params;

impl Database {
    /// Add a sender to the per-account trusted-senders allowlist. Idempotent —
    /// re-adding the same sender refreshes `created_at` via INSERT OR REPLACE.
    /// Only call from explicit user actions (e.g. clicking "Always trust this
    /// sender" on the blocked-images banner).
    pub fn add_trusted_sender(&self, account_id: &str, sender_email: &str) -> Result<()> {
        let conn = self.connection();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR REPLACE INTO trusted_senders (account_id, sender_email, created_at)
             VALUES (?1, ?2, ?3)",
            params![account_id, sender_email.to_lowercase(), now],
        )?;
        Ok(())
    }

    /// Remove a sender from the allowlist. Idempotent.
    pub fn remove_trusted_sender(&self, account_id: &str, sender_email: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM trusted_senders WHERE account_id = ?1 AND sender_email = ?2",
            params![account_id, sender_email.to_lowercase()],
        )?;
        Ok(())
    }

    /// List every trusted sender for an account, newest first.
    pub fn list_trusted_senders(&self, account_id: &str) -> Result<Vec<String>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT sender_email FROM trusted_senders
             WHERE account_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![account_id], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Whether a sender is on the allowlist. Sender comparison is
    /// case-insensitive — addresses are stored lowercased on insert.
    pub fn is_sender_trusted(&self, account_id: &str, sender_email: &str) -> Result<bool> {
        let conn = self.reader();
        let result: std::result::Result<i64, rusqlite::Error> = conn.query_row(
            "SELECT 1 FROM trusted_senders WHERE account_id = ?1 AND sender_email = ?2",
            params![account_id, sender_email.to_lowercase()],
            |row| row.get(0),
        );
        match result {
            Ok(_) => Ok(true),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Database;

    #[test]
    fn add_then_check_returns_true() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");

        assert!(!db.is_sender_trusted("acc-1", "alice@example.com").unwrap());
        db.add_trusted_sender("acc-1", "alice@example.com").unwrap();
        assert!(db.is_sender_trusted("acc-1", "alice@example.com").unwrap());
    }

    #[test]
    fn check_is_case_insensitive() {
        // Email addresses are case-insensitive in practice; if the user trusts
        // "Alice@Example.com" we must still recognise "alice@example.com" on
        // the next email from that sender.
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");

        db.add_trusted_sender("acc-1", "Alice@Example.com").unwrap();
        assert!(db.is_sender_trusted("acc-1", "alice@example.com").unwrap());
        assert!(db.is_sender_trusted("acc-1", "ALICE@EXAMPLE.COM").unwrap());
    }

    #[test]
    fn remove_clears_trust() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");

        db.add_trusted_sender("acc-1", "bob@example.com").unwrap();
        db.remove_trusted_sender("acc-1", "bob@example.com").unwrap();
        assert!(!db.is_sender_trusted("acc-1", "bob@example.com").unwrap());
    }

    #[test]
    fn trust_is_scoped_per_account() {
        // Trusting a sender on account A must not load remote images for the
        // same sender on account B — privacy boundary is per-account.
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.seed_test_account("acc-2");

        db.add_trusted_sender("acc-1", "shared@example.com").unwrap();
        assert!(db.is_sender_trusted("acc-1", "shared@example.com").unwrap());
        assert!(!db.is_sender_trusted("acc-2", "shared@example.com").unwrap());
    }

    #[test]
    fn add_twice_is_idempotent() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");

        db.add_trusted_sender("acc-1", "a@b.com").unwrap();
        db.add_trusted_sender("acc-1", "a@b.com").unwrap();

        let all = db.list_trusted_senders("acc-1").unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0], "a@b.com");
    }

    #[test]
    fn list_returns_only_account_senders() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.seed_test_account("acc-2");

        db.add_trusted_sender("acc-1", "x@a.com").unwrap();
        db.add_trusted_sender("acc-2", "y@b.com").unwrap();

        let acc1 = db.list_trusted_senders("acc-1").unwrap();
        assert_eq!(acc1, vec!["x@a.com"]);
    }
}
