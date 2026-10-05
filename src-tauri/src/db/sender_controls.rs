//! Blocked senders and list unsubscribes (V034). Storage only — address
//! normalization, the provider moves and the unsubscribe calls live in
//! `services::sender_controls` and `services::unsubscribe`.

use std::collections::HashSet;

use rusqlite::{params, OptionalExtension};

use crate::db::Database;
use crate::models::error::Result;
use crate::models::BlockedSender;

impl Database {
    /// Block `address` (already normalized) in one account. Idempotent: a
    /// repeated block keeps the original date.
    pub fn insert_blocked_sender(&self, account_id: &str, address: &str, now: i64) -> Result<()> {
        self.connection().execute(
            "INSERT INTO blocked_senders (account_id, address, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(account_id, address) DO NOTHING",
            params![account_id, address, now],
        )?;
        Ok(())
    }

    /// Unblock. Returns whether a block was removed.
    pub fn delete_blocked_sender(&self, account_id: &str, address: &str) -> Result<bool> {
        let n = self.connection().execute(
            "DELETE FROM blocked_senders WHERE account_id = ?1 AND address = ?2",
            params![account_id, address],
        )?;
        Ok(n > 0)
    }

    pub fn is_sender_blocked(&self, account_id: &str, address: &str) -> Result<bool> {
        let found = self
            .reader()
            .query_row(
                "SELECT 1 FROM blocked_senders WHERE account_id = ?1 AND address = ?2",
                params![account_id, address],
                |_| Ok(()),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// The blocked addresses of one account, for the ingest check.
    pub fn blocked_addresses(&self, account_id: &str) -> Result<HashSet<String>> {
        let conn = self.reader();
        let mut stmt = conn.prepare("SELECT address FROM blocked_senders WHERE account_id = ?1")?;
        let rows = stmt.query_map(params![account_id], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every block, newest first; one account's when `account_id` is given.
    pub fn list_blocked_senders(&self, account_id: Option<&str>) -> Result<Vec<BlockedSender>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT account_id, address, created_at FROM blocked_senders
             WHERE ?1 IS NULL OR account_id = ?1
             ORDER BY created_at DESC, address",
        )?;
        let rows = stmt.query_map(params![account_id], |r| {
            Ok(BlockedSender {
                account_id: r.get(0)?,
                address: r.get(1)?,
                created_at: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Ids of the live messages from `address` (case-insensitive) that sit in
    /// one of `mailboxes`, newest first, at most `limit`.
    pub fn email_ids_from_sender_in(
        &self,
        account_id: &str,
        address: &str,
        mailboxes: &[&str],
        limit: usize,
    ) -> Result<Vec<String>> {
        let conn = self.reader();
        let mut stmt = conn.prepare(
            "SELECT id, mailbox FROM emails
             WHERE account_id = ?1 AND sender_email = ?2 COLLATE NOCASE AND is_deleted = 0
             ORDER BY timestamp DESC, id DESC",
        )?;
        let rows = stmt.query_map(params![account_id, address], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, mailbox) = row?;
            if mailboxes.contains(&mailbox.as_str()) {
                out.push(id);
                if out.len() >= limit {
                    break;
                }
            }
        }
        Ok(out)
    }

    /// Record that the user asked to leave `address`'s list.
    pub fn upsert_sender_unsubscribe(&self, account_id: &str, address: &str, method: &str, now: i64) -> Result<()> {
        self.connection().execute(
            "INSERT INTO sender_unsubscribes (account_id, address, method, requested_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(account_id, address) DO UPDATE SET method = excluded.method, requested_at = excluded.requested_at",
            params![account_id, address, method, now],
        )?;
        Ok(())
    }

    /// When the user last asked to leave `address`'s list, if ever.
    pub fn sender_unsubscribed_at(&self, account_id: &str, address: &str) -> Result<Option<i64>> {
        let at = self
            .reader()
            .query_row(
                "SELECT requested_at FROM sender_unsubscribes WHERE account_id = ?1 AND address = ?2",
                params![account_id, address],
                |r| r.get(0),
            )
            .optional()?;
        Ok(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Email;

    fn db() -> Database {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        db.seed_test_account("acc-2");
        db
    }

    fn email(id: &str, sender: &str, mailbox: &str, ts: i64) -> Email {
        Email {
            id: id.to_string(),
            account_id: "acc-1".to_string(),
            thread_id: format!("t-{id}"),
            message_id: None,
            references: None,
            subject: "s".to_string(),
            sender: "News".to_string(),
            sender_email: sender.to_string(),
            recipients: vec![],
            cc: vec![],
            body: "b".to_string(),
            snippet: "b".to_string(),
            timestamp: ts,
            is_read: true,
            triage_status: None,
            category: "primary".to_string(),
            mailbox: mailbox.to_string(),
            is_sent: false,
            is_starred: false,
            headers: None,
        }
    }

    #[test]
    fn blocks_are_per_account_and_idempotent() {
        let db = db();
        db.insert_blocked_sender("acc-1", "news@example.com", 10).unwrap();
        db.insert_blocked_sender("acc-1", "news@example.com", 20).unwrap();
        assert!(db.is_sender_blocked("acc-1", "news@example.com").unwrap());
        assert!(!db.is_sender_blocked("acc-2", "news@example.com").unwrap());
        let all = db.list_blocked_senders(None).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].created_at, 10, "a repeated block keeps the first date");
    }

    #[test]
    fn unblock_removes_the_block_and_reports_it() {
        let db = db();
        db.insert_blocked_sender("acc-1", "news@example.com", 10).unwrap();
        assert!(db.delete_blocked_sender("acc-1", "news@example.com").unwrap());
        assert!(!db.delete_blocked_sender("acc-1", "news@example.com").unwrap());
        assert!(db.blocked_addresses("acc-1").unwrap().is_empty());
    }

    #[test]
    fn the_schema_refuses_an_address_that_is_not_normalized() {
        let db = db();
        assert!(db.insert_blocked_sender("acc-1", "News@Example.com", 1).is_err());
        assert!(db.insert_blocked_sender("acc-1", "", 1).is_err());
    }

    #[test]
    fn listing_filters_by_account_newest_first() {
        let db = db();
        db.insert_blocked_sender("acc-1", "a@example.com", 1).unwrap();
        db.insert_blocked_sender("acc-1", "b@example.com", 2).unwrap();
        db.insert_blocked_sender("acc-2", "c@example.com", 3).unwrap();
        let one: Vec<_> = db
            .list_blocked_senders(Some("acc-1"))
            .unwrap()
            .into_iter()
            .map(|b| b.address)
            .collect();
        assert_eq!(one, vec!["b@example.com", "a@example.com"]);
        assert_eq!(db.list_blocked_senders(None).unwrap().len(), 3);
    }

    #[test]
    fn sender_messages_match_case_insensitively_within_the_given_mailboxes() {
        let db = db();
        db.insert_emails_batch(&[
            email("m1", "News@Example.com", "inbox", 1),
            email("m2", "news@example.com", "archive", 2),
            email("m3", "news@example.com", "spam", 3),
            email("m4", "other@example.com", "inbox", 4),
        ])
        .unwrap();
        let ids = db
            .email_ids_from_sender_in("acc-1", "news@example.com", &["inbox", "archive"], 10)
            .unwrap();
        assert_eq!(ids, vec!["m2", "m1"]);
        let capped = db
            .email_ids_from_sender_in("acc-1", "news@example.com", &["inbox", "archive"], 1)
            .unwrap();
        assert_eq!(capped, vec!["m2"]);
    }

    #[test]
    fn an_unsubscribe_is_recorded_and_replaced() {
        let db = db();
        assert_eq!(db.sender_unsubscribed_at("acc-1", "news@example.com").unwrap(), None);
        db.upsert_sender_unsubscribe("acc-1", "news@example.com", "one_click", 5)
            .unwrap();
        db.upsert_sender_unsubscribe("acc-1", "news@example.com", "mailto", 9)
            .unwrap();
        assert_eq!(db.sender_unsubscribed_at("acc-1", "news@example.com").unwrap(), Some(9));
        assert!(db
            .upsert_sender_unsubscribe("acc-1", "x@example.com", "carrier-pigeon", 1)
            .is_err());
    }
}
