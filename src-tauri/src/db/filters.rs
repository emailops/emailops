use crate::db::Database;
use crate::models::error::Result;
use crate::models::{SmartFilterPref, SmartFilterSuggestion};
use rusqlite::params;

impl Database {
    pub fn get_filter_prefs(&self, account_id: &str) -> Result<Vec<SmartFilterPref>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, filter_type, filter_value, status, account_id
             FROM smart_filter_prefs WHERE account_id = ?1",
        )?;

        let prefs = stmt
            .query_map(params![account_id], |row| {
                Ok(SmartFilterPref {
                    id: row.get(0)?,
                    filter_type: row.get(1)?,
                    filter_value: row.get(2)?,
                    status: row.get(3)?,
                    account_id: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(prefs)
    }

    pub fn upsert_filter_pref(
        &self,
        id: &str,
        filter_type: &str,
        filter_value: &str,
        status: &str,
        account_id: &str,
    ) -> Result<()> {
        let conn = self.connection();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR REPLACE INTO smart_filter_prefs
             (id, filter_type, filter_value, status, account_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, filter_type, filter_value, status, account_id, now],
        )?;
        Ok(())
    }

    pub fn delete_filter_pref(&self, id: &str, account_id: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "DELETE FROM smart_filter_prefs WHERE id = ?1 AND account_id = ?2",
            params![id, account_id],
        )?;
        Ok(())
    }

    /// Save calculated suggestions, replacing any previous ones for the account
    pub fn save_filter_suggestions(&self, account_id: &str, suggestions: &[SmartFilterSuggestion]) -> Result<()> {
        let mut conn = self.connection();
        let now = chrono::Utc::now().timestamp();

        let tx = conn.transaction()?;

        tx.execute(
            "DELETE FROM smart_filter_suggestions WHERE account_id = ?1",
            params![account_id],
        )?;

        let mut stmt = tx.prepare(
            "INSERT INTO smart_filter_suggestions
             (id, filter_type, filter_value, count, account_id, calculated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;

        for s in suggestions {
            let id = format!("{}:{}:{}", account_id, s.filter_type, s.filter_value);
            stmt.execute(params![id, s.filter_type, s.filter_value, s.count, account_id, now])?;
        }

        drop(stmt);
        tx.commit()?;

        Ok(())
    }

    /// Thread count for a single filter value, scoped exactly like
    /// `get_filtered_emails` (inbox/sent, not deleted, case-insensitive sender
    /// match) so a pinned filter's sidebar count matches the rows shown when
    /// it is clicked. Non-domain/sender types are tag filters: a thread counts
    /// if ANY of its emails carries the tag.
    pub fn count_filter_threads(&self, account_id: &str, filter_type: &str, filter_value: &str) -> Result<i32> {
        let conn = self.reader();
        let count: i32 = match filter_type {
            "domain" => conn.query_row(
                "SELECT COUNT(DISTINCT thread_id) FROM emails
                 WHERE account_id = ?1 AND is_deleted = 0
                   AND mailbox IN ('inbox', 'sent')
                   AND sender_domain = ?2",
                params![account_id, filter_value.to_lowercase()],
                |row| row.get(0),
            )?,
            "sender" => conn.query_row(
                "SELECT COUNT(DISTINCT thread_id) FROM emails
                 WHERE account_id = ?1 AND is_deleted = 0
                   AND mailbox IN ('inbox', 'sent')
                   AND sender_email = ?2 COLLATE NOCASE",
                params![account_id, filter_value],
                |row| row.get(0),
            )?,
            _ => conn.query_row(
                "SELECT COUNT(DISTINCT e.thread_id)
                 FROM email_tags et
                 JOIN emails e ON e.id = et.email_id
                 WHERE et.tag_type = ?2 AND et.tag_value = ?3
                   AND e.account_id = ?1 AND e.is_deleted = 0
                   AND e.mailbox IN ('inbox', 'sent')",
                params![account_id, filter_type, filter_value],
                |row| row.get(0),
            )?,
        };
        Ok(count)
    }

    /// Load previously calculated suggestions for an account
    pub fn get_filter_suggestions(&self, account_id: &str) -> Result<Vec<SmartFilterSuggestion>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT filter_type, filter_value, count
             FROM smart_filter_suggestions WHERE account_id = ?1
             ORDER BY count DESC",
        )?;

        let suggestions = stmt
            .query_map(params![account_id], |row| {
                Ok(SmartFilterSuggestion {
                    filter_type: row.get(0)?,
                    filter_value: row.get(1)?,
                    count: row.get(2)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(suggestions)
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Database;
    use crate::models::SmartFilterSuggestion;

    fn make_suggestion(filter_type: &str, filter_value: &str, count: i32) -> SmartFilterSuggestion {
        SmartFilterSuggestion {
            filter_type: filter_type.to_string(),
            filter_value: filter_value.to_string(),
            count,
        }
    }

    fn insert_email(db: &Database, id: &str, account: &str, thread: &str, sender_email: &str, mailbox: &str) {
        let domain = sender_email.rsplit_once('@').map(|(_, d)| d.to_lowercase()).unwrap();
        db.connection()
            .execute(
                "INSERT INTO emails
                     (id, account_id, thread_id, subject, sender, sender_email, sender_domain,
                      recipients_json, cc_json, snippet, timestamp, is_read, category, mailbox, created_at)
                     VALUES (?1,?2,?3,'subj','sender',?4,?5,'[]','[]','snip',0,0,'primary',?6,0)",
                rusqlite::params![id, account, thread, sender_email, domain, mailbox],
            )
            .unwrap();
    }

    // ── count_filter_threads ──────────────────────────────────────────────────
    // Scoped exactly like get_filtered_emails (inbox/sent, not deleted, NOCASE
    // senders) so a pinned filter's count matches the rows shown on click.

    #[test]
    fn count_filter_threads_sender_is_case_insensitive_and_mailbox_scoped() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc");

        insert_email(&db, "e1", "acc", "t1", "Jorge@Logalty.com", "inbox");
        insert_email(&db, "e2", "acc", "t1", "jorge@logalty.com", "inbox"); // same thread
        insert_email(&db, "e3", "acc", "t2", "jorge@logalty.com", "inbox");
        insert_email(&db, "e4", "acc", "t3", "jorge@logalty.com", "spam"); // out of scope

        let count = db.count_filter_threads("acc", "sender", "jorge@logalty.com").unwrap();
        assert_eq!(count, 2, "2 inbox threads regardless of casing; spam excluded");
    }

    #[test]
    fn count_filter_threads_domain_counts_threads() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc");

        insert_email(&db, "e1", "acc", "t1", "a@aws.com", "inbox");
        insert_email(&db, "e2", "acc", "t1", "b@aws.com", "inbox"); // same thread
        insert_email(&db, "e3", "acc", "t2", "c@aws.com", "sent");

        let count = db.count_filter_threads("acc", "domain", "aws.com").unwrap();
        assert_eq!(count, 2, "threads, not emails");
    }

    #[test]
    fn count_filter_threads_tag_counts_threads_with_any_tagged_email() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc");

        insert_email(&db, "e1", "acc", "t1", "a@x.com", "inbox");
        insert_email(&db, "e2", "acc", "t2", "b@x.com", "inbox");
        db.connection()
            .execute(
                "INSERT INTO email_tags (email_id, tag_type, tag_value, confidence, created_at)
                     VALUES ('e1', 'company', 'Acme', NULL, 0)",
                [],
            )
            .unwrap();

        let count = db.count_filter_threads("acc", "company", "Acme").unwrap();
        assert_eq!(count, 1, "only the thread containing the tagged email");
    }

    // Regression test: two accounts sharing the same domain/sender value must not
    // conflict on the smart_filter_suggestions PRIMARY KEY.
    // Before the fix, id = "filter_type:filter_value" caused a UNIQUE constraint
    // failure when a second account tried to insert the same domain.
    #[test]
    fn save_filter_suggestions_two_accounts_same_domain() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("account-1");
        db.seed_test_account("account-2");

        let suggestions = vec![make_suggestion("domain", "gmail.com", 10)];

        db.save_filter_suggestions("account-1", &suggestions)
            .expect("account-1 suggestions should save without error");

        db.save_filter_suggestions("account-2", &suggestions)
            .expect("account-2 suggestions with same domain must not cause UNIQUE conflict");

        let acc1 = db.get_filter_suggestions("account-1").unwrap();
        let acc2 = db.get_filter_suggestions("account-2").unwrap();
        assert_eq!(acc1.len(), 1);
        assert_eq!(acc2.len(), 1);
        assert_eq!(acc1[0].filter_value, "gmail.com");
        assert_eq!(acc2[0].filter_value, "gmail.com");
    }

    // Saving suggestions for the same account twice must replace, not duplicate.
    #[test]
    fn save_filter_suggestions_replaces_previous_for_same_account() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc");

        db.save_filter_suggestions("acc", &[make_suggestion("domain", "old.com", 5)])
            .unwrap();
        db.save_filter_suggestions("acc", &[make_suggestion("domain", "new.com", 7)])
            .unwrap();

        let saved = db.get_filter_suggestions("acc").unwrap();
        assert_eq!(saved.len(), 1, "previous suggestions must be replaced");
        assert_eq!(saved[0].filter_value, "new.com");
    }
}
