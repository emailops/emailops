//! The local outbox (V032): rows waiting for their `send_at`, and the guarded
//! status transitions the dispatcher and the user's undo / cancel / retry go
//! through. Every transition is one UPDATE whose WHERE clause names the status
//! it starts from, so a race (two ticks, undo against the dispatcher) has
//! exactly one winner and the loser sees `false` / `None`.

use rusqlite::{params, OptionalExtension, Row};

use crate::db::{AccountScope, Database};
use crate::models::error::{AppError, Result};
use crate::models::outbox::{OutboxEntry, OutboxFailureKind, OutboxKind, OutboxOrigin, OutboxStatus, OutgoingMessage};

/// A scheduled row as the dispatcher's planner sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledOutboxRow {
    pub id: String,
    pub send_at: i64,
}

/// What [`Database::insert_outbox`] stores.
pub struct NewOutboxRow<'a> {
    pub id: &'a str,
    pub message: &'a OutgoingMessage,
    pub origin: OutboxOrigin,
    pub send_at: i64,
    pub now: i64,
}

const ENTRY_COLUMNS: &str = "id, account_id, kind, reply_to_email_id, origin, to_addresses, cc_addresses, subject, \
     attachment_count, send_at, status, attempts, last_error, failure_kind, created_at";

fn json_list(raw: String) -> Vec<String> {
    serde_json::from_str(&raw).unwrap_or_default()
}

fn bad_column(column: &str, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        format!("unknown outbox {column} {value:?}").into(),
    )
}

fn entry_from_row(row: &Row<'_>) -> rusqlite::Result<OutboxEntry> {
    let kind: String = row.get(2)?;
    let origin: String = row.get(4)?;
    let status: String = row.get(10)?;
    let failure: Option<String> = row.get(13)?;
    Ok(OutboxEntry {
        id: row.get(0)?,
        account_id: row.get(1)?,
        kind: OutboxKind::parse(&kind).ok_or_else(|| bad_column("kind", &kind))?,
        reply_to_email_id: row.get(3)?,
        origin: OutboxOrigin::parse(&origin).ok_or_else(|| bad_column("origin", &origin))?,
        to_addresses: json_list(row.get(5)?),
        cc_addresses: json_list(row.get(6)?),
        subject: row.get(7)?,
        attachment_count: row.get(8)?,
        send_at: row.get(9)?,
        status: OutboxStatus::parse(&status).ok_or_else(|| bad_column("status", &status))?,
        attempts: row.get(11)?,
        last_error: row.get(12)?,
        failure_kind: failure.as_deref().and_then(OutboxFailureKind::parse),
        created_at: row.get(14)?,
    })
}

