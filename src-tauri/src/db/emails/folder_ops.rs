//! Email-row migrations backing in-app folder management (rename / delete /
//! move). These rewrite or remove `emails` rows together with every table
//! that stores an email id, in one transaction, so the folder ops in
//! `services/emails/folders.rs` never leave dangling references.

use crate::db::Database;
use crate::models::error::{AppError, Result};
use rusqlite::params;

/// `(table, column)` pairs whose declared FOREIGN KEY references `emails`.
/// Discovered dynamically from the live schema so a future table with an
/// `email_id` FK is migrated automatically instead of silently missed.
fn tables_referencing_emails(conn: &rusqlite::Connection) -> Result<Vec<(String, String)>> {
    let mut tables: Vec<String> = Vec::new();
    {
        let mut stmt =
            conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for row in rows {
            tables.push(row?);
        }
    }
    let mut out = Vec::new();
    for table in tables {
        // Table names come from sqlite_master, not user input.
        let mut stmt = conn.prepare(&format!("PRAGMA foreign_key_list(\"{table}\")"))?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?;
        for row in rows {
            let (referenced, from_col) = row?;
            if referenced.eq_ignore_ascii_case("emails") {
                out.push((table.clone(), from_col));
            }
        }
    }
    Ok(out)
}

/// Tables that store email ids WITHOUT a declared FK (so the dynamic
/// discovery above cannot see them): the FTS index joins back via its
/// UNINDEXED `email_id`, and these two bookkeeping tables reference ids
/// loosely. `chat_messages.referenced_email_ids` (a JSON array) is left
/// stale on purpose — the renderer treats unknown ids as an empty allowlist.
const LOOSE_EMAIL_ID_TABLES: &[(&str, &str)] = &[
    ("emails_fts", "email_id"),
    ("sync_failed_emails", "email_id"),
    ("interaction_events", "email_id"),
];

fn rewrite_email_id_everywhere(tx: &rusqlite::Transaction<'_>, old_id: &str, new_id: &str) -> Result<()> {
    for (table, col) in tables_referencing_emails(tx)? {
        tx.execute(
            &format!("UPDATE \"{table}\" SET \"{col}\" = ?1 WHERE \"{col}\" = ?2"),
            params![new_id, old_id],
        )?;
    }
    for (table, col) in LOOSE_EMAIL_ID_TABLES {
        tx.execute(
            &format!("UPDATE \"{table}\" SET \"{col}\" = ?1 WHERE \"{col}\" = ?2"),
            params![new_id, old_id],
        )?;
    }
    Ok(())
}

