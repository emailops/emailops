//! Queries behind the mailbox-state sync: read-state changes still owed to
//! the provider (V029 `emails.read_push_pending_since`).

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

impl Database {
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
    use super::PendingReadPush;
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
