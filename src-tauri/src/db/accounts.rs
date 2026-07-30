use crate::db::Database;
use crate::models::error::Result;
use crate::models::{Account, SyncStatus};
use rusqlite::{params, OptionalExtension};

const ACCOUNT_COLUMNS: &str = "id, provider, email, name, created_at, sort_order, enabled, sync_from_timestamp";

/// `(imap_host, imap_port, imap_username, smtp_host, smtp_port)` as stored in
/// `imap_account_settings`. Aliased so the `get_imap_settings` return type
/// reads as `Option<ImapSettingsRow>` instead of a five-element tuple.
pub type ImapSettingsRow = (String, u16, String, String, u16);

fn row_to_account(row: &rusqlite::Row) -> rusqlite::Result<Account> {
    Ok(Account {
        id: row.get(0)?,
        provider: row.get(1)?,
        email: row.get(2)?,
        name: row.get(3)?,
        created_at: row.get(4)?,
        sort_order: row.get(5)?,
        enabled: row.get::<_, i32>(6)? != 0,
        sync_from_timestamp: row.get(7)?,
    })
}

impl Database {
    pub fn insert_account(&self, account: &Account) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO accounts (id, provider, email, name, created_at, sort_order, enabled, sync_from_timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                account.id,
                account.provider,
                account.email,
                account.name,
                account.created_at,
                account.sort_order,
                account.enabled as i32,
                account.sync_from_timestamp,
            ],
        )?;
        Ok(())
    }

    pub fn get_account(&self, id: &str) -> Result<Option<Account>> {
        let conn = self.reader();
        let sql = format!("SELECT {} FROM accounts WHERE id = ?1", ACCOUNT_COLUMNS);
        let mut stmt = conn.prepare(&sql)?;

        let account = stmt.query_row(params![id], row_to_account);

        match account {
            Ok(a) => Ok(Some(a)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list_accounts(&self) -> Result<Vec<Account>> {
        let conn = self.reader();
        let sql = format!(
            "SELECT {} FROM accounts ORDER BY sort_order ASC, created_at DESC",
            ACCOUNT_COLUMNS
        );
        let mut stmt = conn.prepare(&sql)?;

        let accounts = stmt.query_map([], row_to_account)?;

        let mut result = Vec::new();
        for account in accounts {
            result.push(account?);
        }

        Ok(result)
    }

    pub fn delete_account(&self, id: &str) -> Result<()> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;

        // vec0 virtual tables don't honor FOREIGN KEY ON DELETE CASCADE.
        // Clean their rows up *before* the parent rows go away, while
        // embedding_chunks / memory_fact_chunks still resolve.
        tx.execute(
            "DELETE FROM vec_emails WHERE rowid IN (
                 SELECT rowid FROM embedding_chunks WHERE account_id = ?1
             )",
            params![id],
        )?;
        tx.execute(
            "DELETE FROM vec_memory_facts WHERE rowid IN (
                 SELECT rowid FROM memory_fact_chunks
                 WHERE fact_id IN (SELECT id FROM memory_facts WHERE account_id = ?1)
             )",
            params![id],
        )?;

        // Tables that reference accounts(id) without ON DELETE CASCADE must be
        // cleaned up explicitly. Deleting `emails` cascades to email_bodies,
        // email_extraction_status, embedding_chunks, email_tags, attachments,
        // email_attachment_meta, chat_message_sources via FOREIGN KEYs on
        // email_id.
        // `drafts` MUST go before `emails`: a reply draft's `drafts.email_id`
        // references `emails(id)` with no ON DELETE action, so deleting the
        // emails first raises FOREIGN KEY constraint failed and rolls the whole
        // transaction back — the account becomes permanently undeletable.
        tx.execute("DELETE FROM drafts WHERE account_id = ?1", params![id])?;
        tx.execute("DELETE FROM emails WHERE account_id = ?1", params![id])?;
        tx.execute("DELETE FROM sync_state WHERE account_id = ?1", params![id])?;

        // Dev-mode credential stores have no FK; delete explicitly.
        tx.execute("DELETE FROM dev_tokens WHERE account_id = ?1", params![id])?;
        tx.execute("DELETE FROM dev_imap_creds WHERE account_id = ?1", params![id])?;

        // Final delete cascades the remaining account-scoped tables:
        // attachments, attachment_rules, classification_rules, smart_filter_prefs,
        // smart_filter_suggestions, sync_failed_emails, email_attachment_meta,
        // chat_conversations -> chat_messages -> chat_message_sources,
        // memory_facts -> memory_fact_chunks, thread_states, pending_tasks,
        // interaction_events, tag_priority.
        tx.execute("DELETE FROM accounts WHERE id = ?1", params![id])?;

        tx.commit()?;

        Ok(())
    }

    pub fn account_exists_by_email(&self, email: &str) -> Result<bool> {
        let conn = self.reader();
        let count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM accounts WHERE email = ?1",
            params![email],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn update_account_order(&self, account_ids: &[String]) -> Result<()> {
        let conn = self.connection();
        let mut stmt = conn.prepare("UPDATE accounts SET sort_order = ?1 WHERE id = ?2")?;
        for (idx, id) in account_ids.iter().enumerate() {
            stmt.execute(params![idx as i32, id])?;
        }
        Ok(())
    }

    pub fn update_account_enabled(&self, account_id: &str, enabled: bool) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE accounts SET enabled = ?1 WHERE id = ?2",
            params![enabled as i32, account_id],
        )?;
        Ok(())
    }

    pub fn update_account_sync_from(&self, account_id: &str, sync_from_timestamp: Option<i64>) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE accounts SET sync_from_timestamp = ?1 WHERE id = ?2",
            params![sync_from_timestamp, account_id],
        )?;
        Ok(())
    }

    /// Backfill progress, kept separate from the user's `sync_from_timestamp`
    /// preference. See `V017__accounts_backfill_swept_from.sql` for semantics.
    pub fn get_account_backfill_swept_from(&self, account_id: &str) -> Result<Option<i64>> {
        let conn = self.reader();
        let swept = conn
            .query_row(
                "SELECT backfill_swept_from FROM accounts WHERE id = ?1",
                params![account_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten();
        Ok(swept)
    }

    /// Record that the backfill swept from `swept_from` upward and found
    /// nothing new. Never touches `sync_from_timestamp`.
    pub fn set_account_backfill_swept_from(&self, account_id: &str, swept_from: Option<i64>) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "UPDATE accounts SET backfill_swept_from = ?1 WHERE id = ?2",
            params![swept_from, account_id],
        )?;
        Ok(())
    }

    // Dev-mode token storage (SQLite instead of OS keychain)

    pub fn store_dev_tokens(&self, account_id: &str, tokens_json: &str) -> Result<()> {
        let conn = self.connection();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR REPLACE INTO dev_tokens (account_id, tokens_json, updated_at)
             VALUES (?1, ?2, ?3)",
            params![account_id, tokens_json, now],
        )?;
        Ok(())
    }

    pub fn get_dev_tokens(&self, account_id: &str) -> Result<Option<String>> {
        let conn = self.reader();
        let result = conn.query_row(
            "SELECT tokens_json FROM dev_tokens WHERE account_id = ?1",
            params![account_id],
            |row| row.get(0),
        );
        match result {
            Ok(json) => Ok(Some(json)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn delete_dev_tokens(&self, account_id: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute("DELETE FROM dev_tokens WHERE account_id = ?1", params![account_id])?;
        Ok(())
    }

    // Dev-mode IMAP credential storage (SQLite instead of OS keychain)

    pub fn store_dev_imap_creds(&self, account_id: &str, creds_json: &str) -> Result<()> {
        let conn = self.connection();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR REPLACE INTO dev_imap_creds (account_id, creds_json, updated_at)
             VALUES (?1, ?2, ?3)",
            params![account_id, creds_json, now],
        )?;
        Ok(())
    }

    pub fn get_dev_imap_creds(&self, account_id: &str) -> Result<Option<String>> {
        let conn = self.reader();
        let result = conn.query_row(
            "SELECT creds_json FROM dev_imap_creds WHERE account_id = ?1",
            params![account_id],
            |row| row.get(0),
        );
        match result {
            Ok(json) => Ok(Some(json)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn delete_dev_imap_creds(&self, account_id: &str) -> Result<()> {
        let conn = self.connection();
        conn.execute("DELETE FROM dev_imap_creds WHERE account_id = ?1", params![account_id])?;
        Ok(())
    }

    // ── IMAP server settings (non-secret; mirrors keychain blob in the DB) ──
    //
    // The password lives only in the keychain (or dev_imap_creds in dev mode).
    // These rows let the re-auth dialog pre-fill host/port/username/smtp-host/
    // smtp-port even when the keychain entry is missing, so the user only has
    // to re-enter their password.

    pub fn upsert_imap_settings(
        &self,
        account_id: &str,
        imap_host: &str,
        imap_port: u16,
        imap_username: &str,
        smtp_host: &str,
        smtp_port: u16,
    ) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO imap_account_settings
                 (account_id, imap_host, imap_port, imap_username, smtp_host, smtp_port)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(account_id) DO UPDATE SET
                 imap_host = excluded.imap_host,
                 imap_port = excluded.imap_port,
                 imap_username = excluded.imap_username,
                 smtp_host = excluded.smtp_host,
                 smtp_port = excluded.smtp_port",
            params![
                account_id,
                imap_host,
                imap_port as i64,
                imap_username,
                smtp_host,
                smtp_port as i64,
            ],
        )?;
        Ok(())
    }

    /// Returns `(host, port, username, smtp_host, smtp_port)` if a row exists.
    pub fn get_imap_settings(&self, account_id: &str) -> Result<Option<ImapSettingsRow>> {
        let conn = self.reader();
        let result = conn.query_row(
            "SELECT imap_host, imap_port, imap_username, smtp_host, smtp_port
             FROM imap_account_settings WHERE account_id = ?1",
            params![account_id],
            |row| {
                let imap_port_i64: i64 = row.get(1)?;
                let smtp_port_i64: i64 = row.get(4)?;
                Ok((
                    row.get::<_, String>(0)?,
                    imap_port_i64.clamp(0, u16::MAX as i64) as u16,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    smtp_port_i64.clamp(0, u16::MAX as i64) as u16,
                ))
            },
        );
        match result {
            Ok(t) => Ok(Some(t)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn upsert_sync_status(
        &self,
        account_id: &str,
        status: &str,
        last_sync_at: Option<i64>,
        error: Option<&str>,
    ) -> Result<()> {
        let conn = self.connection();
        conn.execute(
            "INSERT INTO sync_state (account_id, last_sync_at, status, error)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(account_id) DO UPDATE SET
               last_sync_at = COALESCE(excluded.last_sync_at, sync_state.last_sync_at),
               status = excluded.status,
               error = excluded.error",
            params![account_id, last_sync_at, status, error],
        )?;
        Ok(())
    }

    pub fn get_sync_status(&self, account_id: &str) -> Result<SyncStatus> {
        let conn = self.reader();
        let result = conn.query_row(
            "SELECT status, last_sync_at, error FROM sync_state WHERE account_id = ?1",
            params![account_id],
            |row| {
                Ok(SyncStatus {
                    account_id: account_id.to_string(),
                    status: row.get(0)?,
                    last_sync_at: row.get(1)?,
                    error: row.get(2)?,
                })
            },
        );

        match result {
            Ok(status) => Ok(status),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(SyncStatus {
                account_id: account_id.to_string(),
                status: "idle".to_string(),
                last_sync_at: None,
                error: None,
            }),
            Err(e) => Err(e.into()),
        }
    }
}
