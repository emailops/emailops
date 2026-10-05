//! Snoozed conversations (V031 `thread_snoozes`): the records, the SQL the
//! inbox list uses to hide and re-sort them, and the release of a snooze when
//! new mail arrives in the thread.

use super::*;
use crate::models::ThreadSnooze;

/// WHERE fragment: the row's thread is not currently snoozed. `prefix`
/// qualifies the emails columns (`"e."`, `"emails."`). A primary-key seek per
/// candidate row, so it never changes how the outer query is driven.
pub(super) fn not_snoozed_predicate(prefix: &str) -> String {
    format!(
        "NOT EXISTS (SELECT 1 FROM thread_snoozes snz \
         WHERE snz.account_id = {prefix}account_id AND snz.thread_id = {prefix}thread_id \
         AND snz.woke_at IS NULL)"
    )
}

/// WHERE fragment: the row's thread has no snooze record at all, snoozed or
/// woken. The inbox's date-ordered arm uses it, because a woken thread is
/// listed by its wake time in the other arm.
pub(super) fn no_snooze_record_predicate(prefix: &str) -> String {
    format!(
        "NOT EXISTS (SELECT 1 FROM thread_snoozes snz \
         WHERE snz.account_id = {prefix}account_id AND snz.thread_id = {prefix}thread_id)"
    )
}

fn row_to_snooze(row: &rusqlite::Row) -> rusqlite::Result<ThreadSnooze> {
    Ok(ThreadSnooze {
        account_id: row.get(0)?,
        thread_id: row.get(1)?,
        snoozed_until: row.get(2)?,
        created_at: row.get(3)?,
        woke_at: row.get(4)?,
    })
}

const SNOOZE_COLUMNS: &str = "account_id, thread_id, snoozed_until, created_at, woke_at";

/// Inside the ingest transaction: drop the snooze of every thread that just
/// received a new inbound inbox message (Gmail returns a snoozed conversation
/// to the inbox when someone writes to it). `new_rows` are the messages this
/// batch inserted for the first time — a re-download of a stored message is
/// not new mail. A message dated before the snooze was set is backfill, not
/// an arrival, and leaves the snooze alone. Returns the released threads.
pub(super) fn release_snoozes_for_new_mail(
    tx: &rusqlite::Transaction<'_>,
    new_rows: &[&Email],
) -> Result<Vec<(String, String)>> {
    let mut released = Vec::new();
    for email in new_rows {
        let mailbox = normalize_mailbox(&email.mailbox);
        if mailbox != "inbox" || is_sent_flag(email, mailbox) {
            continue;
        }
        let n = tx.execute(
            "DELETE FROM thread_snoozes WHERE account_id = ?1 AND thread_id = ?2 AND created_at < ?3",
            params![email.account_id, email.thread_id, email.timestamp],
        )?;
        if n > 0 {
            released.push((email.account_id.clone(), email.thread_id.clone()));
        }
    }
    Ok(released)
}

/// Whether any snooze record exists — lets the ingest skip the per-row checks
/// entirely on the (usual) empty table.
pub(super) fn any_snooze(tx: &rusqlite::Transaction<'_>) -> Result<bool> {
    Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM thread_snoozes)", [], |row| row.get(0))?)
}