impl Database {
    /// Re-key one email to a new id and mailbox, carrying every dependent row
    /// (tags, bodies, attachment meta, embeddings, FTS, …) along. Used after
    /// a provider-side move: the message gets a new provider id in its target
    /// folder but all local AI state must survive.
    ///
    /// `old_id == new_id` is valid and simply updates the mailbox.
    pub fn migrate_email_id(&self, old_id: &str, new_id: &str, new_mailbox: &str) -> Result<()> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        // The parent row is re-keyed before its children; enforcement waits
        // until COMMIT (the pragma auto-resets when the transaction ends).
        tx.pragma_update(None, "defer_foreign_keys", true)?;
        let updated = tx.execute(
            "UPDATE emails SET id = ?1, mailbox = ?2 WHERE id = ?3",
            params![new_id, new_mailbox, old_id],
        )?;
        if updated == 0 {
            return Err(AppError::NotFound(format!("Email not found: {old_id}")));
        }
        if old_id != new_id {
            rewrite_email_id_everywhere(&tx, old_id, new_id)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Migrate every email of one folder to a renamed folder in place:
    /// `mailbox` moves from `old_mailbox` to `new_mailbox`, and ids carrying
    /// `old_id_prefix` are re-prefixed with `new_id_prefix` (ids of another
    /// shape — e.g. optimistic local rows — keep their id and only change
    /// mailbox). Dependent rows follow the id rewrite. Returns the number of
    /// migrated emails.
    pub fn migrate_folder_emails(
        &self,
        account_id: &str,
        old_mailbox: &str,
        new_mailbox: &str,
        old_id_prefix: &str,
        new_id_prefix: &str,
    ) -> Result<u32> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        tx.pragma_update(None, "defer_foreign_keys", true)?;
        let migrated = tx.execute(
            "UPDATE emails SET
                 id = CASE WHEN substr(id, 1, length(?1)) = ?1
                           THEN ?2 || substr(id, length(?1) + 1)
                           ELSE id END,
                 mailbox = ?3
             WHERE account_id = ?4 AND mailbox = ?5",
            params![old_id_prefix, new_id_prefix, new_mailbox, account_id, old_mailbox],
        )?;
        // The id prefix embeds the account id, so a bare prefix match cannot
        // touch other accounts' rows.
        let mut prefix_targets = tables_referencing_emails(&tx)?;
        prefix_targets.extend(
            LOOSE_EMAIL_ID_TABLES
                .iter()
                .map(|(t, c)| ((*t).to_string(), (*c).to_string())),
        );
        for (table, col) in prefix_targets {
            tx.execute(
                &format!(
                    "UPDATE \"{table}\" SET \"{col}\" = ?2 || substr(\"{col}\", length(?1) + 1)
                     WHERE substr(\"{col}\", 1, length(?1)) = ?1"
                ),
                params![old_id_prefix, new_id_prefix],
            )?;
        }
        tx.commit()?;
        Ok(migrated as u32)
    }

    /// Hard-delete a single email and every dependent row (vec0 embeddings,
    /// loose bookkeeping tables, draft references). Used when a moved
    /// message's target row already exists locally and the stale source row
    /// must go. Idempotent.
    pub fn hard_delete_email(&self, email_id: &str) -> Result<()> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM vec_emails WHERE rowid IN (
                 SELECT rowid FROM embedding_chunks WHERE email_id = ?1)",
            params![email_id],
        )?;
        tx.execute(
            "UPDATE drafts SET email_id = NULL WHERE email_id = ?1",
            params![email_id],
        )?;
        tx.execute("DELETE FROM sync_failed_emails WHERE email_id = ?1", params![email_id])?;
        tx.execute("DELETE FROM interaction_events WHERE email_id = ?1", params![email_id])?;
        tx.execute("DELETE FROM emails WHERE id = ?1", params![email_id])?;
        tx.commit()?;
        Ok(())
    }

    /// Hard-delete every email in one mailbox of an account, including the
    /// rows FK cascades cannot reach (vec0 embeddings, loose bookkeeping
    /// tables) and nulling draft references. Returns the number of deleted
    /// emails. Used when a folder is deleted in-app — unlike the soft
    /// `is_deleted` flag, these messages are gone on the server too.
    pub fn delete_emails_in_mailbox(&self, account_id: &str, mailbox: &str) -> Result<u32> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        // vec0 virtual tables don't honor FK cascades — clean while
        // embedding_chunks still resolves (same pattern as delete_account).
        tx.execute(
            "DELETE FROM vec_emails WHERE rowid IN (
                 SELECT rowid FROM embedding_chunks WHERE email_id IN (
                     SELECT id FROM emails WHERE account_id = ?1 AND mailbox = ?2))",
            params![account_id, mailbox],
        )?;
        tx.execute(
            "UPDATE drafts SET email_id = NULL WHERE email_id IN (
                 SELECT id FROM emails WHERE account_id = ?1 AND mailbox = ?2)",
            params![account_id, mailbox],
        )?;
        tx.execute(
            "DELETE FROM sync_failed_emails WHERE email_id IN (
                 SELECT id FROM emails WHERE account_id = ?1 AND mailbox = ?2)",
            params![account_id, mailbox],
        )?;
        tx.execute(
            "DELETE FROM interaction_events WHERE email_id IN (
                 SELECT id FROM emails WHERE account_id = ?1 AND mailbox = ?2)",
            params![account_id, mailbox],
        )?;
        let deleted = tx.execute(
            "DELETE FROM emails WHERE account_id = ?1 AND mailbox = ?2",
            params![account_id, mailbox],
        )?;
        tx.commit()?;
        Ok(deleted as u32)
    }
}

#[cfg(test)]
mod tests {
    use crate::db::Database;
    use crate::models::Email;

    fn email(id: &str, account: &str, mailbox: &str) -> Email {
        Email {
            id: id.to_string(),
            account_id: account.to_string(),
            thread_id: format!("t-{id}"),
            message_id: Some(format!("<{id}@example.com>")),
            subject: "s".to_string(),
            sender: "Sender".to_string(),
            sender_email: "sender@example.com".to_string(),
            recipients: vec!["me@example.com".to_string()],
            cc: vec![],
            body: "body".to_string(),
            snippet: "body".to_string(),
            timestamp: 1_000,
            is_read: false,
            triage_status: None,
            category: "primary".to_string(),
            mailbox: mailbox.to_string(),
            is_sent: mailbox == "sent",
        }
    }

