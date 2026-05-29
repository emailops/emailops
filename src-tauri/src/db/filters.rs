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
