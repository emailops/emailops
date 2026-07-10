use super::*;

impl Database {
    /// Autocomplete sender email addresses matching a prefix.
    /// Returns distinct sender_email values ordered by recency (most recent email wins).
    pub fn autocomplete_senders(&self, account_id: &str, prefix: &str, limit: i32) -> Result<Vec<(String, String)>> {
        let conn = self.reader();
        // LIKE pattern: case-insensitive prefix match on email or sender name
        let pattern = format!("%{}%", prefix.to_lowercase());
        let mut stmt = conn.prepare(
            "SELECT sender_email, sender, MAX(timestamp) as latest
             FROM emails
             WHERE account_id = ?1
               AND (LOWER(sender_email) LIKE ?2 OR LOWER(sender) LIKE ?2)
             GROUP BY sender_email
             ORDER BY latest DESC
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![account_id, pattern, limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Smart autocomplete for recipients — prioritizes same-domain matches and frequent contacts.
    /// Returns (email, name, is_domain_match) tuples.
    pub fn autocomplete_recipients(
        &self,
        account_id: &str,
        prefix: &str,
        context_domain: Option<&str>,
        limit: i32,
    ) -> Result<Vec<(String, String, bool)>> {
        let conn = self.reader();
        let pattern = format!("%{}%", prefix.to_lowercase());
        let domain_pattern = context_domain
            .map(|d| format!("%@{}", d.to_lowercase()))
            .unwrap_or_default();

        // Extract clean email addresses from recipients_json which may contain
        // "Name <email>" format or plain email strings.
        // Use CASE to extract the part between < and > if present, otherwise use as-is.
        let sql = "
            WITH all_contacts AS (
                SELECT LOWER(sender_email) AS email, sender AS name, timestamp
                FROM emails WHERE account_id = ?1
                UNION ALL
                SELECT LOWER(
                    CASE
                        WHEN INSTR(TRIM(je.value), '<') > 0
                        THEN SUBSTR(TRIM(je.value),
                                    INSTR(TRIM(je.value), '<') + 1,
                                    INSTR(TRIM(je.value), '>') - INSTR(TRIM(je.value), '<') - 1)
                        ELSE TRIM(je.value)
                    END
                ) AS email,
                CASE
                    WHEN INSTR(TRIM(je.value), '<') > 0
                    THEN TRIM(SUBSTR(TRIM(je.value), 1, INSTR(TRIM(je.value), '<') - 1))
                    ELSE ''
                END AS name,
                e.timestamp
                FROM emails e, json_each(e.recipients_json) je
                WHERE e.account_id = ?1 AND LENGTH(TRIM(je.value)) > 3
            )
            SELECT email, MAX(CASE WHEN name != '' THEN name ELSE '' END) AS name, COUNT(*) AS freq,
                   CASE WHEN ?4 != '' AND email LIKE ?4 THEN 1 ELSE 0 END AS domain_match
            FROM all_contacts
            WHERE email LIKE ?2 OR LOWER(name) LIKE ?2
            GROUP BY email
            ORDER BY domain_match DESC, freq DESC, MAX(timestamp) DESC
            LIMIT ?3
        ";

        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(params![account_id, pattern, limit, domain_pattern], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i32>(3)? != 0,
            ))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Get the 0-based position of an email in the inbox, grouped by thread recency.
    ///
    /// Must use the inbox-scoped predicate for consistency with the inbox view
    /// in `get_emails` — otherwise positions reference threads that the inbox
    /// view excludes (e.g. threads where the latest message is a Sent reply).
    ///
    /// Under [`AccountScope::AllEnabled`] the position is computed over the
    /// unified (all enabled accounts) inbox. The target thread is always
    /// resolved within the email's OWN account — thread ids are not globally
    /// unique, so a same-id thread in another account must not hijack the
    /// representative lookup.
    pub fn get_email_inbox_position(&self, scope: crate::db::AccountScope<'_>, email_id: &str) -> Result<i32> {
        let conn = self.reader();
        // The email id binds as ?1 in both variants; the single-account scope
        // appends its account id as ?2.
        let (list_scope_cond, account_param): (&str, Option<&str>) = match scope {
            crate::db::AccountScope::Account(id) => ("e.account_id = ?2", Some(id)),
            crate::db::AccountScope::AllEnabled => {
                ("e.account_id IN (SELECT id FROM accounts WHERE enabled = 1)", None)
            }
        };
        let sql = format!(
            "WITH target AS (
                SELECT rep.id, rep.timestamp
                FROM emails rep
                WHERE rep.account_id = (SELECT account_id FROM emails WHERE id = ?1)
                  AND rep.thread_id = (SELECT thread_id FROM emails WHERE id = ?1)
                  AND {rep_latest}
                LIMIT 1
             )
             SELECT COUNT(*)
             FROM emails e, target
             WHERE {list_scope_cond}
               AND {e_latest}
               AND (
                   e.timestamp > target.timestamp
                   OR (e.timestamp = target.timestamp AND e.id > target.id)
               )",
            rep_latest = latest_inbox_email_predicate("rep"),
            e_latest = latest_inbox_email_predicate("e"),
        );
        let position: i32 = match account_param {
            Some(id) => conn.query_row(&sql, params![email_id, id], |row| row.get(0))?,
            None => conn.query_row(&sql, params![email_id], |row| row.get(0))?,
        };
        Ok(position)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use crate::db::{AccountScope, Database};

    #[test]
    fn inbox_position_single_account_counts_newer_threads() {
        let db = Database::new_for_testing().unwrap();
        insert_email(&db, "e1", "acc1", "t1", 100);
        insert_email(&db, "e2", "acc1", "t2", 200);
        insert_email(&db, "e3", "acc1", "t3", 300);

        assert_eq!(
            db.get_email_inbox_position(AccountScope::Account("acc1"), "e3")
                .unwrap(),
            0
        );
        assert_eq!(
            db.get_email_inbox_position(AccountScope::Account("acc1"), "e1")
                .unwrap(),
            2
        );
    }

    #[test]
    fn inbox_position_all_enabled_counts_across_accounts() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "a1@ex.com");
        insert_account(&db, "acc2", "a2@ex.com");
        insert_account(&db, "acc3", "a3@ex.com");

        insert_email(&db, "e1", "acc1", "t1", 100);
        insert_email(&db, "e2", "acc2", "t2", 200);
        insert_email(&db, "e3", "acc1", "t3", 300);
        // Disabled account's threads must not shift positions.
        insert_email(&db, "e4", "acc3", "t4", 400);
        set_account_enabled(&db, "acc3", false);

        // Unified order: e3 (300), e2 (200), e1 (100).
        assert_eq!(db.get_email_inbox_position(AccountScope::AllEnabled, "e3").unwrap(), 0);
        assert_eq!(db.get_email_inbox_position(AccountScope::AllEnabled, "e2").unwrap(), 1);
        assert_eq!(db.get_email_inbox_position(AccountScope::AllEnabled, "e1").unwrap(), 2);
    }

    #[test]
    fn inbox_position_all_enabled_thread_collision_scopes_target_to_own_account() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "a1@ex.com");
        insert_account(&db, "acc2", "a2@ex.com");

        // Same thread_id string in both accounts. acc2's copy is newer, so it
        // must NOT be picked as the representative for acc1's email.
        insert_email(&db, "e1a", "acc1", "shared", 100);
        insert_email(&db, "e2a", "acc2", "shared", 300);
        insert_email(&db, "e-mid", "acc1", "t-mid", 200);

        // Unified order: e2a (300), e-mid (200), e1a (100).
        assert_eq!(db.get_email_inbox_position(AccountScope::AllEnabled, "e1a").unwrap(), 2);
    }
}