    fn tag_count_for(db: &Database, email_id: &str) -> i64 {
        db.reader()
            .query_row(
                "SELECT COUNT(*) FROM email_tags WHERE email_id = ?1",
                rusqlite::params![email_id],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn insert_tag(db: &Database, email_id: &str) {
        db.connection()
            .execute(
                "INSERT INTO email_tags (email_id, tag_type, tag_value, created_at)
                 VALUES (?1, 'topic', 'dental', 0)",
                rusqlite::params![email_id],
            )
            .unwrap();
    }

    #[test]
    fn migrate_email_id_rekeys_email_and_dependents() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.insert_emails_batch(&[email("acc-1::10", "acc-1", "inbox")]).unwrap();
        insert_tag(&db, "acc-1::10");

        db.migrate_email_id("acc-1::10", "acc-1::FOLDER::QQ::7", "folder:Archiv")
            .unwrap();

        assert!(db.get_email("acc-1::10").unwrap().is_none(), "old id gone");
        let migrated = db.get_email("acc-1::FOLDER::QQ::7").unwrap().expect("new id present");
        assert_eq!(migrated.mailbox, "folder:Archiv");
        assert_eq!(migrated.subject, "s");
        assert_eq!(tag_count_for(&db, "acc-1::10"), 0, "tag re-keyed away from old id");
        assert_eq!(tag_count_for(&db, "acc-1::FOLDER::QQ::7"), 1, "tag follows the email");
    }

    #[test]
    fn migrate_email_id_same_id_updates_mailbox_only() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.insert_emails_batch(&[email("acc-1::10", "acc-1", "inbox")]).unwrap();

        db.migrate_email_id("acc-1::10", "acc-1::10", "folder:Archiv").unwrap();

        let migrated = db.get_email("acc-1::10").unwrap().expect("row kept");
        assert_eq!(migrated.mailbox, "folder:Archiv");
    }

    #[test]
    fn migrate_email_id_missing_email_errors() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        assert!(db.migrate_email_id("nope", "new", "inbox").is_err());
    }

    #[test]
    fn migrate_folder_emails_rewrites_prefix_and_mailbox_scoped_to_account() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.seed_test_account("acc-2");
        db.insert_emails_batch(&[
            email("acc-1::FOLDER::b64old::1", "acc-1", "folder:Kunden"),
            email("acc-1::FOLDER::b64old::2", "acc-1", "folder:Kunden"),
            // Same-shape id in another account's folder must not move.
            email("acc-2::FOLDER::b64old::1", "acc-2", "folder:Kunden"),
            // Different mailbox in the same account must not move.
            email("acc-1::5", "acc-1", "inbox"),
        ])
        .unwrap();
        insert_tag(&db, "acc-1::FOLDER::b64old::1");

        let migrated = db
            .migrate_folder_emails(
                "acc-1",
                "folder:Kunden",
                "folder:Klienten",
                "acc-1::FOLDER::b64old::",
                "acc-1::FOLDER::b64new::",
            )
            .unwrap();

        assert_eq!(migrated, 2);
        let moved = db.get_email("acc-1::FOLDER::b64new::1").unwrap().expect("re-keyed");
        assert_eq!(moved.mailbox, "folder:Klienten");
        assert_eq!(tag_count_for(&db, "acc-1::FOLDER::b64new::1"), 1);
        let other_account = db.get_email("acc-2::FOLDER::b64old::1").unwrap().expect("untouched");
        assert_eq!(other_account.mailbox, "folder:Kunden");
        let inbox = db.get_email("acc-1::5").unwrap().expect("untouched");
        assert_eq!(inbox.mailbox, "inbox");
    }

    #[test]
    fn hard_delete_email_removes_row_and_dependents() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.insert_emails_batch(&[email("acc-1::9", "acc-1", "inbox")]).unwrap();
        insert_tag(&db, "acc-1::9");

        db.hard_delete_email("acc-1::9").unwrap();

        assert!(db.get_email("acc-1::9").unwrap().is_none());
        assert_eq!(tag_count_for(&db, "acc-1::9"), 0);
        // Idempotent on a missing id.
        db.hard_delete_email("acc-1::9").unwrap();
    }

    #[test]
    fn delete_emails_in_mailbox_hard_deletes_with_dependents() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.insert_emails_batch(&[
            email("acc-1::FOLDER::x::1", "acc-1", "folder:Alt"),
            email("acc-1::7", "acc-1", "inbox"),
        ])
        .unwrap();
        insert_tag(&db, "acc-1::FOLDER::x::1");

        let deleted = db.delete_emails_in_mailbox("acc-1", "folder:Alt").unwrap();

        assert_eq!(deleted, 1);
        assert!(db.get_email("acc-1::FOLDER::x::1").unwrap().is_none());
        assert_eq!(tag_count_for(&db, "acc-1::FOLDER::x::1"), 0, "children cascaded");
        assert!(db.get_email("acc-1::7").unwrap().is_some(), "other mailbox untouched");
    }
}
