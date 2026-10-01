//! Per-account email signatures (V033 `account_signatures`). Sanitizing and
//! validation live in `services::signatures`; this is storage only.

use crate::db::Database;
use crate::models::error::Result;
use crate::models::AccountSignature;
use rusqlite::{params, OptionalExtension};

impl Database {
    /// The stored signature of an account, `None` when it never saved one.
    pub fn get_account_signature(&self, account_id: &str) -> Result<Option<AccountSignature>> {
        let conn = self.reader();
        let row = conn
            .query_row(
                "SELECT account_id, html, use_for_new, use_for_replies, updated_at
                 FROM account_signatures WHERE account_id = ?1",
                params![account_id],
                |row| {
                    Ok(AccountSignature {
                        account_id: row.get(0)?,
                        html: row.get(1)?,
                        use_for_new: row.get(2)?,
                        use_for_replies: row.get(3)?,
                        updated_at: Some(row.get(4)?),
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Insert or replace an account's signature.
    pub fn upsert_account_signature(
        &self,
        account_id: &str,
        html: &str,
        use_for_new: bool,
        use_for_replies: bool,
        now: i64,
    ) -> Result<()> {
        self.connection().execute(
            "INSERT INTO account_signatures (account_id, html, use_for_new, use_for_replies, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(account_id) DO UPDATE SET
               html = excluded.html,
               use_for_new = excluded.use_for_new,
               use_for_replies = excluded.use_for_replies,
               updated_at = excluded.updated_at",
            params![account_id, html, use_for_new, use_for_replies, now],
        )?;
        Ok(())
    }
}
