use super::*;

/// How a mailbox view scopes a query, shared by the list and the count so the
/// two can never describe different sets (the pagination `hasMore` comparison
/// depends on that identity).
pub(super) struct MailboxScopeSql {
    /// WHERE fragments, already qualified with the caller's column prefix.
    pub conditions: Vec<String>,
    /// Value to bind to the folder placeholder, for custom-folder views.
    pub folder_value: Option<String>,
    /// Whether the view collapses each thread to a single row (inbox only).
    pub thread_deduped: bool,
}

/// Build the scope for `view`. `prefix` qualifies every column (`"e."` for the
/// aliased list query, `"emails."` for the count); `folder_placeholder` is the
/// `?N` token the caller will bind `folder_value` to.
///
/// The "deleted" view is a union of provider Trash and app-side soft-deletes;
/// all other views exclude soft-deleted rows.
pub(super) fn mailbox_scope_sql(view: &str, prefix: &str, folder_placeholder: &str) -> MailboxScopeSql {
    let mut conditions: Vec<String> = Vec::new();
    let mut folder_value = None;
    let mut thread_deduped = false;

    match view {
        "deleted" => {
            // Flat list — no thread dedup (trash items show individually, like Gmail).
            conditions.push(format!("({prefix}mailbox = 'trash' OR {prefix}is_deleted = 1)"));
        }
        "sent" => {
            // Three ways an email qualifies as "sent":
            //   1. The provider marked it sent (`is_sent`) — the authoritative
            //      signal, and the only one that catches mail sent to yourself
            //      through a send-as alias (Gmail labels it INBOX *and* SENT,
            //      so `mailbox` says 'inbox' and the sender is the alias).
            //   2. It is filed under Sent (`mailbox='sent'`) — covers rows
            //      written before `is_sent` existed.
            //   3. Its sender matches the OWNING account's own address
            //      (correlated on account_id, so under AllEnabled mail between
            //      the user's own accounts is never misclassified as "sent" in
            //      the receiving account).
            // Excludes spam/trash copies.
            conditions.push(format!("{prefix}is_deleted = 0"));
            conditions.push(format!("{prefix}mailbox NOT IN ('spam', 'trash')"));
            conditions.push(format!(
                "({prefix}is_sent = 1 \
                 OR {prefix}mailbox = 'sent' \
                 OR EXISTS (SELECT 1 FROM accounts a WHERE a.id = {prefix}account_id \
                 AND LOWER(a.email) = LOWER({prefix}sender_email)))"
            ));
        }
        "spam" => {
            // Flat per-email list scoped to the mailbox, ordered by timestamp.
            conditions.push(format!("{prefix}is_deleted = 0"));
            conditions.push(format!("{prefix}mailbox = 'spam'"));
        }
        v if v.starts_with("folder:") => {
            // Custom IMAP folder view: flat per-email list (no thread dedup,
            // like spam), scoped to the exact folder mailbox value.
            // Served by idx_emails_account_mailbox.
            conditions.push(format!("{prefix}is_deleted = 0"));
            conditions.push(format!("{prefix}mailbox = {folder_placeholder}"));
            folder_value = Some(v.to_string());
        }
        _ => {
            conditions.push(format!("{prefix}is_deleted = 0"));
            conditions.push(format!("{prefix}mailbox = 'inbox'"));
            thread_deduped = true;
        }
    }

    MailboxScopeSql {
        conditions,
        folder_value,
        thread_deduped,
    }
}

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
                sender_domain, recipients_json, cc_json, snippet, timestamp, is_read, triage_status, category, mailbox, is_sent, created_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)"#,
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
                is_sent_flag(email, mailbox) as i32,
                now,
            ],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO email_bodies (email_id, body) VALUES (?1, ?2)",
            params![email.id, email.body],
        )?;
        Ok(())
    }

    /// Insert a locally constructed Sent copy at send time (optimistic insert).
    /// Mirrors `insert_emails_batch` — emails row + email_bodies + manual FTS
    /// row, all in one transaction — and additionally sets `pending_sync`.
    /// `pending_sync = true` marks a synthetic row (no provider id yet) that
    /// the sync reconciler will replace when the real Sent copy is ingested.
    pub fn insert_sent_email_local(&self, email: &Email, pending_sync: bool) -> Result<()> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        let recipients_json = serde_json::to_string(&email.recipients)?;
        let cc_json = serde_json::to_string(&email.cc)?;
        let sender_domain = extract_sender_domain(&email.sender_email);
        let mailbox = normalize_mailbox(&email.mailbox);
        let now = chrono::Utc::now().timestamp();

        // Remove stale FTS entry before REPLACE (DELETE trigger may not fire
        // during INSERT OR REPLACE without recursive_triggers enabled).
        tx.execute("DELETE FROM emails_fts WHERE email_id = ?1", params![email.id])?;
        tx.execute(
            r#"INSERT OR REPLACE INTO emails
               (id, account_id, thread_id, message_id, subject, sender, sender_email,
                sender_domain, recipients_json, cc_json, snippet, timestamp, is_read,
                triage_status, category, mailbox, is_sent, pending_sync, created_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)"#,
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
                is_sent_flag(email, mailbox) as i32,
                pending_sync as i32,
                now,
            ],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO email_bodies (email_id, body) VALUES (?1, ?2)",
            params![email.id, email.body],
        )?;
        let body_text = crate::util::html::strip_html_for_fts(&email.body);
        tx.execute(
            "INSERT INTO emails_fts(email_id, subject, sender, body) VALUES (?1, ?2, ?3, ?4)",
            params![email.id, email.subject, email.sender, body_text],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// All `pending_sync = 1` rows for an account (uses idx_emails_pending_sync).
    /// These are optimistic sent copies awaiting reconciliation with the
    /// provider's real Sent message.
    pub fn get_pending_sent_emails(&self, account_id: &str) -> Result<Vec<Email>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM emails WHERE account_id = ?1 AND pending_sync = 1",
            EMAIL_COLUMNS
        ))?;
        let emails = stmt.query_map(params![account_id], row_to_email)?;
        let mut result = Vec::new();
        for email in emails {
            result.push(email?);
        }
        Ok(result)
    }

    /// Hard-delete reconciled optimistic sent rows. Only rows still flagged
    /// `pending_sync = 1` are deleted — a stale or buggy reconciliation plan
    /// can never remove a real synced email. FK cascades drop the body /
    /// tags / attachment metas and the `emails_fts_delete` trigger removes
    /// the FTS row.
    pub fn delete_pending_sent_emails(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        for id in ids {
            tx.execute("DELETE FROM emails WHERE id = ?1 AND pending_sync = 1", params![id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Move an email to another thread. Used when a provider's Sent copy
    /// lands under a different thread_id than the local conversation (IMAP
    /// References-hash divergence, Outlook new-mail conversationId) — the
    /// incoming row adopts the local thread so the reply stays in the open
    /// thread view.
    pub fn update_email_thread_id(&self, email_id: &str, thread_id: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE emails SET thread_id = ?1 WHERE id = ?2",
            params![thread_id, email_id],
        )?;
        Ok(())
    }

    /// Un-flag pending optimistic rows older than `cutoff_ts` (seconds since
    /// epoch) so a row the reconciler never matched (e.g. an Outlook
    /// heuristic miss) eventually becomes a normal permanent email instead
    /// of lingering in reconciliation limbo. Returns the number of rows
    /// cleared.
    pub fn clear_stale_pending_sent(&self, account_id: &str, cutoff_ts: i64) -> Result<u32> {
        let conn = self.connection();
        let cleared = conn.execute(
            "UPDATE emails SET pending_sync = 0
             WHERE account_id = ?1 AND pending_sync = 1 AND timestamp < ?2",
            params![account_id, cutoff_ts],
        )?;
        Ok(cleared as u32)
    }

    /// Get emails with keyset pagination.
    /// `cursor` is (timestamp, id) of the last email from the previous page.
    /// Pass None for the first page.
    ///
    /// `scope` selects one account or all enabled accounts (unified inbox).
    /// Under [`AccountScope::AllEnabled`], thread dedup keys on
    /// `(account_id, thread_id)` — same-thread rows from different accounts
    /// each keep their own latest email.
    ///
    /// `mailbox` selects which server-side mailbox to list:
    /// - `None` or `Some("inbox")` → `mailbox='inbox' AND is_deleted=0` (default)
    /// - `Some("sent")` → `mailbox='sent'` (what the provider filed under Sent,
    ///   including send-as alias mail) OR `sender_email` matches the owning
    ///   account's own address (self-sent copies that Gmail also labels INBOX,
    ///   which the mailbox column files as `inbox`)
    /// - `Some("spam")` → `mailbox='spam' AND is_deleted=0`
    /// - `Some("deleted")` → `mailbox='trash' OR is_deleted=1`
    ///   (union: provider Trash + in-app soft-deletes)
    ///
    /// `category`, when set, additionally restricts the result to that Gmail
    /// category (`primary`/`social`/`promotions`/`updates`/`forums`).
    pub fn get_emails(
        &self,
        scope: crate::db::AccountScope<'_>,
        limit: i32,
        offset: i32,
        cursor: Option<(i64, &str)>,
        mailbox: Option<&str>,
        category: Option<&str>,
    ) -> Result<Vec<Email>> {
        let conn = self.reader();
        let order_clause = thread_order_clause("e", false);

        let mut conditions: Vec<String> = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let mut param_idx = 1;
        match scope {
            crate::db::AccountScope::Account(account_id) => {
                conditions.push(format!("e.account_id = ?{param_idx}"));
                params_vec.push(Box::new(account_id.to_string()));
                param_idx += 1;
            }
            crate::db::AccountScope::AllEnabled => {
                conditions.push("e.account_id IN (SELECT id FROM accounts WHERE enabled = 1)".to_string());
            }
        }

        // Mailbox scope, shared with `count_emails` so the list and the count
        // always describe the same set.
        let view = mailbox.unwrap_or("inbox");
        let scope_sql = mailbox_scope_sql(view, "e.", &format!("?{param_idx}"));
        conditions.extend(scope_sql.conditions);
        if let Some(folder) = scope_sql.folder_value {
            params_vec.push(Box::new(folder));
            param_idx += 1;
        }
        if scope_sql.thread_deduped {
            // Inbox view: dedup by thread, picking the latest *inbox* email
            // per thread. Using the cross-mailbox predicate here causes
            // threads where the user replied to disappear (the Sent reply
            // wins the "latest" race but is then excluded by mailbox='inbox').
            conditions.push(format!("({})", latest_inbox_email_predicate("e")));
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

    /// Flag rows the provider reports as sent, leaving every other column —
    /// `mailbox` above all — untouched. Returns how many rows changed.
    ///
    /// The sync skips messages it has already stored, so without this a row the
    /// inbox pass filed as 'inbox' (Gmail labels self-sent mail INBOX *and*
    /// SENT) could never learn it was sent. Only ever sets the flag: a row the
    /// provider once called sent does not stop being sent mail.
    pub fn mark_emails_sent(&self, ids: &[String]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        let mut updated = 0usize;
        // SQLite's default SQLITE_MAX_VARIABLE_NUMBER is 999; chunk to stay safe.
        for chunk in ids.chunks(900) {
            let placeholders: Vec<String> = (1..=chunk.len()).map(|i| format!("?{}", i)).collect();
            let sql = format!(
                "UPDATE emails SET is_sent = 1 WHERE is_sent = 0 AND id IN ({})",
                placeholders.join(", ")
            );
            let params_vec: Vec<Box<dyn rusqlite::ToSql>> = chunk
                .iter()
                .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::ToSql>)
                .collect();
            let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
            updated += tx.execute(&sql, params_refs.as_slice())?;
        }
        tx.commit()?;
        Ok(updated)
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

    /// Count total visible threads in the inbox for the given scope.
    ///
    /// MUST mirror the WHERE clause used by `get_emails` for the default ("inbox")
    /// view (`mailbox = 'inbox' AND is_deleted = 0`). Frontend infinite-scroll
    /// derives `hasMore = emails.length < totalCount`; if this count includes
    /// Sent/Spam/Trash threads while `get_emails` returns only inbox rows,
    /// `hasMore` stays true forever and the inbox loops loadMore endlessly,
    /// appearing "stuck" with the spinner showing.
    ///
    /// The `AllEnabled` variant counts distinct `(account_id, thread_id)` pairs —
    /// a plain `COUNT(DISTINCT thread_id)` would under-count when two accounts
    /// share a thread_id string (both CC'd on one provider thread), which would
    /// desync this count from `get_emails`' per-account thread dedup.
    /// Number of rows the `mailbox` view lists for `scope`. `None` means the
    /// inbox, which counts distinct threads because that view collapses each
    /// thread to one row; every other view is a flat per-email list.
    ///
    /// Must stay in lockstep with [`Database::get_emails`] — both derive their
    /// scope from [`mailbox_scope_sql`] — because the UI compares the length of
    /// the listed page against this count to decide whether to keep paging.
    pub fn count_emails(&self, scope: crate::db::AccountScope<'_>, mailbox: Option<&str>) -> Result<i32> {
        let conn = self.reader();

        let mut conditions: Vec<String> = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let mut param_idx = 1;
        match scope {
            crate::db::AccountScope::Account(account_id) => {
                conditions.push(format!("emails.account_id = ?{param_idx}"));
                params_vec.push(Box::new(account_id.to_string()));
                param_idx += 1;
            }
            crate::db::AccountScope::AllEnabled => {
                conditions.push("emails.account_id IN (SELECT id FROM accounts WHERE enabled = 1)".to_string());
            }
        }

        let scope_sql = mailbox_scope_sql(mailbox.unwrap_or("inbox"), "emails.", &format!("?{param_idx}"));
        conditions.extend(scope_sql.conditions);
        if let Some(folder) = scope_sql.folder_value {
            params_vec.push(Box::new(folder));
        }
        let where_clause = conditions.join(" AND ");

        let sql = match (scope_sql.thread_deduped, scope) {
            (true, crate::db::AccountScope::Account(_)) => {
                format!("SELECT COUNT(DISTINCT emails.thread_id) FROM emails WHERE {where_clause}")
            }
            // Thread ids are only unique per account, so dedup on the pair.
            (true, crate::db::AccountScope::AllEnabled) => format!(
                "SELECT COUNT(*) FROM (SELECT DISTINCT emails.account_id, emails.thread_id \
                 FROM emails WHERE {where_clause})"
            ),
            (false, _) => format!("SELECT COUNT(*) FROM emails WHERE {where_clause}"),
        };

        let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
        let count: i32 = conn.query_row(&sql, params_refs.as_slice(), |row| row.get(0))?;
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

    /// Distinct `category` values across an account's live inbox mail.
    ///
    /// This is what the account *demonstrably* files mail under, as opposed to
    /// what its saved settings say it should — see
    /// `services::accounts::available_categories`, which unions the two so a
    /// never-configured account still gets a tab per category it holds.
    /// Soft-deleted rows are excluded: a category with nothing left to show
    /// should not keep its tab.
    pub fn distinct_inbox_categories(&self, account_id: &str) -> Result<Vec<String>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT category FROM emails
             WHERE account_id = ?1 AND mailbox = 'inbox' AND is_deleted = 0",
        )?;
        let rows = stmt.query_map(params![account_id], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
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
    use crate::db::{AccountScope, Database};

    #[test]
    fn deleted_email_excluded_from_get_emails() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";

        insert_email(&db, "e1", account, "thread-a", 100);
        insert_email(&db, "e2", account, "thread-b", 200);

        // Soft-delete e1
        db.delete_email("e1").unwrap();

        let emails = db
            .get_emails(AccountScope::Account(account), 50, 0, None, None, None)
            .unwrap();
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

        let promos = db
            .get_emails(AccountScope::Account(account), 50, 0, None, None, Some("promotions"))
            .unwrap();
        let ids: Vec<&str> = promos.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["promo1", "promo2"], "only promotions, newest first");

        let all = db
            .get_emails(AccountScope::Account(account), 50, 0, None, None, None)
            .unwrap();
        assert_eq!(all.len(), 3, "no category filter returns every inbox email");
    }

    #[test]
    fn distinct_inbox_categories_reports_what_the_mailbox_actually_holds() {
        let db = Database::new_for_testing().unwrap();

        insert_email_with_category(&db, "p1", "acc1", "thread-a", 300, "primary");
        insert_email_with_category(&db, "p2", "acc1", "thread-b", 290, "primary");
        insert_email_with_category(&db, "u1", "acc1", "thread-c", 200, "updates");
        // Another account's mail must not leak into this account's tab strip.
        insert_email_with_category(&db, "f1", "acc2", "thread-d", 100, "forums");

        let mut got = db.distinct_inbox_categories("acc1").unwrap();
        got.sort();
        assert_eq!(got, vec!["primary".to_string(), "updates".to_string()]);
    }

    #[test]
    fn distinct_inbox_categories_ignores_deleted_mail() {
        // A category whose every message is in the trash has nothing to show,
        // so it must not earn a tab.
        let db = Database::new_for_testing().unwrap();

        insert_email_with_category(&db, "p1", "acc1", "thread-a", 300, "primary");
        insert_email_with_category(&db, "promo1", "acc1", "thread-b", 200, "promotions");
        db.delete_email("promo1").unwrap();

        let got = db.distinct_inbox_categories("acc1").unwrap();
        assert_eq!(got, vec!["primary".to_string()]);
    }

    #[test]
    fn distinct_inbox_categories_is_empty_for_an_account_with_no_mail() {
        let db = Database::new_for_testing().unwrap();
        assert!(db.distinct_inbox_categories("acc-never-synced").unwrap().is_empty());
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
    fn get_emails_all_enabled_merges_accounts_sorted_by_timestamp() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "a1@example.com");
        insert_account(&db, "acc2", "a2@example.com");

        insert_email(&db, "e1", "acc1", "t1", 100);
        insert_email(&db, "e2", "acc2", "t2", 300);
        insert_email(&db, "e3", "acc1", "t3", 200);

        let emails = db
            .get_emails(AccountScope::AllEnabled, 50, 0, None, None, None)
            .unwrap();
        let ids: Vec<&str> = emails.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["e2", "e3", "e1"], "merged across accounts, newest first");
    }

    #[test]
    fn get_emails_all_enabled_excludes_disabled_accounts() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "a1@example.com");
        insert_account(&db, "acc2", "a2@example.com");
        insert_email(&db, "e1", "acc1", "t1", 100);
        insert_email(&db, "e2", "acc2", "t2", 200);
        set_account_enabled(&db, "acc2", false);

        let emails = db
            .get_emails(AccountScope::AllEnabled, 50, 0, None, None, None)
            .unwrap();
        let ids: Vec<&str> = emails.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["e1"], "disabled account's emails must be excluded");
    }

    #[test]
    fn get_emails_all_enabled_sent_view_matches_each_accounts_own_address() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "a1@example.com");
        insert_account(&db, "acc2", "a2@example.com");

        // acc1's own sent mail — must appear.
        insert_contact_email(&db, "s1", "acc1", "t1", "Me", "a1@example.com", "[]", "sent", 300);
        // Mail in acc1's mailbox FROM acc2's address (the user's other account
        // wrote to this one) — received mail, must NOT appear as "sent".
        insert_contact_email(
            &db,
            "r1",
            "acc1",
            "t2",
            "Other Me",
            "a2@example.com",
            "[]",
            "inbox",
            200,
        );
        // acc2's own sent mail — must appear.
        insert_contact_email(&db, "s2", "acc2", "t3", "Me2", "a2@example.com", "[]", "sent", 100);

        let emails = db
            .get_emails(AccountScope::AllEnabled, 50, 0, None, Some("sent"), None)
            .unwrap();
        let ids: Vec<&str> = emails.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["s1", "s2"],
            "sent view must match each account's OWN address only"
        );
    }

    #[test]
    fn get_emails_sent_view_includes_send_as_alias_mail() {
        // Regression: mail the user sent through a Gmail "send-as" alias
        // carries the alias in sender_email, not the account address, so the
        // sender-match predicate alone dropped it from the Sent view even
        // though the provider filed it under Sent (mailbox='sent').
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "a1@example.com");

        // Sent from the account's own address — always matched.
        insert_contact_email(&db, "own", "acc1", "t1", "Me", "a1@example.com", "[]", "sent", 300);
        // Sent through a configured alias — provider filed it under Sent.
        insert_contact_email(&db, "alias", "acc1", "t2", "Me", "me@alias.example", "[]", "sent", 200);

        let emails = db
            .get_emails(AccountScope::Account("acc1"), 50, 0, None, Some("sent"), None)
            .unwrap();
        let ids: Vec<&str> = emails.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["own", "alias"],
            "provider-filed sent mail must appear regardless of the from address"
        );
    }

    #[test]
    fn get_emails_sent_view_excludes_received_mail_in_sent_labelled_thread() {
        // The mailbox column must not become a blanket "show everything"
        // escape hatch: received mail stays out of the Sent view.
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "a1@example.com");
        insert_contact_email(
            &db,
            "received",
            "acc1",
            "t1",
            "Someone",
            "someone@example.com",
            "[]",
            "inbox",
            100,
        );

        let emails = db
            .get_emails(AccountScope::Account("acc1"), 50, 0, None, Some("sent"), None)
            .unwrap();
        assert!(emails.is_empty(), "inbox mail from a third party is not sent mail");
    }

    #[test]
    fn get_emails_all_enabled_thread_id_collision_keeps_both_accounts_rows() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "a1@example.com");
        insert_account(&db, "acc2", "a2@example.com");

        // Same thread_id string in two accounts (both CC'd on one provider
        // thread). The inbox latest-per-thread dedup must key per account and
        // keep one row from EACH account.
        insert_email(&db, "e1a", "acc1", "shared-thread", 100);
        insert_email(&db, "e1b", "acc1", "shared-thread", 150);
        insert_email(&db, "e2a", "acc2", "shared-thread", 120);

        let emails = db
            .get_emails(AccountScope::AllEnabled, 50, 0, None, None, None)
            .unwrap();
        let ids: Vec<&str> = emails.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["e1b", "e2a"],
            "one latest-inbox row per (account, thread), not per thread"
        );
    }

    #[test]
    fn count_emails_all_enabled_counts_account_thread_pairs() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "a1@example.com");
        insert_account(&db, "acc2", "a2@example.com");
        insert_account(&db, "acc3", "a3@example.com");

        // acc1: two emails in one thread → 1; acc2: same thread_id string → 1 more.
        insert_email(&db, "e1", "acc1", "shared-thread", 100);
        insert_email(&db, "e2", "acc1", "shared-thread", 200);
        insert_email(&db, "e3", "acc2", "shared-thread", 150);
        // acc3 disabled — its thread must not count.
        insert_email(&db, "e4", "acc3", "t4", 300);
        set_account_enabled(&db, "acc3", false);

        assert_eq!(
            db.count_emails(AccountScope::AllEnabled, None).unwrap(),
            2,
            "counts distinct (account_id, thread_id) pairs across enabled accounts"
        );
        assert_eq!(
            db.count_emails(AccountScope::Account("acc1"), None).unwrap(),
            1,
            "single-account count unchanged"
        );
    }

    #[test]
    fn count_emails_counts_the_requested_mailbox_view() {
        // Regression: the count always counted inbox threads while the list was
        // scoped to a mailbox. In the Sent view the two disagreed, so the UI
        // either re-requested empty pages forever (few sent, many inbox) or
        // stopped paging early (many sent, few inbox).
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "a1@example.com");

        insert_email(&db, "i1", "acc1", "t1", 100);
        insert_email(&db, "i2", "acc1", "t2", 200);
        insert_contact_email(&db, "s1", "acc1", "t3", "Me", "a1@example.com", "[]", "sent", 300);
        insert_contact_email(&db, "s2", "acc1", "t4", "Me", "me@alias.example", "[]", "sent", 400);

        assert_eq!(
            db.count_emails(AccountScope::Account("acc1"), Some("sent")).unwrap(),
            2,
            "sent view counts sent mail, not inbox threads"
        );
        assert_eq!(
            db.count_emails(AccountScope::Account("acc1"), Some("inbox")).unwrap(),
            2,
            "inbox view still counts inbox threads"
        );
        assert_eq!(
            db.count_emails(AccountScope::Account("acc1"), None).unwrap(),
            2,
            "no mailbox argument keeps the historic inbox behaviour"
        );
    }

    #[test]
    fn count_emails_matches_the_number_of_rows_the_sent_view_lists() {
        // The count and the list must describe the same set — that identity is
        // what the pagination "hasMore" comparison relies on.
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "a1@example.com");
        insert_contact_email(&db, "s1", "acc1", "t1", "Me", "a1@example.com", "[]", "sent", 300);
        insert_contact_email(&db, "s2", "acc1", "t2", "Me", "me@alias.example", "[]", "sent", 200);
        insert_contact_email(&db, "spam1", "acc1", "t3", "X", "x@example.com", "[]", "spam", 150);
        insert_email(&db, "i1", "acc1", "t4", 100);

        let listed = db
            .get_emails(AccountScope::Account("acc1"), 50, 0, None, Some("sent"), None)
            .unwrap();
        let counted = db.count_emails(AccountScope::Account("acc1"), Some("sent")).unwrap();
        assert_eq!(counted as usize, listed.len());
    }

    #[test]
    fn get_emails_sent_view_includes_self_sent_alias_mail_labelled_inbox() {
        // Regression: Gmail labels mail you send to yourself INBOX *and* SENT.
        // The mailbox column records 'inbox' to keep the thread in the inbox
        // view, and when the message went out through a send-as alias the
        // sender is not the account address either — so it matched neither
        // branch of the Sent predicate and vanished from the Sent view.
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "a1@example.com");

        let mut self_sent = local_sent_email("self-alias", "acc1", "t1", 300);
        self_sent.sender_email = "me@alias.example".to_string();
        self_sent.recipients = vec!["a1@example.com".to_string()];
        self_sent.mailbox = "inbox".to_string();
        self_sent.is_sent = true;
        db.insert_email(&self_sent).unwrap();

        // A plain received email must still stay out of the Sent view.
        insert_contact_email(
            &db,
            "received",
            "acc1",
            "t2",
            "Someone",
            "someone@example.com",
            "[]",
            "inbox",
            200,
        );

        let ids: Vec<String> = db
            .get_emails(AccountScope::Account("acc1"), 50, 0, None, Some("sent"), None)
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, vec!["self-alias".to_string()]);
    }

    #[test]
    fn sent_view_and_count_agree_on_self_sent_alias_mail() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "a1@example.com");

        let mut self_sent = local_sent_email("self-alias", "acc1", "t1", 300);
        self_sent.sender_email = "me@alias.example".to_string();
        self_sent.mailbox = "inbox".to_string();
        self_sent.is_sent = true;
        db.insert_email(&self_sent).unwrap();

        let listed = db
            .get_emails(AccountScope::Account("acc1"), 50, 0, None, Some("sent"), None)
            .unwrap();
        let counted = db.count_emails(AccountScope::Account("acc1"), Some("sent")).unwrap();
        assert_eq!(counted as usize, listed.len(), "count must track the widened predicate");
    }

    #[test]
    fn mark_emails_sent_sets_the_flag_without_moving_the_message() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "a1@example.com");
        insert_contact_email(
            &db,
            "self-alias",
            "acc1",
            "t1",
            "Me",
            "me@alias.example",
            "[]",
            "inbox",
            300,
        );

        let updated = db.mark_emails_sent(&["self-alias".to_string()]).unwrap();
        assert_eq!(updated, 1);

        let row = db.get_email_by_id("self-alias").unwrap().expect("row");
        assert!(row.is_sent);
        assert_eq!(row.mailbox, "inbox", "flagging must not relocate the message");

        // Already flagged → nothing left to update, so a repeat pass is a no-op.
        assert_eq!(db.mark_emails_sent(&["self-alias".to_string()]).unwrap(), 0);
        assert_eq!(db.mark_emails_sent(&[]).unwrap(), 0);
    }

    #[test]
    fn insert_email_flags_sent_mailbox_rows_even_without_the_provider_signal() {
        // Providers that only know the folder (IMAP, Outlook) leave is_sent
        // false and rely on the mailbox value; the column must still be right.
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "a1@example.com");

        let mut email = local_sent_email("folder-sent", "acc1", "t1", 100);
        email.is_sent = false;
        email.mailbox = "sent".to_string();
        db.insert_email(&email).unwrap();

        let stored = db.get_email_by_id("folder-sent").unwrap().expect("row");
        assert!(stored.is_sent, "a row filed under Sent is sent mail");
    }

    fn local_sent_email(id: &str, account_id: &str, thread_id: &str, timestamp: i64) -> crate::models::Email {
        crate::models::Email {
            id: id.to_string(),
            account_id: account_id.to_string(),
            thread_id: thread_id.to_string(),
            message_id: Some(format!("<{}@local>", id)),
            subject: "Quarterly report".to_string(),
            sender: "Me".to_string(),
            sender_email: "me@example.com".to_string(),
            recipients: vec!["them@example.com".to_string()],
            cc: vec![],
            body: "Here is the <b>report</b> you asked for".to_string(),
            snippet: "Here is the report you asked for".to_string(),
            timestamp,
            is_read: true,
            triage_status: None,
            category: "primary".to_string(),
            mailbox: "sent".to_string(),
            is_sent: true,
            headers: None,
        }
    }

    #[test]
    fn insert_sent_email_local_writes_row_body_and_fts() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@example.com");

        let email = local_sent_email("local-sent-1", "acc1", "t1", 100);
        db.insert_sent_email_local(&email, false).unwrap();

        let stored = db.get_email("local-sent-1").unwrap().expect("row must exist");
        assert_eq!(stored.mailbox, "sent");
        assert_eq!(stored.recipients, vec!["them@example.com"]);
        assert_eq!(
            db.get_email_body("local-sent-1").unwrap(),
            "Here is the <b>report</b> you asked for"
        );

        // FTS row must exist (searchable body, HTML stripped).
        let fts_count: i32 = db
            .reader()
            .query_row(
                "SELECT COUNT(*) FROM emails_fts WHERE emails_fts MATCH 'report' AND email_id = 'local-sent-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_count, 1, "optimistic sent row must be FTS-indexed");
    }

    #[test]
    fn insert_sent_email_local_pending_flag_round_trips() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@example.com");

        db.insert_sent_email_local(&local_sent_email("pending-1", "acc1", "t1", 100), true)
            .unwrap();
        db.insert_sent_email_local(&local_sent_email("permanent-1", "acc1", "t2", 200), false)
            .unwrap();

        let pending = db.get_pending_sent_emails("acc1").unwrap();
        let ids: Vec<&str> = pending.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["pending-1"], "only pending_sync=1 rows are returned");
    }

    #[test]
    fn delete_pending_sent_emails_removes_row_body_and_fts() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@example.com");

        db.insert_sent_email_local(&local_sent_email("pending-1", "acc1", "t1", 100), true)
            .unwrap();
        db.delete_pending_sent_emails(&["pending-1".to_string()]).unwrap();

        assert!(db.get_email("pending-1").unwrap().is_none(), "row must be hard-deleted");
        assert_eq!(db.get_email_body("pending-1").unwrap(), "", "body row must cascade");
        let fts_count: i32 = db
            .reader()
            .query_row(
                "SELECT COUNT(*) FROM emails_fts WHERE email_id = 'pending-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts_count, 0, "FTS row must be removed by the delete trigger");
    }

    #[test]
    fn delete_pending_sent_emails_never_deletes_non_pending_rows() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@example.com");

        db.insert_sent_email_local(&local_sent_email("permanent-1", "acc1", "t1", 100), false)
            .unwrap();
        db.delete_pending_sent_emails(&["permanent-1".to_string()]).unwrap();

        assert!(
            db.get_email("permanent-1").unwrap().is_some(),
            "non-pending rows are protected from reconciliation deletes"
        );
    }

    #[test]
    fn update_email_thread_id_moves_email_between_threads() {
        let db = Database::new_for_testing().unwrap();
        let account = "acc1";
        insert_email(&db, "e1", account, "thread-a", 100);

        db.update_email_thread_id("e1", "thread-b").unwrap();

        assert!(db.get_thread(account, "thread-a").unwrap().is_empty());
        let thread_b: Vec<String> = db
            .get_thread(account, "thread-b")
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(thread_b, vec!["e1"]);
    }

    #[test]
    fn clear_stale_pending_sent_unflags_only_rows_older_than_cutoff() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@example.com");

        db.insert_sent_email_local(&local_sent_email("old-pending", "acc1", "t1", 100), true)
            .unwrap();
        db.insert_sent_email_local(&local_sent_email("fresh-pending", "acc1", "t2", 500), true)
            .unwrap();

        let cleared = db.clear_stale_pending_sent("acc1", 300).unwrap();
        assert_eq!(cleared, 1, "only the row older than the cutoff is unflagged");

        let pending: Vec<String> = db
            .get_pending_sent_emails("acc1")
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(pending, vec!["fresh-pending"]);
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
