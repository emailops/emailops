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

/// A stored row whose id lives in one IMAP mailbox's UID namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredUidRow {
    pub id: String,
    pub message_id: Option<String>,
    pub timestamp: i64,
}

/// SQL predicate: `col` is `prefix` followed by nothing but digits — an id of
/// exactly one IMAP mailbox. A bare prefix match is not enough: the inbox
/// prefix `{account}::` is also the start of every `{account}::SENT::…` id.
/// `?1` must be bound to the account id and `?2` to the prefix.
fn uid_namespace_sql(col: &str) -> String {
    format!(
        "account_id = ?1 AND substr({col}, 1, length(?2)) = ?2 AND length({col}) > length(?2) \
         AND substr({col}, length(?2) + 1) NOT GLOB '*[^0-9]*'"
    )
}

/// Ids being re-keyed pass through this prefix so that two rows swapping ids
/// (5 → 7 while 7 → 5) never collide on a primary or unique key half-way.
const REKEY_TMP_PREFIX: &str = "uidv-rekey-tmp:";

fn hard_delete_email_in_tx(tx: &rusqlite::Transaction<'_>, email_id: &str) -> Result<()> {
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
    Ok(())
}

impl Database {
    /// Every row of `account_id` — soft-deleted ones included, they occupy the
    /// id too — whose id is `id_prefix` plus a UID, oldest first.
    pub fn emails_in_uid_namespace(&self, account_id: &str, id_prefix: &str) -> Result<Vec<StoredUidRow>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!(
            "SELECT id, message_id, timestamp FROM emails WHERE {} ORDER BY timestamp, id",
            uid_namespace_sql("id")
        ))?;
        let rows = stmt
            .query_map(params![account_id, id_prefix], |row| {
                Ok(StoredUidRow {
                    id: row.get(0)?,
                    message_id: row.get(1)?,
                    timestamp: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Move one IMAP mailbox's stored rows onto the UIDs its messages have
    /// after a UIDVALIDITY change, in one transaction:
    ///
    /// - each `(old_id, new_id)` in `rekeys` is re-keyed, every dependent row
    ///   (tags, bodies, embeddings, FTS, …) following it;
    /// - each id in `remove` — a row no message of the mailbox answers to any
    ///   more — is hard-deleted, so its stale UID cannot shadow new mail;
    /// - failed-download records of that mailbox are dropped: they name UIDs
    ///   that now belong to other messages, and retrying them would store the
    ///   wrong mail under a stale id.
    ///
    /// Set-based rather than row-by-row: a mailbox can hold tens of thousands
    /// of rows, and the FTS table has no index on the email id.
    pub fn apply_uid_rekey(
        &self,
        account_id: &str,
        id_prefix: &str,
        rekeys: &[(String, String)],
        remove: &[String],
    ) -> Result<()> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        tx.pragma_update(None, "defer_foreign_keys", true)?;

        tx.execute(
            &format!("DELETE FROM sync_failed_emails WHERE {}", uid_namespace_sql("email_id")),
            params![account_id, id_prefix],
        )?;
        for id in remove {
            hard_delete_email_in_tx(&tx, id)?;
        }

        tx.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS uid_rekey (old_id TEXT PRIMARY KEY, new_id TEXT NOT NULL);
             DELETE FROM temp.uid_rekey;",
        )?;
        {
            let mut insert = tx.prepare("INSERT INTO temp.uid_rekey (old_id, new_id) VALUES (?1, ?2)")?;
            for (old_id, new_id) in rekeys {
                insert.execute(params![old_id, new_id])?;
            }
        }

        let mut targets = vec![("emails".to_string(), "id".to_string())];
        targets.extend(tables_referencing_emails(&tx)?);
        targets.extend(
            LOOSE_EMAIL_ID_TABLES
                .iter()
                .map(|(t, c)| ((*t).to_string(), (*c).to_string())),
        );
        // Table and column names come from sqlite_master, not user input.
        for (table, col) in &targets {
            tx.execute(
                &format!(
                    "UPDATE \"{table}\" SET \"{col}\" = ?1 || \"{col}\"
                     WHERE \"{col}\" IN (SELECT old_id FROM temp.uid_rekey)"
                ),
                params![REKEY_TMP_PREFIX],
            )?;
        }
        for (table, col) in &targets {
            tx.execute(
                &format!(
                    "UPDATE \"{table}\" SET \"{col}\" = (
                         SELECT new_id FROM temp.uid_rekey
                         WHERE old_id = substr(\"{table}\".\"{col}\", length(?1) + 1))
                     WHERE substr(\"{col}\", 1, length(?1)) = ?1"
                ),
                params![REKEY_TMP_PREFIX],
            )?;
        }
        tx.execute("DELETE FROM temp.uid_rekey", [])?;
        tx.commit()?;
        Ok(())
    }

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
        hard_delete_email_in_tx(&tx, email_id)?;
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
            references: None,
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
            is_starred: false,
            headers: None,
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

    fn all_ids(db: &Database) -> Vec<String> {
        let conn = db.reader();
        let mut stmt = conn.prepare("SELECT id FROM emails ORDER BY id").unwrap();
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        ids
    }

    fn subject_of(db: &Database, id: &str) -> String {
        db.reader()
            .query_row("SELECT subject FROM emails WHERE id = ?1", [id], |r| r.get(0))
            .unwrap()
    }

    fn with_subject(mut email: Email, subject: &str) -> Email {
        email.subject = subject.to_string();
        email
    }

    #[test]
    fn a_uid_namespace_holds_only_ids_that_are_the_prefix_plus_a_uid() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.seed_test_account("acc-10");
        db.insert_emails_batch(&[
            email("acc-1::5", "acc-1", "inbox"),
            email("acc-1::12", "acc-1", "inbox"),
            // The inbox prefix is also the start of every other mailbox's ids.
            email("acc-1::SENT::5", "acc-1", "sent"),
            email("acc-1::FOLDER::QQ::5", "acc-1", "folder:A"),
            email("local-sent-abc", "acc-1", "sent"),
            email("acc-10::5", "acc-10", "inbox"),
        ])
        .unwrap();
        // A locally deleted row still occupies its id.
        db.delete_email("acc-1::12").unwrap();

        let inbox: Vec<String> = db
            .emails_in_uid_namespace("acc-1", "acc-1::")
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        let sent: Vec<String> = db
            .emails_in_uid_namespace("acc-1", "acc-1::SENT::")
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();

        assert_eq!(inbox, vec!["acc-1::12".to_string(), "acc-1::5".to_string()]);
        assert_eq!(sent, vec!["acc-1::SENT::5".to_string()]);
    }

    #[test]
    fn uid_rekey_moves_rows_and_dependents_even_when_ids_swap() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.insert_emails_batch(&[
            with_subject(email("acc-1::5", "acc-1", "inbox"), "five"),
            with_subject(email("acc-1::7", "acc-1", "inbox"), "seven"),
            with_subject(email("acc-1::9", "acc-1", "inbox"), "nine"),
        ])
        .unwrap();
        insert_tag(&db, "acc-1::5");

        // 5 and 7 trade places, 9 moves to a free UID: a row-by-row rename
        // would hit the primary key half-way.
        db.apply_uid_rekey(
            "acc-1",
            "acc-1::",
            &[
                ("acc-1::5".to_string(), "acc-1::7".to_string()),
                ("acc-1::7".to_string(), "acc-1::5".to_string()),
                ("acc-1::9".to_string(), "acc-1::2".to_string()),
            ],
            &[],
        )
        .unwrap();

        assert_eq!(all_ids(&db), vec!["acc-1::2", "acc-1::5", "acc-1::7"]);
        assert_eq!(subject_of(&db, "acc-1::7"), "five");
        assert_eq!(subject_of(&db, "acc-1::5"), "seven");
        assert_eq!(subject_of(&db, "acc-1::2"), "nine");
        assert_eq!(tag_count_for(&db, "acc-1::7"), 1, "the tag follows its email");
        assert_eq!(tag_count_for(&db, "acc-1::5"), 0);
        assert_eq!(db.get_email_body("acc-1::7").unwrap(), "body");
        let fts: i64 = db
            .reader()
            .query_row(
                "SELECT COUNT(*) FROM emails_fts WHERE email_id IN ('acc-1::2', 'acc-1::5', 'acc-1::7')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(fts, 3, "the search index follows the new ids");
    }

    #[test]
    fn uid_rekey_removes_orphans_and_stale_failed_downloads_of_that_mailbox_only() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.insert_emails_batch(&[
            email("acc-1::5", "acc-1", "inbox"),
            email("acc-1::6", "acc-1", "inbox"),
            email("acc-1::SENT::5", "acc-1", "sent"),
        ])
        .unwrap();
        insert_tag(&db, "acc-1::6");
        db.add_failed_email("acc-1", "acc-1::8", "timeout").unwrap();
        db.add_failed_email("acc-1", "acc-1::SENT::8", "timeout").unwrap();

        db.apply_uid_rekey("acc-1", "acc-1::", &[], &["acc-1::6".to_string()])
            .unwrap();

        assert_eq!(all_ids(&db), vec!["acc-1::5", "acc-1::SENT::5"]);
        assert_eq!(tag_count_for(&db, "acc-1::6"), 0);
        let failed: Vec<String> = db
            .get_failed_emails("acc-1")
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            failed,
            vec!["acc-1::SENT::8".to_string()],
            "a failed download of another mailbox keeps its retry"
        );
    }

    #[test]
    fn uid_rekey_can_run_twice_on_one_connection() {
        // The scratch table is per connection and must start empty each time.
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.insert_emails_batch(&[email("acc-1::5", "acc-1", "inbox")]).unwrap();

        db.apply_uid_rekey(
            "acc-1",
            "acc-1::",
            &[("acc-1::5".to_string(), "acc-1::6".to_string())],
            &[],
        )
        .unwrap();
        db.apply_uid_rekey(
            "acc-1",
            "acc-1::",
            &[("acc-1::6".to_string(), "acc-1::7".to_string())],
            &[],
        )
        .unwrap();

        assert_eq!(all_ids(&db), vec!["acc-1::7"]);
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
