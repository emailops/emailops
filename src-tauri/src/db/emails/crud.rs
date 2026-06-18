use super::*;

impl Database {
    pub fn insert_email(&self, email: &Email) -> Result<()> {
        let conn = self.connection();
        let recipients_json = serde_json::to_string(&email.recipients)?;
        let cc_json = serde_json::to_string(&email.cc)?;
        let sender_domain = extract_sender_domain(&email.sender_email);
        let now = chrono::Utc::now().timestamp();

        let mailbox = normalize_mailbox(&email.mailbox);
        conn.execute(
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
        conn.execute(
            "INSERT OR REPLACE INTO email_bodies (email_id, body) VALUES (?1, ?2)",
            params![email.id, email.body],
        )?;
        Ok(())
    }

    /// Get emails with keyset pagination.
    /// `cursor` is (timestamp, id) of the last email from the previous page.
    /// Pass None for the first page.
    ///
    /// `mailbox` selects which server-side mailbox to list:
    /// - `None` or `Some("inbox")` → `mailbox='inbox' AND is_deleted=0` (default)
    /// - `Some("sent")` → `sender_email` matches this account's own address
    ///   (provider-agnostic — Gmail's `in:sent` inbox filter stores sent items with
    ///   `mailbox='inbox'`, so relying on the mailbox column alone would miss them)
    /// - `Some("spam")` → `mailbox='spam' AND is_deleted=0`
    /// - `Some("deleted")` → `mailbox='trash' OR is_deleted=1`
    ///   (union: provider Trash + in-app soft-deletes)
    ///
    /// `category`, when set, additionally restricts the result to that Gmail
    /// category (`primary`/`social`/`promotions`/`updates`/`forums`).
    pub fn get_emails(
        &self,
        account_id: &str,
        limit: i32,
        offset: i32,
        cursor: Option<(i64, &str)>,
        mailbox: Option<&str>,
        category: Option<&str>,
    ) -> Result<Vec<Email>> {
        let conn = self.reader();
        let order_clause = thread_order_clause("e", false);

        let mut conditions = vec![format!("e.account_id = ?1")];
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(account_id.to_string())];
        let mut param_idx = 2;

        // Mailbox scope. The "deleted" view is a union of provider Trash and
        // app-side soft-deletes; all other views exclude soft-deleted rows.
        let view = mailbox.unwrap_or("inbox");
        match view {
            "deleted" => {
                // Union of provider Trash and in-app soft-deletes. Flat list — no
                // thread dedup (trash items are shown individually, like Gmail).
                conditions.push("(e.mailbox = 'trash' OR e.is_deleted = 1)".to_string());
            }
            "sent" => {
                // Match emails whose sender matches the account's own address.
                // Works regardless of whether the email was ingested via the
                // inbox label filter (mailbox='inbox') or the sent mailbox
                // pass (mailbox='sent'). Excludes spam/trash copies.
                conditions.push("e.is_deleted = 0".to_string());
                conditions.push("e.mailbox NOT IN ('spam', 'trash')".to_string());
                conditions
                    .push("LOWER(e.sender_email) = (SELECT LOWER(email) FROM accounts WHERE id = ?1)".to_string());
            }
            "spam" => {
                // Flat per-email list scoped to the mailbox, ordered by timestamp.
                conditions.push("e.is_deleted = 0".to_string());
                conditions.push("e.mailbox = 'spam'".to_string());
            }
            _ => {
                // Inbox view: dedup by thread, picking the latest *inbox* email
                // per thread. Using the cross-mailbox predicate here causes
                // threads where the user replied to disappear (the Sent reply
                // wins the "latest" race but is then excluded by mailbox='inbox').
                conditions.push("e.is_deleted = 0".to_string());
                conditions.push("e.mailbox = 'inbox'".to_string());
                conditions.push(format!("({})", latest_inbox_email_predicate("e")));
            }
        }

        // Optional Gmail-category filter, applied on top of the mailbox scope.
        if let Some(cat) = category {
            conditions.push(format!("e.category = ?{}", param_idx));
            params_vec.push(Box::new(cat.to_string()));
            param_idx += 1;
        }

        // Keyset pagination: skip past the cursor point
        if let Some((ts, id)) = cursor {
            conditions.push(format!(
                "(e.timestamp < ?{} OR (e.timestamp = ?{} AND e.id < ?{}))",
                param_idx,
                param_idx,
                param_idx + 1
            ));
            params_vec.push(Box::new(ts));
            params_vec.push(Box::new(id.to_string()));
            param_idx += 2;
        } else if offset > 0 {
            // Fallback to OFFSET for backward compatibility
            // OFFSET fallback, no extra condition needed
        }

        let where_clause = conditions.join(" AND ");
        let sql = if cursor.is_some() || offset == 0 {
            format!(
                "SELECT {} FROM emails e WHERE {} ORDER BY {} LIMIT ?{}",
                EMAIL_COLUMNS, where_clause, order_clause, param_idx
            )
        } else {
            format!(
                "SELECT {} FROM emails e WHERE {} ORDER BY {} LIMIT ?{} OFFSET ?{}",
                EMAIL_COLUMNS,
                where_clause,
                order_clause,
                param_idx,
                param_idx + 1
            )
        };

        params_vec.push(Box::new(limit));
        if cursor.is_none() && offset > 0 {
            params_vec.push(Box::new(offset));
        }

        let mut stmt = conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

        let emails = stmt.query_map(params_refs.as_slice(), row_to_email)?;

        let mut result = Vec::new();
        for email in emails {
            result.push(email?);
        }

        Ok(result)
    }

