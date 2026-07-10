use crate::db::{AccountScope, Database};
use crate::models::error::Result;
use crate::models::{SmartFilterPref, SmartFilterSuggestion};
use rusqlite::params;

/// Dedup/normalization key for filter prefs and suggestions. Sender addresses
/// are case-insensitive identifiers; every other value compares verbatim.
/// Mirrors `services::filters::suggestion_key` and the frontend's
/// `filterMatchKey`.
pub(crate) fn filter_pref_key(filter_type: &str, filter_value: &str) -> String {
    if filter_type == "sender" {
        format!("{}:{}", filter_type, filter_value.to_lowercase())
    } else {
        format!("{}:{}", filter_type, filter_value)
    }
}

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
    ///
    /// Under `AllEnabled`, counts are distinct `(account_id, thread_id)` pairs —
    /// thread ids are not globally unique across accounts.
    pub fn count_filter_threads(&self, scope: AccountScope<'_>, filter_type: &str, filter_value: &str) -> Result<i32> {
        let conn = self.reader();
        // Scope condition + params. Under Account the account id binds as the
        // LAST param so the filter-value params keep stable low indices.
        let (scope_cond, scope_cond_tagged, account_param): (&str, &str, Option<&str>) = match scope {
            AccountScope::Account(id) => ("account_id = ?2", "e.account_id = ?3", Some(id)),
            AccountScope::AllEnabled => (
                "account_id IN (SELECT id FROM accounts WHERE enabled = 1)",
                "e.account_id IN (SELECT id FROM accounts WHERE enabled = 1)",
                None,
            ),
        };
        let count: i32 = match filter_type {
            "domain" | "sender" => {
                let value_cond = if filter_type == "domain" {
                    "sender_domain = ?1"
                } else {
                    "sender_email = ?1 COLLATE NOCASE"
                };
                let value = if filter_type == "domain" {
                    filter_value.to_lowercase()
                } else {
                    filter_value.to_string()
                };
                let sql = format!(
                    "SELECT COUNT(*) FROM (SELECT DISTINCT account_id, thread_id FROM emails
                     WHERE {value_cond} AND {scope_cond} AND is_deleted = 0
                       AND mailbox IN ('inbox', 'sent'))"
                );
                match account_param {
                    Some(id) => conn.query_row(&sql, params![value, id], |row| row.get(0))?,
                    None => conn.query_row(&sql, params![value], |row| row.get(0))?,
                }
            }
            _ => {
                let sql = format!(
                    "SELECT COUNT(*) FROM (SELECT DISTINCT e.account_id, e.thread_id
                     FROM email_tags et
                     JOIN emails e ON e.id = et.email_id
                     WHERE et.tag_type = ?1 AND et.tag_value = ?2
                       AND {scope_cond_tagged} AND e.is_deleted = 0
                       AND e.mailbox IN ('inbox', 'sent'))"
                );
                match account_param {
                    Some(id) => conn.query_row(&sql, params![filter_type, filter_value, id], |row| row.get(0))?,
                    None => conn.query_row(&sql, params![filter_type, filter_value], |row| row.get(0))?,
                }
            }
        };
        Ok(count)
    }

    /// Aggregate saved suggestions across every enabled account (unified
    /// "All accounts" view): counts SUM per filter value, sender values merge
    /// case-insensitively (mirroring `filter_pref_key`).
    pub fn get_filter_suggestions_all_enabled(&self) -> Result<Vec<SmartFilterSuggestion>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT filter_type, MIN(filter_value), SUM(count) AS total
             FROM smart_filter_suggestions
             WHERE account_id IN (SELECT id FROM accounts WHERE enabled = 1)
             GROUP BY filter_type,
                      CASE WHEN filter_type = 'sender' THEN LOWER(filter_value) ELSE filter_value END
             ORDER BY total DESC",
        )?;

        let suggestions = stmt
            .query_map([], |row| {
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

    /// Union of filter prefs across every enabled account, deduplicated by
    /// filter key with precedence **pinned > removed**: a filter pinned in any
    /// account shows in the unified sidebar; a removed one is hidden only when
    /// no account pins it.
    pub fn get_filter_prefs_all_enabled(&self) -> Result<Vec<SmartFilterPref>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT p.id, p.filter_type, p.filter_value, p.status, p.account_id
             FROM smart_filter_prefs p
             WHERE p.account_id IN (SELECT id FROM accounts WHERE enabled = 1)",
        )?;
        let all: Vec<SmartFilterPref> = stmt
            .query_map([], |row| {
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

        let mut by_key: std::collections::HashMap<String, SmartFilterPref> = std::collections::HashMap::new();
        for pref in all {
            let key = filter_pref_key(&pref.filter_type, &pref.filter_value);
            match by_key.get(&key) {
                // Pinned wins over removed; first entry wins among equals.
                Some(existing) if existing.status == "pinned" || pref.status != "pinned" => {}
                _ => {
                    by_key.insert(key, pref);
                }
            }
        }
        Ok(by_key.into_values().collect())
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

        let count = db
            .count_filter_threads(crate::db::AccountScope::Account("acc"), "sender", "jorge@logalty.com")
            .unwrap();
        assert_eq!(count, 2, "2 inbox threads regardless of casing; spam excluded");
    }

    #[test]
    fn count_filter_threads_domain_counts_threads() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc");

        insert_email(&db, "e1", "acc", "t1", "a@aws.com", "inbox");
        insert_email(&db, "e2", "acc", "t1", "b@aws.com", "inbox"); // same thread
        insert_email(&db, "e3", "acc", "t2", "c@aws.com", "sent");

        let count = db
            .count_filter_threads(crate::db::AccountScope::Account("acc"), "domain", "aws.com")
            .unwrap();
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

        let count = db
            .count_filter_threads(crate::db::AccountScope::Account("acc"), "company", "Acme")
            .unwrap();
        assert_eq!(count, 1, "only the thread containing the tagged email");
    }

    // ── AllEnabled scope ─────────────────────────────────────────────────────

    fn set_enabled(db: &Database, account: &str, enabled: bool) {
        db.connection()
            .execute(
                "UPDATE accounts SET enabled = ?2 WHERE id = ?1",
                rusqlite::params![account, enabled as i32],
            )
            .unwrap();
    }

    #[test]
    fn count_filter_threads_all_enabled_spans_accounts_and_collisions() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc1");
        db.seed_test_account("acc2");
        db.seed_test_account("acc3");

        // Same thread_id string in two accounts → 2 distinct (account, thread) pairs.
        insert_email(&db, "e1", "acc1", "shared", "a@aws.com", "inbox");
        insert_email(&db, "e2", "acc2", "shared", "b@aws.com", "inbox");
        // Disabled account must not count.
        insert_email(&db, "e3", "acc3", "t3", "c@aws.com", "inbox");
        set_enabled(&db, "acc3", false);

        let count = db
            .count_filter_threads(crate::db::AccountScope::AllEnabled, "domain", "aws.com")
            .unwrap();
        assert_eq!(count, 2, "2 (account, thread) pairs; disabled account excluded");
    }

    #[test]
    fn suggestions_all_enabled_sums_counts_and_merges_sender_case_variants() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc1");
        db.seed_test_account("acc2");
        db.seed_test_account("acc3");

        db.save_filter_suggestions(
            "acc1",
            &[
                make_suggestion("domain", "acme.com", 3),
                make_suggestion("sender", "Alice@Ex.com", 2),
            ],
        )
        .unwrap();
        db.save_filter_suggestions(
            "acc2",
            &[
                make_suggestion("domain", "acme.com", 4),
                make_suggestion("sender", "alice@ex.com", 5),
            ],
        )
        .unwrap();
        // Disabled account's suggestions are excluded.
        db.save_filter_suggestions("acc3", &[make_suggestion("domain", "acme.com", 100)])
            .unwrap();
        set_enabled(&db, "acc3", false);

        let merged = db.get_filter_suggestions_all_enabled().unwrap();

        let acme = merged
            .iter()
            .find(|s| s.filter_type == "domain" && s.filter_value == "acme.com")
            .expect("acme.com must be present");
        assert_eq!(acme.count, 7, "domain counts sum across enabled accounts only");

        let alice: Vec<_> = merged
            .iter()
            .filter(|s| s.filter_type == "sender" && s.filter_value.eq_ignore_ascii_case("alice@ex.com"))
            .collect();
        assert_eq!(alice.len(), 1, "sender case variants must merge into one suggestion");
        assert_eq!(alice[0].count, 7, "merged sender count must sum both accounts");
    }

    #[test]
    fn prefs_all_enabled_pinned_beats_removed() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc1");
        db.seed_test_account("acc2");

        // acc1 removed acme.com, acc2 pinned it → unified shows it pinned.
        db.upsert_filter_pref("acc1:domain:acme.com", "domain", "acme.com", "removed", "acc1")
            .unwrap();
        db.upsert_filter_pref("acc2:domain:acme.com", "domain", "acme.com", "pinned", "acc2")
            .unwrap();
        // Removed everywhere → stays removed.
        db.upsert_filter_pref("acc1:domain:spam.com", "domain", "spam.com", "removed", "acc1")
            .unwrap();
        // Sender case variants dedup to one entry.
        db.upsert_filter_pref("acc1:sender:Bob@Ex.com", "sender", "Bob@Ex.com", "pinned", "acc1")
            .unwrap();
        db.upsert_filter_pref("acc2:sender:bob@ex.com", "sender", "bob@ex.com", "removed", "acc2")
            .unwrap();

        let prefs = db.get_filter_prefs_all_enabled().unwrap();

        let acme = prefs
            .iter()
            .find(|p| p.filter_type == "domain" && p.filter_value == "acme.com")
            .expect("acme.com pref must be present");
        assert_eq!(acme.status, "pinned", "pinned in any account must win over removed");

        let spam = prefs
            .iter()
            .find(|p| p.filter_type == "domain" && p.filter_value == "spam.com")
            .expect("spam.com pref must be present");
        assert_eq!(spam.status, "removed");

        let bob: Vec<_> = prefs
            .iter()
            .filter(|p| p.filter_type == "sender" && p.filter_value.eq_ignore_ascii_case("bob@ex.com"))
            .collect();
        assert_eq!(bob.len(), 1, "sender case variants must dedup");
        assert_eq!(bob[0].status, "pinned");
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
