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
    pub fn get_email_inbox_position(&self, account_id: &str, email_id: &str) -> Result<i32> {
        let conn = self.reader();
        let latest_predicate = latest_inbox_email_predicate("e");
        let position: i32 = conn.query_row(
            &format!(
                "WITH target AS (
                    SELECT rep.id, rep.timestamp
                    FROM emails rep
                    WHERE rep.account_id = ?1
                      AND rep.thread_id = (SELECT thread_id FROM emails WHERE id = ?2)
                      AND {}
                    LIMIT 1
                 )
             SELECT COUNT(*)
             FROM emails e, target
             WHERE e.account_id = ?1
               AND {}
               AND (
                   e.timestamp > target.timestamp
                   OR (e.timestamp = target.timestamp AND e.id > target.id)
               )",
                latest_inbox_email_predicate("rep"),
                latest_predicate,
            ),
            params![account_id, email_id],
            |row| row.get(0),
        )?;
        Ok(position)
    }
}