    pub fn get_thread(&self, account_id: &str, thread_id: &str) -> Result<Vec<Email>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM emails WHERE account_id = ?1 AND thread_id = ?2 AND is_deleted = 0 ORDER BY timestamp ASC",
            EMAIL_COLUMNS
        ))?;

        let emails = stmt.query_map(params![account_id, thread_id], row_to_email)?;

        let mut result = Vec::new();
        for email in emails {
            result.push(email?);
        }

        Ok(result)
    }

    /// Fetch the body for a single email from the email_bodies table.
    /// Returns empty string if no row exists (email awaiting re-download).
    pub fn get_email_body(&self, email_id: &str) -> Result<String> {
        let conn = self.reader();
        match conn.query_row(
            "SELECT body FROM email_bodies WHERE email_id = ?1",
            params![email_id],
            |row| row.get::<_, String>(0),
        ) {
            Ok(body) => Ok(body),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(String::new()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_email_by_id(&self, email_id: &str) -> Result<Option<Email>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM emails WHERE id = ?1 AND is_deleted = 0",
            EMAIL_COLUMNS
        ))?;
        let mut rows = stmt.query_map(params![email_id], row_to_email)?;
        match rows.next() {
            Some(Ok(email)) => Ok(Some(email)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// Soft-delete an email. The row is retained so `email_exists()` still
    /// returns true and the sync layer will never re-download it.
    pub fn delete_email(&self, email_id: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute("UPDATE emails SET is_deleted = 1 WHERE id = ?1", params![email_id])?;
        Ok(())
    }

    pub fn mark_as_read(&self, email_id: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute("UPDATE emails SET is_read = 1 WHERE id = ?1", params![email_id])?;
        Ok(())
    }

    pub fn update_triage_status(&self, email_id: &str, status: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE emails SET triage_status = ?1 WHERE id = ?2",
            params![status, email_id],
        )?;
        Ok(())
    }

    pub fn email_exists(&self, email_id: &str) -> Result<bool> {
        let conn = self.reader();
        let count: i32 = conn.query_row("SELECT COUNT(*) FROM emails WHERE id = ?1", params![email_id], |row| {
            row.get(0)
        })?;
        Ok(count > 0)
    }

    /// Check which of the given IDs already exist in the database — one query instead of N.
    /// Returns a HashSet of the IDs that are present (including soft-deleted rows, so the
    /// sync layer never re-downloads an email that was already processed).
    pub fn emails_exist_batch(&self, ids: &[String]) -> Result<std::collections::HashSet<String>> {
        if ids.is_empty() {
            return Ok(std::collections::HashSet::new());
        }
        let conn = self.reader();
        // SQLite's default SQLITE_MAX_VARIABLE_NUMBER is 999; chunk to stay safe.
        let mut existing = std::collections::HashSet::with_capacity(ids.len());
        for chunk in ids.chunks(900) {
            let placeholders: Vec<String> = (1..=chunk.len()).map(|i| format!("?{}", i)).collect();
            let sql = format!("SELECT id FROM emails WHERE id IN ({})", placeholders.join(", "));
            let mut stmt = conn.prepare(&sql)?;
            let params_vec: Vec<Box<dyn rusqlite::ToSql>> = chunk
                .iter()
                .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::ToSql>)
                .collect();
            let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(params_refs.as_slice(), |row| row.get::<_, String>(0))?;
            for row in rows {
                existing.insert(row?);
            }
        }
        Ok(existing)
    }

    pub fn get_email(&self, email_id: &str) -> Result<Option<Email>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!("SELECT {} FROM emails WHERE id = ?1", EMAIL_COLUMNS))?;
        let email = stmt.query_row(params![email_id], row_to_email);

        match email {
            Ok(email) => Ok(Some(email)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Count total visible threads in the inbox for an account.
    ///
    /// MUST mirror the WHERE clause used by `get_emails` for the default ("inbox")
    /// view (`mailbox = 'inbox' AND is_deleted = 0`). Frontend infinite-scroll
    /// derives `hasMore = emails.length < totalCount`; if this count includes
    /// Sent/Spam/Trash threads while `get_emails` returns only inbox rows,
    /// `hasMore` stays true forever and the inbox loops loadMore endlessly,
    /// appearing "stuck" with the spinner showing.
    pub fn count_emails(&self, account_id: &str) -> Result<i32> {
        let conn = self.reader();
        let count: i32 = conn.query_row(
            "SELECT COUNT(DISTINCT thread_id) FROM emails \
             WHERE account_id = ?1 AND is_deleted = 0 AND mailbox = 'inbox'",
            params![account_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Return IDs of emails that have no body content (NULL or empty string).
    /// Used to find emails that need to be re-downloaded.
    pub fn get_emails_with_empty_body(&self, account_id: &str) -> Result<Vec<String>> {
        let conn = self.reader();
        // An email is missing its body if there is no row in email_bodies or the body is empty.
        let mut stmt = conn.prepare(
            "SELECT e.id FROM emails e
             LEFT JOIN email_bodies eb ON eb.email_id = e.id
             WHERE e.account_id = ?1 AND e.is_deleted = 0
               AND (eb.email_id IS NULL OR eb.body = '')",
        )?;
        let ids = stmt.query_map(params![account_id], |row| row.get::<_, String>(0))?;
        Ok(ids.filter_map(|r| r.ok()).collect())
    }

    /// Get all emails for an account, without thread deduplication.
    /// Get emails by a list of IDs, preserving the order of IDs
    pub fn get_emails_by_ids(&self, email_ids: &[String]) -> Result<Vec<Email>> {
        if email_ids.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.reader();

        // Build placeholders for IN clause
        let placeholders: Vec<String> = (1..=email_ids.len()).map(|i| format!("?{}", i)).collect();
        let sql = format!(
            "SELECT {}
             FROM emails WHERE id IN ({})",
            EMAIL_COLUMNS,
            placeholders.join(", ")
        );

        let mut stmt = conn.prepare(&sql)?;

        // Convert to ToSql refs
        let params_vec: Vec<Box<dyn rusqlite::ToSql>> = email_ids
            .iter()
            .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::ToSql>)
            .collect();
        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

        let emails = stmt.query_map(params_refs.as_slice(), row_to_email)?;

        // Collect results and preserve order based on input IDs
        let mut email_map: std::collections::HashMap<String, Email> = std::collections::HashMap::new();
        for email in emails {
            let e = email?;
            email_map.insert(e.id.clone(), e);
        }

        // Return in the order of the input IDs
        let result: Vec<Email> = email_ids.iter().filter_map(|id| email_map.remove(id)).collect();

        Ok(result)
    }

    /// Get the most recent email timestamp for an account (seconds since epoch)
    pub fn get_latest_email_timestamp(&self, account_id: &str) -> Result<Option<i64>> {
        let conn = self.reader();
        let result = conn.query_row(
            "SELECT MAX(timestamp) FROM emails WHERE account_id = ?1",
            params![account_id],
            |row| row.get(0),
        );
        match result {
            Ok(ts) => Ok(ts),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get the most recent email timestamp for one mailbox of an account
    /// (seconds since epoch). Used as the incremental watermark for that
    /// mailbox's sync — scoping by mailbox prevents a locally stored sent
    /// email from poisoning the inbox watermark and causing recently
    /// received emails to be silently filtered out at the provider.
    pub fn get_latest_email_timestamp_for_mailbox(&self, account_id: &str, mailbox: &str) -> Result<Option<i64>> {
        let conn = self.reader();
        let result = conn.query_row(
            "SELECT MAX(timestamp) FROM emails
             WHERE account_id = ?1 AND mailbox = ?2 AND is_deleted = 0",
            params![account_id, mailbox],
            |row| row.get(0),
        );
        match result {
            Ok(ts) => Ok(ts),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get the oldest email timestamp for an account (seconds since epoch)
    pub fn get_oldest_email_timestamp(&self, account_id: &str) -> Result<Option<i64>> {
        let conn = self.reader();
        let result = conn.query_row(
            "SELECT MIN(timestamp) FROM emails WHERE account_id = ?1",
            params![account_id],
            |row| row.get(0),
        );
        match result {
            Ok(ts) => Ok(ts),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get the oldest email timestamp for one mailbox of an account.
    /// Used by the extra-mailbox backfill to set the `before_timestamp`
    /// window when walking backward through a mailbox's history.
    /// Excludes soft-deleted rows so they don't extend the backfill window
    /// past actual mailbox content.
    pub fn get_oldest_email_timestamp_for_mailbox(&self, account_id: &str, mailbox: &str) -> Result<Option<i64>> {
        let conn = self.reader();
        let result = conn.query_row(
            "SELECT MIN(timestamp) FROM emails
             WHERE account_id = ?1 AND mailbox = ?2 AND is_deleted = 0",
            params![account_id, mailbox],
            |row| row.get(0),
        );
        match result {
            Ok(ts) => Ok(ts),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Count emails stored in one mailbox for an account (excluding
    /// soft-deleted rows). Used to report the delta inserted by
    /// `resync_mailbox_full` to the user.
    pub fn count_emails_in_mailbox(&self, account_id: &str, mailbox: &str) -> Result<i64> {
        let conn = self.reader();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM emails
             WHERE account_id = ?1 AND mailbox = ?2 AND is_deleted = 0",
            params![account_id, mailbox],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Minimum timestamp across a set of email IDs.
    ///
    /// The extra-mailbox backfill uses this to advance its cursor backward
    /// when an entire fetched batch turned out to be duplicates already
    /// stored in the DB — we still know how far back the provider's window
    /// reached, just not via the freshly inserted rows. Returns `Ok(None)`
    /// when none of the IDs exist (e.g. all of the provider's refs were
    /// inserted in this same pass with `Err` results from `get_message` so
    /// none made it to the DB, in which case the caller should treat the
    /// page as fruitless and stop).
    pub fn get_min_timestamp_for_ids(&self, ids: &[String]) -> Result<Option<i64>> {
        if ids.is_empty() {
            return Ok(None);
        }
        let conn = self.reader();
        // Chunk the IN-list to stay well under SQLite's 32k parameter limit.
        const CHUNK: usize = 500;
        let mut overall_min: Option<i64> = None;
        for chunk in ids.chunks(CHUNK) {
            let placeholders = chunk
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 1))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!("SELECT MIN(timestamp) FROM emails WHERE id IN ({})", placeholders);
            let mut stmt = conn.prepare(&sql)?;
            let params_dyn: Vec<&dyn rusqlite::ToSql> = chunk.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
            let chunk_min: Option<i64> = stmt.query_row(params_dyn.as_slice(), |row| row.get(0)).unwrap_or(None);
            if let Some(ts) = chunk_min {
                overall_min = Some(overall_min.map_or(ts, |m| m.min(ts)));
            }
        }
        Ok(overall_min)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use crate::db::Database;

    #[test]
    fn deleted_email_excluded_from_get_emails() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_email(&db, "e1", account, "thread-a", 100);
        insert_email(&db, "e2", account, "thread-b", 200);

        // Soft-delete e1
        db.delete_email("e1").unwrap();

        let emails = db.get_emails(account, 50, 0, None, None, None).unwrap();
        let ids: Vec<&str> = emails.iter().map(|e| e.id.as_str()).collect();

        assert!(
            !ids.contains(&"e1"),
            "deleted email must not appear in get_emails, got: {:?}",
            ids
        );
        assert!(ids.contains(&"e2"), "non-deleted email must appear, got: {:?}", ids);
    }

    #[test]
    fn get_emails_filters_by_category() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_email_with_category(&db, "p1", account, "thread-a", 300, "primary");
        insert_email_with_category(&db, "promo1", account, "thread-b", 200, "promotions");
        insert_email_with_category(&db, "promo2", account, "thread-c", 100, "promotions");

        let promos = db.get_emails(account, 50, 0, None, None, Some("promotions")).unwrap();
        let ids: Vec<&str> = promos.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["promo1", "promo2"], "only promotions, newest first");

        let all = db.get_emails(account, 50, 0, None, None, None).unwrap();
        assert_eq!(all.len(), 3, "no category filter returns every inbox email");
    }

    #[test]
    fn deleted_email_excluded_from_get_thread() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_email(&db, "e1", account, "thread-a", 100);
        insert_email(&db, "e2", account, "thread-a", 200);

        // Soft-delete e1
        db.delete_email("e1").unwrap();

        let thread = db.get_thread(account, "thread-a").unwrap();
        let ids: Vec<&str> = thread.iter().map(|e| e.id.as_str()).collect();

        assert!(
            !ids.contains(&"e1"),
            "deleted email must not appear in get_thread, got: {:?}",
            ids
        );
        assert!(
            ids.contains(&"e2"),
            "non-deleted email must appear in thread, got: {:?}",
            ids
        );
    }

    #[test]
    fn email_exists_still_returns_true_for_deleted_email() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_email(&db, "e1", account, "thread-a", 100);
        db.delete_email("e1").unwrap();

        // email_exists must return true for deleted emails to prevent re-sync
        assert!(
            db.email_exists("e1").unwrap(),
            "email_exists must return true for deleted emails to prevent re-downloading"
        );
    }
}