impl Database {
    /// Snooze conversations until `until` (unix seconds). Re-snoozing a
    /// snoozed or woken thread replaces its record.
    pub fn snooze_threads(&self, threads: &[(&str, &str)], until: i64, now: i64) -> Result<usize> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        let mut n = 0;
        for (account_id, thread_id) in threads {
            n += tx.execute(
                "INSERT INTO thread_snoozes (account_id, thread_id, snoozed_until, created_at, woke_at)
                 VALUES (?1, ?2, ?3, ?4, NULL)
                 ON CONFLICT(account_id, thread_id) DO UPDATE SET
                   snoozed_until = excluded.snoozed_until,
                   created_at = excluded.created_at,
                   woke_at = NULL",
                params![account_id, thread_id, until, now],
            )?;
        }
        tx.commit()?;
        Ok(n)
    }

    /// Remove the snooze record of each thread, snoozed or woken. Returns how
    /// many records existed.
    pub fn unsnooze_threads(&self, threads: &[(&str, &str)]) -> Result<usize> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        let mut n = 0;
        for (account_id, thread_id) in threads {
            n += tx.execute(
                "DELETE FROM thread_snoozes WHERE account_id = ?1 AND thread_id = ?2",
                params![account_id, thread_id],
            )?;
        }
        tx.commit()?;
        Ok(n)
    }

    /// Every snooze record (snoozed and woken) in `scope`, soonest wake first.
    pub fn list_thread_snoozes(&self, scope: crate::db::AccountScope<'_>) -> Result<Vec<ThreadSnooze>> {
        let conn = self.reader();
        let (cond, account): (&str, Option<&str>) = match scope {
            crate::db::AccountScope::Account(id) => ("account_id = ?1", Some(id)),
            crate::db::AccountScope::AllEnabled => ("account_id IN (SELECT id FROM accounts WHERE enabled = 1)", None),
        };
        let mut stmt = conn.prepare(&format!(
            "SELECT {SNOOZE_COLUMNS} FROM thread_snoozes WHERE {cond}
             ORDER BY snoozed_until, account_id, thread_id"
        ))?;
        let rows = match account {
            Some(id) => stmt
                .query_map(params![id], row_to_snooze)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            None => stmt
                .query_map([], row_to_snooze)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        };
        Ok(rows)
    }

    /// The records still snoozed (not yet woken), across every account.
    pub fn pending_snoozes(&self) -> Result<Vec<ThreadSnooze>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SNOOZE_COLUMNS} FROM thread_snoozes WHERE woke_at IS NULL ORDER BY snoozed_until"
        ))?;
        let rows = stmt
            .query_map([], row_to_snooze)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Mark due snoozes woken at `now`. A record re-snoozed to a later time
    /// since it was read is left alone. Returns how many woke.
    pub fn mark_snoozes_woken(&self, threads: &[(&str, &str)], now: i64) -> Result<usize> {
        let mut conn = self.connection();
        let tx = conn.transaction()?;
        let mut n = 0;
        for (account_id, thread_id) in threads {
            n += tx.execute(
                "UPDATE thread_snoozes SET woke_at = ?3
                 WHERE account_id = ?1 AND thread_id = ?2 AND woke_at IS NULL AND snoozed_until <= ?3",
                params![account_id, thread_id, now],
            )?;
        }
        tx.commit()?;
        Ok(n)
    }

    /// Delete woken records whose thread is no longer in the inbox (archived,
    /// deleted, filed): they order nothing, and would resurface the thread at
    /// its old wake time if it ever came back.
    pub fn prune_woken_snoozes(&self) -> Result<usize> {
        let conn = self.connection();
        let n = conn.execute(
            "DELETE FROM thread_snoozes
             WHERE woke_at IS NOT NULL
               AND NOT EXISTS (SELECT 1 FROM emails e
                               WHERE e.account_id = thread_snoozes.account_id
                                 AND e.thread_id = thread_snoozes.thread_id
                                 AND e.is_deleted = 0 AND e.mailbox = 'inbox')",
            [],
        )?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::*;
    use crate::db::{AccountScope, Database};
    use crate::models::ThreadSnooze;

    const NOW: i64 = 1_700_000_000;

    fn listed_threads(db: &Database, view: Option<&str>) -> Vec<String> {
        db.get_emails(AccountScope::Account("acc1"), 50, 0, None, view, None)
            .unwrap()
            .into_iter()
            .map(|e| e.thread_id)
            .collect()
    }

    fn snooze(db: &Database, thread: &str, until: i64) {
        db.snooze_threads(&[("acc1", thread)], until, NOW).unwrap();
    }

    #[test]
    fn snooze_upserts_and_unsnooze_removes() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@example.com");
        snooze(&db, "t1", NOW + 100);
        snooze(&db, "t1", NOW + 500);
        let all = db.list_thread_snoozes(AccountScope::Account("acc1")).unwrap();
        assert_eq!(
            all,
            vec![ThreadSnooze {
                account_id: "acc1".into(),
                thread_id: "t1".into(),
                snoozed_until: NOW + 500,
                created_at: NOW,
                woke_at: None,
            }]
        );
        assert_eq!(db.unsnooze_threads(&[("acc1", "t1")]).unwrap(), 1);
        assert!(db
            .list_thread_snoozes(AccountScope::Account("acc1"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_snooze_must_be_a_positive_time() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@example.com");
        assert!(db.snooze_threads(&[("acc1", "t1")], 0, NOW).is_err());
    }

    #[test]
    fn deleting_the_account_drops_its_snoozes() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@example.com");
        snooze(&db, "t1", NOW + 100);
        db.delete_account("acc1").unwrap();
        assert!(db.pending_snoozes().unwrap().is_empty());
    }

    #[test]
    fn waking_only_touches_due_snoozed_records() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@example.com");
        snooze(&db, "due", NOW - 1);
        snooze(&db, "later", NOW + 100);
        let woke = db
            .mark_snoozes_woken(&[("acc1", "due"), ("acc1", "later")], NOW)
            .unwrap();
        assert_eq!(woke, 1);
        let pending: Vec<String> = db.pending_snoozes().unwrap().into_iter().map(|s| s.thread_id).collect();
        assert_eq!(pending, vec!["later".to_string()]);
        let all = db.list_thread_snoozes(AccountScope::Account("acc1")).unwrap();
        assert_eq!(all.iter().find(|s| s.thread_id == "due").unwrap().woke_at, Some(NOW));
    }

    #[test]
    fn the_inbox_hides_snoozed_threads_and_counts_without_them() {
        let db = Database::new_for_testing().unwrap();
        insert_email(&db, "a", "acc1", "t-a", NOW - 10);
        insert_email(&db, "b", "acc1", "t-b", NOW - 20);
        snooze(&db, "t-a", NOW + 3600);
        assert_eq!(listed_threads(&db, None), vec!["t-b".to_string()]);
        assert_eq!(db.count_emails(AccountScope::Account("acc1"), None).unwrap(), 1);
        assert_eq!(db.count_emails(AccountScope::AllEnabled, None).unwrap(), 1);
    }

    #[test]
    fn the_snoozed_view_lists_snoozed_threads_soonest_wake_first() {
        let db = Database::new_for_testing().unwrap();
        insert_email(&db, "a1", "acc1", "t-a", NOW - 50);
        insert_email(&db, "a2", "acc1", "t-a", NOW - 10);
        insert_email(&db, "b", "acc1", "t-b", NOW - 20);
        insert_email(&db, "c", "acc1", "t-c", NOW - 30);
        snooze(&db, "t-a", NOW + 7200);
        snooze(&db, "t-b", NOW + 3600);
        let listed = db
            .get_emails(AccountScope::Account("acc1"), 50, 0, None, Some("snoozed"), None)
            .unwrap();
        let ids: Vec<&str> = listed.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "a2"], "one row per thread, its latest message");
        assert_eq!(
            db.count_emails(AccountScope::Account("acc1"), Some("snoozed")).unwrap(),
            2
        );
        // A woken thread is no longer in the Snoozed view.
        db.mark_snoozes_woken(&[("acc1", "t-b")], NOW + 3600).unwrap();
        assert_eq!(listed_threads(&db, Some("snoozed")), vec!["t-a".to_string()]);
        assert_eq!(
            db.count_emails(AccountScope::Account("acc1"), Some("snoozed")).unwrap(),
            1
        );
    }

    #[test]
    fn a_woken_thread_sorts_by_its_wake_time() {
        let db = Database::new_for_testing().unwrap();
        insert_email(&db, "old", "acc1", "t-old", NOW - 30 * 86_400);
        insert_email(&db, "new", "acc1", "t-new", NOW - 60);
        insert_email(&db, "mid", "acc1", "t-mid", NOW - 86_400);
        snooze(&db, "t-old", NOW - 1);
        db.mark_snoozes_woken(&[("acc1", "t-old")], NOW).unwrap();
        assert_eq!(
            listed_threads(&db, None),
            vec!["t-old".to_string(), "t-new".to_string(), "t-mid".to_string()]
        );
        // Listed once, counted once, and its real date is unchanged.
        assert_eq!(db.count_emails(AccountScope::Account("acc1"), None).unwrap(), 3);
        let rows = db
            .get_emails(AccountScope::Account("acc1"), 50, 0, None, None, None)
            .unwrap();
        assert_eq!(rows[0].timestamp, NOW - 30 * 86_400);
        // Paging keeps the merged order.
        let page2: Vec<String> = db
            .get_emails(AccountScope::Account("acc1"), 2, 2, None, None, None)
            .unwrap()
            .into_iter()
            .map(|e| e.thread_id)
            .collect();
        assert_eq!(page2, vec!["t-mid".to_string()]);
        // Its inbox position follows the same order.
        assert_eq!(
            db.get_email_inbox_position(AccountScope::Account("acc1"), "old")
                .unwrap(),
            0
        );
        assert_eq!(
            db.get_email_inbox_position(AccountScope::Account("acc1"), "new")
                .unwrap(),
            1
        );
        assert_eq!(
            db.get_email_inbox_position(AccountScope::Account("acc1"), "mid")
                .unwrap(),
            2
        );
    }

    #[test]
    fn a_woken_thread_respects_the_category_filter() {
        let db = Database::new_for_testing().unwrap();
        insert_email_with_category(&db, "p", "acc1", "t-p", NOW - 100, "primary");
        insert_email_with_category(&db, "s", "acc1", "t-s", NOW - 200, "social");
        snooze(&db, "t-s", NOW - 1);
        db.mark_snoozes_woken(&[("acc1", "t-s")], NOW).unwrap();
        let primary: Vec<String> = db
            .get_emails(AccountScope::Account("acc1"), 50, 0, None, None, Some("primary"))
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(primary, vec!["p".to_string()]);
    }

    #[test]
    fn the_inbox_query_is_still_driven_by_an_index() {
        let db = Database::new_for_testing().unwrap();
        for scope in [AccountScope::Account("acc1"), AccountScope::AllEnabled] {
            let plan = db.explain_get_emails(scope, 50, 0, None);
            assert!(
                plan.iter()
                    .all(|step| !step.starts_with("SCAN e") || step.contains("INDEX")),
                "the emails table must never be scanned without an index, got plan: {plan:?}"
            );
            assert!(
                plan.iter()
                    .any(|step| step.contains("SEARCH snz USING PRIMARY KEY (account_id=? AND thread_id=?)")),
                "the snooze exclusion must be a primary-key seek, got plan: {plan:?}"
            );
            // The date arm reads the index in date order and stops at the
            // page's end; only the final merge of the two (small) arms sorts.
            let sorts = plan.iter().filter(|step| step.contains("TEMP B-TREE")).count();
            assert_eq!(sorts, 1, "only the merge may sort, got plan: {plan:?}");
        }
    }

    #[test]
    fn pruning_drops_woken_records_whose_thread_left_the_inbox() {
        let db = Database::new_for_testing().unwrap();
        insert_email(&db, "kept", "acc1", "t-kept", NOW - 10);
        insert_email(&db, "gone", "acc1", "t-gone", NOW - 10);
        db.connection()
            .execute("UPDATE emails SET mailbox = 'archive' WHERE id = 'gone'", [])
            .unwrap();
        snooze(&db, "t-kept", NOW - 1);
        snooze(&db, "t-gone", NOW - 1);
        snooze(&db, "t-still-snoozed", NOW + 100);
        db.mark_snoozes_woken(&[("acc1", "t-kept"), ("acc1", "t-gone")], NOW)
            .unwrap();
        assert_eq!(db.prune_woken_snoozes().unwrap(), 1);
        let left: Vec<String> = db
            .list_thread_snoozes(AccountScope::Account("acc1"))
            .unwrap()
            .into_iter()
            .map(|s| s.thread_id)
            .collect();
        assert_eq!(left, vec!["t-kept".to_string(), "t-still-snoozed".to_string()]);
    }

    fn incoming(id: &str, thread: &str, ts: i64) -> crate::models::Email {
        let mut e = email_fixture(id, "acc1", "<p>hi</p>");
        e.thread_id = thread.to_string();
        e.timestamp = ts;
        e
    }

    fn remaining(db: &Database) -> Vec<String> {
        db.list_thread_snoozes(AccountScope::Account("acc1"))
            .unwrap()
            .into_iter()
            .map(|s| s.thread_id)
            .collect()
    }

    #[test]
    fn new_inbound_mail_releases_the_snooze() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@example.com");
        db.insert_emails_batch(&[incoming("m1", "t1", NOW - 100)]).unwrap();
        snooze(&db, "t1", NOW + 3600);
        db.insert_emails_batch(&[incoming("m2", "t1", NOW + 10)]).unwrap();
        assert!(remaining(&db).is_empty());
        assert_eq!(listed_threads(&db, None), vec!["t1".to_string()]);
    }

    #[test]
    fn new_mail_also_releases_a_woken_record() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@example.com");
        snooze(&db, "t1", NOW - 1);
        db.mark_snoozes_woken(&[("acc1", "t1")], NOW).unwrap();
        db.insert_emails_batch(&[incoming("m2", "t1", NOW + 10)]).unwrap();
        assert!(remaining(&db).is_empty());
    }

    #[test]
    fn sent_spam_backfilled_or_re_downloaded_mail_keeps_the_snooze() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@example.com");
        db.insert_emails_batch(&[incoming("m1", "t1", NOW + 50)]).unwrap();
        snooze(&db, "t1", NOW + 3600);

        let mut sent = incoming("s1", "t1", NOW + 10);
        sent.is_sent = true;
        let mut spam = incoming("sp1", "t1", NOW + 10);
        spam.mailbox = "spam".into();
        let backfill = incoming("old", "t1", NOW - 500);
        // The stored message, downloaded again, dated after the snooze.
        let redownload = incoming("m1", "t1", NOW + 50);
        db.insert_emails_batch(&[sent, spam, backfill, redownload]).unwrap();

        assert_eq!(remaining(&db), vec!["t1".to_string()]);
    }

    #[test]
    fn new_mail_in_another_account_s_same_thread_id_keeps_the_snooze() {
        let db = Database::new_for_testing().unwrap();
        insert_account(&db, "acc1", "me@example.com");
        insert_account(&db, "acc2", "other@example.com");
        snooze(&db, "t1", NOW + 3600);
        let mut other = incoming("x1", "t1", NOW + 10);
        other.account_id = "acc2".into();
        db.insert_emails_batch(&[other]).unwrap();
        assert_eq!(remaining(&db), vec!["t1".to_string()]);
    }
}