impl Database {
    /// Store a message in the outbox as `scheduled`.
    pub fn insert_outbox(&self, row: &NewOutboxRow<'_>) -> Result<()> {
        let message = row.message;
        let payload = serde_json::to_string(message)?;
        let to = serde_json::to_string(&message.to)?;
        let cc = serde_json::to_string(&message.cc)?;
        self.connection().execute(
            "INSERT INTO outbox (id, account_id, kind, reply_to_email_id, origin, to_addresses, cc_addresses,
                                 subject, attachment_count, payload, send_at, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'scheduled', ?12, ?12)",
            params![
                row.id,
                message.account_id,
                message.kind().as_str(),
                message.reply_to_email_id,
                row.origin.as_str(),
                to,
                cc,
                message.subject,
                message.attachments.len() as i64,
                payload,
                row.send_at,
                row.now,
            ],
        )?;
        Ok(())
    }

    /// One row, without its payload.
    pub fn get_outbox_entry(&self, id: &str) -> Result<Option<OutboxEntry>> {
        let conn = self.reader();
        let entry = conn
            .query_row(
                &format!("SELECT {ENTRY_COLUMNS} FROM outbox WHERE id = ?1"),
                params![id],
                entry_from_row,
            )
            .optional()?;
        Ok(entry)
    }

    /// The rows the Scheduled view lists — waiting or failed — of one account
    /// or every enabled one, soonest first. Rows still inside an undo window
    /// are left out: the "Sending… Undo" toast already shows them.
    pub fn list_outbox(&self, scope: AccountScope<'_>) -> Result<Vec<OutboxEntry>> {
        let conn = self.reader();
        let filter = "(o.status = 'failed' OR (o.status IN ('scheduled', 'sending') AND o.origin = 'scheduled'))";
        let (sql, account): (String, Option<&str>) = match scope {
            AccountScope::Account(id) => (
                format!(
                    "SELECT {cols} FROM outbox o WHERE o.account_id = ?1 AND {filter} ORDER BY o.send_at, o.created_at",
                    cols = prefixed_columns()
                ),
                Some(id),
            ),
            AccountScope::AllEnabled => (
                format!(
                    "SELECT {cols} FROM outbox o JOIN accounts a ON a.id = o.account_id
                     WHERE a.enabled = 1 AND {filter} ORDER BY o.send_at, o.created_at",
                    cols = prefixed_columns()
                ),
                None,
            ),
        };
        let mut stmt = conn.prepare(&sql)?;
        let rows = match account {
            Some(id) => stmt
                .query_map(params![id], entry_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            None => stmt
                .query_map([], entry_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        };
        Ok(rows)
    }

    /// Every row waiting to be sent, for the dispatcher's planner.
    pub fn scheduled_outbox(&self) -> Result<Vec<ScheduledOutboxRow>> {
        let conn = self.reader();
        let mut stmt = conn.prepare("SELECT id, send_at FROM outbox WHERE status = 'scheduled' ORDER BY send_at")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ScheduledOutboxRow {
                    id: row.get(0)?,
                    send_at: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// `scheduled → sending`, only when the row is due at `now`. Returns the
    /// stored message when this call won the row; `None` when it was not
    /// (cancelled, already claimed, rescheduled later). The flip is committed
    /// before the provider is called, so the row can never be sent twice.
    pub fn claim_outbox(&self, id: &str, now: i64) -> Result<Option<String>> {
        let conn = self.connection();
        let claimed = conn.execute(
            "UPDATE outbox SET status = 'sending', attempts = attempts + 1, updated_at = ?2
             WHERE id = ?1 AND status = 'scheduled' AND send_at <= ?2",
            params![id, now],
        )?;
        if claimed == 0 {
            return Ok(None);
        }
        let payload = conn.query_row("SELECT payload FROM outbox WHERE id = ?1", params![id], |row| {
            row.get::<_, String>(0)
        })?;
        Ok(Some(payload))
    }

    /// `sending → sent`. The payload (attachment bytes included) is dropped.
    pub fn mark_outbox_sent(&self, id: &str, now: i64) -> Result<bool> {
        let n = self.connection().execute(
            "UPDATE outbox SET status = 'sent', payload = '', last_error = NULL, failure_kind = NULL, updated_at = ?2
             WHERE id = ?1 AND status = 'sending'",
            params![id, now],
        )?;
        Ok(n > 0)
    }

    /// `sending → failed`, keeping the payload so the user can retry or edit.
    pub fn mark_outbox_failed(&self, id: &str, error: &str, now: i64) -> Result<bool> {
        let n = self.connection().execute(
            "UPDATE outbox SET status = 'failed', last_error = ?2, failure_kind = 'error', updated_at = ?3
             WHERE id = ?1 AND status = 'sending'",
            params![id, error, now],
        )?;
        Ok(n > 0)
    }

    /// At start-up: every row still `sending` was interrupted mid-send (the
    /// app stopped). It may or may not have gone out, so it becomes `failed`
    /// (`interrupted`) instead of being sent again. Returns the ids.
    pub fn fail_interrupted_outbox(&self, now: i64) -> Result<Vec<String>> {
        let conn = self.connection();
        let mut stmt = conn.prepare(
            "UPDATE outbox SET status = 'failed', failure_kind = 'interrupted', last_error = NULL, updated_at = ?1
             WHERE status = 'sending' RETURNING id",
        )?;
        let ids = stmt
            .query_map(params![now], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    }

    /// `scheduled | failed → cancelled`: returns the message so the composer
    /// can reopen with it, and drops the stored payload. `None` when the row
    /// is no longer cancellable — the dispatcher already took it.
    pub fn cancel_outbox(&self, id: &str, now: i64) -> Result<Option<String>> {
        let conn = self.connection();
        // The write connection is held for both statements, so no other
        // writer (the dispatcher's claim) can slip in between them.
        let payload: Option<String> = conn
            .query_row(
                "SELECT payload FROM outbox WHERE id = ?1 AND status IN ('scheduled', 'failed')",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(payload) = payload else {
            return Ok(None);
        };
        conn.execute(
            "UPDATE outbox SET status = 'cancelled', payload = '', updated_at = ?2
             WHERE id = ?1 AND status IN ('scheduled', 'failed')",
            params![id, now],
        )?;
        Ok(Some(payload))
    }

    /// `scheduled | failed → scheduled` at `now`: send now, or retry.
    pub fn reschedule_outbox_now(&self, id: &str, now: i64) -> Result<bool> {
        let n = self.connection().execute(
            "UPDATE outbox SET status = 'scheduled', send_at = ?2, last_error = NULL, failure_kind = NULL,
                               updated_at = ?2
             WHERE id = ?1 AND status IN ('scheduled', 'failed')",
            params![id, now],
        )?;
        Ok(n > 0)
    }

    /// Delete the finished rows (sent, cancelled) last touched before `before`.
    pub fn prune_finished_outbox(&self, before: i64) -> Result<usize> {
        let n = self.connection().execute(
            "DELETE FROM outbox WHERE status IN ('sent', 'cancelled') AND updated_at < ?1",
            params![before],
        )?;
        Ok(n)
    }

    /// Status of one row, for tests and guards.
    pub fn outbox_status(&self, id: &str) -> Result<OutboxStatus> {
        let status: String = self
            .reader()
            .query_row("SELECT status FROM outbox WHERE id = ?1", params![id], |row| row.get(0))
            .optional()?
            .ok_or_else(|| AppError::NotFound(format!("Outbox message {id} not found")))?;
        OutboxStatus::parse(&status).ok_or_else(|| AppError::InvalidInput(format!("unknown outbox status {status}")))
    }
}

fn prefixed_columns() -> String {
    ENTRY_COLUMNS
        .split(", ")
        .map(|c| format!("o.{c}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::provider::EmailAttachment;

    const NOW: i64 = 1_800_000_000;

    fn message() -> OutgoingMessage {
        OutgoingMessage {
            account_id: "acc-1".into(),
            reply_to_email_id: None,
            to: vec!["ana@example.com".into()],
            cc: vec!["ben@example.com".into()],
            subject: "Plans".into(),
            body: "See you".into(),
            body_html: Some("<p>See you</p>".into()),
            inline_images: vec![],
            attachments: vec![EmailAttachment {
                filename: "notes.txt".into(),
                mime_type: "text/plain".into(),
                data: "aGVsbG8=".into(),
                content_id: None,
                is_inline: false,
            }],
        }
    }

    fn db_with(rows: &[(&str, OutboxOrigin, i64)]) -> Database {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        let msg = message();
        for (id, origin, send_at) in rows {
            db.insert_outbox(&NewOutboxRow {
                id,
                message: &msg,
                origin: *origin,
                send_at: *send_at,
                now: NOW - 100,
            })
            .unwrap();
        }
        db
    }

    fn payload_of(db: &Database, id: &str) -> String {
        db.reader()
            .query_row("SELECT payload FROM outbox WHERE id = ?1", params![id], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn a_stored_message_round_trips_through_its_payload() {
        let db = db_with(&[("o1", OutboxOrigin::Scheduled, NOW + 60)]);
        let stored: OutgoingMessage = serde_json::from_str(&payload_of(&db, "o1")).unwrap();
        assert_eq!(stored, message());
        let entry = db.get_outbox_entry("o1").unwrap().unwrap();
        assert_eq!(entry.to_addresses, vec!["ana@example.com".to_string()]);
        assert_eq!(entry.cc_addresses, vec!["ben@example.com".to_string()]);
        assert_eq!(entry.attachment_count, 1);
        assert_eq!(entry.status, OutboxStatus::Scheduled);
        assert_eq!(entry.kind, OutboxKind::New);
    }

    #[test]
    fn a_reply_needs_its_parent_id_and_a_new_message_must_not_have_one() {
        let db = Database::new_for_testing().unwrap();
        db.seed_test_account("acc-1");
        let conn = db.connection();
        let bad = conn.execute(
            "INSERT INTO outbox (id, account_id, kind, origin, payload, send_at, created_at, updated_at)
             VALUES ('x', 'acc-1', 'reply', 'undo', '{}', 1, 1, 1)",
            [],
        );
        assert!(bad.is_err(), "a reply without a parent must be refused");
        let bad_status = conn.execute(
            "INSERT INTO outbox (id, account_id, kind, origin, payload, send_at, status, created_at, updated_at)
             VALUES ('y', 'acc-1', 'new', 'undo', '{}', 1, 'queued', 1, 1)",
            [],
        );
        assert!(bad_status.is_err(), "an unknown status must be refused");
    }

    #[test]
    fn only_a_due_scheduled_row_can_be_claimed_and_only_once() {
        let db = db_with(&[
            ("due", OutboxOrigin::Undo, NOW),
            ("later", OutboxOrigin::Scheduled, NOW + 1),
        ]);
        assert!(db.claim_outbox("later", NOW).unwrap().is_none(), "not due yet");
        assert!(db.claim_outbox("due", NOW).unwrap().is_some());
        assert!(
            db.claim_outbox("due", NOW).unwrap().is_none(),
            "a second claim must lose"
        );
        assert_eq!(db.outbox_status("due").unwrap(), OutboxStatus::Sending);
        assert_eq!(db.get_outbox_entry("due").unwrap().unwrap().attempts, 1);
    }

    #[test]
    fn sending_ends_sent_without_its_payload_or_failed_with_it() {
        let db = db_with(&[("ok", OutboxOrigin::Undo, NOW), ("ko", OutboxOrigin::Undo, NOW)]);
        db.claim_outbox("ok", NOW).unwrap();
        db.claim_outbox("ko", NOW).unwrap();
        assert!(db.mark_outbox_sent("ok", NOW).unwrap());
        assert!(db.mark_outbox_failed("ko", "server said no", NOW).unwrap());
        assert_eq!(db.outbox_status("ok").unwrap(), OutboxStatus::Sent);
        assert_eq!(payload_of(&db, "ok"), "", "attachment bytes must not outlive the send");
        let failed = db.get_outbox_entry("ko").unwrap().unwrap();
        assert_eq!(failed.status, OutboxStatus::Failed);
        assert_eq!(failed.last_error.as_deref(), Some("server said no"));
        assert_eq!(failed.failure_kind, Some(OutboxFailureKind::Error));
        assert!(
            !payload_of(&db, "ko").is_empty(),
            "a failed row keeps its message for retry/edit"
        );
    }

    #[test]
    fn only_a_row_being_sent_can_be_marked_sent_or_failed() {
        let db = db_with(&[
            ("waiting", OutboxOrigin::Scheduled, NOW + 60),
            ("cancelled", OutboxOrigin::Undo, NOW),
        ]);
        db.cancel_outbox("cancelled", NOW).unwrap();
        assert!(!db.mark_outbox_sent("waiting", NOW).unwrap());
        assert!(!db.mark_outbox_failed("cancelled", "too late", NOW).unwrap());
        assert_eq!(db.outbox_status("waiting").unwrap(), OutboxStatus::Scheduled);
        assert_eq!(db.outbox_status("cancelled").unwrap(), OutboxStatus::Cancelled);
    }

    #[test]
    fn a_row_left_sending_by_a_crash_fails_and_is_never_claimed_again() {
        let db = db_with(&[
            ("crashed", OutboxOrigin::Undo, NOW),
            ("waiting", OutboxOrigin::Scheduled, NOW),
        ]);
        db.claim_outbox("crashed", NOW).unwrap();
        assert_eq!(
            db.fail_interrupted_outbox(NOW + 5).unwrap(),
            vec!["crashed".to_string()]
        );
        let entry = db.get_outbox_entry("crashed").unwrap().unwrap();
        assert_eq!(entry.status, OutboxStatus::Failed);
        assert_eq!(entry.failure_kind, Some(OutboxFailureKind::Interrupted));
        assert!(db.claim_outbox("crashed", NOW + 10).unwrap().is_none());
        assert_eq!(db.outbox_status("waiting").unwrap(), OutboxStatus::Scheduled);
    }

    #[test]
    fn cancel_returns_the_message_once_and_loses_to_a_claim() {
        let db = db_with(&[("a", OutboxOrigin::Undo, NOW), ("b", OutboxOrigin::Undo, NOW)]);
        let payload = db.cancel_outbox("a", NOW).unwrap().expect("cancellable");
        assert_eq!(serde_json::from_str::<OutgoingMessage>(&payload).unwrap(), message());
        assert_eq!(db.outbox_status("a").unwrap(), OutboxStatus::Cancelled);
        assert_eq!(payload_of(&db, "a"), "");
        assert!(db.cancel_outbox("a", NOW).unwrap().is_none(), "already cancelled");
        assert!(
            db.claim_outbox("a", NOW).unwrap().is_none(),
            "a cancelled row is never sent"
        );

        db.claim_outbox("b", NOW).unwrap();
        assert!(
            db.cancel_outbox("b", NOW).unwrap().is_none(),
            "too late: it is being sent"
        );
        assert_eq!(db.outbox_status("b").unwrap(), OutboxStatus::Sending);
    }

    #[test]
    fn a_failed_row_can_be_retried_now_or_cancelled() {
        let db = db_with(&[("f", OutboxOrigin::Scheduled, NOW), ("g", OutboxOrigin::Scheduled, NOW)]);
        for id in ["f", "g"] {
            db.claim_outbox(id, NOW).unwrap();
            db.mark_outbox_failed(id, "offline", NOW).unwrap();
        }
        assert!(db.reschedule_outbox_now("f", NOW + 30).unwrap());
        let retried = db.get_outbox_entry("f").unwrap().unwrap();
        assert_eq!(retried.status, OutboxStatus::Scheduled);
        assert_eq!(retried.send_at, NOW + 30);
        assert_eq!(retried.last_error, None);
        assert!(db.cancel_outbox("g", NOW).unwrap().is_some());
    }

    #[test]
    fn send_now_moves_a_scheduled_row_forward_but_never_a_sent_one() {
        let db = db_with(&[
            ("s", OutboxOrigin::Scheduled, NOW + 86_400),
            ("done", OutboxOrigin::Undo, NOW),
        ]);
        assert!(db.reschedule_outbox_now("s", NOW).unwrap());
        assert!(db.claim_outbox("s", NOW).unwrap().is_some());
        db.claim_outbox("done", NOW).unwrap();
        db.mark_outbox_sent("done", NOW).unwrap();
        assert!(!db.reschedule_outbox_now("done", NOW).unwrap());
    }

    #[test]
    fn the_scheduled_view_lists_waiting_and_failed_rows_but_not_undo_windows() {
        let db = db_with(&[
            ("undo", OutboxOrigin::Undo, NOW + 10),
            ("later", OutboxOrigin::Scheduled, NOW + 3600),
            ("soon", OutboxOrigin::Scheduled, NOW + 60),
            ("undo-failed", OutboxOrigin::Undo, NOW),
            ("cancelled", OutboxOrigin::Scheduled, NOW + 60),
        ]);
        db.claim_outbox("undo-failed", NOW).unwrap();
        db.mark_outbox_failed("undo-failed", "offline", NOW).unwrap();
        db.cancel_outbox("cancelled", NOW).unwrap();
        let ids = |rows: Vec<OutboxEntry>| rows.into_iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(
            ids(db.list_outbox(AccountScope::Account("acc-1")).unwrap()),
            vec!["undo-failed", "soon", "later"]
        );
        assert_eq!(
            ids(db.list_outbox(AccountScope::AllEnabled).unwrap()),
            vec!["undo-failed", "soon", "later"]
        );
        assert!(db.list_outbox(AccountScope::Account("other")).unwrap().is_empty());
    }

    #[test]
    fn finished_rows_are_pruned_after_a_while() {
        let db = db_with(&[
            ("sent", OutboxOrigin::Undo, NOW),
            ("waiting", OutboxOrigin::Scheduled, NOW + 60),
        ]);
        db.claim_outbox("sent", NOW).unwrap();
        db.mark_outbox_sent("sent", NOW).unwrap();
        assert_eq!(db.prune_finished_outbox(NOW).unwrap(), 0, "too recent");
        assert_eq!(db.prune_finished_outbox(NOW + 1).unwrap(), 1);
        assert!(db.get_outbox_entry("sent").unwrap().is_none());
        assert!(db.get_outbox_entry("waiting").unwrap().is_some());
    }
}
