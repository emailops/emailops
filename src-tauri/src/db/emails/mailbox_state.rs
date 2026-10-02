//! Queries behind the mailbox-state sync: read-state changes still owed to
//! the provider (V029 `emails.read_push_pending_since`), and the stored rows
//! the server-to-local refresh compares against the provider.

use super::*;

/// A row whose read state has not reached the provider yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingReadPush {
    pub id: String,
    /// The state to push — the local row is authoritative while it is pending.
    pub is_read: bool,
    /// When the change was made locally (unix seconds).
    pub pending_since: i64,
}

/// A row whose star has not reached the provider yet (V030).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingStarPush {
    pub id: String,
    /// The state to push — the local row is authoritative while it is pending.
    pub is_starred: bool,
    /// When the change was made locally (unix seconds).
    pub pending_since: i64,
}

/// What is stored for a message the refresh pass is about to check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMessageState {
    pub id: String,
    /// RFC 5322 Message-ID — the only handle that survives a server-side move.
    pub message_id: Option<String>,
    pub mailbox: String,
    pub is_read: bool,
    pub is_sent: bool,
    /// A local read-state change has not reached the provider yet.
    pub read_push_pending: bool,
    pub is_starred: bool,
    /// A local star change has not reached the provider yet.
    pub star_push_pending: bool,
}

/// Columns read into a [`StoredMessageState`], in [`stored_state_from_row`] order.
const STORED_STATE_COLUMNS: &str = "id, message_id, mailbox, is_read, is_sent, read_push_pending_since IS NOT NULL, \
     is_starred, star_push_pending_since IS NOT NULL";

fn stored_state_from_row(row: &rusqlite::Row) -> rusqlite::Result<StoredMessageState> {
    Ok(StoredMessageState {
        id: row.get(0)?,
        message_id: row.get(1)?,
        mailbox: row.get(2)?,
        is_read: row.get::<_, i32>(3)? != 0,
        is_sent: row.get::<_, i32>(4)? != 0,
        read_push_pending: row.get(5)?,
        is_starred: row.get::<_, i32>(6)? != 0,
        star_push_pending: row.get(7)?,
    })
}

impl Database {
    /// The account's newest live rows timestamped at or after `since`, at most
    /// `limit` of them — what one refresh pass checks against the provider.
    ///
    /// Left out: soft-deleted rows (the user removed them here), optimistic
    /// Sent rows still waiting for their provider copy (no provider id yet),
    /// and Spam, whose moves `reconcile_spam_moves` owns — a spam message the
    /// provider purged is deliberately kept for the junk detector.
    pub fn state_refresh_candidates(
        &self,
        account_id: &str,
        since: i64,
        limit: usize,
    ) -> Result<Vec<StoredMessageState>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!(
            "SELECT {STORED_STATE_COLUMNS}
             FROM emails
             WHERE account_id = ?1 AND is_deleted = 0 AND timestamp >= ?2
               AND pending_sync = 0 AND mailbox != 'spam'
             ORDER BY timestamp DESC, id DESC
             LIMIT ?3"
        ))?;
        let rows = stmt
            .query_map(params![account_id, since, limit as i64], stored_state_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The account's stored rows among `ids` that a change reported by the
    /// provider may be applied to — the same exclusions as
    /// [`Self::state_refresh_candidates`], without its window: soft-deleted
    /// rows, optimistic Sent rows and Spam are left out.
    pub fn stored_states_for_ids(&self, account_id: &str, ids: &[String]) -> Result<Vec<StoredMessageState>> {
        let conn = self.reader();
        let mut rows = Vec::new();
        // SQLite's default SQLITE_MAX_VARIABLE_NUMBER is 999; chunk to stay safe.
        for chunk in ids.chunks(900) {
            let placeholders: Vec<String> = (2..=chunk.len() + 1).map(|i| format!("?{i}")).collect();
            let sql = format!(
                "SELECT {STORED_STATE_COLUMNS}
                 FROM emails
                 WHERE account_id = ?1 AND is_deleted = 0 AND pending_sync = 0 AND mailbox != 'spam'
                   AND id IN ({})",
                placeholders.join(",")
            );
            let mut bound: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(chunk.len() + 1);
            bound.push(&account_id);
            for id in chunk {
                bound.push(id);
            }
            let mut stmt = conn.prepare(&sql)?;
            let found = stmt
                .query_map(bound.as_slice(), stored_state_from_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            rows.extend(found);
        }
        Ok(rows)
    }

    /// File a row under the mailbox the provider has it in. Refuses —
    /// returning `false` — for a row with a pending local change, a
    /// soft-deleted row and a row in Spam (the spam reconciliation owns those).
    pub fn apply_server_mailbox(&self, email_id: &str, mailbox: &str) -> Result<bool> {
        let changed = self.connection().execute(
            "UPDATE emails SET mailbox = ?2
             WHERE id = ?1 AND mailbox != ?2 AND mailbox != 'spam' AND is_deleted = 0
               AND read_push_pending_since IS NULL",
            params![email_id, mailbox],
        )?;
        Ok(changed > 0)
    }

    /// Soft-delete a row the provider deleted for good. Refuses — returning
    /// `false` — for a row with a pending local change.
    pub fn apply_server_delete(&self, email_id: &str) -> Result<bool> {
        let changed = self.connection().execute(
            "UPDATE emails SET is_deleted = 1
             WHERE id = ?1 AND is_deleted = 0 AND read_push_pending_since IS NULL",
            params![email_id],
        )?;
        Ok(changed > 0)
    }

    /// Take the provider's read state for a row. Refuses — returning `false` —
    /// when a local change is still pending for it: the check is in the
    /// statement itself, so a change made while the refresh was talking to the
    /// provider is not overwritten by what the provider said before it.
    pub fn apply_server_read_state(&self, email_id: &str, is_read: bool) -> Result<bool> {
        let changed = self.connection().execute(
            "UPDATE emails SET is_read = ?2
             WHERE id = ?1 AND is_read != ?2 AND read_push_pending_since IS NULL",
            params![email_id, is_read as i32],
        )?;
        Ok(changed > 0)
    }

    /// Mark one email read and record, in the same statement, that the change
    /// still has to reach the provider. One statement so neither a crash nor a
    /// concurrent sync can see the row read without its marker — the refresh
    /// pass would then take the server's "unread" for the truth.
    pub fn mark_as_read_pending_push(&self, email_id: &str, now: i64) -> Result<()> {
        self.connection().execute(
            "UPDATE emails SET is_read = 1, read_push_pending_since = ?2 WHERE id = ?1",
            params![email_id, now],
        )?;
        Ok(())
    }

    /// Set one email's read state and record, in the same statement, that the
    /// change still has to reach the provider — see
    /// [`Self::mark_as_read_pending_push`]. Used for both directions.
    pub fn set_read_pending_push(&self, email_id: &str, read: bool, now: i64) -> Result<()> {
        self.connection().execute(
            "UPDATE emails SET is_read = ?2, read_push_pending_since = ?3 WHERE id = ?1",
            params![email_id, read as i32, now],
        )?;
        Ok(())
    }

    /// Set one email's read state with nothing owed to the provider (a
    /// provider without mailbox writes, or a locally-composed row).
    pub fn set_read_local(&self, email_id: &str, read: bool) -> Result<()> {
        self.connection().execute(
            "UPDATE emails SET is_read = ?2 WHERE id = ?1",
            params![email_id, read as i32],
        )?;
        Ok(())
    }

    /// Star or unstar one email and record, in the same statement, that the
    /// change still has to reach the provider. One statement for the same
    /// reason as [`Self::mark_as_read_pending_push`].
    pub fn set_starred_pending_push(&self, email_id: &str, starred: bool, now: i64) -> Result<()> {
        self.connection().execute(
            "UPDATE emails SET is_starred = ?2, star_push_pending_since = ?3 WHERE id = ?1",
            params![email_id, starred as i32, now],
        )?;
        Ok(())
    }

    /// Star or unstar one email with nothing owed to the provider.
    pub fn set_starred_local(&self, email_id: &str, starred: bool) -> Result<()> {
        self.connection().execute(
            "UPDATE emails SET is_starred = ?2 WHERE id = ?1",
            params![email_id, starred as i32],
        )?;
        Ok(())
    }

    /// The provider has the row's star (or no longer has the message).
    pub fn clear_star_push_pending(&self, email_id: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE emails SET star_push_pending_since = NULL WHERE id = ?1",
            params![email_id],
        )?;
        Ok(())
    }

    /// Take the provider's star for a row. Refuses — returning `false` — when
    /// a local star change is still pending for it, checked in the statement
    /// itself like [`Self::apply_server_read_state`].
    pub fn apply_server_starred(&self, email_id: &str, starred: bool) -> Result<bool> {
        let changed = self.connection().execute(
            "UPDATE emails SET is_starred = ?2
             WHERE id = ?1 AND is_starred != ?2 AND star_push_pending_since IS NULL",
            params![email_id, starred as i32],
        )?;
        Ok(changed > 0)
    }

    /// Star changes of one account still owed to the provider, oldest first.
    pub fn pending_star_pushes(&self, account_id: &str, limit: usize) -> Result<Vec<PendingStarPush>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, is_starred, star_push_pending_since FROM emails
             WHERE account_id = ?1 AND star_push_pending_since IS NOT NULL
             ORDER BY star_push_pending_since, id
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![account_id, limit as i64], |row| {
                Ok(PendingStarPush {
                    id: row.get(0)?,
                    is_starred: row.get::<_, i32>(1)? != 0,
                    pending_since: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Which of `threads` (`(account_id, thread_id)`) have a live starred
    /// message — a thread is starred when any of its messages is.
    pub fn starred_threads(&self, threads: &[(&str, &str)]) -> Result<std::collections::HashSet<(String, String)>> {
        let mut starred = std::collections::HashSet::new();
        if threads.is_empty() {
            return Ok(starred);
        }
        let conn = self.reader();
        // Two binds per thread; stay well under SQLite's variable limit.
        for chunk in threads.chunks(400) {
            let pairs = vec!["(?, ?)"; chunk.len()].join(", ");
            let sql = format!(
                "WITH wanted(account_id, thread_id) AS (VALUES {pairs})
                 SELECT DISTINCT e.account_id, e.thread_id
                 FROM emails e JOIN wanted w ON e.account_id = w.account_id AND e.thread_id = w.thread_id
                 WHERE e.is_starred = 1 AND e.is_deleted = 0"
            );
            let mut bound: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(chunk.len() * 2);
            for (account_id, thread_id) in chunk {
                bound.push(account_id);
                bound.push(thread_id);
            }
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(bound.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                starred.insert(row?);
            }
        }
        Ok(starred)
    }

    /// The provider has the row's read state (or no longer has the message).
    pub fn clear_read_push_pending(&self, email_id: &str) -> Result<()> {
        self.connection().execute(
            "UPDATE emails SET read_push_pending_since = NULL WHERE id = ?1",
            params![email_id],
        )?;
        Ok(())
    }

    /// Read-state changes of one account still owed to the provider, oldest
    /// first. Soft-deleted rows are included: their marker has to be resolved
    /// like any other.
    pub fn pending_read_pushes(&self, account_id: &str, limit: usize) -> Result<Vec<PendingReadPush>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, is_read, read_push_pending_since FROM emails
             WHERE account_id = ?1 AND read_push_pending_since IS NOT NULL
             ORDER BY read_push_pending_since, id
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![account_id, limit as i64], |row| {
                Ok(PendingReadPush {
                    id: row.get(0)?,
                    is_read: row.get::<_, i32>(1)? != 0,
                    pending_since: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::{PendingReadPush, PendingStarPush};
    use crate::db::emails::test_helpers::insert_email;
    use crate::db::Database;

    fn db_with(ids: &[&str]) -> Database {
        let db = Database::new_for_testing().unwrap();
        for id in ids {
            insert_email(&db, id, "acc-1", "t-1", 1_000);
        }
        db
    }

    #[test]
    fn marking_read_with_a_pending_push_sets_both_in_one_go() {
        let db = db_with(&["m-1"]);

        db.mark_as_read_pending_push("m-1", 500).unwrap();

        assert!(db.get_email("m-1").unwrap().unwrap().is_read);
        assert_eq!(
            db.pending_read_pushes("acc-1", 10).unwrap(),
            vec![PendingReadPush {
                id: "m-1".to_string(),
                is_read: true,
                pending_since: 500
            }]
        );
    }

    #[test]
    fn clearing_the_marker_keeps_the_read_state() {
        let db = db_with(&["m-1"]);
        db.mark_as_read_pending_push("m-1", 500).unwrap();

        db.clear_read_push_pending("m-1").unwrap();

        assert!(db.pending_read_pushes("acc-1", 10).unwrap().is_empty());
        assert!(db.get_email("m-1").unwrap().unwrap().is_read);
    }

    #[test]
    fn pending_pushes_are_per_account_oldest_first_and_capped() {
        let db = db_with(&["m-1", "m-2", "m-3"]);
        insert_email(&db, "other", "acc-2", "t-2", 1_000);
        db.mark_as_read_pending_push("m-1", 300).unwrap();
        db.mark_as_read_pending_push("m-2", 100).unwrap();
        db.mark_as_read_pending_push("m-3", 200).unwrap();
        db.mark_as_read_pending_push("other", 50).unwrap();

        let ids: Vec<String> = db
            .pending_read_pushes("acc-1", 2)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();

        assert_eq!(ids, vec!["m-2".to_string(), "m-3".to_string()]);
    }

    #[test]
    fn marking_unread_with_a_pending_push_records_the_unread_state() {
        let db = db_with(&["m-1"]);
        db.mark_as_read_pending_push("m-1", 400).unwrap();

        db.set_read_pending_push("m-1", false, 500).unwrap();

        assert!(!db.get_email("m-1").unwrap().unwrap().is_read);
        assert_eq!(
            db.pending_read_pushes("acc-1", 10).unwrap(),
            vec![PendingReadPush {
                id: "m-1".to_string(),
                is_read: false,
                pending_since: 500
            }]
        );
    }

    #[test]
    fn a_local_only_read_change_leaves_nothing_pending() {
        let db = db_with(&["m-1"]);

        db.set_read_local("m-1", true).unwrap();
        db.set_read_local("m-1", false).unwrap();

        assert!(!db.get_email("m-1").unwrap().unwrap().is_read);
        assert!(db.pending_read_pushes("acc-1", 10).unwrap().is_empty());
    }

    #[test]
    fn starring_with_a_pending_push_sets_both_and_clearing_keeps_the_star() {
        let db = db_with(&["m-1", "m-2"]);
        insert_email(&db, "other", "acc-2", "t-2", 1_000);
        db.set_starred_pending_push("m-2", true, 300).unwrap();
        db.set_starred_pending_push("m-1", true, 100).unwrap();
        db.set_starred_pending_push("other", true, 50).unwrap();

        assert!(db.get_email("m-1").unwrap().unwrap().is_starred);
        assert_eq!(
            db.pending_star_pushes("acc-1", 10).unwrap(),
            vec![
                PendingStarPush {
                    id: "m-1".to_string(),
                    is_starred: true,
                    pending_since: 100
                },
                PendingStarPush {
                    id: "m-2".to_string(),
                    is_starred: true,
                    pending_since: 300
                },
            ]
        );
        assert_eq!(db.pending_star_pushes("acc-1", 1).unwrap().len(), 1, "capped");

        db.clear_star_push_pending("m-1").unwrap();

        assert!(db.get_email("m-1").unwrap().unwrap().is_starred);
        let left: Vec<String> = db
            .pending_star_pushes("acc-1", 10)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(left, vec!["m-2".to_string()]);
    }

    #[test]
    fn a_local_only_star_leaves_nothing_pending() {
        let db = db_with(&["m-1"]);

        db.set_starred_local("m-1", true).unwrap();

        assert!(db.get_email("m-1").unwrap().unwrap().is_starred);
        assert!(db.pending_star_pushes("acc-1", 10).unwrap().is_empty());
    }

    #[test]
    fn the_servers_star_is_applied_unless_a_local_star_change_is_pending() {
        let db = db_with(&["m-1", "pending"]);
        db.set_starred_pending_push("pending", true, 500).unwrap();

        assert!(db.apply_server_starred("m-1", true).unwrap());
        assert!(!db.apply_server_starred("m-1", true).unwrap(), "already starred");
        assert!(!db.apply_server_starred("pending", false).unwrap());

        assert!(db.get_email("m-1").unwrap().unwrap().is_starred);
        assert!(db.get_email("pending").unwrap().unwrap().is_starred);
    }

    #[test]
    fn starred_threads_are_the_threads_with_any_live_starred_message() {
        let db = Database::new_for_testing().unwrap();
        insert_email(&db, "a1", "acc-1", "t-a", 100);
        insert_email(&db, "a2", "acc-1", "t-a", 200);
        insert_email(&db, "b1", "acc-1", "t-b", 100);
        insert_email(&db, "c1", "acc-1", "t-c", 100);
        insert_email(&db, "same-thread-other-account", "acc-2", "t-b", 100);
        db.set_starred_local("a1", true).unwrap();
        db.set_starred_local("c1", true).unwrap();
        db.delete_email("c1").unwrap();
        db.set_starred_local("same-thread-other-account", true).unwrap();

        let starred = db
            .starred_threads(&[("acc-1", "t-a"), ("acc-1", "t-b"), ("acc-1", "t-c")])
            .unwrap();

        assert_eq!(
            starred,
            std::collections::HashSet::from([("acc-1".to_string(), "t-a".to_string())])
        );
        assert!(db.starred_threads(&[]).unwrap().is_empty());
    }

    #[test]
    fn the_servers_read_state_is_applied_in_both_directions() {
        let db = db_with(&["m-1"]);

        assert!(db.apply_server_read_state("m-1", true).unwrap());
        assert!(db.get_email("m-1").unwrap().unwrap().is_read);
        assert!(!db.apply_server_read_state("m-1", true).unwrap(), "already read");
        assert!(db.apply_server_read_state("m-1", false).unwrap());
        assert!(!db.get_email("m-1").unwrap().unwrap().is_read);
    }

    #[test]
    fn the_servers_read_state_never_overwrites_a_pending_local_change() {
        let db = db_with(&["m-1"]);
        db.mark_as_read_pending_push("m-1", 500).unwrap();

        assert!(!db.apply_server_read_state("m-1", false).unwrap());

        assert!(db.get_email("m-1").unwrap().unwrap().is_read);
    }

    #[test]
    fn refresh_candidates_are_the_newest_live_provider_backed_rows_in_the_window() {
        let db = Database::new_for_testing().unwrap();
        for (id, account, ts) in [
            ("old", "acc-1", 50),
            ("newer", "acc-1", 300),
            ("newest", "acc-1", 400),
            ("deleted", "acc-1", 350),
            ("spam", "acc-1", 360),
            ("unsent", "acc-1", 370),
            ("other-account", "acc-2", 380),
        ] {
            insert_email(&db, id, account, "t-1", ts);
        }
        db.delete_email("deleted").unwrap();
        db.connection()
            .execute_batch(
                "UPDATE emails SET mailbox = 'spam' WHERE id = 'spam';
                 UPDATE emails SET pending_sync = 1, mailbox = 'sent', is_sent = 1 WHERE id = 'unsent';
                 UPDATE emails SET message_id = '<newest@example.com>' WHERE id = 'newest';",
            )
            .unwrap();
        db.mark_as_read_pending_push("newer", 500).unwrap();

        let rows = db.state_refresh_candidates("acc-1", 100, 10).unwrap();

        assert_eq!(
            rows,
            vec![
                super::StoredMessageState {
                    id: "newest".to_string(),
                    message_id: Some("<newest@example.com>".to_string()),
                    mailbox: "inbox".to_string(),
                    is_read: false,
                    is_sent: false,
                    read_push_pending: false,
                    is_starred: false,
                    star_push_pending: false,
                },
                super::StoredMessageState {
                    id: "newer".to_string(),
                    message_id: None,
                    mailbox: "inbox".to_string(),
                    is_read: true,
                    is_sent: false,
                    read_push_pending: true,
                    is_starred: false,
                    star_push_pending: false,
                },
            ]
        );
        assert_eq!(db.state_refresh_candidates("acc-1", 100, 1).unwrap().len(), 1, "capped");
    }

    fn mailbox_of(db: &Database, id: &str) -> (String, bool) {
        db.reader()
            .query_row("SELECT mailbox, is_deleted FROM emails WHERE id = ?1", [id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i32>(1)? != 0))
            })
            .unwrap()
    }

    #[test]
    fn stored_states_for_ids_returns_the_accounts_live_provider_backed_rows() {
        let db = Database::new_for_testing().unwrap();
        for (id, account) in [
            ("kept", "acc-1"),
            ("pending", "acc-1"),
            ("deleted", "acc-1"),
            ("spam", "acc-1"),
            ("unsent", "acc-1"),
            ("not-asked", "acc-1"),
            ("other-account", "acc-2"),
        ] {
            insert_email(&db, id, account, "t-1", 1_000);
        }
        db.delete_email("deleted").unwrap();
        db.connection()
            .execute_batch(
                "UPDATE emails SET mailbox = 'spam' WHERE id = 'spam';
                 UPDATE emails SET pending_sync = 1 WHERE id = 'unsent';",
            )
            .unwrap();
        db.mark_as_read_pending_push("pending", 500).unwrap();
        let asked: Vec<String> = [
            "kept",
            "pending",
            "deleted",
            "spam",
            "unsent",
            "other-account",
            "unknown",
        ]
        .iter()
        .map(|id| id.to_string())
        .collect();

        let mut rows = db.stored_states_for_ids("acc-1", &asked).unwrap();
        rows.sort_by(|a, b| a.id.cmp(&b.id));

        let ids: Vec<(&str, bool)> = rows.iter().map(|r| (r.id.as_str(), r.read_push_pending)).collect();
        assert_eq!(ids, vec![("kept", false), ("pending", true)]);
        assert!(db.stored_states_for_ids("acc-1", &[]).unwrap().is_empty());
    }

    #[test]
    fn stored_states_carry_each_rows_read_and_sent_flags() {
        let db = db_with(&["read-sent", "unread-received"]);
        db.connection()
            .execute_batch(
                "UPDATE emails SET is_read = 1, is_sent = 1 WHERE id = 'read-sent';
                 UPDATE emails SET is_read = 0, is_sent = 0 WHERE id = 'unread-received';",
            )
            .unwrap();
        let ids = vec!["read-sent".to_string(), "unread-received".to_string()];

        let mut rows = db.stored_states_for_ids("acc-1", &ids).unwrap();
        rows.sort_by(|a, b| a.id.cmp(&b.id));

        let flags: Vec<(&str, bool, bool)> = rows.iter().map(|r| (r.id.as_str(), r.is_read, r.is_sent)).collect();
        assert_eq!(
            flags,
            vec![("read-sent", true, true), ("unread-received", false, false)]
        );
    }

    #[test]
    fn stored_states_for_ids_handles_more_ids_than_one_statement_binds() {
        let db = Database::new_for_testing().unwrap();
        let ids: Vec<String> = (0..1_000).map(|i| format!("m-{i:04}")).collect();
        for id in &ids {
            insert_email(&db, id, "acc-1", "t-1", 1_000);
        }
        assert_eq!(db.stored_states_for_ids("acc-1", &ids).unwrap().len(), 1_000);
    }

    #[test]
    fn the_servers_mailbox_is_applied_once() {
        let db = db_with(&["m-1"]);

        assert!(db.apply_server_mailbox("m-1", "trash").unwrap());
        assert_eq!(mailbox_of(&db, "m-1"), ("trash".to_string(), false));
        assert!(!db.apply_server_mailbox("m-1", "trash").unwrap(), "already there");
    }

    #[test]
    fn the_servers_mailbox_never_moves_a_pending_a_deleted_or_a_spam_row() {
        let db = db_with(&["pending", "deleted", "spam"]);
        db.mark_as_read_pending_push("pending", 500).unwrap();
        db.delete_email("deleted").unwrap();
        db.connection()
            .execute("UPDATE emails SET mailbox = 'spam' WHERE id = 'spam'", [])
            .unwrap();

        for id in ["pending", "deleted", "spam"] {
            assert!(!db.apply_server_mailbox(id, "trash").unwrap(), "{id}");
        }
        assert_eq!(mailbox_of(&db, "pending").0, "inbox");
        assert_eq!(mailbox_of(&db, "deleted").0, "inbox");
        assert_eq!(mailbox_of(&db, "spam").0, "spam");
    }

    #[test]
    fn a_server_side_delete_hides_the_row_but_not_one_with_a_pending_change() {
        let db = db_with(&["m-1", "pending"]);
        db.mark_as_read_pending_push("pending", 500).unwrap();

        assert!(db.apply_server_delete("m-1").unwrap());
        assert!(!db.apply_server_delete("m-1").unwrap(), "already deleted");
        assert!(!db.apply_server_delete("pending").unwrap());

        assert!(mailbox_of(&db, "m-1").1);
        assert!(!mailbox_of(&db, "pending").1);
    }

    /// The ALTER runs against mailboxes that already hold mail: every existing
    /// row must come out with nothing pending.
    #[test]
    fn v029_leaves_existing_rows_with_nothing_pending() {
        const V029: &str = include_str!("../../../migrations/V029__read_push_pending.sql");
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE emails (id TEXT PRIMARY KEY, account_id TEXT NOT NULL, is_read INTEGER NOT NULL DEFAULT 0);
             INSERT INTO emails (id, account_id, is_read) VALUES ('m-1', 'acc-1', 1), ('m-2', 'acc-1', 0);",
        )
        .unwrap();

        conn.execute_batch(V029).unwrap();

        let pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM emails WHERE read_push_pending_since IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let kept: i64 = conn
            .query_row("SELECT COUNT(*) FROM emails WHERE is_read = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!((pending, kept), (0, 1));
    }
}
